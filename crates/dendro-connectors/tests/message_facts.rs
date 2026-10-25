//! Native readers project safe message metadata (role, direction, modality,
//! source order) and never message text. Synthetic fixtures only.
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use dendro_connectors::stream::{ingest_jsonl_with_key, FileStats};
use dendro_core::domain_v1::{Direction, Grain, MessageRole, Modality};
use dendro_core::{AnalyticsStore, MessageFact};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "dendro-msg-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn file(&self, name: &str, lines: &[String]) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, lines.join("\n") + "\n").unwrap();
        path
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

const CANARIES: &[&str] = &[
    "CANARY_USER_TEXT",
    "CANARY_THOUGHT",
    "CANARY_REPLY",
    "CANARY_TOOL_INPUT",
    "CANARY_RESULT",
    "CANARY_IMAGE",
    "CANARY_DEVELOPER",
    "C:/CANARY_PATH",
];

fn ingest(store: &AnalyticsStore, harness: &str, path: &Path) -> FileStats {
    ingest_jsonl_with_key(store, &format!("{harness}-src"), harness, path, None).unwrap()
}

fn assert_no_text(store: &AnalyticsStore) {
    for canary in CANARIES {
        assert!(
            !store.contains_text(canary).unwrap(),
            "stored text leaked {canary}"
        );
    }
}

fn shape(messages: &[MessageFact]) -> Vec<(i64, MessageRole, Direction, Vec<Modality>, Grain)> {
    messages
        .iter()
        .map(|m| {
            (
                m.seq,
                m.role,
                m.direction,
                m.modalities.iter().copied().collect(),
                m.grain,
            )
        })
        .collect()
}

fn usage_claude(id: &str, content: &str) -> String {
    format!(
        r#"{{"type":"assistant","sessionId":"c1","timestamp":"2026-10-09T00:00:02Z","message":{{"id":"{id}","role":"assistant","content":{content},"usage":{{"input_tokens":3,"output_tokens":4,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}}}}"#
    )
}

