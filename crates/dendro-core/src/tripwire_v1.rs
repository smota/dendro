//! O(1) statement tripwire. The trigger only bumps a watermark and inserts one
//! outbox row. `I22` runs in this Rust dispatcher after commit.
use duckdb::{params, Connection};

pub const PAYLOAD_DIRTY_SESSION: &str = "dirty_session";

pub const WATERMARK_TRIGGER_BODY: &str = "\
UPDATE _dendro_tripwire_watermark SET counter = counter + 1 WHERE id = 1";

pub const NOTIFY_TRIGGER_BODY: &str = "\
INSERT INTO _dendro_tripwire_outbox (
    id, insight_id, subject_key, turn_k, watermark, payload_kind, created_at, dispatched_at
)
SELECT uuid()::VARCHAR, 'I22', w.subject_key, w.counter, w.watermark, 'dirty_session',
       current_timestamp, NULL
FROM _dendro_tripwire_watermark w
WHERE w.id = 1
  AND (SELECT max_turn_k FROM _dendro_tripwire_threshold WHERE id = 1) >= 0";

/// Records the per-session subject on `_dendro_watermarks`.
/// DuckDB rejects `UPDATE` in `FOR EACH ROW` triggers, so this is an insert.
/// Each invocation is one row and does not scan history.
pub const TRAJECTORY_WATERMARK_BODY: &str = "\
INSERT INTO _dendro_watermarks (subject_key, counter, watermark)
VALUES (
    COALESCE(NEW.subject_key, NEW.session_key, 'unscoped'),
    COALESCE(NEW.turn_k, 0),
    COALESCE(CAST(NEW.turn_k AS VARCHAR), '0')
)";

pub const TRAJECTORY_NOTIFY_BODY: &str = "\
INSERT INTO _dendro_tripwire_outbox (
    id, insight_id, subject_key, turn_k, watermark, payload_kind, created_at, dispatched_at
)
VALUES (
    uuid()::VARCHAR,
    'I22',
    COALESCE(NEW.subject_key, NEW.session_key, 'unscoped'),
    COALESCE(NEW.turn_k, 0),
    COALESCE(CAST(NEW.turn_k AS VARCHAR), '0'),
    'dirty_session',
    current_timestamp,
    NULL
)";

const ALLOWED_READS: &[&str] = &["_dendro_tripwire_watermark", "_dendro_tripwire_threshold"];

const HISTORICAL: &[&str] = &[
    "usage_facts",
    "tool_facts",
    "session_edges",
    "message_facts",
    "process_samples",
    "process_identities",
    "harness_trajectory",
    "harness_envelope",
    "security_facts",
    "queue_facts",
    "source_files",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TripwireError {
    TriggerRejected(&'static str),
    Duck(String),
    Analyzer(String),
}

impl std::fmt::Display for TripwireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TriggerRejected(reason) => write!(f, "trigger rejected: {reason}"),
            Self::Duck(text) | Self::Analyzer(text) => write!(f, "{text}"),
        }
    }
}

