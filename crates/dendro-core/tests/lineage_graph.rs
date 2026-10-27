//! `A06` proof: evidenced reachability, direction, cycles, hop cap, and fallback.

use dendro_core::lineage_graph_v1::{
    blast_radius, duckpgq_open_statements, file_sha256_hex, i24_blast_radius, AnswerStatus,
    FallbackReason, GraphBackend, HopCap, LineageGraph, SecurityObservation, ACTIVE_GRAMMAR_SQL,
    DEFAULT_HOP_CAP, DEPENDENCY_EXPOSURE_NON_CLAIM, DUCKPGQ_EXTENSION_FILE, DUCKPGQ_GRAPH_SQL,
    I24_FROZEN_SQL, LOCK_CONFIGURATION_SQL,
};
use dendro_core::session_lineage::SessionLink;
use duckdb::Connection;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "dendro-a06-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(&path).expect("temp dir");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn edge(parent: &str, child: &str, relation: &str, status: &str) -> SessionLink {
    SessionLink {
        parent_session: parent.to_string(),
        child_session: child.to_string(),
        relation: relation.to_string(),
        status: status.to_string(),
    }
}

fn insert_edges(conn: &Connection, edges: &[SessionLink]) {
    LineageGraph::ensure_fact_tables(conn).expect("fact tables");
    for (index, edge) in edges.iter().enumerate() {
        conn.execute(
            "INSERT INTO session_edges
             (source_key, source_id, harness, parent_session, child_session, relation, status, observed_at, evidence_kind, generation)
             VALUES (?, 'src', 'claude', ?, ?, ?, ?, NULL, 'explicit', 1)",
            duckdb::params![
                format!("edge-{index}"),
                edge.parent_session,
                edge.child_session,
                edge.relation,
                edge.status
            ],
        )
        .expect("insert edge");
    }
}

fn assert_no_network_install(statements: &[String]) {
    for statement in statements {
        let upper = statement.to_ascii_uppercase();
        assert!(
            !upper.trim_start().starts_with("INSTALL"),
            "network install statement: {statement}"
        );
        assert!(!upper.contains(" FROM COMMUNITY"));
        assert!(!statement.contains("http://") && !statement.contains("https://"));
    }
}

const PIN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

#[test]
fn evidenced_only_reachability_skips_absent_truncated_and_conflict() {
    let edges = [
        edge("a", "kept", "spawn", "evidenced"),
        edge("kept", "grand", "delegate", "evidenced"),
        edge("a", "missing", "spawn", "absent"),
        edge("a", "cut", "handoff", "truncated"),
        edge("a", "clash", "spawn", "conflict"),
        edge("missing", "hidden", "spawn", "evidenced"),
    ];
    let got = blast_radius(&edges, "a", HopCap::DEFAULT).expect("walk");
    assert_eq!(got.status, AnswerStatus::Computed);
    assert_eq!(
        got.downstream,
        ["grand".to_string(), "kept".to_string()].into()
    );
    assert!(!got.downstream.contains("missing"));
    assert!(!got.downstream.contains("cut"));
    assert!(!got.downstream.contains("clash"));
    assert!(!got.downstream.contains("hidden"));
    assert_eq!(got.evidence_by_status.get("evidenced"), Some(&3));
    assert_eq!(got.evidence_by_status.get("absent"), Some(&1));
    assert_eq!(got.evidence_by_status.get("truncated"), Some(&1));
    assert_eq!(got.evidence_by_status.get("conflict"), Some(&1));
    assert_eq!(edges.len(), 6);
    assert_eq!(got.non_claim, DEPENDENCY_EXPOSURE_NON_CLAIM);
    assert!(!got.rank_statistic_used);
    assert_eq!(got.work_item_count, None);
}

#[test]
fn direction_spawn_delegate_and_handoff_are_parent_to_child() {
    for relation in ["spawn", "delegate", "delegate_existing", "handoff"] {
        let edges = [edge("parent", "child", relation, "evidenced")];
        let downstream = blast_radius(&edges, "parent", HopCap::DEFAULT)
            .expect(relation)
            .downstream;
        assert!(
            downstream.contains("child"),
            "{relation} did not walk parent to child"
        );
        let upward = blast_radius(&edges, "child", HopCap::DEFAULT)
            .expect(relation)
            .downstream;
        assert!(
            upward.is_empty(),
            "{relation} walked against the direction table"
        );
    }
}

#[test]
fn direction_return_walks_child_to_parent() {
    let edges = [edge("caller", "executor", "return", "evidenced")];
    let from_child = blast_radius(&edges, "executor", HopCap::DEFAULT).expect("return");
    assert!(from_child.downstream.contains("caller"));
    let from_parent = blast_radius(&edges, "caller", HopCap::DEFAULT).expect("return");
    assert!(from_parent.downstream.is_empty());
}

