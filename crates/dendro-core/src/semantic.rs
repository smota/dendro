//! Scoped, explicitly refreshed projections of the canonical report. No alternate metric engine.
use crate::{pipeline::report_json_physical as report_json, storage::AnalyticsStore};
use anyhow::{anyhow, bail, Context, Result};
use duckdb::params;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path},
};

pub const VIEWS: [&str; 8] = [
    "v_seat_idle_ranking",
    "v_handoff_rework_analysis",
    "v_mission_token_burn",
    "v_executive_governance",
    "v_message_mix",
    "v_subagent_lineage",
    "v_security_posture",
    "v_process_resources",
];
// Fixed SQL: identifiers and expressions never come from a conversational request.
const BASE_SIGNATURE: &str = "SELECT kind, count(*) n, CAST(bit_xor(h) AS VARCHAR) digest FROM (
SELECT 'dendro_meta' kind, hash(t) h FROM dendro_meta t UNION ALL
SELECT 'codex_counters' kind, hash(t) h FROM codex_counters t UNION ALL
SELECT 'warnings' kind, hash(t) h FROM warnings t UNION ALL
SELECT 'usage_facts' kind, hash(t) h FROM usage_facts t UNION ALL
SELECT 'tool_facts' kind, hash(t) h FROM tool_facts t UNION ALL
SELECT 'identity_facts' kind, hash(t) h FROM identity_facts t UNION ALL
SELECT 'activity_facts' kind, hash(t) h FROM activity_facts t UNION ALL
SELECT 'queue_facts' kind, hash(t) h FROM queue_facts t UNION ALL
SELECT 'seat_bounds' kind, hash(t) h FROM seat_bounds t UNION ALL
SELECT 'seat_roles' kind, hash(t) h FROM seat_roles t UNION ALL
SELECT 'skill_facts' kind, hash(t) h FROM skill_facts t UNION ALL
SELECT 'destination_snapshots' kind, hash(t) h FROM destination_snapshots t UNION ALL
SELECT 'source_files' kind, hash(t) h FROM source_files t) GROUP BY kind";

const OPTIONAL_SIGNATURES: [(&str, &str); 8] = [
    (
        "file_source_baselines",
        "SELECT count(*) n, CAST(bit_xor(hash(t)) AS VARCHAR) digest FROM file_source_baselines t HAVING count(*) > 0",
    ),
    (
        "openrig_row_records",
        "SELECT count(*) n, CAST(bit_xor(hash(t)) AS VARCHAR) digest FROM openrig_row_records t HAVING count(*) > 0",
    ),
    (
        "mutable_source_disclosures",
        "SELECT count(*) n, CAST(bit_xor(hash(t)) AS VARCHAR) digest FROM mutable_source_disclosures t HAVING count(*) > 0",
    ),
    (
        "message_facts",
        "SELECT count(*) n, CAST(bit_xor(hash(t)) AS VARCHAR) digest FROM message_facts t HAVING count(*) > 0",
    ),
    (
        "process_identities",
        "SELECT count(*) n, CAST(bit_xor(hash(t)) AS VARCHAR) digest FROM process_identities t HAVING count(*) > 0",
    ),
    (
        "process_samples",
        "SELECT count(*) n, CAST(bit_xor(hash(t)) AS VARCHAR) digest FROM process_samples t HAVING count(*) > 0",
    ),
    (
        "session_edges",
        "SELECT count(*) n, CAST(bit_xor(hash(t)) AS VARCHAR) digest FROM session_edges t HAVING count(*) > 0",
    ),
    (
        "security_facts",
        "SELECT count(*) n, CAST(bit_xor(hash(t)) AS VARCHAR) digest FROM security_facts t HAVING count(*) > 0",
    ),
];

fn signature(store: &AnalyticsStore) -> Result<String> {
    let conn = store.connection();
    let mut parts = BTreeMap::<String, String>::new();
    let mut stmt = conn.prepare(BASE_SIGNATURE)?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let kind: String = row.get(0)?;
        let n: i64 = row.get(1)?;
        let digest: String = row.get(2)?;
        parts.insert(kind.clone(), format!("{kind}:{n}:{digest}"));
    }
    for (table, query) in OPTIONAL_SIGNATURES {
        let exists: i64 = conn.query_row(
            "SELECT count(*) FROM information_schema.tables WHERE table_schema = 'main' AND table_name = ?",
            params![table],
            |r| r.get(0),
        )?;
        if exists == 0 {
            parts.insert(table.to_string(), format!("{table}:absent"));
            continue;
        }
        let mut opt_stmt = conn.prepare(query)?;
        let mut opt_rows = opt_stmt.query([])?;
        if let Some(row) = opt_rows.next()? {
            let n: i64 = row.get(0)?;
            let digest: String = row.get(1)?;
            parts.insert(table.to_string(), format!("{table}:{n}:{digest}"));
        } else {
            parts.insert(table.to_string(), format!("{table}:present:0"));
        }
    }
    Ok(format!(
        "[{}]",
        parts.into_values().collect::<Vec<_>>().join(", ")
    ))
}

/// Refresh the four views for exactly this report scope. Source readers close before writes.
/// A concurrent source update aborts rather than labelling an inconsistent snapshot as current.
pub fn refresh(db: &Path, rig: Option<&str>, from: &str, to: &str) -> Result<Value> {
    let request = crate::presentation::ReadRequest::open(db)?;
    refresh_request(&request, rig, from, to).map(|(receipt, _)| receipt)
}

