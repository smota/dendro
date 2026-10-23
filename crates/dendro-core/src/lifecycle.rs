//! Private staging and snapshot lifecycle (S1-S3).
//!
//! A run works inside one private stage directory next to its output, under the
//! output lock. The stage is created as `.dendro-stage-<16 hex>.init` (mode
//! 0700), receives a fixed marker file, and is then renamed to its final name,
//! so every final-name stage carries the marker. Raw source snapshots live in
//! the stage's `snapshot/` directory. Files Dendro creates are 0600 from the
//! creating syscall; nothing is copied with permission-preserving `fs::copy`
//! and the process umask is never changed.
//!
//! Windows atomically creates its unique final directory with a protected
//! private descriptor, then writes the marker before any payload.
//! A crash before that marker leaves an unmarked directory for manual inspection.
//! Windows retains full volume plus 128-bit file identity and pinned ancestors.
//!
//! A run removes its own stage explicitly and reports a failure. A later run,
//! holding the same lock, removes stale stages left by a killed run, but only
//! when the stage is a real directory we own, carries a valid marker and holds
//! allowlisted entries. Anything else is kept and reported.

#[cfg(windows)]
use crate::platform_v1::{self as platform, FileIdentity, PrivateRoot};
use std::fs::{self, File};
#[cfg(unix)]
use std::fs::{DirBuilder, OpenOptions};
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{anyhow, Result};

pub const STAGE_PREFIX: &str = ".dendro-stage-";
pub const MARKER_NAME: &str = ".dendro-stage";
pub const MARKER: &[u8] = b"dendro-stage v1\n";
pub const SNAPSHOT_DIR: &str = "snapshot";
const STAGED_FILES: &[&str] = &["analytics.duckdb", "analytics.duckdb.wal"];
const SNAPSHOT_FILES: &[&str] = &["openrig.sqlite", "openrig.sqlite-wal", "openrig.sqlite-shm"];
const COPY_CHUNK: usize = 1 << 20;

/// Cooperative cancellation. Signals only set this flag; the worker checks it
/// per source line, per row and per copy chunk and returns [`Interrupted`].
pub struct Cancel(AtomicBool);

impl Cancel {
    pub const fn new() -> Self {
        Self(AtomicBool::new(false))
    }

    pub fn request(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn requested(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    pub fn check(&self) -> Result<()> {
        if self.requested() {
            Err(Interrupted.into())
        } else {
            Ok(())
        }
    }
}

impl Default for Cancel {
    fn default() -> Self {
        Self::new()
    }
}

/// The run's cancellation flag, set by the CLI's signal listeners.
pub static RUN_CANCEL: Cancel = Cancel::new();

/// The worker stopped because cancellation was requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Interrupted;

impl std::fmt::Display for Interrupted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("interrupted")
    }
}

impl std::error::Error for Interrupted {}

/// Creates one directory with mode 0700 from the creating syscall. Fails if
/// the path exists, including as a symlink.
#[cfg(unix)]
pub fn create_private_dir(path: &Path) -> Result<()> {
    DirBuilder::new().mode(0o700).create(path)?;
    Ok(())
}

/// Creates a new file with mode 0600 from the creating syscall. Fails if the
/// path exists, including as a symlink (O_CREAT | O_EXCL).
#[cfg(unix)]
pub fn create_private_file(path: &Path) -> Result<File> {
    Ok(OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?)
}

#[cfg(windows)]
fn private_parent(path: &Path) -> Result<(PrivateRoot, platform::ChildName)> {
    let absolute = std::path::absolute(path)?;
    let parent = absolute
        .parent()
        .ok_or_else(|| anyhow!("private child has no parent"))?;
    let name = absolute
        .file_name()
        .ok_or_else(|| anyhow!("private child has no name"))?;
    let name = platform::parse_child_os(name, platform::NativePlatform::Windows)
        .map_err(platform::PlatformError::Rejected)?;
    Ok((platform::open_existing_private_root(parent)?, name))
}

/// Creates an exact child with its private security descriptor in the creating syscall.
#[cfg(windows)]
pub fn create_private_dir(path: &Path) -> Result<()> {
    let (parent, name) = private_parent(path)?;
    platform::create_private_directory(&parent, &name)?;
    Ok(())
}

/// Allocate a snapshot directory and return the actual path. Windows uses a
/// private descriptor at creation and a random leaf under the held parent.
#[cfg(unix)]
pub fn create_private_snapshot_dir(requested: &Path) -> Result<PathBuf> {
    create_private_dir(requested)?;
    Ok(requested.to_path_buf())
}

