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
    let mut lives = Vec::new();
    let mut lifecycle_complete = true;
    for root in roots {
        let shown = dendro_core::redact(&root.name);
        if !root.path.exists() {
            if root.name == "openrig" {
                lifecycle_complete = false;
            }
            notes.push(format!("{shown} missing"));
            continue;
        }
        match root.name.as_str() {
            "claude" | "codex" | "pi" => {
                let files = walk_jsonl(&root.path);
                for file in &files {
                    stream::ingest_jsonl_with_key(
                        &store,
                        &stable_source_id(&root.name, &root.path, file, key),
                        &root.name,
                        file,
                        Some(key),
                    )?;
                }
                notes.push(format!("{shown} files={}", files.len()));
            }
            "openrig" => {
                let count = ingest_openrig(&store, &root.path, frozen, key, &mut lives)?;
                notes.push(format!("openrig events={count}"));
            }
            "agy" | "grok" => notes.push(format!("{shown} present-not-parsed")),
            _ => notes.push(format!("{shown} unsupported")),
        }
    }
    // Lives come from the whole history of every OpenRig root, so they are
    // replaced once. A missing root keeps the stored lives rather than erase them.
    if roots.iter().any(|root| root.name == "openrig") {
        if lifecycle_complete {
            store.replace_seat_bounds(&lives)?;
        } else {
            notes.push("openrig lifecycle unchanged: a root is missing".to_string());
        }
    }
    Ok(notes)
}

fn ingest_openrig(
    store: &AnalyticsStore,
    sqlite_path: &Path,
    frozen: bool,
    key: &[u8],
    lives: &mut Vec<dendro_core::SeatBound>,
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
    let mut lifecycle = Vec::new();
    // K5 reroute marks, by raw qitem id.
    let mut rerouted = std::collections::BTreeSet::<String>::new();
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
        let ts = dendro_core::parse_utc_time(&created_at);
        if let Some(event) = lifecycle_event(&kind, &parsed) {
            if let Some(at) = ts {
                lifecycle.push((at, seq, event));
            }
            continue;
        }
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
        } else if kind == "qitem.fallback_routed" {
            let text = |name: &str| parsed.get(name).and_then(|v| v.as_str());
            let Some(qitem) = text("qitemId") else {
                continue;
            };
            rerouted.insert(qitem.to_string());
            let (Some(original), Some(reroute), Some(when)) =
                (text("originalDestination"), text("rerouteDestination"), ts)
            else {
                continue;
            };
            store.insert_queue(&QueueFact {
                source_key: format!("events:{kind}:{seq}"),
                transition_id: seq,
                qitem_id: dendro_core::seal_identifier(key, "qitem", qitem),
                ts: when,
                state: "pending".to_string(),
                source_seat: dendro_core::seal_identifier(key, "seat", original),
                dest_seat: dendro_core::seal_identifier(key, "seat", reroute),
                closure_reason: None,
                closure_target: None,
                chain_id: None,
                missions: Vec::new(),
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
    lives.extend(
        seat_lives(lifecycle)
            .into_iter()
            .map(|life| dendro_core::SeatBound {
                seat: dendro_core::seal_identifier(key, "seat", &life.seat),
                ..life
            }),
    );
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
            if row
                .7
                .as_deref()
                .is_some_and(|note| note.starts_with("fallback-routed:"))
            {
                rerouted.insert(row.1.clone());
            }
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
    if let Ok(mut chains) = conn.prepare("SELECT qitem_id, chain_of_record FROM queue_items") {
        let rows = chains.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })?;
        for (qitem, chain) in rows.flatten() {
            let entries: Vec<String> = chain
                .as_deref()
                .and_then(|text| serde_json::from_str(text).ok())
                .unwrap_or_default();
            if entries
                .iter()
                .any(|entry| entry.starts_with("fallback-from:"))
            {
                rerouted.insert(qitem);
            }
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
                rerouted.contains(&row.0),
            )?;
        }
    }
    if frozen {
        let _ = std::fs::remove_dir_all(&snap_dir);
    }
    Ok(count)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Lifecycle {
    Launched {
        rig: String,
        node: String,
        seat: String,
    },
    SessionStopped {
        rig: String,
        node: String,
        seat: String,
    },
    NodeRemoved {
        rig: String,
        node: String,
    },
    RigEnded {
        rig: String,
    },
}