/// Return new custody after immutable staged semantic publication for composite ask.
pub fn refresh_request(
    request: &crate::presentation::ReadRequest,
    rig: Option<&str>,
    from: &str,
    to: &str,
) -> Result<(Value, Option<crate::presentation::ReadRequest>)> {
    #[cfg(any(windows, target_os = "linux"))]
    {
        let adopted;
        let selected = if let Some(selected) = request.selected() {
            selected
        } else if request.is_legacy() {
            adopted = crate::selection_binding::adopt_legacy(
                request.logical(),
                &crate::platform_v1::StorageRootChoice::Explicit(
                    request
                        .logical()
                        .parent()
                        .ok_or_else(|| anyhow!("legacy database has no parent"))?
                        .to_path_buf(),
                ),
            )?;
            &adopted
        } else {
            bail!("semantic_uninitialized: initialize the database with dendro run first");
        };
        let mut receipt = Value::Null;
        let refreshed = crate::selection_binding::mutate_selected(selected, |staged, _key| {
            receipt = refresh_physical(staged, rig, from, to)?;
            Ok(())
        })?;
        receipt["analytics_selection_sha256"] = json!(hex::encode(refreshed.selection().sha256));
        Ok((
            receipt,
            Some(crate::presentation::ReadRequest::from_selected(refreshed)),
        ))
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    Ok((refresh_physical(request.database(), rig, from, to)?, None))
}

fn refresh_physical(db: &Path, rig: Option<&str>, from: &str, to: &str) -> Result<Value> {
    crate::pipeline::refuse_secret_path(db)?;
    let before = {
        let s = AnalyticsStore::open_read_only(db)?;
        signature(&s)?
    };
    let report = report_json(db, "decisions", rig, from, to)?;
    let read = AnalyticsStore::open_read_only(db)?;
    let key_id = read.key_id.clone();
    drop(read);
    let store = AnalyticsStore::open_existing(db, &key_id)?;
    let conn = store.connection();
    // Bound derived refresh even when a future report carries a very large extension.
    conn.execute_batch("SET memory_limit='4GB'; SET threads=1")?;
    conn.execute_batch("BEGIN TRANSACTION")?;
    let result = (|| {
        if signature(&store)? != before {
            bail!("source changed during semantic refresh; retry");
        }
        materialize(&store, &report, &before)?;
        Ok(())
    })();
    match result {
        Ok(()) => conn.execute_batch("COMMIT")?,
        Err(e) => {
            conn.execute_batch("ROLLBACK")?;
            return Err(e);
        }
    }
    Ok(
        json!({"status":"refreshed", "rig":rig,"from":from,"to":to,"views":VIEWS,"definition_version":report["definition_version"]}),
    )
}

fn census(value: &Value, out: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                if key == "source_key" {
                    if let Some(s) = value.as_str() {
                        out.insert(s.to_owned());
                    }
                } else if key.ends_with("keys") || key == "evidence" {
                    if let Some(a) = value.as_array() {
                        for v in a {
                            if let Some(s) = v.as_str() {
                                out.insert(s.to_owned());
                            }
                        }
                    }
                }
                census(value, out);
            }
        }
        Value::Array(a) => {
            for v in a {
                census(v, out);
            }
        }
        _ => {}
    }
}
fn stage(name: &str, started: std::time::Instant) {
    if std::env::var_os("DENDRO_TRACE_SEMANTIC").is_some() {
        eprintln!(
            "semantic_materialize stage={name} elapsed_ms={:.3}",
            started.elapsed().as_secs_f64() * 1000.0
        );
    }
}
fn materialize(store: &AnalyticsStore, report: &Value, sig: &str) -> Result<()> {
    let c = store.connection();
    let p = &report["payload"];
    let started = std::time::Instant::now();
    c.execute_batch("CREATE TABLE IF NOT EXISTS dendro_semantic_snapshot (report JSON, signature VARCHAR, refreshed_at TIMESTAMP); DELETE FROM dendro_semantic_snapshot;")?;
    c.execute(
        "INSERT INTO dendro_semantic_snapshot VALUES (?, ?, current_timestamp)",
        params![report.to_string(), sig],
    )?;
    stage("snapshot", started);
    let started = std::time::Instant::now();
    c.execute_batch(include_str!("semantic.sql"))?;
    c.execute(
        "INSERT INTO dendro_semantic_scope VALUES (?, ?, ?, ?, current_timestamp)",
        params![
            p["rig"].as_str(),
            p["from"].as_str(),
            p["to"].as_str(),
            report["definition_version"].as_str()
        ],
    )?;
    let mut keys = BTreeSet::new();
    census(p, &mut keys);
    c.execute_batch("CREATE TABLE IF NOT EXISTS dendro_semantic_provenance (source_key VARCHAR PRIMARY KEY); DELETE FROM dendro_semantic_provenance;")?;
    {
        let mut a = c.appender("dendro_semantic_provenance")?;
        for key in &keys {
            a.append_row(params![key])?;
        }
        a.flush()?;
    }
    stage("schema_provenance", started);
    let started = std::time::Instant::now();
    let rows = |field: &str| p[field].as_array().map(Vec::as_slice).unwrap_or(&[]);
    let mut evidence = BTreeMap::<&str, BTreeSet<&str>>::new();
    for (field, seat_field) in [("activity_events", "seat"), ("demand_events", "dest")] {
        for row in rows(field) {
            if let (Some(seat), Some(key)) = (row[seat_field].as_str(), row["source_key"].as_str())
            {
                evidence.entry(seat).or_default().insert(key);
            }
        }
    }
    let m1: BTreeMap<_, _> = rows("m1_by_seat")
        .iter()
        .filter_map(|r| r["seat"].as_str().map(|s| (s, r)))
        .collect();
    let mut seat_insert=c.prepare("INSERT INTO dendro_semantic_seats VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, CAST(CAST(? AS JSON) AS VARCHAR[]), ?)")?;
    for row in rows("activity") {
        let seat = row["seat"].as_str();
        let source_keys = seat.and_then(|s| evidence.get(s));
        let source_keys = source_keys.map(serde_json::to_string).transpose()?;
        let m1 = seat.and_then(|s| m1.get(s)).map(|v| v.to_string());
        seat_insert.execute(params![
            seat,
            row["observed_secs"].as_i64(),
            row["active_secs"].as_i64(),
            row["idle_with_work_secs"].as_i64(),
            row["idle_without_work_secs"].as_i64(),
            row["waiting_human_secs"].as_i64(),
            row["unknown_secs"].as_i64(),
            row["unparked_container_secs"].as_i64(),
            row["leaf_stall_secs"].as_i64(),
            row["ceremony_ratio"].as_f64(),
            row["tool_categories"].to_string(),
            row.to_string(),
            source_keys,
            m1
        ])?;
    }
    stage("seat_rows", started);
    let started = std::time::Instant::now();
    {
        let mut a = c.appender("dendro_semantic_handoffs")?;
        if let Some(pairs) = p["handoff_friction"]["pairs"].as_array() {
            for row in pairs {
                a.append_row(params![
                    row["source_seat"].as_str(),
                    row["dest_seat"].as_str(),
                    row["count"].as_i64(),
                    row["return_count"].as_i64(),
                    row["return_rate"].as_f64(),
                    row["wait_median_ms"].as_f64(),
                    row["exec_median_ms"].as_f64(),
                    row["ring_boundary"].as_str(),
                    row["source_keys"].to_string(),
                    row.to_string()
                ])?;
            }
        }
        a.flush()?;
    }
    stage("handoff_rows", started);
    let started = std::time::Instant::now();
    let mut lineage: BTreeMap<String, Value> = p["issue_lineage"]["records"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .filter_map(|r| r["source_key"].as_str().map(|s| (s.to_owned(), r.clone())))
        .collect();
    // The canonical lineage projection intentionally omits unlinked/collision usage.
    // Preserve those records using the same canonical session binding resolver.
    let needed: BTreeSet<_> = rows("trace_inputs")
        .iter()
        .filter_map(|r| r["source_key"].as_str())
        .filter(|key| !lineage.contains_key(*key))
        .collect();
    if !needed.is_empty() {
        let identities = store.load_identity()?;
        let bindings = crate::metrics::session_bindings(&identities);
        let mut statement =
            c.prepare("SELECT source_key,harness,native_session FROM usage_facts")?;
        let mut raw = statement.query([])?;
        while let Some(row) = raw.next()? {
            let key: String = row.get(0)?;
            if !needed.contains(key.as_str()) {
                continue;
            }
            let harness: String = row.get(1)?;
            let native: String = row.get(2)?;
            let binding = crate::metrics::binding_of(&bindings, &harness, &native);
            if binding.seat().is_some() {
                bail!("canonical linked usage lineage missing");
            }
            let bucket = if matches!(binding, crate::metrics::SessionBinding::Collision) {
                "ambiguous"
            } else {
                "unlinked_session"
            };
            lineage.insert(key.clone(),json!({"source_key":key,"seat":Value::Null,"bucket":bucket,"missions":[],"issues":[],"phases":[],"binding_status":binding.status(),"evidence":"canonical session binding; tagged lineage is unavailable"}));
        }
    }
    let models = store.load_usage_models()?;
    {
        let mut a = c.appender("dendro_semantic_usage")?;
        for row in rows("trace_inputs") {
            let key = row["source_key"]
                .as_str()
                .context("canonical usage key missing")?;
            let lineage = lineage
                .get(key)
                .context("canonical usage lineage missing")?;
            let (model, effort) = models
                .get(key)
                .map(|(m, e)| (m.as_deref(), e.as_deref()))
                .unwrap_or_default();
            a.append_row(params![
                key,
                lineage["seat"].as_str(),
                model,
                effort,
                lineage["bucket"].as_str(),
                lineage["missions"].to_string(),
                lineage["issues"].to_string(),
                lineage["phases"].to_string(),
                row["input_uncached"].as_i64(),
                row["output"].as_i64(),
                row["cache_read"].as_i64(),
                row["cache_write"].as_i64(),
                lineage.to_string()
            ])?;
        }
        a.flush()?;
    }
    stage("usage_rows", started);
    let started = std::time::Instant::now();
    c.execute("INSERT INTO dendro_semantic_executive VALUES (?, CAST(CAST(? AS JSON) AS VARCHAR[]), ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",params![p.to_string(),serde_json::to_string(&keys)?,p["m1_yield"].to_string(),p["m1_by_seat"].to_string(),p["m2_return_rate"].to_string(),p["m3_rework_chains"].to_string(),p["token_categories"].to_string(),p["link_rates"].to_string(),p["fleet_concurrency"].to_string(),p["watchdog_recovery"].to_string(),p["four_eyes_independence"].to_string(),p["context_compactions"].to_string(),p["cross_slice_cache_bleed"].to_string(),p["task_intake_overhead"].to_string(),p["issue_lineage"].to_string(),p["handoff_friction"]["vortex_loops"].to_string(),p["ring_boundaries"].to_string()])?;
    stage("executive_row", started);
    let started = std::time::Instant::now();
    materialize_optional_fact_tables(c)?;
    stage("optional_fact_rows", started);
    let started = std::time::Instant::now();
    c.execute_batch("UPDATE dendro_semantic_state SET dirty = FALSE WHERE id = 1; CREATE OR REPLACE VIEW dendro_semantic_guard AS SELECT CASE WHEN NOT dirty THEN true ELSE error('Insufficient data: semantic snapshot is stale; refresh required') END AS fresh FROM dendro_semantic_state WHERE id = 1")?;
    c.execute_batch(include_str!("semantic_views.sql"))?;
    stage("group_and_views", started);
    Ok(())
}

fn table_exists(c: &duckdb::Connection, table: &str) -> Result<bool> {
    let exists: i64 = c.query_row(
        "SELECT count(*) FROM information_schema.tables WHERE table_schema = 'main' AND table_name = ?",
        params![table],
        |r| r.get(0),
    )?;
    Ok(exists > 0)
}

fn column_exists(c: &duckdb::Connection, table: &str, column: &str) -> Result<bool> {
    let exists: i64 = c.query_row(
        "SELECT count(*) FROM information_schema.columns WHERE table_schema = 'main' AND table_name = ? AND column_name = ?",
        params![table, column],
        |r| r.get(0),
    )?;
    Ok(exists > 0)
}

fn materialize_optional_fact_tables(c: &duckdb::Connection) -> Result<()> {
    if table_exists(c, "message_facts")? {
        c.execute_batch(
            "INSERT INTO dendro_semantic_message_mix
             SELECT harness,
                    native_session,
                    role,
                    direction,
                    modalities AS modality,
                    COUNT(*)::BIGINT AS message_count,
                    list(source_key ORDER BY source_key) AS source_keys
             FROM message_facts
             GROUP BY harness, native_session, role, direction, modalities
             ORDER BY harness, native_session NULLS LAST, role, direction, modality",
        )?;
    }
    if table_exists(c, "session_edges")? {
        c.execute_batch(
            "INSERT INTO dendro_semantic_subagent_lineage
             SELECT harness,
                    parent_session,
                    COUNT(DISTINCT CASE WHEN child_session <> '' THEN child_session END)::BIGINT AS child_count,
                    COUNT(*)::BIGINT AS edge_count,
                    (COUNT(*) FILTER (WHERE status = 'started'))::BIGINT AS started_count,
                    (COUNT(*) FILTER (WHERE status = 'finished'))::BIGINT AS finished_count,
                    (COUNT(*) FILTER (WHERE status = 'failed'))::BIGINT AS failed_count,
                    (COUNT(*) FILTER (WHERE status = 'unknown'))::BIGINT AS unknown_count,
                    COALESCE(list(DISTINCT child_session ORDER BY child_session) FILTER (WHERE child_session <> ''), []) AS child_sessions,
                    list(DISTINCT relation ORDER BY relation) AS relations,
                    list(source_key ORDER BY source_key) AS source_keys
             FROM session_edges
             GROUP BY harness, parent_session
             ORDER BY harness, parent_session",
        )?;
    }
    if table_exists(c, "security_facts")? {
        c.execute_batch(
            "INSERT INTO dendro_semantic_security_posture
             SELECT harness,
                    kind,
                    value,
                    COUNT(*)::BIGINT AS observation_count,
                    COUNT(DISTINCT native_session)::BIGINT AS session_count,
                    list(source_key ORDER BY source_key) AS source_keys
             FROM security_facts
             WHERE (kind = 'permission_mode' AND value IN ('default', 'auto', 'acceptEdits', 'plan', 'bypassPermissions', 'other'))
                OR (kind = 'permission_decision' AND value IN ('allow', 'deny', 'other'))
                OR (kind = 'sandbox_policy' AND value IN ('danger-full-access', 'read-only', 'workspace-write', 'true', 'false', 'other'))
                OR (kind = 'approval_policy' AND value IN ('never', 'on-request', 'other'))
                OR (kind = 'permission_profile' AND value = 'other')
                OR (kind = 'approved_prefix_count' AND regexp_full_match(value, '[0-9]+'))
                OR (kind = 'sandbox_override_requested' AND value = 'true')
                OR (kind = 'dangerous_command' AND value IN (
                    'recursive_delete', 'pipe_to_shell', 'force_push', 'force_push_with_lease',
                    'chmod_world_writable', 'sudo', 'disk_write_device', 'credential_file_access',
                    'network_exfil_pattern'
                ))
             GROUP BY harness, kind, value
             ORDER BY harness, kind, value",
        )?;
    }
    if table_exists(c, "process_identities")? {
        let has_attribution = column_exists(c, "process_identities", "attribution")?;
        let has_samples = table_exists(c, "process_samples")?;
        let sql = match (has_attribution, has_samples) {
            (true, true) => {
                "INSERT INTO dendro_semantic_process_resources
                 WITH trees AS (
                     SELECT root_key,
                            COALESCE(MAX(harness) FILTER (WHERE identity_key = root_key AND harness <> ''), MAX(harness) FILTER (WHERE harness <> '')) AS harness,
                            COALESCE(MAX(attribution) FILTER (WHERE identity_key = root_key AND attribution IN ('launched', 'discovered')), MAX(attribution) FILTER (WHERE attribution IN ('launched', 'discovered'))) AS attribution,
                            COUNT(DISTINCT identity_key)::BIGINT AS process_count,
                            list(DISTINCT identity_key ORDER BY identity_key) AS source_keys
                     FROM process_identities
                     GROUP BY root_key
                 ),
                 samples_by_unit AS (
                     SELECT pi.root_key,
                            ps.rss_unit,
                            COUNT(*)::BIGINT AS sample_count,
                            CASE WHEN COUNT(*) = COUNT(ps.rss_bytes) THEN MAX(ps.rss_bytes)::BIGINT ELSE NULL END AS max_rss_bytes
                     FROM process_identities pi
                     JOIN process_samples ps ON ps.identity_key = pi.identity_key
                     GROUP BY pi.root_key, ps.rss_unit
                 )
                 SELECT t.root_key, t.harness, t.attribution, s.rss_unit,
                        t.process_count, s.max_rss_bytes,
                        COALESCE(s.sample_count, 0)::BIGINT AS sample_count,
                        t.source_keys
                 FROM trees t
                 LEFT JOIN samples_by_unit s ON s.root_key = t.root_key
                 ORDER BY t.root_key, s.rss_unit NULLS LAST"
            }
            (false, true) => {
                "INSERT INTO dendro_semantic_process_resources
                 WITH trees AS (
                     SELECT root_key,
                            COALESCE(MAX(harness) FILTER (WHERE identity_key = root_key AND harness <> ''), MAX(harness) FILTER (WHERE harness <> '')) AS harness,
                            CAST(NULL AS VARCHAR) AS attribution,
                            COUNT(DISTINCT identity_key)::BIGINT AS process_count,
                            list(DISTINCT identity_key ORDER BY identity_key) AS source_keys
                     FROM process_identities
                     GROUP BY root_key
                 ),
                 samples_by_unit AS (
                     SELECT pi.root_key,
                            ps.rss_unit,
                            COUNT(*)::BIGINT AS sample_count,
                            CASE WHEN COUNT(*) = COUNT(ps.rss_bytes) THEN MAX(ps.rss_bytes)::BIGINT ELSE NULL END AS max_rss_bytes
                     FROM process_identities pi
                     JOIN process_samples ps ON ps.identity_key = pi.identity_key
                     GROUP BY pi.root_key, ps.rss_unit
                 )
                 SELECT t.root_key, t.harness, t.attribution, s.rss_unit,
                        t.process_count, s.max_rss_bytes,
                        COALESCE(s.sample_count, 0)::BIGINT AS sample_count,
                        t.source_keys
                 FROM trees t
                 LEFT JOIN samples_by_unit s ON s.root_key = t.root_key
                 ORDER BY t.root_key, s.rss_unit NULLS LAST"
            }
            (true, false) => {
                "INSERT INTO dendro_semantic_process_resources
                 SELECT root_key,
                        COALESCE(MAX(harness) FILTER (WHERE identity_key = root_key AND harness <> ''), MAX(harness) FILTER (WHERE harness <> '')) AS harness,
                        COALESCE(MAX(attribution) FILTER (WHERE identity_key = root_key AND attribution IN ('launched', 'discovered')), MAX(attribution) FILTER (WHERE attribution IN ('launched', 'discovered'))) AS attribution,
                        CAST(NULL AS VARCHAR) AS rss_unit,
                        COUNT(DISTINCT identity_key)::BIGINT AS process_count,
                        CAST(NULL AS BIGINT) AS max_rss_bytes,
                        0::BIGINT AS sample_count,
                        list(DISTINCT identity_key ORDER BY identity_key) AS source_keys
                 FROM process_identities
                 GROUP BY root_key
                 ORDER BY root_key"
            }
            (false, false) => {
                "INSERT INTO dendro_semantic_process_resources
                 SELECT root_key,
                        COALESCE(MAX(harness) FILTER (WHERE identity_key = root_key AND harness <> ''), MAX(harness) FILTER (WHERE harness <> '')) AS harness,
                        CAST(NULL AS VARCHAR) AS attribution,
                        CAST(NULL AS VARCHAR) AS rss_unit,
                        COUNT(DISTINCT identity_key)::BIGINT AS process_count,
                        CAST(NULL AS BIGINT) AS max_rss_bytes,
                        0::BIGINT AS sample_count,
                        list(DISTINCT identity_key ORDER BY identity_key) AS source_keys
                 FROM process_identities
                 GROUP BY root_key
                 ORDER BY root_key"
            }
        };
        c.execute_batch(sql)?;
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
pub enum Intent {
    Compare,
    Rank,
    Explain,
    Verify,
    Trace,
    HonestLimits,
}
/// Bounded English patterns; unknown requests never become generated SQL.
pub fn parse_intent(question: &str) -> Option<Intent> {
    let q = question
        .trim()
        .trim_end_matches(['?', '.'])
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    match q.as_str() {
        "compare handoff efficiency"
        | "compare rework"
        | "compare token burn"
        | "compare idle"
        | "compare yield"
        | "compare"
        | "compare baseline" => Some(Intent::Compare),
        "which seat had the worst yield"
        | "which seat had worst yield"
        | "which seat had the highest idle time"
        | "which seat has highest idle"
        | "which seat has the highest idle time"
        | "rank idle"
        | "rank yield"
        | "idle"
        | "idle time"
        | "highest idle"
        | "highest idle time"
        | "most idle"
        | "most idle seat"
        | "where is time being wasted"
        | "worst yield"
        | "yield"
        | "token burn"
        | "where do tokens go"
        | "where are tokens burned" => Some(Intent::Rank),
        "why did build rework go up"
        | "explain rework"
        | "explain handoffs"
        | "explain handoff friction"
        | "rework"
        | "handoffs"
        | "handoff friction"
        | "why is there rework" => Some(Intent::Explain),
        "what changed" | "verify changes" | "verify topology change" | "verify" => {
            Some(Intent::Verify)
        }
        "trace idle"
        | "trace rework"
        | "trace handoffs"
        | "trace tokens"
        | "trace yield"
        | "where does idle come from"
        | "provenance"
        | "source keys" => Some(Intent::Trace),
        "honest limits" | "what are the honest limits" | "limits" | "known limits" => {
            Some(Intent::HonestLimits)
        }
        _ => {
            if let Some(rest) = q.strip_prefix("where does ") {
                if let Some(num_str) = rest.strip_suffix("% idle come from") {
                    if let Ok(amount) = num_str.parse::<f64>() {
                        if amount.is_finite() && (0.0..=100.0).contains(&amount) {
                            return Some(Intent::Trace);
                        }
                    }
                }
            }
            score_natural_intent(&q)
        }
    }
}

fn score_natural_intent(q: &str) -> Option<Intent> {
    let words: Vec<&str> = q
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-')
        .filter(|w| !w.is_empty())
        .collect();
    if words.is_empty() {
        return None;
    }
    // Refuse queries with temporal/mission/seat qualifiers or unsupported vocabulary.
    const UNSUPPORTED_QUALIFIERS: &[&str] = &[
        "mission",
        "week",
        "month",
        "year",
        "yesterday",
        "today",
        "since",
        "removed",
        "drop",
        "table",
        "select",
        "insert",
        "delete",
        "update",
        "secrets",
        "secret",
        "launch",
        "speed",
        "forecast",
        "predict",
        "roi",
        "savings",
    ];
    if words
        .iter()
        .any(|w| UNSUPPORTED_QUALIFIERS.contains(w) || w.contains('@'))
    {
        return None;
    }
    let has = |targets: &[&str]| words.iter().any(|w| targets.contains(w));
    if has(&["honest", "limit", "limits", "limitation", "limitations"]) {
        return Some(Intent::HonestLimits);
    }
    if has(&["trace", "provenance"])
        && has(&[
            "idle", "rework", "handoff", "handoffs", "token", "tokens", "yield", "source", "keys",
        ])
    {
        return Some(Intent::Trace);
    }
    if has(&["compare", "comparison", "vs", "versus"])
        && has(&[
            "handoff",
            "handoffs",
            "efficiency",
            "rework",
            "token",
            "tokens",
            "burn",
            "idle",
            "yield",
            "baseline",
        ])
    {
        return Some(Intent::Compare);
    }
    if has(&["verify", "changed"]) && words.len() <= 5 {
        return Some(Intent::Verify);
    }
    if has(&["explain", "why", "bouncing", "friction", "loops", "loop"])
        && has(&[
            "rework", "handoff", "handoffs", "friction", "bounce", "bouncing",
        ])
    {
        return Some(Intent::Explain);
    }
    if has(&[
        "rank", "ranking", "which", "who", "top", "worst", "highest", "most", "show", "list",
        "where",
    ]) && has(&[
        "idle", "wasted", "yield", "token", "tokens", "burn", "burned", "burning",
    ]) {
        return Some(Intent::Rank);
    }
    None
}

/// Rank supported intent categories by keyword overlap with `question` for autocomplete / refusal hints.
pub fn suggest_closest_intents(question: &str) -> Vec<Value> {
    let q = question.to_ascii_lowercase();
    let words: Vec<&str> = q
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| w.len() >= 3)
        .collect();
    let all = supported_intents()["intents"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if words.is_empty() {
        return all;
    }
    let mut scored: Vec<(usize, Value)> = all
        .into_iter()
        .map(|item| {
            let text = item.to_string().to_ascii_lowercase();
            let hits = words.iter().filter(|w| text.contains(*w)).count();
            (hits, item)
        })
        .collect();
    scored.sort_by_key(|a| std::cmp::Reverse(a.0));
    scored.into_iter().map(|(_, item)| item).collect()
}

/// Count projections added in v0.8.0. They are catalog metadata, not ask phrases.
/// Conversational routing stays on the six `intents` entries.
fn v08_semantic_views() -> Value {
    json!([
        {
            "view": "v_message_mix",
            "domain": "harness",
            "aggregation": "counts and group-by",
            "window": "all-time",
            "limit": "message metadata counts only; never message text"
        },
        {
            "view": "v_subagent_lineage",
            "domain": "coordination",
            "insight": "I09",
            "aggregation": "counts and group-by",
            "window": "all-time",
            "limit": "parent-child session lineage counts; depth and cycle verdicts stay on I09"
        },
        {
            "view": "v_security_posture",
            "domain": "security",
            "aggregation": "counts and group-by",
            "window": "all-time",
            "limit": "closed-vocabulary security observations; never command text or authority verdicts"
        },
        {
            "view": "v_process_resources",
            "domain": "environment",
            "insight": "I14",
            "aggregation": "counts, max, and group-by",
            "window": "all-time",
            "limit": "process tree and RSS sample summaries; I14 environment.resources is CPU deltas and max sampled RSS; cost stays on I04"
        }
    ])
}

/// Catalog of supported conversational intents and copy-pasteable example phrases.
///
/// `intents` is the six ask phrases. `semantic_views` names the four v0.8.0 count
/// projections. [`answer_physical`] does not route questions onto those views.
pub fn supported_intents() -> Value {
    json!({
        "intents": [
            {
                "intent": "Rank (Idle)",
                "examples": [
                    "Which seat had the highest idle time?",
                    "highest idle",
                    "where is time being wasted?"
                ],
                "view": "v_seat_idle_ranking"
            },
            {
                "intent": "Rank (Yield / Token Burn)",
                "examples": [
                    "Which seat had the worst yield?",
                    "worst yield",
                    "where do tokens go?"
                ],
                "view": "v_seat_idle_ranking"
            },
            {
                "intent": "Explain (Rework & Handoffs)",
                "examples": [
                    "Why did build rework go up?",
                    "explain rework",
                    "explain handoffs"
                ],
                "view": "v_handoff_rework_analysis"
            },
            {
                "intent": "Compare (Before/After Baseline)",
                "examples": [
                    "Compare handoff efficiency",
                    "compare rework",
                    "compare token burn"
                ],
                "requires": "--baseline <report.json>"
            },
            {
                "intent": "Verify (Topology Change)",
                "examples": [
                    "What changed?",
                    "verify topology change"
                ],
                "requires": "--baseline <report.json>"
            },
            {
                "intent": "Trace & HonestLimits",
                "examples": [
                    "Where does 41% idle come from?",
                    "trace tokens",
                    "Honest limits"
                ],
                "view": "dendro_semantic_provenance"
            }
        ],
        "semantic_views": v08_semantic_views()
    })
}

/// Inspect the currently materialized semantic scope and whether it matches the live signature.
pub fn cached_scope(db: &Path) -> Option<(Option<String>, String, String, bool)> {
    let request = crate::presentation::ReadRequest::open(db).ok()?;
    cached_scope_physical(request.database())
}

/// Physical read: caller retains selection custody.
pub fn cached_scope_physical(db: &Path) -> Option<(Option<String>, String, String, bool)> {
    let s = AnalyticsStore::open_read_only(db).ok()?;
    let (report, expected, guard_fresh): (String, String, bool) = s
        .connection()
        .query_row(
            "SELECT report::VARCHAR, signature, fresh FROM dendro_semantic_snapshot, dendro_semantic_guard",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .ok()?;
    let sig_fresh = guard_fresh && signature(&s).ok().as_deref() == Some(expected.as_str());
    let report: Value = serde_json::from_str(&report).ok()?;
    let p = &report["payload"];
    let rig = p["rig"].as_str().map(str::to_owned);
    let from = p["from"].as_str()?.to_owned();
    let to = p["to"].as_str()?.to_owned();
    Some((rig, from, to, sig_fresh))
}

fn insufficient(reason: &str) -> Value {
    json!({"status":"insufficient_data", "message":format!("Insufficient data: {reason}")})
}
/// Scoped, local read-only answers. Baseline must be a canonical report with the same
/// definition version and explicitly recorded scope; causation is never inferred from a before/after observation.
pub fn answer(db: &Path, question: &str, baseline: Option<&Value>) -> Result<Value> {
    let request = crate::presentation::ReadRequest::open(db)?;
    answer_physical(request.database(), question, baseline)
}

/// Physical read: caller retains selection custody.
///
/// Rank reads `v_seat_idle_ranking` and Explain reads the handoff payload backed by
/// `v_handoff_rework_analysis`. The v0.8.0 views `v_message_mix`, `v_subagent_lineage`,
/// `v_security_posture`, and `v_process_resources` are count projections listed in
/// [`supported_intents`] `semantic_views`; they are not conversational phrases.
pub fn answer_physical(db: &Path, question: &str, baseline: Option<&Value>) -> Result<Value> {
    let s = AnalyticsStore::open_read_only(db)?;
    s.connection().execute_batch("BEGIN TRANSACTION")?;
    let report: String = match s.connection().query_row(
        "SELECT report::VARCHAR FROM dendro_semantic_snapshot, dendro_semantic_guard WHERE fresh",
        [],
        |r| r.get(0),
    ) {
        Ok(report) => report,
        Err(_) => {
            return Ok(insufficient(
                "semantic snapshot unavailable or stale; refresh required",
            ))
        }
    };
    let expected: String =
        s.connection()
            .query_row("SELECT signature FROM dendro_semantic_snapshot", [], |r| {
                r.get(0)
            })?;
    if signature(&s)? != expected {
        return Ok(insufficient("semantic snapshot is stale; refresh required"));
    }
    let report: Value = serde_json::from_str(&report)?;
    let p = &report["payload"];
    let scope = json!({"rig":p["rig"],"from":p["from"],"to":p["to"],"definition_version":report["definition_version"]});
    let refusal = |reason: &str| {
        let mut v = insufficient(reason);
        v["scope"] = scope.clone();
        v
    };
    let Some(intent) = parse_intent(question) else {
        let mut v = refusal("unsupported intent, metric, or question qualifier; select an exact report scope and use a supported phrase");
        v["supported_examples"] = Value::Array(suggest_closest_intents(question));
        return Ok(v);
    };
    let q = question.to_ascii_lowercase();
    let mut response = match intent {
        Intent::HonestLimits => refusal("observational facts cannot establish causation, subjective quality, or unsupported metrics"),
        Intent::Compare | Intent::Verify => {
            let Some(b) = baseline else {return Ok(refusal("no comparison baseline"));};
            if b["metric"] != "decisions" || b["definition_version"] != report["definition_version"] {return Ok(refusal("baseline is not a canonical decisions report with a matching formula"));}
            if b["payload"]["kind"] != "decisions"
                || b["payload"]["from"].as_str().and_then(|s|chrono::DateTime::parse_from_rfc3339(s).ok()).is_none()
                || b["payload"]["to"].as_str().and_then(|s|chrono::DateTime::parse_from_rfc3339(s).ok()).is_none()
                || b["payload"]["handoff_friction"]["thresholds"] != p["handoff_friction"]["thresholds"]
                || b["payload"]["loop_thresholds"] != p["loop_thresholds"] {
                return Ok(refusal("baseline scope or formula options are unavailable or incompatible"));
            }
            if p["usage_rows"].as_u64().unwrap_or(0) == 0 || b["payload"]["usage_rows"].as_u64().unwrap_or(0)==0 {return Ok(refusal("no observed usage in one comparison period"));}
            json!({"status":"ok","current":p,"baseline":b["payload"],"limit":"Observed differences do not prove a topology change caused them."})
        },
        Intent::Rank => {
            let q = question.to_ascii_lowercase();
            let is_idle = q.contains("idle") || q.contains("wasted");
            let is_yield = q.contains("yield") || q.contains("token") || q.contains("burn");
            let sql = if is_idle {"SELECT row_json::VARCHAR FROM v_seat_idle_ranking ORDER BY idle_with_work_secs DESC, seat_id"}
                else if is_yield {"SELECT m1::VARCHAR FROM v_seat_idle_ranking WHERE try_cast(json_extract(m1,'$.tokens_per_closed_item') AS DOUBLE) IS NOT NULL ORDER BY try_cast(json_extract(m1,'$.tokens_per_closed_item') AS DOUBLE) DESC NULLS LAST, seat_id"}
                else {return Ok(refusal("rank supports idle and canonical yield only"));};
            let mut stmt=s.connection().prepare(sql)?;
            let rows=stmt.query_map([],|r|r.get::<_,Option<String>>(0))?.collect::<duckdb::Result<Vec<_>>>()?;
            let rows=rows.into_iter().flatten().map(|v|serde_json::from_str::<Value>(&v)).collect::<std::result::Result<Vec<_>,_>>()?;
            if rows.is_empty() {refusal("no observed seats")} else {json!({"status":"ok","rows":rows,"ordering":if is_idle {"idle_with_work_secs descending"} else {"tokens_per_closed_item descending: worst observed burn per completed item"}})}
        },
        Intent::Explain => {
            if !q.contains("rework") && !q.contains("handoff") {return Ok(refusal("explain supports recorded rework and handoff evidence only"));}
            if p["handoff_edges"].as_array().is_none_or(|a|a.is_empty()) { refusal("no recorded handoff observations") }
            else { json!({"status":"ok","handoff_friction":p["handoff_friction"],"m3_chains":p["m3_chains"],"limit":"Correlated transitions are evidence of recorded rework, not proof of causation."}) }
        },
        Intent::Trace => {let mut keys=BTreeSet::new();census(p,&mut keys); json!({"status":"ok","report":report,"source_keys":keys,"trace":"dendro trace --metric-id <canonical metric> with identical --from/--to/--rig"})},
    };
    response["scope"] = scope;
    Ok(response)
}

fn rank_from_payload(payload: &Value, question: &str) -> std::result::Result<Value, &'static str> {
    let is_idle = question.contains("idle") || question.contains("wasted");
    let is_yield =
        question.contains("yield") || question.contains("token") || question.contains("burn");
    if is_idle {
        let mut rows = payload["activity"].as_array().cloned().unwrap_or_default();
        rows.sort_by(|a, b| {
            let idle = |row: &Value| {
                row["idle_with_work_secs"]
                    .as_f64()
                    .unwrap_or(f64::NEG_INFINITY)
            };
            idle(b)
                .partial_cmp(&idle(a))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| {
                    a["seat"]
                        .as_str()
                        .unwrap_or("")
                        .cmp(b["seat"].as_str().unwrap_or(""))
                })
        });
        if rows.is_empty() {
            Err("no observed seats")
        } else {
            Ok(json!({
                "status": "ok",
                "rows": rows,
                "ordering": "idle_with_work_secs descending"
            }))
        }
    } else if is_yield {
        let mut rows: Vec<Value> = payload["m1_by_seat"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|row| row["tokens_per_closed_item"].as_f64().is_some())
            .collect();
        rows.sort_by(|a, b| {
            let yield_of = |row: &Value| {
                row["tokens_per_closed_item"]
                    .as_f64()
                    .unwrap_or(f64::NEG_INFINITY)
            };
            yield_of(b)
                .partial_cmp(&yield_of(a))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| {
                    a["seat"]
                        .as_str()
                        .unwrap_or("")
                        .cmp(b["seat"].as_str().unwrap_or(""))
                })
        });
        if rows.is_empty() {
            Err("no observed seats")
        } else {
            Ok(json!({
                "status": "ok",
                "rows": rows,
                "ordering": "tokens_per_closed_item descending: worst observed burn per completed item"
            }))
        }
    } else {
        Err("rank supports idle and canonical yield only")
    }
}

/// Evaluate a supported question against an in-memory canonical report.
/// Used when the DuckDB semantic cache is stale or locked; no database is opened.
pub fn answer_from_report(report: &Value, question: &str, baseline: Option<&Value>) -> Value {
    let payload = &report["payload"];
    let scope = json!({
        "rig": payload["rig"],
        "from": payload["from"],
        "to": payload["to"],
        "definition_version": report["definition_version"]
    });
    let refusal = |reason: &str| {
        let mut value = insufficient(reason);
        value["scope"] = scope.clone();
        value
    };
    let Some(intent) = parse_intent(question) else {
        let mut value = refusal(
            "unsupported intent, metric, or question qualifier; select an exact report scope and use a supported phrase",
        );
        value["supported_examples"] = Value::Array(suggest_closest_intents(question));
        return value;
    };
    let q = question.to_ascii_lowercase();
    let mut response = match intent {
        Intent::HonestLimits => {
            return refusal(
                "observational facts cannot establish causation, subjective quality, or unsupported metrics",
            )
        }
        Intent::Compare | Intent::Verify => {
            let Some(baseline) = baseline else {
                return refusal("no comparison baseline");
            };
            if baseline["metric"] != "decisions"
                || baseline["definition_version"] != report["definition_version"]
            {
                return refusal(
                    "baseline is not a canonical decisions report with a matching formula",
                );
            }
            if baseline["payload"]["kind"] != "decisions"
                || baseline["payload"]["from"]
                    .as_str()
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    .is_none()
                || baseline["payload"]["to"]
                    .as_str()
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    .is_none()
                || baseline["payload"]["handoff_friction"]["thresholds"]
                    != payload["handoff_friction"]["thresholds"]
                || baseline["payload"]["loop_thresholds"] != payload["loop_thresholds"]
            {
                return refusal("baseline scope or formula options are unavailable or incompatible");
            }
            if payload["usage_rows"].as_u64().unwrap_or(0) == 0
                || baseline["payload"]["usage_rows"].as_u64().unwrap_or(0) == 0
            {
                return refusal("no observed usage in one comparison period");
            }
            json!({
                "status": "ok",
                "current": payload,
                "baseline": baseline["payload"],
                "limit": "Observed differences do not prove a topology change caused them."
            })
        }
        Intent::Rank => match rank_from_payload(payload, &q) {
            Ok(value) => value,
            Err(reason) => return refusal(reason),
        },
        Intent::Explain => {
            if !q.contains("rework") && !q.contains("handoff") {
                return refusal("explain supports recorded rework and handoff evidence only");
            }
            if payload["handoff_edges"]
                .as_array()
                .is_none_or(|edges| edges.is_empty())
            {
                return refusal("no recorded handoff observations");
            }
            json!({
                "status": "ok",
                "handoff_friction": payload["handoff_friction"],
                "m3_chains": payload["m3_chains"],
                "limit": "Correlated transitions are evidence of recorded rework, not proof of causation."
            })
        }
        Intent::Trace => {
            let mut keys = BTreeSet::new();
            census(payload, &mut keys);
            json!({
                "status": "ok",
                "report": report,
                "source_keys": keys,
                "trace": "dendro trace --metric-id <canonical metric> with identical --from/--to/--rig"
            })
        }
    };
    response["scope"] = scope;
    response
}

/// Creates a new workspace only. Never overwrites an existing directory or follows its symlink.
pub fn init_workspace(destination: &Path, db: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        let _ = (destination, db);
        bail!("nao_exact_workspace_path_unproved: Windows requires init_workspace_with_receipt (or dendro init-nao), which returns the actual unique private workspace path");
    }
    #[cfg(not(windows))]
    init_workspace_with_receipt(destination, db).map(|_| ())
}

