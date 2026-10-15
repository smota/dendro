//! Deterministic diagnostics over canonical projections; no inference service or intervention.
use crate::advisory::AdvisoryOptions;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

fn rows(v: &Value) -> impl Iterator<Item = &Value> {
    v.as_array().into_iter().flatten()
}
fn keys(v: &Value) -> BTreeSet<String> {
    fn visit(v: &Value, out: &mut BTreeSet<String>) {
        match v {
            Value::Object(m) => {
                for (k, v) in m {
                    if k == "source_key" || k.ends_with("_source_key") || k == "review_handoff" {
                        if let Some(s) = v.as_str() {
                            out.insert(s.into());
                        }
                    } else if k == "source_keys" || k == "queue_source_keys" || k == "evidence" {
                        for key in rows(v).filter_map(Value::as_str) {
                            out.insert(key.into());
                        }
                    } else {
                        visit(v, out)
                    }
                }
            }
            Value::Array(a) => {
                for v in a {
                    visit(v, out)
                }
            }
            _ => {}
        }
    }
    let mut result = BTreeSet::new();
    visit(v, &mut result);
    result
}
fn number(v: &Value) -> Option<f64> {
    v.as_f64().filter(|n| n.is_finite())
}
#[allow(clippy::too_many_arguments)]
fn observation(
    id: &str,
    metric: &str,
    value: Option<f64>,
    limit: f64,
    low: bool,
    affected: Value,
    evidence: Value,
    question: &str,
) -> Value {
    let source_keys = keys(&evidence);
    json!({"detector_id":id,"metric":metric,"value":value,"threshold":limit,"comparison":if low{"<"}else{">"},
        "status":if value.is_some(){"observed"}else{"no_data"},"triggered":value.map(|n|if low{n<limit}else{n>limit}),
        "affected":affected,"source_keys":source_keys,"diagnostic_question":question,
        "severity":"warning","trace_metric_id":"anomaly_spotlights"})
}

