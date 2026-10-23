//! Read-only schema3 inspection and private schema4 staging.
//! Stored TIMESTAMP values stay timezone-free. Migration time is operation metadata only.

use std::collections::BTreeMap;
use std::path::Path;

use duckdb::{params, Connection};
use sha2::{Digest, Sha256};

use super::{quote_ident, MigrationError, MIGRATION_TARGET_SCHEMA_VERSION};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    pub name: String,
    pub data_type: String,
}

#[derive(Debug, Clone)]
pub struct SourceInspection {
    pub schema_version: i32,
    pub key_id: String,
    pub tables: Vec<String>,
    pub columns: BTreeMap<String, Vec<Column>>,
    pub counts: BTreeMap<String, i64>,
    pub projection_sha256: String,
    pub generation_fingerprint: String,
}

pub fn inspect_read_only(db: &Path) -> Result<SourceInspection, MigrationError> {
    let conn = super::open_ro(db)?;
    let meta_tables: i64 = conn.query_row(
        "SELECT COUNT(*) FROM information_schema.tables
         WHERE table_catalog = current_database() AND table_schema = 'main' AND table_name = 'dendro_meta'",
        [],
        |row| row.get(0),
    )?;
    if meta_tables != 1 {
        return Err(MigrationError::Refused(super::Refusal {
            code: "refuse_unsupported_schema",
            source_mutated: false,
            pointer_changed: false,
            stage_retained: false,
            detail: "dendro_meta absent".into(),
        }));
    }
    let meta_rows: i64 =
        conn.query_row("SELECT COUNT(*) FROM dendro_meta", [], |row| row.get(0))?;
    if meta_rows != 1 {
        return Err(MigrationError::Refused(super::Refusal {
            code: "refuse_unsupported_schema",
            source_mutated: false,
            pointer_changed: false,
            stage_retained: false,
            detail: format!("dendro_meta rows {meta_rows}"),
        }));
    }
    let (schema_version, key_id): (i32, String) = conn.query_row(
        "SELECT schema_version, key_id FROM dendro_meta",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if schema_version != 3 {
        return Err(MigrationError::Refused(super::Refusal {
            code: "refuse_unsupported_schema",
            source_mutated: false,
            pointer_changed: false,
            stage_retained: false,
            detail: format!("schema {schema_version}"),
        }));
    }
    let view_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM information_schema.tables
         WHERE table_catalog = current_database() AND table_schema = 'main' AND table_type = 'VIEW'",
        [],
        |row| row.get(0),
    )?;
    if view_count != 0 {
        return Err(MigrationError::Refused(super::Refusal {
            code: "source_views_unproved",
            source_mutated: false,
            pointer_changed: false,
            stage_retained: false,
            detail: format!("{view_count} views"),
        }));
    }
    let tables = list_base_tables(&conn)?;
    let mut columns = BTreeMap::new();
    let mut counts = BTreeMap::new();
    for table in &tables {
        columns.insert(table.clone(), list_columns(&conn, table)?);
        counts.insert(table.clone(), count_rows(&conn, table)?);
    }
    let projection_sha256 = projection_hash(&conn, &tables, &columns)?;
    let generation_fingerprint = generation_fingerprint(&conn, &tables)?;
    Ok(SourceInspection {
        schema_version,
        key_id,
        tables,
        columns,
        counts,
        projection_sha256,
        generation_fingerprint,
    })
}

pub fn list_base_tables(conn: &Connection) -> Result<Vec<String>, MigrationError> {
    let mut stmt = conn.prepare(
        "SELECT table_name FROM information_schema.tables
         WHERE table_catalog = current_database() AND table_schema = 'main' AND table_type = 'BASE TABLE'
         ORDER BY table_name",
    )?;
    let mut rows = stmt.query([])?;
    let mut tables = Vec::new();
    while let Some(row) = rows.next()? {
        let name: String = row.get(0)?;
        quote_ident(&name)?;
        tables.push(name);
    }
    Ok(tables)
}

