//! Read-only portfolio aggregation over independently ingested analytics outputs.
use crate::AnalyticsStore;
use anyhow::{ensure, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
};

/// Every source is an existing analytics output, never a request to merge OpenRig roots.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortfolioSource {
    pub db: PathBuf,
    pub rigs: Vec<String>,
    /// Explicit direct-child hold directory (or a copied export); never HOME autodiscovery.
    #[serde(default)]
    pub hold_dir: Option<PathBuf>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Role {
    Supervisory,
    Builder,
    Orchestrator,
    Other,
    Unknown,
}
fn role(evidence: &str) -> Option<Role> {
    let logical = evidence.rsplit_once("agentflow-seat:").map(|(_, r)| r);
    match logical {
        Some("steer.lead") => Some(Role::Supervisory),
        Some("dev.build" | "dev.build-jr" | "dev.build-sr") => Some(Role::Builder),
        Some("orch.arch") => Some(Role::Orchestrator),
        Some("dev.qa" | "rev.review" | "pm.manager" | "pm.analyst" | "pm.lead" | "eval.tester") => {
            Some(Role::Other)
        }
        Some(_) => None,
        None => match evidence.rsplit('|').next() {
            Some("operator" | "supervisor" | "steerer") if evidence.contains("spec:") => {
                Some(Role::Supervisory)
            }
            Some("implementer" | "builder") if evidence.contains("spec:") => Some(Role::Builder),
            Some("architect" | "orchestrator") if evidence.contains("spec:") => {
                Some(Role::Orchestrator)
            }
            _ => None,
        },
    }
}
fn unique<'a, T: Ord>(map: &'a BTreeMap<String, BTreeSet<T>>, key: &str) -> Option<&'a T> {
    let entries = map.get(key)?;
    (entries.len() == 1).then(|| entries.first()).flatten()
}
fn opaque(material: impl AsRef<[u8]>) -> String {
    hex::encode(Sha256::digest(material))
}
fn namespace(source: &str, seat: &str) -> String {
    format!("{source}\0{seat}")
}
fn identifier(value: &str) -> String {
    crate::redact(value)
}
fn parsed_time(value: &str) -> Result<DateTime<Utc>> {
    crate::parse_utc_time(value).ok_or_else(|| anyhow::anyhow!("invalid portfolio timestamp"))
}

