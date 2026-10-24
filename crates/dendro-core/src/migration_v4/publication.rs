//! A validated handoff to the native selection publisher. This module does not
//! select a consumer, relax historical openers, or rewrite the schema3 preimage.
use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::{child, engine, parse_sidecar, refuse_inconsistent_source, reuse_validated};
use super::{MigrationError, Refusal, SourceGuard, StageCandidate};
use super::{CONTENT_SEAL, IDENTITY_NAME, PREIMAGE_DB, PREIMAGE_KEY, STAGE_DB};
use crate::platform_v1::{self as platform, CanonicalIdentity, PrivateRoot};

/// Evidence captured from an admitted, held regular file. Fields are immutable
/// to callers; copying is followed by verifying the destination against them.
#[derive(Debug, PartialEq, Eq)]
pub struct MigrationPublicationFile {
    path: PathBuf,
    sha256: String,
    size: u64,
    identity: CanonicalIdentity,
}

impl MigrationPublicationFile {
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn sha256(&self) -> &str {
        &self.sha256
    }
    pub fn size(&self) -> u64 {
        self.size
    }
    pub fn identity(&self) -> CanonicalIdentity {
        self.identity
    }
}

/// Private-root custody and independently checked migration evidence. Obtaining
/// this value does not publish a pointer. The publisher must call
/// `verify_unchanged` before and after copying and must verify its private copies.
pub struct MigrationPublicationReady {
    root: PrivateRoot,
    database: MigrationPublicationFile,
    preimage_database: MigrationPublicationFile,
    preimage_key: MigrationPublicationFile,
    identity_record: MigrationPublicationFile,
    content_seal: MigrationPublicationFile,
    key_id: String,
    projection_sha256: String,
    source_database_sha256: String,
    held_files: Vec<platform::VerifiedFile>,
    source_guard: SourceGuard,
}

impl std::fmt::Debug for MigrationPublicationReady {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        out.debug_struct("MigrationPublicationReady")
            .field("database", &self.database)
            .field("preimage_database", &self.preimage_database)
            .finish_non_exhaustive()
    }
}

impl MigrationPublicationReady {
    pub fn database(&self) -> &MigrationPublicationFile {
        &self.database
    }
    pub fn key(&self) -> &MigrationPublicationFile {
        &self.preimage_key
    }
    pub fn preimage_database(&self) -> &MigrationPublicationFile {
        &self.preimage_database
    }
    pub fn preimage_key(&self) -> &MigrationPublicationFile {
        &self.preimage_key
    }
    pub fn identity_record(&self) -> &MigrationPublicationFile {
        &self.identity_record
    }
    pub fn content_seal(&self) -> &MigrationPublicationFile {
        &self.content_seal
    }
    pub fn key_id(&self) -> &str {
        &self.key_id
    }
    pub fn projection_sha256(&self) -> &str {
        &self.projection_sha256
    }
    pub fn source_database_sha256(&self) -> &str {
        &self.source_database_sha256
    }
    /// Copy from these admitted handles, then verify the destination and call
    /// `verify_unchanged` before publishing. Keep this object alive throughout.
    /// Cloned handles share a cursor: seek to offset zero before every copy and
    /// serialize access to each source handle.
    pub fn database_handle(&self) -> &std::fs::File {
        self.held_files[0].file()
    }
    pub fn preimage_database_handle(&self) -> &std::fs::File {
        self.held_files[1].file()
    }
    pub fn key_handle(&self) -> &std::fs::File {
        self.held_files[2].file()
    }
    pub fn identity_record_handle(&self) -> &std::fs::File {
        self.held_files[3].file()
    }
    pub fn content_seal_handle(&self) -> &std::fs::File {
        self.held_files[4].file()
    }

    pub fn verify_unchanged(&self) -> Result<(), MigrationError> {
        self.source_guard.ensure()?;
        refuse_inconsistent_source(self.database.path())?;
        refuse_inconsistent_source(self.preimage_database.path())?;
        for ((name, expected), held) in [
            (STAGE_DB, &self.database),
            (PREIMAGE_DB, &self.preimage_database),
            (PREIMAGE_KEY, &self.preimage_key),
            (IDENTITY_NAME, &self.identity_record),
            (CONTENT_SEAL, &self.content_seal),
        ]
        .into_iter()
        .zip(&self.held_files)
        {
            let observed = platform::metadata_from_handle(held.file())?;
            if observed.identity != expected.identity.identity() || observed.size != expected.size {
                return Err(changed("held migration file identity or size changed"));
            }
            if capture(&self.root, name)? != *expected {
                return Err(changed(
                    "migration handoff identity, size or digest changed",
                ));
            }
        }
        refuse_inconsistent_source(self.database.path())?;
        refuse_inconsistent_source(self.preimage_database.path())?;
        self.source_guard.ensure()?;
        Ok(())
    }
}

