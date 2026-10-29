//! Schema 4 → schema 5 cutover.
//! The backup directory `dendro.duckdb.pre-v200.bak` is created exclusively
//! (`DirBuilder` / `mkdir` EEXIST, the same refusal as `O_CREAT | O_EXCL`).
//! Rollback restores that snapshot onto the live database through
//! `dendro.pointer.json` and does not rewrite backup bytes.

mod envelope;

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use duckdb::Connection;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub use envelope::{
    envelope_field, envelope_stored_text, envelope_variant_keys, write_harness_envelope,
    EnvelopeError,
};

pub const MIGRATION_V5_TARGET_SCHEMA_VERSION: i32 = 5;
pub const BACKUP_DIR_NAME: &str = "dendro.duckdb.pre-v200.bak";
pub const POINTER_NAME: &str = "dendro.pointer.json";
pub const STORAGE_VERSION_V68: u64 = 68;
pub const STORAGE_VERSION_V69: u64 = 69;

const SCHEMA5_DDL: &str = "
CREATE TABLE IF NOT EXISTS harness_envelope (
    observation_id VARCHAR PRIMARY KEY,
    harness VARCHAR NOT NULL,
    harness_version VARCHAR NOT NULL,
    captured_at TIMESTAMP NOT NULL,
    envelope_shape VARIANT
);
CREATE TABLE IF NOT EXISTS _dendro_pricing_tiers (
    tier_id VARCHAR PRIMARY KEY,
    profile_pin VARCHAR NOT NULL,
    cohort_pin VARCHAR NOT NULL,
    currency VARCHAR NOT NULL,
    input_rate DOUBLE,
    output_rate DOUBLE
);
";

/// Staged DuckDB 2.0 file. Cutover upgrades this copy, then renames it onto
/// the live database. A successful migration does not leave it beside live.
pub const STAGED_V200_NAME: &str = "dendro.v200.duckdb";
const REPLACE_TEMP_NAME: &str = ".dendro.v200.replacing";
const RESTORE_TEMP_NAME: &str = ".dendro.schema4.restoring";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrationV5Error {
    UncheckpointedDirtyWal,
    LiveWriter,
    BackupExists,
    BackupIsNotLive,
    PointerIsDirectory,
    MissingMember(String),
    InterruptedBeforeCutover,
    Schema(String),
    Io(String),
    Duck(String),
}

impl std::fmt::Display for MigrationV5Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UncheckpointedDirtyWal => {
                write!(f, "uncheckpointed dirty WAL; backup was not created")
            }
            Self::LiveWriter => write!(f, "live writer is active; nothing was copied"),
            Self::BackupExists => write!(f, "{BACKUP_DIR_NAME} already exists"),
            Self::BackupIsNotLive => {
                write!(f, "backup directory is not the live schema-5 database")
            }
            Self::PointerIsDirectory => write!(f, "{POINTER_NAME} is a directory"),
            Self::MissingMember(name) => write!(f, "backup member missing: {name}"),
            Self::InterruptedBeforeCutover => {
                write!(f, "migration stopped before schema-5 cutover")
            }
            Self::Schema(text) | Self::Io(text) | Self::Duck(text) => write!(f, "{text}"),
        }
    }
}

