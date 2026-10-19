#[cfg(test)]
thread_local! {pub(crate) static FRICTION_METADATA_READS: std::cell::Cell<usize> = const {std::cell::Cell::new(0)};}

#[path = "openrig_provenance.rs"]
mod openrig_provenance;
pub use openrig_provenance::{OpenRigIngest, OpenRigRecord, OpenRigRow};

use std::cell::RefCell;
use std::path::Path;

use anyhow::{anyhow, Result};
use duckdb::{params, Connection};

use crate::facts::*;
use crate::file_metadata::FileMetadata;
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FileBatchStage {
    BeforeReaderReplacement,
    AfterReaderReplacement,
    BeforeSourceSessionUpdate,
    BeforeCommit,
}

/// Metadata association captured around an ordinary accepted ingest read.
/// An unavailable attempt keeps earlier stable evidence private and unchanged.
#[derive(Clone, Debug)]
pub struct FileBaselineAttempt {
    pub attempted_at: String,
    pub metadata: Option<FileMetadata>,
}

const FILE_BASELINES_SCHEMA: &str = "CREATE TABLE IF NOT EXISTS file_source_baselines (
    source_id VARCHAR NOT NULL,
    generation BIGINT NOT NULL,
    metadata_version INTEGER NOT NULL,
    latest_association VARCHAR NOT NULL,
    latest_attempt_at VARCHAR NOT NULL,
    last_stable_observed_at VARCHAR,
    last_stable_type VARCHAR,
    last_stable_device VARCHAR,
    last_stable_inode VARCHAR,
    last_stable_size VARCHAR,
    last_stable_mtime_ns VARCHAR,
    association_reason VARCHAR NOT NULL,
    PRIMARY KEY (source_id, generation)
)";

pub struct AnalyticsStore {
    conn: Connection,
    pub key_id: String,
    openrig_rows: RefCell<Option<openrig_provenance::OpenRigRowCache>>,
    openrig_pending: RefCell<Option<Vec<OpenRigRecord>>>,
    canonical_write_active: std::cell::Cell<bool>,
}

impl AnalyticsStore {
    pub(crate) fn connection(&self) -> &Connection {
        &self.conn
    }

    // Derived cache state only: canonical fact schemas remain unchanged.
    pub(super) const SEMANTIC_STATE_SCHEMA: &'static str = "CREATE TABLE IF NOT EXISTS dendro_semantic_state (id INTEGER PRIMARY KEY, dirty BOOLEAN NOT NULL); INSERT INTO dendro_semantic_state VALUES (1, TRUE) ON CONFLICT DO NOTHING";

    pub(super) fn invalidate_semantic(&self) -> Result<()> {
        self.conn.execute(
            "UPDATE dendro_semantic_state SET dirty = TRUE WHERE id = 1 AND NOT dirty",
            [],
        )?;
        Ok(())
    }

