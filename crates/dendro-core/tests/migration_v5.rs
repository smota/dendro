//! A02: exclusive schema-5 backup, member digests, pointer cutover, and rollback.
use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use duckdb::{Config, Connection};
use sha2::{Digest, Sha256};

use dendro_core::migration_v5::{
    migrate_schema4_to_schema5, read_pointer, read_storage_version, require_live_schema5_path,
    rollback_to_pre_v200, MigrateV5Request, MigrationV5Error, BACKUP_DIR_NAME,
    MIGRATION_V5_TARGET_SCHEMA_VERSION,
};
use dendro_core::platform_v1::{self as platform, StorageRootChoice};

#[test]
fn exclusive_backup_pointer_cutover_and_rollback() {
    let root = platform::create_user_private_root_with_choice(
        "dendro-migration-v5-",
        &StorageRootChoice::UserProfile,
    )
    .expect("private root");
    let database = root.path().join("dendro.duckdb");
    write_schema4(&database);
    let privacy = root.path().join("dendro.duckdb.key");
    let provenance = root.path().join("dendro.duckdb.provenance");
    fs::write(&privacy, b"privacy-key-material").unwrap();
    fs::write(&provenance, b"provenance-sidecar").unwrap();
    restrict(&database);
    restrict(&privacy);
    restrict(&provenance);

    let before = fs::read(&database).unwrap();
    let before_version = read_storage_version(&database).expect("schema4 header");

    let receipt = migrate_schema4_to_schema5(MigrateV5Request {
        database: &database,
        privacy_key: &privacy,
        provenance_sidecars: std::slice::from_ref(&provenance),
        live_writer_active: false,
        stop_before_cutover: false,
    })
    .expect("schema 5 migration");

    assert_eq!(
        receipt.target_schema_version,
        MIGRATION_V5_TARGET_SCHEMA_VERSION
    );
    let backup = root.path().join(BACKUP_DIR_NAME);
    assert_eq!(mode(&backup), 0o700, "backup directory mode");
    assert!(receipt
        .members
        .iter()
        .any(|member| member.role == "wal" && member.absent));
    let main = receipt
        .members
        .iter()
        .find(|member| member.role == "main")
        .unwrap();
    let backed_main = backup.join(&main.name);
    assert_eq!(sha256(&backed_main), main.sha256.clone().unwrap());
    assert_eq!(
        fs::read(&backed_main).unwrap(),
        before,
        "backup main bytes drifted from the checkpointed source"
    );
    let key_member = receipt
        .members
        .iter()
        .find(|member| member.role == "privacy_key")
        .unwrap();
    assert_eq!(
        sha256(&backup.join(&key_member.name)),
        key_member.sha256.clone().unwrap()
    );
    assert!(receipt
        .members
        .iter()
        .any(|member| member.role == "provenance"));
    for member in receipt.members.iter().filter(|member| !member.absent) {
        assert_eq!(mode(&backup.join(&member.name)), 0o600, "{}", member.name);
    }
    assert_eq!(mode(&backup.join("receipt.json")), 0o600);

    let pointer_path = root.path().join("dendro.pointer.json");
    assert_eq!(mode(&pointer_path), 0o600);
    let pointer = read_pointer(root.path()).unwrap();
    assert_eq!(pointer.selected, "schema5-live");
    assert_eq!(pointer.live_database, "dendro.duckdb");
    assert_eq!(pointer.live_schema_version, 5);
    assert_eq!(pointer.storage_version, 69);
    assert_eq!(pointer.backup_set_sha256, receipt.backup_set_sha256);
    assert_eq!(pointer.backup_directory, BACKUP_DIR_NAME);
    assert!(
        !root.path().join("dendro.v200.duckdb").exists(),
        "cutover left dendro.v200.duckdb beside the live database"
    );
    assert!(!root.path().join("dendro.v200.duckdb.wal").exists());
    assert!(!root.path().join(".dendro.v200.replacing").exists());

    let live_version = read_storage_version(&database).unwrap_or_else(|error| {
        panic!(
            "live header {error}: {}",
            hex::encode(
                &fs::read(&database).unwrap()
                    [..64.min(fs::metadata(&database).unwrap().len() as usize)]
            )
        )
    });
    let backup_version = read_storage_version(&backed_main).unwrap_or_else(|error| {
        panic!(
            "backup header {error}: {}",
            hex::encode(&fs::read(&backed_main).unwrap()[..64])
        )
    });
    assert_eq!(
        backup_version, before_version,
        "backup storage version changed"
    );
    assert_eq!(
        backup_version, 68,
        "backup storage version {backup_version}"
    );
    assert_eq!(live_version, 69, "cutover storage version {live_version}");

    let schema = query_i64(&database, "SELECT schema_version FROM dendro_meta");
    assert_eq!(schema, 5);
    for table in [
        "harness_envelope",
        "harness_trajectory",
        "_dendro_pricing_tiers",
        "_dendro_tripwire_outbox",
    ] {
        let count = query_i64(
            &database,
            &format!(
                "SELECT count(*)::BIGINT FROM information_schema.tables WHERE table_name = '{table}'"
            ),
        );
        assert_eq!(count, 1, "{table} missing");
    }
    assert!(require_live_schema5_path(&backed_main).is_err());

    let backup_bytes = backup_snapshot(&backup);
    let rerun = migrate_schema4_to_schema5(MigrateV5Request {
        database: &database,
        privacy_key: &privacy,
        provenance_sidecars: std::slice::from_ref(&provenance),
        live_writer_active: false,
        stop_before_cutover: false,
    });
    assert!(matches!(rerun, Err(MigrationV5Error::BackupExists)));
    assert_eq!(backup_snapshot(&backup), backup_bytes);

    let live_before_lock = fs::read(&database).unwrap();
    for name in [
        "dendro.sock",
        "quack.sock",
        "dendro.lock",
        "dendro.duckdb.lock",
    ] {
        let lock = root.path().join(name);
        fs::write(&lock, b"serve").unwrap();
        let refused =
            rollback_to_pre_v200(&database, false).expect_err("lock must refuse rollback");
        assert!(
            matches!(refused, MigrationV5Error::LiveWriter),
            "{name}: {refused}"
        );
        assert_eq!(
            fs::read(&database).unwrap(),
            live_before_lock,
            "rollback replaced the schema-5 database while {name} was present"
        );
        assert_eq!(read_storage_version(&database).unwrap(), 69);
        assert_eq!(
            query_i64(&database, "SELECT schema_version FROM dendro_meta"),
            5
        );
        fs::remove_file(&lock).unwrap();
        assert!(!lock.exists(), "{name} left behind");
    }

    let backup_db = root.path().join(BACKUP_DIR_NAME).join("dendro.duckdb");
    assert_eq!(backup_db, backed_main);
    let mut backup_file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&backup_db)
        .unwrap();
    let mut byte = [0u8; 1];
    backup_file.read_exact(&mut byte).unwrap();
    let original_byte = byte[0];
    byte[0] ^= 0xff;
    backup_file.seek(SeekFrom::Start(0)).unwrap();
    backup_file.write_all(&byte).unwrap();
    backup_file.sync_all().unwrap();
    drop(backup_file);
    let refused =
        rollback_to_pre_v200(&database, false).expect_err("tampered backup must refuse rollback");
    assert!(
        matches!(
            &refused,
            MigrationV5Error::Schema(text) if text == "backup member dendro.duckdb sha256 mismatch"
        ),
        "{refused}"
    );
    assert_eq!(
        fs::read(&database).unwrap(),
        live_before_lock,
        "digest refusal replaced the live database"
    );
    let mut backup_file = fs::OpenOptions::new().write(true).open(&backup_db).unwrap();
    byte[0] = original_byte;
    backup_file.seek(SeekFrom::Start(0)).unwrap();
    backup_file.write_all(&byte).unwrap();
    backup_file.sync_all().unwrap();
    drop(backup_file);
    assert_eq!(sha256(&backup_db), main.sha256.clone().unwrap());

    let loss = rollback_to_pre_v200(&database, false).expect("rollback");
    assert!(loss
        .excluded_schema5_counts
        .contains_key("harness_envelope"));
    assert!(!loss.source_logs_can_replay);
    assert_eq!(
        backup_snapshot(&backup),
        backup_bytes,
        "rollback rewrote the backup"
    );
    let rolled = read_pointer(root.path()).unwrap();
    assert_eq!(rolled.selected, "schema4-live");
    assert_eq!(rolled.live_database, "dendro.duckdb");
    assert_eq!(rolled.live_schema_version, 4);
    assert_eq!(rolled.storage_version, 68);
    assert!(rolled.loss.is_some());
    assert!(database.is_file(), "rollback removed the live database");
    assert_eq!(
        query_i64(&database, "SELECT schema_version FROM dendro_meta"),
        4
    );
    assert_eq!(read_storage_version(&database).unwrap(), 68);
    assert_eq!(read_storage_version(&backed_main).unwrap(), 68);
    assert!(!root.path().join("dendro.v200.duckdb").exists());
    assert!(!root.path().join("dendro.v200.duckdb.wal").exists());
    assert!(!root.path().join(".dendro.v200.replacing").exists());
    drop(root);
}

