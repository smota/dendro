//! Safe Windows selection publication over the separately audited native helper.
//! Blocking API: async callers must run their complete owned request on a blocking
//! worker. No database is renamed. Bundle read pins reject live writable handles.
//! Manifest checksum is SHA256 of serde_json's compact serialization of the typed
//! SelectionPayload (declared struct field order, files sorted by leaf). The raw
//! manifest byte digest separately binds the exact persisted pointer bytes.
//! Host-administrator and same-principal content/ACL changes remain trusted.
#![deny(unsafe_code)]

use super::*;
use dendro_windows_private_io::{ChildName as NativeName, VerifiedDirectory};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs::File;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
const MAX_BUNDLE_FILES: usize = 64;
const ATTEMPTS: u32 = 3;

#[cfg(test)]
std::thread_local! {
    static POST_RENAME_REFUSAL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Lossless on-disk Windows identity. No 32-bit volume or 64-bit ID fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectionIdentity {
    pub volume_serial_64: u64,
    pub file_id: [u8; 16],
}

impl TryFrom<FileIdentity> for SelectionIdentity {
    type Error = PlatformError;
    fn try_from(value: FileIdentity) -> Result<Self, Self::Error> {
        match value {
            FileIdentity::WindowsFileId128 {
                volume_serial_64,
                file_id,
            } if file_id != [0; 16] => Ok(Self {
                volume_serial_64,
                file_id,
            }),
            _ => Err(refusal("publication_full_windows_identity_required")),
        }
    }
}

impl From<SelectionIdentity> for FileIdentity {
    fn from(id: SelectionIdentity) -> Self {
        Self::WindowsFileId128 {
            volume_serial_64: id.volume_serial_64,
            file_id: id.file_id,
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

/// Retains the verified private bundle and read-only, deny-write/delete file pins.
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
    PlatformError::Rejected(Verdict::refuse(code, "T24_windows_selection"))
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

fn held_root(root: &PrivateRoot) -> Result<&File, PlatformError> {
    root._pins
        .last()
        .ok_or_else(|| refusal("ancestor_pin_absent"))
}

fn check_roots(request: &PublicationRequest<'_>) -> Result<(), PlatformError> {
    check_root(request.destination, request.expected_destination)?;
    check_root(request.staging, request.expected_staging)?;
    check_root(request.bundle, request.expected_bundle)?;
    let destination = SelectionIdentity::try_from(request.expected_destination)?;
    for id in [request.expected_staging, request.expected_bundle] {
        if SelectionIdentity::try_from(id)?.volume_serial_64 != destination.volume_serial_64 {
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
        let mut held = windows::open_private_read_pin(request.bundle, &spec.name, true)?;
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
    // outside the manifest, and an active writable DB handle conflicts above.
    let entries = std::fs::read_dir(request.bundle.path())?.collect::<Result<Vec<_>, _>>()?;
    if entries.len() != names.len()
        || entries.iter().any(|entry| {
            entry
                .file_name()
                .to_str()
                .is_none_or(|name| !names.contains(name))
        })
    {
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
    parse_child_name(&bundle_name, NativePlatform::Windows).map_err(PlatformError::Rejected)?;
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
    parse_child_name(&manifest.payload.bundle_name, NativePlatform::Windows)
        .map_err(PlatformError::Rejected)?;
    SelectionIdentity::try_from(FileIdentity::from(manifest.payload.bundle_identity))?;
    let mut names = HashSet::new();
    let mut ids = HashSet::new();
    for file in &manifest.payload.files {
        parse_child_name(&file.name, NativePlatform::Windows).map_err(PlatformError::Rejected)?;
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
    let mut file = windows::open_private_read_pin(root, name, false)?;
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
        let child = parse_child_name(&spec.name, NativePlatform::Windows)
            .map_err(PlatformError::Rejected)?;
        let mut file = windows::open_private_read_pin(&bundle, &child, true)?;
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
    let mut count = 0usize;
    for entry in std::fs::read_dir(bundle.path())? {
        let entry = entry?;
        count += 1;
        if entry
            .file_name()
            .to_str()
            .is_none_or(|name| !names.contains(name))
        {
            return Err(refusal("selected_bundle_undeclared_entry"));
        }
    }
    if count != names.len() {
        return Err(refusal("selected_bundle_file_count_changed"));
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

/// Actual safe native publication. Bounded busy retries retain each failed source
/// and durable identity journal. Old pointers are never unlinked to force progress.
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
    NativeName::parse(request.manifest.as_str())
        .map_err(PlatformError::from)
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
        // Crucially borrow actual admitted held pins. No path-reopened handle is
        // promoted into a native directory capability here.
        let staging = VerifiedDirectory::from_held(held_root(request.staging)?)?;
        let destination = VerifiedDirectory::from_held(held_root(request.destination)?)?;
        let name = NativeName::parse(request.manifest.as_str())?;
        for attempt in 1..=ATTEMPTS {
            check_roots(request)?;
            check_current(request)?;
            let (source_name, journal_name) = generated_names();
            let mut source = staging.create_source(&NativeName::parse(&source_name)?)?;
            let identity = source.identity()?;
            retained.push(RetainedPublicationAttempt {
                source_name: source_name.clone(),
                source_identity: identity,
                journal_name: journal_name.clone(),
                journal_durable: false,
            });
            source.write_all(&bytes)?;
            let journal_child = parse_child_name(&journal_name, NativePlatform::Windows)
                .map_err(PlatformError::Rejected)?;
            let mut journal = request.staging.create_file(&journal_child)?;
            let prepared = serde_json::json!({ "state": "prepared", "manifest": request.manifest.as_str(),
                "source": source_name, "source_identity": SelectionIdentity::try_from(identity)?,
                "destination_identity": SelectionIdentity::try_from(request.expected_destination)?,
                "staging_root_identity": SelectionIdentity::try_from(request.expected_staging)?,
                "writer_identity": SelectionIdentity::try_from(request.writer.expected.identity())?,
                "publisher_identity": SelectionIdentity::try_from(request.publisher.expected.identity())?,
                "expected_current_identity": request.expected_current.map(|value| SelectionIdentity::try_from(value.identity)).transpose()?,
                "expected_current_sha256": request.expected_current.map(|value| value.sha256),
                "bundle_identity": manifest.payload.bundle_identity, "manifest_sha256": manifest_sha256 });
            journal.file_mut().write_all(&json(&prepared)?)?;
            journal.file_mut().write_all(b"\n")?;
            journal.file().sync_all()?;
            request.staging.sync_dir()?;
            retained
                .last_mut()
                .expect("attempt recorded before write")
                .journal_durable = true;
            if source.identity()? != identity {
                return Err(refusal("staging_identity_changed"));
            }
            check_roots(request)?;
            check_current(request)?;
            match source.publish_replace(&destination, &name) {
                Ok(()) => {
                    mutation = SelectionMutation::ReplacedUnconfirmed;
                    #[cfg(test)]
                    if POST_RENAME_REFUSAL.with(|flag| flag.replace(false)) {
                        return Err(refusal("injected_post_rename_failure"));
                    }
                    let (_, published) =
                        read_selection_manifest(request.destination, &request.manifest)?;
                    if published.identity != identity || published.sha256 != manifest_sha256 {
                        return Err(refusal("published_selection_identity_or_digest_changed"));
                    }
                    check_roots(request)?;
                    request.destination.sync_dir()?;
                    journal
                        .file_mut()
                        .write_all(b"{\"state\":\"selected\"}\n")?;
                    journal.file().sync_all()?;
                    mutation = SelectionMutation::Confirmed;
                    return Ok(SelectionPublicationReceipt {
                        manifest_identity: identity,
                        manifest_sha256,
                        bundle_identity: request.expected_bundle,
                        replaced_existing: request.expected_current.is_some(),
                        attempts: attempt,
                        retained: retained.clone(),
                        power_loss_guarantee: false,
                    });
                }
                // NT rename with a deny-delete reader returns ACCESS_DENIED on
                // supported NTFS as well as SHARING/LOCK_VIOLATION. Retry is bounded;
                // genuine permission failures remain errors and gain no bypass.
                Err(error)
                    if matches!(error.raw_os_error(), Some(5 | 32 | 33)) && attempt < ATTEMPTS =>
                {
                    std::thread::sleep(Duration::from_millis(25 * u64::from(attempt)));
                }
                Err(error) => return Err(PlatformError::Io(error)),
            }
        }
        Err(refusal("publication_busy_retry_exhausted"))
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
    use std::os::windows::fs::OpenOptionsExt;

    struct Fixture {
        root: PrivateRoot,
        staging: PrivateRoot,
        bundle: PrivateRoot,
        files: Vec<ClosedBundleFile>,
        writer: CanonicalIdentity,
        publisher: CanonicalIdentity,
    }
    fn name(value: &str) -> ChildName {
        parse_child_name(value, NativePlatform::Windows).unwrap()
    }
    impl Fixture {
        fn new() -> Self {
            let root = create_user_private_root_with_choice(
                ".dendro-selection-test-",
                &StorageRootChoice::UserProfile,
            )
            .unwrap();
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
    fn staging_destination_alias_refuses_before_preparation() {
        let fixture = Fixture::new();
        let mut alias = fixture.request(None);
        alias.staging = &fixture.root;
        alias.expected_staging = fixture.root.metadata().identity;
        let failure = publish_closed_bundle(&alias).unwrap_err();
        assert_eq!(failure.mutation, SelectionMutation::NotChanged);
        assert!(failure.retained.is_empty());
        fixture.cleanup();
    }

    #[test]
    fn absent_then_replacement_is_verified_and_stale_expected_is_rejected() {
        let fixture = Fixture::new();
        let first = publish_closed_bundle(&fixture.request(None)).unwrap();
        assert!(!first.replaced_existing);
        assert_eq!(first.attempts, 1);
        let (manifest, expected) =
            read_selection_manifest(&fixture.root, &name("selection.json")).unwrap();
        assert_eq!(
            FileIdentity::from(manifest.payload.bundle_identity),
            fixture.bundle.metadata().identity
        );
        assert_eq!(expected.identity, first.manifest_identity);
        let second = publish_closed_bundle(&fixture.request(Some(expected))).unwrap();
        assert!(second.replaced_existing);
        assert_ne!(second.manifest_identity, first.manifest_identity);
        let stale = publish_closed_bundle(&fixture.request(Some(expected))).unwrap_err();
        assert_eq!(stale.mutation, SelectionMutation::NotChanged);
        assert_eq!(
            read_selection_manifest(&fixture.root, &name("selection.json"))
                .unwrap()
                .1
                .identity,
            second.manifest_identity
        );
        eprintln!("evidence_branch=selection_absent_replace_stale_full_identity_complete");
        fixture.cleanup();
    }

    #[test]
    fn busy_reader_preserves_old_selection_and_retains_recoverable_attempts() {
        let fixture = Fixture::new();
        let first = publish_closed_bundle(&fixture.request(None)).unwrap();
        let (_, expected) =
            read_selection_manifest(&fixture.root, &name("selection.json")).unwrap();
        let reader = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(fixture.root.path().join("selection.json"))
            .unwrap();
        let failure = publish_closed_bundle(&fixture.request(Some(expected))).unwrap_err();
        eprintln!("busy_reader_failure={failure:?}");
        assert_eq!(failure.mutation, SelectionMutation::NotChanged);
        assert_eq!(failure.retained.len(), ATTEMPTS as usize);
        assert_eq!(
            read_selection_manifest(&fixture.root, &name("selection.json"))
                .unwrap()
                .1
                .identity,
            first.manifest_identity
        );
        for retained in &failure.retained {
            let source =
                open_existing_private_file(&fixture.staging, &name(&retained.source_name)).unwrap();
            assert_eq!(source.metadata().identity, retained.source_identity);
            assert!(fixture
                .staging
                .path()
                .join(&retained.journal_name)
                .is_file());
        }
        drop(reader);
        let recovery = publish_closed_bundle(&fixture.request(Some(expected))).unwrap();
        assert!(recovery.replaced_existing);
        eprintln!("evidence_branch=selection_busy_reader_old_pointer_retained_and_retry_recovery_complete");
        fixture.cleanup();
    }

    #[test]
    fn wrong_bundle_digest_identity_and_live_writer_refuse_before_switch() {
        let mut fixture = Fixture::new();
        fixture.files[0].expected_sha256[0] ^= 1;
        assert_eq!(
            publish_closed_bundle(&fixture.request(None))
                .unwrap_err()
                .mutation,
            SelectionMutation::NotChanged
        );
        fixture.files[0].expected_sha256[0] ^= 1;
        let original = fixture.files[0].expected_identity;
        fixture.files[0].expected_identity = fixture.root.metadata().identity;
        assert!(publish_closed_bundle(&fixture.request(None)).is_err());
        fixture.files[0].expected_identity = original;
        fixture.files[0].expected_size += 1;
        assert!(publish_closed_bundle(&fixture.request(None)).is_err());
        fixture.files[0].expected_size -= 1;
        let writer = std::fs::OpenOptions::new()
            .write(true)
            .share_mode(7)
            .open(fixture.bundle.path().join(fixture.files[0].name.as_str()))
            .unwrap();
        assert!(publish_closed_bundle(&fixture.request(None)).is_err());
        assert!(!fixture.root.path().join("selection.json").exists());
        drop(writer);
        publish_closed_bundle(&fixture.request(None)).unwrap();
        let (_, mut expected) =
            read_selection_manifest(&fixture.root, &name("selection.json")).unwrap();
        expected.sha256[0] ^= 1;
        assert!(publish_closed_bundle(&fixture.request(Some(expected))).is_err());
        let mut changed =
            open_existing_private_file(&fixture.bundle, &fixture.files[0].name).unwrap();
        changed.file_mut().write_all(b"changed").unwrap();
        changed.file().sync_all().unwrap();
        drop(changed);
        assert!(resolve_selection(
            &fixture.root,
            fixture.root.metadata().identity,
            &name("selection.json")
        )
        .is_err());
        eprintln!("evidence_branch=selection_digest_identity_live_writer_refusals_complete");
        fixture.cleanup();
    }

    #[test]
    fn lock_contention_and_undeclared_wal_do_not_select() {
        let fixture = Fixture::new();
        with_exclusive_lock(&fixture.root, &name("writer.lock"), &fixture.writer, |_| {
            assert!(publish_closed_bundle(&fixture.request(None)).is_err());
            Ok(())
        })
        .unwrap();
        let extra = fixture
            .bundle
            .create_file(&name("analytics.duckdb.wal"))
            .unwrap();
        assert!(publish_closed_bundle(&fixture.request(None)).is_err());
        drop(extra);
        assert!(!fixture.root.path().join("selection.json").exists());
        fixture.cleanup();
    }

    #[test]
    fn post_mutation_failure_is_uncertain_and_pointer_resolves_for_recovery() {
        let fixture = Fixture::new();
        POST_RENAME_REFUSAL.with(|flag| flag.set(true));
        let failure = publish_closed_bundle(&fixture.request(None)).unwrap_err();
        assert_eq!(failure.mutation, SelectionMutation::ReplacedUnconfirmed);
        let selected = resolve_selection(
            &fixture.root,
            fixture.root.metadata().identity,
            &name("selection.json"),
        )
        .unwrap();
        assert_eq!(
            selected.selection.identity,
            failure.retained[0].source_identity
        );
        assert_eq!(
            selected.bundle().metadata().identity,
            fixture.bundle.metadata().identity
        );
        let mut bytes = Vec::new();
        selected
            .file("analytics.duckdb")
            .unwrap()
            .try_clone()
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();
        assert_eq!(bytes, b"closed synthetic database");
        drop(selected);
        eprintln!(
            "evidence_branch=selection_post_rename_uncertainty_resolves_verified_bundle_complete"
        );
        fixture.cleanup();
    }

    #[test]
    fn manifest_parser_refuses_unknown_duplicate_noninteger_and_modified_fields() {
        let fixture = Fixture::new();
        publish_closed_bundle(&fixture.request(None)).unwrap();
        let bytes = std::fs::read(fixture.root.path().join("selection.json")).unwrap();
        let manifest = parse_selection_manifest(&bytes).unwrap();
        let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        value["unknown"] = serde_json::json!(true);
        assert!(parse_selection_manifest(&json(&value).unwrap()).is_err());
        let payload = String::from_utf8(json(&manifest.payload).unwrap()).unwrap();
        let checksum = String::from_utf8(json(&manifest.payload_sha256).unwrap()).unwrap();
        let duplicate = format!(
            "{{\"payload\":{payload},\"payload\":{payload},\"payload_sha256\":{checksum}}}"
        );
        assert!(parse_selection_manifest(duplicate.as_bytes()).is_err());
        let mut modified = manifest.clone();
        modified.payload.files[0].sha256[0] ^= 1;
        assert!(parse_selection_manifest(&json(&modified).unwrap()).is_err());
        let mut modified = manifest.clone();
        modified.payload.version = 2;
        modified.payload_sha256 = digest(&json(&modified.payload).unwrap());
        assert!(parse_selection_manifest(&json(&modified).unwrap()).is_err());
        let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        value["payload"]["files"][0]["size"] = serde_json::json!(1.5);
        assert!(parse_selection_manifest(&json(&value).unwrap()).is_err());
        assert!(parse_selection_manifest(&vec![b' '; MAX_MANIFEST_BYTES as usize + 1]).is_err());
        fixture.cleanup();
    }

    #[test]
    fn competing_absent_selection_requests_have_exactly_one_winner() {
        let fixture = Fixture::new();
        let barrier = std::sync::Barrier::new(2);
        let wins = std::thread::scope(|scope| {
            let first = scope.spawn(|| {
                barrier.wait();
                publish_closed_bundle(&fixture.request(None)).is_ok()
            });
            let second = scope.spawn(|| {
                barrier.wait();
                publish_closed_bundle(&fixture.request(None)).is_ok()
            });
            u32::from(first.join().unwrap()) + u32::from(second.join().unwrap())
        });
        assert_eq!(wins, 1);
        resolve_selection(
            &fixture.root,
            fixture.root.metadata().identity,
            &name("selection.json"),
        )
        .unwrap();
        eprintln!("evidence_branch=selection_competing_absent_requests_single_winner_complete");
        fixture.cleanup();
    }
}