impl std::error::Error for MigrationV5Error {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberDigest {
    pub role: String,
    pub name: String,
    pub sha256: Option<String>,
    pub absent: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LossReceipt {
    pub watermark: String,
    pub excluded_schema5_counts: std::collections::BTreeMap<String, i64>,
    pub source_logs_can_replay: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationV5Receipt {
    pub target_schema_version: i32,
    pub storage_version: u64,
    pub backup_directory: String,
    pub members: Vec<MemberDigest>,
    pub backup_set_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PointerManifest {
    pub manifest: String,
    pub selected: String,
    pub live_database: String,
    pub live_schema_version: i32,
    pub storage_version: u64,
    pub backup_directory: String,
    pub backup_set_sha256: String,
    pub members: Vec<MemberDigest>,
    /// SHA-256 of every backup member. Rollback re-reads each file.
    #[serde(default)]
    pub backup_member_digests: Vec<MemberDigest>,
    pub receipt: MigrationV5Receipt,
    pub loss: Option<LossReceipt>,
}

pub struct MigrateV5Request<'a> {
    pub database: &'a Path,
    pub privacy_key: &'a Path,
    pub provenance_sidecars: &'a [PathBuf],
    pub live_writer_active: bool,
    pub stop_before_cutover: bool,
}

pub fn migrate_schema4_to_schema5(
    request: MigrateV5Request<'_>,
) -> Result<MigrationV5Receipt, MigrationV5Error> {
    let database = request
        .database
        .canonicalize()
        .map_err(|error| MigrationV5Error::Io(error.to_string()))?;
    let parent = database
        .parent()
        .ok_or_else(|| MigrationV5Error::Io("database parent absent".into()))?
        .to_path_buf();
    let backup = parent.join(BACKUP_DIR_NAME);
    if backup.exists() {
        return Err(MigrationV5Error::BackupExists);
    }
    if request.live_writer_active {
        if wal_is_dirty(&database) {
            return Err(MigrationV5Error::UncheckpointedDirtyWal);
        }
        return Err(MigrationV5Error::LiveWriter);
    }
    checkpoint_preserving(&database)?;
    if wal_is_dirty(&database) {
        return Err(MigrationV5Error::UncheckpointedDirtyWal);
    }
    if !request.privacy_key.is_file() {
        return Err(MigrationV5Error::MissingMember("privacy_key".into()));
    }
    let provenance = provenance_members(&database, request.provenance_sidecars)?;
    create_backup_dir(&backup)?;
    let members = match copy_backup_set(&database, request.privacy_key, &provenance, &backup) {
        Ok(members) => members,
        Err(error) => {
            let _ = fs::remove_dir_all(&backup);
            return Err(error);
        }
    };
    let backup_set_sha256 = backup_set_digest(&members);
    let pre_version = read_storage_version(&database)?;
    let live_hash = sha256_file(&database)?;
    let receipt = MigrationV5Receipt {
        target_schema_version: MIGRATION_V5_TARGET_SCHEMA_VERSION,
        storage_version: pre_version,
        backup_directory: BACKUP_DIR_NAME.to_string(),
        members: members.clone(),
        backup_set_sha256: backup_set_sha256.clone(),
    };
    let receipt_bytes = serde_json::to_vec_pretty(&receipt)
        .map_err(|error| MigrationV5Error::Io(error.to_string()))?;
    write_exclusive(&backup.join("receipt.json"), &receipt_bytes)?;
    let mut backup_member_digests = members.clone();
    backup_member_digests.push(MemberDigest {
        role: "receipt".to_string(),
        name: "receipt.json".to_string(),
        sha256: Some(hex::encode(Sha256::digest(&receipt_bytes))),
        absent: false,
    });
    let live_name = database
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| MigrationV5Error::Io("database name is not unicode".into()))?
        .to_string();
    let schema4_pointer = PointerManifest {
        manifest: "dendro-pointer-v1".to_string(),
        selected: "schema4-live".to_string(),
        live_database: live_name.clone(),
        live_schema_version: 4,
        storage_version: pre_version,
        backup_directory: BACKUP_DIR_NAME.to_string(),
        backup_set_sha256: backup_set_sha256.clone(),
        members: members.clone(),
        backup_member_digests,
        receipt: receipt.clone(),
        loss: None,
    };
    write_pointer(&parent, &schema4_pointer)?;
    if request.stop_before_cutover {
        return Err(MigrationV5Error::InterruptedBeforeCutover);
    }
    let staged = stage_v200(&database)?;
    if let Err(error) = upgrade_staged(&staged) {
        let _ = fs::remove_file(&staged);
        let _ = fs::remove_file(wal_path(&staged));
        return Err(error);
    }
    if sha256_file(&database)? != live_hash || read_storage_version(&database)? != pre_version {
        return Err(MigrationV5Error::Schema(
            "schema4-live changed before the pointer flip".into(),
        ));
    }
    let storage_version = read_storage_version(&staged)?;
    if storage_version != STORAGE_VERSION_V69 {
        return Err(MigrationV5Error::Schema(format!(
            "staged storage version {storage_version} is not {STORAGE_VERSION_V69}"
        )));
    }
    verify_schema_version(&staged, MIGRATION_V5_TARGET_SCHEMA_VERSION)?;
    let receipt = MigrationV5Receipt {
        storage_version,
        ..receipt
    };
    publish_verified_over_live(&database, &staged)?;
    let pointer = PointerManifest {
        selected: "schema5-live".to_string(),
        live_database: live_name,
        live_schema_version: MIGRATION_V5_TARGET_SCHEMA_VERSION,
        storage_version,
        receipt: receipt.clone(),
        ..schema4_pointer
    };
    write_pointer(&parent, &pointer)?;
    Ok(receipt)
}

pub fn rollback_schema5(database: &Path) -> Result<LossReceipt, MigrationV5Error> {
    rollback_to_pre_v200(database, false)
}

/// Restore the pre-v200 snapshot onto the live database.
/// Every recorded backup digest is re-hashed first, and every regular file in
/// the backup directory must be one of those digests.
/// `has_active_writer` is checked along with `dendro.duckdb.lock`, `quack.sock`,
/// `dendro.sock`, and `dendro.lock` on disk. Backup bytes are not rewritten.
pub fn rollback_to_pre_v200(
    database: &Path,
    has_active_writer: bool,
) -> Result<LossReceipt, MigrationV5Error> {
    let database = database
        .canonicalize()
        .map_err(|error| MigrationV5Error::Io(error.to_string()))?;
    if has_active_writer || active_writer_locks(&database)? {
        return Err(MigrationV5Error::LiveWriter);
    }
    require_live_schema5_path(&database)?;
    let parent = database
        .parent()
        .ok_or_else(|| MigrationV5Error::Io("database parent absent".into()))?;
    let backup = parent.join(BACKUP_DIR_NAME);
    let before = directory_digests(&backup)?;
    let mut pointer = read_pointer(parent)?;
    if pointer.backup_set_sha256 != backup_set_digest(&pointer.members) {
        return Err(MigrationV5Error::Schema(
            "pointer backup digest does not match its members".into(),
        ));
    }
    let digests = if pointer.backup_member_digests.is_empty() {
        pointer.members.clone()
    } else {
        pointer.backup_member_digests.clone()
    };
    verify_backup_member_digests(parent, &digests)?;
    let loss = loss_receipt(&database)?;
    let live_name = database
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| MigrationV5Error::Io("database name is not unicode".into()))?
        .to_string();
    let snapshot = backup.join(&live_name);
    if !snapshot.is_file() {
        return Err(MigrationV5Error::MissingMember(live_name.clone()));
    }
    replace_with_snapshot(&snapshot, &database, &parent.join(RESTORE_TEMP_NAME))?;
    fsync_path(parent)?;
    remove_live_wal_not_in_snapshot(&database, &digests)?;
    let storage_version = read_storage_version(&database)?;
    pointer.selected = "schema4-live".to_string();
    pointer.live_database = live_name;
    pointer.live_schema_version = 4;
    pointer.storage_version = storage_version;
    pointer.loss = Some(loss.clone());
    write_pointer(parent, &pointer)?;
    remove_split_brain_files(parent)?;
    fsync_path(parent)?;
    let after = directory_digests(&backup)?;
    if before != after {
        return Err(MigrationV5Error::Schema(
            "rollback changed backup bytes".into(),
        ));
    }
    Ok(loss)
}

pub fn read_pointer(directory: &Path) -> Result<PointerManifest, MigrationV5Error> {
    let path = directory.join(POINTER_NAME);
    if path.is_dir() {
        return Err(MigrationV5Error::PointerIsDirectory);
    }
    let bytes = fs::read(&path).map_err(|error| MigrationV5Error::Io(error.to_string()))?;
    serde_json::from_slice(&bytes).map_err(|error| MigrationV5Error::Io(error.to_string()))
}

pub fn require_live_schema5_path(path: &Path) -> Result<(), MigrationV5Error> {
    if path
        .components()
        .any(|component| component.as_os_str() == std::ffi::OsStr::new(BACKUP_DIR_NAME))
    {
        return Err(MigrationV5Error::BackupIsNotLive);
    }
    Ok(())
}

/// Storage version from the DuckDB file header.
///
/// Only three 64-byte windows are read: the main header and the database
/// headers at byte offsets 4096 and 8192. Bytes `0..8` are the block checksum.
/// Magic `DUCK` is bytes `8..12`. The storage-version `u64` is bytes `12..20`
/// (little-endian). A v1.5.6 file stores `68` there. DuckDB 2.0 writes storage
/// version `69` into the higher-iteration database header and may write `999`
/// in the main header for files created at v2. A missing file, a short header,
/// or any other magic is an error. There is no default version.
pub fn read_storage_version(path: &Path) -> Result<u64, MigrationV5Error> {
    let mut file = File::open(path).map_err(|error| MigrationV5Error::Io(error.to_string()))?;
    let mut window = [0u8; 64];
    file.seek(SeekFrom::Start(0))
        .map_err(|error| MigrationV5Error::Io(error.to_string()))?;
    if let Err(error) = file.read_exact(&mut window) {
        if error.kind() == std::io::ErrorKind::UnexpectedEof {
            return Err(MigrationV5Error::Schema(format!(
                "{} is not a DuckDB header (magic DUCK missing at bytes 8..12)",
                path.display()
            )));
        }
        return Err(MigrationV5Error::Io(error.to_string()));
    }
    if window.get(8..12) != Some(b"DUCK".as_slice()) {
        return Err(MigrationV5Error::Schema(format!(
            "{} is not a DuckDB header (magic DUCK missing at bytes 8..12)",
            path.display()
        )));
    }
    let main = read_u64(&window, 12).ok_or_else(|| {
        MigrationV5Error::Schema(format!(
            "{} header has no storage version uint64 at bytes 12..20",
            path.display()
        ))
    })?;
    let first = read_block_header(&mut file, 4096, &mut window)?;
    let second = read_block_header(&mut file, 8192, &mut window)?;
    let header = active_database_storage_version(first, second);
    if let Some(version) = header {
        if (69..=80).contains(&version) {
            return Ok(version);
        }
    }
    if main == 999 {
        return header.filter(|version| *version == 69).ok_or_else(|| {
            MigrationV5Error::Schema(format!(
                "{} v2 header is missing storage version 69",
                path.display()
            ))
        });
    }
    if (64..=80).contains(&main) {
        return Ok(main);
    }
    Err(MigrationV5Error::Schema(format!(
        "{} has unrecognized DuckDB storage version {main}",
        path.display()
    )))
}

fn read_block_header(
    file: &mut File,
    block: u64,
    window: &mut [u8; 64],
) -> Result<Option<(u64, u64)>, MigrationV5Error> {
    file.seek(SeekFrom::Start(block))
        .map_err(|error| MigrationV5Error::Io(error.to_string()))?;
    match file.read_exact(window) {
        // `window` starts at `block`, so the header offsets are relative to it.
        Ok(()) => Ok(database_header(window, 0)),
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => Ok(None),
        Err(error) => Err(MigrationV5Error::Io(error.to_string())),
    }
}

fn active_database_storage_version(
    first: Option<(u64, u64)>,
    second: Option<(u64, u64)>,
) -> Option<u64> {
    let chosen = match (first, second) {
        (Some(left), Some(right)) => {
            if left.0 != right.0 {
                if left.0 > right.0 {
                    left
                } else {
                    right
                }
            } else if left.1 >= 69 {
                left
            } else {
                right
            }
        }
        (Some(only), None) | (None, Some(only)) => only,
        (None, None) => return None,
    };
    Some(chosen.1)
}

/// `(iteration, storage_or_serialization_version)` from one database header.
/// `bytes` begins at the header block. The iteration `u64` is at `block + 8`
/// and the version `u64` is at `block + 8 + 48` (offset 56 within the block
/// when `block` is 0).
fn database_header(bytes: &[u8], block: usize) -> Option<(u64, u64)> {
    let base = block + 8;
    Some((read_u64(bytes, base)?, read_u64(bytes, base + 48)?))
}

fn read_u64(bytes: &[u8], offset: usize) -> Option<u64> {
    let slice = bytes.get(offset..offset + 8)?;
    Some(u64::from_le_bytes(slice.try_into().ok()?))
}

pub fn apply_schema5(conn: &Connection) -> Result<(), MigrationV5Error> {
    conn.execute_batch(SCHEMA5_DDL)
        .map_err(|error| MigrationV5Error::Duck(error.to_string()))?;
    conn.execute_batch(crate::retrieval_v1::TRAJECTORY_SQL)
        .map_err(|error| MigrationV5Error::Duck(error.to_string()))?;
    crate::tripwire_v1::ensure_tripwire_tables(conn)
        .map_err(|error| MigrationV5Error::Duck(error.to_string()))?;
    crate::tripwire_v1::install_trajectory_tripwire(conn)
        .map_err(|error| MigrationV5Error::Duck(error.to_string()))?;
    let has_meta: i64 = conn
        .query_row(
            "SELECT count(*) FROM information_schema.tables WHERE table_name = 'dendro_meta'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| MigrationV5Error::Duck(error.to_string()))?;
    if has_meta == 0 {
        conn.execute_batch(
            "CREATE TABLE dendro_meta (schema_version INTEGER NOT NULL, key_id VARCHAR NOT NULL);
             INSERT INTO dendro_meta VALUES (5, 'schema5');",
        )
        .map_err(|error| MigrationV5Error::Duck(error.to_string()))?;
    } else {
        conn.execute(
            "UPDATE dendro_meta SET schema_version = 5 WHERE schema_version IN (3, 4)",
            [],
        )
        .map_err(|error| MigrationV5Error::Duck(error.to_string()))?;
    }
    if table_exists(conn, "usage_facts")? {
        crate::tripwire_v1::install_usage_tripwire(conn)
            .map_err(|error| MigrationV5Error::Duck(error.to_string()))?;
    }
    Ok(())
}

fn stage_v200(live: &Path) -> Result<PathBuf, MigrationV5Error> {
    let staged = staged_v200_path(live)?;
    if staged.exists() {
        fs::remove_file(&staged).map_err(|error| MigrationV5Error::Io(error.to_string()))?;
    }
    fs::copy(live, &staged).map_err(|error| MigrationV5Error::Io(error.to_string()))?;
    #[cfg(unix)]
    set_mode(&staged, 0o600)?;
    Ok(staged)
}

fn staged_v200_path(live: &Path) -> Result<PathBuf, MigrationV5Error> {
    let parent = live
        .parent()
        .ok_or_else(|| MigrationV5Error::Io("database parent absent".into()))?;
    let staged = parent.join(STAGED_V200_NAME);
    if staged == live {
        return Err(MigrationV5Error::Io(
            "staged v200 path aliases the live database".into(),
        ));
    }
    Ok(staged)
}

fn upgrade_staged(staged: &Path) -> Result<(), MigrationV5Error> {
    apply_cutover(staged)
}

fn publish_verified_over_live(live: &Path, staged: &Path) -> Result<(), MigrationV5Error> {
    let parent = live
        .parent()
        .ok_or_else(|| MigrationV5Error::Io("database parent absent".into()))?;
    fsync_path(staged)?;
    fs::rename(staged, live).map_err(|error| MigrationV5Error::Io(error.to_string()))?;
    #[cfg(unix)]
    set_mode(live, 0o600)?;
    remove_if_present(&wal_path(staged))?;
    remove_if_present(&parent.join(REPLACE_TEMP_NAME))?;
    fsync_path(parent)?;
    Ok(())
}

fn verify_schema_version(path: &Path, expected: i32) -> Result<(), MigrationV5Error> {
    let conn = open_live_read(path)?;
    verify_open_connection(&conn, path)?;
    let schema: i32 = conn
        .query_row("SELECT schema_version FROM dendro_meta", [], |row| {
            row.get(0)
        })
        .map_err(|error| MigrationV5Error::Duck(error.to_string()))?;
    if schema != expected {
        return Err(MigrationV5Error::Schema(format!(
            "schema_version {schema} is not {expected}"
        )));
    }
    Ok(())
}

/// Header version is necessary but not sufficient. The open connection must
/// answer `pragma_version`, `duckdb_databases()`, and `pragma_database_list()`.
fn verify_open_connection(conn: &Connection, database: &Path) -> Result<(), MigrationV5Error> {
    let library: String = conn
        .query_row("SELECT library_version FROM pragma_version()", [], |row| {
            row.get(0)
        })
        .map_err(|error| MigrationV5Error::Duck(error.to_string()))?;
    if library.trim().is_empty() {
        return Err(MigrationV5Error::Schema(
            "pragma_version() returned an empty library version".into(),
        ));
    }
    let databases: i64 = conn
        .query_row("SELECT count(*) FROM duckdb_databases()", [], |row| {
            row.get(0)
        })
        .map_err(|error| MigrationV5Error::Duck(error.to_string()))?;
    if databases < 1 {
        return Err(MigrationV5Error::Schema(
            "duckdb_databases() returned no rows".into(),
        ));
    }
    // This DuckDB build exposes `PRAGMA database_list` and `duckdb_databases()`,
    // not the `pragma_database_list()` table function.
    conn.execute_batch("PRAGMA database_list")
        .map_err(|error| MigrationV5Error::Duck(error.to_string()))?;
    let sized: i64 = conn
        .query_row("SELECT count(*) FROM pragma_database_size()", [], |row| {
            row.get(0)
        })
        .map_err(|error| MigrationV5Error::Duck(error.to_string()))?;
    if sized < 1 {
        return Err(MigrationV5Error::Schema(
            "pragma_database_size() returned no rows".into(),
        ));
    }
    let name = database
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| MigrationV5Error::Io("database name is not unicode".into()))?;
    let attached: i64 = conn
        .query_row(
            "SELECT count(*) FROM duckdb_databases()
             WHERE path IS NOT NULL AND contains(path, ?)",
            [name],
            |row| row.get(0),
        )
        .map_err(|error| MigrationV5Error::Duck(error.to_string()))?;
    if attached < 1 {
        return Err(MigrationV5Error::Schema(format!(
            "open connection is not attached to {}",
            database.display()
        )));
    }
    Ok(())
}

fn verify_backup_member_digests(
    parent: &Path,
    digests: &[MemberDigest],
) -> Result<(), MigrationV5Error> {
    let backup = parent.join(BACKUP_DIR_NAME);
    for member in digests {
        if member.absent {
            continue;
        }
        let expected = member.sha256.as_deref().ok_or_else(|| {
            MigrationV5Error::Schema(format!(
                "backup member {} has no recorded sha256 digest",
                member.name
            ))
        })?;
        let path = backup.join(&member.name);
        if !path.is_file() {
            return Err(MigrationV5Error::MissingMember(member.name.clone()));
        }
        let actual = sha256_file(&path)?;
        if !actual.eq_ignore_ascii_case(expected) {
            return Err(MigrationV5Error::Schema(format!(
                "backup member {} sha256 mismatch",
                member.name
            )));
        }
    }
    let recorded: std::collections::BTreeSet<&str> = digests
        .iter()
        .filter(|member| !member.absent)
        .map(|member| member.name.as_str())
        .collect();
    let entries = fs::read_dir(&backup).map_err(|error| MigrationV5Error::Io(error.to_string()))?;
    for entry in entries {
        let entry = entry.map_err(|error| MigrationV5Error::Io(error.to_string()))?;
        let file_type = entry
            .file_type()
            .map_err(|error| MigrationV5Error::Io(error.to_string()))?;
        if !file_type.is_file() {
            continue;
        }
        let name = entry.file_name();
        let name = name
            .to_str()
            .ok_or_else(|| MigrationV5Error::Io("backup file name is not unicode".into()))?;
        if !recorded.contains(name) {
            return Err(MigrationV5Error::Schema(format!(
                "backup file {name} has no recorded digest"
            )));
        }
    }
    Ok(())
}

fn active_writer_locks(database: &Path) -> Result<bool, MigrationV5Error> {
    let mut paths = vec![database.with_extension("duckdb.lock")];
    if let Some(parent) = database.parent() {
        paths.push(parent.join("quack.sock"));
        paths.push(parent.join("dendro.sock"));
        paths.push(parent.join("dendro.lock"));
    }
    Ok(paths.iter().any(|path| path.exists()))
}

fn sha256_file(path: &Path) -> Result<String, MigrationV5Error> {
    let bytes = fs::read(path).map_err(|error| MigrationV5Error::Io(error.to_string()))?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

fn apply_cutover(database: &Path) -> Result<(), MigrationV5Error> {
    let conn = open_cutover(database)?;
    let version: i32 = conn
        .query_row("SELECT schema_version FROM dendro_meta", [], |row| {
            row.get(0)
        })
        .map_err(|error| MigrationV5Error::Duck(error.to_string()))?;
    if version != 4 {
        return Err(MigrationV5Error::Schema(format!(
            "refusing cutover from schema {version}"
        )));
    }
    apply_schema5(&conn)?;
    verify_open_connection(&conn, database)?;
    let schema: i32 = conn
        .query_row("SELECT schema_version FROM dendro_meta", [], |row| {
            row.get(0)
        })
        .map_err(|error| MigrationV5Error::Duck(error.to_string()))?;
    if schema != MIGRATION_V5_TARGET_SCHEMA_VERSION {
        return Err(MigrationV5Error::Schema(format!(
            "staged schema_version {schema} is not {MIGRATION_V5_TARGET_SCHEMA_VERSION}"
        )));
    }
    conn.execute_batch("CHECKPOINT dendro_cutover")
        .or_else(|_| conn.execute_batch("CHECKPOINT"))
        .map_err(|error| MigrationV5Error::Duck(error.to_string()))?;
    drop(conn);
    let wal = wal_path(database);
    if wal.is_file() {
        #[cfg(unix)]
        set_mode(&wal, 0o600)?;
        if fs::metadata(&wal)
            .map(|meta| meta.len() == 0)
            .unwrap_or(false)
        {
            let _ = fs::remove_file(&wal);
        }
    }
    Ok(())
}

fn table_exists(conn: &Connection, name: &str) -> Result<bool, MigrationV5Error> {
    let count: i64 = conn
        .query_row(
            "SELECT count(*) FROM information_schema.tables WHERE table_name = ?",
            [name],
            |row| row.get(0),
        )
        .map_err(|error| MigrationV5Error::Duck(error.to_string()))?;
    Ok(count > 0)
}

fn loss_receipt(database: &Path) -> Result<LossReceipt, MigrationV5Error> {
    let conn = open_live_read(database)?;
    let mut counts = std::collections::BTreeMap::new();
    for table in [
        "harness_envelope",
        "_dendro_tripwire_outbox",
        "harness_trajectory",
        "_dendro_pricing_tiers",
    ] {
        let count = if table_exists(&conn, table)? {
            conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .map_err(|error| MigrationV5Error::Duck(error.to_string()))?
        } else {
            0
        };
        counts.insert(table.to_string(), count);
    }
    let watermark = if table_exists(&conn, "_dendro_tripwire_watermark")? {
        conn.query_row(
            "SELECT watermark FROM _dendro_tripwire_watermark WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .unwrap_or_else(|_| "absent".to_string())
    } else {
        "absent".to_string()
    };
    Ok(LossReceipt {
        watermark,
        excluded_schema5_counts: counts,
        source_logs_can_replay: false,
    })
}

fn provenance_members(
    database: &Path,
    explicit: &[PathBuf],
) -> Result<Vec<PathBuf>, MigrationV5Error> {
    let mut found = Vec::new();
    // Compare canonical identities while retaining source paths for backup names.
    let mut identities = std::collections::HashSet::new();
    if let Some(parent) = database.parent() {
        let db_name = database
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        let prefix = format!("{db_name}.provenance");
        if let Ok(entries) = fs::read_dir(parent) {
            for entry in entries.flatten() {
                let path = entry.path();
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if path.is_file() && (name.starts_with(&prefix) || name.ends_with(".provenance")) {
                    let identity = path
                        .canonicalize()
                        .map_err(|error| MigrationV5Error::Io(error.to_string()))?;
                    if identities.insert(identity) {
                        found.push(path);
                    }
                }
            }
        }
    }
    for path in explicit {
        if !path.is_file() {
            return Err(MigrationV5Error::MissingMember(
                path.file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("provenance")
                    .to_string(),
            ));
        }
        let identity = path
            .canonicalize()
            .map_err(|error| MigrationV5Error::Io(error.to_string()))?;
        if identities.insert(identity) {
            found.push(path.clone());
        }
    }
    found.sort();
    Ok(found)
}

#[cfg(test)]
mod provenance_tests {
    use super::*;
    use crate::platform_v1::{create_user_private_root_with_choice, StorageRootChoice};

    #[test]
    fn discovered_and_explicit_aliases_are_one_member() {
        let root = create_user_private_root_with_choice(
            "dendro-provenance-alias-",
            &StorageRootChoice::UserProfile,
        )
        .unwrap();
        let database = root.path().join("dendro.duckdb");
        let sidecar = root.path().join("dendro.duckdb.provenance");
        fs::write(&database, b"database").unwrap();
        fs::write(&sidecar, b"provenance").unwrap();
        fs::create_dir(root.path().join("nested")).unwrap();
        let alias = root.path().join("nested/../dendro.duckdb.provenance");
        let database = database.canonicalize().unwrap();

        let members = provenance_members(&database, &[sidecar, alias]).unwrap();

        assert_eq!(
            members,
            vec![database.parent().unwrap().join("dendro.duckdb.provenance")]
        );
    }

    #[cfg(unix)]
    #[test]
    fn explicit_symlink_alias_keeps_its_copy_name() {
        let root = create_user_private_root_with_choice(
            "dendro-provenance-symlink-",
            &StorageRootChoice::UserProfile,
        )
        .unwrap();
        let target = root.path().join("original");
        let alias = root.path().join("alias");
        fs::write(&target, b"provenance").unwrap();
        std::os::unix::fs::symlink(&target, &alias).unwrap();

        let members =
            provenance_members(&root.path().join("dendro.duckdb"), &[alias.clone(), target])
                .unwrap();

        assert_eq!(members, vec![alias]);
    }

    #[test]
    fn missing_explicit_member_keeps_missing_member_error() {
        let root = create_user_private_root_with_choice(
            "dendro-provenance-missing-",
            &StorageRootChoice::UserProfile,
        )
        .unwrap();
        let database = root.path().join("dendro.duckdb");
        let missing = root.path().join("missing.provenance");

        assert_eq!(
            provenance_members(&database, &[missing]),
            Err(MigrationV5Error::MissingMember("missing.provenance".into()))
        );
    }

    #[test]
    fn distinct_files_with_same_basename_still_collide_exclusively() {
        let root = create_user_private_root_with_choice(
            "dendro-provenance-collision-",
            &StorageRootChoice::UserProfile,
        )
        .unwrap();
        let first_dir = root.path().join("first");
        let second_dir = root.path().join("second");
        let backup = root.path().join("backup");
        fs::create_dir(&first_dir).unwrap();
        fs::create_dir(&second_dir).unwrap();
        fs::create_dir(&backup).unwrap();
        let first = first_dir.join("same.provenance");
        let second = second_dir.join("same.provenance");
        fs::write(&first, b"first").unwrap();
        fs::write(&second, b"second").unwrap();

        let members = provenance_members(
            &root.path().join("dendro.duckdb"),
            &[first.clone(), second.clone()],
        )
        .unwrap();

        assert_eq!(members, vec![first, second]);
        copy_member(&members[0], &backup, "provenance").unwrap();
        assert!(matches!(
            copy_member(&members[1], &backup, "provenance"),
            Err(MigrationV5Error::Io(_))
        ));
        assert_eq!(fs::read(backup.join("same.provenance")).unwrap(), b"first");
    }
}

fn copy_backup_set(
    database: &Path,
    privacy_key: &Path,
    provenance: &[PathBuf],
    backup: &Path,
) -> Result<Vec<MemberDigest>, MigrationV5Error> {
    let mut members = Vec::new();
    members.push(copy_member(database, backup, "main")?);
    let wal = wal_path(database);
    if wal.is_file() {
        members.push(copy_member(&wal, backup, "wal")?);
    } else {
        members.push(MemberDigest {
            role: "wal".to_string(),
            name: "wal".to_string(),
            sha256: None,
            absent: true,
        });
    }
    members.push(copy_member(privacy_key, backup, "privacy_key")?);
    for path in provenance {
        members.push(copy_member(path, backup, "provenance")?);
    }
    Ok(members)
}

fn copy_member(source: &Path, backup: &Path, role: &str) -> Result<MemberDigest, MigrationV5Error> {
    let name = source
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| MigrationV5Error::Io(format!("{role} name is not unicode")))?;
    let bytes = fs::read(source).map_err(|error| MigrationV5Error::Io(error.to_string()))?;
    let digest = hex::encode(Sha256::digest(&bytes));
    write_exclusive(&backup.join(name), &bytes)?;
    Ok(MemberDigest {
        role: role.to_string(),
        name: name.to_string(),
        sha256: Some(digest),
        absent: false,
    })
}

fn backup_set_digest(members: &[MemberDigest]) -> String {
    let mut lines: Vec<String> = members
        .iter()
        .map(|member| {
            if member.absent {
                format!("{}\t{}\tabsent", member.role, member.name)
            } else {
                format!(
                    "{}\t{}\t{}",
                    member.role,
                    member.name,
                    member.sha256.as_deref().unwrap_or("")
                )
            }
        })
        .collect();
    lines.sort();
    hex::encode(Sha256::digest(lines.join("\n").as_bytes()))
}

fn directory_digests(directory: &Path) -> Result<Vec<(String, String)>, MigrationV5Error> {
    let mut files = Vec::new();
    let entries =
        fs::read_dir(directory).map_err(|error| MigrationV5Error::Io(error.to_string()))?;
    for entry in entries {
        let entry = entry.map_err(|error| MigrationV5Error::Io(error.to_string()))?;
        if entry.path().is_file() {
            let bytes =
                fs::read(entry.path()).map_err(|error| MigrationV5Error::Io(error.to_string()))?;
            files.push((
                entry.file_name().to_string_lossy().into_owned(),
                hex::encode(Sha256::digest(bytes)),
            ));
        }
    }
    files.sort();
    Ok(files)
}

fn create_backup_dir(path: &Path) -> Result<(), MigrationV5Error> {
    if path.symlink_metadata().is_ok() {
        return Err(MigrationV5Error::BackupExists);
    }
    let builder = fs::DirBuilder::new();
    #[cfg(unix)]
    let builder = {
        let mut builder = builder;
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
        builder
    };
    builder.create(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            MigrationV5Error::BackupExists
        } else {
            MigrationV5Error::Io(error.to_string())
        }
    })?;
    #[cfg(unix)]
    set_mode(path, 0o700)?;
    Ok(())
}