/// The canonical per-rig payload is embedded without numeric recomputation.
/// Cross-source evidence is a read-time join, not an analytics database mutation.
pub fn portfolio_json(sources: &[PortfolioSource], from: &str, to: &str) -> Result<Value> {
    ensure!(
        !sources.is_empty(),
        "portfolio requires explicit analytics sources"
    );
    let start = parsed_time(from)?;
    let end = parsed_time(to)?;
    ensure!(
        end > start,
        "portfolio window must be non-empty and half-open"
    );
    let mut databases = BTreeSet::new();
    let mut assigned_rigs = BTreeSet::new();
    let mut selected_rigs = BTreeMap::<String, BTreeSet<String>>::new();
    let mut openrig_origins = BTreeSet::new();
    let mut memberships = BTreeMap::<String, BTreeSet<String>>::new();
    let mut pending_receivers = BTreeMap::<(String, String), (DateTime<Utc>, String)>::new();
    let mut roles = BTreeMap::<String, BTreeSet<Role>>::new();
    let mut role_evidence = BTreeMap::<String, BTreeSet<String>>::new();
    let mut source_census = Vec::new();
    let mut rows = Vec::new();
    let mut all_edges = Vec::new();
    let mut directories = BTreeSet::new();
    let mut holds = Vec::new();
    let mut warnings = BTreeSet::<String>::new();
    let observed_at = Utc::now();
    for source in sources {
        crate::RUN_CANCEL.check()?;
        let read_started_at = Utc::now();
        crate::refuse_secret_path(&source.db)?;
        let database = source
            .db
            .canonicalize()
            .map_err(|_| anyhow::anyhow!("portfolio database unavailable"))?;
        crate::refuse_secret_path(&database)?;
        ensure!(
            databases.insert(database.clone()),
            "duplicate portfolio database"
        );
        ensure!(
            !source.rigs.is_empty(),
            "each portfolio source needs explicit rig mapping"
        );
        for rig in &source.rigs {
            ensure!(
                !rig.is_empty() && assigned_rigs.insert(rig.clone()),
                "duplicate or empty portfolio rig mapping"
            );
        }
        let source_id = opaque(database.as_os_str().as_encoded_bytes());
        selected_rigs.insert(source_id.clone(), source.rigs.iter().cloned().collect());
        let store = AnalyticsStore::open_read_only(&database)?;
        let (source_identity, source_roles, retained_sources, queue) =
            store.read_snapshot(|store| {
                Ok((
                    store.load_identity()?,
                    store.load_seat_roles()?,
                    store.load_sources()?,
                    store.load_queue()?,
                ))
            })?;
        for row in queue.into_iter().filter(|row| row.ts < end) {
            let key = (source_id.clone(), row.qitem_id);
            if row.state == "pending" {
                pending_receivers
                    .entry(key)
                    .or_insert((row.ts, row.source_key));
            } else {
                pending_receivers.remove(&key);
            }
        }
        let origins: BTreeSet<_> = retained_sources
            .iter()
            .filter(|(_, harness, _, _, _)| harness == "openrig")
            .map(|(_, _, path, _, _)| crate::canonical_root(Path::new(path)))
            .collect();
        for origin in origins {
            ensure!(
                openrig_origins.insert(origin),
                "duplicate recorded OpenRig origin across portfolio databases"
            );
        }
        for identity in source_identity {
            memberships
                .entry(namespace(&source_id, &identity.seat))
                .or_default()
                .insert(identity.rig);
        }
        for (seat, _, evidence) in source_roles {
            // An unrecognized explicit observation cannot disappear and leave a false unique role.
            roles
                .entry(namespace(&source_id, &seat))
                .or_default()
                .insert(role(&evidence).unwrap_or(Role::Unknown));
            role_evidence
                .entry(namespace(&source_id, &seat))
                .or_default()
                .insert(identifier(&evidence));
        }
        drop(store);
        // Unscoped edges exactly once per database, so per-rig views cannot duplicate seams.
        let unscoped = crate::report_json(&database, "decisions", None, from, to)?;
        for edge in unscoped["payload"]["ring_boundaries"]["edges"]
            .as_array()
            .into_iter()
            .flatten()
        {
            all_edges.push((source_id.clone(), edge.clone()));
        }
        for rig in &source.rigs {
            let canonical = crate::report_json(&database, "decisions", Some(rig), from, to)?;
            let payload = &canonical["payload"];
            rows.push(json!({"source_id":source_id,"rig":identifier(rig),"status":if payload["usage_rows"].as_u64().unwrap_or(0)>0 {"ok"} else {"no_data"},
                "tokens":payload["tokens"], "ceremony_ratio":payload["tool_categories"]["ceremony_ratio"],
                "effective_parallelism":payload["fleet_concurrency"]["effective_parallelism"],
                "throughput":payload["m1_by_seat"], "return_rate":payload["m2_return_rate"], "rework_rate":payload["m3_rework_proportion"],
                "model_effort_tokens":payload["model_effort_tokens"], "canonical":canonical}));
        }
        source_census.push(json!({"source_id":source_id,"rigs":source.rigs.iter().map(|rig|identifier(rig)).collect::<Vec<_>>(),"core_schema_version":crate::SCHEMA_VERSION,"read_started_at":read_started_at.to_rfc3339(),"read_finished_at":Utc::now().to_rfc3339()}));
        if let Some(directory) = &source.hold_dir {
            crate::refuse_secret_path(directory)?;
            let canonical = directory.canonicalize().ok();
            if canonical
                .as_ref()
                .is_some_and(|path| !directories.insert(path.clone()))
            {
                warnings.insert("duplicate-hold-directory-skipped".into());
            } else {
                let (records, reasons) = read_holds(
                    directory,
                    start,
                    end,
                    observed_at,
                    selected_rigs.get(&source_id).expect("selected source"),
                    |sender| {
                        unique(&memberships, &namespace(&source_id, sender))
                            .map(|rig| identifier(rig))
                    },
                )?;
                for mut record in records {
                    record["source_id"] = json!(source_id);
                    holds.push(record);
                }
                warnings.extend(reasons.into_iter().map(str::to_string));
            }
        }
    }
    let mut counts = BTreeMap::from([
        ("in_ring", 0usize),
        ("cross_rig_seam", 0),
        ("supervisory_plane", 0),
        ("no_data", 0),
    ]);
    let mut edges = Vec::new();
    let mut seen = BTreeSet::new();
    let mut dispatches = Vec::new();
    let mut failed_wakes = Vec::new();
    let canonical_edges = all_edges.len();
    let mut skipped_missing_keys = 0usize;
    let mut excluded_unselected = 0usize;
    for (source_id, edge) in all_edges {
        let text = |field: &str| edge[field].as_str().filter(|value| !value.is_empty());
        let (Some(source_key), Some(from_seat), Some(to_seat)) =
            (text("source_key"), text("from"), text("to"))
        else {
            skipped_missing_keys += 1;
            *counts.get_mut("no_data").expect("known zone") += 1;
            warnings.insert("edge-required-key-missing".into());
            continue;
        };
        // Canonical source keys are database-local. Deduplicate within their explicit output identity.
        if !seen.insert((
            source_id.clone(),
            source_key.to_string(),
            from_seat.to_string(),
            to_seat.to_string(),
        )) {
            continue;
        }
        let from_rig = unique(&memberships, &namespace(&source_id, from_seat));
        let to_rig = unique(&memberships, &namespace(&source_id, to_seat));
        let selected = selected_rigs.get(&source_id).expect("selected source");
        // Only proven unrelated endpoints can be excluded. Missing membership is counted coverage.
        if from_rig.is_some_and(|rig| !selected.contains(rig))
            && to_rig.is_some_and(|rig| !selected.contains(rig))
        {
            excluded_unselected += 1;
            continue;
        }
        let from_role = unique(&roles, &namespace(&source_id, from_seat))
            .filter(|role| **role != Role::Unknown);
        let to_role =
            unique(&roles, &namespace(&source_id, to_seat)).filter(|role| **role != Role::Unknown);
        let zone = match (from_rig, to_rig) {
            (Some(_), Some(_))
                if from_role == Some(&Role::Supervisory) || to_role == Some(&Role::Supervisory) =>
            {
                "supervisory_plane"
            }
            (Some(a), Some(b)) if a == b => "in_ring",
            (Some(_), Some(_)) => "cross_rig_seam",
            _ => "no_data",
        };
        *counts.get_mut(zone).expect("known zone") += 1;
        let bypass = from_role == Some(&Role::Supervisory)
            && to_role == Some(&Role::Builder)
            && from_rig.is_some()
            && to_rig.is_some();
        let observed_failed_wake = edge["failed_nudge"] == true;
        let record = json!({"source_id":source_id,"source_key":identifier(source_key),"qitem_id":edge["qitem_id"],
            "from":identifier(from_seat),"to":identifier(to_seat),"from_rig":from_rig.map(|r|identifier(r)),"to_rig":to_rig.map(|r|identifier(r)),
            "zone":zone,"canonical_boundary":edge["boundary"],"supervisory_bypass":bypass,
            "role_status":if from_role.is_some() && to_role.is_some(){"proven"}else{"no_data"},
            "from_role_source_keys":role_evidence.get(&namespace(&source_id,from_seat)),"to_role_source_keys":role_evidence.get(&namespace(&source_id,to_seat)),
            "observed_failed_wake":observed_failed_wake,"nudge_scope":"latest receiver item observation; not current terminal liveness"});
        let pending = edge["receiver_qitem_id"]
            .as_str()
            .and_then(|item| pending_receivers.get(&(source_id.clone(), item.to_string())));
        let dead_relay = observed_failed_wake
            && to_role == Some(&Role::Supervisory)
            && from_rig.is_some()
            && to_rig.is_some()
            && pending.is_some();
        if bypass || dead_relay {
            let mut evidence = vec![identifier(source_key)];
            for seat in [from_seat, to_seat] {
                if let Some(keys) = role_evidence.get(&namespace(&source_id, seat)) {
                    evidence.extend(keys.iter().cloned());
                }
            }
            if let Some((_, key)) = pending {
                evidence.push(identifier(key));
            }
            dispatches.push(json!({"source_id":source_id,"source_key":identifier(source_key),"source_seat":identifier(from_seat),"dest_seat":identifier(to_seat),"source_rig":from_rig.map(|r|identifier(r)),"dest_rig":to_rig.map(|r|identifier(r)),"classification":if bypass {"proven_bypass"} else {"dead_supervisory_relay"},"pending_duration_secs":pending.map(|(created,_)|(end.min(observed_at)-(*created).max(start)).num_seconds().max(0)),"pending_duration_scope":"latest accepted pending receiver state at window end; not failure duration","evidence":evidence}));
        }
        if observed_failed_wake && zone != "in_ring" {
            failed_wakes.push(record.clone());
        }
        edges.push(record);
    }
    let mut grouped_holds = BTreeMap::<(String, String), Vec<Value>>::new();
    for hold in holds {
        grouped_holds
            .entry((
                hold["to_rig"].as_str().unwrap_or_default().into(),
                hold["hold_id"].as_str().unwrap_or_default().into(),
            ))
            .or_default()
            .push(hold);
    }
    let mut holds = Vec::new();
    for (_, group) in grouped_holds {
        let normalized = |record: &Value| {
            let mut value = record.clone();
            value
                .as_object_mut()
                .expect("hold object")
                .remove("source_key");
            value
        };
        if group
            .iter()
            .skip(1)
            .any(|record| normalized(record) != normalized(&group[0]))
        {
            warnings.insert("conflicting-hold-records-excluded".into());
            continue;
        }
        let mut record = group[0].clone();
        record["source_keys"] = json!(group
            .iter()
            .map(|row| row["source_key"].clone())
            .collect::<Vec<_>>());
        holds.push(record);
    }
    let mut heatmap =
        BTreeMap::<(Option<String>, String, String), (usize, i64, Vec<String>)>::new();
    for hold in &mut holds {
        let from_rig = hold["from_rig"].as_str().map(str::to_string);
        for code in hold["codes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            let key = (
                from_rig.clone(),
                hold["to_rig"].as_str().unwrap_or_default().into(),
                code.into(),
            );
            let entry = heatmap.entry(key).or_default();
            entry.0 += 1;
            entry.1 += hold["completed_elapsed_secs"].as_i64().unwrap_or(0);
            entry.2.extend(
                hold["source_keys"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_string),
            );
        }
    }
    rows.sort_by(|a, b| a["rig"].as_str().cmp(&b["rig"].as_str()));
    let heatmap: Vec<_> = heatmap.into_iter().map(|((from_rig,to_rig,code),(records,elapsed,source_keys))|json!({"from_rig":from_rig,"to_rig":to_rig,"code":code,"hold_records":records,"completed_elapsed_secs":elapsed,"source_keys":source_keys})).collect();
    Ok(
        json!({"schema_version":"dendro-portfolio-v1","product":"AI Fleet Intelligence","from":from,"to":to,
        "payload":{"source_census":source_census,"rigs":rows,"matrix":{"counts":counts,"edges":edges,"coverage":{"canonical_edges":canonical_edges,"excluded_unselected":excluded_unselected,"skipped_missing_keys":skipped_missing_keys},"classification":"explicit recorded membership and roles; unknown membership stays no_data"},
        "supervisory_dispatch_status":if skipped_missing_keys==0 && edges.iter().all(|edge|edge["role_status"]=="proven" && !edge["from_rig"].is_null() && !edge["to_rig"].is_null()) && (!dispatches.is_empty() || !edges.is_empty()) {"observed"}else{"no_data"},"supervisory_dispatch":dispatches,"failed_cross_rig_wakes":failed_wakes,
        "snapshot_scope":"independent read snapshots per database and canonical report; no atomic global snapshot is claimed",
        "admission_holds":{"status":if holds.is_empty(){"no_data"}else{"observed_records"},"records":holds,"heatmap":heatmap,"observed_at":observed_at.to_rfc3339(),
        "scope":"B1-B4 signal snapshots; elapsed endpoint time is not residence time per code; held files do not prove live scheduler state"},"warnings":warnings}}),
    )
}