impl std::error::Error for TripwireError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxRow {
    pub id: String,
    pub insight_id: String,
    pub subject_key: String,
    pub turn_k: i64,
    pub watermark: String,
    pub payload_kind: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VortexAnswer {
    pub status: String,
    pub measures: Vec<(String, String)>,
    pub evidence: Vec<String>,
}

/// Operator pull advisory. `enters_agent_prompt` is always false.
#[derive(Debug, Clone, PartialEq)]
pub struct OperatorAdvisory {
    pub id: String,
    pub insight_id: String,
    pub channel: &'static str,
    pub enters_agent_prompt: bool,
    pub recovery: bool,
    pub measures: Vec<(String, String)>,
    pub evidence: Vec<String>,
}

pub fn render_agent_prompt(existing: &str, advisory: &OperatorAdvisory) -> String {
    let _ = advisory;
    existing.to_string()
}

/// Rejects trigger bodies that scan history, join, or compute a centroid or quantile.
pub fn validate_trigger_sql(sql: &str) -> Result<(), TripwireError> {
    let body = strip_comments(sql);
    if body.trim().is_empty() {
        return Err(TripwireError::TriggerRejected("empty"));
    }
    if body.contains(';') {
        return Err(TripwireError::TriggerRejected("multiple statements"));
    }
    let lower = body.to_ascii_lowercase();
    if contains_word(&lower, "join")
        || lower.contains("quantile")
        || lower.contains("centroid")
        || lower.contains("percentile")
        || lower.contains("markov")
    {
        return Err(TripwireError::TriggerRejected(
            "historical or aggregate work",
        ));
    }
    for table in HISTORICAL {
        if contains_word(&lower, table) {
            return Err(TripwireError::TriggerRejected("historical scan"));
        }
    }
    for target in from_targets(&lower) {
        if !ALLOWED_READS.contains(&target.as_str()) {
            return Err(TripwireError::TriggerRejected("unexpected read"));
        }
    }
    let updates_watermark =
        lower.contains("update _dendro_tripwire_watermark") && lower.contains("where id = 1");
    let inserts_outbox = lower.contains("insert into _dendro_tripwire_outbox");
    let inserts_session_watermark = lower.contains("insert into _dendro_watermarks");
    if !updates_watermark && !inserts_outbox && !inserts_session_watermark {
        return Err(TripwireError::TriggerRejected(
            "not a watermark or outbox write",
        ));
    }
    let records_new_row = lower.contains("new.subject_key") || lower.contains("new.session_key");
    if inserts_outbox && !lower.contains("from _dendro_tripwire_watermark") && !records_new_row {
        return Err(TripwireError::TriggerRejected(
            "outbox insert must read the watermark row or the inserted subject",
        ));
    }
    if inserts_session_watermark && !records_new_row {
        return Err(TripwireError::TriggerRejected(
            "watermark insert must record the inserted subject",
        ));
    }
    if lower.contains("from _dendro_tripwire_threshold") && !lower.contains("where id = 1") {
        return Err(TripwireError::TriggerRejected(
            "threshold read is not a single row",
        ));
    }
    Ok(())
}

pub fn ensure_tripwire_tables(conn: &Connection) -> Result<(), TripwireError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS _dendro_tripwire_outbox (
            id VARCHAR PRIMARY KEY,
            insight_id VARCHAR NOT NULL,
            subject_key VARCHAR NOT NULL,
            turn_k BIGINT NOT NULL,
            watermark VARCHAR NOT NULL,
            payload_kind VARCHAR NOT NULL,
            created_at TIMESTAMP NOT NULL,
            dispatched_at TIMESTAMP
        );
        CREATE TABLE IF NOT EXISTS _dendro_tripwire_watermark (
            id INTEGER PRIMARY KEY,
            counter BIGINT NOT NULL,
            watermark VARCHAR NOT NULL,
            subject_key VARCHAR NOT NULL
        );
        CREATE TABLE IF NOT EXISTS _dendro_tripwire_threshold (
            id INTEGER PRIMARY KEY,
            max_turn_k BIGINT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS _dendro_watermarks (
            subject_key VARCHAR NOT NULL,
            counter BIGINT NOT NULL,
            watermark VARCHAR NOT NULL
        );
        INSERT INTO _dendro_tripwire_watermark
        SELECT 1, 0, '0', 'unscoped'
        WHERE NOT EXISTS (SELECT 1 FROM _dendro_tripwire_watermark WHERE id = 1);
        INSERT INTO _dendro_tripwire_threshold
        SELECT 1, 8
        WHERE NOT EXISTS (SELECT 1 FROM _dendro_tripwire_threshold WHERE id = 1);",
    )
    .map_err(|error| TripwireError::Duck(error.to_string()))
}

pub fn install_usage_tripwire(conn: &Connection) -> Result<(), TripwireError> {
    ensure_tripwire_tables(conn)?;
    validate_trigger_sql(WATERMARK_TRIGGER_BODY)?;
    validate_trigger_sql(NOTIFY_TRIGGER_BODY)?;
    create_trigger(
        conn,
        "dendro_tripwire_mark",
        &format!("AFTER INSERT ON usage_facts FOR EACH STATEMENT {WATERMARK_TRIGGER_BODY}"),
    )?;
    create_trigger(
        conn,
        "dendro_tripwire_notify",
        &format!("AFTER INSERT ON usage_facts FOR EACH STATEMENT {NOTIFY_TRIGGER_BODY}"),
    )?;
    install_trajectory_tripwire(conn)?;
    Ok(())
}

