use hmac::{Hmac, Mac};
use sha2::Sha256;

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

use sha2::Digest;

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
}
