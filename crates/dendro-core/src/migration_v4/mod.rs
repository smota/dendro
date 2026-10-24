//! Schema3 to schema4 staging. `check_migration_admission` stays a refusal seam.
//! Consumer selection is not switched: `publish_selection_manifest` is still unproved.
//! `facts::SCHEMA_VERSION` is intentionally unread as the target label.

mod engine;
mod private_connection;
mod publication;
mod read_validation;
mod revision;
mod rollback;
mod sidecar;

use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use duckdb::Connection;
use sha2::{Digest, Sha256};

use crate::platform_v1::{
    self, parse_child_name, FileIdentity, ObservedMetadata, PlatformError, PrivateRoot,
};

pub use publication::{
    validate_publication_candidate, MigrationPublicationFile, MigrationPublicationReady,
};
pub use read_validation::{validate_selected_schema4, validate_selected_schema4_identity};
pub use revision::{
    insert_fact_revisions, query_revision_cutoff, revision_count, FactRevision, RevisionState,
};
pub use rollback::{
    merge_half_open_ranges, record_v4_only_fact, rollback_to_preserved_schema3, EvidenceScope,
    LegacyTableChange, LossReceipt, SourceRange, V4OnlyFact, Watermark,
};
pub use sidecar::{
    encode_sidecar, parse_sidecar, ParsedSidecar, SidecarError, SIDECAR_LEN, SIDECAR_VERSION,
};

/// Target label stored in the staged `dendro_meta` row. This is not `facts::SCHEMA_VERSION`.
pub const MIGRATION_TARGET_SCHEMA_VERSION: i32 = 4;
pub const PUBLICATION_ATTEMPT_LIMIT: u32 = 3;
const PUBLICATION_REFUSAL: &str = "atomic_existing_manifest_replace_unproved";
const STAGE_DB: &str = "stage.duckdb";
const PREIMAGE_DB: &str = "schema3-preimage.duckdb";
const PREIMAGE_KEY: &str = "schema3-preimage.key";
const IDENTITY_NAME: &str = "stage.identity";
const CONTENT_SEAL: &str = "stage.content.sha256";

/// Historical admission helper. Schema 3 remains refused here so existing callers
/// do not observe a silent admission flip. Staging enters through [`migrate_closed_snapshot`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationRefusal {
    UnsupportedSourceSchema,
    PlatformAndMigrationNotAdmitted,
}

pub fn check_migration_admission(source_schema: i32) -> Result<(), MigrationRefusal> {
    if source_schema != 3 {
        return Err(MigrationRefusal::UnsupportedSourceSchema);
    }
    Err(MigrationRefusal::PlatformAndMigrationNotAdmitted)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterruptBoundary {
    StageCreated,
    PartialDataCopy,
    BeforeTransactionCommit,
    AfterTransactionCommitBeforeIntegrity,
    AfterIntegrityBeforeManifestReplace,
    DuringManifestReplace,
}

#[derive(Debug, Clone)]
pub struct MigrateRequest<'a> {
    pub source_database: &'a Path,
    pub key_sidecar: Option<&'a Path>,
    pub previous_private_root: Option<&'a Path>,
    pub stop_at: Option<InterruptBoundary>,
    /// When set, a mismatch refuses before a target is created.
    pub expected_source_sha256_hex: Option<&'a str>,
    /// Deletes one staged usage row after commit so integrity must refuse.
    pub damage_staged_usage_row: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedIdentity {
    pub kind: &'static str,
    pub volume_or_device: u64,
    pub file_id_hex: String,
}

#[derive(Debug)]
pub struct StageCandidate {
    pub root: PrivateRoot,
    pub database_path: PathBuf,
    pub preimage_database: PathBuf,
    pub preimage_key: PathBuf,
    pub key_id: String,
    pub source_db_sha256: String,
    pub projection_sha256: String,
    pub operation_timestamp: String,
    pub target_schema: i32,
    pub file_identity: PersistedIdentity,
    pub publication_code: &'static str,
    pub publication_attempts: u32,
    pub selected: bool,
    pub reused: bool,
    pub limitations: Vec<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterruptReport {
    pub boundary: InterruptBoundary,
    pub consumer_schema: i32,
    pub source_unchanged: bool,
    pub stage_path: Option<PathBuf>,
    pub stage_selected: bool,
    pub live_db_deleted: bool,
    pub publication_code: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub code: &'static str,
    pub source_mutated: bool,
    pub pointer_changed: bool,
    pub stage_retained: bool,
    pub detail: String,
}

#[derive(Debug)]
pub enum MigrationError {
    Interrupted(InterruptReport),
    Refused(Refusal),
    SourceMutated { before: String, after: String },
    Platform(String),
    Duck(String),
    Io(String),
    IdentityUnproved(&'static str),
    PublicationBlocked(String),
    UnexpectedIdentifier(String),
}

impl From<duckdb::Error> for MigrationError {
    fn from(err: duckdb::Error) -> Self {
        Self::Duck(err.to_string())
    }
}

impl From<PlatformError> for MigrationError {
    fn from(err: PlatformError) -> Self {
        Self::Platform(err.to_string())
    }
}

impl From<std::io::Error> for MigrationError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err.to_string())
    }
}

