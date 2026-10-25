//! Grok session ingest: usage, joined tool calls, watermarks, and redaction.
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use dendro_connectors::{ingest_staged, SourceRoot};
use dendro_core::AnalyticsStore;
use serde_json::json;

const SESSION: &str = "0199d4c1-8a2e-7f3b-9c41-5e6d7a8b9c0d";
const MODEL: &str = "grok-4.7-build-fast";
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        #[cfg(any(windows, target_os = "linux"))]
        let root = dendro_core::platform_v1::create_user_private_root_with_choice(
            &format!("dendro-grok-{}-", NEXT.fetch_add(1, Ordering::Relaxed)),
            &dendro_core::platform_v1::StorageRootChoice::UserProfile,
        )
        .unwrap();
        #[cfg(any(windows, target_os = "linux"))]
        let path = root.path().to_path_buf();
        #[cfg(any(windows, target_os = "linux"))]
        drop(root);
        #[cfg(not(any(windows, target_os = "linux")))]
        let path = std::env::temp_dir().join(format!(
            "dendro-grok-{}-{}",
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

fn copy_fixture(root: &Path) -> PathBuf {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/grok/session");
    let dest = root
        .join(".grok")
        .join("sessions")
        .join("C%3A%5Cwork%5Cdemo")
        .join(SESSION);
    fs::create_dir_all(&dest).unwrap();
    fs::copy(src.join("updates.jsonl"), dest.join("updates.jsonl")).unwrap();
    fs::copy(src.join("summary.json"), dest.join("summary.json")).unwrap();
    fs::write(
        dest.join("chat_history.jsonl"),
        "{\"text\":\"GROK_CHAT_HISTORY_CANARY\"}\n",
    )
    .unwrap();
    fs::write(root.join(".grok").join("auth.json"), "GROK_AUTH_CANARY").unwrap();
    dest.join("updates.jsonl")
}

fn root_of(scratch: &Scratch) -> SourceRoot {
    SourceRoot {
        name: "grok".into(),
        path: scratch.0.join(".grok"),
    }
}

fn database(scratch: &Scratch) -> PathBuf {
    let db = scratch.0.join("analytics.duckdb");
    drop(AnalyticsStore::open_path(&db, "key").unwrap());
    db
}

fn ingest(db: &Path, scratch: &Scratch) -> Vec<String> {
    ingest_staged(db, &[root_of(scratch)], false, b"grok-v080-test-key").unwrap()
}

fn store(db: &Path) -> AnalyticsStore {
    AnalyticsStore::open_path(db, "key").unwrap()
}

const CANARIES: &[&str] = &[
    "list files",
    "Demo thought: list the directory.",
    "Demo closing line.",
    "target_directory",
    "C:\\work\\demo",
    "Demo session",
    "GROK_CHAT_HISTORY_CANARY",
    "GROK_AUTH_CANARY",
    "fixture-only extra field",
    "unknown key kept",
];

fn assert_no_canaries(db: &Path, notes: &[String]) {
    let notes_text = notes.join("\n");
    for canary in CANARIES {
        assert!(!notes_text.contains(canary), "ingest note leaked {canary}");
        assert!(
            !store(db).contains_text(canary).unwrap(),
            "column leaked {canary}"
        );
    }
    let mut bytes = fs::read(db).unwrap();
    let wal = PathBuf::from(format!("{}.wal", db.display()));
    if wal.exists() {
        bytes.extend(fs::read(&wal).unwrap());
    }
    let text = String::from_utf8_lossy(&bytes);
    for canary in CANARIES {
        assert!(!text.contains(canary), "database file leaked {canary}");
    }
}

#[test]
fn fixture_usage_per_turn_and_model_and_joined_tools() {
    let scratch = Scratch::new();
    let db = database(&scratch);
    copy_fixture(&scratch.0);
    let notes = ingest(&db, &scratch);
    assert_aggregate(&notes, 21, 2);
    assert_eq!(note_coverage(&notes), "complete");
    let store = store(&db);
    let usage = store.load_usage().unwrap();
    assert_eq!(usage.len(), 2);
    let expected = [
        (10, 410, 96, 1900, 0, 48, 1_770_000_002),
        (21, 310, 210, 2700, 0, 75, 1_770_000_005),
    ];
    for (row, (seq, input, output, cache_read, cache_write, reasoning, secs)) in
        usage.iter().zip(expected)
    {
        assert_eq!(row.harness, "grok");
        assert_eq!(row.native_session, SESSION);
        assert_eq!(row.seq, seq);
        assert_eq!(row.input_uncached, input);
        assert_eq!(row.output, output);
        assert_eq!(row.cache_read, cache_read);
        assert_eq!(row.cache_write, cache_write);
        assert_eq!(row.reasoning, reasoning);
        assert_eq!(row.currency_total, None);
        assert_eq!(row.message_id, None);
        assert_eq!(row.ts.unwrap().timestamp(), secs);
    }
    let models = store.load_usage_models().unwrap();
    assert_eq!(models.len(), 2);
    for (key, (model, effort)) in &models {
        assert!(usage.iter().any(|row| row.source_key == *key));
        assert_eq!(model.as_deref(), Some(MODEL));
        assert_eq!(effort.as_deref(), Some("medium"));
    }
    let mut input = 0;
    let mut output = 0;
    let mut cache_read = 0;
    let mut reasoning = 0;
    for row in &usage {
        input += row.input_uncached;
        output += row.output;
        cache_read += row.cache_read;
        reasoning += row.reasoning;
    }
    assert_eq!(
        (input, output, cache_read, reasoning),
        (720, 306, 4600, 123)
    );
    assert_eq!(input + output + cache_read, 5626);
    let tools = store.load_tools().unwrap();
    let names: Vec<_> = tools.iter().map(|tool| tool.name.as_str()).collect();
    assert_eq!(names, ["ListDir", "ReadFile"]);
    assert!(tools.iter().all(|tool| tool.failed == Some(false)));
    assert!(tools.iter().all(|tool| tool.native_session == SESSION));
    let coverage = store.load_source_coverage().unwrap();
    assert_eq!(coverage.len(), 1);
    assert!(coverage[0].complete);
    drop(store);
    assert_no_canaries(&db, &notes);
}

#[test]
fn appending_a_line_reingests_only_the_new_turn() {
    let scratch = Scratch::new();
    let db = database(&scratch);
    let updates = copy_fixture(&scratch.0);
    ingest(&db, &scratch);
    let line = json!({
        "method": "_x.ai/session/update",
        "params": {
            "sessionId": SESSION,
            "update": {
                "sessionUpdate": "turn_completed",
                "usage": {
                    "inputTokens": 100,
                    "outputTokens": 10,
                    "reasoningTokens": 4,
                    "cachedReadTokens": 40,
                    "cacheCreationTokens": 0,
                    "totalTokens": 110,
                    "modelUsage": {
                        "grok-4.7-build-fast": {
                            "inputTokens": 100,
                            "outputTokens": 10,
                            "reasoningTokens": 4,
                            "cachedReadTokens": 40,
                            "cacheCreationTokens": 0,
                            "totalTokens": 110
                        }
                    }
                }
            },
            "timestamp": 1770000006
        }
    });
    let mut file = OpenOptions::new().append(true).open(&updates).unwrap();
    writeln!(file, "{line}").unwrap();
    drop(file);
    let notes = ingest(&db, &scratch);
    assert_aggregate(&notes, 1, 1);
    let usage = store(&db).load_usage().unwrap();
    assert_eq!(usage.len(), 3);
    let added = usage.last().unwrap();
    assert_eq!(added.seq, 22);
    assert_eq!(added.input_uncached, 60);
    assert_eq!(added.output, 10);
    assert_eq!(added.cache_read, 40);
    assert_eq!(added.cache_write, 0);
    assert_eq!(added.reasoning, 4);
    assert_eq!(usage.iter().map(|row| row.input_uncached).sum::<i64>(), 780);
    let again = ingest(&db, &scratch);
    assert_aggregate(&again, 0, 0);
    assert_eq!(store(&db).load_usage().unwrap().len(), 3);
}

#[test]
fn malformed_line_and_unknown_kind_are_counted_not_fatal() {
    let scratch = Scratch::new();
    let db = database(&scratch);
    let updates = copy_fixture(&scratch.0);
    ingest(&db, &scratch);
    let before = store(&db).load_usage().unwrap().len();
    let mut file = OpenOptions::new().append(true).open(&updates).unwrap();
    writeln!(file, "not-json").unwrap();
    let unknown = json!({
        "method": "_x.ai/session/update",
        "params": {
            "sessionId": SESSION,
            "update": { "sessionUpdate": "future_kind" }
        },
        "timestamp": 1770000007
    });
    writeln!(file, "{unknown}").unwrap();
    drop(file);
    let notes = ingest(&db, &scratch);
    assert_aggregate(&notes, 2, 0);
    assert_eq!(note_coverage(&notes), "partial");
    let store = store(&db);
    assert_eq!(store.load_usage().unwrap().len(), before);
    let reasons = store.warning_reasons().unwrap();
    assert!(reasons.iter().any(|reason| reason == "malformed-json"));
    assert!(reasons
        .iter()
        .any(|reason| reason == "unknown-record-type: future_kind"));
    let coverage = store.load_source_coverage().unwrap();
    assert_eq!(coverage.len(), 1);
    assert!(!coverage[0].complete);
}

/// The grok aggregate note carries these counts (plus refused/coverage fields).
fn assert_aggregate(notes: &[String], records_read: u64, usage_records: u64) {
    let counts = format!("records_read={records_read} usage_records={usage_records}");
    assert!(
        notes
            .iter()
            .any(|n| n.starts_with("grok files=1 ") && n.contains(&counts)),
        "{notes:?}"
    );
}

fn note_coverage(notes: &[String]) -> &str {
    let n = notes
        .iter()
        .find(|n| n.starts_with("grok files="))
        .expect("aggregate note");
    if n.contains("coverage=partial") {
        "partial"
    } else {
        "complete"
    }
}

fn append_line(updates: &Path, value: serde_json::Value) {
    let mut file = OpenOptions::new().append(true).open(updates).unwrap();
    writeln!(file, "{value}").unwrap();
}

fn turn(update: serde_json::Value, timestamp: i64) -> serde_json::Value {
    json!({
        "method": "_x.ai/session/update",
        "params": {
            "sessionId": SESSION,
            "update": update
        },
        "timestamp": timestamp
    })
}

#[test]
fn turn_completed_without_usage_is_counted_and_not_warned() {
    let scratch = Scratch::new();
    let db = database(&scratch);
    let updates = copy_fixture(&scratch.0);
    append_line(
        &updates,
        turn(
            json!({"sessionUpdate": "turn_completed", "stop_reason": "end_turn"}),
            1_770_000_006,
        ),
    );
    let notes = ingest(&db, &scratch);
    assert_aggregate(&notes, 22, 2);
    let store = store(&db);
    assert_eq!(store.load_usage().unwrap().len(), 2);
    assert!(
        store.warning_reasons().unwrap().is_empty(),
        "{:?}",
        store.warning_reasons().unwrap()
    );
    assert!(store.load_source_coverage().unwrap()[0].complete);
}

#[test]
fn absent_reasoning_and_cache_creation_default_to_zero() {
    let scratch = Scratch::new();
    let db = database(&scratch);
    let updates = copy_fixture(&scratch.0);
    append_line(
        &updates,
        turn(
            json!({
                "sessionUpdate": "turn_completed",
                "usage": {
                    "inputTokens": 80,
                    "outputTokens": 8,
                    "cachedReadTokens": 30,
                    "modelUsage": {
                        "grok-4.7-build-fast": {
                            "inputTokens": 80,
                            "outputTokens": 8,
                            "cachedReadTokens": 30
                        }
                    }
                }
            }),
            1_770_000_006,
        ),
    );
    ingest(&db, &scratch);
    let usage = store(&db).load_usage().unwrap();
    assert_eq!(usage.len(), 3);
    let added = usage.last().unwrap();
    assert_eq!(added.input_uncached, 50);
    assert_eq!(added.output, 8);
    assert_eq!(added.cache_read, 30);
    assert_eq!(added.cache_write, 0);
    assert_eq!(added.reasoning, 0);
    assert!(
        store(&db).warning_reasons().unwrap().is_empty(),
        "{:?}",
        store(&db).warning_reasons().unwrap()
    );
}

#[test]
fn one_invalid_model_usage_keeps_sibling_models() {
    let scratch = Scratch::new();
    let db = database(&scratch);
    let updates = copy_fixture(&scratch.0);
    append_line(
        &updates,
        turn(
            json!({
                "sessionUpdate": "turn_completed",
                "usage": {
                    "inputTokens": 20,
                    "outputTokens": 2,
                    "cachedReadTokens": 5,
                    "modelUsage": {
                        "grok-4.7-build-fast": {
                            "inputTokens": 20,
                            "outputTokens": 2,
                            "cachedReadTokens": 5,
                            "reasoningTokens": 1,
                            "cacheCreationTokens": 0
                        },
                        "grok-bad-entry": {
                            "outputTokens": 2,
                            "cachedReadTokens": 0
                        }
                    }
                }
            }),
            1_770_000_006,
        ),
    );
    let notes = ingest(&db, &scratch);
    assert_aggregate(&notes, 22, 3);
    let store = store(&db);
    let usage = store.load_usage().unwrap();
    assert_eq!(usage.len(), 3);
    let added = usage.last().unwrap();
    assert_eq!(added.input_uncached, 15);
    assert_eq!(added.output, 2);
    assert_eq!(added.cache_read, 5);
    assert_eq!(added.reasoning, 1);
    assert_eq!(added.cache_write, 0);
    let reasons = store.warning_reasons().unwrap();
    // The invalid entry is dropped, so the source is truthfully partial.
    assert!(
        reasons.iter().any(|r| r == "missing-inputTokens"),
        "{reasons:?}"
    );
    assert_eq!(note_coverage(&notes), "partial");
    assert!(!store.load_source_coverage().unwrap()[0].complete);
}

#[test]
fn summary_symlink_and_oversize_are_not_read() {
    {
        let scratch = Scratch::new();
        let db = database(&scratch);
        let updates = copy_fixture(&scratch.0);
        let summary = updates.parent().unwrap().join("summary.json");
        let linked = scratch.0.join("linked-summary.json");
        fs::write(
        &linked,
        r#"{"chat_format_version":1,"reasoning_effort":"high","note":"GROK_SUMMARY_LINK_CANARY"}"#,
    )
    .unwrap();
        fs::remove_file(&summary).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_file(&linked, &summary).expect("symlink fixture unavailable");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&linked, &summary).expect("symlink fixture unavailable");
        ingest(&db, &scratch);
        let store = store(&db);
        let models = store.load_usage_models().unwrap();
        assert!(models.values().all(|(_, effort)| effort.is_none()));
        assert!(models
            .values()
            .all(|(model, _)| model.as_deref() == Some(MODEL)));
        assert!(store.warning_reasons().unwrap().is_empty());
        assert!(!store.contains_text("GROK_SUMMARY_LINK_CANARY").unwrap());
        assert!(!store.contains_text("high").unwrap());
    }

    {
        let scratch = Scratch::new();
        let db = database(&scratch);
        let updates = copy_fixture(&scratch.0);
        let summary = updates.parent().unwrap().join("summary.json");
        let mut body =
            String::from(r#"{"chat_format_version":1,"reasoning_effort":"high","pad":""#);
        body.push_str(&"x".repeat(70_000));
        body.push_str(r#"","note":"GROK_SUMMARY_SIZE_CANARY"}"#);
        fs::write(&summary, &body).unwrap();
        assert!(fs::metadata(&summary).unwrap().len() > 64 * 1024);
        ingest(&db, &scratch);
        let store = store(&db);
        let models = store.load_usage_models().unwrap();
        assert!(models.values().all(|(_, effort)| effort.is_none()));
        assert!(store.warning_reasons().unwrap().is_empty());
        assert!(!store.contains_text("GROK_SUMMARY_SIZE_CANARY").unwrap());
        assert_eq!(store.load_usage().unwrap().len(), 2);
    }
}