#[test]
fn join_walks_to_the_waiter_and_does_not_import_child_descendants() {
    let edges = [
        edge("waiter", "child", "join", "evidenced"),
        edge("child", "grandchild", "spawn", "evidenced"),
        edge("waiter", "other", "spawn", "evidenced"),
    ];
    let parent = blast_radius(&edges, "waiter", HopCap::DEFAULT).expect("join parent");
    assert_eq!(parent.downstream, ["other".to_string()].into());
    assert!(!parent.downstream.contains("child"));
    assert!(!parent.downstream.contains("grandchild"));
    let child = blast_radius(&edges, "child", HopCap::DEFAULT).expect("join child");
    assert!(child.downstream.contains("waiter"));
    assert!(child.downstream.contains("grandchild"));
    assert!(child.downstream.contains("other"));
}

#[test]
fn direction_retry_walks_prior_attempt_to_later_attempt() {
    let edges = [edge("prior", "later", "retry", "evidenced")];
    let forward = blast_radius(&edges, "prior", HopCap::DEFAULT).expect("retry");
    assert!(forward.downstream.contains("later"));
    let backward = blast_radius(&edges, "later", HopCap::DEFAULT).expect("retry");
    assert!(backward.downstream.is_empty());
}

#[test]
fn resume_and_dependency_stay_labeled_and_are_not_traversed() {
    let edges = [
        edge("a", "b", "dependency", "evidenced"),
        edge("b", "c", "resume", "evidenced"),
        edge("a", "kept", "spawn", "evidenced"),
    ];
    let got = blast_radius(&edges, "a", HopCap::DEFAULT).expect("walk");
    assert_eq!(got.downstream, ["kept".to_string()].into());
    assert_eq!(got.evidence_by_relation.get("dependency"), Some(&1));
    assert_eq!(got.evidence_by_relation.get("resume"), Some(&1));
    assert_eq!(edges.len(), 3);
}

#[test]
fn cycles_are_reported_and_edges_stay_stored() {
    let edges = vec![
        edge("a", "b", "spawn", "evidenced"),
        edge("b", "a", "spawn", "evidenced"),
    ];
    let got = blast_radius(&edges, "a", HopCap::DEFAULT).expect("cycle");
    assert_eq!(edges.len(), 2);
    assert_eq!(got.evidence_by_status.get("evidenced"), Some(&2));
    assert!(got.cycles.contains(&vec!["a".to_string(), "b".to_string()]));
    assert!(got.downstream.contains("b"));
    assert_eq!(got.status, AnswerStatus::Computed);
}

#[test]
fn hop_cap_reports_partial_with_truncated_frontier() {
    let mut edges = Vec::new();
    for index in 0..10 {
        edges.push(edge(
            &format!("s{index}"),
            &format!("s{}", index + 1),
            "spawn",
            "evidenced",
        ));
    }
    let got = blast_radius(&edges, "s0", HopCap::Bounded(8)).expect("capped");
    assert_eq!(got.status, AnswerStatus::Partial);
    assert_eq!(got.downstream.len(), 8);
    assert!(got.downstream.contains("s8"));
    assert!(!got.downstream.contains("s9"));
    assert!(!got.downstream.contains("s10"));
    assert_eq!(got.truncated_frontier, ["s9".to_string()].into());
    assert_eq!(got.truncated_frontier_count, 1);
    assert_eq!(got.hop_histogram.get(&8), Some(&1));
    assert_eq!(got.non_claim, DEPENDENCY_EXPOSURE_NON_CLAIM);
    assert_eq!(DEFAULT_HOP_CAP, 8);
    let within = blast_radius(&edges[..3], "s0", HopCap::DEFAULT).expect("short");
    assert_eq!(within.status, AnswerStatus::Computed);
    assert_eq!(within.truncated_frontier_count, 0);
}

#[test]
fn unbounded_path_query_is_refused() {
    let edges = [edge("a", "b", "spawn", "evidenced")];
    let error = blast_radius(&edges, "a", HopCap::Unbounded).expect_err("unbounded");
    assert!(error.to_string().contains("unbounded path query refused"));
    let i24 = i24_blast_radius(&edges, &[], HopCap::Unbounded).expect_err("unbounded i24");
    assert!(i24.to_string().contains("unbounded path query refused"));
}