/// Optional `supervisory_dispatch` is supplied by the portfolio layer from proven topology.
/// Missing topology/hold-file ingestion is never replaced with seat-name guesses.
pub fn evaluate(payload: &Value, options: &AdvisoryOptions) -> Value {
    let mut observations = vec![];
    let categories = &payload["tool_categories"];
    observations.push(observation(
        "A1",
        "fleet_ceremony_ratio",
        number(&categories["ceremony_ratio"]),
        options.ceremony_ratio,
        false,
        json!({"scope":"fleet"}),
        categories.clone(),
        "Which coordination and governance calls account for this ceremony share?",
    ));
    if let Some(by) = categories["by_seat"].as_object() {
        for (seat, row) in by {
            observations.push(observation(
                "A1",
                "seat_ceremony_ratio",
                number(&row["ceremony_ratio"]),
                options.ceremony_ratio,
                false,
                json!({"seat":seat}),
                row.clone(),
                "Which coordination calls dominate this seat's recorded work?",
            ));
        }
    }
    let concurrency = &payload["fleet_concurrency"];
    let activity = &payload["activity"];
    let known = rows(activity).any(|r| {
        r["active_secs"].as_i64().unwrap_or(0)
            + r["idle_with_work_secs"].as_i64().unwrap_or(0)
            + r["idle_without_work_secs"].as_i64().unwrap_or(0)
            + r["waiting_human_secs"].as_i64().unwrap_or(0)
            > 0
    });
    let active_seconds: f64 = rows(&concurrency["histogram"])
        .filter(|r| r["active_seats"].as_u64().unwrap_or(0) > 0)
        .filter_map(|r| number(&r["secs"]))
        .sum();
    let serial_seconds: f64 = rows(&concurrency["histogram"])
        .filter(|r| r["active_seats"] == 1)
        .filter_map(|r| number(&r["secs"]))
        .sum();
    observations.push(observation(
        "A2",
        "single_seat_share_of_active_window",
        (known && active_seconds > 0.0).then(|| serial_seconds / active_seconds),
        options.serialization_ratio,
        false,
        json!({"scope":"fleet"}),
        payload["activity_events"].clone(),
        "Which dependencies kept the fleet at one active seat?",
    ));
    let enough = concurrency["seat_count"]
        .as_u64()
        .is_some_and(|n| n >= options.minimum_seats as u64);
    observations.push(observation(
        "A2",
        "effective_parallelism",
        if known && enough {
            number(&concurrency["effective_parallelism"])
        } else {
            None
        },
        options.parallelism,
        true,
        json!({"scope":"fleet","minimum_seats":options.minimum_seats}),
        payload["activity_events"].clone(),
        "Which observed seats were available while parallelism stayed low?",
    ));
    for row in rows(activity) {
        let seat = row["seat"].as_str();
        let intervals: Vec<_> = rows(&payload["advisory_idle_claimed"]["intervals"])
            .filter(|i| {
                i["classification"] == "unparked_container_hold" && i["seat"].as_str() == seat
            })
            .cloned()
            .collect();
        let mut ranges: Vec<_> = intervals
            .iter()
            .filter_map(|i| {
                Some((
                    chrono::DateTime::parse_from_rfc3339(i["from"].as_str()?).ok()?,
                    chrono::DateTime::parse_from_rfc3339(i["to"].as_str()?).ok()?,
                ))
            })
            .collect();
        ranges.sort();
        let mut union: Vec<(
            chrono::DateTime<chrono::FixedOffset>,
            chrono::DateTime<chrono::FixedOffset>,
        )> = Vec::new();
        for (start, end) in ranges {
            if let Some(last) = union.last_mut() {
                if start <= last.1 {
                    last.1 = last.1.max(end);
                    continue;
                }
            }
            union.push((start, end));
        }
        let hold: f64 = union.iter().map(|(a, b)| (*b - *a).as_seconds_f64()).sum();
        let complete = payload["advisory_idle_claimed"]["coverage"] == "observed";
        let value = number(&row["idle_with_work_secs"])
            .filter(|n| *n > 0.0)
            .filter(|_| complete || hold > 0.0)
            .map(|n| hold / n);
        let mut o=observation("A3","strict_unparked_share_of_idle_with_work",value,options.container_ratio,false,json!({"seat":row["seat"]}),json!(intervals),"Which source-proven container claims should be inspected for parking while child work runs?");
        if !complete {
            o["status"] = json!("partial");
            if o["triggered"] != true {
                o["triggered"] = Value::Null;
            }
        }
        observations.push(o);
    }
    let recovery = &payload["advisory_queue_volume"];
    let total = number(&recovery["items"]);
    observations.push(observation(
        "A3",
        "recovery_item_volume_ratio",
        total
            .filter(|n| *n > 0.0)
            .zip(number(&recovery["recovery_items"]))
            .map(|(all, recovery)| recovery / all),
        options.recovery_ratio,
        false,
        json!({"scope":"fleet"}),
        recovery.clone(),
        "Which recorded recovery reasons dominate queue volume?",
    ));
    let compactions = &payload["context_compactions"];
    let compaction_events: Vec<_> = rows(&compactions["events"])
        .filter(|e| e["hazard_class"] == "mid_item_hazard")
        .collect();
    if compaction_events.is_empty() {
        observations.push(observation(
            "A4",
            "mid_item_context_drop_ratio",
            (payload["usage_rows"].as_u64().is_some_and(|n| n > 1)
                && compactions["mid_item_hazard_count"] == 0)
                .then_some(0.0),
            options.context_drop_ratio,
            false,
            json!({"scope":"fleet"}),
            Value::Null,
            "Which issue transitions preceded a recorded mid-item context drop?",
        ));
    }
    for event in compaction_events {
        observations.push(observation("A4","mid_item_context_drop_ratio",number(&event["drop_ratio"]),options.context_drop_ratio,false,json!({"seat":event["seat"],"session":event["session"],"qitem_id":event["active_item_id"]}),event.clone(),"Which active item and context transition preceded this mid-item drop?"));
    }
    let bleed_events: Vec<_> = rows(&payload["cross_slice_cache_bleed"]["events"]).collect();
    if bleed_events.is_empty() {
        observations.push(observation(
            "A4",
            "cross_slice_inherited_cache_tokens",
            (payload["usage_rows"].as_u64().is_some_and(|n| n > 1)
                && payload["cross_slice_cache_bleed"]["total_inherited_entry_tokens"] == 0)
                .then_some(0.0),
            options.cache_bleed_tokens as f64,
            false,
            json!({"scope":"fleet"}),
            Value::Null,
            "Which issue boundary inherited the recorded cache-read context?",
        ));
    }
    for event in bleed_events {
        observations.push(observation("A4","cross_slice_inherited_cache_tokens",number(&event["inherited_cache_read_tokens"]),options.cache_bleed_tokens as f64,false,json!({"seat":event["seat"],"session":event["session"],"from_issue":event["from_slice"],"to_issue":event["to_slice"]}),event.clone(),"Which issue boundary inherited stale cache and what recorded reuse followed?"));
    }
    let duty: BTreeMap<_, _> = rows(activity)
        .filter_map(|r| {
            let seat = r["seat"].as_str()?;
            let observed = number(&r["observed_secs"])?;
            let unknown = number(&r["unknown_secs"])?;
            (observed > 0.0 && unknown == 0.0)
                .then(|| (seat, number(&r["active_secs"]).map(|n| n / observed)))
        })
        .collect();
    if options.builder_seats.is_empty() {
        observations.push(observation(
            "A5",
            "builder_duty_ratio",
            None,
            options.builder_duty_ratio,
            true,
            json!({"scope":"configured_builders","reason":"no_configured_builder_roster"}),
            Value::Null,
            "Which configured builder tiers are idle while peers are saturated?",
        ));
    }
    for builder in &options.builder_seats {
        let peer_busy = options
            .builder_seats
            .iter()
            .filter(|p| *p != builder)
            .filter_map(|p| duty.get(p.as_str()).copied().flatten())
            .any(|d| d > options.peer_saturation_ratio);
        let value = duty.get(builder.as_str()).copied().flatten();
        let peers_known = options
            .builder_seats
            .iter()
            .filter(|p| *p != builder)
            .any(|p| duty.get(p.as_str()).copied().flatten().is_some());
        let mut duty_observation = observation(
            "A5",
            "builder_duty_ratio",
            value,
            options.builder_duty_ratio,
            true,
            json!({"seat":builder,"peer_saturation_threshold":options.peer_saturation_ratio}),
            activity.clone(),
            "Why is this configured builder tier idle while peer builders are saturated?",
        );
        if !peers_known {
            duty_observation["status"] = json!("partial");
        }
        duty_observation["peer_saturated"] = if peers_known {
            json!(peer_busy)
        } else {
            Value::Null
        };
        if !peer_busy {
            duty_observation["triggered"] = if value.is_some() && peers_known {
                json!(false)
            } else {
                Value::Null
            };
        }
        observations.push(duty_observation);
    }
    // This is a source-complete observed seat-window scope, never a causal turn join.
    let routing: Vec<_> = rows(&payload["routing_effort_evidence"])
        .filter(|e| e["kind"] == "routing_only_observed_seat_window" && e["coverage"] == "complete")
        .collect();
    if routing.is_empty() {
        observations.push(observation("A5","xhigh_routing_only_observed_seat_window_tokens",(payload["routing_effort_status"]=="observed").then_some(0.0),0.0,false,json!({"scope":"routing_only_observed_seat_window","reason":"requires_complete_observed_seat_window_producer"}),Value::Null,"Which source-complete observed routing-only seat windows used xhigh effort?"));
    }
    for event in routing {
        observations.push(observation("A5","xhigh_routing_only_observed_seat_window_tokens",if event["effort"]=="xhigh"{number(&event["tokens"])}else{Some(0.0)},0.0,false,json!({"seat":event["seat"],"scope":"routing_only_observed_seat_window","causal_turn_attribution":false}),event.clone(),"Why did this recorded routing-only seat window use xhigh effort? This does not identify a causal routing turn."));
    }
    let checks: Vec<_> = rows(&payload["four_eyes_independence"]["checks"]).collect();
    if checks.is_empty() {
        observations.push(observation(
            "A5",
            "same_family_review",
            None,
            0.0,
            false,
            json!({"scope":"fleet"}),
            Value::Null,
            "Which build and review witnesses establish model-family diversity?",
        ));
    }
    for check in checks {
        observations.push(observation(
            "A5",
            "same_family_review",
            match check["status"].as_str() {
                Some("same_family") => Some(1.0),
                Some("independent") => Some(0.0),
                _ => None,
            },
            0.0,
            false,
            json!({"builder":check["builder"],"reviewer":check["reviewer"],"chain":check["chain"]}),
            check.clone(),
            "Would a different model family add independent scrutiny on this recorded chain?",
        ));
    }
    let dispatches: Vec<_> = rows(&payload["supervisory_dispatch"]).collect();
    if dispatches.is_empty() {
        observations.push(observation(
            "A6",
            "supervisory_bypass_dispatches",
            (payload["supervisory_dispatch_status"] == "observed").then_some(0.0),
            0.0,
            false,
            json!({"reason":"no_authoritative_portfolio_dispatch_evidence"}),
            Value::Null,
            "Which external dispatches have proven supervisory and builder roles?",
        ));
    }
    for dispatch in dispatches {
        if dispatch["classification"] == "dead_supervisory_relay" {
            observations.push(observation("A6","failed_wake_pending_duration_secs",number(&dispatch["pending_duration_secs"]),options.detached_target_secs as f64,false,json!({"seat":dispatch["dest_seat"]}),dispatch.clone(),"Which recorded supervisory route left this item pending with failed wake evidence?"));
        }
        if dispatch["classification"] == "proven_bypass" {
            observations.push(observation(
                "A6",
                "supervisory_bypass_dispatches",
                Some(1.0),
                0.0,
                false,
                json!({"seat":dispatch["dest_seat"],"source_seat":dispatch["source_seat"]}),
                dispatch.clone(),
                "Why did this proven external dispatch bypass its coordinator?",
            ));
        }
    }
    let detached: Vec<_> = rows(&payload["advisory_idle_claimed"]["intervals"])
        .filter(|f| f["classification"] == "detached_target_wake_failure")
        .collect();
    if detached.is_empty() {
        observations.push(observation(
            "A6",
            "failed_wake_pending_duration_secs",
            (payload["advisory_idle_claimed"]["wake_coverage"] == "observed").then_some(0.0),
            options.detached_target_secs as f64,
            false,
            json!({"scope":"fleet"}),
            Value::Null,
            "Which pending target has recorded failed delivery and lifecycle evidence?",
        ));
    }
    for finding in detached {
        observations.push(observation(
            "A6",
            "failed_wake_pending_duration_secs",
            number(&finding["duration_secs"]),
            options.detached_target_secs as f64,
            false,
            json!({"seat":finding["seat"],"qitem_id":finding["qitem_id"]}),
            finding.clone(),
            "Which transport route or stopped target explains this undelivered wake?",
        ));
    }
    let detectors:Vec<_>=(1..=6).map(|n|{let id=format!("A{n}");let mine:Vec<_>=observations.iter().filter(|o|o["detector_id"]==id).collect();
        let known=mine.iter().filter(|o|o["status"]=="observed").count();json!({"detector_id":id,"coverage":if known==0{"no_data"}else if known<mine.len(){"partial"}else{"observed"},"observations":mine})}).collect();
    let findings: Vec<_> = observations
        .iter()
        .filter(|o| o["triggered"] == true)
        .cloned()
        .collect();
    json!({"definition":"anomaly-spotlights-v1","producer_floor":{"context_drop_ratio":0.5,"minimum_dropped_tokens":50000},"heading":"Fleet Anomalies & Diagnostic Spotlights","thresholds":options,"detectors":detectors,"findings":findings,
        "rework_context":{"formula_version":payload["handoff_friction"]["formula_version"],"vortex_loops":payload["handoff_friction"]["vortex_loops"],"trace_metric_id":"handoff_friction","scope":"canonical overlapping witnesses; never sum loop totals"},
        "scope":"deterministic diagnostics over canonical retained evidence; no causation, financial extrapolation, model calls or runtime intervention"})
}

