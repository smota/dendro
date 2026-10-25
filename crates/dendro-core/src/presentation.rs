//! Presentation of canonical values. No alternative metric definitions or raw HTML metadata.
use anyhow::{bail, Result};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

/// A single logical request; keeps the selected immutable version pinned across all reads.
pub struct ReadRequest {
    logical: std::path::PathBuf,
    database: std::path::PathBuf,
    initialized: bool,
    #[cfg(any(windows, target_os = "linux"))]
    selected: Option<crate::selection_binding::SelectedRead>,
    #[cfg(any(windows, target_os = "linux"))]
    legacy: Option<Box<crate::selection_binding::LegacyRead>>,
}
impl ReadRequest {
    pub fn open(db: &std::path::Path) -> Result<Self> {
        crate::refuse_secret_path(db)?;
        #[cfg(any(windows, target_os = "linux"))]
        {
            use crate::selection_binding::{resolve_logical_db, SelectedDb};
            let resolved =
                resolve_logical_db(db, &crate::platform_v1::StorageRootChoice::LocalAppData)?;
            match resolved {
                SelectedDb::Uninitialized => Ok(Self {
                    logical: db.into(),
                    database: db.into(),
                    initialized: false,
                    selected: None,
                    legacy: None,
                }),
                SelectedDb::Legacy(legacy) => Ok(Self {
                    logical: legacy.logical.clone(),
                    initialized: true,
                    database: legacy.database.clone(),
                    selected: None,
                    legacy: Some(legacy),
                }),
                SelectedDb::Selected(selected) => Ok(Self::from_selected(*selected)),
            }
        }
        #[cfg(not(any(windows, target_os = "linux")))]
        Ok(Self {
            logical: db.into(),
            database: db.into(),
            initialized: db.exists(),
        })
    }
    #[cfg(any(windows, target_os = "linux"))]
    pub fn from_selected(selected: crate::selection_binding::SelectedRead) -> Self {
        Self {
            logical: selected.logical.clone(),
            database: selected.database.clone(),
            initialized: true,
            selected: Some(selected),
            legacy: None,
        }
    }
    pub fn logical(&self) -> &std::path::Path {
        &self.logical
    }
    /// Borrow the physical DB only while this request retains selection custody.
    pub fn database(&self) -> &std::path::Path {
        &self.database
    }
    pub fn initialized(&self) -> bool {
        self.initialized
    }
    /// Cheap pointer check; immutable bundle bytes were verified when this request opened.
    pub fn is_current(&self) -> Result<bool> {
        #[cfg(any(windows, target_os = "linux"))]
        if let Some(selected) = self.selected() {
            return selected.is_current();
        }
        Ok(false)
    }
    #[cfg(any(windows, target_os = "linux"))]
    pub fn selected(&self) -> Option<&crate::selection_binding::SelectedRead> {
        self.selected.as_ref()
    }
    #[cfg(any(windows, target_os = "linux"))]
    pub fn is_legacy(&self) -> bool {
        self.legacy.is_some()
    }

    pub fn open_store(&self) -> Result<crate::AnalyticsStore> {
        #[cfg(any(windows, target_os = "linux"))]
        if let Some(selected) = self.selected() {
            return crate::AnalyticsStore::open_read_only_selected(selected);
        }
        crate::AnalyticsStore::open_read_only(self.database())
    }
    pub fn status(&self, rig: Option<&str>) -> Result<Value> {
        database_status_request(self, rig)
    }
    pub fn window(
        &self,
        rig: Option<&str>,
        window: Option<&str>,
        from: Option<&str>,
        to: Option<&str>,
    ) -> Result<(String, String, String)> {
        resolve_window_request(self, rig, window, from, to)
    }
}

fn rows(v: &Value) -> impl Iterator<Item = &Value> {
    v.as_array().into_iter().flatten()
}
fn cost_text(v: &Value) -> String {
    match v["status"].as_str() {
        Some("complete") => match v["usd"].as_f64().filter(|n| n.is_finite()) {
            Some(amount) => amount.to_string(),
            None => "Unavailable".into(),
        },
        Some("partial") => format!("partial ({} unknown)", number(&v["unknown_rows"])),
        Some("unknown") => format!("unknown ({} unknown)", number(&v["unknown_rows"])),
        _ => "Unavailable".into(),
    }
}
fn category_cell(v: &Value) -> String {
    if v.is_string() {
        label(v)
    } else {
        number(v)
    }
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
            "Input uncached".into(),
            category_cell(&p["token_categories"]["input_uncached"]),
        ],
        vec![
            "Output".into(),
            category_cell(&p["token_categories"]["output"]),
        ],
        vec![
            "Cache read".into(),
            category_cell(&p["token_categories"]["cache_read"]),
        ],
        vec![
            "Cache write".into(),
            category_cell(&p["token_categories"]["cache_write"]),
        ],
        vec![
            "Reasoning".into(),
            category_cell(&p["token_categories"]["reasoning"]),
        ],
        vec!["Observed cost USD".into(), cost_text(&p["observed_cost"])],
        vec!["Estimated cost USD".into(), cost_text(&p["estimated_cost"])],
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
    let roots: Vec<_> = rows(&p["sessions"])
        .filter(|row| row.get("inclusive").is_some())
        .map(|row| {
            vec![
                label(&row["session"]),
                number(&row["tokens"]),
                cost_text(&row["observed_cost"]),
                cost_text(&row["estimated_cost"]),
                number(&row["inclusive"]["tokens"]),
                cost_text(&row["inclusive"]["observed_cost"]),
                cost_text(&row["inclusive"]["estimated_cost"]),
            ]
        })
        .collect();
    if !roots.is_empty() {
        out.push('\n');
        out.push_str(&table(
            &[
                "Session",
                "Exclusive tokens",
                "Observed cost",
                "Estimated cost",
                "Inclusive tokens",
                "Inclusive observed",
                "Inclusive estimated",
            ],
            &roots,
            markdown,
        ));
        out.push_str("Inclusive (with sub-agents) totals are non-additive across root sessions.\n");
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
    let mode_suffix = report["window_resolution"]
        .as_str()
        .map(|_| format!(" · window={}", label(&report["window_resolution"])))
        .unwrap_or_default();
    let mut out = format!(
        "Dendro local explorer · {} to {}{}\nTab/←/→ panels · ↑/↓ seats · Enter detail · 1/2/3 trace · ?/h help · q quit\n\n",
        label(&report["payload"]["from"]),
        label(&report["payload"]["to"]),
        mode_suffix
    );
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
    ReadRequest::open(db)?.status(rig)
}