const MAX_HOLD_BYTES: u64 = 1024 * 1024;
const MAX_HOLD_FILES: usize = 4096;
fn read_holds(
    directory: &Path,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    observed: DateTime<Utc>,
    selected: &BTreeSet<String>,
    sender_rig: impl Fn(&str) -> Option<String>,
) -> Result<(Vec<Value>, Vec<&'static str>)> {
    crate::refuse_secret_path(directory)?;
    if !directory.exists() {
        return Ok((Vec::new(), vec!["hold-directory-missing"]));
    }
    ensure!(
        !fs::symlink_metadata(directory)?.file_type().is_symlink(),
        "symlink hold directory refused"
    );
    let root = directory
        .canonicalize()
        .map_err(|_| anyhow::anyhow!("hold directory unavailable"))?;
    crate::refuse_secret_path(&root)?;
    let mut paths = Vec::new();
    let mut warnings = Vec::new();
    for entry in fs::read_dir(&root).map_err(|_| anyhow::anyhow!("hold directory unavailable"))? {
        let entry = entry?;
        if entry.file_type()?.is_symlink() {
            warnings.push("symlink-hold-skipped");
            continue;
        }
        if entry.file_type()?.is_file() && entry.path().extension().is_some_and(|e| e == "json") {
            ensure!(
                paths.len() < MAX_HOLD_FILES,
                "hold directory exceeds 4096 files"
            );
            paths.push(entry.path());
        }
    }
    paths.sort();
    let mut records = Vec::new();
    for path in paths {
        crate::RUN_CANCEL.check()?;
        crate::refuse_secret_path(&path)?;
        ensure!(
            path.canonicalize()?.parent() == Some(root.as_path()),
            "hold escapes selected directory"
        );
        ensure!(
            !fs::symlink_metadata(&path)?.file_type().is_symlink(),
            "symlink hold refused"
        );
        let file = File::open(&path)?;
        let before = crate::file_metadata::FileMetadata::from_file(&file);
        ensure!(
            before.is_some() && crate::file_metadata::observe_path(&path).metadata == before,
            "hold identity unavailable"
        );
        ensure!(
            file.metadata()?.len() <= MAX_HOLD_BYTES,
            "hold file exceeds 1 MiB"
        );
        let mut bytes = Vec::new();
        let mut held = file.take(MAX_HOLD_BYTES + 1);
        held.read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() as u64 <= MAX_HOLD_BYTES,
            "hold file exceeds 1 MiB"
        );
        ensure!(
            crate::file_metadata::FileMetadata::from_file(held.get_ref()) == before
                && crate::file_metadata::observe_path(&path).metadata == before,
            "hold changed during read"
        );
        let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
            warnings.push("malformed-hold-json");
            continue;
        };
        let text = |field: &str| {
            value[field]
                .as_str()
                .filter(|s| !s.is_empty() && s.len() <= 256)
        };
        let (Some(id), Some(state), Some(rig), Some(target), Some(sender), Some(created)) = (
            text("id"),
            text("state"),
            text("rig"),
            text("target"),
            text("sender"),
            text("createdAt"),
        ) else {
            warnings.push("unsupported-hold-schema");
            continue;
        };
        if !selected.contains(rig) {
            continue;
        }
        if !matches!(state, "held" | "delivered") {
            warnings.push("unsupported-hold-state");
            continue;
        }
        let Some(signals) = value["signals"].as_array() else {
            warnings.push("unsupported-hold-signals");
            continue;
        };
        let codes: BTreeSet<_> = signals
            .iter()
            .filter_map(|s| s["id"].as_str())
            .filter(|s| matches!(*s, "B1" | "B2" | "B3" | "B4"))
            .collect();
        if codes.is_empty() {
            warnings.push("hold-has-no-supported-B1-B4-code");
            continue;
        }
        let Ok(created) = parsed_time(created) else {
            warnings.push("invalid-hold-time");
            continue;
        };
        let delivered = if state == "delivered" {
            text("deliveredAt").and_then(crate::parse_utc_time)
        } else {
            None
        };
        if created > observed
            || delivered.is_some_and(|end| end < created || end > observed)
            || (state == "delivered" && delivered.is_none())
        {
            warnings.push("invalid-hold-interval");
            continue;
        }
        if created >= to
            || (state == "held" && created < from)
            || delivered.is_some_and(|end| end <= from)
        {
            continue;
        }
        // Semantic copies are deduplicated after projection; conflicting snapshots are excluded.
        let completed = delivered.map(|end| (end.min(to) - created.max(from)).num_seconds().max(0));
        let source_key = opaque(
            [
                path.as_os_str().as_encoded_bytes(),
                Sha256::digest(&bytes).as_slice(),
            ]
            .concat(),
        );
        records.push(json!({"source_key":source_key,"hold_id":opaque(id),"to_rig":identifier(rig),"target":identifier(target),"sender":identifier(sender),"from_rig":sender_rig(sender),
            "codes":codes,"record_state":state,"created_at":created.to_rfc3339(),"delivered_at":delivered.map(|t|t.to_rfc3339()),
            "completed_elapsed_secs":completed,"observed_record_age_secs":if state=="held" {Some((observed.min(to)-created.max(from)).num_seconds().max(0))}else{None},"status":"observed_record_not_live_verification"}));
    }
    Ok((records, warnings))
}

