//! Field-level connector fidelity declarations and insight denominator matrix (`dendro#134`).
//!
//! Every connector declares what it supplies at the individual field level (`Observed`,
//! `Derived`, `Partial(reason)`, or `Absent` by default). Every registered insight in
//! [`super::CATALOG`] declares its `required_fields` and `optional_fields`.
//!
//! Evaluating `(insight, connector_set)` yields a closed two-state result:
//! [`DenominatorEvaluation::Computable`] (with the exact included denominator scope and any
//! excluded connectors + missing fields) or [`DenominatorEvaluation::NotComputable`] (with the
//! union of missing required fields and the per-connector breakdown). A connector that lacks a
//! required field is never silently pooled into a mixed-source denominator.

use super::contract::Insight;
use super::reference::CATALOG;
use serde::Serialize;
use std::collections::BTreeSet;
use std::fmt;

/// Closed vocabulary of individual fields supplied by connectors and consumed by insights.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Field {
    /// `usage_facts.input_uncached`: non-cached prompt/input tokens for a turn.
    InputTokens,
    /// `usage_facts.output_tokens`: output/completion tokens for a turn.
    OutputTokens,
    /// `usage_facts.cache_read`: cached prompt tokens read on a turn.
    CacheReadTokens,
    /// `usage_facts.cache_write`: prompt tokens written to cache on a turn.
    CacheWriteTokens,
    /// `usage_facts.reasoning`: reasoning/thinking tokens (included inside output).
    ReasoningTokens,
    /// `usage_facts.currency_total`: source-reported USD cost for a turn.
    ObservedCost,
    /// `usage_facts.message_id`: provider/harness call or message identifier.
    MessageId,
    /// `usage_facts.native_session` / `tool_facts.native_session` / `identity_facts.native_session`.
    NativeSession,
    /// Per-turn/event timestamp (`usage_facts.ts`, `tool_facts.ts`, `activity_facts.ts`, `queue_facts.ts`).
    TurnTimestamp,
    /// `usage_facts.model`: model identifier observed on a turn.
    Model,
    /// `usage_facts.effort`: reasoning/thinking effort binding observed on a turn.
    Effort,
    /// `context_facts.context_used` and `context_facts.context_limit`.
    ContextWindowUsage,
    /// `tool_facts.tool_name`: tool call name / action identifier.
    ToolCallName,
    /// `tool_facts.failed`: settled terminal outcome (`Some(true)` / `Some(false)`).
    ToolOutcome,
    /// `tool_facts.fingerprint`: deterministic argument/target fingerprint.
    ToolFingerprint,
    /// `session_edges`: explicit parent-child lineage edges (`spawn`, `fork`, `resume`, `delegate`).
    SessionEdges,
    /// `skill_facts` with `invoked = false`: declared/loaded capability or skill set.
    SkillDeclarations,
    /// `skill_facts` with `invoked = true`: invoked capability or skill events.
    SkillInvocations,
    /// `activity_facts`: seat running/idle state transitions for duration analysis.
    ActivityIntervals,
    /// `queue_facts`: durable work-queue transitions and closure reasons.
    QueueTransitions,
    /// `process_identities`: host OS process tree identity and environment classification.
    ProcessIdentity,
    /// `process_samples`: host OS CPU, RSS/working-set, and disk I/O samples.
    ProcessResourceSamples,
    /// `syscall_facts`: admitted privileged syscall or ETW effect trace rows.
    PrivilegedEffectTrace,
    /// `security_facts` policy/sandbox/grant/isolation_test kinds.
    SecurityPolicy,
    /// `security_facts` `dangerous_command` / `dangerously_disable_sandbox` checks.
    SecurityCommandCheck,
    /// `source_files` + `warnings`: reader completeness and unprojected warning accounting.
    SourceCoverage,
    /// `harness_trajectory`: 32-element turn trajectory vectors for vortex detection.
    TrajectoryVector,
    /// `harness_envelope`: top-level JSON key/type shape for schema drift detection.
    HarnessEnvelopeShape,
}

impl Field {
    /// Complete ordered list of all 28 fields in the closed vocabulary.
    pub const ALL: &'static [Field] = &[
        Field::InputTokens,
        Field::OutputTokens,
        Field::CacheReadTokens,
        Field::CacheWriteTokens,
        Field::ReasoningTokens,
        Field::ObservedCost,
        Field::MessageId,
        Field::NativeSession,
        Field::TurnTimestamp,
        Field::Model,
        Field::Effort,
        Field::ContextWindowUsage,
        Field::ToolCallName,
        Field::ToolOutcome,
        Field::ToolFingerprint,
        Field::SessionEdges,
        Field::SkillDeclarations,
        Field::SkillInvocations,
        Field::ActivityIntervals,
        Field::QueueTransitions,
        Field::ProcessIdentity,
        Field::ProcessResourceSamples,
        Field::PrivilegedEffectTrace,
        Field::SecurityPolicy,
        Field::SecurityCommandCheck,
        Field::SourceCoverage,
        Field::TrajectoryVector,
        Field::HarnessEnvelopeShape,
    ];

    /// Canonical snake_case identifier for this field.
    pub const fn as_str(self) -> &'static str {
        match self {
            Field::InputTokens => "input_tokens",
            Field::OutputTokens => "output_tokens",
            Field::CacheReadTokens => "cache_read_tokens",
            Field::CacheWriteTokens => "cache_write_tokens",
            Field::ReasoningTokens => "reasoning_tokens",
            Field::ObservedCost => "observed_cost",
            Field::MessageId => "message_id",
            Field::NativeSession => "native_session",
            Field::TurnTimestamp => "turn_timestamp",
            Field::Model => "model",
            Field::Effort => "effort",
            Field::ContextWindowUsage => "context_window_usage",
            Field::ToolCallName => "tool_call_name",
            Field::ToolOutcome => "tool_outcome",
            Field::ToolFingerprint => "tool_fingerprint",
            Field::SessionEdges => "session_edges",
            Field::SkillDeclarations => "skill_declarations",
            Field::SkillInvocations => "skill_invocations",
            Field::ActivityIntervals => "activity_intervals",
            Field::QueueTransitions => "queue_transitions",
            Field::ProcessIdentity => "process_identity",
            Field::ProcessResourceSamples => "process_resource_samples",
            Field::PrivilegedEffectTrace => "privileged_effect_trace",
            Field::SecurityPolicy => "security_policy",
            Field::SecurityCommandCheck => "security_command_check",
            Field::SourceCoverage => "source_coverage",
            Field::TrajectoryVector => "trajectory_vector",
            Field::HarnessEnvelopeShape => "harness_envelope_shape",
        }
    }

    /// High-level domain category for documentation and grouping.
    pub const fn category(self) -> &'static str {
        match self {
            Field::InputTokens
            | Field::OutputTokens
            | Field::CacheReadTokens
            | Field::CacheWriteTokens
            | Field::ReasoningTokens => "tokens",
            Field::ObservedCost => "cost_units",
            Field::MessageId | Field::NativeSession | Field::Model | Field::Effort => "identity",
            Field::TurnTimestamp | Field::ActivityIntervals | Field::QueueTransitions => "timing",
            Field::ContextWindowUsage => "context",
            Field::ToolCallName
            | Field::ToolOutcome
            | Field::ToolFingerprint
            | Field::SkillDeclarations
            | Field::SkillInvocations => "tool_calls",
            Field::SessionEdges => "edges",
            Field::ProcessIdentity
            | Field::ProcessResourceSamples
            | Field::PrivilegedEffectTrace => "environment",
            Field::SecurityPolicy | Field::SecurityCommandCheck => "security",
            Field::SourceCoverage | Field::TrajectoryVector | Field::HarnessEnvelopeShape => {
                "telemetry"
            }
        }
    }

    /// DuckDB storage table and column(s) backing this field.
    pub const fn storage_column(self) -> &'static str {
        match self {
            Field::InputTokens => "usage_facts.input_uncached",
            Field::OutputTokens => "usage_facts.output_tokens",
            Field::CacheReadTokens => "usage_facts.cache_read",
            Field::CacheWriteTokens => "usage_facts.cache_write",
            Field::ReasoningTokens => "usage_facts.reasoning",
            Field::ObservedCost => "usage_facts.currency_total",
            Field::MessageId => "usage_facts.message_id",
            Field::NativeSession => "usage_facts.native_session / tool_facts.native_session",
            Field::TurnTimestamp => "usage_facts.ts / tool_facts.ts / activity_facts.ts",
            Field::Model => "usage_facts.model",
            Field::Effort => "usage_facts.effort",
            Field::ContextWindowUsage => "context_facts.(context_used, context_limit)",
            Field::ToolCallName => "tool_facts.tool_name",
            Field::ToolOutcome => "tool_facts.failed",
            Field::ToolFingerprint => "tool_facts.fingerprint",
            Field::SessionEdges => "session_edges.(parent_session, child_session, relation)",
            Field::SkillDeclarations => "skill_facts (invoked = false)",
            Field::SkillInvocations => "skill_facts (invoked = true)",
            Field::ActivityIntervals => "activity_facts.(seat_id, ts, state)",
            Field::QueueTransitions => "queue_facts.(item_id, ts, transition_seq, state)",
            Field::ProcessIdentity => "process_identities",
            Field::ProcessResourceSamples => "process_samples",
            Field::PrivilegedEffectTrace => "syscall_facts",
            Field::SecurityPolicy => "security_facts (policy / sandbox / grant)",
            Field::SecurityCommandCheck => "security_facts (dangerous_command)",
            Field::SourceCoverage => "source_files + warnings",
            Field::TrajectoryVector => "harness_trajectory.trajectory_vec",
            Field::HarnessEnvelopeShape => "harness_envelope.payload_shape",
        }
    }
}

impl fmt::Display for Field {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Typed reason when a connector supplies a field only under a bounded condition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PartialFidelityReason {
    /// Codex `tool_outcome`: `failed` is `Some(bool)` only when an explicit exit code, error
    /// object, or terminal status marker is present; prose output remains `None` (`unsettled`).
    ExplicitSignalOnly,
    /// Windsurf `tool_outcome`: only `code_action` steps with `status == "done"` emit
    /// `failed = Some(false)`; other steps are skipped.
    CompletedCodeActionOnly,
    /// Goose `input_tokens` / `output_tokens`: only legacy v0.8 header rows carry
    /// `total_input_tokens` / `total_output_tokens`; v1.x message lines carry no token counts.
    LegacyHeaderLifetimeTotals,
    /// Grok `observed_cost`: turn `costUsdTicks` is attributed to a model row only on
    /// single-model turns; multi-model turns without per-model cost emit a zero-token
    /// `grok_turn_cost` row with `model = None`.
    SingleModelTurnOnly,
    /// Claude `effort`: field is parsed when present (`thinkingMetadata.level` / `effort`),
    /// but not observed in G0a captures.
    ParsedWhenPresentNotObservedInCaptures,
}

impl PartialFidelityReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            PartialFidelityReason::ExplicitSignalOnly => "explicit signal only",
            PartialFidelityReason::CompletedCodeActionOnly => "completed code_action only",
            PartialFidelityReason::LegacyHeaderLifetimeTotals => "legacy header lifetime totals",
            PartialFidelityReason::SingleModelTurnOnly => "single-model turn only",
            PartialFidelityReason::ParsedWhenPresentNotObservedInCaptures => {
                "field parsed when present; not observed in captures"
            }
        }
    }
}

impl fmt::Display for PartialFidelityReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Field-level fidelity level for a connector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldFidelity {
    /// Directly parsed from native source fields without arithmetic derivation.
    Observed,
    /// Deterministically computed from native source fields (e.g. cumulative counter deltas or
    /// prompt-minus-cached subtraction).
    Derived,
    /// Parsed only for a subset of events/sessions with a typed reason.
    Partial(PartialFidelityReason),
    /// Not supplied by this connector (`Absent` is the honest default).
    Absent,
}

impl FieldFidelity {
    /// Returns `true` when the connector supplies the field (`Observed`, `Derived`, or `Partial`).
    pub const fn is_supplied(self) -> bool {
        !matches!(self, FieldFidelity::Absent)
    }

    /// Compact human-readable label for tables and diagnostics.
    pub fn label(self) -> String {
        match self {
            FieldFidelity::Observed => "Observed".to_string(),
            FieldFidelity::Derived => "Derived".to_string(),
            FieldFidelity::Partial(reason) => format!("Partial ({reason})"),
            FieldFidelity::Absent => "Absent".to_string(),
        }
    }
}

impl fmt::Display for FieldFidelity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FieldFidelity::Observed => f.write_str("Observed"),
            FieldFidelity::Derived => f.write_str("Derived"),
            FieldFidelity::Partial(reason) => write!(f, "Partial ({reason})"),
            FieldFidelity::Absent => f.write_str("Absent"),
        }
    }
}

/// One field's fidelity declaration inside a [`ConnectorDeclaration`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct FieldDeclaration {
    pub field: Field,
    pub fidelity: FieldFidelity,
    pub code_location: &'static str,
    pub rationale: &'static str,
}

/// Complete field-level fidelity declaration for a single connector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ConnectorDeclaration {
    pub connector: &'static str,
    pub aliases: &'static [&'static str],
    pub capture_class: &'static str,
    pub fields: &'static [FieldDeclaration],
}

impl ConnectorDeclaration {
    /// Look up the [`FieldDeclaration`] for `field`.
    pub fn declaration(&self, field: Field) -> Option<&'static FieldDeclaration> {
        self.fields.iter().find(|decl| decl.field == field)
    }

    /// Return the [`FieldFidelity`] for `field` (`Absent` if omitted).
    pub fn fidelity(&self, field: Field) -> FieldFidelity {
        self.declaration(field)
            .map(|decl| decl.fidelity)
            .unwrap_or(FieldFidelity::Absent)
    }

    /// Returns `true` if `field` is supplied (`Observed`, `Derived`, or `Partial`).
    pub fn supplies(&self, field: Field) -> bool {
        self.fidelity(field).is_supplied()
    }

    /// All fields declared `Observed` on this connector.
    pub fn observed_fields(&self) -> Vec<Field> {
        self.fields
            .iter()
            .filter(|decl| decl.fidelity == FieldFidelity::Observed)
            .map(|decl| decl.field)
            .collect()
    }

    /// All fields declared `Derived` on this connector.
    pub fn derived_fields(&self) -> Vec<Field> {
        self.fields
            .iter()
            .filter(|decl| decl.fidelity == FieldFidelity::Derived)
            .map(|decl| decl.field)
            .collect()
    }

    /// All fields declared `Partial` on this connector, with their typed reasons.
    pub fn partial_fields(&self) -> Vec<(Field, PartialFidelityReason)> {
        self.fields
            .iter()
            .filter_map(|decl| match decl.fidelity {
                FieldFidelity::Partial(reason) => Some((decl.field, reason)),
                _ => None,
            })
            .collect()
    }

    /// All fields declared `Absent` on this connector.
    pub fn absent_fields(&self) -> Vec<Field> {
        Field::ALL
            .iter()
            .copied()
            .filter(|field| self.fidelity(*field) == FieldFidelity::Absent)
            .collect()
    }
}

macro_rules! decl {
    ($field:ident, $fidelity:expr, $loc:literal, $why:literal) => {
        FieldDeclaration {
            field: Field::$field,
            fidelity: $fidelity,
            code_location: $loc,
            rationale: $why,
        }
    };
}

