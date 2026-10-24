//! Linux immutable selection publication using safe rustix held-directory operations.
//! Blocking API: async callers must run their complete owned request on a blocking
//! worker. No database is renamed. Bundle pins use cooperative shared flock.
//! Manifest checksum is SHA256 of serde_json's compact serialization of the typed
//! SelectionPayload (declared struct field order, files sorted by leaf). The raw
//! manifest byte digest separately binds the exact persisted pointer bytes.
//! Host-administrator and same-principal content/ACL changes remain trusted.
#![deny(unsafe_code)]

use super::linux;
use super::*;
use rustix::fs::{renameat_with, RenameFlags};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs::File;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};

const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
const MAX_BUNDLE_FILES: usize = 64;

#[cfg(test)]
std::thread_local! {
    static POST_RENAME_REFUSAL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static INJECT_FOREIGN_EXCHANGE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Native Linux identity. Windows-shaped records cannot deserialize here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectionIdentity {
    pub device: u64,
    pub inode: u64,
}
impl TryFrom<FileIdentity> for SelectionIdentity {
    type Error = PlatformError;
    fn try_from(value: FileIdentity) -> Result<Self, Self::Error> {
        match value {
            FileIdentity::Unix { device, inode } if inode != 0 => Ok(Self { device, inode }),
            _ => Err(refusal("publication_full_linux_identity_required")),
        }
    }
}
impl From<SelectionIdentity> for FileIdentity {
    fn from(value: SelectionIdentity) -> Self {
        Self::Unix {
            device: value.device,
            inode: value.inode,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectionFile {
    pub role: String,
    pub name: String,
    pub identity: SelectionIdentity,
    pub size: u64,
    pub sha256: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectionPayload {
    pub schema: String,
    pub version: u32,
    pub bundle_name: String,
    pub bundle_identity: SelectionIdentity,
    pub files: Vec<SelectionFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectionManifest {
    pub payload: SelectionPayload,
    pub payload_sha256: [u8; 32],
}

/// Retains the verified private bundle and read-only file pins with shared flock.
/// This proves byte-level selection, not database-schema/integrity acceptance.
pub struct ResolvedSelection {
    pub manifest: SelectionManifest,
    pub selection: ExpectedSelection,
    bundle: PrivateRoot,
    files: Vec<(String, File)>,
}

impl ResolvedSelection {
    pub fn bundle(&self) -> &PrivateRoot {
        &self.bundle
    }
    pub fn file(&self, name: &str) -> Option<&File> {
        self.files
            .iter()
            .find(|(leaf, _)| leaf == name)
            .map(|(_, file)| file)
    }
}

/// Expected identities/digests must come from durable candidate/custody records.
/// A new path stat is observation, never permission to replace that object.
pub struct ClosedBundleFile {
    pub role: String,
    pub name: ChildName,
    pub expected_identity: FileIdentity,
    pub expected_size: u64,
    pub expected_sha256: [u8; 32],
}

pub struct PublicationSentinel {
    pub name: ChildName,
    pub expected: CanonicalIdentity,
}

#[derive(Debug, Clone, Copy)]
pub struct ExpectedSelection {
    pub identity: FileIdentity,
    pub sha256: [u8; 32],
}

/// All roots are borrowed actual admitted capabilities, never reconstructed for
/// authorization. Bundle must be a direct child of destination on the same volume.
/// Call outside any old writer-lock closure: this API acquires writer then publisher.
pub struct PublicationRequest<'a> {
    pub destination: &'a PrivateRoot,
    pub expected_destination: FileIdentity,
    pub staging: &'a PrivateRoot,
    pub expected_staging: FileIdentity,
    pub bundle: &'a PrivateRoot,
    pub expected_bundle: FileIdentity,
    pub files: &'a [ClosedBundleFile],
    pub manifest: ChildName,
    pub expected_current: Option<ExpectedSelection>,
    pub writer: PublicationSentinel,
    pub publisher: PublicationSentinel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionMutation {
    NotChanged,
    /// Rename returned success, but the final verification/flush did not complete.
    /// Recover by inspecting the actual self-validating pointer; never delete it.
    ReplacedUnconfirmed,
    Confirmed,
}

#[derive(Debug, Clone)]
pub struct RetainedPublicationAttempt {
    pub source_name: String,
    pub source_identity: FileIdentity,
    pub journal_name: String,
    pub journal_durable: bool,
}

#[derive(Debug)]
pub struct SelectionPublicationReceipt {
    pub manifest_identity: FileIdentity,
    pub manifest_sha256: [u8; 32],
    pub bundle_identity: FileIdentity,
    pub replaced_existing: bool,
    pub attempts: u32,
    pub retained: Vec<RetainedPublicationAttempt>,
    pub power_loss_guarantee: bool,
}

#[derive(Debug)]
pub struct SelectionPublicationFailure {
    pub error: PlatformError,
    pub mutation: SelectionMutation,
    pub retained: Vec<RetainedPublicationAttempt>,
}

impl std::fmt::Display for SelectionPublicationFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "selection publication {:?}: {}",
            self.mutation, self.error
        )
    }
}
impl std::error::Error for SelectionPublicationFailure {}

fn refusal(code: &'static str) -> PlatformError {
    PlatformError::Rejected(Verdict::refuse(code, "T24_linux_selection"))
}

fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn json<T: Serialize>(value: &T) -> Result<Vec<u8>, PlatformError> {
    serde_json::to_vec(value).map_err(|_| refusal("selection_serialization_failed"))
}

fn check_root(root: &PrivateRoot, expected: FileIdentity) -> Result<(), PlatformError> {
    SelectionIdentity::try_from(expected)?;
    root.check_anchor()?;
    if root.metadata().identity != expected {
        return Err(refusal("publication_root_identity_changed"));
    }
    Ok(())
}

fn check_roots(request: &PublicationRequest<'_>) -> Result<(), PlatformError> {
    check_root(request.destination, request.expected_destination)?;
    check_root(request.staging, request.expected_staging)?;
    check_root(request.bundle, request.expected_bundle)?;
    let destination = SelectionIdentity::try_from(request.expected_destination)?;
    for id in [request.expected_staging, request.expected_bundle] {
        if SelectionIdentity::try_from(id)?.device != destination.device {
            return Err(refusal("cross_volume_publication_refused"));
        }
    }
    if request.expected_staging == request.expected_destination
        || request.expected_bundle == request.expected_destination
        || request.expected_bundle == request.expected_staging
    {
        return Err(refusal("publication_roots_alias"));
    }
    let parent = request
        .bundle
        .path()
        .parent()
        .ok_or_else(|| refusal("bundle_parent_absent"))?;
    if reopen_metadata(parent)?.identity != request.expected_destination {
        return Err(refusal("bundle_not_direct_destination_child"));
    }
    Ok(())
}

fn hash_file(file: &mut File) -> Result<([u8; 32], u64), PlatformError> {
    let mut hash = Sha256::new();
    let mut size = 0u64;
    let mut buffer = [0u8; 65536];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        size = size
            .checked_add(read as u64)
            .ok_or_else(|| refusal("bundle_size_overflow"))?;
        hash.update(&buffer[..read]);
    }
    Ok((hash.finalize().into(), size))
}

fn valid_role(role: &str) -> bool {
    !role.is_empty()
        && role.len() <= 40
        && role
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b"_-".contains(&b))
}

fn bundle_pins(
    request: &PublicationRequest<'_>,
) -> Result<(Vec<File>, SelectionPayload), PlatformError> {
    check_roots(request)?;
    if request.files.is_empty() || request.files.len() > MAX_BUNDLE_FILES {
        return Err(refusal("bundle_file_count_outside_bound"));
    }
    let mut names = HashSet::new();
    let mut identities = HashSet::new();
    let mut files = Vec::new();
    let mut pins = Vec::new();
    for spec in request.files {
        if !valid_role(&spec.role)
            || !names.insert(spec.name.as_str())
            || !identities.insert(spec.expected_identity)
        {
            return Err(refusal("bundle_duplicate_or_invalid_file"));
        }
        let mut held = linux::open_private_read_pin(request.bundle, &spec.name)?;
        let meta = metadata_from_handle(&held)?;
        if meta.identity != spec.expected_identity || meta.size != spec.expected_size {
            return Err(refusal("bundle_identity_or_size_changed"));
        }
        let (sha256, size) = hash_file(&mut held)?;
        if sha256 != spec.expected_sha256
            || size != spec.expected_size
            || metadata_from_handle(&held)?.identity != spec.expected_identity
        {
            return Err(refusal("bundle_digest_changed"));
        }
        files.push(SelectionFile {
            role: spec.role.clone(),
            name: spec.name.as_str().to_owned(),
            identity: SelectionIdentity::try_from(spec.expected_identity)?,
            size,
            sha256,
        });
        pins.push(held);
    }
    // Every entry is declared and pinned; WAL/SHM/temp directories cannot hide
    // outside the manifest. Cooperative exclusive writers conflict with read pins.
    let entries = linux::directory_names(request.bundle, MAX_BUNDLE_FILES)?;
    if entries.len() != names.len() || entries.iter().any(|name| !names.contains(name.as_str())) {
        return Err(refusal("bundle_undeclared_entry"));
    }
    files.sort_by(|a, b| a.name.cmp(&b.name));
    let bundle_name = request
        .bundle
        .path()
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| refusal("bundle_name_not_roundtrip"))?
        .to_owned();
    parse_child_name(&bundle_name, NativePlatform::Linux).map_err(PlatformError::Rejected)?;
    Ok((
        pins,
        SelectionPayload {
            schema: "dendro.selection".into(),
            version: 1,
            bundle_name,
            bundle_identity: SelectionIdentity::try_from(request.expected_bundle)?,
            files,
        },
    ))
}