#[cfg(windows)]
pub fn create_private_snapshot_dir(requested: &Path) -> Result<PathBuf> {
    let (parent, name) = private_parent(requested)?;
    if name.as_str() != SNAPSHOT_DIR {
        return Err(anyhow!("snapshot directory must use the snapshot role"));
    }
    let root = platform::create_unique_private_directory(&parent, "snapshot-")?;
    Ok(root.path().to_path_buf())
}

#[cfg(windows)]
fn snapshot_kind(name: &str) -> bool {
    name == SNAPSHOT_DIR
        || name.strip_prefix("snapshot-").is_some_and(|id| {
            id.len() == 16
                && id
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
}

/// A newly created Windows file together with its held private parent.
/// Keeping this wrapper alive retains ancestor custody for every caller,
/// including standalone exports that have no surrounding `Stage`.
#[cfg(windows)]
pub struct PrivateFile {
    file: File,
    _parent: PrivateRoot,
}

#[cfg(windows)]
impl PrivateFile {
    pub fn sync_all(&self) -> std::io::Result<()> {
        self.file.sync_all()
    }
}

#[cfg(windows)]
impl Write for PrivateFile {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.file.write(bytes)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

#[cfg(windows)]
pub fn create_private_file(path: &Path) -> Result<PrivateFile> {
    let (parent, name) = private_parent(path)?;
    let file = parent.create_file(&name)?.into_file();
    Ok(PrivateFile {
        file,
        _parent: parent,
    })
}

/// Copies `src` into a new 0600 file at `dst`, in chunks, checking `cancel`.
pub fn private_copy(src: &Path, dst: &Path, cancel: &Cancel) -> Result<()> {
    let mut input = File::open(src)?;
    let mut output = create_private_file(dst)?;
    let mut buffer = vec![0u8; COPY_CHUNK];
    loop {
        cancel.check()?;
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        output.write_all(&buffer[..read])?;
    }
    output.sync_all()?;
    Ok(())
}

/// Device and inode of a path, without following a final symlink.
#[cfg(unix)]
fn identity(path: &Path) -> Result<(u64, u64)> {
    let meta = fs::symlink_metadata(path)?;
    Ok((meta.dev(), meta.ino()))
}

/// One run's private stage, created inside the locked output parent.
pub struct Stage {
    path: PathBuf,
    #[cfg(unix)]
    uid: u32,
    parent: PathBuf,
    #[cfg(unix)]
    parent_identity: (u64, u64),
    #[cfg(windows)]
    parent_root: PrivateRoot,
    #[cfg(windows)]
    stage_root: Option<PrivateRoot>,
    #[cfg(windows)]
    stage_identity: FileIdentity,
    active: bool,
}

impl Stage {
    /// Allocates `.init`, writes the marker, then publishes the final name.
    #[cfg(unix)]
    pub fn create(parent: &Path) -> Result<Stage> {
        let parent_identity = identity(parent)?;
        let id = random_id();
        let init = parent.join(format!("{STAGE_PREFIX}{id}.init"));
        let path = parent.join(format!("{STAGE_PREFIX}{id}"));
        create_private_dir(&init)?;
        let meta = fs::symlink_metadata(&init)?;
        let uid = meta.uid();
        let mut marker = create_private_file(&init.join(MARKER_NAME))?;
        marker.write_all(MARKER)?;
        marker.sync_all()?;
        drop(marker);
        if fs::symlink_metadata(&path).is_ok() {
            return Err(anyhow!("staging name collision"));
        }
        fs::rename(&init, &path)?;
        crate::output::sync_dir(parent)?;
        Ok(Stage {
            path,
            uid,
            parent: parent.to_path_buf(),
            parent_identity,
            active: true,
        })
    }

    /// Windows pins forbid renaming held directories. Allocate the unique
    /// final name privately, then write and sync the marker before any payload.
    /// An interrupted unmarked initialization is retained by stale recovery.
    #[cfg(windows)]
    pub fn create(parent: &Path) -> Result<Stage> {
        let parent = std::path::absolute(parent)?;
        let parent_root = platform::open_existing_private_root(&parent)?;
        let root = platform::create_unique_private_directory(&parent_root, STAGE_PREFIX)?;
        let path = root.path().to_path_buf();
        let stage_identity = root.metadata().identity;
        let mut stage = Stage {
            path,
            parent,
            parent_root,
            stage_root: Some(root),
            stage_identity,
            active: false,
        };
        let mut marker = create_private_file(&stage.path.join(MARKER_NAME))?;
        marker.write_all(MARKER)?;
        marker.sync_all()?;
        drop(marker);
        // Only a fully written marker grants this run cleanup custody.
        stage.active = true;
        stage
            .stage_root
            .as_ref()
            .expect("new stage retains its root")
            .sync_dir()?;
        stage.parent_root.sync_dir()?;
        Ok(stage)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    #[cfg(unix)]
    pub fn uid(&self) -> u32 {
        self.uid
    }

    /// Refuses when the output parent was replaced since the stage was made.
    #[cfg(unix)]
    pub fn check_anchor(&self) -> Result<()> {
        if identity(&self.parent)? != self.parent_identity {
            return Err(anyhow!("analytics output directory changed during the run"));
        }
        Ok(())
    }

    #[cfg(windows)]
    pub fn check_anchor(&self) -> Result<()> {
        self.parent_root.check_anchor()?;
        let meta = platform::reopen_metadata(&self.path)?;
        if meta.identity != self.stage_identity || meta.kind != platform::ObjectKind::Directory {
            return Err(anyhow!(
                "analytics staging directory changed during the run"
            ));
        }
        Ok(())
    }

    #[cfg(windows)]
    fn release_stage_pin(&mut self) -> Result<()> {
        self.check_anchor()?;
        self.stage_root.take();
        Ok(())
    }

    /// Removes the stage. A failure is returned, never swallowed.
    pub fn finish(mut self) -> Result<()> {
        self.active = false;
        #[cfg(windows)]
        self.release_stage_pin()?;
        remove_stage(&self.path)
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        if self.active {
            #[cfg(windows)]
            if self.release_stage_pin().is_err() {
                eprintln!(
                    "dendro: staging cleanup anchor changed: {}",
                    stage_name(&self.path)
                );
                return;
            }
            if let Err(_error) = remove_stage(&self.path) {
                eprintln!("dendro: staging cleanup failed: {}", stage_name(&self.path));
            }
        }
    }
}

fn remove_stage(path: &Path) -> Result<()> {
    remove_marker_last(path).map_err(|_| anyhow!("staging cleanup failed: {}", stage_name(path)))
}

/// Removes a stage's contents first and its marker last, then the empty
/// directory. The marker is removed only after every other entry is gone, so a
/// content-removal failure leaves a marked stage that the next run can recover,
/// whatever order the directory lists its entries in. If the final empty-directory
/// removal fails after marker deletion, recovery conservatively keeps that
/// unmarked directory. Used by finish, Drop fallback and stale recovery.
fn remove_marker_last(path: &Path) -> std::io::Result<()> {
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let mut failure = None;
    for entry in entries {
        let entry = entry?;
        if entry.file_name() == MARKER_NAME {
            continue;
        }
        let child = entry.path();
        // Never follows a symlink: a link is removed, not its target.
        let removed = match fs::symlink_metadata(&child) {
            Ok(meta) if meta.file_type().is_dir() => fs::remove_dir_all(&child),
            Ok(_) => fs::remove_file(&child),
            Err(error) => Err(error),
        };
        if let Err(error) = removed {
            failure = Some(error);
        }
    }
    if let Some(error) = failure {
        return Err(error);
    }
    match fs::remove_file(path.join(MARKER_NAME)) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    fs::remove_dir(path)
}

/// The stage's own name; Dendro-generated, so it carries no source text.
pub fn stage_name(path: &Path) -> String {
    path.file_name()
        .map(|name| crate::privacy::redact(&name.to_string_lossy()))
        .unwrap_or_default()
}

#[cfg(unix)]
fn random_id() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 8];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// `Some(true)` for a `.init` name, `Some(false)` for a final name, `None` for
/// anything outside the exact stage namespace.
fn stage_kind(name: &str) -> Option<bool> {
    let rest = name.strip_prefix(STAGE_PREFIX)?;
    let (id, init) = match rest.strip_suffix(".init") {
        Some(id) => (id, true),
        None => (rest, false),
    };
    let exact = id.len() == 16
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    exact.then_some(init)
}

#[cfg(unix)]
fn private_regular_file(path: &Path, uid: u32, mode: Option<u32>) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| {
        meta.file_type().is_file()
            && meta.uid() == uid
            && meta.nlink() == 1
            && mode.is_none_or(|mode| meta.mode() & 0o777 == mode)
    })
}

#[cfg(unix)]
fn private_dir(path: &Path, uid: u32) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| {
        meta.file_type().is_dir() && meta.uid() == uid && meta.mode() & 0o777 == 0o700
    })
}