const CLAUDE_FIELDS: &[FieldDeclaration] = &[
    // crates/dendro-connectors/src/stream.rs:1512 (`parse_claude_usage` reads `usage.input_tokens`)
    decl!(
        InputTokens,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/stream.rs:1512",
        "message.usage.input_tokens parsed directly as uncached input tokens"
    ),
    // crates/dendro-connectors/src/stream.rs:1513 (`parse_claude_usage` reads `usage.output_tokens`)
    decl!(
        OutputTokens,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/stream.rs:1513",
        "message.usage.output_tokens parsed directly"
    ),
    // crates/dendro-connectors/src/stream.rs:1514 (`parse_claude_usage` reads `usage.cache_read_input_tokens`)
    decl!(
        CacheReadTokens,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/stream.rs:1514",
        "message.usage.cache_read_input_tokens parsed directly"
    ),
    // crates/dendro-connectors/src/stream.rs:1515 (`parse_claude_usage` reads `usage.cache_creation_input_tokens`)
    decl!(
        CacheWriteTokens,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/stream.rs:1515",
        "message.usage.cache_creation_input_tokens parsed directly"
    ),
    // crates/dendro-connectors/src/stream.rs:1516 (`reasoning: 0` hardcoded; Claude JSONL does not separate reasoning tokens)
    decl!(
        ReasoningTokens,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/stream.rs:1516",
        "Claude transcript usage does not report a separate reasoning token counter"
    ),
    // crates/dendro-connectors/src/stream.rs:1517 (`currency_total: None`)
    decl!(
        ObservedCost,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/stream.rs:1517",
        "Claude transcript usage does not emit observed USD cost"
    ),
    // crates/dendro-connectors/src/stream.rs:1518-1521 (`message.id`)
    decl!(
        MessageId,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/stream.rs:1518-1521",
        "message.id parsed on assistant usage records"
    ),
    // crates/dendro-connectors/src/stream.rs:1522-1526 (`sessionId` or file stem)
    decl!(
        NativeSession,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/stream.rs:1522-1526",
        "sessionId parsed from transcript lines"
    ),
    // crates/dendro-connectors/src/stream.rs:1527,2767-2779 (`timestamp`)
    decl!(
        TurnTimestamp,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/stream.rs:1527",
        "RFC3339/epoch timestamp parsed per usage and tool event"
    ),
    // crates/dendro-connectors/src/stream.rs:83-86 (`extract_usage_model_effort` for `claude`)
    decl!(
        Model,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/stream.rs:83-86",
        "message.model or top-level model parsed per turn"
    ),
    // crates/dendro-connectors/src/stream.rs:87-91 (`extract_usage_model_effort` reads `thinkingMetadata.level` / `effort`)
    decl!(
        Effort,
        FieldFidelity::Partial(PartialFidelityReason::ParsedWhenPresentNotObservedInCaptures),
        "crates/dendro-connectors/src/stream.rs:87-91",
        "field parsed when present; not observed in captures"
    ),
    // crates/dendro-core/src/insight_v1/reference/i07_context_pressure.rs:30 (`context_facts` not populated by connector)
    decl!(
        ContextWindowUsage,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/stream.rs:1495-1536",
        "Claude transcripts do not emit context_used / context_limit facts"
    ),
    // crates/dendro-connectors/src/stream.rs:2146-2149 (`tool_use` content block `name`)
    decl!(
        ToolCallName,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/stream.rs:2146-2149",
        "message.content[].type=tool_use name parsed directly"
    ),
    // crates/dendro-connectors/src/stream.rs:2249-2260,2704-2711 (`tool_result` `is_error` / `toolUseResult`)
    decl!(
        ToolOutcome,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/stream.rs:2249-2260",
        "tool_result is_error and toolUseResult status correlated by tool_use_id"
    ),
    // crates/dendro-connectors/src/stream.rs:2150-2177 (`fingerprint_for_tool`)
    decl!(
        ToolFingerprint,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/stream.rs:2150-2177",
        "tool_use input arguments fingerprinted deterministically"
    ),
    // crates/dendro-connectors/src/lineage.rs:142-165 (`parse_claude_line` sidechain/agentId edges)
    decl!(
        SessionEdges,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/lineage.rs:142-165",
        "isSidechain + sessionId + agentId extracted into explicit spawn edges"
    ),
    // crates/dendro-connectors/src/stream.rs:2186-2222 (only `invoked = true` emitted from transcripts)
    decl!(
        SkillDeclarations,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/stream.rs:2186-2222",
        "Claude JSONL records invoked skills, not static loaded skill manifests"
    ),
    // crates/dendro-connectors/src/stream.rs:2186-2222 (`Skill` tool_use and slash command invocations)
    decl!(
        SkillInvocations,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/stream.rs:2186-2222",
        "Skill tool_use blocks and slash commands recorded with invoked=true"
    ),
    decl!(
        ActivityIntervals,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/stream.rs:1100-1205",
        "activity_facts are supplied by OpenRig coordination logs, not Claude transcripts"
    ),
    decl!(
        QueueTransitions,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/stream.rs:1100-1205",
        "queue_facts are supplied by OpenRig queue logs, not Claude transcripts"
    ),
    decl!(
        ProcessIdentity,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/claude.rs:1-42",
        "host process identities are collected by environment_v1, not session JSONL"
    ),
    decl!(
        ProcessResourceSamples,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/claude.rs:1-42",
        "host process resource samples are collected by environment_v1"
    ),
    decl!(
        PrivilegedEffectTrace,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/claude.rs:1-42",
        "syscall/ETW traces require syscall_v1 or windows_events_v1 collectors"
    ),
    // crates/dendro-connectors/src/security_facts.rs:68-80,95-115 (`permissionMode`)
    decl!(
        SecurityPolicy,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/security_facts.rs:95-115",
        "permissionMode extracted into security_facts"
    ),
    // crates/dendro-connectors/src/security_facts.rs:117-137 (`dangerouslyDisableSandbox` and `dangerous_command`)
    decl!(
        SecurityCommandCheck,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/security_facts.rs:117-137",
        "Bash tool_use command and dangerouslyDisableSandbox inspected for security_facts"
    ),
    // crates/dendro-connectors/src/stream.rs:883-940 (`source_files` + `warnings` committed per file)
    decl!(
        SourceCoverage,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/stream.rs:883-940",
        "source_files watermarks, reader completeness markers, and parse warnings recorded"
    ),
    decl!(
        TrajectoryVector,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/claude.rs:1-42",
        "harness_trajectory vectors are not emitted by the Claude JSONL connector"
    ),
    decl!(
        HarnessEnvelopeShape,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/claude.rs:1-42",
        "harness_envelope shapes are not emitted by the Claude JSONL connector"
    ),
];

const CODEX_FIELDS: &[FieldDeclaration] = &[
    // crates/dendro-connectors/src/stream.rs:1414-1478 (delta from `total_token_usage` / `last_token_usage` minus cached_input_tokens and cache_creation_input_tokens; errors on negative uncached delta)
    decl!(
        InputTokens,
        FieldFidelity::Derived,
        "crates/dendro-connectors/src/stream.rs:1414-1478",
        "derived from cumulative total_token_usage delta (or last_token_usage) minus cached_input_tokens and cache_creation_input_tokens, erroring if negative"
    ),
    // crates/dendro-connectors/src/stream.rs:1418,1441-1445,1479 (delta from `total_token_usage.output_tokens`)
    decl!(
        OutputTokens,
        FieldFidelity::Derived,
        "crates/dendro-connectors/src/stream.rs:1441-1445",
        "derived from monotonic cumulative total_token_usage.output_tokens delta or last_token_usage"
    ),
    // crates/dendro-connectors/src/stream.rs:1416,1431-1435,1480 (`cached_input_tokens` delta)
    decl!(
        CacheReadTokens,
        FieldFidelity::Derived,
        "crates/dendro-connectors/src/stream.rs:1431-1435",
        "derived from monotonic cumulative cached_input_tokens delta or last_token_usage"
    ),
    // crates/dendro-connectors/src/stream.rs:1417,1436-1440,1481 (`cache_creation_input_tokens` delta)
    decl!(
        CacheWriteTokens,
        FieldFidelity::Derived,
        "crates/dendro-connectors/src/stream.rs:1436-1440",
        "derived from monotonic cumulative cache_creation_input_tokens delta or last_token_usage"
    ),
    // crates/dendro-connectors/src/stream.rs:1419,1446-1450,1482 (`reasoning_output_tokens` delta)
    decl!(
        ReasoningTokens,
        FieldFidelity::Derived,
        "crates/dendro-connectors/src/stream.rs:1446-1450",
        "derived from monotonic cumulative reasoning_output_tokens delta or last_token_usage"
    ),
    // crates/dendro-connectors/src/stream.rs:1483 (`currency_total: None`)
    decl!(
        ObservedCost,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/stream.rs:1483",
        "Codex token_count events do not report USD cost"
    ),
    // crates/dendro-connectors/src/stream.rs:1484 (`message_id: None`)
    decl!(
        MessageId,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/stream.rs:1484",
        "Codex token_count events carry no per-call message_id"
    ),
    // crates/dendro-connectors/src/stream.rs:1054-1065,1485-1489 (`session_meta.payload.id`)
    decl!(
        NativeSession,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/stream.rs:1054-1065",
        "session_meta payload.id parsed as native_session"
    ),
    // crates/dendro-connectors/src/stream.rs:1490,2359,2499 (`timestamp`)
    decl!(
        TurnTimestamp,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/stream.rs:1490",
        "event timestamp parsed on token_count and tool call items"
    ),
    // crates/dendro-connectors/src/stream.rs:52-53,1244-1262 (`turn_context` / `session_meta` model)
    decl!(
        Model,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/stream.rs:52-53",
        "model parsed from turn_context or payload"
    ),
    // crates/dendro-connectors/src/stream.rs:54-63,1244-1262 (`collaboration_mode.settings.reasoning_effort` / `effort`)
    decl!(
        Effort,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/stream.rs:54-63",
        "reasoning_effort / effort parsed from turn_context or payload"
    ),
    decl!(
        ContextWindowUsage,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/stream.rs:1402-1493",
        "context_facts table is not populated from Codex rollout logs"
    ),
    // crates/dendro-connectors/src/stream.rs:2287-2373,2405-2516 (`function_call`, `custom_tool_call`, `web_search_call`, `mcp_tool_call`)
    decl!(
        ToolCallName,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/stream.rs:2287-2373",
        "response_item function_call, custom_tool_call, web_search_call, and event_msg tool calls parsed"
    ),
    // crates/dendro-connectors/src/stream.rs:1918-1996,2325-2333,2520-2541 (`codex_output_failed` explicit signals only)
    decl!(
        ToolOutcome,
        FieldFidelity::Partial(PartialFidelityReason::ExplicitSignalOnly),
        "crates/dendro-connectors/src/stream.rs:1918-1996",
        "failed is Some(bool) only on explicit exit_code, status, error, or Process exited with code N; prose output stays None"
    ),
    // crates/dendro-connectors/src/stream.rs:2302-2307,2350-2355,2448-2462 (`fingerprint_for_tool`)
    decl!(
        ToolFingerprint,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/stream.rs:2302-2307",
        "function_call and custom_tool_call arguments fingerprinted deterministically"
    ),
    // crates/dendro-connectors/src/lineage.rs:172-211 (`session_meta.payload.source.subagent.thread_spawn`)
    decl!(
        SessionEdges,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/lineage.rs:172-211",
        "session_meta.payload.source.subagent.thread_spawn.parent_thread_id extracted into spawn edges"
    ),
    decl!(
        SkillDeclarations,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/stream.rs:1206-1240",
        "Codex rollout logs emit invoked skills ($skill / skills/path), not static loaded manifests"
    ),
    // crates/dendro-connectors/src/stream.rs:1206-1240,2309-2324 (heuristic $skill token and skills/<name>/SKILL.md path matching)
    decl!(
        SkillInvocations,
        FieldFidelity::Derived,
        "crates/dendro-connectors/src/stream.rs:1206-1240",
        "derived via heuristic $skill token and skills/<name>/SKILL.md path matching in prompts and tool calls"
    ),
    decl!(
        ActivityIntervals,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/codex.rs:1-40",
        "activity_facts are supplied by OpenRig coordination logs, not Codex rollouts"
    ),
    decl!(
        QueueTransitions,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/codex.rs:1-40",
        "queue_facts are supplied by OpenRig queue logs, not Codex rollouts"
    ),
    decl!(
        ProcessIdentity,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/codex.rs:1-40",
        "host process identities are collected by environment_v1"
    ),
    decl!(
        ProcessResourceSamples,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/codex.rs:1-40",
        "host process resource samples are collected by environment_v1"
    ),
    decl!(
        PrivilegedEffectTrace,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/codex.rs:1-40",
        "syscall/ETW traces require syscall_v1 or windows_events_v1 collectors"
    ),
    // crates/dendro-connectors/src/security_facts.rs:139-227 (`approval_policy`, `sandbox_policy`)
    decl!(
        SecurityPolicy,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/security_facts.rs:139-227",
        "turn_context approval_policy and sandbox_policy extracted into security_facts"
    ),
    // crates/dendro-connectors/src/security_facts.rs:229-268 (`dangerous_command`, `sandbox_permissions=require_escalated`)
    decl!(
        SecurityCommandCheck,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/security_facts.rs:229-268",
        "exec_command / shell_command arguments and sandbox_permissions inspected for security_facts"
    ),
    // crates/dendro-connectors/src/stream.rs:883-940
    decl!(
        SourceCoverage,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/stream.rs:883-940",
        "source_files watermarks, reader completeness markers, and parse warnings recorded"
    ),
    decl!(
        TrajectoryVector,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/codex.rs:1-40",
        "harness_trajectory vectors are not emitted by the Codex rollout connector"
    ),
    decl!(
        HarnessEnvelopeShape,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/codex.rs:1-40",
        "harness_envelope shapes are not emitted by the Codex rollout connector"
    ),
];

