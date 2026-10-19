//! Scoped, explicitly refreshed projections of the canonical report. No alternate metric engine.
use crate::{pipeline::report_json, storage::AnalyticsStore};
use anyhow::{anyhow, bail, Context, Result};
use duckdb::params;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path},
};

pub const VIEWS: [&str; 4] = [
    "v_seat_idle_ranking",
    "v_handoff_rework_analysis",
    "v_mission_token_burn",
    "v_executive_governance",
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

const OPTIONAL_SIGNATURES: [(&str, &str); 3] = [
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
    conn.execute_batch("SET memory_limit='1GB'; SET threads=1")?;
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
    c.execute_batch("UPDATE dendro_semantic_state SET dirty = FALSE WHERE id = 1; CREATE OR REPLACE VIEW dendro_semantic_guard AS SELECT CASE WHEN NOT dirty THEN true ELSE error('Insufficient data: semantic snapshot is stale; refresh required') END AS fresh FROM dendro_semantic_state WHERE id = 1")?;
    c.execute_batch(include_str!("semantic_views.sql"))?;
    stage("group_and_views", started);
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
            let amount = q
                .strip_prefix("where does ")?
                .strip_suffix("% idle come from")?
                .parse::<f64>()
                .ok()?;
            (amount.is_finite() && (0.0..=100.0).contains(&amount)).then_some(Intent::Trace)
        }
    }
}

/// Catalog of supported conversational intents and copy-pasteable example phrases.
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
        ]
    })
}

/// Inspect the currently materialized semantic scope and whether it matches the live signature.
pub fn cached_scope(db: &Path) -> Option<(Option<String>, String, String, bool)> {
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
        v["supported_examples"] = supported_intents()["intents"].clone();
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

/// Creates a new workspace only. Never overwrites an existing directory or follows its symlink.
pub fn init_workspace(destination: &Path, db: &Path) -> Result<()> {
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
    let target = canonical_parent.join(name);
    let database = fs::canonicalize(db)?;
    crate::pipeline::refuse_secret_path(&database)?;
    fs::create_dir(&target).context("workspace must be a new directory")?;
    let result = (|| -> Result<()> {
        let config = include_str!("../../../templates/nao/nao_config.yaml")
            .replace("\"dendro.duckdb\"", &serde_json::to_string(&database)?);
        fs::write(target.join("nao_config.yaml"), config)?;
        fs::write(
            target.join("semantic_models.yaml"),
            include_str!("../../../templates/nao/semantic_models.yaml"),
        )?;
        fs::write(
            target.join("RULES.md"),
            include_str!("../../../templates/nao/prompts.md"),
        )?;
        fs::create_dir(target.join("semantics"))?;
        fs::write(
            target.join("semantics/dendro.md"),
            include_str!("../../../templates/nao/prompts.md"),
        )?;
        fs::write(
            target.join("prompts.md"),
            include_str!("../../../templates/nao/prompts.md"),
        )?;
        fs::write(
            target.join("database.json"),
            serde_json::to_vec_pretty(&json!({"path":database,"read_only":true}))?,
        )?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&target);
    }
    result
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
        let directory = Directory(std::env::current_dir()?.join("target/semantic-tests").join(
            format!("legacy-{}-{}", std::process::id(), rand::random::<u64>()),
        ));
        std::fs::create_dir_all(&directory.0)?;
        let db = directory.0.join("legacy.duckdb");
        {
            let store = AnalyticsStore::open_path(&db, "legacy")?;
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

        // Recreating and then dropping the optional table invalidates the snapshot cleanly.
        {
            let store = AnalyticsStore::open_existing(&db, "legacy")?;
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
        }
        let after_schema_change = answer(&db, "Rank idle", None)?;
        assert_eq!(after_schema_change["status"], "insufficient_data");
        Ok(())
    }
}