fn write_exclusive(path: &Path, bytes: &[u8]) -> Result<(), MigrationV5Error> {
    if path.exists() {
        return Err(MigrationV5Error::Io(format!(
            "{} already exists",
            path.display()
        )));
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|error| MigrationV5Error::Io(error.to_string()))?;
    file.write_all(bytes)
        .map_err(|error| MigrationV5Error::Io(error.to_string()))?;
    file.sync_all()
        .map_err(|error| MigrationV5Error::Io(error.to_string()))?;
    #[cfg(unix)]
    set_mode(path, 0o600)?;
    Ok(())
}

fn write_pointer(directory: &Path, manifest: &PointerManifest) -> Result<(), MigrationV5Error> {
    let final_path = directory.join(POINTER_NAME);
    if final_path.is_dir() {
        return Err(MigrationV5Error::PointerIsDirectory);
    }
    let temp = directory.join(format!(".{POINTER_NAME}.tmp"));
    if temp.exists() {
        fs::remove_file(&temp).map_err(|error| MigrationV5Error::Io(error.to_string()))?;
    }
    let bytes = serde_json::to_vec_pretty(manifest)
        .map_err(|error| MigrationV5Error::Io(error.to_string()))?;
    write_exclusive(&temp, &bytes)?;
    fs::rename(&temp, &final_path).map_err(|error| MigrationV5Error::Io(error.to_string()))?;
    #[cfg(unix)]
    set_mode(&final_path, 0o600)?;
    Ok(())
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> Result<(), MigrationV5Error> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .map_err(|error| MigrationV5Error::Io(error.to_string()))
}

