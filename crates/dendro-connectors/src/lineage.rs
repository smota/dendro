//! Explicit session parent/child evidence. Timing and cwd are not links.
//! A lineage-shaped record that does not name both sessions is a gap.

use std::collections::BTreeMap;

use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ObservedEdge {
    pub body: String,
    pub parent_raw: String,
    pub child_raw: String,
    pub relation: &'static str,
    pub status: &'static str,
    pub observed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub evidence_kind: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Hit {
    None,
    Gap,
    /// Finish names the child identity but not the parent. The caller may resume a stored spawn.
    Resume(ObservedEdge),
    Edge(ObservedEdge),
}

#[derive(Debug, Default)]
pub(crate) struct OpenAgents {
    grok: BTreeMap<String, GrokOpen>,
}

#[derive(Debug, Clone)]
struct GrokOpen {
    parent_raw: Option<String>,
    child_raw: String,
    status: &'static str,
    observed_at: Option<chrono::DateTime<chrono::Utc>>,
    evidence_kind: &'static str,
}

pub(crate) fn observe(
    harness: &str,
    value: &Value,
    file_session: Option<&str>,
    open: &mut OpenAgents,
) -> Hit {
    match harness {
        "grok" => observe_grok(value, open),
        "claude" => observe_claude(value),
        "codex" => observe_codex(value, file_session),
        "pi" => observe_pi(),
        _ => Hit::None,
    }
}

/// Grok `subagent_spawned` / `subagent_finished` (`FORMAT.md`).
/// Spawn requires `parent_session_id` and `child_session_id`.
/// Finish uses the same `subagent_id` + `attempt_id` and does not invent a parent.
/// `tokens_used` is not usage.
fn observe_grok(value: &Value, open: &mut OpenAgents) -> Hit {
    let kind = value
        .pointer("/params/update/sessionUpdate")
        .and_then(Value::as_str);
    if kind != Some("subagent_spawned") && kind != Some("subagent_finished") {
        return Hit::None;
    }
    let update = value.pointer("/params/update");
    let child = id_at(update, "child_session_id");
    let subagent = id_at(update, "subagent_id");
    let attempt = id_at(update, "attempt_id");
    let (Ok(child), Ok(subagent), Ok(attempt)) = (child, subagent, attempt) else {
        return Hit::Gap;
    };
    let body = format!("spawn:{subagent}:{attempt}");
    let observed_at = unix_secs(value.get("timestamp"));
    let entry = open.grok.entry(body.clone()).or_insert_with(|| GrokOpen {
        parent_raw: None,
        child_raw: child.to_string(),
        status: "started",
        observed_at,
        evidence_kind: "grok.subagent_spawned",
    });
    entry.child_raw = child.to_string();
    if kind == Some("subagent_spawned") {
        match id_at(update, "parent_session_id") {
            Ok(parent) => entry.parent_raw = Some(parent.to_string()),
            Err(Field::Absent) | Err(Field::Invalid) => return Hit::Gap,
        }
        if entry.status == "started" {
            entry.evidence_kind = "grok.subagent_spawned";
            entry.observed_at = observed_at.or(entry.observed_at);
        }
    } else {
        entry.status = grok_finish_status(
            update
                .and_then(|item| item.get("status"))
                .and_then(Value::as_str)
                .unwrap_or(""),
        );
        entry.evidence_kind = "grok.subagent_finished";
        entry.observed_at = observed_at.or(entry.observed_at);
        // Finish does not carry `parent_session_id` (FORMAT.md). Keep a spawn parent.
        if let Ok(parent) = id_at(update, "parent_session_id") {
            entry.parent_raw = Some(parent.to_string());
        }
    }
    let Some(parent_raw) = entry.parent_raw.clone() else {
        return Hit::Resume(ObservedEdge {
            body,
            parent_raw: String::new(),
            child_raw: entry.child_raw.clone(),
            relation: "spawn",
            status: entry.status,
            observed_at: entry.observed_at,
            evidence_kind: entry.evidence_kind,
        });
    };
    Hit::Edge(ObservedEdge {
        body,
        parent_raw,
        child_raw: entry.child_raw.clone(),
        relation: "spawn",
        status: entry.status,
        observed_at: entry.observed_at,
        evidence_kind: entry.evidence_kind,
    })
}

fn grok_finish_status(raw: &str) -> &'static str {
    match raw {
        "completed" | "complete" | "finished" | "success" | "succeeded" => "finished",
        "failed" | "error" | "errored" | "cancelled" | "canceled" => "failed",
        _ => "unknown",
    }
}