fn database_status_request(request: &ReadRequest, rig: Option<&str>) -> Result<Value> {
    let db_display = crate::redact(&request.logical().display().to_string());
    if !request.initialized() {
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
    let store = request.open_store()?;
    let conn = store.connection();
    let (min_ts, max_ts): (Option<String>, Option<String>) = if let Some(r) = rig {
        conn.query_row(
            "WITH rig_sessions AS (
                SELECT DISTINCT harness, native_session FROM identity_facts WHERE rig = ?1
             ),
             rig_seats AS (
                SELECT DISTINCT seat FROM identity_facts WHERE rig = ?1 AND seat <> ''
             )
             SELECT
                strftime(date_trunc('second', MIN(ts)), '%Y-%m-%dT%H:%M:%SZ'),
                strftime(date_trunc('second', MAX(ts)), '%Y-%m-%dT%H:%M:%SZ')
             FROM (
                SELECT u.ts FROM usage_facts u JOIN rig_sessions s ON u.harness = s.harness AND u.native_session = s.native_session WHERE u.ts IS NOT NULL
                UNION ALL SELECT t.ts FROM tool_facts t JOIN rig_sessions s ON t.harness = s.harness AND t.native_session = s.native_session WHERE t.ts IS NOT NULL
                UNION ALL SELECT a.ts FROM activity_facts a WHERE right(a.seat, length(?1) + 1) = ('@' || ?1) OR a.seat IN (SELECT seat FROM rig_seats)
                UNION ALL SELECT q.ts FROM queue_facts q WHERE right(q.source_seat, length(?1) + 1) = ('@' || ?1) OR right(q.dest_seat, length(?1) + 1) = ('@' || ?1) OR q.source_seat IN (SELECT seat FROM rig_seats) OR q.dest_seat IN (SELECT seat FROM rig_seats)
                UNION ALL SELECT b.created_at AS ts FROM seat_bounds b WHERE b.created_at IS NOT NULL AND (right(b.seat, length(?1) + 1) = ('@' || ?1) OR b.seat IN (SELECT seat FROM rig_seats))
                UNION ALL SELECT b.stopped_at AS ts FROM seat_bounds b WHERE b.stopped_at IS NOT NULL AND (right(b.seat, length(?1) + 1) = ('@' || ?1) OR b.seat IN (SELECT seat FROM rig_seats))
             )",
            duckdb::params![r],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?
    } else {
        conn.query_row(
            "SELECT
                strftime(date_trunc('second', MIN(ts)), '%Y-%m-%dT%H:%M:%SZ'),
                strftime(date_trunc('second', MAX(ts)), '%Y-%m-%dT%H:%M:%SZ')
             FROM (
                SELECT ts FROM usage_facts WHERE ts IS NOT NULL
                UNION ALL SELECT ts FROM activity_facts
                UNION ALL SELECT ts FROM queue_facts
                UNION ALL SELECT ts FROM tool_facts WHERE ts IS NOT NULL
                UNION ALL SELECT created_at AS ts FROM seat_bounds WHERE created_at IS NOT NULL
                UNION ALL SELECT stopped_at AS ts FROM seat_bounds WHERE stopped_at IS NOT NULL
             )",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?
    };
    let usage_rows: i64 = conn.query_row("SELECT COUNT(*) FROM usage_facts", [], |r| r.get(0))?;
    let activity_rows: i64 =
        conn.query_row("SELECT COUNT(*) FROM activity_facts", [], |r| r.get(0))?;
    let queue_rows: i64 = conn.query_row("SELECT COUNT(*) FROM queue_facts", [], |r| r.get(0))?;
    let mut rig_stmt =
        conn.prepare("SELECT DISTINCT rig FROM identity_facts WHERE rig <> '' ORDER BY rig")?;
    let rigs: Vec<String> = rig_stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<duckdb::Result<Vec<_>>>()?
        .into_iter()
        .map(|s| crate::redact(&s))
        .collect();
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
        .collect::<duckdb::Result<Vec<_>>>()?
        .into_iter()
        .map(|s| crate::redact(&s))
        .collect();
    drop(seat_stmt);
    drop(rig_stmt);

    let (pi_openrig_files, pi_standalone_files): (i64, i64) = conn.query_row(
        "SELECT
            COUNT(CASE WHEN harness = 'pi' AND display_path NOT LIKE '%/.pi/agent/sessions%' AND display_path NOT LIKE '%\\.pi\\agent\\sessions%' THEN 1 END),
            COUNT(CASE WHEN harness = 'pi' AND (display_path LIKE '%/.pi/agent/sessions%' OR display_path LIKE '%\\.pi\\agent\\sessions%') THEN 1 END)
         FROM source_files",
        duckdb::params![],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let (pi_linked_usage, pi_standalone_usage): (i64, i64) = conn.query_row(
        "SELECT
            COUNT(CASE WHEN native_session IN (SELECT native_session FROM identity_facts WHERE harness = 'pi') THEN 1 END),
            COUNT(CASE WHEN native_session NOT IN (SELECT native_session FROM identity_facts WHERE harness = 'pi') THEN 1 END)
         FROM usage_facts WHERE harness = 'pi'",
        duckdb::params![],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let pi_breakdown = json!({
        "openrig_files": pi_openrig_files,
        "standalone_files": pi_standalone_files,
        "linked_usage_rows": pi_linked_usage,
        "standalone_usage_rows": pi_standalone_usage,
    });

    drop(store);
    let semantic =
        crate::semantic::cached_scope_physical(request.database()).map(|(r, f, t, fresh)| {
            json!({
                "rig": r.map(|v| crate::redact(&v)),
                "from": f,
                "to": t,
                "fresh": fresh
            })
        });
    let semantic_fresh = semantic
        .as_ref()
        .and_then(|s| s["fresh"].as_bool())
        .unwrap_or(false);
    let entity_breakdown = build_entity_breakdown(&rigs, &seats);
    Ok(json!({
        "exists": true,
        "db": db_display,
        "status": if min_ts.is_some() { "ready" } else { "empty" },
        "selected_rig": rig.map(crate::redact),
        "min_timestamp": min_ts,
        "max_timestamp": max_ts,
        "usage_rows": usage_rows,
        "activity_rows": activity_rows,
        "queue_rows": queue_rows,
        "rigs": rigs,
        "seats": seats,
        "entity_breakdown": entity_breakdown,
        "semantic_fresh": semantic_fresh,
        "semantic_snapshot": semantic,
        "harnesses": {
            "pi": pi_breakdown.clone(),
        },
        "pi_breakdown": pi_breakdown,
        "next_commands": [
            format!("dendro dashboard --db {}", db_display),
            format!("dendro ask --db {} \"Which seat had the highest idle time?\"", db_display),
            format!("dendro dashboard --db {} --html dendro-dashboard.html", db_display)
        ]
    }))
}

/// Classification of raw identifier strings stored in `seats` / `activity` / `queue_facts`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntityKind {
    AgentSeat,
    QueueEndpoint,
    SystemOrGate,
    UnlinkedOrRawId,
}

/// Returns true when `id` looks like a 26-character Crockford Base32 ULID (`01...`).
pub fn is_raw_ulid(id: &str) -> bool {
    id.len() == 26
        && id.starts_with("01")
        && id
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

/// Deterministic entity classifier separating canonical `<role>@<rig>` seats from queue endpoints,
/// supervisory/system nodes, and unlinked/raw identifiers.
pub fn classify_entity_id(id: &str) -> EntityKind {
    let trimmed = id.trim();
    if trimmed.starts_with("qitem-") {
        return EntityKind::QueueEndpoint;
    }
    if trimmed.starts_with("external:")
        || trimmed.starts_with("system:")
        || trimmed.ends_with("@system")
        || matches!(trimmed, "system" | "steerer" | "steer-lead")
    {
        return EntityKind::SystemOrGate;
    }
    if let Some((left, right)) = trimmed.split_once('@') {
        if !left.is_empty()
            && !right.is_empty()
            && !left.starts_with("qitem-")
            && !is_raw_ulid(right)
        {
            return EntityKind::AgentSeat;
        }
    }
    EntityKind::UnlinkedOrRawId
}

fn build_entity_breakdown(rigs: &[String], seats: &[String]) -> Value {
    let mut agent_seats = Vec::new();
    let mut queue_endpoints = Vec::new();
    let mut system_and_gates = Vec::new();
    let mut unlinked_ids = Vec::new();
    for s in seats {
        match classify_entity_id(s) {
            EntityKind::AgentSeat => agent_seats.push(s.clone()),
            EntityKind::QueueEndpoint => queue_endpoints.push(s.clone()),
            EntityKind::SystemOrGate => system_and_gates.push(s.clone()),
            EntityKind::UnlinkedOrRawId => unlinked_ids.push(s.clone()),
        }
    }
    let mut named_rigs = Vec::new();
    let mut ulid_rigs = Vec::new();
    for r in rigs {
        if is_raw_ulid(r) {
            ulid_rigs.push(r.clone());
        } else {
            named_rigs.push(r.clone());
        }
    }
    json!({
        "agent_seats": agent_seats,
        "agent_seats_count": agent_seats.len(),
        "queue_endpoints_count": queue_endpoints.len(),
        "system_and_gates": system_and_gates,
        "system_and_gates_count": system_and_gates.len(),
        "unlinked_ids": unlinked_ids,
        "unlinked_ids_count": unlinked_ids.len(),
        "named_rigs": named_rigs,
        "ulid_rigs_count": ulid_rigs.len(),
    })
}

/// Resolve an explicit (`--from`/`--to`) or relative (`--window 1h|24h|7d|30d|all`) window,
/// anchoring relative windows to the database's recorded timestamp bounds when available.
/// Because `report_json` uses a half-open `[from, to)` interval (`ts >= from AND ts < to`),
/// `end_d` is set to `max_d + 1s` so events recorded at `max_timestamp` (including sub-second events) are included.
pub fn resolve_window(
    db: &std::path::Path,
    rig: Option<&str>,
    window: Option<&str>,
    from: Option<&str>,
    to: Option<&str>,
) -> Result<(String, String, String)> {
    ReadRequest::open(db)?.window(rig, window, from, to)
}

fn resolve_window_request(
    request: &ReadRequest,
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
    if request.initialized() {
        #[cfg(any(windows, target_os = "linux"))]
        let status = Some(request.status(rig)?);
        #[cfg(not(any(windows, target_os = "linux")))]
        let status = request.status(rig).ok();
        if let Some(status) = status {
            let min_dt = status["min_timestamp"]
                .as_str()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|d| d.with_timezone(&chrono::Utc));
            let max_dt = status["max_timestamp"]
                .as_str()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|d| d.with_timezone(&chrono::Utc));
            if let (Some(min_d), Some(max_d)) = (min_dt, max_dt) {
                let end_d = max_d + chrono::Duration::seconds(1);
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

fn append_discovered_sources(out: &mut String, status: &Value) {
    out.push_str("\nDISCOVERED HARNESS SOURCES\n");
    let Some(sources) = status.get("discovered_sources").and_then(|v| v.as_array()) else {
        out.push_str("  (none attached)\n");
        return;
    };
    if sources.is_empty() {
        out.push_str("  (none)\n");
        return;
    }
    let rendered: Vec<(String, String, String)> = sources
        .iter()
        .map(|s| {
            let name = s["name"].as_str().unwrap_or("?").to_string();
            let path = s["path"].as_str().unwrap_or("?").to_string();
            let state = if s["exists"].as_bool().unwrap_or(false) {
                "present"
            } else {
                "missing"
            };
            (name, state.to_string(), path)
        })
        .collect();
    let name_w = rendered
        .iter()
        .map(|(name, _, _)| name.chars().count())
        .max()
        .unwrap_or(4)
        .max(4);
    out.push_str(&format!(
        "  {name:<name_w$}  {state:<7}  Path\n",
        name = "Name",
        state = "State"
    ));
    for (name, state, path) in rendered {
        out.push_str(&format!("  {name:<name_w$}  {state:<7}  {path}\n"));
    }
}

fn group_agent_seats(agent_seats: &[&str]) -> Vec<(String, Vec<String>)> {
    let mut grouped: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for seat in agent_seats {
        if let Some((name, rig)) = seat.split_once('@') {
            grouped
                .entry(rig.to_string())
                .or_default()
                .push(name.to_string());
        } else {
            grouped
                .entry("(unscoped)".into())
                .or_default()
                .push((*seat).to_string());
        }
    }
    grouped.into_iter().collect()
}

fn write_rig_seat_row(out: &mut String, rig: &str, names: &[String]) {
    let joined = names.join(", ");
    let prefix = format!("  - {rig} ({}): ", names.len());
    if prefix.chars().count() + joined.chars().count() <= 96 {
        out.push_str(&prefix);
        out.push_str(&joined);
        out.push('\n');
        return;
    }
    out.push_str(&format!("  - {rig} ({}):\n", names.len()));
    let mut line = String::from("      ");
    for (i, name) in names.iter().enumerate() {
        let extra = if i == 0 {
            name.clone()
        } else {
            format!(", {name}")
        };
        if line.chars().count() + extra.chars().count() > 96 && i > 0 {
            out.push_str(&line);
            out.push_str(",\n");
            line = format!("      {name}");
        } else {
            line.push_str(&extra);
        }
    }
    if !line.trim().is_empty() {
        out.push_str(&line);
        out.push('\n');
    }
}

fn write_seat_fallback(out: &mut String, seats: &[&str]) {
    if seats.is_empty() {
        out.push_str("  Seats (0): none\n");
        return;
    }
    if seats.len() <= 12 {
        out.push_str(&format!(
            "  Seats ({}): {}\n",
            seats.len(),
            seats.join(", ")
        ));
        return;
    }
    let sample = seats.iter().take(8).copied().collect::<Vec<_>>().join(", ");
    out.push_str(&format!("  Seats ({}): {sample}, ...\n", seats.len()));
}

fn append_topology(out: &mut String, status: &Value) {
    out.push_str("\nFLEET TOPOLOGY (RIGS & AGENT SEATS)\n");
    if !status["exists"].as_bool().unwrap_or(false) {
        out.push_str("  Unavailable until the analytics database exists.\n");
        return;
    }
    let rigs: Vec<&str> = rows(&status["rigs"]).filter_map(|v| v.as_str()).collect();
    let seats: Vec<&str> = rows(&status["seats"]).filter_map(|v| v.as_str()).collect();
    let Some(eb) = status.get("entity_breakdown").filter(|v| !v.is_null()) else {
        out.push_str(&format!(
            "  Rigs ({}): {}\n",
            rigs.len(),
            if rigs.is_empty() {
                "none".into()
            } else {
                rigs.join(", ")
            }
        ));
        write_seat_fallback(out, &seats);
        return;
    };
    let named_rigs: Vec<&str> = rows(&eb["named_rigs"]).filter_map(|v| v.as_str()).collect();
    let ulid_rigs_count = eb["ulid_rigs_count"].as_u64().unwrap_or(0);
    if !named_rigs.is_empty() && ulid_rigs_count > 0 {
        out.push_str(&format!(
            "  Rigs ({} named, +{} ephemeral ULID): {}\n",
            named_rigs.len(),
            ulid_rigs_count,
            named_rigs.join(", ")
        ));
    } else {
        out.push_str(&format!(
            "  Rigs ({}): {}\n",
            rigs.len(),
            if rigs.is_empty() {
                "none".into()
            } else {
                rigs.join(", ")
            }
        ));
    }
    let agent_seats: Vec<&str> = rows(&eb["agent_seats"])
        .filter_map(|v| v.as_str())
        .collect();
    let qitems = eb["queue_endpoints_count"].as_u64().unwrap_or(0);
    let sys_gates = eb["system_and_gates_count"].as_u64().unwrap_or(0);
    let unlinked = eb["unlinked_ids_count"].as_u64().unwrap_or(0);
    if agent_seats.is_empty() {
        write_seat_fallback(out, &seats);
    } else {
        let grouped = group_agent_seats(&agent_seats);
        for (rig, names) in &grouped {
            write_rig_seat_row(out, rig, names);
        }
        let compact = agent_seats.join(", ");
        if compact.chars().count() <= 88 {
            out.push_str(&format!(
                "  Agent Seats ({}): {compact}\n",
                agent_seats.len()
            ));
        } else {
            out.push_str(&format!(
                "  Agent seats: {} across {} rigs\n",
                agent_seats.len(),
                grouped.len()
            ));
        }
    }
    if !agent_seats.is_empty() || qitems > 0 || sys_gates > 0 || unlinked > 0 {
        out.push_str(&format!(
            "  Entity hygiene: {} total IDs ({} agent seats · {} queue endpoints · {} system/gates · {} unlinked)\n",
            seats.len(),
            agent_seats.len(),
            qitems,
            sys_gates,
            unlinked
        ));
    }
}

fn append_quickstart(out: &mut String, status: &Value) {
    out.push_str("\nQUICKSTART & COMMANDS\n");
    out.push_str("Usage: dendro <COMMAND> [OPTIONS]\n\n");
    let body = table(
        &["Command", "Purpose"],
        &[
            vec![
                "status".into(),
                "Inspect coverage, topology, and next commands".into(),
            ],
            vec![
                "dashboard".into(),
                "Open the 6-screen cockpit or export HTML".into(),
            ],
            vec!["ui".into(), "Open the cockpit on an explicit window".into()],
            vec![
                "ask".into(),
                "Answer a supported plain-English question".into(),
            ],
            vec![
                "run".into(),
                "Ingest local harness roots into DuckDB".into(),
            ],
            vec![
                "report".into(),
                "Print an Operational Intelligence report".into(),
            ],
            vec![
                "trace".into(),
                "Print trace inputs for one metric id".into(),
            ],
            vec![
                "simulate".into(),
                "Run a historical topology what-if".into(),
            ],
        ],
        false,
    );
    for line in body.lines() {
        out.push_str("  ");
        out.push_str(line);
        out.push('\n');
    }
    out.push('\n');
    out.push_str("Next commands:\n");
    for cmd in rows(&status["next_commands"]) {
        if let Some(c) = cmd.as_str() {
            out.push_str(&format!("  $ {c}\n"));
        }
    }
}

/// Human-readable summary of `database_status`.
pub fn human_status(status: &Value) -> String {
    let mut out = String::from("=== DENDRO FLEET INTELLIGENCE ===\n");
    out.push_str("\nDATABASE & COVERAGE\n");
    out.push_str(&format!(
        "  Database: {}\n",
        status["db"].as_str().unwrap_or("dendro.duckdb")
    ));
    if !status["exists"].as_bool().unwrap_or(false) {
        out.push_str("  State: NOT INITIALIZED (analytics database does not exist yet)\n");
        append_discovered_sources(&mut out, status);
        append_topology(&mut out, status);
        append_quickstart(&mut out, status);
        return out;
    }
    out.push_str(&format!(
        "  State: {} · usage={} · activity={} · queue={}\n",
        status["status"]
            .as_str()
            .unwrap_or("unknown")
            .to_uppercase(),
        number(&status["usage_rows"]),
        number(&status["activity_rows"]),
        number(&status["queue_rows"])
    ));
    if let Some(rig) = status.get("selected_rig").and_then(|v| v.as_str()) {
        out.push_str(&format!("  Selected rig: {rig}\n"));
    }
    out.push_str(&format!(
        "  Recorded window: {} to {}\n",
        status["min_timestamp"].as_str().unwrap_or("Unavailable"),
        status["max_timestamp"].as_str().unwrap_or("Unavailable")
    ));
    if let Some(pi) = status.get("pi_breakdown").filter(|v| !v.is_null()) {
        out.push_str(&format!(
            "  Pi sources: openrig_files={} · standalone_files={} · linked_usage={} · standalone_usage={}\n",
            number(&pi["openrig_files"]),
            number(&pi["standalone_files"]),
            number(&pi["linked_usage_rows"]),
            number(&pi["standalone_usage_rows"])
        ));
    }
    out.push_str(&format!(
        "  Semantic (Nao) cache: {}\n",
        if status["semantic_fresh"].as_bool().unwrap_or(false) {
            "FRESH"
        } else {
            "STALE / NOT MATERIALIZED (auto-refreshed by 'dendro ask')"
        }
    ));
    append_discovered_sources(&mut out, status);
    append_topology(&mut out, status);
    append_quickstart(&mut out, status);
    out
}

/// Human-readable terminal summary of a `dendro ask` response (`semantic::answer`).
pub fn human_ask_answer(question: &str, ans: &Value) -> String {
    let scope = &ans["scope"];
    let rig_str = scope["rig"]
        .as_str()
        .map(crate::redact)
        .unwrap_or_else(|| "all".into());
    let from_str = scope["from"].as_str().unwrap_or("Unavailable");
    let to_str = scope["to"].as_str().unwrap_or("Unavailable");
    let mut out = format!(
        "Dendro Ask · rig={} · {} to {}\nQuestion: {}\n\n",
        rig_str,
        from_str,
        to_str,
        crate::redact(question)
    );
    if ans["status"] != "ok" {
        out.push_str(&format!(
            "Status: {}\n",
            ans["message"].as_str().unwrap_or("Insufficient data")
        ));
        if let Some(examples) = ans["supported_examples"].as_array() {
            out.push_str("\nClosest supported queries:\n");
            for item in examples.iter().take(3) {
                let intent_name = item["intent"].as_str().unwrap_or("Intent");
                let phrases: Vec<&str> = rows(&item["examples"])
                    .filter_map(|v| v.as_str())
                    .take(2)
                    .collect();
                out.push_str(&format!("  • {intent_name}: {}\n", phrases.join(" | ")));
            }
        }
        return out;
    }
    if let Some(ordering) = ans["ordering"].as_str() {
        out.push_str(&format!("Ordering: {ordering}\n"));
        let r_list: Vec<Vec<String>> = rows(&ans["rows"])
            .take(15)
            .map(|r| {
                if r.get("idle_with_work_secs").is_some() {
                    vec![
                        label(&r["seat"]),
                        number(&r["idle_with_work_secs"]),
                        number(&r["active_secs"]),
                        number(&r["observed_secs"]),
                        percent(&r["ceremony_ratio"]),
                    ]
                } else {
                    vec![
                        label(&r["seat"]),
                        number(&r["tokens_per_closed_item"]),
                        number(&r["closed_items"]),
                        number(&r["tokens"]),
                    ]
                }
            })
            .collect();
        if r_list.first().is_some_and(|r| r.len() == 5) {
            out.push_str(&table(
                &[
                    "Seat",
                    "Idle w/ work s",
                    "Running s",
                    "Observed s",
                    "Ceremony %",
                ],
                &r_list,
                false,
            ));
        } else {
            out.push_str(&table(
                &["Seat", "Tokens / closed item", "Closed items", "Tokens"],
                &r_list,
                false,
            ));
        }
    } else if ans.get("handoff_friction").is_some() {
        let pairs: Vec<Vec<String>> = rows(&ans["handoff_friction"]["pairs"])
            .take(15)
            .map(|r| {
                vec![
                    label(&r["source_seat"]),
                    label(&r["dest_seat"]),
                    number(&r["count"]),
                    number(&r["return_count"]),
                    percent(&r["return_rate"]),
                    label(&r["ring_boundary"]),
                ]
            })
            .collect();
        out.push_str(&table(
            &[
                "From",
                "To",
                "Handoffs",
                "Returns",
                "Return rate",
                "Boundary",
            ],
            &pairs,
            false,
        ));
    } else if ans.get("current").is_some() && ans.get("baseline").is_some() {
        let cur = &ans["current"];
        let base = &ans["baseline"];
        let delta_rows = vec![
            vec![
                "M1 yield (tokens/item)".into(),
                number(&base["m1_yield"]),
                number(&cur["m1_yield"]),
            ],
            vec![
                "M2 return rate".into(),
                percent(&base["m2_return_rate"]),
                percent(&cur["m2_return_rate"]),
            ],
            vec![
                "M3 rework chains".into(),
                number(&base["m3_rework_chains"]),
                number(&cur["m3_rework_chains"]),
            ],
            vec![
                "Recorded tokens".into(),
                number(&base["tokens"]),
                number(&cur["tokens"]),
            ],
        ];
        out.push_str(&table(
            &["Metric", "Baseline", "Current"],
            &delta_rows,
            false,
        ));
    } else if let Some(keys) = ans["source_keys"].as_array() {
        out.push_str(&format!(
            "Provenance source keys in scope: {}\nTrace command: {}\n",
            keys.len(),
            ans["trace"].as_str().unwrap_or("dendro trace")
        ));
    }
    if let Some(limit) = ans["limit"].as_str() {
        out.push_str(&format!("\nEpistemic limit: {limit}\n"));
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
fn project_cost(v: &Value) -> Value {
    let status = match v["status"].as_str() {
        Some("complete") => "complete",
        Some("partial") => "partial",
        _ => "unknown",
    };
    json!({
        "status": status,
        "usd": v["usd"].as_f64().filter(|amount| amount.is_finite()),
        "unknown_rows": v["unknown_rows"].as_u64().unwrap_or(0),
        "known_rows": v["known_rows"].as_u64().unwrap_or(0),
    })
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
    out.insert("observed_cost".into(), project_cost(&p["observed_cost"]));
    out.insert("estimated_cost".into(), project_cost(&p["estimated_cost"]));
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
        #[cfg(any(windows, target_os = "linux"))]
        let private = crate::platform_v1::create_user_private_root_with_choice(
            "dendro-status-test-",
            &crate::platform_v1::StorageRootChoice::UserProfile,
        )
        .unwrap();
        #[cfg(any(windows, target_os = "linux"))]
        let missing = private.path().join("missing.duckdb");
        #[cfg(not(any(windows, target_os = "linux")))]
        let missing = std::env::temp_dir().join("dendro-missing-status-test.duckdb");
        let _ = std::fs::remove_file(&missing);
        let status = database_status(&missing, None).unwrap();
        assert_eq!(status["status"], "not_initialized");
        let rendered = human_status(&status);
        assert!(rendered.contains("=== DENDRO FLEET INTELLIGENCE ==="));
        assert!(rendered.contains("DATABASE & COVERAGE"));
        assert!(rendered.contains("State: NOT INITIALIZED"));
        assert!(rendered.contains("Usage: dendro <COMMAND> [OPTIONS]"));
        assert!(rendered.contains("QUICKSTART & COMMANDS"));
        assert!(rendered.contains("Next commands:"));
        for command in [
            "status",
            "dashboard",
            "ui",
            "ask",
            "run",
            "report",
            "trace",
            "simulate",
        ] {
            assert!(
                rendered
                    .lines()
                    .any(|line| line.split_whitespace().next() == Some(command)),
                "missing command row {command}: {rendered}"
            );
        }
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

    #[test]
    fn test_pi_breakdown_and_human_status() {
        #[cfg(any(windows, target_os = "linux"))]
        let private = crate::platform_v1::create_user_private_root_with_choice(
            "dendro-pi-test-",
            &crate::platform_v1::StorageRootChoice::UserProfile,
        )
        .unwrap();
        #[cfg(any(windows, target_os = "linux"))]
        let dir = private.path().to_path_buf();
        #[cfg(not(any(windows, target_os = "linux")))]
        let dir = std::env::temp_dir().join(format!(
            "dendro-pi-breakdown-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        #[cfg(not(any(windows, target_os = "linux")))]
        {
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
        }
        let db = dir.join("test.duckdb");
        let store = crate::AnalyticsStore::open_path(&db, "key").unwrap();

        // Usage facts: 1 linked (sess-openrig), 2 standalone (sess-standalone)
        let make_usage = |key: &str, session: &str, seq: i64| crate::UsageFact {
            source_key: key.into(),
            generation: 1,
            harness: "pi".into(),
            native_session: session.into(),
            message_id: Some(format!("m{seq}")),
            ts: Some(chrono::Utc::now()),
            seq,
            input_uncached: 10,
            output: 20,
            cache_read: 0,
            cache_write: 0,
            reasoning: 0,
            currency_total: None,
            predecessor_source_key: None,
            predecessor_reason: None,
            prior_input_tokens: None,
            prior_cached_input_tokens: None,
            prior_cache_write_input_tokens: None,
            prior_output_tokens: None,
            prior_reasoning_output_tokens: None,
        };

        // OpenRig source file with 1 linked usage row
        store
            .commit_file_batch(
                "src_openrig",
                "pi",
                "/home/user/.openrig/state/pi/s1.jsonl",
                1,
                0,
                "hash1",
                None,
                &[make_usage("u1", "sess-openrig", 1)],
                &[],
            )
            .unwrap();
        store
            .set_source_session("src_openrig", "sess-openrig")
            .unwrap();

        // Standalone source file (contains agent/sessions) with 2 standalone usage rows
        store
            .commit_file_batch(
                "src_standalone",
                "pi",
                "/home/user/.pi/agent/sessions/s2.jsonl",
                1,
                0,
                "hash2",
                None,
                &[
                    make_usage("u2", "sess-standalone", 2),
                    make_usage("u3", "sess-standalone", 3),
                ],
                &[],
            )
            .unwrap();
        store
            .set_source_session("src_standalone", "sess-standalone")
            .unwrap();

        // Identity fact linking sess-openrig
        store
            .insert_identity(&crate::IdentityFact {
                source_key: "id1".into(),
                harness: "pi".into(),
                native_session: "sess-openrig".into(),
                rig: "r1".into(),
                seat: "s1".into(),
                ts: Some(chrono::Utc::now()),
            })
            .unwrap();

        store.checkpoint().unwrap();
        drop(store);

        let status = database_status(&db, None).unwrap();
        assert_eq!(status["status"], "ready");

        let breakdown = &status["pi_breakdown"];
        assert_eq!(breakdown["openrig_files"], 1);
        assert_eq!(breakdown["standalone_files"], 1);
        assert_eq!(breakdown["linked_usage_rows"], 1);
        assert_eq!(breakdown["standalone_usage_rows"], 2);

        assert_eq!(status["harnesses"]["pi"], *breakdown);

        let rendered = human_status(&status);
        assert!(rendered.contains("State: READY · usage="));
        assert!(rendered.contains(
            "Pi sources: openrig_files=1 · standalone_files=1 · linked_usage=1 · standalone_usage=2"
        ));
        assert!(rendered.contains("Usage: dendro <COMMAND> [OPTIONS]"));

        assert_eq!(
            classify_entity_id("dev-build@dendro-dev"),
            EntityKind::AgentSeat
        );
        assert_eq!(
            classify_entity_id("qitem-20261014170440-a11b2afc9ba28ec2"),
            EntityKind::QueueEndpoint
        );
        assert_eq!(
            classify_entity_id("watchdog@system"),
            EntityKind::SystemOrGate
        );
        assert_eq!(
            classify_entity_id("external:person-authorizes-push-and-pr"),
            EntityKind::SystemOrGate
        );
        assert_eq!(
            classify_entity_id("01M4EB4XC6DTG02W6TMSRR7RAE"),
            EntityKind::UnlinkedOrRawId
        );
        assert!(is_raw_ulid("01M4EB4XC6DTG02W6TMSRR7RAE"));
        assert!(!is_raw_ulid("dendro-dev"));

        let ask_txt = human_ask_answer(
            "which seat burned the most tokens?",
            &json!({
                "status": "ok",
                "scope": {"rig": "r1", "from": "2026-10-01T00:00:00Z", "to": "2026-10-02T00:00:00Z"},
                "ordering": "tokens_per_closed_item descending",
                "rows": [{"seat": "s1@r1", "tokens_per_closed_item": 1500, "closed_items": 2, "tokens": 3000}]
            }),
        );
        assert!(ask_txt.contains("Dendro Ask · rig=r1"));
        assert!(ask_txt.contains("s1@r1"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn human_status_groups_seats_by_rig_and_keeps_compact_invariants() {
        let mut agent_seats: Vec<String> =
            (0..44).map(|i| format!("seat{i:02}@dendro-dev")).collect();
        agent_seats.push("dev-qa@other-rig".into());
        let mut seats = agent_seats.clone();
        seats.push("qitem-20261014170440-a11b2afc9ba28ec2".into());
        seats.push("watchdog@system".into());
        let status = json!({
            "exists": true,
            "db": "dendro.duckdb",
            "status": "ready",
            "usage_rows": 9,
            "activity_rows": 4,
            "queue_rows": 2,
            "min_timestamp": "2026-10-01T00:00:00Z",
            "max_timestamp": "2026-10-02T00:00:00Z",
            "rigs": ["dendro-dev", "other-rig", "01M4EB4XC6DTG02W6TMSRR7RAE"],
            "seats": seats,
            "entity_breakdown": {
                "agent_seats": agent_seats,
                "named_rigs": ["dendro-dev", "other-rig"],
                "ulid_rigs_count": 1,
                "queue_endpoints_count": 1,
                "system_and_gates_count": 1,
                "unlinked_ids_count": 0
            },
            "semantic_fresh": false,
            "pi_breakdown": {
                "openrig_files": 2,
                "standalone_files": 3,
                "linked_usage_rows": 4,
                "standalone_usage_rows": 5
            },
            "discovered_sources": [
                {"name": "pi", "path": "/home/user/.openrig/state/pi", "exists": true},
                {"name": "pi", "path": "/home/user/.pi/agent/sessions", "exists": false}
            ],
            "next_commands": ["dendro dashboard --db dendro.duckdb"]
        });
        let rendered = human_status(&status);
        assert!(rendered.contains("=== DENDRO FLEET INTELLIGENCE ==="));
        assert!(rendered.contains("DATABASE & COVERAGE"));
        assert!(rendered.contains("DISCOVERED HARNESS SOURCES"));
        assert!(rendered.contains("FLEET TOPOLOGY (RIGS & AGENT SEATS)"));
        assert!(rendered.contains("QUICKSTART & COMMANDS"));
        assert!(rendered.contains("State: READY · usage=9 · activity=4 · queue=2"));
        assert!(rendered.contains(
            "Pi sources: openrig_files=2 · standalone_files=3 · linked_usage=4 · standalone_usage=5"
        ));
        assert!(rendered.contains("  - dendro-dev (44):"));
        assert!(rendered.contains("  - other-rig (1): dev-qa"));
        assert!(rendered.contains("Agent seats: 45 across 2 rigs"));
        assert!(rendered.contains(
            "Entity hygiene: 47 total IDs (45 agent seats · 1 queue endpoints · 1 system/gates · 0 unlinked)"
        ));
        assert!(rendered.contains("Rigs (2 named, +1 ephemeral ULID): dendro-dev, other-rig"));
        assert!(!rendered
            .lines()
            .any(|line| line.matches("@dendro-dev").count() > 4));
        let empty = json!({
            "exists": true,
            "db": "dendro.duckdb",
            "status": "empty",
            "usage_rows": 0,
            "activity_rows": 0,
            "queue_rows": 0,
            "rigs": [],
            "seats": [],
            "next_commands": []
        });
        let empty_text = human_status(&empty);
        assert!(empty_text.contains("State: EMPTY · usage=0 · activity=0 · queue=0"));
    }
}
