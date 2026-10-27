#![deny(unsafe_code)]

//! Connected HTTP & SSE Dashboard Server (`dendro dashboard --serve <ADDR>`).
//!
//! Provides dual-serving support for Dendro dashboards:
//! - Serves static web assets from `site/` with path traversal protections.
//! - Exposes uncapped live query API (`GET /api/query?view=<viewId>`).
//! - Streams live updates over Server-Sent Events (`GET /api/events?view=<viewId>`).
//!
//! Security: Only loopback interfaces (`127.0.0.1`, `localhost`, `[::1]`) are allowed.
//! External or wildcard binding (`0.0.0.0`) is deterministically refused.

use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tracing::{info, warn};

/// Validates that a bind address is strictly a loopback address.
pub fn validate_dashboard_bind_address(addr_str: &str) -> Result<SocketAddr> {
    let resolved = if let Some(rest) = addr_str.strip_prefix("localhost:") {
        format!("127.0.0.1:{rest}")
    } else if addr_str == "localhost" {
        "127.0.0.1:8080".to_string()
    } else {
        addr_str.to_string()
    };

    let socket_addr: SocketAddr = resolved
        .parse()
        .with_context(|| format!("invalid bind address '{addr_str}'"))?;

    if !socket_addr.ip().is_loopback() {
        bail!(
            "security refused: dashboard server must bind to a loopback address (127.0.0.1, localhost, [::1]); non-loopback bind address '{}' is forbidden",
            addr_str
        );
    }

    Ok(socket_addr)
}

/// Resolves the dashboard static asset root directory (`site/`).
pub fn resolve_site_dir(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(p) = explicit {
        if p.exists() {
            return Ok(p.to_path_buf());
        }
        bail!("specified --site-dir does not exist: {}", p.display());
    }

    let local = PathBuf::from("site");
    if local.join("dashboards").join("index.html").exists() {
        return Ok(local);
    }

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    if let Some(parent) = manifest_dir.parent().and_then(|p| p.parent()) {
        let repo_site = parent.join("site");
        if repo_site.join("dashboards").join("index.html").exists() {
            return Ok(repo_site);
        }
    }

    let hardcoded = PathBuf::from("/home/sam/code/dendro/site");
    if hardcoded.join("dashboards").join("index.html").exists() {
        return Ok(hardcoded);
    }

    Ok(local)
}

fn mime_type_for_path(path: &Path) -> &'static str {
    match path.extension().and_then(|s| s.to_str()).unwrap_or("") {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" | "cjs" => "application/javascript; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "ico" => "image/x-icon",
        "txt" => "text/plain; charset=utf-8",
        "yaml" | "yml" => "text/yaml; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn resolve_safe_path(site_dir: &Path, req_path: &str) -> Option<PathBuf> {
    let stripped = req_path.trim_start_matches('/');
    for comp in Path::new(stripped).components() {
        if let std::path::Component::ParentDir = comp {
            return None;
        }
    }
    let target = site_dir.join(stripped);
    if target.is_file() {
        Some(target)
    } else {
        None
    }
}

fn parse_query_string(query: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for pair in query.split('&') {
        let mut kv = pair.splitn(2, '=');
        if let (Some(k), Some(v)) = (kv.next(), kv.next()) {
            map.insert(k.to_string(), urlencoding_decode(v));
        }
    }
    map
}

fn urlencoding_decode(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(ch) = chars.next() {
        if ch == '%' {
            let h1 = chars.next();
            let h2 = chars.next();
            if let (Some(c1), Some(c2)) = (h1, h2) {
                let hex_str = format!("{c1}{c2}");
                if let Ok(byte) = u8::from_str_radix(&hex_str, 16) {
                    result.push(byte as char);
                    continue;
                }
                result.push('%');
                result.push(c1);
                result.push(c2);
            } else {
                result.push('%');
                if let Some(c1) = h1 {
                    result.push(c1);
                }
            }
        } else if ch == '+' {
            result.push(' ');
        } else {
            result.push(ch);
        }
    }
    result
}

