use std::fs::{self, File};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use crate::facts::SeatBound;
use crate::metrics::{self, Window};
use crate::output::{self, load_or_create_key, refuse_aliased_output, with_output_lock};
use crate::privacy::{contains_secret, opaque_id, redact};
use crate::storage::AnalyticsStore;

pub struct RunRequest<'a> {
    pub db: &'a Path,
    pub source_roots: &'a [PathBuf],
}

pub fn run_locked<F>(db: &Path, source_roots: &[PathBuf], ingest: F) -> Result<Value>
where
    F: FnOnce(&Path, &[u8]) -> Result<Vec<String>>,
{
    if source_roots.is_empty() {
        return Err(anyhow!("at least one source root is required"));
    }
    refuse_secret_path(db)?;
    for root in source_roots {
        refuse_secret_path(root)?;
    }
    let parent = db.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let lock_path = parent.join(".dendro-output.lock");
    with_output_lock(&lock_path, || run_inner(db, source_roots, ingest))
}

fn run_inner<F>(db: &Path, source_roots: &[PathBuf], ingest: F) -> Result<Value>
where
    F: FnOnce(&Path, &[u8]) -> Result<Vec<String>>,
{
    refuse_aliased_output(db, source_roots)?;
    let before = fs::symlink_metadata(db).ok();
    if before
        .as_ref()
        .is_some_and(|meta| meta.file_type().is_symlink())
    {
        return Err(anyhow!("refusing symlink analytics output"));
    }
    let key_path = PathBuf::from(format!("{}.key", db.display()));
    let creating = !db.exists();
    let key = load_or_create_key(&key_path, creating || key_path.exists())?;
    if db.exists() {
        let preflight = AnalyticsStore::open_read_only(db)?;
        if preflight.key_id != key.id {
            return Err(anyhow!("analytics key id does not match the sidecar"));
        }
    }
    let parent = db.parent().unwrap_or_else(|| Path::new("."));
    let stage = parent.join(format!(".dendro-stage-{}", random_id()));
    fs::create_dir(&stage)?;
    let staged = stage.join("analytics.duckdb");
    let published = (|| {
        if db.exists() {
            fs::copy(db, &staged)?;
        } else {
            let created = AnalyticsStore::open_path(&staged, &key.id)?;
            created.checkpoint()?;
        }
        let notes = ingest(&staged, &key.bytes)?;
        let staged_store = AnalyticsStore::open_staged(&staged)?;
        staged_store.checkpoint()?;
        drop(staged_store);
        File::open(&staged)?.sync_all()?;
        let after = fs::symlink_metadata(db).ok();
        if after
            .as_ref()
            .is_some_and(|meta| meta.file_type().is_symlink())
        {
            return Err(anyhow!("refusing symlink analytics output"));
        }
        if let (Some(left), Some(right)) = (&before, &after) {
            use std::os::unix::fs::MetadataExt;
            if left.ino() != right.ino() || left.dev() != right.dev() {
                return Err(anyhow!("analytics output identity changed during ingest"));
            }
        }
        if before.is_none() && after.is_some() {
            return Err(anyhow!("analytics output appeared during ingest"));
        }
        fs::rename(&staged, db)?;
        if let Some(parent) = db.parent() {
            output::sync_dir(parent)?;
        }
        Ok(notes)
    })();
    let _ = fs::remove_dir_all(&stage);
    Ok(json!({
        "status": "ready",
        "product": "AI Fleet Intelligence",
        "key_id": key.id,
        "sources": published?,
    }))
}

fn random_id() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 8];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

pub fn trace_tokens(store: &AnalyticsStore, window: Window) -> Result<Value> {
    let usage = store.load_usage()?;
    let (input, output, cache_read, cache_write) = metrics::token_totals(
        &usage
            .into_iter()
            .filter(|u| match u.ts {
                Some(ts) => ts >= window.from && ts < window.to,
                None => false,
            })
            .collect::<Vec<_>>(),
    );
    Ok(metrics::report_value(
        "tokens",
        json!({
            "input_uncached": input,
            "output": output,
            "cache_read": cache_read,
            "cache_write": cache_write,
            "from": window.from.to_rfc3339(),
            "to": window.to.to_rfc3339(),
        }),
    ))
}

