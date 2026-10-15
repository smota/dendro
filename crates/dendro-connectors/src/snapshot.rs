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
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{anyhow, Result};
use dendro_core::{lifecycle, Cancel};
use rusqlite::OpenFlags;
use sha2::{Digest, Sha256};

pub const MAX_ATTEMPTS: u32 = 5;
pub const SNAPSHOT_MAIN: &str = "openrig.sqlite";
const SNAPSHOT_WAL: &str = "openrig.sqlite-wal";

/// What a file looked like at one instant: identity, size, mtime.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FileState {
    dev: u64,
    ino: u64,
    size: u64,
    mtime_ns: i128,
}

/// The source main file and its WAL (absent, or with its 32-byte header).
#[derive(Debug, Clone, PartialEq, Eq)]
struct SourceState {
    main: FileState,
    wal: Option<(FileState, Vec<u8>)>,
}

fn file_state(path: &Path) -> std::io::Result<Option<FileState>> {
    match fs::metadata(path) {
        Ok(meta) => Ok(Some(FileState {
            dev: meta.dev(),
            ino: meta.ino(),
            size: meta.size(),
            mtime_ns: meta.mtime() as i128 * 1_000_000_000 + meta.mtime_nsec() as i128,
        })),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn wal_header(path: &Path) -> std::io::Result<Vec<u8>> {
    let mut header = Vec::with_capacity(32);
    File::open(path)?.take(32).read_to_end(&mut header)?;
    Ok(header)
}

fn observe(main: &Path, wal: &Path) -> Result<SourceState> {
    let main_state = file_state(main)?.ok_or_else(|| anyhow!("the OpenRig source is missing"))?;
    let wal_state = match file_state(wal)? {
        Some(state) => match wal_header(wal) {
            Ok(header) => Some((state, header)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        },
        None => None,
    };
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

/// Takes a coherent private copy of `source` into `dir` (created 0700).
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
    lifecycle::create_private_dir(dir)?;
    let source_wal = PathBuf::from(format!("{}-wal", source.display()));
    let main_copy = dir.join(SNAPSHOT_MAIN);
    let wal_copy = dir.join(SNAPSHOT_WAL);
    for attempt in 1..=max_attempts {
        cancel.check()?;
        clear(dir)?;
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
        identity.update(before.main.dev.to_be_bytes());
        identity.update(before.main.ino.to_be_bytes());
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
    clear(dir)?;
    Err(anyhow!(
        "the OpenRig source changed during each of {max_attempts} snapshot attempts; nothing was ingested. Retry in a moment, or retry when the rig is quieter."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let snapshot = snapshot_sqlite(&source, &dir.join("snap"), &Cancel::new()).unwrap();
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
        let snapshot = snapshot_sqlite(&path, &dir.join("snap"), &Cancel::new()).unwrap();
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
        let snapshot =
            snapshot_with(
                &source,
                &dir.join("snap"),
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
        let snapshot = snapshot_with(&path, &dir.join("snap"), &Cancel::new(), 5, |attempt| {
            if attempt == 1 {
                // Same bytes, new inode: a replaced file.
                fs::rename(&spare, &path).unwrap();
            }
            if attempt == 2 {
                // A WAL appears.
                fs::write(PathBuf::from(format!("{}-wal", path.display())), b"").unwrap();
            }
        })
        .unwrap();
        assert_eq!(snapshot.attempts, 3);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn exhaustion_fails_cleanly_and_leaves_no_copy() {
        let dir = scratch("exhaust");
        let (source, writer) = wal_source(&dir);
        let mut next = 10;
        let error = snapshot_with(&source, &dir.join("snap"), &Cancel::new(), 3, |_| {
            next += 1;
            writer.execute("INSERT INTO t VALUES (?)", [next]).unwrap();
        })
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
            fs::read_dir(dir.join("snap")).unwrap().count(),
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
        let error = snapshot_sqlite(&source, &dir.join("snap"), &cancel).unwrap_err();
        assert!(error.downcast_ref::<dendro_core::Interrupted>().is_some());
        let _ = fs::remove_dir_all(&dir);
    }
}