fn get_file_meta(p: &Path) -> (u64, Option<SystemTime>) {
    if let Ok(m) = std::fs::metadata(p) {
        (m.len(), m.modified().ok())
    } else {
        (0, None)
    }
}

/// Executes an uncapped live query against DuckDB views or raw facts.
pub fn execute_live_query(
    db_path: &Path,
    view_id: &str,
    _window: Option<&str>,
    rig: Option<&str>,
) -> Result<serde_json::Value> {
    let canonical_view = match view_id {
        "seat_dynamics" | "v_seat_idle_ranking" => "v_seat_idle_ranking",
        "handoff_flow" | "v_handoff_rework_analysis" => "v_handoff_rework_analysis",
        "cost_and_loops" | "v_mission_token_burn" => "v_mission_token_burn",
        "executive_governance" | "v_executive_governance" => "v_executive_governance",
        "message_mix" | "v_message_mix" => "v_message_mix",
        "subagent_lineage" | "v_subagent_lineage" => "v_subagent_lineage",
        "security_posture" | "v_security_posture" => "v_security_posture",
        "process_resources" | "v_process_resources" => "v_process_resources",
        "trajectory_vortex" | "v_trajectory_vortex" => "v_trajectory_vortex",
        "schema_drift" | "v_schema_drift" => "v_schema_drift",
        "authority_blast_radius" | "v_authority_blast_radius" => "v_authority_blast_radius",
        "counterfactual_routing" | "v_counterfactual_routing" => "v_counterfactual_routing",
        other => other,
    };

    let empty_counts = serde_json::json!({
        "usage_facts": 0,
        "message_facts": 0,
        "activity_facts": 0,
        "queue_facts": 0,
        "security_facts": 0,
        "session_edges": 0,
        "tool_facts": 0,
        "skill_facts": 0
    });

    if !db_path.exists() {
        return Ok(serde_json::json!({
            "product": "AI Fleet Intelligence",
            "view": canonical_view,
            "live": true,
            "table_counts": empty_counts,
            "recorded_window": { "from": null, "to": null },
            "rows": [],
            "insights": {},
            "insights_catalog": []
        }));
    }

    let (db_target, store_opt) = match dendro_core::presentation::ReadRequest::open(db_path) {
        Ok(req) => {
            let s = req.open_store().ok();
            (req.database().to_path_buf(), s)
        }
        Err(_) => {
            let key_id: String = {
                let conn = duckdb::Connection::open(db_path).ok();
                conn.and_then(|c| {
                    c.query_row("SELECT key_id FROM dendro_meta", [], |r| r.get(0))
                        .ok()
                })
                .unwrap_or_else(|| "default".to_string())
            };
            let s = dendro_core::AnalyticsStore::open_existing(db_path, &key_id)
                .or_else(|_| dendro_core::AnalyticsStore::open_path(db_path, &key_id))
                .ok();
            (db_path.to_path_buf(), s)
        }
    };

    // Evaluate all compiled insights I01–I25 if a store handle is available
    let catalog = if let Some(ref store) = store_opt {
        dendro_core::insight_v1::evaluate_catalog(store, &dendro_core::insight_v1::Scope::default())
    } else {
        Vec::new()
    };
    let mut insights_list = Vec::new();
    let mut insights_map = serde_json::Map::new();

    for item in &catalog {
        let l0_map: serde_json::Map<String, serde_json::Value> = item
            .disclosure_l0()
            .fields
            .into_iter()
            .map(|(k, v)| (k.to_string(), serde_json::Value::String(v)))
            .collect();
        let l1_map: serde_json::Map<String, serde_json::Value> = item
            .disclosure_l1()
            .fields
            .into_iter()
            .map(|(k, v)| (k.to_string(), serde_json::Value::String(v)))
            .collect();
        let l2_map: serde_json::Map<String, serde_json::Value> = item
            .disclosure_l2()
            .fields
            .into_iter()
            .map(|(k, v)| (k.to_string(), serde_json::Value::String(v)))
            .collect();
        let l3_map: serde_json::Map<String, serde_json::Value> = item
            .disclosure_l3()
            .fields
            .into_iter()
            .map(|(k, v)| (k.to_string(), serde_json::Value::String(v)))
            .collect();

        let status_str = match &item.status {
            dendro_core::insight_v1::InsightStatus::Computed => "Computed",
            dendro_core::insight_v1::InsightStatus::Partial { .. } => "Partial",
            dendro_core::insight_v1::InsightStatus::Refused { .. } => "Refused",
        };

        let val = serde_json::json!({
            "id": item.id,
            "title": item.name,
            "name": item.name,
            "layer": format!("{:?}", item.domain).to_lowercase(),
            "domain": format!("{:?}", item.domain).to_lowercase(),
            "badge": dendro_core::insight_v1::display_status(&item.status),
            "status": status_str,
            "l0": l0_map,
            "l1": l1_map,
            "l2": l2_map,
            "l3": l3_map,
            "measures": item.measures,
            "population": item.population,
            "coverage": item.coverage,
            "evidence": item.evidence,
        });

        insights_list.push(val.clone());
        insights_map.insert(item.id.to_string(), val);
    }

    let query_conn = match duckdb::Connection::open_with_flags(
        &db_target,
        duckdb::Config::default()
            .access_mode(duckdb::AccessMode::ReadOnly)
            .unwrap_or_default(),
    ) {
        Ok(c) => c,
        Err(_) => duckdb::Connection::open(&db_target)?,
    };

    let conn = &query_conn;
    {
        // Compute exact counts for all 8 fact tables
        let table_names = [
            "usage_facts",
            "message_facts",
            "activity_facts",
            "queue_facts",
            "security_facts",
            "session_edges",
            "tool_facts",
            "skill_facts",
        ];
        let mut table_counts = serde_json::Map::new();
        for tbl in &table_names {
            let exists: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM information_schema.tables WHERE lower(table_name) = lower(?)",
                    duckdb::params![tbl],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            let count: i64 = if exists > 0 {
                conn.query_row(&format!("SELECT COUNT(*) FROM \"{tbl}\""), [], |r| r.get(0))
                    .unwrap_or(0)
            } else {
                0
            };
            table_counts.insert(tbl.to_string(), serde_json::json!(count));
        }

        // Derive recorded window min/max timestamp
        let mut min_ts: Option<String> = None;
        let mut max_ts: Option<String> = None;
        let has_usage = table_counts
            .get("usage_facts")
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
            > 0;
        if has_usage {
            if let Ok((min_val, max_val)) = conn.query_row(
                "SELECT MIN(ts)::VARCHAR, MAX(ts)::VARCHAR FROM usage_facts",
                [],
                |r| {
                    Ok((
                        r.get::<_, Option<String>>(0)?,
                        r.get::<_, Option<String>>(1)?,
                    ))
                },
            ) {
                min_ts = min_val;
                max_ts = max_val;
            }
        }
        let recorded_window = serde_json::json!({
            "from": min_ts,
            "to": max_ts,
        });

        // 1. Hub / General / Status live summary
        if matches!(canonical_view, "hub" | "general" | "status") {
            let mut harnesses = Vec::new();
            if has_usage {
                if let Ok(mut stmt) = conn.prepare(
                    "SELECT harness, COUNT(*)::BIGINT as usage_rows, COALESCE(SUM(input_uncached + output_tokens + cache_read + cache_write), 0)::BIGINT as total_tokens FROM usage_facts GROUP BY harness ORDER BY total_tokens DESC",
                ) {
                    let rows = stmt.query_map([], |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, i64>(1)?,
                            r.get::<_, i64>(2)?,
                        ))
                    });
                    if let Ok(iter) = rows {
                        for item in iter.flatten() {
                            let msg_rows: i64 = if table_counts
                                .get("message_facts")
                                .and_then(|v| v.as_i64())
                                .unwrap_or(0)
                                > 0
                            {
                                conn.query_row(
                                    "SELECT COUNT(*) FROM message_facts WHERE harness = ?",
                                    duckdb::params![item.0],
                                    |r| r.get(0),
                                )
                                .unwrap_or(0)
                            } else {
                                0
                            };
                            harnesses.push(serde_json::json!({
                                "harness": item.0,
                                "usage_rows": item.1,
                                "total_tokens": item.2,
                                "message_rows": msg_rows,
                            }));
                        }
                    }
                }
            }

            let mut rigs = Vec::new();
            let mut scopes = Vec::new();
            if table_counts
                .get("queue_facts")
                .and_then(|v| v.as_i64())
                .unwrap_or(0)
                > 0
            {
                if let Ok(mut stmt) = conn.prepare(
                    "SELECT COALESCE(rig, 'default') as r, COUNT(DISTINCT seat)::BIGINT as s, COUNT(*)::BIGINT as q FROM queue_facts GROUP BY rig",
                ) {
                    if let Ok(iter) = stmt.query_map([], |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, i64>(1)?,
                            r.get::<_, i64>(2)?,
                        ))
                    }) {
                        for (r, s, q) in iter.flatten() {
                            rigs.push(serde_json::json!({
                                "rig": r,
                                "seats": s,
                                "queue_items": q,
                            }));
                            scopes.push(serde_json::json!({
                                "scope": r,
                                "scope_id": r,
                                "actors": s,
                                "queue_items": q,
                            }));
                        }
                    }
                }
            }

            let total_tokens: i64 = if has_usage {
                conn.query_row(
                    "SELECT COALESCE(SUM(input_uncached + output_tokens + cache_read + cache_write), 0)::BIGINT FROM usage_facts",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(0)
            } else {
                0
            };
            let active_seats: i64 = if table_counts
                .get("activity_facts")
                .and_then(|v| v.as_i64())
                .unwrap_or(0)
                > 0
            {
                conn.query_row(
                    "SELECT COUNT(DISTINCT seat)::BIGINT FROM activity_facts",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(0)
            } else {
                0
            };

            return Ok(serde_json::json!({
                "product": "AI Fleet Intelligence",
                "view": "hub",
                "live": true,
                "table_counts": table_counts,
                "recorded_window": recorded_window,
                "harnesses": harnesses,
                "rigs": rigs,
                "scopes": scopes,
                "insights": insights_list,
                "insights_by_id": insights_map,
                "kpis": {
                    "total_tokens": total_tokens,
                    "active_seats": active_seats,
                    "active_actors": active_seats,
                    "m1_yield": if active_seats > 0 { (total_tokens as f64) / (active_seats as f64) } else { 0.0 },
                    "handoff_friction_rate": 0.0,
                    "effective_concurrency": 1.0,
                    "rework_returns": 0
                }
            }));
        }

        // 2. Cockpit / Viewer export
        if matches!(canonical_view, "cockpit" | "viewer") {
            let (from_str, to_str) = match (&min_ts, &max_ts) {
                (Some(f), Some(t)) => (f.as_str(), t.as_str()),
                _ => ("2026-01-01T00:00:00Z", "2026-12-31T23:59:59Z"),
            };
            let mut report = match dendro_core::pipeline::report_json_physical(
                &db_target, "cockpit", rig, from_str, to_str,
            ) {
                Ok(r) => r,
                Err(_) => serde_json::json!({
                    "product": "AI Fleet Intelligence",
                    "from": from_str,
                    "to": to_str,
                    "tokens": 0,
                    "m1_yield": 0,
                    "m3_rework_chains": 0,
                    "m1_by_seat": [],
                    "activity": [],
                    "activity_events": [],
                    "handoff_edges": [],
                    "pair_medians": [],
                    "token_categories": { "input_uncached": 0, "cache_read": 0, "cache_write": 0, "output": 0 },
                    "fleet_concurrency": { "effective_parallelism": 1.0, "histogram": [] },
                    "insights": insights_list,
                    "security": { "status": { "state": "computed" }, "counts": [] }
                }),
            };
            if let Some(obj) = report.as_object_mut() {
                obj.insert("live".to_string(), serde_json::json!(true));
                obj.insert(
                    "table_counts".to_string(),
                    serde_json::Value::Object(table_counts),
                );
                obj.insert("recorded_window".to_string(), recorded_window);
                if !obj.contains_key("insights")
                    || obj
                        .get("insights")
                        .and_then(|v| v.as_array())
                        .map(|a| a.is_empty())
                        .unwrap_or(true)
                {
                    obj.insert("insights".to_string(), serde_json::json!(insights_list));
                }
            }
            return Ok(report);
        }

        // 3. Uncapped View Queries (with semantic view query or facts table fallback)
        let rows = query_uncapped_view_or_fallback(conn, canonical_view)?;

        let mut resp = serde_json::json!({
            "product": "AI Fleet Intelligence",
            "view": canonical_view,
            "live": true,
            "rows": rows,
            "table_counts": table_counts,
            "recorded_window": recorded_window,
            "insights": insights_map,
            "insights_list": insights_list,
        });

        if canonical_view == "v_trajectory_vortex" {
            resp["i22"] = insights_map
                .get("I22")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
        } else if canonical_view == "v_schema_drift" {
            resp["i23"] = insights_map
                .get("I23")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
        } else if canonical_view == "v_authority_blast_radius" {
            resp["i24"] = insights_map
                .get("I24")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
        } else if canonical_view == "v_counterfactual_routing" {
            resp["i25"] = insights_map
                .get("I25")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
        }

        Ok(resp)
    }
}