impl std::fmt::Display for MigrationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Interrupted(report) => write!(f, "interrupted at {:?}", report.boundary),
            Self::Refused(refusal) => write!(f, "refused {}: {}", refusal.code, refusal.detail),
            Self::SourceMutated { before, after } => {
                write!(f, "source digest changed from {before} to {after}")
            }
            Self::Platform(text)
            | Self::Duck(text)
            | Self::Io(text)
            | Self::PublicationBlocked(text) => {
                write!(f, "{text}")
            }
            Self::IdentityUnproved(text) => write!(f, "identity unproved: {text}"),
            Self::UnexpectedIdentifier(text) => write!(f, "unexpected identifier {text}"),
        }
    }
}

impl std::error::Error for MigrationError {}

pub fn migrate_closed_snapshot(
    request: MigrateRequest<'_>,
) -> Result<StageCandidate, MigrationError> {
    migrate_with_root_factory(request, || {
        platform_v1::create_user_private_root(".dendro-t04-")
    })
}

/// Explicit storage selection; a refusal never falls back to another root.
#[cfg(any(windows, target_os = "linux"))]
pub fn migrate_closed_snapshot_with_root_choice(
    request: MigrateRequest<'_>,
    choice: &platform_v1::StorageRootChoice,
) -> Result<StageCandidate, MigrationError> {
    migrate_with_root_factory(request, || {
        platform_v1::create_user_private_root_with_choice(".dendro-t04-", choice)
    })
}

