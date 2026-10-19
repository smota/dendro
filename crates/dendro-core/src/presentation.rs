//! Presentation of canonical values. No alternative metric definitions or raw HTML metadata.
use anyhow::{bail, Result};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

fn rows(v: &Value) -> impl Iterator<Item = &Value> {
    v.as_array().into_iter().flatten()
}
fn number(v: &Value) -> String {
    if let Some(n) = v.as_u64() {
        return n.to_string();
    }
    if let Some(n) = v.as_i64() {
        return n.to_string();
    }
    match v.as_f64().filter(|n| n.is_finite()) {
        Some(n) if n.fract() == 0.0 => format!("{n:.0}"),
        Some(n) => format!("{n:.3}"),
        None => "Unavailable".into(),
    }
}
fn label(v: &Value) -> String {
    crate::redact(v.as_str().unwrap_or("Unknown"))
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .replace('|', "/")
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
fn percent(v: &Value) -> String {
    v.as_f64()
        .filter(|n| n.is_finite())
        .map(|n| format!("{:.1}%", n * 100.0))
        .unwrap_or_else(|| "Unavailable".into())
}
fn table(headers: &[&str], values: &[Vec<String>], markdown: bool) -> String {
    if markdown {
        let mut s = format!(
            "| {} |\n| {} |\n",
            headers.join(" | "),
            headers
                .iter()
                .map(|_| "---")
                .collect::<Vec<_>>()
                .join(" | ")
        );
        for row in values {
            s.push_str(&format!("| {} |\n", row.join(" | ")));
        }
        return s;
    }
    let widths: Vec<_> = headers
        .iter()
        .enumerate()
        .map(|(i, h)| {
            values
                .iter()
                .filter_map(|r| r.get(i))
                .map(|v| v.chars().count())
                .chain([h.len()])
                .max()
                .unwrap_or(1)
                .min(64)
        })
        .collect();
    let line = |row: &[String]| {
        row.iter()
            .enumerate()
            .map(|(i, v)| {
                let text: String = v.chars().take(64).collect();
                format!("{text:width$}", width = widths[i])
            })
            .collect::<Vec<_>>()
            .join("  ")
    };
    let mut s = line(&headers.iter().map(|h| h.to_string()).collect::<Vec<_>>()) + "\n";
    for row in values {
        s.push_str(&line(row));
        s.push('\n');
    }
    s
}
/// Render only report fields whose definitions belong to the canonical engine.
pub fn human(report: &Value, markdown: bool) -> String {
    let p = &report["payload"];
    let title = format!(
        "Dendro {} · {} to {}\n",
        label(&report["metric"]),
        label(&p["from"]),
        label(&p["to"])
    );
    let mut out = if markdown {
        format!("## {title}\n")
    } else {
        title
    };
    if matches!(
        report["metric"].as_str(),
        Some("advisory" | "anomalies" | "spotlights")
    ) {
        let advisory = report["metric"] == "advisory";
        let value = &p[if advisory {
            "advisory_idle_claimed"
        } else {
            "anomaly_spotlights"
        }];
        let findings: Vec<_> = rows(&value["findings"])
            .map(|f| {
                vec![
                    label(&f[if advisory { "classification" } else { "metric" }]),
                    label(if advisory {
                        &f["seat"]
                    } else {
                        &f["affected"]["seat"]
                    }),
                    number(&f[if advisory { "duration_secs" } else { "value" }]),
                    number(
                        &f[if advisory {
                            "threshold_secs"
                        } else {
                            "threshold"
                        }],
                    ),
                ]
            })
            .collect();
        out.push_str(&table(
            &["Finding", "Seat", "Observed", "Threshold"],
            &findings,
            markdown,
        ));
        if findings.is_empty() {
            out.push_str("No findings in evaluated evidence; inspect coverage before treating absence as a negative.\n");
        }
        out.push_str(&format!("Coverage: {}\n", label(&value["coverage"])));
    }
    let summary = vec![
        vec![
            "M1 tokens / nonreturned delivery".into(),
            number(&p["m1_yield"]),
        ],
        vec!["M2 returned handoffs".into(), percent(&p["m2_return_rate"])],
        vec![
            "M3 rework chains".into(),
            percent(&p["m3_rework_proportion"]),
        ],
        vec!["Recorded tokens".into(), number(&p["tokens"])],
        vec![
            "Effective parallelism".into(),
            number(&p["fleet_concurrency"]["effective_parallelism"]),
        ],
    ];
    out.push_str(&table(&["Metric", "Observed value"], &summary, markdown));
    out.push_str("\nM4: ");
    out.push_str(&label(&p["m4_impact"]));
    out.push('\n');
    let seats: Vec<_> = rows(&p["m1_by_seat"])
        .map(|r| {
            vec![
                label(&r["seat"]),
                number(&r["closed_items"]),
                number(&r["tokens"]),
                number(&r["tokens_per_closed_item"]),
            ]
        })
        .collect();
    if !seats.is_empty() {
        out.push('\n');
        out.push_str(&table(
            &["Seat", "Closed items", "Tokens", "Tokens / closed item"],
            &seats,
            markdown,
        ));
    }
    let pairs: Vec<_> = rows(&p["pair_medians"])
        .map(|r| {
            vec![
                label(&r["from"]),
                label(&r["to"]),
                number(&r["wait_median"]),
                number(&r["exec_median"]),
                number(&r["wait_samples"]),
                number(&r["exec_samples"]),
            ]
        })
        .collect();
    if !pairs.is_empty() {
        out.push('\n');
        out.push_str(&table(
            &[
                "From",
                "To",
                "Pickup median s",
                "Execution median s",
                "Pickup samples",
                "Execution samples",
            ],
            &pairs,
            markdown,
        ));
    }
    let activity: Vec<_> = rows(&p["activity"])
        .map(|r| {
            vec![
                label(&r["seat"]),
                number(&r["active_secs"]),
                number(&r["idle_with_work_secs"]),
                number(&r["idle_without_work_secs"]),
                number(&r["waiting_human_secs"]),
                number(&r["unknown_secs"]),
            ]
        })
        .collect();
    if !activity.is_empty() {
        out.push('\n');
        out.push_str(&table(
            &[
                "Seat",
                "Running s",
                "Idle demand s",
                "Idle no demand s",
                "Needs input s",
                "Unknown s",
            ],
            &activity,
            markdown,
        ));
    }
    out.push_str("\nUnavailable means insufficient observed data. No trend or causal effect is inferred.\n[1] Handoff friction · [2] Activity evidence · [3] Token attribution\n");
    out
}
fn blocks(value: f64, total: f64, width: usize, glyph: char) -> String {
    if !value.is_finite() || !total.is_finite() || total <= 0.0 || value <= 0.0 {
        return String::new();
    }
    std::iter::repeat_n(
        glyph,
        ((value / total * width as f64).round() as usize).min(width),
    )
    .collect()
}
/// Plain terminal block graphs remain readable with ANSI colors disabled.
pub fn viz(report: &Value) -> String {
    let p = &report["payload"];
    let mut out = String::from("Concurrency distribution (aggregate; not a temporal ribbon)\n");
    let total: f64 = rows(&p["fleet_concurrency"]["histogram"])
        .filter_map(|r| r["secs"].as_f64())
        .sum();
    for r in rows(&p["fleet_concurrency"]["histogram"]) {
        out.push_str(&format!(
            "k={} {:<40} {} s\n",
            number(&r["active_seats"]),
            blocks(r["secs"].as_f64().unwrap_or(0.0), total, 40, '█'),
            number(&r["secs"])
        ));
    }
    if total <= 0.0 {
        out.push_str("Unavailable: no observed concurrency duration.\n");
    }
    out.push_str(
        "\nPer-seat tool calls: █ engineering / ░ ceremony (counts, not execution duration)\n",
    );
    for r in rows(&p["activity"]) {
        let eng = r["engineering_calls"].as_f64();
        let ceremony = r["ceremony_calls"].as_f64();
        out.push_str(&format!(
            "{} {} {}\n",
            label(&r["seat"]),
            match eng.zip(ceremony) {
                Some((a, b)) if a + b > 0.0 =>
                    blocks(a, a + b, 40, '█') + &blocks(b, a + b, 40, '░'),
                _ => "Unavailable".into(),
            },
            format_args!(
                "engineering={} ceremony={}",
                number(&r["engineering_calls"]),
                number(&r["ceremony_calls"])
            )
        ));
    }
    out
}
const BANDS: [(&str, &str, char); 5] = [
    ("active_secs", "RUNNING", '█'),
    ("idle_with_work_secs", "IDLE-WITH-WORK", '▓'),
    ("idle_without_work_secs", "IDLE-WITHOUT-WORK", '░'),
    ("waiting_human_secs", "NEEDS-INPUT", '▒'),
    ("unknown_secs", "UNKNOWN", '?'),
];
/// Render cached facts for keyboard navigation; no database access occurs here.
pub fn explorer(report: &Value, panel: usize, selected: usize) -> String {
    let mut out=format!("Dendro local explorer · {} to {}\nTab/←/→ panels · ↑/↓ seats · Enter detail · 1/2/3 trace · q quit\n\n",label(&report["payload"]["from"]),label(&report["payload"]["to"]));
    match panel % 3 {
        0 => {
            out.push_str("Activity bands (aggregate duration, not inferred tool execution)\n");
            for (i, r) in rows(&report["payload"]["activity"]).enumerate() {
                let total = r["observed_secs"].as_f64().unwrap_or(0.0);
                let ribbon = BANDS
                    .iter()
                    .map(|(k, _, g)| blocks(r[*k].as_f64().unwrap_or(0.0), total, 40, *g))
                    .collect::<String>();
                out.push_str(&format!(
                    "{} {} {}\n",
                    if i == selected { ">" } else { " " },
                    label(&r["seat"]),
                    if ribbon.is_empty() {
                        "UNKNOWN"
                    } else {
                        &ribbon
                    }
                ));
            }
            out.push_str(
                "█ RUNNING · ▓ IDLE-WITH-WORK · ░ IDLE-WITHOUT-WORK · ▒ NEEDS-INPUT · ? UNKNOWN\n",
            );
            if let Some(r) = rows(&report["payload"]["activity"]).nth(selected) {
                for (k, name, _) in BANDS {
                    out.push_str(&format!("{name}: {} s\n", number(&r[k])));
                }
            }
        }
        1 => out.push_str(&viz(report)),
        _ => out.push_str(&human(report, false)),
    }
    out
}
/// Compute the UI's explicit trailing window; reporting itself always uses fixed strings.
pub fn trailing_window(days: i64) -> Result<(String, String)> {
    if ![1, 7].contains(&days) {
        bail!("UI window must be 24h or 7d");
    }
    let to = chrono::Utc::now();
    Ok((
        (to - chrono::Duration::days(days)).to_rfc3339(),
        to.to_rfc3339(),
    ))
}

/// Inspect an analytics database read-only and summarize recorded bounds, rigs, seats, and cache state.
pub fn database_status(db: &std::path::Path, rig: Option<&str>) -> Result<Value> {
    let db_display = crate::redact(&db.display().to_string());
    if !db.exists() {
        return Ok(json!({
            "exists": false,
            "db": db_display,
            "status": "not_initialized",
            "remediation": format!("dendro dashboard --db {} --auto-ingest", db_display),
            "next_commands": [
                format!("dendro run --db {}", db_display),
                format!("dendro dashboard --db {} --auto-ingest", db_display)
            ]
        }));
    }
    let store = crate::AnalyticsStore::open_read_only(db)?;
    let conn = store.connection();
    let (min_ts, max_ts): (Option<String>, Option<String>) = conn.query_row(
        "SELECT
            strftime(MIN(ts), '%Y-%m-%dT%H:%M:%SZ'),
            strftime(MAX(ts), '%Y-%m-%dT%H:%M:%SZ')
         FROM (
            SELECT ts FROM usage_facts WHERE ts IS NOT NULL
            UNION ALL SELECT ts FROM activity_facts
            UNION ALL SELECT ts FROM queue_facts
            UNION ALL SELECT ts FROM tool_facts WHERE ts IS NOT NULL
            UNION ALL SELECT created_at AS ts FROM seat_bounds WHERE created_at IS NOT NULL
            UNION ALL SELECT stopped_at AS ts FROM seat_bounds WHERE stopped_at IS NOT NULL
         )",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let usage_rows: i64 = conn.query_row("SELECT COUNT(*) FROM usage_facts", [], |r| r.get(0))?;
    let activity_rows: i64 =
        conn.query_row("SELECT COUNT(*) FROM activity_facts", [], |r| r.get(0))?;
    let queue_rows: i64 = conn.query_row("SELECT COUNT(*) FROM queue_facts", [], |r| r.get(0))?;
    let mut rig_stmt =
        conn.prepare("SELECT DISTINCT rig FROM identity_facts WHERE rig <> '' ORDER BY rig")?;
    let rigs: Vec<String> = rig_stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<duckdb::Result<Vec<_>>>()?;
    let mut seat_stmt = conn.prepare(
        "SELECT DISTINCT seat FROM (
            SELECT seat FROM identity_facts WHERE seat <> ''
            UNION SELECT seat FROM activity_facts WHERE seat <> ''
            UNION SELECT seat FROM seat_bounds WHERE seat <> ''
            UNION SELECT source_seat AS seat FROM queue_facts WHERE source_seat <> ''
            UNION SELECT dest_seat AS seat FROM queue_facts WHERE dest_seat <> ''
         ) ORDER BY seat",
    )?;
    let seats: Vec<String> = seat_stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<duckdb::Result<Vec<_>>>()?;
    drop(seat_stmt);
    drop(rig_stmt);
    drop(store);
    let semantic = crate::semantic::cached_scope(db).map(|(r, f, t, fresh)| {
        json!({
            "rig": r,
            "from": f,
            "to": t,
            "fresh": fresh
        })
    });
    let semantic_fresh = semantic
        .as_ref()
        .and_then(|s| s["fresh"].as_bool())
        .unwrap_or(false);
    Ok(json!({
        "exists": true,
        "db": db_display,
        "status": if min_ts.is_some() { "ready" } else { "empty" },
        "selected_rig": rig,
        "min_timestamp": min_ts,
        "max_timestamp": max_ts,
        "usage_rows": usage_rows,
        "activity_rows": activity_rows,
        "queue_rows": queue_rows,
        "rigs": rigs,
        "seats": seats,
        "semantic_fresh": semantic_fresh,
        "semantic_snapshot": semantic,
        "next_commands": [
            format!("dendro dashboard --db {}", db_display),
            format!("dendro ask --db {} \"Which seat had the highest idle time?\"", db_display),
            format!("dendro dashboard --db {} --html dendro-dashboard.html", db_display)
        ]
    }))
}

/// Resolve an explicit (`--from`/`--to`) or relative (`--window 1h|24h|7d|30d|all`) window,
/// anchoring relative windows to the database's recorded timestamp bounds when available.
pub fn resolve_window(
    db: &std::path::Path,
    rig: Option<&str>,
    window: Option<&str>,
    from: Option<&str>,
    to: Option<&str>,
) -> Result<(String, String, String)> {
    if let (Some(f), Some(t)) = (from, to) {
        return Ok((f.to_owned(), t.to_owned(), "explicit".into()));
    }
    let dur = match window.unwrap_or("all") {
        "1h" => Some(chrono::Duration::hours(1)),
        "24h" => Some(chrono::Duration::days(1)),
        "7d" => Some(chrono::Duration::days(7)),
        "30d" => Some(chrono::Duration::days(30)),
        "all" => None,
        _ => bail!("window must be 1h, 24h, 7d, 30d, or all"),
    };
    if db.exists() {
        if let Ok(status) = database_status(db, rig) {
            let min_dt = status["min_timestamp"]
                .as_str()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|d| d.with_timezone(&chrono::Utc));
            let max_dt = status["max_timestamp"]
                .as_str()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|d| d.with_timezone(&chrono::Utc));
            if let (Some(min_d), Some(max_d)) = (min_dt, max_dt) {
                let end_d = if max_d > min_d {
                    max_d
                } else {
                    min_d + chrono::Duration::seconds(1)
                };
                let (start_d, mode) = match dur {
                    Some(d) => ((end_d - d).max(min_d), "anchored_to_recorded_data"),
                    None => (min_d, "recorded_bounds"),
                };
                return Ok((
                    start_d.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                    end_d.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                    mode.into(),
                ));
            }
        }
    }
    let end_d = chrono::Utc::now();
    let start_d = end_d - dur.unwrap_or_else(|| chrono::Duration::days(7));
    Ok((
        start_d.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        end_d.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        "trailing_utc".into(),
    ))
}