pub fn list_columns(conn: &Connection, table: &str) -> Result<Vec<Column>, MigrationError> {
    quote_ident(table)?;
    let mut stmt = conn.prepare(
        "SELECT column_name, data_type FROM information_schema.columns
         WHERE table_catalog = current_database() AND table_schema = 'main' AND table_name = ?
         ORDER BY ordinal_position",
    )?;
    let mut rows = stmt.query(params![table])?;
    let mut columns = Vec::new();
    while let Some(row) = rows.next()? {
        let name: String = row.get(0)?;
        quote_ident(&name)?;
        columns.push(Column {
            name,
            data_type: row.get(1)?,
        });
    }
    if columns.is_empty() {
        return Err(MigrationError::Duck(format!("no columns for {table}")));
    }
    Ok(columns)
}

pub fn count_rows(conn: &Connection, table: &str) -> Result<i64, MigrationError> {
    let quoted = quote_ident(table)?;
    let count: i64 = conn.query_row(&format!("SELECT COUNT(*) FROM {quoted}"), [], |row| {
        row.get(0)
    })?;
    Ok(count)
}

pub fn projection_hash(
    conn: &Connection,
    tables: &[String],
    columns: &BTreeMap<String, Vec<Column>>,
) -> Result<String, MigrationError> {
    projection_hash_with_schema(conn, tables, columns, false)
}

/// Only the admitted 3 -> 4 label change is normalized. Every other metadata
/// column and row participates in the original legacy projection.
pub fn staged_projection_hash(
    conn: &Connection,
    tables: &[String],
    columns: &BTreeMap<String, Vec<Column>>,
) -> Result<String, MigrationError> {
    projection_hash_with_schema(conn, tables, columns, true)
}

fn projection_hash_with_schema(
    conn: &Connection,
    tables: &[String],
    columns: &BTreeMap<String, Vec<Column>>,
    staged: bool,
) -> Result<String, MigrationError> {
    let mut hasher = Sha256::new();
    for table in tables {
        hasher.update(table.as_bytes());
        hasher.update([0]);
        let cols = &columns[table];
        let select = canonical_select(table, cols, staged)?;
        let mut stmt = conn.prepare(&select)?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            for (idx, column) in cols.iter().enumerate() {
                hasher.update(column.name.as_bytes());
                hasher.update([0]);
                if is_float(&column.data_type) {
                    let value: Option<f64> = row.get(idx)?;
                    match value {
                        None => hasher.update([0]),
                        Some(number) => {
                            hasher.update([1]);
                            hasher.update(number.to_bits().to_be_bytes());
                        }
                    }
                } else {
                    let value: Option<String> = row.get(idx)?;
                    match value {
                        None => hasher.update([0]),
                        Some(text) => {
                            hasher.update([1]);
                            hasher.update((text.len() as u64).to_be_bytes());
                            hasher.update(text.as_bytes());
                        }
                    }
                }
            }
            hasher.update([0xff]);
        }
    }
    Ok(hex::encode(hasher.finalize()))
}