fn migrate_with_root_factory(
    request: MigrateRequest<'_>,
    create_root: impl FnOnce() -> Result<PrivateRoot, PlatformError>,
) -> Result<StageCandidate, MigrationError> {
    // Refuse a live WAL before attempting a byte read that a Windows writer may
    // deny. No source connection or target is opened along this refusal path.
    refuse_inconsistent_source(request.source_database)?;
    let guard = SourceGuard::capture(request.source_database, request.key_sidecar)?;
    if let Some(expected) = request.expected_source_sha256_hex {
        if !expected.eq_ignore_ascii_case(&guard.db_hash) {
            guard.ensure()?;
            return Err(MigrationError::Refused(Refusal {
                code: "source_checksum_mismatch",
                source_mutated: false,
                pointer_changed: false,
                stage_retained: false,
                detail: format!("expected {expected} observed {}", guard.db_hash),
            }));
        }
    }
    if let Err(error) = refuse_inconsistent_source(request.source_database) {
        guard.ensure()?;
        return Err(error);
    }
    let inspection = match engine::inspect_read_only(request.source_database) {
        Ok(inspection) => inspection,
        Err(error) => {
            guard.ensure()?;
            return Err(error);
        }
    };
    let sidecar = match guard.sidecar_bytes.as_deref() {
        Some(bytes) => bytes,
        None => {
            guard.ensure()?;
            return Err(MigrationError::Refused(Refusal {
                code: "key_missing",
                source_mutated: false,
                pointer_changed: false,
                stage_retained: false,
                detail: "sidecar absent".into(),
            }));
        }
    };
    let parsed = match parse_sidecar(sidecar) {
        Ok(parsed) => parsed,
        Err(SidecarError::Version) => {
            guard.ensure()?;
            return Err(MigrationError::Refused(Refusal {
                code: "sidecar_version_mismatch",
                source_mutated: false,
                pointer_changed: false,
                stage_retained: false,
                detail: "sidecar version is not 1".into(),
            }));
        }
        Err(other) => {
            guard.ensure()?;
            return Err(MigrationError::Refused(Refusal {
                code: "key_mismatch",
                source_mutated: false,
                pointer_changed: false,
                stage_retained: false,
                detail: format!("{other:?}"),
            }));
        }
    };
    if parsed.key_id != inspection.key_id {
        guard.ensure()?;
        return Err(MigrationError::Refused(Refusal {
            code: "key_mismatch",
            source_mutated: false,
            pointer_changed: false,
            stage_retained: false,
            detail: "sidecar key id does not match dendro_meta".into(),
        }));
    }
    if let Some(previous) = request.previous_private_root {
        let reuse = reuse_validated(previous, &guard, &inspection, &parsed);
        guard.ensure()?;
        if let Some(candidate) = reuse? {
            return Ok(candidate);
        }
    }

    let root = create_root()?;
    let db_name = child(STAGE_DB)?;
    let mut created = root.create_file(&db_name)?;
    let database_path = created.path().to_path_buf();
    if request.stop_at == Some(InterruptBoundary::StageCreated) {
        guard.ensure()?;
        return Err(MigrationError::Interrupted(InterruptReport {
            boundary: InterruptBoundary::StageCreated,
            consumer_schema: inspection.schema_version,
            source_unchanged: true,
            stage_path: Some(database_path),
            stage_selected: false,
            live_db_deleted: false,
            publication_code: None,
        }));
    }

    let preimage_database = write_private_preimage(&root, PREIMAGE_DB, request.source_database)?;
    let mut key_file = root.create_file(&child(PREIMAGE_KEY)?)?;
    let preimage_key = key_file.path().to_path_buf();
    key_file.file_mut().write_all(sidecar)?;
    key_file.file_mut().sync_all()?;
    drop(key_file);
    if sha256_file(&preimage_database)? != guard.db_hash
        || sha256_file(&preimage_key)? != guard.sidecar_hash.clone().unwrap_or_default()
    {
        guard.ensure()?;
        return Err(staged_refusal(
            "preimage_checksum_mismatch",
            "preimage digest mismatch".into(),
        ));
    }

    // DuckDB refuses an existing zero-byte file. Seed the platform-created
    // private file with the verified closed schema3 image through its held
    // handle; never unlink it or ask DuckDB to create an unverified replacement.
    let mut source_file = File::open(request.source_database)?;
    if request.stop_at == Some(InterruptBoundary::PartialDataCopy) {
        std::io::copy(&mut source_file.take(4096), created.file_mut())?;
        created.file_mut().sync_all()?;
        drop(created);
        guard.ensure()?;
        return Err(MigrationError::Interrupted(InterruptReport {
            boundary: InterruptBoundary::PartialDataCopy,
            consumer_schema: 3,
            source_unchanged: true,
            stage_path: Some(database_path),
            stage_selected: false,
            live_db_deleted: false,
            publication_code: None,
        }));
    }
    std::io::copy(&mut source_file, created.file_mut())?;
    created.file_mut().sync_all()?;
    drop(created);
    if sha256_file(&database_path)? != guard.db_hash {
        guard.ensure()?;
        return Err(staged_refusal(
            "stage_seed_checksum_mismatch",
            "stage seed digest mismatch".into(),
        ));
    }
    let operation_timestamp = operation_timestamp_now();
    let conn = open_rw(&database_path)?;
    conn.execute_batch("BEGIN TRANSACTION")?;
    engine::install_schema4_metadata(
        &conn,
        &inspection,
        &guard.db_hash,
        guard.sidecar_hash.as_deref().unwrap_or(""),
        &operation_timestamp,
        "committed_unvalidated",
    )?;
    conn.execute(
        "UPDATE dendro_migration_operation SET source_db_identity = ?, source_key_identity = ? WHERE id = 1",
        duckdb::params![guard.db_identity, guard.sidecar_identity],
    )?;
    if request.stop_at == Some(InterruptBoundary::BeforeTransactionCommit) {
        conn.execute_batch("ROLLBACK")?;
        conn.verify_runtime()?;
        drop(conn);
        guard.ensure()?;
        return Err(MigrationError::Interrupted(InterruptReport {
            boundary: InterruptBoundary::BeforeTransactionCommit,
            consumer_schema: 3,
            source_unchanged: true,
            stage_path: Some(database_path),
            stage_selected: false,
            live_db_deleted: false,
            publication_code: None,
        }));
    }
    conn.execute_batch("COMMIT")?;
    if request.stop_at == Some(InterruptBoundary::AfterTransactionCommitBeforeIntegrity) {
        conn.verify_runtime()?;
        drop(conn);
        guard.ensure()?;
        return Err(MigrationError::Interrupted(InterruptReport {
            boundary: InterruptBoundary::AfterTransactionCommitBeforeIntegrity,
            consumer_schema: 3,
            source_unchanged: true,
            stage_path: Some(database_path),
            stage_selected: false,
            live_db_deleted: false,
            publication_code: None,
        }));
    }
    if request.damage_staged_usage_row {
        conn.execute("DELETE FROM usage_facts WHERE source_key = 'usage-0'", [])?;
    }
    if let Err(error) = validate_committed_stage(&conn, &inspection) {
        conn.verify_runtime()?;
        drop(conn);
        guard.ensure()?;
        return Err(error);
    }
    engine::set_operation_status(&conn, "validated_unpublished")?;
    conn.execute_batch("CHECKPOINT")?;
    let candidate_digest = engine::candidate_digest(&conn)?;
    conn.verify_runtime()?;
    drop(conn);
    if request.stop_at == Some(InterruptBoundary::AfterIntegrityBeforeManifestReplace) {
        guard.ensure()?;
        return Err(MigrationError::Interrupted(InterruptReport {
            boundary: InterruptBoundary::AfterIntegrityBeforeManifestReplace,
            consumer_schema: 3,
            source_unchanged: true,
            stage_path: Some(database_path),
            stage_selected: false,
            live_db_deleted: false,
            publication_code: None,
        }));
    }
    let file_identity = persist_reopen_identity(&root, &db_name)?;
    let mut seal = root.create_file(&child(CONTENT_SEAL)?)?;
    seal.file_mut().write_all(candidate_digest.as_bytes())?;
    seal.file_mut().sync_all()?;
    drop(seal);
    let publication_code = refuse_consumer_switch()?;
    if request.stop_at == Some(InterruptBoundary::DuringManifestReplace) {
        guard.ensure()?;
        return Err(MigrationError::Interrupted(InterruptReport {
            boundary: InterruptBoundary::DuringManifestReplace,
            consumer_schema: 3,
            source_unchanged: true,
            stage_path: Some(database_path),
            stage_selected: false,
            live_db_deleted: false,
            publication_code: Some(publication_code),
        }));
    }
    guard.ensure()?;
    Ok(StageCandidate {
        root,
        database_path,
        preimage_database,
        preimage_key,
        key_id: parsed.key_id,
        source_db_sha256: guard.db_hash,
        projection_sha256: inspection.projection_sha256,
        operation_timestamp,
        target_schema: MIGRATION_TARGET_SCHEMA_VERSION,
        file_identity,
        publication_code,
        publication_attempts: 1,
        selected: false,
        reused: false,
        limitations: limitations(),
    })
}

