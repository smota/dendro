use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: i32 = 3;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageFact {
    pub source_key: String,
    pub harness: String,
    pub native_session: String,
    pub message_id: Option<String>,
    pub ts: Option<DateTime<Utc>>,
    pub seq: i64,
    pub input_uncached: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    pub reasoning: i64,
    pub currency_total: Option<f64>,
    pub generation: i64,
    pub predecessor_source_key: Option<String>,
    pub predecessor_reason: Option<String>,
    pub prior_input_tokens: Option<i64>,
    pub prior_cached_input_tokens: Option<i64>,
    pub prior_cache_write_input_tokens: Option<i64>,
    pub prior_output_tokens: Option<i64>,
    pub prior_reasoning_output_tokens: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolFact {
    pub source_key: String,
    pub harness: String,
    pub native_session: String,
    pub ts: Option<DateTime<Utc>>,
    pub seq: i64,
    pub name: String,
    pub fingerprint: Option<String>,
    pub failed: Option<bool>,
    pub structured_write: bool,
    pub generation: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentityFact {
    pub source_key: String,
    pub harness: String,
    pub native_session: String,
    pub rig: String,
    pub seat: String,
    pub ts: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityFact {
    pub source_key: String,
    pub seat: String,
    pub ts: DateTime<Utc>,
    pub state: String,
    pub seq: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueFact {
    pub source_key: String,
    pub transition_id: i64,
    pub qitem_id: String,
    pub ts: DateTime<Utc>,
    pub state: String,
    pub source_seat: String,
    pub dest_seat: String,
    pub closure_reason: Option<String>,
    pub closure_target: Option<String>,
    pub chain_id: Option<String>,
    pub missions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillFact {
    pub source_key: String,
    pub seat: String,
    pub skill: String,
    pub invoked: bool,
}

/// An item's `queue_items.destination_session` and mission tags at one ingest, with its cutoff
/// (`queue_items.ts_updated`) and whether the source shows a K5 reroute mark.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DestinationSnapshot {
    pub destination: String,
    pub cutoff: String,
    pub reroute_marked: bool,
    /// The item's `mission:` tag values at this reading (K3: tags as set at creation).
    pub missions: Vec<String>,
}

/// One recorded life of a seat (K4). A seat may have several after relaunches.
/// `None` means no event recorded that edge, never a time read from runtime status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeatBound {
    pub seat: String,
    pub created_at: Option<DateTime<Utc>>,
    pub stopped_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CodexCounters {
    pub input_tokens: i64,
    pub cached_input_tokens: i64,
    pub cache_write_input_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_output_tokens: i64,
}

#[derive(Debug, Clone)]
pub struct WarningNote {
    pub path: String,
    pub line: Option<i64>,
    pub reason: String,
}

/// Private reader metadata uses a disjoint source-key namespace, never a native tool name.
pub const READER_NAMESPACE: &str = "dendro-internal:reader:";
pub const READER_VERSION: &str = "v1";
pub const READER_COMPLETE: &str = "DendroReaderComplete";
pub const READER_UNSUPPORTED: &str = "DendroReaderUnsupported";
pub const CODEX_COMMAND: &str = "CommandExecution";
pub const CODEX_CALL: &str = "CodexSeatCall";

#[derive(Debug, Clone)]
pub struct ReaderState {
    pub complete: bool,
    pub native_session: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SourceCoverage {
    pub source_id: String,
    pub harness: String,
    pub native_session: Option<String>,
    pub complete: bool,
}

#[derive(Debug, Clone)]
pub struct CodexObservation {
    pub source_id: String,
    pub fact: ToolFact,
}

/// Closed operational category for tool and command invocations (#52).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum ToolCategory {
    CoordQueue,
    CoordStatus,
    GovernancePhase,
    EngBuildTest,
    EngCodeEdit,
    EngVcs,
    InspectRead,
    #[default]
    Other,
}

impl ToolCategory {
    pub const ALL: [ToolCategory; 8] = [
        ToolCategory::CoordQueue,
        ToolCategory::CoordStatus,
        ToolCategory::GovernancePhase,
        ToolCategory::EngBuildTest,
        ToolCategory::EngCodeEdit,
        ToolCategory::EngVcs,
        ToolCategory::InspectRead,
        ToolCategory::Other,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            ToolCategory::CoordQueue => "coord_queue",
            ToolCategory::CoordStatus => "coord_status",
            ToolCategory::GovernancePhase => "governance_phase",
            ToolCategory::EngBuildTest => "eng_build_test",
            ToolCategory::EngCodeEdit => "eng_code_edit",
            ToolCategory::EngVcs => "eng_vcs",
            ToolCategory::InspectRead => "inspect_read",
            ToolCategory::Other => "other",
        }
    }

    pub fn from_label(label: &str) -> Self {
        match label.trim() {
            "coord_queue" => ToolCategory::CoordQueue,
            "coord_status" => ToolCategory::CoordStatus,
            "governance_phase" => ToolCategory::GovernancePhase,
            "eng_build_test" => ToolCategory::EngBuildTest,
            "eng_code_edit" => ToolCategory::EngCodeEdit,
            "eng_vcs" => ToolCategory::EngVcs,
            "inspect_read" => ToolCategory::InspectRead,
            _ => ToolCategory::Other,
        }
    }

    pub fn is_ceremony(&self) -> bool {
        matches!(
            self,
            ToolCategory::CoordQueue | ToolCategory::CoordStatus | ToolCategory::GovernancePhase
        )
    }

    pub fn is_engineering(&self) -> bool {
        matches!(
            self,
            ToolCategory::EngBuildTest
                | ToolCategory::EngCodeEdit
                | ToolCategory::EngVcs
                | ToolCategory::InspectRead
        )
    }

    /// Classify a tool call from its tool name alone when no raw command/input is available.
    pub fn classify_tool_name(tool_name: &str) -> Self {
        match tool_name {
            "Write"
            | "Edit"
            | "MultiEdit"
            | "NotebookEdit"
            | "write"
            | "edit"
            | "apply_patch"
            | "patch_apply"
            | "replace_file_content"
            | "multi_replace_file_content"
            | "write_to_file" => ToolCategory::EngCodeEdit,
            "Read" | "read" | "view_file" | "view_file_outline" | "view_code_item" | "Glob"
            | "Grep" | "LS" | "grep_search" | "find_by_name" | "list_dir" => {
                ToolCategory::InspectRead
            }
            _ => ToolCategory::Other,
        }
    }
}

/// Enriched queue-item metadata for recovery classification, boundary wake status,
/// and issue/phase/parent lineage (#53, #55).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueItemMeta {
    pub qitem_id: String,
    pub is_recovery: bool,
    pub recovery_reason: Option<String>,
    pub last_nudge_result: Option<String>,
    pub issue_tags: Vec<String>,
    pub phase_tags: Vec<String>,
    pub parent_qitem_id: Option<String>,
}
