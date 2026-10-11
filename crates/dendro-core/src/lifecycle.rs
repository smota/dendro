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
//! A run removes its own stage explicitly and reports a failure. A later run,
//! holding the same lock, removes stale stages left by a killed run, but only
//! when the stage is a real directory we own, carries a valid marker and holds
//! allowlisted entries. Anything else is kept and reported.

use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{Read, Write};
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
pub fn create_private_dir(path: &Path) -> Result<()> {
    DirBuilder::new().mode(0o700).create(path)?;
    Ok(())
}

/// Creates a new file with mode 0600 from the creating syscall. Fails if the
/// path exists, including as a symlink (O_CREAT | O_EXCL).
pub fn create_private_file(path: &Path) -> Result<File> {
    Ok(OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?)
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
fn identity(path: &Path) -> Result<(u64, u64)> {
    let meta = fs::symlink_metadata(path)?;
    Ok((meta.dev(), meta.ino()))
}

/// One run's private stage, created inside the locked output parent.
pub struct Stage {
    path: PathBuf,
    uid: u32,
    parent: PathBuf,
    parent_identity: (u64, u64),
    active: bool,
}

impl Stage {
    /// Allocates `.init`, writes the marker, then publishes the final name.
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

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn uid(&self) -> u32 {
        self.uid
    }

    /// Refuses when the output parent was replaced since the stage was made.
    pub fn check_anchor(&self) -> Result<()> {
        if identity(&self.parent)? != self.parent_identity {
            return Err(anyhow!("analytics output directory changed during the run"));
        }
        Ok(())
    }

    /// Removes the stage. A failure is returned, never swallowed.
    pub fn finish(mut self) -> Result<()> {
        self.active = false;
        remove_stage(&self.path)
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        if self.active {
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
/// partial failure always leaves a marked stage that the next run can recover,
/// whatever order the directory lists its entries in. Used by finish, the
/// Drop fallback and stale recovery.
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

fn private_regular_file(path: &Path, uid: u32, mode: Option<u32>) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| {
        meta.file_type().is_file()
            && meta.uid() == uid
            && meta.nlink() == 1
            && mode.is_none_or(|mode| meta.mode() & 0o777 == mode)
    })
}

fn private_dir(path: &Path, uid: u32) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| {
        meta.file_type().is_dir() && meta.uid() == uid && meta.mode() & 0o777 == 0o700
    })
}

fn valid_marker(dir: &Path, uid: u32) -> bool {
    let path = dir.join(MARKER_NAME);
    private_regular_file(&path, uid, Some(0o600))
        && fs::read(&path).is_ok_and(|bytes| bytes == MARKER)
}

/// Why a stale-looking entry must be kept, or None when it may be removed.
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

#[cfg(test)]
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
