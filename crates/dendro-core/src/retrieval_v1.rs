//! `A07` retrieval extensions: redacted full text, exact trajectory distance,
//! and a local pseudonymized Parquet publisher.
//!
//! None of these paths attaches a remote catalog. Extension network install
//! stays off. The full-text index and the in-memory vector index are
//! derivatives. Dropping either leaves the fact rows in place. Exact
//! `array_cosine_distance` (or `list_cosine_distance` when that is the engine
//! function) is the trajectory oracle. Persistent on-disk HNSW is refused.
//!
//! # Local Parquet DuckLake 1.1 snapshot
//!
//! `publish_pseudonymized_partitions` writes one private directory. That
//! directory is the snapshot. There is no remote catalog and no `ATTACH` of a
//! DuckLake server. Layout:
//!
//! - `catalog-manifest.json` — snapshot manifest. `format` is
//!   `dendro_ducklake_local_v1`. `objects` is an array of
//!   `{sha256, bytes, name}`.
//! - `{sha256}.parquet` — one PLAIN Parquet file per object. The file name is
//!   the hex SHA-256 of its bytes. `bytes` in the manifest is the file length.
//! - A catalog path passed to `plan_ducklake_attach` must stay inside this
//!   same directory. That is the only DuckLake 1.1 attach this publisher admits.

use crate::privacy;
use anyhow::{anyhow, bail, Result};
use duckdb::{params, Connection, OptionalExt};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const CORPUS_TABLE: &str = "_dendro_redacted_diagnostics";

pub const CORPUS_SQL: &str = "\
CREATE TABLE IF NOT EXISTS _dendro_redacted_diagnostics (
    doc_id VARCHAR PRIMARY KEY NOT NULL,
    harness VARCHAR NOT NULL,
    diagnostic_class VARCHAR NOT NULL,
    capability_name VARCHAR NOT NULL,
    remediation_label VARCHAR NOT NULL
)";

/// DuckDB 2.0 incremental full-text index over redacted labels only.
pub const FTS_CREATE_SQL: &str = "\
PRAGMA create_fts_index('_dendro_redacted_diagnostics', 'doc_id', 'diagnostic_class', 'capability_name', 'remediation_label', incremental := 1)";

/// Shared with `migration_v5::apply_schema5`. Either module may create the
/// table first. The column set is the contract.
pub const TRAJECTORY_SQL: &str = "\
CREATE TABLE IF NOT EXISTS harness_trajectory (
    observation_id VARCHAR,
    session_key VARCHAR,
    subject_key VARCHAR,
    harness VARCHAR,
    turn_k BIGINT,
    embedding FLOAT[32],
    vec FLOAT[32]
)";

pub const CANONICAL_DISTANCE_SQL: &str =
    "SELECT array_cosine_distance(vec, target::FLOAT[32]) FROM harness_trajectory";

