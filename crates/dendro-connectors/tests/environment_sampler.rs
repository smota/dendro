//! Real-OS sampling of a short-lived child (a shell/cmd that runs a sleeper).
//! The root is pinned by its exact pid, the way a launcher would feed it.
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use dendro_connectors::environment_v1::{
    cwd_fingerprint, watch, Collector, Config, Isolation, Sealed, SysinfoSource,
};
use dendro_core::domain_v1::{CaptureMethod, EnvironmentKind, Unit};
use dendro_core::AnalyticsStore;

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let dir =
            std::env::temp_dir().join(format!("dendro-env-secret-dirname-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Child_(Child);
impl Drop for Child_ {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// A shell that runs a sleeper as its child: root plus one descendant.
fn spawn_tree(cwd: &Path) -> Child_ {
    #[cfg(windows)]
    let mut command = {
        let mut c = Command::new("cmd");
        c.args(["/c", "ping", "-n", "30", "127.0.0.1"]);
        c
    };
    #[cfg(not(windows))]
    let mut command = {
        let mut c = Command::new("sh");
        c.args(["-c", "sleep 30; true"]);
        c
    };
    Child_(
        command
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    )
}

#[test]
fn samples_a_real_process_tree_once_and_stores_no_paths() {
    let scratch = Scratch::new();
    let child = spawn_tree(&scratch.0);
    let root_pid = child.0.id();
    // No name matching: this machine may run real harnesses that would fill
    // the tracking bound before the pinned root is considered.
    let config = Config {
        harnesses: Vec::new(),
        pinned_roots: [root_pid].into(),
        ..Config::default()
    };
    let mut collector = Collector::new(
        SysinfoSource::new(),
        config,
        Isolation {
            kind: EnvironmentKind::Unknown,
            hint: "undetected",
        },
    );

    // `--once` semantics, repeated until the shell has started its sleeper.
    let stop = AtomicBool::new(false);
    let mut sealed = Sealed::default();
    let key = b"integration-key";
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        watch(
            &mut collector,
            Duration::from_millis(50),
            None,
            true,
            Duration::from_secs(60),
            &stop,
            |batch| {
                let part = batch.seal(key, &[]);
                sealed.identities.extend(part.identities);
                sealed.samples.extend(part.samples);
                Ok(())
            },
        )
        .unwrap();
        if sealed
            .identities
            .iter()
            .any(|i| i.parent_pid == Some(root_pid))
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "descendant of {root_pid} never discovered: {:#?}",
            sealed.identities
        );
        std::thread::sleep(Duration::from_millis(200));
    }

    let root = sealed
        .identities
        .iter()
        .find(|i| i.pid == root_pid)
        .expect("pinned root tracked");
    let child_row = sealed
        .identities
        .iter()
        .find(|i| i.parent_pid == Some(root_pid))
        .unwrap();
    assert_eq!(child_row.root_key, root.identity_key);
    assert!(root.harness.is_some() && child_row.harness.is_none());
    assert!(root.ended_at.is_none());
    assert_eq!(root.session_link, None, "no session evidence, no link");
    let fingerprint = root.cwd_fingerprint.as_deref().expect("cwd fingerprint");
    assert_eq!(fingerprint.len(), 64);
    #[cfg(target_os = "linux")]
    assert_eq!(
        fingerprint,
        cwd_fingerprint(key, &std::fs::canonicalize(&scratch.0).unwrap())
    );
    #[cfg(not(target_os = "linux"))]
    let _ = cwd_fingerprint;

    let sample = sealed
        .samples
        .iter()
        .find(|s| s.identity_key == root.identity_key)
        .unwrap();
    assert!(sample.rss_bytes.unwrap() > 0);
    assert!(sample.cpu_time_us.is_some());
    assert!(sample.visibility_limits.is_empty(), "{sample:?}");
    if cfg!(windows) {
        assert_eq!(sample.rss_unit, Unit::WindowsWorkingSetBytes);
        assert_eq!(sample.capture_method, CaptureMethod::WindowsProcessApi);
    } else if cfg!(target_os = "linux") {
        assert_eq!(sample.rss_unit, Unit::LinuxRssBytes);
        assert_eq!(sample.capture_method, CaptureMethod::Procfs);
    }

    // Store, read back, and look for the working directory's text anywhere.
    let db = scratch.0.join("probe.duckdb");
    let store = AnalyticsStore::open_path(&db, "key").unwrap();
    store
        .commit_process_samples(&sealed.identities, &sealed.samples)
        .unwrap();
    store.checkpoint().unwrap();
    drop(store);
    let config = duckdb::Config::default()
        .access_mode(duckdb::AccessMode::ReadOnly)
        .unwrap();
    let conn = duckdb::Connection::open_with_flags(&db, config).unwrap();
    let identities: i64 = conn
        .query_row("SELECT COUNT(*) FROM process_identities", [], |r| r.get(0))
        .unwrap();
    // Repeated `--once` passes re-seal the same identity; storage upserts by key.
    let distinct: std::collections::BTreeSet<_> =
        sealed.identities.iter().map(|i| &i.identity_key).collect();
    assert_eq!(identities as usize, distinct.len());
    let samples: i64 = conn
        .query_row("SELECT COUNT(*) FROM process_samples", [], |r| r.get(0))
        .unwrap();
    assert_eq!(samples as usize, sealed.samples.len());
    let mut stmt = conn
        .prepare(
            "SELECT table_name, column_name FROM information_schema.columns
             WHERE data_type = 'VARCHAR' AND table_name LIKE 'process_%'",
        )
        .unwrap();
    let columns: Vec<(String, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert!(!columns.is_empty());
    for (table, column) in columns {
        let hits: i64 = conn
            .query_row(
                &format!("SELECT COUNT(*) FROM {table} WHERE {column} LIKE '%secret-dirname%'"),
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(hits, 0, "{table}.{column}");
    }
}