/// Row trigger. Each invocation writes one watermark and one outbox row.
pub fn install_trajectory_tripwire(conn: &Connection) -> Result<(), TripwireError> {
    ensure_tripwire_tables(conn)?;
    validate_trigger_sql(TRAJECTORY_WATERMARK_BODY)?;
    validate_trigger_sql(TRAJECTORY_NOTIFY_BODY)?;
    create_trigger(
        conn,
        "dendro_trajectory_watermarks",
        &format!("AFTER INSERT ON harness_trajectory FOR EACH ROW {TRAJECTORY_WATERMARK_BODY}"),
    )?;
    create_trigger(
        conn,
        "dendro_trajectory_notify",
        &format!("AFTER INSERT ON harness_trajectory FOR EACH ROW {TRAJECTORY_NOTIFY_BODY}"),
    )?;
    Ok(())
}

fn create_trigger(conn: &Connection, name: &str, body: &str) -> Result<(), TripwireError> {
    let sql = format!("CREATE TRIGGER {name} {body}");
    match conn.execute_batch(&sql) {
        Ok(()) => Ok(()),
        Err(error) if error.to_string().to_ascii_lowercase().contains("already") => Ok(()),
        Err(error) => Err(TripwireError::Duck(error.to_string())),
    }
}

pub fn trajectory_vortex(
    conn: &Connection,
    subject_key: &str,
) -> Result<VortexAnswer, TripwireError> {
    evaluate_i22(conn, subject_key)
}

/// Canonical I22 (`k = 3`, cohort centroid cosine distance, frozen quantile)
/// scoped to the dirty outbox subject. `"unscoped"` reads every subject.
pub fn evaluate_i22(conn: &Connection, subject_key: &str) -> Result<VortexAnswer, TripwireError> {
    use crate::insight_v1::{evaluate_i22_scoped, InsightStatus, MeasureValue};

    let result = evaluate_i22_scoped(conn, subject_key);
    let status = match &result.status {
        InsightStatus::Computed => "computed".to_string(),
        InsightStatus::Partial { reason } => format!("partial:{reason}"),
        InsightStatus::Refused { reason } => format!("refused:{reason}"),
    };
    let measures = result
        .measures
        .iter()
        .map(|measure| {
            let value = match measure.value {
                Some(MeasureValue::Int(value)) => value.to_string(),
                Some(MeasureValue::Float(value)) => format!("{value:.12}"),
                None => "unknown".to_string(),
            };
            (measure.id.clone(), value)
        })
        .collect();
    let mut evidence = result.evidence;
    if evidence.is_empty() {
        evidence = observation_ids(conn, subject_key)?;
    }
    Ok(VortexAnswer {
        status,
        measures,
        evidence,
    })
}

fn observation_ids(conn: &Connection, subject_key: &str) -> Result<Vec<String>, TripwireError> {
    let sql = if subject_key == "unscoped" {
        "SELECT COALESCE(observation_id, session_key, 'obs') FROM harness_trajectory
         ORDER BY turn_k, observation_id"
    } else {
        "SELECT COALESCE(observation_id, session_key, 'obs') FROM harness_trajectory
         WHERE COALESCE(subject_key, session_key, 'unscoped') = ?
            OR session_key = ?
         ORDER BY turn_k, observation_id"
    };
    let mut stmt = conn
        .prepare(sql)
        .map_err(|error| TripwireError::Duck(error.to_string()))?;
    let map = |row: &duckdb::Row<'_>| row.get::<_, String>(0);
    let rows = if subject_key == "unscoped" {
        stmt.query_map([], map)
            .map_err(|error| TripwireError::Duck(error.to_string()))?
    } else {
        stmt.query_map(params![subject_key, subject_key], map)
            .map_err(|error| TripwireError::Duck(error.to_string()))?
    };
    let mut ids = Vec::new();
    for row in rows {
        ids.push(row.map_err(|error| TripwireError::Duck(error.to_string()))?);
    }
    Ok(ids)
}

