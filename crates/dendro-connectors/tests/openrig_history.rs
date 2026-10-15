//! Accepted OpenRig history keeps its original producing ingest and survives retention.
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use dendro_connectors::{ingest_staged, SourceRoot};
use dendro_core::{report_json, run_locked_with_openrig, trace_json, AnalyticsStore};
use rusqlite::{params, Connection};
use serde_json::{json, Value};

const FROM: &str = "2026-10-09T00:00:00Z";
const TO: &str = "2026-10-10T00:00:00Z";
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/issue-13-tests")
            .join(format!(
                "history-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn fixture(path: &Path, qitem: &str) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch("CREATE TABLE events(seq INTEGER PRIMARY KEY,type TEXT,payload TEXT,created_at TEXT);
        CREATE TABLE queue_transitions(transition_id INTEGER PRIMARY KEY,qitem_id TEXT,ts TEXT,state TEXT,actor_session TEXT,closure_reason TEXT,closure_target TEXT,transition_note TEXT);
        CREATE TABLE queue_items(qitem_id TEXT PRIMARY KEY,destination_session TEXT,ts_updated TEXT,chain_of_record TEXT,handed_off_from TEXT,tags TEXT);
        CREATE TABLE snapshots(data TEXT);
        CREATE TABLE rigs(id TEXT,name TEXT);").unwrap();
    conn.execute(
        "INSERT INTO events VALUES(1,'agent.session_identity',?,'2026-10-09T01:00:00Z')",
        [
            json!({"sessionId":"session-a","runtime":"claude","sessionName":"seat-a","rigId":"r"})
                .to_string(),
        ],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO events VALUES(2,'seat.activity_changed',?,'2026-10-09T01:00:00Z')",
        [json!({"sessionName":"seat-a","state":"running"}).to_string()],
    )
    .unwrap();
    conn.execute("INSERT INTO events VALUES(3,'queue.created',?,'2026-10-09T01:00:00Z')",
        [json!({"qitemId":qitem,"destinationSession":"seat-a","tags":[],"chainOfRecord":[],"handedOffFrom":null}).to_string()]).unwrap();
    conn.execute(
        "INSERT INTO queue_items VALUES(?,'seat-a','2026-10-09T02:00:00Z','[]',NULL,'[]')",
        [qitem],
    )
    .unwrap();
    conn.execute("INSERT INTO queue_transitions VALUES(1,?,'2026-10-09T02:00:00Z','done','seat-a','handed_off_to','seat-b',NULL)",[qitem]).unwrap();
    conn.execute("INSERT INTO snapshots VALUES(?)",[json!({"nodes":[{"id":"n","resolvedSpecName":"agentflow-seat","logicalId":"dev.qa","skills":["skill-a"]}],"sessions":[{"sessionName":"seat-a","nodeId":"n"}]}).to_string()]).unwrap();
}
fn run(db: &Path, source: &Path) -> Value {
    let roots = [SourceRoot {
        name: "openrig".into(),
        path: source.into(),
    }];
    let paths = [source.to_path_buf()];
    run_locked_with_openrig(db, &paths, &paths, |stage, key| {
        ingest_staged(stage, &roots, false, key)
    })
    .unwrap()
}
fn report(db: &Path) -> Value {
    report_json(db, "decisions", None, FROM, TO).unwrap()["payload"].clone()
}
fn trace(db: &Path) -> Value {
    trace_json(db, "m1", None, FROM, TO).unwrap()["inputs"].clone()
}

#[test]
fn appended_facts_keep_their_first_generation_and_revision() {
    let dir = Scratch::new();
    let source = dir.0.join("openrig.sqlite");
    let db = dir.0.join("analytics.duckdb");
    fixture(&source, "q1");
    run(&db, &source);
    let first = report(&db);
    let old = first["activity_events"][0].clone();
    assert_eq!(old["generation"], 1);
    let conn = Connection::open(&source).unwrap();
    conn.execute(
        "INSERT INTO events VALUES(4,'seat.activity_changed',?,'2026-10-09T03:00:00Z')",
        [json!({"sessionName":"seat-a","state":"idle"}).to_string()],
    )
    .unwrap();
    drop(conn);
    run(&db, &source);
    let second = report(&db);
    assert_eq!(second["activity_events"][0]["generation"], 1);
    assert_eq!(
        second["activity_events"][0]["producing_revision"],
        old["producing_revision"]
    );
    assert_eq!(second["activity_events"][1]["generation"], 2);
    assert_ne!(
        second["activity_events"][1]["producing_revision"],
        old["producing_revision"]
    );
    assert_eq!(second["demand_events"][0]["generation"], 1);
    assert_eq!(second["links"][0]["generation"], 1);
    assert_eq!(second["destination_snapshots"][0]["generation"], 1);
}

#[test]
fn retention_and_missing_source_preserve_totals_and_expose_gone_evidence() {
    let dir = Scratch::new();
    let source = dir.0.join("openrig.sqlite");
    let db = dir.0.join("analytics.duckdb");
    fixture(&source, "q1");
    run(&db, &source);
    let before = report(&db);
    let store = AnalyticsStore::open_read_only(&db).unwrap();
    let roles = store.load_seat_roles().unwrap().len();
    let skills = store.load_skills().unwrap().len();
    drop(store);
    let conn = Connection::open(&source).unwrap();
    conn.execute_batch("DELETE FROM queue_transitions; DELETE FROM snapshots;")
        .unwrap();
    drop(conn);
    run(&db, &source);
    let after = report(&db);
    for key in [
        "tokens",
        "m1_yield",
        "m2_pairs",
        "m2_return_rate",
        "m3_rework_chains",
        "activity",
        "mission_tokens",
    ] {
        assert_eq!(before[key], after[key], "retention changed {key}");
    }
    for (old, new) in before["m1_by_seat"]
        .as_array()
        .unwrap()
        .iter()
        .zip(after["m1_by_seat"].as_array().unwrap())
    {
        for key in [
            "tokens",
            "nonreturn_deliveries",
            "yield",
            "closed_items",
            "tokens_per_closed_item",
        ] {
            assert_eq!(old[key], new[key], "retention changed seat total {key}");
        }
    }
    let store = AnalyticsStore::open_read_only(&db).unwrap();
    assert_eq!(store.load_seat_roles().unwrap().len(), roles);
    assert_eq!(store.load_skills().unwrap().len(), skills);
    drop(store);
    let evidence = trace(&db);
    assert!(evidence["openrig_row_states"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["table_name"] == "snapshots" && r["state"] == "gone" && r["generation"] == 1));
    let handoff = &evidence["handoff_members"][0];
    assert_eq!(handoff["status"], "no longer available");
    assert_eq!(handoff["generation"], 1);
    fs::remove_file(&source).unwrap();
    let missing = trace(&db);
    assert_eq!(missing["openrig_sources"][0]["state"], "gone");
    assert!(missing["openrig_row_states"]
        .as_array()
        .unwrap()
        .iter()
        .all(|r| r["state"] == "gone"));
    assert_eq!(missing["m1_by_seat"], evidence["m1_by_seat"]);
}

#[test]
fn inode_replacement_reuses_ids_without_dropping_new_facts() {
    let dir = Scratch::new();
    let source = dir.0.join("openrig.sqlite");
    let db = dir.0.join("analytics.duckdb");
    fixture(&source, "q1");
    run(&db, &source);
    let replacement = dir.0.join("replacement.sqlite");
    fixture(&replacement, "q2");
    fs::rename(replacement, &source).unwrap();
    run(&db, &source);
    let report = report(&db);
    let demand = report["demand_events"].as_array().unwrap();
    assert!(demand.iter().any(|r| r["generation"] == 1));
    assert!(demand
        .iter()
        .any(|r| r["generation"] == 2 && r["source_key"].as_str().unwrap().ends_with("@g2")));
    assert_eq!(report["handoff_members"].as_array().unwrap().len(), 2);
    let states = &trace(&db)["openrig_row_states"];
    assert!(states
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["generation"] == 1 && r["state"] == "superseded"));
}

#[test]
fn in_place_change_updates_fact_and_announces_changed_source() {
    let dir = Scratch::new();
    let source = dir.0.join("openrig.sqlite");
    let db = dir.0.join("analytics.duckdb");
    fixture(&source, "q1");
    run(&db, &source);
    let conn = Connection::open(&source).unwrap();
    conn.execute(
        "UPDATE events SET payload=? WHERE seq=2",
        params![json!({"sessionName":"seat-a","state":"idle"}).to_string()],
    )
    .unwrap();
    drop(conn);
    let result = run(&db, &source);
    assert!(result["sources"]
        .as_array()
        .unwrap()
        .iter()
        .any(|note| note.as_str().unwrap().contains("openrig source changed")));
    let report = report(&db);
    assert_eq!(report["openrig_sources"][0]["state"], "changed");
    assert_eq!(report["activity_events"].as_array().unwrap().len(), 1);
    assert_eq!(report["activity_events"][0]["generation"], 2);
    assert_eq!(report["activity_events"][0]["state"], "idle");
    assert_eq!(trace(&db)["activity_events"], report["activity_events"]);
}