#[cfg(unix)]
fn valid_marker(dir: &Path, uid: u32) -> bool {
    let path = dir.join(MARKER_NAME);
    private_regular_file(&path, uid, Some(0o600))
        && fs::read(&path).is_ok_and(|bytes| bytes == MARKER)
}

/// Why a stale-looking entry must be kept, or None when it may be removed.
#[cfg(unix)]
fn keep_reason(path: &Path, init: bool, uid: u32) -> Option<&'static str> {
    if !private_dir(path, uid) {
        return Some("not a private directory owned by this user");
    }
    if !valid_marker(path, uid) {
        return Some("no valid Dendro marker");
    }
    let Ok(entries) = fs::read_dir(path) else {
        return Some("unreadable");
    };
    for entry in entries {
        let Ok(entry) = entry else {
            return Some("unreadable");
        };
        let name = entry.file_name().to_string_lossy().to_string();
        let child = entry.path();
        let allowed = if name == MARKER_NAME {
            true
        } else if init {
            false
        } else if STAGED_FILES.contains(&name.as_str()) {
            private_regular_file(&child, uid, None)
        } else if name == SNAPSHOT_DIR {
            private_dir(&child, uid)
                && fs::read_dir(&child).is_ok_and(|inner| {
                    inner.flatten().all(|file| {
                        SNAPSHOT_FILES.contains(&file.file_name().to_string_lossy().as_ref())
                            && private_regular_file(&file.path(), uid, Some(0o600))
                    })
                })
        } else {
            false
        };
        if !allowed {
            return Some("unexpected contents");
        }
    }
    None
}