/// Bounded self-checking parse; does not grant admission to the referenced bundle.
pub fn parse_selection_manifest(bytes: &[u8]) -> Result<SelectionManifest, PlatformError> {
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(refusal("selection_manifest_too_large"));
    }
    let manifest: SelectionManifest =
        serde_json::from_slice(bytes).map_err(|_| refusal("selection_manifest_invalid"))?;
    if manifest.payload.schema != "dendro.selection"
        || manifest.payload.version != 1
        || manifest.payload.files.is_empty()
        || manifest.payload.files.len() > MAX_BUNDLE_FILES
        || manifest
            .payload
            .files
            .windows(2)
            .any(|files| files[0].name >= files[1].name)
        || manifest.payload_sha256 != digest(&json(&manifest.payload)?)
    {
        return Err(refusal("selection_manifest_checksum_or_version"));
    }
    parse_child_name(&manifest.payload.bundle_name, NativePlatform::Linux)
        .map_err(PlatformError::Rejected)?;
    SelectionIdentity::try_from(FileIdentity::from(manifest.payload.bundle_identity))?;
    let mut names = HashSet::new();
    let mut ids = HashSet::new();
    for file in &manifest.payload.files {
        parse_child_name(&file.name, NativePlatform::Linux).map_err(PlatformError::Rejected)?;
        SelectionIdentity::try_from(FileIdentity::from(file.identity))?;
        if !valid_role(&file.role)
            || !names.insert(&file.name)
            || !ids.insert(FileIdentity::from(file.identity))
        {
            return Err(refusal("selection_manifest_file_invalid"));
        }
    }
    Ok(manifest)
}

