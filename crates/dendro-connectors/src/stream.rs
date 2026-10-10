use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use dendro_core::{
    redact, tool_fingerprint, AnalyticsStore, CodexCounters, ToolFact, UsageFact, WarningNote,
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
    ingest_jsonl_with_key(store, source_id, harness, path, None)
}

pub fn ingest_jsonl_with_key(
    store: &AnalyticsStore,
    source_id: &str,
    harness: &str,
    path: &Path,
    tool_key: Option<&[u8]>,
) -> Result<()> {
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
    let mut first_codex_in_this_call = true;
    let mut usages = Vec::new();
    let mut tools = Vec::new();
    let mut warnings = Vec::new();
    let mut seen_messages = HashSet::new();
    for (line_no, bytes) in pending {
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
        match parse_line(harness, &text, &mut counters) {
            Ok(None) => {}
            Ok(Some(bucket)) => {
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
            Err(reason) => warnings.push(WarningNote {
                path: path.display().to_string(),
                line: Some(line_no),
                reason,
            }),
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
    for note in &warnings {
        store.record_warning(note)?;
    }
    Ok(())
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

fn parse_line(
    harness: &str,
    line: &str,
    counters: &mut CodexCounters,
) -> Result<Option<Buckets>, String> {
    let value: Value = serde_json::from_str(line).map_err(|_| "malformed-json".to_string())?;
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
            native_session: redact(session),
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

pub fn copy_sqlite_trio(src: &Path, dest_dir: &Path) -> Result<PathBuf> {
    fs::create_dir_all(dest_dir)?;
    let name = src
        .file_name()
        .ok_or_else(|| anyhow!("sqlite path has no file name"))?;
    let dest = dest_dir.join(name);
    fs::copy(src, &dest)?;
    let src_text = src.display().to_string();
    for suffix in ["-wal", "-shm"] {
        let extra = PathBuf::from(format!("{src_text}{suffix}"));
        if extra.exists() {
            let extra_name = extra.file_name().unwrap();
            fs::copy(&extra, dest_dir.join(extra_name))?;
        }
    }
    Ok(dest)
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
        assert!(queue.iter().any(|row| row
            .missions
            .iter()
            .any(|mission| mission == "plain-mission")));
        assert!(queue.iter().all(|row| row
            .missions
            .iter()
            .all(|mission| !mission.contains("ZZZMISSION"))));
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
        let copied = copy_sqlite_trio(&db_path, &snap).unwrap();
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
            let copied = copy_sqlite_trio(&sqlite, &snap).unwrap();
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
