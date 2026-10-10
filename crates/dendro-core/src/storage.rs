use std::path::Path;
use anyhow::Result;
use duckdb::Connection;
use crate::models::{CanonicalSession, CanonicalToolCall};

pub struct DuckDbStorage {
    conn: Connection,
}

impl DuckDbStorage {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let conn = Connection::open(path)?;
        let storage = Self { conn };
        storage.init_schema()?;
        Ok(storage)
    }

    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let storage = Self { conn };
        storage.init_schema()?;
        Ok(storage)
    }

    fn init_schema(&self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS _dendro_watermarks (
                file_path VARCHAR PRIMARY KEY,
                byte_offset BIGINT NOT NULL,
                last_modified BIGINT NOT NULL,
                session_id VARCHAR,
                updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            );

            CREATE TABLE IF NOT EXISTS fct_sessions (
                session_id VARCHAR PRIMARY KEY,
                harness_name VARCHAR NOT NULL,
                project_path VARCHAR NOT NULL,
                project_name VARCHAR NOT NULL,
                started_at TIMESTAMP NOT NULL,
                ended_at TIMESTAMP,
                duration_seconds BIGINT,
                user_turns_count INTEGER NOT NULL,
                assistant_turns_count INTEGER NOT NULL,
                tool_calls_count INTEGER NOT NULL,
                input_tokens BIGINT NOT NULL,
                output_tokens BIGINT NOT NULL,
                estimated_cost_usd DOUBLE NOT NULL,
                status VARCHAR NOT NULL,
                rig_name VARCHAR,
                seat_role VARCHAR,
                mission_id VARCHAR,
                handoff_from VARCHAR,
                ingested_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            );

            CREATE TABLE IF NOT EXISTS fct_tool_calls (
                call_id VARCHAR PRIMARY KEY,
                session_id VARCHAR NOT NULL,
                tool_name VARCHAR NOT NULL,
                executed_at TIMESTAMP NOT NULL,
                execution_time_ms BIGINT,
                status VARCHAR NOT NULL,
                error_message VARCHAR,
                command_preview VARCHAR,
                ingested_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            );
            "#,
        )?;
        Ok(())
    }

    pub fn insert_session(&self, session: &CanonicalSession) -> Result<()> {
        let rig_name = session.orchestration.as_ref().map(|o| o.rig_name.as_str());
        let seat_role = session.orchestration.as_ref().map(|o| o.seat_role.as_str());
        let mission_id = session.orchestration.as_ref().and_then(|o| o.mission_id.as_deref());
        let handoff_from = session.orchestration.as_ref().and_then(|o| o.handoff_from.as_deref());

        self.conn.execute(
            r#"
            INSERT OR REPLACE INTO fct_sessions (
                session_id, harness_name, project_path, project_name,
                started_at, ended_at, duration_seconds,
                user_turns_count, assistant_turns_count, tool_calls_count,
                input_tokens, output_tokens, estimated_cost_usd, status,
                rig_name, seat_role, mission_id, handoff_from
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
            duckdb::params![
                session.session_id,
                session.harness_name,
                session.project_path,
                session.project_name,
                session.started_at.to_rfc3339(),
                session.ended_at.map(|dt| dt.to_rfc3339()),
                session.duration_seconds,
                session.user_turns_count,
                session.assistant_turns_count,
                session.tool_calls_count,
                session.input_tokens,
                session.output_tokens,
                session.estimated_cost_usd,
                session.status,
                rig_name,
                seat_role,
                mission_id,
                handoff_from,
            ],
        )?;
        Ok(())
    }

    pub fn insert_tool_call(&self, tool: &CanonicalToolCall) -> Result<()> {
        self.conn.execute(
            r#"
            INSERT OR REPLACE INTO fct_tool_calls (
                call_id, session_id, tool_name, executed_at,
                execution_time_ms, status, error_message, command_preview
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            "#,
            duckdb::params![
                tool.call_id,
                tool.session_id,
                tool.tool_name,
                tool.executed_at.to_rfc3339(),
                tool.execution_time_ms,
                tool.status,
                tool.error_message,
                tool.command_preview,
            ],
        )?;
        Ok(())
    }
}