/// Claude Code sub-agent files are named `agent-<id>.jsonl`. Records carry
/// `isSidechain: true`, `sessionId` = the parent session, and `agentId` = the
/// child agent. `parentUuid` is a message chain, not a session link. There is
/// no `parentSessionId`. The filename is not read.
fn observe_claude(value: &Value) -> Hit {
    if value.get("isSidechain") != Some(&Value::Bool(true)) {
        return Hit::None;
    }
    let parent = match id_at(Some(value), "sessionId") {
        Ok(parent) => parent,
        Err(_) => return Hit::Gap,
    };
    let child = match id_at(Some(value), "agentId") {
        Ok(child) => child,
        Err(_) => return Hit::Gap,
    };
    if parent == child {
        return Hit::Gap;
    }
    Hit::Edge(ObservedEdge {
        body: format!("spawn:{child}"),
        parent_raw: parent.to_string(),
        child_raw: child.to_string(),
        relation: "spawn",
        status: "started",
        observed_at: rfc3339_at(value.get("timestamp")),
        evidence_kind: "claude.sidechain",
    })
}

/// Codex spawn evidence is `payload.source.subagent.thread_spawn.parent_thread_id`.
/// The child is `payload.thread_id`, else `payload.id`, else the file session id.
/// `payload.item.sender_thread_id`, `receiver_thread_ids`, `agent_thread_id`, and
/// `turn_attribution.parent_turn_id` are messaging or turn links: counted, not edges.
fn observe_codex(value: &Value, file_session: Option<&str>) -> Hit {
    let payload = value.get("payload");
    let spawn = payload.and_then(|item| item.pointer("/source/subagent/thread_spawn"));
    let parent = match id_at(spawn, "parent_thread_id") {
        Ok(parent) => parent.to_string(),
        Err(Field::Invalid) => return Hit::Gap,
        Err(Field::Absent) => {
            return if codex_message_link(value) {
                Hit::Gap
            } else {
                Hit::None
            };
        }
    };
    let child = match id_at(payload, "thread_id") {
        Ok(id) => id.to_string(),
        Err(Field::Invalid) => return Hit::Gap,
        Err(Field::Absent) => match id_at(payload, "id") {
            Ok(id) => id.to_string(),
            Err(Field::Invalid) => return Hit::Gap,
            Err(Field::Absent) => match file_session.filter(|id| explicit_id(id)) {
                Some(id) => id.to_string(),
                None => return Hit::Gap,
            },
        },
    };
    if child == parent {
        return Hit::Gap;
    }
    Hit::Edge(ObservedEdge {
        body: format!("spawn:{child}"),
        parent_raw: parent,
        child_raw: child,
        relation: "spawn",
        status: "started",
        observed_at: rfc3339_at(value.get("timestamp"))
            .or_else(|| rfc3339_at(payload.and_then(|item| item.get("timestamp")))),
        evidence_kind: "codex.thread_spawn",
    })
}

fn codex_message_link(value: &Value) -> bool {
    let payload = value.get("payload");
    let item = payload.and_then(|value| value.get("item"));
    item.and_then(|value| value.get("sender_thread_id"))
        .is_some()
        || item
            .and_then(|value| value.get("receiver_thread_ids"))
            .is_some()
        || item
            .and_then(|value| value.get("agent_thread_id"))
            .is_some()
        || value
            .pointer("/turn_attribution/parent_turn_id")
            .or_else(|| payload.and_then(|item| item.pointer("/turn_attribution/parent_turn_id")))
            .is_some()
}

/// Pi fixtures and format notes in this repo name no parent-session field.
/// A guessed `parentSessionId` is not evidence.
fn observe_pi() -> Hit {
    Hit::None
}

