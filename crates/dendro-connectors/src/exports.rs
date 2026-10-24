//! Explicit, version-bounded public export adapters. No private storage autodiscovery.
use crate::sdk::ExportRoot;
use anyhow::{bail, ensure, Result};
use chrono::{DateTime, Utc};
use dendro_core::{
    AnalyticsStore, CanonicalEvent, CanonicalSession, CanonicalToolCall, ToolFact, UsageFact,
    Watermark,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy)]
pub(crate) enum Format {
    Cursor,
    Windsurf,
    Goose,
    Devin,
}

/// Kinds `ExportConnector::cursor` accepts. `unsupported-cursor-record` is not a kind.
pub(crate) const CURSOR_RECORD_KINDS: &[&str] =
    &["assistant", "result", "system", "tool_call", "user"];
/// Cascade hook names and transcript `type` values `ExportConnector::windsurf` accepts.
pub(crate) const WINDSURF_RECORD_KINDS: &[&str] = &[
    "code_action",
    "planner_response",
    "post_cascade_response",
    "post_cascade_response_with_transcript",
    "post_mcp_tool_use",
    "post_read_code",
    "post_run_command",
    "post_write_code",
    "user_input",
];
/// Goose message roles and content `type` values. The offset-0 header is positional, not a kind.
pub(crate) const GOOSE_RECORD_KINDS: &[&str] = &[
    "assistant",
    "document",
    "image",
    "text",
    "thinking",
    "toolRequest",
    "toolResponse",
    "user",
];
/// ATIF step `source` values `ExportConnector::devin` accepts.
pub(crate) const DEVIN_RECORD_KINDS: &[&str] = &["agent", "system", "user"];

/// Facts preserve unknown timestamps and absent usage; raw transcripts are never returned.
pub struct ExportBatch {
    pub usage: Vec<UsageFact>,
    pub tools: Vec<ToolFact>,
    pub warnings: Vec<&'static str>,
    pub watermark: Watermark,
    baseline: dendro_core::FileBaselineAttempt,
}