pub fn refuse_consumer_switch() -> Result<&'static str, MigrationError> {
    match platform_v1::publish_selection_manifest() {
        Err(PlatformError::Rejected(verdict)) if verdict.code == PUBLICATION_REFUSAL => {
            Ok(verdict.code)
        }
        Err(err) => Err(MigrationError::PublicationBlocked(err.to_string())),
        Ok(_) => Err(MigrationError::PublicationBlocked(
            "publication returned success without an admitted integration in this lease".into(),
        )),
    }
}

pub fn recover_private_stage(
    source_database: &Path,
    stage_database: &Path,
) -> Result<InterruptReport, MigrationError> {
    let source = open_ro(source_database)?;
    let consumer_schema: i32 =
        source.query_row("SELECT schema_version FROM dendro_meta", [], |row| {
            row.get(0)
        })?;
    drop(source);
    let status = match open_ro(stage_database) {
        Ok(conn) => conn
            .query_row(
                "SELECT status FROM dendro_migration_operation WHERE id = 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .ok(),
        Err(_) => None,
    };
    let selected = status.as_deref() == Some("selected");
    Ok(InterruptReport {
        boundary: InterruptBoundary::AfterIntegrityBeforeManifestReplace,
        consumer_schema,
        source_unchanged: true,
        stage_path: Some(stage_database.to_path_buf()),
        stage_selected: selected,
        live_db_deleted: false,
        publication_code: None,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyCurrencyClass {
    Missing,
    Finite,
    SignedZero,
    PositiveInfinity,
    NegativeInfinity,
    Nan,
}

pub fn classify_currency_bits(bits: Option<u64>) -> LegacyCurrencyClass {
    let Some(bits) = bits else {
        return LegacyCurrencyClass::Missing;
    };
    let number = f64::from_bits(bits);
    if number.is_nan() {
        LegacyCurrencyClass::Nan
    } else if number.is_infinite() && number.is_sign_positive() {
        LegacyCurrencyClass::PositiveInfinity
    } else if number.is_infinite() {
        LegacyCurrencyClass::NegativeInfinity
    } else if bits == 0x8000_0000_0000_0000 {
        LegacyCurrencyClass::SignedZero
    } else {
        LegacyCurrencyClass::Finite
    }
}

pub fn canonical_exact_money_minor_units(_bits: Option<u64>) -> Result<i128, &'static str> {
    Err("legacy_binary64_approximate_not_eligible")
}

pub fn legacy_currency_bits(db: &Path, source_key: &str) -> Result<Option<u64>, MigrationError> {
    let conn = open_ro(db)?;
    let value: Option<f64> = conn.query_row(
        "SELECT currency_total FROM legacy_usage_facts WHERE source_key = ?",
        duckdb::params![source_key],
        |row| row.get(0),
    )?;
    Ok(value.map(f64::to_bits))
}

pub fn field_state(db: &Path, field: &str) -> Result<String, MigrationError> {
    let conn = open_ro(db)?;
    let state: String = conn.query_row(
        "SELECT state FROM dendro_schema4_field_state WHERE field_name = ?",
        duckdb::params![field],
        |row| row.get(0),
    )?;
    Ok(state)
}

pub fn spawn_edge_count(db: &Path) -> Result<i64, MigrationError> {
    let conn = open_ro(db)?;
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM dendro_spawn_edges", [], |row| {
        row.get(0)
    })?;
    Ok(count)
}

pub fn precision_tag(db: &Path, table: &str, column: &str) -> Result<String, MigrationError> {
    let conn = open_ro(db)?;
    let tag: String = conn.query_row(
        "SELECT precision_tag FROM dendro_schema4_precision WHERE table_name = ? AND column_name = ?",
        duckdb::params![table, column],
        |row| row.get(0),
    )?;
    Ok(tag)
}

pub fn assess_volumes(
    source: &ObservedMetadata,
    dest: &ObservedMetadata,
) -> Result<bool, MigrationError> {
    let same = match (&source.identity, &dest.identity) {
        (
            FileIdentity::WindowsFileId128 {
                volume_serial_64: left,
                ..
            },
            FileIdentity::WindowsFileId128 {
                volume_serial_64: right,
                ..
            },
        ) => left == right,
        (FileIdentity::Unix { device: left, .. }, FileIdentity::Unix { device: right, .. }) => {
            left == right
        }
        _ => {
            return Err(MigrationError::IdentityUnproved(
                "volume_identity_not_24byte_or_unix",
            ))
        }
    };
    if !same {
        // Planning helper only. It does not rename and it does not publish.
        let _verdict = crate::platform_v1::plan_cross_volume_then_local_pointer(true, false);
    }
    Ok(same)
}

pub fn sha256_file(path: &Path) -> Result<String, MigrationError> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn reuse_validated(
    previous: &Path,
    guard: &SourceGuard,
    inspection: &engine::SourceInspection,
    parsed: &ParsedSidecar,
) -> Result<Option<StageCandidate>, MigrationError> {
    let root = platform_v1::open_existing_private_root(previous)?;
    let database_path = root.path().join(STAGE_DB);
    if !database_path.is_file() {
        return Ok(None);
    }
    // Private-file admission and the persisted root/database identities precede
    // opening DuckDB. A same-inode edit still requires content verification below.
    if !root.path().join(IDENTITY_NAME).is_file() {
        return Err(MigrationError::IdentityUnproved("persisted_record_missing"));
    }
    let db_name = child(STAGE_DB)?;
    let file_identity = persist_reopen_identity(&root, &db_name)?;
    for name in [PREIMAGE_DB, PREIMAGE_KEY, CONTENT_SEAL, IDENTITY_NAME] {
        let file = platform_v1::open_existing_private_file(&root, &child(name)?)?;
        drop(file);
    }
    let conn = open_ro(&database_path)?;
    let (status, source_hash, projection, key_id, timestamp): (
        String,
        String,
        String,
        String,
        String,
    ) = conn.query_row(
        "SELECT status, source_db_sha256, projection_sha256, key_id, operation_timestamp
             FROM dendro_migration_operation WHERE id = 1",
        [],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        },
    )?;
    let version: i32 = conn.query_row("SELECT schema_version FROM dendro_meta", [], |row| {
        row.get(0)
    })?;
    let operation_matches: i64 = conn.query_row(
        "SELECT COUNT(*) FROM dendro_migration_operation WHERE id = 1
         AND source_schema = 3 AND target_schema = 4 AND source_sidecar_sha256 = ?
         AND generation_fingerprint = ? AND source_db_identity = ? AND source_key_identity = ?",
        duckdb::params![
            guard.sidecar_hash,
            inspection.generation_fingerprint,
            guard.db_identity,
            guard.sidecar_identity
        ],
        |row| row.get(0),
    )?;
    let mut staged_columns = std::collections::BTreeMap::new();
    for table in &inspection.tables {
        staged_columns.insert(table.clone(), engine::list_columns(&conn, table)?);
    }
    engine::ensure_logical_readability(&conn)?;
    let content_matches = staged_columns == inspection.columns
        && engine::legacy_counts_match(&conn, inspection)?
        && engine::staged_projection_hash(&conn, &inspection.tables, &staged_columns)?
            == inspection.projection_sha256;
    let sealed_digest = std::fs::read_to_string(root.path().join(CONTENT_SEAL))?;
    let seal_matches = engine::candidate_digest(&conn)? == sealed_digest;
    let metadata_matches = engine::pristine_metadata_matches(&conn, inspection, &guard.db_hash)?;
    let pristine: i64 = conn.query_row(
        "SELECT (SELECT COUNT(*) FROM dendro_fact_revisions)
              + (SELECT COUNT(*) FROM dendro_spawn_edges)
              + (SELECT COUNT(*) FROM dendro_v4_only_facts)",
        [],
        |row| row.get(0),
    )?;
    let preimage_matches = sha256_file(&root.path().join(PREIMAGE_DB))? == guard.db_hash
        && Some(sha256_file(&root.path().join(PREIMAGE_KEY))?).as_ref()
            == guard.sidecar_hash.as_ref();
    let rollback_matches: i64 = conn.query_row(
        "SELECT COUNT(*) FROM dendro_rollback_preimage WHERE source_db_sha256 = ?
         AND source_sidecar_sha256 = ? AND source_schema = 3
         AND note = 'preserved_source_bundle_not_rewritten'",
        duckdb::params![guard.db_hash, guard.sidecar_hash],
        |row| row.get(0),
    )?;
    let operation_rows = engine::count_rows(&conn, "dendro_migration_operation")?;
    let rollback_rows = engine::count_rows(&conn, "dendro_rollback_preimage")?;
    conn.verify_runtime()?;
    drop(conn);
    if !metadata_matches
        || !seal_matches
        || !content_matches
        || !preimage_matches
        || operation_matches != 1
        || operation_rows != 1
        || rollback_matches != 1
        || rollback_rows != 1
        || pristine != 0
        || status != "validated_unpublished"
        || source_hash != guard.db_hash
        || projection != inspection.projection_sha256
        || key_id != parsed.key_id
        || version != MIGRATION_TARGET_SCHEMA_VERSION
    {
        guard.ensure()?;
        return Err(MigrationError::Refused(Refusal {
            code: "reused_stage_validation_failed",
            source_mutated: false,
            pointer_changed: false,
            stage_retained: true,
            detail: "candidate content, operation, source identity or preimage differs".into(),
        }));
    }
    let publication_code = refuse_consumer_switch()?;
    Ok(Some(StageCandidate {
        root,
        database_path,
        preimage_database: previous.join(PREIMAGE_DB),
        preimage_key: previous.join(PREIMAGE_KEY),
        key_id,
        source_db_sha256: source_hash,
        projection_sha256: projection,
        operation_timestamp: timestamp,
        target_schema: version,
        file_identity,
        publication_code,
        publication_attempts: 1,
        selected: false,
        reused: true,
        limitations: limitations(),
    }))
}

