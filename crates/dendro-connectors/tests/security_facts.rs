//! Security observations from synthetic records. No home-directory sessions.
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use dendro_connectors::stream::ingest_jsonl_with_key;
use dendro_core::{AnalyticsStore, SecurityFact};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "dendro-secfacts-{}-{}",
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

fn ingest(store: &AnalyticsStore, harness: &str, path: &Path) {
    ingest_jsonl_with_key(
        store,
        &format!("{harness}-src"),
        harness,
        path,
        Some(b"security-facts-key"),
    )
    .unwrap();
}

fn kinds(facts: &[SecurityFact], kind: &str) -> Vec<String> {
    facts
        .iter()
        .filter(|fact| fact.kind == kind)
        .map(|fact| fact.value.clone())
        .collect()
}

fn blob(facts: &[SecurityFact]) -> String {
    facts
        .iter()
        .map(|fact| {
            format!(
                "{} {} {} {} {} {}",
                fact.source_key,
                fact.source_id,
                fact.native_session.as_deref().unwrap_or(""),
                fact.kind,
                fact.value,
                fact.source_field
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn claude_mode_changes_dedupe_and_commands_stay_categories() {
    let scratch = Scratch::new();
    let path = scratch.file(
        "claude.jsonl",
        concat!(
            r#"{"type":"permission-mode","sessionId":"sess-sk-SHOULDNOTAPPEAR","timestamp":"2026-10-25T00:00:00Z","permissionMode":"default"}"#,
            "\n",
            r#"{"type":"permission-mode","sessionId":"sess-sk-SHOULDNOTAPPEAR","timestamp":"2026-10-25T00:00:01Z","permissionMode":"default"}"#,
            "\n",
            r#"{"type":"assistant","sessionId":"sess-sk-SHOULDNOTAPPEAR","timestamp":"2026-10-25T00:00:02Z","permissionMode":"plan","permissionDecision":{"decision":"allow","source":"user","reasonType":"manual"},"sandboxed":true,"message":{"id":"m1","role":"assistant","content":[{"type":"text","text":"ok"}]}}"#,
            "\n",
            r#"{"type":"permission-mode","sessionId":"sess-sk-SHOULDNOTAPPEAR","timestamp":"2026-10-25T00:00:03Z","permissionMode":"plan"}"#,
            "\n",
            r#"{"type":"permission-mode","sessionId":"sess-sk-SHOULDNOTAPPEAR","timestamp":"2026-10-25T00:00:04Z","permissionMode":"yolo-mode","permissionDecision":{"decision":"sk-SECRETDECISION","source":"hook","reasonType":"because"}}"#,
            "\n",
            r#"{"type":"assistant","sessionId":"sess-sk-SHOULDNOTAPPEAR","timestamp":"2026-10-25T00:00:05Z","message":{"id":"m2","role":"assistant","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"echo \"rm -rf ./target\""}},{"type":"tool_use","id":"t2","name":"Bash","input":{"command":"rm -rf ./target","dangerouslyDisableSandbox":true}}]}}"#,
            "\n",
        ),
    );
    let store = AnalyticsStore::open_in_memory("key").unwrap();
    ingest(&store, "claude", &path);
    let facts = store.load_security_facts().unwrap();
    assert_eq!(
        kinds(&facts, "permission_mode"),
        vec!["default", "plan", "other"]
    );
    assert_eq!(kinds(&facts, "permission_decision"), vec!["allow", "other"]);
    assert_eq!(kinds(&facts, "sandbox_policy"), vec!["true"]);
    assert_eq!(kinds(&facts, "dangerous_command"), vec!["recursive_delete"]);
    assert_eq!(kinds(&facts, "sandbox_override_requested"), vec!["true"]);
    assert!(facts.iter().all(|fact| fact.harness == "claude"));
    assert!(facts.iter().all(|fact| fact
        .source_field
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '.')));
    let text = blob(&facts);
    assert!(!text.contains("SHOULDNOTAPPEAR"));
    assert!(!text.contains("sk-SECRETDECISION"));
    assert!(!text.contains("rm -rf"));
    assert!(!text.contains("yolo-mode"));
    assert!(!text.contains("because"));
}

#[test]
fn codex_policies_count_prefixes_and_classify_shell_calls() {
    let scratch = Scratch::new();
    let path = scratch.file(
        "codex.jsonl",
        concat!(
            r#"{"type":"session_meta","timestamp":"2026-10-25T00:00:00Z","payload":{"id":"codex-sess"}}"#,
            "\n",
            r#"{"type":"turn_context","timestamp":"2026-10-25T00:00:01Z","payload":{"approval_policy":"never","sandbox_policy":{"type":"read-only"},"permission_profile":{"type":"custom-profile-name"},"state":{"permissions":{"approved_command_prefixes":["git status","ghp_SHOULDNOTSTORE","rm -rf /"]}}}}"#,
            "\n",
            r#"{"type":"turn_context","timestamp":"2026-10-25T00:00:02Z","payload":{"approval_policy":"never","sandbox_policy":{"type":"read-only"}}}"#,
            "\n",
            r#"{"type":"turn_context","timestamp":"2026-10-25T00:00:03Z","payload":{"approval_policy":"on-request","sandbox_policy":{"type":"danger-full-access"}}}"#,
            "\n",
            r#"{"type":"event_msg","timestamp":"2026-10-25T00:00:04Z","payload":{"type":"item_completed","item":{"type":"CommandExecution","command":"echo \"rm -rf\"","exit_code":0}}}"#,
            "\n",
            r#"{"type":"event_msg","timestamp":"2026-10-25T00:00:05Z","payload":{"type":"item_completed","item":{"type":"CommandExecution","command":["rm","-rf","./target"],"exit_code":0}}}"#,
            "\n",
            r#"{"type":"response_item","timestamp":"2026-10-25T00:00:06Z","payload":{"type":"function_call","name":"exec_command","arguments":"{\"cmd\":\"git push --force\",\"dangerouslyDisableSandbox\":true}"}}"#,
            "\n",
        ),
    );
    let store = AnalyticsStore::open_in_memory("key").unwrap();
    ingest(&store, "codex", &path);
    let facts = store.load_security_facts().unwrap();
    assert_eq!(
        kinds(&facts, "approval_policy"),
        vec!["never", "on-request"]
    );
    assert_eq!(
        kinds(&facts, "sandbox_policy"),
        vec!["read-only", "danger-full-access"]
    );
    assert_eq!(kinds(&facts, "permission_profile"), vec!["other"]);
    assert_eq!(kinds(&facts, "approved_prefix_count"), vec!["3"]);
    let mut commands = kinds(&facts, "dangerous_command");
    commands.sort();
    assert_eq!(commands, vec!["force_push", "recursive_delete"]);
    assert_eq!(kinds(&facts, "sandbox_override_requested"), vec!["true"]);
    let profile = facts
        .iter()
        .find(|fact| fact.kind == "permission_profile")
        .unwrap();
    assert_eq!(profile.source_field, "payload.permission_profile.type");
    assert_eq!(profile.value, "other");
    let text = blob(&facts);
    assert!(!text.contains("ghp_SHOULDNOTSTORE"));
    assert!(!text.contains("custom-profile-name"));
    assert!(!text.contains("git status"));
    assert!(!text.contains("rm -rf"));
    assert!(!text.contains("git push"));
}

#[test]
fn pi_classifies_parsed_bash_and_grok_emits_nothing() {
    let scratch = Scratch::new();
    let pi = scratch.file(
        "pi.jsonl",
        concat!(
            r#"{"type":"session","version":3,"id":"pi-sess","timestamp":"2026-10-25T00:00:00Z"}"#,
            "\n",
            r#"{"type":"message","timestamp":"2026-10-25T00:00:01Z","permissionMode":"bypassPermissions","message":{"role":"assistant","content":[{"type":"toolCall","id":"p1","name":"bash","arguments":{"command":"echo \"rm -rf ./target\""}}]}}"#,
            "\n",
            r#"{"type":"message","timestamp":"2026-10-25T00:00:02Z","message":{"role":"assistant","content":[{"type":"toolCall","id":"p2","name":"bash","arguments":{"command":"sudo rm -rf ./target","dangerouslyDisableSandbox":true}}]}}"#,
            "\n",
        ),
    );
    let grok = scratch.file(
        "grok.jsonl",
        concat!(
            r#"{"method":"session/update","params":{"sessionId":"g1","permissionMode":"bypassPermissions","update":{"sessionUpdate":"tool_call","toolCallId":"t1","rawInput":{"command":"rm -rf ./target","dangerouslyDisableSandbox":true},"_meta":{"x.ai/tool":{"name":"bash"}}}}}"#,
            "\n",
        ),
    );
    let store = AnalyticsStore::open_in_memory("key").unwrap();
    ingest(&store, "pi", &pi);
    let facts = store.load_security_facts().unwrap();
    assert!(facts.iter().all(|fact| fact.harness == "pi"));
    assert!(facts.iter().all(|fact| fact.kind != "permission_mode"));
    assert_eq!(
        kinds(&facts, "dangerous_command"),
        vec!["recursive_delete", "sudo"]
    );
    assert_eq!(kinds(&facts, "sandbox_override_requested"), vec!["true"]);
    assert!(!blob(&facts).contains("rm -rf"));
    ingest(&store, "grok", &grok);
    let facts = store.load_security_facts().unwrap();
    assert!(facts.iter().all(|fact| fact.harness != "grok"));
}

#[test]
fn resumed_mode_change_is_not_dropped_and_a_repeat_stays_one_fact() {
    use std::io::Write;
    let scratch = Scratch::new();
    let path = scratch.file(
        "claude.jsonl",
        concat!(
            r#"{"type":"permission-mode","sessionId":"s1","timestamp":"2026-10-25T00:00:00Z","permissionMode":"default"}"#,
            "\n",
            r#"{"type":"permission-mode","sessionId":"s1","timestamp":"2026-10-25T00:00:01Z","permissionMode":"plan"}"#,
            "\n",
        ),
    );
    let store = AnalyticsStore::open_in_memory("key").unwrap();
    ingest(&store, "claude", &path);
    assert_eq!(
        kinds(&store.load_security_facts().unwrap(), "permission_mode"),
        vec!["default", "plan"]
    );
    let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
    writeln!(
        file,
        r#"{{"type":"permission-mode","sessionId":"s1","timestamp":"2026-10-25T00:00:02Z","permissionMode":"acceptEdits"}}"#
    )
    .unwrap();
    file.sync_all().unwrap();
    drop(file);
    ingest(&store, "claude", &path);
    assert_eq!(
        kinds(&store.load_security_facts().unwrap(), "permission_mode"),
        vec!["default", "plan", "acceptEdits"]
    );
    let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
    writeln!(
        file,
        r#"{{"type":"permission-mode","sessionId":"s1","timestamp":"2026-10-25T00:00:03Z","permissionMode":"acceptEdits"}}"#
    )
    .unwrap();
    file.sync_all().unwrap();
    drop(file);
    ingest(&store, "claude", &path);
    let facts = store.load_security_facts().unwrap();
    assert_eq!(
        kinds(&facts, "permission_mode"),
        vec!["default", "plan", "acceptEdits"]
    );
    assert_eq!(facts.len(), 3);
}
