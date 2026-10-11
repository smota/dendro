use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use anyhow::Result;
use dendro_core::{
    tool_fingerprint, AnalyticsStore, CodexCounters, ToolFact, UsageFact, WarningNote,
};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Debug, Default)]
struct Buckets {
    input_uncached: i64,
    output: i64,
    cache_read: i64,
    cache_write: i64,
    reasoning: i64,
    currency_total: Option<f64>,
    message_id: Option<String>,
    native_session: String,
    ts: Option<chrono::DateTime<chrono::Utc>>,
    predecessor_reason: Option<String>,
    prior_input_tokens: Option<i64>,
    prior_cached_input_tokens: Option<i64>,
    prior_cache_write_input_tokens: Option<i64>,
    prior_output_tokens: Option<i64>,
    prior_reasoning_output_tokens: Option<i64>,
}

pub fn ingest_jsonl(
    store: &AnalyticsStore,
    source_id: &str,
    harness: &str,
    path: &Path,
) -> Result<()> {
    ingest_jsonl_with_key(store, source_id, harness, path, None).map(|_| ())
}

/// What one ingest call read from a session file (AC1).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FileStats {
    /// Records read: complete, non-blank JSONL lines read by this call. Lines
    /// before the stored watermark were read by an earlier run and are not
    /// counted again; a trailing partial line is not a record yet.
    pub records_read: u64,
    /// Usage records stored by this call, after dedupe.
    pub usage_records: u64,
}

pub fn ingest_jsonl_with_key(
    store: &AnalyticsStore,
    source_id: &str,
    harness: &str,
    path: &Path,
    tool_key: Option<&[u8]>,
) -> Result<FileStats> {
    let mut file = File::open(path)?;
    let file_len = file.metadata()?.len() as i64;
    let prior = store.source_state(source_id)?;
    let (mut generation, mut offset, expected_hash) =
        prior.clone().unwrap_or((1, 0, String::new()));
    if offset > file_len {
        generation += 1;
        offset = 0;
    } else if offset > 0 {
        let actual = hash_prefix(&mut file, offset as u64)?;
        if actual != expected_hash {
            generation += 1;
            offset = 0;
        }
    }
    file.seek(SeekFrom::Start(0))?;
    let mut hasher = Sha256::new();
    let mut reader = BufReader::new(file);
    let mut prefix = vec![0u8; offset as usize];
    if offset > 0 {
        reader.read_exact(&mut prefix)?;
        hasher.update(&prefix);
    }
    let mut line_no = prefix.iter().filter(|b| **b == b'\n').count() as i64;
    let mut pending = Vec::new();
    let mut buf = Vec::new();
    loop {
        buf.clear();
        let n = reader.read_until(b'\n', &mut buf)?;
        if n == 0 {
            break;
        }
        if !buf.ends_with(b"\n") {
            break;
        }
        hasher.update(&buf);
        offset += buf.len() as i64;
        line_no += 1;
        pending.push((line_no, buf[..buf.len() - 1].to_vec()));
    }
    let prefix_sha = hex::encode(hasher.finalize());
    let reset_generation = generation_was_reset(prior, generation);
    let mut counters = if reset_generation {
        CodexCounters::default()
    } else {
        store.codex_counters(source_id, generation)?
    };
    let start_offset = prefix.len();
    let mut warnings = Vec::new();
    // K10: Pi and Codex sessions are named by the id on line 1 of the file.
    let file_session = match harness {
        "pi" | "codex" => Some(file_session(harness, path, source_id)?),
        _ => None,
    };
    if let Some(session) = &file_session {
        if !session.from_header && start_offset == 0 {
            warnings.push(WarningNote {
                path: path.display().to_string(),
                line: Some(1),
                reason: "missing session header".into(),
            });
        }
    }
    let mut first_codex_in_this_call = true;
    let mut usages = Vec::new();
    let mut tools = Vec::new();
    let mut seen_messages = HashSet::new();
    let mut unknown_types = HashSet::new();
    let records_read = pending
        .iter()
        .filter(|(_, bytes)| !bytes.iter().all(|b| b.is_ascii_whitespace()))
        .count() as u64;
    for (line_no, bytes) in pending {
        dendro_core::RUN_CANCEL.check()?;
        let text = match String::from_utf8(bytes) {
            Ok(text) => text,
            Err(_) => {
                warnings.push(WarningNote {
                    path: path.display().to_string(),
                    line: Some(line_no),
                    reason: "invalid-utf8".into(),
                });
                continue;
            }
        };
        if text.trim().is_empty() {
            continue;
        }
        if let Some(session) = &file_session {
            if session.from_header
                && line_no > 1
                && conflicting_session(harness, &text, &session.raw)
            {
                warnings.push(WarningNote {
                    path: path.display().to_string(),
                    line: Some(line_no),
                    reason: "conflicting session header".into(),
                });
            }
        }
        match parse_line(harness, &text, &mut counters) {
            Ok(None) => {}
            Ok(Some(mut bucket)) => {
                if let Some(session) = &file_session {
                    // The file's line-1 session wins over any per-line id.
                    bucket.native_session = session.raw.clone();
                }
                if let Some(id) = &bucket.message_id {
                    let sealed_session = seal_session(tool_key, harness, &bucket.native_session);
                    let sealed_id = seal_message(tool_key, id);
                    let seen = format!("{harness}\n{sealed_session}\n{sealed_id}");
                    if !seen_messages.insert(seen)
                        || store.message_seen(harness, &sealed_session, &sealed_id, generation)?
                    {
                        continue;
                    }
                }
                usages.push(UsageFact {
                    source_key: format!("{source_id}:{generation}:{line_no}"),
                    harness: harness.to_string(),
                    native_session: seal_session(tool_key, harness, &bucket.native_session),
                    message_id: bucket
                        .message_id
                        .as_deref()
                        .map(|id| seal_message(tool_key, id)),
                    ts: bucket.ts,
                    seq: line_no,
                    input_uncached: bucket.input_uncached,
                    output: bucket.output,
                    cache_read: bucket.cache_read,
                    cache_write: bucket.cache_write,
                    reasoning: bucket.reasoning,
                    currency_total: bucket.currency_total,
                    generation,
                    predecessor_source_key: None,
                    predecessor_reason: bucket.predecessor_reason.clone(),
                    prior_input_tokens: bucket.prior_input_tokens,
                    prior_cached_input_tokens: bucket.prior_cached_input_tokens,
                    prior_cache_write_input_tokens: bucket.prior_cache_write_input_tokens,
                    prior_output_tokens: bucket.prior_output_tokens,
                    prior_reasoning_output_tokens: bucket.prior_reasoning_output_tokens,
                });
                if harness == "codex" && reset_generation && first_codex_in_this_call {
                    if let Some(current) = usages.last_mut() {
                        current.predecessor_reason = Some("reset".into());
                        current.predecessor_source_key = None;
                        current.prior_input_tokens = None;
                        current.prior_cached_input_tokens = None;
                        current.prior_cache_write_input_tokens = None;
                        current.prior_output_tokens = None;
                        current.prior_reasoning_output_tokens = None;
                    }
                } else if bucket.predecessor_reason.as_deref() == Some("prior") {
                    let key = usages
                        .iter()
                        .rev()
                        .nth(1)
                        .map(|prior| prior.source_key.clone())
                        .or(store.latest_usage_key(source_id, generation).ok().flatten());
                    if let Some(current) = usages.last_mut() {
                        current.predecessor_source_key = key;
                    }
                }
                if harness == "codex" {
                    first_codex_in_this_call = false;
                }
            }
            Err(reason) => {
                // One warning per (file, unknown type), at its first occurrence
                // in this or any earlier run, so a repeated type cannot flood.
                let repeat = reason.starts_with(UNKNOWN_RECORD_TYPE)
                    && (!unknown_types.insert(reason.clone())
                        || store.has_warning(&path.display().to_string(), &reason)?);
                if !repeat {
                    warnings.push(WarningNote {
                        path: path.display().to_string(),
                        line: Some(line_no),
                        reason,
                    });
                }
            }
        }
        if harness == "claude" {
            tools.extend(claude_tools(
                &text, tool_key, source_id, generation, line_no,
            ));
        }
    }
    store.commit_file_batch(
        source_id,
        harness,
        &path.display().to_string(),
        generation,
        offset,
        &prefix_sha,
        if harness == "codex" {
            Some(&counters)
        } else {
            None
        },
        &usages,
        &tools,
    )?;
    if let Some(session) = &file_session {
        store.set_source_session(source_id, &seal_session(tool_key, harness, &session.raw))?;
    }
    for note in &warnings {
        store.record_warning(note)?;
    }
    Ok(FileStats {
        records_read,
        usage_records: usages.len() as u64,
    })
}

