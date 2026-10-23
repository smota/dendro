//! Coherent private copies of a live SQLite source (AC12).
//!
//! Dendro never opens a SQLite connection on an original source: SQLite would
//! open the source WAL read-write. Instead the main file and WAL are copied
//! with read-only handles into the run's private stage, WAL first, and SQLite
//! works only on the copy. The source -shm is never opened.
//!
//! A byte copy takes no SQLite lock, so it is not trusted on its own: the
//! source's identity, size, mtime and WAL header are recorded before and after
//! each attempt, and the copy must pass `PRAGMA quick_check`. Any change
//! discards the attempt; after a bounded number of attempts the run fails
//! cleanly and ingests nothing.

use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{anyhow, Result};
use dendro_core::file_metadata::{held_source_metadata, reported_identity, FileMetadata};
use dendro_core::platform_v1::FileIdentity;
use dendro_core::{lifecycle, Cancel};
use rusqlite::OpenFlags;
use sha2::{Digest, Sha256};

pub const MAX_ATTEMPTS: u32 = 5;
pub const SNAPSHOT_MAIN: &str = "openrig.sqlite";
const SNAPSHOT_WAL: &str = "openrig.sqlite-wal";

/// What a file looked like at one instant: identity, size, mtime.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FileState {
    dev: String,
    ino: String,
    size: u64,
    mtime_ns: i128,
}

/// The source main file and its WAL (absent, or with its 32-byte header).
#[derive(Debug, Clone, PartialEq, Eq)]
struct SourceState {
    main: FileState,
    wal: Option<(FileState, Vec<u8>)>,
}

fn held_state(file: &File) -> Result<FileState> {
    let meta = held_source_metadata(file)
        .map_err(|reason| anyhow!("source metadata unavailable: {reason}"))?;
    state_from_metadata(meta)
}

fn state_from_metadata(meta: FileMetadata) -> Result<FileState> {
    reported_identity(&meta).map_err(|reason| anyhow!("source identity unavailable: {reason}"))?;
    Ok(FileState {
        dev: meta.device,
        ino: meta.inode,
        size: meta.size.parse()?,
        mtime_ns: meta.mtime_ns.parse()?,
    })
}

fn observe_file(path: &Path, read_header: bool) -> Result<Option<(FileState, Vec<u8>)>> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let before = held_state(&file)?;
    let mut header = Vec::with_capacity(32);
    if read_header {
        (&file).take(32).read_to_end(&mut header)?;
    }
    if held_state(&file)? != before {
        return Err(anyhow!("source changed during metadata observation"));
    }
    Ok(Some((before, header)))
}

fn observe(main: &Path, wal: &Path) -> Result<SourceState> {
    let main_state = observe_file(main, false)?
        .ok_or_else(|| anyhow!("the OpenRig source is missing"))?
        .0;
    let wal_state = observe_file(wal, true)?;
    Ok(SourceState {
        main: main_state,
        wal: wal_state,
    })
}

/// A coherent private copy and the source revision it was taken from.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub path: PathBuf,
    pub attempts: u32,
    /// sha256 of the accepted source state (sizes, mtimes, identity, WAL header).
    pub revision: String,
    /// Identity of the accepted source, captured with the copied bytes.
    pub source_identity_sha256: String,
    pub main_bytes: u64,
    pub wal_bytes: Option<u64>,
    pub copy_ms: u128,
    pub check_ms: u128,
}

fn revision(state: &SourceState) -> String {
    let mut hash = Sha256::new();
    let main = &state.main;
    hash.update(format!(
        "main {} {} {} {}\n",
        main.dev, main.ino, main.size, main.mtime_ns
    ));
    if let Some((wal, header)) = &state.wal {
        hash.update(format!(
            "wal {} {} {} {}\n",
            wal.dev, wal.ino, wal.size, wal.mtime_ns
        ));
        hash.update(header);
    }
    hex::encode(hash.finalize())
}

fn hash_identity(hash: &mut Sha256, state: &FileState) -> Result<()> {
    let metadata = FileMetadata {
        r#type: "regular".into(),
        device: state.dev.clone(),
        inode: state.ino.clone(),
        size: state.size.to_string(),
        mtime_ns: state.mtime_ns.to_string(),
    };
    match reported_identity(&metadata)
        .map_err(|reason| anyhow!("source identity unavailable: {reason}"))?
    {
        FileIdentity::Unix { device, inode } => {
            // Preserve the existing Linux source-identity digest encoding.
            hash.update(device.to_be_bytes());
            hash.update(inode.to_be_bytes());
        }
        FileIdentity::WindowsFileId128 {
            volume_serial_64,
            file_id,
        } => {
            hash.update(b"windows-file-id-128\0");
            hash.update(volume_serial_64.to_be_bytes());
            hash.update(file_id);
        }
        FileIdentity::WindowsByHandle64 { .. } => {
            return Err(anyhow!("source identity is too narrow"))
        }
    }
    Ok(())
}