/// Removes stale stages in `parent` that a killed or crashed run left behind.
/// Call only while holding the output lock, so no live run owns them. Returns
/// one sanitized note per stage-namespace entry found, removed or kept.
#[cfg(unix)]
pub fn recover_stale(stage: &Stage) -> Result<Vec<String>> {
    stage.check_anchor()?;
    let parent = &stage.parent;
    let parent_meta = fs::metadata(parent)?;
    let shared = parent_meta.mode() & 0o002 != 0 && parent_meta.mode() & 0o1000 == 0;
    let mut notes = Vec::new();
    let mut entries: Vec<_> = fs::read_dir(parent)?.flatten().collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().to_string();
        let Some(init) = stage_kind(&name) else {
            continue;
        };
        let path = entry.path();
        if path == stage.path {
            continue;
        }
        let shown = crate::privacy::redact(&name);
        if shared {
            notes.push(format!(
                "stale staging kept: {shown} (output directory is writable by others)"
            ));
            continue;
        }
        match keep_reason(&path, init, stage.uid) {
            Some(reason) => notes.push(format!("stale staging kept: {shown} ({reason})")),
            None => match remove_marker_last(&path) {
                Ok(()) => notes.push(format!("stale staging removed: {shown}")),
                Err(_) => notes.push(format!("stale staging could not be removed: {shown}")),
            },
        }
    }
    Ok(notes)
}