pub fn opaque_session(key: &[u8], harness: &str, native: &str) -> String {
    opaque_id(key, "session", &format!("{harness}\n{native}"))
}

fn refuse_secret_path(path: &Path) -> Result<()> {
    let shown = path.display().to_string();
    if contains_secret(&shown) {
        return Err(anyhow!(
            "refusing a path that contains a secret-shaped value"
        ));
    }
    Ok(())
}

pub fn report_json(
    db: &Path,
    kind: &str,
    rig: Option<&str>,
    from: &str,
    to: &str,
) -> Result<Value> {
    refuse_secret_path(db)?;
    let window = parse_window(from, to)?;
    if !matches!(
        kind,
        "links" | "activity" | "handoffs" | "tokens" | "decisions"
    ) {
        return Err(anyhow!("unknown report kind {}", redact(kind)));
    }
    let store = AnalyticsStore::open_read_only(db)?;
    let identities = store.load_identity()?;
    let allowed: Option<std::collections::HashSet<(String, String)>> = rig.map(|name| {
        identities
            .iter()
            .filter(|fact| fact.rig == name)
            .map(|fact| (fact.harness.clone(), fact.native_session.clone()))
            .collect()
    });
    let rig_seats: Option<std::collections::HashSet<String>> = rig.map(|name| {
        identities
            .iter()
            .filter(|fact| fact.rig == name)
            .map(|fact| fact.seat.clone())
            .collect()
    });
    let sources = store.load_sources()?;
    let destination_snapshots = store.load_destination_snapshots()?;
    let usage_all = store.load_usage()?;
    let untimestamped_count = usage_all.iter().filter(|row| row.ts.is_none()).count();
    let usage: Vec<_> = usage_all
        .iter()
        .filter(|row| match row.ts {
            Some(ts) => ts >= window.from && ts < window.to,
            None => false,
        })
        .filter(|row| match &allowed {
            Some(set) => set.contains(&(row.harness.clone(), row.native_session.clone())),
            None => true,
        })
        .cloned()
        .collect();
    let queue_all: Vec<_> = store
        .load_queue()?
        .into_iter()
        .filter(|row| match &rig_seats {
            Some(seats) => seats.contains(&row.source_seat) || seats.contains(&row.dest_seat),
            None => true,
        })
        .collect();
    let edges_all = metrics::handoff_edges(&queue_all);
    let edges: Vec<_> = edges_all
        .iter()
        .filter(|edge| {
            queue_all.iter().any(|row| {
                row.source_key == edge.source_key && row.ts >= window.from && row.ts < window.to
            })
        })
        .cloned()
        .collect();
    let (rework, chains, paths) = metrics::m3_rework(&edges);
    let handoffs = edges.len() as i64;
    let returns = edges.iter().filter(|edge| edge.is_return).count() as i64;
    let (input, output, cache_read, cache_write) = metrics::token_totals(&usage);
    let tokens = input + output + cache_read + cache_write;
    let nonreturn = edges.iter().filter(|edge| !edge.delivery_returned).count() as i64;
    let mut seat_tokens = std::collections::BTreeMap::<String, i64>::new();
    for row in &usage {
        let Some(seat) = identities
            .iter()
            .find(|fact| fact.harness == row.harness && fact.native_session == row.native_session)
            .map(|fact| fact.seat.clone())
        else {
            continue;
        };
        *seat_tokens.entry(seat).or_default() +=
            row.input_uncached + row.output + row.cache_read + row.cache_write;
    }
    let mut m1_seat_names = std::collections::BTreeSet::new();
    for seat in seat_tokens.keys() {
        m1_seat_names.insert(seat.clone());
    }
    for edge in &edges {
        m1_seat_names.insert(edge.from_seat.clone());
    }
    let m1_by_seat: Vec<_> = m1_seat_names
        .iter()
        .map(|seat| {
            let seat_total = *seat_tokens.get(seat).unwrap_or(&0);
            let deliveries = edges
                .iter()
                .filter(|edge| edge.from_seat == *seat && !edge.delivery_returned)
                .count() as i64;
            let closed = metrics::closed_item_keys(seat, &queue_all, window);
            let per_closed = if closed.is_empty() {
                serde_json::Value::String("no data".into())
            } else {
                json!(seat_total as f64 / closed.len() as f64)
            };
            json!({
                "seat": seat,
                "tokens": seat_total,
                "nonreturn_deliveries": deliveries,
                "yield": metrics::m1_yield(seat_total, deliveries),
                "closed_items": closed.len(),
                "closed_item_keys": closed,
                "tokens_per_closed_item": per_closed,
            })
        })
        .collect::<Vec<_>>();
    let mut pair_counts = std::collections::BTreeMap::<(String, String), (i64, i64)>::new();
    for edge in &edges {
        let slot = pair_counts
            .entry((edge.from_seat.clone(), edge.to_seat.clone()))
            .or_insert((0, 0));
        slot.1 += 1;
        if edge.is_return {
            slot.0 += 1;
        }
    }
    let m2_pairs: Vec<_> = pair_counts
        .iter()
        .map(|((from_seat, to_seat), (pair_returns, pair_handoffs))| {
            json!({
                "from": from_seat,
                "to": to_seat,
                "returns": pair_returns,
                "handoffs": pair_handoffs,
                "return_rate": metrics::m2_return_rate(*pair_returns, *pair_handoffs),
            })
        })
        .collect();
    let activity: Vec<_> = store
        .load_activity()?
        .into_iter()
        .filter(|row| match &rig_seats {
            Some(seats) => seats.contains(&row.seat),
            None => true,
        })
        .collect();
    let bounds = store.load_seat_bounds()?;
    let mut seats: Vec<String> = activity.iter().map(|row| row.seat.clone()).collect();
    for bound in &bounds {
        if rig_seats
            .as_ref()
            .is_none_or(|seats| seats.contains(&bound.seat))
        {
            seats.push(bound.seat.clone());
        }
    }
    seats.sort();
    seats.dedup();
    let activity_rows: Vec<_> = seats
        .iter()
        .map(|seat| {
            let lives: Vec<SeatBound> = bounds
                .iter()
                .filter(|bound| bound.seat == *seat)
                .cloned()
                .collect();
            let bands = metrics::activity_bands(seat, &activity, &queue_all, window, &lives);
            let spans = metrics::observation_spans(window, &lives);
            let evidence = |recorded: bool| if recorded { "recorded" } else { "unavailable" };
            json!({
                "seat": seat,
                "observed_secs": metrics::observed_secs(&spans),
                "observed_spans": spans
                    .iter()
                    .map(|span| json!({"from": span.start.to_rfc3339(), "to": span.end.to_rfc3339()}))
                    .collect::<Vec<_>>(),
                "launch_evidence": evidence(lives.iter().any(|life| life.created_at.is_some())),
                "stop_evidence": evidence(lives.iter().any(|life| life.stopped_at.is_some())),
                "active_secs": bands.active_secs,
                "idle_with_work_secs": bands.idle_with_work_secs,
                "idle_without_work_secs": bands.idle_without_work_secs,
                "waiting_human_secs": bands.waiting_human_secs,
                "unknown_secs": bands.unknown_secs,
                "sum": bands.sum(),
            })
        })
        .collect();
    let tools: Vec<_> = store
        .load_tools()?
        .into_iter()
        .filter(|tool| match tool.ts {
            Some(ts) => ts >= window.from && ts < window.to,
            None => false,
        })
        .filter(|tool| match &allowed {
            Some(set) => set.contains(&(tool.harness.clone(), tool.native_session.clone())),
            None => rig_seats.is_none(),
        })
        .collect();
    let loops = metrics::tool_loops(&tools, 3, 600);
    let seat_of = |native: &str| {
        identities
            .iter()
            .find(|fact| fact.native_session == native)
            .map(|fact| fact.seat.clone())
    };
    let intakes = metrics::handoff_intakes(&edges, &queue_all, &tools, &usage, seat_of);
    let medians = metrics::pair_medians(&edges, &queue_all, &intakes);
    let mut mission_tokens = std::collections::BTreeMap::<String, i64>::new();
    for row in &usage {
        let mut bound_seats: Vec<String> = identities
            .iter()
            .filter(|fact| fact.harness == row.harness && fact.native_session == row.native_session)
            .map(|fact| fact.seat.clone())
            .collect();
        bound_seats.sort();
        bound_seats.dedup();
        let seat = if bound_seats.len() == 1 {
            bound_seats.pop()
        } else {
            None
        };
        let label = match (seat, row.ts) {
            (Some(seat), Some(ts)) => match metrics::mission_at(&seat, &queue_all, ts) {
                metrics::MissionBucket::Mission(name) => name,
                metrics::MissionBucket::None => "no_mission".into(),
                metrics::MissionBucket::Ambiguous => "ambiguous".into(),
            },
            (None, Some(_)) => "unlinked".into(),
            _ => continue,
        };
        *mission_tokens.entry(label).or_insert(0) +=
            row.input_uncached + row.output + row.cache_read + row.cache_write;
    }
    let mut discovered = std::collections::BTreeSet::<(String, String)>::new();
    for row in &usage_all {
        discovered.insert((row.harness.clone(), row.native_session.clone()));
    }
    for fact in &identities {
        discovered.insert((fact.harness.clone(), fact.native_session.clone()));
    }
    for (source_id, harness, _path, _generation) in &sources {
        let prefix = format!("{source_id}:");
        let has_usage = usage_all
            .iter()
            .any(|row| row.source_key.starts_with(&prefix));
        if !has_usage {
            discovered.insert((harness.clone(), format!("file:{source_id}")));
        }
    }
    let mut link_rates = std::collections::BTreeMap::<String, (i64, i64)>::new();
    for (harness, native) in &discovered {
        let seats: std::collections::BTreeSet<_> = identities
            .iter()
            .filter(|fact| fact.harness == *harness && fact.native_session == *native)
            .map(|fact| fact.seat.clone())
            .collect();
        let entry = link_rates.entry(harness.clone()).or_insert((0, 0));
        entry.1 += 1;
        if seats.len() == 1 {
            entry.0 += 1;
        }
    }
    let links: Vec<_> = discovered
        .into_iter()
        .map(|(harness, native)| {
            let matches: Vec<_> = identities
                .iter()
                .filter(|fact| fact.harness == harness && fact.native_session == native)
                .collect();
            let mut bound: Vec<&str> = matches.iter().map(|fact| fact.seat.as_str()).collect();
            bound.sort();
            bound.dedup();
            let status = if matches.is_empty() {
                "unlinked"
            } else if bound.len() == 1 {
                "linked"
            } else {
                "collision"
            };
            json!({
                "harness": harness,
                "native_session": native,
                "status": status,
                "rig": if bound.len() == 1 { matches.first().map(|fact| fact.rig.clone()) } else { None },
                "seat": if bound.len() == 1 { matches.first().map(|fact| fact.seat.clone()) } else { None },
                "evidence": matches.iter().map(|fact| fact.source_key.clone()).collect::<Vec<_>>(),
            })
        })
        .collect::<Vec<_>>();
    let trace_inputs: Vec<_> = usage
        .iter()
        .map(|row| {
            let suffix = format!(":{}:{}", row.generation, row.seq);
            let source_id = row.source_key.strip_suffix(&suffix);
            let path = source_id.and_then(|id| {
                sources
                    .iter()
                    .find(|source| source.0 == id)
                    .map(|source| source.2.clone())
            });
            json!({
                "source_key": row.source_key,
                "generation": row.generation,
                "line": row.seq,
                "path": path,
                "predecessor_reason": row.predecessor_reason,
                "predecessor_source_key": row.predecessor_source_key,
                "prior_input_tokens": row.prior_input_tokens,
                "prior_cached_input_tokens": row.prior_cached_input_tokens,
                "prior_cache_write_input_tokens": row.prior_cache_write_input_tokens,
                "prior_output_tokens": row.prior_output_tokens,
                "prior_reasoning_output_tokens": row.prior_reasoning_output_tokens,
                "input_uncached": row.input_uncached,
                "output": row.output,
                "cache_read": row.cache_read,
                "cache_write": row.cache_write,
            })
        })
        .collect();
    Ok(metrics::report_value(
        kind,
        json!({
            "product": "AI Fleet Intelligence",
            "rig": rig.map(redact),
            "from": redact(from),
            "to": redact(to),
            "usage_rows": usage.len(),
            "untimestamped_count": untimestamped_count,
            "link_rates": link_rates.iter().map(|(harness, (linked, discovered_count))| json!({
                "harness": harness,
                "linked": linked,
                "discovered": discovered_count,
                "status": if *discovered_count == 0 { "no data" } else { "ok" },
            })).collect::<Vec<_>>(),
            "tokens": tokens,
            "m1_yield": metrics::m1_yield(tokens, nonreturn),
            "m1_by_seat": m1_by_seat,
            "k2_closed_item": metrics::K2_CLOSED_ITEM,
            "m2_return_rate": metrics::m2_return_rate(returns, handoffs),
            "m2_pairs": m2_pairs,
            "m3_rework_chains": rework,
            "m3_chains": chains,
            "m3_paths": paths,
            "m4_impact": metrics::m4_impact(),
            "kind": redact(kind),
            "activity": activity_rows,
            "links": links,
            "handoff_edges": edges.iter().map(|edge| json!({
                "from": edge.from_seat,
                "to": edge.to_seat,
                "return": edge.is_return,
                "qitem_id": edge.qitem_id,
                "source_key": edge.source_key,
            })).collect::<Vec<_>>(),
            "intakes": intakes.iter().map(|row| json!({
                "qitem_id": row.qitem_id,
                "to": row.to_seat,
                "status": row.status,
                "reason": row.reason,
                "tokens": row.tokens,
                "claim": row.claim,
                "write": row.write,
                "samples": row.samples,
            })).collect::<Vec<_>>(),
            "pair_medians": medians.iter().map(|row| json!({
                "from": row.from_seat,
                "to": row.to_seat,
                "wait_samples": row.wait_samples,
                "wait_invalid": row.wait_invalid,
                "wait_median": row.wait_median,
                "intake_samples": row.intake_samples,
                "intake_invalid": row.intake_invalid,
                "intake_median": row.intake_median,
            })).collect::<Vec<_>>(),
            "mission_tokens": mission_tokens,
            "tool_loops": loops.iter().map(|item| json!({"session": item.session, "tool": item.tool, "count": item.count})).collect::<Vec<_>>(),
            "activity_events": activity.iter().map(|row| json!({
                "source_key": row.source_key,
                "seat": row.seat,
                "state": row.state,
                "ts": row.ts.to_rfc3339(),
            })).collect::<Vec<_>>(),
            "demand_events": queue_all.iter().map(|row| json!({
                "source_key": row.source_key,
                "qitem_id": row.qitem_id,
                "state": row.state,
                "dest": row.dest_seat,
                "ts": row.ts.to_rfc3339(),
            })).collect::<Vec<_>>(),
            "nonreturn_handoffs": edges.iter().filter(|edge| !edge.is_return).map(|edge| json!({
                "source_key": edge.source_key,
                "from": edge.from_seat,
                "to": edge.to_seat,
                "qitem_id": edge.qitem_id,
            })).collect::<Vec<_>>(),
            "destination_snapshots": destination_snapshots.iter().map(|(qitem, destination, cutoff)| json!({
                "qitem_id": qitem,
                "destination": destination,
                "snapshot_cutoff": cutoff,
                "historical": false,
            })).collect::<Vec<_>>(),
            "trace_inputs": trace_inputs,
        }),
    ))
}

