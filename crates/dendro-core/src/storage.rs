use std::path::Path;

use anyhow::{anyhow, Result};
use duckdb::{params, Connection};

use crate::facts::*;
use crate::privacy::{activity_state, closure_reason, queue_state, redact};

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
        let conn = Connection::open_with_flags(path, config)
            .map_err(|_| anyhow!("analytics database could not be opened"))?;
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
                source_id VARCHAR NOT NULL,
                predecessor_source_key VARCHAR,
                predecessor_reason VARCHAR,
                prior_input_tokens BIGINT,
                prior_cached_input_tokens BIGINT,
                prior_cache_write_input_tokens BIGINT,
                prior_output_tokens BIGINT,
                prior_reasoning_output_tokens BIGINT
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
                seat VARCHAR NOT NULL,
                created_at TIMESTAMP,
                stopped_at TIMESTAMP
            );
            CREATE TABLE seat_roles (
                seat VARCHAR NOT NULL,
                role VARCHAR NOT NULL,
                evidence VARCHAR NOT NULL,
                PRIMARY KEY (seat, role, evidence)
            );
            CREATE TABLE warnings (
                path VARCHAR NOT NULL,
                line_no BIGINT,
                reason VARCHAR NOT NULL
            );
            CREATE TABLE destination_snapshots (
                source_key VARCHAR PRIMARY KEY,
                qitem_id VARCHAR NOT NULL,
                destination VARCHAR NOT NULL,
                snapshot_cutoff VARCHAR NOT NULL,
                reroute_marked BOOLEAN NOT NULL,
                missions VARCHAR NOT NULL
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
            params![redact(&note.path), note.line, redact(&note.reason)],
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

    pub fn latest_usage_key(&self, source_id: &str, generation: i64) -> Result<Option<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT source_key FROM usage_facts WHERE source_id = ? AND generation = ? ORDER BY seq DESC LIMIT 1",
        )?;
        let mut rows = stmt.query(params![source_id, generation])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row.get(0)?))
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
                redact(source_id),
                redact(harness),
                redact(display_path),
                generation,
                byte_offset,
                redact(prefix_sha256)
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
                    redact(source_id),
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
                       currency_total, generation, source_id,
                       predecessor_source_key, predecessor_reason,
                       prior_input_tokens, prior_cached_input_tokens,
                       prior_cache_write_input_tokens, prior_output_tokens,
                       prior_reasoning_output_tokens
                   ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                   ON CONFLICT (source_key) DO NOTHING"#,
                params![
                    redact(&u.source_key),
                    redact(&u.harness),
                    redact(&u.native_session),
                    u.message_id.as_deref().map(redact),
                    u.ts.map(|t| t.to_rfc3339()),
                    u.seq,
                    u.input_uncached,
                    u.output,
                    u.cache_read,
                    u.cache_write,
                    u.reasoning,
                    u.currency_total,
                    u.generation,
                    redact(source_id),
                    u.predecessor_source_key.as_deref().map(redact),
                    u.predecessor_reason.as_deref(),
                    u.prior_input_tokens,
                    u.prior_cached_input_tokens,
                    u.prior_cache_write_input_tokens,
                    u.prior_output_tokens,
                    u.prior_reasoning_output_tokens
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
                    redact(&t.source_key),
                    redact(&t.harness),
                    redact(&t.native_session),
                    t.ts.map(|v| v.to_rfc3339()),
                    t.seq,
                    redact(&t.name),
                    t.fingerprint.as_deref().map(redact),
                    t.failed,
                    t.structured_write,
                    t.generation,
                    redact(source_id)
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
                redact(&fact.source_key),
                redact(&fact.harness),
                redact(&fact.native_session),
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
                redact(&fact.source_key),
                redact(&fact.seat),
                fact.ts.to_rfc3339(),
                activity_state(Some(fact.state.as_str())),
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
               ON CONFLICT (source_key) DO UPDATE SET
                 chain_id = COALESCE(queue_facts.chain_id, excluded.chain_id)"#,
            params![
                redact(&fact.source_key),
                fact.transition_id,
                redact(&fact.qitem_id),
                fact.ts.to_rfc3339(),
                queue_state(&fact.state),
                redact(&fact.source_seat),
                redact(&fact.dest_seat),
                closure_reason(fact.closure_reason.as_deref()),
                fact.closure_target.as_deref().map(redact),
                fact.chain_id.as_deref().map(redact),
                serde_json::to_string(
                    &fact.missions.iter().map(|m| redact(m)).collect::<Vec<_>>()
                )?
            ],
        )?;
        Ok(())
    }

    /// `reroute_marked`: the source shows a K5 reroute mark for this item.
    pub fn insert_destination_snapshot(
        &self,
        qitem_id: &str,
        destination: &str,
        cutoff: &str,
        reroute_marked: bool,
        missions: &[String],
    ) -> Result<()> {
        let qitem_id = redact(qitem_id);
        let destination = redact(destination);
        let cutoff = redact(cutoff);
        let missions =
            serde_json::to_string(&missions.iter().map(|m| redact(m)).collect::<Vec<_>>())?;
        let source_key = format!("queue_items:{qitem_id}:{destination}:{cutoff}:{missions}");
        self.conn.execute(
            r#"INSERT INTO destination_snapshots (source_key, qitem_id, destination, snapshot_cutoff, reroute_marked, missions)
               VALUES (?, ?, ?, ?, ?, ?)
               ON CONFLICT (source_key) DO UPDATE SET
                 reroute_marked = destination_snapshots.reroute_marked OR excluded.reroute_marked"#,
            params![source_key, qitem_id, destination, cutoff, reroute_marked, missions],
        )?;
        Ok(())
    }

    /// Replaces every seat life. Lives are rebuilt from the whole event history on each ingest.
    pub fn replace_seat_bounds(&self, bounds: &[SeatBound]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM seat_bounds", [])?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO seat_bounds (seat, created_at, stopped_at) VALUES (?, ?, ?)",
            )?;
            for bound in bounds {
                stmt.execute(params![
                    redact(&bound.seat),
                    bound.created_at.map(|t| t.to_rfc3339()),
                    bound.stopped_at.map(|t| t.to_rfc3339()),
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// One recorded role observation for a seat (K7): `check` or `other`, with
    /// the snapshot evidence it came from.
    pub fn insert_seat_role(&self, seat: &str, role: &str, evidence: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO seat_roles (seat, role, evidence) VALUES (?, ?, ?) ON CONFLICT DO NOTHING",
            params![redact(seat), redact(role), redact(evidence)],
        )?;
        Ok(())
    }

    pub fn load_seat_roles(&self) -> Result<Vec<(String, String, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT seat, role, evidence FROM seat_roles ORDER BY seat, role, evidence")?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn load_seat_bounds(&self) -> Result<Vec<SeatBound>> {
        let mut stmt = self
            .conn
            .prepare("SELECT seat, CAST(created_at AS VARCHAR), CAST(stopped_at AS VARCHAR) FROM seat_bounds")?;
        let rows = stmt.query_map([], |row| {
            let created: Option<String> = row.get(1)?;
            let stopped: Option<String> = row.get(2)?;
            Ok(SeatBound {
                seat: row.get(0)?,
                created_at: created.as_deref().and_then(parse_utc_time),
                stopped_at: stopped.as_deref().and_then(parse_utc_time),
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Every stored snapshot, oldest cutoff first, as (qitem, snapshot).
    pub fn load_destination_snapshots(&self) -> Result<Vec<(String, DestinationSnapshot)>> {
        let mut stmt = self.conn.prepare(
            "SELECT qitem_id, destination, snapshot_cutoff, reroute_marked, missions FROM destination_snapshots ORDER BY snapshot_cutoff, qitem_id",
        )?;
        let rows = stmt.query_map([], |row| {
            let missions: String = row.get(4)?;
            Ok((
                row.get::<_, String>(0)?,
                DestinationSnapshot {
                    destination: row.get(1)?,
                    cutoff: row.get(2)?,
                    reroute_marked: row.get(3)?,
                    missions: serde_json::from_str(&missions).unwrap_or_default(),
                },
            ))
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn insert_skill(&self, fact: &SkillFact) -> Result<()> {
        self.conn.execute(
            r#"INSERT INTO skill_facts (source_key, seat, skill, invoked) VALUES (?, ?, ?, ?)
               ON CONFLICT (source_key) DO NOTHING"#,
            params![
                redact(&fact.source_key),
                redact(&fact.seat),
                redact(&fact.skill),
                fact.invoked
            ],
        )?;
        Ok(())
    }

    pub fn load_sources(&self) -> Result<Vec<(String, String, String, i64)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT source_id, harness, display_path, generation FROM source_files")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn load_usage(&self) -> Result<Vec<UsageFact>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT source_key, harness, native_session, message_id, CAST(ts AS VARCHAR), seq,
                      input_uncached, output_tokens, cache_read, cache_write, reasoning,
                      currency_total, generation, predecessor_source_key, predecessor_reason,
                      prior_input_tokens, prior_cached_input_tokens, prior_cache_write_input_tokens,
                      prior_output_tokens, prior_reasoning_output_tokens
               FROM usage_facts ORDER BY ts NULLS LAST, seq"#,
        )?;
        let rows = stmt.query_map([], |row| {
            let ts: Option<String> = row.get(4)?;
            Ok(UsageFact {
                source_key: row.get(0)?,
                harness: row.get(1)?,
                native_session: row.get(2)?,
                message_id: row.get(3)?,
                ts: ts.as_deref().and_then(parse_utc_time),
                seq: row.get(5)?,
                input_uncached: row.get(6)?,
                output: row.get(7)?,
                cache_read: row.get(8)?,
                cache_write: row.get(9)?,
                reasoning: row.get(10)?,
                currency_total: row.get(11)?,
                generation: row.get(12)?,
                predecessor_source_key: row.get(13)?,
                predecessor_reason: row.get(14)?,
                prior_input_tokens: row.get(15)?,
                prior_cached_input_tokens: row.get(16)?,
                prior_cache_write_input_tokens: row.get(17)?,
                prior_output_tokens: row.get(18)?,
                prior_reasoning_output_tokens: row.get(19)?,
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
                ts: ts.as_deref().and_then(parse_utc_time),
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
                ts: parse_utc_time(&ts).unwrap_or(chrono::DateTime::<chrono::Utc>::UNIX_EPOCH),
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
                ts: parse_utc_time(&ts).unwrap_or(chrono::DateTime::<chrono::Utc>::UNIX_EPOCH),
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
                ts: ts.as_deref().and_then(parse_utc_time),
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

    pub fn skill_count(&self) -> Result<i64> {
        self.conn
            .query_row("SELECT COUNT(*) FROM skill_facts", [], |row| row.get(0))
            .map_err(Into::into)
    }

    pub fn warning_reasons(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT reason FROM warnings")?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
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

/// Parses RFC 3339, or a zone-less `YYYY-MM-DD HH:MM:SS[.f]` read as UTC
/// (the OpenRig `events.created_at` form).
pub fn parse_utc_time(value: &str) -> Option<chrono::DateTime<chrono::Utc>> {
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
    use chrono::Utc;

    #[test]
    fn text_columns_match_the_created_schema() {
        let store = AnalyticsStore::open_in_memory("key").unwrap();
        let mut stmt = store
            .conn
            .prepare("SELECT table_name, column_name FROM information_schema.columns WHERE data_type = 'VARCHAR' ORDER BY table_name, column_name")
            .unwrap();
        let rows = stmt
            .query_map([], |row| {
                Ok(format!(
                    "{}.{}",
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?
                ))
            })
            .unwrap();
        let mut got: Vec<String> = rows.map(|row| row.unwrap()).collect();
        got.sort();
        let mut expect: Vec<String> = crate::privacy::TEXT_COLUMNS
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        expect.sort();
        assert_eq!(got, expect);
    }

    #[test]
    fn secret_activity_state_is_stored_as_unknown() {
        let store = AnalyticsStore::open_in_memory("key").unwrap();
        store
            .insert_activity(&ActivityFact {
                source_key: "activity:1".into(),
                seat: "seat-a".into(),
                ts: Utc::now(),
                state: "sk-ant-FAKESTATE".into(),
                seq: 1,
            })
            .unwrap();
        let rows = store.load_activity().unwrap();
        assert_eq!(rows[0].state, "unknown");
        store
            .insert_queue(&QueueFact {
                source_key: "queue:1".into(),
                transition_id: 1,
                qitem_id: "q".into(),
                ts: Utc::now(),
                state: "sk-ant-FAKESTATE".into(),
                source_seat: "seat-a".into(),
                dest_seat: "seat-a".into(),
                closure_reason: Some("handed_off_to".into()),
                closure_target: None,
                chain_id: Some("chain-sk-ant-FAKESTATE".into()),
                missions: vec![],
            })
            .unwrap();
        let queue = store.load_queue().unwrap();
        assert_eq!(queue[0].state, "unknown");
        assert_eq!(queue[0].closure_reason.as_deref(), Some("handed_off_to"));
        assert!(!queue[0]
            .chain_id
            .as_deref()
            .unwrap_or("")
            .contains("FAKESTATE"));
    }

    #[test]
    fn privacy_matrix_keeps_canaries_out_of_every_text_column() {
        let store = AnalyticsStore::open_in_memory("key").unwrap();
        let canary = "sk-ant-QA_SYNTHETIC_MATRIX";
        let now = Utc::now();
        store
            .commit_file_batch(
                canary,
                canary,
                canary,
                1,
                0,
                canary,
                None,
                &[UsageFact {
                    source_key: canary.into(),
                    harness: canary.into(),
                    native_session: canary.into(),
                    message_id: Some(canary.into()),
                    ts: Some(now),
                    seq: 1,
                    input_uncached: 1,
                    output: 1,
                    cache_read: 0,
                    cache_write: 0,
                    reasoning: 0,
                    currency_total: None,
                    generation: 1,
                    predecessor_source_key: None,
                    predecessor_reason: None,
                    prior_input_tokens: None,
                    prior_cached_input_tokens: None,
                    prior_cache_write_input_tokens: None,
                    prior_output_tokens: None,
                    prior_reasoning_output_tokens: None,
                }],
                &[ToolFact {
                    source_key: format!("tool-{canary}"),
                    harness: canary.into(),
                    native_session: canary.into(),
                    ts: Some(now),
                    seq: 1,
                    name: canary.into(),
                    fingerprint: Some(canary.into()),
                    failed: Some(false),
                    structured_write: false,
                    generation: 1,
                }],
            )
            .unwrap();
        store
            .insert_identity(&IdentityFact {
                source_key: format!("id-{canary}"),
                harness: canary.into(),
                native_session: canary.into(),
                rig: canary.into(),
                seat: canary.into(),
                ts: Some(now),
            })
            .unwrap();
        store
            .insert_activity(&ActivityFact {
                source_key: format!("act-{canary}"),
                seat: canary.into(),
                ts: now,
                state: canary.into(),
                seq: 1,
            })
            .unwrap();
        store
            .insert_queue(&QueueFact {
                source_key: format!("q-{canary}"),
                transition_id: 1,
                qitem_id: canary.into(),
                ts: now,
                state: canary.into(),
                source_seat: canary.into(),
                dest_seat: canary.into(),
                closure_reason: Some("handed_off_to".into()),
                closure_target: Some(canary.into()),
                chain_id: Some(canary.into()),
                missions: vec![canary.into()],
            })
            .unwrap();
        store
            .insert_skill(&SkillFact {
                source_key: format!("skill-{canary}"),
                seat: canary.into(),
                skill: canary.into(),
                invoked: true,
            })
            .unwrap();
        store
            .record_warning(&WarningNote {
                path: canary.into(),
                line: Some(1),
                reason: canary.into(),
            })
            .unwrap();
        store
            .insert_destination_snapshot(canary, canary, canary, true, &[canary.to_string()])
            .unwrap();
        store.insert_seat_role(canary, canary, canary).unwrap();
        store
            .replace_seat_bounds(&[SeatBound {
                seat: canary.into(),
                created_at: Some(now),
                stopped_at: None,
            }])
            .unwrap();
        let mut checked = 0usize;
        for qualified in crate::privacy::TEXT_COLUMNS {
            let (table, column) = qualified.split_once('.').unwrap();
            let sql = format!("SELECT COUNT(*) FROM {table} WHERE CAST({column} AS VARCHAR) LIKE '%QA_SYNTHETIC_MATRIX%'");
            let hits: i64 = store.conn.query_row(&sql, [], |row| row.get(0)).unwrap();
            assert_eq!(hits, 0, "{qualified}");
            checked += 1;
        }
        assert_eq!(checked, crate::privacy::TEXT_COLUMNS.len());
        let state: String = store
            .conn
            .query_row("SELECT state FROM activity_facts", [], |row| row.get(0))
            .unwrap();
        assert_eq!(state, "unknown");
        let reason: String = store
            .conn
            .query_row("SELECT closure_reason FROM queue_facts", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(reason, "handed_off_to");
        for table in ["seat_bounds", "destination_snapshots", "seat_roles"] {
            let rows: i64 = store
                .conn
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(rows, 1, "{table} row carries the canary and was checked");
        }
    }

    #[test]
    fn privacy_matrix_covers_every_marker_family_on_disk() {
        let dir = std::env::temp_dir().join(format!("dendro-matrix-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("analytics.duckdb");
        let store = AnalyticsStore::open_path(&path, "key").unwrap();
        let families = [
            "sk-ZZZ1",
            "sk-ant-ZZZ2",
            "ghp_ZZZ3",
            "gho_ZZZ4",
            "github_pat_ZZZ5",
            "xai-ZZZ6",
            "AKIAZZZ7",
            "Bearer ZZZ8",
            "xoxb-ZZZ9",
            "xoxp-ZZZ0",
        ];
        for (index, token) in families.iter().enumerate() {
            let now = Utc::now();
            store
                .insert_activity(&ActivityFact {
                    source_key: format!("act-{token}"),
                    seat: (*token).into(),
                    ts: now,
                    state: (*token).into(),
                    seq: index as i64,
                })
                .unwrap();
            store
                .insert_queue(&QueueFact {
                    source_key: format!("q-{token}"),
                    transition_id: index as i64,
                    qitem_id: (*token).into(),
                    ts: now,
                    state: (*token).into(),
                    source_seat: (*token).into(),
                    dest_seat: (*token).into(),
                    closure_reason: Some((*token).into()),
                    closure_target: Some((*token).into()),
                    chain_id: Some((*token).into()),
                    missions: vec![(*token).into()],
                })
                .unwrap();
            store
                .record_warning(&WarningNote {
                    path: (*token).into(),
                    line: Some(1),
                    reason: (*token).into(),
                })
                .unwrap();
            store
                .insert_identity(&IdentityFact {
                    source_key: format!("id-{token}"),
                    harness: (*token).into(),
                    native_session: (*token).into(),
                    rig: (*token).into(),
                    seat: (*token).into(),
                    ts: Some(now),
                })
                .unwrap();
            store
                .insert_skill(&SkillFact {
                    source_key: format!("skill-{token}"),
                    seat: (*token).into(),
                    skill: (*token).into(),
                    invoked: true,
                })
                .unwrap();
            store
                .commit_file_batch(
                    token,
                    token,
                    token,
                    index as i64 + 1,
                    0,
                    token,
                    None,
                    &[UsageFact {
                        source_key: format!("use-{token}"),
                        harness: (*token).into(),
                        native_session: (*token).into(),
                        message_id: Some((*token).into()),
                        ts: Some(now),
                        seq: index as i64,
                        input_uncached: 1,
                        output: 1,
                        cache_read: 0,
                        cache_write: 0,
                        reasoning: 0,
                        currency_total: None,
                        generation: index as i64 + 1,
                        predecessor_source_key: None,
                        predecessor_reason: None,
                        prior_input_tokens: None,
                        prior_cached_input_tokens: None,
                        prior_cache_write_input_tokens: None,
                        prior_output_tokens: None,
                        prior_reasoning_output_tokens: None,
                    }],
                    &[ToolFact {
                        source_key: format!("tool-{token}"),
                        harness: (*token).into(),
                        native_session: (*token).into(),
                        ts: Some(now),
                        seq: index as i64,
                        name: (*token).into(),
                        fingerprint: Some((*token).into()),
                        failed: None,
                        structured_write: false,
                        generation: index as i64 + 1,
                    }],
                )
                .unwrap();
        }
        for qualified in crate::privacy::TEXT_COLUMNS {
            let (table, column) = qualified.split_once('.').unwrap();
            let sql = format!(
                "SELECT COUNT(*) FROM {table} WHERE CAST({column} AS VARCHAR) LIKE '%ZZZ%'"
            );
            let hits: i64 = store.conn.query_row(&sql, [], |row| row.get(0)).unwrap();
            assert_eq!(hits, 0, "{qualified}");
        }
        let wal = {
            let mut name = path.file_name().unwrap().to_os_string();
            name.push(".wal");
            path.with_file_name(name)
        };
        let key_id: String = store
            .conn
            .query_row("SELECT key_id FROM dendro_meta", [], |row| row.get(0))
            .unwrap();
        assert_eq!(key_id, "key");
        assert!(!key_id.contains("ZZZ"));
        let bounds: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM seat_bounds", [], |row| row.get(0))
            .unwrap();
        assert_eq!(bounds, 0);
        assert!(wal.exists(), "expected a DuckDB wal before checkpoint");
        let wal_bytes = std::fs::read(&wal).unwrap();
        let wal_blob = String::from_utf8_lossy(&wal_bytes);
        for token in families {
            assert!(!wal_blob.contains(token), "wal {token}");
        }
        store.checkpoint().unwrap();
        drop(store);
        let bytes = std::fs::read(&path).unwrap();
        let blob = String::from_utf8_lossy(&bytes);
        for token in families {
            assert!(!blob.contains(token), "{token}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

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
                predecessor_source_key: None,
                predecessor_reason: None,
                prior_input_tokens: None,
                prior_cached_input_tokens: None,
                prior_cache_write_input_tokens: None,
                prior_output_tokens: None,
                prior_reasoning_output_tokens: None,
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