fn wal_path(database: &Path) -> PathBuf {
    let mut name = database.as_os_str().to_owned();
    name.push(".wal");
    PathBuf::from(name)
}

/// Copy `source` to a temp file in the destination parent, fsync it, then
/// rename it over `dest`. Does not copy onto `dest` in place and does not
/// modify `source`.
fn replace_with_snapshot(source: &Path, dest: &Path, temp: &Path) -> Result<(), MigrationV5Error> {
    if temp == dest || temp == source {
        return Err(MigrationV5Error::Io(
            "restore temp aliases the live or backup database".into(),
        ));
    }
    remove_if_present(temp)?;
    fs::copy(source, temp).map_err(|error| MigrationV5Error::Io(error.to_string()))?;
    #[cfg(unix)]
    set_mode(temp, 0o600)?;
    fsync_path(temp)?;
    if let Err(error) = fs::rename(temp, dest) {
        let _ = fs::remove_file(temp);
        return Err(MigrationV5Error::Io(error.to_string()));
    }
    #[cfg(unix)]
    set_mode(dest, 0o600)?;
    Ok(())
}

fn remove_live_wal_not_in_snapshot(
    database: &Path,
    digests: &[MemberDigest],
) -> Result<(), MigrationV5Error> {
    let wal = wal_path(database);
    if wal.symlink_metadata().is_err() {
        return Ok(());
    }
    let wal_name = wal.file_name().and_then(|name| name.to_str()).unwrap_or("");
    if let Some(member) = digests
        .iter()
        .find(|member| !member.absent && member.name == wal_name)
    {
        if let Some(expected) = member.sha256.as_deref() {
            if sha256_file(&wal)?.eq_ignore_ascii_case(expected) {
                return Ok(());
            }
        }
    }
    fs::remove_file(&wal).map_err(|error| MigrationV5Error::Io(error.to_string()))
}