/// Windows recovery admits each candidate's held private root and each leaf's
/// current-principal security descriptor, regular kind and single-link identity.
/// Foreign, reparse, malformed or partly initialized stages are left untouched.
#[cfg(windows)]
fn windows_keep_reason(root: &PrivateRoot, init: bool) -> Option<&'static str> {
    let marker_name =
        match platform::parse_child_name(MARKER_NAME, platform::NativePlatform::Windows) {
            Ok(name) => name,
            Err(_) => return Some("invalid marker name"),
        };
    let Ok(mut marker) = platform::open_existing_private_file(root, &marker_name) else {
        return Some("no private Dendro marker");
    };
    // Read only the fixed marker plus one byte; an arbitrary large file is not
    // trusted merely because it has the marker name.
    let mut bytes = Vec::with_capacity(MARKER.len() + 1);
    if marker
        .file_mut()
        .take((MARKER.len() + 1) as u64)
        .read_to_end(&mut bytes)
        .is_err()
        || bytes != MARKER
    {
        return Some("no valid Dendro marker");
    }
    let Ok(entries) = fs::read_dir(root.path()) else {
        return Some("unreadable");
    };
    for entry in entries {
        let Ok(entry) = entry else {
            return Some("unreadable");
        };
        let raw_name = entry.file_name();
        let Some(name) = raw_name.to_str() else {
            return Some("unexpected contents");
        };
        if name == MARKER_NAME {
            continue;
        }
        if init {
            return Some("unexpected contents");
        }
        if STAGED_FILES.contains(&name) {
            let Ok(name) = platform::parse_child_name(name, platform::NativePlatform::Windows)
            else {
                return Some("unexpected contents");
            };
            if platform::open_existing_private_file(root, &name).is_err() {
                return Some("unexpected contents");
            }
        } else if snapshot_kind(name) {
            let Ok(snapshot) = platform::open_existing_private_root(&entry.path()) else {
                return Some("unexpected contents");
            };
            let Ok(files) = fs::read_dir(snapshot.path()) else {
                return Some("unreadable");
            };
            for file in files {
                let Ok(file) = file else {
                    return Some("unreadable");
                };
                let raw_name = file.file_name();
                let Some(name) = raw_name.to_str() else {
                    return Some("unexpected contents");
                };
                if !SNAPSHOT_FILES.contains(&name) {
                    return Some("unexpected contents");
                }
                let Ok(name) = platform::parse_child_name(name, platform::NativePlatform::Windows)
                else {
                    return Some("unexpected contents");
                };
                if platform::open_existing_private_file(&snapshot, &name).is_err() {
                    return Some("unexpected contents");
                }
            }
        } else {
            return Some("unexpected contents");
        }
    }
    None
}