fn changed(detail: &str) -> MigrationError {
    MigrationError::Refused(Refusal {
        code: "migration_publication_evidence_changed",
        source_mutated: false,
        pointer_changed: false,
        stage_retained: true,
        detail: detail.to_owned(),
    })
}

fn capture(root: &PrivateRoot, name: &str) -> Result<MigrationPublicationFile, MigrationError> {
    let mut file = platform::open_existing_private_file(root, &child(name)?)?;
    let identity = file.canonical_identity()?;
    let before = platform::metadata_from_handle(file.file())?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.file_mut().read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    let after = platform::metadata_from_handle(file.file())?;
    if before != after {
        return Err(changed("migration handoff file changed while hashing"));
    }
    Ok(MigrationPublicationFile {
        path: file.path().to_path_buf(),
        sha256: hex::encode(hash.finalize()),
        size: after.size,
        identity,
    })
}

/// Recompute canonical projections, metadata and original source/preimage linkage
/// using the same validation as stage reuse. Public `StageCandidate` fields are
/// never accepted as authority for file identity, contents or provenance.
pub fn validate_publication_candidate(
    source_database: &Path,
    key_sidecar: &Path,
    candidate: &StageCandidate,
) -> Result<MigrationPublicationReady, MigrationError> {
    refuse_inconsistent_source(source_database)?;
    refuse_inconsistent_source(&candidate.root.path().join(STAGE_DB))?;
    let guard = SourceGuard::capture(source_database, Some(key_sidecar))?;
    let inspection = engine::inspect_read_only(source_database)?;
    let bytes = guard
        .sidecar_bytes
        .as_deref()
        .ok_or_else(|| changed("source key missing"))?;
    let parsed = parse_sidecar(bytes).map_err(|_| changed("source key malformed"))?;
    if parsed.key_id != inspection.key_id {
        return Err(changed("source key does not match schema3"));
    }
    let before_validation = [
        STAGE_DB,
        PREIMAGE_DB,
        PREIMAGE_KEY,
        IDENTITY_NAME,
        CONTENT_SEAL,
    ]
    .into_iter()
    .map(|name| capture(&candidate.root, name))
    .collect::<Result<Vec<_>, _>>()?;
    let validated = reuse_validated(candidate.root.path(), &guard, &inspection, &parsed)?
        .ok_or_else(|| changed("validated staged database missing"))?;
    guard.ensure()?;
    let after_validation = [
        STAGE_DB,
        PREIMAGE_DB,
        PREIMAGE_KEY,
        IDENTITY_NAME,
        CONTENT_SEAL,
    ]
    .into_iter()
    .map(|name| capture(&validated.root, name))
    .collect::<Result<Vec<_>, _>>()?;
    if before_validation != after_validation {
        return Err(changed(
            "migration candidate changed across semantic validation",
        ));
    }
    let held_files = [
        STAGE_DB,
        PREIMAGE_DB,
        PREIMAGE_KEY,
        IDENTITY_NAME,
        CONTENT_SEAL,
    ]
    .into_iter()
    .map(|name| {
        platform::open_existing_private_file(&validated.root, &child(name)?)
            .map_err(MigrationError::from)
    })
    .collect::<Result<Vec<_>, _>>()?;
    let mut evidence = after_validation.into_iter();
    let ready = MigrationPublicationReady {
        database: evidence.next().unwrap(),
        preimage_database: evidence.next().unwrap(),
        preimage_key: evidence.next().unwrap(),
        identity_record: evidence.next().unwrap(),
        content_seal: evidence.next().unwrap(),
        root: validated.root,
        key_id: parsed.key_id,
        projection_sha256: inspection.projection_sha256,
        source_database_sha256: guard.db_hash.clone(),
        held_files,
        source_guard: guard,
    };
    if ready.preimage_database.sha256 != ready.source_guard.db_hash
        || Some(&ready.preimage_key.sha256) != ready.source_guard.sidecar_hash.as_ref()
    {
        return Err(changed("schema3 preimage does not match original bundle"));
    }
    ready.verify_unchanged()?;
    Ok(ready)
}
