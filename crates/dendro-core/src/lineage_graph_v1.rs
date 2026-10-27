//! `A06` property graph over `session_edges` and `security_facts`.
//!
//! A hash-pinned `duckpgq` blob in `extension_directory` is loaded from that
//! file. The open never issues a network install. When the blob is absent,
//! mismatched, or refused, reachability is the deterministic session-lineage
//! fallback over [`crate::session_lineage::SessionLink`].
//!
//! Reachability walks only `status = evidenced` and the fixed direction table.
//! `absent`, `truncated`, and `conflict` rows stay stored and are reported as
//! evidence counts. Cycles are reported and are not deleted. The hop cap
//! defaults to 8. An unbounded path query is refused.

use crate::session_lineage::SessionLink;
use anyhow::{anyhow, bail, Result};
use duckdb::Connection;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Default bound for a blast-radius walk. Callers cannot omit the cap.
pub const DEFAULT_HOP_CAP: u32 = 8;

/// Carried on every blast-radius answer, including computed and partial ones.
pub const DEPENDENCY_EXPOSURE_NON_CLAIM: &str =
    "dependency_exposure_only: structural reachability does not imply inherited or effective security authority";

/// Applied only after a pinned blob has loaded, and only before configuration lock.
pub const ACTIVE_GRAMMAR_SQL: &str = "SET active_grammar_extensions = ['duckpgq']";

pub const LOCK_CONFIGURATION_SQL: &str = "SET lock_configuration=true";

/// File name inside `extension_directory`. Not a client path.
pub const DUCKPGQ_EXTENSION_FILE: &str = "duckpgq.duckdb_extension";

/// Bounded downstream step. The hop number is substituted by the walker.
/// This text is the `I24` query shape. It has no centrality statistic.
pub const I24_FROZEN_SQL: &str = "\
SELECT DISTINCT a.to_session
FROM lineage_arcs a
JOIN lineage_frontier f ON a.from_session = f.session_id";

/// Property-graph DDL. Recorded next to the recursive cross-check. It is not
/// executed on the caller's answer connection.
pub const DUCKPGQ_GRAPH_SQL: &str = "\
CREATE OR REPLACE PROPERTY GRAPH dendro_lineage
VERTEX TABLES (lineage_sessions)
EDGE TABLES (
    lineage_arcs
        SOURCE KEY (from_session) REFERENCES lineage_sessions (session_id)
        DESTINATION KEY (to_session) REFERENCES lineage_sessions (session_id)
        LABEL Traverses
)";

/// Read-only PGQ shape that belongs with [`DUCKPGQ_GRAPH_SQL`]. Not executed
/// on the answer connection, because that would require the property graph.
pub const DUCKPGQ_REACHABILITY_SQL: &str = "\
SELECT DISTINCT b.session_id
FROM GRAPH_TABLE (dendro_lineage
    MATCH (a)-[e:Traverses]->{1,8}(b)
    WHERE a.session_id = origin
    COLUMNS (b.session_id)
)";

/// Bounded read of `session_edges`. No DDL and no writes.
pub const RECURSIVE_REACHABILITY_SQL: &str = "\
WITH RECURSIVE
edges AS (
    SELECT CASE
            WHEN relation IN ('spawn', 'delegate', 'delegate_existing', 'handoff', 'retry')
                THEN parent_session
            WHEN relation IN ('return', 'join') THEN child_session
        END AS from_session,
        CASE
            WHEN relation IN ('spawn', 'delegate', 'delegate_existing', 'handoff', 'retry')
                THEN child_session
            WHEN relation IN ('return', 'join') THEN parent_session
        END AS to_session
    FROM session_edges
    WHERE status = 'evidenced'
),
walk(session_id, hop, seen) AS (
    SELECT origin_session, 0::BIGINT, [origin_session]::VARCHAR[]
    UNION ALL
    SELECT e.to_session, w.hop + 1, list_append(w.seen, e.to_session)
    FROM walk w
    JOIN edges e ON e.from_session = w.session_id
    WHERE w.hop < hop_cap
      AND e.from_session IS NOT NULL
      AND e.to_session IS NOT NULL
      AND e.from_session <> ''
      AND e.to_session <> ''
      AND NOT list_contains(w.seen, e.to_session)
)
SELECT session_id, MIN(hop)::BIGINT AS hop
FROM walk
WHERE hop > 0
  AND session_id <> origin_session