const PI_FIELDS: &[FieldDeclaration] = &[
    // crates/dendro-connectors/src/stream.rs:1550 (`usage.input` / `usage.input_tokens`)
    decl!(
        InputTokens,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/stream.rs:1550",
        "message.usage.input or input_tokens parsed directly"
    ),
    // crates/dendro-connectors/src/stream.rs:1551 (`usage.output` / `usage.output_tokens`)
    decl!(
        OutputTokens,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/stream.rs:1551",
        "message.usage.output or output_tokens parsed directly"
    ),
    // crates/dendro-connectors/src/stream.rs:1552 (`usage.cacheRead` / `usage.cache_read_input_tokens`)
    decl!(
        CacheReadTokens,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/stream.rs:1552",
        "message.usage.cacheRead or cache_read_input_tokens parsed directly"
    ),
    // crates/dendro-connectors/src/stream.rs:1553 (`usage.cacheWrite` / `usage.cache_creation_input_tokens`)
    decl!(
        CacheWriteTokens,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/stream.rs:1553",
        "message.usage.cacheWrite or cache_creation_input_tokens parsed directly"
    ),
    // crates/dendro-connectors/src/stream.rs:1554 (`usage.reasoning` / `usage.reasoning_tokens`)
    decl!(
        ReasoningTokens,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/stream.rs:1554",
        "message.usage.reasoning or reasoning_tokens parsed directly"
    ),
    // crates/dendro-connectors/src/stream.rs:1548,1555 (`usage.cost.total`)
    decl!(
        ObservedCost,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/stream.rs:1548",
        "message.usage.cost.total parsed directly as observed USD"
    ),
    // crates/dendro-connectors/src/stream.rs:1556 (`id` or `message.id`)
    decl!(
        MessageId,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/stream.rs:1556",
        "entry id or message.id parsed per assistant message"
    ),
    // crates/dendro-connectors/src/stream.rs:782-785,1054-1065 (`type=session` `id`)
    decl!(
        NativeSession,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/stream.rs:782-785",
        "session header id parsed as native_session"
    ),
    // crates/dendro-connectors/src/stream.rs:1558,2767-2779 (`timestamp`)
    decl!(
        TurnTimestamp,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/stream.rs:1558",
        "timestamp parsed on message and tool entries"
    ),
    // crates/dendro-connectors/src/stream.rs:66-78,1264-1280 (`model` / `modelId`)
    decl!(
        Model,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/stream.rs:66-78",
        "message.model or model_change modelId parsed"
    ),
    // crates/dendro-connectors/src/stream.rs:70-79,1264-1280 (`thinkingLevel` / `reasoningEffort`)
    decl!(
        Effort,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/stream.rs:70-79",
        "thinking_level_change thinkingLevel or message effort parsed"
    ),
    decl!(
        ContextWindowUsage,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/stream.rs:1538-1571",
        "context_facts table is not populated from Pi transcripts"
    ),
    // crates/dendro-connectors/src/stream.rs:2136-2149 (`toolCall` / `tool_use` content block `name`)
    decl!(
        ToolCallName,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/stream.rs:2136-2149",
        "message.content[].type=toolCall name parsed directly"
    ),
    // crates/dendro-connectors/src/stream.rs:2109-2129,2704-2711 (`role=toolResult` `isError`)
    decl!(
        ToolOutcome,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/stream.rs:2109-2129",
        "toolResult isError / is_error correlated by toolCallId"
    ),
    // crates/dendro-connectors/src/stream.rs:2150-2177 (`fingerprint_for_tool`)
    decl!(
        ToolFingerprint,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/stream.rs:2150-2177",
        "toolCall arguments fingerprinted deterministically"
    ),
    // crates/dendro-connectors/src/lineage.rs:230-234 (Pi sessions emit no cross-session edges)
    decl!(
        SessionEdges,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/lineage.rs:230-234",
        "Pi transcripts carry in-file parentId entry trees, not cross-session lineage edges"
    ),
    decl!(
        SkillDeclarations,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/stream.rs:2186-2222",
        "Pi transcripts emit invoked skills, not static loaded skill manifests"
    ),
    // crates/dendro-connectors/src/stream.rs:2186-2222 (heuristic /skill: command and skills/<name>/SKILL.md read path matching)
    decl!(
        SkillInvocations,
        FieldFidelity::Derived,
        "crates/dendro-connectors/src/stream.rs:2186-2222",
        "derived via heuristic /skill: command and skills/<name>/SKILL.md path matching"
    ),
    decl!(
        ActivityIntervals,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/pi.rs:1-40",
        "activity_facts are supplied by OpenRig coordination logs, not Pi transcripts"
    ),
    decl!(
        QueueTransitions,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/pi.rs:1-40",
        "queue_facts are supplied by OpenRig queue logs, not Pi transcripts"
    ),
    decl!(
        ProcessIdentity,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/pi.rs:1-40",
        "host process identities are collected by environment_v1"
    ),
    decl!(
        ProcessResourceSamples,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/pi.rs:1-40",
        "host process resource samples are collected by environment_v1"
    ),
    decl!(
        PrivilegedEffectTrace,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/pi.rs:1-40",
        "syscall/ETW traces require syscall_v1 or windows_events_v1 collectors"
    ),
    // crates/dendro-connectors/src/security_facts.rs:82-90 (Pi does not emit permission_mode / sandbox_policy)
    decl!(
        SecurityPolicy,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/security_facts.rs:82-90",
        "Pi transcripts do not emit permission_mode, approval_policy, or sandbox_policy facts"
    ),
    // crates/dendro-connectors/src/security_facts.rs:82-90,117-137 (`bash` toolCall `dangerous_command`)
    decl!(
        SecurityCommandCheck,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/security_facts.rs:82-90",
        "bash toolCall command arguments inspected for dangerous_command security_facts"
    ),
    // crates/dendro-connectors/src/stream.rs:883-940
    decl!(
        SourceCoverage,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/stream.rs:883-940",
        "source_files watermarks, reader completeness markers, and parse warnings recorded"
    ),
    decl!(
        TrajectoryVector,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/pi.rs:1-40",
        "harness_trajectory vectors are not emitted by the Pi connector"
    ),
    decl!(
        HarnessEnvelopeShape,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/pi.rs:1-40",
        "harness_envelope shapes are not emitted by the Pi connector"
    ),
];

const GROK_FIELDS: &[FieldDeclaration] = &[
    // crates/dendro-connectors/src/stream.rs:1723-1732,1739 (`prompt_tokens` minus `cached_tokens` and `cache_write_tokens`)
    decl!(
        InputTokens,
        FieldFidelity::Derived,
        "crates/dendro-connectors/src/stream.rs:1723-1732",
        "derived as prompt_tokens.saturating_sub(cached_tokens + cache_write_tokens)"
    ),
    // crates/dendro-connectors/src/stream.rs:1740 (`completion_tokens` / `output_tokens`)
    decl!(
        OutputTokens,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/stream.rs:1740",
        "usage.completion_tokens or output_tokens parsed directly"
    ),
    // crates/dendro-connectors/src/stream.rs:1724,1741 (`cached_tokens` / `cache_read_input_tokens`)
    decl!(
        CacheReadTokens,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/stream.rs:1724",
        "usage.cached_tokens or prompt_tokens_details.cached_tokens parsed directly"
    ),
    // crates/dendro-connectors/src/stream.rs:1725,1742 (`cache_write_tokens` / `cache_creation_input_tokens`)
    decl!(
        CacheWriteTokens,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/stream.rs:1725",
        "usage.cache_write_tokens or cache_creation_input_tokens parsed directly"
    ),
    // crates/dendro-connectors/src/stream.rs:1743 (`reasoning_tokens` / `completion_tokens_details.reasoning_tokens`)
    decl!(
        ReasoningTokens,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/stream.rs:1743",
        "usage.reasoning_tokens or completion_tokens_details.reasoning_tokens parsed directly"
    ),
    // crates/dendro-connectors/src/stream.rs:1573-1591,1647-1665,1733-1736 (`costUsdTicks` on single-model turns; multi-model turns without per-model cost emit a zero-token `grok_turn_cost` row with `model: None`)
    decl!(
        ObservedCost,
        FieldFidelity::Partial(PartialFidelityReason::SingleModelTurnOnly),
        "crates/dendro-connectors/src/stream.rs:1647-1665",
        "turn costUsdTicks attributed to the model row on single-model turns; multi-model turns without per-model cost emit a zero-token grok_turn_cost row with model=None"
    ),
    // crates/dendro-connectors/src/stream.rs:1686,1745 (`message_id: None`)
    decl!(
        MessageId,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/stream.rs:1686",
        "Grok session JSON and message rows do not supply a qualified call message_id"
    ),
    // crates/dendro-connectors/src/stream.rs:1059,1613-1616 (`session_id` / `id`)
    decl!(
        NativeSession,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/stream.rs:1613-1616",
        "session_id or id parsed from Grok session JSON"
    ),
    // crates/dendro-connectors/src/stream.rs:1617,1761-1766 (`created_at` / `updated_at` / `timestamp`)
    decl!(
        TurnTimestamp,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/stream.rs:1761-1766",
        "turn timestamp or session updated_at/created_at parsed"
    ),
    // crates/dendro-connectors/src/stream.rs:1628,1772-1845 (`model`)
    decl!(
        Model,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/stream.rs:1772-1845",
        "turn model or session model parsed"
    ),
    // crates/dendro-connectors/src/stream.rs:1772-1850 (`reasoning_effort` / `effort`)
    decl!(
        Effort,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/stream.rs:1772-1850",
        "reasoning_effort / thinking_effort parsed from turn or session"
    ),
    decl!(
        ContextWindowUsage,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/stream.rs:1573-1768",
        "context_facts table is not populated from Grok sessions"
    ),
    // crates/dendro-connectors/src/stream.rs:2623-2631 (`tool_calls[].name` / `function.name`)
    decl!(
        ToolCallName,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/stream.rs:2623-2631",
        "message.tool_calls[].name or function.name parsed directly"
    ),
    // crates/dendro-connectors/src/stream.rs:2595-2614 (`tool_results` / `is_error`)
    decl!(
        ToolOutcome,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/stream.rs:2595-2614",
        "tool_results status/is_error correlated by tool_call_id"
    ),
    // crates/dendro-connectors/src/stream.rs:2634-2652 (`fingerprint_for_tool`)
    decl!(
        ToolFingerprint,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/stream.rs:2634-2652",
        "tool_calls arguments/input fingerprinted deterministically"
    ),
    // crates/dendro-connectors/src/lineage.rs:61-128 (`parent_session_id` / `Subagent` tool calls)
    decl!(
        SessionEdges,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/lineage.rs:61-128",
        "parent_session_id and Subagent tool_calls extracted into explicit lineage edges"
    ),
    decl!(
        SkillDeclarations,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/grok.rs:1-40",
        "Grok sessions do not emit skill declarations"
    ),
    decl!(
        SkillInvocations,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/grok.rs:1-40",
        "Grok sessions do not emit skill_facts"
    ),
    decl!(
        ActivityIntervals,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/grok.rs:1-40",
        "activity_facts are supplied by OpenRig coordination logs"
    ),
    decl!(
        QueueTransitions,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/grok.rs:1-40",
        "queue_facts are supplied by OpenRig queue logs"
    ),
    decl!(
        ProcessIdentity,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/grok.rs:1-40",
        "host process identities are collected by environment_v1"
    ),
    decl!(
        ProcessResourceSamples,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/grok.rs:1-40",
        "host process resource samples are collected by environment_v1"
    ),
    decl!(
        PrivilegedEffectTrace,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/grok.rs:1-40",
        "syscall/ETW traces require syscall_v1 or windows_events_v1 collectors"
    ),
    decl!(
        SecurityPolicy,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/security_facts.rs:58-93",
        "Grok connector does not emit security_facts policy rows"
    ),
    decl!(
        SecurityCommandCheck,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/security_facts.rs:58-93",
        "Grok connector does not emit security_facts command checks"
    ),
    // crates/dendro-connectors/src/stream.rs:883-940
    decl!(
        SourceCoverage,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/stream.rs:883-940",
        "source_files watermarks, reader completeness markers, and parse warnings recorded"
    ),
    decl!(
        TrajectoryVector,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/grok.rs:1-40",
        "harness_trajectory vectors are not emitted by the Grok connector"
    ),
    decl!(
        HarnessEnvelopeShape,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/grok.rs:1-40",
        "harness_envelope shapes are not emitted by the Grok connector"
    ),
];

const AGY_FIELDS: &[FieldDeclaration] = &[
    // crates/dendro-connectors/src/agy.rs:57-64 (refuses protobuf/LevelDB ingest; emits zero rows)
    decl!(
        InputTokens,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/agy.rs:57-64",
        "AGY .pb/LevelDB capture is metadata-only and refuses transcript fact projection"
    ),
    decl!(
        OutputTokens,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/agy.rs:57-64",
        "AGY .pb/LevelDB capture is metadata-only and refuses transcript fact projection"
    ),
    decl!(
        CacheReadTokens,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/agy.rs:57-64",
        "AGY .pb/LevelDB capture is metadata-only and refuses transcript fact projection"
    ),
    decl!(
        CacheWriteTokens,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/agy.rs:57-64",
        "AGY .pb/LevelDB capture is metadata-only and refuses transcript fact projection"
    ),
    decl!(
        ReasoningTokens,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/agy.rs:57-64",
        "AGY .pb/LevelDB capture is metadata-only and refuses transcript fact projection"
    ),
    decl!(
        ObservedCost,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/agy.rs:57-64",
        "AGY .pb/LevelDB capture is metadata-only and refuses transcript fact projection"
    ),
    decl!(
        MessageId,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/agy.rs:57-64",
        "AGY .pb/LevelDB capture is metadata-only and refuses transcript fact projection"
    ),
    decl!(
        NativeSession,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/agy.rs:57-64",
        "AGY .pb/LevelDB capture writes no session rows to DuckDB"
    ),
    decl!(
        TurnTimestamp,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/agy.rs:57-64",
        "AGY .pb/LevelDB capture writes no timestamped rows to DuckDB"
    ),
    decl!(
        Model,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/agy.rs:57-64",
        "AGY .pb/LevelDB capture writes no usage_facts rows"
    ),
    decl!(
        Effort,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/agy.rs:57-64",
        "AGY .pb/LevelDB capture writes no usage_facts rows"
    ),
    decl!(
        ContextWindowUsage,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/agy.rs:57-64",
        "AGY .pb/LevelDB capture writes no context_facts rows"
    ),
    decl!(
        ToolCallName,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/agy.rs:57-64",
        "AGY .pb/LevelDB capture writes no tool_facts rows"
    ),
    decl!(
        ToolOutcome,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/agy.rs:57-64",
        "AGY .pb/LevelDB capture writes no tool_facts rows"
    ),
    decl!(
        ToolFingerprint,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/agy.rs:57-64",
        "AGY .pb/LevelDB capture writes no tool_facts rows"
    ),
    decl!(
        SessionEdges,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/agy.rs:57-64",
        "AGY .pb/LevelDB capture writes no session_edges rows"
    ),
    decl!(
        SkillDeclarations,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/agy.rs:57-64",
        "AGY .pb/LevelDB capture writes no skill_facts rows"
    ),
    decl!(
        SkillInvocations,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/agy.rs:57-64",
        "AGY .pb/LevelDB capture writes no skill_facts rows"
    ),
    decl!(
        ActivityIntervals,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/agy.rs:57-64",
        "AGY .pb/LevelDB capture writes no activity_facts rows"
    ),
    decl!(
        QueueTransitions,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/agy.rs:57-64",
        "AGY .pb/LevelDB capture writes no queue_facts rows"
    ),
    decl!(
        ProcessIdentity,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/agy.rs:57-64",
        "host process identities are collected by environment_v1"
    ),
    decl!(
        ProcessResourceSamples,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/agy.rs:57-64",
        "host process resource samples are collected by environment_v1"
    ),
    decl!(
        PrivilegedEffectTrace,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/agy.rs:57-64",
        "syscall/ETW traces require syscall_v1 or windows_events_v1 collectors"
    ),
    decl!(
        SecurityPolicy,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/agy.rs:57-64",
        "AGY .pb/LevelDB capture writes no security_facts rows"
    ),
    decl!(
        SecurityCommandCheck,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/agy.rs:57-64",
        "AGY .pb/LevelDB capture writes no security_facts rows"
    ),
    decl!(
        SourceCoverage,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/agy.rs:57-64",
        "AGY ingest refuses before committing source_files coverage rows"
    ),
    decl!(
        TrajectoryVector,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/agy.rs:57-64",
        "AGY .pb/LevelDB capture writes no harness_trajectory rows"
    ),
    decl!(
        HarnessEnvelopeShape,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/agy.rs:57-64",
        "AGY .pb/LevelDB capture writes no harness_envelope rows"
    ),
];