fn persist_reopen_identity(
    root: &PrivateRoot,
    name: &platform_v1::ChildName,
) -> Result<PersistedIdentity, MigrationError> {
    let first = platform_v1::open_existing_private_file(root, name)?;
    let identity = persisted_identity(first.metadata())?;
    drop(first);
    let second = platform_v1::open_existing_private_file(root, name)?;
    let again = persisted_identity(second.metadata())?;
    drop(second);
    if identity != again {
        return Err(MigrationError::IdentityUnproved("reopen_mismatch"));
    }
    let mut record = identity_record(&persisted_identity(root.metadata())?);
    record.extend(identity_record(&identity));
    let identity_name = child(IDENTITY_NAME)?;
    // A second migration reuses the root and the identity file may already exist.
    if root.path().join(IDENTITY_NAME).is_file() {
        let existing = std::fs::read(root.path().join(IDENTITY_NAME))?;
        if existing != record {
            return Err(MigrationError::IdentityUnproved(
                "persisted_record_mismatch",
            ));
        }
    } else {
        let mut file = root.create_file(&identity_name)?;
        file.file_mut().write_all(&record)?;
        file.file_mut().sync_all()?;
    }
    let third = platform_v1::open_existing_private_file(root, name)?;
    let stored = persisted_identity(third.metadata())?;
    drop(third);
    if stored != identity {
        return Err(MigrationError::IdentityUnproved(
            "identity_changed_after_record",
        ));
    }
    Ok(identity)
}