fn enrich_row_actor_scope_aliases(map: &mut serde_json::Map<String, serde_json::Value>) {
    if let Some(seat) = map.get("seat").cloned() {
        if !map.contains_key("actor_id") {
            map.insert("actor_id".to_string(), seat);
        }
    }
    if let Some(seat_id) = map.get("seat_id").cloned() {
        if !map.contains_key("actor_id") {
            map.insert("actor_id".to_string(), seat_id);
        }
    }
    if let Some(from) = map.get("from").cloned() {
        if !map.contains_key("from_actor") {
            map.insert("from_actor".to_string(), from);
        }
    }
    if let Some(to) = map.get("to").cloned() {
        if !map.contains_key("to_actor") {
            map.insert("to_actor".to_string(), to);
        }
    }
    if let Some(rig) = map.get("rig").cloned() {
        if !map.contains_key("scope_id") {
            map.insert("scope_id".to_string(), rig.clone());
        }
        if !map.contains_key("scope") {
            map.insert("scope".to_string(), rig);
        }
    }
}

fn query_uncapped_view_or_fallback(
    conn: &duckdb::Connection,
    view_name: &str,
) -> Result<Vec<serde_json::Value>> {
    // Check if view or table exists in DuckDB
    let exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM information_schema.tables WHERE lower(table_name) = lower(?)",
            duckdb::params![view_name],
            |r| r.get(0),
        )
        .unwrap_or(0);

    if exists > 0 {
        if let Ok(mut stmt) = conn.prepare(&format!("SELECT * FROM \"{view_name}\"")) {
            if let Ok(mapped) = stmt.query_map([], |row| {
                let mut map = serde_json::Map::new();
                let col_names = row.as_ref().column_names();
                for (i, name) in col_names.into_iter().enumerate() {
                    let v_ref = row.get_ref(i).unwrap_or(duckdb::types::ValueRef::Null);
                    map.insert(name, crate::serve_v1::value_ref_to_json(v_ref));
                }
                enrich_row_actor_scope_aliases(&mut map);
                Ok(serde_json::Value::Object(map))
            }) {
                let results: Vec<serde_json::Value> = mapped.filter_map(Result::ok).collect();
                if !results.is_empty() {
                    return Ok(results);
                }
            }
        }
    }

    // Materialized view or raw facts table fallback
    match view_name {
        "v_message_mix" => {
            let has_msg: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM information_schema.tables WHERE lower(table_name) = 'message_facts'",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            if has_msg > 0 {
                let mut stmt = conn.prepare(
                    "SELECT harness, native_session, role, direction, modalities AS modality, COUNT(*)::BIGINT AS message_count FROM message_facts GROUP BY harness, native_session, role, direction, modalities ORDER BY message_count DESC, harness, native_session",
                )?;
                let mapped = stmt.query_map([], |row| {
                    let mut map = serde_json::Map::new();
                    let col_names = row.as_ref().column_names();
                    for (i, name) in col_names.into_iter().enumerate() {
                        let v_ref = row.get_ref(i).unwrap_or(duckdb::types::ValueRef::Null);
                        map.insert(name, crate::serve_v1::value_ref_to_json(v_ref));
                    }
                    enrich_row_actor_scope_aliases(&mut map);
                    Ok(serde_json::Value::Object(map))
                })?;
                return Ok(mapped.filter_map(Result::ok).collect());
            }
        }
        "v_subagent_lineage" => {
            let has_edge: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM information_schema.tables WHERE lower(table_name) = 'session_edges'",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            if has_edge > 0 {
                let mut stmt = conn.prepare(
                    "SELECT harness, parent_session, COUNT(DISTINCT CASE WHEN child_session <> '' THEN child_session END)::BIGINT AS child_count, COUNT(*)::BIGINT AS edge_count, (COUNT(*) FILTER (WHERE status = 'started'))::BIGINT AS started_count, (COUNT(*) FILTER (WHERE status = 'finished'))::BIGINT AS finished_count, (COUNT(*) FILTER (WHERE status = 'failed'))::BIGINT AS failed_count, (COUNT(*) FILTER (WHERE status = 'unknown'))::BIGINT AS unknown_count FROM session_edges GROUP BY harness, parent_session",
                )?;
                let mapped = stmt.query_map([], |row| {
                    let mut map = serde_json::Map::new();
                    let col_names = row.as_ref().column_names();
                    for (i, name) in col_names.into_iter().enumerate() {
                        let v_ref = row.get_ref(i).unwrap_or(duckdb::types::ValueRef::Null);
                        map.insert(name, crate::serve_v1::value_ref_to_json(v_ref));
                    }
                    enrich_row_actor_scope_aliases(&mut map);
                    Ok(serde_json::Value::Object(map))
                })?;
                return Ok(mapped.filter_map(Result::ok).collect());
            }
        }
        "v_mission_token_burn" => {
            let has_usage: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM information_schema.tables WHERE lower(table_name) = 'usage_facts'",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            if has_usage > 0 {
                let mut stmt = conn.prepare(
                    "SELECT harness, model, SUM(input_uncached)::BIGINT as input_uncached, SUM(output_tokens)::BIGINT as output, SUM(cache_read)::BIGINT as cache_read, SUM(cache_write)::BIGINT as cache_write, SUM(currency_total) as currency_total, COUNT(*)::BIGINT as count FROM usage_facts GROUP BY harness, model",
                )?;
                let mapped = stmt.query_map([], |row| {
                    let mut map = serde_json::Map::new();
                    let col_names = row.as_ref().column_names();
                    for (i, name) in col_names.into_iter().enumerate() {
                        let v_ref = row.get_ref(i).unwrap_or(duckdb::types::ValueRef::Null);
                        map.insert(name, crate::serve_v1::value_ref_to_json(v_ref));
                    }
                    enrich_row_actor_scope_aliases(&mut map);
                    Ok(serde_json::Value::Object(map))
                })?;
                return Ok(mapped.filter_map(Result::ok).collect());
            }
        }
        "v_security_posture" => {
            let has_sec: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM information_schema.tables WHERE lower(table_name) = 'security_facts'",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            if has_sec > 0 {
                let mut stmt = conn.prepare(
                    "SELECT harness, native_session, kind, value, source_field, COUNT(*)::BIGINT as count FROM security_facts GROUP BY harness, native_session, kind, value, source_field",
                )?;
                let mapped = stmt.query_map([], |row| {
                    let mut map = serde_json::Map::new();
                    let col_names = row.as_ref().column_names();
                    for (i, name) in col_names.into_iter().enumerate() {
                        let v_ref = row.get_ref(i).unwrap_or(duckdb::types::ValueRef::Null);
                        map.insert(name, crate::serve_v1::value_ref_to_json(v_ref));
                    }
                    enrich_row_actor_scope_aliases(&mut map);
                    Ok(serde_json::Value::Object(map))
                })?;
                return Ok(mapped.filter_map(Result::ok).collect());
            }
        }
        "v_seat_idle_ranking" => {
            let has_act: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM information_schema.tables WHERE lower(table_name) = 'activity_facts'",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            if has_act > 0 {
                let mut stmt = conn.prepare(
                    "SELECT seat, seat AS actor_id, state, COUNT(*)::BIGINT as count FROM activity_facts GROUP BY seat, state",
                )?;
                let mapped = stmt.query_map([], |row| {
                    let mut map = serde_json::Map::new();
                    let col_names = row.as_ref().column_names();
                    for (i, name) in col_names.into_iter().enumerate() {
                        let v_ref = row.get_ref(i).unwrap_or(duckdb::types::ValueRef::Null);
                        map.insert(name, crate::serve_v1::value_ref_to_json(v_ref));
                    }
                    enrich_row_actor_scope_aliases(&mut map);
                    Ok(serde_json::Value::Object(map))
                })?;
                return Ok(mapped.filter_map(Result::ok).collect());
            }
        }
        _ => {}
    }

    Ok(vec![])
}