/// Recovery/read route: private named regular pointer plus full identity and raw
/// bytes digest. Replacement sharing means a valid old reader remains coherent.
pub fn read_selection_manifest(
    root: &PrivateRoot,
    name: &ChildName,
) -> Result<(SelectionManifest, ExpectedSelection), PlatformError> {
    let mut file = linux::open_private_read_pin(root, name)?;
    let meta = metadata_from_handle(&file)?;
    if meta.size > MAX_MANIFEST_BYTES {
        return Err(refusal("selection_manifest_too_large"));
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(MAX_MANIFEST_BYTES + 1)
        .read_to_end(&mut bytes)?;
    let manifest = parse_selection_manifest(&bytes)?;
    Ok((
        manifest,
        ExpectedSelection {
            identity: meta.identity,
            sha256: digest(&bytes),
        },
    ))
}

/// Resolve the currently read immutable version by its persisted manifest IDs
/// and digests, retaining pins for the consumer's read lifetime. No current path
/// observation substitutes for any expected identity in the manifest.
pub fn resolve_selection(
    destination: &PrivateRoot,
    expected_destination: FileIdentity,
    name: &ChildName,
) -> Result<ResolvedSelection, PlatformError> {
    check_root(destination, expected_destination)?;
    let (manifest, selection) = read_selection_manifest(destination, name)?;
    let bundle =
        open_existing_private_root(&destination.path().join(&manifest.payload.bundle_name))?;
    check_root(
        &bundle,
        FileIdentity::from(manifest.payload.bundle_identity),
    )?;
    let mut files = Vec::new();
    for spec in &manifest.payload.files {
        let child =
            parse_child_name(&spec.name, NativePlatform::Linux).map_err(PlatformError::Rejected)?;
        let mut file = linux::open_private_read_pin(&bundle, &child)?;
        let meta = metadata_from_handle(&file)?;
        let (hash, size) = hash_file(&mut file)?;
        if meta.identity != FileIdentity::from(spec.identity)
            || hash != spec.sha256
            || size != spec.size
        {
            return Err(refusal("selected_bundle_identity_or_digest_changed"));
        }
        // Hashing consumes the cursor; consumers read the verified bytes from zero.
        use std::io::{Seek, SeekFrom};
        file.seek(SeekFrom::Start(0))?;
        files.push((spec.name.clone(), file));
    }
    let names: HashSet<_> = manifest
        .payload
        .files
        .iter()
        .map(|file| file.name.as_str())
        .collect();
    let entries = linux::directory_names(&bundle, MAX_BUNDLE_FILES)?;
    if entries.len() != names.len() || entries.iter().any(|name| !names.contains(name.as_str())) {
        return Err(refusal("selected_bundle_undeclared_entry"));
    }
    check_root(destination, expected_destination)?;
    bundle.check_anchor()?;
    Ok(ResolvedSelection {
        manifest,
        selection,
        bundle,
        files,
    })
}

fn check_current(request: &PublicationRequest<'_>) -> Result<(), PlatformError> {
    match (
        read_selection_manifest(request.destination, &request.manifest),
        request.expected_current,
    ) {
        (Ok((_, observed)), Some(expected))
            if observed.identity == expected.identity && observed.sha256 == expected.sha256 =>
        {
            Ok(())
        }
        (Err(PlatformError::Io(error)), None) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(())
        }
        (Err(error), _) => Err(error),
        _ => Err(refusal("selection_precondition_changed")),
    }
}