/// Commands are represented as argv for lossless replay; shell text is display only.
pub fn attach_drilldown(
    value: &mut Value,
    db: &str,
    rig: Option<&str>,
    from: &str,
    to: &str,
    metric: &str,
) {
    let mut argv = vec![
        "dendro".to_owned(),
        "trace".into(),
        "--metric-id".into(),
        metric.into(),
        "--db".into(),
        crate::redact(db),
        "--from".into(),
        from.into(),
        "--to".into(),
        to.into(),
    ];
    if let Some(rig) = rig {
        argv.extend(["--rig".into(), crate::redact(rig)]);
    }
    for (field, flag) in [
        ("container_hold_secs", "--advisory-container-hold-secs"),
        ("leaf_stall_secs", "--advisory-leaf-stall-secs"),
        ("detached_target_secs", "--advisory-detached-target-secs"),
        ("ceremony_ratio", "--spotlight-ceremony-ratio"),
        ("serialization_ratio", "--spotlight-serialization-ratio"),
        ("parallelism", "--spotlight-parallelism"),
        ("minimum_seats", "--spotlight-minimum-seats"),
        ("container_ratio", "--spotlight-container-ratio"),
        ("recovery_ratio", "--spotlight-recovery-ratio"),
        ("context_drop_ratio", "--spotlight-context-drop-ratio"),
        ("cache_bleed_tokens", "--spotlight-cache-bleed-tokens"),
        ("builder_duty_ratio", "--spotlight-builder-duty-ratio"),
        ("peer_saturation_ratio", "--spotlight-peer-saturation-ratio"),
    ] {
        if let Some(n) = value["thresholds"].get(field).filter(|v| v.is_number()) {
            argv.extend([flag.into(), n.to_string()]);
        }
    }
    let builders: Vec<_> = rows(&value["thresholds"]["builder_seats"])
        .filter_map(Value::as_str)
        .collect();
    if !builders.is_empty() {
        argv.extend(["--spotlight-builder-seats".into(), builders.join(",")]);
    }
    let command = argv
        .iter()
        .map(|a| format!("'{}'", a.replace('\'', "'\\''")))
        .collect::<Vec<_>>()
        .join(" ");
    for f in value["findings"].as_array_mut().into_iter().flatten() {
        f["drilldown_argv"] = json!(argv);
        f["drilldown_command"] = json!(command);
    }
}