    /// Join our existing canonical batch, or atomically fence a standalone write.
    /// duckdb-rs 1.x is_autocommit() is a stub, so track only transactions we own.
    pub(super) fn canonical_write<T>(&self, write: impl FnOnce() -> Result<T>) -> Result<T> {
        if self.canonical_write_active.get() {
            return write();
        }
        let tx = self.conn.unchecked_transaction()?;
        self.canonical_write_active.set(true);
        struct Reset<'a>(&'a std::cell::Cell<bool>);
        impl Drop for Reset<'_> {
            fn drop(&mut self) {
                self.0.set(false);
            }
        }
        let _reset = Reset(&self.canonical_write_active);
        self.invalidate_semantic()?;
        let result = write()?;
        tx.commit()?;
        Ok(result)
    }

    pub fn open_in_memory(key_id: &str) -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let store = Self {
            conn,
            key_id: key_id.to_string(),
            openrig_rows: RefCell::new(None),
            openrig_pending: RefCell::new(None),
            canonical_write_active: std::cell::Cell::new(false),
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
            openrig_rows: RefCell::new(None),
            openrig_pending: RefCell::new(None),
            canonical_write_active: std::cell::Cell::new(false),
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
        conn.execute_batch(Self::SEMANTIC_STATE_SCHEMA)?;
        Ok(Self {
            conn,
            key_id: key_id.to_string(),
            openrig_rows: RefCell::new(None),
            openrig_pending: RefCell::new(None),
            canonical_write_active: std::cell::Cell::new(false),
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
        conn.execute_batch(openrig_provenance::SCHEMA)?;
        conn.execute_batch(Self::SEMANTIC_STATE_SCHEMA)?;
        Ok(Self {
            conn,
            key_id,
            openrig_rows: RefCell::new(None),
            openrig_pending: RefCell::new(None),
            canonical_write_active: std::cell::Cell::new(false),
        })
    }

    pub fn open_read_only(path: &Path) -> Result<Self> {
        let config = duckdb::Config::default().access_mode(duckdb::AccessMode::ReadOnly)?;
        let conn = Connection::open_with_flags(path, config).map_err(|_| {
            if !path.exists() {
                anyhow!("analytics database could not be opened (not found; run 'dendro run' or 'dendro dashboard --auto-ingest' first)")
            } else {
                anyhow!("analytics database could not be opened")
            }
        })?;
        refuse_legacy(&conn)?;
        let (version, key_id): (i32, String) = conn.query_row(
            "SELECT schema_version, key_id FROM dendro_meta",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if version != SCHEMA_VERSION {
            return Err(anyhow!("unsupported analytics schema version {version}"));
        }
        Ok(Self {
            conn,
            key_id,
            openrig_rows: RefCell::new(None),
            openrig_pending: RefCell::new(None),
            canonical_write_active: std::cell::Cell::new(false),
        })
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
                prefix_sha256 VARCHAR NOT NULL,
                native_session VARCHAR
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
                prior_reasoning_output_tokens BIGINT,
                model VARCHAR,
                effort VARCHAR
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
                source_id VARCHAR NOT NULL,
                category VARCHAR NOT NULL DEFAULT 'other'
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
                missions VARCHAR NOT NULL,
                is_recovery BOOLEAN NOT NULL DEFAULT FALSE,
                recovery_reason VARCHAR,
                last_nudge_result VARCHAR,
                issue_tags VARCHAR NOT NULL DEFAULT '[]',
                phase_tags VARCHAR NOT NULL DEFAULT '[]',
                parent_qitem_id VARCHAR
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
        self.conn.execute_batch(Self::SEMANTIC_STATE_SCHEMA)?;
        self.conn.execute_batch(FILE_BASELINES_SCHEMA)?;
        self.conn.execute_batch(openrig_provenance::SCHEMA)?;
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
        self.canonical_write(|| {
            self.conn.execute(
                "INSERT INTO warnings (path, line_no, reason) VALUES (?, ?, ?)",
                params![redact(&note.path), note.line, redact(&note.reason)],
            )?;
            Ok(())
        })
    }

    /// Whether this warning was already stored for this path, compared as stored.
    pub fn has_warning(&self, path: &str, reason: &str) -> Result<bool> {
        let mut stmt = self
            .conn
            .prepare("SELECT 1 FROM warnings WHERE path = ? AND reason = ? LIMIT 1")?;
        let mut rows = stmt.query(params![redact(path), redact(reason)])?;
        Ok(rows.next()?.is_some())
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
        self.commit_file_batch_settling(
            source_id,
            harness,
            display_path,
            generation,
            byte_offset,
            prefix_sha256,
            counters,
            usages,
            tools,
            &[],
        )
    }

    /// As `commit_file_batch`, and in the same transaction re-settles the
    /// outcome of tool calls stored by an earlier run (#37).
    #[allow(clippy::too_many_arguments)]
    pub fn commit_file_batch_settling(
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
        settled: &[(String, Option<bool>)],
    ) -> Result<()> {
        self.commit_file_batch_internal(
            source_id,
            harness,
            display_path,
            generation,
            byte_offset,
            prefix_sha256,
            counters,
            usages,
            tools,
            settled,
            None,
            None,
            |_| Ok(()),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn commit_file_batch_reading(
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
        settled: &[(String, Option<bool>)],
        reader: &ReaderState,
    ) -> Result<()> {
        self.commit_file_batch_internal(
            source_id,
            harness,
            display_path,
            generation,
            byte_offset,
            prefix_sha256,
            counters,
            usages,
            tools,
            settled,
            Some(reader),
            None,
            |_| Ok(()),
        )
    }

    /// Commits metadata association together with the exact accepted file batch.
    #[allow(clippy::too_many_arguments)]
    pub fn commit_file_batch_observed(
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
        settled: &[(String, Option<bool>)],
        reader: &ReaderState,
        baseline: &FileBaselineAttempt,
    ) -> Result<()> {
        self.commit_file_batch_internal(
            source_id,
            harness,
            display_path,
            generation,
            byte_offset,
            prefix_sha256,
            counters,
            usages,
            tools,
            settled,
            Some(reader),
            Some(baseline),
            |_| Ok(()),
        )
    }

    // The callback is private: tests inject faults into this actual transaction.
    // Production callers always use the infallible closure above.
    #[allow(clippy::too_many_arguments)]
    fn commit_file_batch_internal(
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
        settled: &[(String, Option<bool>)],
        reader: Option<&ReaderState>,
        baseline: Option<&FileBaselineAttempt>,
        mut checkpoint: impl FnMut(FileBatchStage) -> Result<()>,
    ) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        self.invalidate_semantic()?;
        if let Some(attempt) = baseline {
            tx.execute_batch(FILE_BASELINES_SCHEMA)?;
            let metadata = attempt.metadata.as_ref();
            let verified = metadata.is_some();
            tx.execute(
                "INSERT INTO file_source_baselines VALUES (?, ?, 1, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                 ON CONFLICT (source_id, generation) DO UPDATE SET
                   metadata_version = 1,
                   latest_association = excluded.latest_association,
                   latest_attempt_at = excluded.latest_attempt_at,
                   last_stable_observed_at = CASE WHEN excluded.latest_association = 'verified' THEN excluded.last_stable_observed_at ELSE file_source_baselines.last_stable_observed_at END,
                   last_stable_type = CASE WHEN excluded.latest_association = 'verified' THEN excluded.last_stable_type ELSE file_source_baselines.last_stable_type END,
                   last_stable_device = CASE WHEN excluded.latest_association = 'verified' THEN excluded.last_stable_device ELSE file_source_baselines.last_stable_device END,
                   last_stable_inode = CASE WHEN excluded.latest_association = 'verified' THEN excluded.last_stable_inode ELSE file_source_baselines.last_stable_inode END,
                   last_stable_size = CASE WHEN excluded.latest_association = 'verified' THEN excluded.last_stable_size ELSE file_source_baselines.last_stable_size END,
                   last_stable_mtime_ns = CASE WHEN excluded.latest_association = 'verified' THEN excluded.last_stable_mtime_ns ELSE file_source_baselines.last_stable_mtime_ns END,
                   association_reason = excluded.association_reason",
                params![
                    redact(source_id), generation,
                    if verified { "verified" } else { "unavailable" },
                    redact(&attempt.attempted_at),
                    verified.then(|| redact(&attempt.attempted_at)),
                    metadata.map(|m| redact(&m.r#type)),
                    metadata.map(|m| redact(&m.device)),
                    metadata.map(|m| redact(&m.inode)),
                    metadata.map(|m| redact(&m.size)),
                    metadata.map(|m| redact(&m.mtime_ns)),
                    if verified { "verified" } else { "association_unverified" },
                ],
            )?;
        }
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
        // Bulk-append into a temp table, then one conflict-tolerant insert.
        // A prepared INSERT per row keeps DuckDB result memory growing with the file.
        if !usages.is_empty() {
            tx.execute_batch(
                "CREATE TEMP TABLE usage_stage (
                    source_key VARCHAR, harness VARCHAR, native_session VARCHAR,
                    message_id VARCHAR, ts VARCHAR, seq BIGINT,
                    input_uncached BIGINT, output_tokens BIGINT, cache_read BIGINT,
                    cache_write BIGINT, reasoning BIGINT, currency_total DOUBLE,
                    generation BIGINT, source_id VARCHAR, predecessor_source_key VARCHAR,
                    predecessor_reason VARCHAR, prior_input_tokens BIGINT,
                    prior_cached_input_tokens BIGINT, prior_cache_write_input_tokens BIGINT,
                    prior_output_tokens BIGINT, prior_reasoning_output_tokens BIGINT,
                    model VARCHAR, effort VARCHAR
                )",
            )?;
            {
                let mut appender = tx.appender("usage_stage")?;
                for u in usages {
                    appender.append_row(params![
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
                        u.prior_reasoning_output_tokens,
                        Option::<String>::None,
                        Option::<String>::None
                    ])?;
                }
                appender.flush()?;
            }
            tx.execute_batch(
                "INSERT INTO usage_facts
                 SELECT source_key, harness, native_session, message_id,
                        CAST(ts AS TIMESTAMP), seq, input_uncached, output_tokens,
                        cache_read, cache_write, reasoning, currency_total, generation,
                        source_id, predecessor_source_key, predecessor_reason,
                        prior_input_tokens, prior_cached_input_tokens,
                        prior_cache_write_input_tokens, prior_output_tokens,
                        prior_reasoning_output_tokens, model, effort
                 FROM usage_stage
                 ON CONFLICT (source_key) DO NOTHING;
                 DROP TABLE usage_stage",
            )?;
        }
        if !tools.is_empty() {
            tx.execute_batch(
                "CREATE TEMP TABLE tool_stage (
                    source_key VARCHAR, harness VARCHAR, native_session VARCHAR,
                    ts VARCHAR, seq BIGINT, tool_name VARCHAR, fingerprint VARCHAR,
                    failed BOOLEAN, structured_write BOOLEAN, generation BIGINT,
                    source_id VARCHAR, category VARCHAR
                )",
            )?;
            {
                let mut appender = tx.appender("tool_stage")?;
                for t in tools {
                    appender.append_row(params![
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
                        redact(source_id),
                        ToolCategory::classify_tool_name(&t.name).as_str()
                    ])?;
                }
                appender.flush()?;
            }
            tx.execute_batch(
                "INSERT INTO tool_facts
                 SELECT source_key, harness, native_session, CAST(ts AS TIMESTAMP), seq,
                        tool_name, fingerprint, failed, structured_write, generation, source_id, category
                 FROM tool_stage
                 ON CONFLICT (source_key) DO NOTHING;
                 DROP TABLE tool_stage",
            )?;
        }
        // A tool result appended after its call was stored settles the call's
        // outcome again from every result seen so far (#37).
        if !settled.is_empty() {
            let mut settle_stmt =
                tx.prepare("UPDATE tool_facts SET failed = ? WHERE source_key = ?")?;
            for (source_key, failed) in settled {
                settle_stmt.execute(params![failed, redact(source_key)])?;
            }
        }
        if let Some(reader) = reader {
            checkpoint(FileBatchStage::BeforeReaderReplacement)?;
            let marker = format!(
                "{READER_NAMESPACE}{READER_VERSION}:{}:{generation}",
                redact(source_id)
            );
            tx.execute(
                "DELETE FROM tool_facts WHERE source_key = ?",
                params![marker],
            )?;
            tx.execute(
                "INSERT INTO tool_facts (source_key, harness, native_session, ts, seq, tool_name, fingerprint, failed, structured_write, generation, source_id, category)
                 VALUES (?, ?, ?, NULL, ?, ?, ?, NULL, FALSE, ?, ?, 'other')",
                params![marker, redact(harness), reader.native_session.as_deref().map(redact).unwrap_or_default(),
                    byte_offset, if reader.complete { READER_COMPLETE } else { READER_UNSUPPORTED },
                    redact(prefix_sha256), generation, redact(source_id)],
            )?;
            checkpoint(FileBatchStage::AfterReaderReplacement)?;
            checkpoint(FileBatchStage::BeforeSourceSessionUpdate)?;
            tx.execute(
                "UPDATE source_files SET native_session = ? WHERE source_id = ?",
                params![
                    reader.native_session.as_deref().map(redact),
                    redact(source_id)
                ],
            )?;
        }
        checkpoint(FileBatchStage::BeforeCommit)?;
        tx.commit()?;
        Ok(())
    }

    pub fn record_tool_categories(&self, categories: &[(String, ToolCategory)]) -> Result<()> {
        if categories.is_empty() {
            return Ok(());
        }
        let tx = self.conn.unchecked_transaction()?;
        self.invalidate_semantic()?;
        {
            let mut stmt = tx.prepare("UPDATE tool_facts SET category = ? WHERE source_key = ?")?;
            for (source_key, category) in categories {
                stmt.execute(params![category.as_str(), redact(source_key)])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn load_tool_categories(&self) -> Result<std::collections::BTreeMap<String, ToolCategory>> {
        let mut stmt = self.conn.prepare(
            "SELECT source_key, category FROM tool_facts WHERE NOT starts_with(source_key, 'dendro-internal:reader:')",
        )?;
        let rows = stmt.query_map([], |row| {
            let key: String = row.get(0)?;
            let label: String = row.get(1)?;
            Ok((key, ToolCategory::from_label(&label)))
        })?;
        rows.collect::<Result<_, _>>().map_err(Into::into)
    }

    pub fn record_usage_models(
        &self,
        models: &[(String, Option<String>, Option<String>)],
    ) -> Result<()> {
        if models.is_empty() {
            return Ok(());
        }
        let tx = self.conn.unchecked_transaction()?;
        self.invalidate_semantic()?;
        {
            let mut stmt =
                tx.prepare("UPDATE usage_facts SET model = ?, effort = ? WHERE source_key = ?")?;
            for (source_key, model, effort) in models {
                stmt.execute(params![
                    model.as_deref().map(redact),
                    effort.as_deref().map(redact),
                    redact(source_key)
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    #[allow(clippy::type_complexity)]
    pub fn load_usage_models(
        &self,
    ) -> Result<std::collections::BTreeMap<String, (Option<String>, Option<String>)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT source_key, model, effort FROM usage_facts WHERE model IS NOT NULL OR effort IS NOT NULL")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                (
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ),
            ))
        })?;
        rows.collect::<Result<_, _>>().map_err(Into::into)
    }

    pub fn record_queue_item_meta(&self, meta: &QueueItemMeta) -> Result<()> {
        self.canonical_write(|| {
            let qitem_id = redact(&meta.qitem_id);
            let recovery_reason = meta.recovery_reason.as_deref().map(redact);
            let last_nudge_result = meta.last_nudge_result.as_deref().map(redact);
            let issue_tags = serde_json::to_string(
                &meta
                    .issue_tags
                    .iter()
                    .map(|t| redact(t))
                    .collect::<Vec<_>>(),
            )?;
            let phase_tags = serde_json::to_string(
                &meta
                    .phase_tags
                    .iter()
                    .map(|t| redact(t))
                    .collect::<Vec<_>>(),
            )?;
            let parent_qitem_id = meta.parent_qitem_id.as_deref().map(redact);
            self.conn.execute(
                r#"UPDATE queue_facts SET
                     is_recovery = ?,
                     recovery_reason = COALESCE(?, queue_facts.recovery_reason),
                     last_nudge_result = COALESCE(?, queue_facts.last_nudge_result),
                     issue_tags = ?,
                     phase_tags = ?,
                     parent_qitem_id = COALESCE(?, queue_facts.parent_qitem_id)
                   WHERE qitem_id = ?"#,
                params![
                    meta.is_recovery,
                    recovery_reason,
                    last_nudge_result,
                    issue_tags,
                    phase_tags,
                    parent_qitem_id,
                    qitem_id
                ],
            )?;
            Ok(())
        })
    }

    /// Preserve distinct retained parent readings for friction, without changing legacy aggregation.
    pub fn load_queue_metadata_evidence(&self) -> Result<Vec<crate::vortex::ItemEvidence>> {
        #[cfg(test)]
        FRICTION_METADATA_READS.with(|n| n.set(n.get() + 1));
        let mut stmt = self.conn.prepare(
            "SELECT source_key, qitem_id, parent_qitem_id, is_recovery, recovery_reason, issue_tags FROM queue_facts ORDER BY source_key",
        )?;
        let rows = stmt.query_map([], |row| {
            let tags: Option<String> = row.get(5)?;
            Ok(crate::vortex::ItemEvidence {
                source_key: row.get(0)?,
                qitem_id: row.get(1)?,
                parent_qitem_id: row.get(2)?,
                is_recovery: row.get::<_, Option<bool>>(3)?.unwrap_or(false),
                recovery_reason: row.get(4)?,
                issue_tags: tags
                    .as_deref()
                    .and_then(|v| serde_json::from_str(v).ok())
                    .unwrap_or_default(),
            })
        })?;
        rows.collect::<Result<_, _>>().map_err(Into::into)
    }

    pub fn load_queue_item_meta(
        &self,
    ) -> Result<std::collections::BTreeMap<String, QueueItemMeta>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT qitem_id,
                      BOOL_OR(is_recovery),
                      MAX(recovery_reason),
                      MAX(last_nudge_result),
                      MAX(issue_tags),
                      MAX(phase_tags),
                      MAX(parent_qitem_id)
               FROM queue_facts
               GROUP BY qitem_id"#,
        )?;
        let rows = stmt.query_map([], |row| {
            let qitem_id: String = row.get(0)?;
            let is_recovery: bool = row.get::<_, Option<bool>>(1)?.unwrap_or(false);
            let recovery_reason: Option<String> = row.get(2)?;
            let last_nudge_result: Option<String> = row.get(3)?;
            let issue_tags_raw: Option<String> = row.get(4)?;
            let phase_tags_raw: Option<String> = row.get(5)?;
            let parent_qitem_id: Option<String> = row.get(6)?;
            Ok((
                qitem_id.clone(),
                QueueItemMeta {
                    qitem_id,
                    is_recovery,
                    recovery_reason,
                    last_nudge_result,
                    issue_tags: issue_tags_raw
                        .as_deref()
                        .and_then(|s| serde_json::from_str(s).ok())
                        .unwrap_or_default(),
                    phase_tags: phase_tags_raw
                        .as_deref()
                        .and_then(|s| serde_json::from_str(s).ok())
                        .unwrap_or_default(),
                    parent_qitem_id,
                },
            ))
        })?;
        rows.collect::<Result<_, _>>().map_err(Into::into)
    }

    pub fn insert_identity(&self, fact: &IdentityFact) -> Result<()> {
        self.canonical_write(|| {
            self.conn
                .prepare_cached(
                    r#"INSERT INTO identity_facts (source_key, harness, native_session, rig, seat, ts)
                   VALUES (?, ?, ?, ?, ?, ?)
                   ON CONFLICT (source_key) DO NOTHING"#,
                )?
                .execute(params![
                    redact(&fact.source_key),
                    redact(&fact.harness),
                    redact(&fact.native_session),
                    redact(&fact.rig),
                    redact(&fact.seat),
                    fact.ts.map(|t| t.to_rfc3339())
                ])?;
            Ok(())
        })
    }

    pub fn insert_activity(&self, fact: &ActivityFact) -> Result<()> {
        self.canonical_write(|| {
            self.conn
                .prepare_cached(
                    r#"INSERT INTO activity_facts (source_key, seat, ts, state, seq)
                   VALUES (?, ?, ?, ?, ?)
                   ON CONFLICT (source_key) DO NOTHING"#,
                )?
                .execute(params![
                    redact(&fact.source_key),
                    redact(&fact.seat),
                    fact.ts.to_rfc3339(),
                    activity_state(Some(fact.state.as_str())),
                    fact.seq
                ])?;
            Ok(())
        })
    }

    pub fn insert_queue(&self, fact: &QueueFact) -> Result<()> {
        self.canonical_write(|| {
            let qitem_redacted = redact(&fact.qitem_id);
            let auto_recovery = qitem_redacted.starts_with("qitem-recovery-");
            self.conn
                .prepare_cached(
                    r#"INSERT INTO queue_facts (
                       source_key, transition_id, qitem_id, ts, state, source_seat, dest_seat,
                       closure_reason, closure_target, chain_id, missions, is_recovery, recovery_reason
                   ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                   ON CONFLICT (source_key) DO UPDATE SET
                     chain_id = COALESCE(queue_facts.chain_id, excluded.chain_id),
                     missions = CASE
                       WHEN queue_facts.missions = '[]' AND excluded.missions <> '[]'
                       THEN excluded.missions ELSE queue_facts.missions END"#,
                )?
                .execute(params![
                    redact(&fact.source_key),
                    fact.transition_id,
                    qitem_redacted,
                    fact.ts.to_rfc3339(),
                    queue_state(&fact.state),
                    redact(&fact.source_seat),
                    redact(&fact.dest_seat),
                    closure_reason(fact.closure_reason.as_deref()),
                    fact.closure_target.as_deref().map(redact),
                    fact.chain_id.as_deref().map(redact),
                    serde_json::to_string(
                        &fact.missions.iter().map(|m| redact(m)).collect::<Vec<_>>()
                    )?,
                    auto_recovery,
                    auto_recovery.then(|| "recovery".to_string())
                ])?;
            Ok(())
        })
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
        self.canonical_write(|| {
            let qitem_id = redact(qitem_id);
            let destination = redact(destination);
            let cutoff = redact(cutoff);
            let missions =
                serde_json::to_string(&missions.iter().map(|m| redact(m)).collect::<Vec<_>>())?;
            let source_key = format!("queue_items:{qitem_id}:{destination}:{cutoff}:{missions}");
            self.conn.prepare_cached(
                r#"INSERT INTO destination_snapshots (source_key, qitem_id, destination, snapshot_cutoff, reroute_marked, missions)
                   VALUES (?, ?, ?, ?, ?, ?)
                   ON CONFLICT (source_key) DO UPDATE SET
                     reroute_marked = destination_snapshots.reroute_marked OR excluded.reroute_marked"#)?.execute(
                params![source_key, qitem_id, destination, cutoff, reroute_marked, missions],
            )?;
            Ok(())
        })
    }

    /// Replaces every seat life. Lives are rebuilt from the whole event history on each ingest.
    pub fn replace_seat_bounds(&self, bounds: &[SeatBound]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        self.invalidate_semantic()?;
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
        self.canonical_write(|| {
            self.conn.prepare_cached(
                "INSERT INTO seat_roles (seat, role, evidence) VALUES (?, ?, ?) ON CONFLICT DO NOTHING")?.execute(
                params![redact(seat), redact(role), redact(evidence)],
            )?;
            Ok(())
        })
    }

    pub fn load_seat_roles(&self) -> Result<Vec<(String, String, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT seat, role, evidence FROM seat_roles ORDER BY seat, role, evidence")?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
        let mut roles = rows.collect::<Result<Vec<_>, _>>()?;
        let changed: std::collections::HashSet<_> = self
            .load_openrig_row_records()?
            .into_iter()
            .filter(|r| r.row_state == "changed")
            .map(|r| r.source_key)
            .collect();
        roles.retain(|(_, _, evidence)| !changed.contains(evidence));
        Ok(roles)
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
        self.canonical_write(|| {
            self.conn
                .prepare_cached(
                    r#"INSERT INTO skill_facts (source_key, seat, skill, invoked) VALUES (?, ?, ?, ?)
                   ON CONFLICT (source_key) DO NOTHING"#,
                )?
                .execute(params![
                    redact(&fact.source_key),
                    redact(&fact.seat),
                    redact(&fact.skill),
                    fact.invoked
                ])?;
            Ok(())
        })
    }

    /// Records a session file's native session (K10), sealed by the caller.
    pub fn set_source_session(&self, source_id: &str, native_session: &str) -> Result<()> {
        self.canonical_write(|| {
            self.conn.execute(
                "UPDATE source_files SET native_session = ? WHERE source_id = ?",
                params![redact(native_session), redact(source_id)],
            )?;
            Ok(())
        })
    }

    /// (source_id, harness, display_path, generation, native_session).
    #[allow(clippy::type_complexity)]
    pub fn load_sources(&self) -> Result<Vec<(String, String, String, i64, Option<String>)>> {
        let mut stmt = self.conn.prepare(
            "SELECT source_id, harness, display_path, generation, native_session FROM source_files",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Option<String>>(4)?,
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
        let mut facts = rows.collect::<Result<Vec<_>, _>>()?;
        let changed: std::collections::HashSet<_> = self
            .load_openrig_row_records()?
            .into_iter()
            .filter(|r| r.row_state == "changed")
            .map(|r| r.source_key)
            .collect();
        facts.retain(|fact| !changed.contains(&fact.source_key));
        Ok(facts)
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
        let mut facts = rows.collect::<Result<Vec<_>, _>>()?;
        let changed: std::collections::HashSet<_> = self
            .load_openrig_row_records()?
            .into_iter()
            .filter(|r| r.row_state == "changed")
            .map(|r| r.source_key)
            .collect();
        facts.retain(|fact| !changed.contains(&fact.source_key));
        Ok(facts)
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
        let mut facts = rows.collect::<Result<Vec<_>, _>>()?;
        let changed: std::collections::HashSet<_> = self
            .load_openrig_row_records()?
            .into_iter()
            .filter(|r| r.row_state == "changed")
            .map(|r| r.source_key)
            .collect();
        facts.retain(|fact| !changed.contains(&fact.source_key));
        Ok(facts)
    }

    /// A single committed read snapshot for every input to a report evaluation.
    pub(crate) fn read_snapshot<T>(&self, evaluate: impl FnOnce(&Self) -> Result<T>) -> Result<T> {
        let tx = self.conn.unchecked_transaction()?;
        let result = evaluate(self)?;
        tx.commit()?;
        Ok(result)
    }

    /// Supplemental provenance: never drives or changes load_tools enumeration.
    pub(crate) fn tool_source_ids(
        &self,
    ) -> Result<std::collections::HashMap<String, Option<String>>> {
        let mut stmt = self
            .conn
            .prepare("SELECT source_key, source_id FROM tool_facts")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<Result<_, _>>().map_err(Into::into)
    }

    pub(crate) fn source_prefixes(
        &self,
    ) -> Result<std::collections::HashMap<String, (i64, i64, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT source_id, generation, byte_offset, prefix_sha256 FROM source_files",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?, r.get(3)?))))?;
        rows.collect::<Result<_, _>>().map_err(Into::into)
    }

    pub fn load_tools(&self) -> Result<Vec<ToolFact>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT source_key, harness, native_session, CAST(ts AS VARCHAR), seq, tool_name, fingerprint,
                      failed, structured_write, generation
               FROM tool_facts WHERE NOT starts_with(source_key, 'dendro-internal:reader:') AND (harness <> 'codex' OR tool_name NOT IN ('CommandExecution', 'CodexSeatCall') OR EXISTS (SELECT 1 FROM source_files s WHERE s.source_id = tool_facts.source_id AND s.generation = tool_facts.generation)) ORDER BY native_session, ts NULLS LAST, seq"#,
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

    /// Applied reader state for the committed prefix, never a filesystem probe.
    pub fn load_source_coverage(&self) -> Result<Vec<SourceCoverage>> {
        let mut stmt = self.conn.prepare(
            "SELECT s.source_id, s.harness, s.native_session,
                EXISTS (SELECT 1 FROM tool_facts t WHERE t.source_key = ? || s.source_id || ':' || CAST(s.generation AS VARCHAR)
                    AND t.source_id = s.source_id AND t.harness = s.harness AND t.generation = s.generation
                    AND t.seq = s.byte_offset AND t.seq >= 0 AND t.generation > 0 AND t.fingerprint = s.prefix_sha256
                    AND regexp_full_match(t.fingerprint, '[0-9a-f]{64}')
                    AND t.tool_name = ? AND t.ts IS NULL AND t.failed IS NULL AND NOT t.structured_write
                    AND t.native_session = COALESCE(s.native_session, ''))
             FROM source_files s WHERE s.harness IN ('claude', 'pi', 'codex') ORDER BY s.source_id",
        )?;
        let rows = stmt.query_map(
            params![
                format!("{READER_NAMESPACE}{READER_VERSION}:"),
                READER_COMPLETE
            ],
            |r| {
                Ok(SourceCoverage {
                    source_id: r.get(0)?,
                    harness: r.get(1)?,
                    native_session: r.get(2)?,
                    complete: r.get(3)?,
                })
            },
        )?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn load_codex_observations(&self) -> Result<Vec<CodexObservation>> {
        let mut stmt = self.conn.prepare(
            "SELECT t.source_id, t.source_key, t.native_session, CAST(t.ts AS VARCHAR), t.seq,
                    t.tool_name, t.fingerprint, t.failed, t.generation
             FROM tool_facts t JOIN source_files s ON t.source_id = s.source_id AND t.generation = s.generation
             WHERE t.harness = 'codex' AND s.harness = 'codex'
                AND NOT starts_with(t.source_key, ?) AND t.tool_name IN (?, ?) AND NOT t.structured_write
             ORDER BY t.source_id, t.generation, t.seq",
        )?;
        let rows = stmt.query_map(params![READER_NAMESPACE, CODEX_COMMAND, CODEX_CALL], |r| {
            let ts: Option<String> = r.get(3)?;
            Ok(CodexObservation {
                source_id: r.get(0)?,
                fact: ToolFact {
                    source_key: r.get(1)?,
                    harness: "codex".into(),
                    native_session: r.get(2)?,
                    ts: ts.as_deref().and_then(parse_utc_time),
                    seq: r.get(4)?,
                    name: r.get(5)?,
                    fingerprint: r.get(6)?,
                    failed: r.get(7)?,
                    structured_write: false,
                    generation: r.get(8)?,
                },
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
        let mut facts = rows.collect::<Result<Vec<_>, _>>()?;
        let changed: std::collections::HashSet<_> = self
            .load_openrig_row_records()?
            .into_iter()
            .filter(|r| r.row_state == "changed")
            .map(|r| r.source_key)
            .collect();
        facts.retain(|fact| !changed.contains(&fact.source_key));
        Ok(facts)
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
    fn queue_reingest_backfills_empty_creation_missions_without_replacing_evidence() {
        let store = AnalyticsStore::open_in_memory("key").unwrap();
        let mut fact = QueueFact {
            source_key: "events:queue.created:1".into(),
            transition_id: 1,
            qitem_id: "parent".into(),
            ts: Utc::now(),
            state: "pending".into(),
            source_seat: "seat-a".into(),
            dest_seat: "seat-b".into(),
            closure_reason: None,
            closure_target: None,
            chain_id: None,
            missions: vec![],
        };
        store.insert_queue(&fact).unwrap();
        fact.missions = vec!["creation-mission".into()];
        store.insert_queue(&fact).unwrap();
        assert_eq!(
            store.load_queue().unwrap()[0].missions,
            ["creation-mission"]
        );
        fact.missions = vec!["replacement-mission".into()];
        store.insert_queue(&fact).unwrap();
        fact.missions.clear();
        store.insert_queue(&fact).unwrap();
        assert_eq!(
            store.load_queue().unwrap()[0].missions,
            ["creation-mission"]
        );
    }

    fn synthetic_metadata(size: &str) -> FileMetadata {
        FileMetadata {
            r#type: "regular".into(),
            device: "7".into(),
            inode: "11".into(),
            size: size.into(),
            mtime_ns: "-1".into(),
        }
    }

    fn baseline_row(store: &AnalyticsStore, generation: i64) -> Vec<Option<String>> {
        store
            .conn
            .query_row(
                "SELECT latest_association, latest_attempt_at, last_stable_observed_at,
                    last_stable_type, last_stable_device, last_stable_inode,
                    last_stable_size, last_stable_mtime_ns, association_reason
             FROM file_source_baselines WHERE source_id = ? AND generation = ?",
                params!["source", generation],
                |row| (0..9).map(|i| row.get(i)).collect(),
            )
            .unwrap()
    }

    #[test]
    fn baseline_association_tracks_noops_uncertainty_recovery_and_generations() {
        let store = AnalyticsStore::open_in_memory("key").unwrap();
        let commit = |generation, time: &str, metadata| {
            store
                .commit_file_batch_observed(
                    "source",
                    "pi",
                    "/synthetic/source.jsonl",
                    generation,
                    10,
                    "prefix",
                    None,
                    &[],
                    &[],
                    &[],
                    &ReaderState {
                        complete: true,
                        native_session: None,
                    },
                    &FileBaselineAttempt {
                        attempted_at: time.into(),
                        metadata,
                    },
                )
                .unwrap();
        };
        commit(1, "2026-10-15T00:00:00Z", Some(synthetic_metadata("30")));
        commit(1, "2026-10-15T00:00:01Z", Some(synthetic_metadata("30")));
        let stable = baseline_row(&store, 1);
        assert_eq!(stable[1].as_deref(), Some("2026-10-15T00:00:01Z"));
        assert_eq!(
            stable[2], stable[1],
            "a no-op is a fresh accepted association"
        );
        assert_eq!(
            stable[6].as_deref(),
            Some("30"),
            "full size exceeds watermark"
        );
        commit(1, "2026-10-15T00:00:02Z", None);
        let stale = baseline_row(&store, 1);
        assert_eq!(stale[0].as_deref(), Some("unavailable"));
        assert_eq!(stale[1].as_deref(), Some("2026-10-15T00:00:02Z"));
        assert_eq!(&stale[2..8], &stable[2..8]);
        assert_eq!(stale[8].as_deref(), Some("association_unverified"));
        commit(2, "2026-10-15T00:00:03Z", None);
        assert_eq!(
            baseline_row(&store, 1),
            stale,
            "a new generation cannot update the old one"
        );
        let first_uncertain = baseline_row(&store, 2);
        assert!(first_uncertain[2..8].iter().all(Option::is_none));
        commit(1, "2026-10-15T00:00:04Z", Some(synthetic_metadata("40")));
        let recovered = baseline_row(&store, 1);
        assert_eq!(recovered[0].as_deref(), Some("verified"));
        assert_eq!(recovered[6].as_deref(), Some("40"));
        assert_eq!(recovered[2].as_deref(), Some("2026-10-15T00:00:04Z"));
    }

    #[test]
    fn baseline_batch_failure_rolls_back_ddl_status_and_watermark() {
        let store = AnalyticsStore::open_in_memory("key").unwrap();
        let attempt = FileBaselineAttempt {
            attempted_at: "2026-10-15T00:00:00Z".into(),
            metadata: Some(synthetic_metadata("30")),
        };
        // Exercise old supported databases where the optional table is absent.
        store
            .conn
            .execute_batch("DROP TABLE file_source_baselines")
            .unwrap();
        for prior in [false, true] {
            if prior {
                store
                    .commit_file_batch_observed(
                        "source",
                        "pi",
                        "/synthetic/source.jsonl",
                        1,
                        10,
                        "old",
                        None,
                        &[],
                        &[],
                        &[],
                        &ReaderState {
                            complete: true,
                            native_session: None,
                        },
                        &attempt,
                    )
                    .unwrap();
            }
            let before = prior.then(|| baseline_row(&store, 1));
            let failed_attempt = FileBaselineAttempt {
                attempted_at: "2026-10-15T00:00:01Z".into(),
                metadata: None,
            };
            let failure = store.commit_file_batch_internal(
                "source",
                "pi",
                "/synthetic/source.jsonl",
                1,
                20,
                "new",
                None,
                &[],
                &[],
                &[],
                Some(&ReaderState {
                    complete: true,
                    native_session: None,
                }),
                Some(&failed_attempt),
                |stage| {
                    if stage == FileBatchStage::BeforeCommit {
                        Err(anyhow!("synthetic metadata batch fault"))
                    } else {
                        Ok(())
                    }
                },
            );
            assert!(failure.is_err());
            if let Some(before) = before {
                assert_eq!(baseline_row(&store, 1), before);
                assert_eq!(
                    store.source_state("source").unwrap(),
                    Some((1, 10, "old".into()))
                );
            } else {
                let tables: i64 = store
                    .conn
                    .query_row(
                        "SELECT COUNT(*) FROM information_schema.tables WHERE table_name = ?",
                        params!["file_source_baselines"],
                        |row| row.get(0),
                    )
                    .unwrap();
                assert_eq!(tables, 0, "uncommitted optional DDL must disappear");
                assert!(store.source_state("source").unwrap().is_none());
            }
        }
    }

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
        store.set_source_session(canary, canary).unwrap();
        store
            .replace_seat_bounds(&[SeatBound {
                seat: canary.into(),
                created_at: Some(now),
                stopped_at: None,
            }])
            .unwrap();
        store
            .begin_openrig_ingest(
                canary,
                canary,
                &[OpenRigRow {
                    table_name: canary.into(),
                    row_id: canary.into(),
                    row_digest: canary.into(),
                }],
            )
            .unwrap();
        store
            .qualify_openrig_fact(canary, canary, canary, canary)
            .unwrap();
        store
            .record_mutable_source_disclosure(canary, canary, canary)
            .unwrap();
        store
            .record_usage_models(&[(canary.into(), Some(canary.into()), Some(canary.into()))])
            .unwrap();
        store
            .record_tool_categories(&[(format!("tool-{canary}"), ToolCategory::from_label(canary))])
            .unwrap();
        store
            .record_queue_item_meta(&QueueItemMeta {
                qitem_id: canary.into(),
                is_recovery: true,
                recovery_reason: Some(canary.into()),
                last_nudge_result: Some(canary.into()),
                issue_tags: vec![canary.into()],
                phase_tags: vec![canary.into()],
                parent_qitem_id: Some(canary.into()),
            })
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
    fn failed_file_batch_rolls_back_and_retries_on_same_connection() {
        fn snapshot(store: &AnalyticsStore) -> serde_json::Value {
            let counters: i64 = store
                .conn
                .query_row("SELECT COUNT(*) FROM codex_counters", [], |row| row.get(0))
                .unwrap();
            serde_json::json!({
                "source": store.source_state("src").unwrap(),
                "usage": store.load_usage().unwrap(),
                "tools": store.load_tools().unwrap(),
                "counters": store.codex_counters("src", 1).unwrap(),
                "counter_rows": counters,
            })
        }

        for fail_during_settlement in [false, true] {
            let store = AnalyticsStore::open_in_memory("kid").unwrap();
            // Test-owned constraints fail after usage staging, or after one
            // settlement update. Production schema and code remain unchanged.
            store
                .conn
                .execute_batch(
                    "DROP TABLE tool_facts;
                 CREATE TABLE tool_facts (
                    source_key VARCHAR PRIMARY KEY,
                    harness VARCHAR NOT NULL,
                    native_session VARCHAR NOT NULL,
                    ts TIMESTAMP,
                    seq BIGINT NOT NULL,
                    tool_name VARCHAR NOT NULL CHECK (tool_name <> 'reject'),
                    fingerprint VARCHAR,
                    failed BOOLEAN CHECK (source_key <> 'old-b' OR failed IS NOT TRUE),
                    structured_write BOOLEAN NOT NULL,
                    generation BIGINT NOT NULL,
                    source_id VARCHAR NOT NULL,
                    category VARCHAR NOT NULL DEFAULT 'other'
                 )",
                )
                .unwrap();
            let ts = parse_utc_time("2026-10-08T12:34:56.123456Z").unwrap();
            let usage = UsageFact {
                source_key: "old-usage".into(),
                harness: "codex".into(),
                native_session: "s".into(),
                message_id: None,
                ts: None,
                seq: 1,
                input_uncached: 1,
                output: 2,
                cache_read: 3,
                cache_write: 4,
                reasoning: 5,
                currency_total: None,
                generation: 1,
                predecessor_source_key: None,
                predecessor_reason: None,
                prior_input_tokens: None,
                prior_cached_input_tokens: None,
                prior_cache_write_input_tokens: None,
                prior_output_tokens: None,
                prior_reasoning_output_tokens: None,
            };
            let tool = ToolFact {
                source_key: "old-a".into(),
                harness: "codex".into(),
                native_session: "s".into(),
                ts: None,
                seq: 1,
                name: "read".into(),
                fingerprint: None,
                failed: None,
                structured_write: false,
                generation: 1,
            };
            let old_tools = [
                tool.clone(),
                ToolFact {
                    source_key: "old-b".into(),
                    seq: 2,
                    ..tool.clone()
                },
            ];
            let old_counters = CodexCounters {
                input_tokens: 10,
                cached_input_tokens: 2,
                cache_write_input_tokens: 3,
                output_tokens: 4,
                reasoning_output_tokens: 5,
            };
            store
                .commit_file_batch_settling(
                    "src",
                    "codex",
                    "p",
                    1,
                    10,
                    "old-sha",
                    Some(&old_counters),
                    std::slice::from_ref(&usage),
                    &old_tools,
                    &[],
                )
                .unwrap();
            let before = snapshot(&store);
            let new_counters = CodexCounters {
                input_tokens: 20,
                ..old_counters.clone()
            };
            let new_usage = [
                UsageFact {
                    source_key: "new-null".into(),
                    generation: 2,
                    seq: 3,
                    ..usage.clone()
                },
                UsageFact {
                    source_key: "new-time".into(),
                    generation: 2,
                    seq: 4,
                    ts: Some(ts),
                    message_id: Some("m".into()),
                    currency_total: Some(0.5),
                    predecessor_source_key: Some("old-usage".into()),
                    predecessor_reason: Some("counter-reset".into()),
                    prior_input_tokens: Some(10),
                    prior_cached_input_tokens: Some(2),
                    prior_cache_write_input_tokens: Some(3),
                    prior_output_tokens: Some(4),
                    prior_reasoning_output_tokens: Some(5),
                    ..usage.clone()
                },
            ];
            let mut new_tools = [
                ToolFact {
                    source_key: "new-null".into(),
                    generation: 2,
                    seq: 3,
                    ..tool.clone()
                },
                ToolFact {
                    source_key: "new-time".into(),
                    generation: 2,
                    seq: 4,
                    ts: Some(ts),
                    fingerprint: Some("fingerprint".into()),
                    failed: Some(false),
                    structured_write: true,
                    ..tool.clone()
                },
            ];
            let mut settled = [("old-a".into(), Some(false)), ("old-b".into(), Some(false))];
            if fail_during_settlement {
                settled[1].1 = Some(true);
            } else {
                new_tools[1].name = "reject".into();
            }
            let err = store
                .commit_file_batch_settling(
                    "src",
                    "codex",
                    "p",
                    2,
                    20,
                    "new-sha",
                    Some(&new_counters),
                    &new_usage,
                    &new_tools,
                    &settled,
                )
                .unwrap_err();
            assert!(err.to_string().contains("CHECK constraint failed"), "{err}");
            assert_eq!(
                snapshot(&store),
                before,
                "failure must roll back every file fact"
            );
            let stages: i64 = store.conn.query_row(
                "SELECT COUNT(*) FROM information_schema.tables WHERE table_name IN ('usage_stage', 'tool_stage')",
                [], |row| row.get(0),
            ).unwrap();
            assert_eq!(stages, 0, "failed staging must not obstruct the retry");

            new_tools[1].name = "read".into();
            settled[1].1 = Some(false);
            store
                .commit_file_batch_settling(
                    "src",
                    "codex",
                    "p",
                    2,
                    20,
                    "new-sha",
                    Some(&new_counters),
                    &new_usage,
                    &new_tools,
                    &settled,
                )
                .unwrap();
            assert_eq!(
                store.source_state("src").unwrap(),
                Some((2, 20, "new-sha".into()))
            );
            assert_eq!(
                serde_json::to_value(store.codex_counters("src", 2).unwrap()).unwrap(),
                serde_json::to_value(&new_counters).unwrap()
            );
            let usages = store.load_usage().unwrap();
            let tools = store.load_tools().unwrap();
            assert_eq!(usages.len(), 3);
            assert_eq!(tools.len(), 4);
            for expected in &new_usage {
                let actual = usages
                    .iter()
                    .find(|row| row.source_key == expected.source_key)
                    .unwrap();
                assert_eq!(
                    serde_json::to_value(actual).unwrap(),
                    serde_json::to_value(expected).unwrap()
                );
            }
            for expected in &new_tools {
                let actual = tools
                    .iter()
                    .find(|row| row.source_key == expected.source_key)
                    .unwrap();
                assert_eq!(
                    serde_json::to_value(actual).unwrap(),
                    serde_json::to_value(expected).unwrap()
                );
            }
            assert!(tools
                .iter()
                .filter(|row| row.generation == 1)
                .all(|row| row.failed == Some(false)));
            let after = snapshot(&store);
            store
                .commit_file_batch_settling(
                    "src",
                    "codex",
                    "p",
                    2,
                    20,
                    "new-sha",
                    Some(&new_counters),
                    &new_usage,
                    &new_tools,
                    &settled,
                )
                .unwrap();
            assert_eq!(snapshot(&store), after, "duplicate retry must not add rows");
            // Empty usage/tool/settlement slices still commit the watermark.
            store
                .commit_file_batch_settling(
                    "empty",
                    "claude",
                    "empty-p",
                    1,
                    0,
                    "empty-sha",
                    None,
                    &[],
                    &[],
                    &[],
                )
                .unwrap();
            assert_eq!(
                store.source_state("empty").unwrap(),
                Some((1, 0, "empty-sha".into()))
            );
            assert_eq!(snapshot(&store), after);
        }
    }

    #[test]
    fn reader_batch_faults_roll_back_facts_state_and_session_then_retry() {
        fn snapshot(store: &AnalyticsStore) -> Vec<Vec<String>> {
            [
                "source_files",
                "usage_facts",
                "tool_facts",
                "codex_counters",
            ]
            .iter()
            .map(|table| {
                // Fixed test-owned table names, no external SQL input.
                let sql = format!("SELECT CAST(to_json(t) AS VARCHAR) FROM {table} t ORDER BY 1");
                let mut stmt = store.conn.prepare(&sql).unwrap();
                stmt.query_map([], |row| row.get(0))
                    .unwrap()
                    .collect::<Result<Vec<String>, _>>()
                    .unwrap()
            })
            .collect()
        }
        fn seed(store: &AnalyticsStore) {
            store
                .commit_file_batch_reading(
                    "src",
                    "codex",
                    "p",
                    1,
                    10,
                    &"a".repeat(64),
                    Some(&CodexCounters {
                        input_tokens: 9,
                        ..Default::default()
                    }),
                    &[],
                    &[],
                    &[],
                    &ReaderState {
                        complete: true,
                        native_session: Some("old-session".into()),
                    },
                )
                .unwrap();
        }
        let fact = ToolFact {
            source_key: "src:2:1:codex:0".into(),
            harness: "codex".into(),
            native_session: "new-session".into(),
            ts: None,
            seq: 1,
            name: CODEX_COMMAND.into(),
            fingerprint: Some("opaque".into()),
            failed: Some(true),
            structured_write: false,
            generation: 2,
        };
        let usage = UsageFact {
            source_key: "src:2:1".into(),
            harness: "codex".into(),
            native_session: "new-session".into(),
            message_id: None,
            ts: None,
            seq: 1,
            input_uncached: 2,
            output: 3,
            cache_read: 4,
            cache_write: 0,
            reasoning: 0,
            currency_total: None,
            generation: 2,
            predecessor_source_key: None,
            predecessor_reason: None,
            prior_input_tokens: None,
            prior_cached_input_tokens: None,
            prior_cache_write_input_tokens: None,
            prior_output_tokens: None,
            prior_reasoning_output_tokens: None,
        };
        let reader = ReaderState {
            complete: true,
            native_session: Some("new-session".into()),
        };
        let counters = CodexCounters {
            input_tokens: 20,
            ..Default::default()
        };
        let uninterrupted = AnalyticsStore::open_in_memory("kid").unwrap();
        seed(&uninterrupted);
        uninterrupted
            .commit_file_batch_reading(
                "src",
                "codex",
                "p",
                2,
                30,
                &"b".repeat(64),
                Some(&counters),
                std::slice::from_ref(&usage),
                std::slice::from_ref(&fact),
                &[],
                &reader,
            )
            .unwrap();
        for stage in [
            FileBatchStage::BeforeReaderReplacement,
            FileBatchStage::AfterReaderReplacement,
            FileBatchStage::BeforeSourceSessionUpdate,
            FileBatchStage::BeforeCommit,
        ] {
            let store = AnalyticsStore::open_in_memory("kid").unwrap();
            seed(&store);
            let before = snapshot(&store);
            let mut reached = false;
            let failed = store.commit_file_batch_internal(
                "src",
                "codex",
                "p",
                2,
                30,
                &"b".repeat(64),
                Some(&counters),
                std::slice::from_ref(&usage),
                std::slice::from_ref(&fact),
                &[],
                Some(&reader),
                None,
                |at| {
                    if at == stage {
                        reached = true;
                        Err(anyhow!("synthetic reader batch fault"))
                    } else {
                        Ok(())
                    }
                },
            );
            assert!(reached, "the real transaction must reach {stage:?}");
            assert!(failed
                .unwrap_err()
                .to_string()
                .contains("synthetic reader batch fault"));
            assert_eq!(snapshot(&store), before, "rollback at {stage:?}");
            assert!(store.load_source_coverage().unwrap()[0].complete);
            let stages: i64 = store.conn.query_row("SELECT COUNT(*) FROM information_schema.tables WHERE table_name IN ('usage_stage','tool_stage')", [], |r| r.get(0)).unwrap();
            assert_eq!(stages, 0);
            store
                .commit_file_batch_reading(
                    "src",
                    "codex",
                    "p",
                    2,
                    30,
                    &"b".repeat(64),
                    Some(&counters),
                    std::slice::from_ref(&usage),
                    std::slice::from_ref(&fact),
                    &[],
                    &reader,
                )
                .unwrap();
            assert_eq!(snapshot(&store), snapshot(&uninterrupted));
            assert_eq!(
                store.load_tools().unwrap().len(),
                1,
                "metadata is not a tool"
            );
            assert_eq!(store.load_codex_observations().unwrap().len(), 1);
            assert_eq!(
                store.load_sources().unwrap()[0].4.as_deref(),
                Some("new-session")
            );
            let after = snapshot(&store);
            store
                .commit_file_batch_reading(
                    "src",
                    "codex",
                    "p",
                    2,
                    30,
                    &"b".repeat(64),
                    Some(&counters),
                    std::slice::from_ref(&usage),
                    std::slice::from_ref(&fact),
                    &[],
                    &reader,
                )
                .unwrap();
            assert_eq!(snapshot(&store), after, "same prefix retry is idempotent");
        }
    }

    #[test]
    fn codex_same_line_item_positions_remain_distinct_and_ordered() {
        let store = AnalyticsStore::open_in_memory("synthetic-key-id").unwrap();
        // The frozen structural vector requires 105 positions on one line.
        // Storage carries their full keys and independent ordinals, never line*100+index.
        let observations: Vec<_> = (0..105)
            .map(|position| ToolFact {
                source_key: format!("source:1:1:codex:{position}"),
                harness: "codex".into(),
                native_session: "session".into(),
                ts: parse_utc_time("2026-10-14T00:00:00Z"),
                seq: position + 1,
                name: CODEX_COMMAND.into(),
                fingerprint: Some("b".repeat(64)),
                failed: Some(true),
                structured_write: false,
                generation: 1,
            })
            .collect();
        store
            .commit_file_batch_reading(
                "source",
                "codex",
                "p",
                1,
                1000,
                &"a".repeat(64),
                None,
                &[],
                &observations,
                &[],
                &ReaderState {
                    complete: true,
                    native_session: Some("session".into()),
                },
            )
            .unwrap();
        let rows = store.load_codex_observations().unwrap();
        assert_eq!(rows.len(), 105);
        assert_eq!(
            rows.iter().map(|row| row.fact.seq).collect::<Vec<_>>(),
            (1..=105).collect::<Vec<_>>()
        );
        assert_eq!(
            rows.iter()
                .map(|row| &row.fact.source_key)
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            105
        );
        assert_eq!(crate::metrics::codex_command_loops(&rows)[0].count, 105);
        store
            .commit_file_batch_reading(
                "source",
                "codex",
                "p",
                1,
                1000,
                &"a".repeat(64),
                None,
                &[],
                &observations,
                &[],
                &ReaderState {
                    complete: true,
                    native_session: Some("session".into()),
                },
            )
            .unwrap();
        assert_eq!(store.load_codex_observations().unwrap().len(), 105);
        assert_eq!(store.load_tools().unwrap().len(), 105, "marker excluded");
    }

    #[test]
    fn reader_state_fails_closed_for_snapshot_mismatch_and_never_becomes_a_tool() {
        let store = AnalyticsStore::open_in_memory("kid").unwrap();
        store
            .commit_file_batch_reading(
                "src",
                "codex",
                "p",
                1,
                0,
                &"a".repeat(64),
                None,
                &[],
                &[],
                &[],
                &ReaderState {
                    complete: true,
                    native_session: None,
                },
            )
            .unwrap();
        assert!(store.load_source_coverage().unwrap()[0].complete);
        assert!(store.load_tools().unwrap().is_empty());
        assert!(store.load_codex_observations().unwrap().is_empty());
        for sql in [
            "UPDATE tool_facts SET seq = 1",
            "UPDATE tool_facts SET seq = 0, fingerprint = 'wrong'",
            "UPDATE tool_facts SET fingerprint = 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', generation = 2",
            "UPDATE tool_facts SET generation = 1, tool_name = 'FutureReader'",
            "UPDATE tool_facts SET tool_name = 'DendroReaderComplete', structured_write = TRUE",
        ] {
            store.conn.execute_batch(sql).unwrap();
            assert!(!store.load_source_coverage().unwrap()[0].complete);
            assert!(
                store.load_tools().unwrap().is_empty(),
                "even malformed metadata stays private"
            );
        }
        // A native Claude tool name never selects the metadata namespace.
        store.conn.execute("INSERT INTO tool_facts VALUES (?, 'claude', 'c', NULL, 1, ?, NULL, NULL, FALSE, 1, 'src', 'other')",
            params!["actual-call", READER_COMPLETE]).unwrap();
        assert_eq!(store.load_tools().unwrap().len(), 1);
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
    #[test]
    fn read_snapshot_rolls_back_error_and_allows_the_next_transaction() {
        let store = AnalyticsStore::open_in_memory("synthetic-id").unwrap();
        let err = store
            .read_snapshot(|s| -> Result<()> {
                s.conn.execute(
                    "INSERT INTO source_files VALUES (?, ?, ?, ?, ?, ?, ?)",
                    params!["source", "claude", "synthetic", 1, 0, "a".repeat(64), "s"],
                )?;
                Err(anyhow!("synthetic evaluation error"))
            })
            .unwrap_err();
        assert_eq!(err.to_string(), "synthetic evaluation error");
        let count = store
            .read_snapshot(|s| {
                Ok(s.conn
                    .query_row("SELECT COUNT(*) FROM source_files", [], |r| {
                        r.get::<_, i64>(0)
                    })?)
            })
            .unwrap();
        assert_eq!(
            count, 0,
            "failed evaluation must roll back and release transaction"
        );
    }

    #[test]
    fn orphan_trace_preserves_actual_row_generation_and_latest_coverage_owner() {
        let dir = std::env::temp_dir().join(format!("dendro-trace-orphan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("analytics.duckdb");
        let store = AnalyticsStore::open_path(&db, "synthetic-id").unwrap();
        for (seq, ts) in [
            (0, "2026-10-15T00:00:00Z"),
            (1, "2026-10-15T00:00:01Z"),
            (2, "2026-10-15T00:00:02Z"),
        ] {
            store.conn.execute("INSERT INTO tool_facts VALUES (?, ?, ?, CAST(? AS TIMESTAMP), ?, ?, ?, ?, ?, ?, ?, 'inspect_read')",params![format!("unparsed~row{seq}"),"claude","s",ts,seq,"Read","a".repeat(64),true,false,42,"actual-source"]).unwrap();
        }
        store.checkpoint().unwrap();
        drop(store);
        let trace = crate::pipeline::trace_json(
            &db,
            "tokens",
            None,
            "2026-10-15T00:00:00Z",
            "2026-10-16T00:00:00Z",
        )
        .unwrap();
        let e = &trace["inputs"]["tool_loop_trace"];
        assert_eq!(e["status"], "complete");
        assert_eq!(e["observations"][0]["source_id"], "actual-source");
        assert_eq!(e["observations"][0]["source_key"], "unparsed~row0");
        assert_eq!(e["observations"][0]["generation"], 42);
        assert_eq!(e["sources"][0]["metadata_status"], "unavailable");
        assert_eq!(e["episodes"][0]["span_seconds"], "2");
        let store = AnalyticsStore::open_existing(&db, "synthetic-id").unwrap();
        store
            .conn
            .execute(
                "INSERT INTO source_files VALUES (?, ?, ?, ?, ?, ?, ?)",
                params![
                    "actual-source",
                    "claude",
                    "unavailable-synthetic-source",
                    43,
                    100,
                    "b".repeat(64),
                    "s"
                ],
            )
            .unwrap();
        store.checkpoint().unwrap();
        drop(store);
        let trace = crate::pipeline::trace_json(
            &db,
            "tokens",
            None,
            "2026-10-15T00:00:00Z",
            "2026-10-16T00:00:00Z",
        )
        .unwrap();
        let e = &trace["inputs"]["tool_loop_trace"];
        assert_eq!(e["status"], "complete");
        assert_eq!(e["sources"].as_array().unwrap().len(), 2);
        assert_eq!(e["sources"][0]["generation"], 42);
        assert!(e["sources"][0]["coverage"].is_null());
        assert_eq!(e["sources"][0]["latest_committed_prefix"]["generation"], 43);
        assert!(e["sources"][0]["producing_revision"].is_null());
        assert_eq!(e["sources"][1]["generation"], 43);
        assert!(!e["sources"][1]["coverage"].is_null());
        assert!(!std::path::PathBuf::from(format!("{}.key", db.display())).exists());
        // Losing one essential source identity invalidates the whole partition,
        // without changing the original count or bridging the missing member.
        let store = AnalyticsStore::open_existing(&db, "synthetic-id").unwrap();
        store
            .conn
            .execute(
                "UPDATE tool_facts SET source_id = '' WHERE source_key = ?",
                params!["unparsed~row1"],
            )
            .unwrap();
        store.checkpoint().unwrap();
        drop(store);
        let trace = crate::pipeline::trace_json(
            &db,
            "tokens",
            None,
            "2026-10-15T00:00:00Z",
            "2026-10-16T00:00:00Z",
        )
        .unwrap();
        let inputs = &trace["inputs"];
        let e = &inputs["tool_loop_trace"];
        assert_eq!(inputs["tool_loops"][0]["count"], 3);
        assert_eq!(e["status"], "insufficient_evidence");
        assert!(e["episodes"].as_array().unwrap().is_empty());
        assert!(e["observations"][1]["source_ref"].is_null());
        assert!(e["limitations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l["code"] == "essential_evidence_unavailable" && l["report_index"] == 0));
        // A zero result with lost evidence is also insufficient.
        let store = AnalyticsStore::open_existing(&db, "synthetic-id").unwrap();
        store
            .conn
            .execute("UPDATE tool_facts SET failed = ?", params![false])
            .unwrap();
        store.checkpoint().unwrap();
        drop(store);
        let trace = crate::pipeline::trace_json(
            &db,
            "tokens",
            None,
            "2026-10-15T00:00:00Z",
            "2026-10-16T00:00:00Z",
        )
        .unwrap();
        let e = &trace["inputs"]["tool_loop_trace"];
        assert_eq!(e["status"], "insufficient_evidence");
        assert!(trace["inputs"]["tool_loops"].as_array().unwrap().is_empty());
        assert!(e["limitations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l["code"] == "essential_evidence_unavailable"
                && l["observation_id"] == "o1"
                && l["report_index"].is_null()));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
