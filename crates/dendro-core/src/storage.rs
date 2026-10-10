use std::path::Path;

use anyhow::{anyhow, Result};
use duckdb::{params, Connection};

use crate::facts::*;
use crate::privacy::redact;

/// Legacy entry point retained so the untouched watcher/CLI still typecheck.
/// It does not create or migrate the pre-R1 tables.
pub struct DuckDbStorage;

impl DuckDbStorage {
    pub fn open(path: &std::path::Path) -> Result<AnalyticsStore> {
        if path.exists() {
            let conn = Connection::open(path)?;
            refuse_legacy(&conn)?;
        }
        Err(anyhow!(
            "R1 ingest publishes through the staged analytics path; the legacy opener does not create a schema"
        ))
    }
}

pub struct AnalyticsStore {
    conn: Connection,
    pub key_id: String,
}

impl AnalyticsStore {
    pub fn open_in_memory(key_id: &str) -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let store = Self {
            conn,
            key_id: key_id.to_string(),
        };
        store.create_schema()?;
        Ok(store)
    }

    pub fn open_path(path: &Path, key_id: &str) -> Result<Self> {
        if path.exists() {
            return Self::open_existing(path, key_id);
        }
        let conn = Connection::open(path)?;
        let store = Self {
            conn,
            key_id: key_id.to_string(),
        };
        store.create_schema()?;
        Ok(store)
    }

    pub fn open_existing(path: &Path, key_id: &str) -> Result<Self> {
        let conn = Connection::open(path)?;
        refuse_legacy(&conn)?;
        let stored: String = conn.query_row(
            "SELECT key_id FROM dendro_meta WHERE schema_version = ?",
            params![SCHEMA_VERSION],
            |row| row.get(0),
        )?;
        if stored != key_id {
            return Err(anyhow!("analytics key id does not match the sidecar"));
        }
        Ok(Self {
            conn,
            key_id: key_id.to_string(),
        })
    }

    /// Opens a database this process just created in a private staging directory.
    pub fn open_staged(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        refuse_legacy(&conn)?;
        let (version, key_id): (i32, String) = conn.query_row(
            "SELECT schema_version, key_id FROM dendro_meta",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if version != SCHEMA_VERSION {
            return Err(anyhow!("unsupported analytics schema version {version}"));
        }
        Ok(Self { conn, key_id })
    }

    pub fn open_read_only(path: &Path) -> Result<Self> {
        let config = duckdb::Config::default().access_mode(duckdb::AccessMode::ReadOnly)?;
        let conn = Connection::open_with_flags(path, config)?;
        refuse_legacy(&conn)?;
        let (version, key_id): (i32, String) = conn.query_row(
            "SELECT schema_version, key_id FROM dendro_meta",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if version != SCHEMA_VERSION {
            return Err(anyhow!("unsupported analytics schema version {version}"));
        }
        Ok(Self { conn, key_id })
    }

    fn create_schema(&self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE dendro_meta (
                schema_version INTEGER NOT NULL,
                key_id VARCHAR NOT NULL
            );
            CREATE TABLE source_files (
                source_id VARCHAR PRIMARY KEY,
                harness VARCHAR NOT NULL,
                display_path VARCHAR NOT NULL,
                generation BIGINT NOT NULL,
                byte_offset BIGINT NOT NULL,
                prefix_sha256 VARCHAR NOT NULL
            );
            CREATE TABLE codex_counters (
                source_id VARCHAR NOT NULL,
                generation BIGINT NOT NULL,
                input_tokens BIGINT NOT NULL,
                cached_input_tokens BIGINT NOT NULL,
                cache_write_input_tokens BIGINT NOT NULL,
                output_tokens BIGINT NOT NULL,
                reasoning_output_tokens BIGINT NOT NULL,
                PRIMARY KEY (source_id, generation)
            );
            CREATE TABLE usage_facts (
                source_key VARCHAR PRIMARY KEY,
                harness VARCHAR NOT NULL,
                native_session VARCHAR NOT NULL,
                message_id VARCHAR,
                ts TIMESTAMP,
                seq BIGINT NOT NULL,
                input_uncached BIGINT NOT NULL,
                output_tokens BIGINT NOT NULL,
                cache_read BIGINT NOT NULL,
                cache_write BIGINT NOT NULL,
                reasoning BIGINT NOT NULL,
                currency_total DOUBLE,
                generation BIGINT NOT NULL,
                source_id VARCHAR NOT NULL
            );
            CREATE TABLE tool_facts (
                source_key VARCHAR PRIMARY KEY,
                harness VARCHAR NOT NULL,
                native_session VARCHAR NOT NULL,
                ts TIMESTAMP,
                seq BIGINT NOT NULL,
                tool_name VARCHAR NOT NULL,
                fingerprint VARCHAR,
                failed BOOLEAN,
                structured_write BOOLEAN NOT NULL,
                generation BIGINT NOT NULL,
                source_id VARCHAR NOT NULL
            );
            CREATE TABLE identity_facts (
                source_key VARCHAR PRIMARY KEY,
                harness VARCHAR NOT NULL,
                native_session VARCHAR NOT NULL,
                rig VARCHAR NOT NULL,
                seat VARCHAR NOT NULL,
                ts TIMESTAMP
            );
            CREATE TABLE activity_facts (
                source_key VARCHAR PRIMARY KEY,
                seat VARCHAR NOT NULL,
                ts TIMESTAMP NOT NULL,
                state VARCHAR NOT NULL,
                seq BIGINT NOT NULL
            );
            CREATE TABLE queue_facts (
                source_key VARCHAR PRIMARY KEY,
                transition_id BIGINT NOT NULL,
                qitem_id VARCHAR NOT NULL,
                ts TIMESTAMP NOT NULL,
                state VARCHAR NOT NULL,
                source_seat VARCHAR NOT NULL,
                dest_seat VARCHAR NOT NULL,
                closure_reason VARCHAR,
                closure_target VARCHAR,
                chain_id VARCHAR,
                missions VARCHAR NOT NULL
            );
            CREATE TABLE skill_facts (
                source_key VARCHAR PRIMARY KEY,
                seat VARCHAR NOT NULL,
                skill VARCHAR NOT NULL,
                invoked BOOLEAN NOT NULL
            );
            CREATE TABLE seat_bounds (
                seat VARCHAR PRIMARY KEY,
                created_at TIMESTAMP,
                stopped_at TIMESTAMP
            );
            CREATE TABLE warnings (
                path VARCHAR NOT NULL,
                line_no BIGINT,
                reason VARCHAR NOT NULL
            );
            "#,
        )?;
        self.conn.execute(
            "INSERT INTO dendro_meta (schema_version, key_id) VALUES (?, ?)",
            params![SCHEMA_VERSION, self.key_id],
        )?;
        Ok(())
    }

    pub fn checkpoint(&self) -> Result<()> {
        self.conn.execute_batch("CHECKPOINT")?;
        Ok(())
    }

    pub fn record_warning(&self, note: &WarningNote) -> Result<()> {
        self.conn.execute(
            "INSERT INTO warnings (path, line_no, reason) VALUES (?, ?, ?)",
            params![redact(&note.path), note.line, note.reason],
        )?;
        Ok(())
    }

    pub fn source_state(&self, source_id: &str) -> Result<Option<(i64, i64, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT generation, byte_offset, prefix_sha256 FROM source_files WHERE source_id = ?",
        )?;
        let mut rows = stmt.query(params![source_id])?;
        if let Some(row) = rows.next()? {
            Ok(Some((row.get(0)?, row.get(1)?, row.get(2)?)))
        } else {
            Ok(None)
        }
    }

    pub fn codex_counters(&self, source_id: &str, generation: i64) -> Result<CodexCounters> {
        let mut stmt = self.conn.prepare(
            r#"SELECT input_tokens, cached_input_tokens, cache_write_input_tokens,
                      output_tokens, reasoning_output_tokens
               FROM codex_counters WHERE source_id = ? AND generation = ?"#,
        )?;
        let mut rows = stmt.query(params![source_id, generation])?;
        if let Some(row) = rows.next()? {
            Ok(CodexCounters {
                input_tokens: row.get(0)?,
                cached_input_tokens: row.get(1)?,
                cache_write_input_tokens: row.get(2)?,
                output_tokens: row.get(3)?,
                reasoning_output_tokens: row.get(4)?,
            })
        } else {
            Ok(CodexCounters::default())
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn commit_file_batch(
        &self,
        source_id: &str,
        harness: &str,
        display_path: &str,
        generation: i64,
        byte_offset: i64,
        prefix_sha256: &str,
        counters: Option<&CodexCounters>,
        usages: &[UsageFact],
        tools: &[ToolFact],
    ) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            r#"INSERT INTO source_files (source_id, harness, display_path, generation, byte_offset, prefix_sha256)
               VALUES (?, ?, ?, ?, ?, ?)
               ON CONFLICT (source_id) DO UPDATE SET
                 generation = excluded.generation,
                 byte_offset = excluded.byte_offset,
                 prefix_sha256 = excluded.prefix_sha256"#,
            params![
                source_id,
                harness,
                redact(display_path),
                generation,
                byte_offset,
                prefix_sha256
            ],
        )?;
        if let Some(c) = counters {
            tx.execute(
                r#"INSERT INTO codex_counters (
                       source_id, generation, input_tokens, cached_input_tokens,
                       cache_write_input_tokens, output_tokens, reasoning_output_tokens
                   ) VALUES (?, ?, ?, ?, ?, ?, ?)
                   ON CONFLICT (source_id, generation) DO UPDATE SET
                     input_tokens = excluded.input_tokens,
                     cached_input_tokens = excluded.cached_input_tokens,
                     cache_write_input_tokens = excluded.cache_write_input_tokens,
                     output_tokens = excluded.output_tokens,
                     reasoning_output_tokens = excluded.reasoning_output_tokens"#,
                params![
                    source_id,
                    generation,
                    c.input_tokens,
                    c.cached_input_tokens,
                    c.cache_write_input_tokens,
                    c.output_tokens,
                    c.reasoning_output_tokens
                ],
            )?;
        }
        for u in usages {
            tx.execute(
                r#"INSERT INTO usage_facts (
                       source_key, harness, native_session, message_id, ts, seq,
                       input_uncached, output_tokens, cache_read, cache_write, reasoning,
                       currency_total, generation, source_id
                   ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                   ON CONFLICT (source_key) DO NOTHING"#,
                params![
                    u.source_key,
                    u.harness,
                    u.native_session,
                    u.message_id,
                    u.ts.map(|t| t.to_rfc3339()),
                    u.seq,
                    u.input_uncached,
                    u.output,
                    u.cache_read,
                    u.cache_write,
                    u.reasoning,
                    u.currency_total,
                    u.generation,
                    source_id
                ],
            )?;
        }
        for t in tools {
            tx.execute(
                r#"INSERT INTO tool_facts (
                       source_key, harness, native_session, ts, seq, tool_name, fingerprint,
                       failed, structured_write, generation, source_id
                   ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                   ON CONFLICT (source_key) DO NOTHING"#,
                params![
                    t.source_key,
                    t.harness,
                    t.native_session,
                    t.ts.map(|v| v.to_rfc3339()),
                    t.seq,
                    redact(&t.name),
                    t.fingerprint,
                    t.failed,
                    t.structured_write,
                    t.generation,
                    source_id
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn insert_identity(&self, fact: &IdentityFact) -> Result<()> {
        self.conn.execute(
            r#"INSERT INTO identity_facts (source_key, harness, native_session, rig, seat, ts)
               VALUES (?, ?, ?, ?, ?, ?)
               ON CONFLICT (source_key) DO NOTHING"#,
            params![
                fact.source_key,
                fact.harness,
                fact.native_session,
                redact(&fact.rig),
                redact(&fact.seat),
                fact.ts.map(|t| t.to_rfc3339())
            ],
        )?;
        Ok(())
    }

    pub fn insert_activity(&self, fact: &ActivityFact) -> Result<()> {
        self.conn.execute(
            r#"INSERT INTO activity_facts (source_key, seat, ts, state, seq)
               VALUES (?, ?, ?, ?, ?)
               ON CONFLICT (source_key) DO NOTHING"#,
            params![
                fact.source_key,
                redact(&fact.seat),
                fact.ts.to_rfc3339(),
                fact.state,
                fact.seq
            ],
        )?;
        Ok(())
    }

    pub fn insert_queue(&self, fact: &QueueFact) -> Result<()> {
        self.conn.execute(
            r#"INSERT INTO queue_facts (
                   source_key, transition_id, qitem_id, ts, state, source_seat, dest_seat,
                   closure_reason, closure_target, chain_id, missions
               ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
               ON CONFLICT (source_key) DO NOTHING"#,
            params![
                fact.source_key,
                fact.transition_id,
                fact.qitem_id,
                fact.ts.to_rfc3339(),
                fact.state,
                redact(&fact.source_seat),
                redact(&fact.dest_seat),
                fact.closure_reason.as_deref().map(redact),
                fact.closure_target.as_deref().map(redact),
                fact.chain_id.as_deref(),
                serde_json::to_string(
                    &fact.missions.iter().map(|m| redact(m)).collect::<Vec<_>>()
                )?
            ],
        )?;
        Ok(())
    }

    pub fn insert_skill(&self, fact: &SkillFact) -> Result<()> {
        self.conn.execute(
            r#"INSERT INTO skill_facts (source_key, seat, skill, invoked) VALUES (?, ?, ?, ?)
               ON CONFLICT (source_key) DO NOTHING"#,
            params![
                fact.source_key,
                redact(&fact.seat),
                redact(&fact.skill),
                fact.invoked
            ],
        )?;
        Ok(())
    }

    pub fn load_usage(&self) -> Result<Vec<UsageFact>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT source_key, harness, native_session, message_id, CAST(ts AS VARCHAR), seq,
                      input_uncached, output_tokens, cache_read, cache_write, reasoning,
                      currency_total, generation
               FROM usage_facts ORDER BY ts NULLS LAST, seq"#,
        )?;
        let rows = stmt.query_map([], |row| {
            let ts: Option<String> = row.get(4)?;
            Ok(UsageFact {
                source_key: row.get(0)?,
                harness: row.get(1)?,
                native_session: row.get(2)?,
                message_id: row.get(3)?,
                ts: ts.as_deref().and_then(parse_stored_ts),
                seq: row.get(5)?,
                input_uncached: row.get(6)?,
                output: row.get(7)?,
                cache_read: row.get(8)?,
                cache_write: row.get(9)?,
                reasoning: row.get(10)?,
                currency_total: row.get(11)?,
                generation: row.get(12)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn load_identity(&self) -> Result<Vec<IdentityFact>> {
        let mut stmt = self.conn.prepare(
            "SELECT source_key, harness, native_session, rig, seat, CAST(ts AS VARCHAR) FROM identity_facts",
        )?;
        let rows = stmt.query_map([], |row| {
            let ts: Option<String> = row.get(5)?;
            Ok(IdentityFact {
                source_key: row.get(0)?,
                harness: row.get(1)?,
                native_session: row.get(2)?,
                rig: row.get(3)?,
                seat: row.get(4)?,
                ts: ts.as_deref().and_then(parse_stored_ts),
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn load_activity(&self) -> Result<Vec<ActivityFact>> {
        let mut stmt = self.conn.prepare(
            "SELECT source_key, seat, CAST(ts AS VARCHAR), state, seq FROM activity_facts ORDER BY seat, ts, seq",
        )?;
        let rows = stmt.query_map([], |row| {
            let ts: String = row.get(2)?;
            Ok(ActivityFact {
                source_key: row.get(0)?,
                seat: row.get(1)?,
                ts: parse_stored_ts(&ts).unwrap_or(chrono::DateTime::<chrono::Utc>::UNIX_EPOCH),
                state: row.get(3)?,
                seq: row.get(4)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn load_queue(&self) -> Result<Vec<QueueFact>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT source_key, transition_id, qitem_id, CAST(ts AS VARCHAR), state, source_seat, dest_seat,
                      closure_reason, closure_target, chain_id, missions
               FROM queue_facts ORDER BY ts, transition_id"#,
        )?;
        let rows = stmt.query_map([], |row| {
            let ts: String = row.get(3)?;
            let missions: String = row.get(10)?;
            Ok(QueueFact {
                source_key: row.get(0)?,
                transition_id: row.get(1)?,
                qitem_id: row.get(2)?,
                ts: parse_stored_ts(&ts).unwrap_or(chrono::DateTime::<chrono::Utc>::UNIX_EPOCH),
                state: row.get(4)?,
                source_seat: row.get(5)?,
                dest_seat: row.get(6)?,
                closure_reason: row.get(7)?,
                closure_target: row.get(8)?,
                chain_id: row.get(9)?,
                missions: serde_json::from_str(&missions).unwrap_or_default(),
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn load_tools(&self) -> Result<Vec<ToolFact>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT source_key, harness, native_session, CAST(ts AS VARCHAR), seq, tool_name, fingerprint,
                      failed, structured_write, generation
               FROM tool_facts ORDER BY native_session, ts NULLS LAST, seq"#,
        )?;
        let rows = stmt.query_map([], |row| {
            let ts: Option<String> = row.get(3)?;
            Ok(ToolFact {
                source_key: row.get(0)?,
                harness: row.get(1)?,
                native_session: row.get(2)?,
                ts: ts.as_deref().and_then(parse_stored_ts),
                seq: row.get(4)?,
                name: row.get(5)?,
                fingerprint: row.get(6)?,
                failed: row.get(7)?,
                structured_write: row.get(8)?,
                generation: row.get(9)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn load_skills(&self) -> Result<Vec<SkillFact>> {
        let mut stmt = self
            .conn
            .prepare("SELECT source_key, seat, skill, invoked FROM skill_facts")?;
        let rows = stmt.query_map([], |row| {
            Ok(SkillFact {
                source_key: row.get(0)?,
                seat: row.get(1)?,
                skill: row.get(2)?,
                invoked: row.get(3)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn message_seen(
        &self,
        harness: &str,
        native_session: &str,
        message_id: &str,
        generation: i64,
    ) -> Result<bool> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM usage_facts WHERE harness = ? AND native_session = ? AND message_id = ? AND generation = ?",
            params![harness, native_session, message_id, generation],
            |row| row.get(0),
        )?;
        Ok(n > 0)
    }

    pub fn latest_usage_totals(&self, source_id: &str) -> Result<(i64, i64, i64, i64)> {
        self.conn
            .query_row(
                r#"SELECT COALESCE(SUM(input_uncached), 0), COALESCE(SUM(output_tokens), 0),
                      COALESCE(SUM(cache_read), 0), COALESCE(SUM(cache_write), 0)
               FROM usage_facts
               WHERE source_id = ?
                 AND generation = (SELECT MAX(generation) FROM source_files WHERE source_id = ?)"#,
                params![source_id, source_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .map_err(Into::into)
    }

    pub fn warning_count(&self) -> Result<i64> {
        self.conn
            .query_row("SELECT COUNT(*) FROM warnings", [], |row| row.get(0))
            .map_err(Into::into)
    }

    pub fn contains_text(&self, needle: &str) -> Result<bool> {
        let tables = [
            "SELECT display_path FROM source_files",
            "SELECT native_session FROM usage_facts",
            "SELECT tool_name FROM tool_facts",
            "SELECT COALESCE(fingerprint, '') FROM tool_facts",
            "SELECT seat FROM activity_facts",
            "SELECT missions FROM queue_facts",
            "SELECT reason FROM warnings",
            "SELECT skill FROM skill_facts",
            "SELECT rig FROM identity_facts",
            "SELECT seat FROM identity_facts",
        ];
        for sql in tables {
            let mut stmt = self.conn.prepare(sql)?;
            let mut rows = stmt.query([])?;
            while let Some(row) = rows.next()? {
                let value: String = row.get(0)?;
                if value.contains(needle) {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
}

fn parse_stored_ts(value: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(value) {
        return Some(parsed.with_timezone(&chrono::Utc));
    }
    for format in ["%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%d %H:%M:%S"] {
        if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(value.trim(), format) {
            return Some(naive.and_utc());
        }
    }
    None
}

fn refuse_legacy(conn: &Connection) -> Result<()> {
    let legacy: i64 = conn.query_row(
        "SELECT COUNT(*) FROM information_schema.tables WHERE table_name IN ('fct_sessions', 'fct_tool_calls')",
        [],
        |row| row.get(0),
    )?;
    let meta: i64 = conn.query_row(
        "SELECT COUNT(*) FROM information_schema.tables WHERE table_name = 'dendro_meta'",
        [],
        |row| row.get(0),
    )?;
    if legacy > 0 || meta == 0 {
        return Err(anyhow!(
            "refusing analytics database without R1 schema version; no migration is performed"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_schema_is_refused_without_rewrite() {
        let dir = std::env::temp_dir().join(format!("dendro-legacy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("old.duckdb");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE fct_tool_calls (call_id VARCHAR, error_message VARCHAR, command_preview VARCHAR)",
        )
        .unwrap();
        drop(conn);
        let err = match AnalyticsStore::open_existing(&path, "abc") {
            Ok(_) => panic!("legacy database was opened"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("refusing"));
        let conn = Connection::open(&path).unwrap();
        let cols: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM information_schema.columns WHERE table_name = 'fct_tool_calls' AND column_name = 'command_preview'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(cols, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn batch_rollback_leaves_watermark() {
        let store = AnalyticsStore::open_in_memory("kid").unwrap();
        let err = store.commit_file_batch(
            "src",
            "claude",
            "p",
            1,
            10,
            "sha",
            None,
            &[UsageFact {
                source_key: "k".into(),
                harness: "claude".into(),
                native_session: "s".into(),
                message_id: None,
                ts: None,
                seq: 1,
                input_uncached: 1,
                output: 1,
                cache_read: 0,
                cache_write: 0,
                reasoning: 0,
                currency_total: None,
                generation: 1,
            }],
            &[],
        );
        assert!(err.is_ok());
        let tx = store.conn.unchecked_transaction().unwrap();
        tx.execute(
            "INSERT INTO warnings (path, line_no, reason) VALUES ('x', 1, 'bad')",
            [],
        )
        .unwrap();
        drop(tx);
        assert_eq!(store.warning_count().unwrap(), 0);
        assert!(store.source_state("src").unwrap().is_some());
    }
}