/// Human-readable summary of `database_status`.
pub fn human_status(status: &Value) -> String {
    let mut out = format!(
        "Dendro Status · db={}\n",
        status["db"].as_str().unwrap_or("dendro.duckdb")
    );
    if !status["exists"].as_bool().unwrap_or(false) {
        out.push_str("State: NOT INITIALIZED (analytics database does not exist yet)\n");
        if let Some(sources) = status["discovered_sources"].as_array() {
            out.push_str("Discovered local harness sources:\n");
            for s in sources {
                out.push_str(&format!(
                    "  - {}: {} ({})\n",
                    s["name"].as_str().unwrap_or("?"),
                    s["path"].as_str().unwrap_or("?"),
                    if s["exists"].as_bool().unwrap_or(false) {
                        "present"
                    } else {
                        "missing"
                    }
                ));
            }
        }
        out.push_str("\nQuickstart (1 command):\n");
        for cmd in rows(&status["next_commands"]) {
            if let Some(c) = cmd.as_str() {
                out.push_str(&format!("  $ {c}\n"));
            }
        }
        return out;
    }
    out.push_str(&format!(
        "State: {} · usage={} · activity={} · queue={}\n",
        status["status"]
            .as_str()
            .unwrap_or("unknown")
            .to_uppercase(),
        number(&status["usage_rows"]),
        number(&status["activity_rows"]),
        number(&status["queue_rows"])
    ));
    out.push_str(&format!(
        "Recorded window: {} to {}\n",
        status["min_timestamp"].as_str().unwrap_or("Unavailable"),
        status["max_timestamp"].as_str().unwrap_or("Unavailable")
    ));
    let rigs: Vec<&str> = rows(&status["rigs"]).filter_map(|v| v.as_str()).collect();
    let seats: Vec<&str> = rows(&status["seats"]).filter_map(|v| v.as_str()).collect();
    out.push_str(&format!(
        "Rigs ({}): {}\n",
        rigs.len(),
        if rigs.is_empty() {
            "none".into()
        } else {
            rigs.join(", ")
        }
    ));
    out.push_str(&format!(
        "Seats ({}): {}\n",
        seats.len(),
        if seats.is_empty() {
            "none".into()
        } else {
            seats.join(", ")
        }
    ));
    out.push_str(&format!(
        "Semantic (Nao) cache: {}\n",
        if status["semantic_fresh"].as_bool().unwrap_or(false) {
            "FRESH"
        } else {
            "STALE / NOT MATERIALIZED (auto-refreshed by 'dendro ask')"
        }
    ));
    out.push_str("\nNext commands:\n");
    for cmd in rows(&status["next_commands"]) {
        if let Some(c) = cmd.as_str() {
            out.push_str(&format!("  $ {c}\n"));
        }
    }
    out
}

