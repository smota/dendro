//! Sanitized `harness_envelope.envelope_shape` writer.
//! Unknown keys are HMAC-replaced. Secret-shaped keys are dropped.
//! Bounds and body or secret values refuse the whole row.
use duckdb::Connection;
use serde_json::{Map, Value};

use crate::privacy::{self, hmac_domain};

pub const MAX_DEPTH: usize = 8;
pub const MAX_ENCODED_BYTES: usize = 4096;
pub const MAX_KEYS: usize = 64;
pub const MAX_TOKEN_BYTES: usize = 64;
pub const MAX_ARRAY_ITEMS: usize = 16;

const ALLOWED_KEYS: &[&str] = &[
    "mcp_tools_enabled",
    "tool_arg_types",
    "provider_request_count",
    "provider_error_count",
    "capability_flags",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvelopeError {
    UnknownHarness,
    Bounds(&'static str),
    BodyCanary,
    SecretValue,
    NotStored,
    Duck(String),
}

impl std::fmt::Display for EnvelopeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownHarness => write!(f, "harness is not on the envelope allowlist"),
            Self::Bounds(reason) => write!(f, "envelope bounds refused: {reason}"),
            Self::BodyCanary => write!(f, "envelope body canary refused"),
            Self::SecretValue => write!(f, "envelope secret value refused"),
            Self::NotStored => write!(f, "envelope row was not stored"),
            Self::Duck(text) => write!(f, "{text}"),
        }
    }
}

impl std::error::Error for EnvelopeError {}

pub fn harness_admitted(harness: &str, harness_version: &str) -> bool {
    matches!(
        (harness, harness_version),
        ("codex" | "claude" | "grok" | "pi", "1")
    )
}

pub fn write_harness_envelope(
    conn: &Connection,
    privacy_key: &[u8],
    observation_id: &str,
    harness: &str,
    harness_version: &str,
    captured_at: &str,
    shape: &Value,
) -> Result<(), EnvelopeError> {
    if !harness_admitted(harness, harness_version) {
        return Err(EnvelopeError::UnknownHarness);
    }
    check_bounds(shape)?;
    let sanitized = sanitize_keys(shape, privacy_key)?;
    let encoded = serde_json::to_vec(&sanitized).map_err(|_| EnvelopeError::Bounds("encoded"))?;
    if encoded.len() > MAX_ENCODED_BYTES {
        return Err(EnvelopeError::Bounds("encoded size"));
    }
    let json = String::from_utf8(encoded).map_err(|_| EnvelopeError::Bounds("encoded"))?;
    conn.execute(
        "INSERT INTO harness_envelope (
            observation_id, harness, harness_version, captured_at, envelope_shape
         ) VALUES (?, ?, ?, CAST(? AS TIMESTAMP), CAST(CAST(? AS JSON) AS VARIANT))",
        duckdb::params![observation_id, harness, harness_version, captured_at, json],
    )
    .map_err(|error| EnvelopeError::Duck(error.to_string()))?;
    Ok(())
}

pub fn envelope_stored_text(
    conn: &Connection,
    observation_id: &str,
) -> Result<String, EnvelopeError> {
    conn.query_row(
        "SELECT COALESCE(CAST(envelope_shape AS VARCHAR), '')
         FROM harness_envelope WHERE observation_id = ?",
        [observation_id],
        |row| row.get(0),
    )
    .map_err(|error| EnvelopeError::Duck(error.to_string()))
}

pub fn envelope_variant_keys(
    conn: &Connection,
    observation_id: &str,
) -> Result<Vec<String>, EnvelopeError> {
    let mut stmt = conn
        .prepare(
            "SELECT unnest(variant_keys(envelope_shape))::VARCHAR
             FROM harness_envelope WHERE observation_id = ?",
        )
        .map_err(|error| EnvelopeError::Duck(error.to_string()))?;
    let rows = stmt
        .query_map([observation_id], |row| row.get::<_, String>(0))
        .map_err(|error| EnvelopeError::Duck(error.to_string()))?;
    let mut keys = Vec::new();
    for row in rows {
        keys.push(row.map_err(|error| EnvelopeError::Duck(error.to_string()))?);
    }
    keys.sort();
    Ok(keys)
}