/// The K4 lifecycle events. `seat.fresh_launched` is launch evidence only: it
/// never proves the retired generation stopped. `session.detached` is a binding
/// detach, not a stop.
fn lifecycle_event(kind: &str, payload: &Value) -> Option<Lifecycle> {
    let text = |name: &str| {
        payload
            .get(name)
            .and_then(|v| v.as_str())
            .map(str::to_string)
    };
    let rig = text("rigId").unwrap_or_default();
    let node = text("nodeId").unwrap_or_default();
    match kind {
        "node.launched" | "seat.fresh_launched" => Some(Lifecycle::Launched {
            rig,
            node,
            seat: text("sessionName")?,
        }),
        "session.stopped" => Some(Lifecycle::SessionStopped {
            rig,
            node,
            seat: text("sessionName")?,
        }),
        "node.removed" => Some(Lifecycle::NodeRemoved {
            rig,
            node: text("nodeId")?,
        }),
        "rig.stopped" | "rig.deleted" => Some(Lifecycle::RigEnded {
            rig: text("rigId")?,
        }),
        _ => None,
    }
}

/// Builds each seat's recorded lives from lifecycle events, ordered by time then
/// sequence. A relaunch of a running seat continues its life. A stop with no
/// earlier launch keeps an unknown start. A stop is never inferred.
fn seat_lives(mut events: Vec<(DateTime<Utc>, i64, Lifecycle)>) -> Vec<dendro_core::SeatBound> {
    use std::collections::{BTreeMap, BTreeSet};
    events.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    let mut node_seat = BTreeMap::<(String, String), String>::new();
    let mut rig_seats = BTreeMap::<String, BTreeSet<String>>::new();
    let mut open = BTreeMap::<String, DateTime<Utc>>::new();
    let mut seen = BTreeSet::<String>::new();
    let mut lives = Vec::new();
    // A stop closes the open life, or records one with an unknown start for a
    // seat never seen before. A repeated stop changes nothing.
    let stop = |seat: &str,
                at: DateTime<Utc>,
                open: &mut BTreeMap<String, DateTime<Utc>>,
                seen: &mut BTreeSet<String>,
                lives: &mut Vec<_>| {
        if let Some(start) = open.remove(seat) {
            lives.push((seat.to_string(), Some(start), Some(at)));
        } else if seen.insert(seat.to_string()) {
            lives.push((seat.to_string(), None, Some(at)));
        }
    };
    for (at, _, event) in events {
        match event {
            Lifecycle::Launched { rig, node, seat } => {
                node_seat.insert((rig.clone(), node), seat.clone());
                rig_seats.entry(rig).or_default().insert(seat.clone());
                seen.insert(seat.clone());
                open.entry(seat).or_insert(at);
            }
            Lifecycle::SessionStopped { rig, node, seat } => {
                node_seat.insert((rig.clone(), node), seat.clone());
                rig_seats.entry(rig).or_default().insert(seat.clone());
                stop(&seat, at, &mut open, &mut seen, &mut lives);
            }
            Lifecycle::NodeRemoved { rig, node } => {
                if let Some(seat) = node_seat.get(&(rig, node)).cloned() {
                    stop(&seat, at, &mut open, &mut seen, &mut lives);
                }
            }
            Lifecycle::RigEnded { rig } => {
                for seat in rig_seats.get(&rig).cloned().unwrap_or_default() {
                    stop(&seat, at, &mut open, &mut seen, &mut lives);
                }
            }
        }
    }
    lives.extend(
        open.into_iter()
            .map(|(seat, start)| (seat, Some(start), None)),
    );
    lives
        .into_iter()
        .map(|(seat, created_at, stopped_at)| dendro_core::SeatBound {
            seat,
            created_at,
            stopped_at,
        })
        .collect()
}

fn parse_missions(note: Option<&str>) -> Vec<String> {
    let Some(note) = note else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<String>>(note).unwrap_or_default()
}