#[test]
fn claude_messages_carry_roles_directions_modalities_and_order() {
    use Direction::*;
    use MessageRole::*;
    let scratch = Scratch::new();
    let lines = [
        r#"{"type":"user","sessionId":"c1","timestamp":"2026-10-09T00:00:01Z","message":{"role":"user","content":"CANARY_USER_TEXT"}}"#.to_string(),
        usage_claude("m1", r#"[{"type":"thinking","thinking":"CANARY_THOUGHT"}]"#),
        usage_claude(
            "m2",
            r#"[{"type":"text","text":"CANARY_REPLY"},{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"C:/CANARY_PATH","note":"CANARY_TOOL_INPUT"}}]"#,
        ),
        r#"{"type":"user","sessionId":"c1","timestamp":"2026-10-09T00:00:03Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"CANARY_RESULT"}]}}"#.to_string(),
        r#"{"type":"user","timestamp":"2026-10-09T00:00:04Z","message":{"role":"user","content":[{"type":"image","source":{"data":"CANARY_IMAGE"}}]}}"#.to_string(),
        r#"{"type":"queue-operation","operation":"enqueue"}"#.to_string(),
    ];
    let path = scratch.file("c.jsonl", &lines);
    let store = AnalyticsStore::open_in_memory("key").unwrap();
    let stats = ingest(&store, "claude", &path);
    let messages = store.load_messages().unwrap();
    assert_eq!(
        shape(&messages),
        [
            (1, User, Inbound, vec![Modality::Text], Grain::Message),
            (2, Assistant, Internal, vec![Modality::Text], Grain::Message),
            (
                3,
                Assistant,
                Outbound,
                vec![Modality::Text, Modality::Structured],
                Grain::Message
            ),
            (4, Tool, Inbound, vec![Modality::Structured], Grain::Message),
            (5, User, Inbound, vec![Modality::Image], Grain::Message),
        ]
    );
    // The last line names no session: none is borrowed from its neighbours.
    let sessions: Vec<_> = messages
        .iter()
        .map(|m| m.native_session.as_deref())
        .collect();
    assert_eq!(
        sessions,
        [Some("c1"), Some("c1"), Some("c1"), Some("c1"), None]
    );
    assert!(messages
        .iter()
        .all(|m| m.harness == "claude" && m.generation == 1));
    // Source order is the record order; a record's own time is kept.
    assert!(messages.windows(2).all(|pair| pair[0].seq < pair[1].seq));
    assert_eq!(
        messages[0].ts.unwrap().to_rfc3339(),
        "2026-10-09T00:00:01+00:00"
    );
    // Records now projected as messages are projected, only the queue record is not.
    assert_eq!(stats.coverage.projected, 5);
    assert_eq!(stats.coverage.not_projected, 1);
    assert_no_text(&store);
}

#[test]
fn pi_messages_carry_roles_directions_modalities_and_order() {
    use Direction::*;
    use MessageRole::*;
    let scratch = Scratch::new();
    let lines = [
        r#"{"type":"session","id":"p1","timestamp":"2026-10-09T00:00:00Z"}"#.to_string(),
        r#"{"type":"message","id":"a","timestamp":"2026-10-09T00:00:01Z","message":{"role":"user","content":[{"type":"text","text":"CANARY_USER_TEXT"}]}}"#.to_string(),
        r#"{"type":"message","id":"b","timestamp":"2026-10-09T00:00:02Z","message":{"role":"assistant","content":[{"type":"thinking","thinking":"CANARY_THOUGHT"},{"type":"toolCall","id":"k1","name":"read","arguments":{"path":"C:/CANARY_PATH"}}],"usage":{"input":1,"output":2,"cacheRead":0,"cacheWrite":0}}}"#.to_string(),
        r#"{"type":"message","id":"c","timestamp":"2026-10-09T00:00:03Z","message":{"role":"toolResult","toolCallId":"k1","content":[{"type":"text","text":"CANARY_RESULT"}]}}"#.to_string(),
        r#"{"type":"message","id":"d","timestamp":"2026-10-09T00:00:04Z","message":{"role":"bashExecution","content":"CANARY_REPLY"}}"#.to_string(),
    ];
    let path = scratch.file("p.jsonl", &lines);
    let store = AnalyticsStore::open_in_memory("key").unwrap();
    let stats = ingest(&store, "pi", &path);
    let messages = store.load_messages().unwrap();
    assert_eq!(
        shape(&messages),
        [
            (2, User, Inbound, vec![Modality::Text], Grain::Message),
            (
                3,
                Assistant,
                Outbound,
                vec![Modality::Text, Modality::Structured],
                Grain::Message
            ),
            (4, Tool, Inbound, vec![Modality::Text], Grain::Message),
            (
                5,
                MessageRole::Unknown,
                Direction::Unknown,
                vec![Modality::Text],
                Grain::Message
            ),
        ]
    );
    assert!(messages
        .iter()
        .all(|m| m.native_session.as_deref() == Some("p1")));
    assert_eq!(stats.coverage.projected, 4);
    assert_no_text(&store);
}

#[test]
fn codex_messages_read_response_items_once() {
    use Direction::*;
    use MessageRole::*;
    let scratch = Scratch::new();
    let item = |payload: &str, second: u32| {
        format!(
            r#"{{"type":"response_item","timestamp":"2026-10-09T00:00:{second:02}Z","payload":{payload}}}"#
        )
    };
    let lines = [
        r#"{"type":"session_meta","payload":{"id":"x1"}}"#.to_string(),
        item(r#"{"type":"message","role":"developer","content":[{"type":"input_text","text":"CANARY_DEVELOPER"}]}"#, 1),
        item(r#"{"type":"message","role":"user","content":[{"type":"input_text","text":"CANARY_USER_TEXT"},{"type":"input_image","image_url":"CANARY_IMAGE"}]}"#, 2),
        // The event copy of the same user message is not read a second time.
        r#"{"type":"event_msg","timestamp":"2026-10-09T00:00:02Z","payload":{"type":"user_message","message":"CANARY_USER_TEXT"}}"#.to_string(),
        item(r#"{"type":"reasoning","summary":[{"type":"summary_text","text":"CANARY_THOUGHT"}]}"#, 3),
        item(r#"{"type":"function_call","name":"shell","arguments":"CANARY_TOOL_INPUT"}"#, 4),
        item(r#"{"type":"message","role":"assistant","content":[{"type":"output_text","text":"CANARY_REPLY"}]}"#, 5),
    ];
    let path = scratch.file("x.jsonl", &lines);
    let store = AnalyticsStore::open_in_memory("key").unwrap();
    let stats = ingest(&store, "codex", &path);
    let messages = store.load_messages().unwrap();
    assert_eq!(
        shape(&messages),
        [
            (2, Developer, Internal, vec![Modality::Text], Grain::Message),
            (
                3,
                User,
                Inbound,
                vec![Modality::Text, Modality::Image],
                Grain::Message
            ),
            (5, Assistant, Internal, vec![Modality::Text], Grain::Message),
            (7, Assistant, Outbound, vec![Modality::Text], Grain::Message),
        ]
    );
    assert!(messages
        .iter()
        .all(|m| m.native_session.as_deref() == Some("x1")));
    assert!(stats.coverage.projected >= 4);
    assert_no_text(&store);
}

fn grok_fixture(scratch: &Scratch) -> PathBuf {
    let dir = scratch.0.join("session");
    fs::create_dir_all(&dir).unwrap();
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/grok/session");
    fs::copy(src.join("updates.jsonl"), dir.join("updates.jsonl")).unwrap();
    fs::copy(src.join("summary.json"), dir.join("summary.json")).unwrap();
    dir.join("updates.jsonl")
}

#[test]
fn grok_chunks_are_ordered_fragments_and_reingest_adds_only_new_ones() {
    use Direction::*;
    use MessageRole::*;
    let scratch = Scratch::new();
    let updates = grok_fixture(&scratch);
    let store = AnalyticsStore::open_in_memory("key").unwrap();
    let first = ingest(&store, "grok", &updates);
    let messages = store.load_messages().unwrap();
    let text = || vec![Modality::Text];
    assert_eq!(
        shape(&messages),
        [
            (1, User, Inbound, text(), Grain::Fragment),
            (3, Assistant, Internal, text(), Grain::Fragment),
            (9, Assistant, Outbound, text(), Grain::Fragment),
            (11, User, Inbound, text(), Grain::Fragment),
            (20, Assistant, Outbound, text(), Grain::Fragment),
        ]
    );
    assert!(messages
        .iter()
        .all(|m| m.native_session.as_deref() == Some("0199d4c1-8a2e-7f3b-9c41-5e6d7a8b9c0d")));
    assert_eq!(messages[0].ts.unwrap().timestamp(), 1_770_000_000);
    let keys: Vec<_> = messages.iter().map(|m| m.source_key.clone()).collect();
    assert_no_text(&store);
    for canary in [
        "list files",
        "Demo thought: list the directory.",
        "Demo closing line.",
    ] {
        assert!(!store.contains_text(canary).unwrap(), "leaked {canary}");
    }

    // Unchanged file: nothing is added.
    ingest(&store, "grok", &updates);
    assert_eq!(store.load_messages().unwrap().len(), 5);

    // One appended chunk adds exactly one new fact; the old facts are untouched.
    let mut file = OpenOptions::new().append(true).open(&updates).unwrap();
    writeln!(
        file,
        r#"{{"method":"session/update","params":{{"sessionId":"0199d4c1-8a2e-7f3b-9c41-5e6d7a8b9c0d","update":{{"sessionUpdate":"user_message_chunk","content":{{"type":"image","data":"CANARY_IMAGE"}}}}}},"timestamp":1770000009}}"#
    )
    .unwrap();
    drop(file);
    let second = ingest(&store, "grok", &updates);
    let messages = store.load_messages().unwrap();
    assert_eq!(messages.len(), 6);
    assert_eq!(
        messages
            .iter()
            .take(5)
            .map(|m| m.source_key.clone())
            .collect::<Vec<_>>(),
        keys
    );
    let last = messages.last().unwrap();
    assert_eq!(
        (
            last.seq,
            last.role,
            last.modalities.iter().copied().collect::<Vec<_>>()
        ),
        (22, User, vec![Modality::Image])
    );
    assert_no_text(&store);

    // Coverage: message chunks are projected, not left in the unprojected count.
    assert!(first.coverage.projected >= 5 + 2);
    assert_eq!(second.coverage.projected, 1);
    assert_eq!(second.coverage.not_projected, 0);
}
