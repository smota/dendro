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
const SIGNATURE: &str = "SELECT CAST(list(kind || ':' || n || ':' || digest ORDER BY kind) AS VARCHAR) FROM (SELECT kind, count(*) n, bit_xor(h) digest FROM (SELECT 'dendro_meta' kind, hash(t) h FROM dendro_meta t UNION ALL
SELECT 'file_source_baselines' kind, hash(t) h FROM file_source_baselines t UNION ALL
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
SELECT 'source_files' kind, hash(t) h FROM source_files t UNION ALL
SELECT 'openrig_row_records' kind, hash(t) h FROM openrig_row_records t UNION ALL
SELECT 'mutable_source_disclosures' kind, hash(t) h FROM mutable_source_disclosures t) GROUP BY kind)";
fn signature(store: &AnalyticsStore) -> Result<String> {
    Ok(store.connection().query_row(SIGNATURE, [], |r| r.get(0))?)
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
        | "compare yield" => Some(Intent::Compare),
        "which seat had the worst yield"
        | "which seat had worst yield"
        | "which seat had the highest idle time"
        | "which seat has highest idle"
        | "which seat has the highest idle time"
        | "rank idle"
        | "rank yield" => Some(Intent::Rank),
        "why did build rework go up" | "explain rework" | "explain handoffs" => {
            Some(Intent::Explain)
        }
        "what changed" | "verify changes" | "verify topology change" => Some(Intent::Verify),
        "trace idle"
        | "trace rework"
        | "trace tokens"
        | "trace yield"
        | "where does idle come from" => Some(Intent::Trace),
        "honest limits" | "what are the honest limits" => Some(Intent::HonestLimits),
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
        return Ok(refusal("unsupported intent, metric, or question qualifier; select an exact report scope and use a supported phrase"));
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
            let sql = if q.contains("idle") {"SELECT row_json::VARCHAR FROM v_seat_idle_ranking ORDER BY idle_with_work_secs DESC, seat_id"}
                else if q.contains("yield") {"SELECT m1::VARCHAR FROM v_seat_idle_ranking WHERE try_cast(json_extract(m1,'$.tokens_per_closed_item') AS DOUBLE) IS NOT NULL ORDER BY try_cast(json_extract(m1,'$.tokens_per_closed_item') AS DOUBLE) DESC NULLS LAST, seat_id"}
                else {return Ok(refusal("rank supports idle and canonical yield only"));};
            let mut stmt=s.connection().prepare(sql)?;
            let rows=stmt.query_map([],|r|r.get::<_,Option<String>>(0))?.collect::<duckdb::Result<Vec<_>>>()?;
            let rows=rows.into_iter().flatten().map(|v|serde_json::from_str::<Value>(&v)).collect::<std::result::Result<Vec<_>,_>>()?;
            if rows.is_empty() {refusal("no observed seats")} else {json!({"status":"ok","rows":rows,"ordering":if q.contains("idle") {"idle_with_work_secs descending"} else {"tokens_per_closed_item descending: worst observed burn per completed item"}})}
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
}
