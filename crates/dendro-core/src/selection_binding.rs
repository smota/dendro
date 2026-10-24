//! Native logical database binding and immutable selected-version custody.
//! The logical leaf is a legacy database or a typed marker. It is never replaced.
#![deny(unsafe_code)]

use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
#[cfg(windows)]
use dendro_windows_private_io::{ChildName as NativeName, VerifiedDirectory};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::lifecycle::{self, Stage, RUN_CANCEL};
use crate::output::{self, KeyMaterial};
use crate::platform_v1::{self as p, ChildName, FileIdentity, PrivateRoot};
use crate::storage::AnalyticsStore;

mod cleanup;
mod migration;
pub use cleanup::{request_retained_bundle_cleanup, CleanupRefusal};
pub use migration::{migrate_logical_db, rollback_logical_db};

const MARKER: &[u8] = b"DENDRO-LOGICAL-DB v1\n";
const MAX_RECORD: u64 = 64 * 1024;

#[cfg(test)]
thread_local! {
    pub(crate) static RECOVERY_STAGE_OPENS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub(crate) fn open_recovery_stage(path: &Path) -> Result<PrivateRoot> {
    #[cfg(test)]
    RECOVERY_STAGE_OPENS.with(|count| count.set(count.get() + 1));
    Ok(p::open_existing_private_root(path)?)
}

#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Identity {
    volume_serial_64: u64,
    file_id: [u8; 16],
}

#[cfg(windows)]
impl TryFrom<FileIdentity> for Identity {
    type Error = anyhow::Error;
    fn try_from(value: FileIdentity) -> Result<Self> {
        let p::FileIdentity::WindowsFileId128 {
            volume_serial_64,
            file_id,
        } = value
        else {
            return Err(anyhow!("full Windows file identity required"));
        };
        if file_id == [0; 16] {
            return Err(anyhow!("zero Windows file identity refused"));
        }
        Ok(Self {
            volume_serial_64,
            file_id,
        })
    }
}

