#![cfg(any(windows, target_os = "linux"))]
//! Actual selected-publication migration and restart-safe rollback, synthetic only.
use std::io::Write;
use std::path::{Path, PathBuf};

use dendro_core::migration_v4 as m;
use dendro_core::platform_v1 as p;
use dendro_core::selection_binding as binding;
use duckdb::Connection;

struct Fixture {
    root: p::PrivateRoot,
    db: PathBuf,
    key: PathBuf,
    db_hash: String,
    key_hash: String,
}
impl Fixture {
    fn build() -> Self {
        Self::build_with_schema(false)
    }
    fn build_with_schema(production: bool) -> Self {
        let native = if cfg!(windows) {
            p::NativePlatform::Windows
        } else {
            p::NativePlatform::Linux
        };
        #[cfg(windows)]
        let root = p::create_user_private_root_with_choice(
            "dendro-selected-migration-",
            &p::StorageRootChoice::UserProfile,
        )
        .unwrap();
        #[cfg(not(windows))]
        let root = p::create_user_private_root("dendro-selected-migration-").unwrap();
        let db = root.path().join("analytics.duckdb");
        let key = root.path().join("analytics.duckdb.key");
        if production {
            let store =
                dendro_core::AnalyticsStore::open_path(&db, "630dcd2966c4336691125448bbb25b4f")
                    .unwrap();
            store.checkpoint().unwrap();
            drop(store);
        } else {
            let conn = Connection::open(&db).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&db, std::fs::Permissions::from_mode(0o600)).unwrap();
            }
            conn.execute_batch(include_str!(
                "../../../.agentflow/oracles/v080/T04/schema3-fixture.sql"
            ))
            .unwrap();
            conn.execute_batch("CHECKPOINT").unwrap();
            drop(conn);
        }
        let mut key_file = root
            .create_file(&p::parse_child_name("analytics.duckdb.key", native).unwrap())
            .unwrap();
        key_file
            .file_mut()
            .write_all(
                &m::encode_sidecar(
                    m::SIDECAR_VERSION,
                    &std::array::from_fn(|index| index as u8),
                )
                .unwrap(),
            )
            .unwrap();
        key_file.file_mut().sync_all().unwrap();
        drop(key_file);
        let db_hash = m::sha256_file(&db).unwrap();
        let key_hash = m::sha256_file(&key).unwrap();
        Self {
            root,
            db,
            key,
            db_hash,
            key_hash,
        }
    }
    fn choice(&self) -> p::StorageRootChoice {
        p::StorageRootChoice::Explicit(self.root.path().to_path_buf())
    }
    fn assert_unchanged(&self) {
        assert_eq!(m::sha256_file(&self.db).unwrap(), self.db_hash);
        assert_eq!(m::sha256_file(&self.key).unwrap(), self.key_hash);
    }
}

#[test]
fn semantic_refresh_is_readable_and_rollback_remains_available() {
    let fixture = Fixture::build_with_schema(true);
    let migrated = binding::migrate_logical_db(&fixture.db, &fixture.choice()).unwrap();
    drop(migrated);
    dendro_core::semantic::refresh(
        &fixture.db,
        None,
        "2026-10-22T00:00:00Z",
        "2026-10-23T00:00:00Z",
    )
    .unwrap();
    assert_eq!(schema(&reopen(&fixture).database), 4);
    let (restored, _) =
        binding::rollback_logical_db(&fixture.db, &fixture.choice(), false).unwrap();
    assert_eq!(schema(&restored.database), 3);
    assert_eq!(m::sha256_file(&restored.database).unwrap(), fixture.db_hash);
    fixture.assert_unchanged();
}

#[test]
fn unsupported_candidate_schema_is_refused_before_pointer_switch() {
    let fixture = Fixture::build();
    let selected = binding::migrate_logical_db(&fixture.db, &fixture.choice()).unwrap();
    let before = selected.selection();
    let result = binding::mutate_selected(&selected, |staged, _| {
        let conn = Connection::open(staged)?;
        conn.execute_batch("CREATE TABLE unreviewed_cache (value INTEGER); CHECKPOINT;")?;
        Ok(())
    });
    assert!(result.is_err());
    drop(selected);
    assert_eq!(reopen(&fixture).selection().sha256, before.sha256);
    fixture.assert_unchanged();
}