/// Create an external-consumer workspace and return its explicit snapshot provenance.
pub fn init_workspace_with_receipt(destination: &Path, db: &Path) -> Result<Value> {
    crate::pipeline::refuse_secret_path(destination)?;
    if destination
        .components()
        .any(|c| matches!(c, Component::ParentDir))
    {
        bail!("workspace parent traversal refused");
    }
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    for ancestor in parent.ancestors() {
        if !ancestor.as_os_str().is_empty()
            && fs::symlink_metadata(ancestor)?.file_type().is_symlink()
        {
            bail!("workspace symlink ancestor refused");
        }
    }
    let canonical_parent = fs::canonicalize(parent)?;
    let name = destination
        .file_name()
        .ok_or_else(|| anyhow!("workspace must name a new directory"))?;
    let requested_target = canonical_parent.join(name);
    match fs::symlink_metadata(&requested_target) {
        Ok(_) => bail!("workspace must name a new directory"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let request = crate::presentation::ReadRequest::open(db)?;
    #[cfg(any(windows, target_os = "linux"))]
    let request = if request.is_legacy() {
        let selected = crate::selection_binding::adopt_legacy(
            request.logical(),
            &crate::platform_v1::StorageRootChoice::Explicit(
                request
                    .logical()
                    .parent()
                    .ok_or_else(|| anyhow!("legacy database has no parent"))?
                    .to_path_buf(),
            ),
        )?;
        crate::presentation::ReadRequest::from_selected(selected)
    } else {
        request
    };
    if !request.initialized() {
        bail!("nao_uninitialized: initialize and materialize the database first");
    }
    #[cfg(windows)]
    let parent_root = crate::platform_v1::open_existing_private_root(&canonical_parent)?;
    #[cfg(windows)]
    let workspace_root =
        crate::platform_v1::create_unique_private_directory(&parent_root, "nao-workspace-")?;
    #[cfg(windows)]
    let target = workspace_root.path().to_path_buf();
    #[cfg(not(windows))]
    let target = requested_target.clone();
    #[cfg(not(windows))]
    crate::lifecycle::create_private_dir(&target).context("workspace must be a new directory")?;
    let result = (|| -> Result<Value> {
        #[cfg(any(windows, target_os = "linux"))]
        let database = {
            let snapshot = target.join("analytics.duckdb");
            crate::lifecycle::private_copy(request.database(), &snapshot, &crate::RUN_CANCEL)?;
            // Verify the copied DB opens read-only before exposing a consumer configuration.
            let _store = AnalyticsStore::open_read_only(&snapshot)?;
            snapshot
        };
        #[cfg(not(any(windows, target_os = "linux")))]
        let database = fs::canonicalize(request.database())?;
        crate::pipeline::refuse_secret_path(&database)?;
        let config = include_str!("../../../templates/nao/nao_config.yaml")
            .replace("\r\n", "\n")
            .replace("\"dendro.duckdb\"", &serde_json::to_string(&database)?);
        #[cfg(any(windows, target_os = "linux"))]
        let config = {
            format!("# Dedicated private snapshot of one selected version; does not follow later publications.\n# Re-run init-nao into a new workspace to refresh. Local-file connections are read-only in the pinned Nao DuckDBConfig.connect.\n{config}").replace("    type: duckdb\n", "    type: duckdb\n    read_only: true\n")
        };
        write_workspace_file(&target.join("nao_config.yaml"), config.as_bytes())?;
        write_workspace_file(
            &target.join("semantic_models.yaml"),
            include_bytes!("../../../templates/nao/semantic_models.yaml"),
        )?;
        let rules = include_str!("../../../templates/nao/prompts.md").to_owned();
        #[cfg(windows)]
        let rules = format!("{rules}\nThis workspace queries a dedicated private, version-scoped database snapshot in read-only mode. It does not follow later Dendro publications. To refresh, run dendro init-nao into a new workspace; never replace the snapshot in place.\n");
        write_workspace_file(&target.join("RULES.md"), rules.as_bytes())?;
        #[cfg(not(windows))]
        {
            crate::lifecycle::create_private_dir(&target.join("semantics"))?;
            write_workspace_file(&target.join("semantics/dendro.md"), rules.as_bytes())?;
        }
        write_workspace_file(&target.join("prompts.md"), rules.as_bytes())?;
        #[cfg(windows)]
        write_workspace_file(&target.join("semantics-dendro.md"), rules.as_bytes())?;
        let metadata = json!({"path":database,"read_only":true});
        #[cfg(target_os = "linux")]
        let metadata = {
            let mut metadata = metadata;
            metadata["workspace"] = json!(target);
            metadata["scope"] = json!("immutable_version_snapshot");
            metadata["analytics_selection_sha256"] = json!(hex::encode(
                request
                    .selected()
                    .ok_or_else(|| anyhow!("nao_selected_snapshot_required"))?
                    .selection()
                    .sha256
            ));
            metadata["refresh"] =
                json!("run dendro init-nao into a new workspace to refresh this snapshot");
            metadata
        };
        #[cfg(windows)]
        let metadata = {
            let mut metadata = metadata;
            let selected = request
                .selected()
                .ok_or_else(|| anyhow!("nao_selected_snapshot_required"))?;
            metadata["scope"] = json!("immutable_version_snapshot");
            metadata["workspace"] = json!(target);
            metadata["context_files"] = json!([
                "RULES.md",
                "prompts.md",
                "semantic_models.yaml",
                "semantics-dendro.md"
            ]);
            metadata["integration_status"] = json!("snapshot_created_runtime_integration_pending");
            metadata["semantic_context_status"] = json!("unsupported_exact_semantics_directory; intended semantics/dendro.md artifact preserved as top-level semantics-dendro.md; automatic Nao consumption of this alternate path is not verified");
            metadata["limitations"] = json!(["Windows admitted native API cannot create the exact semantics/ child directory; generated semantic prose is preserved in semantics-dendro.md", "Nao runtime consumption and full T19 acceptance are pending; this is a snapshot scaffold, not a ready integration"]);
            metadata["requested_workspace"] = json!(requested_target);
            metadata["workspace_allocation"] = json!(
                "unique_private_sibling; use the returned workspace path, not requested_workspace"
            );
            metadata["analytics_selection_sha256"] =
                json!(hex::encode(selected.selection().sha256));
            metadata["refresh"] = json!("run dendro init-nao into a new workspace; this snapshot does not follow later publications");
            metadata
        };
        write_workspace_file(
            &target.join("database.json"),
            &serde_json::to_vec_pretty(&metadata)?,
        )?;
        Ok(metadata)
    })();
    #[cfg(windows)]
    drop(workspace_root);
    if result.is_err() {
        let _ = fs::remove_dir_all(&target);
    }
    result
}

fn write_workspace_file(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let mut file = crate::lifecycle::create_private_file(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ActivityFact, IdentityFact, QueueFact, QueueItemMeta, SeatBound, SkillFact, ToolCategory,
        WarningNote,
    };
    #[test]
    fn large_report_extension_materializes_without_parent_json_fanout() -> Result<()> {
        struct Directory(std::path::PathBuf);
        impl Drop for Directory {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let directory = Directory(std::env::current_dir()?.join("target/semantic-tests").join(
            format!("large-{}-{}", std::process::id(), rand::random::<u64>()),
        ));
        std::fs::create_dir_all(&directory.0)?;
        let db = directory.0.join("analytics.duckdb");
        {
            let store = AnalyticsStore::open_path(&db, "large")?;
            store.connection().execute_batch("INSERT INTO usage_facts (source_key,harness,native_session,ts,seq,input_uncached,output_tokens,cache_read,cache_write,reasoning,generation,source_id)
          SELECT 'source:0:' || i,'claude','session',CAST('2026-10-01 00:00:02' AS TIMESTAMP)+i*INTERVAL '1 second',i,11,7,5,3,0,0,'source' FROM range(1,5501) t(i)")?;
        }
        let mut report = report_json(
            &db,
            "decisions",
            None,
            "2026-10-01T00:00:00Z",
            "2026-10-02T00:00:00Z",
        )?;
        report["payload"]["regression_extension"] = json!("x".repeat(16 * 1024 * 1024));
        let store = AnalyticsStore::open_existing(&db, "large")?;
        let c = store.connection();
        c.execute_batch("SET memory_limit='512MB'; SET threads=1; BEGIN TRANSACTION")?;
        materialize(&store, &report, &signature(&store)?)?;
        c.execute_batch("COMMIT")?;
        let count: i64 = c.query_row("SELECT count(*) FROM dendro_semantic_usage", [], |r| {
            r.get(0)
        })?;
        assert_eq!(count, 5500);
        let conserved: i64 = c.query_row(
            "SELECT sum(input_uncached+output+cache_read+cache_write) FROM v_mission_token_burn",
            [],
            |r| r.get(0),
        )?;
        assert_eq!(conserved, 5500 * 26);
        let exact: String = c.query_row(
            "SELECT row_json::VARCHAR FROM v_executive_governance",
            [],
            |r| r.get(0),
        )?;
        assert_eq!(serde_json::from_str::<Value>(&exact)?, report["payload"]);
        Ok(())
    }
    #[test]
    fn supported_writers_atomically_invalidate_derived_state() -> Result<()> {
        let s = AnalyticsStore::open_in_memory("test")?;
        let c = s.connection();
        macro_rules! dirty {
            ($operation:expr) => {{
                c.execute_batch("UPDATE dendro_semantic_state SET dirty=FALSE")?;
                $operation?;
                let dirty: bool =
                    c.query_row("SELECT dirty FROM dendro_semantic_state", [], |r| r.get(0))?;
                assert!(dirty, "{}", stringify!($operation));
            }};
        }
        let ts = chrono::DateTime::parse_from_rfc3339("2026-10-01T00:00:00Z")?
            .with_timezone(&chrono::Utc);
        dirty!(s.insert_identity(&IdentityFact {
            source_key: "id".into(),
            harness: "claude".into(),
            native_session: "native".into(),
            rig: "rig".into(),
            seat: "seat".into(),
            ts: Some(ts)
        }));
        dirty!(s.insert_activity(&ActivityFact {
            source_key: "activity".into(),
            seat: "seat".into(),
            ts,
            state: "running".into(),
            seq: 1
        }));
        dirty!(s.insert_queue(&QueueFact {
            source_key: "queue".into(),
            transition_id: 1,
            qitem_id: "item".into(),
            ts,
            state: "pending".into(),
            source_seat: "from".into(),
            dest_seat: "seat".into(),
            closure_reason: None,
            closure_target: None,
            chain_id: None,
            missions: vec![]
        }));
        dirty!(s.record_queue_item_meta(&QueueItemMeta {
            qitem_id: "item".into(),
            is_recovery: true,
            recovery_reason: Some("stalled-after-claim".into()),
            last_nudge_result: None,
            issue_tags: vec!["16".into()],
            phase_tags: vec![],
            parent_qitem_id: None
        }));
        dirty!(s.insert_destination_snapshot("item", "seat", "cutoff", false, &[]));
        dirty!(s.replace_seat_bounds(&[SeatBound {
            seat: "seat".into(),
            created_at: Some(ts),
            stopped_at: None
        }]));
        dirty!(s.insert_seat_role("seat", "builder", "record"));
        dirty!(s.insert_skill(&SkillFact {
            source_key: "skill".into(),
            seat: "seat".into(),
            skill: "test".into(),
            invoked: true
        }));
        dirty!(s.record_warning(&WarningNote {
            path: "file".into(),
            line: Some(1),
            reason: "synthetic".into()
        }));
        dirty!(
            s.record_tool_categories(&[("tool".into(), ToolCategory::from_label("eng_code_edit"))])
        );
        dirty!(s.record_usage_models(&[(
            "usage".into(),
            Some("model".into()),
            Some("low".into())
        )]));
        dirty!(s.commit_file_batch("source", "claude", "file", 0, 1, "hash", None, &[], &[]));
        dirty!(s.set_source_session("source", "native"));
        dirty!(s.with_openrig_fact_batch(|| Ok(())));
        dirty!(s.begin_openrig_ingest("rig-source", "revision", &[]));
        dirty!(s.record_mutable_source_disclosure("item", "seat", "changed"));
        c.execute_batch("UPDATE dendro_semantic_state SET dirty=FALSE")?;
        let result: Result<()> = s.canonical_write(|| bail!("injected failure"));
        assert!(result.is_err());
        assert!(
            !c.query_row("SELECT dirty FROM dendro_semantic_state", [], |r| r
                .get::<_, bool>(0))?
        );
        let result: Result<()> = s.with_openrig_fact_batch(|| {
            s.insert_seat_role("rollback", "builder", "test")?;
            bail!("reader failed")
        });
        assert!(result.is_err());
        assert!(
            !c.query_row("SELECT dirty FROM dendro_semantic_state", [], |r| r
                .get::<_, bool>(0))?
        );
        assert_eq!(
            c.query_row(
                "SELECT count(*) FROM seat_roles WHERE seat='rollback'",
                [],
                |r| r.get::<_, i64>(0)
            )?,
            0
        );
        Ok(())
    }

    #[test]
    fn legacy_sidecar_without_file_source_baselines_refreshes_and_invalidates_cleanly() -> Result<()>
    {
        struct Directory(std::path::PathBuf);
        impl Drop for Directory {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        #[cfg(any(windows, target_os = "linux"))]
        let private = crate::platform_v1::create_user_private_root_with_choice(
            "dendro-semantic-test-",
            &crate::platform_v1::StorageRootChoice::UserProfile,
        )?;
        #[cfg(any(windows, target_os = "linux"))]
        let directory = Directory(private.path().to_path_buf());
        #[cfg(not(any(windows, target_os = "linux")))]
        let directory = Directory(std::env::current_dir()?.join("target/semantic-tests").join(
            format!("legacy-{}-{}", std::process::id(), rand::random::<u64>()),
        ));
        std::fs::create_dir_all(&directory.0)?;
        let db = directory.0.join("legacy.duckdb");
        let key = crate::output::load_or_create_key(&directory.0.join("legacy.duckdb.key"), true)?;
        {
            let store = AnalyticsStore::open_path(&db, &key.id)?;
            let ts = chrono::DateTime::parse_from_rfc3339("2026-10-01T00:05:00Z")?
                .with_timezone(&chrono::Utc);
            store.insert_activity(&ActivityFact {
                source_key: "act-1".into(),
                seat: "dev.build@rig".into(),
                ts,
                state: "idle".into(),
                seq: 1,
            })?;
            store
                .connection()
                .execute_batch("DROP TABLE file_source_baselines")?;
            store.checkpoint()?;
        }
        let before_refresh = answer(&db, "Rank idle", None)?;
        assert_eq!(before_refresh["status"], "insufficient_data");

        let refreshed = refresh(&db, None, "2026-10-01T00:00:00Z", "2026-10-02T00:00:00Z")?;
        assert_eq!(refreshed["status"], "refreshed");

        let answered = answer(&db, "Rank idle", None)?;
        assert_eq!(answered["status"], "ok");

        // An owned candidate schema change invalidates derived state without changing an immutable selection.
        let change_schema = |path: &Path| -> Result<()> {
            let store = AnalyticsStore::open_existing(path, &key.id)?;
            store.connection().execute_batch(
                "CREATE TABLE file_source_baselines (
                    source_id VARCHAR NOT NULL,
                    generation BIGINT NOT NULL,
                    metadata_version INTEGER NOT NULL,
                    latest_association VARCHAR NOT NULL,
                    latest_attempt_at VARCHAR NOT NULL,
                    last_stable_observed_at VARCHAR,
                    last_stable_type VARCHAR,
                    last_stable_device VARCHAR,
                    last_stable_inode VARCHAR,
                    last_stable_size VARCHAR,
                    last_stable_mtime_ns VARCHAR,
                    association_reason VARCHAR NOT NULL,
                    PRIMARY KEY (source_id, generation)
                )",
            )?;
            store.checkpoint()?;
            Ok(())
        };
        #[cfg(any(windows, target_os = "linux"))]
        {
            let request = crate::presentation::ReadRequest::open(&db)?;
            crate::selection_binding::mutate_selected(request.selected().unwrap(), |staged, _| {
                change_schema(staged)
            })?;
        }
        #[cfg(not(any(windows, target_os = "linux")))]
        change_schema(&db)?;
        let after_schema_change = answer(&db, "Rank idle", None)?;
        assert_eq!(after_schema_change["status"], "insufficient_data");
        Ok(())
    }

    #[test]
    fn answer_from_report_ranks_without_a_semantic_cache() {
        let report = json!({
            "metric": "decisions",
            "definition_version": "acceptance-r1-a1",
            "payload": {
                "rig": "dendro-dev",
                "from": "2026-10-08T00:00:00Z",
                "to": "2026-10-09T00:00:00Z",
                "activity": [
                    {"seat": "rev.review@dendro-dev", "idle_with_work_secs": 10, "active_secs": 5},
                    {"seat": "dev.build@dendro-dev", "idle_with_work_secs": 40, "active_secs": 20}
                ],
                "m1_by_seat": [
                    {"seat": "dev.build@dendro-dev", "tokens": 100, "closed_items": 2, "tokens_per_closed_item": 50},
                    {"seat": "rev.review@dendro-dev", "tokens": 90, "closed_items": 1, "tokens_per_closed_item": 90},
                    {"seat": "orch@dendro-dev", "tokens": 10, "closed_items": 0, "tokens_per_closed_item": null}
                ],
                "handoff_edges": []
            }
        });
        let idle = answer_from_report(&report, "highest idle", None);
        assert_eq!(idle["status"], "ok");
        assert_eq!(idle["rows"][0]["seat"], "dev.build@dendro-dev");
        assert_eq!(idle["scope"]["rig"], "dendro-dev");
        let yield_rank = answer_from_report(&report, "worst yield", None);
        assert_eq!(yield_rank["rows"][0]["seat"], "rev.review@dendro-dev");
        assert_eq!(yield_rank["rows"].as_array().unwrap().len(), 2);
        let refused = answer_from_report(&report, "launch the fleet", None);
        assert_eq!(refused["status"], "insufficient_data");
        assert!(refused["supported_examples"].as_array().is_some());
        let explain = answer_from_report(&report, "explain rework", None);
        assert_eq!(explain["status"], "insufficient_data");
    }

    fn minimal_report() -> Value {
        json!({
            "metric": "decisions",
            "definition_version": "acceptance-r1-a1",
            "payload": {
                "rig": "test-rig",
                "from": "2026-10-01T00:00:00Z",
                "to": "2026-10-02T00:00:00Z",
                "activity": [],
                "activity_events": [],
                "demand_events": [],
                "m1_by_seat": [],
                "handoff_friction": {"pairs": [], "vortex_loops": []},
                "trace_inputs": [],
                "issue_lineage": {"records": []},
                "m1_yield": null,
                "m2_return_rate": null,
                "m3_rework_chains": 0,
                "token_categories": {},
                "link_rates": [],
                "fleet_concurrency": {},
                "watchdog_recovery": {},
                "four_eyes_independence": {},
                "context_compactions": {},
                "cross_slice_cache_bleed": {},
                "task_intake_overhead": {},
                "ring_boundaries": {}
            }
        })
    }

    #[test]
    fn new_fact_views_build_on_empty_store_and_when_lazy_tables_are_absent() -> Result<()> {
        let store = AnalyticsStore::open_in_memory("empty-views")?;
        let report = minimal_report();
        let c = store.connection();
        materialize(&store, &report, &signature(&store)?)?;
        for (view, query) in [
            (
                "v_seat_idle_ranking",
                "SELECT count(*) FROM v_seat_idle_ranking",
            ),
            (
                "v_handoff_rework_analysis",
                "SELECT count(*) FROM v_handoff_rework_analysis",
            ),
            (
                "v_mission_token_burn",
                "SELECT count(*) FROM v_mission_token_burn",
            ),
            (
                "v_executive_governance",
                "SELECT count(*) FROM v_executive_governance",
            ),
            ("v_message_mix", "SELECT count(*) FROM v_message_mix"),
            (
                "v_subagent_lineage",
                "SELECT count(*) FROM v_subagent_lineage",
            ),
            (
                "v_security_posture",
                "SELECT count(*) FROM v_security_posture",
            ),
            (
                "v_process_resources",
                "SELECT count(*) FROM v_process_resources",
            ),
        ] {
            let count: i64 = c.query_row(query, [], |r| r.get(0))?;
            if view != "v_executive_governance" {
                assert_eq!(count, 0, "expected empty view {view}");
            } else {
                assert_eq!(count, 1);
            }
        }
        c.execute_batch(
            "DROP TABLE message_facts;
             DROP TABLE session_edges;
             DROP TABLE security_facts;
             DROP TABLE process_samples;
             DROP TABLE process_identities;",
        )?;
        let absent_sig = signature(&store)?;
        assert!(absent_sig.contains("message_facts:absent"));
        assert!(absent_sig.contains("session_edges:absent"));
        assert!(absent_sig.contains("security_facts:absent"));
        assert!(absent_sig.contains("process_identities:absent"));
        assert!(absent_sig.contains("process_samples:absent"));
        materialize(&store, &report, &absent_sig)?;
        for (view, query) in [
            ("v_message_mix", "SELECT count(*) FROM v_message_mix"),
            (
                "v_subagent_lineage",
                "SELECT count(*) FROM v_subagent_lineage",
            ),
            (
                "v_security_posture",
                "SELECT count(*) FROM v_security_posture",
            ),
            (
                "v_process_resources",
                "SELECT count(*) FROM v_process_resources",
            ),
        ] {
            let count: i64 = c.query_row(query, [], |r| r.get(0))?;
            assert_eq!(
                count, 0,
                "expected 0 rows when lazy table is absent for {view}"
            );
        }
        Ok(())
    }

    #[test]
    #[allow(clippy::type_complexity)]
    fn new_fact_views_populate_exact_expected_rows_and_preserve_unknown_and_stale_states(
    ) -> Result<()> {
        use crate::domain_v1::{
            CaptureMethod, Direction, EnvironmentKind, Grain, MessageRole, Modality, Unit,
            VisibilityLimit,
        };
        use crate::facts::{
            MessageFact, ProcessIdentityFact, ProcessSampleFact, ReaderState, SecurityFact,
            SessionEdgeFact,
        };
        use crate::storage::FileBaselineAttempt;

        let store = AnalyticsStore::open_in_memory("populated-views")?;
        let ts = chrono::DateTime::parse_from_rfc3339("2026-10-01T00:05:00Z")?
            .with_timezone(&chrono::Utc);

        store.commit_file_batch_observed_messages(
            "src-msg",
            "claude",
            "session.jsonl",
            1,
            128,
            &"a".repeat(64),
            None,
            &[],
            &[],
            &[
                MessageFact {
                    source_key: "msg:1".into(),
                    harness: "claude".into(),
                    native_session: Some("sess-1".into()),
                    ts: Some(ts),
                    seq: 1,
                    role: MessageRole::User,
                    direction: Direction::Inbound,
                    modalities: BTreeSet::from([Modality::Text]),
                    grain: Grain::Message,
                    generation: 1,
                },
                MessageFact {
                    source_key: "msg:2".into(),
                    harness: "claude".into(),
                    native_session: Some("sess-1".into()),
                    ts: Some(ts),
                    seq: 2,
                    role: MessageRole::User,
                    direction: Direction::Inbound,
                    modalities: BTreeSet::from([Modality::Text]),
                    grain: Grain::Message,
                    generation: 1,
                },
                MessageFact {
                    source_key: "msg:3".into(),
                    harness: "claude".into(),
                    native_session: Some("sess-1".into()),
                    ts: Some(ts),
                    seq: 3,
                    role: MessageRole::Assistant,
                    direction: Direction::Outbound,
                    modalities: BTreeSet::from([Modality::Text, Modality::Structured]),
                    grain: Grain::Message,
                    generation: 1,
                },
            ],
            &[],
            &ReaderState {
                complete: true,
                native_session: Some("sess-1".into()),
            },
            &FileBaselineAttempt {
                attempted_at: "2026-10-01T00:05:00Z".into(),
                metadata: None,
            },
        )?;

        store.commit_session_edges(&[
            SessionEdgeFact {
                source_key: "edge:1".into(),
                source_id: "src-edge".into(),
                harness: "claude".into(),
                parent_session: "root-a".into(),
                child_session: "child-1".into(),
                relation: "spawn".into(),
                status: "finished".into(),
                observed_at: Some(ts),
                evidence_kind: "explicit".into(),
                generation: 1,
            },
            SessionEdgeFact {
                source_key: "edge:2".into(),
                source_id: "src-edge".into(),
                harness: "claude".into(),
                parent_session: "root-a".into(),
                child_session: "child-2".into(),
                relation: "delegate_existing".into(),
                status: "failed".into(),
                observed_at: None,
                evidence_kind: "explicit".into(),
                generation: 1,
            },
            SessionEdgeFact {
                source_key: "edge:3".into(),
                source_id: "src-edge".into(),
                harness: "codex".into(),
                parent_session: "root-b".into(),
                child_session: "child-3".into(),
                relation: "spawn".into(),
                status: "started".into(),
                observed_at: Some(ts),
                evidence_kind: "explicit".into(),
                generation: 1,
            },
        ])?;

        store.commit_security_facts(&[
            SecurityFact {
                source_key: "sec:1".into(),
                source_id: "src-sec".into(),
                harness: "claude".into(),
                native_session: Some("sess-1".into()),
                ts: Some(ts),
                kind: "permission_mode".into(),
                value: "auto".into(),
                source_field: "permissionMode".into(),
                generation: 1,
            },
            SecurityFact {
                source_key: "sec:2".into(),
                source_id: "src-sec".into(),
                harness: "claude".into(),
                native_session: Some("sess-2".into()),
                ts: Some(ts),
                kind: "permission_mode".into(),
                value: "auto".into(),
                source_field: "permissionMode".into(),
                generation: 1,
            },
            SecurityFact {
                source_key: "sec:3".into(),
                source_id: "src-sec".into(),
                harness: "codex".into(),
                native_session: Some("sess-3".into()),
                ts: None,
                kind: "dangerous_command".into(),
                value: "force_push".into(),
                source_field: "command".into(),
                generation: 1,
            },
        ])?;
        // Direct out-of-vocabulary row must be excluded by closed-vocabulary filter.
        store.connection().execute(
            "INSERT INTO security_facts VALUES ('sec:bad', 'src-sec', 'claude', 'sess-1', NULL, 'dangerous_command', 'raw_command_text', 'command', 1)",
            [],
        )?;

        store.commit_process_samples(
            &[
                ProcessIdentityFact {
                    identity_key: "proc-root-1".into(),
                    root_key: "proc-root-1".into(),
                    pid: 100,
                    parent_pid: Some(1),
                    platform: "linux".into(),
                    start_time_unix_s: 1_700_000_000,
                    exe_name: "claude".into(),
                    harness: Some("claude".into()),
                    cwd_fingerprint: None,
                    session_link: Some("sess-1".into()),
                    environment_kind: EnvironmentKind::Host,
                    isolation_hint: "none".into(),
                    first_seen: ts,
                    last_seen: ts,
                    ended_at: None,
                    visibility_limits: BTreeSet::new(),
                    attribution: Some("launched".into()),
                },
                ProcessIdentityFact {
                    identity_key: "proc-child-1".into(),
                    root_key: "proc-root-1".into(),
                    pid: 101,
                    parent_pid: Some(100),
                    platform: "linux".into(),
                    start_time_unix_s: 1_700_000_001,
                    exe_name: "cargo".into(),
                    harness: None,
                    cwd_fingerprint: None,
                    session_link: None,
                    environment_kind: EnvironmentKind::Host,
                    isolation_hint: "none".into(),
                    first_seen: ts,
                    last_seen: ts,
                    ended_at: None,
                    visibility_limits: BTreeSet::new(),
                    attribution: None,
                },
                ProcessIdentityFact {
                    identity_key: "proc-root-unreadable".into(),
                    root_key: "proc-root-unreadable".into(),
                    pid: 200,
                    parent_pid: Some(1),
                    platform: "windows".into(),
                    start_time_unix_s: 1_700_000_002,
                    exe_name: "codex".into(),
                    harness: Some("codex".into()),
                    cwd_fingerprint: None,
                    session_link: None,
                    environment_kind: EnvironmentKind::Host,
                    isolation_hint: "none".into(),
                    first_seen: ts,
                    last_seen: ts,
                    ended_at: None,
                    visibility_limits: BTreeSet::from([VisibilityLimit::PermissionDenied]),
                    attribution: Some("discovered".into()),
                },
            ],
            &[
                ProcessSampleFact {
                    watch_id: "w1".into(),
                    identity_key: "proc-root-1".into(),
                    seq: 1,
                    ts,
                    cpu_time_us: Some(1000),
                    rss_bytes: Some(4096),
                    rss_unit: Unit::LinuxRssBytes,
                    disk_read_bytes: Some(0),
                    disk_write_bytes: Some(0),
                    capture_method: CaptureMethod::Procfs,
                    visibility_limits: BTreeSet::new(),
                },
                ProcessSampleFact {
                    watch_id: "w1".into(),
                    identity_key: "proc-child-1".into(),
                    seq: 2,
                    ts,
                    cpu_time_us: Some(500),
                    rss_bytes: Some(8192),
                    rss_unit: Unit::LinuxRssBytes,
                    disk_read_bytes: Some(0),
                    disk_write_bytes: Some(0),
                    capture_method: CaptureMethod::Procfs,
                    visibility_limits: BTreeSet::new(),
                },
                ProcessSampleFact {
                    watch_id: "w1".into(),
                    identity_key: "proc-root-unreadable".into(),
                    seq: 1,
                    ts,
                    cpu_time_us: None,
                    rss_bytes: None,
                    rss_unit: Unit::WindowsWorkingSetBytes,
                    disk_read_bytes: None,
                    disk_write_bytes: None,
                    capture_method: CaptureMethod::WindowsProcessApi,
                    visibility_limits: BTreeSet::from([VisibilityLimit::PermissionDenied]),
                },
            ],
        )?;

        let report = minimal_report();
        let before_sig = signature(&store)?;
        let c = store.connection();
        materialize(&store, &report, &before_sig)?;

        let mut stmt = c.prepare(
            "SELECT harness, native_session, role, direction, modality, message_count, rig, window_from, window_to
             FROM v_message_mix ORDER BY modality",
        )?;
        let msg_rows: Vec<(
            String,
            Option<String>,
            String,
            String,
            String,
            i64,
            Option<String>,
            String,
            String,
        )> = stmt
            .query_map([], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                    r.get(8)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        assert_eq!(
            msg_rows,
            vec![
                (
                    "claude".into(),
                    Some("sess-1".into()),
                    "user".into(),
                    "inbound".into(),
                    "text".into(),
                    2,
                    Some("test-rig".into()),
                    "2026-10-01T00:00:00Z".into(),
                    "2026-10-02T00:00:00Z".into(),
                ),
                (
                    "claude".into(),
                    Some("sess-1".into()),
                    "assistant".into(),
                    "outbound".into(),
                    "text,structured".into(),
                    1,
                    Some("test-rig".into()),
                    "2026-10-01T00:00:00Z".into(),
                    "2026-10-02T00:00:00Z".into(),
                ),
            ]
        );

        let mut stmt = c.prepare(
            "SELECT harness, parent_session, child_count, edge_count, started_count, finished_count, failed_count, unknown_count
             FROM v_subagent_lineage ORDER BY harness, parent_session",
        )?;
        let lineage_rows: Vec<(String, String, i64, i64, i64, i64, i64, i64)> = stmt
            .query_map([], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        assert_eq!(
            lineage_rows,
            vec![
                ("claude".into(), "root-a".into(), 2, 2, 0, 1, 1, 0),
                ("codex".into(), "root-b".into(), 1, 1, 1, 0, 0, 0),
            ]
        );

        let mut stmt = c.prepare(
            "SELECT harness, kind, value, observation_count, session_count
             FROM v_security_posture ORDER BY harness, kind, value",
        )?;
        let sec_rows: Vec<(String, String, String, i64, i64)> = stmt
            .query_map([], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        assert_eq!(
            sec_rows,
            vec![
                (
                    "claude".into(),
                    "permission_mode".into(),
                    "auto".into(),
                    2,
                    2
                ),
                (
                    "codex".into(),
                    "dangerous_command".into(),
                    "force_push".into(),
                    1,
                    1
                ),
            ]
        );

        let mut stmt = c.prepare(
            "SELECT root_key, harness, attribution, rss_unit, process_count, max_rss_bytes, sample_count
             FROM v_process_resources ORDER BY root_key, rss_unit",
        )?;
        let proc_rows: Vec<(
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            i64,
            Option<i64>,
            i64,
        )> = stmt
            .query_map([], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        assert_eq!(
            proc_rows,
            vec![
                (
                    "proc-root-1".into(),
                    Some("claude".into()),
                    Some("launched".into()),
                    Some("linux_rss_bytes".into()),
                    2,
                    Some(8192),
                    2,
                ),
                (
                    "proc-root-unreadable".into(),
                    Some("codex".into()),
                    Some("discovered".into()),
                    Some("windows_working_set_bytes".into()),
                    1,
                    None,
                    1,
                ),
            ]
        );
        let cpu_cols: i64 = c.query_row(
            "SELECT count(*) FROM information_schema.columns WHERE table_name = 'v_process_resources' AND column_name LIKE '%cpu%'",
            [],
            |r| r.get(0),
        )?;
        assert_eq!(
            cpu_cols, 0,
            "CPU deltas must not be computed or exposed in v_process_resources"
        );

        // Error state: dirty semantic guard refuses reads on all four new views.
        c.execute_batch("UPDATE dendro_semantic_state SET dirty = TRUE WHERE id = 1")?;
        for (view, query) in [
            ("v_message_mix", "SELECT count(*) FROM v_message_mix"),
            (
                "v_subagent_lineage",
                "SELECT count(*) FROM v_subagent_lineage",
            ),
            (
                "v_security_posture",
                "SELECT count(*) FROM v_security_posture",
            ),
            (
                "v_process_resources",
                "SELECT count(*) FROM v_process_resources",
            ),
        ] {
            let err = c.query_row(query, [], |r| r.get::<_, i64>(0)).unwrap_err();
            assert!(
                err.to_string().contains("semantic snapshot is stale"),
                "expected stale guard error for {view}, got {err}"
            );
        }

        // Signature changes when lazy tables are modified.
        store.commit_security_facts(&[SecurityFact {
            source_key: "sec:4".into(),
            source_id: "src-sec".into(),
            harness: "claude".into(),
            native_session: Some("sess-4".into()),
            ts: Some(ts),
            kind: "approval_policy".into(),
            value: "never".into(),
            source_field: "payload.approval_policy".into(),
            generation: 1,
        }])?;
        assert_ne!(signature(&store)?, before_sig);

        let models_yaml = include_str!("../../../templates/nao/semantic_models.yaml");
        for view in VIEWS {
            assert!(
                models_yaml.contains(view),
                "missing {view} in semantic_models.yaml"
            );
        }
        Ok(())
    }
}