pub const COSINE_TOLERANCE: f64 = 1e-5;
pub const TRAJECTORY_DIM: usize = 32;
const BM25_K1: f64 = 1.2;
const BM25_B: f64 = 0.75;
const LABEL_MAX: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexMode {
    FtsIncremental,
    TokenBm25Fallback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DistanceSource {
    ArrayCosineDistance,
    ListCosineDistance,
    RustExact,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Nearest {
    pub session_key: String,
    pub distance: f64,
    pub source: DistanceSource,
    pub index_rejected: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CanonicalNeighbor {
    pub session_key: String,
    pub distance: f64,
    pub exact_wins: bool,
    pub within_tolerance: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedactedDiagnostic {
    pub doc_id: String,
    pub harness: String,
    pub diagnostic_class: String,
    pub capability_name: String,
    pub remediation_label: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit {
    pub doc_id: String,
    pub score: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublishContext {
    Operator,
    AnswerPath,
    DendroAdvise,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExportMeasure {
    pub cohort_label: String,
    pub harness: String,
    pub measure_name: String,
    pub measure_value: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedObject {
    pub sha256_hex: String,
    pub bytes: u64,
    pub file_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublishOp {
    CopyParquet { destination: PathBuf },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishReceipt {
    pub objects: Vec<PublishedObject>,
    pub directory_mode: u32,
    pub operations: Vec<PublishOp>,
}

pub struct InMemoryTrajectoryIndex {
    rows: Vec<(String, [f32; TRAJECTORY_DIM])>,
}

impl InMemoryTrajectoryIndex {
    pub fn rebuild(rows: impl IntoIterator<Item = (String, [f32; TRAJECTORY_DIM])>) -> Self {
        let mut rows: Vec<_> = rows.into_iter().collect();
        rows.sort_by(|left, right| left.0.cmp(&right.0));
        Self { rows }
    }

    pub fn nearest(&self, target: &[f32; TRAJECTORY_DIM]) -> Option<(String, f64)> {
        let mut best: Option<(String, f64)> = None;
        for (key, vector) in &self.rows {
            let distance = cosine_distance(vector, target);
            if !distance.is_finite() {
                continue;
            }
            let replace = match &best {
                None => true,
                Some((best_key, best_distance)) => {
                    distance < *best_distance - f64::EPSILON
                        || ((distance - best_distance).abs() <= f64::EPSILON && key < best_key)
                }
            };
            if replace {
                best = Some((key.clone(), distance));
            }
        }
        best
    }
}

pub fn refuse_persistent_hnsw(path: &Path) -> Result<()> {
    let _ = path;
    bail!(
        "persistent on-disk HNSW is refused; rebuild the in-memory index from harness_trajectory"
    );
}

pub fn cosine_distance(left: &[f32; TRAJECTORY_DIM], right: &[f32; TRAJECTORY_DIM]) -> f64 {
    let mut dot = 0.0_f64;
    let mut left_norm = 0.0_f64;
    let mut right_norm = 0.0_f64;
    for index in 0..TRAJECTORY_DIM {
        let x = f64::from(left[index]);
        let y = f64::from(right[index]);
        dot += x * y;
        left_norm += x * x;
        right_norm += y * y;
    }
    if left_norm == 0.0 || right_norm == 0.0 {
        return f64::NAN;
    }
    1.0 - (dot / (left_norm.sqrt() * right_norm.sqrt()))
}

pub fn distances_agree(exact: f64, other: f64) -> bool {
    exact.is_finite() && other.is_finite() && (exact - other).abs() <= COSINE_TOLERANCE
}

/// Exact distance wins when an approximate hit disagrees beyond the frozen tolerance.
pub fn canonical_neighbor(
    exact_key: &str,
    exact_distance: f64,
    approximate_key: &str,
    approximate_distance: f64,
) -> CanonicalNeighbor {
    let within_tolerance =
        distances_agree(exact_distance, approximate_distance) && exact_key == approximate_key;
    if within_tolerance {
        CanonicalNeighbor {
            session_key: exact_key.to_string(),
            distance: exact_distance,
            exact_wins: false,
            within_tolerance: true,
        }
    } else {
        CanonicalNeighbor {
            session_key: exact_key.to_string(),
            distance: exact_distance,
            exact_wins: true,
            within_tolerance: false,
        }
    }
}

pub fn ensure_corpus(conn: &Connection) -> Result<()> {
    disarm_extension_network(conn)?;
    conn.execute_batch(CORPUS_SQL)?;
    Ok(())
}

pub fn admit_redacted_field(field: &str, value: &str) -> Result<()> {
    if value.is_empty() {
        bail!("redacted corpus refused an empty label in {field}");
    }
    if value.contains('/') || value.contains('\\') {
        bail!("redacted corpus refused a path in {field}");
    }
    if contains_secret_marker(value) {
        bail!("redacted corpus refused a secret token in {field}");
    }
    let folded = value.to_ascii_lowercase();
    if folded.contains("prompt")
        || folded.contains("ignore previous")
        || folded.contains("you are")
        || folded.contains("<|")
        || value.contains("```")
    {
        bail!("redacted corpus refused prompt text in {field}");
    }
    if folded.contains("argv")
        || value.contains('|')
        || value.contains(';')
        || (value.chars().any(char::is_whitespace) && value.contains('-'))
    {
        bail!("redacted corpus refused argv text in {field}");
    }
    if value.chars().any(char::is_whitespace)
        || value.chars().any(|ch| ch.is_control())
        || value.len() > LABEL_MAX
    {
        bail!("redacted corpus refused free text in {field}");
    }
    if !value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | ':' | '-'))
    {
        bail!("redacted corpus refused free text in {field}");
    }
    Ok(())
}

fn contains_secret_marker(value: &str) -> bool {
    privacy::contains_secret(value)
        || [
            "sk-",
            "sk-ant-",
            "ghp_",
            "gho_",
            "github_pat_",
            "xai-",
            "AKIA",
            "Bearer ",
            "xoxb-",
            "xoxp-",
        ]
        .iter()
        .any(|marker| value.contains(marker))
}

pub fn insert_diagnostic(conn: &Connection, row: &RedactedDiagnostic) -> Result<()> {
    ensure_corpus(conn)?;
    admit_redacted_field("doc_id", &row.doc_id)?;
    admit_redacted_field("harness", &row.harness)?;
    admit_redacted_field("diagnostic_class", &row.diagnostic_class)?;
    admit_redacted_field("capability_name", &row.capability_name)?;
    admit_redacted_field("remediation_label", &row.remediation_label)?;
    conn.execute(
        "INSERT INTO _dendro_redacted_diagnostics
         (doc_id, harness, diagnostic_class, capability_name, remediation_label)
         VALUES (?, ?, ?, ?, ?)",
        params![
            row.doc_id,
            row.harness,
            row.diagnostic_class,
            row.capability_name,
            row.remediation_label
        ],
    )?;
    Ok(())
}

pub fn list_diagnostics(conn: &Connection) -> Result<Vec<RedactedDiagnostic>> {
    ensure_corpus(conn)?;
    let mut stmt = conn.prepare(
        "SELECT doc_id, harness, diagnostic_class, capability_name, remediation_label
         FROM _dendro_redacted_diagnostics
         ORDER BY doc_id",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(RedactedDiagnostic {
            doc_id: row.get(0)?,
            harness: row.get(1)?,
            diagnostic_class: row.get(2)?,
            capability_name: row.get(3)?,
            remediation_label: row.get(4)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

pub fn corpus_count(conn: &Connection) -> Result<i64> {
    ensure_corpus(conn)?;
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM _dendro_redacted_diagnostics",
        [],
        |row| row.get(0),
    )?)
}

pub fn create_derivative_index(conn: &Connection) -> Result<IndexMode> {
    ensure_corpus(conn)?;
    if fts_index(conn).is_ok() {
        return Ok(IndexMode::FtsIncremental);
    }
    rebuild_token_index(conn)?;
    Ok(IndexMode::TokenBm25Fallback)
}

pub fn drop_derivative_index(conn: &Connection) -> Result<()> {
    disarm_extension_network(conn)?;
    let _ = conn.execute_batch("PRAGMA drop_fts_index('_dendro_redacted_diagnostics')");
    conn.execute_batch("DROP TABLE IF EXISTS _dendro_diagnostic_token_index")?;
    Ok(())
}

pub fn search_diagnostics(conn: &Connection, query: &str) -> Result<Vec<SearchHit>> {
    ensure_corpus(conn)?;
    if let Ok(Some(hits)) = fts_search(conn, query) {
        return Ok(hits);
    }
    token_bm25(conn, query)
}

pub fn ensure_trajectory(conn: &Connection) -> Result<()> {
    disarm_extension_network(conn)?;
    conn.execute_batch(TRAJECTORY_SQL)?;
    Ok(())
}

pub fn insert_trajectory(
    conn: &Connection,
    session_key: &str,
    harness: &str,
    vector: &[f32; TRAJECTORY_DIM],
) -> Result<()> {
    ensure_trajectory(conn)?;
    admit_redacted_field("session_key", session_key)?;
    admit_redacted_field("harness", harness)?;
    let turn_k: i64 = conn.query_row(
        "SELECT CAST(COALESCE(MAX(turn_k), 0) + 1 AS BIGINT)
         FROM harness_trajectory
         WHERE session_key = ? OR subject_key = ?",
        params![session_key, session_key],
        |row| row.get(0),
    )?;
    let observation_id = format!("{session_key}:{turn_k}");
    let literal = float32_array_literal(vector)?;
    let sql = format!(
        "INSERT INTO harness_trajectory (
            observation_id, session_key, subject_key, harness, turn_k, embedding, vec
         ) VALUES (?, ?, ?, ?, ?, {literal}, {literal})"
    );
    conn.execute(
        &sql,
        params![observation_id, session_key, session_key, harness, turn_k],
    )?;
    Ok(())
}

pub fn trajectory_count(conn: &Connection) -> Result<i64> {
    ensure_trajectory(conn)?;
    Ok(
        conn.query_row("SELECT COUNT(*) FROM harness_trajectory", [], |row| {
            row.get(0)
        })?,
    )
}

pub fn distance_source(conn: &Connection) -> DistanceSource {
    if probe_distance(
        conn,
        "array_cosine_distance([1.0::FLOAT]::FLOAT[1], [1.0::FLOAT]::FLOAT[1])",
    ) {
        DistanceSource::ArrayCosineDistance
    } else if probe_distance(conn, "list_cosine_distance([1.0::FLOAT], [1.0::FLOAT])") {
        DistanceSource::ListCosineDistance
    } else {
        DistanceSource::RustExact
    }
}

pub fn canonical_nearest(
    conn: &Connection,
    rows: &[(String, [f32; TRAJECTORY_DIM])],
    target: &[f32; TRAJECTORY_DIM],
) -> Result<Nearest> {
    disarm_extension_network(conn)?;
    let source = distance_source(conn);
    let scan = deduped_scan_rows(conn, rows)?;
    let mut best: Option<(String, f64)> = None;
    for candidate in &scan {
        let rust_distance = cosine_distance(&candidate.vector, target);
        let distance = match source {
            DistanceSource::RustExact => rust_distance,
            DistanceSource::ArrayCosineDistance | DistanceSource::ListCosineDistance => {
                let sql_distance = sql_distance(conn, &candidate.sql_key, target, source)?;
                if !distances_agree(sql_distance, rust_distance) {
                    bail!("trajectory distance disagreement between the sql oracle and rust");
                }
                sql_distance
            }
        };
        if !distance.is_finite() {
            continue;
        }
        let replace = match &best {
            None => true,
            Some((best_key, best_distance)) => {
                distance < *best_distance
                    || ((distance - best_distance).abs() <= COSINE_TOLERANCE
                        && &candidate.identity < best_key)
            }
        };
        if replace {
            best = Some((candidate.identity.clone(), distance));
        }
    }
    let Some((session_key, distance)) = best else {
        bail!("trajectory nearest neighbor is undefined");
    };
    let index = InMemoryTrajectoryIndex::rebuild(
        scan.iter()
            .map(|candidate| (candidate.identity.clone(), candidate.vector)),
    );
    let mut index_rejected = false;
    if let Some((indexed_key, indexed_distance)) = index.nearest(target) {
        if !distances_agree(distance, indexed_distance) || indexed_key != session_key {
            index_rejected = true;
        }
    }
    Ok(Nearest {
        session_key,
        distance,
        source,
        index_rejected,
    })
}

pub fn pseudonymize_identifier(key: &[u8], raw: &str) -> Result<String> {
    if key.is_empty() || raw.is_empty() {
        bail!("pseudonym input is empty");
    }
    let sealed = privacy::hmac_domain(key, "ducklake-v1", raw);
    if sealed.contains(raw) {
        bail!("pseudonym retained the raw identifier");
    }
    Ok(sealed)
}

pub fn plan_ducklake_attach(export_dir: &Path, catalog: &Path) -> Result<PathBuf> {
    let export_dir = std::path::absolute(export_dir)?;
    let catalog = std::path::absolute(catalog)?;
    if catalog == export_dir || !catalog.starts_with(&export_dir) {
        bail!("DuckLake catalog must stay inside the private export directory");
    }
    Ok(catalog)
}

pub fn publish_pseudonymized_partitions(
    export_dir: &Path,
    context: PublishContext,
    rows: &[ExportMeasure],
    forbidden_raw: &[&str],
    privacy_key: &[u8],
) -> Result<PublishReceipt> {
    match context {
        PublishContext::AnswerPath => bail!("DuckLake publish refused from an answer path"),
        PublishContext::DendroAdvise => bail!("DuckLake publish refused from dendro advise"),
        PublishContext::Operator => {}
    }
    if privacy_key.is_empty() {
        bail!("privacy key is empty");
    }
    let mut forbidden: Vec<Vec<u8>> = forbidden_raw
        .iter()
        .filter(|value| !value.is_empty())
        .map(|value| value.as_bytes().to_vec())
        .collect();
    forbidden.push(privacy_key.to_vec());
    for row in rows {
        admit_redacted_field("cohort_label", &row.cohort_label)?;
        admit_redacted_field("harness", &row.harness)?;
        admit_redacted_field("measure_name", &row.measure_name)?;
        if !row.measure_value.is_finite() {
            bail!("export measure is not finite");
        }
        for needle in &forbidden {
            let label_hit = [&row.cohort_label, &row.harness, &row.measure_name]
                .into_iter()
                .any(|label| needle_in(label.as_bytes(), needle));
            if label_hit {
                bail!("export row contains a raw identifier or prompt");
            }
        }
    }
    prepare_export_dir(export_dir)?;
    let mut operations = Vec::new();
    let mut created = Vec::new();
    if !rows.is_empty() {
        let object = write_parquet_partition(export_dir, rows, &mut operations)?;
        created.push(object);
    }
    write_manifest(export_dir)?;
    if let Err(error) = scan_export(export_dir, &forbidden) {
        for object in &created {
            let _ = fs::remove_file(export_dir.join(&object.file_name));
        }
        return Err(error);
    }
    let objects = list_parquet_objects(export_dir)?;
    Ok(PublishReceipt {
        objects,
        directory_mode: directory_mode(export_dir)?,
        operations,
    })
}

fn fts_index(conn: &Connection) -> Result<()> {
    load_extension(conn, "fts", None)?;
    conn.execute_batch(FTS_CREATE_SQL)?;
    Ok(())
}

/// `LOAD` of a filesystem blob requires a 64-hex SHA-256 pin and a matching
/// digest. An empty pin, a short pin, or a digest mismatch is refused before
/// `LOAD`. A built-in name such as `fts` is not a path and is not hashed.
fn load_extension(conn: &Connection, extension: &str, sha256_hex: Option<&str>) -> Result<()> {
    let is_path = extension.contains('/')
        || extension.contains('\\')
        || extension.ends_with(".duckdb_extension");
    if is_path {
        let Some(pin) = sha256_hex
            .map(str::trim)
            .filter(|pin| pin.len() == 64 && pin.chars().all(|ch| ch.is_ascii_hexdigit()))
        else {
            bail!("refusing unpinned extension blob");
        };
        let bytes =
            fs::read(Path::new(extension)).map_err(|_| anyhow!("extension blob is missing"))?;
        let actual = hex::encode(Sha256::digest(&bytes));
        if !actual.eq_ignore_ascii_case(pin) {
            bail!("extension digest does not match the pinned sha256");
        }
        if extension
            .chars()
            .any(|ch| ch == '\0' || ch == '\n' || ch == '\r' || ch == '\'')
        {
            bail!("extension path is not a safe sql literal");
        }
        conn.execute_batch(&format!("LOAD '{extension}'"))?;
        return Ok(());
    }
    if extension.is_empty()
        || !extension
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        bail!("extension name is not a safe identifier");
    }
    conn.execute_batch(&format!("LOAD {extension}"))?;
    Ok(())
}

fn fts_search(conn: &Connection, query: &str) -> Result<Option<Vec<SearchHit>>> {
    let Some(schema) = fts_schema(conn)? else {
        return Ok(None);
    };
    if !schema
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        bail!("fts schema name is not a safe identifier");
    }
    let sql = format!(
        "SELECT doc_id, {schema}.match_bm25(doc_id, ?) AS score
         FROM _dendro_redacted_diagnostics
         WHERE score IS NOT NULL
         ORDER BY score DESC, doc_id ASC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![query], |row| {
        Ok(SearchHit {
            doc_id: row.get(0)?,
            score: row.get(1)?,
        })
    })?;
    let hits = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(anyhow::Error::from)?;
    Ok(Some(hits))
}

fn fts_schema(conn: &Connection) -> Result<Option<String>> {
    let mut stmt = conn.prepare(
        "SELECT table_schema || '.' || table_name
         FROM information_schema.tables
         WHERE table_name LIKE 'fts_main%'
         ORDER BY table_name",
    )?;
    let mut rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    Ok(rows.next().transpose()?)
}

fn rebuild_token_index(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "DROP TABLE IF EXISTS _dendro_diagnostic_token_index;
         CREATE TABLE _dendro_diagnostic_token_index (
            doc_id VARCHAR NOT NULL,
            token VARCHAR NOT NULL,
            tf INTEGER NOT NULL
         )",
    )?;
    for row in list_diagnostics(conn)? {
        let mut counts: BTreeMap<String, i64> = BTreeMap::new();
        for token in tokenize(&document_text(&row)) {
            *counts.entry(token).or_default() += 1;
        }
        for (token, tf) in counts {
            conn.execute(
                "INSERT INTO _dendro_diagnostic_token_index VALUES (?, ?, ?)",
                params![row.doc_id, token, tf],
            )?;
        }
    }
    Ok(())
}

fn token_bm25(conn: &Connection, query: &str) -> Result<Vec<SearchHit>> {
    let docs = list_diagnostics(conn)?;
    let query_tokens = tokenize(query);
    if query_tokens.is_empty() || docs.is_empty() {
        return Ok(Vec::new());
    }
    let tokenized: Vec<(String, Vec<String>)> = docs
        .iter()
        .map(|row| (row.doc_id.clone(), tokenize(&document_text(row))))
        .collect();
    let average = tokenized
        .iter()
        .map(|(_, tokens)| tokens.len() as f64)
        .sum::<f64>()
        / tokenized.len() as f64;
    if average == 0.0 {
        return Ok(Vec::new());
    }
    let mut hits = Vec::new();
    let total = tokenized.len() as f64;
    for (doc_id, tokens) in &tokenized {
        let mut score = 0.0;
        for query_token in &query_tokens {
            let tf = tokens.iter().filter(|token| token == &query_token).count() as f64;
            if tf == 0.0 {
                continue;
            }
            let docs_with = tokenized
                .iter()
                .filter(|(_, doc_tokens)| doc_tokens.iter().any(|token| token == query_token))
                .count() as f64;
            let idf = ((total - docs_with + 0.5) / (docs_with + 0.5) + 1.0).ln();
            let denom = tf + BM25_K1 * (1.0 - BM25_B + BM25_B * tokens.len() as f64 / average);
            score += idf * (tf * (BM25_K1 + 1.0)) / denom;
        }
        if score > 0.0 {
            hits.push(SearchHit {
                doc_id: doc_id.clone(),
                score,
            });
        }
    }
    hits.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.doc_id.cmp(&right.doc_id))
    });
    Ok(hits)
}

fn document_text(row: &RedactedDiagnostic) -> String {
    format!(
        "{} {} {}",
        row.diagnostic_class, row.capability_name, row.remediation_label
    )
}

fn tokenize(value: &str) -> Vec<String> {
    value
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(|token| token.to_ascii_lowercase())
        .collect()
}

fn probe_distance(conn: &Connection, expression: &str) -> bool {
    conn.query_row(&format!("SELECT {expression}"), [], |row| {
        row.get::<_, f64>(0)
    })
    .is_ok()
}

const TRAJECTORY_TURN_MATCH_SQL: &str = "\
WHERE observation_id = ?1 OR session_key = ?1
ORDER BY CASE WHEN observation_id = ?1 THEN 0 ELSE 1 END, COALESCE(turn_k, 0) DESC, rowid DESC
LIMIT 1";

struct MatchedTurn {
    observation_id: String,
    session_key: String,
    turn_k: i64,
    rowid: i64,
    vector: [f32; TRAJECTORY_DIM],
}

struct ScanCandidate {
    identity: String,
    sql_key: String,
    vector: [f32; TRAJECTORY_DIM],
}

fn matched_turn(conn: &Connection, key: &str) -> Result<Option<MatchedTurn>> {
    let sql = format!(
        "SELECT COALESCE(observation_id, ''),
                COALESCE(session_key, ''),
                CAST(COALESCE(turn_k, 0) AS BIGINT),
                CAST(rowid AS BIGINT),
                vec
         FROM harness_trajectory
         {TRAJECTORY_TURN_MATCH_SQL}"
    );
    let parsed = conn
        .query_row(&sql, params![key], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, duckdb::types::Value>(4)?,
            ))
        })
        .optional()?;
    let Some((observation_id, session_key, turn_k, rowid, value)) = parsed else {
        return Ok(None);
    };
    Ok(Some(MatchedTurn {
        observation_id,
        session_key,
        turn_k,
        rowid,
        vector: vector_from_value(value)?,
    }))
}