GROUP BY session_id";

pub const SESSION_EDGES_SQL: &str = "\
CREATE TABLE IF NOT EXISTS session_edges (
    source_key VARCHAR PRIMARY KEY,
    source_id VARCHAR NOT NULL,
    harness VARCHAR NOT NULL,
    parent_session VARCHAR NOT NULL,
    child_session VARCHAR NOT NULL,
    relation VARCHAR NOT NULL,
    status VARCHAR NOT NULL,
    observed_at TIMESTAMP,
    evidence_kind VARCHAR NOT NULL,
    generation BIGINT NOT NULL
)";

pub const SECURITY_FACTS_SQL: &str = "\
CREATE TABLE IF NOT EXISTS security_facts (
    source_key VARCHAR PRIMARY KEY,
    source_id VARCHAR NOT NULL,
    harness VARCHAR NOT NULL,
    native_session VARCHAR,
    ts TIMESTAMP,
    kind VARCHAR NOT NULL,
    value VARCHAR NOT NULL,
    source_field VARCHAR NOT NULL,
    generation BIGINT NOT NULL
)";

/// `None` is an unbounded path and is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HopCap {
    Bounded(u32),
    Unbounded,
}

impl HopCap {
    pub const DEFAULT: Self = Self::Bounded(DEFAULT_HOP_CAP);
}

