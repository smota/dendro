//! Selected schema4 admission depends on retained schema3 custody, never a label.
use std::path::Path;

use super::{engine, open_ro, parse_sidecar, refuse_inconsistent_source, sha256_file};
use super::{MigrationError, Refusal, MIGRATION_TARGET_SCHEMA_VERSION};

/// Bounded metadata check for an already hash-verified, immutable selected
/// bundle. Full schema/preimage validation belongs to candidate admission.
pub fn validate_selected_schema4_identity(
    database: &Path,
    key_id: &str,
    source_hash: &str,
    key_hash: &str,
    projection: &str,
) -> Result<(), MigrationError> {
    let conn = open_ro(database)?;
    let valid: i64 = conn.query_row("SELECT COUNT(*) FROM dendro_meta m JOIN dendro_migration_operation o ON o.id = 1 JOIN dendro_rollback_preimage p ON p.source_db_sha256 = o.source_db_sha256 AND p.source_sidecar_sha256 = o.source_sidecar_sha256 WHERE m.schema_version = 4 AND m.key_id = ? AND o.key_id = m.key_id AND o.source_schema = 3 AND o.target_schema = 4 AND o.source_db_sha256 = ? AND o.source_sidecar_sha256 = ? AND o.projection_sha256 = ? AND o.status = 'validated_unpublished' AND p.source_schema = 3 AND p.note = 'preserved_source_bundle_not_rewritten'", duckdb::params![key_id, source_hash, key_hash, projection], |row| row.get(0))?;
    let counts: (i64,i64,i64) = conn.query_row("SELECT (SELECT COUNT(*) FROM dendro_meta),(SELECT COUNT(*) FROM dendro_migration_operation),(SELECT COUNT(*) FROM dendro_rollback_preimage)", [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)))?;
    if valid != 1 || counts != (1, 1, 1) {
        return Err(refused());
    }
    conn.verify_runtime()?;
    Ok(())
}

pub fn validate_selected_schema4(
    database: &Path,
    preserved_schema3: &Path,
    preserved_key: &Path,
) -> Result<String, MigrationError> {
    refuse_inconsistent_source(database)?;
    refuse_inconsistent_source(preserved_schema3)?;
    let source_hash = sha256_file(preserved_schema3)?;
    let key_hash = sha256_file(preserved_key)?;
    let inspection = engine::inspect_read_only(preserved_schema3)?;
    let key = parse_sidecar(&std::fs::read(preserved_key)?).map_err(|_| refused())?;
    let conn = open_ro(database)?;
    let rows: i64 = conn.query_row("SELECT COUNT(*) FROM dendro_meta", [], |row| row.get(0))?;
    let (version, key_id): (i32, String) =
        conn.query_row("SELECT schema_version,key_id FROM dendro_meta", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?;
    if rows != 1
        || version != MIGRATION_TARGET_SCHEMA_VERSION
        || key_id != key.key_id
        || key_id != inspection.key_id
        || !engine::selected_metadata_matches(&conn, &inspection, &source_hash)?
    {
        return Err(refused());
    }
    let sidecar_rows: i64 = conn.query_row("SELECT COUNT(*) FROM dendro_migration_operation WHERE id = 1 AND source_sidecar_sha256 = ?", duckdb::params![key_hash], |row| row.get(0))?;
    if sidecar_rows != 1 {
        return Err(refused());
    }
    engine::ensure_logical_readability(&conn)?;
    conn.verify_runtime()?;
    drop(conn);
    if source_hash != sha256_file(preserved_schema3)? || key_hash != sha256_file(preserved_key)? {
        return Err(refused());
    }
    refuse_inconsistent_source(database)?;
    Ok(key_id)
}

fn refused() -> MigrationError {
    MigrationError::Refused(Refusal {
        code: "selected_schema4_migration_custody_invalid",
        source_mutated: false,
        pointer_changed: false,
        stage_retained: true,
        detail: "schema4 metadata or retained schema3/key linkage differs".into(),
    })
}