fn vector_from_value(value: duckdb::types::Value) -> Result<[f32; TRAJECTORY_DIM]> {
    match value {
        duckdb::types::Value::Array(values) | duckdb::types::Value::List(values) => {
            vector_from_components(values)
        }
        duckdb::types::Value::Text(text) => vector_from_text(&text),
        _ => bail!("trajectory vec is not an array"),
    }
}

fn vector_from_components(values: Vec<duckdb::types::Value>) -> Result<[f32; TRAJECTORY_DIM]> {
    if values.len() != TRAJECTORY_DIM {
        bail!("trajectory vec has the wrong dimension");
    }
    let mut vector = [0.0; TRAJECTORY_DIM];
    for (index, value) in values.into_iter().enumerate() {
        vector[index] = match value {
            duckdb::types::Value::Float(value) => value,
            duckdb::types::Value::Double(value) => value as f32,
            _ => bail!("trajectory component is not a float"),
        };
    }
    Ok(vector)
}

fn vector_from_text(text: &str) -> Result<[f32; TRAJECTORY_DIM]> {
    let trimmed = text.trim().trim_matches(['[', ']']);
    let mut vector = [0.0; TRAJECTORY_DIM];
    let mut count = 0;
    for part in trimmed.split(',') {
        if count >= TRAJECTORY_DIM {
            bail!("trajectory vec has the wrong dimension");
        }
        let value = part
            .trim()
            .parse::<f32>()
            .map_err(|_| anyhow!("trajectory component is not a float"))?;
        if !value.is_finite() {
            bail!("trajectory component must be finite");
        }
        vector[count] = value;
        count += 1;
    }
    if count != TRAJECTORY_DIM {
        bail!("trajectory vec has the wrong dimension");
    }
    Ok(vector)
}