/// Removes every file a previous attempt left in the private snapshot directory.
fn clear(dir: &Path) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        fs::remove_file(entry?.path())?;
    }
    Ok(())
}

/// `PRAGMA quick_check` on the private copy. SQLite may write its own index
/// and checkpoint the private WAL here: this is the copy, never the source.
fn quick_check(path: &Path) -> Result<bool> {
    let conn = rusqlite::Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    let result: String = conn.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    Ok(result == "ok")
}

/// Takes a coherent private copy of `source` into a private directory allocated
/// for `dir`. The returned path identifies the actual directory on Windows.
pub fn snapshot_sqlite(source: &Path, dir: &Path, cancel: &Cancel) -> Result<Snapshot> {
    snapshot_with(source, dir, cancel, MAX_ATTEMPTS, |_| {})
}

/// As [`snapshot_sqlite`]; `between` runs after copying and before the second
/// observation (tests use it to change the source mid-attempt).
pub(crate) fn snapshot_with(
    source: &Path,
    dir: &Path,
    cancel: &Cancel,
    max_attempts: u32,
    mut between: impl FnMut(u32),
) -> Result<Snapshot> {
    let dir = lifecycle::create_private_snapshot_dir(dir)?;
    let source_wal = PathBuf::from(format!("{}-wal", source.display()));
    let main_copy = dir.join(SNAPSHOT_MAIN);
    let wal_copy = dir.join(SNAPSHOT_WAL);
    for attempt in 1..=max_attempts {
        cancel.check()?;
        clear(&dir)?;
        let started = Instant::now();
        let before = observe(source, &source_wal)?;
        // WAL first: a checkpoint between the two copies leaves its frames in
        // both, and replaying them is idempotent.
        let copied = (|| -> Result<()> {
            if before.wal.is_some() {
                lifecycle::private_copy(&source_wal, &wal_copy, cancel)?;
            }
            lifecycle::private_copy(source, &main_copy, cancel)
        })();
        match copied {
            Ok(()) => {}
            Err(error) if error.downcast_ref::<dendro_core::Interrupted>().is_some() => {
                return Err(error)
            }
            // A WAL that vanished mid-copy is a change: try again.
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|io| io.kind() == std::io::ErrorKind::NotFound) =>
            {
                continue
            }
            Err(error) => return Err(error),
        }
        between(attempt);
        let after = observe(source, &source_wal)?;
        let copy_ms = started.elapsed().as_millis();
        if before != after {
            continue;
        }
        let checked = Instant::now();
        if !quick_check(&main_copy)? {
            continue;
        }
        let mut identity = Sha256::new();
        hash_identity(&mut identity, &before.main)?;
        return Ok(Snapshot {
            source_identity_sha256: hex::encode(identity.finalize()),
            path: main_copy,
            attempts: attempt,
            revision: revision(&before),
            main_bytes: before.main.size,
            wal_bytes: before.wal.as_ref().map(|(wal, _)| wal.size),
            copy_ms,
            check_ms: checked.elapsed().as_millis(),
        });
    }
    clear(&dir)?;
    Err(anyhow!(
        "the OpenRig source changed during each of {max_attempts} snapshot attempts; nothing was ingested. Retry in a moment, or retry when the rig is quieter."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    struct Scratch(Option<dendro_core::platform_v1::PrivateRoot>);

    #[cfg(windows)]
    impl std::ops::Deref for Scratch {
        type Target = Path;
        fn deref(&self) -> &Path {
            self.0.as_ref().unwrap().path()
        }
    }

    #[cfg(windows)]
    impl AsRef<Path> for Scratch {
        fn as_ref(&self) -> &Path {
            self.0.as_ref().unwrap().path()
        }
    }

    #[cfg(windows)]
    impl Drop for Scratch {
        fn drop(&mut self) {
            let root = self.0.take().unwrap();
            let path = root.path().to_path_buf();
            drop(root);
            if let Err(error) = fs::remove_dir_all(path) {
                assert_eq!(
                    error.kind(),
                    std::io::ErrorKind::NotFound,
                    "remove owned snapshot fixture after releasing handles: {error}"
                );
            }
        }
    }

    #[cfg(windows)]
    fn scratch(name: &str) -> Scratch {
        use dendro_core::platform_v1::{create_user_private_root_with_choice, StorageRootChoice};
        Scratch(Some(
            create_user_private_root_with_choice(
                &format!("dendro-snapshot-{name}-"),
                &StorageRootChoice::UserProfile,
            )
            .expect("explicit UserProfile admitted snapshot fixture"),
        ))
    }

    #[cfg(not(windows))]
    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("dendro-snapshot-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A WAL-mode source with checkpointed rows and one row only in the WAL,
    /// kept open so the WAL stays.
    fn wal_source(dir: &Path) -> (PathBuf, rusqlite::Connection) {
        let path = dir.join("source.sqlite");
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.pragma_update(None, "journal_mode", "WAL").unwrap();
        conn.pragma_update(None, "wal_autocheckpoint", 0).unwrap();
        conn.execute("CREATE TABLE t(id INTEGER PRIMARY KEY)", [])
            .unwrap();
        conn.execute("INSERT INTO t VALUES (1)", []).unwrap();
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
            .unwrap();
        conn.execute("INSERT INTO t VALUES (2)", []).unwrap();
        (path, conn)
    }

    fn ids(path: &Path) -> Vec<i64> {
        let conn = rusqlite::Connection::open(path).unwrap();
        let mut stmt = conn.prepare("SELECT id FROM t ORDER BY id").unwrap();
        stmt.query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    #[test]
    fn a_stable_source_is_copied_with_its_wal_only_rows_and_no_shm() {
        let dir = scratch("stable");
        let (source, _writer) = wal_source(&dir);
        let shm = PathBuf::from(format!("{}-shm", source.display()));
        let shm_before = fs::read(&shm).unwrap();
        let snapshot =
            snapshot_sqlite(&source, &dir.join(lifecycle::SNAPSHOT_DIR), &Cancel::new()).unwrap();
        assert_eq!(snapshot.attempts, 1);
        assert!(
            snapshot.wal_bytes.unwrap() > 0,
            "the source WAL was not empty"
        );
        assert_eq!(
            ids(&snapshot.path),
            vec![1, 2],
            "the WAL-only row is in the copy"
        );
        assert_eq!(
            fs::read(&shm).unwrap(),
            shm_before,
            "the source -shm is untouched"
        );
        assert_eq!(snapshot.revision.len(), 64);
        #[cfg(windows)]
        {
            let source_state = observe_file(&source, false).unwrap().unwrap().0;
            assert!(source_state.dev.starts_with("w128:"));
            assert_eq!(source_state.ino.len(), "w128:".len() + 32);
            let actual_dir = snapshot.path.parent().unwrap();
            assert_ne!(actual_dir, dir.join(lifecycle::SNAPSHOT_DIR));
            dendro_core::platform_v1::open_existing_private_root(actual_dir)
                .expect("native protected snapshot directory admitted");
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_source_without_a_wal_gets_none_created() {
        let dir = scratch("nowal");
        let path = dir.join("source.sqlite");
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch("CREATE TABLE t(id INTEGER PRIMARY KEY); INSERT INTO t VALUES (1);")
            .unwrap();
        drop(conn);
        let snapshot =
            snapshot_sqlite(&path, &dir.join(lifecycle::SNAPSHOT_DIR), &Cancel::new()).unwrap();
        assert_eq!(snapshot.wal_bytes, None);
        assert!(!PathBuf::from(format!("{}-wal", path.display())).exists());
        assert!(!PathBuf::from(format!("{}-shm", path.display())).exists());
        assert_eq!(ids(&snapshot.path), vec![1]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn any_change_during_an_attempt_discards_it_and_retries() {
        let dir = scratch("retry");
        let (source, writer) = wal_source(&dir);
        let wal = PathBuf::from(format!("{}-wal", source.display()));
        // Attempt 1: a new commit (WAL size and mtime change). Attempt 2: a
        // TRUNCATE checkpoint (WAL header and size change). Attempt 3: stable.
        let snapshot = snapshot_with(
            &source,
            &dir.join(lifecycle::SNAPSHOT_DIR),
            &Cancel::new(),
            5,
            |attempt| match attempt {
                1 => {
                    writer.execute("INSERT INTO t VALUES (3)", []).unwrap();
                }
                2 => {
                    writer
                        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
                        .unwrap();
                }
                _ => {}
            },
        )
        .unwrap();
        assert_eq!(snapshot.attempts, 3);
        assert_eq!(
            ids(&snapshot.path),
            vec![1, 2, 3],
            "the accepted copy is the final coherent state"
        );
        assert_eq!(snapshot.wal_bytes, Some(fs::metadata(&wal).unwrap().len()));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn identity_replacement_and_an_appearing_wal_are_changes() {
        let dir = scratch("identity");
        let path = dir.join("source.sqlite");
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch("CREATE TABLE t(id INTEGER PRIMARY KEY); INSERT INTO t VALUES (1);")
            .unwrap();
        drop(conn);
        let spare = dir.join("spare.sqlite");
        fs::copy(&path, &spare).unwrap();
        let snapshot = snapshot_with(
            &path,
            &dir.join(lifecycle::SNAPSHOT_DIR),
            &Cancel::new(),
            5,
            |attempt| {
                if attempt == 1 {
                    // Same bytes, new inode: a replaced file.
                    fs::rename(&spare, &path).unwrap();
                }
                if attempt == 2 {
                    // A WAL appears.
                    fs::write(PathBuf::from(format!("{}-wal", path.display())), b"").unwrap();
                }
            },
        )
        .unwrap();
        assert_eq!(snapshot.attempts, 3);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn exhaustion_fails_cleanly_and_leaves_no_copy() {
        let dir = scratch("exhaust");
        let (source, writer) = wal_source(&dir);
        let mut next = 10;
        let error = snapshot_with(
            &source,
            &dir.join(lifecycle::SNAPSHOT_DIR),
            &Cancel::new(),
            3,
            |_| {
                next += 1;
                writer.execute("INSERT INTO t VALUES (?)", [next]).unwrap();
            },
        )
        .unwrap_err()
        .to_string();
        assert_eq!(
            error,
            "the OpenRig source changed during each of 3 snapshot attempts; nothing was ingested. Retry in a moment, or retry when the rig is quieter."
        );
        assert!(
            !error.contains(&dir.display().to_string()),
            "no path in the message"
        );
        assert_eq!(
            fs::read_dir(
                fs::read_dir(&dir)
                    .unwrap()
                    .map(|entry| entry.unwrap().path())
                    .find(|path| {
                        path.file_name()
                            .unwrap()
                            .to_string_lossy()
                            .starts_with("snapshot")
                    })
                    .expect("allocated snapshot directory"),
            )
            .unwrap()
            .count(),
            0,
            "no partial copy kept"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn cancellation_stops_between_attempts() {
        let dir = scratch("cancel");
        let (source, _writer) = wal_source(&dir);
        let cancel = Cancel::new();
        cancel.request();
        let error =
            snapshot_sqlite(&source, &dir.join(lifecycle::SNAPSHOT_DIR), &cancel).unwrap_err();
        assert!(error.downcast_ref::<dendro_core::Interrupted>().is_some());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn windows_identity_keeps_high_64_bits_in_revision_and_identity_digest() {
        let metadata = FileMetadata {
            r#type: "regular".into(),
            device: "w128:18446744073709551615".into(),
            inode: "w128:01000000000000000000000000000000".into(),
            size: "7".into(),
            mtime_ns: "42".into(),
        };
        let first = state_from_metadata(metadata.clone()).unwrap();
        let mut changed = metadata;
        changed.inode = "w128:01000000000000000000000000000080".into();
        let second = state_from_metadata(changed).unwrap();
        let digest = |state: &FileState| {
            let mut hash = Sha256::new();
            hash_identity(&mut hash, state).unwrap();
            hex::encode(hash.finalize())
        };
        assert_ne!(digest(&first), digest(&second));
        assert_ne!(
            revision(&SourceState {
                main: first,
                wal: None
            }),
            revision(&SourceState {
                main: second,
                wal: None
            }),
        );
    }

    #[test]
    fn unix_revision_and_identity_digest_preserve_existing_encoding() {
        let state = FileState {
            dev: "1".into(),
            ino: "2".into(),
            size: 3,
            mtime_ns: -4,
        };
        assert_eq!(
            revision(&SourceState {
                main: state.clone(),
                wal: None
            }),
            hex::encode(Sha256::digest(b"main 1 2 3 -4\n")),
        );
        let mut actual = Sha256::new();
        hash_identity(&mut actual, &state).unwrap();
        let mut expected = Sha256::new();
        expected.update(1_u64.to_be_bytes());
        expected.update(2_u64.to_be_bytes());
        assert_eq!(actual.finalize(), expected.finalize());
    }

    #[test]
    fn unknown_or_nonregular_metadata_is_an_error_not_absence() {
        let dir = scratch("unknown");
        assert!(observe_file(&dir, true).is_err());
        assert_eq!(observe_file(&dir.join("missing"), true).unwrap(), None);
        let unusable = FileMetadata {
            r#type: "regular".into(),
            device: "w128:1".into(),
            inode: "w128:00000000000000000000000000000000".into(),
            size: "0".into(),
            mtime_ns: "0".into(),
        };
        assert!(state_from_metadata(unusable).is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
