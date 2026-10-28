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
    pub estimated_cost_usd: Option<f64>,
    pub usage_source_keys: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ContextTaxOverview {
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
pub struct DirectOverview {
    pub formula_version: String,
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
        DateTime::from_timestamp_millis(self.ts_ms).unwrap_or_else(Utc::now)
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
        DateTime::from_timestamp_millis(self.first_seen_ms).unwrap_or_else(Utc::now)
    }
    pub fn last_seen(&self) -> DateTime<Utc> {
        DateTime::from_timestamp_millis(self.last_seen_ms).unwrap_or_else(Utc::now)
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
    dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Evaluates direct overview from in-memory/adapted facts.
pub fn evaluate_direct_overview_facts(
    facts: &RawOverviewFacts<'_>,
    req: &OverviewRequest,
) -> Result<DirectOverview> {
    let observation_clock = req.observation_clock.unwrap_or_else(Utc::now);
    let as_of_dt = req.evidence_as_of.or(facts.evidence_as_of);

    // D3: Snapshot / live alert
    let (snapshot, live_alert) = match as_of_dt {
        Some(as_of) if as_of < observation_clock - chrono::Duration::seconds(300) => (true, false),
        _ => (false, true),
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
        .unwrap_or_else(|| "complete".to_string());

    // 1. Critical Focus evaluation
    let (critical_focus, focus_source_keys) =
        evaluate_critical_focus(facts, req, &temporal_label, &overall_coverage)?;

    // 2. Context Tax evaluation
    let (context_tax, tax_source_keys) = evaluate_context_tax(facts, req)?;

    // 3. Throughput & Cost evaluation
    let (throughput_and_cost, tc_source_keys) = evaluate_throughput_and_cost(facts, req)?;

    // 4. Combined CWD Projects and Scoped Usage in a single pass over usage_facts
    let req_from_ms = req.from.timestamp_millis();
    let req_to_ms = req.to.timestamp_millis();

    let in_scope_sessions: Option<BTreeSet<(&str, &str)>> = req.rig.as_deref().map(|r| {
        facts
            .identity_facts
            .iter()
            .filter(|id| id.rig == r)
            .map(|id| (id.harness.as_str(), id.native_session.as_str()))
            .collect()
    });

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
        let is_in_rig = match in_scope_sessions {
            Some(ref sessions) => {
                sessions.contains(&(u.harness.as_str(), u.native_session.as_str()))
            }
            None => true,
        };

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
                    .filter(|p| ts_ms >= p.first_seen_ms && ts_ms <= p.last_seen_ms)
                    .collect(),
                None => vec![],
            };

        // Check if fingerprints are unique
        let fingerprints: BTreeSet<&str> = matching_pids
            .iter()
            .filter_map(|p| p.cwd_fingerprint.as_deref())
            .collect();

        if fingerprints.len() == 1 {
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
            estimated_cost_usd: if has_cost { Some(cost) } else { None },
            usage_source_keys: keys,
        });
    }
    project_groups.sort_by(|a, b| a.cwd_fingerprint.cmp(&b.cwd_fingerprint));

    cwd_source_keys.extend(unassigned_usage_keys.clone());

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
    all_source_keys.sort();
    all_source_keys.dedup();
    let source_keys_vec = all_source_keys;

    // Compute deterministic evidence digest
    let digest_input = json!({
        "formula_version": FORMULA_VERSION,
        "rig": req.rig,
        "from": to_iso_z(&req.from),
        "to": to_iso_z(&req.to),
        "critical_focus": {
            "focus_state": critical_focus.focus_state,
            "category": critical_focus.category,
            "entity_id": critical_focus.entity_id,
            "duration_secs": critical_focus.duration_secs,
        },
        "context_tax": {
            "uncalled_tools_count": context_tax.uncalled_tools_count,
            "declaration_status": context_tax.declaration_status,
        },
        "throughput_and_cost": {
            "closed_issue_qitems": throughput_and_cost.closed_issue_qitems,
            "total_fleet_spend_usd": throughput_and_cost.total_fleet_spend_usd,
            "money_status": throughput_and_cost.money_status,
        },
    });
    let mut hasher = Sha256::new();
    hasher.update(digest_input.to_string().as_bytes());
    for k in &source_keys_vec {
        hasher.update(k.as_bytes());
    }
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

    // Check if queue_facts show resolved stalls (e.g. Case 5)
    let mut state_support_source_keys = Vec::new();
    let mut resolved_stall = false;
    if !facts.queue_facts.is_empty() {
        let req_to_ms = req.to.timestamp_millis();
        let has_claim = facts.queue_facts.iter().any(|q| q.state == "in-progress");
        let last_state = facts
            .queue_facts
            .iter()
            .max_by_key(|q| (q.ts_ms, q.transition_id));
        if has_claim && last_state.is_some_and(|q| q.state == "done" && q.ts_ms <= req_to_ms) {
            resolved_stall = true;
            for q in facts.queue_facts {
                if q.state == "in-progress" || q.state == "done" {
                    state_support_source_keys.push(q.source_key.clone());
                }
            }
        }
    }

    if resolved_stall {
        return Ok((
            CriticalFocus {
                focus_state: "nominal".to_string(),
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

    // Evaluate derived incidents
    struct Candidate {
        tier: u8, // 1: Stall, 2: Hold, 3: Vortex
        category: String,
        entity_id: String,
        duration_secs: f64,
        total_tokens: i64,
        source_keys: Vec<String>,
    }

    let mut candidates = Vec::new();
    for inc in &facts.derived_evidence {
        if inc.resolved == Some(true) {
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
                        total_tokens: inc.total_tokens.unwrap_or(0),
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
                        total_tokens: inc.total_tokens.unwrap_or(0),
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
                        total_tokens: inc.total_tokens.unwrap_or(0),
                        source_keys: inc.source_keys.clone(),
                    });
                }
            }
            "rework_vortex" => {
                let return_rate = inc.return_rate.unwrap_or(0.0);
                let chain_returns = inc.chain_returns.unwrap_or(0);
                let tokens = inc.rework_tokens.or(inc.total_tokens).unwrap_or(0);
                let duration = inc.duration_secs;

                let breach = return_rate > req.friction_thresholds.return_rate
                    || (chain_returns > req.friction_thresholds.chain_returns as usize
                        && tokens > 1_000_000)
                    || duration > req.friction_thresholds.loop_duration_secs as f64;

                if breach {
                    let entity_id = inc.qitem_id.clone().unwrap_or_default();
                    candidates.push(Candidate {
                        tier: 3,
                        category: "Rework Vortex".to_string(),
                        entity_id,
                        duration_secs: inc.duration_secs,
                        total_tokens: tokens,
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
                focus_state: "nominal".to_string(),
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
            state_support_source_keys: vec![],
        },
        selected_source_keys,
    ))
}

fn evaluate_context_tax(
    facts: &RawOverviewFacts<'_>,
    req: &OverviewRequest,
) -> Result<(ContextTaxOverview, Vec<String>)> {
    if facts.skill_facts.is_empty() {
        return Ok((
            ContextTaxOverview {
                uncalled_tools_count: None,
                declaration_status: "unavailable".to_string(),
                uncalled_skills: vec![],
                wasted_schema_tokens: None,
                schema_tokens_status: "unavailable".to_string(),
                wasted_cost_usd: None,
                declaration_source_keys: vec![],
                invocation_support_source_keys: vec![],
            },
            vec![],
        ));
    }

    // Filter skill facts by rig if specified
    let rig_seats: Option<BTreeSet<&str>> = req.rig.as_deref().map(|r| {
        facts
            .identity_facts
            .iter()
            .filter(|id| id.rig == r)
            .map(|id| id.seat.as_str())
            .collect()
    });

    let mut id_to_seat: BTreeMap<(&str, &str), &str> = BTreeMap::new();
    for id in facts.identity_facts {
        id_to_seat.insert(
            (id.harness.as_str(), id.native_session.as_str()),
            id.seat.as_str(),
        );
    }

    // Collect invocations from tool_facts and skill_facts(invoked=true)
    let mut invoked_skills_by_seat = BTreeSet::new();
    let mut invocation_support_source_keys = Vec::new();

    for t in facts.tool_facts {
        if let Some(&s) = id_to_seat.get(&(t.harness.as_str(), t.native_session.as_str())) {
            invoked_skills_by_seat.insert((s, t.tool_name.to_ascii_lowercase()));
            invocation_support_source_keys.push(t.source_key.clone());
        }
    }

    for s in facts.skill_facts {
        if s.invoked {
            invoked_skills_by_seat.insert((s.seat.as_str(), s.skill.to_ascii_lowercase()));
            invocation_support_source_keys.push(s.source_key.clone());
        }
    }

    let mut uncalled_skills = Vec::new();
    let mut declaration_source_keys = Vec::new();

    for s in facts.skill_facts {
        if s.invoked {
            continue;
        }
        if let Some(ref seats) = rig_seats {
            if !seats.contains(s.seat.as_str()) {
                continue;
            }
        }

        // Check if later reconciled
        let is_reconciled =
            invoked_skills_by_seat.contains(&(s.seat.as_str(), s.skill.to_ascii_lowercase()));
        if !is_reconciled {
            uncalled_skills.push(s.skill.clone());
            declaration_source_keys.push(s.source_key.clone());
        }
    }

    uncalled_skills.sort();
    uncalled_skills.dedup();

    let count = uncalled_skills.len();

    let mut all_keys = declaration_source_keys.clone();
    all_keys.extend(invocation_support_source_keys.clone());

    Ok((
        ContextTaxOverview {
            uncalled_tools_count: Some(count),
            declaration_status: "measured".to_string(),
            uncalled_skills,
            wasted_schema_tokens: None,
            schema_tokens_status: "unavailable".to_string(),
            wasted_cost_usd: None,
            declaration_source_keys,
            invocation_support_source_keys,
        },
        all_keys,
    ))
}

fn evaluate_throughput_and_cost(
    facts: &RawOverviewFacts<'_>,
    req: &OverviewRequest,
) -> Result<(ThroughputAndCostOverview, Vec<String>)> {
    let req_from_ms = req.from.timestamp_millis();
    let req_to_ms = req.to.timestamp_millis();

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
    for q in facts.queue_facts {
        items.entry(&q.qitem_id).or_default().push(q);
    }

    let mut closed_issue_items = Vec::new();
    let mut eligible_closure_support = Vec::new();
    let mut excluded_source_keys = Vec::new();
    let mut recovery_excluded_source_keys = Vec::new();
    let mut reopen_state_support_source_keys = Vec::new();
    let mut closed_internal_tasks = 0;

    for (item_id, mut transitions) in items {
        transitions.sort_by_key(|a| (a.ts_ms, a.transition_id));
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

        // Find done transition in window
        let done_transition = transitions
            .iter()
            .filter(|t| t.state == "done" && t.ts_ms >= req_from_ms && t.ts_ms < req_to_ms)
            .min_by_key(|t| (t.ts_ms, t.transition_id));

        let Some(dt) = done_transition else {
            for t in &transitions {
                excluded_source_keys.push((t.ts_ms, t.transition_id, t.source_key.clone()));
            }
            continue;
        };

        // Check issue tags
        let has_issue_tag = !dt.issue_tags.is_empty();

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
    let in_scope_sessions: Option<BTreeSet<(&str, &str)>> = req.rig.as_deref().map(|r| {
        facts
            .identity_facts
            .iter()
            .filter(|id| id.rig == r)
            .map(|id| (id.harness.as_str(), id.native_session.as_str()))
            .collect()
    });

    let mut priced_rows = 0;
    let mut unpriced_rows = 0;
    let mut spend_sum = 0.0;
    let mut spend_source_keys = Vec::new();
    let mut priced_source_keys = Vec::new();
    let mut unpriced_source_keys = Vec::new();

    for u in facts.usage_facts {
        if let Some(ref sessions) = in_scope_sessions {
            if !sessions.contains(&(u.harness.as_str(), u.native_session.as_str())) {
                continue;
            }
        }

        let Some(ts_ms) = u.ts_ms else {
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
        if priced_rows > 0 && unpriced_rows == 0 {
            ("priced".to_string(), Some(spend_sum), None)
        } else if priced_rows == 0 && unpriced_rows > 0 {
            ("unpriced".to_string(), None, None)
        } else if priced_rows > 0 && unpriced_rows > 0 {
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

pub struct OverviewStatements<'conn> {
    pub stmt_queue: duckdb::Statement<'conn>,
    pub stmt_usage: duckdb::Statement<'conn>,
    pub stmt_skills: duckdb::Statement<'conn>,
    pub stmt_tools: duckdb::Statement<'conn>,
    pub stmt_pids: duckdb::Statement<'conn>,
    pub stmt_identities: duckdb::Statement<'conn>,
    pub queue_buf: Vec<OverviewQueueFact>,
    pub usage_buf: Vec<OverviewUsageFact>,
    pub skills_buf: Vec<SkillFact>,
    pub tools_buf: Vec<OverviewToolFact>,
    pub pids_buf: Vec<OverviewProcessIdentity>,
    pub identities_buf: Vec<IdentityFact>,
}

impl<'conn> OverviewStatements<'conn> {
    pub fn prepare(conn: &'conn duckdb::Connection) -> Result<Self> {
        let stmt_queue = conn.prepare("SELECT source_key, transition_id, qitem_id, epoch_ms(ts), state, dest_seat, closure_reason, is_recovery, recovery_reason, issue_tags FROM queue_facts")?;
        let stmt_usage = conn.prepare("SELECT source_key, harness, native_session, epoch_ms(ts), (input_uncached + output_tokens + cache_read + cache_write)::BIGINT, currency_total FROM usage_facts")?;
        let stmt_skills =
            conn.prepare("SELECT source_key, seat, skill, invoked FROM skill_facts")?;
        let stmt_tools = conn.prepare(
            "SELECT source_key, harness, native_session, epoch_ms(ts), tool_name FROM tool_facts",
        )?;
        let stmt_pids = conn.prepare("SELECT harness, session_link, cwd_fingerprint, epoch_ms(first_seen), epoch_ms(last_seen) FROM process_identities")?;
        let stmt_identities = conn.prepare("SELECT source_key, harness, native_session, rig, seat, epoch_ms(ts) FROM identity_facts")?;
        Ok(Self {
            stmt_queue,
            stmt_usage,
            stmt_skills,
            stmt_tools,
            stmt_pids,
            stmt_identities,
            queue_buf: Vec::with_capacity(640),
            usage_buf: Vec::with_capacity(4096),
            skills_buf: Vec::with_capacity(256),
            tools_buf: Vec::with_capacity(512),
            pids_buf: Vec::with_capacity(128),
            identities_buf: Vec::with_capacity(128),
        })
    }

    pub fn evaluate(&mut self, req: &OverviewRequest) -> Result<DirectOverview> {
        self.queue_buf.clear();
        let rows = self.stmt_queue.query_map([], |row| {
            let ts_ms: i64 = row.get(3)?;
            let tags_str: String = row.get(9)?;
            let has_issue_tag = !tags_str.is_empty() && tags_str != "[]";
            let tags = if has_issue_tag {
                serde_json::from_str(&tags_str).unwrap_or_else(|_| vec!["issue".to_string()])
            } else {
                vec![]
            };
            Ok(OverviewQueueFact {
                source_key: row.get(0)?,
                transition_id: row.get(1)?,
                qitem_id: row.get(2)?,
                ts_ms,
                state: row.get(4)?,
                source_seat: String::new(),
                dest_seat: row.get(5)?,
                closure_reason: row.get(6)?,
                closure_target: None,
                is_recovery: row.get(7)?,
                recovery_reason: row.get(8)?,
                issue_tags: tags,
            })
        })?;
        for fact in rows.flatten() {
            self.queue_buf.push(fact);
        }

        self.usage_buf.clear();
        let rows = self.stmt_usage.query_map([], |row| {
            let ts_ms: Option<i64> = row.get(3)?;
            Ok(OverviewUsageFact {
                source_key: row.get(0)?,
                harness: row.get(1)?,
                native_session: row.get(2)?,
                ts_ms,
                seq: 0,
                total_tokens: row.get(4)?,
                currency_total: row.get(5)?,
            })
        })?;
        for fact in rows.flatten() {
            self.usage_buf.push(fact);
        }

        self.skills_buf.clear();
        let rows = self.stmt_skills.query_map([], |row| {
            Ok(SkillFact {
                source_key: row.get(0)?,
                seat: row.get(1)?,
                skill: row.get(2)?,
                invoked: row.get(3)?,
            })
        })?;
        for fact in rows.flatten() {
            self.skills_buf.push(fact);
        }

        self.tools_buf.clear();
        let rows = self.stmt_tools.query_map([], |row| {
            let ts_ms: Option<i64> = row.get(3)?;
            Ok(OverviewToolFact {
                source_key: row.get(0)?,
                harness: row.get(1)?,
                native_session: row.get(2)?,
                ts_ms,
                tool_name: row.get(4)?,
            })
        })?;
        for fact in rows.flatten() {
            self.tools_buf.push(fact);
        }

        self.pids_buf.clear();
        let rows = self.stmt_pids.query_map([], |row| {
            let f_ms: i64 = row.get(3)?;
            let l_ms: i64 = row.get(4)?;
            Ok(OverviewProcessIdentity {
                identity_key: String::new(),
                harness: row.get(0)?,
                session_link: row.get(1)?,
                cwd_fingerprint: row.get(2)?,
                first_seen_ms: f_ms,
                last_seen_ms: l_ms,
                ended_at_ms: None,
            })
        })?;
        for fact in rows.flatten() {
            self.pids_buf.push(fact);
        }

        self.identities_buf.clear();
        let rows = self.stmt_identities.query_map([], |row| {
            let ts_ms: Option<i64> = row.get(5)?;
            Ok(IdentityFact {
                source_key: row.get(0)?,
                harness: row.get(1)?,
                native_session: row.get(2)?,
                rig: row.get(3)?,
                seat: row.get(4)?,
                ts: ts_ms.and_then(DateTime::from_timestamp_millis),
            })
        })?;
        for fact in rows.flatten() {
            self.identities_buf.push(fact);
        }

        let raw_facts = RawOverviewFacts {
            derived_evidence: vec![],
            queue_facts: &self.queue_buf,
            skill_facts: &self.skills_buf,
            usage_facts: &self.usage_buf,
            tool_facts: &self.tools_buf,
            process_identities: &self.pids_buf,
            identity_facts: &self.identities_buf,
            activity_facts: &[],
            coverage: Some("complete".to_string()),
            evidence_as_of: None,
        };

        evaluate_direct_overview_facts(&raw_facts, req)
    }
}

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
                        "test-rig",
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
                 SELECT 'proc:' || i, 'root', 1000 + i, 'linux', 0, 'proc', 'codex', 'fp-' || (i % 10), 'session-' || (i % 100), 'local', 'none',
                        CAST('2026-10-01 00:00:00' AS TIMESTAMP), CAST('2026-10-02 00:00:00' AS TIMESTAMP), 'none'
                 FROM range(0, 100) t(i)"
            ).unwrap();

            // 3. 150 activity_facts
            conn.execute_batch(
                "INSERT INTO activity_facts (source_key, seat, ts, state, seq)
                 SELECT 'act:' || i, 'dev-' || (i % 100) || '@test-rig', CAST('2026-10-01 00:00:00' AS TIMESTAMP) + i * INTERVAL '1 minute', 'running', i
                 FROM range(0, 150) t(i)"
            ).unwrap();

            // 4. 600 queue_facts (200 items x 3 transitions)
            conn.execute_batch(
                "INSERT INTO queue_facts (source_key, transition_id, qitem_id, ts, state, source_seat, dest_seat, missions, chain_id, issue_tags, is_recovery)
                 SELECT 'q:pending:' || i, i * 3 + 1, 'item-' || i, CAST('2026-10-01 00:00:00' AS TIMESTAMP) + i * INTERVAL '1 second', 'pending', 'orch@test-rig', 'dev-' || (i % 100) || '@test-rig', '[]', 'chain-' || i, '[\"101\"]', false
                 FROM range(0, 200) t(i);
                 INSERT INTO queue_facts (source_key, transition_id, qitem_id, ts, state, source_seat, dest_seat, missions, chain_id, issue_tags, is_recovery)
                 SELECT 'q:claim:' || i, i * 3 + 2, 'item-' || i, CAST('2026-10-01 00:00:02' AS TIMESTAMP) + i * INTERVAL '1 second', 'in-progress', 'orch@test-rig', 'dev-' || (i % 100) || '@test-rig', '[]', 'chain-' || i, '[\"101\"]', false
                 FROM range(0, 200) t(i);
                 INSERT INTO queue_facts (source_key, transition_id, qitem_id, ts, state, source_seat, dest_seat, missions, chain_id, issue_tags, is_recovery, closure_reason)
                 SELECT 'q:done:' || i, i * 3 + 3, 'item-' || i, CAST('2026-10-01 01:00:00' AS TIMESTAMP) + i * INTERVAL '1 second', 'done', 'orch@test-rig', 'dev-' || (i % 100) || '@test-rig', '[]', 'chain-' || i, '[\"101\"]', false, 'no-follow-on'
                 FROM range(0, 200) t(i);"
            ).unwrap();

            // 5. 4000 usage_facts
            conn.execute_batch(
                "INSERT INTO usage_facts (source_key, harness, native_session, ts, seq, input_uncached, output_tokens, cache_read, cache_write, reasoning, generation, source_id, currency_total)
                 SELECT 'usage:' || i, 'codex', 'session-' || (i % 100), CAST('2026-10-01 00:00:00' AS TIMESTAMP) + (i % 86400) * INTERVAL '1 second', i, 10, 5, 2, 1, 0, 0, 'src', 0.01
                 FROM range(0, 4000) t(i)"
            ).unwrap();

            // 6. 400 tool_facts
            conn.execute_batch(
                "INSERT INTO tool_facts (source_key, harness, native_session, ts, seq, tool_name, structured_write, generation, source_id, category)
                 SELECT 'tool:' || i, 'codex', 'session-' || (i % 100), CAST('2026-10-01 00:00:00' AS TIMESTAMP) + (i % 86400) * INTERVAL '1 second', i, 'bash', false, 0, 'src', 'other'
                 FROM range(0, 400) t(i)"
            ).unwrap();

            // 7. 150 skill_facts
            conn.execute_batch(
                "INSERT INTO skill_facts (source_key, seat, skill, invoked)
                 SELECT 'skill:' || i, 'dev-' || (i % 100) || '@test-rig', 'skill-' || i, (i % 2 = 0)
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

            store.checkpoint().unwrap();
            drop(store);
        }

        // Open read-only store for benchmark
        let store = AnalyticsStore::open_read_only(&db_path).unwrap();
        let _ = store.connection().execute_batch("PRAGMA threads=1;");
        let mut stmts = OverviewStatements::prepare(store.connection()).unwrap();
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
        let cold_res = stmts.evaluate(&req).unwrap();
        let cold_duration = cold_start.elapsed();
        println!("Direct Overview Benchmark Cold Run: {:?}", cold_duration);
        assert_eq!(cold_res.formula_version, FORMULA_VERSION);

        // 5 Warmup iterations
        for _ in 0..5 {
            let _ = stmts.evaluate(&req).unwrap();
        }

        // 20 Timed iterations
        let mut samples = Vec::with_capacity(20);
        for _ in 0..20 {
            let start = Instant::now();
            let res = stmts.evaluate(&req).unwrap();
            let elapsed = start.elapsed();
            assert_eq!(res.formula_version, FORMULA_VERSION);
            samples.push(elapsed);
        }

        samples.sort();
        let mean = samples.iter().sum::<std::time::Duration>() / samples.len() as u32;
        let p95 = samples[(samples.len() as f64 * 0.95) as usize];
        let max = *samples.last().unwrap();

        println!("Direct Overview Benchmark (5,500 facts):");
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