fn deduped_scan_rows(
    conn: &Connection,
    rows: &[(String, [f32; TRAJECTORY_DIM])],
) -> Result<Vec<ScanCandidate>> {
    let mut best: BTreeMap<String, (i64, i64, String, [f32; TRAJECTORY_DIM])> = BTreeMap::new();
    let mut unmatched = Vec::new();
    for (key, vector) in rows {
        let Some(turn) = matched_turn(conn, key)? else {
            unmatched.push((key.clone(), *vector));
            continue;
        };
        let identity = if !turn.session_key.is_empty() {
            turn.session_key.clone()
        } else if !turn.observation_id.is_empty() {
            turn.observation_id.clone()
        } else {
            key.clone()
        };
        let sql_key = if !turn.observation_id.is_empty() {
            turn.observation_id
        } else if !turn.session_key.is_empty() {
            turn.session_key
        } else {
            key.clone()
        };
        let rank = (turn.turn_k, turn.rowid);
        let replace = match best.get(&identity) {
            None => true,
            Some((turn_k, rowid, _, _)) => rank > (*turn_k, *rowid),
        };
        if replace {
            best.insert(identity, (turn.turn_k, turn.rowid, sql_key, turn.vector));
        }
    }
    let mut scan: Vec<ScanCandidate> = best
        .into_iter()
        .map(|(identity, (_, _, sql_key, vector))| ScanCandidate {
            identity,
            sql_key,
            vector,
        })
        .collect();
    for (key, vector) in unmatched {
        if scan.iter().any(|candidate| candidate.identity == key) {
            continue;
        }
        scan.push(ScanCandidate {
            identity: key.clone(),
            sql_key: key,
            vector,
        });
    }
    Ok(scan)
}