const CURSOR_FIELDS: &[FieldDeclaration] = &[
    // crates/dendro-connectors/src/exports.rs:284-319 (`cursor()` emits only `tool_facts`, no `usage_facts`)
    decl!(
        InputTokens,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:284-319",
        "Cursor export parser emits tool_facts only and no usage_facts"
    ),
    decl!(
        OutputTokens,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:284-319",
        "Cursor export parser emits tool_facts only and no usage_facts"
    ),
    decl!(
        CacheReadTokens,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:284-319",
        "Cursor export parser emits tool_facts only and no usage_facts"
    ),
    decl!(
        CacheWriteTokens,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:284-319",
        "Cursor export parser emits tool_facts only and no usage_facts"
    ),
    decl!(
        ReasoningTokens,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:284-319",
        "Cursor export parser emits tool_facts only and no usage_facts"
    ),
    decl!(
        ObservedCost,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:284-319",
        "Cursor export parser emits tool_facts only and no usage_facts"
    ),
    decl!(
        MessageId,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:284-319",
        "Cursor export parser emits no usage_facts message_id"
    ),
    // crates/dendro-connectors/src/exports.rs:285 (`req_str(&value, "composerId")`)
    decl!(
        NativeSession,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/exports.rs:285",
        "composerId parsed as native_session on Cursor export lines"
    ),
    // crates/dendro-connectors/src/exports.rs:310-312 (`opt_time(&value, "createdAt")`)
    decl!(
        TurnTimestamp,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/exports.rs:310-312",
        "createdAt parsed on Cursor tool_call events"
    ),
    decl!(
        Model,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:284-319",
        "Cursor export parser emits no usage_facts model"
    ),
    decl!(
        Effort,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:284-319",
        "Cursor export parser emits no usage_facts effort"
    ),
    decl!(
        ContextWindowUsage,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:284-319",
        "Cursor export parser emits no context_facts"
    ),
    // crates/dendro-connectors/src/exports.rs:298-302,313 (`tool_call.name`)
    decl!(
        ToolCallName,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/exports.rs:298-302",
        "tool_call.name parsed directly into tool_facts"
    ),
    // crates/dendro-connectors/src/exports.rs:303-309 (`tool_call.status` -> `"completed"` / `"error"` / `"failed"`)
    decl!(
        ToolOutcome,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/exports.rs:303-309",
        "tool_call.status mapped to Some(false) for completed and Some(true) for error/failed"
    ),
    // crates/dendro-connectors/src/exports.rs:278 (`fingerprint: None`)
    decl!(
        ToolFingerprint,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:278",
        "ExportAcc::tool sets fingerprint to None"
    ),
    decl!(
        SessionEdges,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:117-125",
        "Cursor export parser emits no session_edges"
    ),
    decl!(
        SkillDeclarations,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:117-125",
        "Cursor export parser emits no skill_facts"
    ),
    decl!(
        SkillInvocations,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:117-125",
        "Cursor export parser emits no skill_facts"
    ),
    decl!(
        ActivityIntervals,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:117-125",
        "Cursor export parser emits no activity_facts"
    ),
    decl!(
        QueueTransitions,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:117-125",
        "Cursor export parser emits no queue_facts"
    ),
    decl!(
        ProcessIdentity,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:117-125",
        "host process identities are collected by environment_v1"
    ),
    decl!(
        ProcessResourceSamples,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:117-125",
        "host process resource samples are collected by environment_v1"
    ),
    decl!(
        PrivilegedEffectTrace,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:117-125",
        "syscall/ETW traces require syscall_v1 or windows_events_v1 collectors"
    ),
    decl!(
        SecurityPolicy,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:117-125",
        "Cursor export parser emits no security_facts"
    ),
    decl!(
        SecurityCommandCheck,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:117-125",
        "Cursor export parser emits no security_facts"
    ),
    // crates/dendro-connectors/src/exports.rs:117-125
    decl!(
        SourceCoverage,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/exports.rs:117-125",
        "commit_file_batch_observed records source_files and reader completeness"
    ),
    decl!(
        TrajectoryVector,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:117-125",
        "Cursor export parser emits no harness_trajectory rows"
    ),
    decl!(
        HarnessEnvelopeShape,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:117-125",
        "Cursor export parser emits no harness_envelope rows"
    ),
];

const WINDSURF_FIELDS: &[FieldDeclaration] = &[
    decl!(
        InputTokens,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:320-350",
        "Windsurf export parser emits tool_facts only and no usage_facts"
    ),
    decl!(
        OutputTokens,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:320-350",
        "Windsurf export parser emits tool_facts only and no usage_facts"
    ),
    decl!(
        CacheReadTokens,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:320-350",
        "Windsurf export parser emits tool_facts only and no usage_facts"
    ),
    decl!(
        CacheWriteTokens,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:320-350",
        "Windsurf export parser emits tool_facts only and no usage_facts"
    ),
    decl!(
        ReasoningTokens,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:320-350",
        "Windsurf export parser emits tool_facts only and no usage_facts"
    ),
    decl!(
        ObservedCost,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:320-350",
        "Windsurf export parser emits tool_facts only and no usage_facts"
    ),
    decl!(
        MessageId,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:320-350",
        "Windsurf export parser emits no usage_facts message_id"
    ),
    // crates/dendro-connectors/src/exports.rs:321,546-548 (`session_from_rel` digest of relative file path)
    decl!(
        NativeSession,
        FieldFidelity::Derived,
        "crates/dendro-connectors/src/exports.rs:321,546-548",
        "derived from relative export file path SHA-256 digest (windsurf:<hex>)"
    ),
    // crates/dendro-connectors/src/exports.rs:329,344 (`ts` is always `None`)
    decl!(
        TurnTimestamp,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:329,344",
        "Windsurf trajectory export parser passes None for tool_facts timestamp"
    ),
    decl!(
        Model,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:320-350",
        "Windsurf export parser emits no usage_facts model"
    ),
    decl!(
        Effort,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:320-350",
        "Windsurf export parser emits no usage_facts effort"
    ),
    decl!(
        ContextWindowUsage,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:320-350",
        "Windsurf export parser emits no context_facts"
    ),
    // crates/dendro-connectors/src/exports.rs:322-347 (`action_type` / `step_type == "code_action"`)
    decl!(
        ToolCallName,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/exports.rs:322-347",
        "code_action step_type and action_type parsed into tool_facts"
    ),
    // crates/dendro-connectors/src/exports.rs:327-345 (only `status == "done"` emits `Some(false)`)
    decl!(
        ToolOutcome,
        FieldFidelity::Partial(PartialFidelityReason::CompletedCodeActionOnly),
        "crates/dendro-connectors/src/exports.rs:327-345",
        "only code_action steps with status == done are recorded (with failed = Some(false))"
    ),
    decl!(
        ToolFingerprint,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:278",
        "ExportAcc::tool sets fingerprint to None"
    ),
    decl!(
        SessionEdges,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:117-125",
        "Windsurf export parser emits no session_edges"
    ),
    decl!(
        SkillDeclarations,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:117-125",
        "Windsurf export parser emits no skill_facts"
    ),
    decl!(
        SkillInvocations,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:117-125",
        "Windsurf export parser emits no skill_facts"
    ),
    decl!(
        ActivityIntervals,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:117-125",
        "Windsurf export parser emits no activity_facts"
    ),
    decl!(
        QueueTransitions,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:117-125",
        "Windsurf export parser emits no queue_facts"
    ),
    decl!(
        ProcessIdentity,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:117-125",
        "host process identities are collected by environment_v1"
    ),
    decl!(
        ProcessResourceSamples,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:117-125",
        "host process resource samples are collected by environment_v1"
    ),
    decl!(
        PrivilegedEffectTrace,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:117-125",
        "syscall/ETW traces require syscall_v1 or windows_events_v1 collectors"
    ),
    decl!(
        SecurityPolicy,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:117-125",
        "Windsurf export parser emits no security_facts"
    ),
    decl!(
        SecurityCommandCheck,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:117-125",
        "Windsurf export parser emits no security_facts"
    ),
    // crates/dendro-connectors/src/exports.rs:117-125
    decl!(
        SourceCoverage,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/exports.rs:117-125",
        "commit_file_batch_observed records source_files and reader completeness"
    ),
    decl!(
        TrajectoryVector,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:117-125",
        "Windsurf export parser emits no harness_trajectory rows"
    ),
    decl!(
        HarnessEnvelopeShape,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:117-125",
        "Windsurf export parser emits no harness_envelope rows"
    ),
];

const GOOSE_FIELDS: &[FieldDeclaration] = &[
    // crates/dendro-connectors/src/exports.rs:359-379 (`total_input_tokens` on legacy header lines only)
    decl!(
        InputTokens,
        FieldFidelity::Partial(PartialFidelityReason::LegacyHeaderLifetimeTotals),
        "crates/dendro-connectors/src/exports.rs:359-379",
        "only legacy Goose v0.8 session header rows supply total_input_tokens; v1.x message rows omit token counts"
    ),
    // crates/dendro-connectors/src/exports.rs:359-379 (`total_output_tokens` on legacy header lines only)
    decl!(
        OutputTokens,
        FieldFidelity::Partial(PartialFidelityReason::LegacyHeaderLifetimeTotals),
        "crates/dendro-connectors/src/exports.rs:359-379",
        "only legacy Goose v0.8 session header rows supply total_output_tokens; v1.x message rows omit token counts"
    ),
    // crates/dendro-connectors/src/exports.rs:271,375 (`ExportAcc::usage` writes `cache_read: 0`)
    decl!(
        CacheReadTokens,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:271,375",
        "Goose exports do not report cache_read tokens (ExportAcc::usage writes 0)"
    ),
    // crates/dendro-connectors/src/exports.rs:270,375 (`ExportAcc::usage` writes `cache_write: 0`)
    decl!(
        CacheWriteTokens,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:270,375",
        "Goose exports do not report cache_write tokens (ExportAcc::usage writes 0)"
    ),
    decl!(
        ReasoningTokens,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:268,375",
        "Goose exports do not report reasoning tokens"
    ),
    decl!(
        ObservedCost,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:269,375",
        "Goose exports do not report observed USD cost"
    ),
    decl!(
        MessageId,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:270,375",
        "Goose usage rows do not carry a qualified message_id"
    ),
    // crates/dendro-connectors/src/exports.rs:352,546-548 (`session_from_rel`)
    decl!(
        NativeSession,
        FieldFidelity::Derived,
        "crates/dendro-connectors/src/exports.rs:352,546-548",
        "derived from relative export file path SHA-256 digest (goose:<hex>)"
    ),
    // crates/dendro-connectors/src/exports.rs:373-375,387-389 (`updated_at` / `created`)
    decl!(
        TurnTimestamp,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/exports.rs:373-375,387-389",
        "updated_at and created timestamps parsed from Goose export rows"
    ),
    decl!(
        Model,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:273,375",
        "Goose export parser writes model: None on usage_facts"
    ),
    decl!(
        Effort,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:274,375",
        "Goose export parser writes effort: None on usage_facts"
    ),
    decl!(
        ContextWindowUsage,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:351-425",
        "Goose export parser emits no context_facts"
    ),
    // crates/dendro-connectors/src/exports.rs:405-409 (`toolRequest` `toolCall.value.name`)
    decl!(
        ToolCallName,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/exports.rs:405-409",
        "content[].type=toolRequest toolCall.value.name parsed into tool_facts"
    ),
    // crates/dendro-connectors/src/exports.rs:397-409 (skips `"error"` status and passes `None` for `failed` on `"success"`)
    decl!(
        ToolOutcome,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:397-409",
        "Goose export parser skips error toolCall rows and writes failed=None on success rows"
    ),
    decl!(
        ToolFingerprint,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:278",
        "ExportAcc::tool sets fingerprint to None"
    ),
    decl!(
        SessionEdges,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:117-125",
        "Goose export parser emits no session_edges"
    ),
    decl!(
        SkillDeclarations,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:117-125",
        "Goose export parser emits no skill_facts"
    ),
    decl!(
        SkillInvocations,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:117-125",
        "Goose export parser emits no skill_facts"
    ),
    decl!(
        ActivityIntervals,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:117-125",
        "Goose export parser emits no activity_facts"
    ),
    decl!(
        QueueTransitions,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:117-125",
        "Goose export parser emits no queue_facts"
    ),
    decl!(
        ProcessIdentity,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:117-125",
        "host process identities are collected by environment_v1"
    ),
    decl!(
        ProcessResourceSamples,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:117-125",
        "host process resource samples are collected by environment_v1"
    ),
    decl!(
        PrivilegedEffectTrace,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:117-125",
        "syscall/ETW traces require syscall_v1 or windows_events_v1 collectors"
    ),
    decl!(
        SecurityPolicy,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:117-125",
        "Goose export parser emits no security_facts"
    ),
    decl!(
        SecurityCommandCheck,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:117-125",
        "Goose export parser emits no security_facts"
    ),
    // crates/dendro-connectors/src/exports.rs:117-125
    decl!(
        SourceCoverage,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/exports.rs:117-125",
        "commit_file_batch_observed records source_files and reader completeness"
    ),
    decl!(
        TrajectoryVector,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:117-125",
        "Goose export parser emits no harness_trajectory rows"
    ),
    decl!(
        HarnessEnvelopeShape,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:117-125",
        "Goose export parser emits no harness_envelope rows"
    ),
];

const DEVIN_FIELDS: &[FieldDeclaration] = &[
    // crates/dendro-connectors/src/exports.rs:455-475 (`prompt_tokens` when `cached_tokens == 0`)
    decl!(
        InputTokens,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/exports.rs:455-475",
        "llm_cost prompt_tokens parsed when cached_tokens is zero or absent"
    ),
    // crates/dendro-connectors/src/exports.rs:458-475 (`completion_tokens`)
    decl!(
        OutputTokens,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/exports.rs:458-475",
        "llm_cost completion_tokens parsed directly"
    ),
    // crates/dendro-connectors/src/exports.rs:459-464,271 (refuses positive `cached_tokens`; `ExportAcc::usage` writes `0`)
    decl!(
        CacheReadTokens,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:459-464",
        "Devin parser rejects positive cached_tokens and ExportAcc::usage writes 0 for cache_read"
    ),
    // crates/dendro-connectors/src/exports.rs:270,475 (`ExportAcc::usage` writes `cache_write: 0`)
    decl!(
        CacheWriteTokens,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:270,475",
        "Devin exports do not report cache_write tokens (ExportAcc::usage writes 0)"
    ),
    decl!(
        ReasoningTokens,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:268,475",
        "Devin exports do not report reasoning tokens"
    ),
    // crates/dendro-connectors/src/exports.rs:465-475 (`estimated_cost_usd`)
    decl!(
        ObservedCost,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/exports.rs:465-475",
        "llm_cost estimated_cost_usd parsed into usage_facts.currency_total"
    ),
    decl!(
        MessageId,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:270,475",
        "Devin llm_cost events carry no message_id"
    ),
    // crates/dendro-connectors/src/exports.rs:427 (`req_str(&value, "session_id")`)
    decl!(
        NativeSession,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/exports.rs:427",
        "session_id parsed on every Devin export event"
    ),
    // crates/dendro-connectors/src/exports.rs:452-454 (`req_time(&value, "timestamp")`)
    decl!(
        TurnTimestamp,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/exports.rs:452-454",
        "timestamp required and parsed on Devin llm_cost and tool_execution events"
    ),
    decl!(
        Model,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:273,475",
        "Devin export parser writes model: None on usage_facts"
    ),
    decl!(
        Effort,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:274,475",
        "Devin export parser writes effort: None on usage_facts"
    ),
    decl!(
        ContextWindowUsage,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:426-503",
        "Devin export parser emits no context_facts"
    ),
    // crates/dendro-connectors/src/exports.rs:480-498 (`tool_execution` `tool_name` / `tool` / `action`)
    decl!(
        ToolCallName,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/exports.rs:480-498",
        "tool_execution tool_name/tool/action parsed into tool_facts"
    ),
    // crates/dendro-connectors/src/exports.rs:495 (`acc.tool(..., None)`)
    decl!(
        ToolOutcome,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:495",
        "Devin tool_execution parser always passes failed=None"
    ),
    decl!(
        ToolFingerprint,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:278",
        "ExportAcc::tool sets fingerprint to None"
    ),
    decl!(
        SessionEdges,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:117-125",
        "Devin export parser emits no session_edges"
    ),
    decl!(
        SkillDeclarations,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:117-125",
        "Devin export parser emits no skill_facts"
    ),
    decl!(
        SkillInvocations,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:117-125",
        "Devin export parser emits no skill_facts"
    ),
    decl!(
        ActivityIntervals,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:117-125",
        "Devin export parser emits no activity_facts"
    ),
    decl!(
        QueueTransitions,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:117-125",
        "Devin export parser emits no queue_facts"
    ),
    decl!(
        ProcessIdentity,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:117-125",
        "host process identities are collected by environment_v1"
    ),
    decl!(
        ProcessResourceSamples,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:117-125",
        "host process resource samples are collected by environment_v1"
    ),
    decl!(
        PrivilegedEffectTrace,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:117-125",
        "syscall/ETW traces require syscall_v1 or windows_events_v1 collectors"
    ),
    decl!(
        SecurityPolicy,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:117-125",
        "Devin export parser emits no security_facts"
    ),
    decl!(
        SecurityCommandCheck,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:117-125",
        "Devin export parser emits no security_facts"
    ),
    // crates/dendro-connectors/src/exports.rs:117-125
    decl!(
        SourceCoverage,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/exports.rs:117-125",
        "commit_file_batch_observed records source_files and reader completeness"
    ),
    decl!(
        TrajectoryVector,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:117-125",
        "Devin export parser emits no harness_trajectory rows"
    ),
    decl!(
        HarnessEnvelopeShape,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/exports.rs:117-125",
        "Devin export parser emits no harness_envelope rows"
    ),
];

