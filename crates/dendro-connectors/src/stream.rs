use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use anyhow::Result;
use dendro_core::file_metadata::{observe_path, FileMetadata};
use dendro_core::{
    classify_tool_invocation, tool_fingerprint, AnalyticsStore, CodexCounters, MessageFact,
    SkillFact, ToolCategory, ToolFact, UsageFact, WarningNote,
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
    /// Set when one line carries its own model (Grok `modelUsage` keys).
    /// Empty means the file-level attribution still applies.
    model: Option<String>,
}

#[derive(Default)]
struct UsageAttribution {
    model: Option<String>,
    effort: Option<String>,
}

impl UsageAttribution {
    /// Prefix records are observed too, restoring context after a watermark
    /// resume without carrying metadata from a replaced file generation.
    fn observe(&mut self, harness: &str, value: &Value) {
        let kind = value.get("type").and_then(Value::as_str);
        match harness {
            "codex" if matches!(kind, Some("session_meta" | "turn_context" | "event_msg")) => {
                update_attribution(&mut self.model, value, &["/payload/model", "/model"]);
                update_attribution(
                    &mut self.effort,
                    value,
                    &[
                        "/payload/effort",
                        "/payload/reasoning_effort",
                        "/payload/collaboration_mode/settings/reasoning_effort",
                        "/effort",
                        "/reasoning_effort",
                    ],
                );
            }
            "pi" => match kind {
                Some("model_change") => {
                    update_attribution(&mut self.model, value, &["/modelId", "/model"]);
                }
                Some("thinking_level_change") => {
                    update_attribution(&mut self.effort, value, &["/thinkingLevel"]);
                }
                Some("message")
                    if value.pointer("/message/role").and_then(Value::as_str)
                        == Some("assistant") =>
                {
                    update_attribution(&mut self.model, value, &["/message/model"]);
                    update_attribution(&mut self.effort, value, &["/message/thinkingLevel"]);
                }
                _ => {}
            },
            "claude" if kind == Some("assistant") => {
                // Claude records carry their own model; never borrow one from
                // another message (a file may also contain several sessions).
                *self = Self::default();
                update_attribution(&mut self.model, value, &["/message/model"]);
                update_attribution(
                    &mut self.effort,
                    value,
                    &["/message/effort", "/message/reasoning_effort", "/effort"],
                );
            }
            _ => {}
        }
    }
}

fn update_attribution(target: &mut Option<String>, value: &Value, paths: &[&str]) {
    if let Some(field) = paths.iter().find_map(|path| value.pointer(path)) {
        // An explicitly invalid or null value clears stale context.
        *target = field
            .as_str()
            .filter(|s| safe_attribution_token(s))
            .map(str::to_string);
    }
}

fn safe_attribution_token(token: &str) -> bool {
    !token.is_empty()
        && token.len() <= 64
        && token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_'))
        && !dendro_core::contains_secret(token)
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
    pub coverage: crate::sdk::Coverage,
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
    ingest_jsonl_with_key_checking(store, source_id, harness, path, tool_key, || Ok(()))
}

/// Read a sealed prefix through the same read-only handle used by the parser.
/// Verification happens both before parsing and against the bytes consumed.
pub fn ingest_jsonl_with_cutoff(
    store: &AnalyticsStore,
    source_id: &str,
    harness: &str,
    path: &Path,
    tool_key: Option<&[u8]>,
    cutoff: Option<&dendro_core::FileCutoff>,
) -> Result<FileStats> {
    ingest_jsonl_bounded(store, source_id, harness, path, tool_key, cutoff, || Ok(()))
}

// Tests substitute names at the real read boundary; production has no fault switch.
fn ingest_jsonl_with_key_checking(
    store: &AnalyticsStore,
    source_id: &str,
    harness: &str,
    path: &Path,
    tool_key: Option<&[u8]>,
    after_header: impl FnOnce() -> Result<()>,
) -> Result<FileStats> {
    ingest_jsonl_bounded(
        store,
        source_id,
        harness,
        path,
        tool_key,
        None,
        after_header,
    )
}

fn ingest_jsonl_bounded(
    store: &AnalyticsStore,
    source_id: &str,
    harness: &str,
    path: &Path,
    tool_key: Option<&[u8]>,
    cutoff: Option<&dendro_core::FileCutoff>,
    after_header: impl FnOnce() -> Result<()>,
) -> Result<FileStats> {
    let limits = crate::capture_registry_v1::Registry::reader_limits(harness);
    ingest_jsonl_limited(
        store,
        source_id,
        harness,
        path,
        tool_key,
        cutoff,
        after_header,
        &limits,
    )
}

#[allow(clippy::too_many_arguments)]
fn ingest_jsonl_limited(
    store: &AnalyticsStore,
    source_id: &str,
    harness: &str,
    path: &Path,
    tool_key: Option<&[u8]>,
    cutoff: Option<&dendro_core::FileCutoff>,
    after_header: impl FnOnce() -> Result<()>,
    limits: &crate::sdk::ReadLimits,
) -> Result<FileStats> {
    let mut header = Some(after_header);
    let read = ingest_jsonl_file(
        store,
        source_id,
        harness,
        path,
        tool_key,
        cutoff,
        &mut header,
        limits,
    )?;
    let total = FileStats {
        coverage: read.coverage,
        records_read: read.records_read,
        usage_records: read.usage_records,
    };
    // Only partial coverage is a warning; a complete read is not a defect. An
    // idle re-run that reads no new bytes adds no row for the same defect.
    if total.coverage.partial && read.advanced {
        store.record_warning(&WarningNote {
            path: path.display().to_string(),
            line: None,
            reason: total.coverage.summary(),
        })?;
    }
    Ok(total)
}

struct FileOutcome {
    coverage: crate::sdk::Coverage,
    records_read: u64,
    usage_records: u64,
    /// The watermark moved: this call consumed bytes beyond the stored one.
    advanced: bool,
}

enum CappedLine {
    Eof,
    /// Unterminated, or an oversize line that never found a newline. Not committed.
    Partial,
    Line(Vec<u8>),
    /// A newline-terminated line longer than `limit`. The body was not kept.
    Oversize {
        len: u64,
    },
}

/// Read one JSONL line without retaining more than `limit` bytes of it.
/// An oversize line is hashed and counted, then discarded up to its newline.
fn read_capped_line(
    reader: &mut impl BufRead,
    limit: u64,
    hasher: &mut Sha256,
) -> Result<CappedLine> {
    let limit = usize::try_from(limit).unwrap_or(usize::MAX);
    if limit == 0 {
        anyhow::bail!("record limit is zero");
    }
    let checkpoint = hasher.clone();
    let mut buf = Vec::new();
    let mut oversize = false;
    let mut total = 0u64;
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            if !oversize && buf.is_empty() {
                return Ok(CappedLine::Eof);
            }
            *hasher = checkpoint;
            return Ok(CappedLine::Partial);
        }
        if let Some(i) = available.iter().position(|byte| *byte == b'\n') {
            let take = i + 1;
            if !oversize && buf.len().saturating_add(take) <= limit {
                buf.extend_from_slice(&available[..take]);
                hasher.update(&buf);
                reader.consume(take);
                return Ok(CappedLine::Line(buf));
            }
            if !oversize && !buf.is_empty() {
                hasher.update(&buf);
                total += buf.len() as u64;
                buf.clear();
            }
            // `available` is the reader's fixed scratch, not a copy of the line.
            hasher.update(&available[..take]);
            total += take as u64;
            reader.consume(take);
            return Ok(CappedLine::Oversize { len: total });
        }
        let n = available.len();
        if !oversize {
            let room = limit - buf.len();
            if n > room {
                buf.extend_from_slice(&available[..room]);
                hasher.update(&buf);
                total += buf.len() as u64;
                buf.clear();
                reader.consume(room);
                oversize = true;
                continue;
            }
            buf.extend_from_slice(available);
            reader.consume(n);
            continue;
        }
        hasher.update(&available[..n]);
        total += n as u64;
        reader.consume(n);
    }
}

