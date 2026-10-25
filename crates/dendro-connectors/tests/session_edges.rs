//! Explicit sub-agent lineage only. Synthetic fixtures; no home-directory sessions.
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use dendro_connectors::stream::ingest_jsonl_with_key;
use dendro_core::AnalyticsStore;

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "dendro-edges-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn file(&self, name: &str, body: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, body).unwrap();
        path
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn ingest(
    store: &AnalyticsStore,
    harness: &str,
    path: &Path,
) -> dendro_connectors::stream::FileStats {
    ingest_jsonl_with_key(
        store,
        &format!("{harness}-src"),
        harness,
        path,
        Some(b"edge-key"),
    )
    .unwrap()
}

#[test]
fn claude_sidechain_fixture_emits_spawn_or_counts_the_gap() {
    let scratch = Scratch::new();
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/claude/sidechain.jsonl");
    let store = AnalyticsStore::open_in_memory("key").unwrap();
    let stats = ingest(&store, "claude", &fixture);
    assert_eq!(stats.coverage.edges, 1);
    assert_eq!(stats.coverage.lineage_not_explicit, 1);
    assert!(stats.coverage.summary().contains("edges=1"));
    assert!(stats.coverage.summary().contains("lineage_not_explicit=1"));
    let edges = store.load_session_edges().unwrap();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].relation, "spawn");
    assert_eq!(edges[0].status, "started");
    assert_eq!(edges[0].evidence_kind, "claude.sidechain");
    assert_eq!(edges[0].parent_session, "parent-session");
    assert_eq!(edges[0].child_session, "child-agent");
    assert!(!store.contains_text("CANARY_SIDECHAIN_TEXT").unwrap());
    // Sidechain usage stays on sessionId (the parent). Do not re-key to agentId.
    let parent = scratch.file(
        "parent.jsonl",
        concat!(
            r#"{"type":"assistant","sessionId":"parent-session","timestamp":"2026-10-25T00:00:00Z","message":{"id":"m-parent","role":"assistant","content":[{"type":"text","text":"ok"}],"usage":{"input_tokens":8,"output_tokens":4,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}"#,
            "\n",
        ),
    );
    ingest(&store, "claude", &parent);
    let usage = store.load_usage().unwrap();
    assert!(usage
        .iter()
        .all(|row| row.native_session == "parent-session"));
    let parent_total: i64 = usage
        .iter()
        .map(|row| row.input_uncached + row.output + row.cache_read + row.cache_write)
        .sum();
    assert_eq!(parent_total, 19);
    let roots = store.lineage_roots("claude", "child-agent").unwrap();
    assert_eq!(
        roots.roots,
        std::collections::BTreeSet::from(["parent-session".into()])
    );
    assert_eq!(
        store.children_of("claude", "parent-session").unwrap().len(),
        1
    );
    let _ = scratch;
}

#[test]
fn codex_thread_spawn_is_an_edge_and_pi_has_none() {
    let scratch = Scratch::new();
    let codex = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/codex/subagent.jsonl");
    let pi = scratch.file(
        "pi.jsonl",
        concat!(
            r#"{"type":"session","id":"child-pi","parentSessionId":"parent-pi"}"#,
            "\n",
            r#"{"type":"session","id":"plain-pi"}"#,
            "\n",
        ),
    );
    let store = AnalyticsStore::open_in_memory("key").unwrap();
    let codex_stats = ingest(&store, "codex", &codex);
    let pi_stats = ingest(&store, "pi", &pi);
    assert_eq!(codex_stats.coverage.edges, 1);
    assert_eq!(codex_stats.coverage.lineage_not_explicit, 2);
    assert_eq!(pi_stats.coverage.edges, 0);
    assert_eq!(pi_stats.coverage.lineage_not_explicit, 0);
    let edges = store.load_session_edges().unwrap();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].harness, "codex");
    assert_eq!(edges[0].parent_session, "parent-thread");
    assert_eq!(edges[0].child_session, "child-thread");
    assert_eq!(edges[0].evidence_kind, "codex.thread_spawn");
    assert_eq!(edges[0].relation, "spawn");
}