/// Connected HTTP & SSE Dashboard Server instance.
pub struct ConnectedServer {
    listener: TcpListener,
    db_path: PathBuf,
    site_dir: PathBuf,
    rig: Option<String>,
    window: Option<String>,
}

impl ConnectedServer {
    pub async fn bind(
        addr_str: &str,
        db_path: PathBuf,
        site_dir: PathBuf,
        rig: Option<String>,
        window: Option<String>,
    ) -> Result<Self> {
        let socket_addr = validate_dashboard_bind_address(addr_str)?;
        let listener = TcpListener::bind(socket_addr)
            .await
            .with_context(|| format!("failed to bind dashboard server to {socket_addr}"))?;
        Ok(Self {
            listener,
            db_path,
            site_dir,
            rig,
            window,
        })
    }

    pub fn local_addr(&self) -> Result<SocketAddr> {
        self.listener
            .local_addr()
            .context("failed to query local address")
    }

    pub async fn run(self) -> Result<()> {
        let local_addr = self.local_addr()?;
        info!("Dendro Connected Dashboard Server active at http://{local_addr}/");

        loop {
            let (socket, _) = match self.listener.accept().await {
                Ok(conn) => conn,
                Err(e) => {
                    warn!("dashboard server accept error: {e}");
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    continue;
                }
            };

            let db = self.db_path.clone();
            let site = self.site_dir.clone();
            let rig = self.rig.clone();
            let window = self.window.clone();

            tokio::spawn(async move {
                if let Err(e) = handle_http_connection(socket, db, site, rig, window).await {
                    tracing::debug!("dashboard connection ended: {e}");
                }
            });
        }
    }
}