fn sql_distance(
    conn: &Connection,
    key: &str,
    target: &[f32; TRAJECTORY_DIM],
    source: DistanceSource,
) -> Result<f64> {
    let function = match source {
        DistanceSource::ArrayCosineDistance => "array_cosine_distance",
        DistanceSource::ListCosineDistance => "list_cosine_distance",
        DistanceSource::RustExact => bail!("sql distance requested without an engine function"),
    };
    let literal = float32_array_literal(target)?;
    let sql = format!(
        "SELECT {function}(vec, {literal}) FROM harness_trajectory {TRAJECTORY_TURN_MATCH_SQL}"
    );
    Ok(conn.query_row(&sql, params![key], |row| row.get(0))?)
}

fn float32_array_literal(values: &[f32; TRAJECTORY_DIM]) -> Result<String> {
    let mut parts = Vec::with_capacity(TRAJECTORY_DIM);
    for value in values {
        if !value.is_finite() {
            bail!("trajectory component must be finite");
        }
        let rendered = format!("{value:.8}");
        if !rendered
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'.' | b'-' | b'+' | b'e' | b'E'))
        {
            bail!("trajectory literal refused");
        }
        parts.push(rendered);
    }
    Ok(format!("[{}]::FLOAT[32]", parts.join(", ")))
}

