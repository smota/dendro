use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

const SECRET_MARKERS: &[&str] = &[
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

/// Redact secret-shaped substrings. Ordinary identifiers are unchanged.
pub fn contains_secret(input: &str) -> bool {
    redact(input) != input
}

pub fn redact(input: &str) -> String {
    let mut out = input.to_string();
    for marker in SECRET_MARKERS {
        let mut search_from = 0;
        while let Some(rel) = out[search_from..].find(marker) {
            let start = search_from + rel;
            let mut end = start + marker.len();
            let bytes = out.as_bytes();
            while end < bytes.len() && is_token_byte(bytes[end]) {
                end += 1;
            }
            if end > start + marker.len() || *marker == "Bearer " {
                out.replace_range(start..end, "[redacted]");
                search_from = start + "[redacted]".len();
            } else {
                search_from = start + marker.len();
            }
        }
    }
    out
}

fn is_token_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.'
}

pub fn key_id(key: &[u8]) -> String {
    let digest = Sha256::digest(key);
    hex::encode(&digest[..16])
}

pub fn hmac_domain(key: &[u8], domain: &str, material: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(key).expect("hmac key");
    mac.update(domain.as_bytes());
    mac.update(&[0]);
    mac.update(material.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

/// Tool fingerprint. `lexical` is the exact input slice, or the trimmed string body.
/// Missing input returns None so equality stays unknown.
pub fn tool_fingerprint(key: &[u8], tool: &str, lexical: Option<&str>) -> Option<String> {
    let lexical = lexical?;
    let trimmed = lexical.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(hmac_domain(
        key,
        "tool-input",
        &format!("{tool}\n{trimmed}"),
    ))
}

pub fn opaque_id(key: &[u8], domain: &str, native: &str) -> String {
    hmac_domain(key, domain, native)
}

/// Join key plus redacted display. Two raw values that redact the same stay distinct.
pub fn seal_identifier(key: &[u8], domain: &str, raw: &str) -> String {
    if let Some((opaque, display)) = raw.split_once('\t') {
        if display == "[redacted]"
            && !opaque.is_empty()
            && opaque.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return raw.to_string();
        }
    }
    let display = redact(raw);
    if display == raw {
        raw.to_string()
    } else {
        let opaque = opaque_id(key, domain, raw);
        format!("{opaque}\t{display}")
    }
}

/// Closed activity vocabulary. Anything else, including a missing value, is unknown.
pub fn activity_state(raw: Option<&str>) -> String {
    match raw {
        Some("running" | "idle" | "needs_input" | "unknown") => {
            raw.unwrap_or("unknown").to_string()
        }
        _ => "unknown".to_string(),
    }
}

/// Installed queue states. Unknown is neither open demand nor a successful close.
pub fn queue_state(raw: &str) -> String {
    match raw {
        "pending" | "in-progress" | "blocked" | "done" | "failed" | "denied" | "canceled"
        | "handed-off" | "unknown" => raw.to_string(),
        _ => "unknown".to_string(),
    }
}

/// Open text. The handoff sentinel and ordinary reasons stay; secret-shaped text is redacted.
pub fn closure_reason(raw: Option<&str>) -> Option<String> {
    raw.map(redact)
}

pub const TEXT_COLUMNS: &[&str] = &[
    "dendro_meta.key_id",
    "source_files.source_id",
    "source_files.harness",
    "source_files.display_path",
    "source_files.prefix_sha256",
    "codex_counters.source_id",
    "usage_facts.source_key",
    "usage_facts.harness",
    "usage_facts.native_session",
    "usage_facts.message_id",
    "usage_facts.source_id",
    "usage_facts.predecessor_source_key",
    "usage_facts.predecessor_reason",
    "tool_facts.source_key",
    "tool_facts.harness",
    "tool_facts.native_session",
    "tool_facts.tool_name",
    "tool_facts.fingerprint",
    "tool_facts.source_id",
    "identity_facts.source_key",
    "identity_facts.harness",
    "identity_facts.native_session",
    "identity_facts.rig",
    "identity_facts.seat",
    "activity_facts.source_key",
    "activity_facts.seat",
    "activity_facts.state",
    "queue_facts.source_key",
    "queue_facts.qitem_id",
    "queue_facts.state",
    "queue_facts.source_seat",
    "queue_facts.dest_seat",
    "queue_facts.closure_reason",
    "queue_facts.closure_target",
    "queue_facts.chain_id",
    "queue_facts.missions",
    "skill_facts.source_key",
    "skill_facts.seat",
    "skill_facts.skill",
    "seat_bounds.seat",
    "warnings.path",
    "warnings.reason",
    "destination_snapshots.source_key",
    "destination_snapshots.qitem_id",
    "destination_snapshots.destination",
    "destination_snapshots.snapshot_cutoff",
    "destination_snapshots.missions",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_secret_shapes_and_keeps_ordinary_ids() {
        let text = "mission:dendro-r1 tool=Write path=src/lib.rs sk-ant-secret123 ghp_abcdef";
        let got = redact(text);
        assert!(got.contains("mission:dendro-r1"));
        assert!(got.contains("tool=Write"));
        assert!(!got.contains("sk-ant-secret123"));
        assert!(!got.contains("ghp_abcdef"));
        assert!(got.contains("[redacted]"));
    }

    #[test]
    fn lexical_json_does_not_collapse_and_needs_the_key() {
        let key_a = b"key-a-32-bytes-pad-pad-pad-pad!!";
        let key_b = b"key-b-32-bytes-pad-pad-pad-pad!!";
        let spaced = r#"{"z": 1, "a": 2}"#;
        let ordered = r#"{"a": 2, "z": 1}"#;
        let left = tool_fingerprint(key_a, "Write", Some(spaced)).unwrap();
        let right = tool_fingerprint(key_a, "Write", Some(ordered)).unwrap();
        assert_ne!(left, right);
        assert_ne!(
            left,
            tool_fingerprint(key_b, "Write", Some(spaced)).unwrap()
        );
        assert!(tool_fingerprint(key_a, "Write", None).is_none());
    }

    #[test]
    fn activity_and_queue_states_drop_secret_text_without_changing_known_values() {
        assert_eq!(activity_state(Some("running")), "running");
        assert_eq!(activity_state(Some("idle")), "idle");
        assert_eq!(activity_state(Some("needs_input")), "needs_input");
        assert_eq!(activity_state(None), "unknown");
        assert_eq!(activity_state(Some("sk-ant-FAKESTATE")), "unknown");
        assert_eq!(queue_state("pending"), "pending");
        assert_eq!(queue_state("handed-off"), "handed-off");
        assert_eq!(queue_state("sk-ant-FAKESTATE"), "unknown");
        assert_eq!(
            closure_reason(Some("handed_off_to")).as_deref(),
            Some("handed_off_to")
        );
        assert_eq!(
            closure_reason(Some("no-follow-on")).as_deref(),
            Some("no-follow-on")
        );
        assert_eq!(
            closure_reason(Some("superseded")).as_deref(),
            Some("superseded")
        );
        assert!(closure_reason(None).is_none());
        let secret = closure_reason(Some("note sk-ant-FAKESTATE")).unwrap();
        assert!(!secret.contains("FAKESTATE"));
        assert!(secret.contains("note"));
    }

    #[test]
    fn seal_does_not_wrap_an_already_sealed_identifier() {
        let key = b"key-a-32-bytes-pad-pad-pad-pad!!";
        let once = seal_identifier(key, "chain", "item-sk-ant-FAKESTATE");
        let twice = seal_identifier(key, "chain", &once);
        assert_eq!(once, twice);
        assert!(!twice.contains("FAKESTATE"));
    }

    #[test]
    fn sealed_secret_ids_stay_distinct() {
        let key = b"key-a-32-bytes-pad-pad-pad-pad!!";
        let left = seal_identifier(key, "session", "sess-sk-ant-SECRETA");
        let right = seal_identifier(key, "session", "sess-sk-ant-SECRETB");
        assert_ne!(left, right);
        assert!(!left.contains("SECRETA"));
        assert!(!right.contains("SECRETB"));
        assert!(left.contains("[redacted]"));
        assert_eq!(left.split('\t').nth(1), right.split('\t').nth(1));
    }
}
