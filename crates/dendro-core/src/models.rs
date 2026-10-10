use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Watermark {
    pub file_path: String,
    pub byte_offset: u64,
    pub last_modified: u64,
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestrationContext {
    pub rig_name: String,
    pub seat_role: String,
    pub mission_id: Option<String>,
    pub handoff_from: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanonicalSession {
    pub session_id: String,
    pub harness_name: String,
    pub project_path: String,
    pub project_name: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub duration_seconds: Option<i64>,
    pub user_turns_count: u32,
    pub assistant_turns_count: u32,
    pub tool_calls_count: u32,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub estimated_cost_usd: f64,
    pub status: String,
    pub orchestration: Option<OrchestrationContext>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanonicalToolCall {
    pub call_id: String,
    pub session_id: String,
    pub tool_name: String,
    pub executed_at: DateTime<Utc>,
    pub execution_time_ms: Option<u64>,
    pub status: String,
    pub error_message: Option<String>,
    pub command_preview: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CanonicalEvent {
    SessionStarted(CanonicalSession),
    SessionUpdated(CanonicalSession),
    ToolExecuted(CanonicalToolCall),
}
