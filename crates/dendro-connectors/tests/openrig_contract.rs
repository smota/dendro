//! OpenRig 0.7.6 consumer contract. Fixtures are schema only.
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use dendro_connectors::openrig::openrig_sqlite_path;
use dendro_connectors::{ingest_staged, SourceRoot};
use dendro_core::run_locked_with_openrig;
use rusqlite::Connection;

const SCHEMA: &str = include_str!("fixtures/openrig_076_schema.sql");
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let root = dendro_core::platform_v1::create_user_private_root_with_choice(
            &format!(
                "dendro-openrig-contract-{}-",
                NEXT.fetch_add(1, Ordering::Relaxed)
            ),
            &dendro_core::platform_v1::StorageRootChoice::UserProfile,
        )
        .unwrap();
        let path = root.path().to_path_buf();
        drop(root);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn write_db(path: &Path, sql: &str) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(sql).unwrap();
}

fn ingest(db: &Path, source: &Path) -> Result<serde_json::Value, anyhow::Error> {
    let roots = [SourceRoot {
        name: "openrig".into(),
        path: source.into(),
    }];
    let paths = [source.to_path_buf()];
    run_locked_with_openrig(db, &paths, &paths, |stage, key| {
        ingest_staged(stage, &roots, false, key)
    })
}