const OPENRIG_FIELDS: &[FieldDeclaration] = &[
    decl!(
        InputTokens,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/ingest.rs:468-518",
        "OpenRig coordination state does not carry LLM token counters"
    ),
    decl!(
        OutputTokens,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/ingest.rs:468-518",
        "OpenRig coordination state does not carry LLM token counters"
    ),
    decl!(
        CacheReadTokens,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/ingest.rs:468-518",
        "OpenRig coordination state does not carry LLM token counters"
    ),
    decl!(
        CacheWriteTokens,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/ingest.rs:468-518",
        "OpenRig coordination state does not carry LLM token counters"
    ),
    decl!(
        ReasoningTokens,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/ingest.rs:468-518",
        "OpenRig coordination state does not carry LLM token counters"
    ),
    decl!(
        ObservedCost,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/ingest.rs:468-518",
        "OpenRig coordination state does not carry observed USD cost"
    ),
    decl!(
        MessageId,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/ingest.rs:468-518",
        "OpenRig coordination state does not carry LLM call message_id"
    ),
    // crates/dendro-connectors/src/ingest.rs:519-546 (`identity_facts` `native_session`)
    decl!(
        NativeSession,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/ingest.rs:519-546",
        "seat/session bindings parsed from OpenRig state into identity_facts"
    ),
    // crates/dendro-connectors/src/ingest.rs:640-715,790-885 (`activity_facts.ts` and `queue_facts.ts`)
    decl!(
        TurnTimestamp,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/ingest.rs:640-715",
        "timestamps parsed on OpenRig activity and queue transition records"
    ),
    decl!(
        Model,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/ingest.rs:468-518",
        "OpenRig connector does not write usage_facts.model"
    ),
    decl!(
        Effort,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/ingest.rs:468-518",
        "OpenRig connector does not write usage_facts.effort"
    ),
    decl!(
        ContextWindowUsage,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/ingest.rs:468-518",
        "OpenRig connector does not write context_facts"
    ),
    decl!(
        ToolCallName,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/ingest.rs:468-518",
        "OpenRig connector writes only the synthetic reader-complete marker in tool_facts"
    ),
    decl!(
        ToolOutcome,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/ingest.rs:468-518",
        "OpenRig connector writes no tool execution outcomes"
    ),
    decl!(
        ToolFingerprint,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/ingest.rs:468-518",
        "OpenRig connector writes no tool argument fingerprints"
    ),
    decl!(
        SessionEdges,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/ingest.rs:468-518",
        "OpenRig connector does not populate session_edges"
    ),
    // crates/dendro-connectors/src/ingest.rs:996-1009 (`agent_skills.json` -> `skill_facts` with `invoked = false`)
    decl!(
        SkillDeclarations,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/ingest.rs:996-1009",
        "agent_skills.json entries recorded in skill_facts with invoked=false"
    ),
    decl!(
        SkillInvocations,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/ingest.rs:996-1009",
        "OpenRig agent_skills.json declares loaded skills (invoked=false), not runtime invocations"
    ),
    // crates/dendro-connectors/src/ingest.rs:640-715 (`activity_facts`)
    decl!(
        ActivityIntervals,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/ingest.rs:640-715",
        "seat activity logs parsed into activity_facts (seat_id, ts, state)"
    ),
    // crates/dendro-connectors/src/ingest.rs:790-885 (`queue_facts`)
    decl!(
        QueueTransitions,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/ingest.rs:790-885",
        "durable queue transitions and closure_reason parsed into queue_facts"
    ),
    decl!(
        ProcessIdentity,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/openrig.rs:1-42",
        "host process identities are collected by environment_v1"
    ),
    decl!(
        ProcessResourceSamples,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/openrig.rs:1-42",
        "host process resource samples are collected by environment_v1"
    ),
    decl!(
        PrivilegedEffectTrace,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/openrig.rs:1-42",
        "syscall/ETW traces require syscall_v1 or windows_events_v1 collectors"
    ),
    decl!(
        SecurityPolicy,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/openrig.rs:1-42",
        "OpenRig connector does not write security_facts"
    ),
    decl!(
        SecurityCommandCheck,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/openrig.rs:1-42",
        "OpenRig connector does not write security_facts"
    ),
    // crates/dendro-connectors/src/ingest.rs:468-518
    decl!(
        SourceCoverage,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/ingest.rs:468-518",
        "OpenRig ingest commits source_files watermarks and reader completeness markers"
    ),
    decl!(
        TrajectoryVector,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/openrig.rs:1-42",
        "OpenRig connector does not write harness_trajectory rows"
    ),
    decl!(
        HarnessEnvelopeShape,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/openrig.rs:1-42",
        "OpenRig connector does not write harness_envelope rows"
    ),
];

const ENVIRONMENT_FIELDS: &[FieldDeclaration] = &[
    decl!(
        InputTokens,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/environment_v1/mod.rs:344-496",
        "environment_v1 samples OS processes and emits no LLM token facts"
    ),
    decl!(
        OutputTokens,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/environment_v1/mod.rs:344-496",
        "environment_v1 samples OS processes and emits no LLM token facts"
    ),
    decl!(
        CacheReadTokens,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/environment_v1/mod.rs:344-496",
        "environment_v1 samples OS processes and emits no LLM token facts"
    ),
    decl!(
        CacheWriteTokens,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/environment_v1/mod.rs:344-496",
        "environment_v1 samples OS processes and emits no LLM token facts"
    ),
    decl!(
        ReasoningTokens,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/environment_v1/mod.rs:344-496",
        "environment_v1 samples OS processes and emits no LLM token facts"
    ),
    decl!(
        ObservedCost,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/environment_v1/mod.rs:344-496",
        "environment_v1 samples OS processes and emits no cost facts"
    ),
    decl!(
        MessageId,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/environment_v1/mod.rs:344-496",
        "environment_v1 samples OS processes and emits no message_id"
    ),
    decl!(
        NativeSession,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/environment_v1/mod.rs:344-496",
        "environment_v1 does not populate usage_facts or tool_facts native_session"
    ),
    decl!(
        TurnTimestamp,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/environment_v1/mod.rs:344-496",
        "environment_v1 records process sample timestamps in process_samples, not turn timestamps"
    ),
    decl!(
        Model,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/environment_v1/mod.rs:344-496",
        "environment_v1 emits no model bindings"
    ),
    decl!(
        Effort,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/environment_v1/mod.rs:344-496",
        "environment_v1 emits no effort bindings"
    ),
    decl!(
        ContextWindowUsage,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/environment_v1/mod.rs:344-496",
        "environment_v1 emits no context_facts"
    ),
    decl!(
        ToolCallName,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/environment_v1/mod.rs:344-496",
        "environment_v1 emits no tool_facts"
    ),
    decl!(
        ToolOutcome,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/environment_v1/mod.rs:344-496",
        "environment_v1 emits no tool_facts"
    ),
    decl!(
        ToolFingerprint,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/environment_v1/mod.rs:344-496",
        "environment_v1 emits no tool_facts"
    ),
    decl!(
        SessionEdges,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/environment_v1/mod.rs:344-496",
        "environment_v1 emits no session_edges"
    ),
    decl!(
        SkillDeclarations,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/environment_v1/mod.rs:344-496",
        "environment_v1 emits no skill_facts"
    ),
    decl!(
        SkillInvocations,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/environment_v1/mod.rs:344-496",
        "environment_v1 emits no skill_facts"
    ),
    decl!(
        ActivityIntervals,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/environment_v1/mod.rs:344-496",
        "environment_v1 emits no activity_facts"
    ),
    decl!(
        QueueTransitions,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/environment_v1/mod.rs:344-496",
        "environment_v1 emits no queue_facts"
    ),
    // crates/dendro-connectors/src/environment_v1/mod.rs:344-428 (`ProcessIdentityFact`)
    decl!(
        ProcessIdentity,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/environment_v1/mod.rs:344-428",
        "OS process tree identity, exe_name, platform, and environment_kind sampled into process_identities"
    ),
    // crates/dendro-connectors/src/environment_v1/mod.rs:430-496 (`ProcessSampleFact`)
    decl!(
        ProcessResourceSamples,
        FieldFidelity::Observed,
        "crates/dendro-connectors/src/environment_v1/mod.rs:430-496",
        "OS process CPU time, RSS/working set, and disk I/O counters sampled into process_samples"
    ),
    decl!(
        PrivilegedEffectTrace,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/environment_v1/mod.rs:344-496",
        "syscall/ETW traces are collected by syscall_v1 or windows_events_v1, not environment_v1"
    ),
    decl!(
        SecurityPolicy,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/environment_v1/mod.rs:344-496",
        "environment_v1 emits no security_facts"
    ),
    decl!(
        SecurityCommandCheck,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/environment_v1/mod.rs:344-496",
        "environment_v1 emits no security_facts"
    ),
    decl!(
        SourceCoverage,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/environment_v1/mod.rs:344-496",
        "environment_v1 writes process tables rather than source_files watermarks"
    ),
    decl!(
        TrajectoryVector,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/environment_v1/mod.rs:344-496",
        "environment_v1 emits no harness_trajectory rows"
    ),
    decl!(
        HarnessEnvelopeShape,
        FieldFidelity::Absent,
        "crates/dendro-connectors/src/environment_v1/mod.rs:344-496",
        "environment_v1 emits no harness_envelope rows"
    ),
];

/// Canonical field-level fidelity declarations for every registered connector.
pub static CONNECTOR_DECLARATIONS: &[ConnectorDeclaration] = &[
    ConnectorDeclaration {
        connector: "claude",
        aliases: &["claude-code"],
        capture_class: "native_transcript",
        fields: CLAUDE_FIELDS,
    },
    ConnectorDeclaration {
        connector: "codex",
        aliases: &[],
        capture_class: "native_transcript",
        fields: CODEX_FIELDS,
    },
    ConnectorDeclaration {
        connector: "pi",
        aliases: &[],
        capture_class: "native_transcript",
        fields: PI_FIELDS,
    },
    ConnectorDeclaration {
        connector: "grok",
        aliases: &[],
        capture_class: "native_transcript",
        fields: GROK_FIELDS,
    },
    ConnectorDeclaration {
        connector: "agy",
        aliases: &["antigravity"],
        capture_class: "metadata_only",
        fields: AGY_FIELDS,
    },
    ConnectorDeclaration {
        connector: "cursor",
        aliases: &[],
        capture_class: "export_only",
        fields: CURSOR_FIELDS,
    },
    ConnectorDeclaration {
        connector: "windsurf",
        aliases: &[],
        capture_class: "export_only",
        fields: WINDSURF_FIELDS,
    },
    ConnectorDeclaration {
        connector: "goose",
        aliases: &[],
        capture_class: "export_only",
        fields: GOOSE_FIELDS,
    },
    ConnectorDeclaration {
        connector: "devin",
        aliases: &[],
        capture_class: "export_only",
        fields: DEVIN_FIELDS,
    },
    ConnectorDeclaration {
        connector: "openrig",
        aliases: &[],
        capture_class: "coordination_topology",
        fields: OPENRIG_FIELDS,
    },
    ConnectorDeclaration {
        connector: "environment",
        aliases: &["environment_v1"],
        capture_class: "host_process_sampler",
        fields: ENVIRONMENT_FIELDS,
    },
];

/// Look up a connector declaration by canonical name or registered alias (ASCII case-insensitive).
pub fn connector_declaration(name: &str) -> Option<&'static ConnectorDeclaration> {
    let trimmed = name.trim();
    if trimmed.is_empty() || !trimmed.is_ascii() {
        return None;
    }
    CONNECTOR_DECLARATIONS.iter().find(|decl| {
        decl.connector.eq_ignore_ascii_case(trimmed)
            || decl
                .aliases
                .iter()
                .any(|alias| alias.eq_ignore_ascii_case(trimmed))
    })
}

/// Normalize a connector name or alias to its canonical connector id.
pub fn canonical_connector_id(name: &str) -> Option<&'static str> {
    connector_declaration(name).map(|decl| decl.connector)
}

/// One connector admitted into an insight's denominator scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IncludedConnector {
    pub connector: &'static str,
    /// Required or optional fields on this insight that the connector supplies with `Partial` fidelity.
    pub partial_fields: Vec<(Field, PartialFidelityReason)>,
}

/// One connector excluded from an insight's denominator scope, with the required fields it lacks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConnectorExclusion {
    pub connector: String,
    pub missing: Vec<Field>,
    pub reason: String,
}

/// Denominator scope for a computable `(insight, connector_set)` evaluation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DenominatorScope {
    pub insight_id: &'static str,
    pub insight_name: &'static str,
    /// Connectors that supply every required field and contribute to the denominator.
    pub included: Vec<IncludedConnector>,
    /// Connectors excluded from the denominator so they are never silently pooled.
    pub excluded: Vec<ConnectorExclusion>,
}

impl DenominatorScope {
    pub fn includes(&self, connector: &str) -> bool {
        let Some(canonical) = canonical_connector_id(connector) else {
            return false;
        };
        self.included.iter().any(|inc| inc.connector == canonical)
    }

    pub fn is_excluded(&self, connector: &str) -> bool {
        let canonical = canonical_connector_id(connector).unwrap_or(connector);
        self.excluded
            .iter()
            .any(|exc| exc.connector.eq_ignore_ascii_case(canonical))
    }
}

/// Closed two-state evaluation of `(insight, connector_set)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum DenominatorEvaluation {
    Computable {
        scope: DenominatorScope,
    },
    NotComputable {
        insight_id: &'static str,
        insight_name: &'static str,
        missing: Vec<Field>,
        by_connector: Vec<ConnectorExclusion>,
    },
}

impl DenominatorEvaluation {
    pub const fn is_computable(&self) -> bool {
        matches!(self, DenominatorEvaluation::Computable { .. })
    }

    pub const fn is_not_computable(&self) -> bool {
        matches!(self, DenominatorEvaluation::NotComputable { .. })
    }
}