#[allow(clippy::too_many_arguments)]
fn ingest_jsonl_file<F: FnOnce() -> Result<()>>(
    store: &AnalyticsStore,
    source_id: &str,
    harness: &str,
    path: &Path,
    tool_key: Option<&[u8]>,
    cutoff: Option<&dendro_core::FileCutoff>,
    after_header: &mut Option<F>,
    limits: &crate::sdk::ReadLimits,
) -> Result<FileOutcome> {
    let mismatch = |reason: &str| {
        anyhow::anyhow!(
            "mismatch: source {} {}",
            dendro_core::redact(
                cutoff
                    .map(|entry| entry.source_id.as_str())
                    .unwrap_or(source_id)
            ),
            reason
        )
    };
    let mut file = File::open(path).map_err(|error| {
        if cutoff.is_some() {
            mismatch("file missing or unreadable")
        } else {
            error.into()
        }
    })?;
    if let Some(entry) = cutoff {
        if file.metadata()?.len() < entry.byte_length {
            return Err(mismatch("file shorter than byte cutoff"));
        }
        if hash_prefix(&mut file, entry.byte_length)? != entry.prefix_sha256 {
            return Err(mismatch("file prefix digest differs"));
        }
    }
    let mut held_before = FileMetadata::from_file(&file);
    let mut named_before = observe_path(path);
    let byte_limit = cutoff
        .map(|entry| entry.byte_length)
        .unwrap_or(file.metadata()?.len());
    let file_len = i64::try_from(byte_limit)?;
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
    let prefix_end = offset;
    let mut hasher = Sha256::new();
    // The bounded header read uses the same opened file as the streaming parser.
    // A pathname replacement cannot silently supply a different session header.
    let file_session = match harness {
        "pi" | "codex" | "grok" => Some(file_session(
            harness,
            &mut (&mut file).take(byte_limit),
            source_id,
        )?),
        _ => None,
    };
    if let Some(after_header) = after_header.take() {
        after_header()?;
    }
    file.seek(SeekFrom::Start(0))?;
    let mut reader = BufReader::with_capacity(64 * 1024, file.take(byte_limit));
    let mut line_no = 0i64;
    let mut consumed = 0i64;
    let reset_generation = generation_was_reset(prior, generation);
    let partial_reason = format!("coverage-partial generation={generation}");
    let prior_partial =
        !reset_generation && store.has_warning(&path.display().to_string(), &partial_reason)?;
    let mut counters = if reset_generation {
        CodexCounters::default()
    } else {
        store.codex_counters(source_id, generation)?
    };
    let start_offset = prefix_end;
    let mut warnings = Vec::new();
    // K10: Pi and Codex sessions are named by the id on line 1 of the file.
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
    let mut usage_models = Vec::new();
    let mut usage_attribution = UsageAttribution::default();
    let mut tools = Vec::new();
    let mut messages: Vec<MessageFact> = Vec::new();
    let mut session_edges: Vec<dendro_core::SessionEdgeFact> = Vec::new();
    let mut security_facts: Vec<dendro_core::SecurityFact> = Vec::new();
    let mut security_dedupe = crate::security_facts::PolicyDedupe::default();
    security_dedupe.seed_from(&store.load_security_facts()?, source_id, generation);
    let mut open_agents = crate::lineage::OpenAgents::default();
    let mut tool_categories: Vec<(String, ToolCategory)> = Vec::new();
    let mut seen_messages = HashSet::new();
    let mut unknown_types = HashSet::new();
    let mut records_read = 0u64;
    let mut coverage = crate::sdk::Coverage::default();
    let mut coverage_defect = prior_partial;
    let mut reader_complete = true;
    let mut source_session = file_session
        .as_ref()
        .map(|session| seal_session(tool_key, harness, &session.raw));
    let mut codex_ordinal = 0i64;
    let mut claude_sessions = HashSet::new();

    // Claude, Pi, and Grok tool calls are read in their own pass over every
    // complete line of this generation, so usage dedupe never drops a call
    // (#35, Q5). Codex tool calls are not read in this version (Q6, #40):
    // each Codex file carries one warning saying so, kept once across reruns.
    if harness == "codex" {
        let shown = path.display().to_string();
        if !store.has_warning(&shown, CODEX_TOOLS_NOT_READ)? {
            warnings.push(WarningNote {
                path: shown,
                line: None,
                reason: CODEX_TOOLS_NOT_READ.into(),
            });
        }
    }
    let mut tool_scan = if matches!(harness, "claude" | "pi" | "grok") {
        Some(ToolScan::new(
            harness,
            file_session.as_ref().map(|session| session.raw.as_str()),
            0,
            tool_key,
            source_id,
            generation,
        ))
    } else {
        None
    };
    if harness == "grok" {
        // Model and effort live in the sibling summary, not on usage lines.
        apply_grok_summary(
            path,
            &mut usage_attribution,
            &mut reader_complete,
            &mut warnings,
        );
    }
    let attempt_limit = limits.effective_line_attempts();
    let mut usage_records = 0u64;
    let initial_offset = offset;
    // One streaming pass: each full batch is committed and the same reader,
    // hasher and tool scan carry on from the committed position, so every
    // line is parsed once however many batches the file needs.
    let advanced = loop {
        let mut batch_capped = false;
        let mut decoded_in_batch = 0usize;
        let mut attempts_in_batch = 0usize;
        let batch_start = offset;
        loop {
            if decoded_in_batch >= limits.decoded_records_per_batch
                || attempts_in_batch >= attempt_limit
            {
                batch_capped = true;
                break;
            }
            dendro_core::RUN_CANCEL.check()?;
            let capped = read_capped_line(&mut reader, limits.encoded_bytes, &mut hasher)?;
            let buf = match capped {
                CappedLine::Eof => break,
                CappedLine::Partial => {
                    reader_complete = false;
                    coverage.partial = true;
                    coverage.held_tail = 1;
                    warnings.push(WarningNote {
                        path: path.display().to_string(),
                        line: Some(line_no + 1),
                        reason: "unterminated-tail-held".into(),
                    });
                    break;
                }
                CappedLine::Oversize { len } => {
                    consumed += i64::try_from(len)?;
                    line_no += 1;
                    let is_new = consumed > prefix_end;
                    attempts_in_batch += usize::from(is_new);
                    reader_complete = false;
                    if !is_new {
                        if let Some(scan) = tool_scan.as_mut() {
                            scan.new_from = line_no;
                        }
                    }
                    if is_new {
                        records_read += 1;
                        coverage.quarantined_oversize += 1;
                        warnings.push(WarningNote {
                            path: path.display().to_string(),
                            line: Some(line_no),
                            reason: crate::sdk::oversize_reason(limits).into(),
                        });
                    }
                    continue;
                }
                CappedLine::Line(buf) => {
                    consumed += i64::try_from(buf.len())?;
                    buf
                }
            };
            if !buf.ends_with(b"\n") {
                break;
            }
            line_no += 1;
            let is_new = consumed > prefix_end;
            // The legacy prefix is re-read on every batch; only new lines spend the caps.
            attempts_in_batch += usize::from(is_new);
            if !is_new {
                if let Some(scan) = tool_scan.as_mut() {
                    scan.new_from = line_no;
                }
            }
            let bytes = &buf[..buf.len() - 1];
            if is_new && !bytes.iter().all(|byte| byte.is_ascii_whitespace()) {
                records_read += 1;
            }
            let text = match std::str::from_utf8(bytes) {
                Ok(text) => text,
                Err(_) => {
                    reader_complete = false;
                    if is_new {
                        coverage.malformed += 1;
                        warnings.push(WarningNote {
                            path: path.display().to_string(),
                            line: Some(line_no),
                            reason: "invalid-utf8".into(),
                        });
                    }
                    continue;
                }
            };
            if text.trim().is_empty() {
                continue;
            }
            if crate::sdk::container_depth(bytes) > limits.nesting_depth {
                reader_complete = false;
                if is_new {
                    coverage.quarantined_depth += 1;
                    warnings.push(WarningNote {
                        path: path.display().to_string(),
                        line: Some(line_no),
                        reason: "nesting_depth_exceeds_limit".into(),
                    });
                }
                continue;
            }
            // Reader completion describes every complete line, including a legacy prefix.
            #[cfg(test)]
            PARSED_LINES.with(|parsed| parsed.set(parsed.get() + 1));
            let value = match serde_json::from_str::<Value>(text) {
                Ok(value) => {
                    decoded_in_batch += usize::from(is_new);
                    value
                }
                Err(_) => {
                    reader_complete = false;
                    if is_new {
                        coverage.malformed += 1;
                        warnings.push(WarningNote {
                            path: path.display().to_string(),
                            line: Some(line_no),
                            reason: "malformed-json".into(),
                        });
                    }
                    continue;
                }
            };
            if unknown_record_type(harness, &value).is_some() {
                reader_complete = false;
            }
            usage_attribution.observe(harness, &value);
            if harness == "claude" {
                if let Some(id) = value.get("sessionId").and_then(Value::as_str) {
                    claude_sessions.insert(seal_session(tool_key, harness, id));
                }
            }
            let mut codex_skills = Vec::new();
            let mut codex_categories = Vec::new();
            let tools_before = tools.len();
            if harness == "codex" && unknown_record_type(harness, &value).is_none() {
                reader_complete &= observe_codex(
                    &value,
                    text,
                    source_session.as_deref().unwrap_or(""),
                    source_id,
                    generation,
                    line_no,
                    &mut codex_ordinal,
                    tool_key,
                    &mut tools,
                    &mut codex_skills,
                    &mut codex_categories,
                )?;
            }
            if is_new {
                for skill in codex_skills {
                    store.insert_skill(&skill)?;
                }
                tool_categories.extend(codex_categories);
            }
            if !is_new {
                if let Some(scan) = tool_scan.as_mut() {
                    if unknown_record_type(harness, &value).is_none() {
                        scan.observe(line_no, text, &value);
                    }
                }
                continue;
            }
            if let Some(session) = &file_session {
                if session.from_header
                    && line_no > 1
                    && conflicting_session(harness, text, &session.raw)
                {
                    warnings.push(WarningNote {
                        path: path.display().to_string(),
                        line: Some(line_no),
                        reason: "conflicting session header".into(),
                    });
                }
            }
            if let Some(reason) = unknown_record_type(harness, &value) {
                coverage_defect = true;
                coverage.unknown_kind += 1;
                let repeat = !unknown_types.insert(reason.clone())
                    || store.has_warning(&path.display().to_string(), &reason)?;
                if !repeat {
                    warnings.push(WarningNote {
                        path: path.display().to_string(),
                        line: Some(line_no),
                        reason,
                    });
                }
                continue;
            }
            crate::security_facts::observe(
                &crate::security_facts::ObserveCtx {
                    harness,
                    source_id,
                    generation,
                    line_no,
                    tool_key,
                    file_session: file_session.as_ref().map(|session| session.raw.as_str()),
                    ts: timestamp_of(&value),
                },
                &value,
                &mut security_dedupe,
                &mut security_facts,
            );
            let mut projected_tool = tools.len() > tools_before;
            if let Some(scan) = tool_scan.as_mut() {
                let before = scan.calls.len() + scan.results.len() + scan.skills.len();
                scan.observe(line_no, text, &value);
                projected_tool |=
                    scan.calls.len() + scan.results.len() + scan.skills.len() > before;
            }
            // Safe message metadata only: role, direction, modality, source order.
            let projected_message = match crate::messages::observe(harness, &value) {
                Some(obs) => {
                    let session = file_session
                        .as_ref()
                        .map(|session| session.raw.as_str())
                        .or(obs.session.as_deref())
                        .map(|raw| seal_session(tool_key, harness, raw));
                    messages.push(MessageFact {
                        source_key: format!("{source_id}:{generation}:{line_no}:message"),
                        harness: harness.to_string(),
                        native_session: session,
                        ts: if harness == "grok" {
                            grok_timestamp(&value)
                        } else {
                            timestamp_of(&value)
                        },
                        seq: line_no,
                        role: obs.role,
                        direction: obs.direction,
                        modalities: obs.modalities,
                        grain: obs.grain,
                        generation,
                    });
                    true
                }
                None => false,
            };
            let mut projected_edge = false;
            let mut lineage_gap = false;
            match crate::lineage::observe(
                harness,
                &value,
                file_session.as_ref().map(|session| session.raw.as_str()),
                &mut open_agents,
            ) {
                crate::lineage::Hit::None => {}
                crate::lineage::Hit::Gap => lineage_gap = true,
                crate::lineage::Hit::Resume(edge) => {
                    let source_key = format!("{source_id}:lineage:{}", edge.body);
                    match store.session_edge_parent(&source_key)? {
                        Some(parent) => {
                            session_edges.push(session_edge_fact(
                                source_id, harness, generation, tool_key, &edge, &parent,
                            ));
                            projected_edge = true;
                            coverage.edges += 1;
                        }
                        None => lineage_gap = true,
                    }
                }
                crate::lineage::Hit::Edge(edge) => {
                    let parent = edge.parent_raw.clone();
                    session_edges.push(session_edge_fact(
                        source_id, harness, generation, tool_key, &edge, &parent,
                    ));
                    projected_edge = true;
                    coverage.edges += 1;
                }
            }
            match parse_value(harness, &value, &mut counters) {
                Ok((buckets, partial)) => {
                    if !partial.is_empty() {
                        reader_complete = false;
                        coverage_defect = true;
                    }
                    if projected_tool || projected_message || !buckets.is_empty() || projected_edge
                    {
                        coverage.projected += 1;
                    } else if lineage_gap {
                        coverage.lineage_not_explicit += 1;
                    } else if !partial.is_empty() {
                        coverage.unsupported_schema += 1;
                    } else {
                        coverage.not_projected += 1;
                    }
                    for reason in partial {
                        warnings.push(WarningNote {
                            path: path.display().to_string(),
                            line: Some(line_no),
                            reason,
                        });
                    }
                    let split = buckets.len() > 1;
                    for (index, mut bucket) in buckets.into_iter().enumerate() {
                        if let Some(session) = &file_session {
                            // The file's line-1 session wins over any per-line id.
                            bucket.native_session = session.raw.clone();
                        }
                        if let Some(id) = &bucket.message_id {
                            let sealed_session =
                                seal_session(tool_key, harness, &bucket.native_session);
                            let sealed_id = seal_message(tool_key, id);
                            let seen = format!("{harness}\n{sealed_session}\n{sealed_id}");
                            if !seen_messages.insert(seen)
                                || store.message_seen(
                                    harness,
                                    &sealed_session,
                                    &sealed_id,
                                    generation,
                                )?
                            {
                                continue;
                            }
                        }
                        let source_key = if split {
                            format!("{source_id}:{generation}:{line_no}:{index}")
                        } else {
                            format!("{source_id}:{generation}:{line_no}")
                        };
                        let model = bucket
                            .model
                            .clone()
                            .or_else(|| usage_attribution.model.clone());
                        usage_models.push((
                            source_key.clone(),
                            model,
                            usage_attribution.effort.clone(),
                        ));
                        usages.push(UsageFact {
                            source_key,
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
                }
                Err(reason) => {
                    reader_complete = false;
                    coverage_defect = true;
                    if projected_tool {
                        coverage.projected += 1;
                    } else {
                        coverage.malformed += 1;
                    }
                    warnings.push(WarningNote {
                        path: path.display().to_string(),
                        line: Some(line_no),
                        reason,
                    });
                }
            }
        }
        if harness == "claude" {
            // A file with several session identities has no unique source linkage.
            source_session = if claude_sessions.len() == 1 {
                claude_sessions.iter().next().cloned()
            } else {
                None
            };
        }
        coverage_defect |= coverage.unknown_kind
            + coverage.quarantined_oversize
            + coverage.quarantined_depth
            + coverage.malformed
            + coverage.unsupported_schema
            != 0;
        coverage.partial |= !reader_complete || coverage_defect;
        reader_complete &= !coverage_defect;
        // A batch that stopped early is not a finished reader: a crash before the
        // pass drains the file must not report the unread tail as read.
        let batch_complete = reader_complete && !batch_capped;
        let prefix_sha = hex::encode(hasher.clone().finalize());
        if let Some(entry) = cutoff {
            let at_cutoff = consumed as u64 == entry.byte_length;
            if consumed as u64 > entry.byte_length
                || ((at_cutoff || !batch_capped)
                    && (consumed as u64 != entry.byte_length || prefix_sha != entry.prefix_sha256))
            {
                return Err(mismatch(
                    "file prefix changed while reading or incomplete cutoff",
                ));
            }
        }
        offset = consumed;
        let mut settled = Vec::new();
        if let Some(scan) = tool_scan.as_mut() {
            let pass = scan.flush(line_no);
            tools = pass.new_calls;
            settled = pass.settled;
            tool_categories.extend(pass.categories);
            for skill in pass.skills {
                store.insert_skill(&skill)?;
            }
        }
        let held_after = FileMetadata::from_file(reader.get_ref().get_ref());
        let named_after = observe_path(path);
        let metadata = stable_ingest_metadata(
            held_before,
            named_before.metadata,
            held_after,
            named_after.metadata,
        );
        let baseline = dendro_core::FileBaselineAttempt {
            attempted_at: named_after
                .checked_at
                .unwrap_or_else(|| chrono::Utc::now().to_rfc3339()),
            metadata,
        };
        store.commit_file_batch_observed_messages(
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
            &messages,
            &settled,
            &dendro_core::ReaderState {
                complete: batch_complete,
                native_session: source_session.clone(),
            },
            &baseline,
        )?;
        store.commit_session_edges(&session_edges)?;
        store.commit_security_facts(&security_facts)?;
        store.record_tool_categories(&tool_categories)?;
        store.record_usage_models(&usage_models)?;
        for note in &warnings {
            store.record_warning(note)?;
        }
        // One row per defective generation, however many runs or batches see it.
        if coverage_defect && !store.has_warning(&path.display().to_string(), &partial_reason)? {
            store.record_warning(&WarningNote {
                path: path.display().to_string(),
                line: None,
                reason: partial_reason.clone(),
            })?;
        }
        usage_records += usages.len() as u64;
        usages.clear();
        tools.clear();
        messages.clear();
        session_edges.clear();
        security_facts.clear();
        warnings.clear();
        tool_categories.clear();
        usage_models.clear();
        if !(batch_capped && consumed != batch_start) {
            break consumed != initial_offset;
        }
        // The next batch is judged against what this one left on the file.
        held_before = FileMetadata::from_file(reader.get_ref().get_ref());
        named_before = observe_path(path);
    };
    Ok(FileOutcome {
        coverage,
        records_read,
        usage_records,
        advanced,
    })
}

/// Both held and no-follow named metadata must agree across the complete read.
/// Comparison intentionally excludes consumed offsets: a partial tail is file data.
fn stable_ingest_metadata(
    held_before: Option<FileMetadata>,
    named_before: Option<FileMetadata>,
    held_after: Option<FileMetadata>,
    named_after: Option<FileMetadata>,
) -> Option<FileMetadata> {
    let before = held_before?;
    (named_before.as_ref() == Some(&before)
        && held_after.as_ref() == Some(&before)
        && named_after.as_ref() == Some(&before))
    .then_some(before)
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
fn file_session(harness: &str, file: &mut impl Read, source_id: &str) -> Result<FileSession> {
    let mut line = Vec::new();
    BufReader::new(file.take(HEADER_LIMIT)).read_until(b'\n', &mut line)?;
    // Header identity is evidence only after its line is complete, just like
    // usage/tool records. Otherwise a zero-byte manifest could not replay it.
    let id = line
        .strip_suffix(b"\n")
        .and_then(|line| serde_json::from_slice::<Value>(line).ok())
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
        ("grok", _) => value.pointer("/params/sessionId"),
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

fn session_edge_fact(
    source_id: &str,
    harness: &str,
    generation: i64,
    tool_key: Option<&[u8]>,
    edge: &crate::lineage::ObservedEdge,
    parent_raw: &str,
) -> dendro_core::SessionEdgeFact {
    dendro_core::SessionEdgeFact {
        // `body` holds raw child/attempt ids; seal it like the session columns.
        source_key: format!(
            "{source_id}:lineage:{}",
            seal_session(tool_key, harness, &edge.body)
        ),
        source_id: source_id.to_string(),
        harness: harness.to_string(),
        parent_session: seal_session(tool_key, harness, parent_raw),
        child_session: seal_session(tool_key, harness, &edge.child_raw),
        relation: edge.relation.to_string(),
        status: edge.status.to_string(),
        observed_at: edge.observed_at,
        evidence_kind: edge.evidence_kind.to_string(),
        generation,
    }
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

fn seal_skill(key: Option<&[u8]>, raw: &str) -> String {
    match key {
        Some(key) => dendro_core::seal_identifier(key, "skill", raw),
        None => dendro_core::seal_identifier(b"", "skill", raw),
    }
}

pub(crate) fn extract_skills_from_text(text: &str) -> Vec<String> {
    let needle = "skills/";
    let mut search_from = 0;
    let mut skills = Vec::new();
    while let Some(pos) = text[search_from..].find(needle) {
        let actual_pos = search_from + pos;
        let is_valid_prefix = actual_pos == 0
            || text.as_bytes()[actual_pos - 1] == b'/'
            || text.as_bytes()[actual_pos - 1] == b'\\'
            || text.as_bytes()[actual_pos - 1] == b'"'
            || text.as_bytes()[actual_pos - 1] == b'\''
            || text.as_bytes()[actual_pos - 1].is_ascii_whitespace();
        let rest = &text[actual_pos + needle.len()..];
        if is_valid_prefix {
            if let Some((name, after)) = rest.split_once('/') {
                let matches_skill_md = after == "SKILL.md"
                    || after.starts_with("SKILL.md/")
                    || after.starts_with("SKILL.md\\")
                    || after.starts_with("SKILL.md\"")
                    || after.starts_with("SKILL.md'")
                    || after.starts_with("SKILL.md ")
                    || after.starts_with("SKILL.md\t")
                    || after.starts_with("SKILL.md\n")
                    || after.starts_with("SKILL.md\r")
                    || after.starts_with("SKILL.md;")
                    || after.starts_with("SKILL.md&")
                    || after.starts_with("SKILL.md|");
                let is_clean_name = !name.is_empty()
                    && name
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.');
                if matches_skill_md && is_clean_name && !skills.contains(&name.to_string()) {
                    skills.push(name.to_string());
                }
            }
        }
        search_from = actual_pos + needle.len();
    }
    skills
}

fn collect_skills_from_value(val: &Value, out: &mut Vec<String>) {
    match val {
        Value::String(s) => {
            if let Ok(decoded) = serde_json::from_str::<Value>(s) {
                if !matches!(decoded, Value::String(_)) {
                    collect_skills_from_value(&decoded, out);
                    return;
                }
            }
            for skill in extract_skills_from_text(s) {
                if !out.contains(&skill) {
                    out.push(skill);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_skills_from_value(item, out);
            }
        }
        Value::Object(map) => {
            for v in map.values() {
                collect_skills_from_value(v, out);
            }
        }
        _ => {}
    }
}

fn extract_skills_from_codex_call(call: &Value) -> Vec<String> {
    let mut skills = Vec::new();
    let name = call.get("name").and_then(Value::as_str).unwrap_or_default();
    let arg_val = call.get("arguments").or_else(|| call.get("input"));
    let parsed_args: Option<Value> = match arg_val {
        Some(Value::Object(_)) => arg_val.cloned(),
        Some(Value::String(s)) => serde_json::from_str(s).ok(),
        _ => None,
    };
    if matches!(name, "Skill" | "use_skill") {
        if let Some(s) = parsed_args
            .as_ref()
            .and_then(|inp| {
                inp.get("skill")
                    .or_else(|| inp.get("name"))
                    .or_else(|| inp.get("skill_name"))
            })
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            skills.push(s.to_string());
        }
    }
    if let Some(args) = arg_val {
        collect_skills_from_value(args, &mut skills);
    }
    if let Some(cmd) = call.get("command") {
        collect_skills_from_value(cmd, &mut skills);
    }
    skills
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
pub(crate) const CLAUDE_RECORD_TYPES: &[&str] = &[
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
pub(crate) const CODEX_RECORD_TYPES: &[&str] = &[
    "compacted",
    "event_msg",
    "response_item",
    "session_meta",
    "token_usage_record",
    "turn_context",
    "world_state",
];
pub(crate) const PI_RECORD_TYPES: &[&str] = &[
    "custom",
    "message",
    "model_change",
    "session",
    "session_info",
    "thinking_level_change",
];
/// Grok `params.update.sessionUpdate` values from the session format.
/// Every named kind is a record (`records_read`). Only `turn_completed`
/// becomes usage, and only `tool_call` / `tool_call_update` become tool
/// facts. Message, thought, hook, compaction, and background-task bodies
/// are counted and then dropped. `subagent_spawned` / `subagent_finished`
/// are lineage evidence. Explicit `parent_session_id` and `child_session_id`
/// become a `session_edges` spawn. `tokens_used` is not a usage fact.
pub(crate) const GROK_RECORD_TYPES: &[&str] = &[
    "agent_message_chunk",
    "agent_thought_chunk",
    "auto_compact_completed",
    "auto_compact_started",
    "background_tasks",
    "compaction_checkpoint",
    "hook_execution",
    "subagent_finished",
    "subagent_spawned",
    "tool_call",
    "tool_call_update",
    "turn_completed",
    "user_message_chunk",
];

pub(crate) const UNKNOWN_RECORD_TYPE: &str = "unknown-record-type";
pub(crate) const CODEX_TOOLS_NOT_READ: &str = "tool calls not read for codex";

/// The warning reason for a record of a type the parser does not know, or
/// None. The type is shown only when it is a plain identifier of at most 64
/// characters; storage still applies the AC10 redaction to it.
fn unknown_record_type(harness: &str, value: &Value) -> Option<String> {
    if harness == "grok" {
        return unknown_named_kind(grok_session_update(value), GROK_RECORD_TYPES);
    }
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

fn unknown_named_kind(kind: Option<&str>, known: &[&str]) -> Option<String> {
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

/// `sessionUpdate` when the envelope method is one this reader accepts.
fn grok_session_update(value: &Value) -> Option<&str> {
    let method = value.get("method").and_then(Value::as_str)?;
    if method != "session/update" && method != "_x.ai/session/update" {
        return None;
    }
    value
        .pointer("/params/update/sessionUpdate")
        .and_then(Value::as_str)
}

fn parse_value(
    harness: &str,
    value: &Value,
    counters: &mut CodexCounters,
) -> Result<(Vec<Buckets>, Vec<String>), String> {
    match harness {
        "codex" => {
            parse_codex(value, counters).map(|bucket| (bucket.into_iter().collect(), Vec::new()))
        }
        "claude" => parse_claude(value).map(|bucket| (bucket.into_iter().collect(), Vec::new())),
        "pi" => parse_pi(value).map(|bucket| (bucket.into_iter().collect(), Vec::new())),
        "grok" => parse_grok(value),
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
        model: None,
    }))
}

/// Usage stays on `sessionId`. In a sidechain file (`agent-<id>.jsonl`) that
/// id is the parent session, so the parent's per-session total includes the
/// sidechain usage. `agentId` is the child on the lineage edge only. Do not
/// re-key these rows onto `agentId`.
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
        model: None,
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
        model: None,
    }))
}

/// One usage row per model inside `turn_completed`. The parent usage object is
/// not stored again: `modelUsage` repeats it, and writing both would double
/// count. A turn with no `modelUsage` object stores the parent once.
/// `subagent_finished.tokens_used` is a lineage statistic. FORMAT.md does not
/// define parent `turn_completed.usage` as including the child, and the child
/// session file carries its own `turn_completed` rows. Do not add `tokens_used`
/// here or fleet totals would count the child twice. Claude Task tool results
/// are not usage records either; only assistant `/message/usage` is stored.
/// A `turn_completed` with no usage object is normal and stores nothing.
/// One bad `modelUsage` entry warns and is skipped; sibling models are kept.
fn parse_grok(value: &Value) -> Result<(Vec<Buckets>, Vec<String>), String> {
    if grok_session_update(value) != Some("turn_completed") {
        return Ok((Vec::new(), Vec::new()));
    }
    let Some(update) = value.pointer("/params/update") else {
        return Ok((Vec::new(), Vec::new()));
    };
    let Some(usage) = update.get("usage").filter(|usage| usage.is_object()) else {
        return Ok((Vec::new(), Vec::new()));
    };
    let session = value
        .pointer("/params/sessionId")
        .and_then(Value::as_str)
        .unwrap_or("grok-session");
    let ts = grok_timestamp(value);
    if let Some(models) = usage.get("modelUsage").and_then(Value::as_object) {
        if !models.is_empty() {
            let mut rows = Vec::new();
            let mut skipped = Vec::new();
            for (name, part) in models {
                if !part.is_object() {
                    skipped.push("invalid-model-usage".into());
                    continue;
                }
                let model = safe_attribution_token(name).then(|| name.clone());
                match grok_usage_row(part, session, ts, model) {
                    Ok(row) => rows.push(row),
                    Err(reason) => skipped.push(reason),
                }
            }
            return Ok((rows, skipped));
        }
    }
    grok_usage_row(usage, session, ts, None).map(|row| (vec![row], Vec::new()))
}

/// `inputTokens` is the prompt total: sampled turns have
/// `totalTokens == inputTokens + outputTokens`, so cache is not an extra
/// addend. That matches Codex `input_tokens` (cache included) and not Claude
/// `input_tokens` (already exclusive of `cache_read_input_tokens`). Exclusive
/// uncached is `inputTokens - cachedReadTokens - cacheCreationTokens`, and the
/// cache columns are stored beside it. Copying Claude's assignment would add
/// `cachedReadTokens` a second time whenever the read fits inside input.
/// FORMAT.md still marks that inclusion unverified; a negative remainder is
/// `negative-exclusive-input` and the line is warned, not stored, so cache is
/// never added on top of a total that cannot hold it. Checked on 523 local
/// `turn_completed` records: `cacheCreationTokens` is never greater than 0,
/// `totalTokens == inputTokens + outputTokens` on every turn that has usage,
/// and `cachedReadTokens + cacheCreationTokens` never exceeds `inputTokens`.
/// Keep this refusal. Do not clamp the uncached remainder to 0.
/// `reasoningTokens` is inside `outputTokens` and stays in `reasoning` only.
/// Absent `reasoningTokens` and `cacheCreationTokens` are 0. `inputTokens`,
/// `outputTokens`, and `cachedReadTokens` stay required.
fn grok_usage_row(
    usage: &Value,
    session: &str,
    ts: Option<chrono::DateTime<chrono::Utc>>,
    model: Option<String>,
) -> Result<Buckets, String> {
    let input = json_i64(usage, "inputTokens")?;
    let cache_read = json_i64(usage, "cachedReadTokens")?;
    let cache_write = json_i64_or_zero(usage, "cacheCreationTokens")?;
    let input_uncached = input
        .checked_sub(cache_read)
        .and_then(|value| value.checked_sub(cache_write))
        .ok_or("negative-exclusive-input")?;
    if input_uncached < 0 {
        return Err("negative-exclusive-input".into());
    }
    Ok(Buckets {
        input_uncached,
        output: json_i64(usage, "outputTokens")?,
        cache_read,
        cache_write,
        reasoning: json_i64_or_zero(usage, "reasoningTokens")?,
        currency_total: None,
        message_id: None,
        native_session: session.to_string(),
        ts,
        predecessor_reason: None,
        prior_input_tokens: None,
        prior_cached_input_tokens: None,
        prior_cache_write_input_tokens: None,
        prior_output_tokens: None,
        prior_reasoning_output_tokens: None,
        model,
    })
}

fn grok_timestamp(value: &Value) -> Option<chrono::DateTime<chrono::Utc>> {
    value
        .get("timestamp")
        .and_then(Value::as_i64)
        .and_then(|secs| chrono::DateTime::from_timestamp(secs, 0))
}

/// Sibling `summary.json` only. Never `auth.json` or any other file in the directory.
/// The read is capped at 64 KiB. A symlink is skipped, same as `walk_jsonl`.
const GROK_SUMMARY_MAX_BYTES: u64 = 64 * 1024;

fn apply_grok_summary(
    jsonl: &Path,
    attribution: &mut UsageAttribution,
    reader_complete: &mut bool,
    warnings: &mut Vec<WarningNote>,
) {
    let Some(path) = jsonl.parent().map(|dir| dir.join("summary.json")) else {
        return;
    };
    if path.file_name().and_then(|name| name.to_str()) != Some("summary.json") {
        return;
    }
    let meta = match fs::symlink_metadata(&path) {
        Ok(meta) => meta,
        Err(_) => return,
    };
    if meta.file_type().is_symlink() || !meta.file_type().is_file() {
        return;
    }
    if meta.len() > GROK_SUMMARY_MAX_BYTES {
        return;
    }
    let file = match File::open(&path) {
        Ok(file) => file,
        Err(_) => {
            *reader_complete = false;
            warnings.push(WarningNote {
                path: path.display().to_string(),
                line: None,
                reason: "malformed-json".into(),
            });
            return;
        }
    };
    let mut buf = Vec::new();
    if file
        .take(GROK_SUMMARY_MAX_BYTES)
        .read_to_end(&mut buf)
        .is_err()
    {
        *reader_complete = false;
        warnings.push(WarningNote {
            path: path.display().to_string(),
            line: None,
            reason: "malformed-json".into(),
        });
        return;
    }
    let Ok(text) = std::str::from_utf8(&buf) else {
        *reader_complete = false;
        warnings.push(WarningNote {
            path: path.display().to_string(),
            line: None,
            reason: "malformed-json".into(),
        });
        return;
    };
    let value = match serde_json::from_str::<Value>(text) {
        Ok(value) => value,
        Err(_) => {
            *reader_complete = false;
            warnings.push(WarningNote {
                path: path.display().to_string(),
                line: None,
                reason: "malformed-json".into(),
            });
            return;
        }
    };
    attribution.model = value
        .get("current_model_id")
        .and_then(Value::as_str)
        .filter(|token| safe_attribution_token(token))
        .map(str::to_string);
    attribution.effort = value
        .get("reasoning_effort")
        .and_then(Value::as_str)
        .filter(|token| safe_attribution_token(token))
        .map(str::to_string);
    let supported = match value.get("chat_format_version") {
        None => true,
        Some(version) => version.as_i64() == Some(1),
    };
    if !supported {
        *reader_complete = false;
        warnings.push(WarningNote {
            path: path.display().to_string(),
            line: None,
            reason: "unknown-grok-format-version".into(),
        });
    }
}

/// Reads only native execution outcomes and real call boundaries. No result joins.
#[allow(clippy::too_many_arguments)]
fn observe_codex(
    value: &Value,
    text: &str,
    session: &str,
    source_id: &str,
    generation: i64,
    line: i64,
    ordinal: &mut i64,
    key: Option<&[u8]>,
    tools: &mut Vec<ToolFact>,
    skills: &mut Vec<SkillFact>,
    categories: &mut Vec<(String, ToolCategory)>,
) -> Result<bool> {
    let envelope = value.get("type").and_then(Value::as_str);
    let kind = value.pointer("/payload/type").and_then(Value::as_str);
    let (name, failed, fingerprint, category) = match (envelope, kind) {
        (Some("event_msg"), Some("item_completed")) => {
            let Some(item) = value.pointer("/payload/item") else {
                return Ok(false);
            };
            match item.get("type").and_then(Value::as_str) {
                Some("CommandExecution") => {
                    let cmd_text =
                        if let Some(parts) = item.get("command").and_then(Value::as_array) {
                            parts
                                .iter()
                                .filter_map(Value::as_str)
                                .collect::<Vec<_>>()
                                .join(" ")
                        } else if let Some(cmd) = item.get("command").and_then(Value::as_str) {
                            cmd.to_string()
                        } else {
                            String::new()
                        };
                    let cmd_raw = item
                        .get("command")
                        .map(Value::to_string)
                        .unwrap_or_else(|| cmd_text.clone());
                    let category =
                        classify_tool_invocation("codex", "CommandExecution", Some(&cmd_raw));
                    for (idx, skill_name) in
                        extract_skills_from_text(&cmd_text).into_iter().enumerate()
                    {
                        skills.push(SkillFact {
                            source_key: format!("{source_id}:{generation}:{line}:skill:{idx}"),
                            seat: format!("codex:{session}"),
                            skill: seal_skill(key, &skill_name),
                            invoked: true,
                        });
                    }
                    let failed = item
                        .get("exit_code")
                        .and_then(Value::as_i64)
                        .map(|code| code != 0)
                        .or_else(|| {
                            item.get("exit_code")
                                .and_then(Value::as_u64)
                                .map(|code| code != 0)
                        });
                    let valid =
                        item.get("command")
                            .and_then(Value::as_array)
                            .is_some_and(|parts| {
                                !parts.is_empty() && parts.iter().all(Value::is_string)
                            });
                    let lexical = raw_codex_command(text);
                    let fingerprint = if valid {
                        key.zip(lexical).map(|(key, raw)| {
                            dendro_core::codex_command_fingerprint(key, raw.get())
                        })
                    } else {
                        None
                    };
                    (dendro_core::CODEX_COMMAND, failed, fingerprint, category)
                }
                // Known non-command native items are not extra calls.
                Some(
                    "AgentMessage" | "Reasoning" | "Plan" | "FileChange" | "McpToolCall"
                    | "WebSearch" | "ImageView" | "UserMessage" | "Extension" | "ContextCompaction",
                ) => return Ok(true),
                Some("tool_call" | "custom_tool_call" | "function_call") => {
                    for (idx, skill_name) in
                        extract_skills_from_codex_call(item).into_iter().enumerate()
                    {
                        skills.push(SkillFact {
                            source_key: format!("{source_id}:{generation}:{line}:skill:{idx}"),
                            seat: format!("codex:{session}"),
                            skill: seal_skill(key, &skill_name),
                            invoked: true,
                        });
                    }
                    return Ok(true);
                }
                _ => return Ok(false),
            }
        }
        (Some("response_item"), Some("custom_tool_call" | "function_call" | "tool_call")) => {
            let category = if let Some(payload) = value.pointer("/payload") {
                for (idx, skill_name) in extract_skills_from_codex_call(payload)
                    .into_iter()
                    .enumerate()
                {
                    skills.push(SkillFact {
                        source_key: format!("{source_id}:{generation}:{line}:skill:{idx}"),
                        seat: format!("codex:{session}"),
                        skill: seal_skill(key, &skill_name),
                        invoked: true,
                    });
                }
                let tool_name = payload
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("custom_tool_call");
                let arg_str = if let Some(args) = payload.get("arguments") {
                    if let Some(s) = args.as_str() {
                        Some(s.to_string())
                    } else {
                        Some(args.to_string())
                    }
                } else if let Some(input) = payload.get("input") {
                    if let Some(s) = input.as_str() {
                        Some(s.to_string())
                    } else {
                        Some(input.to_string())
                    }
                } else if let Some(cmd) = payload.get("command") {
                    if let Some(s) = cmd.as_str() {
                        Some(s.to_string())
                    } else {
                        Some(cmd.to_string())
                    }
                } else {
                    None
                };
                classify_tool_invocation("codex", tool_name, arg_str.as_deref())
            } else {
                ToolCategory::Other
            };
            (dendro_core::CODEX_CALL, None, None, category)
        }
        (
            Some("response_item"),
            Some("custom_tool_call_output" | "function_call_output" | "message" | "reasoning"),
        ) => return Ok(true),
        (Some("response_item"), _) => return Ok(false),
        (
            Some("event_msg"),
            Some(
                "token_count"
                | "user_message"
                | "agent_message"
                | "agent_reasoning"
                | "task_started"
                | "task_complete"
                | "task_completed"
                | "turn_started"
                | "turn_complete"
                | "turn_aborted"
                | "item_started"
                | "item_updated"
                | "context_compacted"
                | "thread_settings_applied"
                | "warning",
            ),
        ) => return Ok(true),
        (Some("event_msg"), _) => return Ok(false),
        // Non-observation envelopes recognized by the existing harness reader.
        _ => return Ok(true),
    };
    *ordinal = ordinal
        .checked_add(1)
        .ok_or_else(|| anyhow::anyhow!("Codex observation order overflow"))?;
    let source_key = format!("{source_id}:{generation}:{line}:codex:0");
    categories.push((source_key.clone(), category));
    tools.push(ToolFact {
        source_key,
        harness: "codex".into(),
        native_session: session.into(),
        ts: timestamp_of(value),
        seq: *ordinal,
        name: name.into(),
        fingerprint,
        failed,
        structured_write: false,
        generation,
    });
    Ok(true)
}

fn raw_codex_command(text: &str) -> Option<&serde_json::value::RawValue> {
    use serde_json::value::RawValue;
    let line: std::collections::HashMap<&str, &RawValue> = serde_json::from_str(text).ok()?;
    let payload: std::collections::HashMap<&str, &RawValue> =
        serde_json::from_str(line.get("payload")?.get()).ok()?;
    let item: std::collections::HashMap<&str, &RawValue> =
        serde_json::from_str(payload.get("item")?.get()).ok()?;
    item.get("command").copied()
}

/// The tool calls of one Claude or Pi file generation (#35, #36, #37, Q5,
/// Phase 1 0002). A call is the first call block of its (session, call id), at
/// that position and time; a repeat of the id is the same call. Claude: a
/// `tool_use` block, settled by `tool_result` blocks (`tool_use_id`,
/// `is_error`). Pi: a `toolCall` block, settled by `toolResult` messages
/// (`toolCallId`, `isError`), in the file's K10 session. Its outcome comes only
/// from the matching results in the same file and session: all true is a
/// failure, all false a success, anything else (none, absent, not a boolean,
/// disagreeing) unknown. A result with no
/// call creates nothing. Rejected records (unknown type, malformed) give
/// nothing. Lines up to `new_from` were stored by an earlier run: their calls
/// are not inserted again, and their outcome is re-settled only when a result
/// for them arrives in the new lines.
struct ToolPass {
    new_calls: Vec<ToolFact>,
    settled: Vec<(String, Option<bool>)>,
    skills: Vec<SkillFact>,
    categories: Vec<(String, ToolCategory)>,
}

struct ToolCall {
    fact: ToolFact,
    line: i64,
    results: Vec<Option<bool>>,
    result_in_new_lines: bool,
    category: ToolCategory,
}

struct ToolScan<'a> {
    harness: &'a str,
    file_session: Option<&'a str>,
    /// Lines up to here were stored by an earlier run or batch.
    new_from: i64,
    key: Option<&'a [u8]>,
    source_id: &'a str,
    generation: i64,
    calls: Vec<ToolCall>,
    by_id: std::collections::HashMap<(String, String), usize>,
    results: Vec<((String, String), Option<bool>, i64)>,
    skills: Vec<(i64, SkillFact)>,
}

impl<'a> ToolScan<'a> {
    fn new(
        harness: &'a str,
        file_session: Option<&'a str>,
        new_from: i64,
        key: Option<&'a [u8]>,
        source_id: &'a str,
        generation: i64,
    ) -> Self {
        Self {
            harness,
            file_session,
            new_from,
            key,
            source_id,
            generation,
            calls: Vec::new(),
            by_id: std::collections::HashMap::new(),
            results: Vec::new(),
            skills: Vec::new(),
        }
    }

    fn observe(&mut self, line_no: i64, text: &str, value: &Value) {
        if self.harness == "grok" {
            self.observe_grok(line_no, value);
            return;
        }
        let harness = self.harness;
        let file_session = self.file_session;
        let key = self.key;
        let source_id = self.source_id;
        let generation = self.generation;
        let session = match file_session {
            Some(session) => session.to_string(),
            None => value
                .get("sessionId")
                .and_then(|item| item.as_str())
                .unwrap_or("claude-session")
                .to_string(),
        };
        if harness == "pi"
            && value
                .pointer("/message/role")
                .and_then(|item| item.as_str())
                == Some("toolResult")
        {
            if let Some(id) = value
                .pointer("/message/toolCallId")
                .and_then(|item| item.as_str())
            {
                let outcome = value
                    .pointer("/message/isError")
                    .and_then(|item| item.as_bool());
                self.results
                    .push(((session.clone(), id.to_string()), outcome, line_no));
            }
            return;
        }
        let Some(content) = value
            .pointer("/message/content")
            .and_then(|item| item.as_array())
        else {
            return;
        };
        let (call_type, input_field) = if harness == "pi" {
            ("toolCall", "arguments")
        } else {
            ("tool_use", "input")
        };
        let ts = timestamp_of(value);
        let raw_blocks = raw_content_blocks(text);
        let mut line_skill_idx = 0usize;
        for (index, block) in content.iter().enumerate() {
            match block.get("type").and_then(|item| item.as_str()) {
                Some(kind) if kind == call_type => {
                    let Some(name) = block.get("name").and_then(|item| item.as_str()) else {
                        continue;
                    };
                    let id = block.get("id").and_then(|item| item.as_str());
                    if let Some(id) = id {
                        if self.by_id.contains_key(&(session.clone(), id.to_string())) {
                            continue;
                        }
                        self.by_id
                            .insert((session.clone(), id.to_string()), self.calls.len());
                    }
                    let lexical = tool_input_lexical(block, raw_blocks.get(index), input_field);
                    let fingerprint =
                        key.and_then(|key| tool_fingerprint(key, name, lexical.as_deref()));
                    let input_val = block
                        .get(input_field)
                        .or_else(|| block.get("input"))
                        .or_else(|| block.get("arguments"));
                    let input_str = input_val.map(|v| {
                        if let Some(s) = v.as_str() {
                            s.to_string()
                        } else {
                            v.to_string()
                        }
                    });
                    let raw_command_or_input = lexical.as_deref().or(input_str.as_deref());
                    let category = classify_tool_invocation(harness, name, raw_command_or_input);
                    let parsed_input: Option<Value> = match input_val {
                        Some(Value::Object(_)) => input_val.cloned(),
                        Some(Value::String(s)) => serde_json::from_str(s).ok(),
                        _ => None,
                    };
                    let extracted_skills: Vec<String> = if matches!(name, "Skill" | "use_skill") {
                        parsed_input
                            .as_ref()
                            .and_then(|inp| {
                                inp.get("skill")
                                    .or_else(|| inp.get("name"))
                                    .or_else(|| inp.get("skill_name"))
                            })
                            .and_then(Value::as_str)
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .into_iter()
                            .collect()
                    } else if matches!(name, "Read" | "read" | "view_file") {
                        parsed_input
                            .as_ref()
                            .and_then(|inp| inp.get("file_path").or_else(|| inp.get("path")))
                            .and_then(Value::as_str)
                            .map(extract_skills_from_text)
                            .unwrap_or_default()
                    } else {
                        Vec::new()
                    };
                    for skill_name in extracted_skills {
                        self.skills.push((
                            line_no,
                            SkillFact {
                                source_key: format!(
                                    "{source_id}:{generation}:{line_no}:skill:{line_skill_idx}"
                                ),
                                seat: format!("{harness}:{}", seal_session(key, harness, &session)),
                                skill: seal_skill(key, &skill_name),
                                invoked: true,
                            },
                        ));
                        line_skill_idx += 1;
                    }
                    self.calls.push(ToolCall {
                        fact: ToolFact {
                            source_key: format!("{source_id}:{generation}:{line_no}:{index}"),
                            harness: harness.into(),
                            // Sealed like the usage session, so tools join the same
                            // session and seat and two secret-shaped sessions stay distinct.
                            native_session: seal_session(key, harness, &session),
                            ts,
                            seq: line_no * 100 + index as i64,
                            name: name.to_string(),
                            fingerprint,
                            failed: None,
                            structured_write: if harness == "pi" {
                                matches!(name, "write" | "edit")
                            } else {
                                matches!(name, "Write" | "Edit" | "MultiEdit" | "NotebookEdit")
                            },
                            generation,
                        },
                        line: line_no,
                        results: Vec::new(),
                        result_in_new_lines: false,
                        category,
                    });
                }
                Some("tool_result") if harness == "claude" => {
                    if let Some(id) = block.get("tool_use_id").and_then(|item| item.as_str()) {
                        let outcome = block.get("is_error").and_then(|item| item.as_bool());
                        self.results
                            .push(((session.clone(), id.to_string()), outcome, line_no));
                    }
                }
                _ => {}
            }
        }
    }

    /// Join `tool_call` to later `tool_call_update` rows by `toolCallId`.
    /// `rawInput` is fingerprinted and dropped. Progress updates have no
    /// `status` and do not settle the call. `completed` is success and
    /// `failed` is failure, matching the Claude `is_error` boolean.
    fn observe_grok(&mut self, line_no: i64, value: &Value) {
        let kind = grok_session_update(value);
        let session = match self.file_session {
            Some(session) => session.to_string(),
            None => value
                .pointer("/params/sessionId")
                .and_then(Value::as_str)
                .unwrap_or("grok-session")
                .to_string(),
        };
        let Some(update) = value.pointer("/params/update") else {
            return;
        };
        match kind {
            Some("tool_call_update") => {
                let Some(id) = update.get("toolCallId").and_then(Value::as_str) else {
                    return;
                };
                let outcome = match update.get("status").and_then(Value::as_str) {
                    Some("failed") => Some(true),
                    Some("completed") => Some(false),
                    Some(_) => None,
                    None => return,
                };
                self.results
                    .push(((session, id.to_string()), outcome, line_no));
            }
            Some("tool_call") => {
                let Some(id) = update.get("toolCallId").and_then(Value::as_str) else {
                    return;
                };
                if self.by_id.contains_key(&(session.clone(), id.to_string())) {
                    return;
                }
                let Some(name) = update
                    .get("_meta")
                    .and_then(|meta| meta.get("x.ai/tool"))
                    .and_then(|tool| tool.get("name"))
                    .and_then(Value::as_str)
                    .filter(|name| !name.is_empty() && name.len() <= 64)
                else {
                    return;
                };
                let kind_name = update
                    .get("_meta")
                    .and_then(|meta| meta.get("x.ai/tool"))
                    .and_then(|tool| tool.get("kind"))
                    .and_then(Value::as_str);
                let lexical = update.get("rawInput").map(Value::to_string);
                let fingerprint = self
                    .key
                    .and_then(|key| tool_fingerprint(key, name, lexical.as_deref()));
                let category = classify_tool_invocation(self.harness, name, None);
                let source_key = format!("{}:{}:{line_no}:0", self.source_id, self.generation);
                self.by_id
                    .insert((session.clone(), id.to_string()), self.calls.len());
                self.calls.push(ToolCall {
                    fact: ToolFact {
                        source_key,
                        harness: self.harness.into(),
                        native_session: seal_session(self.key, self.harness, &session),
                        ts: grok_timestamp(value),
                        seq: line_no,
                        name: name.to_string(),
                        fingerprint,
                        failed: None,
                        structured_write: kind_name == Some("edit"),
                        generation: self.generation,
                    },
                    line: line_no,
                    results: Vec::new(),
                    result_in_new_lines: false,
                    category,
                });
            }
            _ => {}
        }
    }

    /// Settle what the lines read so far say, then treat them all as stored:
    /// the scan keeps its calls so a later batch can still settle them.
    fn flush(&mut self, upto: i64) -> ToolPass {
        let new_from = self.new_from;
        let results = std::mem::take(&mut self.results);
        for (id, outcome, line_no) in results {
            if let Some(&at) = self.by_id.get(&id) {
                self.calls[at].results.push(outcome);
                self.calls[at].result_in_new_lines |= line_no > new_from;
            }
        }
        let mut pass = ToolPass {
            new_calls: Vec::new(),
            settled: Vec::new(),
            skills: Vec::new(),
            categories: Vec::new(),
        };
        for (line, fact) in std::mem::take(&mut self.skills) {
            if line > new_from {
                pass.skills.push(fact);
            }
        }
        for call in &mut self.calls {
            let first = call.results.first().copied().flatten();
            call.fact.failed = match first {
                Some(outcome) if call.results.iter().all(|item| *item == Some(outcome)) => {
                    Some(outcome)
                }
                _ => None,
            };
            if call.line > new_from {
                pass.categories
                    .push((call.fact.source_key.clone(), call.category));
                pass.new_calls.push(call.fact.clone());
            } else if call.result_in_new_lines {
                pass.settled
                    .push((call.fact.source_key.clone(), call.fact.failed));
            }
            call.result_in_new_lines = false;
        }
        self.new_from = upto;
        pass
    }
}

/// The exact source text of each `message.content` block, in order, so a
/// tool input is compared as written (§4 rule 2, Q4).
fn raw_content_blocks(text: &str) -> Vec<&serde_json::value::RawValue> {
    use serde_json::value::RawValue;
    #[derive(serde::Deserialize)]
    struct Line<'a> {
        #[serde(borrow)]
        message: Option<&'a RawValue>,
    }
    #[derive(serde::Deserialize)]
    struct Message<'a> {
        #[serde(borrow)]
        content: Option<&'a RawValue>,
    }
    let blocks = || -> Option<Vec<&RawValue>> {
        let line: Line = serde_json::from_str(text).ok()?;
        let message: Message = serde_json::from_str(line.message?.get()).ok()?;
        serde_json::from_str(message.content?.get()).ok()
    };
    blocks().unwrap_or_default()
}

/// A tool call's input for its fingerprint: a string input is its body, as
/// before; any other input is its exact JSON text with only the ends trimmed.
/// No reordering or other normalization, and the text is never stored.
fn tool_input_lexical(
    block: &Value,
    raw_block: Option<&&serde_json::value::RawValue>,
    field: &str,
) -> Option<String> {
    let input = block.get(field)?;
    if let Some(body) = input.as_str() {
        return Some(body.to_string());
    }
    let raw: std::collections::HashMap<&str, &serde_json::value::RawValue> =
        serde_json::from_str(raw_block?.get()).ok()?;
    Some(raw.get(field)?.get().trim().to_string())
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

fn json_i64_or_zero(value: &Value, key: &str) -> Result<i64, String> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(0),
        Some(item) => item.as_i64().ok_or_else(|| format!("non-integer-{key}")),
    }
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

    #[test]
    fn ingest_baseline_uses_full_size_and_refreshes_partial_tail_noops() {
        let dir = std::env::temp_dir().join(format!("dendro-baseline-tail-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let database = dir.join("analytics.duckdb");
        let baseline = |harness: &str| -> (String, String, String) {
            duckdb::Connection::open(&database).unwrap().query_row(
                "SELECT latest_association, last_stable_size, latest_attempt_at FROM file_source_baselines WHERE source_id = ?",
                [harness], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            ).unwrap()
        };
        for harness in ["claude", "pi", "codex"] {
            let store = AnalyticsStore::open_path(&database, "key").unwrap();
            let path = dir.join(format!("{harness}.jsonl"));
            let header = match harness {
                "pi" => r#"{"type":"session","id":"session"}"#,
                "codex" => r#"{"type":"session_meta","payload":{"id":"session"}}"#,
                _ => r#"{"type":"system","sessionId":"session"}"#,
            };
            fs::write(&path, format!("{header}\n{{\"partial\":")).unwrap();
            ingest_jsonl(&store, harness, harness, &path).unwrap();
            let offset = store.source_state(harness).unwrap().unwrap().1;
            store.checkpoint().unwrap();
            drop(store);
            let before = baseline(harness);
            assert_eq!(before.0, "verified", "{harness}");
            assert_eq!(before.1, fs::metadata(&path).unwrap().len().to_string());
            assert!(before.1.parse::<i64>().unwrap() > offset);
            fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap()
                .write_all(b"42")
                .unwrap();
            let store = AnalyticsStore::open_path(&database, "key").unwrap();
            let stats = ingest_jsonl_with_key(&store, harness, harness, &path, None).unwrap();
            assert_eq!(stats.records_read, 0);
            store.checkpoint().unwrap();
            drop(store);
            let after = baseline(harness);
            assert_eq!(after.0, "verified");
            assert_eq!(after.1, fs::metadata(&path).unwrap().len().to_string());
            assert_ne!(after.1, before.1);
            assert_ne!(after.2, before.2);
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn header_reads_the_held_file_when_its_name_is_replaced() {
        let dir = std::env::temp_dir().join(format!("dendro-header-held-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        for harness in ["pi", "codex"] {
            let header = |id| {
                if harness == "pi" {
                    format!(r#"{{"type":"session","id":"{id}"}}"#)
                } else {
                    format!(r#"{{"type":"session_meta","payload":{{"id":"{id}"}}}}"#)
                }
            };
            let path = dir.join(format!("{harness}.jsonl"));
            fs::write(&path, format!("{}\n", header("original"))).unwrap();
            let mut held = File::open(&path).unwrap();
            let before = FileMetadata::from_file(&held);
            let named_before = observe_path(&path).metadata;
            let replacement = dir.join("replacement.jsonl");
            fs::write(&replacement, format!("{}\n", header("replacement"))).unwrap();
            fs::rename(&replacement, &path).unwrap();
            let session = file_session(harness, &mut held, "source").unwrap();
            assert!(session.from_header);
            assert_eq!(session.raw, "original");
            assert!(stable_ingest_metadata(
                before,
                named_before,
                FileMetadata::from_file(&held),
                observe_path(&path).metadata
            )
            .is_none());
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn accepted_ingest_substitution_keeps_old_header_and_tracks_association_recovery() {
        let dir = std::env::temp_dir().join(format!("dendro-ingest-swap-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let database = dir.join("analytics.duckdb");
        let path = dir.join("pi.jsonl");
        let original = concat!(
            "{\"type\":\"session\",\"id\":\"original\"}\n",
            "{\"type\":\"message\",\"id\":\"m\",\"message\":{\"role\":\"assistant\",\"usage\":{\"input\":3,\"output\":1,\"cacheRead\":0,\"cacheWrite\":0}}}\n"
        );
        fs::write(&path, original).unwrap();
        let store = AnalyticsStore::open_path(&database, "key").unwrap();
        let replacement = dir.join("replacement.jsonl");
        let swap = |text: &str| -> Result<()> {
            fs::write(&replacement, text)?;
            fs::rename(&replacement, &path)?;
            Ok(())
        };
        let baseline = || -> Vec<Option<String>> {
            store.checkpoint().unwrap();
            duckdb::Connection::open(&database)
                .unwrap()
                .query_row(
                    "SELECT latest_association, latest_attempt_at, last_stable_observed_at,
                        last_stable_type, last_stable_device, last_stable_inode,
                        last_stable_size, last_stable_mtime_ns, association_reason
                 FROM file_source_baselines WHERE source_id = 'source' AND generation = 1",
                    [],
                    |row| (0..9).map(|i| row.get(i)).collect(),
                )
                .unwrap()
        };
        ingest_jsonl_with_key_checking(&store, "source", "pi", &path, None, || {
            swap(&original.replace("original", "replacement"))
        })
        .unwrap();
        let usage = store.load_usage().unwrap();
        assert_eq!(usage.len(), 1);
        assert_eq!(usage[0].native_session, "original");
        assert_eq!(
            store.load_sources().unwrap()[0].4.as_deref(),
            Some("original")
        );
        let first = baseline();
        assert_eq!(first[0].as_deref(), Some("unavailable"));
        assert!(first[2..8].iter().all(Option::is_none));
        assert_eq!(first[8].as_deref(), Some("association_unverified"));
        // Return the original content so this generation's producing prefix is unchanged.
        fs::write(&path, original).unwrap();
        ingest_jsonl(&store, "source", "pi", &path).unwrap();
        let stable = baseline();
        assert_eq!(stable[0].as_deref(), Some("verified"));
        assert!(stable[2..8].iter().all(Option::is_some));
        let noop =
            ingest_jsonl_with_key_checking(&store, "source", "pi", &path, None, || swap(original))
                .unwrap();
        assert_eq!(noop.records_read, 0);
        let stale = baseline();
        assert_eq!(stale[0].as_deref(), Some("unavailable"));
        assert_ne!(stale[1], stable[1]);
        assert_eq!(
            &stale[2..8],
            &stable[2..8],
            "accepted uncertainty retains prior stable data"
        );
        assert_eq!(store.source_state("source").unwrap().unwrap().0, 1);
        ingest_jsonl(&store, "source", "pi", &path).unwrap();
        let recovered = baseline();
        assert_eq!(recovered[0].as_deref(), Some("verified"));
        assert_ne!(
            recovered[5], stable[5],
            "stable recovery records the replacement inode"
        );
        assert_eq!(
            store.load_usage().unwrap().len(),
            1,
            "metadata transitions never duplicate facts"
        );
        drop(store);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn metadata_association_rejects_substitutions_and_unavailable_checks() {
        let metadata = |inode: &str| FileMetadata {
            r#type: "regular".into(),
            device: "7".into(),
            inode: inode.into(),
            size: "40".into(),
            mtime_ns: "123".into(),
        };
        assert!(stable_ingest_metadata(
            Some(metadata("1")),
            Some(metadata("1")),
            Some(metadata("1")),
            Some(metadata("1"))
        )
        .is_some());
        for position in 0..4 {
            let mut values = [
                Some(metadata("1")),
                Some(metadata("1")),
                Some(metadata("1")),
                Some(metadata("1")),
            ];
            values[position] = Some(metadata("2"));
            let [a, b, c, d] = values;
            assert!(stable_ingest_metadata(a, b, c, d).is_none());
            let mut values = [
                Some(metadata("1")),
                Some(metadata("1")),
                Some(metadata("1")),
                Some(metadata("1")),
            ];
            values[position] = None;
            let [a, b, c, d] = values;
            assert!(stable_ingest_metadata(a, b, c, d).is_none());
        }
    }

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

        fn with_message_metadata(line: String, model: &str, field: &str, effort: &str) -> String {
            let mut value: Value = serde_json::from_str(&line).unwrap();
            value["message"]["model"] = model.into();
            value["message"][field] = effort.into();
            value.to_string()
        }

        fn model_rows(f: &Fixture) -> Vec<(Option<String>, Option<String>)> {
            let models = f.db.load_usage_models().unwrap();
            let mut usages = f.db.load_usage().unwrap();
            usages.sort_by_key(|usage| (usage.generation, usage.seq));
            usages
                .iter()
                .map(|usage| models.get(&usage.source_key).cloned().unwrap_or_default())
                .collect()
        }

        #[test]
        fn codex_model_effort_tracks_context_and_resumes_from_prefix() {
            let fresh = Fixture::new("codex-model-fresh");
            let split = Fixture::new("codex-model-split");
            let part1 = vec![
                r#"{"type":"session_meta","payload":{"id":"sku-session","model":"gpt-6.1","effort":"low"}}"#.into(),
                codex_count(10, None),
                r#"{"type":"turn_context","payload":{"model":"gpt-6.1-sol","collaboration_mode":{"settings":{"reasoning_effort":"high"}}}}"#.into(),
            ];
            let part2 = vec![
                codex_count(15, None),
                r#"{"type":"event_msg","payload":{"type":"model_selected","model":"gpt-6","reasoning_effort":"medium"}}"#.into(),
                codex_count(20, None),
                r#"{"type":"turn_context","payload":{"effort":"xhigh"}}"#.into(),
                codex_count(25, None),
            ];
            let whole = fresh.file("sku.jsonl", &[part1.clone(), part2.clone()].concat());
            fresh.ingest("codex", &whole);
            let file = split.file("sku.jsonl", &part1);
            split.ingest("codex", &file);
            // Resume with the context in the prior prefix and no metadata on
            // the next token_count. Prefix counters must not be reapplied.
            split.append(&file, &part2);
            split.ingest("codex", &file);
            split.ingest("codex", &file);
            assert_eq!(model_rows(&fresh), model_rows(&split));
            assert_eq!(
                model_rows(&split),
                [
                    (Some("gpt-6.1".into()), Some("low".into())),
                    (Some("gpt-6.1-sol".into()), Some("high".into())),
                    (Some("gpt-6".into()), Some("medium".into())),
                    (Some("gpt-6".into()), Some("xhigh".into())),
                ]
            );
            assert_eq!(split.sessions("codex").values().sum::<i64>(), 25);
        }

        #[test]
        fn pi_model_effort_tracks_changes_messages_and_resumes() {
            let f = Fixture::new("pi-model");
            let file = f.file(
                "sku.jsonl",
                &[
                    pi_header("sku-session"),
                    r#"{"type":"model_change","modelId":"grok-4"}"#.into(),
                    r#"{"type":"thinking_level_change","thinkingLevel":"medium"}"#.into(),
                    pi_usage("first", 10),
                ],
            );
            f.ingest("pi", &file);
            f.append(
                &file,
                &[
                    pi_usage("resumed", 20),
                    with_message_metadata(
                        pi_usage("override", 30),
                        "claude-opus-4.1",
                        "thinkingLevel",
                        "high",
                    ),
                    pi_usage("after-override", 40),
                    r#"{"type":"model_change","model":"gpt-6"}"#.into(),
                    r#"{"type":"thinking_level_change","thinkingLevel":"low"}"#.into(),
                    pi_usage("changed", 50),
                ],
            );
            f.ingest("pi", &file);
            assert_eq!(
                model_rows(&f),
                [
                    (Some("grok-4".into()), Some("medium".into())),
                    (Some("grok-4".into()), Some("medium".into())),
                    (Some("claude-opus-4.1".into()), Some("high".into())),
                    (Some("claude-opus-4.1".into()), Some("high".into())),
                    (Some("gpt-6".into()), Some("low".into())),
                ]
            );
        }

        #[test]
        fn claude_model_effort_is_per_message_and_keeps_usage_dedupe() {
            let f = Fixture::new("claude-model");
            let first = with_message_metadata(
                claude_usage("first", 10, None),
                "claude-opus-4.1",
                "effort",
                "high",
            );
            let file = f.file(
                "sku.jsonl",
                &[
                    first.clone(),
                    first,
                    claude_usage("unknown", 20, None),
                    with_message_metadata(
                        claude_usage("last", 30, None),
                        "claude-sonnet-4.5",
                        "reasoning_effort",
                        "medium",
                    ),
                ],
            );
            f.ingest("claude", &file);
            assert_eq!(
                model_rows(&f),
                [
                    (Some("claude-opus-4.1".into()), Some("high".into())),
                    (None, None),
                    (Some("claude-sonnet-4.5".into()), Some("medium".into())),
                ]
            );
        }

        #[test]
        fn usage_model_metadata_rejects_secrets_and_invalid_tokens() {
            for token in [
                "",
                " ",
                "claude/opus",
                "gpt model",
                "éffort",
                "sk-model-secret-canary",
                "ghp_model-secret-canary",
                "Bearer model-secret-canary",
            ] {
                assert!(!safe_attribution_token(token));
            }
            assert!(!safe_attribution_token(&"a".repeat(65)));
            assert!(safe_attribution_token(&"a".repeat(64)));
            assert!(safe_attribution_token("GPT-6.1_sol"));

            let f = Fixture::new("model-privacy");
            for harness in ["codex", "pi", "claude"] {
                let lines = match harness {
                    "codex" => vec![
                        codex_header("session"),
                        r#"{"type":"turn_context","payload":{"model":"gpt-6","effort":"high"}}"#.into(),
                        r#"{"type":"turn_context","payload":{"model":"sk-model-secret-canary","effort":"ghp_effort-secret-canary"}}"#.into(),
                        codex_count(10, None),
                    ],
                    "pi" => vec![
                        pi_header("session"),
                        r#"{"type":"model_change","modelId":"gpt-6"}"#.into(),
                        r#"{"type":"thinking_level_change","thinkingLevel":"high"}"#.into(),
                        r#"{"type":"model_change","modelId":"sk-model-secret-canary"}"#.into(),
                        r#"{"type":"thinking_level_change","thinkingLevel":"ghp_effort-secret-canary"}"#.into(),
                        pi_usage("message", 10),
                    ],
                    _ => vec![with_message_metadata(
                        claude_usage("message", 10, None),
                        "sk-model-secret-canary",
                        "effort",
                        "ghp_effort-secret-canary",
                    )],
                };
                let file = f.file(&format!("{harness}.jsonl"), &lines);
                f.ingest(harness, &file);
            }
            assert_eq!(f.db.load_usage().unwrap().len(), 3);
            assert!(f.db.load_usage_models().unwrap().is_empty());
        }

        #[test]
        fn replacement_generation_drops_previous_model_context() {
            let f = Fixture::new("model-generation");
            let file = f.file(
                "sku.jsonl",
                &[
                    codex_header("session"),
                    r#"{"type":"turn_context","payload":{"model":"gpt-6","effort":"high"}}"#.into(),
                    codex_count(10, None),
                ],
            );
            f.ingest("codex", &file);
            fs::write(
                &file,
                format!(
                    "{}\n{}\n",
                    codex_header("new-session"),
                    codex_count(5, None)
                ),
            )
            .unwrap();
            f.ingest("codex", &file);
            assert_eq!(
                model_rows(&f),
                [(Some("gpt-6".into()), Some("high".into())), (None, None)]
            );
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
                    "coverage-partial generation=1",
                    "coverage=partial projected=3 unprojected=3 held_tail=0 edges=0 (unknown_kind=1 malformed=2)",
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

        /// One Claude assistant line: message id, session, time offset in
        /// seconds, content blocks as raw JSON text, usage input tokens.
        fn chunk(msg: &str, session: &str, at: u32, blocks: &[String], input: i64) -> String {
            format!(
                "{{\"type\":\"assistant\",\"sessionId\":\"{session}\",\"timestamp\":\"2026-10-09T00:00:{at:02}Z\",\"message\":{{\"id\":\"{msg}\",\"content\":[{}],\"usage\":{{\"input_tokens\":{input},\"output_tokens\":0,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0}}}}}}",
                blocks.join(",")
            )
        }

        fn tool_use(id: &str, name: &str, input: &str) -> String {
            format!(
                "{{\"type\":\"tool_use\",\"id\":\"{id}\",\"name\":\"{name}\",\"input\":{input}}}"
            )
        }

        /// A user line carrying tool results, as real Claude logs record them.
        fn results(session: &str, at: u32, outcomes: &[(&str, &str)]) -> String {
            let blocks: Vec<String> = outcomes
                .iter()
                .map(|(id, is_error)| {
                    let flag = if is_error.is_empty() {
                        String::new()
                    } else {
                        format!(",\"is_error\":{is_error}")
                    };
                    format!("{{\"type\":\"tool_result\",\"tool_use_id\":\"{id}\",\"content\":\"x\"{flag}}}")
                })
                .collect();
            format!(
                "{{\"type\":\"user\",\"sessionId\":\"{session}\",\"timestamp\":\"2026-10-09T00:00:{at:02}Z\",\"message\":{{\"role\":\"user\",\"content\":[{}]}}}}",
                blocks.join(",")
            )
        }

        /// (seq, failed) of every stored tool call, in order.
        fn calls(f: &Fixture) -> Vec<(i64, Option<bool>)> {
            let mut out: Vec<_> =
                f.db.load_tools()
                    .unwrap()
                    .into_iter()
                    .map(|t| (t.seq, t.failed))
                    .collect();
            out.sort();
            out
        }

        #[test]
        fn streamed_copies_keep_each_distinct_tool_once_and_usage_once_35() {
            let f = Fixture::new("stream-tools");
            let read = tool_use("t1", "Read", "{\"p\":1}");
            let c = f.file(
                "c.jsonl",
                &[
                    chunk("m1", "s", 1, std::slice::from_ref(&read), 5),
                    chunk("m1", "s", 2, &[tool_use("t2", "Write", "{\"p\":2}")], 5),
                    chunk("m1", "s", 3, &[tool_use("t3", "Read", "{\"p\":3}")], 5),
                    chunk("m1", "s", 4, &[read], 5),
                ],
            );
            f.ingest("claude", &c);
            assert_eq!(
                f.db.load_usage().unwrap().len(),
                1,
                "usage dedupe unchanged"
            );
            let tools = f.db.load_tools().unwrap();
            assert_eq!(tools.len(), 3, "a streamed repeat of t1 is the same call");
            assert_eq!(tools.iter().filter(|t| t.structured_write).count(), 1);
            f.ingest("claude", &c);
            assert_eq!(
                f.db.load_tools().unwrap().len(),
                3,
                "identical rerun adds nothing"
            );
        }

        #[test]
        fn tool_outcomes_come_only_from_matching_results_37() {
            let f = Fixture::new("results");
            let uses: Vec<String> = ["a", "b", "c", "d", "e", "f", "g"]
                .iter()
                .map(|id| tool_use(id, "Read", "{\"p\":1}"))
                .collect();
            let mut old_shape = tool_use("g", "Read", "{\"p\":1}");
            old_shape.insert_str(old_shape.len() - 1, ",\"is_error\":true");
            let c = f.file(
                "c.jsonl",
                &[
                    chunk("m1", "s", 1, &uses[..6], 1),
                    chunk("m2", "s", 2, &[old_shape], 1),
                    results(
                        "s",
                        3,
                        &[
                            ("a", "true"),
                            ("b", "false"),
                            ("c", ""),
                            ("d", "\"yes\""),
                            ("f", "true"),
                            ("f", "false"),
                            ("z", "true"),
                        ],
                    ),
                    // The same id in another session never settles this call.
                    results("other", 4, &[("e", "true")]),
                ],
            );
            f.ingest("claude", &c);
            let got: Vec<Option<bool>> = calls(&f).into_iter().map(|(_, failed)| failed).collect();
            assert_eq!(
                got,
                [
                    Some(true),  // a: true
                    Some(false), // b: false
                    None,        // c: absent
                    None,        // d: not a boolean
                    None,        // e: only another session's result
                    None,        // f: disagreeing results
                    None,        // g: is_error on tool_use is not an outcome; no result
                ],
                "the orphan result z creates no call"
            );
        }

        #[test]
        fn rejected_records_yield_no_tool_facts_36() {
            let f = Fixture::new("rejected-tools");
            let mut lines = vec![chunk(
                "m1",
                "s",
                1,
                &[tool_use("ok", "Read", "{\"p\":1}")],
                1,
            )];
            for n in 0..3 {
                lines.push(format!(
                    "{{\"type\":\"future_kind\",\"sessionId\":\"s\",\"timestamp\":\"2026-10-09T00:00:0{}Z\",\"message\":{{\"id\":\"x{n}\",\"content\":[{}]}}}}",
                    n + 2,
                    tool_use(&format!("u{n}"), "Read", "{\"p\":1}")
                ));
            }
            lines.push(format!(
                "{{\"type\":\"assistant\",\"message\":{{\"content\":[{}]",
                tool_use("broken", "Read", "1")
            ));
            let c = f.file("c.jsonl", &lines);
            f.ingest("claude", &c);
            assert_eq!(
                f.db.load_tools().unwrap().len(),
                1,
                "only the known record's call"
            );
        }

        /// The same bytes ingested at once and in parts give the same calls.
        fn split_equals_fresh(name: &str, parts: &[&[String]]) -> Vec<(i64, Option<bool>)> {
            let all: Vec<String> = parts.iter().flat_map(|p| p.iter().cloned()).collect();
            let fresh = Fixture::new(&format!("{name}-fresh"));
            let whole = fresh.file("c.jsonl", &all);
            fresh.ingest("claude", &whole);
            let split = Fixture::new(&format!("{name}-split"));
            let file = split.file("c.jsonl", parts[0]);
            split.ingest("claude", &file);
            for part in &parts[1..] {
                split.append(&file, part);
                split.ingest("claude", &file);
            }
            split.ingest("claude", &file);
            assert_eq!(
                calls(&fresh),
                calls(&split),
                "{name}: fresh and resumed agree"
            );
            calls(&split)
        }

        #[test]
        fn appended_results_settle_and_unsettle_like_a_fresh_ingest_37() {
            let uses: Vec<String> = ["a", "b", "c"]
                .iter()
                .map(|id| tool_use(id, "Read", "{\"p\":1}"))
                .collect();
            let part1 = vec![chunk("m1", "s", 1, &uses, 1)];
            let part2 = vec![results(
                "s",
                2,
                &[("a", "true"), ("b", "true"), ("c", "true")],
            )];
            let part3 = vec![results("s", 3, &[("b", "false")])];
            let settled = split_equals_fresh("append", &[&part1, &part2]);
            assert!(settled.iter().all(|(_, failed)| *failed == Some(true)));
            // A later disagreeing result turns a failure back to unknown.
            let conflicted = split_equals_fresh("conflict", &[&part1, &part2, &part3]);
            let got: Vec<_> = conflicted.iter().map(|(_, failed)| *failed).collect();
            assert_eq!(got, [Some(true), None, Some(true)]);
            // A result arriving before its call's line is appended is kept.
            let early = split_equals_fresh("result-first", &[&part2, &part1]);
            assert!(early.iter().all(|(_, failed)| *failed == Some(true)));
        }

        #[test]
        fn a_replaced_file_reads_its_new_generation_whole_37() {
            let f = Fixture::new("replace-tools");
            let c = f.file(
                "c.jsonl",
                &[chunk(
                    "m1",
                    "s",
                    1,
                    &[tool_use("a", "Read", "{\"p\":1}")],
                    1,
                )],
            );
            f.ingest("claude", &c);
            fs::write(
                &c,
                format!(
                    "{}\n{}\n",
                    chunk("m9", "s", 5, &[tool_use("a", "Read", "{\"p\":1}")], 1),
                    results("s", 6, &[("a", "true")])
                ),
            )
            .unwrap();
            f.ingest("claude", &c);
            let tools = f.db.load_tools().unwrap();
            let gens: Vec<(i64, Option<bool>)> =
                tools.iter().map(|t| (t.generation, t.failed)).collect();
            assert!(gens.contains(&(1, None)), "generation 1 is kept as it was");
            assert!(
                gens.contains(&(2, Some(true))),
                "generation 2 is settled on its own"
            );
        }

        #[test]
        fn object_inputs_match_only_when_written_the_same_q4() {
            let f = Fixture::new("fingerprints");
            let raw = |id: &str, input: &str| {
                format!(
                    "{{\"type\":\"tool_use\",\"id\":\"{id}\",\"name\":\"Read\",\"input\":{input}}}"
                )
            };
            let c = f.file(
                "c.jsonl",
                &[chunk(
                    "m1",
                    "s",
                    1,
                    &[
                        raw("same1", "{\"path\":\"/a\",\"n\":1}"),
                        raw("same2", "  {\"path\":\"/a\",\"n\":1}  "),
                        raw("other", "{\"path\":\"/b\",\"n\":1}"),
                        raw("inner", "{\"path\": \"/a\",\"n\":1}"),
                        raw("order", "{\"n\":1,\"path\":\"/a\"}"),
                        raw("text", "\"  body  \""),
                    ],
                    1,
                )],
            );
            f.ingest("claude", &c);
            let tools = f.db.load_tools().unwrap();
            let fp = |seq_index: i64| {
                tools
                    .iter()
                    .find(|t| t.seq % 100 == seq_index)
                    .and_then(|t| t.fingerprint.clone())
            };
            assert!(
                tools.iter().all(|t| t.fingerprint.is_some()),
                "every object input is fingerprinted"
            );
            assert_eq!(fp(0), fp(1), "outer whitespace only");
            assert_ne!(fp(0), fp(2), "different input");
            assert_ne!(fp(0), fp(3), "interior whitespace is not normalized");
            assert_ne!(fp(0), fp(4), "key order is not normalized");
            assert_eq!(
                fp(5),
                Some(tool_fingerprint(KEY, "Read", Some("  body  ")).unwrap()),
                "a string input keeps its body fingerprint"
            );
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

        /// A Pi assistant message with tool calls: (id, name, raw JSON arguments).
        fn pi_calls(msg: &str, at: u32, calls: &[(&str, &str, &str)]) -> String {
            let blocks: Vec<String> = calls
                .iter()
                .map(|(id, name, args)| {
                    format!("{{\"type\":\"toolCall\",\"id\":\"{id}\",\"name\":\"{name}\",\"arguments\":{args}}}")
                })
                .collect();
            format!(
                "{{\"type\":\"message\",\"id\":\"{msg}\",\"timestamp\":\"2026-10-09T00:00:{at:02}Z\",\"message\":{{\"role\":\"assistant\",\"content\":[{}],\"usage\":{{\"input\":1,\"output\":0,\"cacheRead\":0,\"cacheWrite\":0}}}}}}",
                blocks.join(",")
            )
        }

        /// A Pi toolResult message; `is_error` is raw JSON, or empty to omit it.
        fn pi_result(at: u32, id: &str, is_error: &str) -> String {
            let flag = if is_error.is_empty() {
                String::new()
            } else {
                format!(",\"isError\":{is_error}")
            };
            format!(
                "{{\"type\":\"message\",\"id\":\"r-{id}-{at}\",\"timestamp\":\"2026-10-09T00:00:{at:02}Z\",\"message\":{{\"role\":\"toolResult\",\"toolCallId\":\"{id}\",\"toolName\":\"read\",\"content\":[{{\"type\":\"text\",\"text\":\"x\"}}]{flag}}}}}"
            )
        }

        #[test]
        fn pi_tool_calls_are_settled_by_their_tool_results_q5() {
            let f = Fixture::new("pi-tools");
            let read = |id: &'static str| (id, "read", "{\"path\":\"/a\"}");
            let p = f.file(
                "p.jsonl",
                &[
                    pi_header("pi-s"),
                    pi_calls("m1", 1, &[read("a"), read("b"), read("c"), read("d")]),
                    pi_calls("m2", 2, &[read("e"), read("a")]),
                    pi_result(3, "a", "true"),
                    pi_result(3, "b", "false"),
                    pi_result(3, "c", ""),
                    pi_result(3, "e", "true"),
                    pi_result(3, "e", "false"),
                    pi_result(3, "orphan", "true"),
                ],
            );
            f.ingest("pi", &p);
            let got: Vec<Option<bool>> = calls(&f).into_iter().map(|(_, failed)| failed).collect();
            // a true; b false; c isError absent; d no result; e disagreeing.
            // The repeated id a is the same call; the orphan creates none.
            assert_eq!(got, [Some(true), Some(false), None, None, None]);
            let tools = f.db.load_tools().unwrap();
            assert!(tools
                .iter()
                .all(|t| t.harness == "pi" && t.fingerprint.is_some()));
            assert_eq!(
                tools[0].native_session,
                f.sealed("pi", "pi-s"),
                "the call joins the file's K10 session"
            );
            assert_eq!(f.db.load_usage().unwrap().len(), 2, "usage unchanged");
        }

        #[test]
        fn pi_write_and_edit_are_structured_writes_bash_and_read_are_not_q5() {
            let f = Fixture::new("pi-writes");
            let p = f.file(
                "p.jsonl",
                &[
                    pi_header("pi-w"),
                    pi_calls(
                        "m1",
                        1,
                        &[
                            ("w", "write", "{\"path\":\"/a\"}"),
                            ("e", "edit", "{\"path\":\"/a\"}"),
                            ("b", "bash", "{\"command\":\"ls\"}"),
                            ("r", "read", "{\"path\":\"/a\"}"),
                        ],
                    ),
                ],
            );
            f.ingest("pi", &p);
            let writes: Vec<String> =
                f.db.load_tools()
                    .unwrap()
                    .into_iter()
                    .filter(|t| t.structured_write)
                    .map(|t| t.name)
                    .collect();
            assert_eq!(writes, ["write", "edit"]);
        }

        #[test]
        fn pi_results_appended_later_match_a_fresh_ingest_q5() {
            let read = |id: &'static str| (id, "read", "{\"path\":\"/a\"}");
            let header = vec![
                pi_header("pi-x"),
                pi_calls("m1", 1, &[read("a"), read("b"), read("c")]),
            ];
            let results = vec![
                pi_result(2, "a", "true"),
                pi_result(2, "b", "true"),
                pi_result(2, "c", "true"),
            ];
            let conflict = vec![pi_result(3, "b", "false")];
            let all: Vec<String> = header
                .iter()
                .chain(&results)
                .chain(&conflict)
                .cloned()
                .collect();
            let fresh = Fixture::new("pi-fresh");
            let whole = fresh.file("p.jsonl", &all);
            fresh.ingest("pi", &whole);
            let split = Fixture::new("pi-split");
            let file = split.file("p.jsonl", &header);
            split.ingest("pi", &file);
            split.append(&file, &results);
            split.ingest("pi", &file);
            assert!(calls(&split)
                .iter()
                .all(|(_, failed)| *failed == Some(true)));
            split.append(&file, &conflict);
            split.ingest("pi", &file);
            assert_eq!(calls(&fresh), calls(&split));
            let got: Vec<_> = calls(&split)
                .into_iter()
                .map(|(_, failed)| failed)
                .collect();
            assert_eq!(
                got,
                [Some(true), None, Some(true)],
                "a late conflict is unknown"
            );
        }

        #[test]
        fn codex_partial_replay_and_unsupported_append_track_committed_prefix() {
            let f = Fixture::new("codex-prefix");
            let command = r#"{"type":"event_msg","timestamp":"2026-10-09T00:00:02Z","payload":{"type":"item_completed","item":{"type":"CommandExecution","command":["probe"],"exit_code":1}}}"#.to_string();
            let p = f.file(
                "x.jsonl",
                &[codex_header("cx"), codex_count(9, None), command.clone()],
            );
            f.ingest("codex", &p);
            assert_eq!(f.db.load_codex_observations().unwrap().len(), 1);
            assert!(f.db.load_source_coverage().unwrap()[0].complete);
            let before = f.db.source_state("src-x.jsonl").unwrap();
            let mut tail = fs::OpenOptions::new().append(true).open(&p).unwrap();
            write!(tail, "{command}").unwrap();
            drop(tail);
            f.ingest("codex", &p);
            assert_eq!(f.db.source_state("src-x.jsonl").unwrap(), before);
            assert_eq!(f.db.load_codex_observations().unwrap().len(), 1);
            let mut tail = fs::OpenOptions::new().append(true).open(&p).unwrap();
            writeln!(tail).unwrap();
            drop(tail);
            f.ingest("codex", &p);
            f.ingest("codex", &p);
            assert_eq!(f.db.load_codex_observations().unwrap().len(), 2);
            assert_eq!(
                f.db.load_usage().unwrap().len(),
                1,
                "prefix usage not recounted"
            );
            f.append(&p, &["malformed".into()]);
            f.ingest("codex", &p);
            assert!(!f.db.load_source_coverage().unwrap()[0].complete);
            assert_eq!(
                f.db.load_codex_observations().unwrap().len(),
                2,
                "valid prefix remains visible"
            );
            fs::write(&p, format!("{}\n{command}\n", codex_header("cx"))).unwrap();
            f.ingest("codex", &p);
            let facts = f.db.load_codex_observations().unwrap();
            assert_eq!(facts.len(), 1, "obsolete generation excluded");
            assert_eq!(facts[0].fact.generation, 2);
            assert!(f.db.load_source_coverage().unwrap()[0].complete);
        }

        #[test]
        fn reader_coverage_never_assigns_a_multisession_claude_source_to_the_first_session() {
            let f = Fixture::new("claude-reader-identity");
            let p = f.file(
                "x.jsonl",
                &[
                    r#"{"type":"user","sessionId":"first","message":{"content":"one"}}"#.into(),
                    r#"{"type":"user","sessionId":"second","message":{"content":"two"}}"#.into(),
                ],
            );
            f.ingest("claude", &p);
            let source = &f.db.load_source_coverage().unwrap()[0];
            assert!(
                source.complete,
                "prefix processing is independent of linkage"
            );
            assert!(
                source.native_session.is_none(),
                "no guessed source attribution"
            );
        }

        #[test]
        fn each_codex_file_says_once_that_its_tool_calls_are_not_read_q6() {
            let f = Fixture::new("codex-not-read");
            let x = f.file("x.jsonl", &[codex_header("cx"), codex_count(4, None)]);
            let y = f.file("y.jsonl", &[codex_header("cy"), codex_count(2, None)]);
            f.ingest("codex", &x);
            f.ingest("codex", &y);
            f.append(&x, &[codex_count(6, None)]);
            f.ingest("codex", &x);
            f.ingest("codex", &x);
            let notes = f
                .warnings()
                .into_iter()
                .filter(|w| w == CODEX_TOOLS_NOT_READ)
                .count();
            assert_eq!(notes, 2, "one per Codex file, kept once across reruns");
            assert!(f.db.load_tools().unwrap().is_empty(), "no Codex tool facts");
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
    fn native_command_inputs_outputs_and_metadata_leave_no_canary_in_wal() {
        let dir = std::env::temp_dir().join(format!("dendro-command-wal-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("codex.jsonl");
        let raw = r#"{"type":"session_meta","payload":{"id":"session"}}
{"type":"event_msg","payload":{"type":"item_completed","item":{"type":"CommandExecution","command":["sk-FAKE40A","CONTENT40_ARGS"],"exit_code":1,"stdout":"CONTENT40_STDOUT","stderr":"CONTENT40_STDERR","prompt":"CONTENT40_PROMPT","other":"CONTENT40_EXTRA"}}}
"#;
        fs::write(&path, raw).unwrap();
        let db = dir.join("analytics.duckdb");
        let store = AnalyticsStore::open_path(&db, "synthetic-key-id").unwrap();
        ingest_jsonl_with_key(
            &store,
            "source",
            "codex",
            &path,
            Some(b"QA40_SYNTHETIC_KEY_A_NOT_A_CREDENTIAL"),
        )
        .unwrap();
        assert_eq!(store.load_codex_observations().unwrap().len(), 1);
        assert!(store.load_source_coverage().unwrap()[0].complete);
        let wal = dir.join("analytics.duckdb.wal");
        assert!(
            fs::metadata(&wal).unwrap().len() > 0,
            "nonvacuous pre-checkpoint WAL"
        );
        // Native DuckDB holds its WAL exclusively on Windows. Live WAL byte
        // visibility is unverified there; decoded and closed-database scans
        // below remain required. Unix also inspects the live WAL bytes.
        #[cfg(not(windows))]
        let bytes = fs::read(&wal).unwrap();
        #[cfg(not(windows))]
        assert!(!bytes.is_empty(), "nonvacuous pre-checkpoint WAL");
        let needles = [
            "sk-FAKE40A",
            "CONTENT40_ARGS",
            "CONTENT40_STDOUT",
            "CONTENT40_STDERR",
            "CONTENT40_PROMPT",
            "CONTENT40_EXTRA",
        ];
        for needle in needles {
            assert!(
                !store.contains_text(needle).unwrap(),
                "decoded physical text scan"
            );
            #[cfg(not(windows))]
            assert!(
                !String::from_utf8_lossy(&bytes).contains(needle),
                "WAL canary"
            );
        }
        assert_eq!(fs::read_to_string(&path).unwrap(), raw, "source custody");
        drop(store);
        let persisted = fs::read(&db).unwrap();
        for needle in needles {
            assert!(
                !String::from_utf8_lossy(&persisted).contains(needle),
                "closed database canary"
            );
        }
        let _ = fs::remove_dir_all(&dir);
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
        let stats = ingest_jsonl_with_key(&db, "c1", "claude", &path, None).unwrap();
        assert_eq!(db.latest_usage_totals("c1").unwrap(), (2, 3, 4, 5));
        assert_eq!(db.warning_count().unwrap(), 4);
        assert!(stats.coverage.partial);
        assert_eq!(stats.coverage.malformed, 1);
        assert_eq!(stats.coverage.held_tail, 1);
        assert!(db
            .has_warning(&path.display().to_string(), &stats.coverage.summary())
            .unwrap());
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
        #[cfg(windows)]
        let fixture_root = dendro_core::platform_v1::create_user_private_root_with_choice(
            "dendro-ingest-fixture-",
            &dendro_core::platform_v1::StorageRootChoice::UserProfile,
        )
        .expect("explicit UserProfile staged ingest fixture");
        #[cfg(windows)]
        let dir = fixture_root.path().to_path_buf();
        #[cfg(not(windows))]
        let dir = std::env::temp_dir().join(format!("dendro-source-cols-{}", std::process::id()));
        #[cfg(not(windows))]
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
        #[cfg(windows)]
        let fixture_root = dendro_core::platform_v1::create_user_private_root_with_choice(
            "dendro-ingest-fixture-",
            &dendro_core::platform_v1::StorageRootChoice::UserProfile,
        )
        .expect("explicit UserProfile staged ingest fixture");
        #[cfg(windows)]
        let dir = fixture_root.path().to_path_buf();
        #[cfg(not(windows))]
        let dir = std::env::temp_dir().join(format!("dendro-source-msg-{}", std::process::id()));
        #[cfg(not(windows))]
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
        #[cfg(windows)]
        let fixture_root = dendro_core::platform_v1::create_user_private_root_with_choice(
            "dendro-ingest-fixture-",
            &dendro_core::platform_v1::StorageRootChoice::UserProfile,
        )
        .expect("explicit UserProfile staged ingest fixture");
        #[cfg(windows)]
        let dir = fixture_root.path().to_path_buf();
        #[cfg(not(windows))]
        let dir = std::env::temp_dir().join(format!("dendro-source-id-{}", std::process::id()));
        #[cfg(not(windows))]
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
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
        #[cfg(windows)]
        let fixture_root = dendro_core::platform_v1::create_user_private_root_with_choice(
            "dendro-ingest-fixture-",
            &dendro_core::platform_v1::StorageRootChoice::UserProfile,
        )
        .expect("explicit UserProfile staged ingest fixture");
        #[cfg(windows)]
        let dir = fixture_root.path().to_path_buf();
        #[cfg(not(windows))]
        let dir = std::env::temp_dir().join(format!("dendro-two-roots-{}", std::process::id()));
        #[cfg(not(windows))]
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
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
        #[cfg(windows)]
        let fixture_root = dendro_core::platform_v1::create_user_private_root_with_choice(
            "dendro-ingest-fixture-",
            &dendro_core::platform_v1::StorageRootChoice::UserProfile,
        )
        .expect("explicit UserProfile staged ingest fixture");
        #[cfg(windows)]
        let dir = fixture_root.path().to_path_buf();
        #[cfg(not(windows))]
        let dir = std::env::temp_dir().join(format!("dendro-source-rest-{}", std::process::id()));
        #[cfg(not(windows))]
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
    fn claude_and_pi_extract_skill_invocations() {
        let dir = std::env::temp_dir().join(format!("dendro-skill-stream-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let db = AnalyticsStore::open_path(&dir.join("test.duckdb"), "k").unwrap();

        // 1. Claude session invoking Skill tool and reading a SKILL.md
        let claude_path = dir.join("claude.jsonl");
        let claude_data = concat!(
            "{\"sessionId\":\"c-sess-1\",\"type\":\"assistant\",\"message\":{\"content\":[",
            "{\"type\":\"tool_use\",\"id\":\"call_1\",\"name\":\"Skill\",\"input\":{\"skill\":\"unit-test\"}}",
            "]}}\n",
            "{\"sessionId\":\"c-sess-1\",\"type\":\"assistant\",\"message\":{\"content\":[",
            "{\"type\":\"tool_use\",\"id\":\"call_2\",\"name\":\"view_file\",\"input\":{\"file_path\":\"/path/to/skills/git-workflow/SKILL.md\"}}",
            "]}}\n",
            "{\"sessionId\":\"c-sess-1\",\"type\":\"assistant\",\"message\":{\"content\":[",
            "{\"type\":\"tool_use\",\"id\":\"call_3\",\"name\":\"Skill\",\"input\":{\"skill\":\"sk-ant-secret-token\"}}",
            "]}}\n",
        );
        write(&claude_path, claude_data);
        ingest_jsonl_with_key(&db, "claude-src", "claude", &claude_path, Some(b"key")).unwrap();

        // 2. Pi session invoking use_skill and read
        let pi_path = dir.join("pi.jsonl");
        let pi_data = concat!(
            "{\"sessionId\":\"pi-sess-1\"}\n",
            "{\"sessionId\":\"pi-sess-1\",\"type\":\"message\",\"message\":{\"role\":\"assistant\",\"content\":[",
            "{\"type\":\"toolCall\",\"id\":\"pi_call_1\",\"name\":\"use_skill\",\"arguments\":{\"name\":\"pr-review\"}}",
            "]}}\n",
            "{\"sessionId\":\"pi-sess-1\",\"type\":\"message\",\"message\":{\"role\":\"assistant\",\"content\":[",
            "{\"type\":\"toolCall\",\"id\":\"pi_call_2\",\"name\":\"read\",\"arguments\":{\"path\":\"skills/deploy/SKILL.md\"}}",
            "]}}\n",
        );
        write(&pi_path, pi_data);
        ingest_jsonl_with_key(&db, "pi-src", "pi", &pi_path, Some(b"key")).unwrap();

        let skills = db.load_skills().unwrap();
        assert_eq!(skills.len(), 5);
        for skill in &skills {
            assert!(skill.invoked);
            assert!(!skill.skill.contains("sk-ant-secret-token"));
        }
        assert!(skills.iter().any(|s| s.skill == "unit-test"));
        assert!(skills.iter().any(|s| s.skill == "git-workflow"));
        assert!(skills.iter().any(|s| s.skill == "pr-review"));
        assert!(skills.iter().any(|s| s.skill == "deploy"));
        assert!(skills.iter().any(|s| s.skill.contains("[redacted]")));

        drop(db);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn codex_extracts_skill_from_command_execution() {
        let dir = std::env::temp_dir().join(format!("dendro-skill-codex-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let db = AnalyticsStore::open_path(&dir.join("test.duckdb"), "k").unwrap();

        let codex_path = dir.join("codex.jsonl");
        let codex_data = concat!(
            "{\"session_id\":\"codex-sess-1\"}\n",
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"item_completed\",\"item\":{\"type\":\"CommandExecution\",\"exit_code\":0,\"command\":[\"cat\",\"/opt/skills/build-step/SKILL.md\"]}}}\n",
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"item_completed\",\"item\":{\"type\":\"CommandExecution\",\"exit_code\":0,\"command\":\"node /skills/sk-ant-codex-secret/SKILL.md\"}}}\n",
        );
        write(&codex_path, codex_data);
        ingest_jsonl_with_key(&db, "codex-src", "codex", &codex_path, Some(b"key")).unwrap();

        let skills = db.load_skills().unwrap();
        assert_eq!(skills.len(), 2);
        for skill in &skills {
            assert!(skill.invoked);
            assert!(!skill.skill.contains("sk-ant-codex-secret"));
        }
        assert!(skills.iter().any(|s| s.skill == "build-step"));
        assert!(skills.iter().any(|s| s.skill.contains("[redacted]")));

        drop(db);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn multiple_skills_on_same_line_have_unique_deterministic_keys() {
        let dir = std::env::temp_dir().join(format!("dendro-multi-skill-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let db = AnalyticsStore::open_path(&dir.join("test.duckdb"), "k").unwrap();

        // 1. Codex line with multiple skills in a single command
        let codex_path = dir.join("codex.jsonl");
        let codex_data = concat!(
            "{\"session_id\":\"codex-multi\"}\n",
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"item_completed\",\"item\":{\"type\":\"CommandExecution\",\"exit_code\":0,\"command\":\"cat skills/alpha/SKILL.md skills/beta/SKILL.md\"}}}\n",
        );
        write(&codex_path, codex_data);
        ingest_jsonl_with_key(&db, "codex-src", "codex", &codex_path, Some(b"key")).unwrap();

        // 2. Claude line with multiple tool uses
        let claude_path = dir.join("claude.jsonl");
        let claude_data = concat!(
            "{\"sessionId\":\"claude-multi\",\"type\":\"assistant\",\"message\":{\"content\":[",
            "{\"type\":\"tool_use\",\"id\":\"call_1\",\"name\":\"Skill\",\"input\":{\"skill\":\"gamma\"}},",
            "{\"type\":\"tool_use\",\"id\":\"call_2\",\"name\":\"view_file\",\"input\":{\"file_path\":\"/skills/delta/SKILL.md\"}}",
            "]}}\n",
        );
        write(&claude_path, claude_data);
        ingest_jsonl_with_key(&db, "claude-src", "claude", &claude_path, Some(b"key")).unwrap();

        let skills = db.load_skills().unwrap();
        assert_eq!(skills.len(), 4);

        let alpha = skills.iter().find(|s| s.skill == "alpha").unwrap();
        assert!(alpha.source_key.ends_with(":skill:0"));
        assert!(alpha.seat.starts_with("codex:"));

        let beta = skills.iter().find(|s| s.skill == "beta").unwrap();
        assert!(beta.source_key.ends_with(":skill:1"));
        assert!(beta.seat.starts_with("codex:"));

        let gamma = skills.iter().find(|s| s.skill == "gamma").unwrap();
        assert!(gamma.source_key.ends_with(":skill:0"));
        assert!(gamma.seat.starts_with("claude:"));

        let delta = skills.iter().find(|s| s.skill == "delta").unwrap();
        assert!(delta.source_key.ends_with(":skill:1"));
        assert!(delta.seat.starts_with("claude:"));

        drop(db);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn messages_and_prompts_mentioning_skills_are_ignored() {
        let dir = std::env::temp_dir().join(format!("dendro-skill-ignore-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let db = AnalyticsStore::open_path(&dir.join("test.duckdb"), "k").unwrap();

        // Codex message mentioning SKILL.md
        let codex_path = dir.join("codex.jsonl");
        let codex_data = concat!(
            "{\"session_id\":\"codex-msg\"}\n",
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"item_completed\",\"item\":{\"type\":\"AgentMessage\",\"text\":\"Please refer to skills/secret-agent/SKILL.md\"}}}\n",
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"item_completed\",\"item\":{\"type\":\"UserMessage\",\"text\":\"I want you to use skills/user-prompt/SKILL.md\"}}}\n",
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"item_completed\",\"item\":{\"type\":\"Reasoning\",\"text\":\"Thinking about skills/reasoning-skill/SKILL.md\"}}}\n",
        );
        write(&codex_path, codex_data);
        ingest_jsonl_with_key(&db, "codex-src", "codex", &codex_path, Some(b"key")).unwrap();

        // Claude text message mentioning SKILL.md (not a tool call)
        let claude_path = dir.join("claude.jsonl");
        let claude_data = concat!(
            "{\"sessionId\":\"claude-msg\",\"type\":\"assistant\",\"message\":{\"content\":[",
            "{\"type\":\"text\",\"text\":\"You should read skills/fake-skill/SKILL.md\"}",
            "]}}\n",
        );
        write(&claude_path, claude_data);
        ingest_jsonl_with_key(&db, "claude-src", "claude", &claude_path, Some(b"key")).unwrap();

        let skills = db.load_skills().unwrap();
        assert_eq!(
            skills.len(),
            0,
            "Non-tool messages must not produce skill facts"
        );

        drop(db);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn frozen_sqlite_copy_preserves_source_hashes_and_reads_wal() {
        #[cfg(windows)]
        let root = dendro_core::platform_v1::create_user_private_root_with_choice(
            "dendro-frozen-",
            &dendro_core::platform_v1::StorageRootChoice::UserProfile,
        )
        .expect("explicit UserProfile frozen snapshot fixture");
        #[cfg(windows)]
        let dir = root.path().to_path_buf();
        #[cfg(not(windows))]
        let dir = std::env::temp_dir().join(format!("dendro-frozen-{}", std::process::id()));
        #[cfg(not(windows))]
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
        #[cfg(windows)]
        let snap = dir.join(dendro_core::lifecycle::SNAPSHOT_DIR);
        #[cfg(not(windows))]
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

    #[test]
    fn codex_json_encoded_function_call_arguments_extract_skills() {
        let dir = std::env::temp_dir().join(format!("dendro-codex-fc-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("codex.jsonl");
        let content = concat!(
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"sess-fc\"}}\n",
            "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"exec_command\",\"arguments\":\"{\\\"cmd\\\":\\\"cat skills/build-step/SKILL.md\\\"}\"}}\n"
        );
        fs::write(&path, content).unwrap();
        let db = store();
        ingest_jsonl(&db, "codex-fc", "codex", &path).unwrap();
        let skills = db.load_skills().unwrap();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].seat, "codex:sess-fc");
        assert_eq!(skills[0].skill, "build-step");
        assert!(skills[0].invoked);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_all_8_tool_categories_across_claude_codex_pi() {
        let dir = std::env::temp_dir().join(format!("dendro-cat-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let db = store();

        // 1. Claude session with tool calls covering all categories
        let claude_path = dir.join("claude.jsonl");
        let claude_content = concat!(
            "{\"sessionId\":\"c-sess\",\"type\":\"assistant\",\"message\":{\"content\":[",
            "{\"type\":\"tool_use\",\"id\":\"c_1\",\"name\":\"Write\",\"input\":{\"path\":\"foo.rs\",\"content\":\"// test\"}},",
            "{\"type\":\"tool_use\",\"id\":\"c_2\",\"name\":\"Read\",\"input\":{\"file_path\":\"foo.rs\"}},",
            "{\"type\":\"tool_use\",\"id\":\"c_3\",\"name\":\"bash\",\"input\":{\"command\":\"cargo check\"}},",
            "{\"type\":\"tool_use\",\"id\":\"c_4\",\"name\":\"bash\",\"input\":{\"command\":\"git status\"}},",
            "{\"type\":\"tool_use\",\"id\":\"c_5\",\"name\":\"bash\",\"input\":{\"command\":\"rig queue pop\"}},",
            "{\"type\":\"tool_use\",\"id\":\"c_6\",\"name\":\"bash\",\"input\":{\"command\":\"rig whoami\"}},",
            "{\"type\":\"tool_use\",\"id\":\"c_7\",\"name\":\"bash\",\"input\":{\"command\":\"agentflow-sdlc next\"}},",
            "{\"type\":\"tool_use\",\"id\":\"c_8\",\"name\":\"bash\",\"input\":{\"command\":\"curl -s https://example.com\"}}",
            "]}}\n"
        );
        fs::write(&claude_path, claude_content).unwrap();
        ingest_jsonl(&db, "src-claude", "claude", &claude_path).unwrap();

        // 2. Codex session
        let codex_path = dir.join("codex.jsonl");
        let codex_content = concat!(
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"sess-codex\"}}\n",
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"item_completed\",\"item\":{\"type\":\"CommandExecution\",\"exit_code\":0,\"command\":\"cargo test\"}}}\n",
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"item_completed\",\"item\":{\"type\":\"CommandExecution\",\"exit_code\":0,\"command\":\"git diff\"}}}\n",
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"item_completed\",\"item\":{\"type\":\"CommandExecution\",\"exit_code\":0,\"command\":\"rig ps\"}}}\n",
            "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"exec_command\",\"arguments\":\"{\\\"cmd\\\":\\\"cat skills/build/SKILL.md\\\"}\"}}\n"
        );
        fs::write(&codex_path, codex_content).unwrap();
        ingest_jsonl(&db, "src-codex", "codex", &codex_path).unwrap();

        // 3. Pi session
        let pi_path = dir.join("pi.jsonl");
        let pi_content = concat!(
            "{\"sessionId\":\"pi-sess\"}\n",
            "{\"sessionId\":\"pi-sess\",\"type\":\"message\",\"message\":{\"role\":\"assistant\",\"content\":[",
            "{\"type\":\"toolCall\",\"id\":\"p_1\",\"name\":\"write\",\"arguments\":{\"path\":\"bar.rs\",\"content\":\"\"}},",
            "{\"type\":\"toolCall\",\"id\":\"p_2\",\"name\":\"read\",\"arguments\":{\"path\":\"bar.rs\"}},",
            "{\"type\":\"toolCall\",\"id\":\"p_3\",\"name\":\"bash\",\"arguments\":{\"command\":\"npm test\"}}",
            "]}}\n"
        );
        fs::write(&pi_path, pi_content).unwrap();
        ingest_jsonl(&db, "src-pi", "pi", &pi_path).unwrap();

        let categories = db.load_tool_categories().unwrap();

        // Check Claude categories
        assert_eq!(
            categories.get("src-claude:1:1:0"),
            Some(&ToolCategory::EngCodeEdit)
        ); // Write
        assert_eq!(
            categories.get("src-claude:1:1:1"),
            Some(&ToolCategory::InspectRead)
        ); // Read
        assert_eq!(
            categories.get("src-claude:1:1:2"),
            Some(&ToolCategory::EngBuildTest)
        ); // cargo check
        assert_eq!(
            categories.get("src-claude:1:1:3"),
            Some(&ToolCategory::EngVcs)
        ); // git status
        assert_eq!(
            categories.get("src-claude:1:1:4"),
            Some(&ToolCategory::CoordQueue)
        ); // rig queue pop
        assert_eq!(
            categories.get("src-claude:1:1:5"),
            Some(&ToolCategory::CoordStatus)
        ); // rig whoami
        assert_eq!(
            categories.get("src-claude:1:1:6"),
            Some(&ToolCategory::GovernancePhase)
        ); // agentflow-sdlc
        assert_eq!(
            categories.get("src-claude:1:1:7"),
            Some(&ToolCategory::Other)
        ); // curl

        // Check Codex categories
        assert_eq!(
            categories.get("src-codex:1:2:codex:0"),
            Some(&ToolCategory::EngBuildTest)
        ); // cargo test
        assert_eq!(
            categories.get("src-codex:1:3:codex:0"),
            Some(&ToolCategory::EngVcs)
        ); // git diff
        assert_eq!(
            categories.get("src-codex:1:4:codex:0"),
            Some(&ToolCategory::CoordStatus)
        ); // rig ps
        assert_eq!(
            categories.get("src-codex:1:5:codex:0"),
            Some(&ToolCategory::InspectRead)
        ); // cat

        // Check Pi categories
        assert_eq!(
            categories.get("src-pi:1:2:0"),
            Some(&ToolCategory::EngCodeEdit)
        ); // write
        assert_eq!(
            categories.get("src-pi:1:2:1"),
            Some(&ToolCategory::InspectRead)
        ); // read
        assert_eq!(
            categories.get("src-pi:1:2:2"),
            Some(&ToolCategory::EngBuildTest)
        ); // npm test

        let _ = fs::remove_dir_all(&dir);
    }
}

pub fn walk_jsonl(root: &Path) -> Vec<PathBuf> {
    collect_jsonl(root)
        .map(|found| found.files)
        .unwrap_or_default()
}

/// Files found under a root, and how many entries or directories below the
/// root could not be listed or inspected and were skipped.
pub(crate) struct JsonlDiscovery {
    pub files: Vec<PathBuf>,
    pub skipped: u64,
}

/// An unreadable root is an error. One bad entry below it is skipped and
/// counted, so the readable files around it are still discovered.
pub(crate) fn walk_jsonl_checked(root: &Path) -> Result<JsonlDiscovery> {
    collect_jsonl(root)
}

#[cfg(test)]
thread_local! {
    static UNINSPECTABLE: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
    static PARSED_LINES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Tests make one named entry fail at the real inspection boundary, as a file
/// that vanishes or turns unreadable between listing and inspection does.
#[cfg(test)]
pub(crate) fn make_uninspectable(path: Option<PathBuf>) {
    UNINSPECTABLE.with(|slot| *slot.borrow_mut() = path);
}

fn inspect_entry(path: &Path) -> std::io::Result<fs::Metadata> {
    #[cfg(test)]
    if UNINSPECTABLE.with(|slot| slot.borrow().as_deref() == Some(path)) {
        return Err(std::io::ErrorKind::NotFound.into());
    }
    fs::symlink_metadata(path)
}

fn collect_jsonl(root: &Path) -> Result<JsonlDiscovery> {
    let mut out = Vec::new();
    let mut skipped = 0u64;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if dir == root => return Err(error.into()),
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => {
                    skipped += 1;
                    continue;
                }
            };
            let path = entry.path();
            let meta = match inspect_entry(&path) {
                Ok(meta) => meta,
                Err(_) => {
                    skipped += 1;
                    continue;
                }
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
    Ok(JsonlDiscovery {
        files: out,
        skipped,
    })
}

#[cfg(test)]
mod sealed_header_tests {
    #[test]
    fn partial_first_header_is_not_identity_evidence_beyond_the_complete_line_cutoff() {
        let header = br#"{"type":"session_meta","payload":{"id":"future-session"}}"#;
        let partial = super::file_session("codex", &mut &header[..], "source").unwrap();
        assert!(!partial.from_header);
        let mut complete = header.to_vec();
        complete.push(b'\n');
        let complete = super::file_session("codex", &mut &complete[..], "source").unwrap();
        assert!(complete.from_header);
        assert_eq!(complete.raw, "future-session");
    }
}

#[cfg(test)]
mod hostile_input_tests {
    use super::*;
    use crate::sdk::ReadLimits;

    const LINE: &str = r#"{"type":"system","sessionId":"s"}"#;

    fn limits(encoded_bytes: u64, per_batch: usize) -> ReadLimits {
        ReadLimits {
            encoded_bytes,
            nesting_depth: 64,
            decoded_records_per_batch: per_batch,
            line_attempts: None,
        }
    }

    fn run(
        name: &str,
        body: &str,
        limits: &ReadLimits,
    ) -> (AnalyticsStore, std::path::PathBuf, FileStats) {
        let dir =
            std::env::temp_dir().join(format!("dendro-hostile-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let store = AnalyticsStore::open_path(&dir.join("a.duckdb"), "key").unwrap();
        let path = dir.join("claude.jsonl");
        fs::write(&path, body).unwrap();
        let stats = ingest_jsonl_limited(
            &store,
            "claude",
            "claude",
            &path,
            None,
            None,
            || Ok(()),
            limits,
        )
        .unwrap();
        (store, path, stats)
    }

    #[test]
    fn oversize_line_is_quarantined_and_following_lines_ingest() {
        let big = "x".repeat(5000);
        let body = format!("{LINE}\n{{\"pad\":\"{big}\"}}\n{LINE}\n");
        let limits = limits(200, 1024);
        let (store, path, stats) = run("oversize", &body, &limits);
        assert_eq!(stats.records_read, 3);
        let shown = path.display().to_string();
        assert!(store
            .has_warning(&shown, "encoded_record_exceeds_adapter_limit")
            .unwrap());
        let (_, offset, _) = store.source_state("claude").unwrap().unwrap();
        assert_eq!(offset as usize, body.len());
        // A re-run reads nothing new and does not move the watermark.
        let again = ingest_jsonl_limited(
            &store,
            "claude",
            "claude",
            &path,
            None,
            None,
            || Ok(()),
            &limits,
        )
        .unwrap();
        assert_eq!(again.records_read, 0);
        let (_, offset, _) = store.source_state("claude").unwrap().unwrap();
        assert_eq!(offset as usize, body.len());
    }

    #[test]
    fn capped_line_reader_never_retains_more_than_the_limit() {
        let body = format!("{}\nok\n", "y".repeat(100_000));
        let mut reader = BufReader::with_capacity(64, body.as_bytes());
        let mut hasher = Sha256::new();
        match read_capped_line(&mut reader, 50, &mut hasher).unwrap() {
            CappedLine::Oversize { len } => assert_eq!(len, 100_001),
            _ => panic!("expected oversize"),
        }
        match read_capped_line(&mut reader, 50, &mut hasher).unwrap() {
            CappedLine::Line(line) => assert_eq!(line, b"ok\n"),
            _ => panic!("expected line"),
        }
        let mut whole = Sha256::new();
        whole.update(body.as_bytes());
        assert_eq!(hasher.finalize(), whole.finalize());
    }

    #[test]
    fn unterminated_oversize_tail_is_not_committed() {
        let body = format!("{LINE}\n{}", "z".repeat(5000));
        let (store, _, _) = run("tail", &body, &limits(200, 1024));
        let (_, offset, _) = store.source_state("claude").unwrap().unwrap();
        assert_eq!(offset as usize, LINE.len() + 1);
    }

    #[test]
    fn depth_64_is_accepted_and_65_is_quarantined() {
        let nest = |n: usize| format!("{}{}", "[".repeat(n), "]".repeat(n));
        let body = format!("{}\n{}\n", nest(64), nest(65));
        let (store, path, stats) = run("depth", &body, &limits(8 * 1024, 1024));
        assert_eq!(stats.records_read, 2);
        let shown = path.display().to_string();
        assert!(store
            .has_warning(&shown, "nesting_depth_exceeds_limit")
            .unwrap());
        let only_64 = format!("{}\n", nest(64));
        let (store, path, _) = run("depth64", &only_64, &limits(8 * 1024, 1024));
        assert!(!store
            .has_warning(&path.display().to_string(), "nesting_depth_exceeds_limit")
            .unwrap());
        // Brackets inside strings do not count.
        let quoted = format!("{{\"s\":\"{}\"}}\n", "[".repeat(200));
        let (store, path, _) = run("depthstr", &quoted, &limits(8 * 1024, 1024));
        assert!(!store
            .has_warning(&path.display().to_string(), "nesting_depth_exceeds_limit")
            .unwrap());
    }

    #[test]
    fn batch_cap_continues_without_losing_records() {
        let body = format!("{LINE}\n").repeat(10);
        let (store, _, stats) = run("batch", &body, &limits(8 * 1024, 3));
        assert_eq!(stats.records_read, 10);
        let (_, offset, _) = store.source_state("claude").unwrap().unwrap();
        assert_eq!(offset as usize, body.len());
    }

    fn read_facts(store: &AnalyticsStore) -> String {
        let mut usage: Vec<_> = store
            .load_usage()
            .unwrap()
            .iter()
            .map(|fact| format!("{fact:?}"))
            .collect();
        let mut tools: Vec<_> = store
            .load_tools()
            .unwrap()
            .iter()
            .map(|fact| format!("{fact:?}"))
            .collect();
        let mut messages: Vec<_> = store
            .load_messages()
            .unwrap()
            .iter()
            .map(|fact| format!("{fact:?}"))
            .collect();
        usage.sort();
        tools.sort();
        messages.sort();
        format!("{usage:#?}\n{tools:#?}\n{messages:#?}")
    }

    /// Tool calls and their results fall on opposite sides of a batch cap, and
    /// every fourth call fails, so pairing has to survive the batch boundary.
    fn conversation(rounds: usize) -> String {
        let mut body = String::new();
        for round in 0..rounds {
            let failed = round % 4 == 0;
            body.push_str(&format!(
                "{{\"type\":\"assistant\",\"sessionId\":\"s\",\"timestamp\":\"2026-10-09T00:00:01Z\",\"message\":{{\"id\":\"a{round}\",\"role\":\"assistant\",\"content\":[{{\"type\":\"tool_use\",\"id\":\"t{round}\",\"name\":\"Read\",\"input\":{{\"file_path\":\"f{round}\"}}}}],\"usage\":{{\"input_tokens\":3,\"output_tokens\":4,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0}}}}}}\n"
            ));
            body.push_str(&format!(
                "{{\"type\":\"user\",\"sessionId\":\"s\",\"timestamp\":\"2026-10-09T00:00:02Z\",\"message\":{{\"role\":\"user\",\"content\":[{{\"type\":\"tool_result\",\"tool_use_id\":\"t{round}\",\"is_error\":{failed},\"content\":\"ok\"}}]}}}}\n"
            ));
            body.push_str(
                "{\"type\":\"user\",\"sessionId\":\"s\",\"timestamp\":\"2026-10-09T00:00:03Z\",\"message\":{\"role\":\"user\",\"content\":\"hello\"}}\n",
            );
        }
        body
    }

    fn parsed_lines() -> u64 {
        PARSED_LINES.with(|parsed| parsed.get())
    }

    #[test]
    fn a_multi_batch_read_matches_a_single_batch_read_and_parses_each_line_once() {
        // Many small batches against one batch over the same file.
        let rounds = 40;
        let body = conversation(rounds);
        let lines = body.lines().count() as u64;
        let before = parsed_lines();
        let (batched, path, stats) = run("many-batches", &body, &limits(8 * 1024, 7));
        let batched_parses = parsed_lines() - before;
        let before = parsed_lines();
        let (single, _, whole) = run("one-batch", &body, &limits(8 * 1024, 1 << 20));
        let single_parses = parsed_lines() - before;
        assert_eq!(batched_parses, lines, "every line is parsed exactly once");
        assert_eq!(single_parses, lines);
        assert_eq!(stats.records_read, whole.records_read);
        assert_eq!(stats.usage_records, whole.usage_records);
        assert_eq!(read_facts(&batched), read_facts(&single));
        let failed = batched
            .load_tools()
            .unwrap()
            .iter()
            .filter(|tool| tool.failed == Some(true))
            .count();
        assert_eq!(failed, rounds.div_ceil(4));
        let (_, offset, digest) = batched.source_state("claude").unwrap().unwrap();
        assert_eq!(offset as usize, body.len());
        assert_eq!(
            digest,
            single.source_state("claude").unwrap().unwrap().2,
            "the carried hash equals a whole-file hash"
        );
        // An idle re-run reads the prefix once and adds nothing.
        let again = ingest_jsonl_limited(
            &batched,
            "claude",
            "claude",
            &path,
            None,
            None,
            || Ok(()),
            &limits(8 * 1024, 7),
        )
        .unwrap();
        assert_eq!(again.records_read, 0);
        assert_eq!(read_facts(&batched), read_facts(&single));
    }

    #[cfg(any(windows, target_os = "linux"))]
    #[test]
    fn idle_reruns_of_a_defective_file_add_no_warning_rows() {
        let body = format!("{LINE}\nnot-json\n{LINE}\n");
        // A private root: only the staged open serves direct runtime queries.
        let root = dendro_core::platform_v1::create_user_private_root_with_choice(
            "dendro-idle-",
            &dendro_core::platform_v1::StorageRootChoice::UserProfile,
        )
        .unwrap();
        let db = root.path().join("a.duckdb");
        drop(AnalyticsStore::open_path(&db, "key").unwrap());
        let store = AnalyticsStore::open_staged(&db).unwrap();
        let path = root.path().join("claude.jsonl");
        fs::write(&path, &body).unwrap();
        let first = ingest_jsonl_limited(
            &store,
            "claude",
            "claude",
            &path,
            None,
            None,
            || Ok(()),
            &limits(8 * 1024, 1024),
        )
        .unwrap();
        assert!(first.coverage.partial);
        let rerun = |store: &AnalyticsStore| {
            ingest_jsonl_limited(
                store,
                "claude",
                "claude",
                &path,
                None,
                None,
                || Ok(()),
                &limits(8 * 1024, 1024),
            )
            .unwrap()
        };
        let rows = store.warning_count().unwrap();
        assert!(rows > 0);
        for _ in 0..2 {
            let again = rerun(&store);
            assert_eq!(again.records_read, 0);
        }
        assert_eq!(store.warning_count().unwrap(), rows);
        // New bytes that are defective in the same generation add no second
        // partial-generation row.
        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        std::io::Write::write_all(&mut file, b"still-not-json\n").unwrap();
        let grown = rerun(&store);
        assert_eq!(grown.records_read, 1);
        let partial_rows: i64 = store
            .with_private_connection(|conn| {
                Ok(conn.query_row(
                    "SELECT COUNT(*) FROM warnings WHERE reason LIKE 'coverage-partial generation=%'",
                    [],
                    |row| row.get(0),
                )?)
            })
            .unwrap();
        assert_eq!(partial_rows, 1);
    }

    #[test]
    fn registry_limits_never_exceed_the_global_ceilings() {
        for source in ["claude", "codex", "pi", "grok", "unknown"] {
            let limits = crate::capture_registry_v1::Registry::reader_limits(source);
            assert!(limits.encoded_bytes <= crate::sdk::GLOBAL_MAX_RECORD_BYTES);
            assert!(limits.nesting_depth <= crate::sdk::GLOBAL_MAX_DEPTH);
            assert!(limits.decoded_records_per_batch <= crate::sdk::GLOBAL_MAX_BATCH_RECORDS);
        }
    }
}
