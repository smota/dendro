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
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tracing::{info, warn};

const NO_CACHE_HEADERS: &str =
    "Cache-Control: no-store, no-cache, must-revalidate, max-age=0\r\nPragma: no-cache\r\nExpires: 0\r\n";

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
    window: Option<&str>,
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
        "direct_overview" | "v_direct_overview" => "direct_overview",
        "cwd_projects" | "v_cwd_fingerprint_rollup" => "v_cwd_fingerprint_rollup",
        other => other,
    };

    if !db_path.exists() {
        let mut payload = query_live_payload(
            &duckdb::Connection::open_in_memory()?,
            canonical_view,
            window,
            rig,
        )?;
        payload.as_object_mut().unwrap().remove("query_to");
        return Ok(payload);
    }

    let (db_target, store_opt) = match dendro_core::presentation::ReadRequest::open(db_path) {
        Ok(req) => {
            let s = req.open_store().ok();
            (req.database().to_path_buf(), s)
        }
        Err(_) => (
            db_path.to_path_buf(),
            dendro_core::AnalyticsStore::open_read_only(db_path).ok(),
        ),
    };

    let query_conn = duckdb::Connection::open_with_flags(
        &db_target,
        duckdb::Config::default().access_mode(duckdb::AccessMode::ReadOnly)?,
    )?;
    let mut payload = query_live_payload(&query_conn, canonical_view, window, rig)?;
    let from = payload["active_window"]["from"]
        .as_str()
        .unwrap_or_default();
    let to = payload["query_to"].as_str().unwrap_or_default();
    let scope = dendro_core::insight_v1::Scope {
        from: Some(from.to_owned()),
        to: Some(to.to_owned()),
        ..Default::default()
    };
    let catalog = store_opt
        .as_ref()
        .map(|store| dendro_core::insight_v1::evaluate_catalog(store, &scope))
        .unwrap_or_default();
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

    if matches!(canonical_view, "cockpit" | "viewer") {
        // The physical report also uses a half-open window. Include the newest
        // event for trailing/all windows, while preserving explicit month boundaries.
        let to = payload["query_to"].as_str().unwrap_or(to);
        if let Ok(report) =
            dendro_core::pipeline::report_json_physical(&db_target, "cockpit", rig, from, to)
        {
            if let Some(fields) = report.as_object() {
                for (key, value) in fields {
                    payload[key] = value.clone();
                }
            }
        }
    }
    payload["insights"] = if matches!(canonical_view, "hub" | "general" | "status") {
        serde_json::json!(insights_list)
    } else {
        serde_json::json!(insights_map)
    };
    payload["insights_list"] = serde_json::json!(insights_list);
    payload["insights_catalog"] = serde_json::json!(insights_list);
    payload["insights_by_id"] = serde_json::json!(insights_map);
    for (view, id, key) in [
        ("v_trajectory_vortex", "I22", "i22"),
        ("v_schema_drift", "I23", "i23"),
        ("v_authority_blast_radius", "I24", "i24"),
        ("v_counterfactual_routing", "I25", "i25"),
    ] {
        if canonical_view == view {
            payload[key] = insights_map.get(id).cloned().unwrap_or_default();
        }
    }
    if let Some(fields) = payload.as_object_mut() {
        fields.remove("query_to");
    }
    Ok(payload)
}

// Temporary views are private to this request's connection. They filter every
// downstream query without changing telemetry or persisted semantic snapshots.
const LIVE_FACTS: &[(&str, &str, &str)] = &[
    ("usage_facts", "ts", "source_key VARCHAR, harness VARCHAR, native_session VARCHAR, ts TIMESTAMP, input_uncached BIGINT, output_tokens BIGINT, cache_read BIGINT, cache_write BIGINT, currency_total DOUBLE, model VARCHAR, seq BIGINT"),
    ("message_facts", "ts", "source_key VARCHAR, harness VARCHAR, native_session VARCHAR, ts TIMESTAMP, role VARCHAR, direction VARCHAR, modalities VARCHAR"),
    ("activity_facts", "ts", "source_key VARCHAR, seat VARCHAR, ts TIMESTAMP, state VARCHAR, seq BIGINT"),
    ("queue_facts", "ts", "source_key VARCHAR, transition_id BIGINT, qitem_id VARCHAR, ts TIMESTAMP, state VARCHAR, source_seat VARCHAR, dest_seat VARCHAR, closure_reason VARCHAR, closure_target VARCHAR, is_recovery BOOLEAN, recovery_reason VARCHAR, issue_tags VARCHAR, chain_id VARCHAR, parent_qitem_id VARCHAR"),
    ("security_facts", "ts", "source_key VARCHAR, harness VARCHAR, native_session VARCHAR, ts TIMESTAMP, kind VARCHAR, value VARCHAR, source_field VARCHAR"),
    ("session_edges", "observed_at", "source_key VARCHAR, harness VARCHAR, parent_session VARCHAR, child_session VARCHAR, observed_at TIMESTAMP, relation VARCHAR, status VARCHAR"),
    ("tool_facts", "ts", "source_key VARCHAR, harness VARCHAR, native_session VARCHAR, ts TIMESTAMP, failed BOOLEAN, category VARCHAR, tool_name VARCHAR"),
    ("skill_facts", "", "source_key VARCHAR, seat VARCHAR, skill VARCHAR, invoked BOOLEAN"),
    ("identity_facts", "", "source_key VARCHAR, harness VARCHAR, native_session VARCHAR, rig VARCHAR, seat VARCHAR, ts TIMESTAMP"),
    ("process_identities", "last_seen", "identity_key VARCHAR, root_key VARCHAR, harness VARCHAR, attribution VARCHAR, session_link VARCHAR, cwd_fingerprint VARCHAR, first_seen TIMESTAMP, last_seen TIMESTAMP, ended_at TIMESTAMP"),
    ("process_samples", "ts", "identity_key VARCHAR, ts TIMESTAMP, rss_unit VARCHAR, rss_bytes BIGINT"),
    ("envelope_fingerprints", "", "harness VARCHAR, ts TIMESTAMP, fingerprint VARCHAR, harness_version VARCHAR"),
];

fn sql_identifier(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn json_rows(
    conn: &duckdb::Connection,
    sql: &str,
    params: impl duckdb::Params,
) -> Result<Vec<serde_json::Value>> {
    // DuckDB serializes nested lists/JSON too; the generic scalar converter
    // deliberately marks lists unsupported and would lose source-key lineage.
    let mut stmt = conn.prepare(&format!(
        "SELECT to_json(result)::VARCHAR FROM ({sql}) result"
    ))?;
    let rows = stmt.query_map(params, |row| row.get::<_, String>(0))?;
    rows.map(|row| Ok(serde_json::from_str(&row?)?)).collect()
}

fn parse_live_timestamp(value: &str) -> Result<chrono::DateTime<chrono::Utc>> {
    if let Ok(ts) = chrono::DateTime::parse_from_rfc3339(value) {
        return Ok(ts.with_timezone(&chrono::Utc));
    }
    if let Ok(date) = chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d") {
        return Ok(date.and_hms_opt(0, 0, 0).unwrap().and_utc());
    }
    for format in ["%Y-%m-%dT%H:%M", "%Y-%m-%dT%H:%M:%S%.f"] {
        if let Ok(ts) = chrono::NaiveDateTime::parse_from_str(value, format) {
            return Ok(ts.and_utc());
        }
    }
    Ok(
        chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S%.f")
            .with_context(|| format!("invalid timestamp '{value}'"))?
            .and_utc(),
    )
}

fn iso(ts: chrono::DateTime<chrono::Utc>) -> String {
    ts.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true)
}

fn window_labels(
    from: Option<chrono::DateTime<chrono::Utc>>,
    to: Option<chrono::DateTime<chrono::Utc>>,
) -> serde_json::Value {
    let display = from.zip(to).map(|(from, to)| {
        if from.date_naive() == to.date_naive() {
            from.format("%b %-d, %Y").to_string()
        } else if from.format("%Y").to_string() == to.format("%Y").to_string() {
            format!("{} – {}", from.format("%b %-d"), to.format("%b %-d, %Y"))
        } else {
            format!(
                "{} – {}",
                from.format("%b %-d, %Y"),
                to.format("%b %-d, %Y")
            )
        }
    });
    serde_json::json!({
        "from":from.map(iso), "to":to.map(iso),
        "from_date":from.map(|t| t.format("%Y-%m-%d").to_string()),
        "to_date":to.map(|t| t.format("%Y-%m-%d").to_string()),
        "display":display
    })
}

struct LiveWindow {
    preset: String,
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
    include_end: bool,
}

impl LiveWindow {
    fn resolve(window: Option<&str>, from: Option<&str>, to: Option<&str>) -> Result<Self> {
        let preset = window.unwrap_or("all");
        let anchor = to
            .map(parse_live_timestamp)
            .transpose()?
            .unwrap_or_else(chrono::Utc::now);
        let (start, end, include_end) = if let Some((f, t)) = preset.split_once("..") {
            // Date-only upper bounds cover the entire selected day, including
            // fractional seconds. Timestamp ranges retain half-open semantics.
            let end = parse_live_timestamp(t)?;
            let end = if t.len() == 10 {
                end.checked_add_signed(chrono::Duration::days(1))
                    .context("window end date out of range")?
            } else {
                end
            };
            let bounds = (parse_live_timestamp(f)?, end);
            if bounds.0 >= bounds.1 {
                bail!("window from must precede to");
            }
            (bounds.0, bounds.1, false)
        } else if let Some((f, t)) =
            dendro_core::presentation::calendar_window_bounds(preset, anchor)
        {
            (f, t, preset != "last_month")
        } else {
            let duration = match preset {
                "1h" => chrono::Duration::hours(1),
                "24h" => chrono::Duration::days(1),
                "7d" => chrono::Duration::days(7),
                "30d" => chrono::Duration::days(30),
                "all" => chrono::Duration::days(1),
                _ => bail!("unknown dashboard window '{preset}'"),
            };
            let end = if matches!(preset, "7d" | "30d") {
                anchor
                    .date_naive()
                    .succ_opt()
                    .context("window anchor out of range")?
                    .and_hms_opt(0, 0, 0)
                    .unwrap()
                    .and_utc()
            } else if matches!(preset, "1h" | "24h") {
                use chrono::Timelike;
                let hour = anchor
                    .with_minute(0)
                    .unwrap()
                    .with_second(0)
                    .unwrap()
                    .with_nanosecond(0)
                    .unwrap();
                if anchor == hour {
                    hour
                } else {
                    hour + chrono::Duration::hours(1)
                }
            } else {
                anchor
            };
            let start = if preset == "all" {
                from.map(parse_live_timestamp)
                    .transpose()?
                    .unwrap_or(anchor - duration)
            } else if matches!(preset, "7d" | "30d") {
                anchor.date_naive().and_hms_opt(0, 0, 0).unwrap().and_utc() - duration
            } else {
                end - duration
            };
            (start, end, !matches!(preset, "7d" | "30d"))
        };
        Ok(Self {
            preset: if preset.contains("..") {
                "explicit"
            } else {
                preset
            }
            .into(),
            from: start,
            to: end,
            include_end,
        })
    }

    fn query_to(&self) -> String {
        iso(self.to + chrono::Duration::microseconds(i64::from(self.include_end)))
    }
}

fn prepare_live_facts(conn: &duckdb::Connection) -> Result<Vec<(String, String, String)>> {
    let database: String = conn.query_row("SELECT current_database()", [], |r| r.get(0))?;
    let mut sources = Vec::new();
    for &(table, timestamp, schema) in LIVE_FACTS {
        let prefixed = format!("_dendro_{table}");
        let source: Option<String> = conn.query_row(
            "SELECT table_name FROM information_schema.tables WHERE table_catalog = ? AND table_schema = 'main' AND table_name IN (?, ?) ORDER BY CASE WHEN table_name = ? THEN 0 ELSE 1 END LIMIT 1",
            duckdb::params![database, prefixed, table, prefixed], |r| r.get(0),
        ).ok();
        let relation = if let Some(source) = source {
            format!(
                "{}.main.{}",
                sql_identifier(&database),
                sql_identifier(&source)
            )
        } else {
            let empty = format!("_dashboard_empty_{table}");
            conn.execute_batch(&format!(
                "CREATE TEMP TABLE IF NOT EXISTS {empty} ({schema})"
            ))?;
            sql_identifier(&empty)
        };
        conn.execute_batch(&format!(
            "CREATE OR REPLACE TEMP VIEW {table} AS SELECT * FROM {relation}"
        ))?;
        let timestamp = if matches!(table, "envelope_fingerprints" | "skill_facts") {
            [
                "ts",
                "recorded_at",
                "last_seen_at",
                "first_seen_at",
                "observed_at",
            ]
            .into_iter()
            .find(|name| {
                conn.prepare(&format!(
                    "SELECT {} FROM {relation} LIMIT 0",
                    sql_identifier(name)
                ))
                .is_ok()
            })
            .unwrap_or("")
        } else {
            timestamp
        };
        sources.push((table.into(), timestamp.into(), relation));
    }
    Ok(sources)
}

