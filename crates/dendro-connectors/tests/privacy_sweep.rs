//! Gap 1: every credential family through every reachable source text field,
//! then every report kind and trace id (all rigs and one linked rig), the
//! database bytes and its WAL. Each field carries its own tag, so a leak names
//! the field. Ordinary values must survive and distinct secrets stay distinct.

use std::fs;
use std::path::Path;

use dendro_connectors::{ingest_staged, SourceRoot};
use dendro_core::{report_json, trace_json, AnalyticsStore};
use serde_json::{json, Value};

const FAMILIES: &[&str] = &[
    "sk-",
    "sk-ant-",
    "ghp_",
    "gho_",
    "github_pat_",
    "xai-",
    "AKIA",
    "Bearer ",
    "xoxb-",
    "xoxp-",
];
const REPORTS: &[&str] = &["links", "activity", "handoffs", "tokens", "decisions"];
const TRACES: &[&str] = &[
    "m1", "m2", "m3", "m4", "tokens", "intake", "medians", "activity",
];
const FROM: &str = "2026-10-09T00:00:00Z";
const TO: &str = "2026-10-10T00:00:00Z";
const RIG: &str = "rig-plain";

/// Every source text field the sweep injects, by tag.
const FIELDS: &[&str] = &[
    "dir", "csess", "csess2", "cmsg", "tname", "tinput", "xdir", "xsess", "pdir", "pmsg", "rigid",
    "rigname", "isess", "runtime", "iseat", "seat1", "seat2", "state", "node", "node2", "q1", "q2",
    "q3", "reroute", "note", "reason", "target", "tstate", "cutoff", "mission", "logical",
    "agentref", "spec",
];

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

