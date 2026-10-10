use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::{DateTime, Utc};
use dendro_core::{ActivityFact, AnalyticsStore, IdentityFact, QueueFact};
use rusqlite::OpenFlags;
use serde_json::Value;

use crate::stream::{self, walk_jsonl};

#[derive(Debug, Clone)]
pub struct SourceRoot {
    pub name: String,
    pub path: PathBuf,
}

pub fn ingest_staged(
    staged_db: &Path,
    roots: &[SourceRoot],
    frozen: bool,
    key: &[u8],
) -> Result<Vec<String>> {
    let store = AnalyticsStore::open_staged(staged_db)?;
    let mut notes = Vec::new();
    for root in roots {
        let shown = dendro_core::redact(&root.name);
        if !root.path.exists() {
            notes.push(format!("{shown} missing"));
            continue;
        }
        match root.name.as_str() {
            "claude" | "codex" | "pi" => {
                let files = walk_jsonl(&root.path);
                for (index, file) in files.iter().enumerate() {
                    stream::ingest_jsonl_with_key(
                        &store,
                        &format!("{}:{}", root.name, index),
                        &root.name,
                        file,
                        Some(key),
                    )?;
                }
                notes.push(format!("{shown} files={}", files.len()));
            }
            "openrig" => {
                let count = ingest_openrig(&store, &root.path, frozen, key)?;
                notes.push(format!("openrig events={count}"));
            }
            "agy" | "grok" => notes.push(format!("{shown} present-not-parsed")),
            _ => notes.push(format!("{shown} unsupported")),
        }
    }
    Ok(notes)
}