pub(crate) struct ExportConnector {
    root: ExportRoot,
    key: Vec<u8>,
    harness: &'static str,
    format: Format,
}
impl ExportConnector {
    pub(crate) fn new(root: PathBuf, key: Vec<u8>, harness: &'static str, format: Format) -> Self {
        Self {
            root: ExportRoot::new(root),
            key,
            harness,
            format,
        }
    }
    pub(crate) fn discover(&self) -> Result<Vec<PathBuf>> {
        self.root.discover()
    }
    pub(crate) fn read_batch(&self, path: &Path, prior: Option<&Watermark>) -> Result<ExportBatch> {
        ensure!(!self.key.is_empty(), "export privacy key must not be empty");
        self.read_batch_with_cutoff(path, prior, None)
    }
    fn read_batch_with_cutoff(
        &self,
        path: &Path,
        prior: Option<&Watermark>,
        cutoff: Option<&dendro_core::FileCutoff>,
    ) -> Result<ExportBatch> {
        let limits = crate::capture_registry_v1::Registry::reader_limits(self.harness);
        let raw = self
            .root
            .read_bounded(path, prior, cutoff, &limits, |_| true)?;
        if matches!(self.format, Format::Goose) && prior.is_none() && raw.watermark.byte_offset > 0
        {
            ensure!(
                raw.records.first().is_some_and(|(offset, _)| *offset == 0),
                "unsupported Goose legacy header"
            );
        }
        let mut batch = ExportBatch {
            baseline: dendro_core::FileBaselineAttempt {
                attempted_at: Utc::now().to_rfc3339(),
                metadata: raw.metadata,
            },
            usage: Vec::new(),
            tools: Vec::new(),
            warnings: raw.warnings,
            watermark: raw.watermark,
        };
        match self.format {
            Format::Cursor | Format::Windsurf => batch
                .warnings
                .push("token-usage-unavailable-in-public-export"),
            Format::Goose => batch
                .warnings
                .push("goose-legacy-header-usage-only-context-window-unknown"),
            Format::Devin => batch
                .warnings
                .push("devin-atif-export-only-not-cloud-webhooks"),
        }
        for (offset, value) in raw.records {
            match self.format {
                Format::Cursor => self.cursor(&value, offset, &mut batch)?,
                Format::Windsurf => self.windsurf(&value, offset, &mut batch)?,
                Format::Goose => self.goose(&value, offset, &mut batch)?,
                Format::Devin => self.devin(&value, offset, &mut batch)?,
            }
        }
        if batch.usage.is_empty() && prior.is_none() {
            batch.warnings.push("token-usage-no-data");
        }
        batch.warnings.sort_unstable();
        batch.warnings.dedup();
        Ok(batch)
    }
    fn session(&self, native: &str) -> String {
        dendro_core::opaque_id(
            &self.key,
            &format!("export-session:{}", self.harness),
            native,
        )
    }
    fn source_key(&self, batch: &ExportBatch, offset: u64, ordinal: usize, kind: &str) -> String {
        dendro_core::opaque_id(
            &self.key,
            "export-record",
            &format!(
                "{}:{}:{offset}:{ordinal}:{kind}",
                self.harness, batch.watermark.file_path
            ),
        )
    }
    #[allow(clippy::too_many_arguments)]
    fn tool(
        &self,
        batch: &mut ExportBatch,
        offset: u64,
        ordinal: usize,
        native: &str,
        name: &str,
        ts: Option<DateTime<Utc>>,
        failed: Option<bool>,
    ) {
        // No arbitrary provider labels, arguments, command previews, or source text survive.
        let name = match name {
            "readToolCall" | "read_file" | "post_read_code" => "read_file",
            "writeToolCall" | "write_file" | "code_action" | "post_write_code" => "write_file",
            "shellToolCall" | "shell" | "developer__shell" | "post_run_command" => "shell",
            "edit" | "developer__text_editor" => "edit_file",
            "post_mcp_tool_use" => "mcp_tool",
            _ => "other",
        };
        batch.tools.push(ToolFact {
            source_key: self.source_key(batch, offset, ordinal, "tool"),
            harness: self.harness.into(),
            native_session: self.session(native),
            ts,
            seq: offset as i64,
            name: name.into(),
            fingerprint: None,
            failed,
            structured_write: matches!(name, "write_file" | "edit_file"),
            generation: 1,
        });
    }
    #[allow(clippy::too_many_arguments)]
    fn usage(
        &self,
        batch: &mut ExportBatch,
        offset: u64,
        ordinal: usize,
        native: &str,
        ts: Option<DateTime<Utc>>,
        input: i64,
        output: i64,
        cache: i64,
        cost: Option<f64>,
    ) {
        batch.usage.push(UsageFact {
            source_key: self.source_key(batch, offset, ordinal, "usage"),
            harness: self.harness.into(),
            native_session: self.session(native),
            message_id: None,
            ts,
            seq: offset as i64,
            input_uncached: input,
            output,
            cache_read: cache,
            cache_write: 0,
            reasoning: 0,
            currency_total: cost,
            generation: 1,
            predecessor_source_key: None,
            predecessor_reason: None,
            prior_input_tokens: None,
            prior_cached_input_tokens: None,
            prior_cache_write_input_tokens: None,
            prior_output_tokens: None,
            prior_reasoning_output_tokens: None,
        });
    }
    fn cursor(&self, value: &Value, offset: u64, batch: &mut ExportBatch) -> Result<()> {
        let native = required_str(value, "session_id")?;
        match required_str(value, "type")? {
            "system" if value["subtype"] == "init" => {}
            "user" | "assistant" | "result" => {}
            "tool_call" => {
                if value["subtype"] == "started" {
                    return Ok(());
                }
                ensure!(
                    value["subtype"] == "completed",
                    "unsupported Cursor tool lifecycle"
                );
                required_str(value, "call_id")?;
                let object = value["tool_call"]
                    .as_object()
                    .ok_or_else(|| anyhow::anyhow!("unsupported Cursor tool shape"))?;
                ensure!(object.len() == 1, "unsupported Cursor tool shape");
                let (name, detail) = object.iter().next().expect("one entry");
                let failed = if detail.pointer("/result/success").is_some() {
                    Some(false)
                } else if detail.pointer("/result/error").is_some() {
                    Some(true)
                } else {
                    None
                };
                let ts = value["timestamp_ms"]
                    .as_i64()
                    .and_then(DateTime::from_timestamp_millis);
                self.tool(batch, offset, 0, native, name, ts, failed);
            }
            _ => batch.warnings.push("unsupported-cursor-record"),
        }
        Ok(())
    }
    fn windsurf(&self, value: &Value, offset: u64, batch: &mut ExportBatch) -> Result<()> {
        // Transcript has no native session field: the export pathname defines one trajectory.
        let native = batch.watermark.file_path.clone();
        if let Some(action) = value["agent_action_name"].as_str() {
            ensure!(
                value["tool_info"].is_object(),
                "unsupported Cascade hook shape"
            );
            match action {
                "post_read_code" | "post_write_code" | "post_run_command" | "post_mcp_tool_use" => {
                    self.tool(batch, offset, 0, &native, action, None, None)
                }
                // Never follow transcript_path or arbitrary file references from an export.
                "post_cascade_response" | "post_cascade_response_with_transcript" => {}
                _ => batch.warnings.push("unsupported-cascade-hook"),
            }
        } else {
            let kind = required_str(value, "type")?;
            ensure!(
                value["status"].is_string(),
                "unsupported Cascade transcript shape"
            );
            match kind {
                "user_input" | "planner_response" => {}
                "code_action" if value["status"] == "done" => {
                    self.tool(batch, offset, 0, &native, kind, None, Some(false))
                }
                _ => batch.warnings.push("unsupported-cascade-step"),
            }
        }
        Ok(())
    }
    fn goose(&self, value: &Value, offset: u64, batch: &mut ExportBatch) -> Result<()> {
        let native = batch.watermark.file_path.clone();
        if offset == 0 {
            ensure!(
                value["working_dir"].is_string() && value["message_count"].is_u64(),
                "unsupported Goose legacy header; SQLite/modern exports are not supported"
            );
            // Legacy totals occur only in the first header; prefer lifetime totals, never add current-turn counts.
            let (input, output) = if !value["accumulated_input_tokens"].is_null()
                || !value["accumulated_output_tokens"].is_null()
            {
                (
                    counter(value, "accumulated_input_tokens")?,
                    counter(value, "accumulated_output_tokens")?,
                )
            } else {
                (
                    counter(value, "input_tokens")?,
                    counter(value, "output_tokens")?,
                )
            };
            if let (Some(input), Some(output)) = (input, output) {
                let ts = value["created_at"]
                    .as_str()
                    .and_then(dendro_core::parse_utc_time);
                self.usage(batch, offset, 0, &native, ts, input, output, 0, None);
            } else {
                batch.warnings.push("goose-header-token-usage-unknown");
            }
            return Ok(());
        }
        let role = required_str(value, "role")?;
        ensure!(
            matches!(role, "user" | "assistant"),
            "unsupported Goose message role"
        );
        let ts = value["created"]
            .as_i64()
            .and_then(|n| DateTime::from_timestamp(n, 0));
        let content = value["content"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("unsupported Goose content shape"))?;
        for (ordinal, item) in content.iter().enumerate() {
            match item["type"].as_str() {
                Some("toolRequest") => {
                    required_str(item, "id")?;
                    match item.pointer("/toolCall/status").and_then(Value::as_str) {
                        Some("success") => {}
                        Some("error") => {
                            batch.warnings.push("goose-tool-request-error-not-counted");
                            continue;
                        }
                        _ => bail!("unsupported Goose tool request status"),
                    }
                    let name = item
                        .pointer("/toolCall/value/name")
                        .and_then(Value::as_str)
                        .ok_or_else(|| anyhow::anyhow!("unsupported Goose tool request"))?;
                    self.tool(batch, offset, ordinal, &native, name, ts, None);
                }
                Some("toolResponse" | "text" | "thinking" | "image" | "document") => {}
                _ => batch.warnings.push("unsupported-goose-content"),
            }
        }
        Ok(())
    }
    fn devin(&self, value: &Value, offset: u64, batch: &mut ExportBatch) -> Result<()> {
        // One compact ATIF trajectory per file; immutable snapshot, not a webhook protocol.
        ensure!(
            offset == 0,
            "ATIF export must contain one trajectory per file"
        );
        ensure!(
            value["schema_version"] == "ATIF-v1.7",
            "unsupported Devin export; expected compact ATIF-v1.7 trajectory"
        );
        let native = required_str(value, "session_id")?;
        let steps = value["steps"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("missing ATIF steps"))?;
        ensure!(
            !steps.is_empty() && steps.len() <= 4096,
            "unsupported ATIF step count"
        );
        for (ordinal, step) in steps.iter().enumerate() {
            ensure!(
                step["step_id"].as_u64() == Some(ordinal as u64 + 1),
                "ATIF step IDs must be sequential"
            );
            ensure!(
                step["is_copied_context"] != true,
                "copied ATIF context is unsupported"
            );
            let source = required_str(step, "source")?;
            ensure!(
                matches!(source, "system" | "user" | "agent"),
                "unsupported ATIF source"
            );
            if source != "agent" {
                continue;
            }
            let ts = step["timestamp"]
                .as_str()
                .and_then(dendro_core::parse_utc_time);
            if let Some(metrics) = step.get("metrics").filter(|v| !v.is_null()) {
                let input = counter(metrics, "prompt_tokens")?;
                let output = counter(metrics, "completion_tokens")?;
                // ATIF does not guarantee cached-token inclusion semantics across exporters.
                // Retain prompt total as input, and do not double-count a cached component.
                ensure!(
                    counter(metrics, "cached_tokens")?.is_none_or(|cached| cached == 0),
                    "ATIF cached prompt inclusion is unproven; nonzero cached_tokens unsupported"
                );
                let cost = metrics
                    .get("cost")
                    .filter(|v| !v.is_null())
                    .map(|v| {
                        v.as_f64()
                            .filter(|n| n.is_finite() && *n >= 0.0)
                            .ok_or_else(|| anyhow::anyhow!("invalid ATIF cost"))
                    })
                    .transpose()?;
                if let (Some(input), Some(output)) = (input, output) {
                    self.usage(batch, offset, ordinal, native, ts, input, output, 0, cost);
                } else {
                    batch.warnings.push("atif-step-token-usage-unknown");
                }
            }
            if let Some(tools) = step.get("tool_calls").filter(|v| !v.is_null()) {
                let tools = tools
                    .as_array()
                    .ok_or_else(|| anyhow::anyhow!("invalid ATIF tools"))?;
                ensure!(tools.len() <= 4096, "too many ATIF tools");
                for (index, tool) in tools.iter().enumerate() {
                    let name = required_str(tool, "function_name")?;
                    required_str(tool, "tool_call_id")?;
                    self.tool(
                        batch,
                        offset,
                        ordinal * 4096 + index,
                        native,
                        name,
                        ts,
                        None,
                    );
                }
            }
        }
        Ok(())
    }
    pub(crate) fn extract(
        &self,
        path: &Path,
        prior: Option<&Watermark>,
    ) -> Result<(Vec<CanonicalEvent>, Watermark)> {
        let batch = self.read_batch(path, prior)?;
        let mut events = Vec::new();
        for fact in batch.usage {
            let (Some(ts), Some(cost)) = (fact.ts, fact.currency_total) else {
                bail!("legacy CanonicalSession cannot represent unknown timestamp/cost; use read_batch or ingest_file");
            };
            events.push(CanonicalEvent::SessionUpdated(CanonicalSession {
                session_id: fact.native_session,
                harness_name: fact.harness,
                project_path: String::new(),
                project_name: String::new(),
                started_at: ts,
                ended_at: None,
                duration_seconds: None,
                user_turns_count: 0,
                assistant_turns_count: 0,
                tool_calls_count: 0,
                input_tokens: fact.input_uncached as u64,
                output_tokens: fact.output as u64,
                estimated_cost_usd: cost,
                status: "usage_delta".into(),
                orchestration: None,
            }));
        }
        for fact in batch.tools {
            let Some(ts) = fact.ts else {
                bail!("legacy CanonicalToolCall cannot represent unknown timestamp; use read_batch or ingest_file");
            };
            events.push(CanonicalEvent::ToolExecuted(CanonicalToolCall {
                call_id: fact.source_key,
                session_id: fact.native_session,
                tool_name: fact.name,
                executed_at: ts,
                execution_time_ms: None,
                status: match fact.failed {
                    Some(true) => "failed",
                    Some(false) => "completed",
                    None => "unknown",
                }
                .into(),
                error_message: None,
                command_preview: None,
            }));
        }
        Ok((events, batch.watermark))
    }
    pub(crate) fn ingest_file(
        &self,
        store: &AnalyticsStore,
        path: &Path,
    ) -> Result<Vec<&'static str>> {
        self.ingest_file_with_cutoff(store, path, None)
            .map(|(warnings, _)| warnings)
    }
    pub(crate) fn ingest_file_with_cutoff(
        &self,
        store: &AnalyticsStore,
        path: &Path,
        cutoff: Option<&dendro_core::FileCutoff>,
    ) -> Result<(Vec<&'static str>, Watermark)> {
        ensure!(!self.key.is_empty(), "export privacy key must not be empty");
        let resolved = path
            .canonicalize()
            .map_err(|_| anyhow::anyhow!("export unavailable"))?;
        let identity = hex::encode(Sha256::digest(resolved.as_os_str().as_encoded_bytes()));
        let source = dendro_core::opaque_id(
            &self.key,
            "export-source",
            &format!("{}:{identity}", self.harness),
        );
        let mut prior = store
            .source_state(&source)?
            .map(|(_, offset, digest)| Watermark {
                file_path: identity,
                byte_offset: offset as u64,
                last_modified: 0,
                session_id: Some(digest),
            });
        let mut warnings = std::collections::BTreeSet::new();
        loop {
            let batch = self.read_batch_with_cutoff(path, prior.as_ref(), cutoff)?;
            let old_offset = prior.as_ref().map_or(0, |w| w.byte_offset);
            warnings.extend(batch.warnings);
            store.commit_file_batch_observed(
                &source,
                self.harness,
                &source,
                1,
                i64::try_from(batch.watermark.byte_offset)?,
                batch.watermark.session_id.as_deref().expect("SDK digest"),
                None,
                &batch.usage,
                &batch.tools,
                &[],
                &dendro_core::ReaderState {
                    complete: false,
                    native_session: None,
                },
                &batch.baseline,
            )?;
            let done = batch.watermark.byte_offset == old_offset;
            prior = Some(batch.watermark);
            if done {
                break;
            }
        }
        Ok((
            warnings.into_iter().collect(),
            prior.expect("at least one batch"),
        ))
    }
}
fn required_str<'a>(value: &'a Value, field: &str) -> Result<&'a str> {
    value[field]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("unsupported export: missing required string"))
}
fn counter(value: &Value, field: &str) -> Result<Option<i64>> {
    match value.get(field).filter(|v| !v.is_null()) {
        None => Ok(None),
        Some(v) => {
            Ok(Some(v.as_i64().filter(|n| *n >= 0).ok_or_else(|| {
                anyhow::anyhow!("invalid export token counter")
            })?))
        }
    }
}