/// One complete portfolio record per rig, followed by one global evidence record.
pub fn portfolio_ndjson(report: &Value) -> Result<String> {
    ensure!(
        report["schema_version"] == "dendro-portfolio-v1",
        "unsupported portfolio schema"
    );
    let mut output = String::new();
    for row in report["payload"]["rigs"].as_array().into_iter().flatten() {
        output.push_str(&serde_json::to_string(
            &json!({"type":"rig","payload":row}),
        )?);
        output.push('\n');
    }
    let mut global = report["payload"].clone();
    global
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("invalid portfolio payload"))?
        .remove("rigs");
    output.push_str(&serde_json::to_string(
        &json!({"type":"portfolio_evidence","payload":global}),
    )?);
    output.push('\n');
    Ok(output)
}
pub fn portfolio_table(report: &Value) -> Result<String> {
    ensure!(
        report["schema_version"] == "dendro-portfolio-v1",
        "unsupported portfolio schema"
    );
    let mut output = String::from(
        "rig\ttokens\tceremony_ratio\teffective_parallelism\treturn_rate\trework_rate\n",
    );
    for row in report["payload"]["rigs"].as_array().into_iter().flatten() {
        let fields = [
            row["rig"]
                .as_str()
                .unwrap_or("unknown")
                .replace(['\n', '\r', '\t'], " "),
            display(&row["tokens"]),
            display(&row["ceremony_ratio"]),
            display(&row["effective_parallelism"]),
            display(&row["return_rate"]),
            display(&row["rework_rate"]),
        ];
        output.push_str(&fields.join("\t"));
        output.push('\n');
    }
    Ok(output)
}
fn display(value: &Value) -> String {
    if value.is_null() {
        "no_data".into()
    } else {
        value.to_string()
    }
}