fn notes(value: &serde_json::Value) -> String {
    value["sources"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|note| note.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn handed_off_facts(db: &Path) -> usize {
    let report = dendro_core::report_json(
        db,
        "decisions",
        None,
        "2026-11-02T00:00:00Z",
        "2026-11-03T00:00:00Z",
    )
    .unwrap();
    report["payload"]["demand_events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["state"] == "handed-off")
        .count()
}

#[test]
fn openrig_home_is_the_openrig_directory() {
    assert_eq!(
        openrig_sqlite_path(Some("/tmp/openrig-home"), None),
        PathBuf::from("/tmp/openrig-home/openrig.sqlite")
    );
    assert_eq!(
        openrig_sqlite_path(None, Some("/home/example")),
        PathBuf::from("/home/example/.openrig/openrig.sqlite")
    );
}

#[test]
fn openrig_076_fixture_is_supported() {
    let dir = Scratch::new();
    let source = dir.0.join("openrig.sqlite");
    let db = dir.0.join("analytics.duckdb");
    write_db(&source, SCHEMA);
    let ran = ingest(&db, &source).unwrap();
    let text = notes(&ran);
    assert!(text.contains("contract=supported"), "{text}");
    assert!(
        text.contains("migration=141_session_agent_runner.sql"),
        "{text}"
    );
}

#[test]
fn missing_required_column_refuses_and_writes_nothing() {
    let dir = Scratch::new();
    let source = dir.0.join("openrig.sqlite");
    let db = dir.0.join("analytics.duckdb");
    write_db(
        &source,
        &format!(
            "{SCHEMA}
             DROP TABLE queue_transitions;
             CREATE TABLE queue_transitions (
               transition_id INTEGER PRIMARY KEY,
               ts TEXT,
               state TEXT,
               transition_note TEXT,
               actor_session TEXT,
               closure_reason TEXT,
               closure_target TEXT
             );"
        ),
    );
    let error = ingest(&db, &source).unwrap_err().to_string();
    assert!(error.contains("contract=unsupported"), "{error}");
    assert!(
        error.contains("missing-column:queue_transitions.qitem_id"),
        "{error}"
    );
    if db.exists() {
        let report = dendro_core::report_json(
            &db,
            "decisions",
            None,
            "2026-11-01T00:00:00Z",
            "2026-11-03T00:00:00Z",
        );
        match report {
            Err(error) => assert!(
                error.to_string().contains("no publication state yet"),
                "{error}"
            ),
            Ok(report) => {
                let events = report["payload"]["activity_events"]
                    .as_array()
                    .map(Vec::len)
                    .unwrap_or(0);
                assert_eq!(events, 0, "refused source wrote activity facts");
            }
        }
    }
}

#[test]
fn added_nullable_column_is_additive_and_not_a_row_mutation() {
    let dir = Scratch::new();
    let source = dir.0.join("openrig.sqlite");
    let db = dir.0.join("analytics.duckdb");
    write_db(&source, SCHEMA);
    let first = notes(&ingest(&db, &source).unwrap());
    assert!(first.contains("contract=supported"), "{first}");
    let conn = Connection::open(&source).unwrap();
    conn.execute_batch("ALTER TABLE events ADD COLUMN dendro_probe TEXT;")
        .unwrap();
    drop(conn);
    let second = notes(&ingest(&db, &source).unwrap());
    assert!(second.contains("contract=additive"), "{second}");
    assert!(!second.contains("openrig source changed"), "{second}");
}

#[test]
fn stateless_seat_ping_keeps_agent_activity() {
    let dir = Scratch::new();
    let source = dir.0.join("openrig.sqlite");
    let db = dir.0.join("analytics.duckdb");
    write_db(
        &source,
        &format!(
            "{SCHEMA}
             INSERT INTO events (type, payload, created_at) VALUES
             ('seat.activity_changed', '{{\"sessionName\":\"seat\",\"at\":\"2026-11-02T04:36:36Z\"}}', '2026-11-02 04:36:36'),
             ('agent.activity', '{{\"sessionName\":\"seat\",\"activity\":{{\"state\":\"idle\",\"eventAt\":\"2026-11-02T04:36:36Z\"}}}}', '2026-11-02 04:36:36');"
        ),
    );
    ingest(&db, &source).unwrap();
    let report = dendro_core::report_json(
        &db,
        "decisions",
        None,
        "2026-11-02T00:00:00Z",
        "2026-11-03T00:00:00Z",
    )
    .unwrap();
    let events = report["payload"]["activity_events"].as_array().unwrap();
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0]["state"], "idle");
}

#[test]
fn unknown_event_is_counted_and_handoff_is_not_doubled() {
    let dir = Scratch::new();
    let source = dir.0.join("openrig.sqlite");
    let db = dir.0.join("analytics.duckdb");
    write_db(
        &source,
        &format!(
            "{SCHEMA}
             INSERT INTO events (type, payload, created_at) VALUES
             ('watchdog.evaluation_skipped', '{{}}', '2026-11-02 04:36:36'),
             ('queue.handed_off', '{{\"qitemId\":\"q1\",\"fromSession\":\"seat-a\",\"toSession\":\"seat-b\",\"closureReason\":\"done\"}}', '2026-11-02 04:40:00');
             INSERT INTO queue_transitions (qitem_id, ts, state, actor_session, closure_target)
             VALUES ('q1', '2026-11-02T04:40:00Z', 'handed-off', 'seat-a', 'seat-b');"
        ),
    );
    let ran = ingest(&db, &source).unwrap();
    let text = notes(&ran);
    assert!(text.contains("unknown_events=1"), "{text}");
    assert_eq!(
        handed_off_facts(&db),
        1,
        "event and transition handoff were both stored"
    );
}

#[test]
fn event_only_handoff_is_one_queue_fact() {
    let dir = Scratch::new();
    let source = dir.0.join("openrig.sqlite");
    let db = dir.0.join("analytics.duckdb");
    write_db(
        &source,
        &format!(
            "{SCHEMA}
             INSERT INTO events (type, payload, created_at) VALUES
             ('queue.handed_off', '{{\"qitemId\":\"q1\",\"fromSession\":\"seat-a\",\"toSession\":\"seat-b\"}}', '2026-11-02 04:40:00');"
        ),
    );
    ingest(&db, &source).unwrap();
    assert_eq!(handed_off_facts(&db), 1);
}

#[test]
fn transition_only_handoff_is_one_queue_fact() {
    let dir = Scratch::new();
    let source = dir.0.join("openrig.sqlite");
    let db = dir.0.join("analytics.duckdb");
    write_db(
        &source,
        &format!(
            "{SCHEMA}
             INSERT INTO queue_transitions (qitem_id, ts, state, actor_session, closure_target)
             VALUES ('q1', '2026-11-02T04:40:00Z', 'handed-off', 'seat-a', 'seat-b');"
        ),
    );
    ingest(&db, &source).unwrap();
    assert_eq!(handed_off_facts(&db), 1);
}