/// Builds the sources for one family; returns the roots.
fn sources(dir: &Path, c: &dyn Fn(&str) -> String) -> Vec<SourceRoot> {
    let (s1, s2) = (c("seat1"), c("seat2"));
    let claude = dir.join("claude");
    let usage = |session: &str, id: &str| {
        json!({"type": "assistant", "sessionId": session, "timestamp": "2026-10-09T01:00:00Z",
            "message": {"id": id, "usage": {"input_tokens": 5, "output_tokens": 1,
                "cache_read_input_tokens": 0, "cache_creation_input_tokens": 0},
                "content": [{"type": "tool_use", "name": c("tname"), "input": c("tinput"), "is_error": true}]}})
        .to_string()
    };
    write(
        &claude.join(c("dir")).join("s.jsonl"),
        &format!(
            "{}\n{}\n{}\n",
            usage(&c("csess"), &c("cmsg")),
            usage(&c("csess2"), "m-plain"),
            usage("plain-session", "m-plain-2"),
        ),
    );
    let codex = dir.join("codex");
    write(
        &codex.join(c("xdir")).join("s.jsonl"),
        &format!(
            "{}\n",
            json!({"type": "event_msg", "timestamp": "2026-10-09T01:00:00Z",
                "payload": {"type": "token_count", "session_id": c("xsess"),
                    "info": {"total_token_usage": {"input_tokens": 9, "cached_input_tokens": 1,
                        "cache_write_input_tokens": 0, "output_tokens": 2, "reasoning_output_tokens": 0}}}})
        ),
    );
    let pi = dir.join("pi");
    write(
        &pi.join(c("pdir")).join("s.jsonl"),
        &format!(
            "{}\n",
            json!({"type": "message", "id": c("pmsg"), "timestamp": "2026-10-09T01:00:00Z",
                "message": {"role": "assistant", "usage": {"input": 3, "output": 1, "cacheRead": 0, "cacheWrite": 0}}})
        ),
    );
    let sqlite = dir.join("openrig.sqlite");
    let conn = rusqlite::Connection::open(&sqlite).unwrap();
    conn.execute_batch(
        "CREATE TABLE events (seq INTEGER, type TEXT, payload TEXT, created_at TEXT);
         CREATE TABLE rigs (id TEXT, name TEXT);
         CREATE TABLE queue_transitions (transition_id INTEGER, qitem_id TEXT, ts TEXT, state TEXT,
           actor_session TEXT, closure_reason TEXT, closure_target TEXT, transition_note TEXT);
         CREATE TABLE queue_items (qitem_id TEXT, destination_session TEXT, ts_updated TEXT,
           chain_of_record TEXT, handed_off_from TEXT, tags TEXT);
         CREATE TABLE snapshots (data TEXT);",
    )
    .unwrap();
    for (id, name) in [
        ("rid-plain".to_string(), RIG.to_string()),
        (c("rigid"), c("rigname")),
    ] {
        conn.execute("INSERT INTO rigs VALUES (?, ?)", [id, name])
            .unwrap();
    }
    let (q1, q2) = (c("q1"), c("q2"));
    let events = [
        (
            "agent.session_identity",
            json!({"sessionId": c("csess"), "runtime": "claude-code", "sessionName": s1, "rigId": "rid-plain"}),
        ),
        (
            "agent.session_identity",
            json!({"sessionId": c("csess2"), "runtime": "claude-code", "sessionName": s2, "rigId": "rid-plain"}),
        ),
        (
            "agent.session_identity",
            json!({"sessionId": c("isess"), "runtime": c("runtime"), "sessionName": c("iseat"), "rigId": c("rigid")}),
        ),
        (
            "seat.activity_changed",
            json!({"sessionName": s1, "state": "running", "at": "2026-10-09T00:40:00Z"}),
        ),
        (
            "seat.activity_changed",
            json!({"sessionName": s2, "state": c("state"), "at": "2026-10-09T00:41:00Z"}),
        ),
        (
            "node.launched",
            json!({"rigId": "rid-plain", "nodeId": c("node"), "sessionName": s1}),
        ),
        (
            "session.stopped",
            json!({"rigId": "rid-plain", "nodeId": c("node2"), "sessionName": s2}),
        ),
        (
            "node.removed",
            json!({"rigId": "rid-plain", "nodeId": c("node")}),
        ),
        (
            "queue.created",
            json!({"qitemId": q1, "sourceSession": s2, "destinationSession": s1}),
        ),
        (
            "queue.claimed",
            json!({"qitemId": q1, "destinationSession": s1, "claimedAt": "2026-10-09T00:50:00Z"}),
        ),
        (
            "queue.created",
            json!({"qitemId": q2, "sourceSession": s1, "destinationSession": s2}),
        ),
        (
            "qitem.fallback_routed",
            json!({"qitemId": c("q3"), "originalDestination": s1, "rerouteDestination": c("reroute"), "reason": "x"}),
        ),
        (
            "queue.unclaimed",
            json!({"qitemId": q2, "destinationSession": s2}),
        ),
    ];
    for (seq, (kind, payload)) in events.iter().enumerate() {
        conn.execute(
            "INSERT INTO events VALUES (?, ?, ?, '2026-10-09 00:30:00')",
            rusqlite::params![seq as i64 + 1, kind, payload.to_string()],
        )
        .unwrap();
    }
    let transitions = [
        (
            1,
            q1.clone(),
            "handed-off".to_string(),
            s1.clone(),
            "handed_off_to".to_string(),
            s2.clone(),
            c("note"),
        ),
        (
            2,
            q2.clone(),
            "done".to_string(),
            s2.clone(),
            c("reason"),
            c("target"),
            "fallback-routed: a -> b (x)".to_string(),
        ),
        (
            3,
            q2.clone(),
            c("tstate"),
            s2.clone(),
            "no-follow-on".to_string(),
            s1.clone(),
            "plain note".to_string(),
        ),
    ];
    for (id, item, state, actor, reason, target, note) in transitions {
        conn.execute(
            "INSERT INTO queue_transitions VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params![
                id,
                item,
                format!("2026-10-09T01:1{id}:00.000Z"),
                state,
                actor,
                reason,
                target,
                note
            ],
        )
        .unwrap();
    }
    conn.execute(
        "INSERT INTO queue_items VALUES (?, ?, ?, '[]', NULL, ?)",
        rusqlite::params![
            q1,
            s1,
            c("cutoff"),
            json!([format!("mission:{}", c("mission"))]).to_string()
        ],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO queue_items VALUES (?, ?, '2026-10-09T01:12:00.000Z', ?, ?, ?)",
        rusqlite::params![
            q2,
            s2,
            json!([q1]).to_string(),
            q1,
            json!(["mission:plain-mission"]).to_string()
        ],
    )
    .unwrap();
    let snapshot = json!({
        "nodes": [{"id": "n1", "logicalId": c("logical"), "agentRef": c("agentref"), "resolvedSpecName": c("spec")}],
        "sessions": [{"nodeId": "n1", "sessionName": s1}],
    });
    conn.execute("INSERT INTO snapshots VALUES (?)", [snapshot.to_string()])
        .unwrap();
    drop(conn);
    vec![
        SourceRoot {
            name: "claude".into(),
            path: claude,
        },
        SourceRoot {
            name: "codex".into(),
            path: codex,
        },
        SourceRoot {
            name: "pi".into(),
            path: pi,
        },
        SourceRoot {
            name: "openrig".into(),
            path: sqlite,
        },
    ]
}