#[test]
fn i24_answer_carries_the_dependency_exposure_non_claim() {
    let edges = [
        edge("root", "child", "spawn", "evidenced"),
        edge("root", "skipped", "spawn", "absent"),
    ];
    let facts = [SecurityObservation {
        native_session: Some("root".to_string()),
        kind: "grant".to_string(),
        value: "revoked".to_string(),
    }];
    let got = i24_blast_radius(&edges, &facts, HopCap::DEFAULT).expect("i24");
    assert_eq!(got.status, AnswerStatus::Computed);
    assert_eq!(got.downstream, ["child".to_string()].into());
    assert_eq!(got.downstream_count, 1);
    assert_eq!(got.non_claim, DEPENDENCY_EXPOSURE_NON_CLAIM);
    assert!(!got.rank_statistic_used);
    assert_eq!(got.artifact_count, None);
    assert_eq!(got.evidence_by_status.get("absent"), Some(&1));

    let failed = [SecurityObservation {
        native_session: Some("root".to_string()),
        kind: "grant".to_string(),
        value: "failed".to_string(),
    }];
    assert_eq!(
        i24_blast_radius(&edges, &failed, HopCap::DEFAULT)
            .expect("failed")
            .downstream_count,
        1
    );
    let sandbox = [SecurityObservation {
        native_session: Some("root".to_string()),
        kind: "sandbox".to_string(),
        value: "unverified".to_string(),
    }];
    assert_eq!(
        i24_blast_radius(&edges, &sandbox, HopCap::DEFAULT)
            .expect("sandbox")
            .non_claim,
        DEPENDENCY_EXPOSURE_NON_CLAIM
    );
}

#[test]
fn i24_without_a_decision_is_not_assessed_and_still_states_the_non_claim() {
    let edges = [edge("root", "child", "spawn", "evidenced")];
    let facts = [SecurityObservation {
        native_session: Some("root".to_string()),
        kind: "permission_decision".to_string(),
        value: "allow".to_string(),
    }];
    let got = i24_blast_radius(&edges, &facts, HopCap::DEFAULT).expect("none");
    assert_eq!(got.status, AnswerStatus::NotAssessed);
    assert!(got.downstream.is_empty());
    assert_eq!(got.non_claim, DEPENDENCY_EXPOSURE_NON_CLAIM);
    assert!(!got.rank_statistic_used);
    assert_eq!(edges.len(), 1);
}

#[test]
fn i24_does_not_call_pagerank() {
    let source = include_str!("../src/lineage_graph_v1.rs");
    assert!(!source.to_ascii_lowercase().contains("pagerank"));
    assert!(!I24_FROZEN_SQL.to_ascii_lowercase().contains("pagerank"));
    assert!(!DUCKPGQ_GRAPH_SQL.to_ascii_lowercase().contains("pagerank"));
    let edges = [edge("root", "child", "spawn", "evidenced")];
    let facts = [SecurityObservation {
        native_session: Some("root".to_string()),
        kind: "grant".to_string(),
        value: "revoked".to_string(),
    }];
    let got = i24_blast_radius(&edges, &facts, HopCap::DEFAULT).expect("i24");
    assert!(!got.rank_statistic_used);
}

#[test]
fn duckpgq_grammar_is_set_before_lock_and_install_is_never_planned() {
    let statements = duckpgq_open_statements(
        "/private/extensions",
        "/private/extensions/duckpgq.duckdb_extension",
    )
    .expect("plan");
    let grammar = statements
        .iter()
        .position(|sql| sql == ACTIVE_GRAMMAR_SQL)
        .expect("grammar");
    let lock = statements
        .iter()
        .position(|sql| sql == LOCK_CONFIGURATION_SQL)
        .expect("lock");
    assert!(grammar < lock);
    assert_eq!(
        statements[grammar],
        "SET active_grammar_extensions = ['duckpgq']"
    );
    assert_no_network_install(&statements);
    assert!(statements.iter().any(|sql| sql.starts_with("LOAD ")));
}

#[test]
fn extension_absent_uses_the_session_lineage_fallback() {
    let dir = TempDir::new("absent");
    let conn = Connection::open_in_memory().expect("memory");
    let graph = LineageGraph::open(&conn, dir.path(), PIN).expect("open");
    assert_eq!(
        graph.backend(),
        &GraphBackend::SessionLineageFallback {
            reason: FallbackReason::BlobAbsent
        }
    );
    assert!(graph
        .statements()
        .iter()
        .any(|sql| sql == LOCK_CONFIGURATION_SQL));
    assert!(!graph
        .statements()
        .iter()
        .any(|sql| sql == ACTIVE_GRAMMAR_SQL));
    assert!(graph
        .statements()
        .iter()
        .all(|sql| !sql.to_ascii_uppercase().trim_start().starts_with("LOAD")));
    assert_no_network_install(graph.statements());
}