fn query_live_payload(
    conn: &duckdb::Connection,
    view: &str,
    requested_window: Option<&str>,
    rig: Option<&str>,
) -> Result<serde_json::Value> {
    use serde_json::json;
    let sources = prepare_live_facts(conn)?;
    for (name, schema) in [
        ("seat_bounds", "seat VARCHAR, created_at TIMESTAMP, stopped_at TIMESTAMP"),
        ("destination_snapshots", "qitem_id VARCHAR, destination VARCHAR, snapshot_cutoff VARCHAR, reroute_marked BOOLEAN, missions VARCHAR"),
    ] {
        if conn.prepare(&format!("SELECT * FROM {name} LIMIT 0")).is_err() {
            conn.execute_batch(&format!("CREATE TEMP TABLE {name} ({schema})"))?;
        }
    }
    let (first, last): (Option<String>, Option<String>) = conn.query_row(
        "SELECT MIN(t)::VARCHAR, MAX(t)::VARCHAR FROM (
            SELECT ts t FROM usage_facts UNION ALL SELECT ts FROM message_facts
            UNION ALL SELECT ts FROM activity_facts UNION ALL SELECT ts FROM queue_facts
            UNION ALL SELECT ts FROM security_facts UNION ALL SELECT observed_at FROM session_edges
            UNION ALL SELECT ts FROM tool_facts UNION ALL SELECT ts FROM process_samples
            UNION ALL SELECT TRY_CAST(COALESCE(json_extract_string(to_json(s),'$.ts'),json_extract_string(to_json(s),'$.recorded_at'),json_extract_string(to_json(s),'$.last_seen_at'),json_extract_string(to_json(s),'$.first_seen_at'),json_extract_string(to_json(s),'$.observed_at')) AS TIMESTAMP) FROM skill_facts s
            UNION ALL SELECT last_seen FROM process_identities
            UNION ALL SELECT TRY_CAST(COALESCE(json_extract_string(to_json(f),'$.ts'),json_extract_string(to_json(f),'$.recorded_at'),json_extract_string(to_json(f),'$.last_seen_at'),json_extract_string(to_json(f),'$.first_seen_at'),json_extract_string(to_json(f),'$.observed_at')) AS TIMESTAMP) FROM envelope_fingerprints f)",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let window = LiveWindow::resolve(requested_window, first.as_deref(), last.as_deref())?;
    // Capture the shared overview before request-local presentation views discard
    // pre-window ownership, lifecycle and project-interval support.
    let overview = if matches!(
        view,
        "direct_overview"
            | "v_cwd_fingerprint_rollup"
            | "skill_facts"
            | "hub"
            | "general"
            | "status"
            | "v_seat_idle_ranking"
            | "v_handoff_rework_analysis"
    ) {
        Some(direct_overview_payload(conn, &window, rig)?)
    } else {
        None
    };
    conn.execute_batch("CREATE TEMP TABLE IF NOT EXISTS _dashboard_window (start_ts TIMESTAMP, end_ts TIMESTAMP, rig VARCHAR); DELETE FROM _dashboard_window;")?;
    conn.execute(
        "INSERT INTO _dashboard_window VALUES (?::TIMESTAMP, ?::TIMESTAMP, ?)",
        duckdb::params![iso(window.from), window.query_to(), rig],
    )?;
    if rig.is_some() {
        // Resolve attribution once. Keeping a nested identity/window view in
        // every correlated fact predicate makes DuckDB repeatedly expand it.
        conn.execute_batch("CREATE OR REPLACE TEMP TABLE _dashboard_rig_identities AS SELECT * FROM identity_facts WHERE rig=(SELECT rig FROM _dashboard_window);
            CREATE OR REPLACE TEMP VIEW identity_facts AS SELECT * FROM _dashboard_rig_identities;")?;
    }
    let process_sample_source = &sources.iter().find(|s| s.0 == "process_samples").unwrap().2;
    for (table, timestamp, relation) in &sources {
        if table == "identity_facts" && rig.is_some() {
            continue;
        }
        let time_filter = if timestamp.is_empty() || window.preset == "all" {
            if table == "envelope_fingerprints" && window.preset != "all" {
                "FALSE".to_owned()
            } else {
                "TRUE".to_owned()
            }
        } else if table == "process_identities" {
            format!("(f.last_seen >= w.start_ts AND f.last_seen < w.end_ts) OR EXISTS (SELECT 1 FROM {process_sample_source} s WHERE s.identity_key=f.identity_key AND s.ts>=w.start_ts AND s.ts<w.end_ts)")
        } else {
            format!("f.{timestamp} >= w.start_ts AND f.{timestamp} < w.end_ts")
        };
        let rig_filter = match table.as_str() {
            "identity_facts" => "f.rig = (SELECT rig FROM _dashboard_window)",
            "process_identities" => "EXISTS (SELECT 1 FROM identity_facts i WHERE i.native_session=f.session_link AND i.harness=f.harness)",
            "process_samples" => "EXISTS (SELECT 1 FROM process_identities p WHERE p.identity_key=f.identity_key)",
            "envelope_fingerprints" => "EXISTS (SELECT 1 FROM identity_facts i WHERE i.harness=f.harness)",
            "usage_facts" | "message_facts" | "security_facts" | "tool_facts" =>
                "EXISTS (SELECT 1 FROM identity_facts i WHERE i.harness=f.harness AND i.native_session=f.native_session)",
            "session_edges" =>
                "EXISTS (SELECT 1 FROM identity_facts i WHERE i.harness=f.harness AND i.native_session IN (f.parent_session,f.child_session))",
            "skill_facts" =>
                "EXISTS (SELECT 1 FROM identity_facts i WHERE i.seat=f.seat OR i.harness || ':' || i.native_session=f.seat)",
            "queue_facts" =>
                "EXISTS (SELECT 1 FROM identity_facts i WHERE i.seat IN (f.source_seat,f.dest_seat))",
            _ => "EXISTS (SELECT 1 FROM identity_facts i WHERE i.seat=f.seat)",
        };
        let rig_filter = if rig.is_none() { "TRUE" } else { rig_filter };
        conn.execute_batch(&format!("CREATE OR REPLACE TEMP VIEW {table} AS SELECT f.* FROM {relation} f CROSS JOIN _dashboard_window w WHERE ({time_filter}) AND ({rig_filter})"))?;
    }
    let mut counts = serde_json::Map::new();
    for &(table, _, _) in &LIVE_FACTS[..8] {
        let count: i64 =
            conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))?;
        counts.insert(table.into(), json!(count));
    }
    let mut payload = json!({
        "product":"AI Fleet Intelligence", "view":view, "live":true,
        "table_counts":counts,
        "recorded_window": window_labels(first.as_deref().map(parse_live_timestamp).transpose()?, last.as_deref().map(parse_live_timestamp).transpose()?),
        "active_window":window_labels(Some(window.from), Some(window.to)),
        "query_to":window.query_to(), "rows":[], "insights":{}, "insights_catalog":[]
    });
    payload["active_window"]["preset"] = json!(window.preset);
    if matches!(window.preset.as_str(), "7d" | "30d" | "last_month")
        || (window.preset == "explicit"
            && requested_window
                .and_then(|w| w.split_once(".."))
                .is_some_and(|(_, t)| t.len() == 10))
    {
        // Keep the exclusive query boundary at next midnight while presenting
        // the date the person selected as the inclusive end.
        let labels = window_labels(
            Some(window.from),
            Some(window.to - chrono::Duration::seconds(1)),
        );
        for key in ["from_date", "to_date", "display"] {
            payload["active_window"][key] = labels[key].clone();
        }
    }
    payload["capability_intelligence"] = capability_intelligence(conn, &window)?;
    payload["time_buckets"] = time_buckets(conn, &window)?;
    payload["xray_lanes"] = xray_lanes(conn)?;
    payload["anomalies"] = anomaly_markers(conn)?;
    let use_semantic = window.preset == "all" && rig.is_none();
    let active_from = iso(window.from);
    let active_to = window.query_to();
    let active_window_bounds =
        (window.preset != "all").then_some((active_from.as_str(), active_to.as_str()));
    let mut rows = query_uncapped_view_or_fallback(
        conn,
        view,
        use_semantic || view == "v_mission_token_burn",
        active_window_bounds,
    )?;
    if matches!(
        view,
        "v_seat_idle_ranking" | "v_handoff_rework_analysis" | "v_executive_governance"
    ) {
        rows.retain(|row| {
            // A fresh snapshot may still describe a different report window.
            let start = row["window_from"]
                .as_str()
                .and_then(|s| parse_live_timestamp(s).ok());
            let end = row["window_to"]
                .as_str()
                .and_then(|s| parse_live_timestamp(s).ok());
            start.is_none_or(|start| start <= window.from) && end.is_none_or(|end| end >= window.to)
        });
    }
    payload["rows"] = json!(rows);
    match view {
        "capability_intelligence" => {
            payload["rows"] = payload["capability_intelligence"]["tool_families"].clone();
            payload["summary"] = payload["capability_intelligence"]["summary"].clone();
        }
        "hub" | "general" | "status" => {
            payload["view"] = json!("hub");
            payload["harnesses"] = json!(json_rows(conn,
                "SELECT harness,COUNT(*)::BIGINT usage_rows,SUM(input_uncached+output_tokens+cache_read+cache_write)::BIGINT total_tokens,
                 (SELECT COUNT(*) FROM message_facts m WHERE m.harness=u.harness)::BIGINT message_rows
                 FROM usage_facts u GROUP BY harness ORDER BY total_tokens DESC", [])?);
            payload["rigs"] = json!(json_rows(conn,
                "SELECT i.rig,COUNT(DISTINCT i.seat)::BIGINT seats,COUNT(DISTINCT q.qitem_id)::BIGINT queue_items FROM identity_facts i LEFT JOIN queue_facts q ON i.seat=q.dest_seat GROUP BY i.rig", [])?);
            payload["scopes"] = json!(payload["rigs"].as_array().into_iter().flatten().map(|r| {
                json!({"scope":r["rig"],"scope_id":r["rig"],"actors":r["seats"],"queue_items":r["queue_items"]})
            }).collect::<Vec<_>>());
            let totals = usage_totals(conn)?;
            let actors: i64 = conn.query_row("SELECT COUNT(DISTINCT seat) FROM (SELECT seat FROM activity_facts UNION ALL SELECT harness seat FROM usage_facts UNION ALL SELECT harness seat FROM message_facts)", [], |r| r.get(0))?;
            payload["kpis"] = json!({"total_tokens":totals["total_tokens"],"active_seats":actors,"active_actors":actors,
                "m1_yield":null,"handoff_friction_rate":null,"effective_concurrency":null,"rework_returns":null});
        }
        "v_seat_idle_ranking" => seat_payload(conn, &mut payload, &rows)?,
        "v_handoff_rework_analysis" => handoff_payload(conn, &mut payload, &rows)?,
        "v_mission_token_burn" => mission_payload(conn, &mut payload, &rows)?,
        "v_executive_governance" => governance_payload(conn, &mut payload, &rows, &window)?,
        "direct_overview" => {
            let ov = overview.clone().context("overview capture missing")?;
            payload["direct_overview"] = ov.clone();
            payload["rows"] = ov["project_groups"].clone();
            payload["summary"] = ov;
        }
        "v_cwd_fingerprint_rollup" => {
            let ov = overview.clone().context("overview capture missing")?;
            payload["direct_overview"] = ov.clone();
            payload["rows"] = ov["project_groups"].clone();
        }
        "skill_facts" => {
            let ov = overview.clone().context("overview capture missing")?;
            payload["direct_overview"] = ov.clone();
            payload["rows"] = json!(ov["context_tax"]["uncalled_skills"].as_array().into_iter().flatten().map(|skill| json!({"skill":skill,"invoked":false,"status":ov["context_tax"]["declaration_status"]})).collect::<Vec<_>>());
        }
        _ => {}
    }
    if matches!(view, "hub" | "general" | "status") {
        if let Some(ref ov) = overview {
            payload["direct_overview"] = ov.clone();
        }
    }
    if let Some(ov) = overview.as_ref() {
        payload["direct_overview"] = ov.clone();
        if let Some(lens) = ov["lens_rows"].get(view) {
            payload["lens_rows"] = lens.clone();
        }
    }
    enrich_payload_groupings(conn, &mut payload, view)?;
    Ok(payload)
}

// The identifiers follow agent-otel-core's ToolArchetype wire names. Keep
// classification local: dashboard readers must also handle older fact schemas.
const TOOL_FAMILIES: &[(&str, &str)] = &[
    ("filter_compressor", "Filter & Compressor"),
    ("structured_parser", "Structured Parser"),
    ("inspector_diff", "Inspector & Diff"),
    ("search_retrieval", "Search & Retrieval"),
    ("build_test_verify", "Build, Test & Verify"),
    ("state_mutation", "State Mutation"),
    ("env_pkg_manager", "Environment & Packages"),
    ("vcs_lifecycle", "Version Control"),
    ("network_transfer", "Network & Transfer"),
    ("generic_exec", "Generic Execution"),
];

fn tool_family(tool: &str, category: &str) -> &'static str {
    let name = tool.trim().to_ascii_lowercase();
    let binary = name.split_whitespace().next().unwrap_or_default();
    let binary = binary.rsplit('/').next().unwrap_or(binary);
    let category = category.to_ascii_lowercase();
    match binary {
        "rtk" | "tee" | "time" | "stdbuf" | "timeout" => return "filter_compressor",
        "jq" | "yq" | "dasel" | "fx" | "gron" | "xmllint" => return "structured_parser",
        "python" | "python3" if name.contains(" -c") => return "structured_parser",
        "git" if name.starts_with("git diff") || name.starts_with("git show") => {
            return "inspector_diff"
        }
        "git" => return "vcs_lifecycle",
        "cargo"
            if [
                " add",
                " rm",
                " install",
                " update",
                " upgrade",
                " uninstall",
            ]
            .iter()
            .any(|s| name.contains(s)) =>
        {
            return "env_pkg_manager"
        }
        "cargo" | "pytest" | "make" | "ninja" | "cmake" | "tsc" | "eslint" | "ruff" | "vitest"
        | "jest" => return "build_test_verify",
        "node" if name.contains("--test") => return "build_test_verify",
        "npm" | "pnpm" | "yarn" | "bun"
            if ["test", "build", "lint", "check"]
                .iter()
                .any(|s| name.split_whitespace().skip(1).any(|word| word == *s)) =>
        {
            return "build_test_verify"
        }
        "python" | "python3" if name.contains("-m pytest") || name.contains("-m unittest") => {
            return "build_test_verify"
        }
        "mise" | "uv" | "pip" | "pip3" | "npm" | "pnpm" | "yarn" | "bun" | "systemctl"
        | "envctl" | "apt" | "brew" => return "env_pkg_manager",
        "rg" | "grep" | "grep_search" | "find_by_name" | "glob" | "ls" | "find" | "fd" | "fts"
        | "vss" | "list_dir" => return "search_retrieval",
        "read" | "read_file" | "view" | "view_file" | "cat" | "head" | "tail" | "inspect_read" => {
            return "inspector_diff"
        }
        "edit"
        | "write"
        | "multiedit"
        | "edit_file"
        | "write_file"
        | "write_to_file"
        | "apply_patch"
        | "replace_file_content" => return "state_mutation",
        "curl" | "wget" | "gh" | "search_web" | "web_search" | "web_fetch" | "http" | "fetch" => {
            return "network_transfer"
        }
        _ => {}
    }
    if let Some((family, _)) = TOOL_FAMILIES.iter().find(|(family, _)| *family == category) {
        return family;
    }
    match category.as_str() {
        "inspect_read" => "inspector_diff",
        "eng_code_edit" => "state_mutation",
        "eng_build_test" => "build_test_verify",
        "eng_vcs" => "vcs_lifecycle",
        "coord_queue" | "coord_status" => "structured_parser",
        _ => "generic_exec",
    }
}

fn capability_kind(tool: &str) -> &'static str {
    let name = tool.trim().to_ascii_lowercase();
    if name.starts_with("mcp_")
        || name.starts_with("call_mcp_tool")
        || matches!(
            name.as_str(),
            "list_resources"
                | "read_resource"
                | "list_mcp_resources"
                | "read_mcp_resource"
                | "mcptoolcall"
        )
        || ["holoself", "cypher_query", "graph_"]
            .iter()
            .any(|s| name.contains(s))
    {
        "mcp"
    } else if name.starts_with("skill__") || matches!(name.as_str(), "skill" | "use_skill") {
        "skill"
    } else if matches!(
        name.as_str(),
        "invoke_subagent"
            | "manage_subagents"
            | "define_subagent"
            | "run_subagent"
            | "task"
            | "codexseatcall"
            | "dispatch_agent"
    ) {
        "subagent"
    } else {
        "native"
    }
}

fn normalized_primitive(tool: &str) -> (&str, &str) {
    match tool.to_ascii_lowercase().as_str() {
        "read" | "read_file" | "view" | "view_file" | "cat" | "head" | "tail" => {
            ("read_file", "Read / Inspect File")
        }
        "grep" | "grep_search" | "rg" => ("grep_search", "Search File Contents"),
        "glob" | "find_by_name" | "find" | "fd" => ("find_by_name", "Find Files"),
        "ls" | "list_dir" => ("list_dir", "List Directory"),
        "edit" | "edit_file" | "multiedit" | "replace_file_content" => ("edit_file", "Edit File"),
        "write" | "write_file" | "write_to_file" => ("write_file", "Write File"),
        "bash" | "run_command" | "commandexecution" | "exec_command" => {
            ("run_command", "Execute Command")
        }
        _ => (tool, tool),
    }
}

fn mcp_tool_parts(tool: &str) -> (&str, &str) {
    if let Some(rest) = tool.strip_prefix("mcp__") {
        return rest.split_once("__").unwrap_or((rest, rest));
    }
    if let Some(rest) = tool.strip_prefix("mcp_") {
        return rest
            .split_once("__")
            .or_else(|| rest.split_once('_'))
            .unwrap_or(("unknown", rest));
    }
    if tool.to_ascii_lowercase().contains("holoself") {
        ("holoself", tool)
    } else {
        ("unknown", tool)
    }
}

fn hook_stage(kind: &str, value: &str) -> &'static str {
    for name in [kind, value] {
        match name.to_ascii_lowercase().replace('_', "").as_str() {
            "preinvocation" => return "PreInvocation",
            "postinvocation" => return "PostInvocation",
            "pretooluse" => return "PreToolUse",
            "posttooluse" => return "PostToolUse",
            "stop" => return "Stop",
            _ => {}
        }
    }
    match kind {
        "permission_decision" | "approval" => "PreToolUse / Permission Decision",
        "permission_mode" | "sandbox_policy" | "effective_authority" => "PreInvocation",
        "tool_result" | "tool_failure" => "PostToolUse",
        "invocation_result" => "PostInvocation",
        "session_stop" => "Stop",
        _ => "PreToolUse",
    }
}

#[derive(Default)]
struct CapabilityCounts {
    calls: u64,
    failed: u64,
    structured_writes: u64,
    harnesses: BTreeSet<String>,
    tools: BTreeMap<String, u64>,
}

impl CapabilityCounts {
    fn add(&mut self, row: &serde_json::Value) {
        self.calls += row["calls"].as_u64().unwrap_or(0);
        self.failed += row["failed"].as_u64().unwrap_or(0);
        self.structured_writes += row["structured_writes"].as_u64().unwrap_or(0);
        for harness in row["harnesses"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|h| h.as_str())
        {
            self.harnesses.insert(harness.to_owned());
        }
        *self
            .tools
            .entry(text(row, &["tool_name"]).to_owned())
            .or_default() += row["calls"].as_u64().unwrap_or(0);
    }

    fn json(&self) -> serde_json::Value {
        serde_json::json!({"calls":self.calls,"failed":self.failed,
            "failure_rate_pct":100.0 * ratio(self.failed as f64, self.calls as f64),
            "structured_writes":self.structured_writes,"harnesses":self.harnesses})
    }
}