macro_rules! export_connector {
    ($name:ident, $harness:literal, $format:ident) => {
        pub struct $name {
            inner: crate::exports::ExportConnector,
        }
        impl $name {
            /// Only explicit, authorized export directories are read; no implicit HOME discovery.
            pub fn new(
                root: impl Into<std::path::PathBuf>,
                privacy_key: impl Into<Vec<u8>>,
            ) -> Self {
                Self {
                    inner: crate::exports::ExportConnector::new(
                        root.into(),
                        privacy_key.into(),
                        $harness,
                        crate::exports::Format::$format,
                    ),
                }
            }
            pub fn read_batch(
                &self,
                path: &std::path::Path,
                watermark: Option<&dendro_core::Watermark>,
            ) -> anyhow::Result<crate::exports::ExportBatch> {
                self.inner.read_batch(path, watermark)
            }
            pub fn ingest_file(
                &self,
                store: &dendro_core::AnalyticsStore,
                path: &std::path::Path,
            ) -> anyhow::Result<Vec<&'static str>> {
                self.inner.ingest_file(store, path)
            }
        }
        #[async_trait::async_trait]
        impl dendro_core::HarnessConnector for $name {
            fn name(&self) -> &'static str {
                $harness
            }
            fn discover_files(&self) -> anyhow::Result<Vec<std::path::PathBuf>> {
                self.inner.discover()
            }
            fn extract_incremental(
                &self,
                path: &std::path::Path,
                watermark: Option<&dendro_core::Watermark>,
            ) -> anyhow::Result<(Vec<dendro_core::CanonicalEvent>, dendro_core::Watermark)> {
                self.inner.extract(path, watermark)
            }
        }
    };
}
pub(crate) use export_connector;
