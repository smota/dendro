use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use dendro_core::{AnalyticsStore, CodexCounters, UsageFact, WarningNote};
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
}

pub fn ingest_jsonl(
    store: &AnalyticsStore,
    source_id: &str,
    harness: &str,
    path: &Path,
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
    let mut counters = if generation_was_reset(prior, generation) {
        CodexCounters::default()
    } else {
        store.codex_counters(source_id, generation)?
    };
    let mut usages = Vec::new();
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
                    let key = format!("{}\n{}\n{id}", harness, bucket.native_session);
                    if !seen_messages.insert(key)
                        || store.message_seen(harness, &bucket.native_session, id, generation)?
                    {
                        continue;
                    }
                }
                usages.push(UsageFact {
                    source_key: format!("{source_id}:{generation}:{line_no}"),
                    harness: harness.to_string(),
                    native_session: bucket.native_session,
                    message_id: bucket.message_id,
                    ts: None,
                    seq: line_no,
                    input_uncached: bucket.input_uncached,
                    output: bucket.output,
                    cache_read: bucket.cache_read,
                    cache_write: bucket.cache_write,
                    reasoning: bucket.reasoning,
                    currency_total: bucket.currency_total,
                    generation,
                });
            }
            Err(reason) => warnings.push(WarningNote {
                path: path.display().to_string(),
                line: Some(line_no),
                reason,
            }),
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
        &[],
    )?;
    for note in &warnings {
        store.record_warning(note)?;
    }
    Ok(())
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
    }))
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
}
