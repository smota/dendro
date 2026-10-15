//! A replacement's historical lifetimes survive later ordinary ingests too.
use std::fs;
use std::path::{Path, PathBuf};

use dendro_connectors::{ingest_staged, SourceRoot};
use dendro_core::{report_json, run_locked_with_openrig, AnalyticsStore};
use rusqlite::{params, Connection};
use serde_json::{json, Value};

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn fixture(path: &Path, seat: &str) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(
        "CREATE TABLE events(seq INTEGER PRIMARY KEY,type TEXT,payload TEXT,created_at TEXT)",
    )
    .unwrap();
    for (seq, kind, hour, payload) in [
        (
            1,
            "node.launched",
            1,
            json!({"rigId":"rig","nodeId":"node","sessionName":seat}),
        ),
        (
            2,
            "seat.activity_changed",
            1,
            json!({"sessionName":seat,"state":"running"}),
        ),
        (
            3,
            "session.stopped",
            2,
            json!({"rigId":"rig","nodeId":"node","sessionName":seat}),
        ),
    ] {
        conn.execute(
            "INSERT INTO events VALUES(?,?,?,?)",
            params![
                seq,
                kind,
                payload.to_string(),
                format!("2026-10-09T{hour:02}:00:00Z")
            ],
        )
        .unwrap();
    }
}

fn run(db: &Path, source: &Path) {
    let roots = [SourceRoot {
        name: "openrig".into(),
        path: source.into(),
    }];
    let paths = [source.to_path_buf()];
    run_locked_with_openrig(db, &paths, &paths, |stage, key| {
        ingest_staged(stage, &roots, false, key)
    })
    .unwrap();
}
fn activity(db: &Path) -> Value {
    report_json(
        db,
        "activity",
        None,
        "2026-10-09T00:00:00Z",
        "2026-10-10T00:00:00Z",
    )
    .unwrap()["payload"]["activity"]
        .clone()
}
fn lives(db: &Path) -> Value {
    let store = AnalyticsStore::open_read_only(db).unwrap();
    let mut rows = store.load_seat_bounds().unwrap();
    rows.sort_by(|a, b| a.seat.cmp(&b.seat));
    json!(rows)
}

#[test]
fn replaced_source_lifetimes_survive_identical_third_ingest_and_later_append() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/issue-13-tests")
        .join(format!("lifecycle-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let scratch = Scratch(dir);
    let source = scratch.0.join("openrig.sqlite");
    let replacement = scratch.0.join("replacement.sqlite");
    let db = scratch.0.join("analytics.duckdb");
    fixture(&source, "old-seat");
    run(&db, &source);
    fixture(&replacement, "new-seat");
    fs::rename(&replacement, &source).unwrap();
    run(&db, &source);
    let baseline_lives = lives(&db);
    assert_eq!(baseline_lives.as_array().unwrap().len(), 2);
    let baseline_activity = activity(&db);
    run(&db, &source);
    assert_eq!(lives(&db), baseline_lives);
    assert_eq!(activity(&db), baseline_activity);
    let conn = Connection::open(&source).unwrap();
    conn.execute(
        "INSERT INTO events VALUES(4,'unrelated', '{}','2026-10-09T03:00:00Z')",
        [],
    )
    .unwrap();
    drop(conn);
    run(&db, &source);
    assert_eq!(lives(&db), baseline_lives);
    assert_eq!(activity(&db), baseline_activity);
}