fn persisted_identity(meta: &ObservedMetadata) -> Result<PersistedIdentity, MigrationError> {
    match meta.identity {
        FileIdentity::WindowsFileId128 {
            volume_serial_64,
            file_id,
        } => Ok(PersistedIdentity {
            kind: "windows_volume_u64_file_id_128",
            volume_or_device: volume_serial_64,
            file_id_hex: hex::encode(file_id),
        }),
        FileIdentity::Unix { device, inode } => Ok(PersistedIdentity {
            kind: "unix_device_inode",
            volume_or_device: device,
            file_id_hex: format!("{inode:016x}"),
        }),
        FileIdentity::WindowsByHandle64 { .. } => Err(MigrationError::IdentityUnproved(
            "by_handle_64_is_not_admission",
        )),
    }
}

fn identity_record(identity: &PersistedIdentity) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend(identity.kind.as_bytes());
    bytes.push(b'\n');
    bytes.extend(identity.volume_or_device.to_le_bytes());
    bytes.extend(hex::decode(&identity.file_id_hex).unwrap_or_default());
    bytes
}

fn write_private_preimage(
    root: &PrivateRoot,
    name: &str,
    source: &Path,
) -> Result<PathBuf, MigrationError> {
    let child_name = child(name)?;
    let mut created = root.create_file(&child_name)?;
    let path = created.path().to_path_buf();
    let mut source_file = File::open(source)?;
    std::io::copy(&mut source_file, created.file_mut())?;
    created.file_mut().sync_all()?;
    drop(created);
    Ok(path)
}

/// Retain these pins until DuckDB closes. Library-created spill leaves use
/// effective-private-parent admission; this never changes the process umask.
pub struct PrivateSpill {
    spill: PrivateRoot,
    extensions: PrivateRoot,
}

impl PrivateSpill {
    pub fn spill_path(&self) -> &Path {
        self.spill.path()
    }
    pub fn extensions_path(&self) -> &Path {
        self.extensions.path()
    }
}

pub fn configure_private_spill(
    conn: &Connection,
    root: &PrivateRoot,
) -> Result<PrivateSpill, MigrationError> {
    let spill = platform_v1::create_unique_private_directory(root, "spill-")?;
    let extensions = platform_v1::create_unique_private_directory(root, "ext-")?;
    configure_spill_paths(conn, spill.path(), extensions.path())?;
    Ok(PrivateSpill { spill, extensions })
}

fn configure_spill_paths(
    conn: &Connection,
    spill: &Path,
    extensions: &Path,
) -> Result<(), MigrationError> {
    let spill_sql = sql_path(spill)?;
    conn.execute_batch(&format!("SET temp_directory='{spill_sql}'"))?;
    configure_extension_directory(conn, extensions)
}

fn configure_extension_directory(
    conn: &Connection,
    extensions: &Path,
) -> Result<(), MigrationError> {
    let ext_sql = sql_path(extensions)?;
    // The bundled DuckDB supports this setting. Every failure is material.
    conn.execute_batch(&format!("SET extension_directory='{ext_sql}'"))?;
    Ok(())
}

