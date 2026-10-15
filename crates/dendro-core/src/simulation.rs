//! Deterministic historical what-ifs. Assumptions never become observed causal savings.
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

fn one() -> f64 {
    1.0
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CostWeights {
    pub uncached: f64,
    pub cache_read: f64,
}
impl Default for CostWeights {
    fn default() -> Self {
        Self {
            uncached: 1.0,
            cache_read: 0.1,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TierRule {
    pub seat: String,
    #[serde(default = "one")]
    pub uncached_multiplier: f64,
    #[serde(default = "one")]
    pub cache_read_multiplier: f64,
    pub work_class: Option<String>,
    pub model_family: Option<String>,
    pub effort: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoundaryModel {
    pub turns: u64,
    pub stale_context_tokens: u64,
    pub clean_context_tokens: u64,
    pub cold_boot_tokens: u64,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Scenario {
    pub remove: Vec<String>,
    pub replacement_seat: Option<String>,
    pub direct_handoffs: bool,
    pub tier_policy: Vec<TierRule>,
    pub boundary_reset_threshold: Option<u64>,
    pub weights: CostWeights,
    pub boundary: Option<BoundaryModel>,
    pub enforce_model_family_diversity: bool,
}
#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq, Eq)]
struct Tokens {
    input_uncached: i64,
    output: i64,
    cache_read: i64,
    cache_write: i64,
}
impl Tokens {
    fn from_value(v: &Value) -> Option<Self> {
        let read = |field: &str| v[field].as_i64().filter(|n| *n >= 0);
        Some(Self {
            input_uncached: read("input_uncached")?,
            output: read("output")?,
            cache_read: read("cache_read")?,
            cache_write: read("cache_write")?,
        })
    }
    fn add(&mut self, other: Self) -> Result<()> {
        self.input_uncached = self
            .input_uncached
            .checked_add(other.input_uncached)
            .ok_or_else(|| anyhow::anyhow!("token aggregate overflow"))?;
        self.output = self
            .output
            .checked_add(other.output)
            .ok_or_else(|| anyhow::anyhow!("token aggregate overflow"))?;
        self.cache_read = self
            .cache_read
            .checked_add(other.cache_read)
            .ok_or_else(|| anyhow::anyhow!("token aggregate overflow"))?;
        self.cache_write = self
            .cache_write
            .checked_add(other.cache_write)
            .ok_or_else(|| anyhow::anyhow!("token aggregate overflow"))?;
        Ok(())
    }
    fn weighted(self, w: &CostWeights) -> Result<f64> {
        finite(self.input_uncached as f64 * w.uncached + self.cache_read as f64 * w.cache_read)
    }
}
fn finite(value: f64) -> Result<f64> {
    if value.is_finite() && value >= 0.0 {
        Ok(value)
    } else {
        bail!("numeric input or projected result must be finite and nonnegative")
    }
}
fn signed(value: f64) -> Result<f64> {
    if value.is_finite() {
        Ok(value)
    } else {
        bail!("projected numeric result overflow")
    }
}
fn no_data(reason: &str) -> Value {
    json!({"status":"no_data","reason":reason})
}
fn validate(s: &Scenario) -> Result<()> {
    finite(s.weights.uncached)?;
    finite(s.weights.cache_read)?;
    let removed: BTreeSet<_> = s.remove.iter().collect();
    if removed.iter().any(|seat| seat.trim().is_empty()) {
        bail!("removed seat must not be empty");
    }
    if s.replacement_seat
        .as_ref()
        .is_some_and(|seat| seat.trim().is_empty() || removed.contains(seat))
    {
        bail!("replacement must be nonempty and cannot itself be removed");
    }
    let mut seats = BTreeSet::new();
    for rule in &s.tier_policy {
        if rule.seat.trim().is_empty() || !seats.insert(&rule.seat) {
            bail!("tier policy seat must be nonempty and unique");
        }
        finite(rule.uncached_multiplier)?;
        finite(rule.cache_read_multiplier)?;
        if rule
            .work_class
            .as_deref()
            .is_some_and(|v| !matches!(v, "L1" | "P0" | "P1" | "P2" | "P3"))
        {
            bail!("unsupported tier work_class");
        }
        if rule
            .model_family
            .as_ref()
            .is_some_and(|v| v.trim().is_empty())
            || rule.effort.as_ref().is_some_and(|v| v.trim().is_empty())
        {
            bail!("tier model family and effort labels must not be empty");
        }
    }
    Ok(())
}

/// A standalone boundary cost comparison, independent of historical usage projections.
pub fn boundary_cost(model: &BoundaryModel, weights: &CostWeights) -> Result<Value> {
    finite(weights.uncached)?;
    finite(weights.cache_read)?;
    let warm = finite(model.turns as f64 * model.stale_context_tokens as f64 * weights.cache_read)?;
    let cold_boot = finite(model.cold_boot_tokens as f64 * weights.uncached)?;
    let reset = finite(
        cold_boot + model.turns as f64 * model.clean_context_tokens as f64 * weights.cache_read,
    )?;
    let improvement = model.stale_context_tokens as f64 - model.clean_context_tokens as f64;
    let per_turn = improvement * weights.cache_read;
    let break_even = if per_turn > 0.0 {
        Some(finite(cold_boot / per_turn)?)
    } else {
        None
    };
    let first_saving = break_even.map(|v| v.floor() + 1.0);
    let reduction = signed(warm - reset)?;
    let ratio = if warm > 0.0 {
        Some(signed(reduction / warm)?)
    } else {
        None
    };
    Ok(
        json!({"status":"projected","warm_input_units":warm,"boundary_input_units":reset,
        "reduction_input_units":reduction,"reduction_ratio":ratio,
        "break_even_turns":break_even,"first_strictly_cheaper_integer_turn":first_saving,
        "assumptions":model,"weights":weights,"unit":"uncached-equivalent input units, not currency",
        "limit":"Independent scenario; never add this reduction to tier/routing reductions without disjoint source evidence."}),
    )
}
struct Usage {
    seat: Option<String>,
    tokens: Tokens,
}
struct Routing {
    value: Value,
    usage: BTreeSet<String>,
    keys: BTreeSet<String>,
    available: bool,
}
fn direct_routing(p: &Value, usage: &BTreeMap<String, Usage>) -> Result<Routing> {
    if p["handoff_friction"]["formula_version"] != "handoff-friction-v1" {
        return Ok(Routing {
            value: no_data("unsupported canonical handoff formula"),
            usage: BTreeSet::new(),
            keys: BTreeSet::new(),
            available: false,
        });
    }
    let mut leaves = BTreeMap::<String, BTreeSet<String>>::new();
    if let Some(checks) = p["four_eyes_independence"]["checks"].as_array() {
        for check in checks {
            if check["source_key"].as_str().is_none_or(str::is_empty)
                || check["review_handoff"].as_str().is_none_or(str::is_empty)
            {
                continue;
            }
            for field in ["builder", "reviewer"] {
                if let Some(seat) = check[field].as_str() {
                    let keys = leaves.entry(seat.to_owned()).or_default();
                    for name in ["source_key", "review_handoff"] {
                        if let Some(key) = check[name].as_str() {
                            keys.insert(key.to_owned());
                        }
                    }
                }
            }
        }
    }
    if leaves.len() < 2 {
        return Ok(Routing {
            value: no_data("no canonical verified leaf-role evidence"),
            usage: BTreeSet::new(),
            keys: BTreeSet::new(),
            available: false,
        });
    }
    let pairs = p["handoff_friction"]["pairs"].as_array();
    let Some(pairs) = pairs else {
        return Ok(Routing {
            value: no_data("no canonical handoff pair samples"),
            usage: BTreeSet::new(),
            keys: BTreeSet::new(),
            available: false,
        });
    };
    let mut samples = Vec::new();
    for pair in pairs {
        if let Some(rows) = pair["samples"].as_array() {
            for sample in rows {
                samples.push((pair, sample));
            }
        }
    }
    let mut keys = BTreeSet::new();
    let mut bypassed_usage = BTreeSet::new();
    let mut paths = Vec::new();
    let mut total_ms = 0.0;
    let mut used_hops = BTreeSet::new();
    for (pair, first) in &samples {
        if pair["ring_boundary"] != "in_ring"
            || pair["identity_evidence"] != "proven"
            || pair["coverage"] != "complete"
        {
            continue;
        }
        let (Some(from), Some(mid), Some(child), Some(key)) = (
            first["from"].as_str(),
            first["to"].as_str(),
            first["receiver_qitem_id"].as_str(),
            first["source_key"].as_str(),
        ) else {
            continue;
        };
        let coordinator = mid
            .split('@')
            .next()
            .unwrap_or(mid)
            .replace('.', "-")
            .starts_with("orch-");
        if !coordinator
            || !leaves.contains_key(from)
            || first["lineage_status"] != "complete"
            || first["recovery"] != false
        {
            continue;
        }
        let matches: Vec<_> = samples
            .iter()
            .filter(|(_, second)| {
                second["qitem_id"] == child
                    && second["from"] == mid
                    && second["root"] == first["root"]
            })
            .collect();
        if matches.len() != 1 {
            continue;
        }
        let (second_pair, second) = matches[0];
        if second_pair["ring_boundary"] != "in_ring"
            || second_pair["identity_evidence"] != "proven"
            || second_pair["coverage"] != "complete"
        {
            continue;
        }
        let (Some(to), Some(next_key)) = (second["to"].as_str(), second["source_key"].as_str())
        else {
            continue;
        };
        if from == to
            || !leaves.contains_key(to)
            || second["lineage_status"] != "complete"
            || second["recovery"] != false
            || used_hops.contains(key)
            || used_hops.contains(next_key)
        {
            continue;
        }
        let (Some(wait), Some(exec)) = (
            pair["wait_median_ms"].as_f64(),
            pair["exec_median_ms"].as_f64(),
        ) else {
            continue;
        };
        finite(wait)?;
        finite(exec)?;
        let Some(assigned) = first["assigned_usage_keys"].as_array() else {
            continue;
        };
        if assigned.is_empty() {
            continue;
        }
        let mut assigned_keys = BTreeSet::new();
        let mut proven = true;
        for value in assigned {
            let Some(k) = value.as_str() else {
                proven = false;
                break;
            };
            if !usage.get(k).is_some_and(|u| u.seat.as_deref() == Some(mid)) {
                proven = false;
                break;
            }
            assigned_keys.insert(k.to_owned());
        }
        if !proven {
            continue;
        }
        used_hops.insert(key.to_owned());
        used_hops.insert(next_key.to_owned());
        keys.extend([key.to_owned(), next_key.to_owned()]);
        keys.extend(leaves[from].iter().cloned());
        keys.extend(leaves[to].iter().cloned());
        for hop in [*first, *second] {
            for field in ["claim_source_key", "terminal_source_key"] {
                if let Some(k) = hop[field].as_str() {
                    keys.insert(k.to_owned());
                }
            }
        }
        bypassed_usage.extend(assigned_keys.iter().cloned());
        let ms = finite(wait + exec)?;
        total_ms = finite(total_ms + ms)?;
        paths.push(json!({"from":from,"via":mid,"to":to,"source_keys":[key,next_key],"assigned_usage_keys":assigned_keys,
            "removed_hop_wait_median_ms":wait,"removed_hop_exec_median_ms":exec,"projected_cycle_reduction_ms":ms}));
    }
    let available = !paths.is_empty();
    let value = if available {
        json!({"status":"projected","paths":paths,"median_based_projected_total_cycle_reduction_ms":total_ms,
        "source_keys":keys,"assigned_usage_keys":bypassed_usage,"limit":"Assumes the intermediate coordinator hop is unnecessary. Summed pair medians are a scenario estimate, not observed elapsed time saved."})
    } else {
        no_data("no unique complete in-ring parent-linked leaf/coordinator/leaf path with valid latency and usage provenance")
    };
    Ok(Routing {
        value,
        usage: bypassed_usage,
        keys,
        available,
    })
}

fn projected_families(p: &Value, s: &Scenario) -> Value {
    let Some(checks) = p["four_eyes_independence"]["checks"]
        .as_array()
        .filter(|v| !v.is_empty())
    else {
        return no_data("no canonical builder/reviewer chains");
    };
    let target = |seat: &str| {
        if s.remove.iter().any(|v| v == seat) {
            s.replacement_seat.as_deref().map(str::to_owned)
        } else {
            Some(seat.to_owned())
        }
    };
    let mut results = Vec::new();
    let mut all_independent = true;
    for check in checks {
        let builder = check["builder"].as_str().and_then(&target);
        let reviewer = check["reviewer"].as_str().and_then(&target);
        let families =
            |role: &str, field: &str, target: &Option<String>| -> (BTreeSet<String>, bool) {
                if let Some(family) = target
                    .as_deref()
                    .and_then(|seat| s.tier_policy.iter().find(|r| r.seat == seat))
                    .and_then(|r| r.model_family.as_ref())
                {
                    return (BTreeSet::from([family.to_owned()]), true);
                }
                let known = check[role].as_str() == target.as_deref()
                    && matches!(
                        check["status"].as_str(),
                        Some("independent" | "same_family")
                    );
                (
                    check[field]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(Value::as_str)
                                .map(str::to_owned)
                                .collect()
                        })
                        .unwrap_or_default(),
                    known,
                )
            };
        let (a, known_a) = families("builder", "builder_families", &builder);
        let (b, known_b) = families("reviewer", "reviewer_families", &reviewer);
        let provenance = check["source_key"].as_str().is_some_and(|s| !s.is_empty())
            && check["review_handoff"]
                .as_str()
                .is_some_and(|s| !s.is_empty());
        let status = if !provenance {
            "no_data"
        } else if builder.is_some() && builder == reviewer {
            "same_seat"
        } else if !known_a || !known_b || a.is_empty() || b.is_empty() {
            "no_data"
        } else if a.is_disjoint(&b) {
            "independent_under_assumptions"
        } else {
            "same_family"
        };
        all_independent &= status == "independent_under_assumptions";
        results.push(json!({"chain":check["chain"],"builder":check["builder"],"reviewer":check["reviewer"],"projected_builder":builder,"projected_reviewer":reviewer,"builder_families":a,"reviewer_families":b,"status":status,"source_keys":[check["source_key"],check["review_handoff"]]}));
    }
    json!({"status":"projected","all_independent_under_assumptions":all_independent,"checks":results,"limit":"Historical independence is unchanged; projected model families are operator assumptions."})
}

/// Simulate a historical topology without opening a database or operating any runtime.
pub fn simulate(report: &Value, scenario: &Scenario) -> Result<Value> {
    validate(scenario)?;
    if report["metric"] != "decisions" || report["definition_version"] != "acceptance-r1-a1" {
        return Ok(no_data(
            "canonical decisions baseline is unavailable or unsupported",
        ));
    }
    let p = &report["payload"];
    let scope = json!({"rig":p["rig"],"from":p["from"],"to":p["to"],"definition_version":report["definition_version"]});
    let unavailable = |reason: &str| {
        let mut v = no_data(reason);
        v["scope"] = scope.clone();
        v
    };
    let dates = p["from"]
        .as_str()
        .and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok())
        .zip(
            p["to"]
                .as_str()
                .and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok()),
        );
    if p["kind"] != "decisions" || dates.is_none_or(|(from, to)| to <= from) {
        return Ok(unavailable("canonical report scope is missing or invalid"));
    }
    let Some(inputs) = p["trace_inputs"].as_array().filter(|a| !a.is_empty()) else {
        return Ok(unavailable("no canonical timestamped usage baseline"));
    };
    let Some(lineage) = p["issue_lineage"]["records"].as_array() else {
        return Ok(unavailable(
            "canonical per-usage seat lineage is unavailable",
        ));
    };
    let mut seats_by_key = BTreeMap::new();
    for row in lineage {
        if let Some(key) = row["source_key"].as_str() {
            if seats_by_key.insert(key, row["seat"].as_str()).is_some() {
                return Ok(unavailable("duplicate canonical lineage key"));
            }
        }
    }
    let mut usage = BTreeMap::new();
    let mut baseline = Tokens::default();
    let mut seats = BTreeSet::new();
    for row in inputs {
        let Some(key) = row["source_key"].as_str() else {
            return Ok(unavailable("usage source key missing"));
        };
        let Some(tokens) = Tokens::from_value(row) else {
            return Ok(unavailable("canonical token components missing or invalid"));
        };
        let seat = seats_by_key.get(key).copied().flatten();
        if let Some(seat) = seat {
            seats.insert(seat.to_owned());
        }
        if usage
            .insert(
                key.to_owned(),
                Usage {
                    seat: seat.map(str::to_owned),
                    tokens,
                },
            )
            .is_some()
        {
            return Ok(unavailable("duplicate canonical usage key"));
        }
        baseline.add(tokens)?;
    }
    if p["usage_rows"].as_u64() != Some(usage.len() as u64) {
        return Ok(unavailable(
            "canonical usage census does not match baseline",
        ));
    }
    let total = baseline
        .input_uncached
        .checked_add(baseline.output)
        .and_then(|n| n.checked_add(baseline.cache_read))
        .and_then(|n| n.checked_add(baseline.cache_write))
        .ok_or_else(|| anyhow::anyhow!("token aggregate overflow"))?;
    if Tokens::from_value(&p["token_categories"]) != Some(baseline)
        || p["tokens"].as_i64() != Some(total)
    {
        return Ok(unavailable(
            "canonical token totals do not match source-key census",
        ));
    }
    let baseline_units = baseline.weighted(&scenario.weights)?;
    let removed: BTreeSet<_> = scenario.remove.iter().cloned().collect();
    let mut reasons = Vec::new();
    if removed.iter().any(|seat| !seats.contains(seat)) {
        reasons.push("removed seat has no observed canonical usage");
    }
    if scenario
        .replacement_seat
        .as_ref()
        .is_some_and(|seat| !seats.contains(seat))
    {
        reasons.push("replacement seat has no observed canonical usage");
    }
    if scenario
        .tier_policy
        .iter()
        .any(|r| !seats.contains(&r.seat) && scenario.replacement_seat.as_ref() != Some(&r.seat))
    {
        reasons.push("tier policy names a seat without canonical usage");
    }
    let routing = if scenario.direct_handoffs {
        direct_routing(p, &usage)?
    } else {
        Routing {
            value: json!({"status":"not_requested"}),
            usage: BTreeSet::new(),
            keys: BTreeSet::new(),
            available: true,
        }
    };
    if !routing.available {
        reasons.push("direct routing baseline is insufficient");
    }
    let mut retained = Tokens::default();
    let mut bypassed = Tokens::default();
    let mut unassigned = Tokens::default();
    let mut unassigned_records = 0usize;
    let mut projected_units = 0.0;
    let mut allocation = BTreeMap::<String, usize>::new();
    for (key, row) in &usage {
        if routing.usage.contains(key) {
            bypassed.add(row.tokens)?;
            continue;
        }
        retained.add(row.tokens)?;
        let target = if row.seat.as_ref().is_some_and(|seat| removed.contains(seat)) {
            if let Some(replacement) = scenario.replacement_seat.as_ref() {
                Some(replacement.as_str())
            } else {
                unassigned.add(row.tokens)?;
                unassigned_records += 1;
                None
            }
        } else {
            row.seat.as_deref()
        };
        let target_label = target.unwrap_or("unlinked_or_unassigned");
        *allocation.entry(target_label.to_owned()).or_default() += 1;
        let rule = target.and_then(|seat| scenario.tier_policy.iter().find(|r| r.seat == seat));
        let uncached = rule.map_or(1.0, |r| r.uncached_multiplier);
        let cache = rule.map_or(1.0, |r| r.cache_read_multiplier);
        projected_units = finite(
            projected_units
                + row.tokens.input_uncached as f64 * scenario.weights.uncached * uncached
                + row.tokens.cache_read as f64 * scenario.weights.cache_read * cache,
        )?;
    }
    if unassigned_records > 0 {
        reasons.push("removed-seat engineering demand has no explicit replacement; total savings cannot be inferred");
    }
    let mut recombined = retained;
    recombined.add(bypassed)?;
    let families = projected_families(p, scenario);
    if scenario.enforce_model_family_diversity
        && families["all_independent_under_assumptions"] != true
    {
        reasons.push("model-family independence is not established under this scenario");
    }
    let boundary = match (&scenario.boundary, scenario.boundary_reset_threshold) {
        (Some(model), threshold) => {
            let mut v = boundary_cost(model, &scenario.weights)?;
            v["eligible"] = json!(threshold.is_none_or(|n| model.stale_context_tokens >= n));
            v["threshold_tokens"] = json!(threshold);
            v["source_keys"] = json!([]);
            v["baseline_reference_source_keys"] = json!(usage.keys().collect::<Vec<_>>());
            v["term_origin"]=json!("turns/stale/clean/cold_boot are explicit operator assumptions; baseline reference keys anchor the selected historical scope, not counterfactual measurements");
            v
        }
        (None, Some(_)) => {
            no_data("boundary threshold requires explicit turns/stale/clean/cold-boot assumptions")
        }
        (None, None) => json!({"status":"not_requested"}),
    };
    if scenario.boundary_reset_threshold.is_some() && scenario.boundary.is_none() {
        reasons.push("boundary threshold needs explicit operator cost assumptions");
    }
    let available = reasons.is_empty();
    let reduction = signed(baseline_units - projected_units)?;
    let reduction_ratio = if available && baseline_units > 0.0 {
        Some(signed(reduction / baseline_units)?)
    } else {
        None
    };
    let mut all_keys: BTreeSet<String> = usage.keys().cloned().collect();
    all_keys.extend(routing.keys);
    if let Some(checks) = families["checks"].as_array() {
        for check in checks {
            if let Some(keys) = check["source_keys"].as_array() {
                all_keys.extend(keys.iter().filter_map(Value::as_str).map(str::to_owned));
            }
        }
    }
    if let Some(events) = p["context_compactions"]["events"].as_array() {
        for event in events {
            for field in ["pre_source_key", "post_source_key"] {
                if let Some(k) = event[field].as_str() {
                    all_keys.insert(k.to_owned());
                }
            }
            if let Some(keys) = event["queue_source_keys"].as_array() {
                all_keys.extend(keys.iter().filter_map(Value::as_str).map(str::to_owned));
            }
        }
    }
    Ok(
        json!({"status":if available{"projected"}else{"no_data"},"scope":scope,"reasons":reasons,"scenario":scenario,
        "baseline":{"tokens":baseline,"input_units":baseline_units,"usage_records":usage.len(),"source_keys":usage.keys().collect::<Vec<_>>()},
        "projection":{"input_units":if available{Some(projected_units)}else{None},"reduction_input_units":if available{Some(reduction)}else{None},
            "reduction_ratio":reduction_ratio,"unit":"uncached-equivalent input units, not currency"},
        "conservation":{"retained_historical_tokens":retained,"bypassed_coordination_tokens":bypassed,"recombined":recombined,"conserved":baseline==recombined,"unassigned_work_tokens":unassigned,"unassigned_work_records":unassigned_records,"bypassed_records":routing.usage.len(),"allocation_records":allocation},
        "routing":routing.value,"boundary":boundary,"model_family_diversity":families,
        "compaction_hazards":{"observed_mid_item_hazards":p["context_compactions"]["mid_item_hazard_count"],"source_events":p["context_compactions"]["events"],"projected_eliminated_hazards":Value::Null,"limit":"Historical what-ifs cannot prove actual hazards would have been eliminated."},
        "source_keys":all_keys,"assumptions":["Historical engineering demand is conserved when a seat is removed.","Tier multipliers and model/effort labels are explicit operator assumptions, not validated productivity predictions.","Direct bypass removes only deduplicated assigned coordinator input usage; destination engineering work remains.","Boundary comparison is independent and must not be added to overlapping tier/routing reductions.","No rig mutation, agent rerun, cloud request or causal validation occurred."]}),
    )
}
