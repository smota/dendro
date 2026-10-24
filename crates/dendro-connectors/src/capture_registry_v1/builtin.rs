//! Production readers. This table is the only dispatch list ingest consults.
use std::sync::OnceLock;

use crate::exports::Format;
use crate::sdk::{ReadLimits, GLOBAL_MAX_BATCH_RECORDS, GLOBAL_MAX_DEPTH, GLOBAL_MAX_RECORD_BYTES};
use crate::stream::{CLAUDE_RECORD_TYPES, CODEX_RECORD_TYPES, GROK_RECORD_TYPES, PI_RECORD_TYPES};

use super::{Admission, Descriptor, Registry, CONTRACT};

/// Native JSONL ids plus OpenRig. OpenRig has no capture descriptor; the size
/// gate still treats the name as native (T05-A1-12).
pub(crate) const NATIVE_SOURCE_IDS: &[&str] = &["claude", "codex", "pi", "openrig", "grok"];

const USAGE_AND_TOOL: &[&str] = &["usage", "tool"];
const USAGE_ONLY: &[&str] = &["usage"];
const TOOL_ONLY: &[&str] = &["tool"];
const NONE: &[&str] = &[];

/// The stream reader has no separate producer byte profile. Stating the global
/// ceiling here keeps `native_record_size_not_admitted` enforced for a missing
/// or looser admission. It does not cap the stream reader; M1 does not change
/// what that reader parses.
const NATIVE_LIMITS: ReadLimits = ReadLimits {
    encoded_bytes: GLOBAL_MAX_RECORD_BYTES,
    nesting_depth: GLOBAL_MAX_DEPTH,
    decoded_records_per_batch: GLOBAL_MAX_BATCH_RECORDS,
    line_attempts: None,
};
const EXPORT_LIMITS: ReadLimits = ReadLimits::export();
/// AGY's admitted bounds are 1 MiB and 128 decoded records. No line-attempt grammar.
const AGY_LIMITS: ReadLimits = ReadLimits {
    encoded_bytes: 1024 * 1024,
    nesting_depth: 64,
    decoded_records_per_batch: 128,
    line_attempts: None,
};

#[allow(clippy::too_many_arguments)]
const fn production(
    source_id: &'static str,
    schema: &'static str,
    capture_mode: &'static str,
    record_kinds: &'static [&'static str],
    fact_grains: &'static [&'static str],
    privacy_projection: &'static str,
    admitted_native_record_bytes: Option<u64>,
    limits: ReadLimits,
    admission: Admission,
) -> Descriptor {
    Descriptor {
        contract: CONTRACT,
        contribution_version: "1.0.0",
        source_id,
        schema,
        capture_mode,
        record_kinds,
        fact_grains,
        privacy_projection,
        trust: "unverified",
        admitted_native_record_bytes,
        fixture_bytes: None,
        fixture_digest: None,
        limits,
        admission,
        dynamic: false,
        max_bound_processes: None,
        max_samples_per_invocation: None,
    }
}

static CLAUDE: Descriptor = production(
    "claude",
    "claude/native-jsonl/1",
    "native-jsonl",
    CLAUDE_RECORD_TYPES,
    USAGE_AND_TOOL,
    "usage and tool facts; raw message text is not stored",
    Some(GLOBAL_MAX_RECORD_BYTES),
    NATIVE_LIMITS,
    Admission::Implemented,
);
static CODEX: Descriptor = production(
    "codex",
    "codex/native-jsonl/1",
    "native-jsonl",
    CODEX_RECORD_TYPES,
    USAGE_ONLY,
    "usage facts; tool calls are not read",
    Some(GLOBAL_MAX_RECORD_BYTES),
    NATIVE_LIMITS,
    Admission::Implemented,
);
static PI: Descriptor = production(
    "pi",
    "pi/native-jsonl/1",
    "native-jsonl",
    PI_RECORD_TYPES,
    USAGE_AND_TOOL,
    "usage and tool facts; raw message text is not stored",
    Some(GLOBAL_MAX_RECORD_BYTES),
    NATIVE_LIMITS,
    Admission::Implemented,
);
static GROK: Descriptor = production(
    "grok",
    "grok/session-update/1",
    "native-jsonl",
    GROK_RECORD_TYPES,
    USAGE_AND_TOOL,
    "usage and tool facts; message and hook bodies are not stored",
    Some(GLOBAL_MAX_RECORD_BYTES),
    NATIVE_LIMITS,
    Admission::Implemented,
);
static CURSOR: Descriptor = production(
    "cursor",
    "cursor/verified-export/1",
    "verified-export-only",
    crate::exports::CURSOR_RECORD_KINDS,
    TOOL_ONLY,
    "tool facts from the public export; token usage is not in the export",
    None,
    EXPORT_LIMITS,
    Admission::Implemented,
);
static WINDSURF: Descriptor = production(
    "windsurf",
    "windsurf/verified-export/1",
    "verified-export-only",
    crate::exports::WINDSURF_RECORD_KINDS,
    TOOL_ONLY,
    "tool facts from the public export; token usage is not in the export",
    None,
    EXPORT_LIMITS,
    Admission::Implemented,
);
static GOOSE: Descriptor = production(
    "goose",
    "goose/verified-export/1",
    "verified-export-only",
    crate::exports::GOOSE_RECORD_KINDS,
    USAGE_AND_TOOL,
    "legacy header usage and tool facts",
    None,
    EXPORT_LIMITS,
    Admission::Implemented,
);
static DEVIN: Descriptor = production(
    "devin",
    "devin/atif-v1.7",
    "verified-export-only",
    crate::exports::DEVIN_RECORD_KINDS,
    USAGE_AND_TOOL,
    "ATIF-v1.7 agent usage and tool facts",
    None,
    EXPORT_LIMITS,
    Admission::Implemented,
);
static AGY: Descriptor = production(
    "agy",
    "agy/parser-pending",
    "parser-pending",
    NONE,
    NONE,
    "no facts; parser is pending",
    None,
    AGY_LIMITS,
    Admission::ParserPending,
);