/// Evaluate whether `insight` is computable over `connectors`, and partition `connectors`
/// into the included denominator scope and excluded connectors.
pub fn evaluate_denominator<S: AsRef<str>>(
    insight: &dyn Insight,
    connectors: &[S],
) -> DenominatorEvaluation {
    let required = insight.required_fields();
    let optional = insight.optional_fields();
    let mut included = Vec::new();
    let mut excluded = Vec::new();
    let mut seen = BTreeSet::new();

    for raw in connectors {
        let raw_name = raw.as_ref().trim();
        match connector_declaration(raw_name) {
            Some(decl) => {
                if !seen.insert(decl.connector.to_string()) {
                    continue;
                }
                let missing: Vec<Field> = required
                    .iter()
                    .copied()
                    .filter(|field| !decl.supplies(*field))
                    .collect();
                if missing.is_empty() {
                    let partial_fields = required
                        .iter()
                        .chain(optional.iter())
                        .copied()
                        .filter_map(|field| match decl.fidelity(field) {
                            FieldFidelity::Partial(reason) => Some((field, reason)),
                            _ => None,
                        })
                        .collect();
                    included.push(IncludedConnector {
                        connector: decl.connector,
                        partial_fields,
                    });
                } else {
                    let field_list = missing
                        .iter()
                        .map(|field| field.as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    excluded.push(ConnectorExclusion {
                        connector: decl.connector.to_string(),
                        missing,
                        reason: format!("missing required field(s): {field_list}"),
                    });
                }
            }
            None => {
                let key = if raw_name.is_empty() {
                    "<empty>".to_string()
                } else {
                    raw_name.to_string()
                };
                if !seen.insert(key.clone()) {
                    continue;
                }
                let missing = required.to_vec();
                excluded.push(ConnectorExclusion {
                    connector: key,
                    missing,
                    reason: "unregistered connector".to_string(),
                });
            }
        }
    }

    if included.is_empty() {
        let mut union_missing = Vec::new();
        for field in required {
            if (excluded.is_empty() || excluded.iter().any(|exc| exc.missing.contains(field)))
                && !union_missing.contains(field)
            {
                union_missing.push(*field);
            }
        }
        DenominatorEvaluation::NotComputable {
            insight_id: insight.id(),
            insight_name: insight.name(),
            missing: union_missing,
            by_connector: excluded,
        }
    } else {
        DenominatorScope {
            insight_id: insight.id(),
            insight_name: insight.name(),
            included,
            excluded,
        }
        .into_computable()
    }
}

impl DenominatorScope {
    fn into_computable(self) -> DenominatorEvaluation {
        DenominatorEvaluation::Computable { scope: self }
    }
}

/// Evaluate denominator compatibility by insight id (e.g. `"I01"`) or canonical name (`"model.invocations"`).
pub fn evaluate_denominator_by_id<S: AsRef<str>>(
    insight_id_or_name: &str,
    connectors: &[S],
) -> Option<DenominatorEvaluation> {
    let insight = CATALOG
        .iter()
        .copied()
        .find(|item| item.id() == insight_id_or_name || item.name() == insight_id_or_name)?;
    Some(evaluate_denominator(insight, connectors))
}

/// One row of the full `CATALOG × CONNECTOR_DECLARATIONS` denominator matrix.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DenominatorMatrixRow {
    pub insight_id: &'static str,
    pub insight_name: &'static str,
    pub required_fields: &'static [Field],
    pub optional_fields: &'static [Field],
    pub included_connectors: Vec<&'static str>,
    pub excluded_connectors: Vec<ConnectorExclusion>,
}

/// Compute the complete denominator matrix across all 25 catalog insights and all 11 connectors.
pub fn denominator_matrix() -> Vec<DenominatorMatrixRow> {
    let all_connectors: Vec<&'static str> = CONNECTOR_DECLARATIONS
        .iter()
        .map(|decl| decl.connector)
        .collect();
    CATALOG
        .iter()
        .map(|insight| {
            let eval = evaluate_denominator(*insight, &all_connectors);
            let (included_connectors, excluded_connectors) = match eval {
                DenominatorEvaluation::Computable { scope } => (
                    scope
                        .included
                        .into_iter()
                        .map(|inc| inc.connector)
                        .collect(),
                    scope.excluded,
                ),
                DenominatorEvaluation::NotComputable { by_connector, .. } => {
                    (Vec::new(), by_connector)
                }
            };
            DenominatorMatrixRow {
                insight_id: insight.id(),
                insight_name: insight.name(),
                required_fields: insight.required_fields(),
                optional_fields: insight.optional_fields(),
                included_connectors,
                excluded_connectors,
            }
        })
        .collect()
}