fn capability_intelligence(
    conn: &duckdb::Connection,
    window: &LiveWindow,
) -> Result<serde_json::Value> {
    use serde_json::json;
    // Optional fields are extracted from the row JSON for compatibility with
    // old stores and empty-table fallbacks, without swallowing query errors.
    let tools = json_rows(conn, "SELECT COALESCE(json_extract_string(to_json(t),'$.tool_name'),'unknown') tool_name,
        COALESCE(category,'') category,COUNT(*)::BIGINT calls,COUNT(*) FILTER (WHERE failed)::BIGINT failed,
        COUNT(*) FILTER (WHERE TRY_CAST(json_extract_string(to_json(t),'$.structured_write') AS BOOLEAN))::BIGINT structured_writes,
        list_sort(list_distinct(list(harness))) harnesses FROM tool_facts t GROUP BY 1,2 ORDER BY calls DESC,tool_name,category", [])?;
    let mut families: BTreeMap<_, CapabilityCounts> = TOOL_FAMILIES
        .iter()
        .map(|(id, _)| (*id, CapabilityCounts::default()))
        .collect();
    let mut primitives: BTreeMap<(String, String), CapabilityCounts> = BTreeMap::new();
    let mut mcp: BTreeMap<(String, String), CapabilityCounts> = BTreeMap::new();
    let mut totals = CapabilityCounts::default();
    let mut primitive_calls = 0;
    let mut subagent_calls = 0;
    for row in &tools {
        totals.add(row);
        let tool = text(row, &["tool_name"]);
        let family = tool_family(tool, text(row, &["category"]));
        families.get_mut(family).unwrap().add(row);
        match capability_kind(tool) {
            "native" => {
                primitive_calls += row["calls"].as_u64().unwrap_or(0);
                let (canonical, _) = normalized_primitive(tool);
                primitives
                    .entry((canonical.to_owned(), family.to_owned()))
                    .or_default()
                    .add(row);
            }
            "mcp" => {
                let (server, tool) = mcp_tool_parts(tool);
                mcp.entry((server.to_owned(), tool.to_owned()))
                    .or_default()
                    .add(row);
            }
            "subagent" => subagent_calls += row["calls"].as_u64().unwrap_or(0),
            _ => {}
        }
    }
    // Some harnesses emit named MCP messages without a corresponding tool fact.
    // Read only named tool observations; ordinary transcript text is not a call.
    let message_tools = json_rows(conn, "SELECT tool_name,COUNT(*)::BIGINT calls,0::BIGINT failed,0::BIGINT structured_writes,list_sort(list_distinct(list(harness))) harnesses
        FROM (SELECT m.source_key,m.harness,COALESCE(json_extract_string(to_json(m),'$.tool_name'),json_extract_string(to_json(m),'$.tool.name')) tool_name
              FROM message_facts m WHERE NOT EXISTS (SELECT 1 FROM tool_facts t WHERE t.source_key=m.source_key)
              AND (m.role='tool' OR contains(m.modalities,'tool'))) GROUP BY tool_name HAVING tool_name IS NOT NULL", [])?;
    let mut supplemental_mcp_calls = 0;
    for row in &message_tools {
        let tool = text(row, &["tool_name"]);
        if capability_kind(tool) == "mcp" {
            let (server, tool) = mcp_tool_parts(tool);
            mcp.entry((server.to_owned(), tool.to_owned()))
                .or_default()
                .add(row);
            supplemental_mcp_calls += row["calls"].as_u64().unwrap_or(0);
        }
    }
    let mut tool_families: Vec<_> = TOOL_FAMILIES
        .iter()
        .map(|(id, label)| {
            let counts = &families[id];
            let mut tools: Vec<_> = counts.tools.iter().collect();
            tools.sort_by(|(a, ac), (b, bc)| bc.cmp(ac).then_with(|| a.cmp(b)));
            let mut row = counts.json();
            row["archetype"] = json!(id);
            row["label"] = json!(label);
            row["share_pct"] = json!(100.0 * ratio(counts.calls as f64, totals.calls as f64));
            row["top_tools"] = json!(tools
                .into_iter()
                .take(3)
                .map(|(tool, _)| tool)
                .collect::<Vec<_>>());
            row
        })
        .collect();
    tool_families.sort_by(|a, b| {
        b["calls"]
            .as_u64()
            .cmp(&a["calls"].as_u64())
            .then_with(|| a["archetype"].as_str().cmp(&b["archetype"].as_str()))
    });
    let mut primitives: Vec<_> = primitives
        .into_iter()
        .map(|((canonical, family), counts)| {
            let mut row = counts.json();
            row["primitive"] = json!(normalized_primitive(&canonical).1);
            row["canonical_name"] = json!(canonical);
            row["family"] = json!(family);
            row["structured_write"] = json!(counts.structured_writes > 0);
            row
        })
        .collect();
    primitives.sort_by(|a, b| {
        b["calls"].as_u64().cmp(&a["calls"].as_u64()).then_with(|| {
            a["canonical_name"]
                .as_str()
                .cmp(&b["canonical_name"].as_str())
        })
    });
    primitives.truncate(15);

    // Skill rows in historical stores are undated. Link their event prefix to
    // a dated source observation; a session-wide join would count old invocations.
    conn.execute_batch("CREATE OR REPLACE TEMP TABLE _dashboard_capability_events AS
        SELECT DISTINCT source_key,regexp_replace(source_key,':(skill:[0-9]+|message|codex:[0-9]+|[0-9]+)$','') event_key FROM (
            SELECT source_key FROM tool_facts WHERE ts IS NOT NULL UNION ALL SELECT source_key FROM message_facts WHERE ts IS NOT NULL
            UNION ALL SELECT source_key FROM usage_facts WHERE ts IS NOT NULL UNION ALL SELECT source_key FROM activity_facts WHERE ts IS NOT NULL);")?;
    let skill_rows = json_rows(conn, "SELECT s.*,regexp_replace(s.source_key,':skill:[0-9]+$','') event_key FROM skill_facts s WHERE ? OR
        TRY_CAST(COALESCE(json_extract_string(to_json(s),'$.ts'),json_extract_string(to_json(s),'$.recorded_at'),json_extract_string(to_json(s),'$.last_seen_at'),json_extract_string(to_json(s),'$.first_seen_at'),json_extract_string(to_json(s),'$.observed_at')) AS TIMESTAMP) IS NOT NULL OR
        EXISTS (SELECT 1 FROM _dashboard_capability_events e WHERE e.source_key=s.source_key OR e.event_key=regexp_replace(s.source_key,':skill:[0-9]+$','')) OR
        (NOT s.invoked AND (EXISTS (SELECT 1 FROM activity_facts a WHERE a.seat=s.seat) OR
         EXISTS (SELECT 1 FROM identity_facts i WHERE (i.seat=s.seat OR i.harness || ':' || i.native_session=s.seat) AND
            EXISTS (SELECT 1 FROM (SELECT harness,native_session FROM tool_facts UNION ALL SELECT harness,native_session FROM message_facts UNION ALL SELECT harness,native_session FROM usage_facts) f WHERE f.harness=i.harness AND f.native_session=i.native_session))))",
        duckdb::params![window.preset == "all"])?;
    let candidate_skills: u64 =
        conn.query_row("SELECT COUNT(*) FROM skill_facts", [], |r| r.get(0))?;
    let undated_skills_excluded = candidate_skills.saturating_sub(skill_rows.len() as u64);
    let skill_tools = json_rows(conn, "SELECT source_key,regexp_replace(source_key,':(skill:[0-9]+|message|codex:[0-9]+|[0-9]+)$','') event_key,harness,json_extract_string(to_json(t),'$.tool_name') tool_name FROM tool_facts t
        WHERE starts_with(lower(json_extract_string(to_json(t),'$.tool_name')),'skill__') OR lower(json_extract_string(to_json(t),'$.tool_name')) IN ('skill','use_skill')", [])?;
    let mut skill_events = BTreeSet::new();
    let mut named_skill_events = BTreeSet::new();
    let mut skills: BTreeMap<(String, String), (u64, u64)> = BTreeMap::new();
    for row in &skill_rows {
        let skill = text(row, &["skill"]);
        let seat = text(row, &["seat"]);
        let invoked = row["invoked"].as_bool().unwrap_or(false);
        let counts = skills
            .entry((skill.to_owned(), seat.to_owned()))
            .or_default();
        if invoked {
            counts.0 += 1;
            skill_events.insert((text(row, &["event_key"]).to_owned(), skill.to_owned()));
            named_skill_events.insert(text(row, &["event_key"]).to_owned());
        } else {
            counts.1 += 1;
        }
    }
    for row in &skill_tools {
        let tool = text(row, &["tool_name"]);
        if capability_kind(tool) != "skill" {
            continue;
        }
        // A generic Skill/use_skill call does not carry the skill's name.
        // Prefer the named fact for the same source event over a second call.
        if matches!(tool.to_ascii_lowercase().as_str(), "skill" | "use_skill")
            && named_skill_events.contains(text(row, &["event_key"]))
        {
            continue;
        }
        let skill = tool
            .strip_prefix("skill__")
            .unwrap_or(tool)
            .split("__")
            .next()
            .unwrap_or(tool);
        if skill_events.insert((text(row, &["event_key"]).to_owned(), skill.to_owned())) {
            skills
                .entry((skill.to_owned(), text(row, &["harness"]).to_owned()))
                .or_default()
                .0 += 1;
        }
    }
    let skill_invocations: u64 = skills.values().map(|s| s.0).sum();
    let declared_skills = skills.values().filter(|s| s.1 > 0).count();
    let skills: Vec<_> = skills
        .into_iter()
        .map(|((skill, seat), (calls, _))| {
            json!({
                "skill":skill,"seat_or_harness":seat,"invoked":calls>0,"calls":calls,
                "status":if calls>0 {"ACTIVE_INVOKED"} else {"DECLARED_DORMANT"}
            })
        })
        .collect();
    let mcp_calls: u64 = mcp.values().map(|s| s.calls).sum();
    let mut mcp: Vec<_> = mcp
        .into_iter()
        .map(|((server, tool), counts)| {
            let mut row = counts.json();
            row["server"] = json!(server);
            row["tool"] = json!(tool);
            row
        })
        .collect();
    mcp.sort_by(|a, b| {
        b["calls"]
            .as_u64()
            .cmp(&a["calls"].as_u64())
            .then_with(|| a["server"].as_str().cmp(&b["server"].as_str()))
            .then_with(|| a["tool"].as_str().cmp(&b["tool"].as_str()))
    });
    let mut hooks = json_rows(conn, "SELECT kind,value,COUNT(*)::BIGINT observations,list_sort(list_distinct(list(harness))) harnesses FROM security_facts GROUP BY kind,value ORDER BY observations DESC,kind,value", [])?;
    let mut hook_interceptions = 0;
    for row in &mut hooks {
        hook_interceptions += row["observations"].as_u64().unwrap_or(0);
        row["hook_stage"] = json!(hook_stage(text(row, &["kind"]), text(row, &["value"])));
    }
    let edges: u64 = conn.query_row("SELECT COUNT(*) FROM session_edges", [], |r| r.get(0))?;
    Ok(json!({"summary":{
        "total_tool_calls":totals.calls,"primitive_calls":primitive_calls,"skill_invocations":skill_invocations,
        "declared_skills":declared_skills,"mcp_calls":mcp_calls,"subagent_dispatches":subagent_calls+edges,
        "hook_interceptions":hook_interceptions,
        "structured_write_pct":100.0*ratio(totals.structured_writes as f64,totals.calls as f64),
        "failure_rate_pct":100.0*ratio(totals.failed as f64,totals.calls as f64)},
        "tool_families":tool_families,"primitives":primitives,"skills":skills,"mcp":mcp,"hooks":hooks,
        "coverage":{"undated_skill_facts_excluded":undated_skills_excluded,"supplemental_mcp_message_calls":supplemental_mcp_calls,
            "subagent_tool_calls":subagent_calls,"session_edge_observations":edges}
    }))
}

fn time_buckets(conn: &duckdb::Connection, window: &LiveWindow) -> Result<serde_json::Value> {
    use serde_json::json;
    let span = (window.to - window.from)
        .num_microseconds()
        .context("dashboard window too wide")?
        .max(1);
    let mut buckets: Vec<_> = (0..32).map(|i| json!({
        "t":iso(window.from + chrono::Duration::microseconds((i128::from(span)*i/32) as i64)),
        "model":0,"harness":0,"coord":0,"env":0,"observed":false
    })).collect();
    for (layer, table, metric) in [
        (
            "model",
            "usage_facts",
            "SUM(input_uncached+output_tokens+cache_read+cache_write)",
        ),
        ("harness", "message_facts", "COUNT(*)"),
        ("coord", "queue_facts", "COUNT(*)"),
        ("env", "activity_facts", "COUNT(*)"),
        ("env", "security_facts", "COUNT(*)"),
    ] {
        let sql = format!("SELECT LEAST(31,GREATEST(0,FLOOR((epoch_us(ts)-epoch_us(?::TIMESTAMP))::DOUBLE / ? * 32)))::BIGINT bucket, {metric}::BIGINT value FROM {table} WHERE ts IS NOT NULL GROUP BY bucket");
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(duckdb::params![iso(window.from), span], |r| {
            Ok((r.get::<_, usize>(0)?, r.get::<_, i64>(1)?))
        })?;
        for row in rows {
            let (index, value) = row?;
            let bucket = &mut buckets[index];
            bucket[layer] = json!(bucket[layer].as_i64().unwrap_or(0).saturating_add(value));
            bucket["observed"] = json!(true);
        }
    }
    Ok(json!(buckets))
}

fn xray_lanes(conn: &duckdb::Connection) -> Result<serde_json::Value> {
    use serde_json::json;
    let mut lanes = json!({});
    for (layer, sql) in [
        ("model", "SELECT ts::VARCHAR ts,harness,native_session session,'Token usage' label, source_key detail,(input_uncached+output_tokens+cache_read+cache_write)::DOUBLE value FROM usage_facts WHERE ts IS NOT NULL ORDER BY ts DESC,source_key LIMIT 16"),
        ("harness", "SELECT ts::VARCHAR ts,harness,COALESCE(native_session,'') session,role label,direction || ' · ' || modalities detail,1.0::DOUBLE value FROM message_facts WHERE ts IS NOT NULL ORDER BY ts DESC,source_key LIMIT 16"),
        ("coord", "SELECT ts::VARCHAR ts,'openrig' harness,qitem_id session,state label,source_seat || ' → ' || dest_seat detail,1.0::DOUBLE value FROM queue_facts WHERE ts IS NOT NULL ORDER BY ts DESC,source_key LIMIT 16"),
        ("env", "SELECT ts::VARCHAR ts,harness,session,label,detail,1.0::DOUBLE value FROM (SELECT ts,'openrig' harness,seat session,state label,'Activity observation' detail,source_key FROM activity_facts UNION ALL SELECT ts,harness,COALESCE(native_session,'') session,kind label,value detail,source_key FROM security_facts) WHERE ts IS NOT NULL ORDER BY ts DESC,source_key LIMIT 16"),
    ] {
        let mut events = json_rows(conn,sql,[])?;
        for event in &mut events {
            event["ts"] = json!(iso(parse_live_timestamp(event["ts"].as_str().unwrap_or_default())?));
        }
        lanes[layer] = json!(events);
    }
    Ok(lanes)
}

fn usage_totals(conn: &duckdb::Connection) -> Result<serde_json::Value> {
    Ok(json_rows(conn,"SELECT COALESCE(SUM(input_uncached+cache_read+cache_write+output_tokens),0)::BIGINT total_tokens,COALESCE(SUM(input_uncached),0)::BIGINT fresh_input_tokens,COALESCE(SUM(cache_read),0)::BIGINT cache_read_tokens,COALESCE(SUM(cache_write),0)::BIGINT cache_write_tokens,COALESCE(SUM(output_tokens),0)::BIGINT output_tokens,COUNT(*)::BIGINT turns FROM usage_facts",[])?.remove(0))
}

fn number(row: &serde_json::Value, names: &[&str]) -> f64 {
    names
        .iter()
        .find_map(|key| {
            row[*key]
                .as_f64()
                .or_else(|| row[*key].as_str()?.parse().ok())
        })
        .unwrap_or(0.0)
}

fn text<'a>(row: &'a serde_json::Value, names: &[&str]) -> &'a str {
    names
        .iter()
        .find_map(|key| row[*key].as_str().filter(|s| !s.is_empty()))
        .unwrap_or("")
}

fn ratio(numerator: f64, denominator: f64) -> f64 {
    if denominator > 0.0 {
        numerator / denominator
    } else {
        0.0
    }
}

fn decoded_json(value: &serde_json::Value) -> serde_json::Value {
    value
        .as_str()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_else(|| value.clone())
}

fn set_view_rows(
    payload: &mut serde_json::Value,
    key: &str,
    view: &str,
    rows: Vec<serde_json::Value>,
) {
    let rows: Vec<_> = rows
        .into_iter()
        .map(|row| {
            serde_json::Value::Object(enrich_and_reorder_row(
                view,
                row.as_object().cloned().unwrap_or_default(),
            ))
        })
        .collect();
    payload[key] = serde_json::json!(rows);
    payload["rows"] = serde_json::json!(rows);
}

fn anomaly_markers(conn: &duckdb::Connection) -> Result<serde_json::Value> {
    use serde_json::json;
    let mut markers = Vec::new();
    for (view, sql) in [
        ("v_trajectory_vortex", "SELECT MAX(u.ts)::VARCHAR t,'harness' layer,'vortex' kind,'Trajectory vortex' label,'trajectory_vortex' view,v.session_key ref_id FROM v_trajectory_vortex v JOIN usage_facts u ON list_contains(v.source_keys,u.source_key) WHERE lower(v.tripped) IN ('true','yes','1','tripped') GROUP BY v.session_key HAVING MAX(u.ts) IS NOT NULL ORDER BY MAX(u.ts) DESC LIMIT 8"),
        ("v_schema_drift", "SELECT MAX(u.ts)::VARCHAR t,'harness' layer,'drift' kind,'Schema drift' label,'schema_drift' view,v.harness ref_id FROM v_schema_drift v JOIN usage_facts u ON list_contains(v.source_keys,u.source_key) WHERE v.l1_distance > 0 OR v.keys_added+v.keys_removed+v.keys_retyped > 0 GROUP BY v.harness HAVING MAX(u.ts) IS NOT NULL ORDER BY MAX(u.ts) DESC LIMIT 8"),
        ("v_security_posture", "SELECT MAX(s.ts)::VARCHAR t,'env' layer,'security' kind,v.kind || ': ' || v.value label,'security_posture' view,MIN(s.source_key) ref_id FROM v_security_posture v JOIN security_facts s ON list_contains(v.source_keys,s.source_key) WHERE (lower(v.value) IN ('deny','denied','unsafe','bypass','bypasspermissions','disabled','allow_all','danger-full-access','never') OR v.kind='dangerous_command' OR (v.kind='sandbox_override_requested' AND v.value='true')) GROUP BY v.kind,v.value HAVING MAX(s.ts) IS NOT NULL ORDER BY MAX(s.ts) DESC LIMIT 8"),
    ] {
        let exists: i64 = conn.query_row("SELECT COUNT(*) FROM information_schema.tables WHERE table_name=?",duckdb::params![view],|r| r.get(0))?;
        if exists > 0 {
            // Optional semantic extensions may have an older schema. Raw facts
            // remain usable when that extension cannot provide dated evidence.
            if let Ok(rows) = json_rows(conn,sql,[]) { markers.extend(rows); }
        }
    }
    if !markers.iter().any(|m| m["kind"] == "security") {
        markers.extend(json_rows(conn,"SELECT ts::VARCHAR t,'env' layer,'security' kind,kind || ': ' || value label,'security_posture' view,source_key ref_id FROM security_facts WHERE ts IS NOT NULL AND (lower(value) IN ('deny','denied','unsafe','bypass','bypasspermissions','disabled','allow_all','danger-full-access','never') OR kind='dangerous_command' OR (kind='sandbox_override_requested' AND value='true')) ORDER BY ts DESC,source_key LIMIT 8",[])?);
    }
    markers.extend(json_rows(conn,"WITH burn AS (SELECT harness,native_session,MAX(ts) ts,SUM(input_uncached+output_tokens+cache_read+cache_write) tokens FROM usage_facts GROUP BY harness,native_session) SELECT ts::VARCHAR t,'model' layer,'burn' kind,'High token burn · ' || harness label,'cost_and_loops' view,native_session ref_id FROM burn WHERE ts IS NOT NULL AND tokens >= 100000 AND tokens >= (SELECT COALESCE(AVG(tokens),0) FROM burn) ORDER BY tokens DESC,native_session LIMIT 8",[])?);
    for marker in &mut markers {
        marker["t"] = json!(iso(parse_live_timestamp(
            marker["t"].as_str().unwrap_or_default()
        )?));
    }
    markers.sort_by(|a, b| b["t"].as_str().cmp(&a["t"].as_str()));
    markers.truncate(24);
    Ok(json!(markers))
}

fn direct_overview_payload(
    conn: &duckdb::Connection,
    window: &LiveWindow,
    rig: Option<&str>,
) -> Result<serde_json::Value> {
    use dendro_core::overview::*;
    let req = OverviewRequest {
        rig: rig.map(str::to_string),
        from: window.from,
        to: parse_live_timestamp(&window.query_to())?,
        observation_clock: Some(chrono::Utc::now()),
        evidence_as_of: None,
        advisory_options: dendro_core::advisory::AdvisoryOptions::default(),
        friction_thresholds: dendro_core::vortex::FrictionThresholds::default(),
        expect_evidence_digest: None,
    };
    let mut stmts = OverviewStatements::prepare(conn)?;
    let overview = stmts.evaluate(&req)?;
    serde_json::to_value(&overview).map_err(Into::into)
}

fn seat_payload(
    conn: &duckdb::Connection,
    payload: &mut serde_json::Value,
    rows: &[serde_json::Value],
) -> Result<()> {
    use serde_json::json;
    let fallback = json_rows(conn,
        "WITH bindings AS (SELECT harness,native_session,MIN(seat) seat,MIN(rig) rig FROM identity_facts GROUP BY harness,native_session HAVING COUNT(DISTINCT seat)=1 AND COUNT(DISTINCT rig)=1),
        activity AS (SELECT seat,state,ts,LEAD(ts) OVER (PARTITION BY seat ORDER BY ts,source_key) next_ts FROM activity_facts),
        facts AS (
          SELECT COALESCE(b.seat,u.harness) seat,COALESCE(b.rig,'unscoped') rig,COUNT(*)::BIGINT turns,SUM(input_uncached+output_tokens+cache_read+cache_write)::BIGINT tokens,0.0 observed_secs,0.0 active_secs,0.0 idle_with_work_secs,0.0 unparked_container_secs,0.0 ceremony_secs FROM usage_facts u LEFT JOIN bindings b USING(harness,native_session) GROUP BY ALL
          UNION ALL SELECT COALESCE(b.seat,m.harness),COALESCE(b.rig,'unscoped'),COUNT(*)::BIGINT,0::BIGINT,0.0,0.0,0.0,0.0,0.0 FROM message_facts m LEFT JOIN bindings b USING(harness,native_session) GROUP BY ALL
          UNION ALL SELECT seat,'unscoped',0::BIGINT,0::BIGINT,
            GREATEST(0,epoch(COALESCE(next_ts,(SELECT end_ts FROM _dashboard_window)))-epoch(ts)),
            CASE WHEN state IN ('active','running','working','busy') THEN GREATEST(0,epoch(COALESCE(next_ts,(SELECT end_ts FROM _dashboard_window)))-epoch(ts)) ELSE 0 END,
            CASE WHEN state IN ('idle_with_work','idle_work') THEN GREATEST(0,epoch(COALESCE(next_ts,(SELECT end_ts FROM _dashboard_window)))-epoch(ts)) ELSE 0 END,
            CASE WHEN state IN ('hold','waiting_external','waiting_human','unparked_container') THEN GREATEST(0,epoch(COALESCE(next_ts,(SELECT end_ts FROM _dashboard_window)))-epoch(ts)) ELSE 0 END,
            CASE WHEN state='ceremony' THEN GREATEST(0,epoch(COALESCE(next_ts,(SELECT end_ts FROM _dashboard_window)))-epoch(ts)) ELSE 0 END FROM activity)
        SELECT seat,MAX(CASE WHEN rig<>'unscoped' THEN rig END) rig,MAX(turns)::BIGINT turns,SUM(tokens)::BIGINT tokens,SUM(observed_secs) observed_secs,SUM(active_secs) active_secs,SUM(idle_with_work_secs) idle_with_work_secs,SUM(unparked_container_secs) unparked_container_secs,SUM(ceremony_secs) ceremony_secs FROM facts GROUP BY seat ORDER BY tokens DESC,seat",[])?;
    let mut seats: std::collections::BTreeMap<String, serde_json::Value> = fallback
        .into_iter()
        .map(|row| (text(&row, &["seat"]).to_owned(), row))
        .collect();
    for row in rows {
        let seat = text(row, &["seat", "seat_id", "actor_id"]);
        if !seat.is_empty()
            && (row.get("observed_secs").is_some() || row.get("active_pct").is_some())
        {
            let entry = seats
                .entry(seat.to_owned())
                .or_insert_with(|| json!({"seat":seat}));
            if let Some(fields) = row.as_object() {
                for (key, value) in fields {
                    if !value.is_null() {
                        entry[key] = value.clone();
                    }
                }
            }
        }
    }
    let mut seats: Vec<_> = seats.into_values().collect();
    for row in &mut seats {
        let seat = text(row, &["seat", "seat_id", "actor_id"]).to_owned();
        let observed = number(row, &["observed_secs"]);
        let active = number(row, &["active_pct"])
            .max(100.0 * ratio(number(row, &["active_secs"]), observed));
        let idle = number(row, &["idle_work_pct"])
            .max(100.0 * ratio(number(row, &["idle_with_work_secs"]), observed));
        let hold = number(row, &["idle_hold_pct"]).max(
            100.0
                * ratio(
                    number(row, &["unparked_container_secs", "waiting_external_secs"]),
                    observed,
                ),
        );
        let ceremony = row
            .get("ceremony_pct")
            .and_then(|v| v.as_f64())
            .unwrap_or_else(|| {
                100.0
                    * row
                        .get("ceremony_ratio")
                        .and_then(|v| v.as_f64())
                        .unwrap_or_else(|| ratio(number(row, &["ceremony_secs"]), observed))
            });
        row["seat"] = json!(seat);
        row["seat_id"] = json!(seat);
        row["rig"] = json!(text(row, &["rig", "scope_id"]).to_owned());
        row["active_pct"] = json!(active.clamp(0.0, 100.0));
        row["idle_work_pct"] = json!(idle.clamp(0.0, 100.0));
        row["idle_hold_pct"] = json!(hold.clamp(0.0, 100.0));
        row["ceremony_pct"] = json!(ceremony.clamp(0.0, 100.0));
        row["ceremony_ratio"] = json!(ceremony / 100.0);
        row["first_pass_yield_pct"] = json!(number(row, &["first_pass_yield_pct"]));
        row["turns"] = json!(number(row, &["turns"]) as i64);
        row["tokens"] = json!(number(row, &["tokens"]) as i64);
        row["status"] = json!(if observed > 0.0 {
            "observed"
        } else {
            "telemetry_only"
        });
        for field in [
            "observed_secs",
            "active_secs",
            "idle_with_work_secs",
            "idle_between_work_secs",
            "unparked_container_secs",
            "leaf_stall_secs",
        ] {
            if row.get(field).is_none() {
                row[field] = json!(0);
            }
        }
    }
    let sum = |field: &str| seats.iter().map(|r| number(r, &[field])).sum::<f64>();
    let n = seats.len() as f64;
    payload["summary"] = json!({"active_pct":ratio(sum("active_pct"),n),"yield_pct":ratio(sum("first_pass_yield_pct"),n),"idle_work_pct":ratio(sum("idle_work_pct"),n),"holds":sum("unparked_container_secs") as i64,"ceremony_pct":ratio(sum("ceremony_pct"),n),"unparked_holds_sec":sum("unparked_container_secs"),"avg_ceremony_ratio":ratio(sum("ceremony_pct"),n)/100.0,"fleet_m1_yield":null});
    set_view_rows(payload, "seats", "v_seat_idle_ranking", seats);
    Ok(())
}

fn handoff_payload(
    conn: &duckdb::Connection,
    payload: &mut serde_json::Value,
    rows: &[serde_json::Value],
) -> Result<()> {
    use serde_json::json;
    let mut pairs: Vec<_> = rows
        .iter()
        .filter(|r| !text(r, &["from_seat", "source_seat", "from_actor"]).is_empty())
        .cloned()
        .collect();
    if pairs.is_empty() {
        pairs = json_rows(conn,
            "WITH items AS (SELECT source_seat,dest_seat,qitem_id,MIN(ts) queued,
                MIN(ts) FILTER (WHERE state IN ('claimed','in_progress','running')) started,
                MAX(ts) FILTER (WHERE state IN ('closed','done','completed')) finished,
                BOOL_OR(is_recovery OR state IN ('returned','rework')) returned
             FROM queue_facts GROUP BY source_seat,dest_seat,qitem_id)
             SELECT source_seat from_seat,dest_seat to_seat,SUBSTR(MAX(COALESCE(finished,started,queued))::VARCHAR,1,10) date_bucket,'openrig' harness,COUNT(*)::BIGINT handoffs,
                COUNT(*) FILTER (WHERE returned)::BIGINT rework_returns,
                COALESCE(MEDIAN(GREATEST(0,epoch(started)-epoch(queued))) FILTER (WHERE started IS NOT NULL),0)*1000 wait_median_ms,
                COALESCE(MEDIAN(GREATEST(0,epoch(finished)-epoch(started))) FILTER (WHERE started IS NOT NULL AND finished IS NOT NULL),0)*1000 exec_turnaround_median_ms,
                CASE WHEN source_seat=dest_seat THEN 'self' ELSE 'actor' END ring_boundary
             FROM items GROUP BY source_seat,dest_seat ORDER BY handoffs DESC,source_seat,dest_seat",[])?;
    }
    if pairs.is_empty() {
        pairs = json_rows(conn,"SELECT harness,parent_session native_session,parent_session from_seat,child_session to_seat,SUBSTR(MAX(observed_at)::VARCHAR,1,10) date_bucket,COUNT(*)::BIGINT handoffs,COUNT(*) FILTER (WHERE status IN ('returned','rework'))::BIGINT rework_returns,relation ring_boundary FROM session_edges GROUP BY harness,parent_session,child_session,relation ORDER BY handoffs DESC,from_seat,to_seat",[])?;
    }
    for row in &mut pairs {
        let from = text(row, &["from_seat", "source_seat", "from_actor", "from"]).to_owned();
        let to = text(row, &["to_seat", "dest_seat", "to_actor", "to"]).to_owned();
        let handoffs = number(row, &["handoffs", "count"]) as i64;
        let returns = number(row, &["returns", "rework_returns", "return_count"]) as i64;
        let rate = ratio(returns as f64, handoffs as f64);
        let wait = row
            .get("median_wait_s")
            .and_then(|v| v.as_f64())
            .unwrap_or_else(|| number(row, &["wait_median_ms"]) / 1000.0);
        let exec = row
            .get("median_exec_s")
            .and_then(|v| v.as_f64())
            .unwrap_or_else(|| {
                number(row, &["exec_turnaround_median_ms", "exec_median_ms"]) / 1000.0
            });
        let boundary = text(row, &["boundary", "ring_boundary"]).to_owned();
        let dimensions = row.clone();
        *row = json!({"from_seat":from,"to_seat":to,"from":from,"to":to,"boundary":boundary,"ring_boundary":boundary,"handoffs":handoffs,"returns":returns,"rework_returns":returns,"return_rate_pct":100.0*rate,"return_rate":rate,"median_wait_s":wait,"median_exec_s":exec,"wait_median_ms":wait*1000.0,"exec_turnaround_median_ms":exec*1000.0});
        for key in [
            "harness",
            "rig",
            "date_bucket",
            "source_keys",
            "window_to",
            "native_session",
        ] {
            if !dimensions[key].is_null() {
                row[key] = dimensions[key].clone();
            }
        }
    }
    let total = pairs.iter().map(|r| number(r, &["handoffs"])).sum::<f64>();
    let returns = pairs.iter().map(|r| number(r, &["returns"])).sum::<f64>();
    let median = |key: &str| {
        let mut v: Vec<_> = pairs.iter().map(|r| number(r, &[key])).collect();
        v.sort_by(f64::total_cmp);
        if v.is_empty() {
            0.0
        } else {
            (v[(v.len() - 1) / 2] + v[v.len() / 2]) / 2.0
        }
    };
    let boundaries = pairs
        .iter()
        .filter_map(|r| r["boundary"].as_str().filter(|s| !s.is_empty()))
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    payload["summary"] = json!({"total_handoffs":total as i64,"return_rate_pct":100.0*ratio(returns,total),"fleet_return_rate":ratio(returns,total),"median_wait_s":median("median_wait_s"),"median_exec_s":median("median_exec_s"),"wait_median_secs":median("median_wait_s"),"exec_median_secs":median("median_exec_s"),"boundaries":boundaries,"boundary_crossings":boundaries});
    set_view_rows(payload, "pairs", "v_handoff_rework_analysis", pairs);
    Ok(())
}

fn normalize_mission(row: &serde_json::Value) -> serde_json::Value {
    use serde_json::json;
    let fresh = number(
        row,
        &["fresh_input_tokens", "fresh_input", "input_uncached"],
    ) as i64;
    let cache = number(row, &["cache_read_tokens", "cache_read"]) as i64;
    let write = number(row, &["cache_write_tokens", "cache_write"]) as i64;
    let output = number(row, &["output_tokens", "output"]) as i64;
    let total = fresh + cache + write + output;
    let cache_ratio = ratio(cache as f64, (fresh + cache + write) as f64);
    let harness = text(row, &["harness"]);
    let seat = text(row, &["seat", "seat_id", "actor", "actor_id"]);
    let mission = text(row, &["mission_id", "mission", "title"]);
    let title = if mission.is_empty() {
        format!(
            "Direct Session · {}",
            if harness.is_empty() {
                "unscoped"
            } else {
                harness
            }
        )
    } else {
        text(row, &["title", "mission", "mission_id"]).to_owned()
    };
    let id = if mission.is_empty() {
        text(row, &["native_session", "session"])
    } else {
        mission
    };
    json!({"mission_id":id,"mission":title,"title":title,"seat":seat,"seat_id":seat,"actor":seat,"harness":harness,"native_session":row["native_session"],"rig":row["rig"],"date_bucket":row["date_bucket"],
        "total_tokens":total,"fresh_input_tokens":fresh,"fresh_input":fresh,"input_uncached":fresh,"cache_read_tokens":cache,"cache_read":cache,"cache_write":write,"output_tokens":output,"output":output,
        "cache_ratio_pct":cache_ratio*100.0,"cache_ratio":cache_ratio,"intake_overhead_pct":number(row,&["intake_overhead_pct"]),"turns":number(row,&["turns","count"]) as i64,"loop_retries":number(row,&["loop_retries","retries"]) as i64,"retries":number(row,&["loop_retries","retries"]) as i64,"cost_usd":number(row,&["cost_usd","currency_total"]),"status":"observed"})
}

fn mission_payload(
    conn: &duckdb::Connection,
    payload: &mut serde_json::Value,
    rows: &[serde_json::Value],
) -> Result<()> {
    use serde_json::json;
    let totals = usage_totals(conn)?;
    conn.execute_batch("CREATE TEMP TABLE IF NOT EXISTS _dashboard_mission_sources (source_key VARCHAR, mission VARCHAR, seat VARCHAR); DELETE FROM _dashboard_mission_sources;")?;
    // Use source-key membership, never aggregate subtraction, to combine partial
    // semantic coverage with direct sessions without duplicating a single token.
    {
        let mut insert = conn.prepare("INSERT INTO _dashboard_mission_sources VALUES (?, ?, ?)")?;
        conn.execute_batch("BEGIN TRANSACTION")?;
        let result = (|| -> Result<()> {
            for row in rows {
                let parsed = row["missions"]
                    .as_str()
                    .and_then(|v| serde_json::from_str::<serde_json::Value>(v).ok());
                let names = parsed.as_ref().unwrap_or(&row["missions"]);
                let label = text(row, &["mission_id", "mission", "title"]);
                let label = if label.is_empty() {
                    names
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|n| n.as_str())
                                .collect::<Vec<_>>()
                                .join(" · ")
                        })
                        .unwrap_or_default()
                } else {
                    label.to_owned()
                };
                if label.is_empty() {
                    continue;
                }
                if let Some(keys) = row["source_keys"].as_array() {
                    for key in keys.iter().filter_map(|v| v.as_str()) {
                        insert.execute(duckdb::params![
                            key,
                            label,
                            text(row, &["seat", "seat_id", "actor_id"])
                        ])?;
                    }
                }
            }
            Ok(())
        })();
        match result {
            Ok(()) => conn.execute_batch("COMMIT")?,
            Err(error) => {
                conn.execute_batch("ROLLBACK")?;
                return Err(error);
            }
        }
    }
    let missions: Vec<_> = json_rows(conn,
        "WITH bindings AS (SELECT harness,native_session,MIN(seat) seat,MIN(rig) rig FROM identity_facts GROUP BY harness,native_session HAVING COUNT(DISTINCT seat)=1 AND COUNT(DISTINCT rig)=1),
         attributed AS (SELECT source_key,MIN(mission) mission,MIN(seat) seat FROM _dashboard_mission_sources GROUP BY source_key HAVING COUNT(DISTINCT mission)=1)
         SELECT u.harness,CASE WHEN a.mission IS NULL THEN u.native_session ELSE a.mission END native_session,
           a.mission,COALESCE(NULLIF(a.seat,''),b.seat,u.harness) seat,COALESCE(b.rig,u.harness) rig,SUBSTR(u.ts::VARCHAR,1,10) date_bucket,
           SUM(input_uncached)::BIGINT input_uncached,SUM(cache_read)::BIGINT cache_read,SUM(cache_write)::BIGINT cache_write,SUM(output_tokens)::BIGINT output,COUNT(*)::BIGINT turns,COALESCE(SUM(currency_total),0)::DOUBLE cost_usd
         FROM usage_facts u LEFT JOIN bindings b USING(harness,native_session) LEFT JOIN attributed a USING(source_key)
         GROUP BY ALL ORDER BY SUM(input_uncached+cache_read+cache_write+output_tokens) DESC,u.harness,native_session", [])?
         .iter().map(normalize_mission).collect();
    let mut summary = totals;
    let cache_ratio = ratio(
        number(&summary, &["cache_read_tokens"]),
        number(&summary, &["fresh_input_tokens"])
            + number(&summary, &["cache_read_tokens"])
            + number(&summary, &["cache_write_tokens"]),
    );
    summary["cache_ratio_pct"] = json!(100.0 * cache_ratio);
    summary["cache_read_ratio"] = json!(cache_ratio);
    summary["intake_overhead_pct"] = json!(0.0);
    summary["task_intake_overhead_pct"] = json!(0.0);
    payload["summary"] = summary;
    set_view_rows(payload, "missions", "v_mission_token_burn", missions);
    Ok(())
}

fn governance_payload(
    conn: &duckdb::Connection,
    payload: &mut serde_json::Value,
    rows: &[serde_json::Value],
    window: &LiveWindow,
) -> Result<()> {
    use serde_json::json;
    let mut records = Vec::new();
    for row in rows {
        let embedded = decoded_json(&row["row_json"]);
        let mut record = if embedded["payload"].is_object() {
            embedded["payload"].clone()
        } else if embedded.is_object() {
            embedded
        } else {
            row.clone()
        };
        for key in [
            "m1_yield",
            "four_eyes_independence",
            "watchdog_recovery",
            "vortex_loops",
            "context_compactions",
            "cross_slice_cache_bleed",
        ] {
            record[key] = decoded_json(&record[key]);
        }
        if record["vortex_loops"].is_null() {
            record["vortex_loops"] = record["handoff_friction"]["vortex_loops"].clone();
        }
        if let Some(loops) = record["vortex_loops"].as_array() {
            record["vortex_loops"] = json!(loops.len());
        }
        if let Some(events) = record["context_compactions"]["events"].as_array() {
            record["context_compactions"] = json!(events.len());
        }
        if record["cross_slice_cache_bleed"].is_object() {
            let tokens = usage_totals(conn)?;
            record["cross_slice_cache_bleed"] = if number(&tokens, &["total_tokens"]) > 0.0 {
                json!(ratio(
                    number(
                        &record["cross_slice_cache_bleed"],
                        &["total_cumulative_bleed_tokens"]
                    ),
                    number(&tokens, &["total_tokens"])
                ))
            } else {
                serde_json::Value::Null
            };
        }
        let review = &record["four_eyes_independence"];
        if review["compliant"].is_null() {
            let checks = review["checks"].as_array();
            let known = checks.is_some_and(|c| {
                !c.is_empty()
                    && c.iter().all(|c| {
                        matches!(c["status"].as_str(), Some("independent" | "same_family"))
                    })
            });
            let compliant = known.then(|| number(review, &["same_family_chains"]) == 0.0);
            record["four_eyes_independence"]["compliant"] = json!(compliant);
        }
        let recovery = &record["watchdog_recovery"];
        if recovery.is_object() && recovery["total_resumes"].is_null() {
            record["watchdog_recovery"]["total_resumes"] =
                json!(number(recovery, &["recovery_closed_items"]) as i64);
        }
        record["window_from"] = json!(iso(window.from));
        record["window_to"] = json!(iso(window.to));
        record["definition_version"] = json!(text(row, &["definition_version"]).to_owned());
        record["rig"] = json!(text(row, &["rig", "scope_id"]).to_owned());
        records.push(record);
    }
    if records.is_empty() {
        let telemetry: i64 = conn.query_row("SELECT (SELECT COUNT(*) FROM usage_facts)+(SELECT COUNT(*) FROM message_facts)+(SELECT COUNT(*) FROM activity_facts)+(SELECT COUNT(*) FROM queue_facts)",[],|r| r.get(0))?;
        if telemetry > 0 {
            let resumes: i64 = conn.query_row(
                "SELECT COUNT(DISTINCT qitem_id) FROM queue_facts WHERE is_recovery",
                [],
                |r| r.get(0),
            )?;
            records.push(json!({"rig":"unscoped","window_from":iso(window.from),"window_to":iso(window.to),"definition_version":"telemetry-only","m1_yield":null,"four_eyes_independence":{"compliant":null,"status":"unknown"},"watchdog_recovery":{"total_resumes":resumes,"status":"observed"},"vortex_loops":null,"context_compactions":null,"fresh":false}));
        }
    }
    let record = records.first().cloned().unwrap_or_default();
    let summary = json!({"m1_yield":record["m1_yield"],"cross_slice_cache_bleed":record["cross_slice_cache_bleed"],"vortex_loops":record["vortex_loops"],"watchdog_health":if record["watchdog_recovery"]["status"] == "observed" { "Observed" } else { "Unknown" },"four_eyes_status":match record["four_eyes_independence"]["compliant"].as_bool() { Some(true)=>"Compliant", Some(false)=>"Violation", None=>"Unknown" },"kpi_m1":record["m1_yield"],"kpi_bleed":record["cross_slice_cache_bleed"],"kpi_vortex":record["vortex_loops"],"kpi_watchdog":record["watchdog_recovery"],"kpi_peer_review":record["four_eyes_independence"]});
    payload["summary"] = summary;
    set_view_rows(payload, "records", "v_executive_governance", records);
    Ok(())
}

fn enrich_payload_groupings(
    conn: &duckdb::Connection,
    payload: &mut serde_json::Value,
    view: &str,
) -> Result<()> {
    use serde_json::json;
    // Resolve dimensions from aggregate evidence, rather than loading message
    // content or assigning every row the report's end date. Identity timestamps
    // describe attribution, not the time of the attributed event.
    let dimensions = json_rows(conn, "WITH bindings AS MATERIALIZED (
        SELECT harness,native_session,MIN(seat) seat,MIN(rig) rig FROM identity_facts GROUP BY ALL HAVING COUNT(DISTINCT seat)=1 AND COUNT(DISTINCT rig)=1),
        events AS MATERIALIZED (
            SELECT harness,native_session,ts FROM usage_facts UNION ALL SELECT harness,native_session,ts FROM message_facts
            UNION ALL SELECT harness,native_session,ts FROM security_facts UNION ALL SELECT harness,native_session,ts FROM tool_facts
            UNION ALL SELECT harness,parent_session,observed_at FROM session_edges
            UNION ALL SELECT p.harness,p.session_link,COALESCE(MAX(s.ts),p.last_seen) FROM process_identities p LEFT JOIN process_samples s USING(identity_key) GROUP BY p.identity_key,p.harness,p.session_link,p.last_seen),
        facts AS MATERIALIZED (SELECT e.harness,e.native_session,b.seat,b.rig,e.ts FROM events e LEFT JOIN bindings b USING(harness,native_session)
            UNION ALL SELECT COALESCE(b.harness,'openrig'),b.native_session,a.seat,b.rig,a.ts FROM activity_facts a LEFT JOIN bindings b USING(seat)
            UNION ALL SELECT 'openrig',NULL,q.source_seat,b.rig,q.ts FROM queue_facts q LEFT JOIN (SELECT seat,MIN(rig) rig FROM identity_facts GROUP BY seat HAVING COUNT(DISTINCT rig)=1) b ON b.seat=q.source_seat)
        SELECT 'session' dimension,native_session key,harness,CASE WHEN COUNT(DISTINCT rig)=1 THEN MIN(rig) END rig,SUBSTR(MAX(ts)::VARCHAR,1,10) date_bucket FROM facts WHERE native_session IS NOT NULL GROUP BY harness,native_session
        UNION ALL SELECT 'seat',seat,CASE WHEN COUNT(DISTINCT harness)=1 THEN MIN(harness) ELSE 'mixed' END,CASE WHEN COUNT(DISTINCT rig)=1 THEN MIN(rig) END,SUBSTR(MAX(ts)::VARCHAR,1,10) FROM facts WHERE seat IS NOT NULL GROUP BY seat
        UNION ALL SELECT 'harness',harness,harness,CASE WHEN COUNT(DISTINCT rig)=1 THEN MIN(rig) END,SUBSTR(MAX(ts)::VARCHAR,1,10) FROM facts GROUP BY harness", [])?;
    let lookup: HashMap<_, _> = dimensions
        .iter()
        .map(|r| {
            let dimension = text(r, &["dimension"]);
            (
                (
                    dimension,
                    text(r, &["key"]),
                    if dimension == "session" {
                        text(r, &["harness"])
                    } else {
                        ""
                    },
                ),
                r,
            )
        })
        .collect();
    let latest_event_date = dimensions
        .iter()
        .filter_map(|row| row["date_bucket"].as_str())
        .max();
    let key = match view {
        "v_seat_idle_ranking" => "seats",
        "v_handoff_rework_analysis" => "pairs",
        "v_mission_token_burn" => "missions",
        "v_executive_governance" => "records",
        _ => "rows",
    };
    let mut rows = payload[key].as_array().cloned().unwrap_or_default();
    for row in &mut rows {
        let session = text(
            row,
            &[
                "session_key",
                "native_session",
                "origin_session",
                "parent_session",
                "session",
            ],
        );
        let seat = text(
            row,
            &["seat", "seat_id", "actor_id", "from_seat", "source_seat"],
        );
        let harness = text(row, &["harness", "tool", "tool_name"]);
        let evidence = lookup
            .get(&("session", session, harness))
            .or_else(|| lookup.get(&("seat", seat, "")))
            .or_else(|| lookup.get(&("harness", harness, "")))
            .or_else(|| lookup.get(&("harness", seat, "")))
            .copied();
        let harness = if !harness.is_empty() {
            harness.to_owned()
        } else {
            evidence
                .map(|r| text(r, &["harness"]).to_owned())
                .unwrap_or_else(|| {
                    if matches!(view, "v_handoff_rework_analysis" | "v_executive_governance") {
                        "openrig".into()
                    } else {
                        "unknown".into()
                    }
                })
        };
        let rig = text(row, &["rig", "workspace", "scope_id"]);
        let rig = if !rig.is_empty() && !matches!(rig, "unscoped" | "default") {
            rig.to_owned()
        } else {
            evidence
                .map(|r| text(r, &["rig"]).to_owned())
                .filter(|r| !r.is_empty())
                .unwrap_or_else(|| harness.clone())
        };
        let date = text(
            row,
            &[
                "date_bucket",
                "ts",
                "recorded_at",
                "observed_at",
                "last_seen_at",
                "first_seen_at",
                "last_seen",
            ],
        );
        let date = if !date.is_empty() {
            Some(date.chars().take(10).collect::<String>())
        } else {
            evidence
                .map(|r| text(r, &["date_bucket"]).to_owned())
                .filter(|d| !d.is_empty())
                .or_else(|| {
                    (view == "v_executive_governance")
                        .then(|| latest_event_date.map(str::to_owned))
                        .flatten()
                })
        };
        row["harness"] = json!(harness);
        row["rig"] = json!(rig);
        row["date_bucket"] = json!(date);
        if row["total_tokens"].is_null()
            && (!row["tokens"].is_null() || !row["observed_tokens"].is_null())
        {
            row["total_tokens"] = json!(number(row, &["tokens", "observed_tokens"]) as i64);
        }
    }
    payload[key] = json!(rows);
    payload["rows"] = json!(rows);
    payload["groupings"] = row_groupings(&rows, view);
    Ok(())
}

fn row_groupings(rows: &[serde_json::Value], view: &str) -> serde_json::Value {
    use serde_json::json;
    let metric = match view {
        "v_trajectory_vortex" => "turn_k",
        "v_schema_drift" => "l1_distance",
        "v_authority_blast_radius" => "downstream_sessions",
        "v_counterfactual_routing" => "observed_tokens",
        "v_message_mix" => "message_count",
        "v_subagent_lineage" => "edge_count",
        "v_security_posture" => "observation_count",
        "v_process_resources" => "sample_count",
        "v_seat_idle_ranking" => "active_pct",
        "v_handoff_rework_analysis" => "handoffs",
        "v_executive_governance" => "m1_yield",
        _ => "total_tokens",
    };
    let average_metric = matches!(metric, "l1_distance" | "active_pct" | "m1_yield");
    let mut groupings = json!({});
    for (name, field) in [
        ("by_harness", "harness"),
        ("by_date", "date_bucket"),
        ("by_rig", "rig"),
        ("by_status", "status"),
    ] {
        let mut groups = std::collections::BTreeMap::<String, (usize, i64, f64, usize)>::new();
        for row in rows {
            let value = text(row, &[field]);
            let entry = groups
                .entry(if value.is_empty() { "unknown" } else { value }.to_owned())
                .or_default();
            entry.0 += 1;
            entry.1 = entry
                .1
                .saturating_add(number(row, &["total_tokens", "tokens", "observed_tokens"]) as i64);
            if row.get(metric).is_some_and(|v| !v.is_null()) {
                entry.2 += number(row, &[metric]);
                entry.3 += 1;
            }
        }
        groupings[name] = json!(groups.into_iter().map(|(value,(count,total_tokens,sum,measured))| {
            let primary = if measured==0 { None } else { Some(if average_metric { sum/measured as f64 } else { sum }) };
            let mut group = json!({"key":value,"label":value,field:value,"count":count,"total_tokens":total_tokens,"share_pct":100.0*ratio(count as f64,rows.len() as f64),"primary_metric":metric,"primary_metric_value":primary});
            if metric != "total_tokens" { group[metric] = json!(primary); }
            group
        }).collect::<Vec<_>>());
    }
    groupings
}

fn enrich_and_reorder_row(
    view_name: &str,
    mut map: serde_json::Map<String, serde_json::Value>,
) -> serde_json::Map<String, serde_json::Value> {
    let is_target = matches!(
        view_name,
        "executive_governance"
            | "v_executive_governance"
            | "seat_dynamics"
            | "v_seat_idle_ranking"
            | "handoff_flow"
            | "v_handoff_rework_analysis"
            | "cost_and_loops"
            | "v_mission_token_burn"
    );

    // 1. Resolve scope_id
    let scope_val = map
        .get("scope_id")
        .or_else(|| map.get("scope"))
        .or_else(|| map.get("rig"))
        .cloned()
        .or_else(|| {
            if is_target {
                Some(serde_json::Value::String("default".to_string()))
            } else {
                None
            }
        });

    // 2. Resolve actor_id
    let actor_val = map
        .get("actor_id")
        .or_else(|| map.get("seat_id"))
        .or_else(|| map.get("seat"))
        .cloned();

    // 3. Resolve from_actor
    let from_actor_val = map
        .get("from_actor")
        .or_else(|| map.get("from_seat"))
        .or_else(|| map.get("source_seat"))
        .or_else(|| map.get("from"))
        .cloned();

    // 4. Resolve to_actor
    let to_actor_val = map
        .get("to_actor")
        .or_else(|| map.get("to_seat"))
        .or_else(|| map.get("dest_seat"))
        .or_else(|| map.get("to"))
        .cloned();

    let mut new_map = serde_json::Map::new();

    // Place scope_id first
    if let Some(scope) = scope_val {
        new_map.insert("scope_id".to_string(), scope);
    }

    // Place actor identifiers next
    if let Some(actor) = actor_val {
        new_map.insert("actor_id".to_string(), actor);
    }
    if let Some(from_a) = from_actor_val {
        new_map.insert("from_actor".to_string(), from_a);
    }
    if let Some(to_a) = to_actor_val {
        new_map.insert("to_actor".to_string(), to_a);
    }

    // Standard columns (excluding keys we already handled or want deferred to the end)
    const EXCLUDED_KEYS: &[&str] = &[
        "scope_id",
        "actor_id",
        "from_actor",
        "to_actor",
        "rig",
        "scope",
        "seat",
        "seat_id",
        "from_seat",
        "to_seat",
        "source_seat",
        "dest_seat",
        "from",
        "to",
    ];

    for (k, v) in map.iter() {
        if !EXCLUDED_KEYS.contains(&k.as_str()) {
            new_map.insert(k.clone(), v.clone());
        }
    }

    // Append raw/legacy keys after the clean actor/scope/metrics columns
    const LEGACY_APPEND_ORDER: &[&str] = &[
        "scope",
        "rig",
        "seat",
        "seat_id",
        "from_seat",
        "to_seat",
        "source_seat",
        "dest_seat",
        "from",
        "to",
    ];

    for key in LEGACY_APPEND_ORDER {
        if let Some(val) = map.remove(*key) {
            new_map.insert(key.to_string(), val);
        }
    }

    new_map
}

fn query_uncapped_view_or_fallback(
    conn: &duckdb::Connection,
    view_name: &str,
    use_semantic: bool,
    active_window_bounds: Option<(&str, &str)>,
) -> Result<Vec<serde_json::Value>> {
    let exists: i64 = conn.query_row(
        "SELECT COUNT(*) FROM information_schema.tables WHERE table_name = ?",
        duckdb::params![view_name],
        |r| r.get(0),
    )?;
    // Aggregate snapshots cannot be clipped by their report date: recompute
    // from the request-local, filtered facts instead. Mission snapshots supply
    // attribution only; mission_payload re-aggregates their source keys.
    if exists > 0
        && use_semantic
        && (active_window_bounds.is_none() || view_name == "v_mission_token_burn")
    {
        if let Ok(rows) = json_rows(
            conn,
            &format!("SELECT * FROM {}", sql_identifier(view_name)),
            [],
        ) {
            if !rows.is_empty() {
                return Ok(rows
                    .into_iter()
                    .map(|row| {
                        serde_json::Value::Object(enrich_and_reorder_row(
                            view_name,
                            row.as_object().cloned().unwrap_or_default(),
                        ))
                    })
                    .collect());
            }
        }
    }
    let sql = match view_name {
        "v_message_mix" => "SELECT harness,native_session,SUBSTR(ts::VARCHAR,1,10) date_bucket,role,direction,modalities modality,COUNT(*)::BIGINT message_count FROM message_facts GROUP BY ALL ORDER BY message_count DESC,harness,native_session,date_bucket",
        "v_subagent_lineage" => "SELECT harness,parent_session,SUBSTR(observed_at::VARCHAR,1,10) date_bucket,COUNT(DISTINCT child_session)::BIGINT child_count,COUNT(*)::BIGINT edge_count,COUNT(*) FILTER (WHERE status='started')::BIGINT started_count,COUNT(*) FILTER (WHERE status='finished')::BIGINT finished_count,COUNT(*) FILTER (WHERE status='failed')::BIGINT failed_count,COUNT(*) FILTER (WHERE status='unknown')::BIGINT unknown_count FROM session_edges GROUP BY ALL",
        "v_security_posture" => "SELECT harness,native_session,SUBSTR(ts::VARCHAR,1,10) date_bucket,kind,value,source_field,COUNT(*)::BIGINT count,COUNT(*)::BIGINT observation_count FROM security_facts GROUP BY ALL",
        "v_process_resources" => "WITH samples AS (SELECT identity_key,SUBSTR(ts::VARCHAR,1,10) date_bucket,rss_unit,COUNT(*)::BIGINT sample_count,CASE WHEN COUNT(*)=COUNT(rss_bytes) THEN MAX(rss_bytes) END max_rss_bytes FROM process_samples GROUP BY ALL)
            SELECT p.root_key,p.harness,COALESCE(s.date_bucket,SUBSTR(p.last_seen::VARCHAR,1,10)) date_bucket,s.rss_unit,COUNT(DISTINCT p.identity_key)::BIGINT process_count,COALESCE(SUM(s.sample_count),0)::BIGINT sample_count,MAX(s.max_rss_bytes)::BIGINT max_rss_bytes FROM process_identities p LEFT JOIN samples s USING(identity_key) GROUP BY ALL ORDER BY sample_count DESC,p.root_key",
        "v_trajectory_vortex" | "v_schema_drift" | "v_authority_blast_radius" | "v_counterfactual_routing" => return intelligence_fallback(conn, view_name),
        // The view-specific adapters below build complete payloads from facts.
        "v_mission_token_burn" | "v_seat_idle_ranking" | "v_handoff_rework_analysis" | "v_executive_governance" => return Ok(Vec::new()),
        _ => return Ok(Vec::new()),
    };
    json_rows(conn, sql, [])
}

fn intelligence_fallback(conn: &duckdb::Connection, view: &str) -> Result<Vec<serde_json::Value>> {
    use serde_json::json;
    let sql = match view {
        "v_trajectory_vortex" => "WITH turns AS (
            SELECT harness,native_session session_key,COUNT(*)::BIGINT turn_k,
                ROUND(SUM(CASE WHEN role IN ('assistant','tool') THEN 1 ELSE 0 END)::DOUBLE/GREATEST(COUNT(*),1),3) self_transition_rate,
                COALESCE(STDDEV_POP(COALESCE(TRY_CAST(json_extract_string(to_json(m),'$.content_length') AS DOUBLE),LENGTH(json_extract_string(to_json(m),'$.content')),LENGTH(COALESCE(role,'') || COALESCE(modalities,'')))),0) length_variance,
                SUBSTR(MAX(ts)::VARCHAR,1,10) date_bucket
            FROM message_facts m WHERE NULLIF(native_session,'') IS NOT NULL GROUP BY harness,native_session HAVING COUNT(*)>=2),
            usage AS (SELECT harness,native_session session_key,COALESCE(SUM(input_uncached+output_tokens+cache_read+cache_write),0)::BIGINT total_tokens FROM usage_facts GROUP BY ALL)
            SELECT t.*,LEAST(0.82,GREATEST(0.08,0.08+0.74*length_variance/(length_variance+turn_k+1))) cosine_distance,
                COALESCE(u.total_tokens,0)::BIGINT total_tokens FROM turns t LEFT JOIN usage u USING(harness,session_key) ORDER BY turn_k DESC,harness,session_key",
        "v_schema_drift" => "WITH observations AS MATERIALIZED (
            SELECT COALESCE(harness,'unknown') harness,
                COALESCE(json_extract_string(to_json(f),'$.harness_version'),json_extract_string(to_json(f),'$.version'),'observed') version,
                SUBSTR(COALESCE(json_extract_string(to_json(f),'$.ts'),json_extract_string(to_json(f),'$.recorded_at'),json_extract_string(to_json(f),'$.last_seen_at'),json_extract_string(to_json(f),'$.first_seen_at'),json_extract_string(to_json(f),'$.observed_at')),1,10) date_bucket,
                COALESCE(json_extract_string(to_json(f),'$.fingerprint'),json_extract_string(to_json(f),'$.key_path'),json_extract_string(to_json(f),'$.schema_hash'),'envelope') category
            FROM envelope_fingerprints f
            UNION ALL SELECT harness,'metadata',SUBSTR(ts::VARCHAR,1,10),'message:' || COALESCE(role,'unknown') || ':' || COALESCE(modalities,'unknown') FROM message_facts WHERE NOT EXISTS (SELECT 1 FROM envelope_fingerprints)
            UNION ALL SELECT harness,'metadata',SUBSTR(ts::VARCHAR,1,10),'model:' || COALESCE(json_extract_string(to_json(u),'$.model'),'unknown') FROM usage_facts u WHERE NOT EXISTS (SELECT 1 FROM envelope_fingerprints)
            UNION ALL SELECT COALESCE(b.harness,'openrig'),'metadata',SUBSTR(a.ts::VARCHAR,1,10),'activity:' || COALESCE(a.state,'unknown') FROM activity_facts a LEFT JOIN (SELECT seat,MIN(harness) harness FROM identity_facts GROUP BY seat HAVING COUNT(DISTINCT harness)=1) b USING(seat) WHERE NOT EXISTS (SELECT 1 FROM envelope_fingerprints)),
            counts AS MATERIALIZED (SELECT harness,date_bucket,category,MAX(version) version,COUNT(*)::BIGINT n FROM observations GROUP BY ALL),
            days AS MATERIALIZED (SELECT harness,date_bucket,SUM(n) n,MAX(version) version FROM counts GROUP BY harness,date_bucket),
            baseline AS MATERIALIZED (SELECT * FROM days QUALIFY ROW_NUMBER() OVER (PARTITION BY harness ORDER BY date_bucket NULLS LAST)=1),
            vocabulary AS MATERIALIZED (SELECT harness,date_bucket,category FROM counts UNION SELECT d.harness,d.date_bucket,c.category FROM days d JOIN baseline b USING(harness) JOIN counts c ON c.harness=b.harness AND c.date_bucket IS NOT DISTINCT FROM b.date_bucket)
            SELECT d.harness,d.date_bucket,d.version || ' · ' || COALESCE(d.date_bucket,'undated') harness_version,d.n::BIGINT observed_envelopes,
                COUNT(*) FILTER (WHERE c.n IS NOT NULL AND bc.n IS NULL)::BIGINT keys_added,
                COUNT(*) FILTER (WHERE c.n IS NULL AND bc.n IS NOT NULL)::BIGINT keys_removed,0::BIGINT keys_retyped,
                COUNT(*) FILTER (WHERE c.n IS NOT NULL)::BIGINT observed_modalities,
                LEAST(0.45,GREATEST(0.01,SUM(ABS(COALESCE(c.n,0)::DOUBLE/d.n-COALESCE(bc.n,0)::DOUBLE/b.n))/2)) l1_distance
            FROM days d JOIN baseline b USING(harness) JOIN vocabulary v ON v.harness=d.harness AND v.date_bucket IS NOT DISTINCT FROM d.date_bucket
            LEFT JOIN counts c ON c.harness=v.harness AND c.date_bucket IS NOT DISTINCT FROM v.date_bucket AND c.category=v.category
            LEFT JOIN counts bc ON bc.harness=b.harness AND bc.date_bucket IS NOT DISTINCT FROM b.date_bucket AND bc.category=v.category
            GROUP BY d.harness,d.date_bucket,d.version,d.n ORDER BY d.date_bucket DESC,d.harness",
        "v_authority_blast_radius" => "WITH RECURSIVE edges AS (SELECT DISTINCT harness,parent_session,child_session,observed_at FROM session_edges WHERE NULLIF(parent_session,'') IS NOT NULL AND NULLIF(child_session,'') IS NOT NULL),
            walk(harness,origin_session,child_session,depth,path,ts) AS (
                SELECT harness,parent_session,child_session,1,[parent_session,child_session],observed_at FROM edges WHERE parent_session<>child_session
                UNION ALL SELECT w.harness,w.origin_session,e.child_session,w.depth+1,list_append(w.path,e.child_session),GREATEST(w.ts,e.observed_at) FROM walk w JOIN edges e ON e.harness=w.harness AND e.parent_session=w.child_session WHERE w.depth<4 AND NOT list_contains(w.path,e.child_session)),
            graph AS (SELECT harness,origin_session,COUNT(DISTINCT child_session)::BIGINT downstream,MAX(depth)::BIGINT max_hop,MAX(ts) ts FROM walk GROUP BY ALL),
            checks AS (SELECT harness,native_session origin_session,ts,'security:' || COALESCE(source_field,kind) scope FROM security_facts
                UNION ALL SELECT COALESCE(b.harness,'openrig'),COALESCE(b.native_session,a.seat),a.ts,'activity:' || a.seat || ':' || a.state FROM activity_facts a LEFT JOIN (SELECT seat,MIN(harness) harness,MIN(native_session) native_session FROM identity_facts GROUP BY seat HAVING COUNT(DISTINCT harness || ':' || native_session)=1) b USING(seat)),
            authority AS (SELECT harness,origin_session,COUNT(*)::BIGINT cycles_touched,COUNT(DISTINCT scope)::BIGINT scopes,MAX(ts) ts FROM checks WHERE NULLIF(origin_session,'') IS NOT NULL GROUP BY ALL)
            SELECT COALESCE(g.harness,a.harness) harness,COALESCE(g.origin_session,a.origin_session) origin_session,
                (COALESCE(g.downstream,0)+COALESCE(a.scopes,0))::BIGINT downstream_sessions,COALESCE(g.max_hop,1)::BIGINT max_hop,
                COALESCE(a.cycles_touched,0)::BIGINT cycles_touched,SUBSTR(GREATEST(g.ts,a.ts)::VARCHAR,1,10) date_bucket
            FROM graph g FULL OUTER JOIN authority a USING(harness,origin_session) ORDER BY downstream_sessions DESC,cycles_touched DESC,harness,origin_session LIMIT 200",
        "v_counterfactual_routing" => "WITH sessions AS (
            SELECT harness,native_session session_key,COALESCE(json_extract_string(to_json(u),'$.model'),'unknown') model,
                COALESCE(SUM(input_uncached+output_tokens+cache_read+cache_write),0)::BIGINT observed_tokens,
                COALESCE(SUM(input_uncached+cache_read+cache_write),0)::DOUBLE input_tokens,COALESCE(SUM(cache_read),0)::DOUBLE cached_tokens,
                COALESCE(AVG(input_uncached+cache_read+cache_write),0)::DOUBLE context_tokens,COALESCE(SUM(currency_total),0)::DOUBLE observed_cost,
                SUBSTR(MAX(ts)::VARCHAR,1,10) date_bucket
            FROM usage_facts u WHERE NULLIF(native_session,'') IS NOT NULL GROUP BY ALL)
            SELECT *,LEAST(0.98,GREATEST(0.10,context_tokens/128000.0)) simulated_context_pressure,
                ROUND(100.0*(1.0-CASE WHEN input_tokens>0 THEN cached_tokens/input_tokens ELSE 0 END),1) cache_bleed_inflection,
                ROUND(CAST(observed_tokens AS DOUBLE) * 0.75, 4) simulated_cost,
                'normalized_token_units' AS simulated_cost_unit
            FROM sessions ORDER BY observed_tokens DESC,harness,session_key,model LIMIT 200",
        _ => return Ok(Vec::new()),
    };
    let mut rows = json_rows(conn, sql, [])?;
    for row in &mut rows {
        let (status, reason) = match view {
            "v_trajectory_vortex" => {
                let turns = number(row, &["turn_k"]) as i64;
                let rate = number(row, &["self_transition_rate"]);
                let tripped = turns >= 25 && rate >= 0.55;
                row["tripped"] = json!(tripped);
                row["metric_basis"] = json!("Length-variance proxy; metadata lengths when content lengths are unavailable. Not an embedding cosine measurement.");
                (
                    if tripped {
                        "VORTEX_TRIPPED"
                    } else if turns >= 12 && rate >= 0.45 {
                        "ELEVATED_LOOP"
                    } else {
                        "CONVERGING"
                    },
                    format!(
                        "{turns} turns · {:.0}% self-transition rate ({} tokens)",
                        rate * 100.0,
                        number(row, &["total_tokens"]) as i64
                    ),
                )
            }
            "v_schema_drift" => {
                let distance = number(row, &["l1_distance"]);
                (if distance >= 0.25 { "SCHEMA_DRIFT" } else if distance >= 0.10 { "MINOR_SHIFT" } else { "STABLE" },
                    format!("{} observed envelopes · {} modalities/signatures · {:.3} distribution shift from first observed day; retyping unavailable without typed envelopes",number(row,&["observed_envelopes"]) as i64,number(row,&["observed_modalities"]) as i64,distance))
            }
            "v_authority_blast_radius" => {
                let downstream = number(row, &["downstream_sessions"]) as i64;
                let hop = number(row, &["max_hop"]) as i64;
                row["non_claim"] =
                    json!("Observed dependency exposure only — not a causal fault claim");
                (
                    if hop >= 3 || downstream >= 5 {
                        "HIGH_EXPOSURE"
                    } else if downstream >= 2 {
                        "BOUNDED_FANOUT"
                    } else {
                        "CONTAINED"
                    },
                    format!(
                        "{downstream} downstream scopes · max hop {hop} · {} authority checks",
                        number(row, &["cycles_touched"]) as i64
                    ),
                )
            }
            _ => {
                let bleed = number(row, &["cache_bleed_inflection"]);
                row["total_tokens"] = row["observed_tokens"].clone();
                row["simulation_basis"] = json!("Mean observed input against a 128k assumed context envelope; illustrative 25% tier/cache discount on observed cost, or normalized token units.");
                (
                    if bleed >= 35.0 {
                        "CACHE_BLEED_INFLECTION"
                    } else if number(row, &["simulated_context_pressure"]) >= 0.65 {
                        "ROUTING_CANDIDATE"
                    } else {
                        "OPTIMAL_ENVELOPE"
                    },
                    format!(
                        "Model {} · {} tokens · {bleed:.1}% uncached bleed",
                        text(row, &["model"]),
                        number(row, &["observed_tokens"]) as i64
                    ),
                )
            }
        };
        row["status"] = json!(status);
        row["reason"] = json!(reason);
    }
    Ok(rows)
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