#[test]
fn child_usage_is_counted_once_when_parent_records_embed_a_token_statistic() {
    // Parent turn usage is the parent turn only. `tokens_used` and a Claude
    // Task result are not added again on top of the child session's own usage.
    let scratch = Scratch::new();
    let parent = scratch.file(
        "parent.jsonl",
        concat!(
            r#"{"method":"_x.ai/session/update","timestamp":1770000100,"params":{"sessionId":"parent-sess","update":{"sessionUpdate":"turn_completed","usage":{"inputTokens":100,"outputTokens":10,"cachedReadTokens":0,"cacheCreationTokens":0,"reasoningTokens":0,"totalTokens":110,"modelUsage":{"grok-4":{"inputTokens":100,"outputTokens":10,"cachedReadTokens":0,"cacheCreationTokens":0,"reasoningTokens":0,"totalTokens":110}}}}}}"#,
            "\n",
            r#"{"method":"_x.ai/session/update","timestamp":1770000101,"params":{"sessionId":"parent-sess","update":{"sessionUpdate":"subagent_spawned","subagent_id":"sub-1","attempt_id":"att-1","child_session_id":"child-sess","parent_session_id":"parent-sess"}}}"#,
            "\n",
            r#"{"method":"_x.ai/session/update","timestamp":1770000102,"params":{"sessionId":"parent-sess","update":{"sessionUpdate":"subagent_finished","subagent_id":"sub-1","attempt_id":"att-1","child_session_id":"child-sess","status":"completed","tokens_used":999,"output":"CANARY_CHILD_OUTPUT"}}}"#,
            "\n",
        ),
    );
    let child = scratch.file(
        "child.jsonl",
        concat!(
            r#"{"method":"_x.ai/session/update","timestamp":1770000103,"params":{"sessionId":"child-sess","update":{"sessionUpdate":"turn_completed","usage":{"inputTokens":40,"outputTokens":7,"cachedReadTokens":0,"cacheCreationTokens":0,"reasoningTokens":0,"totalTokens":47,"modelUsage":{"grok-4":{"inputTokens":40,"outputTokens":7,"cachedReadTokens":0,"cacheCreationTokens":0,"reasoningTokens":0,"totalTokens":47}}}}}}"#,
            "\n",
        ),
    );
    let claude_parent = scratch.file(
        "claude-parent.jsonl",
        concat!(
            r#"{"type":"assistant","sessionId":"claude-parent","timestamp":"2026-10-25T00:00:01Z","message":{"id":"m-parent","role":"assistant","content":[{"type":"tool_use","id":"t1","name":"Task","input":{"prompt":"CANARY_TASK","tokens":99999}}],"usage":{"input_tokens":8,"output_tokens":4,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}"#,
            "\n",
            r#"{"type":"user","sessionId":"claude-parent","timestamp":"2026-10-25T00:00:02Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"CANARY_TASK_RESULT tokens 99999"}]}}"#,
            "\n",
        ),
    );
    let store = AnalyticsStore::open_in_memory("key").unwrap();
    let parent_stats = ingest(&store, "grok", &parent);
    ingest(&store, "grok", &child);
    ingest(&store, "claude", &claude_parent);
    assert_eq!(parent_stats.coverage.edges, 2);
    let edges = store.load_session_edges().unwrap();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].status, "finished");
    assert_eq!(edges[0].parent_session, "parent-sess");
    assert_eq!(edges[0].child_session, "child-sess");
    assert_eq!(edges[0].relation, "spawn");
    assert!(!store.contains_text("CANARY_CHILD_OUTPUT").unwrap());
    assert!(!store.contains_text("CANARY_TASK").unwrap());
    let usage = store.load_usage().unwrap();
    let fleet: i64 = usage
        .iter()
        .map(|row| row.input_uncached + row.output + row.cache_read + row.cache_write)
        .sum();
    let mut by_session = std::collections::BTreeMap::<String, i64>::new();
    for row in &usage {
        *by_session.entry(row.native_session.clone()).or_default() +=
            row.input_uncached + row.output + row.cache_read + row.cache_write;
    }
    let session_sum: i64 = by_session.values().sum();
    assert_eq!(fleet, session_sum);
    // Grok parent 100+10, grok child 40+7, claude parent 8+4. Not +999 or +99999.
    assert_eq!(fleet, 169);
    assert_eq!(by_session.get("parent-sess").copied(), Some(110));
    assert_eq!(by_session.get("child-sess").copied(), Some(47));
    assert_eq!(by_session.get("claude-parent").copied(), Some(12));
    assert!(usage
        .iter()
        .all(|row| row.output != 999 && row.output != 99999));
}

#[test]
fn grok_fixture_links_spawn_to_finished_child() {
    let scratch = Scratch::new();
    let updates =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/grok/session/updates.jsonl");
    let store = AnalyticsStore::open_in_memory("key").unwrap();
    let stats = ingest(&store, "grok", &updates);
    assert!(stats.coverage.edges >= 1);
    assert!(stats.coverage.summary().contains("edges="));
    let edges = store.load_session_edges().unwrap();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].status, "finished");
    assert_eq!(edges[0].relation, "spawn");
    assert_eq!(
        edges[0].parent_session,
        "0199d4c1-8a2e-7f3b-9c41-5e6d7a8b9c0d"
    );
    assert_eq!(
        edges[0].child_session,
        "0199d4cb-4e6f-7a71-9b8c-9e0f1a2b3c4d"
    );
    assert!(!edges[0].evidence_kind.contains("Demo"));
    let usage = store.load_usage().unwrap();
    let tokens: i64 = usage
        .iter()
        .map(|row| row.input_uncached + row.output + row.cache_read + row.cache_write)
        .sum();
    assert_eq!(tokens, 5626);
    assert!(usage.iter().all(|row| row.output != 640));
    let _ = scratch;
}
