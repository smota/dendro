use anyhow::Result;
use async_trait::async_trait;
use dendro_core::{RuntimeConnector, RuntimeSnapshot};
use std::path::PathBuf;

pub struct OpenRigConnector;

impl OpenRigConnector {
    pub fn new() -> Self {
        Self
    }

    pub fn db_path() -> PathBuf {
        let home = std::env::var("OPENRIG_HOME")
            .or_else(|_| std::env::var("HOME"))
            .unwrap_or_else(|_| ".".into());
        PathBuf::from(home).join(".openrig").join("openrig.sqlite")
    }
}

impl Default for OpenRigConnector {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl RuntimeConnector for OpenRigConnector {
    fn name(&self) -> &'static str {
        "openrig"
    }

    fn extract_topology(&self) -> Result<RuntimeSnapshot> {
        let db_path = Self::db_path();
        let mut rigs = vec![];
        if db_path.exists() {
            if let Ok(conn) = rusqlite::Connection::open_with_flags(
                &db_path,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            ) {
                if let Ok(mut stmt) = conn.prepare("SELECT name FROM rigs") {
                    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
                    for row in rows.flatten() {
                        rigs.push(row);
                    }
                }
            }
        }
        Ok(RuntimeSnapshot {
            runtime_name: "openrig".to_string(),
            active_rigs: rigs,
        })
    }
}

use anyhow::{bail, Context};
use dendro_core::{OpenRigCutoff, OpenRigRow, QueueItemCutoff};
use rusqlite::{params, types::ValueRef, Connection};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub struct MutableDisclosure {
    pub qitem_id: String,
    pub seat: String,
    pub reason: String,
}

pub struct PreparedOpenRig {
    pub cutoff: OpenRigCutoff,
    pub rows: Vec<OpenRigRow>,
    pub disclosures: Vec<MutableDisclosure>,
}

struct TablePrefix {
    max: Option<i64>,
    count: u64,
    digest: String,
    rows: Vec<OpenRigRow>,
}

/// Hash typed, length-delimited SQLite values, including every source column.
/// This avoids JSON formatting ambiguity and detects changes to unused columns too.
fn table_prefix(conn: &Connection, table: &str, limit: Option<Option<i64>>) -> Result<TablePrefix> {
    let sql = match table {
        "events" => "SELECT seq, * FROM events WHERE (?1 IS NULL OR seq <= ?1) ORDER BY seq",
        "queue_transitions" => "SELECT transition_id, * FROM queue_transitions WHERE (?1 IS NULL OR transition_id <= ?1) ORDER BY transition_id",
        "snapshots" => "SELECT rowid, * FROM snapshots WHERE (?1 IS NULL OR rowid <= ?1) ORDER BY rowid",
        "rigs" => "SELECT rowid, * FROM rigs ORDER BY rowid",
        _ => bail!("unsupported OpenRig table"),
    };
    let mut total = Sha256::new();
    let mut result = TablePrefix {
        max: None,
        count: 0,
        digest: String::new(),
        rows: Vec::new(),
    };
    if limit == Some(None) {
        result.digest = hex::encode(total.finalize());
        return Ok(result);
    }
    let Ok(mut stmt) = conn.prepare(sql) else {
        result.digest = hex::encode(total.finalize());
        return Ok(result);
    };
    let columns = stmt.column_count();
    let mut rows = if table == "rigs" {
        stmt.query([])?
    } else {
        stmt.query(params![limit.flatten()])?
    };
    while let Some(row) = rows.next()? {
        dendro_core::RUN_CANCEL.check()?;
        let id: i64 = row.get(0)?;
        let mut hash = Sha256::new();
        for index in 0..columns {
            match row.get_ref(index)? {
                ValueRef::Null => hash.update([0]),
                ValueRef::Integer(value) => {
                    hash.update([1]);
                    hash.update(value.to_le_bytes());
                }
                ValueRef::Real(value) => {
                    hash.update([2]);
                    hash.update(value.to_bits().to_le_bytes());
                }
                ValueRef::Text(value) | ValueRef::Blob(value) => {
                    hash.update([if matches!(row.get_ref(index)?, ValueRef::Text(_)) {
                        3
                    } else {
                        4
                    }]);
                    hash.update((value.len() as u64).to_le_bytes());
                    hash.update(value);
                }
            }
        }
        let bytes = hash.finalize();
        total.update(bytes);
        result.rows.push(OpenRigRow {
            table_name: table.into(),
            row_id: id.to_string(),
            row_digest: hex::encode(bytes),
        });
        result.max = Some(id);
        result.count += 1;
    }
    result.digest = hex::encode(total.finalize());
    Ok(result)
}