fn remove_split_brain_files(parent: &Path) -> Result<(), MigrationV5Error> {
    let staged = parent.join(STAGED_V200_NAME);
    remove_if_present(&staged)?;
    remove_if_present(&wal_path(&staged))?;
    remove_if_present(&parent.join(REPLACE_TEMP_NAME))?;
    Ok(())
}

fn remove_if_present(path: &Path) -> Result<(), MigrationV5Error> {
    if path.symlink_metadata().is_ok() {
        fs::remove_file(path).map_err(|error| MigrationV5Error::Io(error.to_string()))?;
    }
    Ok(())
}

/// `sync_all` on a file, or on a directory opened read-only.
fn fsync_path(path: &Path) -> Result<(), MigrationV5Error> {
    let file = File::open(path).map_err(|error| MigrationV5Error::Io(error.to_string()))?;
    file.sync_all()
        .map_err(|error| MigrationV5Error::Io(error.to_string()))
}

fn wal_is_dirty(database: &Path) -> bool {
    fs::metadata(wal_path(database))
        .map(|meta| meta.is_file() && meta.len() > 0)
        .unwrap_or(false)
}

fn checkpoint_preserving(database: &Path) -> Result<(), MigrationV5Error> {
    let conn = open_preserving(database)?;
    conn.execute_batch("CHECKPOINT")
        .map_err(|error| MigrationV5Error::Duck(error.to_string()))?;
    drop(conn);
    if let Ok(meta) = fs::metadata(wal_path(database)) {
        if meta.len() == 0 {
            let _ = fs::remove_file(wal_path(database));
        }
    }
    Ok(())
}