/// Deterministic refusal reason string for `InsightStatus::Refused` when an insight is `NotComputable`.
pub fn format_not_computable_reason(
    missing: &[Field],
    by_connector: &[ConnectorExclusion],
) -> String {
    let fields = if missing.is_empty() {
        "required_fields".to_string()
    } else {
        missing
            .iter()
            .map(|field| field.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    if by_connector.is_empty() {
        format!("not_computable: missing {fields}")
    } else {
        let per_conn = by_connector
            .iter()
            .map(|exc| {
                let miss = exc
                    .missing
                    .iter()
                    .map(|field| field.as_str())
                    .collect::<Vec<_>>()
                    .join("+");
                if miss.is_empty() {
                    exc.connector.clone()
                } else {
                    format!("{}:{miss}", exc.connector)
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
        format!("not_computable: missing {fields} ({per_conn})")
    }
}

/// Format excluded connectors as `"excluded: <conn> — missing <f1>, <f2>"`.
pub fn format_exclusions_list(excluded: &[ConnectorExclusion]) -> Option<String> {
    if excluded.is_empty() {
        return None;
    }
    let parts = excluded
        .iter()
        .map(|item| {
            let miss = item
                .missing
                .iter()
                .map(|field| field.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            format!("excluded: {} — missing {miss}", item.connector)
        })
        .collect::<Vec<_>>();
    Some(parts.join("; "))
}

/// Format denominator inclusion/exclusion notes for `InsightResult.coverage` on mixed sources.
pub fn format_denominator_disclosure(scope: &DenominatorScope) -> Option<String> {
    format_exclusions_list(&scope.excluded)
}

fn distinct_canonical_harnesses_in_conn(conn: &duckdb::Connection, sql: &str) -> Vec<String> {
    let Ok(mut stmt) = conn.prepare(sql) else {
        return Vec::new();
    };
    let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(0)) else {
        return Vec::new();
    };
    let mut set = BTreeSet::new();
    for raw in rows.flatten() {
        if let Some(canonical) = canonical_connector_id(&raw) {
            set.insert(canonical.to_string());
        }
    }
    set.into_iter().collect()
}

fn exists_in_conn(conn: &duckdb::Connection, sql: &str) -> bool {
    conn.query_row(sql, [], |_| Ok(())).is_ok()
}

fn exists_for_harness_in_conn(conn: &duckdb::Connection, sql: &str, harness: &str) -> bool {
    use duckdb::params;
    conn.query_row(sql, params![harness], |_| Ok(())).is_ok()
}

/// Per-evaluation cache of connector presence in `AnalyticsStore` so `evaluate` / `evaluate_catalog`
/// never re-runs identical table scans across insights.
pub struct StoreDenominatorCache<'a> {
    store: &'a crate::AnalyticsStore,
    usage_harnesses: std::cell::OnceCell<Vec<String>>,
    source_harnesses: std::cell::OnceCell<Vec<String>>,
    tool_harnesses: std::cell::OnceCell<Vec<String>>,
    edge_harnesses: std::cell::OnceCell<Vec<String>>,
    security_harnesses: std::cell::OnceCell<Vec<String>>,
    context_harnesses: std::cell::OnceCell<Vec<String>>,
    skill_connectors: std::cell::OnceCell<Vec<String>>,
    has_activity: std::cell::OnceCell<bool>,
    has_queue: std::cell::OnceCell<bool>,
    has_skill_declarations: std::cell::OnceCell<bool>,
}

impl<'a> StoreDenominatorCache<'a> {
    pub fn new(store: &'a crate::AnalyticsStore) -> Self {
        Self {
            store,
            usage_harnesses: std::cell::OnceCell::new(),
            source_harnesses: std::cell::OnceCell::new(),
            tool_harnesses: std::cell::OnceCell::new(),
            edge_harnesses: std::cell::OnceCell::new(),
            security_harnesses: std::cell::OnceCell::new(),
            context_harnesses: std::cell::OnceCell::new(),
            skill_connectors: std::cell::OnceCell::new(),
            has_activity: std::cell::OnceCell::new(),
            has_queue: std::cell::OnceCell::new(),
            has_skill_declarations: std::cell::OnceCell::new(),
        }
    }

    fn usage(&self) -> &[String] {
        self.usage_harnesses.get_or_init(|| {
            distinct_canonical_harnesses_in_conn(
                self.store.connection(),
                "SELECT DISTINCT harness FROM usage_facts",
            )
        })
    }

    fn sources(&self) -> &[String] {
        self.source_harnesses.get_or_init(|| {
            distinct_canonical_harnesses_in_conn(
                self.store.connection(),
                "SELECT DISTINCT harness FROM source_files",
            )
        })
    }

    fn tools(&self) -> &[String] {
        self.tool_harnesses.get_or_init(|| {
            distinct_canonical_harnesses_in_conn(
                self.store.connection(),
                "SELECT DISTINCT harness FROM tool_facts WHERE NOT starts_with(source_key, '__dendro_reader__:')",
            )
        })
    }

    fn edges(&self) -> &[String] {
        self.edge_harnesses.get_or_init(|| {
            distinct_canonical_harnesses_in_conn(
                self.store.connection(),
                "SELECT DISTINCT harness FROM session_edges",
            )
        })
    }

    fn security(&self) -> &[String] {
        self.security_harnesses.get_or_init(|| {
            distinct_canonical_harnesses_in_conn(
                self.store.connection(),
                "SELECT DISTINCT harness FROM security_facts",
            )
        })
    }

    fn context(&self) -> &[String] {
        self.context_harnesses.get_or_init(|| {
            distinct_canonical_harnesses_in_conn(
                self.store.connection(),
                "SELECT DISTINCT harness FROM context_facts",
            )
        })
    }

    fn skills(&self) -> &[String] {
        self.skill_connectors.get_or_init(|| {
            let conn = self.store.connection();
            let mut set = BTreeSet::new();
            if let Ok(mut stmt) = conn.prepare("SELECT DISTINCT seat, source_key FROM skill_facts")
            {
                if let Ok(rows) = stmt.query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                }) {
                    for (seat, key) in rows.flatten() {
                        for cand in [
                            seat.trim(),
                            seat.split(':').next().unwrap_or(""),
                            key.split(':').next().unwrap_or(""),
                        ] {
                            if let Some(canonical) = canonical_connector_id(cand) {
                                set.insert(canonical.to_string());
                            }
                        }
                    }
                }
            }
            set.into_iter().collect()
        })
    }

    fn has_activity(&self) -> bool {
        *self.has_activity.get_or_init(|| {
            exists_in_conn(
                self.store.connection(),
                "SELECT 1 FROM activity_facts LIMIT 1",
            )
        })
    }

    fn has_queue(&self) -> bool {
        *self.has_queue.get_or_init(|| {
            exists_in_conn(self.store.connection(), "SELECT 1 FROM queue_facts LIMIT 1")
        })
    }

    fn has_skill_declarations(&self) -> bool {
        *self.has_skill_declarations.get_or_init(|| {
            exists_in_conn(
                self.store.connection(),
                "SELECT 1 FROM skill_facts WHERE invoked = FALSE LIMIT 1",
            )
        })
    }

    pub fn present_connectors(&self, insight: &dyn Insight) -> Vec<String> {
        let conn = self.store.connection();
        let mut set = BTreeSet::new();
        let extend = |set: &mut BTreeSet<String>, items: &[String]| {
            for item in items {
                set.insert(item.clone());
            }
        };

        match insight.id() {
            "I01" | "I02" | "I04" | "I05" | "I21" | "I25" => {
                extend(&mut set, self.usage());
                extend(&mut set, self.sources());
            }
            "I03" | "I11" => {
                extend(&mut set, self.usage());
                extend(&mut set, self.edges());
                extend(&mut set, self.sources());
            }
            "I06" => {
                extend(&mut set, self.tools());
                extend(&mut set, self.sources());
                extend(&mut set, self.skills());
            }
            "I07" => {
                extend(&mut set, self.context());
                extend(&mut set, self.sources());
            }
            "I08" => {
                extend(&mut set, self.tools());
                extend(&mut set, self.sources());
            }
            "I09" => {
                extend(&mut set, self.edges());
                extend(&mut set, self.sources());
            }
            "I10" => {
                if self.has_activity() {
                    set.insert("openrig".to_string());
                }
            }
            "I12" | "I19" => {
                if self.has_queue() {
                    set.insert("openrig".to_string());
                }
            }
            "I20" => {
                if self.has_queue() {
                    set.insert("openrig".to_string());
                }
                extend(&mut set, self.usage());
            }
            "I13" => {
                if exists_in_conn(conn, "SELECT 1 FROM process_identities LIMIT 1") {
                    set.insert("environment".to_string());
                }
            }
            "I14" => {
                if exists_in_conn(conn, "SELECT 1 FROM process_samples LIMIT 1") {
                    set.insert("environment".to_string());
                }
            }
            "I15" => {
                if exists_in_conn(conn, "SELECT 1 FROM syscall_facts LIMIT 1") {
                    extend(&mut set, self.sources());
                    if set.is_empty() {
                        extend(&mut set, self.usage());
                    }
                }
            }
            "I16" | "I17" => {
                extend(&mut set, self.security());
                extend(&mut set, self.sources());
            }
            "I18" => {
                extend(&mut set, self.sources());
            }
            "I22" => {
                for item in distinct_canonical_harnesses_in_conn(
                    conn,
                    "SELECT DISTINCT harness FROM harness_trajectory WHERE harness IS NOT NULL",
                ) {
                    set.insert(item);
                }
            }
            "I23" => {
                for item in distinct_canonical_harnesses_in_conn(
                    conn,
                    "SELECT DISTINCT harness FROM harness_envelope WHERE harness IS NOT NULL",
                ) {
                    set.insert(item);
                }
            }
            "I24" => {
                extend(&mut set, self.edges());
                extend(&mut set, self.security());
                extend(&mut set, self.sources());
            }
            _ => {
                extend(&mut set, self.usage());
                extend(&mut set, self.sources());
            }
        }
        set.into_iter().collect()
    }

    pub fn evaluate_denominator<S: AsRef<str>>(
        &self,
        insight: &dyn Insight,
        connectors: &[S],
        scoped_single_harness: bool,
    ) -> DenominatorEvaluation {
        let required = insight.required_fields();
        let optional = insight.optional_fields();
        let mut included = Vec::new();
        let mut excluded = Vec::new();

        let mut seen = BTreeSet::new();
        let mut resolved_decls = Vec::new();
        let mut unknown_connectors = Vec::new();
        for raw in connectors {
            let name = raw.as_ref().trim();
            if name.is_empty() {
                continue;
            }
            let canonical = canonical_connector_id(name).unwrap_or(name);
            if !seen.insert(canonical.to_string()) {
                continue;
            }
            if let Some(decl) = connector_declaration(canonical) {
                resolved_decls.push(decl);
            } else {
                unknown_connectors.push(canonical.to_string());
            }
        }

        for unknown in unknown_connectors {
            excluded.push(ConnectorExclusion {
                connector: unknown,
                missing: required.to_vec(),
                reason: "unregistered connector".to_string(),
            });
        }

        for decl in &resolved_decls {
            let missing: Vec<Field> = required
                .iter()
                .copied()
                .filter(|field| {
                    !self.connector_supplies_field(
                        decl,
                        *field,
                        insight,
                        &resolved_decls,
                        scoped_single_harness,
                    )
                })
                .collect();
            if missing.is_empty() {
                let mut partial_fields = Vec::new();
                for field in required.iter().chain(optional.iter()) {
                    if let FieldFidelity::Partial(reason) = decl.fidelity(*field) {
                        if !partial_fields.iter().any(|(f, _)| f == field) {
                            partial_fields.push((*field, reason));
                        }
                    }
                }
                included.push(IncludedConnector {
                    connector: decl.connector,
                    partial_fields,
                });
            } else {
                let field_list = missing
                    .iter()
                    .map(|field| field.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                excluded.push(ConnectorExclusion {
                    connector: decl.connector.to_string(),
                    missing,
                    reason: format!("missing required field(s): {field_list}"),
                });
            }
        }

        if included.is_empty() {
            let mut missing_union = BTreeSet::new();
            for exc in &excluded {
                for field in &exc.missing {
                    missing_union.insert(*field);
                }
            }
            let missing = if missing_union.is_empty() {
                required.to_vec()
            } else {
                missing_union.into_iter().collect()
            };
            DenominatorEvaluation::NotComputable {
                insight_id: insight.id(),
                insight_name: insight.name(),
                missing,
                by_connector: excluded,
            }
        } else {
            DenominatorEvaluation::Computable {
                scope: DenominatorScope {
                    insight_id: insight.id(),
                    insight_name: insight.name(),
                    included,
                    excluded,
                },
            }
        }
    }

    fn connector_supplies_field(
        &self,
        decl: &ConnectorDeclaration,
        field: Field,
        insight: &dyn Insight,
        connectors: &[&ConnectorDeclaration],
        scoped_single_harness: bool,
    ) -> bool {
        if decl.supplies(field) {
            return true;
        }
        if scoped_single_harness {
            return false;
        }
        let conn = self.store.connection();
        let has_source_file = self
            .sources()
            .iter()
            .any(|h| h.eq_ignore_ascii_case(decl.connector));

        match field {
            Field::ObservedCost => {
                exists_for_harness_in_conn(
                    conn,
                    "SELECT 1 FROM usage_facts WHERE lower(harness) = lower(?) AND currency_total IS NOT NULL LIMIT 1",
                    decl.connector,
                ) || (!has_source_file
                    && insight.id() == "I04"
                    && connectors.len() == 1
                    && decl.supplies(Field::InputTokens))
                    || (insight.id() == "I20"
                        && decl.connector == "openrig"
                        && exists_in_conn(
                            conn,
                            "SELECT 1 FROM usage_facts WHERE currency_total IS NOT NULL LIMIT 1",
                        ))
            }
            Field::QueueTransitions => {
                insight.id() == "I20"
                    && self.has_queue()
                    && (decl.supplies(Field::ObservedCost)
                        || exists_for_harness_in_conn(
                            conn,
                            "SELECT 1 FROM usage_facts WHERE lower(harness) = lower(?) AND currency_total IS NOT NULL LIMIT 1",
                            decl.connector,
                        ))
            }
            Field::SkillDeclarations => {
                self.has_skill_declarations() && decl.supplies(Field::SkillInvocations)
            }
            Field::SkillInvocations | Field::ToolCallName if decl.connector == "openrig" => {
                insight.id() == "I06"
                    && connectors.iter().any(|other| {
                        other.supplies(Field::SkillInvocations)
                            && other.supplies(Field::ToolCallName)
                    })
            }
            Field::ContextWindowUsage => exists_for_harness_in_conn(
                conn,
                "SELECT 1 FROM context_facts WHERE lower(harness) = lower(?) AND context_used IS NOT NULL AND context_limit IS NOT NULL LIMIT 1",
                decl.connector,
            ),
            Field::SessionEdges => {
                !has_source_file
                    && connectors.len() == 1
                    && !connectors.iter().any(|other| other.supplies(Field::SessionEdges))
                    && self
                        .edges()
                        .iter()
                        .any(|h| h.eq_ignore_ascii_case(decl.connector))
            }
            Field::PrivilegedEffectTrace => {
                exists_in_conn(conn, "SELECT 1 FROM syscall_facts LIMIT 1")
            }
            Field::TrajectoryVector => {
                exists_in_conn(conn, "SELECT 1 FROM harness_trajectory LIMIT 1")
            }
            Field::HarnessEnvelopeShape => exists_for_harness_in_conn(
                conn,
                "SELECT 1 FROM harness_envelope WHERE lower(harness) = lower(?) LIMIT 1",
                decl.connector,
            ),
            _ => false,
        }
    }
}

/// Discover the canonical connectors present in `store` for `insight`.
pub fn present_connectors_for_insight(
    insight: &dyn Insight,
    store: &crate::AnalyticsStore,
) -> Vec<String> {
    StoreDenominatorCache::new(store).present_connectors(insight)
}

/// Evaluate denominator scope for `insight` over `connectors` with awareness of `store`.
pub fn evaluate_denominator_in_store<S: AsRef<str>>(
    insight: &dyn Insight,
    connectors: &[S],
    store: &crate::AnalyticsStore,
    scoped_single_harness: bool,
) -> DenominatorEvaluation {
    StoreDenominatorCache::new(store).evaluate_denominator(
        insight,
        connectors,
        scoped_single_harness,
    )
}

/// Canonical population table name for an insight when refusing with `not_computable`.
pub fn canonical_population_what(insight_id: &str) -> &'static str {
    match insight_id {
        "I01" | "I02" | "I03" | "I04" | "I05" | "I11" | "I21" | "I25" => "usage_facts",
        "I06" => "skill_facts",
        "I07" => "context_facts",
        "I08" => "tool_facts",
        "I09" | "I24" => "session_edges",
        "I10" => "activity_facts",
        "I12" | "I19" | "I20" => "queue_facts",
        "I13" => "process_identities",
        "I14" => "process_samples",
        "I15" => "syscall_facts",
        "I16" | "I17" => "security_facts",
        "I18" => "source_files",
        "I22" => "harness_trajectory",
        "I23" => "harness_envelope",
        _ => "usage_facts",
    }
}

/// Render the deterministic Markdown tables for `docs/providers/provider-matrix.md`
/// between `<!-- BEGIN D5_FIELD_FIDELITY -->` and `<!-- END D5_FIELD_FIDELITY -->`.
pub fn render_provider_fidelity_markdown() -> String {
    let mut out = String::new();
    out.push_str("<!-- BEGIN D5_FIELD_FIDELITY -->\n");
    out.push_str("## Field-Level Fidelity Declarations (`dendro#134`)\n\n");
    out.push_str(
        "Every connector declares what it supplies per field (`Observed`, `Derived`, `Partial (<reason>)`, or `Absent`). `Absent` is the default.\n\n",
    );

    out.push_str("| Field | Category | Storage |");
    for decl in CONNECTOR_DECLARATIONS {
        out.push_str(&format!(" `{}` |", decl.connector));
    }
    out.push('\n');

    out.push_str("| :--- | :--- | :--- |");
    for _ in CONNECTOR_DECLARATIONS {
        out.push_str(" :--- |");
    }
    out.push('\n');

    for field in Field::ALL {
        out.push_str(&format!(
            "| `{}` | `{}` | `{}` |",
            field.as_str(),
            field.category(),
            field.storage_column()
        ));
        for decl in CONNECTOR_DECLARATIONS {
            out.push_str(&format!(" {} |", decl.fidelity(*field).label()));
        }
        out.push('\n');
    }

    out.push_str("\n## Insight Denominator Matrix (`I01`–`I25`)\n\n");
    out.push_str(
        "Each insight declares its required and optional fields in the compiled registry. A connector contributes to an insight's denominator only when it supplies every required field; excluded connectors are never pooled into mixed-source denominators.\n\n",
    );
    out.push_str(
        "| Insight | Name | Required Fields | Optional Fields | Denominator (Included) | Excluded Connectors (Missing Required Fields) |\n",
    );
    out.push_str("| :--- | :--- | :--- | :--- | :--- | :--- |\n");

    for row in denominator_matrix() {
        let req = row
            .required_fields
            .iter()
            .map(|f| format!("`{}`", f.as_str()))
            .collect::<Vec<_>>()
            .join(", ");
        let opt = if row.optional_fields.is_empty() {
            "none".to_string()
        } else {
            row.optional_fields
                .iter()
                .map(|f| format!("`{}`", f.as_str()))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let inc = if row.included_connectors.is_empty() {
            "none (`NotComputable`)".to_string()
        } else {
            row.included_connectors
                .iter()
                .map(|c| format!("`{c}`"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let exc = row
            .excluded_connectors
            .iter()
            .map(|e| {
                let miss = e
                    .missing
                    .iter()
                    .map(|f| f.as_str())
                    .collect::<Vec<_>>()
                    .join("+");
                format!("`{}` ({miss})", e.connector)
            })
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&format!(
            "| `{}` | `{}` | {} | {} | {} | {} |\n",
            row.insight_id, row.insight_name, req, opt, inc, exc
        ));
    }

    out.push_str("<!-- END D5_FIELD_FIDELITY -->");
    out
}

fn table_exists_in_conn(conn: &duckdb::Connection, name: &str) -> bool {
    conn.query_row(
        "SELECT COUNT(*) FROM information_schema.tables WHERE lower(table_name) = lower(?)",
        duckdb::params![name],
        |row| row.get::<_, i64>(0),
    )
    .unwrap_or(0)
        > 0
}

fn count_where_in_conn(conn: &duckdb::Connection, sql: &str) -> anyhow::Result<i64> {
    Ok(conn.query_row(sql, [], |row| row.get::<_, i64>(0))?)
}

/// Query `store` for a token [`Field`], returning `Some(value)` when the parser wrote the field
/// (including `Some(0)` when the parser wrote a real `0`) and `None` when the field was not written.
pub fn token_field_in_store(
    store: &crate::AnalyticsStore,
    field: Field,
) -> anyhow::Result<Option<i64>> {
    use duckdb::OptionalExt;

    let conn = store.connection();
    let sql = match field {
        Field::InputTokens => "SELECT input_uncached FROM usage_facts LIMIT 1",
        Field::OutputTokens => "SELECT output_tokens FROM usage_facts LIMIT 1",
        Field::CacheReadTokens => {
            "SELECT u.cache_read
             FROM usage_facts u
             LEFT JOIN source_files s ON u.source_id = s.source_id
             WHERE (s.source_id IS NOT NULL AND s.source_id <> s.display_path)
                OR u.cache_read <> 0
             LIMIT 1"
        }
        Field::CacheWriteTokens => {
            "SELECT u.cache_write
             FROM usage_facts u
             LEFT JOIN source_files s ON u.source_id = s.source_id
             WHERE (s.source_id IS NOT NULL AND s.source_id <> s.display_path)
                OR u.cache_write <> 0
             LIMIT 1"
        }
        Field::ReasoningTokens => {
            "SELECT u.reasoning
             FROM usage_facts u
             LEFT JOIN source_files s ON u.source_id = s.source_id
             WHERE (s.source_id IS NOT NULL AND s.source_id <> s.display_path AND s.native_session IS NULL)
                OR u.reasoning <> 0
             LIMIT 1"
        }
        _ => return Ok(None),
    };
    Ok(conn
        .query_row(sql, [], |row| row.get::<_, i64>(0))
        .optional()?)
}

/// Inspect `store` after ingesting a connector's fixture and return the set of [`Field`]s
/// that have concrete populated evidence in `store`.
pub fn populated_fields_in_store(store: &crate::AnalyticsStore) -> anyhow::Result<BTreeSet<Field>> {
    use crate::facts::READER_NAMESPACE;
    use duckdb::params;

    let conn = store.connection();
    let mut present = BTreeSet::new();

    for token_field in [
        Field::InputTokens,
        Field::OutputTokens,
        Field::CacheReadTokens,
        Field::CacheWriteTokens,
        Field::ReasoningTokens,
    ] {
        if token_field_in_store(store, token_field)?.is_some() {
            present.insert(token_field);
        }
    }
    if count_where_in_conn(
        conn,
        "SELECT COUNT(*) FROM usage_facts WHERE currency_total IS NOT NULL",
    )? > 0
    {
        present.insert(Field::ObservedCost);
    }
    if count_where_in_conn(
        conn,
        "SELECT COUNT(*) FROM usage_facts WHERE message_id IS NOT NULL AND trim(message_id) <> ''",
    )? > 0
    {
        present.insert(Field::MessageId);
    }

    let has_session = count_where_in_conn(
        conn,
        "SELECT COUNT(*) FROM usage_facts WHERE trim(native_session) <> '' AND lower(native_session) <> 'unknown'",
    )? > 0
        || conn
            .query_row(
                "SELECT COUNT(*) FROM tool_facts WHERE NOT starts_with(source_key, ?) AND trim(native_session) <> '' AND lower(native_session) <> 'unknown'",
                params![READER_NAMESPACE],
                |row| row.get::<_, i64>(0),
            )?
            > 0
        || count_where_in_conn(
            conn,
            "SELECT COUNT(*) FROM identity_facts WHERE trim(native_session) <> ''",
        )? > 0;
    if has_session {
        present.insert(Field::NativeSession);
    }

    let has_ts = count_where_in_conn(
        conn,
        "SELECT COUNT(*) FROM usage_facts WHERE ts IS NOT NULL",
    )? > 0
        || conn
            .query_row(
                "SELECT COUNT(*) FROM tool_facts WHERE NOT starts_with(source_key, ?) AND ts IS NOT NULL",
                params![READER_NAMESPACE],
                |row| row.get::<_, i64>(0),
            )?
            > 0
        || count_where_in_conn(
            conn,
            "SELECT COUNT(*) FROM activity_facts WHERE ts IS NOT NULL",
        )? > 0
        || count_where_in_conn(
            conn,
            "SELECT COUNT(*) FROM queue_facts WHERE ts IS NOT NULL",
        )? > 0;
    if has_ts {
        present.insert(Field::TurnTimestamp);
    }

    if count_where_in_conn(
        conn,
        "SELECT COUNT(*) FROM usage_facts WHERE model IS NOT NULL AND trim(model) <> ''",
    )? > 0
    {
        present.insert(Field::Model);
    }
    if count_where_in_conn(
        conn,
        "SELECT COUNT(*) FROM usage_facts WHERE effort IS NOT NULL AND trim(effort) <> ''",
    )? > 0
    {
        present.insert(Field::Effort);
    }
    if table_exists_in_conn(conn, "context_facts")
        && count_where_in_conn(conn, "SELECT COUNT(*) FROM context_facts")? > 0
    {
        present.insert(Field::ContextWindowUsage);
    }

    if conn.query_row(
        "SELECT COUNT(*) FROM tool_facts WHERE NOT starts_with(source_key, ?) AND trim(tool_name) <> ''",
        params![READER_NAMESPACE],
        |row| row.get::<_, i64>(0),
    )? > 0
    {
        present.insert(Field::ToolCallName);
    }
    if conn.query_row(
        "SELECT COUNT(*) FROM tool_facts WHERE NOT starts_with(source_key, ?) AND failed IS NOT NULL",
        params![READER_NAMESPACE],
        |row| row.get::<_, i64>(0),
    )? > 0
    {
        present.insert(Field::ToolOutcome);
    }
    if conn.query_row(
        "SELECT COUNT(*) FROM tool_facts WHERE NOT starts_with(source_key, ?) AND fingerprint IS NOT NULL AND trim(fingerprint) <> ''",
        params![READER_NAMESPACE],
        |row| row.get::<_, i64>(0),
    )? > 0
    {
        present.insert(Field::ToolFingerprint);
    }

    if table_exists_in_conn(conn, "session_edges")
        && count_where_in_conn(conn, "SELECT COUNT(*) FROM session_edges")? > 0
    {
        present.insert(Field::SessionEdges);
    }
    if count_where_in_conn(
        conn,
        "SELECT COUNT(*) FROM skill_facts WHERE invoked = FALSE",
    )? > 0
    {
        present.insert(Field::SkillDeclarations);
    }
    if count_where_in_conn(
        conn,
        "SELECT COUNT(*) FROM skill_facts WHERE invoked = TRUE",
    )? > 0
    {
        present.insert(Field::SkillInvocations);
    }
    if count_where_in_conn(conn, "SELECT COUNT(*) FROM activity_facts")? > 0 {
        present.insert(Field::ActivityIntervals);
    }
    if count_where_in_conn(conn, "SELECT COUNT(*) FROM queue_facts")? > 0 {
        present.insert(Field::QueueTransitions);
    }
    if table_exists_in_conn(conn, "process_identities")
        && count_where_in_conn(conn, "SELECT COUNT(*) FROM process_identities")? > 0
    {
        present.insert(Field::ProcessIdentity);
    }
    if table_exists_in_conn(conn, "process_samples")
        && count_where_in_conn(conn, "SELECT COUNT(*) FROM process_samples")? > 0
    {
        present.insert(Field::ProcessResourceSamples);
    }
    if table_exists_in_conn(conn, "syscall_facts")
        && count_where_in_conn(conn, "SELECT COUNT(*) FROM syscall_facts")? > 0
    {
        present.insert(Field::PrivilegedEffectTrace);
    }
    if table_exists_in_conn(conn, "security_facts") {
        if count_where_in_conn(
            conn,
            "SELECT COUNT(*) FROM security_facts WHERE kind IN ('permission_mode', 'permission_decision', 'approval_policy', 'permission_profile', 'approved_prefix_count', 'sandbox_policy', 'isolation_test')",
        )? > 0
        {
            present.insert(Field::SecurityPolicy);
        }
        if count_where_in_conn(
            conn,
            "SELECT COUNT(*) FROM security_facts WHERE kind IN ('dangerous_command', 'sandbox_override_requested', 'dangerously_disable_sandbox', 'sandbox_permissions')",
        )? > 0
        {
            present.insert(Field::SecurityCommandCheck);
        }
    }
    if count_where_in_conn(conn, "SELECT COUNT(*) FROM source_files")? > 0 {
        present.insert(Field::SourceCoverage);
    }
    if table_exists_in_conn(conn, "harness_trajectory")
        && count_where_in_conn(conn, "SELECT COUNT(*) FROM harness_trajectory")? > 0
    {
        present.insert(Field::TrajectoryVector);
    }
    if table_exists_in_conn(conn, "harness_envelope")
        && count_where_in_conn(conn, "SELECT COUNT(*) FROM harness_envelope")? > 0
    {
        present.insert(Field::HarnessEnvelopeShape);
    }

    Ok(present)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_connector_declares_all_fields_in_canonical_order_with_code_citations() {
        assert_eq!(Field::ALL.len(), 28);
        assert_eq!(CONNECTOR_DECLARATIONS.len(), 11);
        for conn in CONNECTOR_DECLARATIONS {
            assert_eq!(
                conn.fields.len(),
                Field::ALL.len(),
                "connector {} must declare all {} fields",
                conn.connector,
                Field::ALL.len()
            );
            for (decl, expected) in conn.fields.iter().zip(Field::ALL.iter()) {
                assert_eq!(
                    decl.field, *expected,
                    "connector {} field order mismatch",
                    conn.connector
                );
                assert!(
                    decl.code_location
                        .starts_with("crates/dendro-connectors/src/"),
                    "connector {} field {} must cite crates/dendro-connectors/src/: got {}",
                    conn.connector,
                    decl.field,
                    decl.code_location
                );
                assert!(
                    !decl.rationale.trim().is_empty(),
                    "connector {} field {} rationale must not be empty",
                    conn.connector,
                    decl.field
                );
            }
        }
    }

    #[test]
    fn matrix_test_every_insight_by_every_connector_is_computable_or_not_computable() {
        assert_eq!(CATALOG.len(), 25);
        for insight in CATALOG {
            assert!(
                !insight.required_fields().is_empty(),
                "insight {} ({}) must declare at least one required field",
                insight.id(),
                insight.name()
            );
            for conn in CONNECTOR_DECLARATIONS {
                let eval = evaluate_denominator(*insight, &[conn.connector]);
                match eval {
                    DenominatorEvaluation::Computable { scope } => {
                        assert_eq!(scope.insight_id, insight.id());
                        assert_eq!(scope.included.len(), 1);
                        assert_eq!(scope.included[0].connector, conn.connector);
                        assert!(scope.excluded.is_empty());
                        for req in insight.required_fields() {
                            assert!(
                                conn.supplies(*req),
                                "Computable ({}, {}) but {} does not supply {}",
                                insight.id(),
                                conn.connector,
                                conn.connector,
                                req
                            );
                        }
                    }
                    DenominatorEvaluation::NotComputable {
                        insight_id,
                        missing,
                        by_connector,
                        ..
                    } => {
                        assert_eq!(insight_id, insight.id());
                        assert!(
                            !missing.is_empty(),
                            "NotComputable ({}, {}) must report non-empty missing fields",
                            insight.id(),
                            conn.connector
                        );
                        assert_eq!(by_connector.len(), 1);
                        assert_eq!(by_connector[0].connector, conn.connector);
                        assert_eq!(by_connector[0].missing, missing);
                        for field in &missing {
                            assert_eq!(
                                conn.fidelity(*field),
                                FieldFidelity::Absent,
                                "missing field {} for {} must be Absent",
                                field,
                                conn.connector
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn mixed_source_denominator_excludes_connector_missing_required_field() {
        // I01 requires NativeSession (MessageId is optional); claude, codex, grok, devin stay in,
        // while agy is excluded for missing NativeSession.
        let i01 = CATALOG.iter().copied().find(|i| i.id() == "I01").unwrap();
        let eval_i01 = evaluate_denominator(i01, &["claude", "codex", "grok", "devin", "agy"]);
        match eval_i01 {
            DenominatorEvaluation::Computable { scope } => {
                assert_eq!(
                    scope
                        .included
                        .iter()
                        .map(|c| c.connector)
                        .collect::<Vec<_>>(),
                    vec!["claude", "codex", "grok", "devin"]
                );
                assert_eq!(scope.excluded.len(), 1);
                assert_eq!(scope.excluded[0].connector, "agy");
                assert_eq!(scope.excluded[0].missing, vec![Field::NativeSession]);
            }
            other => panic!("expected Computable for I01, got {other:?}"),
        }

        // I02 over {claude, goose}: claude supplies all 4 token fields; goose lacks cache fields.
        let i02 = CATALOG.iter().copied().find(|i| i.id() == "I02").unwrap();
        let eval_i02 = evaluate_denominator(i02, &["claude", "goose"]);
        match eval_i02 {
            DenominatorEvaluation::Computable { scope } => {
                assert_eq!(scope.included.len(), 1);
                assert_eq!(scope.included[0].connector, "claude");
                assert_eq!(scope.excluded.len(), 1);
                assert_eq!(scope.excluded[0].connector, "goose");
                assert_eq!(
                    scope.excluded[0].missing,
                    vec![Field::CacheReadTokens, Field::CacheWriteTokens]
                );
                assert_eq!(
                    format_denominator_disclosure(&scope).as_deref(),
                    Some("excluded: goose — missing cache_read_tokens, cache_write_tokens")
                );
            }
            other => panic!("expected Computable with goose excluded, got {other:?}"),
        }

        // Codex tool_outcome is Partial(ExplicitSignalOnly) -> I08 over {claude, codex} includes both,
        // and records codex's Partial(ExplicitSignalOnly) on ToolOutcome.
        let i08 = CATALOG.iter().copied().find(|i| i.id() == "I08").unwrap();
        let eval_i08 = evaluate_denominator(i08, &["claude", "codex", "devin"]);
        match eval_i08 {
            DenominatorEvaluation::Computable { scope } => {
                assert_eq!(
                    scope
                        .included
                        .iter()
                        .map(|c| c.connector)
                        .collect::<Vec<_>>(),
                    vec!["claude", "codex"]
                );
                assert_eq!(
                    scope.included[1].partial_fields,
                    vec![(
                        Field::ToolOutcome,
                        PartialFidelityReason::ExplicitSignalOnly
                    )]
                );
                assert_eq!(scope.excluded.len(), 1);
                assert_eq!(scope.excluded[0].connector, "devin");
                assert_eq!(scope.excluded[0].missing, vec![Field::ToolOutcome]);
            }
            other => panic!("expected Computable for I08, got {other:?}"),
        }
    }

    #[test]
    fn excluded_connector_rows_in_every_table_family_do_not_affect_results() {
        use crate::insight_v1::{evaluate_catalog, Scope};
        use crate::AnalyticsStore;

        let seed_included = |store: &AnalyticsStore| {
            store
                .connection()
                .execute_batch(
                    "CREATE TABLE IF NOT EXISTS context_facts (
                        source_key VARCHAR PRIMARY KEY,
                        harness VARCHAR NOT NULL,
                        native_session VARCHAR,
                        ts TIMESTAMP,
                        context_used BIGINT NOT NULL,
                        context_limit BIGINT
                    );
                    CREATE TABLE IF NOT EXISTS authority_rules (
                        rule_id VARCHAR NOT NULL,
                        kind VARCHAR NOT NULL,
                        expected_value VARCHAR NOT NULL
                    );
                    INSERT INTO source_files (source_id, harness, display_path, generation, byte_offset, prefix_sha256, native_session)
                        VALUES ('src-claude', 'claude', 'claude.jsonl', 1, 100, '00', 'claude-s1');
                    INSERT INTO usage_facts (source_key, harness, native_session, message_id, ts, seq, input_uncached, output_tokens, cache_read, cache_write, reasoning, currency_total, generation, source_id, model, effort)
                        VALUES ('claude:u1', 'claude', 'claude-s1', 'msg-1', TIMESTAMP '2026-10-10 10:00:01', 1, 120, 45, 30, 15, -1, NULL, 1, 'src-claude', 'claude-sonnet-4-5', 'high');
                    INSERT INTO tool_facts (source_key, harness, native_session, ts, seq, tool_name, fingerprint, failed, structured_write, generation, source_id, category)
                        VALUES ('claude:t1', 'claude', 'claude-s1', TIMESTAMP '2026-10-10 10:00:01', 1, 'Bash', 'fp-1', FALSE, FALSE, 1, 'src-claude', 'eng_build_test');
                    INSERT INTO skill_facts (source_key, seat, skill, invoked)
                        VALUES
                            ('openrig:decl-1', 'openrig:dev', 'commit', FALSE),
                            ('claude:sk-1', 'claude', 'commit', TRUE);
                    INSERT INTO session_edges (source_key, source_id, harness, parent_session, child_session, relation, status, observed_at, evidence_kind, generation)
                        VALUES ('claude:e1', 'src-claude', 'claude', 'claude-s1', 'claude-child-1', 'subagent', 'evidenced', TIMESTAMP '2026-10-10 10:00:01', 'explicit', 1);
                    INSERT INTO context_facts (source_key, harness, native_session, ts, context_used, context_limit)
                        VALUES ('claude:ctx-1', 'claude', 'claude-s1', TIMESTAMP '2026-10-10 10:00:01', 50000, 200000);
                    INSERT INTO security_facts (source_key, source_id, harness, native_session, ts, kind, value, source_field, generation)
                        VALUES
                            ('claude:sec-1', 'src-claude', 'claude', 'claude-s1', TIMESTAMP '2026-10-10 10:00:01', 'permission_mode', 'default', 'permissionMode', 1),
                            ('claude:sec-2', 'src-claude', 'claude', 'claude-s1', TIMESTAMP '2026-10-10 10:00:01', 'sandbox_policy', 'workspace-write', 'sandboxPolicy', 1);
                    INSERT INTO authority_rules (rule_id, kind, expected_value)
                        VALUES ('r1', 'permission_mode', 'default');",
                )
                .unwrap();
        };

        let store_clean = AnalyticsStore::open_in_memory("table-family-clean").unwrap();
        seed_included(&store_clean);

        let store_polluted = AnalyticsStore::open_in_memory("table-family-polluted").unwrap();
        seed_included(&store_polluted);
        // Insert excluded-connector (`goose`) rows into every table family:
        // `usage_facts`, `tool_facts`, `skill_facts`, `session_edges`, `context_facts`, `security_facts`.
        store_polluted
            .connection()
            .execute_batch(
                "INSERT INTO source_files (source_id, harness, display_path, generation, byte_offset, prefix_sha256, native_session)
                    VALUES ('src-goose', 'goose', 'goose.jsonl', 1, 100, '11', 'goose-s1');
                INSERT INTO usage_facts (source_key, harness, native_session, message_id, ts, seq, input_uncached, output_tokens, cache_read, cache_write, reasoning, currency_total, generation, source_id, model, effort)
                    VALUES ('goose:u1', 'goose', 'goose-s1', NULL, TIMESTAMP '2026-10-10 10:00:02', 1, 9000, 4000, 0, 0, 0, NULL, 1, 'src-goose', NULL, NULL);
                INSERT INTO tool_facts (source_key, harness, native_session, ts, seq, tool_name, fingerprint, failed, structured_write, generation, source_id, category)
                    VALUES ('goose:t1', 'goose', 'goose-s1', TIMESTAMP '2026-10-10 10:00:02', 1, 'shell', NULL, NULL, FALSE, 1, 'src-goose', 'eng_build_test');
                INSERT INTO skill_facts (source_key, seat, skill, invoked)
                    VALUES ('goose:sk-1', 'goose', 'goose-skill', TRUE);
                INSERT INTO session_edges (source_key, source_id, harness, parent_session, child_session, relation, status, observed_at, evidence_kind, generation)
                    VALUES ('goose:e1', 'src-goose', 'goose', 'claude-s1', 'goose-child-99', 'subagent', 'evidenced', TIMESTAMP '2026-10-10 10:00:02', 'explicit', 1);
                INSERT INTO context_facts (source_key, harness, native_session, ts, context_used, context_limit)
                    VALUES ('goose:ctx-1', 'goose', 'goose-s1', TIMESTAMP '2026-10-10 10:00:02', 195000, NULL);
                INSERT INTO security_facts (source_key, source_id, harness, native_session, ts, kind, value, source_field, generation)
                    VALUES
                        ('goose:sec-1', 'src-goose', 'goose', 'goose-s1', TIMESTAMP '2026-10-10 10:00:02', 'permission_mode', 'bypassPermissions', 'permissionMode', 1),
                        ('goose:sec-2', 'src-goose', 'goose', 'goose-s1', TIMESTAMP '2026-10-10 10:00:02', 'sandbox_policy', 'danger-full-access', 'sandboxPolicy', 1),
                        ('goose:sec-3', 'src-goose', 'goose', 'goose-s1', TIMESTAMP '2026-10-10 10:00:02', 'dangerous_command', 'rm -rf /', 'command', 1);",
            )
            .unwrap();

        let clean_results = evaluate_catalog(&store_clean, &Scope::default());
        let polluted_results = evaluate_catalog(&store_polluted, &Scope::default());

        for id in [
            "I02", "I03", "I05", "I06", "I07", "I08", "I09", "I11", "I16", "I17", "I21", "I24",
            "I25",
        ] {
            let clean = clean_results.iter().find(|r| r.id == id).unwrap();
            let polluted = polluted_results.iter().find(|r| r.id == id).unwrap();
            assert_eq!(
                clean.status, polluted.status,
                "excluded goose rows in table family must not affect {id} status"
            );
            assert_eq!(
                clean.population.n, polluted.population.n,
                "excluded goose rows in table family must not affect {id} population.n"
            );
            assert_eq!(
                clean.measures, polluted.measures,
                "excluded goose rows in table family must not affect {id} measures"
            );
            assert!(
                polluted.coverage.contains("excluded: goose — missing"),
                "{id} coverage must disclose goose exclusion: {}",
                polluted.coverage
            );
        }
    }
}