#[test]
fn live_writer_with_dirty_wal_does_not_create_a_backup() {
    let root = platform::create_user_private_root_with_choice(
        "dendro-migration-v5-wal-",
        &StorageRootChoice::UserProfile,
    )
    .expect("private root");
    let database = root.path().join("dendro.duckdb");
    write_schema4(&database);
    let privacy = root.path().join("dendro.duckdb.key");
    fs::write(&privacy, b"privacy-key-material").unwrap();
    restrict(&database);
    restrict(&privacy);
    append_dirty_row(&database);
    let error = migrate_schema4_to_schema5(MigrateV5Request {
        database: &database,
        privacy_key: &privacy,
        provenance_sidecars: &[],
        live_writer_active: true,
        stop_before_cutover: false,
    })
    .expect_err("live writer must refuse");
    assert!(
        matches!(
            error,
            MigrationV5Error::UncheckpointedDirtyWal | MigrationV5Error::LiveWriter
        ),
        "{error}"
    );
    assert!(!root.path().join(BACKUP_DIR_NAME).exists());
    drop(root);
}

fn write_schema4(path: &Path) {
    let config = Config::default()
        .with("storage_compatibility_version", "v1.5.6")
        .expect("v1.5.6 compatibility");
    let conn = Connection::open_with_flags(path, config).expect("create schema4");
    conn.execute_batch(
        "CREATE TABLE dendro_meta (schema_version INTEGER NOT NULL, key_id VARCHAR NOT NULL);
         INSERT INTO dendro_meta VALUES (4, 'migration-v5');
         CHECKPOINT;",
    )
    .unwrap();
}

fn append_dirty_row(path: &Path) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS kept (id INTEGER);
         INSERT INTO kept VALUES (1);",
    )
    .unwrap();
}

fn query_i64(path: &Path, sql: &str) -> i64 {
    let conn = Connection::open(path).unwrap();
    conn.query_row(sql, [], |row| row.get(0)).unwrap()
}

fn sha256(path: &Path) -> String {
    hex::encode(Sha256::digest(fs::read(path).unwrap()))
}

fn backup_snapshot(directory: &Path) -> Vec<(String, Vec<u8>)> {
    let mut files = Vec::new();
    for entry in fs::read_dir(directory).unwrap() {
        let entry = entry.unwrap();
        if entry.path().is_file() {
            files.push((
                entry.file_name().to_string_lossy().into_owned(),
                fs::read(entry.path()).unwrap(),
            ));
        }
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));
    files
}

#[cfg(unix)]
fn mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[cfg(unix)]
fn restrict(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

#[cfg(not(unix))]
fn mode(_path: &Path) -> u32 {
    0o600
}

#[cfg(not(unix))]
fn restrict(_path: &Path) {}
