//! Safe message metadata read from native records: who spoke, which way the
//! message went, what kinds of content it carried, and where it sits in the
//! source. Only closed enums and the record's own order are produced. Message
//! text, tool inputs, paths and content are never read into the result.
use std::collections::BTreeSet;

use dendro_core::domain_v1::{Direction, Grain, MessageRole, Modality};
use serde_json::Value;

/// What one native record says about its message. No text is carried.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MessageObs {
    pub role: MessageRole,
    pub direction: Direction,
    pub modalities: BTreeSet<Modality>,
    pub grain: Grain,
    /// Session id the record itself names, when its harness puts one on the line.
    pub session: Option<String>,
}

/// The message a record projects, or None when the record is not a message.
pub(crate) fn observe(harness: &str, value: &Value) -> Option<MessageObs> {
    match harness {
        "claude" => claude(value),
        "pi" => pi(value),
        "codex" => codex(value),
        "grok" => grok(value),
        _ => None,
    }
}

/// Role label to (role, direction). An unmappable label is unknown, never guessed.
fn role_of(label: Option<&str>) -> (MessageRole, Direction) {
    match label {
        Some("user") => (MessageRole::User, Direction::Inbound),
        Some("assistant") => (MessageRole::Assistant, Direction::Outbound),
        Some("system") => (MessageRole::System, Direction::Internal),
        Some("developer") => (MessageRole::Developer, Direction::Internal),
        Some("tool" | "toolResult" | "tool_result") => (MessageRole::Tool, Direction::Inbound),
        _ => (MessageRole::Unknown, Direction::Unknown),
    }
}

fn modality_of(kind: &str) -> Modality {
    match kind {
        "text" | "input_text" | "output_text" | "thinking" | "redacted_thinking" | "reasoning" => {
            Modality::Text
        }
        "image" | "input_image" | "output_image" => Modality::Image,
        "audio" | "input_audio" | "output_audio" => Modality::Audio,
        "video" => Modality::Video,
        "tool_use" | "server_tool_use" | "tool_result" | "toolCall" | "toolResult" | "resource"
        | "resource_link" => Modality::Structured,
        _ => Modality::Unknown,
    }
}

fn kinds(content: Option<&Value>) -> Vec<Option<&str>> {
    match content {
        Some(Value::String(_)) => vec![Some("text")],
        Some(Value::Array(blocks)) => blocks
            .iter()
            .map(|block| block.get("type").and_then(Value::as_str))
            .collect(),
        _ => Vec::new(),
    }
}

fn modalities(kinds: &[Option<&str>]) -> BTreeSet<Modality> {
    let set: BTreeSet<Modality> = kinds
        .iter()
        .map(|kind| kind.map_or(Modality::Unknown, modality_of))
        .collect();
    if set.is_empty() {
        BTreeSet::from([Modality::Unknown])
    } else {
        set
    }
}

fn all_of(kinds: &[Option<&str>], accepted: &[&str]) -> bool {
    !kinds.is_empty()
        && kinds
            .iter()
            .all(|kind| kind.is_some_and(|kind| accepted.contains(&kind)))
}

/// A turn made only of reasoning blocks never left the agent: internal.
fn conversation(
    label: Option<&str>,
    content: Option<&Value>,
) -> (MessageRole, Direction, BTreeSet<Modality>) {
    let kinds = kinds(content);
    let (mut role, mut direction) = role_of(label);
    if role == MessageRole::User && all_of(&kinds, &["tool_result"]) {
        role = MessageRole::Tool;
    }
    if role == MessageRole::Assistant && all_of(&kinds, &["thinking", "redacted_thinking"]) {
        direction = Direction::Internal;
    }
    (role, direction, modalities(&kinds))
}

fn claude(value: &Value) -> Option<MessageObs> {
    let session = value
        .get("sessionId")
        .and_then(Value::as_str)
        .map(str::to_string);
    let (role, direction, modalities) = match value.get("type").and_then(Value::as_str)? {
        kind @ ("user" | "assistant") => {
            let message = value.get("message")?;
            let label = message.get("role").and_then(Value::as_str).unwrap_or(kind);
            conversation(Some(label), message.get("content"))
        }
        // Only a system record that carries text is a message.
        "system" if value.get("content").is_some_and(Value::is_string) => {
            conversation(Some("system"), value.get("content"))
        }
        _ => return None,
    };
    Some(MessageObs {
        role,
        direction,
        modalities,
        grain: Grain::Message,
        session,
    })
}

fn pi(value: &Value) -> Option<MessageObs> {
    if value.get("type").and_then(Value::as_str) != Some("message") {
        return None;
    }
    let message = value.get("message")?;
    let (role, direction, modalities) = conversation(
        message.get("role").and_then(Value::as_str),
        message.get("content"),
    );
    Some(MessageObs {
        role,
        direction,
        modalities,
        grain: Grain::Message,
        session: None,
    })
}

/// Codex writes each conversation item as a `response_item`; the matching
/// `event_msg` copies are not read, so a message is never counted twice.
fn codex(value: &Value) -> Option<MessageObs> {
    if value.get("type").and_then(Value::as_str) != Some("response_item") {
        return None;
    }
    let payload = value.get("payload")?;
    let (role, direction, modalities) = match payload.get("type").and_then(Value::as_str)? {
        "message" => conversation(
            payload.get("role").and_then(Value::as_str),
            payload.get("content"),
        ),
        "reasoning" => (
            MessageRole::Assistant,
            Direction::Internal,
            BTreeSet::from([Modality::Text]),
        ),
        _ => return None,
    };
    Some(MessageObs {
        role,
        direction,
        modalities,
        grain: Grain::Message,
        session: None,
    })
}