#[cfg(windows)]
impl From<Identity> for FileIdentity {
    fn from(id: Identity) -> Self {
        Self::WindowsFileId128 {
            volume_serial_64: id.volume_serial_64,
            file_id: id.file_id,
        }
    }
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Identity {
    device: u64,
    inode: u64,
}
#[cfg(target_os = "linux")]
impl TryFrom<FileIdentity> for Identity {
    type Error = anyhow::Error;
    fn try_from(value: FileIdentity) -> Result<Self> {
        match value {
            FileIdentity::Unix { device, inode } if inode != 0 => Ok(Self { device, inode }),
            _ => Err(anyhow!("native Linux device/inode identity required")),
        }
    }
}
#[cfg(target_os = "linux")]
impl From<Identity> for FileIdentity {
    fn from(value: Identity) -> Self {
        Self::Unix {
            device: value.device,
            inode: value.inode,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pointer {
    identity: Identity,
    sha256: [u8; 32],
}

impl TryFrom<p::ExpectedSelection> for Pointer {
    type Error = anyhow::Error;
    fn try_from(value: p::ExpectedSelection) -> Result<Self> {
        Ok(Self {
            identity: value.identity.try_into()?,
            sha256: value.sha256,
        })
    }
}

impl From<Pointer> for p::ExpectedSelection {
    fn from(value: Pointer) -> Self {
        Self {
            identity: value.identity.into(),
            sha256: value.sha256,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BindingRecord {
    schema: String,
    version: u32,
    state: String,
    anchor: Identity,
    anchor_kind: String,
    parent: Identity,
    logical_leaf: String,
    manifest: String,
    writer: String,
    writer_identity: Identity,
    publisher: String,
    publisher_identity: Identity,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PublicationState {
    schema: String,
    version: u32,
    binding: String,
    generation: u32,
    previous: Option<Pointer>,
    selected: Pointer,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BundleProvenance {
    schema: String,
    version: u32,
    database_sha256: [u8; 32],
    key_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BundleVersion {
    schema: String,
    version: u32,
    database_sha256: [u8; 32],
    key_sha256: [u8; 32],
    provenance_sha256: [u8; 32],
}

pub struct LogicalDbBinding {
    pub logical: PathBuf,
    pub root: PrivateRoot,
    record: BindingRecord,
    hash: String,
    expected: Option<p::ExpectedSelection>,
    generation: u32,
    torn_state: Option<String>,
    torn_binding: bool,
}

pub struct LegacyRead {
    pub logical: PathBuf,
    pub database: PathBuf,
    pub key: PathBuf,
    _root: PrivateRoot,
    _anchor: File,
}

pub enum SelectedDb {
    Uninitialized,
    Legacy(Box<LegacyRead>),
    Selected(Box<SelectedRead>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryDisposition {
    NothingToRecover,
    ResumeRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionRecoveryOutcome {
    Reconciled,
    Unchanged,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecoveryStage {
    schema: String,
    stage: String,
    identity: Identity,
}

fn recovery_stage_leaf(
    domain: &str,
    identity: FileIdentity,
    digest: Option<[u8; 32]>,
) -> Result<String> {
    let mut hash = Sha256::new();
    hash.update(hash_identity(identity.try_into()?).as_bytes());
    if let Some(digest) = digest {
        hash.update(digest);
    }
    Ok(format!("{domain}-{}.json", hex::encode(hash.finalize())))
}

/// A locator is only a lookup hint. The original prepared journal must still
/// prove every pointer, parent, lock, predecessor and stage identity.
pub(crate) fn record_recovery_stage(
    root: &PrivateRoot,
    domain: &str,
    identity: FileIdentity,
    digest: Option<[u8; 32]>,
    stage: &PrivateRoot,
) -> Result<()> {
    let stage_leaf = stage
        .path()
        .file_name()
        .and_then(|v| v.to_str())
        .ok_or_else(|| anyhow!("recovery stage name invalid"))?;
    write_record(
        root,
        &recovery_stage_leaf(domain, identity, digest)?,
        &RecoveryStage {
            schema: "dendro.recovery-stage/v1".into(),
            stage: stage_leaf.into(),
            identity: stage.metadata().identity.try_into()?,
        },
    )?;
    Ok(())
}

pub(crate) fn recovery_stage(
    root: &PrivateRoot,
    domain: &str,
    identity: FileIdentity,
    digest: Option<[u8; 32]>,
    prefix: &str,
) -> Result<Option<PrivateRoot>> {
    let leaf = recovery_stage_leaf(domain, identity, digest)?;
    if optional_file(root, &leaf)?.is_none() {
        return Ok(None);
    }
    let record: RecoveryStage = read_record(root, &leaf)?;
    name(&record.stage)?;
    if record.schema != "dendro.recovery-stage/v1" || !record.stage.starts_with(prefix) {
        return Err(anyhow!("invalid recovery stage locator"));
    }
    root.check_anchor()?;
    let stage = open_recovery_stage(&root.path().join(&record.stage))
        .context("locator names a missing or unadmitted recovery stage")?;
    if stage.metadata().identity != record.identity.into() {
        return Err(anyhow!("recovery stage locator identity changed"));
    }
    Ok(Some(stage))
}

impl std::fmt::Display for RecoveryDisposition {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NothingToRecover => write!(
                out,
                "nothing_to_recover: no selected-storage custody exists; nothing to recover"
            ),
            Self::ResumeRequired => write!(
                out,
                "resume_required: run dendro run to complete the first selected publication"
            ),
        }
    }
}

impl std::error::Error for RecoveryDisposition {}

/// Owned request snapshot. The immutable file pins stay held until every reader
/// opened from this snapshot has closed. Do not clone or resolve it again mid-request.
pub struct SelectedRead {
    pub logical: PathBuf,
    pub database: PathBuf,
    pub key: PathBuf,
    pub custody: Arc<p::ResolvedSelection>,
    pub binding: LogicalDbBinding,
}

impl SelectedRead {
    pub fn selection(&self) -> p::ExpectedSelection {
        self.custody.selection
    }

    pub fn is_current(&self) -> Result<bool> {
        let binding = reopen_binding(&self.binding)?;
        let expected = self.selection();
        if binding.expected.map(|s| (s.identity, s.sha256))
            != Some((expected.identity, expected.sha256))
        {
            return Ok(false);
        }
        let (_, observed) =
            p::read_selection_manifest(&binding.root, &name(&binding.record.manifest)?)?;
        Ok(observed.identity == expected.identity && observed.sha256 == expected.sha256)
    }
}

fn name(value: &str) -> Result<ChildName> {
    p::parse_child_name(value, p::current_native_platform())
        .map_err(p::PlatformError::Rejected)
        .map_err(Into::into)
}

fn hash_file(path: &Path) -> Result<([u8; 32], u64)> {
    let mut input = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut total = 0u64;
    let mut block = [0u8; 64 * 1024];
    loop {
        let read = input.read(&mut block)?;
        if read == 0 {
            break;
        }
        hasher.update(&block[..read]);
        total += read as u64;
    }
    Ok((hasher.finalize().into(), total))
}

fn hash_identity(id: Identity) -> String {
    let mut hasher = Sha256::new();
    #[cfg(windows)]
    {
        hasher.update(id.volume_serial_64.to_le_bytes());
        hasher.update(id.file_id);
    }
    #[cfg(target_os = "linux")]
    {
        hasher.update(b"linux-device-inode-v1");
        hasher.update(id.device.to_le_bytes());
        hasher.update(id.inode.to_le_bytes());
    }
    hex::encode(hasher.finalize())
}

fn read_record<T: for<'a> Deserialize<'a>>(root: &PrivateRoot, leaf: &str) -> Result<T> {
    let mut file = p::open_existing_private_file(root, &name(leaf)?)?;
    if file.metadata().size > MAX_RECORD {
        return Err(anyhow!("selection custody record too large"));
    }
    let mut bytes = Vec::new();
    file.file_mut()
        .take(MAX_RECORD + 1)
        .read_to_end(&mut bytes)?;
    serde_json::from_slice(&bytes).context("invalid selection custody record")
}

fn read_prepared_journal(root: &PrivateRoot, leaf: &str) -> Result<serde_json::Value> {
    let mut file = p::open_existing_private_file(root, &name(leaf)?)?;
    if file.metadata().size > MAX_RECORD {
        return Err(anyhow!("selection recovery journal too large"));
    }
    let mut bytes = Vec::new();
    file.file_mut()
        .take(MAX_RECORD + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_RECORD {
        return Err(anyhow!("selection recovery journal too large"));
    }
    let first = bytes
        .split(|byte| *byte == b'\n')
        .next()
        .unwrap_or_default();
    serde_json::from_slice(first).context("invalid prepared selection journal")
}

fn read_pinned_record<T: for<'a> Deserialize<'a>>(file: &File) -> Result<T> {
    let mut held = file.try_clone()?;
    if held.metadata()?.len() > MAX_RECORD {
        return Err(anyhow!("selected sidecar too large"));
    }
    held.seek(SeekFrom::Start(0))?;
    let mut bytes = Vec::new();
    held.take(MAX_RECORD + 1).read_to_end(&mut bytes)?;
    serde_json::from_slice(&bytes).context("invalid selected sidecar")
}

fn selected_key(selected: &SelectedRead) -> Result<KeyMaterial> {
    let mut held = selected
        .custody
        .file("analytics.key")
        .ok_or_else(|| anyhow!("selected key role missing"))?
        .try_clone()?;
    held.seek(SeekFrom::Start(0))?;
    output::decode_key(&mut held)
}

#[cfg(windows)]
pub(crate) fn write_record<T: Serialize>(
    root: &PrivateRoot,
    leaf: &str,
    value: &T,
) -> Result<FileIdentity> {
    let bytes = serde_json::to_vec(value)?;
    if bytes.len() as u64 > MAX_RECORD {
        return Err(anyhow!("selection custody record too large"));
    }
    root.check_anchor()?;
    let native = VerifiedDirectory::from_held(root.held_directory_file()?)
        .context("record held directory")?;
    let temp_leaf = format!("record-{:032x}.tmp", rand::random::<u128>());
    let mut source = native
        .create_source(&NativeName::parse(&temp_leaf)?)
        .context("record create temp source")?;
    let identity = source.identity()?;
    let publish = (|| -> Result<()> {
        source.write_all(&bytes)?;
        if source.identity()? != identity {
            return Err(anyhow!("temporary custody record identity changed"));
        }
        source.sync_all().context("record flush held temp")?;
        root.sync_dir()?;
        root.check_anchor()?;
        source
            .rename_absent(&native, &NativeName::parse(leaf)?)
            .context("record rename absent")?;
        Ok(())
    })();
    if let Err(error) = publish {
        drop(source);
        // Cleanup is only for our exact temporary file. If the rename may
        // have succeeded, the absent temp is left alone and the caller still
        // receives the uncertain result for explicit recovery.
        if root.check_anchor().is_ok()
            && optional_file(root, &temp_leaf)
                .ok()
                .flatten()
                .is_some_and(|file| file.metadata().identity == identity)
        {
            let _ = fs::remove_file(root.path().join(&temp_leaf));
        }
        return Err(error);
    }
    if source.identity()? != identity {
        return Err(anyhow!("published custody record identity changed"));
    }
    drop(source);
    let reopened =
        p::open_existing_private_file(root, &name(leaf)?).context("record reopen final")?;
    if reopened.metadata().identity != identity {
        return Err(anyhow!("selection custody record identity changed"));
    }
    root.sync_dir()?;
    Ok(identity)
}

#[cfg(target_os = "linux")]
pub(crate) fn write_record<T: Serialize>(
    root: &PrivateRoot,
    leaf: &str,
    value: &T,
) -> Result<FileIdentity> {
    let bytes = serde_json::to_vec(value)?;
    if bytes.len() as u64 > MAX_RECORD {
        return Err(anyhow!("selection custody record too large"));
    }
    root.check_anchor()?;
    let temporary = name(&format!("record-{:032x}.tmp", rand::random::<u128>()))?;
    let mut source = p::LinuxPublicationSource::create(root, &temporary)?;
    let identity = source.identity()?;
    source.write_all(&bytes)?;
    source.sync_all()?;
    root.sync_dir()?;
    source
        .publish_checked(root, root.metadata().identity, &name(leaf)?, None)
        .context("record rename absent; temporary file retained")?;
    if p::open_existing_private_file(root, &name(leaf)?)?
        .metadata()
        .identity
        != identity
    {
        return Err(anyhow!("published custody record identity changed"));
    }
    root.sync_dir()?;
    Ok(identity)
}

fn optional_file(root: &PrivateRoot, leaf: &str) -> Result<Option<p::VerifiedFile>> {
    match fs::symlink_metadata(root.path().join(leaf)) {
        Ok(_) => Ok(Some(p::open_existing_private_file(root, &name(leaf)?)?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn optional_anchor(root: &PrivateRoot, leaf: &ChildName) -> Result<Option<File>> {
    match fs::symlink_metadata(root.path().join(leaf.as_str())) {
        Ok(_) => Ok(Some(p::open_existing_private_read_pin(root, leaf)?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn key_path(db: &Path) -> PathBuf {
    PathBuf::from(format!("{}.key", db.display()))
}

fn is_marker(file: &File) -> Result<bool> {
    if p::metadata_from_handle(file)?.size != MARKER.len() as u64 {
        return Ok(false);
    }
    let mut held = file.try_clone()?;
    held.seek(SeekFrom::Start(0))?;
    let mut bytes = [0u8; MARKER.len()];
    held.read_exact(&mut bytes)?;
    Ok(bytes == MARKER)
}

fn root_and_leaf(
    db: &Path,
    _choice: &p::StorageRootChoice,
) -> Result<(PrivateRoot, PathBuf, ChildName)> {
    let logical = std::path::absolute(db)?;
    let parent = logical
        .parent()
        .ok_or_else(|| anyhow!("logical database has no parent"))?;
    let leaf = p::parse_child_os(
        logical
            .file_name()
            .ok_or_else(|| anyhow!("logical database has no leaf"))?,
        p::current_native_platform(),
    )
    .map_err(p::PlatformError::Rejected)?;
    let root = p::open_existing_private_root(parent).map_err(|e| anyhow!("analytics parent is not admitted ({e}); run dendro storage-init, then --storage-root <printed path>, or --db <admitted path>"))?;
    Ok((root, logical, leaf))
}

/// Creates one reusable protected root; the exact returned path is the durable
/// configuration value. No broad-parent bootstrap file or fallback is created.
pub fn create_userprofile_storage_root() -> Result<PathBuf> {
    let root = p::create_user_private_root_with_choice(
        "dendro-store-",
        &p::StorageRootChoice::UserProfile,
    )?;
    Ok(root.path().to_path_buf())
}

fn binding_names(anchor: Identity) -> (String, String, String, String, String) {
    let hash = hash_identity(anchor);
    (
        hash.clone(),
        format!("binding-{hash}.json"),
        format!("selection-{hash}.json"),
        format!("writer-{hash}.lock"),
        format!("publisher-{hash}.lock"),
    )
}

fn state_name(hash: &str, generation: u32) -> String {
    format!("selected-{hash}-{generation:08}.json")
}

fn read_states_mode(
    root: &PrivateRoot,
    hash: &str,
    allow_torn_top: bool,
) -> Result<(u32, Option<p::ExpectedSelection>, Option<String>)> {
    let mut generation = 0u32;
    let mut previous: Option<Pointer> = None;
    let mut torn = None;
    loop {
        let next = generation
            .checked_add(1)
            .ok_or_else(|| anyhow!("selection state generation overflow"))?;
        let leaf = state_name(hash, next);
        if optional_file(root, &leaf)?.is_none() {
            break;
        }
        let state: PublicationState = match read_record(root, &leaf) {
            Ok(state) => state,
            Err(_) if allow_torn_top => {
                torn = Some(leaf);
                break;
            }
            Err(error) => return Err(error),
        };
        if state.schema != "dendro.selection-state"
            || state.version != 1
            || state.binding != hash
            || state.generation != next
            || state.previous != previous
        {
            return Err(anyhow!("contradictory selection custody state"));
        }
        previous = Some(state.selected);
        generation = next;
    }
    let prefix = format!("selected-{hash}-");
    for entry in fs::read_dir(root.path())? {
        RUN_CANCEL.check()?;
        let entry = entry?;
        if let Some(leaf) = entry.file_name().to_str() {
            if leaf.starts_with(&prefix)
                && leaf.ends_with(".json")
                && leaf != state_name(hash, generation)
                && torn.as_deref() != Some(leaf)
            {
                let ordinal = leaf
                    .strip_prefix(&prefix)
                    .and_then(|s| s.strip_suffix(".json"));
                if ordinal.is_some_and(|n| n.parse::<u32>().is_ok_and(|n| n > generation)) {
                    return Err(anyhow!("selection custody state has a gap"));
                }
            }
        }
    }
    Ok((generation, previous.map(Into::into), torn))
}

fn load_binding_mode(
    root: PrivateRoot,
    logical: PathBuf,
    anchor: File,
    allow_torn_top: bool,
) -> Result<Option<LogicalDbBinding>> {
    let anchor_id: Identity = p::metadata_from_handle(&anchor)?.identity.try_into()?;
    let parent_id: Identity = root.metadata().identity.try_into()?;
    let (hash, binding_leaf, manifest, writer, publisher) = binding_names(anchor_id);
    let Some(_) = optional_file(&root, &binding_leaf)? else {
        if optional_file(&root, &manifest)?.is_some() {
            return Err(anyhow!("orphan selection manifest requires recovery"));
        }
        return Ok(None);
    };
    let record: BindingRecord = read_record(&root, &binding_leaf)?;
    if record.schema != "dendro.logical-binding"
        || record.version != 1
        || record.state != "migration_started"
        || record.anchor != anchor_id
        || record.parent != parent_id
        || record.manifest != manifest
        || record.writer != writer
        || record.publisher != publisher
        || record.anchor_kind
            != if is_marker(&anchor)? {
                "marker"
            } else {
                "legacy"
            }
    {
        return Err(anyhow!(
            "logical database binding identity or namespace changed"
        ));
    }
    for (leaf, expected) in [
        (&record.writer, record.writer_identity),
        (&record.publisher, record.publisher_identity),
    ] {
        let file = p::open_existing_private_file(&root, &name(leaf)?)?;
        if file.metadata().identity != FileIdentity::from(expected) {
            return Err(anyhow!("canonical selection lock identity changed"));
        }
    }
    let (generation, expected, torn_state) = read_states_mode(&root, &hash, allow_torn_top)?;
    Ok(Some(LogicalDbBinding {
        logical,
        root,
        record,
        hash,
        expected,
        generation,
        torn_state,
        torn_binding: false,
    }))
}

fn load_binding(
    root: PrivateRoot,
    logical: PathBuf,
    anchor: File,
) -> Result<Option<LogicalDbBinding>> {
    load_binding_mode(root, logical, anchor, false)
}

fn refuse_displaced_binding(
    root: &PrivateRoot,
    logical: &Path,
    current: Option<FileIdentity>,
) -> Result<()> {
    let requested = logical
        .file_name()
        .ok_or_else(|| anyhow!("logical database has no leaf"))?
        .to_string_lossy();
    let actual_leaf = if let Some(identity) = current {
        root.check_anchor()?;
        let canonical = fs::canonicalize(logical)?;
        let observed = p::reopen_metadata(&canonical)?;
        let parent = canonical
            .parent()
            .ok_or_else(|| anyhow!("canonical logical parent missing"))?;
        if observed.identity != identity
            || p::reopen_metadata(parent)?.identity != root.metadata().identity
        {
            return Err(anyhow!(
                "logical final path differs from held anchor or parent"
            ));
        }
        root.check_anchor()?;
        canonical
            .file_name()
            .ok_or_else(|| anyhow!("canonical logical leaf missing"))?
            .to_string_lossy()
            .into_owned()
    } else {
        requested.into_owned()
    };
    let parent: Identity = root.metadata().identity.try_into()?;
    let mut ambiguous_absent_unicode = false;
    let mut unproven_torn_binding = false;
    for entry in fs::read_dir(root.path())? {
        RUN_CANCEL.check()?;
        let entry = entry?;
        let Some(file_name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !file_name.starts_with("binding-") || !file_name.ends_with(".json") {
            continue;
        }
        let record: BindingRecord = match read_record(root, &file_name) {
            Ok(record) => record,
            Err(_) => {
                unproven_torn_binding = true;
                continue;
            }
        };
        if record.parent == parent {
            let exact = record.logical_leaf == actual_leaf;
            let ascii_alias = cfg!(windows)
                && record.logical_leaf.is_ascii()
                && actual_leaf.is_ascii()
                && record.logical_leaf.eq_ignore_ascii_case(&actual_leaf);
            if exact || ascii_alias {
                if current != Some(record.anchor.into()) {
                    return Err(anyhow!("logical anchor changed or disappeared after selection binding; recovery required"));
                }
                return Ok(());
            }
            ambiguous_absent_unicode |= cfg!(windows)
                && current.is_none()
                && (!record.logical_leaf.is_ascii() || !actual_leaf.is_ascii());
        }
    }
    if ambiguous_absent_unicode {
        return Err(anyhow!("absent Unicode logical name is ambiguous with an existing binding; use its exact canonical spelling or a separate admitted root"));
    }
    if unproven_torn_binding {
        return Err(anyhow!("unbound logical candidate is ambiguous with a torn binding; recover that binding first"));
    }
    Ok(())
}

fn selected_from_binding(binding: LogicalDbBinding) -> Result<SelectedRead> {
    let expected = binding.expected.ok_or_else(|| {
        anyhow!("selection migration started; no publication state yet; run dendro run to complete the first selected publication")
    })?;
    let resolved = p::resolve_selection(
        &binding.root,
        binding.record.parent.into(),
        &name(&binding.record.manifest)?,
    )
    .context("selection pointer missing or invalid; recovery required")?;
    if resolved.selection.identity != expected.identity
        || resolved.selection.sha256 != expected.sha256
    {
        return Err(anyhow!(
            "selection pointer differs from persisted publication custody; recovery required"
        ));
    }
    validate_bundle(&resolved)?;
    migration::validate_legacy_anchor(&binding, &resolved)?;
    let database = resolved.bundle().path().join("analytics.duckdb");
    let key = resolved.bundle().path().join("analytics.key");
    Ok(SelectedRead {
        logical: binding.logical.clone(),
        database,
        key,
        custody: Arc::new(resolved),
        binding,
    })
}

fn validate_bundle(resolved: &p::ResolvedSelection) -> Result<()> {
    let files = &resolved.manifest.payload.files;
    for (role, leaf) in [
        ("database", "analytics.duckdb"),
        ("key", "analytics.key"),
        ("provenance", "provenance.json"),
        ("version", "version.json"),
    ] {
        if files
            .iter()
            .filter(|f| f.role == role && f.name == leaf)
            .count()
            != 1
            || resolved.file(leaf).is_none()
        {
            return Err(anyhow!("selected bundle missing unique {role} role"));
        }
    }
    if !matches!(files.len(), 4 | 9 | 11) {
        return Err(anyhow!("selected bundle has unexpected roles"));
    }
    migration::validate_migration_custody(resolved)?;
    let bundle = resolved.bundle().path();
    let provenance: BundleProvenance =
        read_pinned_record(resolved.file("provenance.json").unwrap())?;
    let version: BundleVersion = read_pinned_record(resolved.file("version.json").unwrap())?;
    if provenance.schema != "dendro.bundle-provenance"
        || provenance.version != 1
        || version.schema != "dendro.bundle-version"
        || version.version != 1
    {
        return Err(anyhow!("selected bundle sidecar schema changed"));
    }
    let db_hash = files
        .iter()
        .find(|f| f.name == "analytics.duckdb")
        .unwrap()
        .sha256;
    let key_hash = files
        .iter()
        .find(|f| f.name == "analytics.key")
        .unwrap()
        .sha256;
    let provenance_hash = files
        .iter()
        .find(|f| f.name == "provenance.json")
        .unwrap()
        .sha256;
    if provenance.database_sha256 != db_hash
        || version.database_sha256 != db_hash
        || version.key_sha256 != key_hash
        || version.provenance_sha256 != provenance_hash
    {
        return Err(anyhow!("selected bundle sidecars disagree"));
    }
    let mut held = resolved.file("analytics.key").unwrap().try_clone()?;
    held.seek(SeekFrom::Start(0))?;
    let key = output::decode_key(&mut held)?;
    let store = AnalyticsStore::open_read_only(&bundle.join("analytics.duckdb"))?;
    if store.key_id != key.id || store.key_id != provenance.key_id {
        return Err(anyhow!(
            "selected database key id differs from verified key"
        ));
    }
    Ok(())
}

/// Resolve one logical path once. Before binding, an admitted legacy database
/// remains readable. Once binding exists, no stale legacy fallback is possible.
pub fn resolve_logical_db(db: &Path, choice: &p::StorageRootChoice) -> Result<SelectedDb> {
    let (root, logical, leaf) = root_and_leaf(db, choice)?;
    let Some(anchor) = optional_anchor(&root, &leaf)? else {
        refuse_displaced_binding(&root, &logical, None)?;
        return Ok(SelectedDb::Uninitialized);
    };
    let marker = is_marker(&anchor)?;
    let anchor_id = p::metadata_from_handle(&anchor)?.identity;
    refuse_displaced_binding(&root, &logical, Some(anchor_id))?;
    if let Some(binding) = load_binding(root, logical.clone(), anchor)? {
        return selected_from_binding(binding).map(|read| SelectedDb::Selected(Box::new(read)));
    }
    if marker {
        return Err(anyhow!(
            "logical marker has no binding record; run dendro run to complete the first selected publication"
        ));
    }
    let root = p::open_existing_private_root(logical.parent().unwrap())?;
    let anchor = p::open_existing_private_read_pin(&root, &leaf)?;
    if p::metadata_from_handle(&anchor)?.identity != anchor_id {
        return Err(anyhow!("legacy database identity changed"));
    }
    let key = key_path(&logical);
    Ok(SelectedDb::Legacy(Box::new(LegacyRead {
        database: logical.clone(),
        logical,
        key,
        _root: root,
        _anchor: anchor,
    })))
}

fn can_resume_unpublished_binding(db: &Path, choice: &p::StorageRootChoice) -> Result<bool> {
    let (root, logical, leaf) = root_and_leaf(db, choice)?;
    let Some(anchor) = optional_anchor(&root, &leaf)? else {
        return Ok(false);
    };
    refuse_displaced_binding(
        &root,
        &logical,
        Some(p::metadata_from_handle(&anchor)?.identity),
    )?;
    let marker = is_marker(&anchor)?;
    match load_binding(root, logical, anchor)? {
        Some(binding) => Ok(binding.expected.is_none()
            && optional_file(&binding.root, &binding.record.manifest)?.is_none()),
        None => Ok(marker),
    }
}

fn admit_for_write(db: &Path, choice: &p::StorageRootChoice) -> Result<LogicalDbBinding> {
    let (root, logical, leaf) = root_and_leaf(db, choice)?;
    let admitted_parent = root.metadata().identity;
    let before = optional_anchor(&root, &leaf)?;
    refuse_displaced_binding(
        &root,
        &logical,
        before
            .as_ref()
            .map(|f| p::metadata_from_handle(f).map(|m| m.identity))
            .transpose()?,
    )?;
    let anchor = match before {
        Some(file) => file,
        None => {
            let mut file = root.create_file(&leaf)?;
            file.file_mut().write_all(MARKER)?;
            file.file().sync_all()?;
            root.sync_dir()?;
            file.into_file()
        }
    };
    let admitted_anchor = p::metadata_from_handle(&anchor)?.identity;
    if let Some(binding) = load_binding(root, logical.clone(), anchor)? {
        return Ok(binding);
    }
    let root = p::open_existing_private_root(
        logical
            .parent()
            .ok_or_else(|| anyhow!("logical database has no parent"))?,
    )?;
    let anchor = p::open_existing_private_read_pin(&root, &leaf)?;
    if root.metadata().identity != admitted_parent
        || p::metadata_from_handle(&anchor)?.identity != admitted_anchor
    {
        return Err(anyhow!("logical parent or anchor changed during admission"));
    }
    let anchor_id: Identity = p::metadata_from_handle(&anchor)?.identity.try_into()?;
    let anchor_kind = if is_marker(&anchor)? {
        "marker"
    } else {
        "legacy"
    };
    let canonical = fs::canonicalize(&logical)?;
    if p::reopen_metadata(&canonical)?.identity != FileIdentity::from(anchor_id)
        || p::reopen_metadata(
            canonical
                .parent()
                .ok_or_else(|| anyhow!("canonical parent missing"))?,
        )?
        .identity
            != root.metadata().identity
    {
        return Err(anyhow!("logical final path changed during binding"));
    }
    let canonical_leaf = canonical
        .file_name()
        .ok_or_else(|| anyhow!("canonical logical leaf missing"))?
        .to_string_lossy()
        .into_owned();
    let parent_id: Identity = root.metadata().identity.try_into()?;
    let (hash, binding_leaf, manifest, writer, publisher) = binding_names(anchor_id);
    // A crash can leave either canonical sentinel before the binding record.
    // Re-admit only a verified private regular single-link sentinel in this
    // exact root; never replace a pre-existing lock or infer a new authority.
    let sentinel = |leaf: &str| -> Result<Identity> {
        let file = match optional_file(&root, leaf)? {
            Some(file) => file,
            None => match root.create_file(&name(leaf)?) {
                Ok(file) => file,
                Err(p::PlatformError::Rejected(verdict))
                    if verdict.code == "exclusive_create_exists" =>
                {
                    p::open_existing_private_file(&root, &name(leaf)?)?
                }
                Err(error) => return Err(error.into()),
            },
        };
        file.canonical_identity()?.identity().try_into()
    };
    let writer_id = sentinel(&writer)?;
    let publisher_id = sentinel(&publisher)?;
    root.sync_dir()?;
    let record = BindingRecord {
        schema: "dendro.logical-binding".into(),
        version: 1,
        state: "migration_started".into(),
        anchor: anchor_id,
        anchor_kind: anchor_kind.into(),
        parent: parent_id,
        logical_leaf: canonical_leaf,
        manifest,
        writer,
        writer_identity: writer_id,
        publisher,
        publisher_identity: publisher_id,
    };
    write_record(&root, &binding_leaf, &record)?;
    Ok(LogicalDbBinding {
        logical: std::path::absolute(db)?,
        root,
        record,
        hash,
        expected: None,
        generation: 0,
        torn_state: None,
        torn_binding: false,
    })
}

fn current_key(binding: &LogicalDbBinding, selected: Option<&SelectedRead>) -> Result<KeyMaterial> {
    if let Some(selected) = selected {
        selected_key(selected)
    } else {
        output::load_or_create_key(
            &key_path(&binding.logical),
            binding.record.anchor_kind == "marker",
        )
    }
}

fn reopen_binding(binding: &LogicalDbBinding) -> Result<LogicalDbBinding> {
    let root = p::open_existing_private_root(binding.root.path())?;
    let leaf = p::parse_child_os(
        binding.logical.file_name().unwrap(),
        p::current_native_platform(),
    )
    .map_err(p::PlatformError::Rejected)?;
    let anchor = p::open_existing_private_read_pin(&root, &leaf)?;
    load_binding(root, binding.logical.clone(), anchor)?
        .ok_or_else(|| anyhow!("binding disappeared"))
}

fn make_bundle(
    binding: &LogicalDbBinding,
    staged: &Path,
    key_source: &Path,
    key: &KeyMaterial,
) -> Result<(PrivateRoot, Vec<p::ClosedBundleFile>)> {
    let bundle = p::create_unique_private_directory(&binding.root, "bundle-")?;
    lifecycle::private_copy(staged, &bundle.path().join("analytics.duckdb"), &RUN_CANCEL)?;
    lifecycle::private_copy(
        key_source,
        &bundle.path().join("analytics.key"),
        &RUN_CANCEL,
    )?;
    let copied_key = p::open_existing_private_file(&bundle, &name("analytics.key")?)?;
    let mut copied_key_handle = copied_key.file().try_clone()?;
    let decoded_copy = output::decode_key(&mut copied_key_handle)?;
    if decoded_copy.id != key.id || decoded_copy.bytes != key.bytes {
        return Err(anyhow!(
            "candidate copied key differs from verified source key"
        ));
    }
    let (database_sha256, _) = hash_file(&bundle.path().join("analytics.duckdb"))?;
    let (key_sha256, _) = hash_file(&bundle.path().join("analytics.key"))?;
    let provenance = BundleProvenance {
        schema: "dendro.bundle-provenance".into(),
        version: 1,
        database_sha256,
        key_id: key.id.clone(),
    };
    write_record(&bundle, "provenance.json", &provenance)?;
    let (provenance_sha256, _) = hash_file(&bundle.path().join("provenance.json"))?;
    let version = BundleVersion {
        schema: "dendro.bundle-version".into(),
        version: 1,
        database_sha256,
        key_sha256,
        provenance_sha256,
    };
    write_record(&bundle, "version.json", &version)?;
    let mut files = Vec::new();
    for (role, leaf) in [
        ("database", "analytics.duckdb"),
        ("key", "analytics.key"),
        ("provenance", "provenance.json"),
        ("version", "version.json"),
    ] {
        let verified = p::open_existing_private_file(&bundle, &name(leaf)?)?;
        let (expected_sha256, expected_size) = hash_file(verified.path())?;
        files.push(p::ClosedBundleFile {
            role: role.into(),
            name: name(leaf)?,
            expected_identity: verified.metadata().identity,
            expected_size,
            expected_sha256,
        });
    }
    // A read-only semantic check is required before the byte-level primitive.
    let store = AnalyticsStore::open_read_only(&bundle.path().join("analytics.duckdb"))?;
    if store.key_id != key.id {
        return Err(anyhow!("candidate database key id mismatch"));
    }
    drop(store);
    bundle.sync_dir()?;
    Ok((bundle, files))
}

fn append_state(binding: &LogicalDbBinding, selected: p::ExpectedSelection) -> Result<()> {
    let generation = binding
        .generation
        .checked_add(1)
        .ok_or_else(|| anyhow!("selection generation overflow"))?;
    let state = PublicationState {
        schema: "dendro.selection-state".into(),
        version: 1,
        binding: binding.hash.clone(),
        generation,
        previous: binding.expected.map(Pointer::try_from).transpose()?,
        selected: selected.try_into()?,
    };
    write_record(
        &binding.root,
        &state_name(&binding.hash, generation),
        &state,
    )?;
    Ok(())
}

pub(crate) fn quarantine_torn_record(root: &PrivateRoot, leaf: &str) -> Result<()> {
    let verified = p::open_existing_private_file(root, &name(leaf)?)?;
    let identity = verified.metadata().identity;
    drop(verified);
    // The destination lives in a freshly created private directory rather than
    // a check-then-rename random leaf that could overwrite retained evidence.
    let quarantine = p::create_unique_private_directory(root, "torn-")?;
    root.check_anchor()?;
    quarantine.check_anchor()?;
    fs::rename(
        root.path().join(leaf),
        quarantine.path().join("record.json"),
    )?;
    let held = p::open_existing_private_file(&quarantine, &name("record.json")?)?;
    if held.metadata().identity != identity {
        return Err(anyhow!("torn record quarantine identity changed"));
    }
    quarantine.sync_dir()?;
    root.sync_dir()?;
    Ok(())
}

fn load_recovery_binding(
    root: PrivateRoot,
    logical: PathBuf,
    anchor: File,
) -> Result<LogicalDbBinding> {
    let anchor_id: Identity = p::metadata_from_handle(&anchor)?.identity.try_into()?;
    let parent_id: Identity = root.metadata().identity.try_into()?;
    let (hash, binding_leaf, manifest, writer, publisher) = binding_names(anchor_id);
    if read_record::<BindingRecord>(&root, &binding_leaf).is_ok() {
        return load_binding_mode(root, logical, anchor, true)?
            .ok_or_else(|| anyhow!("selection binding disappeared during recovery"));
    }
    let writer_identity: Identity = p::open_existing_private_file(&root, &name(&writer)?)?
        .canonical_identity()?
        .identity()
        .try_into()?;
    let publisher_identity: Identity = p::open_existing_private_file(&root, &name(&publisher)?)?
        .canonical_identity()?
        .identity()
        .try_into()?;
    let canonical = fs::canonicalize(&logical)?;
    if p::reopen_metadata(&canonical)?.identity != FileIdentity::from(anchor_id)
        || p::reopen_metadata(
            canonical
                .parent()
                .ok_or_else(|| anyhow!("canonical parent absent"))?,
        )?
        .identity
            != root.metadata().identity
    {
        return Err(anyhow!("torn binding anchor final path changed"));
    }
    let logical_leaf = canonical
        .file_name()
        .ok_or_else(|| anyhow!("canonical leaf absent"))?
        .to_string_lossy()
        .into_owned();
    let record = BindingRecord {
        schema: "dendro.logical-binding".into(),
        version: 1,
        state: "migration_started".into(),
        anchor: anchor_id,
        anchor_kind: if is_marker(&anchor)? {
            "marker"
        } else {
            "legacy"
        }
        .into(),
        parent: parent_id,
        logical_leaf,
        manifest,
        writer,
        writer_identity,
        publisher,
        publisher_identity,
    };
    let (generation, expected, torn_state) = read_states_mode(&root, &hash, true)?;
    Ok(LogicalDbBinding {
        logical,
        root,
        record,
        hash,
        expected,
        generation,
        torn_state,
        torn_binding: true,
    })
}

fn matching_prepared_journals(
    binding: &LogicalDbBinding,
    resolved: &p::ResolvedSelection,
    previous: Option<Pointer>,
) -> Result<usize> {
    let current: Pointer = resolved.selection.try_into()?;
    let mut matches = 0usize;
    let mut examine_stage = |stage: &PrivateRoot| -> Result<()> {
        let mut examined = 0usize;
        for item in fs::read_dir(stage.path())? {
            RUN_CANCEL.check()?;
            let item = item?;
            let Some(journal_name) = item.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if !journal_name.starts_with("selection-journal-") || !journal_name.ends_with(".jsonl")
            {
                continue;
            }
            examined += 1;
            // A publication has at most three attempts. Bound hostile content
            // in one stage, never the number of lifetime publications.
            if examined > 16 {
                return Err(anyhow!("selection recovery stage has too many journals"));
            }
            let Ok(journal) = read_prepared_journal(stage, &journal_name) else {
                continue;
            };
            let matches_pointer = journal.get("state")
                == Some(&serde_json::Value::String("prepared".into()))
                && journal.get("manifest").and_then(|v| v.as_str())
                    == Some(binding.record.manifest.as_str())
                && journal.get("source_identity") == Some(&serde_json::to_value(current.identity)?)
                && journal.get("manifest_sha256") == Some(&serde_json::to_value(current.sha256)?)
                && journal.get("destination_identity")
                    == Some(&serde_json::to_value(binding.record.parent)?)
                && journal.get("staging_root_identity")
                    == Some(&serde_json::to_value(Identity::try_from(
                        stage.metadata().identity,
                    )?)?)
                && journal.get("writer_identity")
                    == Some(&serde_json::to_value(binding.record.writer_identity)?)
                && journal.get("publisher_identity")
                    == Some(&serde_json::to_value(binding.record.publisher_identity)?)
                && journal.get("expected_current_identity")
                    == Some(&serde_json::to_value(previous.map(|p| p.identity))?)
                && journal.get("expected_current_sha256")
                    == Some(&serde_json::to_value(previous.map(|p| p.sha256))?)
                && journal.get("bundle_identity")
                    == Some(&serde_json::to_value(
                        resolved.manifest.payload.bundle_identity,
                    )?)
                && journal
                    .get("source")
                    .and_then(|v| v.as_str())
                    .is_some_and(|leaf| name(leaf).is_ok() && !stage.path().join(leaf).exists());
            if matches_pointer {
                matches += 1;
            }
        }
        Ok(())
    };
    if let Some(stage) = recovery_stage(
        &binding.root,
        "recovery-selection",
        resolved.manifest.payload.bundle_identity.into(),
        None,
        "publication-",
    )? {
        examine_stage(&stage)?;
    } else {
        // Pre-locator stores remain recoverable. Stream their retained history
        // with constant memory; this compatibility path is linear in history.
        for entry in fs::read_dir(binding.root.path())? {
            RUN_CANCEL.check()?;
            let entry = entry?;
            if !entry
                .file_name()
                .to_string_lossy()
                .starts_with("publication-")
            {
                continue;
            }
            if let Ok(stage) = open_recovery_stage(&entry.path()) {
                examine_stage(&stage)?;
            }
        }
    }
    Ok(matches)
}

/// Explicit recovery for a pointer switched by the native primitive whose
/// consumer receipt was interrupted. Only its own durable prepared journal may
/// authorize adopting the observed pointer; a self-consistent rollback cannot.
pub fn recover_logical_db(db: &Path, choice: &p::StorageRootChoice) -> Result<SelectedRead> {
    recover_logical_db_with_outcome(db, choice).map(|(read, _)| read)
}

pub fn recover_logical_db_with_outcome(
    db: &Path,
    choice: &p::StorageRootChoice,
) -> Result<(SelectedRead, SelectionRecoveryOutcome)> {
    let (root, logical, leaf) = root_and_leaf(db, choice)?;
    let anchor = p::open_existing_private_read_pin(&root, &leaf)?;
    let marker = is_marker(&anchor)?;
    let anchor_id: Identity = p::metadata_from_handle(&anchor)?.identity.try_into()?;
    let (_, binding_leaf, manifest_leaf, writer_leaf, publisher_leaf) = binding_names(anchor_id);
    if optional_file(&root, &binding_leaf)?.is_none() {
        refuse_displaced_binding(&root, &logical, Some(anchor_id.into()))?;
        if !marker
            && optional_file(&root, &manifest_leaf)?.is_none()
            && optional_file(&root, &writer_leaf)?.is_none()
            && optional_file(&root, &publisher_leaf)?.is_none()
        {
            return Err(RecoveryDisposition::NothingToRecover.into());
        }
        if optional_file(&root, &manifest_leaf)?.is_none() {
            return Err(RecoveryDisposition::ResumeRequired.into());
        }
        // A crash during torn-binding repair can leave this leaf absent. The
        // same canonical locks, anchor and unique prepared journal are required
        // below before restoring it; absence alone grants no authority.
    }
    let binding = load_recovery_binding(root, logical, anchor)?;
    if binding.expected.is_none()
        && binding.torn_state.is_none()
        && !binding.torn_binding
        && optional_file(&binding.root, &binding.record.manifest)?.is_none()
    {
        return Err(RecoveryDisposition::ResumeRequired.into());
    }
    let writer = name(&binding.record.writer)?;
    let publisher = name(&binding.record.publisher)?;
    let writer_id =
        p::canonical_identity_from_persisted_record(binding.record.writer_identity.into());
    let publisher_id =
        p::canonical_identity_from_persisted_record(binding.record.publisher_identity.into());
    let mut outcome = SelectionRecoveryOutcome::Unchanged;
    p::with_exclusive_lock(&binding.root, &writer, &writer_id, |_| {
        p::with_exclusive_lock(&binding.root, &publisher, &publisher_id, |_| {
            Ok((|| -> Result<()> {
                // The pre-lock snapshot is only a lock-address hint. Reopen the
                // anchor and state under both canonical sentinels.
                let (root, logical, leaf) = root_and_leaf(&binding.logical, choice)?;
                let anchor = p::open_existing_private_read_pin(&root, &leaf)?;
                let current_binding = load_recovery_binding(root, logical, anchor)?;
                if current_binding.record.anchor != binding.record.anchor
                    || current_binding.record.parent != binding.record.parent
                    || current_binding.record.writer_identity != binding.record.writer_identity
                    || current_binding.record.publisher_identity
                        != binding.record.publisher_identity
                {
                    return Err(anyhow!("selection binding changed before recovery locks"));
                }
                let resolved = p::resolve_selection(
                    &current_binding.root,
                    current_binding.record.parent.into(),
                    &name(&current_binding.record.manifest)?,
                )?;
                validate_bundle(&resolved)?;
                let pointer_is_expected = current_binding.expected.is_some_and(|old| {
                    old.identity == resolved.selection.identity
                        && old.sha256 == resolved.selection.sha256
                });
                if pointer_is_expected
                    && !current_binding.torn_binding
                    && current_binding.torn_state.is_none()
                {
                    return Ok(());
                }
                let previous = if pointer_is_expected && current_binding.generation > 0 {
                    let last: PublicationState = read_record(
                        &current_binding.root,
                        &state_name(&current_binding.hash, current_binding.generation),
                    )?;
                    last.previous
                } else {
                    current_binding
                        .expected
                        .map(Pointer::try_from)
                        .transpose()?
                };
                if pointer_is_expected && current_binding.torn_state.is_some() {
                    return Err(anyhow!(
                        "torn top state with unchanged pointer requires manual recovery"
                    ));
                }
                let prepared_matches = if !pointer_is_expected || current_binding.torn_binding {
                    Some(matching_prepared_journals(
                        &current_binding,
                        &resolved,
                        previous,
                    )?)
                } else {
                    None
                };
                if !pointer_is_expected && prepared_matches != Some(1) {
                    return Err(anyhow!("selection pointer has no unique matching prepared journal; manual recovery required"));
                }
                if current_binding.torn_binding && prepared_matches != Some(1) {
                    return Err(anyhow!(
                        "torn binding has no unique matching prepared journal"
                    ));
                }
                if current_binding.torn_binding {
                    let leaf = format!("binding-{}.json", current_binding.hash);
                    if optional_file(&current_binding.root, &leaf)?.is_some() {
                        quarantine_torn_record(&current_binding.root, &leaf)?;
                    }
                    write_record(&current_binding.root, &leaf, &current_binding.record)?;
                }
                if let Some(torn) = &current_binding.torn_state {
                    quarantine_torn_record(&current_binding.root, torn)?;
                }
                if !pointer_is_expected {
                    append_state(&current_binding, resolved.selection)?;
                }
                outcome = SelectionRecoveryOutcome::Reconciled;
                Ok(())
            })())
        })
    })??;
    match resolve_logical_db(&binding.logical, choice)? {
        SelectedDb::Selected(read) => Ok((*read, outcome)),
        _ => Err(anyhow!(
            "selection recovery did not produce a selected bundle"
        )),
    }
}

fn publish_candidate(
    binding: &LogicalDbBinding,
    bundle: &PrivateRoot,
    files: &[p::ClosedBundleFile],
) -> Result<SelectedRead> {
    migration::validate_migration_candidate(bundle, files)?;
    let staging = p::create_unique_private_directory(&binding.root, "publication-")?;
    record_recovery_stage(
        &binding.root,
        "recovery-selection",
        bundle.metadata().identity,
        None,
        &staging,
    ).with_context(|| format!("recovery locator write failed; candidate retained at {} and publication stage retained at {}", bundle.path().display(), staging.path().display()))?;
    let request = p::PublicationRequest {
        destination: &binding.root,
        expected_destination: binding.record.parent.into(),
        staging: &staging,
        expected_staging: staging.metadata().identity,
        bundle,
        expected_bundle: bundle.metadata().identity,
        files,
        manifest: name(&binding.record.manifest)?,
        expected_current: binding.expected,
        writer: p::PublicationSentinel {
            name: name(&binding.record.writer)?,
            expected: p::canonical_identity_from_persisted_record(
                binding.record.writer_identity.into(),
            ),
        },
        publisher: p::PublicationSentinel {
            name: name(&binding.record.publisher)?,
            expected: p::canonical_identity_from_persisted_record(
                binding.record.publisher_identity.into(),
            ),
        },
    };
    let receipt = match p::publish_closed_bundle(&request) {
        Ok(receipt) => receipt,
        Err(failure) => {
            if failure.mutation == p::SelectionMutation::ReplacedUnconfirmed {
                let observed = p::resolve_selection(
                    &binding.root,
                    binding.record.parent.into(),
                    &name(&binding.record.manifest)?,
                );
                let classification = match observed {
                    Ok(ref resolved)
                        if binding.expected.is_some_and(|old| {
                            old.identity == resolved.selection.identity
                                && old.sha256 == resolved.selection.sha256
                        }) =>
                    {
                        "old"
                    }
                    Ok(ref resolved) => {
                        let ours = failure.retained.iter().any(|attempt| {
                            if attempt.source_identity != resolved.selection.identity
                                || !attempt.journal_durable
                            {
                                return false;
                            }
                            let journal = read_prepared_journal(&staging, &attempt.journal_name);
                            journal.ok().is_some_and(|record| {
                                record.get("manifest_sha256")
                                    == serde_json::to_value(resolved.selection.sha256)
                                        .ok()
                                        .as_ref()
                            })
                        });
                        if ours {
                            "new"
                        } else {
                            "foreign"
                        }
                    }
                    Err(_) => "invalid",
                };
                return Err(anyhow!("selection replacement uncertain; resolved pointer: {classification}; candidate retained at {}; run recovery before another write: {failure}", bundle.path().display()));
            }
            return Err(anyhow!(
                "selection unchanged; recoverable candidate retained at {}: {failure}",
                bundle.path().display()
            ));
        }
    };
    let selected = p::ExpectedSelection {
        identity: receipt.manifest_identity,
        sha256: receipt.manifest_sha256,
    };
    append_state(binding, selected)
        .context("selection switched but custody state is incomplete; recovery required")?;
    match resolve_logical_db(
        &binding.logical,
        &p::StorageRootChoice::Explicit(binding.root.path().to_path_buf()),
    )? {
        SelectedDb::Selected(read) if read.selection().identity == selected.identity && read.selection().sha256 == selected.sha256 => Ok(*read),
        SelectedDb::Selected(_) => Err(anyhow!("selection advanced after this publication; exact selected version is no longer current")),
        _ => Err(anyhow!(
            "selection switched but did not resolve; recovery required"
        )),
    }
}

/// Writer path: build a private candidate under the canonical writer sentinel,
/// release that lock and every writable DB handle, then publish with CAS.
pub fn run_locked_with_selection<F>(
    db: &Path,
    choice: &p::StorageRootChoice,
    source_roots: &[PathBuf],
    openrig_roots: &[PathBuf],
    ingest: F,
) -> Result<serde_json::Value>
where
    F: FnOnce(&Path, &[u8]) -> Result<Vec<String>>,
{
    run_locked_with_selection_receipt(db, choice, source_roots, openrig_roots, None, ingest)
        .map(|(report, _)| report)
}

/// Returns the exact selection published by this run. `required_source` is
/// verified under the canonical writer lock against the version being copied.
pub fn run_locked_with_selection_receipt<F>(
    db: &Path,
    choice: &p::StorageRootChoice,
    source_roots: &[PathBuf],
    openrig_roots: &[PathBuf],
    required_source: Option<p::ExpectedSelection>,
    ingest: F,
) -> Result<(serde_json::Value, p::ExpectedSelection)>
where
    F: FnOnce(&Path, &[u8]) -> Result<Vec<String>>,
{
    run_locked_with_destination_expectation(
        db,
        choice,
        source_roots,
        openrig_roots,
        required_source.map(Some),
        ingest,
    )
}

/// `Some(None)` pins an absent destination selection as firmly as
/// `Some(Some(selection))` pins a present one. Sealed replay uses this so a
/// concurrent first publication cannot silently become its input.
pub(crate) fn run_locked_with_destination_expectation<F>(
    db: &Path,
    choice: &p::StorageRootChoice,
    source_roots: &[PathBuf],
    openrig_roots: &[PathBuf],
    destination_expectation: Option<Option<p::ExpectedSelection>>,
    ingest: F,
) -> Result<(serde_json::Value, p::ExpectedSelection)>
where
    F: FnOnce(&Path, &[u8]) -> Result<Vec<String>>,
{
    crate::pipeline::refuse_secret_path(db)?;
    for source in source_roots.iter().chain(openrig_roots) {
        crate::pipeline::refuse_secret_path(source)?;
    }
    output::refuse_aliased_output_with_openrig(db, source_roots, openrig_roots)?;
    match resolve_logical_db(db, choice) {
        Ok(SelectedDb::Selected(read)) => {
            crate::pipeline::refuse_bound_openrig_root(&read.database, openrig_roots)?
        }
        Ok(SelectedDb::Legacy(read)) => {
            crate::pipeline::refuse_bound_openrig_root(&read.database, openrig_roots)?
        }
        Ok(SelectedDb::Uninitialized) => {}
        Err(error) if can_resume_unpublished_binding(db, choice)? => {
            let _ = error;
        }
        Err(error) => return Err(error),
    }
    let binding = admit_for_write(db, choice)?;
    let mut ingest = Some(ingest);
    let mut build_candidate = || -> Result<_> {
        p::with_exclusive_lock(
            &binding.root,
            &name(&binding.record.writer)?,
            &p::canonical_identity_from_persisted_record(binding.record.writer_identity.into()),
            |_| {
                Ok((|| -> Result<_> {
                    let current_binding = reopen_binding(&binding)?;
                    if let Some(required) = destination_expectation {
                        if current_binding
                            .expected
                            .map(|old| (old.identity, old.sha256))
                            != required.map(|expected| (expected.identity, expected.sha256))
                        {
                            return Err(anyhow!(
                                "sealed replay source selection changed before writer copy"
                            ));
                        }
                    }
                    let selected_source = if current_binding.expected.is_some() {
                        Some(selected_from_binding(reopen_binding(&current_binding)?)?)
                    } else {
                        None
                    };
                    let source = selected_source
                        .as_ref()
                        .map(|s| s.database.as_path())
                        .or_else(|| {
                            (current_binding.record.anchor_kind == "legacy")
                                .then_some(current_binding.logical.as_path())
                        });
                    if let Some(source) = &source {
                        super::pipeline::refuse_bound_openrig_root(source, openrig_roots)?;
                        if selected_source.is_none() {
                            output::refuse_existing_output_wal(source)?;
                        }
                    }
                    let key = current_key(&current_binding, selected_source.as_ref())?;
                    if let Some(source) = &source {
                        let old = AnalyticsStore::open_read_only(source)?;
                        if old.key_id != key.id {
                            return Err(anyhow!("analytics key id does not match the sidecar"));
                        }
                    }
                    let stage = Stage::create(current_binding.root.path())?;
                    let recovery = lifecycle::recover_stale(&stage)?;
                    let staged = stage.path().join("analytics.duckdb");
                    if let Some(source) = &source {
                        lifecycle::private_copy(source, &staged, &RUN_CANCEL)?;
                    } else {
                        let store = AnalyticsStore::open_path(&staged, &key.id)?;
                        store.checkpoint()?;
                    }
                    let notes = ingest.take().expect("one writer closure")(&staged, &key.bytes)?;
                    RUN_CANCEL.check()?;
                    output::refuse_existing_output_wal(&staged)?;
                    let store = AnalyticsStore::open_staged(&staged)?;
                    store.checkpoint()?;
                    drop(store);
                    super::pipeline::sync_staged_output(&stage)?;
                    RUN_CANCEL.check()?;
                    let key_source = selected_source
                        .as_ref()
                        .map(|s| s.key.clone())
                        .unwrap_or_else(|| key_path(&current_binding.logical));
                    let (bundle, mut files) =
                        make_bundle(&current_binding, &staged, &key_source, &key)?;
                    if let Some(selected) = selected_source.as_ref() {
                        migration::carry_migration_custody(selected, &bundle, &mut files)?;
                    }
                    stage.finish()?;
                    Ok((bundle, files, notes, recovery, current_binding))
                })())
            },
        )?
    };
    let built = if binding.record.anchor_kind == "legacy" && binding.expected.is_none() {
        output::with_output_lock(
            &binding.root.path().join(".dendro-output.lock"),
            build_candidate,
        )?
    } else {
        build_candidate()?
    };
    let (bundle, files, notes, recovered, binding) = built;
    RUN_CANCEL.check().context(format!(
        "publication cancelled; recoverable candidate retained at {}",
        bundle.path().display()
    ))?;
    let read = publish_candidate(&binding, &bundle, &files)?;
    let published = read.selection();
    Ok((
        serde_json::json!({"status":"ready", "product":"AI Fleet Intelligence", "key_id": selected_key(&read)?.id, "sources":notes, "staging":recovered}),
        published,
    ))
}

/// Selected derived writes use the same closed-candidate and pointer-CAS path.
/// The supplied snapshot is the expected old version; a later selection refuses.
pub fn mutate_selected<F>(selected: &SelectedRead, mutate: F) -> Result<SelectedRead>
where
    F: FnOnce(&Path, &[u8]) -> Result<()>,
{
    let binding = reopen_binding(&selected.binding)?;
    if binding.expected.map(|s| (s.identity, s.sha256))
        != Some((selected.selection().identity, selected.selection().sha256))
    {
        return Err(anyhow!("stale selection; retry materialization"));
    }
    let key = selected_key(selected)?;
    let (bundle, files) = p::with_exclusive_lock(
        &binding.root,
        &name(&binding.record.writer)?,
        &p::canonical_identity_from_persisted_record(binding.record.writer_identity.into()),
        |_| {
            Ok((|| -> Result<_> {
                let stage = Stage::create(binding.root.path())?;
                let staged = stage.path().join("analytics.duckdb");
                lifecycle::private_copy(&selected.database, &staged, &RUN_CANCEL)?;
                mutate(&staged, &key.bytes)?;
                let store = AnalyticsStore::open_staged(&staged)?;
                store.checkpoint()?;
                drop(store);
                super::pipeline::sync_staged_output(&stage)?;
                let (bundle, mut files) = make_bundle(&binding, &staged, &selected.key, &key)?;
                migration::carry_migration_custody(selected, &bundle, &mut files)?;
                let result = (bundle, files);
                stage.finish()?;
                Ok(result)
            })())
        },
    )??;
    RUN_CANCEL.check().context(format!(
        "derived publication cancelled; recoverable candidate retained at {}",
        bundle.path().display()
    ))?;
    publish_candidate(&binding, &bundle, &files)
}

/// Adopt an admitted legacy database through a no-op closed publication.
/// The legacy anchor remains byte-for-byte and only the selected copy may be mutated.
pub fn adopt_legacy(db: &Path, choice: &p::StorageRootChoice) -> Result<SelectedRead> {
    match resolve_logical_db(db, choice)? {
        SelectedDb::Selected(selected) => return Ok(*selected),
        SelectedDb::Uninitialized => return Err(anyhow!("no legacy database to adopt")),
        SelectedDb::Legacy(_) => {}
    }
    let (_, published) =
        run_locked_with_selection_receipt(db, choice, &[], &[], None, |_staged, _key| {
            Ok(Vec::new())
        })?;
    match resolve_logical_db(db, choice)? {
        SelectedDb::Selected(selected)
            if selected.selection().identity == published.identity
                && selected.selection().sha256 == published.sha256 =>
        {
            Ok(*selected)
        }
        SelectedDb::Selected(_) => Err(anyhow!("selection advanced after legacy adoption; retry")),
        _ => Err(anyhow!(
            "legacy adoption did not publish a selected database"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copied_candidate_key_must_match_previously_decoded_key_bytes() {
        let root = p::create_user_private_root_with_choice(
            "dendro-key-copy-test-",
            &p::StorageRootChoice::UserProfile,
        )
        .unwrap();
        let logical = root.path().join("analytics.duckdb");
        let choice = p::StorageRootChoice::Explicit(root.path().to_path_buf());
        let binding = admit_for_write(&logical, &choice).unwrap();
        let expected = output::load_or_create_key(&key_path(&logical), true).unwrap();
        let other = root.path().join("other.key");
        let foreign = output::load_or_create_key(&other, true).unwrap();
        assert_ne!(expected.id, foreign.id);
        let staged = root.path().join("candidate.duckdb");
        let store = AnalyticsStore::open_path(&staged, &expected.id).unwrap();
        store.checkpoint().unwrap();
        drop(store);
        let error = make_bundle(&binding, &staged, &other, &expected)
            .err()
            .unwrap();
        assert!(
            error.to_string().contains("copied key differs"),
            "{error:#}"
        );
        assert!(optional_file(&root, &binding.record.manifest)
            .unwrap()
            .is_none());
    }

    #[test]
    fn sealed_replay_absent_destination_expectation_refuses_concurrent_first_publication() {
        let root = p::create_user_private_root_with_choice(
            "dendro-absent-race-",
            &p::StorageRootChoice::UserProfile,
        )
        .unwrap();
        let logical = root.path().join("analytics.duckdb");
        let choice = p::StorageRootChoice::Explicit(root.path().to_path_buf());
        run_locked_with_selection(&logical, &choice, &[], &[], |staged, _| {
            let store = AnalyticsStore::open_staged(staged)?;
            store.checkpoint()?;
            Ok(Vec::new())
        })
        .unwrap();
        let mut entered = false;
        let error = run_locked_with_destination_expectation(
            &logical,
            &choice,
            &[],
            &[],
            Some(None),
            |_, _| {
                entered = true;
                Ok(Vec::new())
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("selection changed"), "{error:#}");
        assert!(!entered);
    }

    #[test]
    fn custody_record_is_flushed_and_renamed_to_final_without_temp() {
        let root = p::create_user_private_root_with_choice(
            "dendro-atomic-record-",
            &p::StorageRootChoice::UserProfile,
        )
        .unwrap();
        write_record(
            &root,
            "binding-test.json",
            &serde_json::json!({"proof":"complete"}),
        )
        .unwrap();
        let record: serde_json::Value = read_record(&root, "binding-test.json").unwrap();
        assert_eq!(record["proof"], "complete");
        assert!(!fs::read_dir(root.path())
            .unwrap()
            .filter_map(Result::ok)
            .any(|entry| entry.file_name().to_string_lossy().starts_with("record-")));
    }

    #[test]
    fn recovery_history_uses_one_stage_and_legacy_history_has_no_lifetime_limit() {
        let root = p::create_user_private_root_with_choice(
            "dendro-history-recovery-",
            &p::StorageRootChoice::UserProfile,
        )
        .unwrap();
        let choice = p::StorageRootChoice::Explicit(root.path().to_path_buf());
        let logical = root.path().join("analytics.duckdb");
        run_locked_with_selection(&logical, &choice, &[], &[], |staged, _| {
            let store = AnalyticsStore::open_staged(staged)?;
            store.checkpoint()?;
            Ok(vec![])
        })
        .unwrap();
        let find = |prefix: &str| {
            fs::read_dir(root.path())
                .unwrap()
                .map(|e| e.unwrap().path())
                .find(|p| p.file_name().unwrap().to_string_lossy().starts_with(prefix))
                .unwrap()
        };
        let state = find("selected-");
        let locator = find("recovery-selection-");
        let state_bytes = fs::read(&state).unwrap();
        for ordinal in 0..4100 {
            let stage = root
                .path()
                .join(format!("publication-history-{ordinal:04}"));
            fs::create_dir(&stage).unwrap();
            fs::write(
                stage.join("selection-journal-old.jsonl"),
                match ordinal % 3 {
                    0 => b"{\"state\":\"prepared\"}".as_slice(),
                    1 => b"{\"state\":\"published\"}",
                    _ => b"{",
                },
            )
            .unwrap();
        }
        fs::remove_file(&state).unwrap();
        RECOVERY_STAGE_OPENS.with(|n| n.set(0));
        let (read, outcome) = recover_logical_db_with_outcome(&logical, &choice).unwrap();
        assert_eq!(outcome, SelectionRecoveryOutcome::Reconciled);
        assert!(read.is_current().unwrap());
        assert_eq!(RECOVERY_STAGE_OPENS.with(|n| n.get()), 1);
        drop(read);
        assert_eq!(fs::read(&state).unwrap(), state_bytes);
        fs::remove_file(&state).unwrap();
        fs::remove_file(locator).unwrap();
        RECOVERY_STAGE_OPENS.with(|n| n.set(0));
        recover_logical_db(&logical, &choice).unwrap();
        assert_eq!(RECOVERY_STAGE_OPENS.with(|n| n.get()), 4101);
        assert_eq!(
            fs::read_dir(root.path())
                .unwrap()
                .filter_map(Result::ok)
                .filter(|e| e
                    .file_name()
                    .to_string_lossy()
                    .starts_with("publication-history-"))
                .count(),
            4100
        );
    }
}
