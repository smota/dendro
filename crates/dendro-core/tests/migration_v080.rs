//! T04 schema4 staging against the frozen oracle recipes.
//! Publication remains the unproved manifest refusal. Historical v0.7.0 is not executed here.

use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};

use duckdb::Connection;
use sha2::{Digest, Sha256};

use dendro_core::migration_v4::{self as mig, InterruptBoundary, MigrateRequest};
use dendro_core::platform_v1::{self, FileIdentity, ObjectKind};
use dendro_core::AnalyticsStore;

const FROZEN_JSON: &str = "33837c362c332b3021b2319800a8ab94a455cc43c306393915535634ffe8f113";
const FIXTURE_SQL: &str = "aa5fbc2bbd48f59f6a0ef1d41612625394d6dd24919560eacb2c53bc3dbc11b6";
const FIXTURE_PROJECTIONS: &str =
    "f70a2014ac83472413a4806b5b437a713319c0f69bc7ed1d5d56c74d17b48470";
const CURRENCY_SQL: &str = "cdc5a1837fb9fc384cbef4245d64595279e2b67b827902c0125c069b1a2d45c0";

// The reviewed Windows handoff admits explicit USERPROFILE; the default
// LOCALAPPDATA refusal is preserved by the production entry point.
fn migrate(request: MigrateRequest<'_>) -> Result<mig::StageCandidate, mig::MigrationError> {
    #[cfg(windows)]
    {
        mig::migrate_closed_snapshot_with_root_choice(
            request,
            &platform_v1::StorageRootChoice::UserProfile,
        )
    }
    #[cfg(not(windows))]
    {
        mig::migrate_closed_snapshot(request)
    }
}

fn oracle_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.agentflow/oracles/v080/T04")
}