async fn handle_http_connection(
    mut stream: TcpStream,
    db_path: PathBuf,
    site_dir: PathBuf,
    rig: Option<String>,
    window: Option<String>,
) -> Result<()> {
    let mut buf = [0u8; 4096];
    let n = stream.read(&mut buf).await?;
    if n == 0 {
        return Ok(());
    }

    let req_str = String::from_utf8_lossy(&buf[..n]);
    let first_line = req_str.lines().next().unwrap_or("");
    let mut parts = first_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let uri = parts.next().unwrap_or("/");

    if method != "GET" && method != "HEAD" {
        let resp =
            "HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
        stream.write_all(resp.as_bytes()).await?;
        return Ok(());
    }

    let (path, query) = match uri.find('?') {
        Some(idx) => (&uri[..idx], &uri[idx + 1..]),
        None => (uri, ""),
    };

    // 1. Root redirect
    if path == "/" {
        let resp = "HTTP/1.1 302 Found\r\nLocation: /dashboards/index.html\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
        stream.write_all(resp.as_bytes()).await?;
        return Ok(());
    }

    // 2. Uncapped Live Query API
    if path == "/api/query" {
        let params = parse_query_string(query);
        let view_id = params.get("view").map(|s| s.as_str()).unwrap_or("hub");
        let query_rig = params.get("rig").cloned().or_else(|| rig.clone());
        let query_window = params.get("window").cloned().or_else(|| window.clone());

        let payload = match execute_live_query(
            &db_path,
            view_id,
            query_window.as_deref(),
            query_rig.as_deref(),
        ) {
            Ok(p) => p,
            Err(e) => serde_json::json!({
                "error": e.to_string(),
                "live": true,
                "view": view_id,
                "rows": [],
                "table_counts": {},
            }),
        };

        let body = serde_json::to_vec(&payload)?;
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(header.as_bytes()).await?;
        if method == "GET" {
            stream.write_all(&body).await?;
        }
        return Ok(());
    }

    // 3. Live SSE Stream
    if path == "/api/events" {
        let params = parse_query_string(query);
        let view_id = params.get("view").map(|s| s.as_str()).unwrap_or("hub");
        let query_rig = params.get("rig").cloned().or_else(|| rig.clone());
        let query_window = params.get("window").cloned().or_else(|| window.clone());

        let header = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\nAccess-Control-Allow-Origin: *\r\n\r\n";
        stream.write_all(header.as_bytes()).await?;

        let initial_payload = match execute_live_query(
            &db_path,
            view_id,
            query_window.as_deref(),
            query_rig.as_deref(),
        ) {
            Ok(p) => p,
            Err(e) => serde_json::json!({
                "error": e.to_string(),
                "live": true,
                "view": view_id,
                "rows": [],
            }),
        };
        let init_json = serde_json::to_string(&initial_payload)?;
        let init_msg = format!("data: {init_json}\n\n");
        stream.write_all(init_msg.as_bytes()).await?;
        stream.flush().await?;

        let mut last_meta = get_file_meta(&db_path);

        loop {
            tokio::time::sleep(Duration::from_secs(4)).await;

            let current_meta = get_file_meta(&db_path);
            if current_meta != last_meta {
                last_meta = current_meta;
                if let Ok(updated) = execute_live_query(
                    &db_path,
                    view_id,
                    query_window.as_deref(),
                    query_rig.as_deref(),
                ) {
                    if let Ok(json_str) = serde_json::to_string(&updated) {
                        let msg = format!("data: {json_str}\n\n");
                        if stream.write_all(msg.as_bytes()).await.is_err() {
                            break;
                        }
                        if stream.flush().await.is_err() {
                            break;
                        }
                    }
                }
            } else {
                if stream.write_all(b": keepalive\n\n").await.is_err() {
                    break;
                }
                if stream.flush().await.is_err() {
                    break;
                }
            }
        }
        return Ok(());
    }

    // 4. Static site assets
    if let Some(file_path) = resolve_safe_path(&site_dir, path) {
        if let Ok(content) = tokio::fs::read(&file_path).await {
            let mime = mime_type_for_path(&file_path);
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n",
                content.len()
            );
            stream.write_all(header.as_bytes()).await?;
            if method == "GET" {
                stream.write_all(&content).await?;
            }
            return Ok(());
        }
    }

    let not_found =
        "HTTP/1.1 404 Not Found\r\nContent-Length: 9\r\nConnection: close\r\n\r\nNot Found";
    stream.write_all(not_found.as_bytes()).await?;
    Ok(())
}

/// Starts the dashboard server from CLI arguments.
pub async fn start_dashboard_server(
    db: &Path,
    bind_addr: &str,
    site_dir_override: Option<&Path>,
    rig: Option<&str>,
    window: Option<&str>,
    _from: Option<&str>,
    _to: Option<&str>,
) -> Result<()> {
    let site_dir = resolve_site_dir(site_dir_override)?;
    let server = ConnectedServer::bind(
        bind_addr,
        db.to_path_buf(),
        site_dir,
        rig.map(|s| s.to_string()),
        window.map(|s| s.to_string()),
    )
    .await?;

    let addr = server.local_addr()?;
    println!("Dendro Connected Dashboard Server active at http://{addr}/");
    println!("Press Ctrl+C to stop.");
    server.run().await
}