fn alias(value: &Value, aliases: &mut BTreeMap<String, String>) -> Value {
    let Some(s) = value.as_str().filter(|s| !s.is_empty()) else {
        return Value::Null;
    };
    let next = format!("Seat {}", aliases.len() + 1);
    json!(aliases.entry(s.to_owned()).or_insert(next))
}
fn nums(raw: &Value, keys: &[&str]) -> Map<String, Value> {
    keys.iter()
        .map(|k| {
            (
                (*k).into(),
                raw[*k]
                    .as_f64()
                    .filter(|n| n.is_finite() && *n >= 0.0)
                    .map_or(Value::Null, |_| raw[*k].clone()),
            )
        })
        .collect()
}
fn project_rows(
    raw: &Value,
    numeric: &[&str],
    labels: &[&str],
    aliases: &mut BTreeMap<String, String>,
) -> Value {
    Value::Array(
        rows(raw)
            .take(50_000)
            .map(|r| {
                let mut m = nums(r, numeric);
                for k in labels {
                    m.insert((*k).into(), alias(&r[*k], aliases));
                }
                Value::Object(m)
            })
            .collect(),
    )
}
fn time(v: &Value) -> Value {
    v.as_str()
        .and_then(crate::parse_utc_time)
        .map(|t| json!(t.to_rfc3339()))
        .unwrap_or(Value::Null)
}
/// Sanitizes the serialized payload itself. Native labels, locators and arbitrary strings never enter HTML.
pub fn public_projection(report: &Value) -> Value {
    let p = &report["payload"];
    let mut aliases = BTreeMap::new();
    let mut out = nums(p, &["tokens", "m1_yield", "m3_rework_chains"]);
    out.insert("product".into(), json!("AI Fleet Intelligence"));
    out.insert("from".into(), time(&p["from"]));
    out.insert("to".into(), time(&p["to"]));
    out.insert(
        "m1_by_seat".into(),
        project_rows(
            &p["m1_by_seat"],
            &[
                "tokens",
                "closed_items",
                "nonreturn_deliveries",
                "tokens_per_closed_item",
            ],
            &["seat"],
            &mut aliases,
        ),
    );
    let mut activity = project_rows(
        &p["activity"],
        &[
            "observed_secs",
            "active_secs",
            "idle_with_work_secs",
            "idle_without_work_secs",
            "waiting_human_secs",
            "unknown_secs",
            "unparked_container_secs",
            "leaf_stall_secs",
            "engineering_calls",
            "ceremony_calls",
        ],
        &["seat"],
        &mut aliases,
    );
    for (safe, raw) in activity
        .as_array_mut()
        .into_iter()
        .flatten()
        .zip(rows(&p["activity"]))
    {
        safe["observed_spans"] = Value::Array(
            rows(&raw["observed_spans"])
                .take(50_000)
                .map(|r| json!({"from":time(&r["from"]),"to":time(&r["to"])}))
                .collect(),
        );
    }
    out.insert("activity".into(), activity);
    out.insert(
        "pair_medians".into(),
        project_rows(
            &p["pair_medians"],
            &["wait_median", "wait_samples", "exec_median", "exec_samples"],
            &["from", "to"],
            &mut aliases,
        ),
    );
    out.insert(
        "m2_pairs".into(),
        project_rows(
            &p["m2_pairs"],
            &["handoffs", "return_rate"],
            &["from", "to"],
            &mut aliases,
        ),
    );
    let mut edges = project_rows(&p["handoff_members"], &[], &["from", "to"], &mut aliases);
    for (safe, raw) in edges
        .as_array_mut()
        .into_iter()
        .flatten()
        .zip(rows(&p["handoff_members"]))
    {
        safe["is_return"] = raw["is_return"].as_bool().map_or(Value::Null, Value::Bool);
    }
    out.insert("handoff_members".into(), edges);
    let mut events = project_rows(&p["activity_events"], &[], &["seat"], &mut aliases);
    for (safe, raw) in events
        .as_array_mut()
        .into_iter()
        .flatten()
        .zip(rows(&p["activity_events"]))
    {
        safe["ts"] = time(&raw["ts"]);
        safe["state"] = match raw["state"].as_str() {
            Some(s @ ("running" | "idle" | "needs_input")) => json!(s),
            _ => json!("unknown"),
        };
    }
    out.insert("activity_events".into(), events);
    let mut compactions = project_rows(
        &p["context_compactions"]["events"],
        &["pre_drop_context_tokens", "post_drop_context_tokens"],
        &["seat"],
        &mut aliases,
    );
    for (safe, raw) in compactions
        .as_array_mut()
        .into_iter()
        .flatten()
        .zip(rows(&p["context_compactions"]["events"]))
    {
        safe["ts"] = time(&raw["ts"]);
    }
    out.insert("context_compactions".into(), json!({"events":compactions}));
    let mut intervals = project_rows(
        &p["advisory_idle_claimed"]["intervals"],
        &[],
        &["seat"],
        &mut aliases,
    );
    for (safe, raw) in intervals
        .as_array_mut()
        .into_iter()
        .flatten()
        .zip(rows(&p["advisory_idle_claimed"]["intervals"]))
    {
        safe["from"] = time(&raw["from"]);
        safe["to"] = time(&raw["to"]);
        safe["classification"] = match raw["classification"].as_str() {
            Some(s @ ("unparked_container_hold" | "true_leaf_stall")) => json!(s),
            _ => json!("unknown"),
        };
    }
    out.insert(
        "advisory_idle_claimed".into(),
        json!({"intervals":intervals}),
    );

    out.insert(
        "token_categories".into(),
        Value::Object(nums(
            &p["token_categories"],
            &["input_uncached", "cache_read", "cache_write", "output"],
        )),
    );
    let mut concurrency = nums(&p["fleet_concurrency"], &["effective_parallelism"]);
    concurrency.insert(
        "histogram".into(),
        project_rows(
            &p["fleet_concurrency"]["histogram"],
            &["active_seats", "secs"],
            &[],
            &mut aliases,
        ),
    );
    out.insert("fleet_concurrency".into(), Value::Object(concurrency));
    out.insert(
        "task_intake_overhead".into(),
        Value::Object(nums(
            &p["task_intake_overhead"],
            &[
                "assigned_mission_tokens",
                "task_intake_overhead",
                "ambiguous_tokens",
                "unlinked_session",
            ],
        )),
    );
    let mut boundaries = project_rows(
        &p["ring_boundaries"]["edges"],
        &[],
        &["from", "to"],
        &mut aliases,
    );
    for (safe, raw) in boundaries
        .as_array_mut()
        .into_iter()
        .flatten()
        .zip(rows(&p["ring_boundaries"]["edges"]))
    {
        safe["boundary"] = match raw["boundary"].as_str() {
            Some(s @ ("in_ring" | "cross_rig_seam" | "supervisory_plane")) => json!(s),
            _ => json!("unknown"),
        };
    }
    out.insert("ring_boundaries".into(), json!({"edges":boundaries}));
    json!({"metric":"decisions","payload":out})
}
pub fn offline_html(report: &Value) -> Result<String> {
    fn bounded(value: &Value) -> bool {
        match value {
            Value::Array(a) => a.len() <= 50_000 && a.iter().all(bounded),
            Value::Object(m) => m.values().all(bounded),
            _ => true,
        }
    }
    if !bounded(&report["payload"]) {
        bail!("offline export exceeds 50000 rows per array; narrow the report window");
    }
    let safe = serde_json::to_string(&public_projection(report))?
        .replace('&', "\\u0026")
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029");
    if safe.len() > 10 * 1024 * 1024 {
        bail!("offline export exceeds 10 MiB; narrow the report window");
    }
    let marker = "<script id=\"dendro-data\" type=\"application/json\">null</script>";
    let template = include_str!("../../../site/viewer.html");
    if template.matches(marker).count() != 1 {
        bail!("offline viewer embedding contract changed");
    }
    Ok(template.replacen(
        marker,
        &format!("<script id=\"dendro-data\" type=\"application/json\">{safe}</script>"),
        1,
    ))
}
fn detail_json(value: &Value) -> Result<String> {
    let mut out = String::new();
    for ch in crate::redact(&serde_json::to_string_pretty(value)?).chars() {
        if ch != '\n' && ch.is_control() {
            use std::fmt::Write as _;
            let _ = write!(out, "\\u{:04x}", ch as u32);
        } else {
            out.push(ch);
        }
    }
    Ok(out)
}