pub fn markdown(value: &Value) -> String {
    let advisory = value["definition"] == "advisory-idle-claimed-v1";
    let mut out = if advisory {
        String::from("## Idle With Claimed Work Advisories\n\n")
    } else {
        String::from("## Fleet Anomalies & Diagnostic Spotlights\n\n")
    };
    for f in rows(&value["findings"]) {
        out.push_str(&format!(
            "- {} / {}: observed {}, threshold {} {}. {}\n  Trace: `{}`\n",
            f["detector_id"].as_str().unwrap_or("advisory"),
            f["metric"]
                .as_str()
                .or_else(|| f["classification"].as_str())
                .unwrap_or("unknown"),
            if advisory {
                &f["duration_secs"]
            } else {
                &f["value"]
            },
            f["comparison"].as_str().unwrap_or(">"),
            if advisory {
                &f["threshold_secs"]
            } else {
                &f["threshold"]
            },
            f["diagnostic_question"]
                .as_str()
                .or_else(|| f["recommendation"].as_str())
                .unwrap_or("Inspect retained evidence."),
            f["drilldown_command"].as_str().unwrap_or("")
        ));
    }
    if rows(&value["findings"]).next().is_none() {
        out.push_str("No established threshold breaches in retained evidence. Inspect detector coverage before interpreting this absence.\n");
    }
    out
}