/// A session file's native session: the line-1 header id, or a per-file
/// fallback that never links by guess.
struct FileSession {
    raw: String,
    from_header: bool,
}

/// Longest line 1 read for the header; a longer first line is malformed.
const HEADER_LIMIT: u64 = 1 << 20;

/// Reads only line 1, buffered and bounded, independent of the watermark.
fn file_session(harness: &str, path: &Path, source_id: &str) -> Result<FileSession> {
    let mut line = Vec::new();
    BufReader::new(File::open(path)?.take(HEADER_LIMIT)).read_until(b'\n', &mut line)?;
    let id = serde_json::from_slice::<Value>(line.strip_suffix(b"\n").unwrap_or(&line))
        .ok()
        .and_then(|value| header_id(harness, &value));
    Ok(match id {
        Some(raw) => FileSession {
            raw,
            from_header: true,
        },
        None => FileSession {
            raw: format!("{harness}-file:{source_id}"),
            from_header: false,
        },
    })
}

/// The native id a header line declares: Pi `{"type":"session","id"}`, Codex
/// `{"type":"session_meta","payload":{"id"}}`.
fn header_id(harness: &str, value: &Value) -> Option<String> {
    let kind = value.get("type").and_then(|v| v.as_str());
    let id = match (harness, kind) {
        ("pi", Some("session")) => value.get("id"),
        ("codex", Some("session_meta")) => value.pointer("/payload/id"),
        _ => None,
    };
    id.and_then(|v| v.as_str())
        .filter(|id| !id.trim().is_empty())
        .map(str::to_string)
}

/// A later line that names a different session: a repeated header, or a
/// Codex line whose payload.session_id differs. Same-id repeats are fine.
fn conflicting_session(harness: &str, text: &str, first: &str) -> bool {
    if !text.contains("session") {
        return false;
    }
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return false;
    };
    let named = header_id(harness, &value).or_else(|| {
        (harness == "codex")
            .then(|| {
                value
                    .pointer("/payload/session_id")
                    .and_then(|v| v.as_str())
            })
            .flatten()
            .map(str::to_string)
    });
    named.is_some_and(|id| id != first)
}

fn seal_session(key: Option<&[u8]>, harness: &str, raw: &str) -> String {
    match key {
        Some(key) => dendro_core::seal_identifier(key, &format!("session:{harness}"), raw),
        None => raw.to_string(),
    }
}

fn seal_message(key: Option<&[u8]>, raw: &str) -> String {
    match key {
        Some(key) => dendro_core::seal_identifier(key, "message", raw),
        None => raw.to_string(),
    }
}

fn generation_was_reset(prior: Option<(i64, i64, String)>, generation: i64) -> bool {
    prior.map(|(g, _, _)| g != generation).unwrap_or(false)
}

fn hash_prefix(file: &mut File, len: u64) -> Result<String> {
    file.seek(SeekFrom::Start(0))?;
    let mut limited = file.take(len);
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = limited.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Top-level record types each parser knows (AC2, F4). Derived from the sealed
/// corpus of the harness versions in use on 2026-10-11 (Claude 15, Codex 7,
/// Pi 6), not from a published schema. Any other type, or none, is unknown.
const CLAUDE_RECORD_TYPES: &[&str] = &[
    "agent-name",
    "assistant",
    "atis-latch",
    "attachment",
    "cost-state",
    "custom-title",
    "file-history-delta",
    "file-history-snapshot",
    "last-prompt",
    "mode",
    "permission-mode",
    "pr-link",
    "queue-operation",
    "system",
    "user",
];
const CODEX_RECORD_TYPES: &[&str] = &[
    "compacted",
    "event_msg",
    "response_item",
    "session_meta",
    "token_usage_record",
    "turn_context",
    "world_state",
];
const PI_RECORD_TYPES: &[&str] = &[
    "custom",
    "message",
    "model_change",
    "session",
    "session_info",
    "thinking_level_change",
];

pub(crate) const UNKNOWN_RECORD_TYPE: &str = "unknown-record-type";

/// The warning reason for a record of a type the parser does not know, or
/// None. The type is shown only when it is a plain identifier of at most 64
/// characters; storage still applies the AC10 redaction to it.
fn unknown_record_type(harness: &str, value: &Value) -> Option<String> {
    let known = match harness {
        "claude" => CLAUDE_RECORD_TYPES,
        "codex" => CODEX_RECORD_TYPES,
        "pi" => PI_RECORD_TYPES,
        _ => return None,
    };
    let kind = value.get("type").and_then(|v| v.as_str());
    if kind.is_some_and(|kind| known.contains(&kind)) {
        return None;
    }
    let plain = kind.filter(|kind| {
        !kind.is_empty()
            && kind.len() <= 64
            && kind
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
    });
    Some(match plain {
        Some(kind) => format!("{UNKNOWN_RECORD_TYPE}: {kind}"),
        None => UNKNOWN_RECORD_TYPE.to_string(),
    })
}

fn parse_line(
    harness: &str,
    line: &str,
    counters: &mut CodexCounters,
) -> Result<Option<Buckets>, String> {
    let value: Value = serde_json::from_str(line).map_err(|_| "malformed-json".to_string())?;
    if let Some(reason) = unknown_record_type(harness, &value) {
        return Err(reason);
    }
    match harness {
        "codex" => parse_codex(&value, counters),
        "claude" => parse_claude(&value),
        "pi" => parse_pi(&value),
        _ => Err("unknown-harness".into()),
    }
}

fn parse_codex(value: &Value, counters: &mut CodexCounters) -> Result<Option<Buckets>, String> {
    let kind = value.get("type").and_then(|v| v.as_str()).unwrap_or("");
    if kind == "token_usage_record" {
        return Ok(None);
    }
    if kind != "event_msg" {
        return Ok(None);
    }
    let payload = value.get("payload").ok_or("missing-payload")?;
    if payload.get("type").and_then(|v| v.as_str()) != Some("token_count") {
        return Ok(None);
    }
    let total = payload
        .pointer("/info/total_token_usage")
        .ok_or("missing-total-usage")?;
    let next = CodexCounters {
        input_tokens: json_i64(total, "input_tokens")?,
        cached_input_tokens: json_i64(total, "cached_input_tokens")?,
        cache_write_input_tokens: json_i64(total, "cache_write_input_tokens")?,
        output_tokens: json_i64(total, "output_tokens")?,
        reasoning_output_tokens: json_i64(total, "reasoning_output_tokens")?,
    };
    let reset = next.input_tokens < counters.input_tokens
        || next.cached_input_tokens < counters.cached_input_tokens
        || next.cache_write_input_tokens < counters.cache_write_input_tokens
        || next.output_tokens < counters.output_tokens
        || next.reasoning_output_tokens < counters.reasoning_output_tokens;
    let input_delta = if reset {
        next.input_tokens
    } else {
        next.input_tokens - counters.input_tokens
    };
    let cached_delta = if reset {
        next.cached_input_tokens
    } else {
        next.cached_input_tokens - counters.cached_input_tokens
    };
    let write_delta = if reset {
        next.cache_write_input_tokens
    } else {
        next.cache_write_input_tokens - counters.cache_write_input_tokens
    };
    let output_delta = if reset {
        next.output_tokens
    } else {
        next.output_tokens - counters.output_tokens
    };
    let reasoning_delta = if reset {
        next.reasoning_output_tokens
    } else {
        next.reasoning_output_tokens - counters.reasoning_output_tokens
    };
    let (predecessor_reason, priors) = if reset {
        ("reset".to_string(), None)
    } else if counters.input_tokens == 0
        && counters.cached_input_tokens == 0
        && counters.cache_write_input_tokens == 0
        && counters.output_tokens == 0
        && counters.reasoning_output_tokens == 0
    {
        ("baseline".to_string(), None)
    } else {
        (
            "prior".to_string(),
            Some((
                counters.input_tokens,
                counters.cached_input_tokens,
                counters.cache_write_input_tokens,
                counters.output_tokens,
                counters.reasoning_output_tokens,
            )),
        )
    };
    *counters = next;
    let input_uncached = input_delta - cached_delta - write_delta;
    if input_uncached < 0 {
        return Err("negative-exclusive-input".into());
    }
    Ok(Some(Buckets {
        input_uncached,
        output: output_delta,
        cache_read: cached_delta,
        cache_write: write_delta,
        reasoning: reasoning_delta,
        currency_total: None,
        message_id: None,
        native_session: value
            .pointer("/payload/session_id")
            .and_then(|v| v.as_str())
            .unwrap_or("codex-session")
            .to_string(),
        ts: timestamp_of(value),
        predecessor_reason: Some(predecessor_reason),
        prior_input_tokens: priors.map(|p| p.0),
        prior_cached_input_tokens: priors.map(|p| p.1),
        prior_cache_write_input_tokens: priors.map(|p| p.2),
        prior_output_tokens: priors.map(|p| p.3),
        prior_reasoning_output_tokens: priors.map(|p| p.4),
    }))
}

fn parse_claude(value: &Value) -> Result<Option<Buckets>, String> {
    if value.get("type").and_then(|v| v.as_str()) != Some("assistant") {
        return Ok(None);
    }
    let usage = value.pointer("/message/usage").ok_or("missing-usage")?;
    Ok(Some(Buckets {
        input_uncached: json_i64(usage, "input_tokens")?,
        output: json_i64(usage, "output_tokens")?,
        cache_read: json_i64(usage, "cache_read_input_tokens")?,
        cache_write: json_i64(usage, "cache_creation_input_tokens")?,
        reasoning: 0,
        currency_total: None,
        message_id: value
            .pointer("/message/id")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        native_session: value
            .get("sessionId")
            .and_then(|v| v.as_str())
            .unwrap_or("claude-session")
            .to_string(),
        ts: timestamp_of(value),
        predecessor_reason: None,
        prior_input_tokens: None,
        prior_cached_input_tokens: None,
        prior_cache_write_input_tokens: None,
        prior_output_tokens: None,
        prior_reasoning_output_tokens: None,
    }))
}

fn parse_pi(value: &Value) -> Result<Option<Buckets>, String> {
    if value.get("type").and_then(|v| v.as_str()) != Some("message") {
        return Ok(None);
    }
    let message = value.get("message").ok_or("missing-message")?;
    if message.get("role").and_then(|v| v.as_str()) != Some("assistant") {
        return Ok(None);
    }
    let usage = message.get("usage").ok_or("missing-usage")?;
    let currency_total = usage.pointer("/cost/total").and_then(|v| v.as_f64());
    Ok(Some(Buckets {
        input_uncached: json_i64(usage, "input")?,
        output: json_i64(usage, "output")?,
        cache_read: json_i64(usage, "cacheRead")?,
        cache_write: json_i64(usage, "cacheWrite")?,
        reasoning: json_i64(usage, "reasoning").unwrap_or(0),
        currency_total,
        message_id: value.get("id").and_then(|v| v.as_str()).map(str::to_string),
        native_session: "pi-session".into(),
        ts: timestamp_of(value),
        predecessor_reason: None,
        prior_input_tokens: None,
        prior_cached_input_tokens: None,
        prior_cache_write_input_tokens: None,
        prior_output_tokens: None,
        prior_reasoning_output_tokens: None,
    }))
}

fn claude_tools(
    text: &str,
    key: Option<&[u8]>,
    source_id: &str,
    generation: i64,
    line_no: i64,
) -> Vec<ToolFact> {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    let Some(content) = value
        .pointer("/message/content")
        .and_then(|item| item.as_array())
    else {
        return Vec::new();
    };
    let session = value
        .get("sessionId")
        .and_then(|item| item.as_str())
        .unwrap_or("claude-session");
    let ts = timestamp_of(&value);
    let mut tools = Vec::new();
    for (index, block) in content.iter().enumerate() {
        if block.get("type").and_then(|item| item.as_str()) != Some("tool_use") {
            continue;
        }
        let Some(name) = block.get("name").and_then(|item| item.as_str()) else {
            continue;
        };
        let lexical = block.get("input").and_then(|item| item.as_str());
        let fingerprint = key.and_then(|key| tool_fingerprint(key, name, lexical));
        let failed = block.get("is_error").and_then(|item| item.as_bool());
        tools.push(ToolFact {
            source_key: format!("{source_id}:{generation}:{line_no}:{index}"),
            harness: "claude".into(),
            // Sealed like the usage session, so tools join the same session and seat
            // and two different secret-shaped sessions stay distinct.
            native_session: seal_session(key, "claude", session),
            ts,
            seq: line_no * 100 + index as i64,
            name: name.to_string(),
            fingerprint,
            failed,
            structured_write: matches!(name, "Write" | "Edit" | "MultiEdit" | "NotebookEdit"),
            generation,
        });
    }
    tools
}

fn timestamp_of(value: &Value) -> Option<chrono::DateTime<chrono::Utc>> {
    let text = value
        .get("timestamp")
        .and_then(|item| item.as_str())
        .or_else(|| {
            value
                .pointer("/message/timestamp")
                .and_then(|item| item.as_str())
        })?;
    chrono::DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|parsed| parsed.with_timezone(&chrono::Utc))
}