fn field_digest(value: Option<&str>) -> String {
    let mut hash = Sha256::new();
    match value {
        Some(value) => {
            hash.update([1]);
            hash.update(value.as_bytes());
        }
        None => hash.update([0]),
    }
    hex::encode(hash.finalize())
}

#[derive(Clone)]
struct MutableItem {
    id: String,
    fields: [Option<String>; 5],
}

fn mutable_items(conn: &Connection) -> Result<Vec<MutableItem>> {
    let Ok(mut stmt) = conn.prepare("SELECT * FROM queue_items ORDER BY qitem_id") else {
        return Ok(Vec::new());
    };
    let names = stmt
        .column_names()
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let position = |name: &str| names.iter().position(|column| column == name);
    let Some(id_index) = position("qitem_id") else {
        return Ok(Vec::new());
    };
    let indices = [
        "destination_session",
        "ts_updated",
        "chain_of_record",
        "handed_off_from",
        "tags",
    ]
    .map(position);
    let rows = stmt.query_map([], |row| {
        let mut fields: [Option<String>; 5] = Default::default();
        for (index, column) in indices.iter().enumerate() {
            if let Some(column) = column {
                fields[index] = row.get(*column)?;
            }
        }
        Ok(MutableItem {
            id: row.get(id_index)?,
            fields,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

fn item_cutoff(item: &MutableItem, key: &[u8]) -> QueueItemCutoff {
    QueueItemCutoff {
        qitem_id: dendro_core::redact(&dendro_core::seal_identifier(key, "qitem", &item.id)),
        destination_session_digest: field_digest(item.fields[0].as_deref()),
        ts_updated_digest: field_digest(item.fields[1].as_deref()),
        chain_of_record_digest: field_digest(item.fields[2].as_deref()),
        handed_off_from_digest: field_digest(item.fields[3].as_deref()),
        tags_digest: field_digest(item.fields[4].as_deref()),
        mission_tags_digest: {
            let mut tags: Vec<String> = item.fields[4]
                .as_deref()
                .and_then(|v| serde_json::from_str::<Vec<String>>(v).ok())
                .unwrap_or_default()
                .into_iter()
                .filter(|tag| tag.starts_with("mission:"))
                .collect();
            tags.sort();
            tags.dedup();
            field_digest(Some(
                &serde_json::to_string(&tags).expect("string array serializes"),
            ))
        },
    }
}

pub(crate) fn created_items(conn: &Connection) -> Result<BTreeMap<String, serde_json::Value>> {
    let mut items = BTreeMap::new();
    let Ok(mut stmt) =
        conn.prepare("SELECT payload FROM events WHERE type = 'queue.created' ORDER BY seq")
    else {
        return Ok(items);
    };
    for payload in stmt.query_map([], |row| row.get::<_, String>(0))? {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&payload?) else {
            continue;
        };
        if let Some(id) = value.get("qitemId").and_then(|value| value.as_str()) {
            items.entry(id.to_string()).or_insert(value);
        }
    }
    Ok(items)
}

/// Items with a transition before any timed destination record still depend
/// on the mutable fallback. Later creation events cannot establish that past.
fn fallback_dependent_items(conn: &Connection) -> Result<BTreeSet<String>> {
    let mut first_destination = BTreeMap::new();
    let mut events = conn.prepare("SELECT type, payload, created_at FROM events WHERE type IN ('queue.created', 'qitem.fallback_routed')")?;
    let rows = events.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    for row in rows {
        let (kind, payload, created_at) = row?;
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&payload) else {
            continue;
        };
        let Some(item) = value.get("qitemId").and_then(|value| value.as_str()) else {
            continue;
        };
        let destination = if kind == "queue.created" {
            "destinationSession"
        } else {
            "rerouteDestination"
        };
        if value
            .get(destination)
            .and_then(|value| value.as_str())
            .is_none()
        {
            continue;
        }
        let at = value
            .get("claimedAt")
            .and_then(|value| value.as_str())
            .and_then(dendro_core::parse_utc_time)
            .or_else(|| dendro_core::parse_utc_time(&created_at));
        if let Some(at) = at {
            first_destination
                .entry(item.to_string())
                .and_modify(|first: &mut chrono::DateTime<chrono::Utc>| *first = (*first).min(at))
                .or_insert(at);
        }
    }
    let mut dependent = BTreeSet::new();
    if let Ok(mut transitions) = conn.prepare("SELECT qitem_id, ts, state FROM queue_transitions") {
        let rows = transitions.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        for row in rows {
            let (item, ts, state) = row?;
            if state == "pending" {
                continue;
            }
            if let Some(at) = dendro_core::parse_utc_time(&ts) {
                if first_destination.get(&item).is_none_or(|first| *first > at) {
                    dependent.insert(item);
                }
            }
        }
    }
    Ok(dependent)
}

/// Capture and validate the coherent private SQLite copy, then bound that copy.
/// No SQLite connection is made to the live producer's database or its SHM file.
pub fn prepare_openrig_snapshot(
    path: &Path,
    source_id: &str,
    key: &[u8],
    sealed: Option<&OpenRigCutoff>,
) -> Result<PreparedOpenRig> {
    let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)
        .context("private OpenRig snapshot could not be opened")?;
    let events = table_prefix(&conn, "events", sealed.map(|cutoff| cutoff.events_max_seq))?;
    let transitions = table_prefix(
        &conn,
        "queue_transitions",
        sealed.map(|cutoff| cutoff.queue_transitions_max_id),
    )?;
    let snapshots = table_prefix(
        &conn,
        "snapshots",
        sealed.map(|cutoff| cutoff.snapshots_max_rowid),
    )?;
    let rigs = table_prefix(&conn, "rigs", None)?;
    let items = mutable_items(&conn)?;
    let cutoff = OpenRigCutoff {
        source_id: dendro_core::redact(source_id),
        events_max_seq: events.max,
        events_count: events.count,
        events_digest: events.digest,
        queue_transitions_max_id: transitions.max,
        queue_transitions_count: transitions.count,
        queue_transitions_digest: transitions.digest,
        snapshots_max_rowid: snapshots.max,
        snapshots_count: snapshots.count,
        snapshots_digest: snapshots.digest,
        queue_items: items.iter().map(|item| item_cutoff(item, key)).collect(),
        rigs_count: rigs.count,
        rigs_digest: rigs.digest,
    };
    let mut disclosures = Vec::new();
    if let Some(expected) = sealed {
        for (table, count, digest, expected_count, expected_digest) in [
            (
                "events",
                cutoff.events_count,
                &cutoff.events_digest,
                expected.events_count,
                &expected.events_digest,
            ),
            (
                "queue_transitions",
                cutoff.queue_transitions_count,
                &cutoff.queue_transitions_digest,
                expected.queue_transitions_count,
                &expected.queue_transitions_digest,
            ),
            (
                "snapshots",
                cutoff.snapshots_count,
                &cutoff.snapshots_digest,
                expected.snapshots_count,
                &expected.snapshots_digest,
            ),
        ] {
            if count != expected_count {
                bail!(
                    "mismatch: source {} {table} prefix row count changed",
                    dendro_core::redact(source_id)
                );
            }
            if digest != expected_digest {
                bail!(
                    "mismatch: source {} {table} prefix digest changed",
                    dendro_core::redact(source_id)
                );
            }
        }
        // SQL text is fixed; values are always bound. These mutations affect only
        // the private stage copy after all immutable evidence has been verified.
        for (sql, max) in [
            (
                "DELETE FROM events WHERE ?1 IS NULL OR seq > ?1",
                expected.events_max_seq,
            ),
            (
                "DELETE FROM queue_transitions WHERE ?1 IS NULL OR transition_id > ?1",
                expected.queue_transitions_max_id,
            ),
            (
                "DELETE FROM snapshots WHERE ?1 IS NULL OR rowid > ?1",
                expected.snapshots_max_rowid,
            ),
        ] {
            if let Err(error) = conn.execute(sql, params![max]) {
                if !error.to_string().contains("no such table") {
                    return Err(error.into());
                }
            }
        }
        let created = created_items(&conn)?;
        let fallback_dependent = fallback_dependent_items(&conn)?;
        let expected_items = expected
            .queue_items
            .iter()
            .map(|item| (item.qitem_id.as_str(), item))
            .collect::<BTreeMap<_, _>>();
        let mut seen = BTreeSet::new();
        for item in items {
            let observed = item_cutoff(&item, key);
            let Some(original) = expected_items.get(observed.qitem_id.as_str()) else {
                conn.execute(
                    "DELETE FROM queue_items WHERE qitem_id = ?1",
                    params![item.id],
                )?;
                continue;
            };
            seen.insert(observed.qitem_id.clone());
            if &observed == *original {
                continue;
            }
            let evidence = created.get(&item.id);
            let event_string = |name: &str| {
                evidence
                    .and_then(|value| value.get(name))
                    .and_then(|value| value.as_str())
            };
            let mut reasons = Vec::new();
            let mut unbacked_destination = false;
            for (index, name, field, changed) in [
                (
                    0,
                    "destinationSession",
                    "destination_session",
                    observed.destination_session_digest != original.destination_session_digest,
                ),
                (
                    2,
                    "chainOfRecord",
                    "chain_of_record",
                    observed.chain_of_record_digest != original.chain_of_record_digest,
                ),
                (
                    3,
                    "handedOffFrom",
                    "handed_off_from",
                    observed.handed_off_from_digest != original.handed_off_from_digest,
                ),
                (
                    4,
                    "tags",
                    "tags",
                    observed.tags_digest != original.tags_digest
                        && (original.mission_tags_digest.is_empty()
                            || observed.mission_tags_digest != original.mission_tags_digest),
                ),
            ] {
                if !changed {
                    continue;
                }
                let value = evidence.and_then(|value| value.get(name)).map(|value| {
                    if value.is_null() {
                        None
                    } else if let Some(text) = value.as_str() {
                        Some(text.to_string())
                    } else {
                        Some(value.to_string())
                    }
                });
                if let Some(value) = value {
                    // A creation event may follow an earlier transition. Its
                    // destination cannot establish the mutable fallback that was
                    // used before that event, unless it matches the sealed value.
                    if index == 0
                        && field_digest(value.as_deref()) != original.destination_session_digest
                        && fallback_dependent.contains(&item.id)
                    {
                        unbacked_destination = true;
                        reasons.push(
                            "destination_session changed without backing for the sealed fallback"
                                .to_string(),
                        );
                        continue;
                    }
                    let sql = match index {
                        0 => "UPDATE queue_items SET destination_session = ?1 WHERE qitem_id = ?2",
                        2 => "UPDATE queue_items SET chain_of_record = ?1 WHERE qitem_id = ?2",
                        3 => "UPDATE queue_items SET handed_off_from = ?1 WHERE qitem_id = ?2",
                        _ => "UPDATE queue_items SET tags = ?1 WHERE qitem_id = ?2",
                    };
                    conn.execute(sql, params![value, item.id])?;
                } else {
                    reasons.push(format!("{field} changed without queue.created backing"));
                }
            }
            if observed.ts_updated_digest != original.ts_updated_digest
                && event_string("destinationSession").is_none()
                && fallback_dependent.contains(&item.id)
            {
                reasons.push(
                    "ts_updated changed without queue.created destination backing".to_string(),
                );
            }
            if !reasons.is_empty() {
                // A changed fallback can move the item away from its original
                // seat. Its old destination cannot be recovered from a digest;
                // disclose globally rather than attributing the warning only to
                // the new seat and silently losing the old seat's denominator.
                let fallback_changed = observed.destination_session_digest
                    != original.destination_session_digest
                    || observed.ts_updated_digest != original.ts_updated_digest;
                let destination = (!unbacked_destination)
                    .then(|| event_string("destinationSession"))
                    .flatten()
                    .or_else(|| {
                        (!fallback_changed && !unbacked_destination)
                            .then_some(item.fields[0].as_deref())
                            .flatten()
                    })
                    .unwrap_or("unknown");
                disclosures.push(MutableDisclosure {
                    qitem_id: observed.qitem_id,
                    seat: dendro_core::seal_identifier(key, "seat", destination),
                    reason: format!("not reproducible: mutable source: {}", reasons.join("; ")),
                });
            }
        }
        for item in &expected.queue_items {
            if !seen.contains(&item.qitem_id) {
                disclosures.push(MutableDisclosure {
                    qitem_id: item.qitem_id.clone(),
                    seat: "unknown".into(),
                    reason: "not reproducible: mutable source: queue_items row disappeared".into(),
                });
            }
        }
        if cutoff.rigs_count != expected.rigs_count || cutoff.rigs_digest != expected.rigs_digest {
            disclosures.push(MutableDisclosure {
                qitem_id: "unknown".into(),
                seat: "unknown".into(),
                reason: "not reproducible: mutable source: rigs changed".into(),
            });
        }
    }
    let mut rows: Vec<OpenRigRow> = events
        .rows
        .into_iter()
        .chain(transitions.rows)
        .chain(snapshots.rows)
        .collect();
    for item in sealed
        .map(|entry| &entry.queue_items)
        .unwrap_or(&cutoff.queue_items)
    {
        rows.push(OpenRigRow {
            table_name: "queue_items".into(),
            row_id: item.qitem_id.clone(),
            row_digest: hex::encode(Sha256::digest(serde_json::to_vec(item)?)),
        });
    }
    Ok(PreparedOpenRig {
        cutoff: sealed.cloned().unwrap_or(cutoff),
        rows,
        disclosures,
    })
}

#[cfg(test)]
mod cutoff_tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "dendro-cutoff-{}-{}.sqlite",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("CREATE TABLE events(seq INTEGER PRIMARY KEY,type TEXT,payload TEXT,created_at TEXT); CREATE TABLE queue_transitions(transition_id INTEGER PRIMARY KEY,qitem_id TEXT,state TEXT); CREATE TABLE snapshots(data TEXT); CREATE TABLE rigs(id TEXT,name TEXT); CREATE TABLE queue_items(qitem_id TEXT PRIMARY KEY,destination_session TEXT,ts_updated TEXT,chain_of_record TEXT,handed_off_from TEXT,tags TEXT);").unwrap();
            conn.execute("INSERT INTO events VALUES(1,'queue.created',?1,'2026-01-01T00:00:00Z')",params![json!({"qitemId":"q1","destinationSession":"seat-a","chainOfRecord":[],"handedOffFrom":null,"tags":["mission:m1"]}).to_string()]).unwrap();
            conn.execute_batch("INSERT INTO queue_transitions VALUES(1,'q1','closed'); INSERT INTO snapshots VALUES('{}'); INSERT INTO rigs VALUES('r1','rig'); INSERT INTO queue_items VALUES('q1','seat-a','2026-01-01T00:00:00Z','[]',NULL,'[\"mission:m1\"]');").unwrap();
            Self(path)
        }
        fn capture(&self) -> OpenRigCutoff {
            prepare_openrig_snapshot(&self.0, "source-1", b"key", None)
                .unwrap()
                .cutoff
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[test]
    fn sqlite_cutoff_excludes_growth_and_new_mutable_items() {
        let fixture = Fixture::new();
        let cutoff = fixture.capture();
        let conn = Connection::open(&fixture.0).unwrap();
        conn.execute_batch("INSERT INTO events VALUES(2,'queue.created','{}','2026-01-02T00:00:00Z'); INSERT INTO queue_transitions VALUES(2,'q2','closed'); INSERT INTO snapshots VALUES('{}'); INSERT INTO queue_items VALUES('q2','seat-b',NULL,NULL,NULL,NULL);").unwrap();
        drop(conn);
        let result =
            prepare_openrig_snapshot(&fixture.0, "source-1", b"key", Some(&cutoff)).unwrap();
        assert!(result.disclosures.is_empty());
        assert_eq!(result.cutoff, cutoff);
        let conn = Connection::open(&fixture.0).unwrap();
        for table in ["events", "queue_transitions", "snapshots", "queue_items"] {
            let sql = match table {
                "events" => "SELECT COUNT(*) FROM events",
                "queue_transitions" => "SELECT COUNT(*) FROM queue_transitions",
                "snapshots" => "SELECT COUNT(*) FROM snapshots",
                _ => "SELECT COUNT(*) FROM queue_items",
            };
            assert_eq!(conn.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        }
    }

    #[test]
    fn sqlite_cutoff_refuses_deleted_and_rewritten_rows_in_every_immutable_table() {
        for sql in [
            "DELETE FROM events WHERE seq=1",
            "UPDATE events SET payload='changed' WHERE seq=1",
            "DELETE FROM queue_transitions WHERE transition_id=1",
            "UPDATE queue_transitions SET state='pending' WHERE transition_id=1",
            "DELETE FROM snapshots",
            "UPDATE snapshots SET data='changed'",
        ] {
            let fixture = Fixture::new();
            let cutoff = fixture.capture();
            Connection::open(&fixture.0)
                .unwrap()
                .execute_batch(sql)
                .unwrap();
            let error = prepare_openrig_snapshot(&fixture.0, "source-1", b"key", Some(&cutoff))
                .err()
                .unwrap()
                .to_string();
            assert!(error.contains("mismatch: source source-1"), "{error}");
        }
    }

    #[test]
    fn mutable_item_is_reconstructed_from_creation_payload() {
        let fixture = Fixture::new();
        let cutoff = fixture.capture();
        Connection::open(&fixture.0).unwrap().execute_batch("UPDATE queue_items SET destination_session='seat-b',tags='[\"mission:m2\"]',chain_of_record='[\"other\"]',handed_off_from='other'").unwrap();
        let prepared =
            prepare_openrig_snapshot(&fixture.0, "source-1", b"key", Some(&cutoff)).unwrap();
        assert!(prepared.disclosures.is_empty());
        let conn = Connection::open(&fixture.0).unwrap();
        let destination: String = conn
            .query_row("SELECT destination_session FROM queue_items", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(destination, "seat-a");
    }

    #[test]
    fn mutable_item_without_creation_backing_is_disclosed() {
        let fixture = Fixture::new();
        Connection::open(&fixture.0)
            .unwrap()
            .execute_batch("DELETE FROM events")
            .unwrap();
        let cutoff = fixture.capture();
        Connection::open(&fixture.0)
            .unwrap()
            .execute_batch("UPDATE queue_items SET destination_session='seat-b'")
            .unwrap();
        let prepared =
            prepare_openrig_snapshot(&fixture.0, "source-1", b"key", Some(&cutoff)).unwrap();
        assert_eq!(prepared.disclosures.len(), 1);
        assert!(prepared.disclosures[0]
            .reason
            .contains("not reproducible: mutable source"));
    }

    #[test]
    fn non_mission_tag_append_does_not_disclose_an_unchanged_metric() {
        let fixture = Fixture::new();
        let conn = Connection::open(&fixture.0).unwrap();
        conn.execute(
            "UPDATE events SET payload = ?1",
            params![json!({"qitemId":"q1", "destinationSession":"seat-a"}).to_string()],
        )
        .unwrap();
        drop(conn);
        let cutoff = fixture.capture();
        Connection::open(&fixture.0)
            .unwrap()
            .execute(
                "UPDATE queue_items SET tags = ?1",
                params![json!(["mission:m1", "prompt-episode:later"]).to_string()],
            )
            .unwrap();
        let prepared =
            prepare_openrig_snapshot(&fixture.0, "source-1", b"key", Some(&cutoff)).unwrap();
        assert!(prepared.disclosures.is_empty());
    }

    #[test]
    fn manifest_contains_only_redacted_ids_and_hex_digests() {
        let fixture = Fixture::new();
        Connection::open(&fixture.0)
            .unwrap()
            .execute(
                "UPDATE queue_items SET qitem_id=?1",
                params!["ghp_manifestcanary123456789012345678901234567890"],
            )
            .unwrap();
        let cutoff = prepare_openrig_snapshot(
            &fixture.0,
            "ghp_sourcecanary123456789012345678901234567890",
            b"key",
            None,
        )
        .unwrap()
        .cutoff;
        let serialized = serde_json::to_string(&cutoff).unwrap();
        assert!(!serialized.contains("manifestcanary"));
        assert!(!serialized.contains("sourcecanary"));
        assert!(!serialized.contains("destinationSession"));
        assert!(!serialized.contains("seat-a"));
        assert_eq!(cutoff.events_digest.len(), 64);
    }
}