pub fn assertion(payload: &Value, max_rework: f64, max_idle: f64) -> Value {
    let rework = if payload["m3_unknown_chains"].as_u64().is_none_or(|n| n > 0) {
        None
    } else {
        number(&payload["m3_rework_proportion"])
    };
    let mut idle = 0.0;
    let mut observed = 0.0;
    let mut unknown = false;
    for seat in rows(&payload["activity"]) {
        idle += number(&seat["idle_with_work_secs"]).unwrap_or(0.0);
        observed += number(&seat["observed_secs"]).unwrap_or(0.0);
        unknown |= number(&seat["unknown_secs"]).is_none_or(|n| n > 0.0);
    }
    let idle_rate = (observed > 0.0 && !unknown).then(|| idle / observed);
    let breach = rework.is_some_and(|n| n > max_rework) || idle_rate.is_some_and(|n| n > max_idle);
    let complete = rework.is_some() && idle_rate.is_some();
    json!({"definition":"operator-assertion-v1","status":if breach{"breach"}else if complete{"pass"}else{"insufficient_evidence"},"exit_code":if breach{2}else if complete{0}else{3},
        "rework_rate":rework,"max_rework_rate":max_rework,"idle_with_work_rate":idle_rate,"max_idle_rate":max_idle,
        "locator_summary":locator_summary(&json!({"m3":payload["m3_rework_members"],"activity":payload["activity_events"]})),"advisory_only":true})
}

fn locator_summary(value: &Value) -> Value {
    use sha2::{Digest, Sha256};
    let all = keys(value);
    let fingerprints: Vec<_> = all
        .iter()
        .take(20)
        .map(|key| format!("sha256:{:x}", Sha256::digest(key.as_bytes())))
        .collect();
    json!({"locator_fingerprints":fingerprints,"total":all.len(),"omitted":all.len().saturating_sub(20),"scope":"bounded SHA256 locators; use local trace to resolve original source keys"})
}

pub fn ndjson_records(value: &Value) -> Vec<Value> {
    let mut coverage = json!({"record_type":"coverage","definition":value["definition"]});
    if let Some(v) = value.get("coverage") {
        coverage["coverage"] = v.clone();
    }
    if let Some(v) = value.get("detectors") {
        coverage["detectors"] = v.clone();
    }
    let mut records = vec![coverage];
    records.extend(rows(&value["findings"]).map(|f| json!({"record_type":"finding","finding":f})));
    records
}