/// Reads undispatched outbox rows and runs canonical `I22` after commit.
pub fn dispatch_pending_tripwires(
    conn: &Connection,
) -> Result<Vec<OperatorAdvisory>, TripwireError> {
    dispatch_after_commit(conn)
}

/// Reads undispatched outbox rows and runs `I22` outside the fact transaction.
pub fn dispatch_after_commit(conn: &Connection) -> Result<Vec<OperatorAdvisory>, TripwireError> {
    dispatch_with(conn, false, |conn, row| {
        evaluate_i22(conn, &row.subject_key)
    })
}

/// Recovery read of the same outbox. It does not scan fact tables.
pub fn recover_undispatched(conn: &Connection) -> Result<Vec<OperatorAdvisory>, TripwireError> {
    dispatch_with(conn, true, |conn, row| evaluate_i22(conn, &row.subject_key))
}

pub fn dispatch_with<F>(
    conn: &Connection,
    recovery: bool,
    analyzer: F,
) -> Result<Vec<OperatorAdvisory>, TripwireError>
where
    F: Fn(&Connection, &OutboxRow) -> Result<VortexAnswer, TripwireError>,
{
    let pending = undispatched(conn)?;
    let mut notices = Vec::new();
    for row in pending {
        if row.payload_kind != PAYLOAD_DIRTY_SESSION || row.insight_id != "I22" {
            return Err(TripwireError::Analyzer(
                "outbox payload is not the closed dirty-session notification".into(),
            ));
        }
        let answer = analyzer(conn, &row)?;
        let updated = conn
            .execute(
                "UPDATE _dendro_tripwire_outbox
                 SET dispatched_at = current_timestamp
                 WHERE id = ? AND dispatched_at IS NULL",
                params![row.id],
            )
            .map_err(|error| TripwireError::Duck(error.to_string()))?;
        if updated == 0 {
            continue;
        }
        notices.push(OperatorAdvisory {
            id: row.id,
            insight_id: "I22".to_string(),
            channel: "operator-pull",
            enters_agent_prompt: false,
            recovery,
            measures: answer.measures,
            evidence: answer.evidence,
        });
    }
    Ok(notices)
}

pub fn undispatched(conn: &Connection) -> Result<Vec<OutboxRow>, TripwireError> {
    let mut stmt = conn
        .prepare(
            "SELECT id, insight_id, subject_key, turn_k, watermark, payload_kind
             FROM _dendro_tripwire_outbox
             WHERE dispatched_at IS NULL
             ORDER BY created_at, id",
        )
        .map_err(|error| TripwireError::Duck(error.to_string()))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(OutboxRow {
                id: row.get(0)?,
                insight_id: row.get(1)?,
                subject_key: row.get(2)?,
                turn_k: row.get(3)?,
                watermark: row.get(4)?,
                payload_kind: row.get(5)?,
            })
        })
        .map_err(|error| TripwireError::Duck(error.to_string()))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|error| TripwireError::Duck(error.to_string()))?);
    }
    Ok(out)
}

fn strip_comments(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len());
    let chars: Vec<char> = sql.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '-' && chars.get(index + 1) == Some(&'-') {
            index += 2;
            while index < chars.len() && chars[index] != '\n' {
                index += 1;
            }
            continue;
        }
        out.push(chars[index]);
        index += 1;
    }
    out
}

fn contains_word(haystack: &str, word: &str) -> bool {
    haystack
        .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
        .any(|part| part == word)
}

fn from_targets(lower: &str) -> Vec<String> {
    let mut targets = Vec::new();
    let bytes = lower.as_bytes();
    let mut rest = lower;
    while let Some(found) = rest.find("from ") {
        let after = &rest[found + 5..];
        let name: String = after
            .chars()
            .skip_while(|ch| ch.is_whitespace())
            .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
            .collect();
        if !name.is_empty() {
            targets.push(name);
        }
        let next = found + 5;
        if next >= bytes.len() {
            break;
        }
        rest = &rest[next..];
    }
    targets
}
