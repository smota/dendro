//! DuckDB 2.0.0 runtime probe and schema-4 shadow of the six shipped insights.
use anyhow::{anyhow, Result};
use duckdb::params;

use crate::insight_v1::contract::MeasureValue;
use crate::insight_v1::{evaluate, InsightStatus, Scope};
use crate::AnalyticsStore;
use std::path::Path;

const SHADOW_KEY: &str = "shadow-v200";
const SHADOW_HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryLockProbe {
    pub version: String,
    pub temp_directory: String,
    pub extension_directory: String,
    pub autoinstall_known_extensions: String,
    pub autoload_known_extensions: String,
    pub lock_configuration: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShadowInsight {
    pub id: String,
    pub status: String,
    pub measures: Vec<(String, String)>,
    pub evidence: Vec<String>,
}

/// Opens `database` through `library_privacy` and reports the locked settings.
#[cfg(any(windows, target_os = "linux"))]
pub fn probe_private_library(database: &Path) -> Result<LibraryLockProbe> {
    let runtime = crate::storage::PrivateLibraryRuntime::prepare(database)?;
    let conn = runtime.open(false)?;
    let version: String = conn.query_row("SELECT version()", [], |row| row.get(0))?;
    let probe = LibraryLockProbe {
        version,
        temp_directory: setting(&conn, "temp_directory")?,
        extension_directory: setting(&conn, "extension_directory")?,
        autoinstall_known_extensions: setting(&conn, "autoinstall_known_extensions")?,
        autoload_known_extensions: setting(&conn, "autoload_known_extensions")?,
        lock_configuration: setting(&conn, "lock_configuration")?,
    };
    runtime.verify(&conn)?;
    Ok(probe)
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn probe_private_library(_database: &Path) -> Result<LibraryLockProbe> {
    Err(anyhow!(
        "private library probe is admitted on native linux and windows"
    ))
}

#[cfg(any(windows, target_os = "linux"))]
fn setting(conn: &duckdb::Connection, key: &str) -> Result<String> {
    conn.query_row("SELECT current_setting(?)::VARCHAR", [key], |row| {
        row.get(0)
    })
    .map_err(|error| anyhow!(error))
}

/// Writes a schema-4 database whose fact rows the six reference insights can read.
/// The file is created with `storage_compatibility_version = v1.5.6`.
pub fn write_schema4_shadow_fixture(database: &Path) -> Result<()> {
    let store = AnalyticsStore::open_path_preserving_v156(database, SHADOW_KEY)?;
    let conn = store.connection();
    conn.execute(
        "INSERT INTO usage_facts (
            source_key, harness, native_session, message_id, ts, seq,
            input_uncached, output_tokens, cache_read, cache_write, reasoning,
            currency_total, generation, source_id, model
         ) VALUES ('u1', 'codex', 's1', NULL, TIMESTAMP '2026-10-15 00:00:00', 1,
            100, 40, 10, 5, 4, 0.02, 1, 'shadow-src', 'gpt-test')",
        [],
    )?;
    conn.execute(
        "INSERT INTO tool_facts (
            source_key, harness, native_session, ts, seq, tool_name, fingerprint,
            failed, structured_write, generation, source_id, category
         ) VALUES ('t1', 'codex', 's1', TIMESTAMP '2026-10-15 00:00:00', 1, 'read',
            NULL, FALSE, FALSE, 1, 'shadow-src', 'other')",
        [],
    )?;
    conn.execute(
        "INSERT INTO session_edges (
            source_key, source_id, harness, parent_session, child_session,
            relation, status, observed_at, evidence_kind, generation
         ) VALUES ('e1', 'shadow-src', 'codex', 'parent', 'child', 'spawn', 'evidenced',
            TIMESTAMP '2026-10-15 00:00:00', 'explicit', 1)",
        [],
    )?;
    conn.execute(
        "INSERT INTO process_identities (
            identity_key, root_key, pid, parent_pid, platform, start_time_unix_s,
            exe_name, harness, cwd_fingerprint, session_link, environment_kind,
            isolation_hint, first_seen, last_seen, ended_at, visibility_limits, attribution
         ) VALUES ('root-a', 'root-a', 100, NULL, 'linux', 1700000000, 'codex', 'codex',
            NULL, 's1', 'host', 'none',
            TIMESTAMP '2026-10-15 00:00:00', TIMESTAMP '2026-10-15 00:01:00',
            NULL, '', 'launched')",
        [],
    )?;
    for (seq, ts, cpu, rss, disk_read, disk_write) in [
        (1_i64, "2026-10-15 00:00:00", 10_i64, 1000_i64, 5_i64, 1_i64),
        (2, "2026-10-15 00:00:01", 30, 1500, 9, 4),
    ] {
        conn.execute(
            "INSERT INTO process_samples (
                watch_id, identity_key, seq, ts, cpu_time_us, rss_bytes, rss_unit,
                disk_read_bytes, disk_write_bytes, capture_method, visibility_limits
             ) VALUES ('w1', 'root-a', ?, CAST(? AS TIMESTAMP), ?, ?, 'bytes', ?, ?, 'procfs', '')",
            params![seq, ts, cpu, rss, disk_read, disk_write],
        )?;
    }
    conn.execute(
        "INSERT INTO source_files (
            source_id, harness, display_path, generation, byte_offset, prefix_sha256, native_session
         ) VALUES ('shadow-src', 'codex', 'shadow-src', 1, 0, ?, 's1')",
        params![SHADOW_HASH],
    )?;
    conn.execute(
        "INSERT INTO warnings (path, line_no, reason) VALUES ('shadow-src', 1, 'coverage-partial')",
        [],
    )?;
    conn.execute_batch(
        "CREATE TABLE dendro_migration_operation (
            id INTEGER PRIMARY KEY,
            source_schema INTEGER NOT NULL,
            target_schema INTEGER NOT NULL,
            source_db_sha256 VARCHAR NOT NULL,
            source_sidecar_sha256 VARCHAR NOT NULL,
            key_id VARCHAR NOT NULL,
            generation_fingerprint VARCHAR NOT NULL,
            projection_sha256 VARCHAR NOT NULL,
            operation_timestamp VARCHAR NOT NULL,
            status VARCHAR NOT NULL,
            source_db_identity VARCHAR,
            source_key_identity VARCHAR
        );
        CREATE TABLE dendro_rollback_preimage (
            source_db_sha256 VARCHAR NOT NULL,
            source_sidecar_sha256 VARCHAR NOT NULL,
            source_schema INTEGER NOT NULL,
            note VARCHAR NOT NULL
        );",
    )?;
    conn.execute(
        "INSERT INTO dendro_migration_operation (
            id, source_schema, target_schema, source_db_sha256, source_sidecar_sha256,
            key_id, generation_fingerprint, projection_sha256, operation_timestamp, status
         ) VALUES (1, 3, 4, ?, ?, ?, 'shadow-v200', ?, '2026-10-27T00:00:00Z', 'validated_unpublished')",
        params![SHADOW_HASH, SHADOW_HASH, SHADOW_KEY, SHADOW_HASH],
    )?;
    conn.execute(
        "INSERT INTO dendro_rollback_preimage (
            source_db_sha256, source_sidecar_sha256, source_schema, note
         ) VALUES (?, ?, 3, 'preserved_source_bundle_not_rewritten')",
        params![SHADOW_HASH, SHADOW_HASH],
    )?;
    conn.execute(
        "UPDATE dendro_meta SET schema_version = 4 WHERE schema_version = 3",
        [],
    )?;
    store.checkpoint()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut wal_name = database.as_os_str().to_owned();
        wal_name.push(".wal");
        let wal = std::path::PathBuf::from(wal_name);
        if wal.is_file() {
            std::fs::set_permissions(&wal, std::fs::Permissions::from_mode(0o600))?;
        }
    }
    Ok(())
}

/// Opens a schema-4 shadow database and runs I02, I04, I08, I09, I14, and I18.
pub fn evaluate_schema4_shadow(database: &Path) -> Result<Vec<ShadowInsight>> {
    let store = AnalyticsStore::open_read_only(database)?;
    let mut views = Vec::new();
    for result in evaluate(&store, &Scope::default()) {
        views.push(ShadowInsight {
            id: result.id.to_string(),
            status: status_token(&result.status),
            measures: result
                .measures
                .iter()
                .map(|measure| (measure.id.clone(), measure_text(&measure.value)))
                .collect(),
            evidence: result.evidence,
        });
    }
    Ok(views)
}

fn status_token(status: &InsightStatus) -> String {
    match status {
        InsightStatus::Computed => "computed".to_string(),
        InsightStatus::Partial { reason } => format!("partial:{reason}"),
        InsightStatus::Refused { reason } => format!("refused:{reason}"),
    }
}

fn measure_text(value: &Option<MeasureValue>) -> String {
    match value {
        None => "unknown".to_string(),
        Some(MeasureValue::Int(value)) => value.to_string(),
        Some(MeasureValue::Float(value)) => format!("{value:.12}"),
    }
}
