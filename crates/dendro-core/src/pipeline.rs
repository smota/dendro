use std::fs::{self, File};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use crate::metrics::{self, Window};
use crate::output::{self, load_or_create_key, refuse_aliased_output, with_output_lock};
use crate::privacy::opaque_id;
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

pub fn report_json(
    db: &Path,
    kind: &str,
    rig: Option<&str>,
    from: &str,
    to: &str,
) -> Result<Value> {
    let window = parse_window(from, to)?;
    if !matches!(
        kind,
        "links" | "activity" | "handoffs" | "tokens" | "decisions"
    ) {
        return Err(anyhow!("unknown report kind {kind}"));
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
    let usage: Vec<_> = store
        .load_usage()?
        .into_iter()
        .filter(|row| match row.ts {
            Some(ts) => ts >= window.from && ts < window.to,
            None => false,
        })
        .filter(|row| match &allowed {
            Some(set) => set.contains(&(row.harness.clone(), row.native_session.clone())),
            None => true,
        })
        .collect();
    let queue: Vec<_> = store
        .load_queue()?
        .into_iter()
        .filter(|row| row.ts >= window.from && row.ts < window.to)
        .filter(|row| match &rig_seats {
            Some(seats) => seats.contains(&row.source_seat) || seats.contains(&row.dest_seat),
            None => true,
        })
        .collect();
    let edges = metrics::handoff_edges(&queue);
    let (rework, chains, paths) = metrics::m3_rework(&edges);
    let handoffs = edges.len() as i64;
    let returns = edges.iter().filter(|edge| edge.is_return).count() as i64;
    let (input, output, cache_read, cache_write) = metrics::token_totals(&usage);
    let tokens = input + output + cache_read + cache_write;
    let nonreturn = handoffs - returns;
    let activity = store.load_activity()?;
    let mut seats: Vec<String> = activity.iter().map(|row| row.seat.clone()).collect();
    seats.sort();
    seats.dedup();
    let activity_rows: Vec<_> = seats
        .iter()
        .map(|seat| {
            let bands = metrics::activity_bands(seat, &activity, &queue, window, None, None);
            json!({
                "seat": seat,
                "active_secs": bands.active_secs,
                "idle_with_work_secs": bands.idle_with_work_secs,
                "idle_without_work_secs": bands.idle_without_work_secs,
                "waiting_human_secs": bands.waiting_human_secs,
                "unknown_secs": bands.unknown_secs,
                "sum": bands.sum(),
            })
        })
        .collect();
    let tools = store.load_tools()?;
    let loops = metrics::tool_loops(&tools, 3, 600);
    let seat_of = |native: &str| {
        identities
            .iter()
            .find(|fact| fact.native_session == native)
            .map(|fact| fact.seat.clone())
    };
    let intakes = metrics::handoff_intakes(&edges, &queue, &tools, &usage, seat_of);
    let medians = metrics::pair_medians(&edges, &queue, &intakes);
    let mut mission_tokens = std::collections::BTreeMap::<String, i64>::new();
    for row in &usage {
        let seat = identities
            .iter()
            .find(|fact| fact.harness == row.harness && fact.native_session == row.native_session)
            .map(|fact| fact.seat.clone());
        let label = match (seat, row.ts) {
            (Some(seat), Some(ts)) => match metrics::mission_at(&seat, &queue, ts) {
                metrics::MissionBucket::Mission(name) => name,
                metrics::MissionBucket::None => "no_mission".into(),
                metrics::MissionBucket::Ambiguous => "ambiguous".into(),
            },
            _ => "no_mission".into(),
        };
        *mission_tokens.entry(label).or_insert(0) +=
            row.input_uncached + row.output + row.cache_read + row.cache_write;
    }
    let mut discovered = std::collections::BTreeSet::<(String, String)>::new();
    for row in &usage {
        discovered.insert((row.harness.clone(), row.native_session.clone()));
    }
    let links: Vec<_> = discovered
        .into_iter()
        .map(|(harness, native)| {
            let matches: Vec<_> = identities
                .iter()
                .filter(|fact| fact.harness == harness && fact.native_session == native)
                .collect();
            let status = if matches.is_empty() {
                "unlinked"
            } else {
                "linked"
            };
            json!({
                "harness": harness,
                "native_session": native,
                "status": status,
                "rig": matches.first().map(|fact| fact.rig.clone()),
                "seat": matches.first().map(|fact| fact.seat.clone()),
                "evidence": matches.iter().map(|fact| fact.source_key.clone()).collect::<Vec<_>>(),
            })
        })
        .collect::<Vec<_>>();
    Ok(metrics::report_value(
        kind,
        json!({
            "product": "AI Fleet Intelligence",
            "rig": rig,
            "from": from,
            "to": to,
            "usage_rows": usage.len(),
            "tokens": tokens,
            "m1_yield": metrics::m1_yield(tokens, nonreturn),
            "m2_return_rate": metrics::m2_return_rate(returns, handoffs),
            "m3_rework_chains": rework,
            "m3_chains": chains,
            "m3_paths": paths,
            "m4_impact": metrics::m4_impact(),
            "kind": kind,
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
            "trace_inputs": usage.iter().map(|row| json!({
                "source_key": row.source_key,
                "input_uncached": row.input_uncached,
                "output": row.output,
                "cache_read": row.cache_read,
                "cache_write": row.cache_write,
            })).collect::<Vec<_>>(),
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
        "m1" => ("tokens / nonreturn_handoffs", payload["m1_yield"].clone()),
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
        other => return Err(anyhow!("unknown metric id {other}")),
    };
    Ok(json!({
        "metric_id": metric_id,
        "formula_version": "acceptance-r1-a1",
        "formula": formula,
        "value": value,
        "inputs": {
            "tokens": payload["tokens"],
            "m1_yield": payload["m1_yield"],
            "m2_return_rate": payload["m2_return_rate"],
            "m3_rework_chains": payload["m3_rework_chains"],
            "m3_chains": payload["m3_chains"],
            "rows": payload["trace_inputs"],
            "intakes": payload["intakes"],
            "pair_medians": payload["pair_medians"],
            "window": {"from": from, "to": to},
        },
        "from": from,
        "to": to,
        "rig": rig,
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
