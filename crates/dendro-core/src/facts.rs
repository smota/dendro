use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: i32 = 2;

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

/// An item's `queue_items.destination_session` at one ingest, with its cutoff
/// (`queue_items.ts_updated`) and whether the source shows a K5 reroute mark.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DestinationSnapshot {
    pub destination: String,
    pub cutoff: String,
    pub reroute_marked: bool,
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