async fn execute_live_query_async(
    db_path: &Path,
    view: &str,
    window: Option<&str>,
    rig: Option<&str>,
) -> Result<serde_json::Value> {
    let db = db_path.to_path_buf();
    let view = view.to_owned();
    let window = window.map(str::to_owned);
    let rig = rig.map(str::to_owned);
    tokio::task::spawn_blocking(move || {
        execute_live_query(&db, &view, window.as_deref(), rig.as_deref())
    })
    .await?
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
        let query_window = match (params.get("from"), params.get("to")) {
            (Some(from), Some(to)) => Some(format!("{from}..{to}")),
            _ => params.get("window").cloned().or_else(|| window.clone()),
        };

        let mut payload = match execute_live_query_async(
            &db_path,
            view_id,
            query_window.as_deref(),
            query_rig.as_deref(),
        )
        .await
        {
            Ok(p) => p,
            Err(e) => serde_json::json!({
                "error": e.to_string(),
                "live": true,
                "view": view_id,
                "rows": [],
                "table_counts": {},
            }),
        };

        if let Some(expected) = params.get("expect_evidence_digest") {
            if payload["direct_overview"]["digest"].as_str() != Some(expected.as_str()) {
                payload = serde_json::json!({"error":"evidence digest mismatch; refresh the overview before reopening this lens","rows":[]});
            }
        }
        let body = serde_json::to_vec(&payload)?;
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: http://127.0.0.1:4242\r\n{NO_CACHE_HEADERS}Connection: close\r\n\r\n",
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
        let query_window = match (params.get("from"), params.get("to")) {
            (Some(from), Some(to)) => Some(format!("{from}..{to}")),
            _ => params.get("window").cloned().or_else(|| window.clone()),
        };

        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n{NO_CACHE_HEADERS}Connection: keep-alive\r\nAccess-Control-Allow-Origin: http://127.0.0.1:4242\r\n\r\n"
        );
        stream.write_all(header.as_bytes()).await?;

        let initial_payload = match execute_live_query_async(
            &db_path,
            view_id,
            query_window.as_deref(),
            query_rig.as_deref(),
        )
        .await
        {
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
                if let Ok(updated) = execute_live_query_async(
                    &db_path,
                    view_id,
                    query_window.as_deref(),
                    query_rig.as_deref(),
                )
                .await
                {
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
                "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: http://127.0.0.1:4242\r\n{NO_CACHE_HEADERS}Connection: close\r\n\r\n",
                content.len()
            );
            stream.write_all(header.as_bytes()).await?;
            if method == "GET" {
                stream.write_all(&content).await?;
            }
            return Ok(());
        }
    }

    let not_found = format!(
        "HTTP/1.1 404 Not Found\r\nContent-Length: 9\r\n{NO_CACHE_HEADERS}Connection: close\r\n\r\nNot Found"
    );
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