pub fn envelope_field(
    conn: &Connection,
    observation_id: &str,
    key: &str,
) -> Result<String, EnvelopeError> {
    if !key
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        return Err(EnvelopeError::Bounds("field"));
    }
    let sql = format!(
        "SELECT COALESCE(CAST(variant_extract(envelope_shape, '{key}') AS VARCHAR), '')
         FROM harness_envelope WHERE observation_id = ?"
    );
    conn.query_row(&sql, [observation_id], |row| row.get(0))
        .map_err(|error| EnvelopeError::Duck(error.to_string()))
}

fn check_bounds(value: &Value) -> Result<(), EnvelopeError> {
    if nesting_depth(value) > MAX_DEPTH {
        return Err(EnvelopeError::Bounds("nesting depth"));
    }
    if key_count(value) > MAX_KEYS {
        return Err(EnvelopeError::Bounds("key count"));
    }
    let encoded = serde_json::to_vec(value).map_err(|_| EnvelopeError::Bounds("encoded"))?;
    if encoded.len() > MAX_ENCODED_BYTES {
        return Err(EnvelopeError::Bounds("encoded size"));
    }
    check_values(value)
}

fn check_values(value: &Value) -> Result<(), EnvelopeError> {
    match value {
        Value::Null => Err(EnvelopeError::Bounds("null")),
        Value::Bool(_) | Value::Number(_) => Ok(()),
        Value::String(text) => classify_string(text),
        Value::Array(items) => {
            if items.len() > MAX_ARRAY_ITEMS {
                return Err(EnvelopeError::Bounds("array length"));
            }
            for item in items {
                check_values(item)?;
            }
            Ok(())
        }
        Value::Object(map) => {
            for child in map.values() {
                check_values(child)?;
            }
            Ok(())
        }
    }
}

fn classify_string(text: &str) -> Result<(), EnvelopeError> {
    if text.len() > MAX_TOKEN_BYTES {
        return Err(EnvelopeError::Bounds("string token"));
    }
    if privacy::contains_secret(text) {
        return Err(EnvelopeError::SecretValue);
    }
    let lower = text.to_ascii_lowercase();
    if text.chars().any(char::is_whitespace)
        || text.contains('/')
        || text.contains('\\')
        || lower.contains("prompt")
        || lower.contains("transcript")
        || lower.contains("argv")
    {
        return Err(EnvelopeError::BodyCanary);
    }
    Ok(())
}

fn sanitize_keys(value: &Value, privacy_key: &[u8]) -> Result<Value, EnvelopeError> {
    match value {
        Value::Object(map) => {
            let mut out = Map::new();
            for (key, child) in map {
                let stored_key = if ALLOWED_KEYS.contains(&key.as_str()) {
                    key.clone()
                } else if secret_or_path_key(key) {
                    continue;
                } else {
                    format!(
                        "k_{}",
                        &hmac_domain(privacy_key, "envelope-key-v1", key)[..32]
                    )
                };
                if stored_key == *key && !ALLOWED_KEYS.contains(&key.as_str()) {
                    return Err(EnvelopeError::SecretValue);
                }
                out.insert(stored_key, sanitize_keys(child, privacy_key)?);
            }
            Ok(Value::Object(out))
        }
        Value::Array(items) => Ok(Value::Array(
            items
                .iter()
                .map(|item| sanitize_keys(item, privacy_key))
                .collect::<Result<Vec<_>, _>>()?,
        )),
        other => Ok(other.clone()),
    }
}

fn secret_or_path_key(key: &str) -> bool {
    privacy::contains_secret(key) || key.contains('/') || key.contains('\\') || key.contains("..")
}

fn nesting_depth(value: &Value) -> usize {
    match value {
        Value::Object(map) => 1 + map.values().map(nesting_depth).max().unwrap_or(0),
        Value::Array(items) => 1 + items.iter().map(nesting_depth).max().unwrap_or(0),
        _ => 0,
    }
}

fn key_count(value: &Value) -> usize {
    match value {
        Value::Object(map) => map.len() + map.values().map(key_count).sum::<usize>(),
        Value::Array(items) => items.iter().map(key_count).sum(),
        _ => 0,
    }
}