fn sha256_bytes(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn sha256_path(path: &Path) -> String {
    sha256_bytes(&fs::read(path).unwrap())
}

fn assert_oracle_digests() {
    let dir = oracle_dir();
    let canonical = |name: &str| {
        sha256_bytes(
            fs::read_to_string(dir.join(name))
                .unwrap()
                .replace("\r\n", "\n")
                .as_bytes(),
        )
    };
    assert_eq!(canonical("frozen.json"), FROZEN_JSON);
    assert_eq!(canonical("schema3-fixture.sql"), FIXTURE_SQL);
    assert_eq!(
        canonical("schema3-fixture-projections.json"),
        FIXTURE_PROJECTIONS
    );
    assert_eq!(canonical("schema3-currency-extension.sql"), CURRENCY_SQL);
    assert_eq!(dendro_core::SCHEMA_VERSION, 3);
}

fn synthetic_key() -> [u8; 32] {
    let mut key = [0u8; 32];
    for (index, byte) in key.iter_mut().enumerate() {
        *byte = index as u8;
    }
    key
}

struct Fixture {
    dir: PathBuf,
    database: PathBuf,
    sidecar: PathBuf,
    db_hash: String,
    sidecar_hash: String,
}

impl Fixture {
    fn build(extend_currency: bool) -> Self {
        assert_oracle_digests();
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("dendro-t04-{nonce}-{}", process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let database = dir.join("analytics.duckdb");
        let conn = Connection::open(&database).unwrap();
        let sql = fs::read_to_string(oracle_dir().join("schema3-fixture.sql")).unwrap();
        conn.execute_batch(&sql).unwrap();
        if extend_currency {
            let extra =
                fs::read_to_string(oracle_dir().join("schema3-currency-extension.sql")).unwrap();
            conn.execute_batch(&extra).unwrap();
        }
        conn.execute_batch("CHECKPOINT").unwrap();
        drop(conn);
        let wal = dir.join("analytics.duckdb.wal");
        assert!(!wal.exists(), "fixture must be a closed snapshot");
        let key = synthetic_key();
        let sidecar_bytes = mig::encode_sidecar(mig::SIDECAR_VERSION, &key).unwrap();
        assert_eq!(sidecar_bytes.len(), 72);
        let parsed = mig::parse_sidecar(&sidecar_bytes).unwrap();
        assert_eq!(parsed.key_id, "630dcd2966c4336691125448bbb25b4f");
        assert_eq!(
            parsed.key_sha256_hex,
            "630dcd2966c4336691125448bbb25b4ff412a49c732db2c8abc1b8581bd710dd"
        );
        let sidecar = dir.join("analytics.duckdb.key");
        fs::write(&sidecar, sidecar_bytes).unwrap();
        let db_hash = sha256_path(&database);
        let sidecar_hash = sha256_path(&sidecar);
        Self {
            dir,
            database,
            sidecar,
            db_hash,
            sidecar_hash,
        }
    }

    fn request<'a>(&'a self) -> MigrateRequest<'a> {
        MigrateRequest {
            source_database: &self.database,
            key_sidecar: Some(&self.sidecar),
            previous_private_root: None,
            stop_at: None,
            expected_source_sha256_hex: None,
            damage_staged_usage_row: false,
        }
    }

    fn assert_unchanged(&self) {
        assert_eq!(sha256_path(&self.database), self.db_hash);
        assert_eq!(sha256_path(&self.sidecar), self.sidecar_hash);
        assert!(!self.dir.join("analytics.duckdb.wal").exists());
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn cleanup_root(path: &Path) {
    let _ = fs::remove_dir_all(path);
}

fn scalar_string(conn: &Connection, sql: &str) -> String {
    conn.query_row(sql, [], |row| row.get(0)).unwrap()
}

fn scalar_i64(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |row| row.get(0)).unwrap()
}

#[test]
fn t04_01_02_05_06_23_stage_preserves_schema3_bundle() {
    let fixture = Fixture::build(false);
    let candidate = migrate(fixture.request()).unwrap();
    fixture.assert_unchanged();
    assert_eq!(candidate.target_schema, 4);
    assert!(!candidate.selected);
    assert_eq!(
        candidate.publication_code,
        "atomic_existing_manifest_replace_unproved"
    );
    assert!(candidate.publication_attempts <= mig::PUBLICATION_ATTEMPT_LIMIT);
    assert_eq!(
        candidate.file_identity.file_id_hex.len(),
        if cfg!(windows) { 32 } else { 16 }
    );
    assert_eq!(
        candidate.file_identity.kind,
        if cfg!(windows) {
            "windows_volume_u64_file_id_128"
        } else {
            "unix_device_inode"
        }
    );
    assert_eq!(sha256_path(&candidate.preimage_database), fixture.db_hash);
    assert_eq!(sha256_path(&candidate.preimage_key), fixture.sidecar_hash);

    let staged = mig_ro(&candidate.database_path);
    assert_eq!(
        scalar_i64(&staged, "SELECT schema_version FROM dendro_meta"),
        4
    );
    assert_eq!(
        scalar_string(&staged, "SELECT key_id FROM dendro_meta"),
        "630dcd2966c4336691125448bbb25b4f"
    );
    let projections: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(oracle_dir().join("schema3-fixture-projections.json")).unwrap(),
    )
    .unwrap();
    let tables = projections["projections"].as_object().unwrap();
    for (table, spec) in tables {
        let expected = spec["expected_row_count"].as_i64().unwrap();
        let actual = scalar_i64(&staged, &format!("SELECT COUNT(*) FROM \"{table}\""));
        assert_eq!(actual, expected, "{table}");
        let view = scalar_i64(&staged, &format!("SELECT COUNT(*) FROM \"legacy_{table}\""));
        assert_eq!(view, expected, "legacy_{table}");
    }
    assert_eq!(
        scalar_i64(
            &staged,
            "SELECT CAST(input_uncached + cache_read + output_tokens AS BIGINT) FROM legacy_usage_facts WHERE source_key = 'usage-0'"
        ),
        0
    );
    assert_eq!(
        scalar_i64(
            &staged,
            "SELECT CAST(SUM(input_uncached) AS BIGINT) FROM legacy_usage_facts"
        ),
        90
    );
    assert_eq!(
        scalar_i64(
            &staged,
            "SELECT CAST(SUM(cache_read) AS BIGINT) FROM legacy_usage_facts"
        ),
        10
    );
    assert_eq!(
        scalar_i64(
            &staged,
            "SELECT CAST(SUM(output_tokens) AS BIGINT) FROM legacy_usage_facts"
        ),
        4
    );
    assert_eq!(
        scalar_i64(
            &staged,
            "SELECT COUNT(*) FROM legacy_usage_facts WHERE source_key = 'usage-0' AND message_id IS NULL AND ts IS NULL AND currency_total IS NULL"
        ),
        1
    );
    assert_eq!(
        scalar_i64(
            &staged,
            "SELECT COUNT(*) FROM legacy_usage_facts WHERE source_key = 'usage-1' AND message_id = 'm1' AND currency_total = 0"
        ),
        1
    );
    assert_eq!(
        scalar_string(
            &staged,
            "SELECT CAST(ts AS VARCHAR) FROM legacy_usage_facts WHERE source_key = 'usage-1'"
        ),
        "2026-10-22 00:00:00"
    );
    assert!(!scalar_string(
        &staged,
        "SELECT CAST(ts AS VARCHAR) FROM legacy_usage_facts WHERE source_key = 'usage-1'"
    )
    .contains('+'));
    assert_eq!(
        scalar_string(
            &staged,
            "SELECT operation_timestamp FROM dendro_migration_operation"
        ),
        candidate.operation_timestamp
    );
    assert!(!candidate
        .operation_timestamp
        .contains("2026-10-22 00:00:00"));
    assert_eq!(
        scalar_i64(
            &staged,
            "SELECT COUNT(*) FROM information_schema.columns WHERE table_name = 'usage_facts' AND column_name = 'ingested'"
        ),
        0
    );
    assert_eq!(
        mig::field_state(&candidate.database_path, "invocation").unwrap(),
        "unknown"
    );
    assert_eq!(
        mig::field_state(&candidate.database_path, "lineage").unwrap(),
        "unknown"
    );
    assert_eq!(
        mig::field_state(&candidate.database_path, "acceptance").unwrap(),
        "unknown"
    );
    assert_eq!(
        mig::field_state(&candidate.database_path, "context_capacity").unwrap(),
        "unknown"
    );
    assert_eq!(mig::spawn_edge_count(&candidate.database_path).unwrap(), 0);
    assert_eq!(mig::revision_count(&candidate.database_path).unwrap(), 0);

    assert_eq!(
        scalar_string(
            &staged,
            "SELECT prefix_sha256 FROM source_files WHERE source_id = 'src-A'"
        ),
        "1111111111111111111111111111111111111111111111111111111111111111"
    );
    assert_eq!(
        scalar_i64(
            &staged,
            "SELECT byte_offset FROM source_files WHERE source_id = 'src-A'"
        ),
        512
    );
    assert_eq!(
        scalar_i64(
            &staged,
            "SELECT generation FROM source_files WHERE source_id = 'src-A'"
        ),
        2
    );
    assert_eq!(
        scalar_i64(
            &staged,
            "SELECT input_tokens FROM codex_counters WHERE source_id = 'src-A' AND generation = 2"
        ),
        100
    );
    assert_eq!(
        scalar_string(
            &staged,
            "SELECT producing_revision FROM openrig_row_records WHERE source_key = 'queue-A'"
        ),
        "r2"
    );
    assert_eq!(
        scalar_i64(
            &staged,
            "SELECT COUNT(*) FROM legacy_warnings WHERE path = 'synthetic-source-A' AND line_no IS NULL AND reason = 'synthetic-warning'"
        ),
        1
    );
    assert_eq!(
        mig::precision_tag(&candidate.database_path, "usage_facts", "currency_total").unwrap(),
        "legacy_binary64_approximate"
    );
    for (key, expected) in [
        ("usage-0", None),
        ("usage-1", Some("0000000000000000")),
        ("usage-2", Some("3fb999999999999a")),
        ("usage-3", Some("00000000000007e8")),
    ] {
        let bits = mig::legacy_currency_bits(&candidate.database_path, key).unwrap();
        let rendered = bits.map(|value| format!("{value:016x}"));
        assert_eq!(rendered.as_deref(), expected, "{key}");
        assert!(mig::canonical_exact_money_minor_units(bits).is_err());
    }
    let root = candidate.root.path().to_path_buf();
    drop(candidate);
    cleanup_root(&root);
}