fn canonical_select(table: &str, cols: &[Column], staged: bool) -> Result<String, MigrationError> {
    let quoted_table = quote_ident(table)?;
    let mut exprs = Vec::with_capacity(cols.len());
    for column in cols {
        let quoted = quote_ident(&column.name)?;
        if staged && table == "dendro_meta" && column.name == "schema_version" {
            exprs.push(format!(
                "CAST(CASE WHEN {quoted} = 4 THEN 3 ELSE {quoted} END AS VARCHAR)"
            ));
        } else if is_float(&column.data_type) {
            exprs.push(quoted);
        } else {
            exprs.push(format!("CAST({quoted} AS VARCHAR)"));
        }
    }
    let order = (1..=cols.len())
        .map(|index| index.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    Ok(format!(
        "SELECT {} FROM {quoted_table} ORDER BY {order}",
        exprs.join(", ")
    ))
}

fn generation_fingerprint(conn: &Connection, tables: &[String]) -> Result<String, MigrationError> {
    if !tables.iter().any(|table| table == "source_files") {
        return Ok("source_files_absent".into());
    }
    let mut stmt = conn.prepare(
        "SELECT source_id, generation, byte_offset, prefix_sha256
         FROM source_files ORDER BY source_id",
    )?;
    let mut rows = stmt.query([])?;
    let mut hasher = Sha256::new();
    while let Some(row) = rows.next()? {
        let source_id: String = row.get(0)?;
        let generation: i64 = row.get(1)?;
        let byte_offset: i64 = row.get(2)?;
        let prefix: String = row.get(3)?;
        hasher.update(source_id.as_bytes());
        hasher.update(generation.to_be_bytes());
        hasher.update(byte_offset.to_be_bytes());
        hasher.update(prefix.as_bytes());
    }
    Ok(hex::encode(hasher.finalize()))
}

pub fn is_float(data_type: &str) -> bool {
    let upper = data_type.to_ascii_uppercase();
    matches!(
        upper.as_str(),
        "DOUBLE" | "FLOAT" | "REAL" | "FLOAT4" | "FLOAT8"
    )
}

const SCHEMA4_METADATA_DDL: &str = r#"
        CREATE TABLE dendro_migration_operation (
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
        CREATE TABLE dendro_schema4_precision (
            table_name VARCHAR NOT NULL,
            column_name VARCHAR NOT NULL,
            precision_tag VARCHAR NOT NULL,
            PRIMARY KEY (table_name, column_name)
        );
        CREATE TABLE dendro_schema4_field_state (
            field_name VARCHAR PRIMARY KEY,
            state VARCHAR NOT NULL
        );
        CREATE TABLE dendro_fact_revisions (
            fact_id VARCHAR NOT NULL,
            revision BIGINT NOT NULL,
            ingested BIGINT NOT NULL,
            quantity DOUBLE,
            retracted BOOLEAN NOT NULL,
            state VARCHAR NOT NULL,
            PRIMARY KEY (fact_id, revision)
        );
        CREATE TABLE dendro_spawn_edges (
            parent_id VARCHAR NOT NULL,
            child_id VARCHAR NOT NULL,
            PRIMARY KEY (parent_id, child_id)
        );
        CREATE TABLE dendro_v4_only_facts (
            fact_id VARCHAR PRIMARY KEY,
            ts VARCHAR NOT NULL,
            source_id VARCHAR NOT NULL,
            generation BIGINT NOT NULL,
            range_start BIGINT NOT NULL,
            range_end BIGINT NOT NULL
        );
        CREATE TABLE dendro_rollback_preimage (
            source_db_sha256 VARCHAR NOT NULL,
            source_sidecar_sha256 VARCHAR NOT NULL,
            source_schema INTEGER NOT NULL,
            note VARCHAR NOT NULL
        );
        "#;

fn install_legacy_annotations(
    conn: &Connection,
    inspection: &SourceInspection,
) -> Result<(), MigrationError> {
    for (table, columns) in &inspection.columns {
        for column in columns {
            if is_float(&column.data_type) {
                conn.execute(
                    "INSERT INTO dendro_schema4_precision (table_name, column_name, precision_tag)
                     VALUES (?, ?, 'legacy_binary64_approximate')",
                    params![table, column.name],
                )?;
            }
        }
        // Compatibility views keep the copied grain, including legacy nulls and binary64 values.
        let quoted = quote_ident(table)?;
        let view = quote_ident(&format!("legacy_{table}"))?;
        conn.execute_batch(&format!("CREATE VIEW {view} AS SELECT * FROM {quoted}"))?;
    }
    conn.execute_batch(
        "
        INSERT INTO dendro_schema4_field_state (field_name, state) VALUES
            ('invocation', 'unknown'),
            ('lineage', 'unknown'),
            ('acceptance', 'unknown'),
            ('context_capacity', 'unknown');
        ",
    )?;
    Ok(())
}

pub fn install_schema4_metadata(
    conn: &Connection,
    inspection: &SourceInspection,
    source_db_sha256: &str,
    source_sidecar_sha256: &str,
    operation_timestamp: &str,
    status: &str,
) -> Result<(), MigrationError> {
    let updated = conn.execute(
        "UPDATE dendro_meta SET schema_version = ? WHERE schema_version = 3",
        params![MIGRATION_TARGET_SCHEMA_VERSION],
    )?;
    if updated != 1 {
        return Err(MigrationError::Duck(format!(
            "schema version update changed {updated} rows"
        )));
    }
    conn.execute_batch(SCHEMA4_METADATA_DDL)?;
    install_legacy_annotations(conn, inspection)?;
    conn.execute(
        "INSERT INTO dendro_migration_operation (
            id, source_schema, target_schema, source_db_sha256, source_sidecar_sha256, key_id,
            generation_fingerprint, projection_sha256, operation_timestamp, status
         ) VALUES (1, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        params![
            inspection.schema_version,
            MIGRATION_TARGET_SCHEMA_VERSION,
            source_db_sha256,
            source_sidecar_sha256,
            inspection.key_id,
            inspection.generation_fingerprint,
            inspection.projection_sha256,
            operation_timestamp,
            status
        ],
    )?;
    conn.execute(
        "INSERT INTO dendro_rollback_preimage (
            source_db_sha256, source_sidecar_sha256, source_schema, note
         ) VALUES (?, ?, 3, 'preserved_source_bundle_not_rewritten')",
        params![source_db_sha256, source_sidecar_sha256],
    )?;
    Ok(())
}

/// Re-derive the admitted metadata from the source contract. This is independent
/// of the writable on-disk candidate seal. Unknown or additional v4 content
/// prevents a no-loss rollback claim and prevents pristine-stage reuse.
pub fn pristine_metadata_matches(
    conn: &Connection,
    inspection: &SourceInspection,
    source_db_sha256: &str,
) -> Result<bool, MigrationError> {
    let expected = Connection::open_in_memory()?;
    expected.execute_batch(SCHEMA4_METADATA_DDL)?;
    let metadata_tables = list_base_tables(&expected)?;
    let mut expected_tables = inspection.tables.clone();
    expected_tables.extend(metadata_tables.iter().cloned());
    expected_tables.sort_unstable();
    if list_base_tables(conn)? != expected_tables {
        return Ok(false);
    }
    for table in &metadata_tables {
        if list_columns(conn, table)? != list_columns(&expected, table)? {
            return Ok(false);
        }
    }
    // View SQL does not depend on the source table's columns. Empty stubs let
    // DuckDB itself canonicalize the expected SELECT * view definitions.
    for table in &inspection.tables {
        expected.execute_batch(&format!(
            "CREATE TABLE {} (stub INTEGER)",
            quote_ident(table)?
        ))?;
    }
    install_legacy_annotations(&expected, inspection)?;
    for table in ["dendro_schema4_field_state", "dendro_schema4_precision"] {
        let tables = vec![table.to_string()];
        let columns = BTreeMap::from([(table.to_string(), list_columns(&expected, table)?)]);
        if projection_hash(conn, &tables, &columns)?
            != projection_hash(&expected, &tables, &columns)?
        {
            return Ok(false);
        }
    }
    if view_definitions(conn)? != view_definitions(&expected)? {
        return Ok(false);
    }
    for table in [
        "dendro_fact_revisions",
        "dendro_spawn_edges",
        "dendro_v4_only_facts",
    ] {
        if count_rows(conn, table)? != 0 {
            return Ok(false);
        }
    }
    let operation_matches: i64 = conn.query_row(
        "SELECT COUNT(*) FROM dendro_migration_operation WHERE id = 1
         AND source_schema = 3 AND target_schema = 4 AND source_db_sha256 = ?
         AND key_id = ? AND generation_fingerprint = ? AND projection_sha256 = ?
         AND operation_timestamp <> '' AND status = 'validated_unpublished'",
        params![
            source_db_sha256,
            inspection.key_id,
            inspection.generation_fingerprint,
            inspection.projection_sha256
        ],
        |row| row.get(0),
    )?;
    let preimage_matches: i64 = conn.query_row(
        "SELECT COUNT(*) FROM dendro_rollback_preimage AS p
         JOIN dendro_migration_operation AS o ON o.id = 1
         WHERE p.source_db_sha256 = ? AND p.source_schema = 3
         AND p.source_sidecar_sha256 = o.source_sidecar_sha256
         AND p.note = 'preserved_source_bundle_not_rewritten'",
        params![source_db_sha256],
        |row| row.get(0),
    )?;
    Ok(operation_matches == 1
        && preimage_matches == 1
        && count_rows(conn, "dendro_migration_operation")? == 1
        && count_rows(conn, "dendro_rollback_preimage")? == 1)
}

fn view_definitions(conn: &Connection) -> Result<Vec<(String, String)>, MigrationError> {
    let mut stmt = conn.prepare(
        "SELECT table_name, view_definition FROM information_schema.views
         WHERE table_catalog = current_database() AND table_schema = 'main' ORDER BY table_name",
    )?;
    let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(MigrationError::from)
}

pub fn set_operation_status(conn: &Connection, status: &str) -> Result<(), MigrationError> {
    let updated = conn.execute(
        "UPDATE dendro_migration_operation SET status = ? WHERE id = 1",
        params![status],
    )?;
    if updated != 1 {
        return Err(MigrationError::Duck(format!(
            "operation status update changed {updated} rows"
        )));
    }
    Ok(())
}

/// DuckDB does not implement SQLite's PRAGMA integrity_check. Read every base
/// table cell through the engine instead. This proves logical readability only;
/// equivalence and source/preimage checks remain independent admission gates.
pub fn ensure_logical_readability(conn: &Connection) -> Result<(), MigrationError> {
    candidate_digest(conn)?;
    Ok(())
}

/// A closed candidate seal covers all base table schemas/values and all view
/// definitions. It is an additional corruption detector, not a source of authority.
pub fn candidate_digest(conn: &Connection) -> Result<String, MigrationError> {
    let tables = list_base_tables(conn)?;
    let mut columns = BTreeMap::new();
    let mut hasher = Sha256::new();
    for table in &tables {
        let cols = list_columns(conn, table)?;
        hasher.update((table.len() as u64).to_be_bytes());
        hasher.update(table.as_bytes());
        for col in &cols {
            for value in [&col.name, &col.data_type] {
                hasher.update((value.len() as u64).to_be_bytes());
                hasher.update(value.as_bytes());
            }
        }
        columns.insert(table.clone(), cols);
    }
    hasher.update(projection_hash(conn, &tables, &columns)?.as_bytes());
    let mut stmt = conn.prepare(
        "SELECT table_name, view_definition FROM information_schema.views
         WHERE table_catalog = current_database() AND table_schema = 'main' ORDER BY table_name",
    )?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        for idx in 0..2 {
            let value: String = row.get(idx)?;
            hasher.update((value.len() as u64).to_be_bytes());
            hasher.update(value.as_bytes());
        }
    }
    Ok(hex::encode(hasher.finalize()))
}