#[test]
fn remigration_never_discards_retained_rollback_loss_lineage() {
    let fixture = Fixture::build();
    drop(binding::migrate_logical_db(&fixture.db, &fixture.choice()).unwrap());
    let (restored, _) =
        binding::rollback_logical_db(&fixture.db, &fixture.choice(), false).unwrap();
    let before = restored.selection();
    drop(restored);
    let error = binding::migrate_logical_db(&fixture.db, &fixture.choice())
        .err()
        .expect("retained lineage requires an explicit new migration epoch");
    assert!(error
        .to_string()
        .contains("remigration_refused_retained_rollback_lineage"));
    let unchanged = reopen(&fixture);
    assert_eq!(unchanged.selection().sha256, before.sha256);
    assert!(unchanged.custody.file("failed-schema4.duckdb").is_some());
    assert!(unchanged.custody.file("rollback-loss.json").is_some());
}

#[test]
fn schema4_validator_rejects_label_columns_key_and_unknown_tables() {
    let fixture = Fixture::build();
    let selected = binding::migrate_logical_db(&fixture.db, &fixture.choice()).unwrap();
    let source = selected
        .custody
        .bundle()
        .path()
        .join("schema3-preimage.duckdb");
    let key = selected
        .custody
        .bundle()
        .path()
        .join("schema3-preimage.key");
    for (index, sql) in [
        "UPDATE dendro_meta SET schema_version=5;",
        "DROP VIEW legacy_usage_facts; ALTER TABLE usage_facts ALTER COLUMN output_tokens SET DATA TYPE VARCHAR; CREATE VIEW legacy_usage_facts AS SELECT * FROM usage_facts;",
        "UPDATE dendro_migration_operation SET source_sidecar_sha256='invented';",
        "CREATE TABLE unreviewed_extension (value INTEGER);",
    ].iter().enumerate() {
        let copy = fixture.root.path().join(format!("tampered-{index}.duckdb"));
        std::fs::copy(&selected.database, &copy).unwrap();
        let conn = Connection::open(&copy).unwrap();
        conn.execute_batch(sql).unwrap();
        conn.execute_batch("CHECKPOINT").unwrap();
        drop(conn);
        assert!(m::validate_selected_schema4(&copy, &source, &key).is_err(), "tamper case {index} admitted");
    }
}

#[test]
fn old_binary_style_anchor_write_is_detected_without_changing_selection() {
    let fixture = Fixture::build();
    let selected = binding::migrate_logical_db(&fixture.db, &fixture.choice()).unwrap();
    let manifest = selected.selection();
    let manifest_path = std::fs::read_dir(fixture.root.path())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            p::reopen_metadata(path).is_ok_and(|metadata| metadata.identity == manifest.identity)
        })
        .expect("persisted selected manifest identity");
    let manifest_before = std::fs::read(&manifest_path).unwrap();
    let selected_db = selected.database.clone();
    let selected_hash = m::sha256_file(&selected_db).unwrap();
    drop(selected);
    let conn = Connection::open(&fixture.db).unwrap();
    conn.execute_batch("UPDATE usage_facts SET output_tokens=output_tokens+1; CHECKPOINT;")
        .unwrap();
    drop(conn);
    let error = binding::resolve_logical_db(&fixture.db, &fixture.choice())
        .err()
        .expect("old anchor write must be explicit divergence");
    assert!(error
        .to_string()
        .contains("legacy_anchor_diverged_after_selection"));
    assert_eq!(m::sha256_file(&selected_db).unwrap(), selected_hash);
    assert_eq!(std::fs::read(manifest_path).unwrap(), manifest_before);
}
fn schema(path: &Path) -> i32 {
    let conn = Connection::open_with_flags(
        path,
        duckdb::Config::default()
            .access_mode(duckdb::AccessMode::ReadOnly)
            .unwrap(),
    )
    .unwrap();
    conn.query_row("SELECT schema_version FROM dendro_meta", [], |row| {
        row.get(0)
    })
    .unwrap()
}
fn reopen(fixture: &Fixture) -> binding::SelectedRead {
    match binding::resolve_logical_db(&fixture.db, &fixture.choice()).unwrap() {
        binding::SelectedDb::Selected(read) => *read,
        _ => panic!("selection lost after restart"),
    }
}