#[cfg(test)]
mod live_payload_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn custom_dates_include_the_last_day_and_have_readable_labels() {
        let c = fixture();
        c.execute_batch("INSERT INTO usage_facts VALUES ('last','codex','direct','2026-10-25 23:59:59.999999',1,0,0,0,0),('next','codex','direct','2026-10-26 00:00:00',9,0,0,0,0)").unwrap();
        let p = query_live_payload(&c, "hub", Some("2026-10-20..2026-10-25"), None).unwrap();
        assert_eq!(p["active_window"]["from"], "2026-10-20T00:00:00Z");
        assert_eq!(p["active_window"]["from_date"], "2026-10-20");
        assert_eq!(p["active_window"]["to_date"], "2026-10-25");
        assert_eq!(p["active_window"]["display"], "Oct 20 – Oct 25, 2026");
        assert_eq!(p["kpis"]["total_tokens"], 1);
        let same = LiveWindow::resolve(Some("2026-10-25..2026-10-25"), None, None).unwrap();
        assert!(same.to > same.from);
        let local =
            LiveWindow::resolve(Some("2026-10-20T09:30..2026-10-20T11:45"), None, None).unwrap();
        assert_eq!(iso(local.from), "2026-10-20T09:30:00Z");
        assert_eq!(iso(local.to), "2026-10-20T11:45:00Z");
        let recorded = &p["recorded_window"];
        assert_eq!(recorded["from_date"], "2024-01-31");
        assert_eq!(recorded["to_date"], "2026-10-26");
        assert_eq!(recorded["display"], "Jan 31, 2024 – Oct 26, 2026");
    }

    #[test]
    fn relative_presets_round_fractional_anchors_without_losing_latest_event() {
        for preset in ["1h", "24h", "7d", "30d"] {
            let w =
                LiveWindow::resolve(Some(preset), None, Some("2026-10-27T07:54:43.759Z")).unwrap();
            assert!(w.to >= parse_live_timestamp("2026-10-27T07:54:43.759Z").unwrap());
            assert!(iso(w.from).ends_with(":00:00Z"));
            assert!(iso(w.to).ends_with(":00:00Z"));
            assert_eq!(
                (w.to - w.from).num_hours(),
                match preset {
                    "1h" => 1,
                    "24h" => 24,
                    "7d" => 192,
                    _ => 744,
                }
            );
        }
    }

    #[test]
    fn capability_intelligence_aggregates_windowed_facts_and_exposes_the_view() {
        let path = std::env::temp_dir().join(format!(
            "dendro-capabilities-{}-{}.duckdb",
            std::process::id(),
            rand::random::<u64>()
        ));
        {
            let c = duckdb::Connection::open(&path).unwrap();
            c.execute_batch("CREATE TABLE _dendro_tool_facts (source_key VARCHAR, harness VARCHAR, native_session VARCHAR, ts TIMESTAMP, tool_name VARCHAR, category VARCHAR, failed BOOLEAN, structured_write BOOLEAN);
                INSERT INTO _dendro_tool_facts VALUES
                ('r1','claude','s','2026-10-25 10:00:00','Read','inspect_read',false,false),
                ('r2','codex','s','2026-10-25 10:00:00','read_file','inspect_read',true,false),
                ('w','agy','s','2026-10-25 10:00:00','Edit','eng_code_edit',false,true),
                ('g','claude','s','2026-10-25 10:00:00','Grep','',false,false),
                ('b','codex','s','2026-10-25 10:00:00','cargo test','eng_build_test',false,false),
                ('v','codex','s','2026-10-25 10:00:00','git commit','eng_vcs',false,false),
                ('f','codex','s','2026-10-25 10:00:00','rtk','',false,false),
                ('j','agy','s','2026-10-25 10:00:00','jq','',false,false),
                ('e','agy','s','2026-10-25 10:00:00','uv','',false,false),
                ('n','agy','s','2026-10-25 10:00:00','curl','',false,false),
                ('m','claude','s','2026-10-25 10:00:00','mcp__holoself__query','',false,false),
                ('k','claude','s','2026-10-25 10:00:00','skill__testing__run','',false,false),
                ('a','agy','s','2026-10-25 10:00:00','invoke_subagent','',false,false),
                ('old','claude','s','2026-10-19 10:00:00','mcp__old__query','',false,false);
                CREATE TABLE _dendro_skill_facts (source_key VARCHAR,seat VARCHAR,skill VARCHAR,invoked BOOLEAN,ts TIMESTAMP);
                INSERT INTO _dendro_skill_facts VALUES ('k','claude','testing',true,'2026-10-25 10:00:00'),('d','claude','dormant',false,'2026-10-25 10:00:00'),('old','claude','expired',true,'2026-10-19 10:00:00');
                CREATE TABLE _dendro_security_facts (source_key VARCHAR,harness VARCHAR,native_session VARCHAR,ts TIMESTAMP,kind VARCHAR,value VARCHAR,source_field VARCHAR);
                INSERT INTO _dendro_security_facts VALUES ('h','codex','s','2026-10-25 10:00:00','permission_decision','allow','policy'),('old','codex','s','2026-10-19 10:00:00','permission_decision','deny','policy');
                CREATE TABLE _dendro_session_edges (source_key VARCHAR,harness VARCHAR,parent_session VARCHAR,child_session VARCHAR,observed_at TIMESTAMP,relation VARCHAR,status VARCHAR);
                INSERT INTO _dendro_session_edges VALUES ('edge','agy','s','child','2026-10-25 10:00:00','spawn','started'),('old','agy','s','child','2026-10-19 10:00:00','spawn','started');").unwrap();
        }
        let p = execute_live_query(
            &path,
            "capability_intelligence",
            Some("2026-10-20..2026-10-25"),
            None,
        )
        .unwrap();
        std::fs::remove_file(path).unwrap();
        let ci = &p["capability_intelligence"];
        for key in ["tool_families", "primitives", "skills", "mcp", "hooks"] {
            assert!(!ci[key].as_array().unwrap().is_empty(), "{key}: {ci}");
        }
        assert_eq!(ci["summary"]["total_tool_calls"], 13);
        assert_eq!(ci["summary"]["primitive_calls"], 10);
        assert_eq!(ci["summary"]["skill_invocations"], 1);
        assert_eq!(ci["summary"]["declared_skills"], 1);
        assert_eq!(ci["summary"]["mcp_calls"], 1);
        assert_eq!(ci["summary"]["subagent_dispatches"], 2);
        assert_eq!(ci["summary"]["hook_interceptions"], 1);
        assert_eq!(ci["tool_families"].as_array().unwrap().len(), 10);
        assert_eq!(
            ci["tool_families"]
                .as_array()
                .unwrap()
                .iter()
                .map(|f| f["calls"].as_u64().unwrap())
                .sum::<u64>(),
            13
        );
        let read = ci["primitives"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["canonical_name"] == "read_file")
            .unwrap();
        assert_eq!(read["calls"], 2);
        assert_eq!(read["failed"], 1);
        assert_eq!(read["failure_rate_pct"], 50.0);
        assert_eq!(ci["mcp"][0]["server"], "holoself");
        assert_eq!(ci["mcp"][0]["tool"], "query");
        assert_eq!(
            ci["hooks"][0]["hook_stage"],
            "PreToolUse / Permission Decision"
        );
        assert_eq!(ci["skills"].as_array().unwrap().len(), 2);
        assert_eq!(p["rows"].as_array().unwrap().len(), 10);
        assert_eq!(
            p["rows"][0]["archetype"],
            ci["tool_families"][0]["archetype"]
        );
    }

    #[test]
    fn undated_skill_facts_require_event_evidence_and_respect_rig_scope() {
        let c = fixture();
        c.execute_batch("CREATE TABLE tool_facts (source_key VARCHAR,harness VARCHAR,native_session VARCHAR,ts TIMESTAMP,tool_name VARCHAR,category VARCHAR,failed BOOLEAN,structured_write BOOLEAN);
            INSERT INTO tool_facts VALUES ('src:1:3:0','codex','direct','2024-02-15 11:40:00','Skill','',false,false),('src:1:2:0','codex','direct','2024-01-31 12:00:00','skill__expired__run','',false,false),('foreign','claude','foreign','2024-02-15 11:40:00','skill__foreign__run','',false,false);
            CREATE TABLE skill_facts (source_key VARCHAR,seat VARCHAR,skill VARCHAR,invoked BOOLEAN);
            INSERT INTO skill_facts VALUES ('src:1:3:skill:0','codex:direct','testing',true),('src:1:2:skill:0','codex:direct','expired',true),('unattributed','codex:direct','undated',true),('declaration','builder','dormant',false),('foreign','claude:foreign','foreign',true);
            CREATE TABLE identity_facts (source_key VARCHAR,harness VARCHAR,native_session VARCHAR,rig VARCHAR,seat VARCHAR,ts TIMESTAMP);
            INSERT INTO identity_facts VALUES ('id','codex','direct','fleet','builder','2024-02-15 11:40:00'),('other','claude','foreign','other','foreign','2024-02-15 11:40:00');").unwrap();
        let p = query_live_payload(
            &c,
            "capability_intelligence",
            Some("2024-02-15..2024-02-15"),
            Some("fleet"),
        )
        .unwrap();
        let ci = &p["capability_intelligence"];
        assert_eq!(ci["summary"]["skill_invocations"], 1);
        assert_eq!(ci["summary"]["declared_skills"], 1);
        assert_eq!(ci["coverage"]["undated_skill_facts_excluded"], 2);
        assert_eq!(ci["skills"].as_array().unwrap().len(), 2);
        assert!(ci["skills"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["skill"] == "dormant" && s["status"] == "DECLARED_DORMANT"));
        assert_eq!(p["active_window"]["display"], "Feb 15, 2024");
    }

    #[test]
    fn mcp_message_evidence_and_hook_stages_are_preserved() {
        let c = fixture();
        c.execute_batch("CREATE TABLE tool_facts (source_key VARCHAR,harness VARCHAR,native_session VARCHAR,ts TIMESTAMP,tool_name VARCHAR,category VARCHAR,failed BOOLEAN,structured_write BOOLEAN);
            INSERT INTO tool_facts VALUES ('dup','codex','direct','2024-02-15 11:40:00','mcp__github__query','',false,false),('alias','codex','direct','2024-02-15 11:40:00','holoself_query','',false,false);
            ALTER TABLE message_facts ADD COLUMN tool_name VARCHAR;
            INSERT INTO message_facts VALUES ('dup','codex','direct','2024-02-15 11:40:00','tool','out','tool_use','mcp__github__query'),('extra','codex','direct','2024-02-15 11:40:00','tool','out','tool_use','mcp__graph__query'),('old-mcp','codex','direct','2024-01-31 12:00:00','tool','out','tool_use','mcp__expired__query');
            INSERT INTO security_facts VALUES ('pre','codex','direct','2024-02-15 11:40:00','hook_event','PreInvocation','hook'),('pt','codex','direct','2024-02-15 11:40:00','hook_event','PreToolUse','hook'),('post-tool','codex','direct','2024-02-15 11:40:00','hook_event','PostToolUse','hook'),('post','codex','direct','2024-02-15 11:40:00','hook_event','PostInvocation','hook'),('stop','codex','direct','2024-02-15 11:40:00','hook_event','Stop','hook');").unwrap();
        let p = query_live_payload(&c, "capability_intelligence", Some("today"), None).unwrap();
        let ci = &p["capability_intelligence"];
        assert_eq!(ci["summary"]["total_tool_calls"], 2);
        assert_eq!(ci["summary"]["mcp_calls"], 3);
        assert_eq!(ci["coverage"]["supplemental_mcp_message_calls"], 1);
        assert_eq!(ci["summary"]["hook_interceptions"], 6);
        for stage in [
            "PreInvocation",
            "PreToolUse",
            "PostToolUse",
            "PostInvocation",
            "Stop",
        ] {
            assert!(
                ci["hooks"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|h| h["hook_stage"] == stage),
                "{stage}: {ci}"
            );
        }
    }

    fn fixture() -> duckdb::Connection {
        let c = duckdb::Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE usage_facts (source_key VARCHAR, harness VARCHAR, native_session VARCHAR, ts TIMESTAMP, input_uncached BIGINT, output_tokens BIGINT, cache_read BIGINT, cache_write BIGINT, currency_total DOUBLE);
            INSERT INTO usage_facts VALUES ('old','codex','old','2024-01-31 12:00:00',1000,100,200,50,1),('boundary','codex','old','2024-02-01 00:00:00',7,0,0,0,0), ('a','codex','direct','2024-02-15 11:30:00',100,20,30,5,0.1), ('b','codex','direct','2024-02-15 12:00:00',200,40,60,10,0.2);
            CREATE TABLE message_facts (source_key VARCHAR, harness VARCHAR, native_session VARCHAR, ts TIMESTAMP, role VARCHAR, direction VARCHAR, modalities VARCHAR);
            INSERT INTO message_facts VALUES ('m','codex','direct','2024-02-15 11:45:00','assistant','out','text');
            CREATE TABLE activity_facts (source_key VARCHAR, seat VARCHAR, ts TIMESTAMP, state VARCHAR);
            INSERT INTO activity_facts VALUES ('act','builder','2024-02-15 11:40:00','running');
            CREATE TABLE security_facts (source_key VARCHAR, harness VARCHAR, native_session VARCHAR, ts TIMESTAMP, kind VARCHAR, value VARCHAR, source_field VARCHAR);
            INSERT INTO security_facts VALUES ('sec','codex','direct','2024-02-15 11:50:00','approval','denied','policy');
            CREATE TABLE queue_facts (source_key VARCHAR, qitem_id VARCHAR, ts TIMESTAMP, state VARCHAR, source_seat VARCHAR, dest_seat VARCHAR, is_recovery BOOLEAN);
            INSERT INTO queue_facts VALUES ('q','item','2024-02-15 11:55:00','claimed','planner','builder',false);
            CREATE TABLE session_edges (source_key VARCHAR, harness VARCHAR, parent_session VARCHAR, child_session VARCHAR, observed_at TIMESTAMP, relation VARCHAR, status VARCHAR);
            INSERT INTO session_edges VALUES ('edge','codex','parent','child','2024-02-15 11:59:00','spawn','finished');").unwrap();
        c
    }

    #[test]
    fn intelligence_fallbacks_use_windowed_telemetry_and_roll_up_rows() {
        for view in [
            "v_trajectory_vortex",
            "v_schema_drift",
            "v_authority_blast_radius",
            "v_counterfactual_routing",
        ] {
            let c = fixture();
            c.execute_batch("INSERT INTO message_facts VALUES ('m2','codex','direct','2024-02-15 11:46:00','tool','out','tool_result');
                CREATE VIEW v_trajectory_vortex AS SELECT 'stale' session_key WHERE false;
                CREATE VIEW v_schema_drift AS SELECT 'stale' harness WHERE false;
                CREATE VIEW v_authority_blast_radius AS SELECT 'stale' origin_session WHERE false;
                CREATE VIEW v_counterfactual_routing AS SELECT 'stale' session_key WHERE false;").unwrap();
            let p = query_live_payload(&c, view, Some("today"), None).unwrap();
            let rows = p["rows"].as_array().unwrap();
            assert!(!rows.is_empty(), "{view}: {p}");
            assert!(
                rows.iter().all(
                    |r| matches!(r["harness"].as_str(), Some("codex" | "openrig"))
                        && r["date_bucket"] == "2024-02-15"
                        && r["rig"] == r["harness"]
                ),
                "{view}: {rows:?}"
            );
            let groups = p["groupings"]["by_harness"].as_array().unwrap();
            assert_eq!(
                groups
                    .iter()
                    .map(|g| g["count"].as_u64().unwrap())
                    .sum::<u64>(),
                rows.len() as u64
            );
            assert!(
                (groups
                    .iter()
                    .map(|g| g["share_pct"].as_f64().unwrap())
                    .sum::<f64>()
                    - 100.0)
                    .abs()
                    < 0.001
            );
            if view == "v_trajectory_vortex" {
                assert_eq!(rows[0]["turn_k"], 2);
                assert_eq!(rows[0]["total_tokens"], 465);
                assert_eq!(rows[0]["tripped"], false);
            }
            if view == "v_counterfactual_routing" {
                assert_eq!(rows[0]["observed_tokens"], 465);
                assert!(rows.iter().all(|r| r["session_key"] != "old"));
            }
        }
    }

    #[test]
    fn empty_or_unreadable_semantic_view_uses_facts_for_all_time() {
        let c = fixture();
        c.execute_batch("INSERT INTO message_facts VALUES ('m2','codex','direct','2024-02-15 11:46:00','tool','out','tool_result');").unwrap();
        prepare_live_facts(&c).unwrap();
        for definition in [
            "SELECT 1 broken WHERE false",
            "SELECT CAST('broken' AS BIGINT) broken",
        ] {
            c.execute_batch(&format!(
                "CREATE OR REPLACE VIEW v_trajectory_vortex AS {definition}"
            ))
            .unwrap();
            let rows =
                query_uncapped_view_or_fallback(&c, "v_trajectory_vortex", true, None).unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0]["session_key"], "direct");
            assert_eq!(rows[0]["total_tokens"], 465);
        }
    }

    #[test]
    fn historical_semantic_rows_do_not_escape_empty_explicit_window() {
        for (view, key) in [
            ("v_seat_idle_ranking", "seats"),
            ("v_handoff_rework_analysis", "pairs"),
            ("v_mission_token_burn", "missions"),
            ("v_executive_governance", "records"),
            ("v_message_mix", "rows"),
            ("v_subagent_lineage", "rows"),
            ("v_security_posture", "rows"),
            ("v_process_resources", "rows"),
            ("v_trajectory_vortex", "rows"),
            ("v_schema_drift", "rows"),
            ("v_authority_blast_radius", "rows"),
            ("v_counterfactual_routing", "rows"),
        ] {
            let c = fixture();
            c.execute_batch(&format!("CREATE VIEW {view} AS SELECT 'old' native_session,'codex' harness,'2024-01-31' date_bucket")).unwrap();
            let p = query_live_payload(
                &c,
                view,
                Some("2030-01-01T00:00:00Z..2030-02-01T00:00:00Z"),
                None,
            )
            .unwrap();
            assert_eq!(p[key], json!([]), "{view}: {p}");
            assert_eq!(p["rows"], json!([]), "{view}: {p}");
            assert_eq!(p["groupings"]["by_date"], json!([]));
        }
    }

    #[test]
    fn dated_fingerprints_and_process_samples_respect_window_and_rig() {
        let c = fixture();
        c.execute_batch("CREATE TABLE identity_facts (source_key VARCHAR,harness VARCHAR,native_session VARCHAR,rig VARCHAR,seat VARCHAR,ts TIMESTAMP);
            INSERT INTO identity_facts VALUES ('i','codex','direct','fleet','builder','2024-01-01');
            CREATE TABLE _dendro_envelope_fingerprints (harness VARCHAR,recorded_at TIMESTAMP,fingerprint VARCHAR,harness_version VARCHAR);
            INSERT INTO _dendro_envelope_fingerprints VALUES ('codex','2024-01-31','old','v1'),('codex','2024-02-15','current','v2');
            CREATE TABLE process_identities (identity_key VARCHAR,root_key VARCHAR,harness VARCHAR,attribution VARCHAR,session_link VARCHAR,first_seen TIMESTAMP,last_seen TIMESTAMP);
            INSERT INTO process_identities VALUES ('p','p','codex','discovered','direct','2024-01-01','2024-03-01');
            CREATE TABLE process_samples (identity_key VARCHAR,ts TIMESTAMP,rss_unit VARCHAR,rss_bytes BIGINT);
            INSERT INTO process_samples VALUES ('p','2024-01-31','bytes',999),('p','2024-02-15 11:45:00','bytes',123);").unwrap();
        let window = Some("2024-02-15..2024-02-16");
        let p = query_live_payload(&c, "v_schema_drift", window, Some("fleet")).unwrap();
        assert_eq!(p["rows"].as_array().unwrap().len(), 1);
        assert_eq!(p["rows"][0]["observed_envelopes"], 1);
        assert_eq!(p["rows"][0]["harness_version"], "v2 · 2024-02-15");
        assert_eq!(p["rows"][0]["rig"], "fleet");
        let p = query_live_payload(&c, "v_process_resources", window, Some("fleet")).unwrap();
        assert_eq!(p["rows"][0]["sample_count"], 1);
        assert_eq!(p["rows"][0]["max_rss_bytes"], 123);
        assert_eq!(p["rows"][0]["date_bucket"], "2024-02-15");
        assert_eq!(p["groupings"]["by_rig"][0]["rig"], "fleet");
        let p = query_live_payload(&c, "v_executive_governance", window, Some("fleet")).unwrap();
        assert_eq!(p["records"][0]["date_bucket"], "2024-02-15");
    }

    #[test]
    fn blast_radius_walks_observed_edges_and_stops_at_cycles() {
        let c = fixture();
        c.execute_batch("INSERT INTO session_edges VALUES ('e2','codex','child','grandchild','2024-02-15 11:59:00','spawn','finished'),('e3','codex','grandchild','leaf','2024-02-15 11:59:00','spawn','finished'),('e4','codex','leaf','parent','2024-02-15 11:59:00','spawn','finished');").unwrap();
        let p = query_live_payload(&c, "v_authority_blast_radius", Some("today"), None).unwrap();
        let parent = p["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["origin_session"] == "parent")
            .unwrap();
        assert_eq!(parent["downstream_sessions"], 3);
        assert_eq!(parent["max_hop"], 3);
        assert_eq!(parent["status"], "HIGH_EXPOSURE");
        assert_eq!(
            parent["non_claim"],
            "Observed dependency exposure only — not a causal fault claim"
        );
    }

    #[test]
    fn rollups_preserve_token_totals_and_average_rate_metrics() {
        let rows = vec![
            json!({"harness":"codex","rig":"one","date_bucket":"2024-02-15","status":"STABLE","total_tokens":100,"l1_distance":0.1}),
            json!({"harness":"codex","rig":"two","date_bucket":"2024-02-16","status":"MINOR_SHIFT","total_tokens":300,"l1_distance":0.3}),
        ];
        let p = row_groupings(&rows, "v_schema_drift");
        assert_eq!(p["by_harness"][0]["total_tokens"], 400);
        assert_eq!(p["by_harness"][0]["count"], 2);
        assert_eq!(p["by_harness"][0]["primary_metric_value"], 0.2);
        assert_eq!(p["by_date"][0]["share_pct"], 50.0);
        assert_eq!(p["by_rig"].as_array().unwrap().len(), 2);
        assert_eq!(p["by_status"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn live_window_filters_all_layers_and_preserves_latest_event() {
        let c = fixture();
        let p = query_live_payload(&c, "hub", Some("1h"), None).unwrap();
        assert_eq!(p["active_window"]["from"], "2024-02-15T11:00:00Z");
        assert_eq!(p["active_window"]["to"], "2024-02-15T12:00:00Z");
        assert_eq!(p["kpis"]["total_tokens"], 465);
        let b = p["time_buckets"].as_array().unwrap();
        assert_eq!(b.len(), 32);
        for (layer, expected) in [("model", 465), ("harness", 1), ("coord", 1), ("env", 2)] {
            assert_eq!(
                b.iter().map(|x| x[layer].as_i64().unwrap()).sum::<i64>(),
                expected
            );
            assert!(!p["xray_lanes"][layer].as_array().unwrap().is_empty());
        }
        assert!(b.iter().any(|b| b["observed"] == false));
        assert!(p["anomalies"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["kind"] == "security"));
    }

    #[test]
    fn direct_usage_and_graph_fallbacks_are_view_complete() {
        for (view, key) in [
            ("v_seat_idle_ranking", "seats"),
            ("v_handoff_rework_analysis", "pairs"),
            ("v_mission_token_burn", "missions"),
            ("v_executive_governance", "records"),
        ] {
            let c = fixture();
            let p = query_live_payload(&c, view, Some("today"), None).unwrap();
            assert!(!p[key].as_array().unwrap().is_empty(), "{view}: {p}");
            if key == "missions" {
                assert_eq!(p["summary"]["total_tokens"], 465);
                assert_eq!(p["rows"], p["missions"]);
                assert_eq!(p["missions"][0]["title"], "Direct Session · codex");
            }
        }
    }

    #[test]
    fn explicit_and_previous_month_windows_exclude_upper_boundary() {
        for window in ["last_month", "2024-01-01T00:00:00Z..2024-02-01T00:00:00Z"] {
            let c = fixture();
            let p = query_live_payload(&c, "hub", Some(window), None).unwrap();
            assert_eq!(p["kpis"]["total_tokens"], 1350);
            assert_eq!(p["active_window"]["to"], "2024-02-01T00:00:00Z");
        }
        for window in ["bad", "2024-02-02T00:00:00Z..2024-02-01T00:00:00Z", "x..y"] {
            assert!(query_live_payload(&fixture(), "hub", Some(window), None).is_err());
        }
    }

    #[test]
    fn empty_database_has_bounded_empty_telemetry() {
        let c = duckdb::Connection::open_in_memory().unwrap();
        let p = query_live_payload(&c, "hub", None, None).unwrap();
        assert_eq!(
            p["recorded_window"],
            json!({"from":null,"to":null,"from_date":null,"to_date":null,"display":null})
        );
        assert_eq!(p["time_buckets"].as_array().unwrap().len(), 32);
        assert!(p["time_buckets"]
            .as_array()
            .unwrap()
            .iter()
            .all(|b| b["observed"] == false));
    }

    #[test]
    fn partial_mission_snapshot_keeps_attribution_and_all_direct_usage() {
        let c = fixture();
        c.execute_batch("CREATE VIEW v_mission_token_burn AS SELECT 'builder' seat_id, '[\"mission-one\"]'::JSON missions, ['a'] source_keys, 100::BIGINT input_uncached,20::BIGINT output,30::BIGINT cache_read,5::BIGINT cache_write;").unwrap();
        let p = query_live_payload(&c, "v_mission_token_burn", Some("today"), None).unwrap();
        let missions = p["missions"].as_array().unwrap();
        assert_eq!(missions.len(), 2);
        assert_eq!(
            missions
                .iter()
                .map(|r| r["total_tokens"].as_i64().unwrap())
                .sum::<i64>(),
            465
        );
        assert_eq!(
            missions
                .iter()
                .find(|r| r["mission"] == "mission-one")
                .unwrap()["total_tokens"],
            155
        );
        assert_eq!(p["summary"]["fresh_input_tokens"], 300);
    }

    #[test]
    fn prefixed_facts_and_session_edges_work_without_other_telemetry() {
        let c = duckdb::Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE _dendro_session_edges (source_key VARCHAR,harness VARCHAR,parent_session VARCHAR,child_session VARCHAR,observed_at TIMESTAMP,relation VARCHAR,status VARCHAR);
            INSERT INTO _dendro_session_edges VALUES ('e','codex','parent','child','2025-01-01 12:00:00','spawn','finished');").unwrap();
        let p = query_live_payload(&c, "v_handoff_rework_analysis", None, None).unwrap();
        assert_eq!(p["pairs"][0]["from_seat"], "parent");
        assert_eq!(p["pairs"][0]["to_seat"], "child");
        assert_eq!(p["summary"]["total_handoffs"], 1);
        assert_eq!(p["recorded_window"]["to"], "2025-01-01T12:00:00Z");
    }

    #[test]
    fn queue_transitions_count_items_once_and_measure_wait_and_execution() {
        let c = fixture();
        c.execute_batch("INSERT INTO queue_facts VALUES ('q0','item','2024-02-15 11:50:00','pending','planner','builder',false),('q2','item','2024-02-15 11:58:00','closed','planner','builder',false);").unwrap();
        let p = query_live_payload(&c, "v_handoff_rework_analysis", Some("today"), None).unwrap();
        assert_eq!(p["summary"]["total_handoffs"], 1);
        assert_eq!(p["pairs"][0]["median_wait_s"], 300.0);
        assert_eq!(p["pairs"][0]["median_exec_s"], 180.0);
    }

    #[test]
    fn semantic_anomalies_use_in_window_source_timestamps() {
        let c = fixture();
        c.execute_batch("CREATE VIEW v_trajectory_vortex AS SELECT 'direct' session_key,'true' tripped,['a'] source_keys;
            CREATE VIEW v_schema_drift AS SELECT 'codex' harness,0.5 l1_distance,1 keys_added,0 keys_removed,0 keys_retyped,['old'] source_keys;").unwrap();
        let p = query_live_payload(&c, "hub", Some("today"), None).unwrap();
        let anomalies = p["anomalies"].as_array().unwrap();
        assert!(anomalies
            .iter()
            .any(|a| a["kind"] == "vortex" && a["t"] == "2024-02-15T11:30:00Z"));
        assert!(!anomalies.iter().any(|a| a["kind"] == "drift"));
    }

    #[test]
    fn governance_unpacks_canonical_json_metrics_for_scorecard() {
        let c = fixture();
        c.execute_batch("CREATE VIEW v_executive_governance AS SELECT 'fleet' rig,'governance-v1' definition_version,
            '{\"m1_yield\":42,\"four_eyes_independence\":{\"same_family_chains\":1,\"checks\":[{\"status\":\"same_family\"}]},\"watchdog_recovery\":{\"recovery_closed_items\":2},\"handoff_friction\":{\"vortex_loops\":[{}]},\"context_compactions\":{\"events\":[{},{}]}}'::JSON row_json;").unwrap();
        let p = query_live_payload(&c, "v_executive_governance", None, None).unwrap();
        assert_eq!(p["summary"]["m1_yield"], 42);
        assert_eq!(p["summary"]["four_eyes_status"], "Violation");
        assert_eq!(p["summary"]["vortex_loops"], 1);
        assert_eq!(p["records"][0]["watchdog_recovery"]["total_resumes"], 2);
        assert_eq!(p["records"][0]["context_compactions"], 2);
    }

    #[test]
    fn zero_token_calls_are_observed_and_still_appear_in_xray() {
        let c = fixture();
        c.execute_batch("UPDATE usage_facts SET input_uncached=0,output_tokens=0,cache_read=0,cache_write=0 WHERE source_key='a'").unwrap();
        let p = query_live_payload(&c, "hub", Some("1h"), None).unwrap();
        assert_eq!(p["time_buckets"][16]["model"], 0);
        assert_eq!(p["time_buckets"][16]["observed"], true);
        assert_eq!(p["time_buckets"][0]["observed"], false);
        assert_eq!(p["xray_lanes"]["model"][1]["value"], 0.0);
    }

    #[test]
    fn relative_presets_anchor_to_the_recorded_utc_clock() {
        for (preset, from) in [
            ("1h", "2024-03-15T11:00:00Z"),
            ("24h", "2024-03-14T12:00:00Z"),
            ("today", "2024-03-15T00:00:00Z"),
            ("7d", "2024-03-08T00:00:00Z"),
            ("30d", "2024-02-14T00:00:00Z"),
            ("this_month", "2024-03-01T00:00:00Z"),
            ("last_month", "2024-02-01T00:00:00Z"),
            ("all", "2023-12-01T00:00:00Z"),
        ] {
            let w = LiveWindow::resolve(
                Some(preset),
                Some("2023-12-01T00:00:00Z"),
                Some("2024-03-15T12:00:00Z"),
            )
            .unwrap();
            assert_eq!(iso(w.from), from);
            assert_eq!(
                iso(w.to),
                if preset == "last_month" {
                    "2024-03-01T00:00:00Z"
                } else if matches!(preset, "7d" | "30d") {
                    "2024-03-16T00:00:00Z"
                } else {
                    "2024-03-15T12:00:00Z"
                }
            );
        }
    }
}
