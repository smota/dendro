use std::fs::{self, File};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use crate::metrics::{self, Window};
use crate::output::{self, load_or_create_key, refuse_aliased_output, with_output_lock};
use crate::privacy::{opaque_id, redact};
use crate::storage::AnalyticsStore;

pub struct RunRequest<'a> {
    pub db: &'a Path,
    pub source_roots: &'a [PathBuf],
}

pub fn run_locked(db: &Path, source_roots: &[PathBuf]) -> Result<Value> {
    let parent = db.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let lock_path = parent.join(".dendro-output.lock");
    with_output_lock(&lock_path, || run_inner(db, source_roots))
}

fn run_inner(db: &Path, source_roots: &[PathBuf]) -> Result<Value> {
    refuse_aliased_output(db, source_roots)?;
    let key_path = PathBuf::from(format!("{}.key", db.display()));
    let creating = !db.exists();
    let key = load_or_create_key(&key_path, creating || key_path.exists())?;
    if creating && !key_path.exists() {
        return Err(anyhow!("key publication failed"));
    }
    let store = if db.exists() {
        AnalyticsStore::open_existing(db, &key.id)?
    } else {
        let parent = db.parent().unwrap_or_else(|| Path::new("."));
        let stage = parent.join(format!(".dendro-stage-{}", std::process::id()));
        let _ = fs::remove_dir_all(&stage);
        fs::create_dir_all(&stage)?;
        let staged = stage.join("analytics.duckdb");
        let store = AnalyticsStore::open_path(&staged, &key.id)?;
        store.checkpoint()?;
        drop(store);
        let staged_file = File::open(&staged)?;
        staged_file.sync_all()?;
        fs::rename(&staged, db)?;
        if let Some(parent) = db.parent() {
            output::sync_dir(parent)?;
        }
        let _ = fs::remove_dir_all(&stage);
        AnalyticsStore::open_existing(db, &key.id)?
    };
    let _ = store;
    Ok(json!({
        "status": "ready",
        "key_id": key.id,
        "sources": source_roots.iter().map(|p| redact(&p.display().to_string())).collect::<Vec<_>>(),
        "note": "ingest parsers attach in the connector layer",
    }))
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
    let store = AnalyticsStore::open_read_only(db)?;
    let usage = store.load_usage()?;
    let queue = store.load_queue()?;
    let edges = metrics::handoff_edges(&queue);
    let (rework, chains, paths) = metrics::m3_rework(&edges);
    let handoffs = edges.len() as i64;
    let returns = edges.iter().filter(|edge| edge.is_return).count() as i64;
    let (input, output, cache_read, cache_write) = metrics::token_totals(&usage);
    let tokens = input + output + cache_read + cache_write;
    let nonreturn = handoffs - returns;
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
            "trace_inputs": usage.iter().map(|row| &row.source_key).collect::<Vec<_>>(),
        }),
    ))
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
        let _ = fs::remove_dir_all(&dir);
    }
}