pub fn issue_draft(
    payload: &Value,
    metric: &str,
    price: Option<f64>,
) -> Result<String, anyhow::Error> {
    let selected = match metric {
        "m3" => {
            json!({"value":payload["m3_rework_proportion"],"unknown_chains":payload["m3_unknown_chains"],"members":payload["m3_rework_members"],"unknown_members":payload["m3_unknown_members"]})
        }
        "handoff_friction" => payload["handoff_friction"].clone(),
        "advisory_idle_claimed" => payload["advisory_idle_claimed"].clone(),
        "anomaly_spotlights" => payload["anomaly_spotlights"].clone(),
        _ => return Err(anyhow::anyhow!("unsupported issue-draft metric")),
    };
    if price.is_some_and(|n| !n.is_finite() || n < 0.0) {
        return Err(anyhow::anyhow!(
            "token price must be finite and nonnegative"
        ));
    }
    let pairs: Vec<_> = rows(&payload["handoff_friction"]["pairs"]).collect();
    let measured = payload["handoff_friction"]["assigned_usage_tokens"]
        .as_object()
        .and_then(|assigned| {
            assigned
                .values()
                .try_fold(0u64, |sum, n| sum.checked_add(n.as_u64()?))
        })
        .filter(|n| *n > 0 || payload["handoff_friction"]["coverage"] == "complete");
    let cost = measured
        .zip(price)
        .map(|(n, p)| (n as f64 / 1_000_000.0) * p);
    if cost.is_some_and(|n| !n.is_finite()) {
        return Err(anyhow::anyhow!("financial calculation is out of bounds"));
    }
    let summary=match metric{
        "m3"=>format!("Observed M3 rework proportion: {}. Unresolved chains: {}.",number(&payload["m3_rework_proportion"]).map(|v|v.to_string()).unwrap_or_else(||"unknown".into()),payload["m3_unknown_chains"].as_u64().map(|v|v.to_string()).unwrap_or_else(||"unknown".into())),
        "handoff_friction"=>format!("Observed pairs: {}. Closed loop witnesses: {}. Coverage: {}.",pairs.len(),rows(&selected["vortex_loops"]).count(),selected["coverage"].as_str().unwrap_or("unknown")),
        _=>format!("{} established findings. Coverage: {}. Findings are diagnostic investigations, not causal savings claims.",rows(&selected["findings"]).count(),selected["coverage"].as_str().unwrap_or("inspect per-detector coverage")),
    };
    let mut out=format!("# Investigate observed fleet {metric}\n\n## Evidence\n\nWindow: {} to {} (half-open).\n\n{summary}\n\nHandoff-friction rework-attributed token lower bound: {}.\n\n",payload["from"].as_str().unwrap_or("unknown"),payload["to"].as_str().unwrap_or("unknown"),measured.map(|n|n.to_string()).unwrap_or_else(||"unknown (overlap, missing assignment keys or incomplete zero)".into()));
    for finding in rows(&selected["findings"]).take(10) {
        let label = finding["classification"]
            .as_str()
            .or_else(|| finding["metric"].as_str())
            .unwrap_or("diagnostic");
        let safe: String = crate::redact(label).chars().take(120).collect();
        out.push_str(&format!("- {safe}\n"));
    }
    if let Some(cost) = cost {
        out.push_str(&format!("\nObserved-window attributed token cost lower bound: USD {cost:.6}, using operator price USD {:.6} per million tokens. No claim of causal waste. No monthly extrapolation.\n",price.unwrap()));
    } else {
        out.push_str("\nFinancial impact: unknown; requires globally unique attributed usage and explicit pricing. No monthly extrapolation.\n");
    }
    out.push_str(&format!("\n## Proposed investigation\n\nCompare the same metric and scope in a later window. Local drilldown: `dendro trace --metric-id {} --db <local-analytics-db>`.\n\n## Bounded source fingerprints\n\n```json\n{}\n```\n",if metric=="m3"{"m3"}else{metric},serde_json::to_string_pretty(&locator_summary(&selected))?));
    Ok(crate::redact(&out))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rich() -> Value {
        json!({"tool_categories":{"ceremony_ratio":0.5,"categories":{"coord_queue":{"source_keys":["tool:1"]}}},
        "activity":[{"seat":"junior","observed_secs":100,"active_secs":1,"idle_with_work_secs":80,"idle_without_work_secs":19,"waiting_human_secs":0,"unknown_secs":0,"unparked_container_secs":60,"container_hold_evidence":[{"source_key":"claim:1"}]},{"seat":"senior","observed_secs":100,"active_secs":90,"idle_with_work_secs":10,"idle_without_work_secs":0,"waiting_human_secs":0,"unknown_secs":0}],
        "fleet_concurrency":{"seat_count":4,"effective_parallelism":1.0,"histogram":[{"active_seats":1,"secs":90},{"active_seats":2,"secs":10}]},"activity_events":[{"source_key":"activity:1"}],
        "advisory_queue_volume":{"items":10,"recovery_items":2,"source_keys":["recovery:1"]},
        "context_compactions":{"events":[{"hazard_class":"mid_item_hazard","drop_ratio":0.6,"seat":"senior","pre_source_key":"usage:1","post_source_key":"usage:2"}]},
        "cross_slice_cache_bleed":{"events":[{"seat":"senior","inherited_cache_read_tokens":100001,"source_keys":["usage:3"]}]},
        "four_eyes_independence":{"checks":[{"status":"same_family","builder":"senior","reviewer":"review","source_key":"handoff:1"}]},
        "supervisory_dispatch":[{"classification":"proven_bypass","source_seat":"steer","dest_seat":"senior","source_key":"dispatch:1"}],
        "advisory_idle_claimed":{"findings":[{"classification":"detached_target_wake_failure","seat":"relay","duration_secs":301,"source_keys":["wake:1"]}]},"m3_rework_proportion":0.2,"m3_unknown_chains":0})
    }
    #[test]
    fn all_detectors_trigger_over_canonical_evidence_and_absence_is_no_data() {
        let options = AdvisoryOptions {
            builder_seats: vec!["junior".into(), "senior".into()],
            ..AdvisoryOptions::default()
        };
        let v = evaluate(&rich(), &options);
        let ids: BTreeSet<_> = rows(&v["findings"])
            .filter_map(|r| r["detector_id"].as_str())
            .collect();
        assert_eq!(ids, BTreeSet::from(["A1", "A2", "A3", "A4", "A5", "A6"]));
        let empty = evaluate(&json!({}), &options);
        assert!(rows(&empty["findings"]).next().is_none());
        assert!(rows(&empty["detectors"]).all(|d| d["coverage"] == "no_data"));
    }
    #[test]
    fn strict_boundary_and_operator_overrides_are_effective() {
        let mut p = rich();
        p["tool_categories"]["ceremony_ratio"] = json!(0.4);
        p["advisory_queue_volume"]["recovery_items"] = json!(1);
        let v = evaluate(&p, &AdvisoryOptions::default());
        assert!(!rows(&v["findings"]).any(|f| f["detector_id"] == "A1"));
        assert!(!rows(&v["findings"]).any(|f| f["metric"] == "recovery_item_volume_ratio"));
        let options = AdvisoryOptions {
            ceremony_ratio: 0.39,
            ..AdvisoryOptions::default()
        };
        assert!(rows(&evaluate(&p, &options)["findings"]).any(|f| f["detector_id"] == "A1"));
    }
    #[test]
    fn drilldown_keeps_window_and_operator_thresholds_and_quotes_paths() {
        let options = AdvisoryOptions {
            ceremony_ratio: 0.01,
            ..AdvisoryOptions::default()
        };
        let mut v = evaluate(&rich(), &options);
        attach_drilldown(
            &mut v,
            "a 'quoted' db",
            Some("rig"),
            "from",
            "to",
            "anomaly_spotlights",
        );
        let argv = v["findings"][0]["drilldown_argv"].as_array().unwrap();
        assert!(argv.contains(&json!("a 'quoted' db")));
        assert!(argv.contains(&json!("--spotlight-ceremony-ratio")));
        assert!(argv.contains(&json!("0.01")));
    }
    #[test]
    fn assertions_have_separate_pass_breach_and_unknown_statuses() {
        let mut p = rich();
        assert_eq!(assertion(&p, 0.15, 0.2)["exit_code"], 2);
        p["m3_rework_proportion"] = json!(0.15);
        assert_eq!(assertion(&p, 0.15, 0.5)["exit_code"], 0);
        p["m3_unknown_chains"] = json!(1);
        assert_eq!(assertion(&p, 0.15, 0.5)["exit_code"], 3);
        assert_eq!(assertion(&json!({}), 0.15, 0.2)["exit_code"], 3);
    }
    #[test]
    fn draft_is_local_markdown_and_price_is_explicit_without_monthly_claim() {
        let mut p = rich();
        p["handoff_friction"] = json!({"pairs":[{"rework_tokens":1000000,"assigned_usage_keys":["unique"]}],"assigned_usage_tokens":{"unique":1000000},"coverage":"complete"});
        let no_price = issue_draft(&p, "m3", None).unwrap();
        assert!(no_price.starts_with("# Investigate"));
        assert!(no_price.contains("Financial impact: unknown"));
        let priced = issue_draft(&p, "m3", Some(3.0)).unwrap();
        assert!(priced.contains("USD 3.000000"));
        assert!(priced.contains("No monthly extrapolation"));
        assert!(issue_draft(&p, "m3", Some(f64::NAN)).is_err());
        p["handoff_friction"] = json!({"pairs":[{"rework_tokens":0,"coverage":"no_data"}]});
        assert!(issue_draft(&p, "m3", Some(3.0))
            .unwrap()
            .contains("Financial impact: unknown"));
    }
    #[test]
    fn strict_container_intervals_ignore_legacy_shared_tag_hold_values() {
        let mut p = rich();
        p["advisory_queue_volume"] = json!({"items":10,"recovery_items":0});
        p["advisory_idle_claimed"] = json!({"coverage":"observed","intervals":[]});
        assert!(
            !rows(&evaluate(&p, &AdvisoryOptions::default())["findings"])
                .any(|f| f["detector_id"] == "A3")
        );
        p["advisory_idle_claimed"]["intervals"] = json!([{"classification":"unparked_container_hold","seat":"junior","from":"2026-10-15T00:00:00Z","to":"2026-10-15T00:01:00Z","source_keys":["strict-parent","strict-child"]}]);
        assert!(rows(&evaluate(&p, &AdvisoryOptions::default())["findings"])
            .any(|f| f["metric"] == "strict_unparked_share_of_idle_with_work"));
    }
    #[test]
    fn canonical_union_prices_shared_pair_usage_once_and_drafts_actual_metric() {
        let mut p = rich();
        p["from"] = json!("2026-10-15T00:00:00Z");
        p["to"] = json!("2026-10-15T01:00:00Z");
        p["handoff_friction"] = json!({"coverage":"complete","assigned_usage_tokens":{"shared":7},"pairs":[{"rework_tokens":7,"assigned_usage_keys":["shared"]},{"rework_tokens":7,"assigned_usage_keys":["shared"]}]});
        let draft = issue_draft(&p, "handoff_friction", Some(1_000_000.0)).unwrap();
        assert!(draft.contains("USD 7.000000"));
        assert!(!draft.contains("USD 14.000000"));
        p["advisory_idle_claimed"] = json!({"coverage":"partial","findings":[{"classification":"true_leaf_stall","source_keys":["private-session-sk-test-secret"]}]});
        let draft = issue_draft(&p, "advisory_idle_claimed", None).unwrap();
        assert!(draft.contains("true_leaf_stall"));
        assert!(!draft.contains("Observed M3"));
        assert!(!draft.contains("private-session"));
        assert!(!draft.contains("sk-test-secret"));
        p.as_object_mut().unwrap().remove("m3_unknown_chains");
        assert_eq!(assertion(&p, 1.0, 1.0)["exit_code"], 3);
    }
    #[test]
    fn known_negative_evidence_is_observed_and_low_context_override_is_rejected() {
        let mut p = rich();
        p["usage_rows"] = json!(2);
        p["context_compactions"] = json!({"events":[],"mid_item_hazard_count":0});
        p["cross_slice_cache_bleed"] = json!({"events":[],"total_inherited_entry_tokens":0});
        p["advisory_idle_claimed"] = json!({"wake_coverage":"observed","intervals":[]});
        let v = evaluate(
            &p,
            &AdvisoryOptions {
                builder_seats: vec!["junior".into(), "senior".into()],
                peer_saturation_ratio: 1.0,
                ..AdvisoryOptions::default()
            },
        );
        for detector in rows(&v["detectors"]) {
            for observation in rows(&detector["observations"]) {
                if [
                    "mid_item_context_drop_ratio",
                    "cross_slice_inherited_cache_tokens",
                    "failed_wake_pending_duration_secs",
                ]
                .contains(&observation["metric"].as_str().unwrap_or(""))
                {
                    assert_eq!(observation["value"], 0.0);
                    assert_eq!(observation["status"], "observed");
                }
            }
        }
        assert!(AdvisoryOptions {
            context_drop_ratio: 0.49,
            ..AdvisoryOptions::default()
        }
        .validate()
        .is_err());
    }
}