#[test]
fn extension_hash_mismatch_does_not_load() {
    let dir = TempDir::new("mismatch");
    fs::write(
        dir.path().join(DUCKPGQ_EXTENSION_FILE),
        b"not-the-pinned-blob",
    )
    .expect("blob");
    let conn = Connection::open_in_memory().expect("memory");
    let graph = LineageGraph::open(&conn, dir.path(), PIN).expect("open");
    assert_eq!(
        graph.backend(),
        &GraphBackend::SessionLineageFallback {
            reason: FallbackReason::HashMismatch
        }
    );
    assert!(graph
        .statements()
        .iter()
        .all(|sql| !sql.to_ascii_uppercase().trim_start().starts_with("LOAD")));
    assert_no_network_install(graph.statements());
}

#[test]
fn extension_off_fallback_matches_pure_reachability() {
    let edges = [
        edge("a", "b", "spawn", "evidenced"),
        edge("b", "c", "handoff", "evidenced"),
        edge("c", "a", "return", "evidenced"),
        edge("a", "skip", "spawn", "truncated"),
        edge("b", "d", "join", "evidenced"),
        edge("prior", "later", "retry", "conflict"),
    ];
    let pure = blast_radius(&edges, "a", HopCap::Bounded(8)).expect("pure");
    let dir = TempDir::new("fallback");
    fs::write(
        dir.path().join(DUCKPGQ_EXTENSION_FILE),
        b"refused-extension-bytes",
    )
    .expect("blob");
    let pin = file_sha256_hex(&dir.path().join(DUCKPGQ_EXTENSION_FILE)).expect("hash");
    let conn = Connection::open_in_memory().expect("memory");
    let mut graph = LineageGraph::open(&conn, dir.path(), &pin).expect("open");
    assert_eq!(
        graph.backend(),
        &GraphBackend::SessionLineageFallback {
            reason: FallbackReason::LoadRefused
        }
    );
    insert_edges(&conn, &edges);
    let stored = graph
        .evaluate(&conn, "a", HopCap::Bounded(8))
        .expect("stored");
    assert_eq!(stored.downstream, pure.downstream);
    assert_eq!(stored.cycles, pure.cycles);
    assert_eq!(stored.truncated_frontier, pure.truncated_frontier);
    assert_eq!(stored.hop_histogram, pure.hop_histogram);
    assert_eq!(stored.evidence_by_status, pure.evidence_by_status);
    assert_eq!(stored.non_claim, DEPENDENCY_EXPOSURE_NON_CLAIM);
    assert!(!stored.rank_statistic_used);
    assert!(stored.downstream.contains("b"));
    assert!(stored.downstream.contains("c"));
    assert!(!stored.downstream.contains("skip"));
    let remaining: i64 = conn
        .query_row("SELECT COUNT(*) FROM session_edges", [], |row| row.get(0))
        .expect("count");
    assert_eq!(remaining, i64::try_from(edges.len()).expect("len"));
    assert!(graph.statements().iter().any(|sql| sql == I24_FROZEN_SQL));
    assert_no_network_install(graph.statements());
}

#[test]
fn origin_cycle_fallback_agrees_with_bounded_sql_walk() {
    let edges = [
        edge("origin", "a", "spawn", "evidenced"),
        edge("a", "origin", "spawn", "evidenced"),
    ];
    let pure = blast_radius(&edges, "origin", HopCap::Bounded(8)).expect("pure");
    let dir = TempDir::new("origin-cycle");
    fs::write(
        dir.path().join(DUCKPGQ_EXTENSION_FILE),
        b"refused-extension-bytes",
    )
    .expect("blob");
    let pin = file_sha256_hex(&dir.path().join(DUCKPGQ_EXTENSION_FILE)).expect("hash");
    let conn = Connection::open_in_memory().expect("memory");
    let mut graph = LineageGraph::open(&conn, dir.path(), &pin).expect("open");
    assert_eq!(
        graph.backend(),
        &GraphBackend::SessionLineageFallback {
            reason: FallbackReason::LoadRefused
        }
    );
    insert_edges(&conn, &edges);
    let stored = graph
        .evaluate(&conn, "origin", HopCap::Bounded(8))
        .expect("stored");
    assert_eq!(stored.status, AnswerStatus::Computed);
    assert!(stored.cycle_detected);
    assert!(pure.cycle_detected);
    assert_eq!(stored.status, pure.status);
    assert_eq!(stored.cycles, pure.cycles);
    assert!(stored
        .cycles
        .contains(&vec!["a".to_string(), "origin".to_string()]));
    assert_eq!(stored.downstream, pure.downstream);
    assert!(stored.downstream.contains("a"));
    assert!(!stored.downstream.contains("origin"));
    assert_eq!(stored.hops.get("origin").copied(), Some(0));
    assert_eq!(stored.truncated_frontier, pure.truncated_frontier);
}