#[test]
fn real_migration_reopens_and_rollback_selects_exact_preserved_pair() {
    let fixture = Fixture::build();
    let selected = binding::migrate_logical_db(&fixture.db, &fixture.choice()).unwrap();
    assert_eq!(schema(&selected.database), 4);
    assert_eq!(selected.custody.manifest.payload.files.len(), 9);
    let first = selected.selection();
    fixture.assert_unchanged();
    drop(selected);
    let reopened = reopen(&fixture);
    assert_eq!(reopened.selection().sha256, first.sha256);
    drop(reopened);
    let same = binding::migrate_logical_db(&fixture.db, &fixture.choice()).unwrap();
    assert_eq!(
        same.selection().sha256,
        first.sha256,
        "repeat migration is idempotent"
    );
    drop(same);
    let (rollback, loss) =
        binding::rollback_logical_db(&fixture.db, &fixture.choice(), false).unwrap();
    assert_eq!(schema(&rollback.database), 3);
    assert_eq!(m::sha256_file(&rollback.database).unwrap(), fixture.db_hash);
    assert_eq!(m::sha256_file(&rollback.key).unwrap(), fixture.key_hash);
    assert!(loss.pointer_changed && loss.full_continuity_claim && !loss.destructive_downmigration);
    assert_eq!(loss.excluded_fact_count, 0);
    assert_eq!(rollback.custody.manifest.payload.files.len(), 11);
    assert!(loss.failed_schema4_path.is_file());
    drop(rollback);
    let reopened = reopen(&fixture);
    assert_eq!(schema(&reopened.database), 3);
    assert!(reopened.custody.file("rollback-loss.json").is_some());
    fixture.assert_unchanged();
}

#[test]
fn rollback_retains_exact_loss_after_later_schema4_publication_and_restart() {
    let fixture = Fixture::build();
    let selected = binding::migrate_logical_db(&fixture.db, &fixture.choice()).unwrap();
    let updated = binding::mutate_selected(&selected, |staged, _| {
        for (id, ts, range) in [
            ("v4-A", "2026-10-22 00:00:02", [512, 544]),
            ("v4-B", "2026-10-22 00:00:03", [544, 576]),
        ] {
            m::record_v4_only_fact(
                staged,
                &m::V4OnlyFact {
                    id: id.into(),
                    ts: ts.into(),
                    source_id: "src-A".into(),
                    generation: 2,
                    byte_range_half_open: range,
                },
            )?;
        }
        Ok(())
    })
    .unwrap();
    drop(selected);
    let failed_hash = m::sha256_file(&updated.database).unwrap();
    drop(updated);
    let (rollback, loss) =
        binding::rollback_logical_db(&fixture.db, &fixture.choice(), false).unwrap();
    assert_eq!(loss.excluded_fact_count, 2);
    assert!(!loss.replay_available && loss.unresolved_loss && !loss.full_continuity_claim);
    assert_eq!(
        m::sha256_file(&loss.failed_schema4_path).unwrap(),
        failed_hash
    );
    assert_eq!(
        loss.excluded_source_ranges[0].byte_range_half_open,
        [512, 576]
    );
    drop(rollback);
    let reopened = reopen(&fixture);
    let next = binding::mutate_selected(&reopened, |_staged, _| Ok(())).unwrap();
    assert!(
        next.custody.file("rollback-loss.json").is_some(),
        "subsequent writes retain durable rollback evidence"
    );
    drop(next);
    drop(reopened);
    assert_eq!(schema(&reopen(&fixture).database), 3);
    fixture.assert_unchanged();
}

#[test]
fn rollback_derives_actual_new_usage_and_updated_source_coverage() {
    let fixture = Fixture::build();
    let selected = binding::migrate_logical_db(&fixture.db, &fixture.choice()).unwrap();
    let appended = binding::mutate_selected(&selected, |staged, _| {
        let conn = Connection::open(staged)?;
        conn.execute_batch("INSERT INTO usage_facts SELECT * REPLACE ('runtime-usage-new' AS source_key, TIMESTAMP '2026-10-22 00:00:04' AS ts) FROM usage_facts WHERE source_key = 'usage-1'; UPDATE source_files SET byte_offset = 768 WHERE source_id = 'src-A' AND generation = 2; CHECKPOINT;")?;
        Ok(())
    }).unwrap();
    drop(appended);
    drop(selected);
    let (rolled_back, loss) =
        binding::rollback_logical_db(&fixture.db, &fixture.choice(), false).unwrap();
    assert_eq!(loss.excluded_fact_count, 1);
    assert_eq!(loss.new_usage_fact_count, 1);
    assert_eq!(loss.new_usage_unknown_timestamp_count, 0);
    assert!(!loss.full_continuity_claim && loss.unresolved_loss);
    assert_eq!(
        loss.postmigration_watermarks
            .iter()
            .find(|w| w.source_id == "src-A" && w.generation == 2)
            .unwrap()
            .byte_offset,
        768
    );
    assert_eq!(
        loss.last_common_watermarks
            .iter()
            .find(|w| w.source_id == "src-A" && w.generation == 2)
            .unwrap()
            .byte_offset,
        512
    );
    assert!(loss
        .legacy_table_changes
        .iter()
        .any(|table| table.table == "usage_facts" && table.added_rows == 1));
    assert_eq!(
        m::sha256_file(&rolled_back.database).unwrap(),
        fixture.db_hash
    );
    drop(rolled_back);
    assert_eq!(schema(&reopen(&fixture).database), 3);
    fixture.assert_unchanged();
}