fn leaks(text: &str, family: usize) -> Vec<&'static str> {
    FIELDS
        .iter()
        .copied()
        .filter(|field| text.contains(&format!("ZZ{family}x{field}")))
        .collect()
}

#[test]
fn every_family_through_every_source_field_stays_out_of_storage_and_output() {
    let mut assertions = 0usize;
    for (family, marker) in FAMILIES.iter().enumerate() {
        let c = |field: &str| format!("{marker}ZZ{family}x{field}");
        let dir =
            std::env::temp_dir().join(format!("dendro-sweep-{}-{family}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let roots = sources(&dir, &c);
        let db = dir.join("analytics.duckdb");
        drop(AnalyticsStore::open_path(&db, "key").unwrap());
        let notes = ingest_staged(&db, &roots, false, b"sweep-key-32-bytes-pad-pad-pad!!").unwrap();
        assert!(
            leaks(&notes.join("\n"), family).is_empty(),
            "{marker}: ingest notes"
        );

        // Every report kind and trace id, unscoped and for the linked rig.
        let mut outputs: Vec<(String, Value)> = Vec::new();
        for rig in [None, Some(RIG)] {
            for kind in REPORTS {
                outputs.push((
                    format!("report {kind} {rig:?}"),
                    report_json(&db, kind, rig, FROM, TO).unwrap(),
                ));
            }
            for id in TRACES {
                outputs.push((
                    format!("trace {id} {rig:?}"),
                    trace_json(&db, id, rig, FROM, TO).unwrap(),
                ));
            }
        }
        for (name, value) in &outputs {
            let found = leaks(&value.to_string(), family);
            assert!(found.is_empty(), "{marker}: {name} leaks fields {found:?}");
            assertions += 1;
        }

        // Nonempty linked-rig outputs, so the scan read real values.
        let linked = report_json(&db, "decisions", Some(RIG), FROM, TO).unwrap();
        let p = &linked["payload"];
        assert!(
            !p["m1_by_seat"].as_array().unwrap().is_empty(),
            "{marker}: m1 rows"
        );
        assert!(
            !p["handoff_members"].as_array().unwrap().is_empty(),
            "{marker}: handoffs"
        );
        assert!(
            !p["activity"].as_array().unwrap().is_empty(),
            "{marker}: activity"
        );
        assert!(
            !p["mission_by_seat"].as_array().unwrap().is_empty(),
            "{marker}: missions"
        );
        assert!(p["tokens"].as_i64().unwrap() > 0, "{marker}: tokens");
        assertions += 5;

        // Ordinary values survive.
        let all = outputs
            .iter()
            .map(|(_, v)| v.to_string())
            .collect::<String>();
        for plain in ["plain-mission", "handed_off_to", "running"] {
            assert!(all.contains(plain), "{marker}: ordinary {plain}");
            assertions += 1;
        }

        // Distinct secrets stay distinct; tools join their usage session.
        let store = AnalyticsStore::open_path(&db, "key").unwrap();
        let mut sessions: Vec<String> = store
            .load_usage()
            .unwrap()
            .into_iter()
            .filter(|row| row.harness == "claude")
            .map(|row| row.native_session)
            .collect();
        sessions.sort();
        sessions.dedup();
        assert_eq!(
            sessions.len(),
            3,
            "{marker}: two secret and one plain session stay apart"
        );
        assert!(sessions.contains(&"plain-session".to_string()));
        let mut tool_sessions: Vec<String> = store
            .load_tools()
            .unwrap()
            .into_iter()
            .map(|row| row.native_session)
            .collect();
        tool_sessions.sort();
        tool_sessions.dedup();
        assert_eq!(
            tool_sessions, sessions,
            "{marker}: tool sessions are the usage sessions"
        );
        let seats: Vec<String> = store
            .load_identity()
            .unwrap()
            .into_iter()
            .map(|row| row.seat)
            .collect();
        assert_eq!(
            seats
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            3,
            "{marker}: secret seats keep distinct opaque ids"
        );
        assertions += 4;
        drop(store);

        // Database bytes and any WAL.
        for entry in fs::read_dir(&dir).unwrap().flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("analytics.duckdb") {
                let bytes = fs::read(entry.path()).unwrap();
                let found = leaks(&String::from_utf8_lossy(&bytes), family);
                assert!(
                    found.is_empty(),
                    "{marker}: {name} bytes leak fields {found:?}"
                );
                assertions += 1;
            }
        }
        let _ = fs::remove_dir_all(&dir);
    }
    println!(
        "privacy sweep: {} families x {} fields, {assertions} assertions",
        FAMILIES.len(),
        FIELDS.len()
    );
    assert!(assertions >= FAMILIES.len() * (REPORTS.len() + TRACES.len()) * 2);
}

/// Negative control: an ordinary, non-secret marker must reach storage and
/// output, so the sweep above can see a leak. Prints which source fields reach
/// report or trace output (the source-to-output map).
#[test]
fn the_sweep_sees_ordinary_values_that_reach_output() {
    let family = 99;
    let c = |field: &str| format!("plainZZ{family}x{field}");
    let dir = std::env::temp_dir().join(format!("dendro-sweep-control-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let roots = sources(&dir, &c);
    let db = dir.join("analytics.duckdb");
    drop(AnalyticsStore::open_path(&db, "key").unwrap());
    ingest_staged(&db, &roots, false, b"sweep-key-32-bytes-pad-pad-pad!!").unwrap();
    let mut all = String::new();
    for rig in [None, Some(RIG)] {
        for kind in REPORTS {
            all.push_str(&report_json(&db, kind, rig, FROM, TO).unwrap().to_string());
        }
        for id in TRACES {
            all.push_str(&trace_json(&db, id, rig, FROM, TO).unwrap().to_string());
        }
    }
    let reached = leaks(&all, family);
    println!("fields reaching output: {reached:?}");
    for field in ["seat1", "seat2", "q1", "q2", "csess", "mission"] {
        assert!(
            reached.contains(&field),
            "control field {field} must reach output"
        );
    }
    let bytes: String = fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("analytics.duckdb")
        })
        .map(|e| String::from_utf8_lossy(&fs::read(e.path()).unwrap()).to_string())
        .collect();
    let stored = leaks(&bytes, family);
    println!("fields reaching storage: {stored:?}");
    assert!(stored.len() >= reached.len());
    let _ = fs::remove_dir_all(&dir);
}