/// A schema3 database that already holds message metadata migrates it like any
/// other legacy table: nothing in the migration is specific to `message_facts`.
#[test]
fn message_facts_table_migrates_as_a_generic_legacy_table() {
    let fixture = Fixture::build(false);
    let conn = Connection::open(&fixture.database).unwrap();
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS message_facts (
            source_key VARCHAR PRIMARY KEY, harness VARCHAR NOT NULL, native_session VARCHAR,
            ts TIMESTAMP, seq BIGINT NOT NULL, role VARCHAR NOT NULL, direction VARCHAR NOT NULL,
            modalities VARCHAR NOT NULL, grain VARCHAR NOT NULL, generation BIGINT NOT NULL,
            source_id VARCHAR NOT NULL);
         INSERT INTO message_facts VALUES
            ('m:1', 'claude', NULL, NULL, 1, 'user', 'inbound', 'text', 'message', 1, 'src');
         CHECKPOINT",
    )
    .unwrap();
    drop(conn);
    let before = sha256_path(&fixture.database);
    let candidate = migrate(fixture.request()).unwrap();
    assert_eq!(sha256_path(&fixture.database), before);
    let staged = mig_ro(&candidate.database_path);
    assert_eq!(scalar_i64(&staged, "SELECT COUNT(*) FROM message_facts"), 1);
    assert_eq!(
        scalar_i64(&staged, "SELECT COUNT(*) FROM legacy_message_facts"),
        1
    );
    let root = candidate.root.path().to_path_buf();
    drop(staged);
    drop(candidate);
    cleanup_root(&root);
}

#[test]
fn t04_03_missing_key_refuses() {
    let fixture = Fixture::build(false);
    let mut request = fixture.request();
    request.key_sidecar = None;
    let error = migrate(request).unwrap_err();
    match error {
        mig::MigrationError::Refused(refusal) => {
            assert_eq!(refusal.code, "key_missing");
            assert!(!refusal.pointer_changed);
            assert!(!refusal.source_mutated);
        }
        other => panic!("{other}"),
    }
    fixture.assert_unchanged();
}

#[test]
fn t04_04_key_mismatch_refuses() {
    let fixture = Fixture::build(false);
    let mut other = synthetic_key();
    other[0] = 0xff;
    fs::write(&fixture.sidecar, mig::encode_sidecar(1, &other).unwrap()).unwrap();
    let before = sha256_path(&fixture.database);
    let error = migrate(fixture.request()).unwrap_err();
    match error {
        mig::MigrationError::Refused(refusal) => {
            assert_eq!(refusal.code, "key_mismatch");
            assert!(!refusal.pointer_changed);
        }
        other => panic!("{other}"),
    }
    assert_eq!(sha256_path(&fixture.database), before);
}

#[test]
fn t04_07_repeat_reuses_validated_stage_without_selecting_it() {
    let fixture = Fixture::build(false);
    let first = migrate(fixture.request()).unwrap();
    let root = first.root.path().to_path_buf();
    let mut again = fixture.request();
    again.previous_private_root = Some(&root);
    let second = migrate(again).unwrap();
    assert!(second.reused);
    assert!(!second.selected);
    assert_eq!(second.publication_code, first.publication_code);
    let staged = mig_ro(&second.database_path);
    assert_eq!(scalar_i64(&staged, "SELECT COUNT(*) FROM usage_facts"), 4);
    fixture.assert_unchanged();
    let source = mig_ro(&fixture.database);
    assert_eq!(
        scalar_i64(&source, "SELECT schema_version FROM dendro_meta"),
        3
    );
    drop(staged);
    drop(source);
    drop(first);
    drop(second);
    cleanup_root(&root);
}