fn ingest_openrig(
    store: &AnalyticsStore,
    sqlite_path: &Path,
    frozen: bool,
    key: &[u8],
) -> Result<i64> {
    let snap_dir = std::env::temp_dir().join(format!("dendro-snap-{}", uuid_suffix()));
    let open_path = if frozen {
        std::fs::create_dir_all(&snap_dir)?;
        stream::copy_sqlite_trio(sqlite_path, &snap_dir)?
    } else {
        sqlite_path.to_path_buf()
    };
    let uri = format!("file:{}?mode=ro", open_path.display());
    let conn = rusqlite::Connection::open_with_flags(
        uri,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )?;
    let mut rig_names = std::collections::HashMap::<String, String>::new();
    if let Ok(mut rigs) = conn.prepare("SELECT id, name FROM rigs") {
        let rows = rigs.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows.flatten() {
            rig_names.insert(row.0, row.1);
        }
    }
    let mut count = 0i64;
    let mut stmt = conn.prepare("SELECT seq, type, payload, created_at FROM events")?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
        ))
    })?;
    for row in rows {
        let (seq, kind, payload, created_at) = row?;
        count += 1;
        let Ok(parsed) = serde_json::from_str::<Value>(&payload) else {
            continue;
        };
        let ts = DateTime::parse_from_rfc3339(&created_at)
            .ok()
            .map(|value| value.with_timezone(&Utc));
        if kind == "agent.session_identity" {
            let Some(session_id) = parsed.get("sessionId").and_then(|v| v.as_str()) else {
                continue;
            };
            let Some(runtime) = parsed.get("runtime").and_then(|v| v.as_str()) else {
                continue;
            };
            let Some(seat) = parsed.get("sessionName").and_then(|v| v.as_str()) else {
                continue;
            };
            let rig_id = parsed.get("rigId").and_then(|v| v.as_str()).unwrap_or("");
            let rig = rig_names.get(rig_id).map(String::as_str).unwrap_or(rig_id);
            store.insert_identity(&IdentityFact {
                source_key: format!("events:{seq}"),
                harness: normalize_harness(key, runtime),
                native_session: dendro_core::seal_identifier(
                    key,
                    &format!("session:{}", normalize_harness(key, runtime)),
                    session_id,
                ),
                rig: dendro_core::seal_identifier(key, "rig", rig),
                seat: dendro_core::seal_identifier(key, "seat", seat),
                ts,
            })?;
        } else if kind == "seat.activity_changed" {
            let state = dendro_core::activity_state(parsed.get("state").and_then(|v| v.as_str()));
            let Some(seat) = parsed
                .get("sessionName")
                .and_then(|v| v.as_str())
                .or_else(|| parsed.get("seat").and_then(|v| v.as_str()))
            else {
                continue;
            };
            let when = parsed
                .get("at")
                .and_then(|v| v.as_str())
                .and_then(|text| DateTime::parse_from_rfc3339(text).ok())
                .map(|value| value.with_timezone(&Utc))
                .or(ts);
            let Some(when) = when else { continue };
            store.insert_activity(&ActivityFact {
                source_key: format!("seat.activity_changed:{seq}"),
                seat: dendro_core::seal_identifier(key, "seat", seat),
                ts: when,
                state,
                seq,
            })?;
        } else if kind == "node.launched" {
            let Some(seat) = parsed.get("sessionName").and_then(|v| v.as_str()) else {
                continue;
            };
            let Some(when) = ts else { continue };
            store.upsert_seat_bound(&dendro_core::SeatBound {
                seat: dendro_core::seal_identifier(key, "seat", seat),
                created_at: Some(when),
                stopped_at: None,
            })?;
        } else if kind == "queue.created" || kind == "queue.claimed" {
            let Some(qitem) = parsed.get("qitemId").and_then(|v| v.as_str()) else {
                continue;
            };
            let Some(destination) = parsed.get("destinationSession").and_then(|v| v.as_str())
            else {
                continue;
            };
            let when = parsed
                .get("claimedAt")
                .and_then(|v| v.as_str())
                .and_then(|text| DateTime::parse_from_rfc3339(text).ok())
                .map(|value| value.with_timezone(&Utc))
                .or(ts);
            let Some(when) = when else { continue };
            let source = parsed
                .get("sourceSession")
                .and_then(|v| v.as_str())
                .unwrap_or(destination);
            store.insert_queue(&QueueFact {
                source_key: format!("events:{kind}:{seq}"),
                transition_id: seq,
                qitem_id: dendro_core::seal_identifier(key, "qitem", qitem),
                ts: when,
                state: if kind == "queue.claimed" {
                    "in-progress".to_string()
                } else {
                    "pending".to_string()
                },
                source_seat: dendro_core::seal_identifier(key, "seat", source),
                dest_seat: dendro_core::seal_identifier(key, "seat", destination),
                closure_reason: None,
                closure_target: None,
                chain_id: None,
                missions: Vec::new(),
            })?;
        }
    }
    let mut transitions = conn.prepare(
        "SELECT transition_id, qitem_id, ts, state, actor_session, closure_reason, closure_target, transition_note FROM queue_transitions",
    );
    if let Ok(ref mut stmt) = transitions {
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<String>>(7)?,
            ))
        })?;
        for row in rows.flatten() {
            let ts = DateTime::parse_from_rfc3339(&row.2)
                .ok()
                .map(|value| value.with_timezone(&Utc));
            let Some(ts) = ts else { continue };
            let actor_raw = row.4.clone().unwrap_or_default();
            let actor = dendro_core::seal_identifier(key, "seat", &actor_raw);
            let target = match row.6.as_deref() {
                Some(value) => dendro_core::seal_identifier(key, "seat", value),
                None => "unknown".to_string(),
            };
            store.insert_queue(&QueueFact {
                source_key: format!("queue_transitions:{}", row.0),
                transition_id: row.0,
                qitem_id: dendro_core::seal_identifier(key, "qitem", &row.1),
                ts,
                state: dendro_core::queue_state(&row.3),
                source_seat: actor,
                dest_seat: target,
                closure_reason: dendro_core::closure_reason(row.5.as_deref()),
                closure_target: row
                    .6
                    .as_deref()
                    .map(|target| dendro_core::seal_identifier(key, "seat", target)),
                chain_id: None,
                missions: parse_missions(row.7.as_deref())
                    .into_iter()
                    .map(|mission| dendro_core::seal_identifier(key, "mission", &mission))
                    .collect(),
            })?;
        }
    }
    let mut items =
        conn.prepare("SELECT qitem_id, destination_session, ts_updated FROM queue_items");
    if let Ok(ref mut stmt) = items {
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?;
        for row in rows.flatten() {
            let Some(destination) = row.1 else { continue };
            let cutoff = row.2.unwrap_or_else(|| "unknown".to_string());
            store.insert_destination_snapshot(
                &dendro_core::seal_identifier(key, "qitem", &row.0),
                &dendro_core::seal_identifier(key, "seat", &destination),
                &cutoff,
            )?;
        }
    }
    if frozen {
        let _ = std::fs::remove_dir_all(&snap_dir);
    }
    Ok(count)
}

fn parse_missions(note: Option<&str>) -> Vec<String> {
    let Some(note) = note else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<String>>(note).unwrap_or_default()
}

fn normalize_harness(key: &[u8], runtime: &str) -> String {
    let name = if runtime == "claude-code" {
        "claude"
    } else {
        runtime
    };
    dendro_core::seal_identifier(key, "harness", name)
}

fn uuid_suffix() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{nanos:x}")
}