fn disarm_extension_network(conn: &Connection) -> Result<()> {
    let locked = setting(conn, "lock_configuration")?;
    let autoinstall = setting(conn, "autoinstall_known_extensions")?;
    let autoload = setting(conn, "autoload_known_extensions")?;
    if locked == "true" {
        if autoinstall != "false" || autoload != "false" {
            bail!("retrieval refuses a locked configuration that can fetch extensions");
        }
        return Ok(());
    }
    conn.execute_batch(
        "SET autoinstall_known_extensions=false; SET autoload_known_extensions=false;",
    )?;
    Ok(())
}

fn setting(conn: &Connection, key: &str) -> Result<String> {
    Ok(
        conn.query_row("SELECT current_setting(?)::VARCHAR", params![key], |row| {
            row.get(0)
        })?,
    )
}

fn prepare_export_dir(path: &Path) -> Result<()> {
    if let Ok(meta) = fs::symlink_metadata(path) {
        if meta.file_type().is_symlink() {
            bail!("export directory refuses a symlink");
        }
        if !meta.is_dir() {
            bail!("export path is not a directory");
        }
    } else {
        fs::create_dir(path)?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
        let mode = fs::metadata(path)?.permissions().mode() & 0o777;
        if mode != 0o700 {
            bail!("export directory mode {mode:o} is not 0700");
        }
    }
    Ok(())
}

fn directory_mode(path: &Path) -> Result<u32> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        Ok(fs::metadata(path)?.permissions().mode() & 0o777)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(0)
    }
}

fn write_parquet_partition(
    export_dir: &Path,
    rows: &[ExportMeasure],
    operations: &mut Vec<PublishOp>,
) -> Result<PublishedObject> {
    let bytes = plain_parquet(rows)?;
    let sha256_hex = hex::encode(Sha256::digest(&bytes));
    let file_name = format!("{sha256_hex}.parquet");
    let final_path = export_dir.join(&file_name);
    if final_path.exists() {
        let existing = fs::read(&final_path)?;
        if existing != bytes {
            bail!("content-addressed partition {sha256_hex} already exists with different bytes");
        }
    } else {
        let temp_path = export_dir.join(format!(
            "writing-{}-{}.parquet",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or(0)
        ));
        let mut temp = TempFile {
            path: temp_path.clone(),
            armed: true,
        };
        {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp_path)?;
            file.write_all(&bytes)?;
        }
        fs::rename(&temp_path, &final_path)?;
        temp.armed = false;
        set_private_file(&final_path)?;
    }
    operations.push(PublishOp::CopyParquet {
        destination: final_path,
    });
    Ok(PublishedObject {
        sha256_hex,
        bytes: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        file_name,
    })
}