fn json_i64(value: &Value, key: &str) -> Result<i64, String> {
    value
        .get(key)
        .ok_or_else(|| format!("missing-{key}"))?
        .as_i64()
        .ok_or_else(|| format!("non-integer-{key}"))
}

pub fn file_sha256(path: &Path) -> Result<String> {
    let bytes = fs::read(path)?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::OpenFlags;
    use std::io::Write;

    mod header_identity {
        use super::*;

        const KEY: &[u8] = b"header-key-32-bytes-pad-pad-pad!";

        fn pi_header(id: &str) -> String {
            format!("{{\"type\":\"session\",\"version\":3,\"id\":\"{id}\",\"timestamp\":\"2026-10-09T00:00:00Z\"}}")
        }

        fn pi_usage(msg: &str, input: i64) -> String {
            format!("{{\"type\":\"message\",\"id\":\"{msg}\",\"timestamp\":\"2026-10-09T00:00:01Z\",\"message\":{{\"role\":\"assistant\",\"usage\":{{\"input\":{input},\"output\":0,\"cacheRead\":0,\"cacheWrite\":0}}}}}}")
        }

        fn codex_header(id: &str) -> String {
            format!("{{\"type\":\"session_meta\",\"timestamp\":\"2026-10-09T00:00:00Z\",\"payload\":{{\"id\":\"{id}\"}}}}")
        }

        fn codex_count(total: i64, session: Option<&str>) -> String {
            let sid = session
                .map(|s| format!(",\"session_id\":\"{s}\""))
                .unwrap_or_default();
            format!("{{\"type\":\"event_msg\",\"timestamp\":\"2026-10-09T00:00:01Z\",\"payload\":{{\"type\":\"token_count\"{sid},\"info\":{{\"total_token_usage\":{{\"input_tokens\":{total},\"cached_input_tokens\":0,\"cache_write_input_tokens\":0,\"output_tokens\":0,\"reasoning_output_tokens\":0}}}}}}}}")
        }

        struct Fixture {
            dir: PathBuf,
            db: AnalyticsStore,
        }

        impl Fixture {
            fn new(name: &str) -> Self {
                let dir = std::env::temp_dir()
                    .join(format!("dendro-header-{name}-{}", std::process::id()));
                let _ = fs::remove_dir_all(&dir);
                fs::create_dir_all(&dir).unwrap();
                let db = AnalyticsStore::open_path(&dir.join("a.duckdb"), "key").unwrap();
                Fixture { dir, db }
            }

            fn file(&self, name: &str, lines: &[String]) -> PathBuf {
                let path = self.dir.join(name);
                fs::write(
                    &path,
                    lines.iter().map(|l| format!("{l}\n")).collect::<String>(),
                )
                .unwrap();
                path
            }

            fn append(&self, path: &Path, lines: &[String]) {
                let mut f = fs::OpenOptions::new().append(true).open(path).unwrap();
                for l in lines {
                    writeln!(f, "{l}").unwrap();
                }
            }

            fn ingest(&self, harness: &str, path: &Path) {
                let id = format!("src-{}", path.file_name().unwrap().to_string_lossy());
                ingest_jsonl_with_key(&self.db, &id, harness, path, Some(KEY)).unwrap();
            }

            /// (sealed session, total tokens) per session.
            fn sessions(&self, harness: &str) -> std::collections::BTreeMap<String, i64> {
                let mut out = std::collections::BTreeMap::new();
                for u in self
                    .db
                    .load_usage()
                    .unwrap()
                    .into_iter()
                    .filter(|u| u.harness == harness)
                {
                    *out.entry(u.native_session).or_insert(0) +=
                        u.input_uncached + u.cache_read + u.cache_write + u.output;
                }
                out
            }

            fn sealed(&self, harness: &str, raw: &str) -> String {
                seal_session(Some(KEY), harness, raw)
            }

            fn warnings(&self) -> Vec<String> {
                self.db.warning_reasons().unwrap()
            }
        }

        impl Drop for Fixture {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.dir);
            }
        }

        fn claude_usage(msg: &str, input: i64, at: Option<&str>) -> String {
            let ts = at
                .map(|t| format!(",\"timestamp\":\"{t}\""))
                .unwrap_or_default();
            format!("{{\"type\":\"assistant\",\"sessionId\":\"s\"{ts},\"message\":{{\"id\":\"{msg}\",\"usage\":{{\"input_tokens\":{input},\"output_tokens\":0,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0}}}}}}")
        }

        fn unknown_warnings(f: &Fixture) -> Vec<String> {
            f.warnings()
                .into_iter()
                .filter(|w| w.starts_with(UNKNOWN_RECORD_TYPE))
                .collect()
        }

        #[test]
        fn an_unknown_claude_record_warns_beside_malformed_lines_qa_fixture() {
            // QA's six lines: valid 10, malformed JSON, invalid UTF-8, an
            // invented type, valid 20, no timestamp 5.
            let f = Fixture::new("unknown-qa");
            let path = f.dir.join("c.jsonl");
            let mut bytes = Vec::new();
            for line in [
                claude_usage("m1", 10, Some("2026-10-09T00:00:01Z")).into_bytes(),
                b"{not json".to_vec(),
                vec![0xff, 0xfe, b'{', b'}'],
                b"{\"type\":\"qa_unknown_record_version_999\",\"timestamp\":\"2026-10-09T00:00:02Z\"}".to_vec(),
                claude_usage("m2", 20, Some("2026-10-09T00:00:03Z")).into_bytes(),
                claude_usage("m3", 5, None).into_bytes(),
            ] {
                bytes.extend(line);
                bytes.push(b'\n');
            }
            fs::write(&path, bytes).unwrap();
            f.ingest("claude", &path);
            let mut reasons = f.warnings();
            reasons.sort();
            assert_eq!(
                reasons,
                [
                    "invalid-utf8",
                    "malformed-json",
                    "unknown-record-type: qa_unknown_record_version_999"
                ]
            );
            let usage = f.db.load_usage().unwrap();
            let timed: i64 = usage
                .iter()
                .filter(|u| u.ts.is_some())
                .map(|u| u.input_uncached)
                .sum();
            let untimed: i64 = usage
                .iter()
                .filter(|u| u.ts.is_none())
                .map(|u| u.input_uncached)
                .sum();
            assert_eq!((timed, untimed), (30, 5));
        }

        #[test]
        fn each_harness_warns_once_per_file_and_unknown_type() {
            let f = Fixture::new("unknown-parity");
            let unknown = |kind: &str| {
                format!("{{\"type\":\"{kind}\",\"timestamp\":\"2026-10-09T00:00:02Z\"}}")
            };
            let c = f.file(
                "c.jsonl",
                &[
                    claude_usage("m1", 1, Some("2026-10-09T00:00:01Z")),
                    unknown("future_kind"),
                    unknown("future_kind"),
                    "{\"timestamp\":\"2026-10-09T00:00:03Z\"}".to_string(),
                ],
            );
            let x = f.file(
                "x.jsonl",
                &[
                    codex_header("cx"),
                    unknown("future_kind"),
                    codex_count(4, None),
                ],
            );
            let p = f.file(
                "p.jsonl",
                &[pi_header("pi"), unknown("future_kind"), pi_usage("a", 2)],
            );
            f.ingest("claude", &c);
            f.ingest("codex", &x);
            f.ingest("pi", &p);
            let mut found = unknown_warnings(&f);
            found.sort();
            // Claude: the type once (not per line) and one missing-type record.
            assert_eq!(
                found,
                [
                    "unknown-record-type",
                    "unknown-record-type: future_kind",
                    "unknown-record-type: future_kind",
                    "unknown-record-type: future_kind",
                ]
            );
            // Known records still count beside unknown ones.
            assert_eq!(f.sessions("codex").values().sum::<i64>(), 4);
            assert_eq!(f.sessions("pi").values().sum::<i64>(), 2);
            // Appending the same unknown type later adds no warning.
            f.append(&c, &[unknown("future_kind"), unknown("other_kind")]);
            f.ingest("claude", &c);
            let after = unknown_warnings(&f);
            assert_eq!(after.len(), found.len() + 1);
            assert!(after.contains(&"unknown-record-type: other_kind".to_string()));
        }

        #[test]
        fn an_unknown_type_name_never_shows_secret_or_odd_text() {
            let f = Fixture::new("unknown-secret");
            let c = f.file(
                "c.jsonl",
                &[
                    "{\"type\":\"sk-ant-api03-ZZTYPEsecretvalue\"}".to_string(),
                    "{\"type\":\"has space\"}".to_string(),
                    format!("{{\"type\":\"{}\"}}", "a".repeat(65)),
                    "{\"type\":7}".to_string(),
                ],
            );
            f.ingest("claude", &c);
            let found = unknown_warnings(&f);
            assert_eq!(found.len(), 2, "{found:?}");
            assert!(found.iter().all(|w| !w.contains("ZZTYPEsecretvalue")));
            assert!(found.contains(&"unknown-record-type: [redacted]".to_string()));
            assert!(found.contains(&"unknown-record-type".to_string()));
        }

        #[test]
        fn pi_sessions_come_from_line_one_and_never_collapse() {
            let f = Fixture::new("pi");
            let a = f.file(
                "a.jsonl",
                &[
                    pi_header("native-pi-1"),
                    pi_usage("same", 10),
                    pi_usage("same", 10),
                ],
            );
            let b = f.file("b.jsonl", &[pi_header("native-pi-2"), pi_usage("same", 20)]);
            f.ingest("pi", &a);
            f.ingest("pi", &b);
            let sessions = f.sessions("pi");
            assert_eq!(
                sessions.get(&f.sealed("pi", "native-pi-1")),
                Some(&10),
                "in-session duplicate counted once"
            );
            assert_eq!(
                sessions.get(&f.sealed("pi", "native-pi-2")),
                Some(&20),
                "same local id, other session"
            );
            assert_eq!(sessions.values().sum::<i64>(), 30);
            assert!(!sessions.contains_key(&f.sealed("pi", "pi-session")));
        }

        #[test]
        fn codex_sessions_come_from_session_meta_with_reset_arithmetic_unchanged() {
            let f = Fixture::new("codex");
            let a = f.file(
                "a.jsonl",
                &[
                    codex_header("native-codex-1"),
                    codex_count(10, None),
                    codex_count(5, None),
                ],
            );
            let b = f.file(
                "b.jsonl",
                &[codex_header("native-codex-2"), codex_count(20, None)],
            );
            f.ingest("codex", &a);
            f.ingest("codex", &b);
            let sessions = f.sessions("codex");
            assert_eq!(
                sessions.get(&f.sealed("codex", "native-codex-1")),
                Some(&15),
                "10, then reset 5"
            );
            assert_eq!(
                sessions.get(&f.sealed("codex", "native-codex-2")),
                Some(&20)
            );
            assert_eq!(sessions.values().sum::<i64>(), 35);
            let reasons: Vec<_> =
                f.db.load_usage()
                    .unwrap()
                    .into_iter()
                    .filter_map(|u| u.predecessor_reason)
                    .collect();
            assert!(
                reasons.contains(&"reset".to_string()) && reasons.contains(&"baseline".to_string()),
                "{reasons:?}"
            );
        }

        #[test]
        fn a_missing_or_malformed_header_uses_a_per_file_fallback_and_is_never_rescued() {
            for (harness, later) in [
                ("pi", pi_header("late-id")),
                ("codex", codex_header("late-id")),
            ] {
                let f = Fixture::new(&format!("missing-{harness}"));
                let usage = |t| {
                    if harness == "pi" {
                        pi_usage("m", t)
                    } else {
                        codex_count(t, Some("late-id"))
                    }
                };
                let a = f.file("a.jsonl", &[usage(4), later.clone(), usage(6)]);
                let b = f.file("b.jsonl", &["not json".to_string(), usage(9)]);
                f.ingest(harness, &a);
                f.ingest(harness, &b);
                let sessions = f.sessions(harness);
                let fa = f.sealed(harness, &format!("{harness}-file:src-a.jsonl"));
                let fb = f.sealed(harness, &format!("{harness}-file:src-b.jsonl"));
                assert!(
                    sessions.contains_key(&fa) && sessions.contains_key(&fb),
                    "{harness}: {sessions:?}"
                );
                assert_eq!(
                    sessions.len(),
                    2,
                    "{harness}: files never merge, the late id never rescues"
                );
                assert!(!sessions.contains_key(&f.sealed(harness, "late-id")));
                assert_eq!(
                    f.warnings()
                        .iter()
                        .filter(|w| *w == "missing session header")
                        .count(),
                    2,
                    "{harness}"
                );
            }
        }

        #[test]
        fn a_later_conflicting_header_or_codex_id_warns_and_keeps_the_first() {
            let f = Fixture::new("conflict");
            let p = f.file(
                "p.jsonl",
                &[
                    pi_header("first"),
                    pi_usage("a", 3),
                    pi_header("first"),
                    pi_header("second"),
                    pi_usage("b", 4),
                ],
            );
            let c = f.file(
                "c.jsonl",
                &[
                    codex_header("first"),
                    codex_count(3, Some("other")),
                    codex_count(7, Some("first")),
                ],
            );
            f.ingest("pi", &p);
            f.ingest("codex", &c);
            assert_eq!(
                f.sessions("pi"),
                [(f.sealed("pi", "first"), 7)].into_iter().collect()
            );
            assert_eq!(
                f.sessions("codex"),
                [(f.sealed("codex", "first"), 7)].into_iter().collect()
            );
            let conflicts = f
                .warnings()
                .iter()
                .filter(|w| *w == "conflicting session header")
                .count();
            assert_eq!(
                conflicts, 2,
                "one per real conflict; a same-id repeat is not one"
            );
        }

        #[test]
        fn a_conflict_appended_after_ingest_keeps_earlier_facts_and_adds_only_new_tokens() {
            let f = Fixture::new("append");
            let p = f.file("p.jsonl", &[pi_header("first"), pi_usage("a", 10)]);
            f.ingest("pi", &p);
            let before = f.db.load_usage().unwrap();
            f.append(&p, &[pi_header("second"), pi_usage("b", 5)]);
            f.ingest("pi", &p);
            let after = f.db.load_usage().unwrap();
            assert_eq!(after.len(), 2);
            assert_eq!(
                after[0].source_key, before[0].source_key,
                "earlier fact untouched"
            );
            assert_eq!(
                f.sessions("pi"),
                [(f.sealed("pi", "first"), 15)].into_iter().collect(),
                "exactly +5, same session"
            );
            assert!(f
                .warnings()
                .contains(&"conflicting session header".to_string()));
            // Replacement with a new header: new generation, new session.
            fs::write(
                &p,
                format!("{}\n{}\n", pi_header("replaced"), pi_usage("c", 7)),
            )
            .unwrap();
            f.ingest("pi", &p);
            let sessions = f.sessions("pi");
            assert_eq!(sessions.get(&f.sealed("pi", "replaced")), Some(&7));
            let generations: std::collections::BTreeSet<_> =
                f.db.load_usage()
                    .unwrap()
                    .iter()
                    .map(|u| u.generation)
                    .collect();
            assert_eq!(generations.len(), 2, "replacement is a new generation");
        }

        #[test]
        fn a_zero_usage_file_records_its_header_session_for_discovery() {
            let f = Fixture::new("zero");
            let p = f.file("p.jsonl", &[pi_header("idle-session")]);
            f.ingest("pi", &p);
            let sources = f.db.load_sources().unwrap();
            let row = sources.iter().find(|s| s.0 == "src-p.jsonl").unwrap();
            assert_eq!(
                row.4.as_deref(),
                Some(f.sealed("pi", "idle-session").as_str())
            );
            assert!(f.db.load_usage().unwrap().is_empty());
        }

        #[test]
        fn a_secret_shaped_header_id_is_sealed_and_distinct() {
            let f = Fixture::new("secret");
            let a = f.file(
                "a.jsonl",
                &[pi_header("sk-ant-ZZHDRaaaa"), pi_usage("m", 1)],
            );
            let b = f.file(
                "b.jsonl",
                &[pi_header("sk-ant-ZZHDRbbbb"), pi_usage("m", 2)],
            );
            f.ingest("pi", &a);
            f.ingest("pi", &b);
            let sessions = f.sessions("pi");
            assert_eq!(sessions.len(), 2);
            assert!(sessions
                .keys()
                .all(|k| k.contains("[redacted]") && !k.contains("ZZHDR")));
        }
    }

    fn store() -> AnalyticsStore {
        AnalyticsStore::open_in_memory("kid").unwrap()
    }

    fn write(path: &Path, body: &str) {
        let mut file = File::create(path).unwrap();
        file.write_all(body.as_bytes()).unwrap();
    }

    #[test]
    fn partial_tail_and_unknown_line_do_not_move_totals() {
        let dir = std::env::temp_dir().join(format!("dendro-stream-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("claude.jsonl");
        write(
            &path,
            "{\"type\":\"assistant\",\"sessionId\":\"s\",\"message\":{\"id\":\"m1\",\"usage\":{\"input_tokens\":2,\"output_tokens\":3,\"cache_read_input_tokens\":4,\"cache_creation_input_tokens\":5,\"cache_creation\":{\"ephemeral_5m_input_tokens\":5}}}}\nnot-json\n{\"type\":\"assistant\",\"sessionId\":\"s\",\"message\":{\"id\":\"m1\",\"usage\":{\"input_tokens\":9,\"output_tokens\":9,\"cache_read_input_tokens\":9,\"cache_creation_input_tokens\":9}}}\npartial",
        );
        let db = store();
        ingest_jsonl(&db, "c1", "claude", &path).unwrap();
        assert_eq!(db.latest_usage_totals("c1").unwrap(), (2, 3, 4, 5));
        assert_eq!(db.warning_count().unwrap(), 1);
        let state = db.source_state("c1").unwrap().unwrap();
        assert!(state.1 < fs::metadata(&path).unwrap().len() as i64);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn source_tool_names_do_not_keep_credential_families() {
        let families = [
            "sk-ZZZ1",
            "sk-ant-ZZZ2",
            "ghp_ZZZ3",
            "gho_ZZZ4",
            "github_pat_ZZZ5",
            "xai-ZZZ6",
            "AKIAZZZ7",
            "xoxb-ZZZ8",
            "xoxp-ZZZ9",
            "Bearer ZZZ0",
        ];
        let dir = std::env::temp_dir().join(format!("dendro-tool-secret-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("claude.jsonl");
        let mut body = String::new();
        for (index, token) in families.iter().enumerate() {
            body.push_str(&format!(
                "{{\"type\":\"assistant\",\"sessionId\":\"s{index}\",\"message\":{{\"id\":\"m{index}\",\"content\":[{{\"type\":\"tool_use\",\"name\":\"{token}\",\"input\":\"x\",\"is_error\":false}}]}}}}\n"
            ));
        }
        body.push_str("{\"type\":\"assistant\",\"sessionId\":\"plain\",\"message\":{\"id\":\"plain\",\"content\":[{\"type\":\"tool_use\",\"name\":\"Read\",\"input\":\"x\",\"is_error\":false}]}}\n");
        write(&path, &body);
        let db_path = dir.join("analytics.duckdb");
        let db = AnalyticsStore::open_path(&db_path, "key").unwrap();
        ingest_jsonl(&db, "c1", "claude", &path).unwrap();
        let tools = db.load_tools().unwrap();
        assert!(tools.iter().any(|tool| tool.name == "Read"));
        assert!(tools.iter().all(|tool| !tool.name.contains("ZZZ")));
        drop(db);
        let bytes = fs::read(&db_path).unwrap();
        let blob = String::from_utf8_lossy(&bytes);
        for token in families {
            assert!(!blob.contains(token), "{token}");
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn source_identity_activity_and_warning_paths_drop_credential_text() {
        let dir = std::env::temp_dir().join(format!("dendro-source-cols-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let secret_dir = dir.join("sk-ant-ZZZPATH");
        fs::create_dir_all(&secret_dir).unwrap();
        let jsonl = secret_dir.join("session.jsonl");
        write(
            &jsonl,
            concat!(
                "not-json\n",
                "{\"type\":\"assistant\",\"sessionId\":\"sk-ant-ZZZSESS\",\"message\":{\"id\":\"m1\",\"usage\":{\"input_tokens\":1,\"output_tokens\":1,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0}}}\n",
                "{\"type\":\"assistant\",\"sessionId\":\"plain-session\",\"message\":{\"id\":\"m2\",\"usage\":{\"input_tokens\":1,\"output_tokens\":1,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0}}}\n",
            ),
        );
        let sqlite_path = dir.join("openrig.sqlite");
        let conn = rusqlite::Connection::open(&sqlite_path).unwrap();
        conn.execute(
            "CREATE TABLE events (seq INTEGER, type TEXT, payload TEXT, created_at TEXT)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO events VALUES (1,'seat.activity_changed',?,?)",
            rusqlite::params![
                "{\"sessionName\":\"plain-activity-seat\",\"state\":\"sk-ant-ZZZSTATE\",\"at\":\"2026-10-09T00:00:00Z\"}",
                "2026-10-09T00:00:00Z"
            ],
        ).unwrap();
        conn.execute(
            "INSERT INTO events VALUES (2,'node.launched',?,?)",
            rusqlite::params![
                "{\"sessionName\":\"sk-ant-ZZZSEAT\"}",
                "2026-10-09T00:00:00Z"
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO events VALUES (3,'agent.session_identity',?,?)",
            rusqlite::params![
                "{\"sessionId\":\"plain-session\",\"runtime\":\"claude\",\"sessionName\":\"plain-seat\",\"rigId\":\"rig-one\"}",
                "2026-10-09T00:00:00Z"
            ],
        ).unwrap();
        // Live OpenRig writes created_at as UTC seconds without a zone (K4).
        for (seq, kind, payload, created) in [
            (
                4,
                "node.launched",
                "{\"rigId\":\"rig-one\",\"nodeId\":\"n1\",\"sessionName\":\"life-seat\"}",
                "2026-10-09 01:00:00",
            ),
            (
                5,
                "session.detached",
                "{\"rigId\":\"rig-one\",\"nodeId\":\"n1\",\"sessionName\":\"life-seat\"}",
                "2026-10-09 01:30:00",
            ),
            (
                6,
                "rig.stopped",
                "{\"rigId\":\"rig-one\"}",
                "2026-10-09 02:00:00",
            ),
            (
                7,
                "node.launched",
                "{\"rigId\":\"rig-one\",\"nodeId\":\"n1\",\"sessionName\":\"life-seat\"}",
                "2026-10-09 03:00:00",
            ),
        ] {
            conn.execute(
                "INSERT INTO events VALUES (?,?,?,?)",
                rusqlite::params![seq, kind, payload, created],
            )
            .unwrap();
        }
        drop(conn);
        let db_path = dir.join("analytics.duckdb");
        drop(AnalyticsStore::open_path(&db_path, "key").unwrap());
        let roots = [
            crate::SourceRoot {
                name: "claude".into(),
                path: secret_dir,
            },
            crate::SourceRoot {
                name: "openrig".into(),
                path: sqlite_path,
            },
        ];
        crate::ingest_staged(&db_path, &roots, false, b"source-column-test-key").unwrap();
        let db = AnalyticsStore::open_path(&db_path, "key").unwrap();
        let activity = db.load_activity().unwrap();
        assert!(activity.iter().any(|row| row.state == "unknown"));
        assert!(activity.iter().all(|row| !row.state.contains("ZZZ")));
        let seats = db.load_seat_bounds().unwrap();
        assert!(seats.iter().all(|row| !row.seat.contains("ZZZSEAT")));
        let at = |text: &str| dendro_core::parse_utc_time(text);
        let mut lives: Vec<_> = seats
            .iter()
            .filter(|row| row.seat == "life-seat")
            .map(|row| (row.created_at, row.stopped_at))
            .collect();
        lives.sort();
        assert_eq!(
            lives,
            vec![
                (at("2026-10-09T01:00:00Z"), at("2026-10-09T02:00:00Z")),
                (at("2026-10-09T03:00:00Z"), None),
            ],
            "detach is not a stop; rig stop ends the life; relaunch opens a new one"
        );
        let identities = db.load_identity().unwrap();
        assert!(identities.iter().any(|row| row.seat == "plain-seat"));
        assert!(identities
            .iter()
            .any(|row| row.native_session == "plain-session"));
        let usage = db.load_usage().unwrap();
        assert!(usage
            .iter()
            .any(|row| row.native_session == "plain-session"));
        assert!(usage
            .iter()
            .all(|row| !row.native_session.contains("ZZZSESS")));
        drop(db);
        let bytes = fs::read(&db_path).unwrap();
        let blob = String::from_utf8_lossy(&bytes);
        for token in ["ZZZPATH", "ZZZSESS", "ZZZSTATE", "ZZZSEAT"] {
            assert!(!blob.contains(token), "{token}");
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn source_message_closure_and_mission_drop_credential_text() {
        let dir = std::env::temp_dir().join(format!("dendro-source-msg-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let claude = dir.join("claude");
        fs::create_dir_all(&claude).unwrap();
        write(
            &claude.join("s.jsonl"),
            "{\"type\":\"assistant\",\"sessionId\":\"plain-session\",\"message\":{\"id\":\"sk-ant-ZZZMSG\",\"usage\":{\"input_tokens\":1,\"output_tokens\":1,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0}}}\n",
        );
        let sqlite_path = dir.join("openrig.sqlite");
        let conn = rusqlite::Connection::open(&sqlite_path).unwrap();
        conn.execute(
            "CREATE TABLE events (seq INTEGER, type TEXT, payload TEXT, created_at TEXT)",
            [],
        )
        .unwrap();
        conn.execute(
            "CREATE TABLE queue_transitions (transition_id INTEGER, qitem_id TEXT, ts TEXT, state TEXT, actor_session TEXT, closure_reason TEXT, closure_target TEXT, transition_note TEXT)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO queue_transitions VALUES (1,'plain-item','2026-10-09T00:00:00Z','handed-off','plain-seat','handed_off_to','plain-seat',?)",
            rusqlite::params!["[\"plain-mission\",\"sk-ant-ZZZMISSION\"]"],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO queue_transitions VALUES (2,'plain-item','2026-10-09T00:00:10Z','done','plain-seat','note sk-ant-ZZZREASON','plain-seat','[]')",
            [],
        )
        .unwrap();
        // Missions come from mission: tags on the item (K3), not from notes.
        conn.execute(
            "CREATE TABLE queue_items (qitem_id TEXT, destination_session TEXT, ts_updated TEXT, tags TEXT)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO queue_items VALUES ('plain-item','plain-seat','2026-10-09T00:00:10Z',?)",
            rusqlite::params![
                "[\"mission:plain-mission\",\"mission:sk-ant-ZZZMISSION\",\"slice:x\"]"
            ],
        )
        .unwrap();
        drop(conn);
        let db_path = dir.join("analytics.duckdb");
        drop(AnalyticsStore::open_path(&db_path, "key").unwrap());
        let roots = [
            crate::SourceRoot {
                name: "claude".into(),
                path: claude,
            },
            crate::SourceRoot {
                name: "openrig".into(),
                path: sqlite_path,
            },
        ];
        crate::ingest_staged(&db_path, &roots, false, b"source-column-test-key").unwrap();
        let db = AnalyticsStore::open_path(&db_path, "key").unwrap();
        let usage = db.load_usage().unwrap();
        assert!(usage.iter().all(|row| !row
            .message_id
            .as_deref()
            .unwrap_or("")
            .contains("ZZZMSG")));
        let queue = db.load_queue().unwrap();
        assert!(queue
            .iter()
            .any(|row| row.closure_reason.as_deref() == Some("handed_off_to")));
        assert!(queue.iter().all(|row| !row
            .closure_reason
            .as_deref()
            .unwrap_or("")
            .contains("ZZZREASON")));
        assert!(
            queue.iter().all(|row| row.missions.is_empty()),
            "notes are not missions"
        );
        let snapshots = db.load_destination_snapshots().unwrap();
        let missions: Vec<&String> = snapshots.iter().flat_map(|(_, s)| &s.missions).collect();
        assert!(missions.iter().any(|mission| *mission == "plain-mission"));
        assert!(missions
            .iter()
            .all(|mission| !mission.contains("ZZZMISSION")));
        assert_eq!(
            missions.len(),
            2,
            "only mission: tags, the secret one sealed"
        );
        assert!(queue.iter().all(|row| row.chain_id.is_none()));
        assert_eq!(db.skill_count().unwrap(), 0);
        drop(db);
        let bytes = fs::read(&db_path).unwrap();
        let blob = String::from_utf8_lossy(&bytes);
        for token in ["ZZZMSG", "ZZZREASON", "ZZZMISSION"] {
            assert!(!blob.contains(token), "{token}");
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn source_identity_follows_path_not_directory_order() {
        let dir = std::env::temp_dir().join(format!("dendro-source-id-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let root = dir.join("codex");
        fs::create_dir_all(&root).unwrap();
        let line = |input: i64| {
            format!(
                "{{\"type\":\"event_msg\",\"payload\":{{\"type\":\"token_count\",\"info\":{{\"total_token_usage\":{{\"input_tokens\":{input},\"cached_input_tokens\":0,\"cache_write_input_tokens\":0,\"output_tokens\":0,\"reasoning_output_tokens\":0,\"total_tokens\":{input}}}}}}}}}\n"
            )
        };
        write(&root.join("a.jsonl"), &line(10));
        write(&root.join("b.jsonl"), &line(20));
        let db_path = dir.join("analytics.duckdb");
        drop(AnalyticsStore::open_path(&db_path, "key").unwrap());
        let roots = [crate::SourceRoot {
            name: "codex".into(),
            path: root.clone(),
        }];
        let ingest =
            || crate::ingest_staged(&db_path, &roots, false, b"source-column-test-key").unwrap();
        let id = |name: &str| {
            crate::ingest::stable_source_id(
                "codex",
                &root,
                &root.join(name),
                b"source-column-test-key",
            )
        };
        ingest();
        let db = AnalyticsStore::open_path(&db_path, "key").unwrap();
        let a = db.source_state(&id("a.jsonl")).unwrap().unwrap();
        let b = db.source_state(&id("b.jsonl")).unwrap().unwrap();
        assert!(db.source_state("codex:0").unwrap().is_none());
        assert_ne!(id("a.jsonl"), id("b.jsonl"));
        drop(db);
        let mut extra = fs::OpenOptions::new()
            .append(true)
            .open(root.join("b.jsonl"))
            .unwrap();
        extra.write_all(line(30).as_bytes()).unwrap();
        ingest();
        let db = AnalyticsStore::open_path(&db_path, "key").unwrap();
        let b_after = db.source_state(&id("b.jsonl")).unwrap().unwrap();
        assert_eq!(b_after.0, b.0);
        assert!(b_after.1 > b.1);
        assert_eq!(db.source_state(&id("a.jsonl")).unwrap().unwrap().0, a.0);
        drop(db);
        fs::remove_file(root.join("a.jsonl")).unwrap();
        write(&root.join("c.jsonl"), &line(4));
        ingest();
        let db = AnalyticsStore::open_path(&db_path, "key").unwrap();
        assert!(db.source_state(&id("c.jsonl")).unwrap().is_some());
        assert!(db.source_state(&id("a.jsonl")).unwrap().is_some());
        assert_ne!(
            db.source_state(&id("c.jsonl")).unwrap().unwrap().2,
            db.source_state(&id("a.jsonl")).unwrap().unwrap().2
        );
        drop(db);
        write(&root.join("b.jsonl"), &line(1));
        ingest();
        let db = AnalyticsStore::open_path(&db_path, "key").unwrap();
        let replaced = db.source_state(&id("b.jsonl")).unwrap().unwrap();
        assert!(replaced.0 > b.0);
        drop(db);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn same_relative_name_under_two_roots_does_not_share_a_watermark() {
        let dir = std::env::temp_dir().join(format!("dendro-two-roots-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let left = dir.join("left");
        let right = dir.join("right");
        fs::create_dir_all(&left).unwrap();
        fs::create_dir_all(&right).unwrap();
        let line = |input: i64| {
            format!(
                "{{\"type\":\"event_msg\",\"payload\":{{\"type\":\"token_count\",\"info\":{{\"total_token_usage\":{{\"input_tokens\":{input},\"cached_input_tokens\":0,\"cache_write_input_tokens\":0,\"output_tokens\":0,\"reasoning_output_tokens\":0,\"total_tokens\":{input}}}}}}}}}\n"
            )
        };
        write(&left.join("x.jsonl"), &line(10));
        write(&right.join("x.jsonl"), &line(40));
        let db_path = dir.join("analytics.duckdb");
        let left_id = crate::ingest::stable_source_id(
            "codex",
            &left,
            &left.join("x.jsonl"),
            b"source-column-test-key",
        );
        let right_id = crate::ingest::stable_source_id(
            "codex",
            &right,
            &right.join("x.jsonl"),
            b"source-column-test-key",
        );
        assert_ne!(left_id, right_id);
        let secret_left = dir.join("sk-ant-ZZZONE");
        let secret_right = dir.join("sk-ant-ZZZTWO");
        let secret_left_id = crate::ingest::stable_source_id(
            "codex",
            &secret_left,
            &secret_left.join("x.jsonl"),
            b"source-column-test-key",
        );
        let secret_right_id = crate::ingest::stable_source_id(
            "codex",
            &secret_right,
            &secret_right.join("x.jsonl"),
            b"source-column-test-key",
        );
        assert_ne!(secret_left_id, secret_right_id);
        assert!(!secret_left_id.contains("ZZZONE"));
        assert!(!secret_right_id.contains("ZZZTWO"));
        drop(AnalyticsStore::open_path(&db_path, "key").unwrap());
        let roots = [
            crate::SourceRoot {
                name: "codex".into(),
                path: left.clone(),
            },
            crate::SourceRoot {
                name: "codex".into(),
                path: right.clone(),
            },
        ];
        crate::ingest_staged(&db_path, &roots, false, b"source-column-test-key").unwrap();
        let db = AnalyticsStore::open_path(&db_path, "key").unwrap();
        let left_state = db.source_state(&left_id).unwrap().unwrap();
        let right_state = db.source_state(&right_id).unwrap().unwrap();
        assert_ne!(left_state.2, right_state.2);
        drop(db);
        write(&left.join("x.jsonl"), &line(1));
        crate::ingest_staged(&db_path, &roots, false, b"source-column-test-key").unwrap();
        let db = AnalyticsStore::open_path(&db_path, "key").unwrap();
        assert!(db.source_state(&left_id).unwrap().unwrap().0 > left_state.0);
        assert_eq!(
            db.source_state(&right_id).unwrap().unwrap().0,
            right_state.0
        );
        drop(db);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn source_hash_counter_snapshot_and_warning_reason_are_not_raw_secrets() {
        let dir = std::env::temp_dir().join(format!("dendro-source-rest-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let codex = dir.join("codex");
        fs::create_dir_all(&codex).unwrap();
        write(
            &codex.join("rollout.jsonl"),
            "not-json sk-ant-ZZZLINE\n{\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"total_token_usage\":{\"input_tokens\":10,\"cached_input_tokens\":0,\"cache_write_input_tokens\":0,\"output_tokens\":0,\"reasoning_output_tokens\":0,\"total_tokens\":10}}}}\n",
        );
        let sqlite_path = dir.join("openrig.sqlite");
        let conn = rusqlite::Connection::open(&sqlite_path).unwrap();
        conn.execute(
            "CREATE TABLE events (seq INTEGER, type TEXT, payload TEXT, created_at TEXT)",
            [],
        )
        .unwrap();
        conn.execute(
            "CREATE TABLE queue_items (qitem_id TEXT, destination_session TEXT, ts_updated TEXT)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO queue_items VALUES ('plain-item','sk-ant-ZZZDEST','2026-10-09T00:00:00Z')",
            [],
        )
        .unwrap();
        drop(conn);
        let db_path = dir.join("analytics.duckdb");
        let source_id = crate::ingest::stable_source_id(
            "codex",
            &codex,
            &codex.join("rollout.jsonl"),
            b"source-column-test-key",
        );
        drop(AnalyticsStore::open_path(&db_path, "key").unwrap());
        let roots = [
            crate::SourceRoot {
                name: "codex".into(),
                path: codex,
            },
            crate::SourceRoot {
                name: "openrig".into(),
                path: sqlite_path,
            },
        ];
        crate::ingest_staged(&db_path, &roots, false, b"source-column-test-key").unwrap();
        let db = AnalyticsStore::open_path(&db_path, "key").unwrap();
        let state = db.source_state(&source_id).unwrap().unwrap();
        assert_eq!(state.2.len(), 64);
        assert!(state.2.chars().all(|ch| ch.is_ascii_hexdigit()));
        assert!(!state.2.contains("ZZZ"));
        let snapshots = db.load_destination_snapshots().unwrap();
        assert!(snapshots
            .iter()
            .all(|row| !row.1.destination.contains("ZZZDEST")));
        assert!(!snapshots.is_empty());
        let reasons = db.warning_reasons().unwrap();
        assert!(reasons.iter().any(|reason| reason == "malformed-json"));
        assert!(reasons.iter().all(|reason| !reason.contains("ZZZLINE")));
        drop(db);
        let bytes = fs::read(&db_path).unwrap();
        let blob = String::from_utf8_lossy(&bytes);
        for token in ["ZZZLINE", "ZZZDEST"] {
            assert!(!blob.contains(token), "{token}");
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn codex_uses_only_cumulative_token_count_and_reset() {
        let dir = std::env::temp_dir().join(format!("dendro-codex-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rollout.jsonl");
        write(
            &path,
            concat!(
                "{\"type\":\"token_usage_record\",\"payload\":{\"usage\":{\"input_tokens\":999}}}\n",
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"total_token_usage\":{\"input_tokens\":10,\"cached_input_tokens\":4,\"cache_write_input_tokens\":1,\"output_tokens\":2,\"reasoning_output_tokens\":1,\"total_tokens\":12}}}}\n",
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"total_token_usage\":{\"input_tokens\":30,\"cached_input_tokens\":10,\"cache_write_input_tokens\":1,\"output_tokens\":5,\"reasoning_output_tokens\":2,\"total_tokens\":35}}}}\n",
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"total_token_usage\":{\"input_tokens\":5,\"cached_input_tokens\":0,\"cache_write_input_tokens\":0,\"output_tokens\":1,\"reasoning_output_tokens\":0,\"total_tokens\":6}}}}\n",
            ),
        );
        let db = store();
        ingest_jsonl(&db, "x", "codex", &path).unwrap();
        // first exclusive 10-4-1=5; second delta input 20 cached 6 write 0 => 14; reset contributes 5.
        assert_eq!(db.latest_usage_totals("x").unwrap().0, 5 + 14 + 5);
        assert_eq!(db.latest_usage_totals("x").unwrap().1, 2 + 3 + 1);
        let rows = db.load_usage().unwrap();
        assert_eq!(rows[0].predecessor_reason.as_deref(), Some("baseline"));
        assert!(rows[0].prior_input_tokens.is_none());
        assert_eq!(rows[1].predecessor_reason.as_deref(), Some("prior"));
        assert_eq!(rows[1].prior_input_tokens, Some(10));
        assert_eq!(
            rows[1].predecessor_source_key.as_deref(),
            Some(rows[0].source_key.as_str())
        );
        assert_eq!(rows[2].predecessor_reason.as_deref(), Some("reset"));
        assert!(rows[2].prior_input_tokens.is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn codex_append_keeps_same_generation_prior_and_replacement_resets() {
        let dir = std::env::temp_dir().join(format!("dendro-codex-append-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rollout.jsonl");
        write(
            &path,
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"total_token_usage\":{\"input_tokens\":10,\"cached_input_tokens\":0,\"cache_write_input_tokens\":0,\"output_tokens\":0,\"reasoning_output_tokens\":0,\"total_tokens\":10}}}}\n",
        );
        let db = store();
        ingest_jsonl(&db, "x", "codex", &path).unwrap();
        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(b"{\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"total_token_usage\":{\"input_tokens\":30,\"cached_input_tokens\":0,\"cache_write_input_tokens\":0,\"output_tokens\":0,\"reasoning_output_tokens\":0,\"total_tokens\":30}}}}\n").unwrap();
        ingest_jsonl(&db, "x", "codex", &path).unwrap();
        let rows = db.load_usage().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].predecessor_reason.as_deref(), Some("prior"));
        assert_eq!(rows[1].prior_input_tokens, Some(10));
        assert_eq!(rows[1].generation, rows[0].generation);
        assert_eq!(
            rows[1].predecessor_source_key.as_deref(),
            Some(rows[0].source_key.as_str())
        );
        write(
            &path,
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"total_token_usage\":{\"input_tokens\":4,\"cached_input_tokens\":0,\"cache_write_input_tokens\":0,\"output_tokens\":0,\"reasoning_output_tokens\":0,\"total_tokens\":4}}}}\n",
        );
        ingest_jsonl(&db, "x", "codex", &path).unwrap();
        let rows = db.load_usage().unwrap();
        let newest = rows.iter().max_by_key(|row| row.generation).unwrap();
        assert!(newest.generation > rows[0].generation);
        assert_eq!(newest.predecessor_reason.as_deref(), Some("reset"));
        assert!(newest.prior_input_tokens.is_none());
        assert_eq!(newest.input_uncached, 4);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn append_adds_only_new_lines_and_replacement_drops_old_generation() {
        let dir = std::env::temp_dir().join(format!("dendro-append-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("pi.jsonl");
        write(
            &path,
            "{\"type\":\"message\",\"id\":\"a\",\"message\":{\"role\":\"assistant\",\"usage\":{\"input\":1,\"output\":2,\"cacheRead\":3,\"cacheWrite\":4,\"reasoning\":9,\"totalTokens\":99,\"cost\":{\"total\":1.5}}}}\n",
        );
        let db = store();
        ingest_jsonl(&db, "p", "pi", &path).unwrap();
        assert_eq!(db.latest_usage_totals("p").unwrap(), (1, 2, 3, 4));
        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(b"{\"type\":\"message\",\"id\":\"b\",\"message\":{\"role\":\"assistant\",\"usage\":{\"input\":10,\"output\":0,\"cacheRead\":0,\"cacheWrite\":0,\"reasoning\":0,\"totalTokens\":10,\"cost\":{\"total\":0}}}}\n").unwrap();
        ingest_jsonl(&db, "p", "pi", &path).unwrap();
        assert_eq!(db.latest_usage_totals("p").unwrap().0, 11);
        write(
            &path,
            "{\"type\":\"message\",\"id\":\"c\",\"message\":{\"role\":\"assistant\",\"usage\":{\"input\":7,\"output\":0,\"cacheRead\":0,\"cacheWrite\":0,\"reasoning\":0,\"totalTokens\":7,\"cost\":{\"total\":0}}}}\n",
        );
        ingest_jsonl(&db, "p", "pi", &path).unwrap();
        assert_eq!(db.latest_usage_totals("p").unwrap().0, 7);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn frozen_sqlite_copy_preserves_source_hashes_and_reads_wal() {
        let dir = std::env::temp_dir().join(format!("dendro-frozen-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("src.sqlite");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.pragma_update(None, "journal_mode", "WAL").unwrap();
        conn.pragma_update(None, "wal_autocheckpoint", 0).unwrap();
        conn.execute("CREATE TABLE t(id INTEGER)", []).unwrap();
        conn.execute("INSERT INTO t VALUES (1)", []).unwrap();
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
            .unwrap();
        conn.execute("INSERT INTO t VALUES (2)", []).unwrap();
        let snap = dir.join("snap");
        let copied = crate::snapshot::snapshot_sqlite(&db_path, &snap, &dendro_core::Cancel::new())
            .unwrap()
            .path;
        let before = ["", "-wal", "-shm"].map(|suffix| {
            let path = PathBuf::from(format!("{}{suffix}", db_path.display()));
            (
                suffix.to_string(),
                path.exists().then(|| file_sha256(&path).unwrap()),
            )
        });
        let uri = format!("file:{}?mode=ro", copied.display());
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI;
        let ro = rusqlite::Connection::open_with_flags(uri, flags).unwrap();
        let mut ids = Vec::new();
        let mut stmt = ro.prepare("SELECT id FROM t ORDER BY id").unwrap();
        let rows = stmt.query_map([], |row| row.get::<_, i64>(0)).unwrap();
        for id in rows {
            ids.push(id.unwrap());
        }
        let after = ["", "-wal", "-shm"].map(|suffix| {
            let path = PathBuf::from(format!("{}{suffix}", db_path.display()));
            (
                suffix.to_string(),
                path.exists().then(|| file_sha256(&path).unwrap()),
            )
        });
        assert_eq!(before, after);
        assert_eq!(ids, vec![1, 2]);
        drop(conn);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore = "reads this machine's local logs; run explicitly for AC1"]
    fn ac1_real_logs_finish_within_five_minutes() {
        let started = std::time::Instant::now();
        let home = std::env::var("HOME").unwrap();
        let db = store();
        let mut found = Vec::new();
        let roots = [
            ("claude", PathBuf::from(&home).join(".claude/projects")),
            ("codex", PathBuf::from(&home).join(".codex/sessions")),
            ("pi", PathBuf::from(&home).join(".openrig/state/pi")),
        ];
        for (harness, root) in roots {
            if !root.exists() {
                found.push(format!("{harness} missing"));
                continue;
            }
            let mut files = 0usize;
            for entry in walk_jsonl(&root) {
                files += 1;
                ingest_jsonl(&db, &format!("{harness}:{files}"), harness, &entry).unwrap();
            }
            found.push(format!("{harness} files={files}"));
        }
        let sqlite = PathBuf::from(&home).join(".openrig/openrig.sqlite");
        if sqlite.exists() {
            let snap = std::env::temp_dir().join(format!("dendro-ac1-snap-{}", std::process::id()));
            let copied =
                crate::snapshot::snapshot_sqlite(&sqlite, &snap, &dendro_core::Cancel::new())
                    .unwrap()
                    .path;
            let uri = format!("file:{}?mode=ro", copied.display());
            let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI;
            let conn = rusqlite::Connection::open_with_flags(uri, flags).unwrap();
            let n: i64 = conn
                .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
                .unwrap();
            found.push(format!("openrig events={n}"));
            let _ = fs::remove_dir_all(&snap);
        } else {
            found.push("openrig missing".into());
        }
        for label in ["agy", "grok"] {
            found.push(format!("{label} not-required"));
        }
        let elapsed = started.elapsed();
        eprintln!("AC1 {} ms {:?}", elapsed.as_millis(), found);
        assert!(elapsed.as_secs() <= 300, "AC1 exceeded 300s: {elapsed:?}");
    }
}

pub fn walk_jsonl(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let meta = match fs::symlink_metadata(&path) {
                Ok(meta) => meta,
                Err(_) => continue,
            };
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.file_type().is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
                out.push(path);
            }
        }
    }
    out
}