#[cfg(any(windows, target_os = "linux"))]
fn open_preserving(database: &Path) -> Result<Connection, MigrationV5Error> {
    let runtime = crate::storage::PrivateLibraryRuntime::prepare(database)
        .map_err(|error| MigrationV5Error::Io(error.to_string()))?;
    runtime
        .open_preserving_v156(false)
        .map_err(|error| MigrationV5Error::Duck(error.to_string()))
}

#[cfg(not(any(windows, target_os = "linux")))]
fn open_preserving(database: &Path) -> Result<Connection, MigrationV5Error> {
    let config = duckdb::Config::default()
        .with("storage_compatibility_version", "v1.5.6")
        .map_err(|error| MigrationV5Error::Duck(error.to_string()))?;
    Connection::open_with_flags(database, config)
        .map_err(|error| MigrationV5Error::Duck(error.to_string()))
}

#[cfg(any(windows, target_os = "linux"))]
fn open_cutover(database: &Path) -> Result<Connection, MigrationV5Error> {
    let runtime = crate::storage::PrivateLibraryRuntime::prepare(database)
        .map_err(|error| MigrationV5Error::Io(error.to_string()))?;
    runtime
        .attach_upgrading_v200()
        .map_err(|error| MigrationV5Error::Duck(error.to_string()))
}