pub(crate) fn stable_source_id(root_name: &str, root: &Path, file: &Path, key: &[u8]) -> String {
    let relative = file.strip_prefix(root).unwrap_or(file);
    dendro_core::seal_identifier(
        key,
        "source",
        &format!("{root_name}\n{}\n{}", root.display(), relative.display()),
    )
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000 + secs, 0).unwrap()
    }

    fn launched(rig: &str, node: &str, seat: &str) -> Lifecycle {
        Lifecycle::Launched {
            rig: rig.into(),
            node: node.into(),
            seat: seat.into(),
        }
    }

    fn lives_of(
        events: Vec<(DateTime<Utc>, i64, Lifecycle)>,
        seat: &str,
    ) -> Vec<(Option<DateTime<Utc>>, Option<DateTime<Utc>>)> {
        seat_lives(events)
            .into_iter()
            .filter(|life| life.seat == seat)
            .map(|life| (life.created_at, life.stopped_at))
            .collect()
    }

    #[test]
    fn relaunch_after_a_stop_starts_a_new_life() {
        let events = vec![
            (at(0), 1, launched("r", "n", "s")),
            (
                at(10),
                2,
                Lifecycle::SessionStopped {
                    rig: "r".into(),
                    node: "n".into(),
                    seat: "s".into(),
                },
            ),
            (at(20), 3, launched("r", "n", "s")),
        ];
        assert_eq!(
            lives_of(events, "s"),
            vec![(Some(at(0)), Some(at(10))), (Some(at(20)), None)]
        );
    }

    #[test]
    fn relaunch_of_a_running_seat_continues_its_life() {
        let events = vec![
            (at(0), 1, launched("r", "n", "s")),
            (at(5), 2, launched("r", "n", "s")),
        ];
        assert_eq!(lives_of(events, "s"), vec![(Some(at(0)), None)]);
    }

    #[test]
    fn a_fresh_launch_opens_a_life_and_never_closes_one() {
        let payload = serde_json::json!({
            "rigId": "r", "nodeId": "n", "sessionName": "s", "sessionId": "new",
            "supersededSessionIds": ["old"], "retiringGeneration": "g1", "newGeneration": "g2"
        });
        let fresh = || lifecycle_event("seat.fresh_launched", &payload).unwrap();
        assert_eq!(fresh(), launched("r", "n", "s"));
        // While running: no stop for the retired generation, the life continues.
        let events = vec![(at(0), 1, launched("r", "n", "s")), (at(5), 2, fresh())];
        assert_eq!(lives_of(events, "s"), vec![(Some(at(0)), None)]);
        // After a stop: it opens a new life.
        let events = vec![
            (at(0), 1, launched("r", "n", "s")),
            (at(5), 2, lifecycle_event("rig.stopped", &payload).unwrap()),
            (at(9), 3, fresh()),
        ];
        assert_eq!(
            lives_of(events, "s"),
            vec![(Some(at(0)), Some(at(5))), (Some(at(9)), None)]
        );
        // As the only event: it opens the first life.
        assert_eq!(
            lives_of(vec![(at(3), 1, fresh())], "s"),
            vec![(Some(at(3)), None)]
        );
    }

    #[test]
    fn rig_stop_and_delete_end_every_seat_of_that_rig_only() {
        for kind in ["rig.stopped", "rig.deleted"] {
            let ended = lifecycle_event(kind, &serde_json::json!({"rigId": "a"})).unwrap();
            let events = vec![
                (at(0), 1, launched("a", "n1", "a1")),
                (at(1), 2, launched("a", "n2", "a2")),
                (at(2), 3, launched("b", "n3", "b1")),
                (at(9), 4, ended),
            ];
            assert_eq!(
                lives_of(events.clone(), "a1"),
                vec![(Some(at(0)), Some(at(9)))]
            );
            assert_eq!(
                lives_of(events.clone(), "a2"),
                vec![(Some(at(1)), Some(at(9)))]
            );
            assert_eq!(lives_of(events, "b1"), vec![(Some(at(2)), None)], "{kind}");
        }
    }

    #[test]
    fn node_removal_resolves_the_seat_through_its_launch() {
        let removed = |node: &str| Lifecycle::NodeRemoved {
            rig: "r".into(),
            node: node.into(),
        };
        let events = vec![
            (at(0), 1, launched("r", "n", "s")),
            (at(4), 2, removed("other")),
            (at(7), 3, removed("n")),
        ];
        let all = seat_lives(events);
        assert_eq!(all.len(), 1, "an unknown node names no seat");
        assert_eq!(all[0].stopped_at, Some(at(7)));
    }

    #[test]
    fn detach_status_and_unknown_events_are_not_stops() {
        let payload = serde_json::json!({"rigId": "r", "nodeId": "n", "sessionName": "s"});
        for kind in [
            "session.detached",
            "session.status_changed",
            "session.cleaned",
        ] {
            assert_eq!(lifecycle_event(kind, &payload), None, "{kind}");
        }
        let events = vec![(at(0), 1, launched("r", "n", "s"))];
        assert_eq!(lives_of(events, "s"), vec![(Some(at(0)), None)]);
    }

    #[test]
    fn a_stop_without_a_launch_keeps_an_unknown_start_and_counts_once() {
        let stopped = || Lifecycle::SessionStopped {
            rig: "r".into(),
            node: "n".into(),
            seat: "s".into(),
        };
        let events = vec![(at(3), 1, stopped()), (at(8), 2, stopped())];
        assert_eq!(lives_of(events, "s"), vec![(None, Some(at(3)))]);
    }

    #[test]
    fn equal_times_order_by_sequence() {
        let stopped = Lifecycle::SessionStopped {
            rig: "r".into(),
            node: "n".into(),
            seat: "s".into(),
        };
        // Listed out of order; seq 1 (launch) runs before seq 2 (stop).
        let events = vec![(at(5), 2, stopped), (at(5), 1, launched("r", "n", "s"))];
        assert_eq!(lives_of(events, "s"), vec![(Some(at(5)), Some(at(5)))]);
    }

    fn openrig_root(dir: &Path, name: &str, seat: &str) -> SourceRoot {
        let path = dir.join(format!("{name}.sqlite"));
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute(
            "CREATE TABLE events (seq INTEGER, type TEXT, payload TEXT, created_at TEXT)",
            [],
        )
        .unwrap();
        let payload =
            format!("{{\"rigId\":\"{name}\",\"nodeId\":\"n\",\"sessionName\":\"{seat}\"}}");
        conn.execute(
            "INSERT INTO events VALUES (1,'node.launched',?,'2026-10-09 01:00:00')",
            [&payload],
        )
        .unwrap();
        SourceRoot {
            name: "openrig".into(),
            path,
        }
    }

    fn stored_seats(db: &Path) -> Vec<String> {
        let store = AnalyticsStore::open_path(db, "key").unwrap();
        let mut seats: Vec<String> = store
            .load_seat_bounds()
            .unwrap()
            .into_iter()
            .map(|life| life.seat)
            .collect();
        seats.sort();
        seats
    }

    #[test]
    fn openrig_roots_do_not_erase_each_others_lives() {
        let dir = std::env::temp_dir().join(format!("dendro-roots-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("analytics.duckdb");
        drop(AnalyticsStore::open_path(&db, "key").unwrap());
        let roots = [
            openrig_root(&dir, "rig-a", "seat-a"),
            openrig_root(&dir, "rig-b", "seat-b"),
        ];
        let key = b"roots-test-key";
        ingest_staged(&db, &roots, false, key).unwrap();
        assert_eq!(stored_seats(&db), vec!["seat-a", "seat-b"]);
        // A second run rebuilds the same lives, without duplicates.
        ingest_staged(&db, &roots, false, key).unwrap();
        assert_eq!(stored_seats(&db), vec!["seat-a", "seat-b"]);
        // A run with one root missing keeps every stored life.
        let partial = [
            roots[0].clone(),
            SourceRoot {
                name: "openrig".into(),
                path: dir.join("absent.sqlite"),
            },
        ];
        let notes = ingest_staged(&db, &partial, false, key).unwrap();
        assert!(notes
            .iter()
            .any(|note| note == "openrig lifecycle unchanged: a root is missing"));
        assert_eq!(stored_seats(&db), vec!["seat-a", "seat-b"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn k5_reroute_marks_and_unclaim_reach_the_k2_report() {
        let dir = std::env::temp_dir().join(format!("dendro-k5-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("openrig.sqlite");
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE events (seq INTEGER, type TEXT, payload TEXT, created_at TEXT);
             CREATE TABLE queue_transitions (transition_id INTEGER, qitem_id TEXT, ts TEXT, state TEXT,
               actor_session TEXT, closure_reason TEXT, closure_target TEXT, transition_note TEXT);
             CREATE TABLE queue_items (qitem_id TEXT, destination_session TEXT, ts_updated TEXT,
               chain_of_record TEXT);",
        )
        .unwrap();
        let mut seq = 0;
        let mut event = |kind: &str, payload: serde_json::Value, at: &str| {
            seq += 1;
            conn.execute(
                "INSERT INTO events VALUES (?,?,?,?)",
                rusqlite::params![seq, kind, payload.to_string(), at],
            )
            .unwrap();
        };
        for item in ["q-note", "q-chain", "q-event", "q-clean"] {
            event(
                "queue.created",
                serde_json::json!({"qitemId": item, "sourceSession": "orch", "destinationSession": "seat-a"}),
                "2026-10-09 01:00:00",
            );
        }
        event(
            "qitem.fallback_routed",
            serde_json::json!({"qitemId": "q-event", "originalDestination": "seat-a",
                "rerouteDestination": "seat-b", "reason": "unreachable"}),
            "2026-10-09 01:30:00",
        );
        event(
            "queue.unclaimed",
            serde_json::json!({"qitemId": "q-clean", "destinationSession": "seat-a"}),
            "2026-10-09 01:40:00",
        );
        drop(event);
        let mut transition = 0;
        let mut step = |item: &str, state: &str, note: Option<&str>| {
            transition += 1;
            conn.execute(
                "INSERT INTO queue_transitions VALUES (?,?,?,?,?,?,?,?)",
                rusqlite::params![
                    transition,
                    item,
                    format!("2026-10-09T02:00:{transition:02}.000Z"),
                    state,
                    "actor",
                    Option::<String>::None,
                    Option::<String>::None,
                    note
                ],
            )
            .unwrap();
        };
        step(
            "q-note",
            "pending",
            Some("fallback-routed: seat-a → seat-b (gone)"),
        );
        for item in ["q-note", "q-chain", "q-event", "q-clean", "q-nocreate"] {
            step(item, "done", None);
        }
        drop(step);
        for (item, chain) in [
            ("q-note", "[]"),
            ("q-chain", "[\"fallback-from:seat-a\"]"),
            ("q-event", "[]"),
            ("q-clean", "[]"),
            ("q-nocreate", "[]"),
        ] {
            let destination = if item == "q-event" {
                "seat-b"
            } else {
                "seat-a"
            };
            conn.execute(
                "INSERT INTO queue_items VALUES (?,?,'2026-10-09T02:00:09.000Z',?)",
                rusqlite::params![item, destination, chain],
            )
            .unwrap();
        }
        drop(conn);
        let db = dir.join("analytics.duckdb");
        drop(AnalyticsStore::open_path(&db, "key").unwrap());
        let roots = [SourceRoot {
            name: "openrig".into(),
            path,
        }];
        ingest_staged(&db, &roots, false, b"k5-test-key").unwrap();
        let store = AnalyticsStore::open_path(&db, "key").unwrap();
        let marked: std::collections::BTreeMap<String, bool> = store
            .load_destination_snapshots()
            .unwrap()
            .into_iter()
            .map(|(item, snapshot)| (item, snapshot.reroute_marked))
            .collect();
        assert_eq!(marked["q-note"], true, "transition note mark");
        assert_eq!(marked["q-chain"], true, "chain_of_record mark");
        assert_eq!(marked["q-event"], true, "reroute event mark");
        assert_eq!(marked["q-clean"], false);
        assert_eq!(marked["q-nocreate"], false);
        drop(store);
        let report = dendro_core::report_json(
            &db,
            "decisions",
            None,
            "2026-10-09T00:00:00Z",
            "2026-10-10T00:00:00Z",
        )
        .unwrap();
        let payload = &report["payload"];
        let mut insufficient: Vec<&str> = payload["k2_insufficient_history_members"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["qitem"].as_str().unwrap())
            .collect();
        insufficient.sort();
        assert_eq!(insufficient, vec!["q-chain", "q-nocreate", "q-note"]);
        assert_eq!(payload["k2_closed_items_with_insufficient_history"], 3);
        let closed: Vec<(String, String)> = metrics_members(payload);
        assert_eq!(
            closed,
            vec![
                ("q-clean".to_string(), "events:queue.created:4".to_string()),
                (
                    "q-event".to_string(),
                    "events:qitem.fallback_routed:5".to_string()
                ),
            ],
            "unclaim neither closes nor moves q-clean; the reroute moves q-event to seat-b"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn metrics_members(payload: &serde_json::Value) -> Vec<(String, String)> {
        let mut members: Vec<(String, String)> = payload["m1_by_seat"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|seat| seat["closed_item_members"].as_array().unwrap().clone())
            .map(|item| {
                (
                    item["qitem"].as_str().unwrap().to_string(),
                    item["destination_evidence"].as_str().unwrap().to_string(),
                )
            })
            .collect();
        members.sort();
        members
    }

    #[test]
    fn openrig_created_at_is_utc_seconds_without_a_zone() {
        assert_eq!(
            dendro_core::parse_utc_time("2026-10-10 15:38:41"),
            Some(Utc.with_ymd_and_hms(2026, 10, 10, 15, 38, 41).unwrap())
        );
    }
}