/// Uncompressed PLAIN Parquet, one row group, required UTF8 and DOUBLE columns.
/// Bytes are deterministic for a given row set. No extension download is required.
fn plain_parquet(rows: &[ExportMeasure]) -> Result<Vec<u8>> {
    let mut rows = rows.to_vec();
    rows.sort_by(|left, right| {
        (
            &left.cohort_label,
            &left.harness,
            &left.measure_name,
            left.measure_value.to_bits(),
        )
            .cmp(&(
                &right.cohort_label,
                &right.harness,
                &right.measure_name,
                right.measure_value.to_bits(),
            ))
    });
    let columns: [(&str, ColumnData); 4] = [
        (
            "cohort_label",
            ColumnData::Text(rows.iter().map(|row| row.cohort_label.clone()).collect()),
        ),
        (
            "harness",
            ColumnData::Text(rows.iter().map(|row| row.harness.clone()).collect()),
        ),
        (
            "measure_name",
            ColumnData::Text(rows.iter().map(|row| row.measure_name.clone()).collect()),
        ),
        (
            "measure_value",
            ColumnData::Double(rows.iter().map(|row| row.measure_value).collect()),
        ),
    ];
    let mut pages = Vec::with_capacity(columns.len());
    for (_, data) in &columns {
        pages.push(data_page(data)?);
    }
    let mut body = Vec::new();
    body.extend_from_slice(b"PAR1");
    let mut offsets = Vec::with_capacity(pages.len());
    for page in &pages {
        offsets.push(i64::try_from(body.len()).map_err(|_| anyhow!("parquet offset overflow"))?);
        body.extend_from_slice(page);
    }
    let footer = file_metadata(&columns, &pages, &offsets)?;
    let footer_len = u32::try_from(footer.len()).map_err(|_| anyhow!("parquet footer overflow"))?;
    body.extend_from_slice(&footer);
    body.extend_from_slice(&footer_len.to_le_bytes());
    body.extend_from_slice(b"PAR1");
    Ok(body)
}

enum ColumnData {
    Text(Vec<String>),
    Double(Vec<f64>),
}

fn data_page(data: &ColumnData) -> Result<Vec<u8>> {
    let values = match data {
        ColumnData::Text(values) => {
            let mut plain = Vec::new();
            for value in values {
                let bytes = value.as_bytes();
                let len =
                    u32::try_from(bytes.len()).map_err(|_| anyhow!("parquet value overflow"))?;
                plain.extend_from_slice(&len.to_le_bytes());
                plain.extend_from_slice(bytes);
            }
            plain
        }
        ColumnData::Double(values) => {
            let mut plain = Vec::new();
            for value in values {
                plain.extend_from_slice(&value.to_le_bytes());
            }
            plain
        }
    };
    let count = i32::try_from(match data {
        ColumnData::Text(values) => values.len(),
        ColumnData::Double(values) => values.len(),
    })
    .map_err(|_| anyhow!("parquet row overflow"))?;
    let size = i32::try_from(values.len()).map_err(|_| anyhow!("parquet page overflow"))?;
    let mut header = Thrift::default();
    header.field(1, 5);
    header.i32(0);
    header.field(2, 5);
    header.i32(size);
    header.field(3, 5);
    header.i32(size);
    header.field(4, 12);
    header.begin_struct();
    header.field(1, 5);
    header.i32(count);
    header.field(2, 5);
    header.i32(0);
    header.field(3, 5);
    header.i32(3);
    header.field(4, 5);
    header.i32(3);
    header.stop();
    header.stop();
    let mut page = header.bytes;
    page.extend_from_slice(&values);
    Ok(page)
}

fn file_metadata(
    columns: &[(&str, ColumnData)],
    pages: &[Vec<u8>],
    offsets: &[i64],
) -> Result<Vec<u8>> {
    let rows = i64::try_from(match columns.first() {
        Some((_, ColumnData::Text(values))) => values.len(),
        Some((_, ColumnData::Double(values))) => values.len(),
        None => 0,
    })
    .map_err(|_| anyhow!("parquet row overflow"))?;
    let children = i32::try_from(columns.len()).map_err(|_| anyhow!("parquet schema overflow"))?;
    let mut meta = Thrift::default();
    meta.field(1, 5);
    meta.i32(1);
    meta.field(2, 9);
    meta.list(columns.len() + 1, 12);
    meta.begin_struct();
    write_root_schema(&mut meta, children);
    for (name, data) in columns {
        meta.begin_struct();
        write_column_schema(&mut meta, name, data);
    }
    meta.field(3, 6);
    meta.i64(rows);
    meta.field(4, 9);
    meta.list(1, 12);
    meta.begin_struct();
    write_row_group(&mut meta, columns, pages, offsets, rows)?;
    meta.field(6, 8);
    meta.binary(b"dendro-retrieval-v1");
    meta.stop();
    Ok(meta.bytes)
}

fn write_root_schema(meta: &mut Thrift, children: i32) {
    meta.field(4, 8);
    meta.binary(b"schema");
    meta.field(5, 5);
    meta.i32(children);
    meta.stop();
}

fn write_column_schema(meta: &mut Thrift, name: &str, data: &ColumnData) {
    let physical = match data {
        ColumnData::Text(_) => 6,
        ColumnData::Double(_) => 5,
    };
    meta.field(1, 5);
    meta.i32(physical);
    meta.field(3, 5);
    meta.i32(0);
    meta.field(4, 8);
    meta.binary(name.as_bytes());
    if matches!(data, ColumnData::Text(_)) {
        meta.field(6, 5);
        meta.i32(0);
    }
    meta.stop();
}

fn write_row_group(
    meta: &mut Thrift,
    columns: &[(&str, ColumnData)],
    pages: &[Vec<u8>],
    offsets: &[i64],
    rows: i64,
) -> Result<()> {
    meta.field(1, 9);
    meta.list(columns.len(), 12);
    let mut total = 0_i64;
    for (index, (name, data)) in columns.iter().enumerate() {
        let size =
            i64::try_from(pages[index].len()).map_err(|_| anyhow!("parquet page overflow"))?;
        total += size;
        meta.begin_struct();
        write_column_chunk(meta, name, data, offsets[index], size, rows);
    }
    meta.field(2, 6);
    meta.i64(total);
    meta.field(3, 6);
    meta.i64(rows);
    meta.stop();
    Ok(())
}