fn generated_names() -> (String, String) {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let tick = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let suffix = format!(
        "{:x}-{:x}-{:x}",
        std::process::id(),
        tick,
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    (
        format!("selection-stage-{suffix}.json"),
        format!("selection-journal-{suffix}.jsonl"),
    )
}

/// One safe native publication attempt under the canonical locks. Busy or failed
/// attempts retain their source and durable journal; no old pointer is unlinked.
pub fn publish_closed_bundle(
    request: &PublicationRequest<'_>,
) -> Result<SelectionPublicationReceipt, SelectionPublicationFailure> {
    let fail = |error| SelectionPublicationFailure {
        error,
        mutation: SelectionMutation::NotChanged,
        retained: Vec::new(),
    };
    if request.writer.expected == request.publisher.expected
        || request.writer.name == request.publisher.name
        || request.manifest == request.writer.name
        || request.manifest == request.publisher.name
    {
        return Err(fail(refusal("publication_lock_alias")));
    }
    parse_child_name(request.manifest.as_str(), NativePlatform::Linux)
        .map_err(PlatformError::Rejected)
        .map_err(fail)?;
    check_roots(request).map_err(fail)?;
    with_exclusive_lock(
        request.destination,
        &request.writer.name,
        &request.writer.expected,
        |_| {
            with_exclusive_lock(
                request.destination,
                &request.publisher.name,
                &request.publisher.expected,
                |_| Ok(publish_locked(request)),
            )
        },
    )
    .map_err(fail)?
}

/// A verified newly created file retained across one atomic publication attempt.
/// All Linux publishers use this same expected-current check and fd-relative switch.
/// For replacement, the crate caller must hold its canonical publisher/writer
/// sentinel locks. Absent-only custody records need no replacement lock. Root
/// identity and same-device admission are still enforced here. On an exchange
/// mismatch inspect the actual pointer and the journal source leaf: the displaced
/// object is retained at that source name. No automatic destructive rollback.
pub(crate) struct LinuxPublicationSource<'a> {
    staging: &'a PrivateRoot,
    name: ChildName,
    source: VerifiedFile,
    published: bool,
}
impl<'a> LinuxPublicationSource<'a> {
    pub(crate) fn create(
        staging: &'a PrivateRoot,
        name: &ChildName,
    ) -> Result<Self, PlatformError> {
        staging.check_anchor()?;
        Ok(Self {
            staging,
            name: name.clone(),
            source: staging.create_file(name)?,
            published: false,
        })
    }
    pub(crate) fn identity(&self) -> Result<FileIdentity, PlatformError> {
        metadata_from_handle(self.source.file()).map(|meta| meta.identity)
    }
    pub(crate) fn sync_all(&self) -> Result<(), PlatformError> {
        self.source.file().sync_all().map_err(Into::into)
    }
    pub(crate) fn write_all(&mut self, bytes: &[u8]) -> Result<(), PlatformError> {
        self.source.file_mut().write_all(bytes).map_err(Into::into)
    }
    pub(crate) fn publish_checked(
        &mut self,
        destination: &PrivateRoot,
        expected_destination: FileIdentity,
        name: &ChildName,
        expected: Option<ExpectedSelection>,
    ) -> Result<(), PlatformError> {
        if self.published {
            return Err(refusal("publication_source_already_used"));
        }
        self.staging.check_anchor()?;
        destination.check_anchor()?;
        if destination.metadata().identity != expected_destination {
            return Err(refusal("publication_root_identity_changed"));
        }
        let staging = SelectionIdentity::try_from(self.staging.metadata().identity)?;
        let destination_id = SelectionIdentity::try_from(expected_destination)?;
        if staging.device != destination_id.device {
            return Err(refusal("cross_volume_publication_refused"));
        }
        if expected.is_some() && self.staging.metadata().identity == expected_destination {
            return Err(refusal("publication_roots_alias"));
        }
        let identity = self.identity()?;
        if open_existing_private_file(self.staging, &self.name)?
            .metadata()
            .identity
            != identity
        {
            return Err(refusal("staging_identity_changed"));
        }
        let current = read_regular_pointer(destination, name)?;
        match (current.as_ref(), expected) {
            (None, None) => {}
            (Some((observed, _)), Some(expected))
                if observed.identity == expected.identity && observed.sha256 == expected.sha256 => {
            }
            _ => return Err(refusal("selection_precondition_changed")),
        }
        self.sync_all()?;
        self.staging.sync_dir()?;
        // Canonical writer/publisher locks serialize admitted writers. Linux has no
        // inode-CAS rename against a hostile same-uid namespace mutator; that trust
        // boundary is explicit. EXCHANGE retains the exact old file for recovery.
        #[cfg(test)]
        if expected.is_some() && INJECT_FOREIGN_EXCHANGE.with(|flag| flag.replace(false)) {
            let foreign = parse_child_name("injected-foreign.json", NativePlatform::Linux)
                .map_err(PlatformError::Rejected)?;
            let mut file = destination.create_file(&foreign)?;
            file.file_mut().write_all(b"foreign occupant")?;
            file.file().sync_all()?;
            std::fs::rename(
                destination.path().join(foreign.as_str()),
                destination.path().join(name.as_str()),
            )?;
        }
        renameat_with(
            linux::held_root(self.staging)?,
            self.name.as_str(),
            linux::held_root(destination)?,
            name.as_str(),
            if expected.is_some() {
                RenameFlags::EXCHANGE
            } else {
                RenameFlags::NOREPLACE
            },
        )
        .map_err(linux::map_errno)?;
        self.published = true;
        if let Some(expected) = expected {
            let old = read_regular_pointer(self.staging, &self.name)?
                .ok_or_else(|| refusal("publication_exchanged_pointer_missing"))?
                .0;
            if old.identity != expected.identity || old.sha256 != expected.sha256 {
                return Err(refusal(
                    "publication_exchanged_pointer_changed_recovery_required",
                ));
            }
        }
        if open_existing_private_file(destination, name)?
            .metadata()
            .identity
            != identity
        {
            return Err(refusal("published_selection_identity_changed"));
        }
        destination.sync_dir()?;
        self.staging.sync_dir()?;
        Ok(())
    }
    pub(crate) fn was_published(&self) -> bool {
        self.published
    }
}

