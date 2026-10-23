//! F13 public-pipeline checks for live SQLite cutoffs and durable row history.
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use dendro_connectors::{ingest_staged_with_manifest, SourceRoot};
use dendro_core::{report_json, run_locked_with_manifest, trace_json, CutoffManifest};
use rusqlite::{params, Connection};
use serde_json::{json, Value};

fn database_bytes(db: &Path) -> Vec<u8> {
    let request = dendro_core::presentation::ReadRequest::open(db).unwrap();
    fs::read(request.database()).unwrap()
}

const FROM: &str = "2026-10-09T00:00:00Z";
const TO: &str = "2026-10-10T00:00:00Z";
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    dir: PathBuf,
    db: PathBuf,
    manifest: PathBuf,
    sqlite: PathBuf,
    roots: Vec<SourceRoot>,
}
impl Fixture {
    fn new() -> Self {
        #[cfg(windows)]
        let root = dendro_core::platform_v1::create_user_private_root_with_choice(
            &format!(
                "dendro-integration-{}-",
                NEXT.fetch_add(1, Ordering::Relaxed)
            ),
            &dendro_core::platform_v1::StorageRootChoice::UserProfile,
        )
        .unwrap();
        #[cfg(windows)]
        let dir = root.path().to_path_buf();
        #[cfg(windows)]
        drop(root);
        #[cfg(not(windows))]
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/issue-13-tests")
            .join(format!(
                "openrig-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        fs::create_dir_all(dir.join("claude")).unwrap();
        fs::write(dir.join("claude/session.jsonl"), format!("{}\n",json!({"type":"assistant","sessionId":"session-a","timestamp":"2026-10-09T02:00:00Z","message":{"id":"m1","usage":{"input_tokens":100,"output_tokens":20,"cache_read_input_tokens":0,"cache_creation_input_tokens":0},"content":[]}}))).unwrap();
        let sqlite = dir.join("openrig.sqlite");
        seed_sqlite(&sqlite, "q1", 1, 1, "seat-a");
        Self {
            db: dir.join("analytics.duckdb"),
            manifest: dir.join("cutoff.json"),
            roots: vec![
                SourceRoot {
                    name: "claude".into(),
                    path: dir.join("claude"),
                },
                SourceRoot {
                    name: "openrig".into(),
                    path: sqlite.clone(),
                },
            ],
            dir,
            sqlite,
        }
    }
    fn run(&self, write: bool, sealed: bool) -> anyhow::Result<Value> {
        run(
            &self.db,
            &self.roots,
            write.then_some(self.manifest.as_path()),
            sealed.then_some(self.manifest.as_path()),
        )
    }
    fn payload(&self) -> Value {
        payload(&self.db)
    }
    fn trace(&self) -> Value {
        trace_json(&self.db, "tokens", None, FROM, TO).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn seed_sqlite(path: &Path, item: &str, seq: i64, transition: i64, destination: &str) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch("CREATE TABLE events(seq INTEGER PRIMARY KEY,type TEXT,payload TEXT,created_at TEXT); CREATE TABLE queue_transitions(transition_id INTEGER PRIMARY KEY,qitem_id TEXT,ts TEXT,state TEXT,actor_session TEXT,closure_reason TEXT,closure_target TEXT,transition_note TEXT); CREATE TABLE snapshots(data TEXT); CREATE TABLE rigs(id TEXT,name TEXT); CREATE TABLE queue_items(qitem_id TEXT PRIMARY KEY,destination_session TEXT,ts_updated TEXT,chain_of_record TEXT,handed_off_from TEXT,tags TEXT);").unwrap();
    conn.execute("INSERT INTO rigs VALUES('rig-id','rig-a')", [])
        .unwrap();
    event(
        &conn,
        seq,
        "agent.session_identity",
        json!({"sessionId":"session-a","runtime":"claude","sessionName":"seat-a","rigId":"rig-id"}),
        "2026-10-09T00:00:00Z",
    );
    add_item(&conn, item, seq + 1, transition, destination);
    event(
        &conn,
        seq + 3,
        "seat.activity_changed",
        json!({"sessionName":destination,"state":"running"}),
        "2026-10-09T01:00:00Z",
    );
    conn.execute(
        "INSERT INTO snapshots VALUES (?1)",
        params![json!({"nodes":[],"sessions":[]}).to_string()],
    )
    .unwrap();
}
fn event(conn: &Connection, seq: i64, kind: &str, payload: Value, at: &str) {
    conn.execute(
        "INSERT INTO events VALUES(?1,?2,?3,?4)",
        params![seq, kind, payload.to_string(), at],
    )
    .unwrap();
}
fn add_item(conn: &Connection, item: &str, seq: i64, transition: i64, destination: &str) {
    event(
        conn,
        seq,
        "queue.created",
        json!({"qitemId":item,"sourceSession":"orchestrator","destinationSession":destination,"tags":["mission:mission-a"],"chainOfRecord":[],"handedOffFrom":null}),
        "2026-10-09T01:00:00Z",
    );
    event(
        conn,
        seq + 1,
        "queue.claimed",
        json!({"qitemId":item,"destinationSession":destination}),
        "2026-10-09T01:30:00Z",
    );
    conn.execute("INSERT INTO queue_transitions VALUES(?1,?2,'2026-10-09T03:00:00Z','done',?3,'no-follow-on',NULL,NULL)",params![transition,item,destination]).unwrap();
    conn.execute("INSERT INTO queue_items VALUES(?1,?2,'2026-10-09T03:00:00Z','[]',NULL,'[\"mission:mission-a\"]')",params![item,destination]).unwrap();
}
fn run(
    db: &Path,
    roots: &[SourceRoot],
    write: Option<&Path>,
    sealed: Option<&Path>,
) -> anyhow::Result<Value> {
    let paths = roots
        .iter()
        .map(|root| root.path.clone())
        .collect::<Vec<_>>();
    let openrig = roots
        .iter()
        .filter(|root| root.name == "openrig")
        .map(|root| root.path.clone())
        .collect::<Vec<_>>();
    run_locked_with_manifest(
        db,
        &paths,
        &openrig,
        write,
        sealed,
        |stage, key, manifest| ingest_staged_with_manifest(stage, roots, false, key, manifest),
    )
}
fn payload(db: &Path) -> Value {
    report_json(db, "decisions", None, FROM, TO).unwrap()["payload"].clone()
}
fn totals(p: &Value) -> Value {
    let mut result = json!({});
    for field in [
        "tokens",
        "usage_rows",
        "token_categories",
        "unlinked_tokens",
        "m1_yield",
        "m2_return_rate",
        "m3_chains",
        "m3_rework_chains",
        "mission_tokens",
        "k2_closed_items_with_insufficient_history",
        "activity",
    ] {
        result[field] = p[field].clone();
    }
    result["seats"]=json!(p["m1_by_seat"].as_array().unwrap().iter().map(|seat|json!({"seat":seat["seat"],"tokens":seat["tokens"],"closed_items":seat["closed_items"],"tokens_per_closed_item":seat["tokens_per_closed_item"],"nonreturn_deliveries":seat["nonreturn_deliveries"],"yield":seat["yield"]})).collect::<Vec<_>>());
    result
}
fn assert_trace_recomputes_tokens(fixture: &Fixture) {
    let trace = fixture.trace();
    let tokens: i64 = trace["inputs"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            ["input_uncached", "output", "cache_read", "cache_write"]
                .into_iter()
                .map(|name| row[name].as_i64().unwrap())
                .sum::<i64>()
        })
        .sum();
    assert_eq!(tokens, fixture.payload()["tokens"].as_i64().unwrap());
    assert_eq!(trace["value"], tokens);
}

#[test]
fn sealed_sqlite_replay_excludes_live_growth_and_preserves_every_total() {
    let fixture = Fixture::new();
    fixture.run(true, false).unwrap();
    let baseline = totals(&fixture.payload());
    let conn = Connection::open(&fixture.sqlite).unwrap();
    add_item(&conn, "q2", 5, 2, "seat-a");
    conn.execute("INSERT INTO snapshots VALUES('{}')", [])
        .unwrap();
    drop(conn);
    let source_before = fs::read(&fixture.sqlite).unwrap();
    fixture.run(false, true).unwrap();
    assert_eq!(totals(&fixture.payload()), baseline);
    assert_eq!(fs::read(&fixture.sqlite).unwrap(), source_before);
    fixture.run(false, false).unwrap();
    let p = fixture.payload();
    assert_eq!(p["m1_by_seat"][0]["closed_items"], 2);
    assert_trace_recomputes_tokens(&fixture);
}

#[test]
fn append_keeps_old_fact_generation_and_assigns_new_generation_to_new_rows() {
    let fixture = Fixture::new();
    fixture.run(false, false).unwrap();
    let conn = Connection::open(&fixture.sqlite).unwrap();
    add_item(&conn, "q2", 5, 2, "seat-a");
    drop(conn);
    fixture.run(false, false).unwrap();
    let trace = fixture.trace();
    let rows = trace["inputs"]["demand_events"].as_array().unwrap();
    assert!(rows
        .iter()
        .any(|row| row["source_key"] == "queue_transitions:1" && row["generation"] == 1));
    assert!(rows
        .iter()
        .any(|row| row["source_key"] == "queue_transitions:2@g2" && row["generation"] == 2));
    let old = rows
        .iter()
        .find(|row| row["source_key"] == "queue_transitions:1")
        .unwrap();
    let new = rows
        .iter()
        .find(|row| row["source_key"] == "queue_transitions:2@g2")
        .unwrap();
    assert_ne!(old["producing_revision"], new["producing_revision"]);
    assert_eq!(old["row_state"], "present");
    assert_eq!(old["state"], "done");
    assert_trace_recomputes_tokens(&fixture);
}

#[test]
fn retention_keeps_totals_and_marks_deleted_rows_gone_with_producing_generation() {
    let fixture = Fixture::new();
    fixture.run(false, false).unwrap();
    let baseline = totals(&fixture.payload());
    Connection::open(&fixture.sqlite)
        .unwrap()
        .execute_batch("DELETE FROM queue_transitions; DELETE FROM snapshots;")
        .unwrap();
    fixture.run(false, false).unwrap();
    assert_eq!(totals(&fixture.payload()), baseline);
    let trace = fixture.trace();
    for table in ["queue_transitions", "snapshots"] {
        assert!(trace["inputs"]["openrig_row_states"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["table_name"] == table
                && row["state"] == "gone"
                && row["generation"] == 1
                && row["status"] == "no longer available"));
    }
    assert!(trace["inputs"]["demand_events"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["source_key"] == "queue_transitions:1"
            && row["row_state"] == "gone"
            && row["status"] == "no longer available"
            && row["generation"] == 1));
    assert_trace_recomputes_tokens(&fixture);
    fs::remove_file(&fixture.sqlite).unwrap();
    let trace = fixture.trace();
    assert!(trace["inputs"]["openrig_sources"]
        .as_array()
        .unwrap()
        .iter()
        .all(|source| source["state"] == "gone"));
    assert_eq!(fixture.payload()["tokens"], 120);
}

#[test]
fn replacement_reused_keys_create_new_generation_without_silently_dropping_rows() {
    let fixture = Fixture::new();
    fixture.run(false, false).unwrap();
    fs::rename(&fixture.sqlite, fixture.dir.join("retired.sqlite")).unwrap();
    seed_sqlite(&fixture.sqlite, "replacement-item", 1, 1, "seat-a");
    fixture.run(false, false).unwrap();
    let trace = fixture.trace();
    let states = trace["inputs"]["openrig_row_states"].as_array().unwrap();
    assert!(states
        .iter()
        .any(|row| row["table_name"] == "queue_transitions"
            && row["row_id"] == "1"
            && row["generation"] == 1
            && ["changed", "superseded"].contains(&row["state"].as_str().unwrap_or(""))));
    assert!(trace["inputs"]["demand_events"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["qitem_id"] == "replacement-item" && row["generation"] == 2));
    assert_eq!(fixture.payload()["m1_by_seat"][0]["closed_items"], 2);
    assert_trace_recomputes_tokens(&fixture);
}

#[test]
fn sqlite_mismatch_refuses_publish_and_unsealed_w3_reports_changed_source() {
    for mutation in ["DELETE FROM events WHERE seq=2", "UPDATE events SET payload='{\"qitemId\":\"q1\",\"destinationSession\":\"seat-b\"}' WHERE seq=2", "DELETE FROM queue_transitions", "UPDATE queue_transitions SET closure_reason='cancelled' WHERE transition_id=1", "DELETE FROM snapshots", "UPDATE snapshots SET data='{\"changed\":true}'"] {
        let fixture=Fixture::new(); fixture.run(true,false).unwrap(); let bytes=database_bytes(&fixture.db);
        let manifest:CutoffManifest=serde_json::from_slice(&fs::read(&fixture.manifest).unwrap()).unwrap();
        Connection::open(&fixture.sqlite).unwrap().execute_batch(mutation).unwrap();
        let error=fixture.run(false,true).unwrap_err().to_string(); assert!(error.contains("mismatch"),"{error}"); assert!(error.contains(&manifest.openrig[0].source_id),"{error}"); assert_eq!(database_bytes(&fixture.db),bytes);
        if mutation.starts_with("UPDATE events") {
            let result=fixture.run(false,false).unwrap(); assert!(result["sources"].as_array().unwrap().iter().any(|note|note.as_str().unwrap().contains("openrig source changed")));
            let trace=fixture.trace(); assert!(trace["inputs"]["openrig_row_states"].as_array().unwrap().iter().any(|row|row["state"]=="changed"));
            assert!(trace["inputs"]["demand_events"].as_array().unwrap().iter().any(|row|row["source_key"]=="events:queue.created:2@g2" && row["dest"]=="seat-b" && row["generation"]==2));
            assert_trace_recomputes_tokens(&fixture);
        }
    }
}

#[test]
fn mutable_creation_backing_reproduces_totals_and_unsupported_fields_disclose_in_report_and_trace()
{
    let fixture = Fixture::new();
    fixture.run(true, false).unwrap();
    let baseline = totals(&fixture.payload());
    Connection::open(&fixture.sqlite).unwrap().execute_batch("UPDATE queue_items SET destination_session='seat-b',tags='[\"mission:other\"]',chain_of_record='[\"other\"]',handed_off_from='other',ts_updated='2026-10-09T04:00:00Z'").unwrap();
    fixture.run(false, true).unwrap();
    let report = fixture.payload();
    assert_eq!(totals(&report), baseline);
    assert!(report["mutable_source_disclosures"]
        .as_array()
        .unwrap()
        .is_empty());

    let unsupported = Fixture::new();
    Connection::open(&unsupported.sqlite)
        .unwrap()
        .execute(
            "UPDATE events SET payload=?1 WHERE seq=2",
            params![json!({"qitemId":"q1","destinationSession":"seat-a"}).to_string()],
        )
        .unwrap();
    unsupported.run(true, false).unwrap();
    Connection::open(&unsupported.sqlite)
        .unwrap()
        .execute_batch(
            "UPDATE queue_items SET tags='[\"mission:other\"]',chain_of_record='[\"other\"]'",
        )
        .unwrap();
    unsupported.run(false, true).unwrap();
    let report = unsupported.payload();
    let trace = unsupported.trace();
    assert_eq!(
        report["m1_by_seat"][0]["tokens_per_closed_item"],
        "not reproducible: mutable source"
    );
    assert_eq!(
        report["mission_tokens"]["status"],
        "not reproducible: mutable source"
    );
    for disclosures in [
        &report["mutable_source_disclosures"],
        &trace["inputs"]["mutable_source_disclosures"],
    ] {
        assert!(disclosures
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["qitem_id"] == "q1"
                && item["reason"]
                    .as_str()
                    .unwrap()
                    .contains("tags changed without queue.created backing")));
    }
    assert_eq!(
        trace["inputs"]["m1_by_seat"][0]["tokens_per_closed_item"],
        "not reproducible: mutable source"
    );
}