static PRODUCTION: &[Descriptor] = &[CLAUDE, CODEX, PI, GROK, CURSOR, WINDSURF, GOOSE, DEVIN, AGY];

impl Registry {
    /// The process-wide production reader table. Duplicate source ids are refused.
    pub fn builtin() -> &'static Self {
        static BUILTIN: OnceLock<Registry> = OnceLock::new();
        BUILTIN.get_or_init(|| {
            Registry::from_production(PRODUCTION)
                .unwrap_or_else(|error| panic!("builtin capture readers are admissible: {error}"))
        })
    }
}

pub(crate) fn verified_export(source_id: &str) -> Option<(&'static str, Format)> {
    match source_id {
        "cursor" => Some(("cursor", Format::Cursor)),
        "windsurf" => Some(("windsurf", Format::Windsurf)),
        "goose" => Some(("goose", Format::Goose)),
        "devin" => Some(("devin", Format::Devin)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture_registry_v1::{Availability, ReaderRoute};

    fn refused(result: Result<Registry, &'static str>) -> &'static str {
        match result {
            Err(error) => error,
            Ok(_) => panic!("production registration admitted a refused descriptor"),
        }
    }

    #[test]
    fn production_registration_rejects_a_duplicate_source_and_keeps_contribution_rules() {
        assert_eq!(CLAUDE.validate(), Err("missing_conformance_fixture"));
        assert_eq!(AGY.validate(), Err("source_not_admitted"));
        assert!(Registry::from_production(PRODUCTION).is_ok());
        static DUP: [Descriptor; 2] = [CLAUDE, CLAUDE];
        assert_eq!(
            refused(Registry::from_production(&DUP)),
            "incompatible_contribution_contract"
        );
        static UNSIZED: Descriptor = Descriptor {
            admitted_native_record_bytes: None,
            ..CLAUDE
        };
        static UNSIZED_SET: &[Descriptor] = &[UNSIZED];
        assert_eq!(
            refused(Registry::from_production(UNSIZED_SET)),
            "native_record_size_not_admitted"
        );
    }

    #[test]
    fn builtin_kinds_are_the_parser_lists() {
        let registry = Registry::builtin();
        let kinds = |id: &str| registry.descriptor(id).unwrap().record_kinds;
        assert_eq!(kinds("claude"), CLAUDE_RECORD_TYPES);
        assert_eq!(CLAUDE_RECORD_TYPES.len(), 15);
        assert_eq!(kinds("codex"), CODEX_RECORD_TYPES);
        assert_eq!(CODEX_RECORD_TYPES.len(), 7);
        assert_eq!(kinds("pi"), PI_RECORD_TYPES);
        assert_eq!(PI_RECORD_TYPES.len(), 6);
        assert_eq!(kinds("grok"), GROK_RECORD_TYPES);
        assert_eq!(
            GROK_RECORD_TYPES,
            &[
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
            ][..]
        );
        assert_eq!(kinds("cursor"), crate::exports::CURSOR_RECORD_KINDS);
        assert_eq!(
            crate::exports::CURSOR_RECORD_KINDS,
            &["assistant", "result", "system", "tool_call", "user"][..]
        );
        assert_eq!(kinds("windsurf"), crate::exports::WINDSURF_RECORD_KINDS);
        assert_eq!(
            crate::exports::WINDSURF_RECORD_KINDS,
            &[
                "code_action",
                "planner_response",
                "post_cascade_response",
                "post_cascade_response_with_transcript",
                "post_mcp_tool_use",
                "post_read_code",
                "post_run_command",
                "post_write_code",
                "user_input",
            ][..]
        );
        assert_eq!(kinds("goose"), crate::exports::GOOSE_RECORD_KINDS);
        assert_eq!(
            crate::exports::GOOSE_RECORD_KINDS,
            &[
                "assistant",
                "document",
                "image",
                "text",
                "thinking",
                "toolRequest",
                "toolResponse",
                "user",
            ][..]
        );
        assert_eq!(kinds("devin"), crate::exports::DEVIN_RECORD_KINDS);
        assert_eq!(
            crate::exports::DEVIN_RECORD_KINDS,
            &["agent", "system", "user"][..]
        );
        assert!(kinds("agy").is_empty());
        for id in ["claude", "codex", "pi", "grok"] {
            assert_eq!(registry.route(id), ReaderRoute::NativeJsonl, "{id}");
            assert_eq!(
                registry.descriptor(id).unwrap().availability(),
                Availability::Implemented,
                "{id}"
            );
        }
        for id in ["cursor", "windsurf", "goose", "devin"] {
            assert_eq!(registry.route(id), ReaderRoute::VerifiedExport, "{id}");
            assert!(verified_export(id).is_some(), "{id}");
        }
        assert_eq!(registry.route("agy"), ReaderRoute::PresentNotParsed);
        assert!(verified_export("claude").is_none());
        assert!(verified_export("agy").is_none());
    }
}