fn validate_committed_stage(
    conn: &Connection,
    inspection: &engine::SourceInspection,
) -> Result<(), MigrationError> {
    let validation = (|| -> Result<(), MigrationError> {
        let counts_ok = engine::legacy_counts_match(conn, inspection)?;
        let mut staged_columns = std::collections::BTreeMap::new();
        for table in &inspection.tables {
            staged_columns.insert(table.clone(), engine::list_columns(conn, table)?);
        }
        let staged_projection =
            engine::staged_projection_hash(conn, &inspection.tables, &staged_columns)?;
        let projection_ok = staged_projection == inspection.projection_sha256
            && staged_columns == inspection.columns;
        engine::ensure_logical_readability(conn)?;
        let version: i32 = conn.query_row("SELECT schema_version FROM dendro_meta", [], |row| {
            row.get(0)
        })?;
        let revisions = engine::count_rows(conn, "dendro_fact_revisions")?;
        if version != MIGRATION_TARGET_SCHEMA_VERSION
            || revisions != 0
            || !counts_ok
            || !projection_ok
        {
            return Err(MigrationError::Duck(format!(
                "counts={counts_ok} projection={projection_ok} version={version} revisions={revisions}"
            )));
        }
        Ok(())
    })();
    if let Err(error) = validation {
        let retention = engine::set_operation_status(conn, "integrity_failed").and_then(|()| {
            conn.execute_batch("CHECKPOINT")
                .map_err(MigrationError::from)
        });
        return Err(staged_refusal(
            "staged_integrity_check",
            format!("logical validation failed: {error}; quarantine_status={retention:?}"),
        ));
    }
    Ok(())
}

fn staged_refusal(code: &'static str, detail: String) -> MigrationError {
    MigrationError::Refused(Refusal {
        code,
        source_mutated: false,
        pointer_changed: false,
        stage_retained: true,
        detail,
    })
}

fn refuse_inconsistent_source(db: &Path) -> Result<(), MigrationError> {
    let wal = wal_path(db);
    if wal.exists() {
        return Err(MigrationError::Refused(Refusal {
            code: "refuse_without_admitted_consistency_mechanism",
            source_mutated: false,
            pointer_changed: false,
            stage_retained: false,
            detail: "source wal present; blind copy refused".into(),
        }));
    }
    Ok(())
}

fn wal_path(db: &Path) -> PathBuf {
    let mut name = db.file_name().unwrap_or_default().to_os_string();
    name.push(".wal");
    db.with_file_name(name)
}

struct SourceGuard {
    database: PathBuf,
    sidecar: Option<PathBuf>,
    db_hash: String,
    sidecar_hash: Option<String>,
    sidecar_bytes: Option<Vec<u8>>,
    db_identity: String,
    sidecar_identity: Option<String>,
    siblings: Vec<(PathBuf, String)>,
}

impl SourceGuard {
    fn capture(database: &Path, sidecar: Option<&Path>) -> Result<Self, MigrationError> {
        let db_identity = physical_identity(database)?;
        let sidecar_identity = match sidecar {
            Some(path) if path.exists() => Some(physical_identity(path)?),
            _ => None,
        };
        let db_hash = sha256_file(database)?;
        let sidecar_bytes = match sidecar {
            Some(path) if path.exists() => Some(std::fs::read(path)?),
            _ => None,
        };
        let sidecar_hash = sidecar_bytes
            .as_ref()
            .map(|bytes| hex::encode(Sha256::digest(bytes)));
        let mut siblings = Vec::new();
        if let Some(parent) = database.parent() {
            let prefix = database.file_name().unwrap_or_default();
            if let Ok(entries) = std::fs::read_dir(parent) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path == database || sidecar.is_some_and(|side| path == side) {
                        continue;
                    }
                    let name = entry.file_name();
                    let rendered = name.to_string_lossy();
                    // A live writer owns the wal. Its bytes are the refusal signal, not a digest we stabilize.
                    if rendered.ends_with(".wal") {
                        continue;
                    }
                    if rendered.starts_with(prefix.to_string_lossy().as_ref()) && path.is_file() {
                        siblings.push((path.clone(), sha256_file(&path)?));
                    }
                }
            }
        }
        Ok(Self {
            database: database.to_path_buf(),
            sidecar: sidecar.map(Path::to_path_buf),
            db_hash,
            sidecar_hash,
            sidecar_bytes,
            db_identity,
            sidecar_identity,
            siblings,
        })
    }

    fn ensure(&self) -> Result<(), MigrationError> {
        refuse_inconsistent_source(&self.database)?;
        let after = sha256_file(&self.database)?;
        if after != self.db_hash || physical_identity(&self.database)? != self.db_identity {
            return Err(MigrationError::SourceMutated {
                before: self.db_hash.clone(),
                after,
            });
        }
        if let Some(path) = &self.sidecar {
            if path.exists() {
                let after = sha256_file(path)?;
                if Some(after.as_str()) != self.sidecar_hash.as_deref()
                    || Some(physical_identity(path)?).as_ref() != self.sidecar_identity.as_ref()
                {
                    return Err(MigrationError::SourceMutated {
                        before: self.sidecar_hash.clone().unwrap_or_default(),
                        after,
                    });
                }
            } else if self.sidecar_hash.is_some() {
                return Err(MigrationError::SourceMutated {
                    before: self.sidecar_hash.clone().unwrap_or_default(),
                    after: "missing".into(),
                });
            }
        }
        for (path, hash) in &self.siblings {
            if !path.exists() || sha256_file(path)? != *hash {
                return Err(MigrationError::SourceMutated {
                    before: hash.clone(),
                    after: "sibling changed".into(),
                });
            }
        }
        refuse_inconsistent_source(&self.database)?;
        Ok(())
    }
}