fn parse_window(from: &str, to: &str) -> Result<Window> {
    let from = chrono::DateTime::parse_from_rfc3339(from)
        .map_err(|_| anyhow!("invalid from timestamp"))?
        .with_timezone(&chrono::Utc);
    let to = chrono::DateTime::parse_from_rfc3339(to)
        .map_err(|_| anyhow!("invalid to timestamp"))?
        .with_timezone(&chrono::Utc);
    if to <= from {
        return Err(anyhow!("report window must be non-empty and half-open"));
    }
    Ok(Window { from, to })
}

pub fn trace_json(
    db: &Path,
    metric_id: &str,
    rig: Option<&str>,
    from: &str,
    to: &str,
) -> Result<Value> {
    let report = report_json(db, "decisions", rig, from, to)?;
    let payload = &report["payload"];
    let (formula, value) = match metric_id {
        "m1" => (
            "seat tokens / non-returned deliveries, plus tokens per closed item",
            json!({
                "yield": payload["m1_yield"],
                "by_seat": payload["m1_by_seat"],
                "closed_item": payload["k2_closed_item"],
            }),
        ),
        "m2" => ("returns / handoffs", payload["m2_return_rate"].clone()),
        "m3" => (
            "rework_chains / chains",
            payload["m3_rework_chains"].clone(),
        ),
        "m4" => ("impact is insufficient data", payload["m4_impact"].clone()),
        "tokens" => (
            "sum of four token buckets in the half-open window",
            payload["tokens"].clone(),
        ),
        "intake" => (
            "claim-inclusive to first structured write, exclusive",
            payload["intakes"].clone(),
        ),
        "medians" => (
            "even-sample median of wait seconds and intake tokens",
            payload["pair_medians"].clone(),
        ),
        "activity" => (
            "five disjoint seat bands clipped to the half-open window",
            payload["activity"].clone(),
        ),
        other => return Err(anyhow!("unknown metric id {}", redact(other))),
    };
    Ok(json!({
        "metric_id": metric_id,
        "formula_version": "acceptance-r1-a1",
        "formula": formula,
        "value": value,
        "inputs": {
            "tokens": payload["tokens"],
            "m1_yield": payload["m1_yield"],
            "m1_by_seat": payload["m1_by_seat"],
            "k2_closed_item": payload["k2_closed_item"],
            "m2_pairs": payload["m2_pairs"],
            "m2_return_rate": payload["m2_return_rate"],
            "m3_rework_chains": payload["m3_rework_chains"],
            "m3_chains": payload["m3_chains"],
            "rows": payload["trace_inputs"],
            "intakes": payload["intakes"],
            "pair_medians": payload["pair_medians"],
            "activity": payload["activity"],
            "activity_events": payload["activity_events"],
            "demand_events": payload["demand_events"],
            "destination_snapshots": payload["destination_snapshots"],
            "nonreturn_handoffs": payload["nonreturn_handoffs"],
            "window": {"from": from, "to": to},
        },
        "from": redact(from),
        "to": redact(to),
        "rig": rig.map(redact),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AnalyticsStore;

    #[test]
    fn report_json_does_not_invent_a_yield_for_an_empty_database() {
        let dir = std::env::temp_dir().join(format!("dendro-report-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let db = dir.join("analytics.duckdb");
        let store = AnalyticsStore::open_path(&db, "kid").unwrap();
        store.checkpoint().unwrap();
        drop(store);
        let report = report_json(
            &db,
            "decisions",
            Some("agentflow-dev"),
            "2026-10-08T00:00:00Z",
            "2026-10-11T00:00:00Z",
        )
        .unwrap();
        assert!(report["payload"]["m1_yield"].is_null());
        assert_eq!(
            report["payload"]["m4_impact"]
                .as_str()
                .unwrap()
                .contains("insufficient data"),
            true
        );
        assert!(report_json(
            &db,
            "not-a-kind",
            None,
            "2026-10-08T00:00:00Z",
            "2026-10-11T00:00:00Z"
        )
        .is_err());
        assert!(report_json(&db, "tokens", None, "not-a-time", "2026-10-11T00:00:00Z").is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_roots_fail_before_a_database_is_created() {
        let dir = std::env::temp_dir().join(format!("dendro-empty-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let db = dir.join("analytics.duckdb");
        let err = run_locked(&db, &[], |_, _| Ok(vec![])).unwrap_err();
        assert!(err.to_string().contains("source root"));
        assert!(!db.exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
