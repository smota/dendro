//! Shared Direct Overview evaluation, request types, and evidence digest.
#![deny(unsafe_code)]

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use crate::advisory::AdvisoryOptions;
use crate::vortex::FrictionThresholds;
use crate::{ActivityFact, AnalyticsStore, IdentityFact, SkillFact};

mod attribution;
mod native;

pub const FORMULA_VERSION: &str = "direct-overview-v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OverviewRequest {
    pub rig: Option<String>,
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    pub observation_clock: Option<DateTime<Utc>>,
    pub evidence_as_of: Option<DateTime<Utc>>,
    pub advisory_options: AdvisoryOptions,
    pub friction_thresholds: FrictionThresholds,
    pub expect_evidence_digest: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CriticalFocus {
    pub focus_state: String,
    pub category: Option<String>,
    pub entity_id: Option<String>,
    pub duration_secs: Option<f64>,
    pub temporal_label: String,
    pub selected_source_keys: Vec<String>,
    pub ranked_source_keys: Vec<String>,
    pub state_support_source_keys: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CwdProjectGroup {
    pub cwd_fingerprint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    pub total_tokens: i64,
    pub active_hours: f64,
    pub closed_issue_qitems: usize,
    pub estimated_cost_usd: Option<f64>,
    pub usage_source_keys: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActiveTool {
    pub tool_name: String,
    pub invocation_count: usize,
    pub error_count: usize,
    pub error_rate: Option<f64>,
    pub unknown_outcome_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ContextTaxOverview {
    pub active_tools: Vec<ActiveTool>,
    pub active_tools_status: String,
    pub excluded_source_keys: Vec<String>,
    pub uncalled_tools_count: Option<usize>,
    pub declaration_status: String,
    pub uncalled_skills: Vec<String>,
    pub wasted_schema_tokens: Option<i64>,
    pub schema_tokens_status: String,
    pub wasted_cost_usd: Option<f64>,
    pub declaration_source_keys: Vec<String>,
    pub invocation_support_source_keys: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ThroughputAndCostOverview {
    pub closed_issue_qitems: usize,
    pub closed_internal_tasks: usize,
    pub closed_issue_qitem_ids: Vec<String>,
    pub total_fleet_spend_usd: Option<f64>,
    pub observed_priced_subtotal_usd: Option<f64>,
    pub avg_cost_per_closed_issue_qitem: Option<f64>,
    pub average_label: Option<String>,
    pub money_status: String,
    pub priced_rows: usize,
    pub unpriced_rows: usize,
    pub closed_issue_source_keys: Vec<String>,
    pub spend_source_keys: Vec<String>,
    pub priced_source_keys: Vec<String>,
    pub unpriced_source_keys: Vec<String>,
    pub excluded_source_keys: Vec<String>,
    pub eligible_closure_support_source_keys: Vec<String>,
    pub recovery_excluded_source_keys: Vec<String>,
    pub reopen_state_support_source_keys: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OverviewScope {
    pub rig: Option<String>,
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DirectOverview {
    pub formula_version: String,
    pub scope: OverviewScope,
    pub source_coverage: BTreeMap<String, String>,
    pub lens_rows: BTreeMap<String, Vec<serde_json::Value>>,
    pub observed_active_hours: f64,
    pub unassigned_active_hours: f64,
    pub unassigned_closed_issue_qitems: usize,
    pub activity_source_keys: Vec<String>,
    pub as_of: String,
    pub snapshot: bool,
    pub live_alert: bool,
    pub coverage: String,
    pub evidence_as_of: Option<String>,
    pub critical_focus: CriticalFocus,
    pub context_tax: ContextTaxOverview,
    pub throughput_and_cost: ThroughputAndCostOverview,
    pub project_groups: Vec<CwdProjectGroup>,
    pub assigned_usage_source_keys: Vec<String>,
    pub unassigned_usage_source_keys: Vec<String>,
    pub unassigned_total_tokens: i64,
    pub unassigned_observed_cost_usd: Option<f64>,
    pub all_projects_total_tokens: i64,
    pub all_projects_observed_cost_usd: Option<f64>,
    pub raw_cwd_path_count: usize,
    pub total_tokens: i64,
    pub bounded_observed_priced_subtotal_usd: Option<f64>,
    pub usage_source_keys: Vec<String>,
    pub excluded_source_keys: Vec<String>,
    pub undated_coverage_source_keys: Vec<String>,
    pub source_keys: Vec<String>,
    pub digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DerivedIncident {
    pub classification: String,
    #[serde(default)]
    pub threshold_breached: Option<bool>,
    pub seat: Option<String>,
    pub qitem_id: Option<String>,
    pub hold_id: Option<String>,
    pub gate: Option<String>,
    pub duration_secs: f64,
    pub total_tokens: Option<i64>,
    pub rework_tokens: Option<i64>,
    pub chain_returns: Option<usize>,
    pub return_rate: Option<f64>,
    pub resolved: Option<bool>,
    pub source_keys: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OverviewQueueFact {
    pub source_key: String,
    pub transition_id: i64,
    pub qitem_id: String,
    pub ts_ms: i64,
    pub state: String,
    pub source_seat: String,
    pub dest_seat: String,
    pub closure_reason: Option<String>,
    pub closure_target: Option<String>,
    pub is_recovery: bool,
    pub recovery_reason: Option<String>,
    pub issue_tags: Vec<String>,
}

impl OverviewQueueFact {
    pub fn ts(&self) -> DateTime<Utc> {
        DateTime::from_timestamp_millis(self.ts_ms).unwrap_or(DateTime::<Utc>::MIN_UTC)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OverviewUsageFact {
    pub source_key: String,
    pub harness: String,
    pub native_session: String,
    pub ts_ms: Option<i64>,
    pub seq: i64,
    pub total_tokens: i64,
    pub currency_total: Option<f64>,
}

impl OverviewUsageFact {
    pub fn ts(&self) -> Option<DateTime<Utc>> {
        self.ts_ms.and_then(DateTime::from_timestamp_millis)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OverviewToolFact {
    pub source_key: String,
    pub harness: String,
    pub native_session: String,
    pub ts_ms: Option<i64>,
    pub tool_name: String,
    pub failed: Option<bool>,
}

impl OverviewToolFact {
    pub fn ts(&self) -> Option<DateTime<Utc>> {
        self.ts_ms.and_then(DateTime::from_timestamp_millis)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OverviewProcessIdentity {
    pub identity_key: String,
    pub harness: Option<String>,
    pub session_link: Option<String>,
    pub cwd_fingerprint: Option<String>,
    pub first_seen_ms: i64,
    pub last_seen_ms: i64,
    pub ended_at_ms: Option<i64>,
}

impl OverviewProcessIdentity {
    pub fn first_seen(&self) -> DateTime<Utc> {
        DateTime::from_timestamp_millis(self.first_seen_ms).unwrap_or(DateTime::<Utc>::MIN_UTC)
    }
    pub fn last_seen(&self) -> DateTime<Utc> {
        DateTime::from_timestamp_millis(self.last_seen_ms).unwrap_or(DateTime::<Utc>::MIN_UTC)
    }
}

#[derive(Debug, Clone, Default)]
pub struct RawOverviewFacts<'a> {
    pub derived_evidence: Vec<DerivedIncident>,
    pub queue_facts: &'a [OverviewQueueFact],
    pub skill_facts: &'a [SkillFact],
    pub usage_facts: &'a [OverviewUsageFact],
    pub tool_facts: &'a [OverviewToolFact],
    pub process_identities: &'a [OverviewProcessIdentity],
    pub identity_facts: &'a [IdentityFact],
    pub activity_facts: &'a [ActivityFact],
    pub coverage: Option<String>,
    pub evidence_as_of: Option<DateTime<Utc>>,
}

/// Token count according to conventions: input_uncached + output_tokens + cache_read + cache_write
pub fn token_amount(u: &OverviewUsageFact) -> i64 {
    u.total_tokens
}

pub fn to_iso_z(dt: &DateTime<Utc>) -> String {
    dt.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true)
}

// Stored adapter timestamps use milliseconds. Ceil a boundary so a sub-ms
// endpoint retains the half-open comparison for exactly representable facts.
fn cutoff_ms(ts: DateTime<Utc>) -> i64 {
    ts.timestamp_millis() + i64::from(!ts.timestamp_subsec_nanos().is_multiple_of(1_000_000))
}

/// Evaluates direct overview from in-memory/adapted facts.
pub fn evaluate_direct_overview_facts(
    facts: &RawOverviewFacts<'_>,
    req: &OverviewRequest,
) -> Result<DirectOverview> {
    let mut identities = facts.identity_facts.to_vec();
    identities.sort_by(|a, b| {
        (&a.harness, &a.native_session, a.ts, &a.source_key).cmp(&(
            &b.harness,
            &b.native_session,
            b.ts,
            &b.source_key,
        ))
    });
    let sorted = RawOverviewFacts {
        identity_facts: &identities,
        ..facts.clone()
    };
    evaluate_with_support(&sorted, req, &native::Support::default())
}

fn evaluate_with_support(
    facts: &RawOverviewFacts<'_>,
    req: &OverviewRequest,
    support: &native::Support,
) -> Result<DirectOverview> {
    if req.from >= req.to {
        return Err(anyhow!("overview from must precede to"));
    }
    req.advisory_options.validate()?;
    req.friction_thresholds.validate()?;
    let observation_clock = req.observation_clock.unwrap_or_else(Utc::now);
    let as_of_dt = req.evidence_as_of.or(facts.evidence_as_of);

    // D3: Snapshot / live alert
    let (snapshot, live_alert) = match as_of_dt {
        Some(as_of) if as_of < observation_clock - chrono::Duration::seconds(300) => (true, false),
        Some(_) if req.to >= observation_clock - chrono::Duration::seconds(300) => (false, true),
        _ => (true, false),
    };

    // D3: Temporal label: window.to >= observation_clock - 300s
    let temporal_label = if req.to >= observation_clock - chrono::Duration::seconds(300) {
        "Active Fleet Bottleneck".to_string()
    } else {
        "Historical Window Bottleneck".to_string()
    };

    let overall_coverage = facts
        .coverage
        .clone()
        .unwrap_or_else(|| "unavailable".to_string());

    let live_alert = live_alert && !matches!(overall_coverage.as_str(), "no_data" | "unavailable");
    let snapshot = snapshot || !live_alert;

    // 1. Critical Focus evaluation
    let (critical_focus, focus_source_keys) =
        evaluate_critical_focus(facts, req, &temporal_label, &overall_coverage)?;

    // 2. Context Tax evaluation
    let (context_tax, tax_source_keys) = evaluate_context_tax(facts, req)?;

    // 3. Throughput & Cost evaluation
    let (throughput_and_cost, tc_source_keys) = evaluate_throughput_and_cost(facts, req)?;

    // 4. Combined CWD Projects and Scoped Usage in a single pass over usage_facts
    let req_from_ms = cutoff_ms(req.from);
    let req_to_ms = cutoff_ms(req.to);

    let mut pid_index: HashMap<(&str, &str), Vec<&OverviewProcessIdentity>> =
        HashMap::with_capacity(32);
    for p in facts.process_identities {
        if let (Some(h), Some(s)) = (p.harness.as_deref(), p.session_link.as_deref()) {
            pid_index.entry((h, s)).or_default().push(p);
        }
    }

    let mut project_usage: HashMap<String, (i64, f64, bool, Vec<String>)> =
        HashMap::with_capacity(16);
    let mut unassigned_tokens = 0i64;
    let mut unassigned_cost = 0.0f64;
    let mut unassigned_has_cost = false;
    let mut unassigned_usage_keys = Vec::new();
    let mut assigned_usage_keys = Vec::new();

    let mut all_tokens = 0i64;
    let mut all_cost = 0.0f64;
    let mut all_has_cost = false;

    let mut total_tokens = 0i64;
    let mut bounded_priced_subtotal = 0.0f64;
    let mut has_bounded_priced = false;
    let mut usage_source_keys = Vec::new();
    let mut excluded_source_keys = Vec::new();
    let mut undated_source_keys = Vec::new();

    for u in facts.usage_facts {
        let is_in_rig =
            attribution::session_in_scope(facts, req, &u.harness, &u.native_session, u.ts());

        if !is_in_rig {
            excluded_source_keys.push(u.source_key.clone());
            continue;
        }

        let Some(ts_ms) = u.ts_ms else {
            excluded_source_keys.push(u.source_key.clone());
            undated_source_keys.push(u.source_key.clone());
            continue;
        };

        if ts_ms < req_from_ms || ts_ms >= req_to_ms {
            excluded_source_keys.push(u.source_key.clone());
            continue;
        }

        let tokens = token_amount(u);
        all_tokens += tokens;
        total_tokens += tokens;

        if let Some(cost) = u.currency_total {
            all_cost += cost;
            all_has_cost = true;
            bounded_priced_subtotal += cost;
            has_bounded_priced = true;
        }

        usage_source_keys.push(u.source_key.clone());

        // Find process identities overlapping ts on matching session
        let matching_pids: Vec<&&OverviewProcessIdentity> =
            match pid_index.get(&(u.harness.as_str(), u.native_session.as_str())) {
                Some(candidates) => candidates
                    .iter()
                    .filter(|p| {
                        ts_ms >= p.first_seen_ms
                            && ts_ms <= p.last_seen_ms
                            && p.ended_at_ms.is_none_or(|end| ts_ms < end)
                    })
                    .collect(),
                None => vec![],
            };

        // Check if fingerprints are unique
        let fingerprints: BTreeSet<&str> = matching_pids
            .iter()
            .filter_map(|p| p.cwd_fingerprint.as_deref())
            .collect();

        if fingerprints.len() == 1
            && matching_pids.iter().all(|p| p.cwd_fingerprint.is_some())
            && attribution::binding(facts, &u.harness, &u.native_session, u.ts().unwrap()).is_some()
        {
            let fp = *fingerprints.first().unwrap();
            let entry = project_usage
                .entry(fp.to_string())
                .or_insert_with(|| (0, 0.0, false, Vec::new()));
            entry.0 += tokens;
            if let Some(cost) = u.currency_total {
                entry.1 += cost;
                entry.2 = true;
            }
            entry.3.push(u.source_key.clone());
            assigned_usage_keys.push(u.source_key.clone());
        } else {
            // Ambiguous or unassigned
            unassigned_tokens += tokens;
            if let Some(cost) = u.currency_total {
                unassigned_cost += cost;
                unassigned_has_cost = true;
            }
            unassigned_usage_keys.push(u.source_key.clone());
        }
    }

    let mut project_groups = Vec::new();
    let mut cwd_source_keys = Vec::new();

    for (fp, (tokens, cost, has_cost, keys)) in project_usage {
        cwd_source_keys.extend(keys.clone());
        project_groups.push(CwdProjectGroup {
            cwd_fingerprint: fp,
            alias: None,
            total_tokens: tokens,
            active_hours: 0.0,
            closed_issue_qitems: 0,
            estimated_cost_usd: if has_cost { Some(cost) } else { None },
            usage_source_keys: keys,
        });
    }
    project_groups.sort_by(|a, b| a.cwd_fingerprint.cmp(&b.cwd_fingerprint));

    cwd_source_keys.extend(unassigned_usage_keys.clone());

    let activity = attribution::project_activity(facts, req, support, &mut project_groups);
    let unassigned_closed_issue_qitems =
        attribution::project_closures(facts, req, &throughput_and_cost, &mut project_groups);
    project_groups.sort_by(|a, b| a.cwd_fingerprint.cmp(&b.cwd_fingerprint));

    let mut lens_rows = support.lens_rows.clone();
    let mut projects = project_groups.clone();
    projects.sort_by(|a, b| {
        b.total_tokens
            .cmp(&a.total_tokens)
            .then_with(|| a.cwd_fingerprint.cmp(&b.cwd_fingerprint))
    });
    projects.truncate(5);
    lens_rows.insert(
        "v_cwd_fingerprint_rollup".into(),
        projects
            .iter()
            .map(|p| serde_json::to_value(p).expect("project serialization"))
            .collect(),
    );
    lens_rows.insert("skill_facts".into(),context_tax.uncalled_skills.iter().map(|skill|json!({"skill":skill,"invoked":false,"status":context_tax.declaration_status})).collect());

    // Union all source keys
    let mut all_source_keys: Vec<String> = Vec::with_capacity(
        focus_source_keys.len()
            + tax_source_keys.len()
            + tc_source_keys.len()
            + cwd_source_keys.len(),
    );
    all_source_keys.extend(focus_source_keys);
    all_source_keys.extend(tax_source_keys);
    all_source_keys.extend(tc_source_keys);
    all_source_keys.extend(cwd_source_keys);
    all_source_keys.extend(activity.2.clone());
    all_source_keys.extend(critical_focus.state_support_source_keys.clone());
    all_source_keys.sort();
    all_source_keys.dedup();
    let source_keys_vec = all_source_keys;

    // Canonical safe input operands, exact scope and effective analytic options.
    // Each typed row is length-prefixed; no raw prompt/command/free-text reason
    // enters this projection. Only the presentation observation clock is omitted.
    let mut header = json!({
        "digest_version": "canonical-selected-operands-v2", "formula_version": FORMULA_VERSION,
        "scope": {"rig": req.rig, "from": to_iso_z(&req.from), "to": to_iso_z(&req.to)},
        "options": {"advisory": req.advisory_options, "friction": req.friction_thresholds},
        "coverage": overall_coverage, "source_coverage": support.coverage,
        "evidence_as_of": as_of_dt, "support": support.safe_evidence,
        "context_excluded": context_tax.excluded_source_keys,
    });
    attribution::redact_evidence(&mut header);
    let mut hasher = Sha256::new();
    let bytes = serde_json::to_vec(&header)?;
    hasher.update((bytes.len() as u64).to_be_bytes());
    hasher.update(bytes);
    attribution::hash_evidence(&mut hasher, facts, req)?;
    let digest = format!("{:x}", hasher.finalize());

    // Optional expected digest validation
    if let Some(ref expected) = req.expect_evidence_digest {
        if expected != &digest {
            return Err(anyhow!(
                "evidence digest mismatch: expected {}, got {}",
                expected,
                digest
            ));
        }
    }

    Ok(DirectOverview {
        formula_version: FORMULA_VERSION.to_string(),
        scope: OverviewScope {
            rig: req.rig.clone(),
            from: to_iso_z(&req.from),
            to: to_iso_z(&req.to),
        },
        source_coverage: support.coverage.clone(),
        lens_rows,
        observed_active_hours: activity.0,
        unassigned_active_hours: activity.1,
        unassigned_closed_issue_qitems,
        activity_source_keys: activity.2,
        as_of: to_iso_z(&observation_clock),
        snapshot,
        live_alert,
        coverage: overall_coverage,
        evidence_as_of: as_of_dt.map(|dt| to_iso_z(&dt)),
        critical_focus,
        context_tax,
        throughput_and_cost,
        project_groups,
        assigned_usage_source_keys: assigned_usage_keys,
        unassigned_usage_source_keys: unassigned_usage_keys,
        unassigned_total_tokens: unassigned_tokens,
        unassigned_observed_cost_usd: if unassigned_has_cost {
            Some(unassigned_cost)
        } else {
            None
        },
        all_projects_total_tokens: all_tokens,
        all_projects_observed_cost_usd: if all_has_cost { Some(all_cost) } else { None },
        raw_cwd_path_count: 0,
        total_tokens,
        bounded_observed_priced_subtotal_usd: if has_bounded_priced {
            Some(bounded_priced_subtotal)
        } else {
            None
        },
        usage_source_keys,
        excluded_source_keys,
        undated_coverage_source_keys: undated_source_keys,
        source_keys: source_keys_vec,
        digest,
    })
}

fn evaluate_critical_focus(
    facts: &RawOverviewFacts<'_>,
    req: &OverviewRequest,
    temporal_label: &str,
    coverage: &str,
) -> Result<(CriticalFocus, Vec<String>)> {
    if coverage == "no_data" || coverage == "unavailable" {
        return Ok((
            CriticalFocus {
                focus_state: "unavailable".to_string(),
                category: None,
                entity_id: None,
                duration_secs: None,
                temporal_label: temporal_label.to_string(),
                selected_source_keys: vec![],
                ranked_source_keys: vec![],
                state_support_source_keys: vec![],
            },
            vec![],
        ));
    }

    // Terminal state suppresses only the incident for that item. Other items
    // and seats continue through ranking, even if an unrelated item just finished.
    let mut terminal = BTreeSet::new();
    let mut state_support_rows = Vec::new();
    let mut states = BTreeMap::new();
    for q in facts.queue_facts.iter().filter(|q| {
        q.ts_ms < cutoff_ms(req.to) && attribution::seat_in_scope(facts, req, &q.dest_seat, q.ts())
    }) {
        let entry = states.entry(q.qitem_id.as_str()).or_insert(q);
        if (q.ts_ms, q.transition_id, &q.source_key)
            > (entry.ts_ms, entry.transition_id, &entry.source_key)
        {
            *entry = q;
        }
    }
    for (id, q) in states {
        if matches!(q.state.as_str(), "done" | "denied" | "canceled" | "failed") {
            terminal.insert(id);
            state_support_rows.extend(
                facts
                    .queue_facts
                    .iter()
                    .filter(|r| {
                        r.qitem_id == id
                            && r.ts_ms <= q.ts_ms
                            && attribution::seat_in_scope(facts, req, &r.dest_seat, r.ts())
                    })
                    .map(|r| (r.ts_ms, r.transition_id, r.source_key.clone())),
            );
        }
    }
    state_support_rows.sort();
    let state_support_source_keys = state_support_rows
        .into_iter()
        .map(|(_, _, key)| key)
        .collect();

    // Evaluate derived incidents
    struct Candidate {
        tier: u8, // 1: Stall, 2: Hold, 3: Vortex
        category: String,
        entity_id: String,
        duration_secs: f64,
        total_tokens: Option<i64>,
        source_keys: Vec<String>,
    }

    let mut candidates = Vec::new();
    for inc in &facts.derived_evidence {
        if inc.resolved == Some(true)
            || (inc.classification != "rework_vortex"
                && inc
                    .qitem_id
                    .as_deref()
                    .is_some_and(|id| terminal.contains(id)))
        {
            continue;
        }

        match inc.classification.as_str() {
            "true_leaf_stall" => {
                if inc.duration_secs > req.advisory_options.leaf_stall_secs as f64 {
                    let entity_id = inc
                        .seat
                        .clone()
                        .or_else(|| inc.qitem_id.clone())
                        .unwrap_or_default();
                    candidates.push(Candidate {
                        tier: 1,
                        category: "Leaf Stall".to_string(),
                        entity_id,
                        duration_secs: inc.duration_secs,
                        total_tokens: inc.total_tokens,
                        source_keys: inc.source_keys.clone(),
                    });
                }
            }
            "unparked_container_hold" => {
                if inc.duration_secs > req.advisory_options.container_hold_secs as f64 {
                    let entity_id = inc
                        .seat
                        .clone()
                        .or_else(|| inc.qitem_id.clone())
                        .unwrap_or_default();
                    candidates.push(Candidate {
                        tier: 1,
                        category: "Unparked Container Hold".to_string(),
                        entity_id,
                        duration_secs: inc.duration_secs,
                        total_tokens: inc.total_tokens,
                        source_keys: inc.source_keys.clone(),
                    });
                }
            }
            "admission_gate_hold" => {
                if inc.duration_secs > 600.0 {
                    let entity_id = inc
                        .hold_id
                        .clone()
                        .or_else(|| inc.gate.clone())
                        .unwrap_or_default();
                    candidates.push(Candidate {
                        tier: 2,
                        category: "Admission Gate Hold".to_string(),
                        entity_id,
                        duration_secs: inc.duration_secs,
                        total_tokens: inc.total_tokens,
                        source_keys: inc.source_keys.clone(),
                    });
                }
            }
            "rework_vortex" => {
                let return_rate = inc.return_rate.unwrap_or(0.0);
                let chain_returns = inc.chain_returns.unwrap_or(0);
                let tokens = inc.rework_tokens.or(inc.total_tokens).unwrap_or(0);
                let duration = inc.duration_secs;

                let breach = inc.threshold_breached.unwrap_or(
                    return_rate > req.friction_thresholds.return_rate
                        || chain_returns > req.friction_thresholds.chain_returns as usize
                        || tokens > req.friction_thresholds.rework_tokens as i64
                        || duration > req.friction_thresholds.loop_duration_secs as f64,
                );

                if breach {
                    let entity_id = inc.qitem_id.clone().unwrap_or_default();
                    candidates.push(Candidate {
                        tier: 3,
                        category: "Rework Vortex".to_string(),
                        entity_id,
                        duration_secs: inc.duration_secs,
                        total_tokens: inc.rework_tokens.or(inc.total_tokens),
                        source_keys: inc.source_keys.clone(),
                    });
                }
            }
            _ => {}
        }
    }

    if candidates.is_empty() {
        return Ok((
            CriticalFocus {
                focus_state: if coverage == "complete" {
                    "nominal"
                } else {
                    "unavailable"
                }
                .to_string(),
                category: None,
                entity_id: None,
                duration_secs: None,
                temporal_label: temporal_label.to_string(),
                selected_source_keys: vec![],
                ranked_source_keys: vec![],
                state_support_source_keys,
            },
            vec![],
        ));
    }

    // Rank: tier ASC -> duration DESC -> tokens DESC -> entity_id ASC
    candidates.sort_by(|a, b| {
        a.tier
            .cmp(&b.tier)
            .then_with(|| b.duration_secs.total_cmp(&a.duration_secs))
            .then_with(|| b.total_tokens.cmp(&a.total_tokens))
            .then_with(|| a.entity_id.cmp(&b.entity_id))
    });

    let mut ranked_source_keys = Vec::new();
    for c in &candidates {
        ranked_source_keys.extend(c.source_keys.clone());
    }

    let winner = &candidates[0];
    let selected_source_keys = winner.source_keys.clone();

    Ok((
        CriticalFocus {
            focus_state: "active".to_string(),
            category: Some(winner.category.clone()),
            entity_id: Some(winner.entity_id.clone()),
            duration_secs: Some(winner.duration_secs),
            temporal_label: temporal_label.to_string(),
            selected_source_keys: selected_source_keys.clone(),
            ranked_source_keys,
            state_support_source_keys,
        },
        selected_source_keys,
    ))
}

fn evaluate_context_tax(
    facts: &RawOverviewFacts<'_>,
    req: &OverviewRequest,
) -> Result<(ContextTaxOverview, Vec<String>)> {
    Ok(attribution::context_tax(facts, req))
}

fn evaluate_throughput_and_cost(
    facts: &RawOverviewFacts<'_>,
    req: &OverviewRequest,
) -> Result<(ThroughputAndCostOverview, Vec<String>)> {
    let req_from_ms = cutoff_ms(req.from);
    let req_to_ms = cutoff_ms(req.to);

    let rig_seats: Option<BTreeSet<&str>> = req.rig.as_deref().map(|r| {
        facts
            .identity_facts
            .iter()
            .filter(|id| id.rig == r)
            .map(|id| id.seat.as_str())
            .collect()
    });

    // Group transitions by qitem_id
    let mut items: HashMap<&str, Vec<&OverviewQueueFact>> = HashMap::with_capacity(256);
    for q in facts.queue_facts.iter().filter(|q| {
        q.ts_ms < req_to_ms && attribution::seat_in_scope(facts, req, &q.dest_seat, q.ts())
    }) {
        items.entry(&q.qitem_id).or_default().push(q);
    }

    let mut closed_issue_items = Vec::new();
    let mut eligible_closure_support = Vec::new();
    let mut excluded_source_keys = Vec::new();
    let mut recovery_excluded_source_keys = Vec::new();
    let mut reopen_state_support_source_keys = Vec::new();
    let mut closed_internal_tasks = 0;

    for (item_id, mut transitions) in items {
        transitions.sort_by_key(|a| (a.ts_ms, a.transition_id, &a.source_key));
        let last_transition = transitions.last().unwrap();

        // Check if reopened at cutoff
        if last_transition.state != "done" {
            for t in &transitions {
                if t.state == "done" {
                    reopen_state_support_source_keys.push((
                        t.ts_ms,
                        t.transition_id,
                        t.source_key.clone(),
                    ));
                }
            }
            if transitions.iter().any(|t| t.state == "done") {
                reopen_state_support_source_keys.push((
                    last_transition.ts_ms,
                    last_transition.transition_id,
                    last_transition.source_key.clone(),
                ));
            }
            for t in &transitions {
                if t.state != "done" {
                    excluded_source_keys.push((t.ts_ms, t.transition_id, t.source_key.clone()));
                }
            }
            continue;
        }

        // Check recovery
        let is_recovery = transitions.iter().any(|t| {
            t.is_recovery
                || t.qitem_id.starts_with("qitem-recovery-")
                || t.qitem_id.starts_with("recovery:")
                || t.qitem_id.starts_with("stuck-sweep:")
                || t.recovery_reason.is_some()
        });

        if is_recovery {
            for t in &transitions {
                recovery_excluded_source_keys.push((
                    t.ts_ms,
                    t.transition_id,
                    t.source_key.clone(),
                ));
                excluded_source_keys.push((t.ts_ms, t.transition_id, t.source_key.clone()));
            }
            continue;
        }

        // Check seat rig filter
        if let Some(ref seats) = rig_seats {
            if !seats.contains(last_transition.dest_seat.as_str()) {
                continue;
            }
        }

        // Collapse repeated done readings, but never reach back across a reopen.
        let last_open = transitions.iter().rposition(|t| t.state != "done");
        let done_transition = transitions
            .iter()
            .skip(last_open.map_or(0, |n| n + 1))
            .find(|t| t.state == "done");
        let Some(dt) = done_transition.filter(|t| {
            t.ts_ms >= req_from_ms
                && t.ts_ms < req_to_ms
                && attribution::completion_reason(t.closure_reason.as_deref())
        }) else {
            for t in &transitions {
                excluded_source_keys.push((t.ts_ms, t.transition_id, t.source_key.clone()));
            }
            continue;
        };
        let has_issue_tag = dt
            .issue_tags
            .iter()
            .any(|s| attribution::positive_issue_id(s));
        if !dt.issue_tags.is_empty() && !has_issue_tag {
            excluded_source_keys.push((dt.ts_ms, dt.transition_id, dt.source_key.clone()));
            continue;
        }

        if has_issue_tag {
            closed_issue_items.push((
                dt.ts_ms,
                dt.transition_id,
                item_id.to_string(),
                dt.source_key.clone(),
            ));
            for t in &transitions {
                if t.state == "done" {
                    eligible_closure_support.push((t.ts_ms, t.transition_id, t.source_key.clone()));
                }
            }
        } else {
            closed_internal_tasks += 1;
        }
    }

    closed_issue_items.sort_by(|a, b| (a.0, a.1, &a.2).cmp(&(b.0, b.1, &b.2)));
    eligible_closure_support.sort_by(|a, b| (a.0, a.1, &a.2).cmp(&(b.0, b.1, &b.2)));
    excluded_source_keys.sort_by(|a, b| (a.0, a.1, &a.2).cmp(&(b.0, b.1, &b.2)));
    recovery_excluded_source_keys.sort_by(|a, b| (a.0, a.1, &a.2).cmp(&(b.0, b.1, &b.2)));
    reopen_state_support_source_keys.sort_by(|a, b| (a.0, a.1, &a.2).cmp(&(b.0, b.1, &b.2)));

    let mut closed_issue_qitem_ids = Vec::new();
    let mut closed_issue_source_keys = Vec::new();
    for (_, _, qid, sk) in closed_issue_items {
        if !closed_issue_qitem_ids.contains(&qid) {
            closed_issue_qitem_ids.push(qid);
        }
        if !closed_issue_source_keys.contains(&sk) {
            closed_issue_source_keys.push(sk);
        }
    }

    let mut eligible_closure_support_source_keys = Vec::new();
    for (_, _, sk) in eligible_closure_support {
        if !eligible_closure_support_source_keys.contains(&sk) {
            eligible_closure_support_source_keys.push(sk);
        }
    }

    let mut final_excluded_source_keys = Vec::new();
    for (_, _, sk) in excluded_source_keys {
        if !final_excluded_source_keys.contains(&sk) {
            final_excluded_source_keys.push(sk);
        }
    }

    let mut final_recovery_excluded = Vec::new();
    for (_, _, sk) in recovery_excluded_source_keys {
        if !final_recovery_excluded.contains(&sk) {
            final_recovery_excluded.push(sk);
        }
    }

    let mut final_reopen_state = Vec::new();
    for (_, _, sk) in reopen_state_support_source_keys {
        if !final_reopen_state.contains(&sk) {
            final_reopen_state.push(sk);
        }
    }

    let closed_issue_qitems = closed_issue_qitem_ids.len();

    // 2. Spend evaluation from usage_facts
    let mut priced_rows = 0;
    let mut unpriced_rows = 0;
    let mut unknown_time = false;
    let mut spend_sum = 0.0;
    let mut spend_source_keys = Vec::new();
    let mut priced_source_keys = Vec::new();
    let mut unpriced_source_keys = Vec::new();

    for u in facts.usage_facts {
        if !attribution::session_in_scope(facts, req, &u.harness, &u.native_session, u.ts()) {
            continue;
        }

        let Some(ts_ms) = u.ts_ms else {
            unknown_time = true;
            continue;
        };
        if ts_ms < req_from_ms || ts_ms >= req_to_ms {
            continue;
        }

        match u.currency_total {
            Some(cost) => {
                priced_rows += 1;
                spend_sum += cost;
                spend_source_keys.push(u.source_key.clone());
                priced_source_keys.push(u.source_key.clone());
            }
            None => {
                unpriced_rows += 1;
                unpriced_source_keys.push(u.source_key.clone());
            }
        }
    }

    let (money_status, total_fleet_spend_usd, observed_priced_subtotal_usd) =
        if priced_rows > 0 && unpriced_rows == 0 && !unknown_time {
            ("priced".to_string(), Some(spend_sum), None)
        } else if priced_rows == 0 && unpriced_rows > 0 {
            ("unpriced".to_string(), None, None)
        } else if priced_rows > 0 && (unpriced_rows > 0 || unknown_time) {
            ("partial".to_string(), None, Some(spend_sum))
        } else {
            ("unpriced".to_string(), None, None)
        };

    let (avg_cost_per_closed_issue_qitem, average_label) = if closed_issue_qitems == 0 {
        (None, Some("no closed issue tasks".to_string()))
    } else if let Some(spend) = total_fleet_spend_usd {
        (Some(spend / closed_issue_qitems as f64), None)
    } else {
        (None, None)
    };

    let mut all_keys = closed_issue_source_keys.clone();
    all_keys.extend(spend_source_keys.clone());
    all_keys.extend(unpriced_source_keys.clone());

    Ok((
        ThroughputAndCostOverview {
            closed_issue_qitems,
            closed_internal_tasks,
            closed_issue_qitem_ids,
            total_fleet_spend_usd,
            observed_priced_subtotal_usd,
            avg_cost_per_closed_issue_qitem,
            average_label,
            money_status,
            priced_rows,
            unpriced_rows,
            closed_issue_source_keys,
            spend_source_keys,
            priced_source_keys,
            unpriced_source_keys,
            excluded_source_keys: final_excluded_source_keys,
            eligible_closure_support_source_keys,
            recovery_excluded_source_keys: final_recovery_excluded,
            reopen_state_support_source_keys: final_reopen_state,
        },
        all_keys,
    ))
}

/// Evaluates direct overview from a DuckDB database path.
pub fn evaluate_overview_snapshot_path(db: &Path, req: &OverviewRequest) -> Result<DirectOverview> {
    let store = AnalyticsStore::open_read_only(db)?;
    store.read_snapshot(|s| evaluate_overview_snapshot(s, req))
}

pub use native::OverviewStatements;

/// Evaluates direct overview from a DuckDB database or store.
pub fn evaluate_overview_snapshot(
    store: &AnalyticsStore,
    req: &OverviewRequest,
) -> Result<DirectOverview> {
    let mut stmts = OverviewStatements::prepare(store.connection())?;
    stmts.evaluate(req)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::fs;
    use std::path::PathBuf;

    fn parse_dt(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn test_direct_overview_performance_5500_facts() {
        use std::time::Instant;

        let unique = format!(
            "dendro-test-perf-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        );
        let temp_dir = std::env::temp_dir().join(unique);
        fs::create_dir_all(&temp_dir).unwrap();
        let db_path = temp_dir.join("analytics.duckdb");

        let dataset_digest;
        // Populate database with exact 5,500 facts census:
        // usage 4000 / queue 600 / tool 400 / identity 100 / activity 150 / process_identities 100 / skill 150
        {
            let store = AnalyticsStore::open_path(&db_path, "perf_fixture").unwrap();
            let conn = store.connection();

            // 1. 100 identity_facts
            let mut appender = conn.appender("identity_facts").unwrap();
            for i in 0..100 {
                appender
                    .append_row(duckdb::params![
                        format!("id:{}", i),
                        "codex",
                        format!("session-{}", i),
                        if i == 99 { "foreign-rig" } else { "test-rig" },
                        format!("dev-{}@test-rig", i),
                        "2026-10-01 00:00:00"
                    ])
                    .unwrap();
            }
            appender.flush().unwrap();
            drop(appender);

            // 2. 100 process_identities
            conn.execute_batch(
                "INSERT INTO process_identities (identity_key, root_key, pid, platform, start_time_unix_s, exe_name, harness, cwd_fingerprint, session_link, environment_kind, isolation_hint, first_seen, last_seen, visibility_limits)
                 SELECT 'proc:' || i, 'root', 1000 + i, 'linux', 0, 'proc', 'codex', 'fp-' || (i % 10), 'session-' || (CASE WHEN i=1 THEN 0 ELSE i END), 'local', 'none',
                        CAST('2026-10-01 00:00:00' AS TIMESTAMP), CAST('2026-10-02 00:00:00' AS TIMESTAMP), 'none'
                 FROM range(0, 100) t(i)"
            ).unwrap();

            // 3. 150 activity_facts
            conn.execute_batch(
                "INSERT INTO activity_facts (source_key, seat, ts, state, seq)
                 SELECT 'act:' || i, 'dev-' || (i % 100) || '@test-rig', CAST('2026-10-01 00:00:00' AS TIMESTAMP) + i * INTERVAL '1 minute', CASE WHEN i IN (100,117) THEN 'idle' ELSE 'running' END, i
                 FROM range(0, 150) t(i)"
            ).unwrap();

            // 4. 600 queue_facts (200 items x 3 transitions)
            conn.execute_batch(
                "INSERT INTO queue_facts (source_key, transition_id, qitem_id, ts, state, source_seat, dest_seat, missions, chain_id, issue_tags, is_recovery)
                 SELECT 'events:queue.created:' || i, i * 3 + 1, 'item-' || i, CAST('2026-10-01 00:00:00' AS TIMESTAMP) + i * INTERVAL '1 second', 'pending', 'orch@test-rig', 'dev-' || (i % 100) || '@test-rig', '[]', 'chain-' || i, '[\"101\"]', false
                 FROM range(0, 200) t(i);
                 INSERT INTO queue_facts (source_key, transition_id, qitem_id, ts, state, source_seat, dest_seat, missions, chain_id, issue_tags, is_recovery)
                 SELECT 'events:queue.claimed:' || i, i * 3 + 2, 'item-' || i, CAST('2026-10-01 00:00:02' AS TIMESTAMP) + i * INTERVAL '1 second', 'in-progress', 'orch@test-rig', 'dev-' || (i % 100) || '@test-rig', '[]', 'chain-' || i, '[\"101\"]', false
                 FROM range(0, 200) t(i);
                 INSERT INTO queue_facts (source_key, transition_id, qitem_id, ts, state, source_seat, dest_seat, missions, chain_id, issue_tags, is_recovery, closure_reason)
                 SELECT 'q:done:' || i, i * 3 + 3, 'item-' || i, CASE WHEN i % 17=0 THEN CAST('2026-10-02 00:00:00' AS TIMESTAMP) ELSE CAST('2026-10-01 01:00:00' AS TIMESTAMP) + i * INTERVAL '1 second' END, 'done', 'orch@test-rig', 'dev-' || (i % 100) || '@test-rig', '[]', 'chain-' || i, '[\"101\"]', false, CASE WHEN i%17=1 THEN 'denied' ELSE 'no-follow-on' END
                 FROM range(0, 200) t(i);"
            ).unwrap();

            conn.execute(
                "UPDATE queue_facts SET parent_qitem_id='item-0' WHERE qitem_id='item-4'",
                [],
            )
            .unwrap();
            conn.execute(
                "UPDATE queue_facts SET is_recovery=true WHERE qitem_id='item-3'",
                [],
            )
            .unwrap();

            // 5. 4000 usage_facts
            conn.execute_batch(
                "INSERT INTO usage_facts (source_key, harness, native_session, ts, seq, input_uncached, output_tokens, cache_read, cache_write, reasoning, generation, source_id, currency_total)
                 SELECT 'usage:' || i, 'codex', 'session-' || (i % 100), CASE WHEN i=0 THEN CAST('2026-09-30 23:59:59' AS TIMESTAMP) WHEN i=2 THEN CAST('2026-10-02 00:00:00' AS TIMESTAMP) WHEN i=3 THEN NULL ELSE CAST('2026-10-01 00:00:00' AS TIMESTAMP) + (i % 86400) * INTERVAL '1 second' END, i, 10, 5, 2, 1, 0, 0, 'src', CASE WHEN i%29=0 THEN NULL ELSE 0.01 END
                 FROM range(0, 4000) t(i)"
            ).unwrap();

            // 6. 400 tool_facts
            conn.execute_batch(
                "INSERT INTO tool_facts (source_key, harness, native_session, ts, seq, tool_name, failed, structured_write, generation, source_id, category)
                 SELECT 'tool:' || i, 'codex', 'session-' || (i % 100), CAST('2026-10-01 00:00:00' AS TIMESTAMP) + (i % 86400) * INTERVAL '1 second', i, 'tool-' || (i%7), CASE WHEN i%7=1 THEN NULL ELSE i%7=0 END, false, 0, 'src', 'other'
                 FROM range(0, 400) t(i)"
            ).unwrap();

            // 7. 150 skill_facts
            conn.execute_batch(
                "INSERT INTO skill_facts (source_key, seat, skill, invoked)
                 SELECT 'usage:' || i || ':skill:0', 'dev-' || (i % 100) || '@test-rig', 'tool-' || (i%7), (i % 2 = 0)
                 FROM range(0, 150) t(i)"
            ).unwrap();

            // Verify census counts
            let id_count: i64 = conn
                .query_row("SELECT count(*) FROM identity_facts", [], |r| r.get(0))
                .unwrap();
            let proc_count: i64 = conn
                .query_row("SELECT count(*) FROM process_identities", [], |r| r.get(0))
                .unwrap();
            let act_count: i64 = conn
                .query_row("SELECT count(*) FROM activity_facts", [], |r| r.get(0))
                .unwrap();
            let q_count: i64 = conn
                .query_row("SELECT count(*) FROM queue_facts", [], |r| r.get(0))
                .unwrap();
            let u_count: i64 = conn
                .query_row("SELECT count(*) FROM usage_facts", [], |r| r.get(0))
                .unwrap();
            let t_count: i64 = conn
                .query_row("SELECT count(*) FROM tool_facts", [], |r| r.get(0))
                .unwrap();
            let s_count: i64 = conn
                .query_row("SELECT count(*) FROM skill_facts", [], |r| r.get(0))
                .unwrap();

            assert_eq!(id_count, 100);
            assert_eq!(proc_count, 100);
            assert_eq!(act_count, 150);
            assert_eq!(q_count, 600);
            assert_eq!(u_count, 4000);
            assert_eq!(t_count, 400);
            assert_eq!(s_count, 150);
            assert_eq!(
                id_count + proc_count + act_count + q_count + u_count + t_count + s_count,
                5500
            );

            // Hash the entire synthetic census, including excluded/undated
            // rows, auxiliary populations, and schema/index definitions. This
            // inventory is outside the measured query/evaluation boundary.
            let mut corpus = Sha256::new();
            for (table, order) in [
                ("usage_facts", "source_key"),
                ("queue_facts", "source_key"),
                ("tool_facts", "source_key"),
                ("identity_facts", "source_key"),
                ("activity_facts", "source_key"),
                ("process_identities", "identity_key"),
                ("skill_facts", "source_key"),
                ("seat_bounds", "seat,created_at,stopped_at"),
                ("destination_snapshots", "source_key"),
                ("openrig_row_records", "source_key"),
            ] {
                corpus.update((table.len() as u64).to_be_bytes());
                corpus.update(table.as_bytes());
                // Identifiers come only from this fixed fixture-owned allowlist.
                let mut stmt = conn
                    .prepare(&format!(
                        "SELECT to_json(t)::VARCHAR FROM {table} t ORDER BY {order}"
                    ))
                    .unwrap();
                for row in stmt.query_map([], |r| r.get::<_, String>(0)).unwrap() {
                    let row = row.unwrap();
                    corpus.update((row.len() as u64).to_be_bytes());
                    corpus.update(row.as_bytes());
                }
            }
            let mut stmt=conn.prepare("SELECT to_json(t)::VARCHAR FROM (SELECT type,name,tbl_name,sql FROM sqlite_master WHERE type IN ('table','view','index') ORDER BY type,name) t").unwrap();
            for row in stmt.query_map([], |r| r.get::<_, String>(0)).unwrap() {
                let row = row.unwrap();
                corpus.update((row.len() as u64).to_be_bytes());
                corpus.update(row.as_bytes());
            }
            dataset_digest = format!("{:x}", corpus.finalize());
            store.checkpoint().unwrap();
            drop(store);
        }

        // Fixture selection and database open are outside the timed boundary.
        // This is our own synthetic fixture, opened with DuckDB's read-only flag;
        // production owner-root admission is exercised by the independent tests.
        let conn = duckdb::Connection::open_with_flags(
            &db_path,
            duckdb::Config::default()
                .access_mode(duckdb::AccessMode::ReadOnly)
                .unwrap(),
        )
        .unwrap();
        conn.execute_batch("PRAGMA threads=1;").unwrap();
        assert!(conn
            .execute_batch("CREATE TABLE forbidden_benchmark_write (n INTEGER)")
            .is_err());
        let evaluate = |req: &OverviewRequest| {
            OverviewStatements::prepare(&conn)
                .unwrap()
                .evaluate(req)
                .unwrap()
        };
        let req = OverviewRequest {
            rig: Some("test-rig".to_string()),
            from: parse_dt("2026-10-01T00:00:00Z"),
            to: parse_dt("2026-10-02T00:00:00Z"),
            observation_clock: Some(parse_dt("2026-10-02T00:00:00Z")),
            evidence_as_of: None,
            advisory_options: AdvisoryOptions::default(),
            friction_thresholds: FrictionThresholds::default(),
            expect_evidence_digest: None,
        };

        // 1 Cold run
        let cold_start = Instant::now();
        let cold_res = evaluate(&req);
        let cold_duration = cold_start.elapsed();
        println!("Direct Overview Benchmark Cold Run: {:?}", cold_duration);
        assert_eq!(cold_res.formula_version, FORMULA_VERSION);

        // 5 Warmup iterations
        for _ in 0..5 {
            let _ = evaluate(&req);
        }

        // 20 Timed iterations
        let mut samples = Vec::with_capacity(20);
        for _ in 0..20 {
            let start = Instant::now();
            let res = evaluate(&req);
            let elapsed = start.elapsed();
            assert_eq!(res.formula_version, FORMULA_VERSION);
            samples.push(elapsed);
        }

        samples.sort();
        let mean = samples.iter().sum::<std::time::Duration>() / samples.len() as u32;
        let p95 = samples[(samples.len() as f64 * 0.95) as usize];
        let max = *samples.last().unwrap();

        println!(
            "Direct Overview Benchmark (5,500 facts; profile={}):",
            if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            }
        );
        println!("  Census: usage4000 queue600 tool400 identity100 activity150 process_identities100 skill150; auxiliary seat_bounds=0 destination_snapshots=0 openrig_row_records=0; admission-hold telemetry absent; vortex derived from counted inputs");
        println!("  Full census + schema/index digest: {}", dataset_digest);
        println!("  Scoped evidence/options digest: {}", cold_res.digest);
        println!("  Boundary: prepare+query+load+canonical advisory/vortex/activity+evaluate; read-only connection; threads=1; 5 warmups/20 samples");
        assert!(
            cold_res.observed_active_hours > 0.0,
            "activity computation must be connected"
        );
        assert!(
            !cold_res.context_tax.active_tools.is_empty(),
            "active tool computation must be connected"
        );
        println!("  Mean: {:?}", mean);
        println!("  P95:  {:?}", p95);
        println!("  Max:  {:?}", max);

        assert!(
            max < std::time::Duration::from_millis(50),
            "SLA violation: max iteration was {:?}, must be < 50ms",
            max
        );

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn test_all_22_reference_cases() {
        let manifest_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.agentflow/oracles/issue101/cases.json");
        let content = fs::read_to_string(&manifest_path)
            .expect("must read .agentflow/oracles/issue101/cases.json");
        let v: Value = serde_json::from_str(&content).expect("valid JSON");

        let default_scope = &v["scope"];
        let default_rig = default_scope["rig"].as_str().unwrap().to_string();
        let default_from = parse_dt(default_scope["from"].as_str().unwrap());
        let default_to = parse_dt(default_scope["to"].as_str().unwrap());
        let default_obs_clock = parse_dt(default_scope["observation_clock"].as_str().unwrap());

        let cases = v["cases"].as_array().expect("cases array");
        assert_eq!(cases.len(), 22, "must evaluate exactly 22 reference cases");

        for (idx, case) in cases.iter().enumerate() {
            let id = case["id"].as_str().unwrap();
            let inputs = &case["inputs"];
            let expected = &case["expected"];

            let mut req = OverviewRequest {
                rig: Some(default_rig.clone()),
                from: default_from,
                to: default_to,
                observation_clock: Some(default_obs_clock),
                evidence_as_of: None,
                advisory_options: AdvisoryOptions::default(),
                friction_thresholds: FrictionThresholds::default(),
                expect_evidence_digest: None,
            };

            if let Some(scope_override) = inputs.get("scope_override") {
                if let Some(to_str) = scope_override.get("to").and_then(Value::as_str) {
                    req.to = parse_dt(to_str);
                }
            }

            if let Some(as_of_str) = inputs.get("evidence_as_of").and_then(Value::as_str) {
                req.evidence_as_of = Some(parse_dt(as_of_str));
            }

            // Populate RawOverviewFacts
            let mut derived_evidence = Vec::new();
            let mut queue_facts = Vec::new();
            let mut skill_facts = Vec::new();
            let mut usage_facts = Vec::new();
            let mut tool_facts = Vec::new();
            let mut process_identities = Vec::new();
            let mut identity_facts = Vec::new();

            let mut coverage = None;
            if let Some(cov) = inputs.get("coverage").and_then(Value::as_str) {
                coverage = Some(cov.to_string());
            }

            if inputs.get("common_identity") == Some(&Value::Bool(true)) {
                identity_facts.push(IdentityFact {
                    source_key: "id:build".to_string(),
                    harness: "codex".to_string(),
                    native_session: "session-build".to_string(),
                    rig: "test-rig".to_string(),
                    seat: "dev-build@test-rig".to_string(),
                    ts: Some(default_from),
                });
            }

            if let Some(identities) = inputs.get("identity_facts").and_then(Value::as_array) {
                for id_val in identities {
                    identity_facts.push(IdentityFact {
                        source_key: id_val["source_key"].as_str().unwrap().to_string(),
                        harness: id_val["harness"].as_str().unwrap().to_string(),
                        native_session: id_val["native_session"].as_str().unwrap().to_string(),
                        rig: id_val["rig"].as_str().unwrap().to_string(),
                        seat: id_val["seat"].as_str().unwrap().to_string(),
                        ts: Some(parse_dt(id_val["ts"].as_str().unwrap())),
                    });
                }
            }

            if let Some(derived) = inputs.get("derived_evidence").and_then(Value::as_array) {
                for d in derived {
                    derived_evidence.push(DerivedIncident {
                        classification: d["classification"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string(),
                        threshold_breached: None,
                        seat: d.get("seat").and_then(Value::as_str).map(str::to_string),
                        qitem_id: d
                            .get("qitem_id")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        hold_id: d.get("hold_id").and_then(Value::as_str).map(str::to_string),
                        gate: d.get("gate").and_then(Value::as_str).map(str::to_string),
                        duration_secs: d
                            .get("duration_secs")
                            .and_then(Value::as_f64)
                            .unwrap_or(0.0),
                        total_tokens: d.get("total_tokens").and_then(Value::as_i64),
                        rework_tokens: d.get("rework_tokens").and_then(Value::as_i64),
                        chain_returns: d
                            .get("chain_returns")
                            .and_then(Value::as_u64)
                            .map(|v| v as usize),
                        return_rate: d.get("return_rate").and_then(Value::as_f64),
                        resolved: d.get("resolved").and_then(Value::as_bool),
                        source_keys: d
                            .get("source_keys")
                            .and_then(Value::as_array)
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(Value::as_str)
                                    .map(str::to_string)
                                    .collect()
                            })
                            .unwrap_or_default(),
                    });
                }
            }

            if let Some(q_facts) = inputs.get("queue_facts").and_then(Value::as_array) {
                for q in q_facts {
                    let ts_dt = parse_dt(q["ts"].as_str().unwrap());
                    queue_facts.push(OverviewQueueFact {
                        source_key: q["source_key"].as_str().unwrap().to_string(),
                        transition_id: q["transition_id"].as_i64().unwrap(),
                        qitem_id: q["qitem_id"].as_str().unwrap().to_string(),
                        ts_ms: ts_dt.timestamp_millis(),
                        state: q["state"].as_str().unwrap().to_string(),
                        source_seat: q
                            .get("source_seat")
                            .and_then(Value::as_str)
                            .unwrap_or("orch@test-rig")
                            .to_string(),
                        dest_seat: q
                            .get("dest_seat")
                            .and_then(Value::as_str)
                            .unwrap_or("dev-build@test-rig")
                            .to_string(),
                        closure_reason: q
                            .get("closure_reason")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        closure_target: q
                            .get("closure_target")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        is_recovery: q
                            .get("is_recovery")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                        recovery_reason: q
                            .get("recovery_reason")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        issue_tags: q
                            .get("issue_tags")
                            .and_then(Value::as_array)
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(Value::as_str)
                                    .map(str::to_string)
                                    .collect()
                            })
                            .unwrap_or_default(),
                    });
                }
            }

            if let Some(u_facts) = inputs.get("usage_facts").and_then(Value::as_array) {
                for u in u_facts {
                    let ts_opt = u.get("ts").and_then(Value::as_str).map(parse_dt);
                    let in_uncached = u.get("input_uncached").and_then(Value::as_i64).unwrap_or(0);
                    let out_tokens = u.get("output_tokens").and_then(Value::as_i64).unwrap_or(0);
                    let c_read = u.get("cache_read").and_then(Value::as_i64).unwrap_or(0);
                    let c_write = u.get("cache_write").and_then(Value::as_i64).unwrap_or(0);
                    usage_facts.push(OverviewUsageFact {
                        source_key: u["source_key"].as_str().unwrap().to_string(),
                        harness: u["harness"].as_str().unwrap().to_string(),
                        native_session: u["native_session"].as_str().unwrap().to_string(),
                        ts_ms: ts_opt.map(|dt| dt.timestamp_millis()),
                        seq: u.get("seq").and_then(Value::as_i64).unwrap_or(1),
                        total_tokens: in_uncached + out_tokens + c_read + c_write,
                        currency_total: u.get("currency_total").and_then(Value::as_f64),
                    });
                }
            }

            if let Some(s_facts) = inputs.get("skill_facts").and_then(Value::as_array) {
                for s in s_facts {
                    skill_facts.push(SkillFact {
                        source_key: s["source_key"].as_str().unwrap().to_string(),
                        seat: s["seat"].as_str().unwrap().to_string(),
                        skill: s["skill"].as_str().unwrap().to_string(),
                        invoked: s["invoked"].as_bool().unwrap(),
                    });
                }
            }

            if let Some(t_facts) = inputs.get("tool_facts").and_then(Value::as_array) {
                for t in t_facts {
                    let ts_dt = parse_dt(t["ts"].as_str().unwrap());
                    tool_facts.push(OverviewToolFact {
                        source_key: t["source_key"].as_str().unwrap().to_string(),
                        harness: t["harness"].as_str().unwrap().to_string(),
                        native_session: t["native_session"].as_str().unwrap().to_string(),
                        ts_ms: Some(ts_dt.timestamp_millis()),
                        tool_name: t["tool_name"].as_str().unwrap().to_string(),
                        failed: t.get("failed").and_then(Value::as_bool),
                    });
                }
            }

            if let Some(p_facts) = inputs.get("process_identities").and_then(Value::as_array) {
                for p in p_facts {
                    let f_dt = parse_dt(p["first_seen"].as_str().unwrap());
                    let l_dt = parse_dt(p["last_seen"].as_str().unwrap());
                    let e_dt = p.get("ended_at").and_then(Value::as_str).map(parse_dt);
                    process_identities.push(OverviewProcessIdentity {
                        identity_key: p["identity_key"].as_str().unwrap().to_string(),
                        harness: p.get("harness").and_then(Value::as_str).map(str::to_string),
                        session_link: p
                            .get("session_link")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        cwd_fingerprint: p
                            .get("cwd_fingerprint")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        first_seen_ms: f_dt.timestamp_millis(),
                        last_seen_ms: l_dt.timestamp_millis(),
                        ended_at_ms: e_dt.map(|dt| dt.timestamp_millis()),
                    });
                }
            }

            let facts = RawOverviewFacts {
                derived_evidence,
                queue_facts: &queue_facts,
                skill_facts: &skill_facts,
                usage_facts: &usage_facts,
                tool_facts: &tool_facts,
                process_identities: &process_identities,
                identity_facts: &identity_facts,
                activity_facts: &[],
                coverage,
                evidence_as_of: None,
            };

            let overview = evaluate_direct_overview_facts(&facts, &req).unwrap_or_else(|e| {
                panic!("Evaluation failed for case {}: {} ({:?})", idx + 1, id, e)
            });

            // Verify expected assertions
            if let Some(expected_cat) = expected.get("category").and_then(Value::as_str) {
                assert_eq!(
                    overview.critical_focus.category.as_deref(),
                    Some(expected_cat),
                    "Case {}: category",
                    id
                );
            }
            if let Some(expected_ent) = expected.get("entity_id") {
                if expected_ent.is_null() {
                    assert_eq!(
                        overview.critical_focus.entity_id, None,
                        "Case {}: entity_id null",
                        id
                    );
                } else if let Some(ent_str) = expected_ent.as_str() {
                    assert_eq!(
                        overview.critical_focus.entity_id.as_deref(),
                        Some(ent_str),
                        "Case {}: entity_id",
                        id
                    );
                }
            }
            if let Some(expected_dur) = expected.get("duration_secs").and_then(Value::as_f64) {
                assert_eq!(
                    overview.critical_focus.duration_secs,
                    Some(expected_dur),
                    "Case {}: duration_secs",
                    id
                );
            }
            if let Some(expected_tl) = expected.get("temporal_label").and_then(Value::as_str) {
                assert_eq!(
                    overview.critical_focus.temporal_label, expected_tl,
                    "Case {}: temporal_label",
                    id
                );
            }
            if let Some(expected_keys) =
                expected.get("ranked_source_keys").and_then(Value::as_array)
            {
                let keys: Vec<String> = expected_keys
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect();
                assert_eq!(
                    overview.critical_focus.ranked_source_keys, keys,
                    "Case {}: ranked_source_keys",
                    id
                );
            }
            if let Some(expected_keys) = expected
                .get("selected_source_keys")
                .and_then(Value::as_array)
            {
                let keys: Vec<String> = expected_keys
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect();
                assert_eq!(
                    overview.critical_focus.selected_source_keys, keys,
                    "Case {}: selected_source_keys",
                    id
                );
            }
            if let Some(expected_fs) = expected.get("focus_state").and_then(Value::as_str) {
                assert_eq!(
                    overview.critical_focus.focus_state, expected_fs,
                    "Case {}: focus_state",
                    id
                );
            }
            if let Some(expected_keys) = expected
                .get("state_support_source_keys")
                .and_then(Value::as_array)
            {
                let keys: Vec<String> = expected_keys
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect();
                assert_eq!(
                    overview.critical_focus.state_support_source_keys, keys,
                    "Case {}: state_support_source_keys",
                    id
                );
            }
            if let Some(expected_snap) = expected.get("snapshot").and_then(Value::as_bool) {
                assert_eq!(overview.snapshot, expected_snap, "Case {}: snapshot", id);
            }
            if let Some(expected_la) = expected.get("live_alert").and_then(Value::as_bool) {
                assert_eq!(overview.live_alert, expected_la, "Case {}: live_alert", id);
            }
            if let Some(expected_cov) = expected.get("coverage").and_then(Value::as_str) {
                assert_eq!(overview.coverage, expected_cov, "Case {}: coverage", id);
            }
            if let Some(expected_as_of) = expected.get("evidence_as_of").and_then(Value::as_str) {
                assert_eq!(
                    overview.evidence_as_of.as_deref(),
                    Some(expected_as_of),
                    "Case {}: evidence_as_of",
                    id
                );
            }

            // Context tax assertions
            if let Some(expected_utc) = expected.get("uncalled_tools_count") {
                if expected_utc.is_null() {
                    assert_eq!(
                        overview.context_tax.uncalled_tools_count, None,
                        "Case {}: uncalled_tools_count null",
                        id
                    );
                } else {
                    assert_eq!(
                        overview.context_tax.uncalled_tools_count,
                        Some(expected_utc.as_u64().unwrap() as usize),
                        "Case {}: uncalled_tools_count",
                        id
                    );
                }
            }
            if let Some(expected_ds) = expected.get("declaration_status").and_then(Value::as_str) {
                assert_eq!(
                    overview.context_tax.declaration_status, expected_ds,
                    "Case {}: declaration_status",
                    id
                );
            }
            if let Some(expected_skills) = expected.get("uncalled_skills").and_then(Value::as_array)
            {
                let skills: Vec<String> = expected_skills
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect();
                assert_eq!(
                    overview.context_tax.uncalled_skills, skills,
                    "Case {}: uncalled_skills",
                    id
                );
            }
            if let Some(expected_decl_keys) = expected
                .get("declaration_source_keys")
                .and_then(Value::as_array)
            {
                let keys: Vec<String> = expected_decl_keys
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect();
                assert_eq!(
                    overview.context_tax.declaration_source_keys, keys,
                    "Case {}: declaration_source_keys",
                    id
                );
            }
            if let Some(expected_inv_keys) = expected
                .get("invocation_support_source_keys")
                .and_then(Value::as_array)
            {
                let keys: Vec<String> = expected_inv_keys
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect();
                assert_eq!(
                    overview.context_tax.invocation_support_source_keys, keys,
                    "Case {}: invocation_support_source_keys",
                    id
                );
            }

            // Throughput & Cost assertions
            if let Some(expected_closed) =
                expected.get("closed_issue_qitems").and_then(Value::as_u64)
            {
                assert_eq!(
                    overview.throughput_and_cost.closed_issue_qitems, expected_closed as usize,
                    "Case {}: closed_issue_qitems",
                    id
                );
            }
            if let Some(expected_internal) = expected
                .get("closed_internal_tasks")
                .and_then(Value::as_u64)
            {
                assert_eq!(
                    overview.throughput_and_cost.closed_internal_tasks, expected_internal as usize,
                    "Case {}: closed_internal_tasks",
                    id
                );
            }
            if let Some(expected_ids) = expected
                .get("closed_issue_qitem_ids")
                .and_then(Value::as_array)
            {
                let ids: Vec<String> = expected_ids
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect();
                assert_eq!(
                    overview.throughput_and_cost.closed_issue_qitem_ids, ids,
                    "Case {}: closed_issue_qitem_ids",
                    id
                );
            }
            if let Some(expected_spend) = expected.get("total_fleet_spend_usd") {
                if expected_spend.is_null() {
                    assert_eq!(
                        overview.throughput_and_cost.total_fleet_spend_usd, None,
                        "Case {}: total_fleet_spend_usd null",
                        id
                    );
                } else {
                    assert_eq!(
                        overview.throughput_and_cost.total_fleet_spend_usd,
                        Some(expected_spend.as_f64().unwrap()),
                        "Case {}: total_fleet_spend_usd",
                        id
                    );
                }
            }
            if let Some(expected_sub) = expected.get("observed_priced_subtotal_usd") {
                if expected_sub.is_null() {
                    assert_eq!(
                        overview.throughput_and_cost.observed_priced_subtotal_usd, None,
                        "Case {}: observed_priced_subtotal_usd null",
                        id
                    );
                } else {
                    assert_eq!(
                        overview.throughput_and_cost.observed_priced_subtotal_usd,
                        Some(expected_sub.as_f64().unwrap()),
                        "Case {}: observed_priced_subtotal_usd",
                        id
                    );
                }
            }
            if let Some(expected_avg) = expected.get("avg_cost_per_closed_issue_qitem") {
                if expected_avg.is_null() {
                    assert_eq!(
                        overview.throughput_and_cost.avg_cost_per_closed_issue_qitem, None,
                        "Case {}: avg_cost_per_closed_issue_qitem null",
                        id
                    );
                } else {
                    assert_eq!(
                        overview.throughput_and_cost.avg_cost_per_closed_issue_qitem,
                        Some(expected_avg.as_f64().unwrap()),
                        "Case {}: avg_cost_per_closed_issue_qitem",
                        id
                    );
                }
            }
            if let Some(expected_avg_label) = expected.get("average_label").and_then(Value::as_str)
            {
                assert_eq!(
                    overview.throughput_and_cost.average_label.as_deref(),
                    Some(expected_avg_label),
                    "Case {}: average_label",
                    id
                );
            }
            if let Some(expected_ms) = expected.get("money_status").and_then(Value::as_str) {
                assert_eq!(
                    overview.throughput_and_cost.money_status, expected_ms,
                    "Case {}: money_status",
                    id
                );
            }
            if let Some(expected_pr) = expected.get("priced_rows").and_then(Value::as_u64) {
                assert_eq!(
                    overview.throughput_and_cost.priced_rows, expected_pr as usize,
                    "Case {}: priced_rows",
                    id
                );
            }
            if let Some(expected_ur) = expected.get("unpriced_rows").and_then(Value::as_u64) {
                assert_eq!(
                    overview.throughput_and_cost.unpriced_rows, expected_ur as usize,
                    "Case {}: unpriced_rows",
                    id
                );
            }
            if let Some(expected_keys) = expected
                .get("closed_issue_source_keys")
                .and_then(Value::as_array)
            {
                let keys: Vec<String> = expected_keys
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect();
                assert_eq!(
                    overview.throughput_and_cost.closed_issue_source_keys, keys,
                    "Case {}: closed_issue_source_keys",
                    id
                );
            }
            if let Some(expected_keys) = expected.get("spend_source_keys").and_then(Value::as_array)
            {
                let keys: Vec<String> = expected_keys
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect();
                assert_eq!(
                    overview.throughput_and_cost.spend_source_keys, keys,
                    "Case {}: spend_source_keys",
                    id
                );
            }
            if let Some(expected_keys) =
                expected.get("priced_source_keys").and_then(Value::as_array)
            {
                let keys: Vec<String> = expected_keys
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect();
                assert_eq!(
                    overview.throughput_and_cost.priced_source_keys, keys,
                    "Case {}: priced_source_keys",
                    id
                );
            }
            if let Some(expected_keys) = expected
                .get("unpriced_source_keys")
                .and_then(Value::as_array)
            {
                let keys: Vec<String> = expected_keys
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect();
                assert_eq!(
                    overview.throughput_and_cost.unpriced_source_keys, keys,
                    "Case {}: unpriced_source_keys",
                    id
                );
            }
            if let Some(expected_keys) = expected
                .get("excluded_source_keys")
                .and_then(Value::as_array)
            {
                let keys: Vec<String> = expected_keys
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect();
                let mut combined = overview.throughput_and_cost.excluded_source_keys.clone();
                combined.extend(overview.excluded_source_keys.clone());
                combined.sort();
                combined.dedup();
                for k in &keys {
                    assert!(
                        combined.contains(k),
                        "Case {}: excluded_source_keys missing {}",
                        id,
                        k
                    );
                }
            }
            if let Some(expected_keys) = expected
                .get("eligible_closure_support_source_keys")
                .and_then(Value::as_array)
            {
                let keys: Vec<String> = expected_keys
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect();
                assert_eq!(
                    overview
                        .throughput_and_cost
                        .eligible_closure_support_source_keys,
                    keys,
                    "Case {}: eligible_closure_support_source_keys",
                    id
                );
            }
            if let Some(expected_keys) = expected
                .get("recovery_excluded_source_keys")
                .and_then(Value::as_array)
            {
                let keys: Vec<String> = expected_keys
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect();
                assert_eq!(
                    overview.throughput_and_cost.recovery_excluded_source_keys, keys,
                    "Case {}: recovery_excluded_source_keys",
                    id
                );
            }
            if let Some(expected_keys) = expected
                .get("reopen_state_support_source_keys")
                .and_then(Value::as_array)
            {
                let keys: Vec<String> = expected_keys
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect();
                assert_eq!(
                    overview
                        .throughput_and_cost
                        .reopen_state_support_source_keys,
                    keys,
                    "Case {}: reopen_state_support_source_keys",
                    id
                );
            }

            // CWD Projects assertions
            if let Some(expected_groups) = expected.get("project_groups").and_then(Value::as_array)
            {
                assert_eq!(
                    overview.project_groups.len(),
                    expected_groups.len(),
                    "Case {}: project_groups len",
                    id
                );
                for (g_idx, eg) in expected_groups.iter().enumerate() {
                    let actual_g = &overview.project_groups[g_idx];
                    assert_eq!(
                        actual_g.cwd_fingerprint,
                        eg["cwd_fingerprint"].as_str().unwrap(),
                        "Case {}: cwd_fingerprint",
                        id
                    );
                    assert_eq!(
                        actual_g.total_tokens,
                        eg["total_tokens"].as_i64().unwrap(),
                        "Case {}: total_tokens",
                        id
                    );
                    if let Some(cost) = eg.get("estimated_cost_usd").and_then(Value::as_f64) {
                        assert_eq!(
                            actual_g.estimated_cost_usd,
                            Some(cost),
                            "Case {}: estimated_cost_usd",
                            id
                        );
                    }
                    if let Some(keys) = eg.get("usage_source_keys").and_then(Value::as_array) {
                        let k_vec: Vec<String> = keys
                            .iter()
                            .filter_map(Value::as_str)
                            .map(str::to_string)
                            .collect();
                        assert_eq!(
                            actual_g.usage_source_keys, k_vec,
                            "Case {}: usage_source_keys",
                            id
                        );
                    }
                }
            }
            if let Some(expected_assigned) = expected
                .get("assigned_usage_source_keys")
                .and_then(Value::as_array)
            {
                let keys: Vec<String> = expected_assigned
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect();
                assert_eq!(
                    overview.assigned_usage_source_keys, keys,
                    "Case {}: assigned_usage_source_keys",
                    id
                );
            }
            if let Some(expected_unassigned) = expected
                .get("unassigned_usage_source_keys")
                .and_then(Value::as_array)
            {
                let keys: Vec<String> = expected_unassigned
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect();
                assert_eq!(
                    overview.unassigned_usage_source_keys, keys,
                    "Case {}: unassigned_usage_source_keys",
                    id
                );
            }
            if let Some(expected_ut) = expected
                .get("unassigned_total_tokens")
                .and_then(Value::as_i64)
            {
                assert_eq!(
                    overview.unassigned_total_tokens, expected_ut,
                    "Case {}: unassigned_total_tokens",
                    id
                );
            }
            if let Some(expected_uc) = expected.get("unassigned_observed_cost_usd") {
                if expected_uc.is_null() {
                    assert_eq!(
                        overview.unassigned_observed_cost_usd, None,
                        "Case {}: unassigned_observed_cost_usd null",
                        id
                    );
                } else {
                    assert_eq!(
                        overview.unassigned_observed_cost_usd,
                        Some(expected_uc.as_f64().unwrap()),
                        "Case {}: unassigned_observed_cost_usd",
                        id
                    );
                }
            }
            if let Some(expected_at) = expected
                .get("all_projects_total_tokens")
                .and_then(Value::as_i64)
            {
                assert_eq!(
                    overview.all_projects_total_tokens, expected_at,
                    "Case {}: all_projects_total_tokens",
                    id
                );
            }
            if let Some(expected_ac) = expected.get("all_projects_observed_cost_usd") {
                if expected_ac.is_null() {
                    assert_eq!(
                        overview.all_projects_observed_cost_usd, None,
                        "Case {}: all_projects_observed_cost_usd null",
                        id
                    );
                } else {
                    assert_eq!(
                        overview.all_projects_observed_cost_usd,
                        Some(expected_ac.as_f64().unwrap()),
                        "Case {}: all_projects_observed_cost_usd",
                        id
                    );
                }
            }
            if let Some(expected_rc) = expected.get("raw_cwd_path_count").and_then(Value::as_u64) {
                assert_eq!(
                    overview.raw_cwd_path_count, expected_rc as usize,
                    "Case {}: raw_cwd_path_count",
                    id
                );
            }

            // Scoped usage assertions
            if let Some(expected_tokens) = expected.get("total_tokens").and_then(Value::as_i64) {
                assert_eq!(
                    overview.total_tokens, expected_tokens,
                    "Case {}: total_tokens",
                    id
                );
            }
            if let Some(expected_b_sub) = expected
                .get("bounded_observed_priced_subtotal_usd")
                .and_then(Value::as_f64)
            {
                assert_eq!(
                    overview.bounded_observed_priced_subtotal_usd,
                    Some(expected_b_sub),
                    "Case {}: bounded_observed_priced_subtotal_usd",
                    id
                );
            }
            if let Some(expected_keys) = expected.get("usage_source_keys").and_then(Value::as_array)
            {
                let keys: Vec<String> = expected_keys
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect();
                assert_eq!(
                    overview.usage_source_keys, keys,
                    "Case {}: usage_source_keys",
                    id
                );
            }
            if let Some(expected_keys) = expected
                .get("undated_coverage_source_keys")
                .and_then(Value::as_array)
            {
                let keys: Vec<String> = expected_keys
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect();
                assert_eq!(
                    overview.undated_coverage_source_keys, keys,
                    "Case {}: undated_coverage_source_keys",
                    id
                );
            }
        }
    }
}

#[cfg(test)]
mod repair_regressions;
