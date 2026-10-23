//! F13-5 must disclose the original affected seat when its fallback is lost.
use std::fs;
#[cfg(not(windows))]
use std::path::Path;
use std::path::PathBuf;

use dendro_connectors::{ingest_staged_with_manifest, SourceRoot};
use dendro_core::{report_json, run_locked_with_manifest, trace_json};
use rusqlite::{params, Connection};
use serde_json::{json, Value};

const FROM: &str = "2026-10-09T00:00:00Z";
const TO: &str = "2026-10-10T00:00:00Z";
struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn seat<'a>(payload: &'a Value, name: &str) -> &'a Value {
    payload["m1_by_seat"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["seat"] == name)
        .unwrap()
}

fn check_changed_fallback(creation_mode: u8) {
    #[cfg(windows)]
    let root = dendro_core::platform_v1::create_user_private_root_with_choice(
        "dendro-mutable-fallback-",
        &dendro_core::platform_v1::StorageRootChoice::UserProfile,
    )
    .unwrap();
    #[cfg(windows)]
    let directory = root.path().to_path_buf();
    #[cfg(windows)]
    drop(root);
    #[cfg(not(windows))]
    let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/issue-13-tests")
        .join(format!(
            "mutable-fallback-{}-{creation_mode}",
            std::process::id()
        ));
    fs::create_dir_all(directory.join("claude")).unwrap();
    let scratch = Scratch(directory);
    let db = scratch.0.join("analytics.duckdb");
    let manifest = scratch.0.join("cutoff.json");
    let source = scratch.0.join("openrig.sqlite");
    fs::write(scratch.0.join("claude/session.jsonl"),format!("{}\n",json!({"type":"assistant","sessionId":"session-c","timestamp":"2026-10-09T02:00:00Z","message":{"id":"m1","usage":{"input_tokens":10,"output_tokens":2,"cache_read_input_tokens":3,"cache_creation_input_tokens":4}}}))).unwrap();
    let conn = Connection::open(&source).unwrap();
    conn.execute_batch("CREATE TABLE events(seq INTEGER PRIMARY KEY,type TEXT,payload TEXT,created_at TEXT);
        CREATE TABLE queue_transitions(transition_id INTEGER PRIMARY KEY,qitem_id TEXT,ts TEXT,state TEXT,actor_session TEXT,closure_reason TEXT,closure_target TEXT,transition_note TEXT);
        CREATE TABLE queue_items(qitem_id TEXT PRIMARY KEY,destination_session TEXT,ts_updated TEXT,chain_of_record TEXT,handed_off_from TEXT,tags TEXT);
        CREATE TABLE rigs(id TEXT,name TEXT);
        INSERT INTO rigs VALUES('rig','rig');
        INSERT INTO queue_transitions VALUES(1,'q1','2026-10-09T03:00:00Z','done','old-seat','no-follow-on',NULL,NULL);
        INSERT INTO queue_items VALUES('q1','old-seat','2026-10-09T03:00:00Z','[]',NULL,'[]');").unwrap();
    conn.execute("INSERT INTO events VALUES(1,'agent.session_identity',?,'2026-10-09T00:00:00Z')",params![json!({"sessionId":"session-c","runtime":"claude","sessionName":"old-seat","rigId":"rig"}).to_string()]).unwrap();
    if creation_mode == 1 {
        // K2 can use a mutable snapshot for a closure preceding creation.
        // A later destination event does not prove that earlier destination.
        conn.execute(
            "INSERT INTO events VALUES(2,'queue.created',?,'2026-10-09T04:00:00Z')",
            params![json!({"qitemId":"q1","destinationSession":"future-seat"}).to_string()],
        )
        .unwrap();
    } else if creation_mode == 2 {
        // K2 uses this historical destination, so the mutable fallback is not
        // needed even when it differs from the destination at creation.
        conn.execute(
            "INSERT INTO events VALUES(2,'queue.created',?,'2026-10-09T01:00:00Z')",
            params![json!({"qitemId":"q1","destinationSession":"old-seat"}).to_string()],
        )
        .unwrap();
        conn.execute(
            "UPDATE queue_items SET destination_session='initial-fallback'",
            [],
        )
        .unwrap();
    }
    drop(conn);
    let roots = [
        SourceRoot {
            name: "claude".into(),
            path: scratch.0.join("claude"),
        },
        SourceRoot {
            name: "openrig".into(),
            path: source.clone(),
        },
    ];
    let paths = roots
        .iter()
        .map(|root| root.path.clone())
        .collect::<Vec<_>>();
    let openrig = [source.clone()];
    run_locked_with_manifest(
        &db,
        &paths,
        &openrig,
        Some(&manifest),
        None,
        |stage, key, cutoff| {
            ingest_staged_with_manifest(stage, &roots, false, key, cutoff)
                .map_err(|e| e.context("fixture connector ingest closure"))
        },
    )
    .unwrap();
    let original = report_json(&db, "decisions", None, FROM, TO).unwrap();
    let before = &seat(&original["payload"], "old-seat")["tokens_per_closed_item"];
    if creation_mode > 0 {
        assert_eq!(before, 19.0);
    } else {
        assert_eq!(before, "no data");
    }
    Connection::open(&source)
        .unwrap()
        .execute("UPDATE queue_items SET destination_session='new-seat'", [])
        .unwrap();
    run_locked_with_manifest(
        &db,
        &paths,
        &openrig,
        None,
        Some(&manifest),
        |stage, key, cutoff| {
            ingest_staged_with_manifest(stage, &roots, false, key, cutoff)
                .map_err(|e| e.context("fixture connector ingest closure"))
        },
    )
    .unwrap();
    let report = report_json(&db, "decisions", None, FROM, TO).unwrap();
    let trace = trace_json(&db, "m1", None, FROM, TO).unwrap();
    for payload in [&report["payload"], &trace["inputs"]] {
        if creation_mode == 2 {
            assert_eq!(seat(payload, "old-seat")["tokens_per_closed_item"], 19.0);
            assert!(payload["mutable_source_disclosures"]
                .as_array()
                .unwrap()
                .is_empty());
            continue;
        }
        assert_eq!(
            seat(payload, "old-seat")["tokens_per_closed_item"],
            "not reproducible: mutable source"
        );
        assert!(payload["mutable_source_disclosures"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["reason"]
                .as_str()
                .unwrap()
                .contains("destination_session")));
    }
}

#[test]
fn changed_fallback_destination_without_creation_discloses_original_seat() {
    check_changed_fallback(0);
}

#[test]
fn future_creation_cannot_reconstruct_an_earlier_mutated_fallback_destination() {
    check_changed_fallback(1);
}

#[test]
fn historical_creation_reproduces_metrics_when_mutated_fallback_is_unnecessary() {
    check_changed_fallback(2);
}