impl Default for HopCap {
    fn default() -> Self {
        Self::DEFAULT
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnswerStatus {
    Computed,
    Partial,
    Refused,
    NotAssessed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Traversal {
    /// `spawn`, `delegate`, `delegate_existing`, `handoff`, and `retry`
    /// (prior attempt stored in `parent_session`, later attempt in `child_session`).
    ParentToChild,
    /// `return` and `join`. `join` walks back to the waiting assignment and
    /// does not add the child, or the child's descendants, to the parent's set.
    ChildToParent,
    /// `resume`, `dependency`, and any other stored kind. Labeled, not walked.
    StoredOnly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FallbackReason {
    BlobAbsent,
    HashMismatch,
    LoadRefused,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphBackend {
    Duckpgq,
    SessionLineageFallback { reason: FallbackReason },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlastRadius {
    pub status: AnswerStatus,
    pub origin: String,
    pub downstream: BTreeSet<String>,
    pub hops: BTreeMap<String, u32>,
    pub hop_histogram: BTreeMap<u32, u64>,
    pub cycles: BTreeSet<Vec<String>>,
    /// True when `cycles` is non-empty, including a cycle through the origin.
    pub cycle_detected: bool,
    pub truncated_frontier: BTreeSet<String>,
    pub truncated_frontier_count: u64,
    pub evidence_by_status: BTreeMap<String, u64>,
    pub evidence_by_relation: BTreeMap<String, u64>,
    pub non_claim: &'static str,
    /// Structural walk only. Always false.
    pub rank_statistic_used: bool,
    /// No work-item grain is stored on `session_edges`, so this stays unknown.
    pub work_item_count: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityObservation {
    pub native_session: Option<String>,
    pub kind: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct I24Answer {
    pub status: AnswerStatus,
    pub downstream_count: u64,
    pub downstream: BTreeSet<String>,
    pub hop_histogram: BTreeMap<u32, u64>,
    pub cycles: BTreeSet<Vec<String>>,
    pub truncated_frontier_count: u64,
    pub evidence_by_status: BTreeMap<String, u64>,
    pub evidence_by_relation: BTreeMap<String, u64>,
    pub non_claim: &'static str,
    pub rank_statistic_used: bool,
    /// Typed artifact identities are not on this graph.
    pub artifact_count: Option<u64>,
}

pub struct LineageGraph {
    backend: GraphBackend,
    statements: Vec<String>,
}

impl LineageGraph {
    pub fn backend(&self) -> &GraphBackend {
        &self.backend
    }

    pub fn statements(&self) -> &[String] {
        &self.statements
    }

    /// Verify the pinned blob, record the open statements, and lock configuration.
    /// Grammar is enabled only after a local load succeeds, and only before the lock.
    pub fn open(conn: &Connection, extension_dir: &Path, sha256_hex: &str) -> Result<Self> {
        if !extension_dir.is_dir() {
            bail!("extension directory is absent");
        }
        if configuration_locked(conn) {
            return Ok(Self {
                backend: GraphBackend::SessionLineageFallback {
                    reason: FallbackReason::LoadRefused,
                },
                statements: vec![
                    RECURSIVE_REACHABILITY_SQL.to_string(),
                    DUCKPGQ_GRAPH_SQL.to_string(),
                    DUCKPGQ_REACHABILITY_SQL.to_string(),
                ],
            });
        }
        let blob = extension_dir.join(DUCKPGQ_EXTENSION_FILE);
        let mut statements = Vec::new();
        exec(
            conn,
            &mut statements,
            "SET autoinstall_known_extensions=false",
        )?;
        exec(conn, &mut statements, "SET autoload_known_extensions=false")?;
        let directory = sql_literal(&native_path(extension_dir)?)?;
        exec(
            conn,
            &mut statements,
            &format!("SET extension_directory={directory}"),
        )?;

        let backend = match classify_blob(&blob, sha256_hex)? {
            BlobClass::Absent => GraphBackend::SessionLineageFallback {
                reason: FallbackReason::BlobAbsent,
            },
            BlobClass::HashMismatch => GraphBackend::SessionLineageFallback {
                reason: FallbackReason::HashMismatch,
            },
            BlobClass::Refused => GraphBackend::SessionLineageFallback {
                reason: FallbackReason::LoadRefused,
            },
            BlobClass::Loadable(path) => {
                let actual = file_sha256_hex(&path)?;
                if !pin_matches(&actual, sha256_hex) {
                    GraphBackend::SessionLineageFallback {
                        reason: FallbackReason::HashMismatch,
                    }
                } else {
                    let literal = sql_literal(&native_path(&path)?)?;
                    let load_sql = format!("LOAD {literal}");
                    statements.push(load_sql.clone());
                    if conn.execute_batch(&load_sql).is_err() {
                        GraphBackend::SessionLineageFallback {
                            reason: FallbackReason::LoadRefused,
                        }
                    } else {
                        exec(conn, &mut statements, ACTIVE_GRAMMAR_SQL)?;
                        GraphBackend::Duckpgq
                    }
                }
            }
        };

        exec(
            conn,
            &mut statements,
            "SET allowed_configs=['memory_limit','threads','preserve_insertion_order']",
        )?;
        exec(conn, &mut statements, LOCK_CONFIGURATION_SQL)?;
        if matches!(backend, GraphBackend::Duckpgq) {
            let grammar = statements.iter().position(|sql| sql == ACTIVE_GRAMMAR_SQL);
            let lock = statements
                .iter()
                .position(|sql| sql == LOCK_CONFIGURATION_SQL);
            match (grammar, lock) {
                (Some(grammar), Some(lock)) if grammar < lock => {}
                _ => bail!("duckpgq grammar was not set before lock_configuration"),
            }
        }
        Ok(Self {
            backend,
            statements,
        })
    }

    pub fn ensure_fact_tables(conn: &Connection) -> Result<()> {
        conn.execute_batch(SESSION_EDGES_SQL)?;
        conn.execute_batch(SECURITY_FACTS_SQL)?;
        Ok(())
    }

    /// Read `session_edges` and answer with the fallback. When `duckpgq` loaded,
    /// the same rows are also checked through the bounded SQL walk.
    pub fn evaluate(
        &mut self,
        conn: &Connection,
        origin: &str,
        hop_cap: HopCap,
    ) -> Result<BlastRadius> {
        let edges = load_edges(conn)?;
        let answer = blast_radius(&edges, origin, hop_cap)?;
        self.cross_check(conn, &answer)?;
        Ok(answer)
    }

    pub fn evaluate_i24(&mut self, conn: &Connection, hop_cap: HopCap) -> Result<I24Answer> {
        let edges = load_edges(conn)?;
        let facts = load_security(conn)?;
        let answer = i24_blast_radius(&edges, &facts, hop_cap)?;
        if answer.status != AnswerStatus::NotAssessed {
            if let Some(origin) = facts.iter().find_map(|fact| {
                if is_i24_origin(&fact.kind, &fact.value) {
                    fact.native_session.clone()
                } else {
                    None
                }
            }) {
                let structural = blast_radius(&edges, &origin, hop_cap)?;
                self.cross_check(conn, &structural)?;
            }
        }
        Ok(answer)
    }

    fn cross_check(&mut self, conn: &Connection, answer: &BlastRadius) -> Result<()> {
        self.statements.push(I24_FROZEN_SQL.to_string());
        self.statements.push(RECURSIVE_REACHABILITY_SQL.to_string());
        self.statements.push(DUCKPGQ_GRAPH_SQL.to_string());
        self.statements.push(DUCKPGQ_REACHABILITY_SQL.to_string());
        compare_sql_walk(conn, answer)?;
        Ok(())
    }
}

fn configuration_locked(conn: &Connection) -> bool {
    conn.query_row(
        "SELECT current_setting('lock_configuration')::VARCHAR",
        [],
        |row| row.get::<_, String>(0),
    )
    .map(|value| value.eq_ignore_ascii_case("true"))
    .unwrap_or(false)
}

fn exec(conn: &Connection, statements: &mut Vec<String>, sql: &str) -> Result<()> {
    statements.push(sql.to_string());
    conn.execute_batch(sql)?;
    Ok(())
}

enum BlobClass {
    Absent,
    HashMismatch,
    Refused,
    Loadable(PathBuf),
}

fn classify_blob(path: &Path, sha256_hex: &str) -> Result<BlobClass> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(BlobClass::Absent),
        Err(error) => return Err(error.into()),
    };
    if meta.file_type().is_symlink() || !meta.file_type().is_file() {
        return Ok(BlobClass::Refused);
    }
    let actual = file_sha256_hex(path)?;
    if !pin_matches(&actual, sha256_hex) {
        return Ok(BlobClass::HashMismatch);
    }
    if !extension_magic_ok(path)? {
        return Ok(BlobClass::Refused);
    }
    Ok(BlobClass::Loadable(path.to_path_buf()))
}

fn pin_matches(actual: &str, expected: &str) -> bool {
    let expected = expected.trim();
    expected.len() == 64
        && expected.chars().all(|ch| ch.is_ascii_hexdigit())
        && actual.eq_ignore_ascii_case(expected)
}

fn extension_magic_ok(path: &Path) -> Result<bool> {
    let mut file = File::open(path)?;
    let mut header = [0_u8; 4];
    let read = file.read(&mut header)?;
    if read < 4 {
        return Ok(false);
    }
    Ok(matches!(
        header,
        [0x7f, b'E', b'L', b'F']
            | [0x1f, 0x8b, _, _]
            | [0xfe, 0xed, 0xfa, 0xce]
            | [0xfe, 0xed, 0xfa, 0xcf]
            | [0xcf, 0xfa, 0xed, 0xfe]
            | [0xce, 0xfa, 0xed, 0xfe]
    ))
}

pub fn file_sha256_hex(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Statements used when a pinned blob is about to load. Grammar comes before the lock.
pub fn duckpgq_open_statements(extension_directory: &str, blob_path: &str) -> Result<Vec<String>> {
    let directory = sql_literal(extension_directory)?;
    let blob = sql_literal(blob_path)?;
    Ok(vec![
        "SET autoinstall_known_extensions=false".to_string(),
        "SET autoload_known_extensions=false".to_string(),
        format!("SET extension_directory={directory}"),
        format!("LOAD {blob}"),
        ACTIVE_GRAMMAR_SQL.to_string(),
        "SET allowed_configs=['memory_limit','threads','preserve_insertion_order']".to_string(),
        LOCK_CONFIGURATION_SQL.to_string(),
    ])
}

pub fn traversal_of(relation: &str) -> Traversal {
    match relation {
        "spawn" | "delegate" | "delegate_existing" | "handoff" | "retry" => {
            Traversal::ParentToChild
        }
        "return" | "join" => Traversal::ChildToParent,
        _ => Traversal::StoredOnly,
    }
}

pub fn is_i24_origin(kind: &str, value: &str) -> bool {
    matches!(
        (kind, value),
        ("grant", "revoked") | ("grant", "failed") | ("sandbox", "unverified")
    )
}

pub fn blast_radius(edges: &[SessionLink], origin: &str, hop_cap: HopCap) -> Result<BlastRadius> {
    if origin.is_empty() {
        bail!("origin is empty");
    }
    let HopCap::Bounded(hop_cap) = hop_cap else {
        bail!("unbounded path query refused");
    };
    let projected = project_edges(edges);
    let (hops, truncated_frontier) = shortest_paths(&projected.adjacency, origin, hop_cap)?;
    let cycles = cycles_touched(&projected.adjacency, &hops, origin);
    let mut downstream = BTreeSet::new();
    let mut hop_histogram = BTreeMap::new();
    for (node, hop) in &hops {
        if *hop == 0 {
            continue;
        }
        downstream.insert(node.clone());
        *hop_histogram.entry(*hop).or_default() += 1;
    }
    let status = if truncated_frontier.is_empty() {
        AnswerStatus::Computed
    } else {
        AnswerStatus::Partial
    };
    let cycle_detected = !cycles.is_empty();
    Ok(BlastRadius {
        status,
        origin: origin.to_string(),
        downstream,
        hops,
        hop_histogram,
        cycles,
        cycle_detected,
        truncated_frontier_count: u64::try_from(truncated_frontier.len()).unwrap_or(u64::MAX),
        truncated_frontier,
        evidence_by_status: projected.evidence_by_status,
        evidence_by_relation: projected.evidence_by_relation,
        non_claim: DEPENDENCY_EXPOSURE_NON_CLAIM,
        rank_statistic_used: false,
        work_item_count: None,
    })
}

pub fn i24_blast_radius(
    edges: &[SessionLink],
    facts: &[SecurityObservation],
    hop_cap: HopCap,
) -> Result<I24Answer> {
    if matches!(hop_cap, HopCap::Unbounded) {
        bail!("unbounded path query refused");
    }
    let (evidence_by_status, evidence_by_relation) = evidence_maps(edges);
    let origins = origin_sessions(facts);
    if origins.is_empty() {
        return Ok(I24Answer {
            status: AnswerStatus::NotAssessed,
            downstream_count: 0,
            downstream: BTreeSet::new(),
            hop_histogram: BTreeMap::new(),
            cycles: BTreeSet::new(),
            truncated_frontier_count: 0,
            evidence_by_status,
            evidence_by_relation,
            non_claim: DEPENDENCY_EXPOSURE_NON_CLAIM,
            rank_statistic_used: false,
            artifact_count: None,
        });
    }
    let mut merged_hops: BTreeMap<String, u32> = BTreeMap::new();
    let mut cycles = BTreeSet::new();
    let mut frontier = BTreeSet::new();
    for origin in origins {
        let one = blast_radius(edges, &origin, hop_cap)?;
        if one.non_claim != DEPENDENCY_EXPOSURE_NON_CLAIM || one.rank_statistic_used {
            bail!("blast-radius answer dropped its dependency-exposure non-claim");
        }
        for (node, hop) in one.hops {
            if hop == 0 {
                continue;
            }
            let entry = merged_hops.entry(node).or_insert(hop);
            if hop < *entry {
                *entry = hop;
            }
        }
        cycles.extend(one.cycles);
        frontier.extend(one.truncated_frontier);
    }
    let mut hop_histogram = BTreeMap::new();
    for hop in merged_hops.values() {
        *hop_histogram.entry(*hop).or_default() += 1;
    }
    let status = if frontier.is_empty() {
        AnswerStatus::Computed
    } else {
        AnswerStatus::Partial
    };
    Ok(I24Answer {
        status,
        downstream_count: u64::try_from(merged_hops.len()).unwrap_or(u64::MAX),
        downstream: merged_hops.into_keys().collect(),
        hop_histogram,
        cycles,
        truncated_frontier_count: u64::try_from(frontier.len()).unwrap_or(u64::MAX),
        evidence_by_status,
        evidence_by_relation,
        non_claim: DEPENDENCY_EXPOSURE_NON_CLAIM,
        rank_statistic_used: false,
        artifact_count: None,
    })
}

fn origin_sessions(facts: &[SecurityObservation]) -> BTreeSet<String> {
    facts
        .iter()
        .filter(|fact| is_i24_origin(&fact.kind, &fact.value))
        .filter_map(|fact| {
            fact.native_session
                .as_ref()
                .filter(|session| !session.is_empty())
                .cloned()
        })
        .collect()
}

struct EdgeProjection {
    adjacency: BTreeMap<String, BTreeSet<String>>,
    evidence_by_status: BTreeMap<String, u64>,
    evidence_by_relation: BTreeMap<String, u64>,
}

fn project_edges(edges: &[SessionLink]) -> EdgeProjection {
    let (evidence_by_status, evidence_by_relation) = evidence_maps(edges);
    let mut adjacency: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for edge in edges {
        if edge.status != "evidenced" {
            continue;
        }
        let (from, to) = match traversal_of(&edge.relation) {
            Traversal::ParentToChild => (&edge.parent_session, &edge.child_session),
            Traversal::ChildToParent => (&edge.child_session, &edge.parent_session),
            Traversal::StoredOnly => continue,
        };
        if from.is_empty() || to.is_empty() {
            continue;
        }
        adjacency
            .entry(from.clone())
            .or_default()
            .insert(to.clone());
    }
    EdgeProjection {
        adjacency,
        evidence_by_status,
        evidence_by_relation,
    }
}

fn evidence_maps(edges: &[SessionLink]) -> (BTreeMap<String, u64>, BTreeMap<String, u64>) {
    let mut by_status = BTreeMap::new();
    let mut by_relation = BTreeMap::new();
    for edge in edges {
        *by_status.entry(edge.status.clone()).or_default() += 1;
        *by_relation.entry(edge.relation.clone()).or_default() += 1;
    }
    (by_status, by_relation)
}

fn shortest_paths(
    adj: &BTreeMap<String, BTreeSet<String>>,
    origin: &str,
    hop_cap: u32,
) -> Result<(BTreeMap<String, u32>, BTreeSet<String>)> {
    let mut hops = BTreeMap::new();
    let mut queue = VecDeque::new();
    let mut frontier = BTreeSet::new();
    hops.insert(origin.to_string(), 0);
    queue.push_back(origin.to_string());
    while let Some(node) = queue.pop_front() {
        let depth = hops[&node];
        let Some(nexts) = adj.get(&node) else {
            continue;
        };
        if depth == hop_cap {
            for next in nexts {
                if !hops.contains_key(next) {
                    frontier.insert(next.clone());
                }
            }
            continue;
        }
        for next in nexts {
            if hops.contains_key(next) {
                continue;
            }
            hops.insert(next.clone(), depth + 1);
            queue.push_back(next.clone());
        }
    }
    let relaxed = relax(adj, origin, hop_cap);
    if relaxed != hops {
        bail!("session_lineage fallback diverged");
    }
    Ok((hops, frontier))
}

fn relax(
    adj: &BTreeMap<String, BTreeSet<String>>,
    origin: &str,
    hop_cap: u32,
) -> BTreeMap<String, u32> {
    let mut dist = BTreeMap::new();
    dist.insert(origin.to_string(), 0);
    for _ in 0..hop_cap {
        let snapshot = dist.clone();
        for (node, depth) in snapshot {
            if depth >= hop_cap {
                continue;
            }
            let Some(nexts) = adj.get(&node) else {
                continue;
            };
            for next in nexts {
                let candidate = depth + 1;
                let entry = dist.entry(next.clone()).or_insert(u32::MAX);
                if candidate < *entry {
                    *entry = candidate;
                }
            }
        }
    }
    dist
}

fn cycles_touched(
    adj: &BTreeMap<String, BTreeSet<String>>,
    hops: &BTreeMap<String, u32>,
    origin: &str,
) -> BTreeSet<Vec<String>> {
    let mut ball: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (from, nexts) in adj {
        if !hops.contains_key(from) {
            continue;
        }
        for to in nexts {
            if hops.contains_key(to) {
                ball.entry(from.clone()).or_default().insert(to.clone());
            }
        }
    }
    let mut cycles = BTreeSet::new();
    let mut color: BTreeMap<String, u8> = BTreeMap::new();
    let mut stack = Vec::new();
    dfs_cycles(&ball, origin, &mut color, &mut stack, &mut cycles);
    cycles
}

fn dfs_cycles(
    adj: &BTreeMap<String, BTreeSet<String>>,
    node: &str,
    color: &mut BTreeMap<String, u8>,
    stack: &mut Vec<String>,
    cycles: &mut BTreeSet<Vec<String>>,
) {
    if color.get(node).copied().unwrap_or(0) != 0 {
        return;
    }
    color.insert(node.to_string(), 1);
    stack.push(node.to_string());
    if let Some(nexts) = adj.get(node) {
        for next in nexts {
            match color.get(next).copied().unwrap_or(0) {
                1 => {
                    if let Some(start) = stack.iter().position(|seen| seen == next) {
                        let cycle = canonicalize(&stack[start..]);
                        if !cycle.is_empty() {
                            cycles.insert(cycle);
                        }
                    }
                }
                0 => dfs_cycles(adj, next, color, stack, cycles),
                _ => {}
            }
        }
    }
    stack.pop();
    color.insert(node.to_string(), 2);
}

fn canonicalize(cycle: &[String]) -> Vec<String> {
    let Some(start) = cycle
        .iter()
        .enumerate()
        .min_by(|left, right| left.1.cmp(right.1).then(left.0.cmp(&right.0)))
        .map(|(index, _)| index)
    else {
        return Vec::new();
    };
    cycle[start..]
        .iter()
        .chain(cycle[..start].iter())
        .cloned()
        .collect()
}

fn load_edges(conn: &Connection) -> Result<Vec<SessionLink>> {
    let mut stmt = conn.prepare(
        "SELECT parent_session, child_session, relation, status
         FROM session_edges
         ORDER BY source_key",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(SessionLink {
            parent_session: row.get(0)?,
            child_session: row.get(1)?,
            relation: row.get(2)?,
            status: row.get(3)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn load_security(conn: &Connection) -> Result<Vec<SecurityObservation>> {
    let mut stmt =
        conn.prepare("SELECT native_session, kind, value FROM security_facts ORDER BY source_key")?;
    let rows = stmt.query_map([], |row| {
        Ok(SecurityObservation {
            native_session: row.get(0)?,
            kind: row.get(1)?,
            value: row.get(2)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn compare_sql_walk(conn: &Connection, answer: &BlastRadius) -> Result<()> {
    let cap = answer.hops.values().copied().max().unwrap_or(0);
    let sql_hops = recursive_sql_hops(conn, &answer.origin, cap)?;
    let mut rust_hops = answer.hops.clone();
    rust_hops.remove(&answer.origin);
    if sql_hops != rust_hops {
        bail!("session_lineage fallback diverged from the bounded sql walk");
    }
    let deeper = recursive_sql_hops(conn, &answer.origin, cap.saturating_add(1))?;
    let extra: BTreeSet<String> = deeper
        .into_keys()
        .filter(|node| !rust_hops.contains_key(node))
        .collect();
    if extra != answer.truncated_frontier {
        bail!("session_lineage fallback diverged from the bounded sql walk");
    }
    Ok(())
}

fn recursive_sql_hops(
    conn: &Connection,
    origin: &str,
    hop_cap: u32,
) -> Result<BTreeMap<String, u32>> {
    let origin_sql = sql_literal(origin)?;
    let sql = format!(
        "WITH RECURSIVE
         edges AS (
             SELECT CASE
                     WHEN relation IN ('spawn', 'delegate', 'delegate_existing', 'handoff', 'retry')
                         THEN parent_session
                     WHEN relation IN ('return', 'join') THEN child_session
                 END AS from_session,
                 CASE
                     WHEN relation IN ('spawn', 'delegate', 'delegate_existing', 'handoff', 'retry')
                         THEN child_session
                     WHEN relation IN ('return', 'join') THEN parent_session
                 END AS to_session
             FROM session_edges
             WHERE status = 'evidenced'
         ),
         walk(session_id, hop, seen) AS (
             SELECT {origin_sql}, 0::BIGINT, [{origin_sql}]::VARCHAR[]
             UNION ALL
             SELECT e.to_session, w.hop + 1, list_append(w.seen, e.to_session)
             FROM walk w
             JOIN edges e ON e.from_session = w.session_id
             WHERE w.hop < {hop_cap}
               AND e.from_session IS NOT NULL
               AND e.to_session IS NOT NULL
               AND e.from_session <> ''
               AND e.to_session <> ''
               AND NOT list_contains(w.seen, e.to_session)
         )
         SELECT session_id, MIN(hop)::BIGINT AS hop
         FROM walk
         WHERE hop > 0
           AND session_id <> {origin_sql}
         GROUP BY session_id"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    let mut hops = BTreeMap::new();
    for row in rows {
        let (session, hop) = row?;
        if session == origin {
            continue;
        }
        let hop = u32::try_from(hop).map_err(|_| anyhow!("sql hop does not fit u32"))?;
        hops.insert(session, hop);
    }
    Ok(hops)
}

fn native_path(path: &Path) -> Result<String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("non-Unicode extension path"))
}

fn sql_literal(value: &str) -> Result<String> {
    if value
        .chars()
        .any(|ch| ch == '\0' || ch == '\n' || ch == '\r')
    {
        bail!("sql literal contains a control character");
    }
    Ok(format!("'{}'", value.replace('\'', "''")))
}