fn write_column_chunk(
    meta: &mut Thrift,
    name: &str,
    data: &ColumnData,
    offset: i64,
    size: i64,
    rows: i64,
) {
    let physical = match data {
        ColumnData::Text(_) => 6,
        ColumnData::Double(_) => 5,
    };
    meta.field(2, 6);
    meta.i64(offset);
    meta.field(3, 12);
    meta.begin_struct();
    meta.field(1, 5);
    meta.i32(physical);
    meta.field(2, 9);
    meta.list(1, 5);
    meta.i32(0);
    meta.field(3, 9);
    meta.list(1, 8);
    meta.binary(name.as_bytes());
    meta.field(4, 5);
    meta.i32(0);
    meta.field(5, 6);
    meta.i64(rows);
    meta.field(6, 6);
    meta.i64(size);
    meta.field(7, 6);
    meta.i64(size);
    meta.field(9, 6);
    meta.i64(offset);
    meta.stop();
    meta.stop();
}

#[derive(Default)]
struct Thrift {
    bytes: Vec<u8>,
    last_field: i16,
    stack: Vec<i16>,
}

impl Thrift {
    fn begin_struct(&mut self) {
        self.stack.push(self.last_field);
        self.last_field = 0;
    }

    fn field(&mut self, id: i16, kind: u8) {
        let delta = id - self.last_field;
        if (1..=15).contains(&delta) {
            self.bytes.push(((delta as u8) << 4) | kind);
        } else {
            self.bytes.push(kind);
            self.varint(zigzag_i64(i64::from(id)));
        }
        self.last_field = id;
    }

    fn stop(&mut self) {
        self.bytes.push(0);
        self.last_field = self.stack.pop().unwrap_or(0);
    }

    fn list(&mut self, len: usize, elem: u8) {
        if len <= 14 {
            self.bytes.push(((len as u8) << 4) | elem);
        } else {
            self.bytes.push(0xF0 | elem);
            self.varint(u64::try_from(len).unwrap_or(u64::MAX));
        }
    }

    fn i32(&mut self, value: i32) {
        self.varint(zigzag_i32(value));
    }

    fn i64(&mut self, value: i64) {
        self.varint(zigzag_i64(value));
    }

    fn binary(&mut self, value: &[u8]) {
        self.varint(u64::try_from(value.len()).unwrap_or(u64::MAX));
        self.bytes.extend_from_slice(value);
    }

    fn varint(&mut self, mut value: u64) {
        loop {
            let mut byte = (value & 0x7f) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            self.bytes.push(byte);
            if value == 0 {
                break;
            }
        }
    }
}

fn zigzag_i32(value: i32) -> u64 {
    u64::from(((value << 1) ^ (value >> 31)) as u32)
}

fn zigzag_i64(value: i64) -> u64 {
    ((value << 1) ^ (value >> 63)) as u64
}

/// Writes `catalog-manifest.json` for the local DuckLake 1.1 snapshot.
///
/// The manifest is the directory index, not a second copy of the Parquet
/// payload. Each `objects` entry is `{sha256, bytes, name}` and `name` is
/// `{sha256}.parquet` beside this file. `format` stays
/// `dendro_ducklake_local_v1`.
fn write_manifest(export_dir: &Path) -> Result<()> {
    let objects = list_parquet_objects(export_dir)?;
    let body = json!({
        "format": "dendro_ducklake_local_v1",
        "objects": objects.iter().map(|object| json!({
            "sha256": object.sha256_hex,
            "bytes": object.bytes,
            "name": object.file_name,
        })).collect::<Vec<_>>(),
    });
    let temp_path = export_dir.join("catalog-manifest.json.writing");
    {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)?;
        file.write_all(serde_json::to_vec_pretty(&body)?.as_slice())?;
    }
    let final_path = export_dir.join("catalog-manifest.json");
    if final_path.exists() {
        fs::remove_file(&final_path)?;
    }
    fs::rename(&temp_path, &final_path)?;
    set_private_file(&final_path)?;
    Ok(())
}

fn list_parquet_objects(export_dir: &Path) -> Result<Vec<PublishedObject>> {
    let mut objects = Vec::new();
    for entry in fs::read_dir(export_dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.ends_with(".parquet") || name.starts_with("writing-") {
            continue;
        }
        let bytes = fs::metadata(entry.path())?.len();
        let sha256_hex = name.trim_end_matches(".parquet").to_string();
        objects.push(PublishedObject {
            sha256_hex,
            bytes,
            file_name: name.into_owned(),
        });
    }
    objects.sort_by(|left, right| left.sha256_hex.cmp(&right.sha256_hex));
    Ok(objects)
}

fn scan_export(export_dir: &Path, forbidden: &[Vec<u8>]) -> Result<()> {
    for entry in fs::read_dir(export_dir)? {
        let entry = entry?;
        let meta = fs::symlink_metadata(entry.path())?;
        if meta.file_type().is_symlink() {
            bail!("export directory refuses a symlink");
        }
        if meta.is_dir() {
            bail!("export directory refuses a nested directory");
        }
        let name = entry.file_name();
        let name = name.to_string_lossy().to_ascii_lowercase();
        if name.contains("privacy-key")
            || name.contains("privacy_key")
            || name.contains("sidecar")
            || name.ends_with(".key")
        {
            bail!("export directory contains a privacy sidecar");
        }
        let bytes = fs::read(entry.path())?;
        for needle in forbidden {
            if needle_in(&bytes, needle) {
                bail!("export object contains a forbidden raw identifier, prompt, or privacy key");
            }
        }
    }
    Ok(())
}

fn needle_in(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

fn set_private_file(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

struct TempFile {
    path: PathBuf,
    armed: bool,
}

impl Drop for TempFile {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_file(&self.path);
        }
    }
}