fn physical_identity(path: &Path) -> Result<String, MigrationError> {
    let metadata = platform_v1::reopen_metadata(path)?;
    if metadata.kind != platform_v1::ObjectKind::RegularFile || metadata.reparse {
        return Err(MigrationError::IdentityUnproved("source_not_regular_file"));
    }
    Ok(hex::encode(identity_record(&persisted_identity(
        &metadata,
    )?)))
}

fn limitations() -> Vec<&'static str> {
    let spill = "private_spill_parent_pins_retained_until_database_close; library_leaf_effective_private_parent_policy";
    vec![
        "atomic_existing_manifest_replace_unproved",
        "facts_SCHEMA_VERSION_not_rewired",
        "logical_readability_not_physical_integrity_check",
        spill,
        "linux_acceptance_not_run_on_this_worker",
    ]
}

fn operation_timestamp_now() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    format!("migration-operation-unix-ms:{millis}")
}

fn child(name: &str) -> Result<platform_v1::ChildName, MigrationError> {
    parse_child_name(name, platform_v1::current_native_platform()).map_err(|verdict| {
        MigrationError::Platform(format!("{}:{}", verdict.code, verdict.note.unwrap_or("")))
    })
}

pub(crate) fn open_ro(
    path: &Path,
) -> Result<private_connection::PrivateConnection, MigrationError> {
    private_connection::open(path, true)
}

pub(crate) fn open_rw(
    path: &Path,
) -> Result<private_connection::PrivateConnection, MigrationError> {
    private_connection::open(path, false)
}

pub(crate) fn quote_ident(name: &str) -> Result<String, MigrationError> {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return Err(MigrationError::UnexpectedIdentifier(name.to_string()));
    };
    if !(first == '_' || first.is_ascii_alphabetic())
        || !chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
    {
        return Err(MigrationError::UnexpectedIdentifier(name.to_string()));
    }
    Ok(format!("\"{name}\""))
}

fn sql_path(path: &Path) -> Result<String, MigrationError> {
    let text = path
        .to_str()
        .ok_or_else(|| MigrationError::Io("path is not utf-8".into()))?;
    if text.contains('\0') {
        return Err(MigrationError::Io("path contains a NUL byte".into()));
    }
    // DuckDB accepts forward slashes on Windows. Backslash is not used as an escape.
    Ok(text.replace('\\', "/").replace('\'', "''"))
}

#[cfg(test)]
mod guard_tests {
    use super::*;

    struct SyntheticGuardFiles {
        root: PathBuf,
        db: PathBuf,
        key: PathBuf,
    }
    impl SyntheticGuardFiles {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root =
                std::env::temp_dir().join(format!("dendro-guard-{}-{nonce}", std::process::id()));
            std::fs::create_dir(&root).unwrap();
            let db = root.join("synthetic.duckdb");
            let key = root.join("synthetic.duckdb.key");
            std::fs::write(&db, b"synthetic bytes for source identity guard only").unwrap();
            std::fs::write(&key, encode_sidecar(1, &[1; 32]).unwrap()).unwrap();
            Self { root, db, key }
        }
    }
    impl Drop for SyntheticGuardFiles {
        fn drop(&mut self) {
            for path in [&self.db, &self.key, &wal_path(&self.db)] {
                let _ = std::fs::remove_file(path);
            }
            let _ = std::fs::remove_dir(&self.root);
        }
    }

    #[test]
    fn final_source_guard_refuses_a_wal_created_after_capture() {
        let files = SyntheticGuardFiles::new();
        let guard = SourceGuard::capture(&files.db, Some(&files.key)).unwrap();
        std::fs::write(wal_path(&files.db), b"synthetic late writer indicator").unwrap();
        match guard.ensure().unwrap_err() {
            MigrationError::Refused(refusal) => assert_eq!(
                refusal.code,
                "refuse_without_admitted_consistency_mechanism"
            ),
            other => panic!("{other}"),
        }
    }

    #[test]
    fn sidecar_parser_and_hash_use_the_same_captured_bytes() {
        let files = SyntheticGuardFiles::new();
        let guard = SourceGuard::capture(&files.db, Some(&files.key)).unwrap();
        let captured = guard.sidecar_bytes.as_ref().unwrap();
        let captured_id = parse_sidecar(captured).unwrap().key_id;
        std::fs::write(&files.key, encode_sidecar(1, &[2; 32]).unwrap()).unwrap();
        assert_eq!(parse_sidecar(captured).unwrap().key_id, captured_id);
        assert_ne!(
            parse_sidecar(&std::fs::read(&files.key).unwrap())
                .unwrap()
                .key_id,
            captured_id
        );
        assert_eq!(
            guard.sidecar_hash.as_deref(),
            Some(hex::encode(Sha256::digest(captured)).as_str())
        );
        assert!(matches!(
            guard.ensure(),
            Err(MigrationError::SourceMutated { .. })
        ));
    }
}