/// Grok streams a message as chunks; each chunk is one fragment, in file order.
fn grok(value: &Value) -> Option<MessageObs> {
    let method = value.get("method").and_then(Value::as_str)?;
    if method != "session/update" && method != "_x.ai/session/update" {
        return None;
    }
    let update = value.pointer("/params/update")?;
    let (role, direction) = match update.get("sessionUpdate").and_then(Value::as_str)? {
        "user_message_chunk" => (MessageRole::User, Direction::Inbound),
        "agent_message_chunk" => (MessageRole::Assistant, Direction::Outbound),
        "agent_thought_chunk" => (MessageRole::Assistant, Direction::Internal),
        _ => return None,
    };
    let kind = update.pointer("/content/type").and_then(Value::as_str);
    Some(MessageObs {
        role,
        direction,
        modalities: modalities(&[kind]),
        grain: Grain::Fragment,
        session: value
            .pointer("/params/sessionId")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn set(items: &[Modality]) -> BTreeSet<Modality> {
        items.iter().copied().collect()
    }

    #[test]
    fn claude_roles_directions_and_modalities() {
        let user = observe(
            "claude",
            &json!({"type":"user","sessionId":"s","message":{"role":"user","content":"hello"}}),
        )
        .unwrap();
        assert_eq!(
            (user.role, user.direction, user.session.as_deref()),
            (MessageRole::User, Direction::Inbound, Some("s"))
        );
        assert_eq!(user.modalities, set(&[Modality::Text]));
        let result = observe(
            "claude",
            &json!({"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"x"}]}}),
        )
        .unwrap();
        assert_eq!(
            (result.role, result.direction),
            (MessageRole::Tool, Direction::Inbound)
        );
        assert_eq!(result.modalities, set(&[Modality::Structured]));
        assert_eq!(result.session, None);
        let mixed = observe(
            "claude",
            &json!({"type":"assistant","message":{"role":"assistant","content":[{"type":"text"},{"type":"tool_use"},{"type":"image"},{"type":"novel"}]}}),
        )
        .unwrap();
        assert_eq!(
            (mixed.role, mixed.direction),
            (MessageRole::Assistant, Direction::Outbound)
        );
        assert_eq!(
            mixed.modalities,
            set(&[
                Modality::Text,
                Modality::Structured,
                Modality::Image,
                Modality::Unknown
            ])
        );
        let thought = observe(
            "claude",
            &json!({"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking"}]}}),
        )
        .unwrap();
        assert_eq!(
            (thought.role, thought.direction),
            (MessageRole::Assistant, Direction::Internal)
        );
        let empty = observe(
            "claude",
            &json!({"type":"assistant","message":{"role":"assistant"}}),
        )
        .unwrap();
        assert_eq!(empty.modalities, set(&[Modality::Unknown]));
        let odd = observe(
            "claude",
            &json!({"type":"user","message":{"role":"narrator","content":"x"}}),
        )
        .unwrap();
        assert_eq!(
            (odd.role, odd.direction),
            (MessageRole::Unknown, Direction::Unknown)
        );
        assert!(observe(
            "claude",
            &json!({"type":"system","subtype":"turn_duration"})
        )
        .is_none());
        assert!(observe("claude", &json!({"type":"queue-operation"})).is_none());
    }

    #[test]
    fn pi_codex_and_grok_mappings() {
        let tool = observe(
            "pi",
            &json!({"type":"message","message":{"role":"toolResult","content":[{"type":"text"}]}}),
        )
        .unwrap();
        assert_eq!(
            (tool.role, tool.direction),
            (MessageRole::Tool, Direction::Inbound)
        );
        let pi_odd = observe(
            "pi",
            &json!({"type":"message","message":{"role":"bashExecution"}}),
        )
        .unwrap();
        assert_eq!(
            (pi_odd.role, pi_odd.direction),
            (MessageRole::Unknown, Direction::Unknown)
        );
        assert!(observe("pi", &json!({"type":"custom"})).is_none());
        let developer = observe(
            "codex",
            &json!({"type":"response_item","payload":{"type":"message","role":"developer","content":[{"type":"input_text"}]}}),
        )
        .unwrap();
        assert_eq!(
            (developer.role, developer.direction),
            (MessageRole::Developer, Direction::Internal)
        );
        let reasoning = observe(
            "codex",
            &json!({"type":"response_item","payload":{"type":"reasoning"}}),
        )
        .unwrap();
        assert_eq!(
            (reasoning.role, reasoning.direction),
            (MessageRole::Assistant, Direction::Internal)
        );
        assert!(observe(
            "codex",
            &json!({"type":"event_msg","payload":{"type":"user_message"}})
        )
        .is_none());
        assert!(observe(
            "codex",
            &json!({"type":"response_item","payload":{"type":"function_call"}})
        )
        .is_none());
        let chunk = observe(
            "grok",
            &json!({"method":"session/update","params":{"sessionId":"g","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"image"}}}}),
        )
        .unwrap();
        assert_eq!(
            (
                chunk.role,
                chunk.direction,
                chunk.grain,
                chunk.session.as_deref()
            ),
            (
                MessageRole::Assistant,
                Direction::Outbound,
                Grain::Fragment,
                Some("g")
            )
        );
        assert_eq!(chunk.modalities, set(&[Modality::Image]));
        assert!(observe(
            "grok",
            &json!({"method":"session/update","params":{"update":{"sessionUpdate":"tool_call"}}})
        )
        .is_none());
    }
}