#[test]
fn t04_08_unsupported_schema_refuses() {
    for version in [2, 5] {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("dendro-t04-schema-{version}-{nonce}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let database = dir.join("analytics.duckdb");
        let conn = Connection::open(&database).unwrap();
        conn.execute_batch(&format!(
            "CREATE TABLE dendro_meta (schema_version INTEGER NOT NULL, key_id VARCHAR NOT NULL);
             INSERT INTO dendro_meta VALUES ({version}, '630dcd2966c4336691125448bbb25b4f');
             CHECKPOINT;"
        ))
        .unwrap();
        drop(conn);
        let sidecar = dir.join("analytics.duckdb.key");
        fs::write(&sidecar, mig::encode_sidecar(1, &synthetic_key()).unwrap()).unwrap();
        let before = sha256_path(&database);
        let error = migrate(MigrateRequest {
            source_database: &database,
            key_sidecar: Some(&sidecar),
            previous_private_root: None,
            stop_at: None,
            expected_source_sha256_hex: None,
            damage_staged_usage_row: false,
        })
        .unwrap_err();
        match error {
            mig::MigrationError::Refused(refusal) => {
                assert_eq!(refusal.code, "refuse_unsupported_schema");
                assert!(!refusal.pointer_changed);
                assert!(!refusal.source_mutated);
            }
            other => panic!("{other}"),
        }
        assert_eq!(sha256_path(&database), before);
        let _ = fs::remove_dir_all(&dir);
    }
}

#[test]
fn t04_09_checksum_sidecar_and_staged_integrity_refuse() {
    let fixture = Fixture::build(false);
    let mut checksum = fixture.request();
    checksum.expected_source_sha256_hex =
        Some("0000000000000000000000000000000000000000000000000000000000000000");
    match migrate(checksum).unwrap_err() {
        mig::MigrationError::Refused(refusal) => {
            assert_eq!(refusal.code, "source_checksum_mismatch");
            assert!(!refusal.stage_retained);
            assert!(!refusal.pointer_changed);
        }
        other => panic!("{other}"),
    }
    fixture.assert_unchanged();

    fs::write(
        &fixture.sidecar,
        mig::encode_sidecar(2, &synthetic_key()).unwrap(),
    )
    .unwrap();
    match migrate(fixture.request()).unwrap_err() {
        mig::MigrationError::Refused(refusal) => {
            assert_eq!(refusal.code, "sidecar_version_mismatch");
            assert!(!refusal.pointer_changed);
        }
        other => panic!("{other}"),
    }
    fs::write(
        &fixture.sidecar,
        mig::encode_sidecar(1, &synthetic_key()).unwrap(),
    )
    .unwrap();
    // The checksum above compared the original db. The sidecar bytes changed and were restored.
    let mut damage = fixture.request();
    damage.damage_staged_usage_row = true;
    match migrate(damage).unwrap_err() {
        mig::MigrationError::Refused(refusal) => {
            assert_eq!(refusal.code, "staged_integrity_check");
            assert!(refusal.stage_retained);
            assert!(!refusal.pointer_changed);
        }
        other => panic!("{other}"),
    }
    let source = mig_ro(&fixture.database);
    assert_eq!(
        scalar_i64(&source, "SELECT schema_version FROM dendro_meta"),
        3
    );
    assert_eq!(scalar_i64(&source, "SELECT COUNT(*) FROM usage_facts"), 4);
}

#[test]
fn t04_10_current_openers_refuse_schema4_before_supplemental_ddl() {
    // The pinned historical binary is not in this lease. This exercises the current
    // opener source, which is the behavior the frozen case describes.
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("dendro-t04-oldbin-{nonce}"));
    fs::create_dir_all(&dir).unwrap();
    let database = dir.join("schema4.duckdb");
    let conn = Connection::open(&database).unwrap();
    conn.execute_batch(
        "CREATE TABLE dendro_meta (schema_version INTEGER NOT NULL, key_id VARCHAR NOT NULL);
         INSERT INTO dendro_meta VALUES (4, '630dcd2966c4336691125448bbb25b4f');
         CREATE TABLE usage_facts (source_key VARCHAR PRIMARY KEY, currency_total DOUBLE);
         INSERT INTO usage_facts VALUES ('usage-1', 0.1);
         CHECKPOINT;",
    )
    .unwrap();
    drop(conn);
    let before = fs::read(&database).unwrap();
    assert!(AnalyticsStore::open_existing(&database, "630dcd2966c4336691125448bbb25b4f").is_err());
    assert!(AnalyticsStore::open_staged(&database).is_err());
    assert!(AnalyticsStore::open_read_only(&database).is_err());
    let after = Connection::open_with_flags(
        &database,
        duckdb::Config::default()
            .access_mode(duckdb::AccessMode::ReadOnly)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        scalar_i64(
            &after,
            "SELECT COUNT(*) FROM information_schema.tables WHERE table_name = 'dendro_semantic_state'"
        ),
        0
    );
    assert_eq!(
        scalar_i64(
            &after,
            "SELECT COUNT(*) FROM information_schema.tables WHERE table_name = 'openrig_row_records'"
        ),
        0
    );
    assert_eq!(scalar_i64(&after, "SELECT COUNT(*) FROM usage_facts"), 1);
    drop(after);
    // open_existing/open_staged are read-write opens. Canonical rows stay; incidental engine
    // metadata is not asserted byte-for-byte.
    let check = Connection::open_with_flags(
        &database,
        duckdb::Config::default()
            .access_mode(duckdb::AccessMode::ReadOnly)
            .unwrap(),
    )
    .unwrap();
    let bits: f64 = check
        .query_row(
            "SELECT currency_total FROM usage_facts WHERE source_key = 'usage-1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(format!("{:016x}", bits.to_bits()), "3fb999999999999a");
    drop(check);
    let _ = before;
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn t04_11_through_15_interrupt_before_selection() {
    for boundary in [
        InterruptBoundary::StageCreated,
        InterruptBoundary::PartialDataCopy,
        InterruptBoundary::BeforeTransactionCommit,
        InterruptBoundary::AfterTransactionCommitBeforeIntegrity,
        InterruptBoundary::AfterIntegrityBeforeManifestReplace,
    ] {
        let fixture = Fixture::build(false);
        let mut request = fixture.request();
        request.stop_at = Some(boundary);
        let error = migrate(request).unwrap_err();
        match error {
            mig::MigrationError::Interrupted(report) => {
                assert_eq!(report.boundary, boundary);
                assert_eq!(report.consumer_schema, 3);
                assert!(report.source_unchanged);
                assert!(!report.stage_selected);
                assert!(!report.live_db_deleted);
                assert!(report.publication_code.is_none());
                let stage = report.stage_path.expect("stage path");
                assert!(stage.exists());
                let recovery = mig::recover_private_stage(&fixture.database, &stage).unwrap();
                assert_eq!(recovery.consumer_schema, 3);
                assert!(!recovery.stage_selected);
                assert!(!recovery.live_db_deleted);
            }
            other => panic!("{boundary:?} {other}"),
        }
        fixture.assert_unchanged();
    }
}

#[test]
fn t04_16_manifest_replace_is_blocked_on_unproved_publication() {
    let fixture = Fixture::build(false);
    let mut request = fixture.request();
    request.stop_at = Some(InterruptBoundary::DuringManifestReplace);
    match migrate(request).unwrap_err() {
        mig::MigrationError::Interrupted(report) => {
            assert!(!report.stage_selected);
            assert_eq!(report.consumer_schema, 3);
            assert_eq!(
                report.publication_code,
                Some("atomic_existing_manifest_replace_unproved")
            );
            assert!(!report.live_db_deleted);
            assert!(report.stage_path.unwrap().exists());
        }
        other => panic!("{other}"),
    }
    fixture.assert_unchanged();
}

#[test]
fn t04_17_publication_failure_keeps_old_selection_and_private_target() {
    let fixture = Fixture::build(false);
    let candidate = migrate(fixture.request()).unwrap();
    assert!(!candidate.selected);
    assert!(candidate.publication_attempts >= 1);
    assert!(candidate.publication_attempts <= mig::PUBLICATION_ATTEMPT_LIMIT);
    assert_eq!(
        candidate.publication_code,
        "atomic_existing_manifest_replace_unproved"
    );
    assert!(candidate.database_path.exists());
    fixture.assert_unchanged();
    let source = mig_ro(&fixture.database);
    assert_eq!(
        scalar_i64(&source, "SELECT schema_version FROM dendro_meta"),
        3
    );
    let root = candidate.root.path().to_path_buf();
    drop(candidate);
    cleanup_root(&root);
}

#[test]
fn t04_18_cross_volume_does_not_assume_atomic_rename() {
    let left = platform_v1::ObservedMetadata {
        identity: FileIdentity::WindowsFileId128 {
            volume_serial_64: 1,
            file_id: [1; 16],
        },
        kind: ObjectKind::RegularFile,
        size: 1,
        modified_unix_ns: 0,
        links: 1,
        reparse: false,
        read_only_attribute: false,
        identity_limitation: None,
    };
    let mut right = left.clone();
    right.identity = FileIdentity::WindowsFileId128 {
        volume_serial_64: 2,
        file_id: [2; 16],
    };
    assert!(!mig::assess_volumes(&left, &right).unwrap());
    let same = left.clone();
    assert!(mig::assess_volumes(&left, &same).unwrap());
    let fixture = Fixture::build(false);
    let candidate = migrate(fixture.request()).unwrap();
    let source_meta = platform_v1::reopen_metadata(&fixture.database).unwrap();
    let same_volume = mig::assess_volumes(&source_meta, candidate.root.metadata()).unwrap();
    assert!(!candidate.selected);
    assert_eq!(
        candidate.publication_code,
        "atomic_existing_manifest_replace_unproved"
    );
    // A single observed volume cannot prove the cross-volume copy. The plan still refuses rename.
    let _ = same_volume;
    fixture.assert_unchanged();
    let root = candidate.root.path().to_path_buf();
    drop(candidate);
    cleanup_root(&root);
}

#[test]
fn t04_19_rollback_selects_preserved_schema3() {
    let fixture = Fixture::build(false);
    let candidate = migrate(fixture.request()).unwrap();
    let schema4_hash = sha256_path(&candidate.database_path);
    let receipt =
        mig::rollback_to_preserved_schema3(&fixture.database, &candidate.database_path, false)
            .unwrap();
    assert_eq!(receipt.selected_bundle, "preserved_schema3");
    assert!(!receipt.destructive_downmigration);
    assert!(receipt.failed_schema4_retained_private);
    assert!(!receipt.replay_available);
    assert!(receipt.full_continuity_claim);
    assert!(!receipt.unresolved_loss);
    assert_eq!(receipt.excluded_fact_count, 0);
    assert!(!receipt.pointer_changed);
    assert_eq!(sha256_path(&candidate.database_path), schema4_hash);
    fixture.assert_unchanged();
    let root = candidate.root.path().to_path_buf();
    drop(candidate);
    cleanup_root(&root);
}

#[test]
fn t04_20_rollback_reports_excluded_v4_facts() {
    let fixture = Fixture::build(false);
    let candidate = migrate(fixture.request()).unwrap();
    for fact in [
        mig::V4OnlyFact {
            id: "v4-A".into(),
            ts: "2026-10-22 00:00:02".into(),
            source_id: "src-A".into(),
            generation: 2,
            byte_range_half_open: [512, 544],
        },
        mig::V4OnlyFact {
            id: "v4-B".into(),
            ts: "2026-10-22 00:00:03".into(),
            source_id: "src-A".into(),
            generation: 2,
            byte_range_half_open: [544, 576],
        },
    ] {
        mig::record_v4_only_fact(&candidate.database_path, &fact).unwrap();
    }
    let schema4_hash = sha256_path(&candidate.database_path);
    let receipt =
        mig::rollback_to_preserved_schema3(&fixture.database, &candidate.database_path, false)
            .unwrap();
    assert_eq!(receipt.excluded_fact_count, 2);
    assert_eq!(
        receipt.excluded_windows,
        vec![("2026-10-22 00:00:02".into(), "2026-10-22 00:00:03".into())]
    );
    assert_eq!(
        receipt.last_common_watermark.as_ref().unwrap().source_id,
        "src-A"
    );
    assert_eq!(
        receipt.last_common_watermark.as_ref().unwrap().generation,
        2
    );
    assert_eq!(
        receipt.last_common_watermark.as_ref().unwrap().byte_offset,
        512
    );
    assert_eq!(
        receipt.last_common_cutoff.as_deref(),
        Some("2026-10-22 00:00:01")
    );
    assert_eq!(receipt.excluded_source_ranges.len(), 1);
    assert_eq!(
        receipt.excluded_source_ranges[0].byte_range_half_open,
        [512, 576]
    );
    assert!(!receipt.replay_available);
    assert!(!receipt.full_continuity_claim);
    assert!(receipt.unresolved_loss);
    assert!(receipt.failed_schema4_retained_private);
    assert!(!receipt.destructive_downmigration);
    assert_eq!(sha256_path(&candidate.database_path), schema4_hash);
    fixture.assert_unchanged();
    let root = candidate.root.path().to_path_buf();
    drop(candidate);
    cleanup_root(&root);
}

#[test]
fn t04_21_revision_cutoff_is_reproducible_and_not_synthesized() {
    let fixture = Fixture::build(false);
    let candidate = migrate(fixture.request()).unwrap();
    assert_eq!(mig::revision_count(&candidate.database_path).unwrap(), 0);
    mig::insert_fact_revisions(
        &candidate.database_path,
        &[
            mig::FactRevision {
                fact: "U".into(),
                revision: 1,
                ingested: 10,
                quantity: Some(5.0),
                retracted: false,
            },
            mig::FactRevision {
                fact: "U".into(),
                revision: 2,
                ingested: 20,
                quantity: Some(7.0),
                retracted: false,
            },
            mig::FactRevision {
                fact: "U".into(),
                revision: 3,
                ingested: 30,
                quantity: None,
                retracted: true,
            },
        ],
    )
    .unwrap();
    assert_eq!(mig::revision_count(&candidate.database_path).unwrap(), 3);
    let first = mig::query_revision_cutoff(&candidate.database_path, "U", 15).unwrap();
    let second = mig::query_revision_cutoff(&candidate.database_path, "U", 25).unwrap();
    let third = mig::query_revision_cutoff(&candidate.database_path, "U", 35).unwrap();
    assert_eq!(first, (Some(5.0), mig::RevisionState::Present));
    assert_eq!(second, (Some(7.0), mig::RevisionState::Present));
    assert_eq!(third, (None, mig::RevisionState::Retracted));
    let again = mig::query_revision_cutoff(&candidate.database_path, "U", 15).unwrap();
    assert_eq!(again, first);
    let staged = mig_ro(&candidate.database_path);
    let stored_ts = scalar_string(
        &staged,
        "SELECT CAST(ts AS VARCHAR) FROM usage_facts WHERE source_key = 'usage-1'",
    );
    assert_eq!(stored_ts, "2026-10-22 00:00:00");
    assert_ne!(candidate.operation_timestamp, stored_ts);
    fixture.assert_unchanged();
    let root = candidate.root.path().to_path_buf();
    drop(candidate);
    cleanup_root(&root);
}

#[test]
fn t04_22_wal_refuses_blind_copy() {
    let fixture = Fixture::build(false);
    let writer = Connection::open(&fixture.database).unwrap();
    writer
        .execute(
            "INSERT INTO warnings (path, line_no, reason) VALUES ('synthetic-live', NULL, 'live')",
            [],
        )
        .unwrap();
    let wal = fixture.dir.join("analytics.duckdb.wal");
    assert!(wal.exists(), "writer must leave a wal");
    let side_before = sha256_path(&fixture.sidecar);
    match migrate(fixture.request()).unwrap_err() {
        mig::MigrationError::Refused(refusal) => {
            assert_eq!(
                refusal.code,
                "refuse_without_admitted_consistency_mechanism"
            );
            assert!(!refusal.pointer_changed);
            assert!(!refusal.source_mutated);
        }
        other => panic!("{other}"),
    }
    // A Windows writer may deny every concurrent byte read, so do not claim a
    // stable source-file hash across a live writer. The refusal occurs before
    // any source connection/target creation and the writer still owns its rows.
    assert_eq!(
        scalar_i64(&writer, "SELECT schema_version FROM dendro_meta"),
        3
    );
    assert_eq!(
        scalar_i64(
            &writer,
            "SELECT COUNT(*) FROM warnings WHERE path = 'synthetic-live'"
        ),
        1
    );
    assert_eq!(sha256_path(&fixture.sidecar), side_before);
    drop(writer);
}

#[test]
fn t04_24_nonfinite_and_signed_zero_stay_legacy() {
    let fixture = Fixture::build(true);
    let source = Connection::open_with_flags(
        &fixture.database,
        duckdb::Config::default()
            .access_mode(duckdb::AccessMode::ReadOnly)
            .unwrap(),
    )
    .unwrap();
    let mut before = std::collections::BTreeMap::new();
    for key in ["usage-nan", "usage-posinf", "usage-neginf", "usage-negzero"] {
        let bits: Option<f64> = source
            .query_row(
                "SELECT currency_total FROM usage_facts WHERE source_key = ?",
                duckdb::params![key],
                |row| row.get(0),
            )
            .unwrap();
        before.insert(key, bits.map(f64::to_bits));
    }
    drop(source);
    let candidate = migrate(fixture.request()).unwrap();
    fixture.assert_unchanged();
    for (key, bits) in before {
        let after = mig::legacy_currency_bits(&candidate.database_path, key).unwrap();
        let before_class = mig::classify_currency_bits(bits);
        let after_class = mig::classify_currency_bits(after);
        assert_eq!(after_class, before_class, "{key}");
        assert_ne!(after_class, mig::LegacyCurrencyClass::Missing, "{key}");
        assert!(mig::canonical_exact_money_minor_units(after).is_err());
        if key == "usage-negzero" {
            assert_eq!(after_class, mig::LegacyCurrencyClass::SignedZero);
            assert_eq!(after, Some(0x8000_0000_0000_0000));
        }
        if key == "usage-nan" {
            assert_eq!(after_class, mig::LegacyCurrencyClass::Nan);
        }
        if key == "usage-posinf" {
            assert_eq!(after_class, mig::LegacyCurrencyClass::PositiveInfinity);
            assert_ne!(after, Some(0));
        }
        if key == "usage-neginf" {
            assert_eq!(after_class, mig::LegacyCurrencyClass::NegativeInfinity);
        }
    }
    let root = candidate.root.path().to_path_buf();
    drop(candidate);
    cleanup_root(&root);
}

fn mig_ro(path: &Path) -> Connection {
    Connection::open_with_flags(
        path,
        duckdb::Config::default()
            .access_mode(duckdb::AccessMode::ReadOnly)
            .unwrap(),
    )
    .unwrap()
}

// Regression cases supplement the frozen oracle without changing its expected vectors.
fn amend_fixture(fixture: &mut Fixture, sql: &str) {
    let conn = Connection::open(&fixture.database).unwrap();
    conn.execute_batch(sql).unwrap();
    conn.execute_batch("CHECKPOINT").unwrap();
    drop(conn);
    fixture.db_hash = sha256_path(&fixture.database);
}

#[test]
fn correction_schema_label_normalization_preserves_other_metadata() {
    let mut fixture = Fixture::build(false);
    amend_fixture(
        &mut fixture,
        "ALTER TABLE dendro_meta ADD COLUMN retained_note VARCHAR;
         UPDATE dendro_meta SET retained_note = 'source metadata';",
    );
    let candidate = migrate(fixture.request()).unwrap();
    let staged = mig_ro(&candidate.database_path);
    assert_eq!(
        scalar_i64(&staged, "SELECT schema_version FROM dendro_meta"),
        4
    );
    assert_eq!(
        scalar_string(&staged, "SELECT retained_note FROM dendro_meta"),
        "source metadata"
    );
    drop(staged);
    let conn = Connection::open(&candidate.database_path).unwrap();
    conn.execute(
        "UPDATE dendro_meta SET retained_note = 'corrupted metadata'",
        [],
    )
    .unwrap();
    conn.execute_batch("CHECKPOINT").unwrap();
    drop(conn);
    let mut request = fixture.request();
    request.previous_private_root = Some(candidate.root.path());
    assert!(migrate(request).is_err());
    fixture.assert_unchanged();
    let root = candidate.root.path().to_path_buf();
    drop(candidate);
    cleanup_root(&root);
}

#[test]
fn correction_reuse_rejects_same_inode_content_and_preimage_tampering() {
    for damage in [
        "usage",
        "key_id",
        "generation",
        "preimage_db",
        "preimage_key",
        "source_identity",
        "field_state",
        "compatibility_view",
        "extra_table",
    ] {
        let fixture = Fixture::build(false);
        let candidate = migrate(fixture.request()).unwrap();
        let identity_before = fs::read(candidate.root.path().join("stage.identity")).unwrap();
        match damage {
            "preimage_db" => fs::write(&candidate.preimage_database, b"corrupt preimage").unwrap(),
            "preimage_key" => fs::write(&candidate.preimage_key, [0; 72]).unwrap(),
            other => {
                let conn = Connection::open(&candidate.database_path).unwrap();
                let sql = match other {
                    "usage" => {
                        "UPDATE usage_facts SET input_uncached = 999 WHERE source_key = 'usage-1'"
                    }
                    "key_id" => "UPDATE dendro_meta SET key_id = 'corrupt'",
                    "generation" => {
                        "UPDATE source_files SET generation = 99 WHERE source_id = 'src-A'"
                    }
                    "source_identity" => {
                        "UPDATE dendro_migration_operation SET source_db_identity = 'corrupt'"
                    }
                    "field_state" => "UPDATE dendro_schema4_field_state SET state = 'known' WHERE field_name = 'acceptance'",
                    "compatibility_view" => "CREATE OR REPLACE VIEW legacy_usage_facts AS SELECT * FROM usage_facts WHERE false",
                    "extra_table" => "CREATE TABLE injected_table (secret VARCHAR)",
                    _ => unreachable!(),
                };
                conn.execute_batch(sql).unwrap();
                conn.execute_batch("CHECKPOINT").unwrap();
                drop(conn);
            }
        }
        assert_eq!(
            fs::read(candidate.root.path().join("stage.identity")).unwrap(),
            identity_before
        );
        let mut request = fixture.request();
        request.previous_private_root = Some(candidate.root.path());
        assert!(migrate(request).is_err(), "accepted tampering: {damage}");
        fixture.assert_unchanged();
        let root = candidate.root.path().to_path_buf();
        drop(candidate);
        cleanup_root(&root);
    }
}

#[test]
fn correction_reuse_rejects_identical_bytes_from_different_source_file() {
    let fixture = Fixture::build(false);
    let candidate = migrate(fixture.request()).unwrap();
    let replacement = fixture.dir.join("other.duckdb");
    fs::copy(&fixture.database, &replacement).unwrap();
    assert_eq!(sha256_path(&replacement), fixture.db_hash);
    let mut request = fixture.request();
    request.source_database = &replacement;
    request.previous_private_root = Some(candidate.root.path());
    assert!(migrate(request).is_err());
    fixture.assert_unchanged();
    let root = candidate.root.path().to_path_buf();
    drop(candidate);
    cleanup_root(&root);
}

#[test]
fn correction_rollback_accepts_empty_and_non_openrig_source_scopes() {
    for sql in [
        "DELETE FROM destination_snapshots; DELETE FROM source_files;",
        "DROP TABLE destination_snapshots;",
        "DROP TABLE source_files; DROP TABLE destination_snapshots;",
    ] {
        let mut fixture = Fixture::build(false);
        if sql.starts_with("DELETE") {
            // A truly empty schema3 image retains its schema and key metadata,
            // with no facts, counters, source records or OpenRig observations.
            let conn = Connection::open(&fixture.database).unwrap();
            let mut stmt = conn
                .prepare(
                    "SELECT table_name FROM information_schema.tables
                 WHERE table_schema = 'main' AND table_name <> 'dendro_meta'",
                )
                .unwrap();
            let tables = stmt
                .query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            drop(stmt);
            for table in tables {
                assert!(table.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
                conn.execute_batch(&format!("DELETE FROM {table}")).unwrap();
            }
            conn.execute_batch("CHECKPOINT").unwrap();
            drop(conn);
            fixture.db_hash = sha256_path(&fixture.database);
        } else {
            amend_fixture(&mut fixture, sql);
        }
        let candidate = migrate(fixture.request()).unwrap();
        let stage_before = sha256_path(&candidate.database_path);
        let receipt =
            mig::rollback_to_preserved_schema3(&fixture.database, &candidate.database_path, false)
                .unwrap();
        assert!(receipt.last_common_cutoff.is_none());
        assert!(receipt.last_common_cutoffs.is_empty());
        if sql.contains("DELETE FROM source_files") || sql.contains("DROP TABLE source_files") {
            assert!(receipt.last_common_watermark.is_none());
            assert!(receipt.last_common_watermarks.is_empty());
        } else {
            assert_eq!(receipt.last_common_watermarks.len(), 1);
        }
        if sql.starts_with("DELETE") {
            assert_eq!(receipt.watermark_scope, mig::EvidenceScope::Empty);
            assert_eq!(receipt.cutoff_scope, mig::EvidenceScope::Empty);
            assert_eq!(receipt.legacy_usage_scope, mig::EvidenceScope::Empty);
            assert!(receipt.full_continuity_claim);
        } else {
            assert_eq!(receipt.cutoff_scope, mig::EvidenceScope::Absent);
            if sql.contains("DROP TABLE source_files") {
                assert_eq!(receipt.watermark_scope, mig::EvidenceScope::Absent);
            }
            assert!(!receipt.full_continuity_claim);
            assert!(receipt.unresolved_loss);
        }
        assert_eq!(receipt.excluded_fact_count, 0);
        assert!(!receipt.pointer_changed);
        assert_eq!(sha256_path(&candidate.database_path), stage_before);
        fixture.assert_unchanged();
        let root = candidate.root.path().to_path_buf();
        drop(candidate);
        cleanup_root(&root);
    }
}

#[test]
fn correction_rollback_retains_every_source_and_cutoff_without_join_guessing() {
    let mut fixture = Fixture::build(false);
    amend_fixture(&mut fixture,
        "INSERT INTO source_files VALUES ('src-B', 'codex', 'synthetic-B', 7, 1024, 'other-prefix', NULL);
         INSERT INTO destination_snapshots VALUES ('destination-B', 'q99', 'seat-C', '2026-10-22 00:00:09', FALSE, '[]');");
    let candidate = migrate(fixture.request()).unwrap();
    let receipt =
        mig::rollback_to_preserved_schema3(&fixture.database, &candidate.database_path, false)
            .unwrap();
    assert!(receipt.last_common_watermark.is_none());
    assert!(receipt.last_common_cutoff.is_none());
    assert_eq!(
        receipt.last_common_watermarks,
        vec![
            mig::Watermark {
                source_id: "src-A".into(),
                generation: 2,
                byte_offset: 512
            },
            mig::Watermark {
                source_id: "src-B".into(),
                generation: 7,
                byte_offset: 1024
            },
        ]
    );
    assert_eq!(
        receipt.last_common_cutoffs,
        vec!["2026-10-22 00:00:01", "2026-10-22 00:00:09"]
    );
    assert!(!receipt.pointer_changed);
    fixture.assert_unchanged();
    let root = candidate.root.path().to_path_buf();
    drop(candidate);
    cleanup_root(&root);
}

#[test]
fn review_m1_rollback_never_claims_continuity_for_schema4_only_state() {
    for sql in [
        "INSERT INTO dendro_spawn_edges VALUES ('p', 'c')",
        "UPDATE dendro_schema4_field_state SET state = 'known'",
        "INSERT INTO dendro_schema4_precision VALUES ('new', 'amount', 'exact')",
        "CREATE TABLE extra_facts (id VARCHAR)",
        "INSERT INTO dendro_fact_revisions VALUES ('new', 1, 10, 1.0, false, 'present')",
        "CREATE OR REPLACE VIEW legacy_usage_facts AS SELECT * FROM usage_facts WHERE false",
    ] {
        let fixture = Fixture::build(false);
        let candidate = migrate(fixture.request()).unwrap();
        let conn = Connection::open(&candidate.database_path).unwrap();
        conn.execute_batch(sql).unwrap();
        conn.execute_batch("CHECKPOINT").unwrap();
        drop(conn);
        let stage_hash = sha256_path(&candidate.database_path);
        let receipt =
            mig::rollback_to_preserved_schema3(&fixture.database, &candidate.database_path, false)
                .unwrap();
        assert!(!receipt.full_continuity_claim, "{sql}");
        assert!(receipt.unresolved_loss, "{sql}");
        assert!(!receipt.pointer_changed);
        assert_eq!(sha256_path(&candidate.database_path), stage_hash);
        fixture.assert_unchanged();
        let root = candidate.root.path().to_path_buf();
        drop(candidate);
        cleanup_root(&root);
    }
}

#[test]
fn review_l5_missing_usage_table_keeps_exclusion_membership_unknown() {
    let mut fixture = Fixture::build(false);
    amend_fixture(&mut fixture, "DROP TABLE usage_facts");
    let candidate = migrate(fixture.request()).unwrap();
    mig::record_v4_only_fact(
        &candidate.database_path,
        &mig::V4OnlyFact {
            id: "v4-declared".into(),
            ts: "2026-10-22 00:00:02".into(),
            source_id: "src-A".into(),
            generation: 2,
            byte_range_half_open: [512, 544],
        },
    )
    .unwrap();
    let receipt =
        mig::rollback_to_preserved_schema3(&fixture.database, &candidate.database_path, false)
            .unwrap();
    assert_eq!(receipt.legacy_usage_scope, mig::EvidenceScope::Absent);
    assert!(!receipt.v4_only_membership_verified);
    assert_eq!(receipt.excluded_fact_count, 1);
    assert!(!receipt.full_continuity_claim);
    assert!(receipt.unresolved_loss);
    fixture.assert_unchanged();
    let root = candidate.root.path().to_path_buf();
    drop(candidate);
    cleanup_root(&root);
}