#[derive(Default)]
struct UiState {
    panel: usize,
    selected: usize,
    detail: Option<String>,
    escape: u8,
}

fn step_ui_key(state: &mut UiState, byte: u8, report: &Value, traces: &[Value]) -> Result<bool> {
    match byte {
        b'q' | 3 | 4 => return Ok(false),
        27 => state.escape = 1,
        b'[' if state.escape == 1 => state.escape = 2,
        b'C' if state.escape == 2 => {
            state.panel = (state.panel + 1) % 3;
            state.detail = None;
            state.escape = 0;
        }
        b'D' if state.escape == 2 => {
            state.panel = (state.panel + 2) % 3;
            state.detail = None;
            state.escape = 0;
        }
        b'A' if state.escape == 2 => {
            state.selected = state.selected.saturating_sub(1);
            state.detail = None;
            state.escape = 0;
        }
        b'B' if state.escape == 2 => {
            state.selected = (state.selected + 1).min(
                rows(&report["payload"]["activity"])
                    .count()
                    .saturating_sub(1),
            );
            state.detail = None;
            state.escape = 0;
        }
        b'\t' => {
            state.panel = (state.panel + 1) % 3;
            state.detail = None;
            state.escape = 0;
        }
        b'1'..=b'3' => {
            let i = (byte - b'1') as usize;
            state.detail = Some(format!(
                "Forensic trace {} (cached at this observation window)\n{}\nTab or arrows to return; q quit",
                i + 1,
                detail_json(traces.get(i).unwrap_or(&Value::Null))?
            ));
            state.escape = 0;
        }
        b'\r' | b'\n' => {
            state.detail = Some(format!(
                "Seat evidence (cached)\n{}\nTab to return",
                detail_json(
                    rows(&report["payload"]["activity"])
                        .nth(state.selected)
                        .unwrap_or(&Value::Null)
                )?
            ));
            state.escape = 0;
        }
        b'?' | b'h' => {
            state.detail = Some(
                "Dendro Explorer Help\n\
                 - Tab / Left / Right: cycle panels (Activity Ribbon, Concurrency & Tool Calls, Decision Tables)\n\
                 - Up / Down: select seat in Activity Ribbon\n\
                 - Enter: inspect selected seat's canonical evidence\n\
                 - 1 / 2 / 3: inspect forensic trace (1=handoff_friction, 2=activity, 3=tokens)\n\
                 - q / Ctrl-C: quit\n\
                 Tab or arrows to return"
                    .into(),
            );
            state.escape = 0;
        }
        _ => state.escape = 0,
    }
    Ok(true)
}