#[derive(Clone, Copy)]
enum Field {
    Absent,
    Invalid,
}

fn id_at<'a>(value: Option<&'a Value>, key: &str) -> Result<&'a str, Field> {
    match value.and_then(|item| item.get(key)) {
        None => Err(Field::Absent),
        Some(Value::String(text)) if explicit_id(text) => Ok(text),
        Some(_) => Err(Field::Invalid),
    }
}

fn explicit_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        && !dendro_core::contains_secret(value)
}

fn unix_secs(value: Option<&Value>) -> Option<chrono::DateTime<chrono::Utc>> {
    value
        .and_then(Value::as_i64)
        .and_then(|secs| chrono::DateTime::from_timestamp(secs, 0))
}

fn rfc3339_at(value: Option<&Value>) -> Option<chrono::DateTime<chrono::Utc>> {
    value.and_then(Value::as_str).and_then(|text| {
        chrono::DateTime::parse_from_rfc3339(text)
            .ok()
            .map(|parsed| parsed.with_timezone(&chrono::Utc))
    })
}

#[cfg(test)]
mod tests {
    use super::{observe, Hit, OpenAgents};
    use serde_json::json;

    #[test]
    fn claude_sidechain_without_agent_is_a_gap() {
        let mut open = OpenAgents::default();
        let value = json!({"type":"system","isSidechain":true,"sessionId":"parent-session"});
        assert_eq!(observe("claude", &value, None, &mut open), Hit::Gap);
    }

    #[test]
    fn claude_sidechain_uses_session_id_and_agent_id() {
        let mut open = OpenAgents::default();
        let value = json!({
            "type": "assistant",
            "isSidechain": true,
            "sessionId": "parent-session",
            "parentUuid": "11111111-2222-3333-4444-555555555555",
            "parentSessionId": "ignored-parent",
            "agentId": "child-agent"
        });
        match observe("claude", &value, None, &mut open) {
            Hit::Edge(edge) => {
                assert_eq!(edge.parent_raw, "parent-session");
                assert_eq!(edge.child_raw, "child-agent");
            }
            other => panic!("expected spawn, got {other:?}"),
        }
    }

    #[test]
    fn codex_thread_id_alone_emits_nothing() {
        let mut open = OpenAgents::default();
        let value = json!({
            "type": "session_meta",
            "payload": {"id": "child-codex", "thread_id": "thread-1"}
        });
        assert_eq!(
            observe("codex", &value, Some("child-codex"), &mut open),
            Hit::None
        );
    }

    #[test]
    fn codex_parent_thread_id_is_the_spawn_parent() {
        let mut open = OpenAgents::default();
        let value = json!({
            "type": "session_meta",
            "payload": {
                "id": "child-session",
                "thread_id": "child-thread",
                "source": {"subagent": {"thread_spawn": {"parent_thread_id": "parent-thread"}}}
            }
        });
        match observe("codex", &value, Some("child-session"), &mut open) {
            Hit::Edge(edge) => {
                assert_eq!(edge.parent_raw, "parent-thread");
                assert_eq!(edge.child_raw, "child-thread");
            }
            other => panic!("expected spawn, got {other:?}"),
        }
    }

    #[test]
    fn codex_message_links_are_not_spawn_edges() {
        let mut open = OpenAgents::default();
        let value = json!({
            "type": "event_msg",
            "payload": {"thread_id": "child-thread", "item": {"sender_thread_id": "other"}}
        });
        assert_eq!(
            observe("codex", &value, Some("child-thread"), &mut open),
            Hit::Gap
        );
        let guessed = json!({
            "type": "session_meta",
            "payload": {"id": "child-codex", "parent_session_id": "parent-codex"}
        });
        assert_eq!(
            observe("codex", &guessed, Some("child-codex"), &mut open),
            Hit::None
        );
    }

    #[test]
    fn pi_has_no_documented_parent_field() {
        let mut open = OpenAgents::default();
        let value = json!({"type":"session","id":"pi-session","parentSessionId":"parent-pi"});
        assert_eq!(
            observe("pi", &value, Some("pi-session"), &mut open),
            Hit::None
        );
    }
}