#[cfg(not(any(windows, target_os = "linux")))]
fn open_cutover(database: &Path) -> Result<Connection, MigrationV5Error> {
    let config = duckdb::Config::default()
        .with("autoinstall_known_extensions", "false")
        .map_err(|error| MigrationV5Error::Duck(error.to_string()))?
        .with("autoload_known_extensions", "false")
        .map_err(|error| MigrationV5Error::Duck(error.to_string()))?;
    let conn = Connection::open_with_flags(":memory:", config)
        .map_err(|error| MigrationV5Error::Duck(error.to_string()))?;
    let path = database
        .to_str()
        .ok_or_else(|| MigrationV5Error::Io("database path is not unicode".into()))?
        .replace('\'', "''");
    conn.execute_batch(&format!(
        "ATTACH '{path}' AS dendro_cutover (STORAGE_VERSION 'v2.0.0'); USE dendro_cutover;"
    ))
    .map_err(|error| MigrationV5Error::Duck(error.to_string()))?;
    Ok(conn)
}

#[cfg(any(windows, target_os = "linux"))]
fn open_live_read(database: &Path) -> Result<Connection, MigrationV5Error> {
    let runtime = crate::storage::PrivateLibraryRuntime::prepare_read_only(database)
        .map_err(|error| MigrationV5Error::Io(error.to_string()))?;
    runtime
        .open(true)
        .map_err(|error| MigrationV5Error::Duck(error.to_string()))
}

#[cfg(not(any(windows, target_os = "linux")))]
fn open_live_read(database: &Path) -> Result<Connection, MigrationV5Error> {
    let config = duckdb::Config::default()
        .access_mode(duckdb::AccessMode::ReadOnly)
        .map_err(|error| MigrationV5Error::Duck(error.to_string()))?;
    Connection::open_with_flags(database, config)
        .map_err(|error| MigrationV5Error::Duck(error.to_string()))
}