/// Terminal mode is restored on normal exit and errors. Ctrl-C is read as a key.
/// The caller runs this blocking input loop outside the async executor.
pub fn terminal_ui(report: &Value, traces: &[Value], once: bool) -> Result<()> {
    use std::io::{IsTerminal, Read, Write};
    use std::process::{Command, Stdio};
    if once {
        print!("{}", explorer(report, 0, 0));
        return Ok(());
    }
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        bail!("interactive UI requires a terminal; use --once for a snapshot");
    }
    struct Restore(String);
    impl Drop for Restore {
        fn drop(&mut self) {
            let _ = Command::new("stty")
                .arg(&self.0)
                .stdin(Stdio::inherit())
                .status();
            print!("\x1b[?1049l");
            let _ = std::io::stdout().flush();
        }
    }
    let mode = Command::new("stty")
        .arg("-g")
        .stdin(Stdio::inherit())
        .output()?;
    if !mode.status.success() {
        bail!("could not read terminal mode");
    }
    let original = String::from_utf8(mode.stdout)?.trim().to_owned();
    let restore = Restore(original);
    if !Command::new("stty")
        .args(["-echo", "-icanon", "-isig", "min", "0", "time", "1"])
        .stdin(Stdio::inherit())
        .status()?
        .success()
    {
        bail!("could not configure terminal input");
    }
    print!("\x1b[?1049h");
    let mut input = std::io::stdin().lock();
    let mut state = UiState::default();
    loop {
        let rendered = explorer(report, state.panel, state.selected);
        let text: &str = state.detail.as_deref().unwrap_or(&rendered);
        println!("\x1b[H\x1b[2J{text}");
        std::io::stdout().flush()?;
        let mut byte = [0];
        loop {
            if crate::RUN_CANCEL.requested() {
                return Ok(());
            }
            if input.read(&mut byte)? > 0 {
                break;
            }
        }
        if !step_ui_key(&mut state, byte[0], report, traces)? {
            break;
        }
    }
    drop(restore);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn export_scrubs_serialized_bytes_and_keeps_numeric_values() {
        let r = json!({"payload":{"product":"sk-private","from":"2026-01-01T00:00:00Z","to":"2026-01-02T00:00:00Z","tokens":42,"m1_by_seat":[{"seat":"</script><script>alert(1)</script>ghp_canary","tokens":42,"source_key":"/private/path"}],"metadata":"secretcanary","activity_events":[{"seat":"</script><script>alert(1)</script>ghp_canary","ts":"2026-01-01T00:00:00Z","state":"running"}]}});
        let h = offline_html(&r).unwrap();
        assert!(!h.contains("ghp_canary"));
        assert!(!h.contains("secretcanary"));
        assert!(!h.contains("/private/path"));
        assert!(!h.contains("alert(1)"));
        let p = public_projection(&r);
        assert_eq!(p["payload"]["tokens"], 42);
        assert_eq!(
            p["payload"]["m1_by_seat"][0]["seat"],
            p["payload"]["activity_events"][0]["seat"]
        );
    }
    #[test]
    fn unknown_zero_and_percentages_remain_distinct() {
        let r = json!({"metric":"decisions","payload":{"m2_return_rate":0.5,"m1_yield":null,"tokens":0,"m1_by_seat":[{"seat":"abc\u{1b}[2J","tokens":0}]}});
        let text = human(&r, true);
        assert!(text.contains("50.0%"));
        assert!(text.contains("Unavailable"));
        assert!(!text.contains('\u{1b}'));
        assert!(text.contains("| Recorded tokens | 0 |"));
    }
    #[test]
    fn human_markdown_keeps_large_integer_precision_and_escapes_native_html() {
        let r = json!({"metric":"decisions","payload":{"tokens":9007199254740993u64,"m1_by_seat":[{"seat":"<img src=x onerror=alert(1)>","tokens":9007199254740993u64}]}});
        let out = human(&r, true);
        assert!(out.contains("9007199254740993"));
        assert!(!out.contains("<img"));
        assert!(out.contains("&lt;img"));
    }
    #[test]
    fn aggregate_graphs_and_unknown_panels_do_not_invent_observations() {
        let r = json!({"payload":{"activity":[{"seat":"a","observed_secs":100,"unknown_secs":100}],"fleet_concurrency":{"histogram":[{"active_seats":0,"secs":100}]}}});
        assert!(viz(&r).contains("not a temporal ribbon"));
        assert!(explorer(&r, 0, 0).contains("????????"));
        assert!(trailing_window(2).is_err());
    }
    #[test]
    fn terminal_escape_resets_on_navigation_and_escapes_c1_controls() {
        let r = json!({"payload":{"activity":[{"seat":"s1\u{009b}2J\u{0085}","observed_secs":10},{"seat":"s2","observed_secs":20}]}});
        let traces = vec![json!({"note":"trace\u{009b}31m\u{007f}"})];
        let mut state = UiState::default();
        for key in *b"\t1\n" {
            assert!(step_ui_key(&mut state, 27, &r, &traces).unwrap());
            assert_eq!(state.escape, 1);
            assert!(step_ui_key(&mut state, key, &r, &traces).unwrap());
            assert_eq!(state.escape, 0);
            let panel_before = state.panel;
            assert!(step_ui_key(&mut state, b'[', &r, &traces).unwrap());
            assert_eq!(state.escape, 0);
            assert!(step_ui_key(&mut state, b'C', &r, &traces).unwrap());
            assert_eq!(state.panel, panel_before);
        }
        step_ui_key(&mut state, b'1', &r, &traces).unwrap();
        let trace_detail = state.detail.as_deref().unwrap();
        assert!(!trace_detail.chars().any(|c| c != '\n' && c.is_control()));
        assert!(trace_detail.contains("\\u009b"));
        assert!(trace_detail.contains("\\u007f"));
        step_ui_key(&mut state, b'\r', &r, &traces).unwrap();
        let seat_detail = state.detail.as_deref().unwrap();
        assert!(!seat_detail.chars().any(|c| c != '\n' && c.is_control()));
        assert!(seat_detail.contains("\\u009b"));
        assert!(seat_detail.contains("\\u0085"));
    }
    #[test]
    fn status_window_resolution_and_help_overlay_are_truthful() {
        let missing = std::env::temp_dir().join("dendro-missing-status-test.duckdb");
        let _ = std::fs::remove_file(&missing);
        let status = database_status(&missing, None).unwrap();
        assert_eq!(status["status"], "not_initialized");
        let rendered = human_status(&status);
        assert!(rendered.contains("State: NOT INITIALIZED"));
        assert!(rendered.contains("dendro dashboard --db"));

        let (f, t, mode) = resolve_window(
            &missing,
            None,
            None,
            Some("2026-10-08T00:00:00Z"),
            Some("2026-10-09T00:00:00Z"),
        )
        .unwrap();
        assert_eq!(f, "2026-10-08T00:00:00Z");
        assert_eq!(t, "2026-10-09T00:00:00Z");
        assert_eq!(mode, "explicit");

        let (_, _, fallback_mode) = resolve_window(&missing, None, Some("7d"), None, None).unwrap();
        assert_eq!(fallback_mode, "trailing_utc");
        assert!(resolve_window(&missing, None, Some("2d"), None, None).is_err());

        let r = json!({"payload":{"activity":[{"seat":"s1","observed_secs":10}]}});
        let mut state = UiState::default();
        assert!(step_ui_key(&mut state, b'?', &r, &[]).unwrap());
        assert!(state
            .detail
            .as_deref()
            .unwrap()
            .contains("Dendro Explorer Help"));
        state.detail = None;
        assert!(step_ui_key(&mut state, b'h', &r, &[]).unwrap());
        assert!(state
            .detail
            .as_deref()
            .unwrap()
            .contains("Dendro Explorer Help"));
    }
}