pub fn legacy_counts_match(
    staged: &Connection,
    inspection: &SourceInspection,
) -> Result<bool, MigrationError> {
    for (table, expected) in &inspection.counts {
        let quoted = quote_ident(table)?;
        let actual: i64 =
            staged.query_row(&format!("SELECT COUNT(*) FROM {quoted}"), [], |row| {
                row.get(0)
            })?;
        if actual != *expected {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metadata_fixture() -> (Connection, SourceInspection) {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE dendro_meta (schema_version INTEGER, key_id VARCHAR);
             INSERT INTO dendro_meta VALUES (3, 'synthetic-key');",
        )
        .unwrap();
        let tables = list_base_tables(&conn).unwrap();
        let columns = BTreeMap::from([(
            "dendro_meta".to_string(),
            list_columns(&conn, "dendro_meta").unwrap(),
        )]);
        let inspection = SourceInspection {
            schema_version: 3,
            key_id: "synthetic-key".into(),
            projection_sha256: projection_hash(&conn, &tables, &columns).unwrap(),
            tables,
            columns,
            counts: BTreeMap::from([("dendro_meta".to_string(), 1)]),
            generation_fingerprint: "source_files_absent".into(),
        };
        install_schema4_metadata(
            &conn,
            &inspection,
            "source-hash",
            "side-hash",
            "synthetic-time",
            "validated_unpublished",
        )
        .unwrap();
        (conn, inspection)
    }

    #[test]
    fn derived_metadata_rejects_mutation_even_with_recomputed_content_digest() {
        for sql in [
            "INSERT INTO dendro_spawn_edges VALUES ('p', 'c')",
            "UPDATE dendro_schema4_field_state SET state = 'known'",
            "INSERT INTO dendro_schema4_precision VALUES ('invented', 'money', 'exact')",
            "CREATE TABLE extra_facts (id VARCHAR)",
            "CREATE OR REPLACE VIEW legacy_dendro_meta AS SELECT * FROM dendro_meta WHERE false",
        ] {
            let (conn, inspection) = metadata_fixture();
            assert!(pristine_metadata_matches(&conn, &inspection, "source-hash").unwrap());
            conn.execute_batch(sql).unwrap();
            let _rewritten_seal = candidate_digest(&conn).unwrap();
            assert!(
                !pristine_metadata_matches(&conn, &inspection, "source-hash").unwrap(),
                "{sql}"
            );
        }
    }

    #[test]
    fn unreadable_committed_stage_gets_typed_refusal_and_failed_status() {
        let (conn, inspection) = metadata_fixture();
        set_operation_status(&conn, "committed_unvalidated").unwrap();
        conn.execute_batch("DROP TABLE dendro_fact_revisions")
            .unwrap();
        match super::super::validate_committed_stage(&conn, &inspection).unwrap_err() {
            MigrationError::Refused(refusal) => {
                assert_eq!(refusal.code, "staged_integrity_check");
                assert!(refusal.stage_retained);
                assert!(!refusal.pointer_changed);
            }
            other => panic!("{other}"),
        }
        let status: String = conn
            .query_row("SELECT status FROM dendro_migration_operation", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(status, "integrity_failed");
    }

    #[test]
    fn nested_float_types_do_not_use_the_scalar_binary64_decoder() {
        for nested in ["DOUBLE[]", "STRUCT(value DOUBLE)", "FLOAT[3]"] {
            assert!(!is_float(nested));
        }
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE nested_values (samples DOUBLE[]); INSERT INTO nested_values VALUES ([0.1, 2.0])").unwrap();
        let tables = list_base_tables(&conn).unwrap();
        let columns = BTreeMap::from([(
            "nested_values".to_string(),
            list_columns(&conn, "nested_values").unwrap(),
        )]);
        projection_hash(&conn, &tables, &columns).unwrap();
    }

    #[test]
    fn extension_directory_rejects_invalid_path_before_duckdb_binding() {
        let conn = Connection::open_in_memory().unwrap();
        let error = super::super::configure_extension_directory(&conn, Path::new("invalid\0ext"))
            .unwrap_err();
        assert!(error.to_string().contains("NUL byte"), "{error}");
    }

    #[test]
    fn staged_projection_allows_only_the_schema_label_change() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE dendro_meta (schema_version INTEGER, key_id VARCHAR, retained_note VARCHAR);
             INSERT INTO dendro_meta VALUES (3, 'synthetic-key', 'preserved');",
        ).unwrap();
        let tables = vec!["dendro_meta".to_string()];
        let columns = BTreeMap::from([(
            "dendro_meta".to_string(),
            list_columns(&conn, "dendro_meta").unwrap(),
        )]);
        let original = projection_hash(&conn, &tables, &columns).unwrap();
        conn.execute("UPDATE dendro_meta SET schema_version = 4", [])
            .unwrap();
        assert_ne!(projection_hash(&conn, &tables, &columns).unwrap(), original);
        assert_eq!(
            staged_projection_hash(&conn, &tables, &columns).unwrap(),
            original
        );
        conn.execute("UPDATE dendro_meta SET retained_note = 'corrupted'", [])
            .unwrap();
        assert_ne!(
            staged_projection_hash(&conn, &tables, &columns).unwrap(),
            original
        );
        conn.execute(
            "UPDATE dendro_meta SET retained_note = 'preserved', schema_version = 5",
            [],
        )
        .unwrap();
        assert_ne!(
            staged_projection_hash(&conn, &tables, &columns).unwrap(),
            original
        );
    }
}