#[cfg(windows)]
pub fn recover_stale(stage: &Stage) -> Result<Vec<String>> {
    stage.check_anchor()?;
    let mut notes = Vec::new();
    let mut entries: Vec<_> = fs::read_dir(&stage.parent)?.collect::<std::io::Result<_>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().to_string();
        let Some(init) = stage_kind(&name) else {
            continue;
        };
        let path = entry.path();
        if path == stage.path {
            continue;
        }
        let shown = crate::privacy::redact(&name);
        let root = match platform::open_existing_private_root(&path) {
            Ok(root) => root,
            Err(_) => {
                notes.push(format!(
                    "stale staging kept: {shown} (not a private directory owned by this user)"
                ));
                continue;
            }
        };
        if let Some(reason) = windows_keep_reason(&root, init) {
            notes.push(format!("stale staging kept: {shown} ({reason})"));
            continue;
        }
        // The parent remains pinned. Release only the candidate pin to permit
        // deletion; same-principal/privileged namespace mutation remains the
        // declared native trust boundary, as it is for the Unix recovery path.
        root.check_anchor()?;
        let expected = root.metadata().identity;
        drop(root);
        stage.check_anchor()?;
        if platform::reopen_metadata(&path)?.identity != expected {
            notes.push(format!("stale staging kept: {shown} (identity changed)"));
            continue;
        }
        match remove_marker_last(&path) {
            Ok(()) => notes.push(format!("stale staging removed: {shown}")),
            Err(_) => notes.push(format!("stale staging could not be removed: {shown}")),
        }
    }
    Ok(notes)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dendro-life-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn mode(path: &Path) -> u32 {
        fs::symlink_metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn a_stage_is_private_marked_and_removed_by_finish() {
        let dir = scratch("stage");
        let stage = Stage::create(&dir).unwrap();
        assert_eq!(mode(stage.path()), 0o700);
        let marker = stage.path().join(MARKER_NAME);
        assert_eq!(mode(&marker), 0o600);
        assert_eq!(fs::read(&marker).unwrap(), MARKER);
        assert!(stage_kind(&stage_name(stage.path())) == Some(false));
        let path = stage.path().to_path_buf();
        stage.finish().unwrap();
        assert!(!path.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_dropped_stage_is_removed_as_a_fallback() {
        let dir = scratch("drop");
        let path = {
            let stage = Stage::create(&dir).unwrap();
            stage.path().to_path_buf()
        };
        assert!(!path.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn private_copy_is_0600_refuses_existing_targets_and_stops_on_cancel() {
        let dir = scratch("copy");
        let src = dir.join("src");
        fs::write(&src, vec![7u8; 3 * COPY_CHUNK + 5]).unwrap();
        fs::set_permissions(&src, fs::Permissions::from_mode(0o644)).unwrap();
        let dst = dir.join("dst");
        private_copy(&src, &dst, &Cancel::new()).unwrap();
        assert_eq!(mode(&dst), 0o600, "never inherits the source's 0644");
        assert_eq!(fs::read(&dst).unwrap(), fs::read(&src).unwrap());
        assert!(
            private_copy(&src, &dst, &Cancel::new()).is_err(),
            "existing target"
        );
        std::os::unix::fs::symlink(&src, dir.join("link")).unwrap();
        assert!(
            private_copy(&src, &dir.join("link"), &Cancel::new()).is_err(),
            "symlink target"
        );
        let cancel = Cancel::new();
        cancel.request();
        let error = private_copy(&src, &dir.join("stopped"), &cancel).unwrap_err();
        assert!(error.downcast_ref::<Interrupted>().is_some());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn recovery_removes_only_marked_owned_allowlisted_stages() {
        let dir = scratch("recover");
        let make = |name: &str, marker: bool| {
            let path = dir.join(name);
            create_private_dir(&path).unwrap();
            if marker {
                create_private_file(&path.join(MARKER_NAME))
                    .unwrap()
                    .write_all(MARKER)
                    .unwrap();
            }
            path
        };
        // Stale and removable: final stage with marker, staged db and snapshot.
        let stale = make(".dendro-stage-00000000000000aa", true);
        create_private_file(&stale.join("analytics.duckdb")).unwrap();
        create_private_dir(&stale.join(SNAPSHOT_DIR)).unwrap();
        create_private_file(&stale.join(SNAPSHOT_DIR).join("openrig.sqlite")).unwrap();
        // Marked partial init: removable.
        let init = make(".dendro-stage-00000000000000bb.init", true);
        // Kept: unmarked init, unknown entry, wrong marker, wrong mode, hard link, symlink, bad names.
        let unmarked = make(".dendro-stage-00000000000000cc.init", false);
        let unknown = make(".dendro-stage-00000000000000dd", true);
        fs::write(unknown.join("notes.txt"), "x").unwrap();
        let bad_marker = make(".dendro-stage-00000000000000ee", false);
        fs::write(bad_marker.join(MARKER_NAME), "other").unwrap();
        let wide = make(".dendro-stage-00000000000000ff", true);
        fs::set_permissions(&wide, fs::Permissions::from_mode(0o755)).unwrap();
        let linked = make(".dendro-stage-0000000000000aaa", true);
        create_private_file(&linked.join("analytics.duckdb")).unwrap();
        fs::hard_link(linked.join("analytics.duckdb"), dir.join("outside-link")).unwrap();
        let target = dir.join("target");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("keep"), "x").unwrap();
        std::os::unix::fs::symlink(&target, dir.join(".dendro-stage-0000000000000bbb")).unwrap();
        let not_hex = make(".dendro-stage-NOTHEX", true);
        fs::write(dir.join(".dendro-stage-notes"), "x").unwrap();
        let stage = Stage::create(&dir).unwrap();
        let notes = recover_stale(&stage).unwrap();
        assert!(!stale.exists() && !init.exists());
        for kept in [&unmarked, &unknown, &bad_marker, &wide, &linked, &not_hex] {
            assert!(kept.exists(), "{kept:?}");
        }
        assert!(target.join("keep").exists(), "symlink target untouched");
        assert!(dir.join(".dendro-stage-notes").exists());
        assert!(stage.path().exists(), "own stage untouched");
        assert_eq!(
            notes
                .iter()
                .filter(|n| n.starts_with("stale staging removed"))
                .count(),
            2
        );
        assert_eq!(
            notes
                .iter()
                .filter(|n| n.starts_with("stale staging kept"))
                .count(),
            6
        );
        stage.finish().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    fn assert_intact_marker(stage: &Path) {
        assert_eq!(mode(stage), 0o700, "stage stays private");
        let marker = stage.join(MARKER_NAME);
        assert_eq!(fs::read(&marker).unwrap(), MARKER, "marker content intact");
        assert_eq!(mode(&marker), 0o600, "marker stays 0600");
    }

    /// A stage whose snapshot cannot be emptied: its directory has no write bit.
    fn stage_with_stuck_snapshot(dir: &Path) -> Stage {
        let stage = Stage::create(dir).unwrap();
        // Entries named to sort and list on both sides of the marker.
        for name in ["analytics.duckdb", "analytics.duckdb.wal"] {
            create_private_file(&stage.path().join(name)).unwrap();
        }
        let snapshot = stage.path().join(SNAPSHOT_DIR);
        create_private_dir(&snapshot).unwrap();
        create_private_file(&snapshot.join("openrig.sqlite")).unwrap();
        fs::set_permissions(&snapshot, fs::Permissions::from_mode(0o500)).unwrap();
        stage
    }

    #[test]
    fn a_failed_cleanup_keeps_the_marker_and_the_next_run_recovers_the_stage() {
        let dir = scratch("partial");
        // Explicit finish.
        let stage = stage_with_stuck_snapshot(&dir);
        let path = stage.path().to_path_buf();
        let error = stage.finish().unwrap_err().to_string();
        assert!(
            error.starts_with("staging cleanup failed: .dendro-stage-"),
            "{error}"
        );
        assert_intact_marker(&path);
        assert!(
            !path.join("analytics.duckdb").exists(),
            "other contents may be removed"
        );
        // Drop fallback.
        let dropped_path = {
            let stage = stage_with_stuck_snapshot(&dir);
            stage.path().to_path_buf()
        };
        assert_intact_marker(&dropped_path);
        // Restore only the synthetic permission; the next run recovers both.
        for stuck in [&path, &dropped_path] {
            fs::set_permissions(stuck.join(SNAPSHOT_DIR), fs::Permissions::from_mode(0o700))
                .unwrap();
        }
        let next = Stage::create(&dir).unwrap();
        let notes = recover_stale(&next).unwrap();
        assert!(!path.exists() && !dropped_path.exists(), "{notes:?}");
        assert_eq!(
            notes
                .iter()
                .filter(|n| n.starts_with("stale staging removed"))
                .count(),
            2
        );
        // A stage recovery cannot validate (here a 0500 snapshot directory) is
        // kept with its marker intact. Recovery's removal uses the same
        // marker-last helper as finish and Drop above; failing it after
        // validation passes would need privileges this test does not use.
        let stuck = stage_with_stuck_snapshot(&dir);
        let stuck_path = stuck.path().to_path_buf();
        std::mem::forget(stuck);
        let notes = recover_stale(&next).unwrap();
        assert!(
            notes.iter().any(|n| n.starts_with("stale staging kept")),
            "{notes:?}"
        );
        assert_intact_marker(&stuck_path);
        fs::set_permissions(
            stuck_path.join(SNAPSHOT_DIR),
            fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        next.finish().unwrap();
        let last = Stage::create(&dir).unwrap();
        recover_stale(&last).unwrap();
        assert!(!stuck_path.exists());
        last.finish().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn ordinary_cleanup_still_removes_everything() {
        let dir = scratch("ordinary");
        let stage = Stage::create(&dir).unwrap();
        create_private_file(&stage.path().join("analytics.duckdb")).unwrap();
        create_private_dir(&stage.path().join(SNAPSHOT_DIR)).unwrap();
        create_private_file(&stage.path().join(SNAPSHOT_DIR).join("openrig.sqlite")).unwrap();
        let path = stage.path().to_path_buf();
        stage.finish().unwrap();
        assert!(!path.exists());
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 0);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_others_writable_parent_is_reported_not_cleaned() {
        let dir = scratch("shared");
        let stale = dir.join(".dendro-stage-00000000000000aa");
        create_private_dir(&stale).unwrap();
        create_private_file(&stale.join(MARKER_NAME))
            .unwrap()
            .write_all(MARKER)
            .unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o777)).unwrap();
        let stage = Stage::create(&dir).unwrap();
        let notes = recover_stale(&stage).unwrap();
        assert!(stale.exists());
        assert!(notes[0].contains("writable by others"));
        stage.finish().unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
        let _ = fs::remove_dir_all(&dir);
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;

    fn scratch() -> PathBuf {
        // Use the admitted per-user root, not a permissive TEMP directory.
        let root = platform::create_user_private_root_with_choice(
            "dendro-life",
            &platform::StorageRootChoice::UserProfile,
        )
        .unwrap();
        let path = root.path().to_path_buf();
        drop(root);
        path
    }

    fn stale_dir(parent: &Path) -> PathBuf {
        let root = platform::open_existing_private_root(parent).unwrap();
        platform::create_unique_private_directory(&root, STAGE_PREFIX)
            .unwrap()
            .path()
            .to_path_buf()
    }

    #[test]
    fn windows_private_copy_retains_content_and_refuses_existing_or_cancelled_target() {
        let dir = scratch();
        let source = dir.join("source");
        fs::write(&source, vec![17_u8; COPY_CHUNK + 7]).unwrap();
        let destination = dir.join("destination");
        private_copy(&source, &destination, &Cancel::new()).unwrap();
        let root = platform::open_existing_private_root(&dir).unwrap();
        let name =
            platform::parse_child_name("destination", platform::NativePlatform::Windows).unwrap();
        let verified = platform::open_existing_private_file(&root, &name).unwrap();
        assert_eq!(verified.metadata().links, 1);
        assert!(matches!(
            verified.metadata().identity,
            FileIdentity::WindowsFileId128 { .. }
        ));
        drop(verified);
        drop(root);
        assert_eq!(fs::read(&source).unwrap(), fs::read(&destination).unwrap());
        assert!(private_copy(&source, &destination, &Cancel::new()).is_err());
        let cancel = Cancel::new();
        cancel.request();
        let error = private_copy(&source, &dir.join("cancelled"), &cancel).unwrap_err();
        assert!(error.downcast_ref::<Interrupted>().is_some());
        assert_eq!(fs::read(&source).unwrap(), vec![17_u8; COPY_CHUNK + 7]);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn windows_stage_is_marked_before_payload_and_cleanup_removes_it() {
        let dir = scratch();
        let stage = Stage::create(&dir).unwrap();
        assert_eq!(fs::read(stage.path().join(MARKER_NAME)).unwrap(), MARKER);
        stage.check_anchor().unwrap();
        let path = stage.path().to_path_buf();
        let snapshot = create_private_snapshot_dir(&path.join(SNAPSHOT_DIR)).unwrap();
        create_private_file(&snapshot.join("openrig.sqlite")).unwrap();
        stage.finish().unwrap();
        assert!(!path.exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn windows_recovery_keeps_unmarked_unknown_and_hardlinked_stages() {
        let dir = scratch();
        let marked = |_name: &str| {
            let path = stale_dir(&dir);
            let mut marker = create_private_file(&path.join(MARKER_NAME)).unwrap();
            marker.write_all(MARKER).unwrap();
            marker.sync_all().unwrap();
            path
        };
        let removable = marked(".dendro-stage-00000000000000aa");
        create_private_file(&removable.join("analytics.duckdb")).unwrap();
        let snapshot = create_private_snapshot_dir(&removable.join(SNAPSHOT_DIR)).unwrap();
        create_private_file(&snapshot.join("openrig.sqlite")).unwrap();
        let incomplete = stale_dir(&dir);
        let unknown = marked(".dendro-stage-00000000000000cc");
        create_private_file(&unknown.join("unknown")).unwrap();
        let linked = marked(".dendro-stage-00000000000000dd");
        create_private_file(&linked.join("analytics.duckdb")).unwrap();
        fs::hard_link(linked.join("analytics.duckdb"), dir.join("outside-link")).unwrap();
        let stage = Stage::create(&dir).unwrap();
        let notes = recover_stale(&stage).unwrap();
        assert!(!removable.exists(), "{notes:?}");
        assert!(
            incomplete.exists() && unknown.exists() && linked.exists(),
            "{notes:?}"
        );
        assert!(dir.join("outside-link").exists());
        assert!(stage.path().exists());
        stage.finish().unwrap();
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn windows_cleanup_failure_preserves_marker_until_recovery() {
        let dir = scratch();
        let stage = Stage::create(&dir).unwrap();
        let path = stage.path().to_path_buf();
        let snapshot = create_private_snapshot_dir(&path.join(SNAPSHOT_DIR)).unwrap();
        // An exclusive handle causes a real cleanup failure while held.
        create_private_file(&snapshot.join("openrig.sqlite")).unwrap();
        use std::os::windows::fs::OpenOptionsExt;
        let held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(snapshot.join("openrig.sqlite"))
            .unwrap();
        assert!(stage.finish().is_err());
        assert_eq!(fs::read(path.join(MARKER_NAME)).unwrap(), MARKER);
        drop(held);
        let next = Stage::create(&dir).unwrap();
        let notes = recover_stale(&next).unwrap();
        assert!(!path.exists(), "{notes:?}");
        next.finish().unwrap();
        let dropped_path = {
            let dropped = Stage::create(&dir).unwrap();
            dropped.path().to_path_buf()
        };
        assert!(!dropped_path.exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn windows_marker_validation_bounds_reads_and_keeps_invalid_content() {
        let dir = scratch();
        let path = stale_dir(&dir);
        let mut marker = create_private_file(&path.join(MARKER_NAME)).unwrap();
        marker.write_all(&vec![b'x'; 1024]).unwrap();
        drop(marker);
        let root = platform::open_existing_private_root(&path).unwrap();
        assert_eq!(
            windows_keep_reason(&root, false),
            Some("no valid Dendro marker")
        );
        drop(root);
        fs::remove_dir_all(dir).unwrap();
    }
}