fn read_regular_pointer(
    root: &PrivateRoot,
    name: &ChildName,
) -> Result<Option<(ExpectedSelection, Vec<u8>)>, PlatformError> {
    let mut file = match linux::open_private_read_pin(root, name) {
        Err(PlatformError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(None)
        }
        result => result?,
    };
    let meta = metadata_from_handle(&file)?;
    if meta.size > MAX_MANIFEST_BYTES {
        return Err(refusal("selection_manifest_too_large"));
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(MAX_MANIFEST_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_MANIFEST_BYTES
        || metadata_from_handle(&file)?.identity != meta.identity
    {
        return Err(refusal("selection_manifest_changed"));
    }
    Ok(Some((
        ExpectedSelection {
            identity: meta.identity,
            sha256: digest(&bytes),
        },
        bytes,
    )))
}

fn publish_locked(
    request: &PublicationRequest<'_>,
) -> Result<SelectionPublicationReceipt, SelectionPublicationFailure> {
    let mut retained = Vec::new();
    let mut mutation = SelectionMutation::NotChanged;
    let mut run = || -> Result<SelectionPublicationReceipt, PlatformError> {
        check_current(request)?;
        let (_bundle_pins, payload) = bundle_pins(request)?;
        let manifest = SelectionManifest {
            payload_sha256: digest(&json(&payload)?),
            payload,
        };
        let bytes = json(&manifest)?;
        if bytes.len() as u64 > MAX_MANIFEST_BYTES {
            return Err(refusal("selection_manifest_too_large"));
        }
        let manifest_sha256 = digest(&bytes);
        let (source_name, journal_name) = generated_names();
        let source_child = parse_child_name(&source_name, NativePlatform::Linux)
            .map_err(PlatformError::Rejected)?;
        let mut source = LinuxPublicationSource::create(request.staging, &source_child)?;
        let identity = source.identity()?;
        retained.push(RetainedPublicationAttempt {
            source_name: source_name.clone(),
            source_identity: identity,
            journal_name: journal_name.clone(),
            journal_durable: false,
        });
        source.write_all(&bytes)?;
        source.sync_all()?;
        let mut journal = request.staging.create_file(
            &parse_child_name(&journal_name, NativePlatform::Linux)
                .map_err(PlatformError::Rejected)?,
        )?;
        let prepared = serde_json::json!({ "state":"prepared", "manifest": request.manifest.as_str(),
            "source":source_name, "source_identity":SelectionIdentity::try_from(identity)?,
            "destination_identity":SelectionIdentity::try_from(request.expected_destination)?,
            "staging_root_identity":SelectionIdentity::try_from(request.expected_staging)?,
            "writer_identity":SelectionIdentity::try_from(request.writer.expected.identity())?,
            "publisher_identity":SelectionIdentity::try_from(request.publisher.expected.identity())?,
            "expected_current_identity": request.expected_current.map(|value| SelectionIdentity::try_from(value.identity)).transpose()?,
            "expected_current_sha256": request.expected_current.map(|value|value.sha256),
            "bundle_identity":manifest.payload.bundle_identity, "manifest_sha256":manifest_sha256 });
        journal.file_mut().write_all(&json(&prepared)?)?;
        journal.file_mut().write_all(b"\n")?;
        journal.file().sync_all()?;
        request.staging.sync_dir()?;
        retained[0].journal_durable = true;
        check_roots(request)?;
        check_current(request)?;
        let published = source.publish_checked(
            request.destination,
            request.expected_destination,
            &request.manifest,
            request.expected_current,
        );
        if source.was_published() {
            mutation = SelectionMutation::ReplacedUnconfirmed;
        }
        published?;
        #[cfg(test)]
        if POST_RENAME_REFUSAL.with(|flag| flag.replace(false)) {
            return Err(refusal("injected_post_rename_failure"));
        }
        let (_, selected) = read_selection_manifest(request.destination, &request.manifest)?;
        if selected.identity != identity || selected.sha256 != manifest_sha256 {
            return Err(refusal("published_selection_identity_or_digest_changed"));
        }
        check_roots(request)?;
        request.destination.sync_dir()?;
        journal
            .file_mut()
            .write_all(b"{\"state\":\"selected\"}\n")?;
        journal.file().sync_all()?;
        mutation = SelectionMutation::Confirmed;
        Ok(SelectionPublicationReceipt {
            manifest_identity: identity,
            manifest_sha256,
            bundle_identity: request.expected_bundle,
            replaced_existing: request.expected_current.is_some(),
            attempts: 1,
            retained: retained.clone(),
            power_loss_guarantee: false,
        })
    };
    run().map_err(|error| SelectionPublicationFailure {
        error,
        mutation,
        retained,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture {
        root: PrivateRoot,
        staging: PrivateRoot,
        bundle: PrivateRoot,
        files: Vec<ClosedBundleFile>,
        writer: CanonicalIdentity,
        publisher: CanonicalIdentity,
    }
    fn name(value: &str) -> ChildName {
        parse_child_name(value, NativePlatform::Linux).unwrap()
    }
    impl Fixture {
        fn new() -> Self {
            let root = create_user_private_root(".dendro-selection-test-").unwrap();
            let staging = create_unique_private_directory(&root, "staging-").unwrap();
            let bundle = create_unique_private_directory(&root, "bundle-").unwrap();
            let mut files = Vec::new();
            for (role, leaf, bytes) in [
                (
                    "database",
                    "analytics.duckdb",
                    b"closed synthetic database".as_slice(),
                ),
                ("key", "key.bin", b"synthetic-key".as_slice()),
                (
                    "receipt",
                    "integrity.json",
                    b"synthetic-integrity".as_slice(),
                ),
            ] {
                let mut file = bundle.create_file(&name(leaf)).unwrap();
                file.file_mut().write_all(bytes).unwrap();
                file.file().sync_all().unwrap();
                files.push(ClosedBundleFile {
                    role: role.into(),
                    name: name(leaf),
                    expected_identity: file.metadata().identity,
                    expected_size: bytes.len() as u64,
                    expected_sha256: digest(bytes),
                });
            }
            let writer = root
                .create_file(&name("writer.lock"))
                .unwrap()
                .canonical_identity()
                .unwrap();
            let publisher = root
                .create_file(&name("publisher.lock"))
                .unwrap()
                .canonical_identity()
                .unwrap();
            Self {
                root,
                staging,
                bundle,
                files,
                writer,
                publisher,
            }
        }
        fn request(&self, current: Option<ExpectedSelection>) -> PublicationRequest<'_> {
            PublicationRequest {
                destination: &self.root,
                expected_destination: self.root.metadata().identity,
                staging: &self.staging,
                expected_staging: self.staging.metadata().identity,
                bundle: &self.bundle,
                expected_bundle: self.bundle.metadata().identity,
                files: &self.files,
                manifest: name("selection.json"),
                expected_current: current,
                writer: PublicationSentinel {
                    name: name("writer.lock"),
                    expected: self.writer,
                },
                publisher: PublicationSentinel {
                    name: name("publisher.lock"),
                    expected: self.publisher,
                },
            }
        }
        fn cleanup(self) {
            let path = self.root.path().to_owned();
            drop(self);
            std::fs::remove_dir_all(path).unwrap();
        }
    }

    #[test]
    fn linux_absent_replace_stale_digest_and_read_custody() {
        let fixture = Fixture::new();
        let first = publish_closed_bundle(&fixture.request(None)).unwrap();
        let (_, expected) =
            read_selection_manifest(&fixture.root, &name("selection.json")).unwrap();
        let mut old = linux::open_private_read_pin(&fixture.root, &name("selection.json")).unwrap();
        let second = publish_closed_bundle(&fixture.request(Some(expected))).unwrap();
        assert_ne!(first.manifest_identity, second.manifest_identity);
        assert_eq!(
            metadata_from_handle(&old).unwrap().identity,
            first.manifest_identity
        );
        let mut old_bytes = Vec::new();
        old.read_to_end(&mut old_bytes).unwrap();
        assert_eq!(digest(&old_bytes), expected.sha256);
        let stale = publish_closed_bundle(&fixture.request(Some(expected))).unwrap_err();
        assert_eq!(stale.mutation, SelectionMutation::NotChanged);
        let (_, current) = read_selection_manifest(&fixture.root, &name("selection.json")).unwrap();
        let mut wrong_digest = current;
        wrong_digest.sha256[0] ^= 1;
        assert_eq!(
            publish_closed_bundle(&fixture.request(Some(wrong_digest)))
                .unwrap_err()
                .mutation,
            SelectionMutation::NotChanged
        );
        assert_eq!(
            read_selection_manifest(&fixture.root, &name("selection.json"))
                .unwrap()
                .1
                .identity,
            current.identity
        );
        drop(old);
        fixture.cleanup();
    }

    #[test]
    fn linux_lock_busy_and_post_switch_failure_retain_evidence() {
        let fixture = Fixture::new();
        with_exclusive_lock(&fixture.root, &name("writer.lock"), &fixture.writer, |_| {
            let error = publish_closed_bundle(&fixture.request(None)).unwrap_err();
            assert_eq!(error.mutation, SelectionMutation::NotChanged);
            assert!(matches!(error.error, PlatformError::Rejected(_)));
            Ok(())
        })
        .unwrap();
        assert!(!fixture.root.path().join("selection.json").exists());
        POST_RENAME_REFUSAL.with(|flag| flag.set(true));
        let failure = publish_closed_bundle(&fixture.request(None)).unwrap_err();
        assert_eq!(failure.mutation, SelectionMutation::ReplacedUnconfirmed);
        assert!(failure.retained[0].journal_durable);
        let resolved = resolve_selection(
            &fixture.root,
            fixture.root.metadata().identity,
            &name("selection.json"),
        )
        .unwrap();
        assert_eq!(resolved.manifest.payload.files.len(), 3);
        drop(resolved);
        fixture.cleanup();
    }

    #[test]
    fn linux_cooperative_writer_busy_and_unexpected_destination_never_overwritten() {
        let fixture = Fixture::new();
        let writer = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(fixture.bundle.path().join("analytics.duckdb"))
            .unwrap();
        writer.try_lock().unwrap();
        let failure = publish_closed_bundle(&fixture.request(None)).unwrap_err();
        assert_eq!(failure.mutation, SelectionMutation::NotChanged);
        assert!(!fixture.root.path().join("selection.json").exists());
        drop(writer);
        let mut unexpected = fixture.root.create_file(&name("selection.json")).unwrap();
        unexpected
            .file_mut()
            .write_all(b"unexpected occupant")
            .unwrap();
        let identity = unexpected.metadata().identity;
        drop(unexpected);
        let failure = publish_closed_bundle(&fixture.request(None)).unwrap_err();
        assert_eq!(failure.mutation, SelectionMutation::NotChanged);
        assert_eq!(
            std::fs::read(fixture.root.path().join("selection.json")).unwrap(),
            b"unexpected occupant"
        );
        assert_eq!(
            open_existing_private_file(&fixture.root, &name("selection.json"))
                .unwrap()
                .metadata()
                .identity,
            identity
        );
        fixture.cleanup();
    }

    #[test]
    fn linux_root_displacement_and_group_access_are_refused() {
        use std::os::unix::fs::PermissionsExt;
        let fixture = Fixture::new();
        std::fs::set_permissions(fixture.root.path(), std::fs::Permissions::from_mode(0o750))
            .unwrap();
        assert!(fixture.root.check_anchor().is_err());
        std::fs::set_permissions(fixture.root.path(), std::fs::Permissions::from_mode(0o700))
            .unwrap();
        let path = fixture.root.path().to_path_buf();
        let displaced = path.with_file_name(format!("displaced-{:032x}", rand::random::<u128>()));
        std::fs::rename(&path, &displaced).unwrap();
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(fixture.root.check_anchor().is_err());
        assert_eq!(
            publish_closed_bundle(&fixture.request(None))
                .unwrap_err()
                .mutation,
            SelectionMutation::NotChanged
        );
        std::fs::remove_dir(&path).unwrap();
        std::fs::rename(&displaced, &path).unwrap();
        fixture.cleanup();
    }

    #[test]
    fn linux_alias_and_hostile_bundle_entries_refuse_without_switching() {
        use std::os::unix::fs::PermissionsExt;
        let fixture = Fixture::new();
        let mut alias = fixture.request(None);
        alias.staging = &fixture.root;
        alias.expected_staging = fixture.root.metadata().identity;
        let failure = publish_closed_bundle(&alias).unwrap_err();
        assert_eq!(failure.mutation, SelectionMutation::NotChanged);
        assert!(failure.retained.is_empty());
        fixture.cleanup();
        for case in ["group_mode", "file_acl", "hardlink", "symlink", "directory"] {
            let fixture = Fixture::new();
            let path = fixture.bundle.path().join("analytics.duckdb");
            match case {
                "group_mode" => {
                    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap()
                }
                "file_acl" => {
                    let mut acl = 2u32.to_le_bytes().to_vec();
                    for (tag, permissions, id) in [
                        (1u16, 6u16, u32::MAX),
                        (2, 4, 12345),
                        (4, 0, u32::MAX),
                        (16, 0, u32::MAX),
                        (32, 0, u32::MAX),
                    ] {
                        acl.extend_from_slice(&tag.to_le_bytes());
                        acl.extend_from_slice(&permissions.to_le_bytes());
                        acl.extend_from_slice(&id.to_le_bytes());
                    }
                    let file = std::fs::File::open(&path).unwrap();
                    rustix::fs::fsetxattr(
                        &file,
                        "system.posix_acl_access",
                        &acl,
                        rustix::fs::XattrFlags::empty(),
                    )
                    .unwrap();
                }
                "hardlink" => {
                    std::fs::hard_link(&path, fixture.bundle.path().join("extra-link")).unwrap()
                }
                "symlink" => {
                    std::fs::remove_file(&path).unwrap();
                    std::os::unix::fs::symlink(fixture.root.path().join("writer.lock"), &path)
                        .unwrap();
                }
                "directory" => {
                    create_private_directory(&fixture.bundle, &name("undeclared")).unwrap();
                }
                _ => unreachable!(),
            }
            assert_eq!(
                publish_closed_bundle(&fixture.request(None))
                    .unwrap_err()
                    .mutation,
                SelectionMutation::NotChanged,
                "{case}"
            );
            assert!(
                !fixture.root.path().join("selection.json").exists(),
                "{case}"
            );
            fixture.cleanup();
        }
    }

    #[test]
    fn linux_foreign_exchange_is_uncertain_and_retains_displaced_bytes_at_journal_source() {
        let fixture = Fixture::new();
        publish_closed_bundle(&fixture.request(None)).unwrap();
        let (_, expected) =
            read_selection_manifest(&fixture.root, &name("selection.json")).unwrap();
        INJECT_FOREIGN_EXCHANGE.with(|flag| flag.set(true));
        let failure = publish_closed_bundle(&fixture.request(Some(expected))).unwrap_err();
        assert_eq!(failure.mutation, SelectionMutation::ReplacedUnconfirmed);
        assert!(failure.retained[0].journal_durable);
        assert_eq!(
            std::fs::read(
                fixture
                    .staging
                    .path()
                    .join(&failure.retained[0].source_name)
            )
            .unwrap(),
            b"foreign occupant"
        );
        let (_, live) = read_selection_manifest(&fixture.root, &name("selection.json")).unwrap();
        assert_eq!(live.identity, failure.retained[0].source_identity);
        fixture.cleanup();
    }

    #[test]
    fn linux_identity_records_reject_windows_shape_and_private_root_acl() {
        assert!(serde_json::from_str::<SelectionIdentity>(
            r#"{"volume_serial_64":1,"file_id":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1]}"#
        )
        .is_err());
        let fixture = Fixture::new();
        let identity = SelectionIdentity::try_from(fixture.root.metadata().identity).unwrap();
        assert!(serde_json::to_value(identity)
            .unwrap()
            .get("inode")
            .is_some());
        let mut acl = 2u32.to_le_bytes().to_vec();
        for (tag, permissions, id) in [
            (1u16, 7u16, u32::MAX),
            (2, 4, 12345),
            (4, 0, u32::MAX),
            (16, 0, u32::MAX),
            (32, 0, u32::MAX),
        ] {
            acl.extend_from_slice(&tag.to_le_bytes());
            acl.extend_from_slice(&permissions.to_le_bytes());
            acl.extend_from_slice(&id.to_le_bytes());
        }
        rustix::fs::fsetxattr(
            linux::held_root(&fixture.root).unwrap(),
            "system.posix_acl_default",
            &acl,
            rustix::fs::XattrFlags::empty(),
        )
        .unwrap();
        assert!(fixture.root.check_anchor().is_err());
        assert!(open_existing_private_root(fixture.root.path()).is_err());
        fixture.cleanup();
    }
}
