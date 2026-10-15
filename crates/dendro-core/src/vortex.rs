//! Observed-history handoff friction. Legacy handoff metrics keep their definitions.
use std::collections::{BTreeMap, BTreeSet};

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::metrics::{self, HandoffEdge, SessionBinding, Window};
use crate::{ActivityFact, DestinationSnapshot, IdentityFact, QueueFact, UsageFact};

pub const FORMULA_VERSION: &str = "handoff-friction-v1";

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FrictionThresholds {
    pub return_rate: f64,
    pub chain_returns: u64,
    pub rework_tokens: u64,
    pub loop_duration_secs: u64,
}

impl Default for FrictionThresholds {
    fn default() -> Self {
        Self {
            return_rate: 0.10,
            chain_returns: 2,
            rework_tokens: 1_000_000,
            loop_duration_secs: 600,
        }
    }
}

impl FrictionThresholds {
    pub fn validate(self) -> Result<Self> {
        if !self.return_rate.is_finite() || !(0.0..=1.0).contains(&self.return_rate) {
            return Err(anyhow!(
                "friction-return-rate must be finite and between 0 and 1"
            ));
        }
        Ok(self)
    }
}

pub fn parse_friction_duration(value: &str) -> Result<u64, String> {
    let (digits, multiplier) = if let Some(s) = value.strip_suffix('s') {
        (s, 1)
    } else if let Some(s) = value.strip_suffix('m') {
        (s, 60)
    } else {
        return Err("friction-loop-duration requires an unsigned integer and s or m suffix".into());
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err("friction-loop-duration requires an unsigned integer and s or m suffix".into());
    }
    digits
        .parse::<u64>()
        .ok()
        .and_then(|n| n.checked_mul(multiplier))
        .ok_or_else(|| "friction-loop-duration is out of bounds".into())
}

/// Every retained metadata row, before the legacy item-level MAX aggregation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemEvidence {
    pub source_key: String,
    pub qitem_id: String,
    pub parent_qitem_id: Option<String>,
    pub is_recovery: bool,
    pub recovery_reason: Option<String>,
    pub issue_tags: Vec<String>,
}

pub struct Input<'a> {
    pub queue: &'a [QueueFact],
    pub metadata: &'a [ItemEvidence],
    pub edges: &'a [HandoffEdge],
    /// Rig-scoped usage, including untimed and out-of-window exclusion evidence.
    pub usage: &'a [UsageFact],
    pub identities: &'a [IdentityFact],
    pub activity: &'a [ActivityFact],
    pub snapshots: &'a BTreeMap<String, Vec<DestinationSnapshot>>,
    pub window: Window,
    pub thresholds: FrictionThresholds,
    pub capture_replay: bool,
}

pub struct Evaluation {
    pub value: Value,
    pub replay: Value,
}

fn after(a: &QueueFact, b: &QueueFact) -> bool {
    a.ts > b.ts
        || (a.ts == b.ts
            && a.source_key.starts_with("events:") == b.source_key.starts_with("events:")
            && a.transition_id > b.transition_id)
}

fn successful(row: &QueueFact) -> bool {
    row.state == "handed-off"
        || (row.state == "done"
            && matches!(
                row.closure_reason.as_deref(),
                None | Some("no-follow-on" | "handed_off_to" | "escalation")
            ))
}

fn median(samples: &[f64]) -> Option<f64> {
    let mut samples = samples.to_vec();
    samples.sort_by(f64::total_cmp);
    let mid = samples.len() / 2;
    if samples.is_empty() {
        None
    } else if samples.len().is_multiple_of(2) {
        Some((samples[mid - 1] + samples[mid]) / 2.0)
    } else {
        Some(samples[mid])
    }
}

fn amount(u: &UsageFact) -> Option<u64> {
    [u.input_uncached, u.output, u.cache_read, u.cache_write]
        .into_iter()
        .try_fold(0u64, |sum, n| sum.checked_add(u64::try_from(n).ok()?))
        // Keep the same signed representable domain as stored token facts.
        .filter(|n| *n <= i64::MAX as u64)
}

/// Legacy totals use signed token arithmetic; refuse malformed persisted inputs before it runs.
pub(crate) fn validate_persisted_usage(usage: &[UsageFact]) -> Result<()> {
    let valid = usage.iter().try_fold(0u64, |sum, row| {
        sum.checked_add(amount(row)?)
            .filter(|n| *n <= i64::MAX as u64)
    });
    if valid.is_none() {
        return Err(anyhow!("invalid or overflowing persisted token totals"));
    }
    Ok(())
}

fn assigned_threshold(
    tokens: Option<u64>,
    assigned: &BTreeMap<String, u64>,
    limit: u64,
    complete: bool,
) -> Option<bool> {
    match tokens {
        Some(n) => token_threshold(n, limit, complete),
        None => {
            let lower_bound: u128 = assigned.values().map(|n| u128::from(*n)).sum();
            (lower_bound > u128::from(limit)).then_some(true)
        }
    }
}

#[derive(Clone, Default)]
struct Coverage {
    data: bool,
    activity: bool,
    ambiguous: bool,
    partial: bool,
}
impl Coverage {
    fn name(&self) -> &'static str {
        if !self.data || !self.activity {
            "no_data"
        } else if self.ambiguous {
            "ambiguous"
        } else if self.partial {
            "partial"
        } else {
            "complete"
        }
    }
    fn merge(&mut self, other: &Self) {
        self.data |= other.data;
        self.activity |= other.activity;
        self.ambiguous |= other.ambiguous;
        self.partial |= other.partial;
    }
}
fn threshold(observed: f64, limit: f64, complete: bool) -> Option<bool> {
    if observed > limit {
        Some(true)
    } else if complete {
        Some(false)
    } else {
        None
    }
}
fn token_threshold(observed: u64, limit: u64, complete: bool) -> Option<bool> {
    if observed > limit {
        Some(true)
    } else if complete {
        Some(false)
    } else {
        None
    }
}
fn any_status(values: impl IntoIterator<Item = Option<bool>>) -> Option<bool> {
    let values: Vec<_> = values.into_iter().collect();
    if values.contains(&Some(true)) {
        Some(true)
    } else if values.contains(&None) {
        None
    } else {
        Some(false)
    }
}

struct Run<'a> {
    claim: &'a QueueFact,
    end: Option<&'a QueueFact>,
    seat: Option<String>,
    success: bool,
    ambiguous: bool,
}

#[derive(Default)]
struct Item<'a> {
    rows: Vec<&'a QueueFact>,
    parents: BTreeSet<&'a str>,
    recovery: bool,
    issues: BTreeSet<&'a str>,
    creations: Vec<&'a QueueFact>,
    runs: Vec<usize>,
    conflict: bool,
}

struct Edge<'a> {
    row: &'a QueueFact,
    to: &'a str,
    recovery: bool,
    root: String,
    lineage: &'static str,
    is_return: bool,
    child: Option<&'a str>,
    run: Option<usize>,
    wait: Option<f64>,
    exec: Option<f64>,
    invalid: BTreeSet<&'static str>,
    coverage: Coverage,
    assigned: BTreeMap<String, u64>,
}

fn ancestry<'a>(id: &'a str, items: &BTreeMap<&'a str, Item<'a>>) -> (Vec<&'a str>, &'static str) {
    let mut path = Vec::new();
    let mut current = id;
    let mut seen = BTreeSet::new();
    loop {
        if !seen.insert(current) {
            return (path, "ambiguous_cycle");
        }
        if path.len() >= 128 {
            return (path, "incomplete_depth_limit");
        }
        path.push(current);
        let Some(item) = items.get(current) else {
            return (path, "incomplete_missing_parent");
        };
        if item.conflict || item.parents.len() > 1 {
            return (path, "ambiguous_parent");
        }
        match item.parents.first() {
            Some(parent) if items.contains_key(parent) => current = parent,
            Some(_) => return (path, "incomplete_missing_parent"),
            None => {
                let hinted = item
                    .rows
                    .iter()
                    .any(|r| r.chain_id.as_deref().is_some_and(|c| c != current));
                return (
                    path,
                    if hinted {
                        "incomplete_missing_parent"
                    } else {
                        "complete"
                    },
                );
            }
        }
    }
}

fn dedup<T: Serialize>(rows: &[T], key: impl Fn(&T) -> &str) -> (Vec<&T>, BTreeSet<String>) {
    let mut unique = BTreeMap::<&str, (&T, Value)>::new();
    let mut conflicts = BTreeSet::new();
    for row in rows {
        let k = key(row);
        let value = serde_json::to_value(row).expect("fact serialization");
        if let Some((_, prior)) = unique.get(k) {
            if *prior != value {
                conflicts.insert(k.to_owned());
            }
        } else {
            unique.insert(k, (row, value));
        }
    }
    (unique.into_values().map(|(r, _)| r).collect(), conflicts)
}

/// Evaluate only accepted facts. No source files, keys or runtime state are read.
pub fn evaluate(input: Input<'_>) -> Evaluation {
    let (queue, queue_conflicts) = dedup(input.queue, |r| &r.source_key);
    let mut items = BTreeMap::<&str, Item<'_>>::new();
    for row in &queue {
        let item = items.entry(&row.qitem_id).or_default();
        item.rows.push(row);
        item.conflict |= queue_conflicts.contains(&row.source_key);
        if row.source_key.starts_with("events:queue.created:") {
            item.creations.push(row);
        }
    }
    for meta in input.metadata {
        if let Some(item) = items.get_mut(meta.qitem_id.as_str()) {
            if let Some(parent) = &meta.parent_qitem_id {
                item.parents.insert(parent);
            }
            item.recovery |= meta.is_recovery || meta.recovery_reason.is_some();
            item.issues
                .extend(meta.issue_tags.iter().map(String::as_str));
        }
    }
    let mut children = BTreeMap::<&str, Vec<&str>>::new();
    for (&id, item) in &items {
        for parent in &item.parents {
            children.entry(parent).or_default().push(id);
        }
    }
    let mut runs = Vec::<Run<'_>>::new();
    for item in items.values_mut() {
        item.rows
            .sort_by_key(|r| (r.ts, r.transition_id, &r.source_key));
        let owned_rows: Vec<_> = item.rows.iter().map(|r| (*r).clone()).collect();
        let mut claim: Option<&QueueFact> = None;
        for &row in &item.rows {
            if row.state == "in-progress" {
                if claim.is_none() {
                    claim = Some(row);
                }
            } else if let Some(start) = claim.take() {
                let tie = !after(row, start);
                let simultaneous_conflict = item.rows.iter().any(|other| {
                    (other.ts == row.ts
                        && other.source_key != row.source_key
                        && other.state != row.state
                        && other.source_key.starts_with("events:")
                            != row.source_key.starts_with("events:"))
                        || (other.ts == start.ts
                            && other.state != "in-progress"
                            && other.source_key.starts_with("events:")
                                != start.source_key.starts_with("events:"))
                });
                let seat = metrics::destination_at(start, &owned_rows, input.snapshots)
                    .ok()
                    .map(|r| r.0);
                item.runs.push(runs.len());
                runs.push(Run {
                    claim: start,
                    end: Some(row),
                    seat,
                    success: successful(row) && !tie && !simultaneous_conflict,
                    ambiguous: tie || simultaneous_conflict || item.conflict,
                });
            }
        }
        if let Some(start) = claim {
            let seat = metrics::destination_at(start, &owned_rows, input.snapshots)
                .ok()
                .map(|r| r.0);
            item.runs.push(runs.len());
            runs.push(Run {
                claim: start,
                end: None,
                seat,
                success: false,
                ambiguous: item.conflict,
            });
        }
    }
    // Per-seat run timelines are swept once below. Unknown ownership is retained separately.
    let mut by_seat = BTreeMap::<&str, Vec<usize>>::new();
    let mut unknown_runs = Vec::new();
    for (index, run) in runs.iter().enumerate() {
        if let Some(seat) = &run.seat {
            by_seat.entry(seat).or_default().push(index);
        } else {
            unknown_runs.push(index);
        }
    }
    unknown_runs.sort_by_key(|i| runs[*i].claim.ts);
    for timeline in by_seat.values_mut() {
        timeline.sort_by_key(|i| runs[*i].claim.ts);
    }
    let active_seats: BTreeSet<_> = input
        .activity
        .iter()
        .filter(|r| r.ts >= input.window.from && r.ts < input.window.to)
        .map(|r| r.seat.as_str())
        .collect();
    let selected: BTreeSet<_> = input.edges.iter().map(|e| e.source_key.as_str()).collect();
    let mut edges = Vec::<Edge<'_>>::new();
    let mut edge_by_item = BTreeMap::<&str, Vec<usize>>::new();
    for row in &queue {
        if row.closure_reason.as_deref() != Some("handed_off_to")
            || queue_conflicts.contains(&row.source_key)
        {
            continue;
        }
        let item = &items[row.qitem_id.as_str()];
        let (path, status) = ancestry(&row.qitem_id, &items);
        let root = if matches!(status, "complete" | "incomplete_missing_parent") {
            path.last().unwrap().to_string()
        } else {
            row.qitem_id.clone()
        };
        let to = row.closure_target.as_deref().unwrap_or(&row.dest_seat);
        let mut invalid = BTreeSet::new();
        let candidates: Vec<_> = children
            .get(row.qitem_id.as_str())
            .into_iter()
            .flatten()
            .filter(|id| items[**id].creations.iter().any(|c| c.dest_seat == to))
            .copied()
            .collect();
        let child = if candidates.len() == 1 && items[candidates[0]].parents.len() == 1 {
            Some(candidates[0])
        } else {
            invalid.insert(if candidates.is_empty() {
                "missing_receiver_child"
            } else {
                "ambiguous_receiver_child"
            });
            None
        };
        let mut wait = None;
        let mut exec = None;
        let mut receiver_run = None;
        if let Some(id) = child {
            let receiver = &items[id];
            if receiver.creations.len() != 1 || receiver.conflict {
                invalid.insert("ambiguous_receiver_creation");
            } else {
                let creation = receiver.creations[0];
                receiver_run = receiver
                    .runs
                    .iter()
                    .copied()
                    .find(|i| runs[*i].claim.ts >= creation.ts);
                if let Some(index) = receiver_run {
                    let run = &runs[index];
                    if run.seat.as_deref() != Some(to) {
                        invalid.insert("unproven_receiver_owner");
                    } else {
                        wait = Some((run.claim.ts - creation.ts).as_seconds_f64() * 1000.0);
                        if run.ambiguous {
                            invalid.insert("unordered_receiver_transitions");
                        } else if run.success {
                            exec = run
                                .end
                                .map(|end| (end.ts - run.claim.ts).as_seconds_f64() * 1000.0);
                        } else {
                            invalid.insert("interrupted_or_open_first_claim");
                        }
                    }
                } else {
                    invalid.insert("missing_receiver_claim");
                }
            }
        }
        let coverage = Coverage {
            activity: active_seats.contains(to),
            partial: !invalid.is_empty() || status != "complete",
            ambiguous: item.conflict,
            ..Coverage::default()
        };
        edge_by_item
            .entry(&row.qitem_id)
            .or_default()
            .push(edges.len());
        edges.push(Edge {
            row,
            to,
            recovery: item.recovery,
            root,
            lineage: status,
            is_return: false,
            child,
            run: receiver_run,
            wait,
            exec,
            invalid,
            coverage,
            assigned: BTreeMap::new(),
        });
    }
    for i in 0..edges.len() {
        if edges[i].recovery {
            continue;
        }
        let (path, _) = ancestry(&edges[i].row.qitem_id, &items);
        if edges[i].lineage.starts_with("ambiguous") {
            continue;
        }
        let mut visits = BTreeSet::new();
        // The source of this productive segment is a proven productive visit too.
        visits.insert(edges[i].row.source_seat.as_str());
        for ancestor in path.iter().skip(1) {
            for &index in edge_by_item.get(ancestor).into_iter().flatten() {
                if !edges[index].recovery && !items[*ancestor].conflict {
                    visits.insert(edges[index].row.source_seat.as_str());
                    visits.insert(edges[index].to);
                }
            }
        }
        edges[i].is_return = visits.contains(edges[i].to);
    }
    let selected_indices: Vec<_> = edges
        .iter()
        .enumerate()
        .filter(|(_, e)| {
            selected.contains(e.row.source_key.as_str())
                && e.row.ts >= input.window.from
                && e.row.ts < input.window.to
        })
        .map(|(i, _)| i)
        .collect();
    let mut run_edges = BTreeMap::<usize, Vec<usize>>::new();
    for &i in &selected_indices {
        if !edges[i].recovery && edges[i].is_return && edges[i].exec.is_some() {
            if let Some(run) = edges[i].run {
                run_edges.entry(run).or_default().push(i);
            }
        }
    }
    let (usage, usage_conflicts) = dedup(input.usage, |u| &u.source_key);
    let bindings = metrics::session_bindings(input.identities);
    let mut usage_by_seat = BTreeMap::<String, Vec<&UsageFact>>::new();
    let mut usage_evidence = BTreeMap::<String, Value>::new();
    let mut seat_coverage = BTreeMap::<String, Coverage>::new();
    for u in &usage {
        let tokens = amount(u);
        let binding = metrics::binding_of(&bindings, &u.harness, &u.native_session);
        let reason = if usage_conflicts.contains(&u.source_key) {
            Some("conflicting_duplicate_usage")
        } else if tokens.is_none() {
            Some("invalid_or_overflowing_token_components")
        } else if u.ts.is_none() {
            Some("missing_usage_timestamp")
        } else if u
            .ts
            .is_some_and(|t| t < input.window.from || t >= input.window.to)
        {
            Some("outside_report_window")
        } else if matches!(binding, SessionBinding::Collision) {
            Some("ambiguous_session_binding")
        } else if binding.seat().is_none() {
            Some("unlinked_session")
        } else {
            None
        };
        if let Some(seat) = binding.seat() {
            if reason != Some("outside_report_window") {
                let coverage = seat_coverage.entry(seat.into()).or_default();
                coverage.data = true;
                coverage.partial |= reason.is_some();
                coverage.ambiguous |= reason == Some("conflicting_duplicate_usage");
            }
        }
        if matches!(binding, SessionBinding::Collision) && reason != Some("outside_report_window") {
            for identity in input.identities.iter().filter(|identity| {
                identity.harness == u.harness && identity.native_session == u.native_session
            }) {
                let scope = seat_coverage.entry(identity.seat.clone()).or_default();
                scope.data = true;
                scope.ambiguous = true;
            }
        }
        if let Some(reason) = reason {
            usage_evidence.insert(u.source_key.clone(), json!({"source_key":u.source_key,"tokens":tokens,"status":"excluded","reason":reason}));
        } else if let Some(seat) = binding.seat() {
            usage_by_seat.entry(seat.into()).or_default().push(u);
        }
    }
    for (&seat, timeline) in &by_seat {
        let Some(usages) = usage_by_seat.get_mut(seat) else {
            continue;
        };
        usages.sort_by_key(|u| u.ts);
        let mut cursor = 0;
        let mut unknown_cursor = 0;
        let mut unknown_active = BTreeSet::new();
        let mut active = BTreeSet::new();
        for u in usages.iter() {
            let ts = u.ts.expect("timestamp checked");
            while cursor < timeline.len() && runs[timeline[cursor]].claim.ts <= ts {
                active.insert(timeline[cursor]);
                cursor += 1;
            }
            active.retain(|i| runs[*i].end.is_none_or(|end| end.ts > ts));
            while unknown_cursor < unknown_runs.len()
                && runs[unknown_runs[unknown_cursor]].claim.ts <= ts
            {
                unknown_active.insert(unknown_runs[unknown_cursor]);
                unknown_cursor += 1;
            }
            unknown_active.retain(|i| runs[*i].end.is_none_or(|end| end.ts > ts));
            let candidates: Vec<_> = active
                .iter()
                .flat_map(|i| run_edges.get(i).into_iter().flatten())
                .copied()
                .collect();
            let uncertain = active
                .iter()
                .any(|i| runs[*i].end.is_none() || runs[*i].ambiguous)
                || !unknown_active.is_empty();
            let proven_active = active
                .iter()
                .filter(|i| runs[**i].end.is_some() && !runs[**i].ambiguous)
                .count();
            let status = if proven_active > 1 || candidates.len() > 1 {
                "concurrent_runs"
            } else if uncertain {
                "incomplete_run_boundaries"
            } else if candidates.is_empty() {
                "outside_return_interval"
            } else {
                "assigned"
            };
            let tokens = amount(u).expect("tokens checked");
            let coverage = seat_coverage.entry(seat.into()).or_default();
            coverage.data = true;
            if !candidates.is_empty() {
                coverage.ambiguous |= status == "concurrent_runs";
                coverage.partial |= status == "incomplete_run_boundaries";
            }
            let edge = if status == "assigned" {
                let index = candidates[0];
                edges[index].assigned.insert(u.source_key.clone(), tokens);
                Some(edges[index].row.source_key.as_str())
            } else {
                None
            };
            usage_evidence.insert(u.source_key.clone(), json!({"source_key":u.source_key,"tokens":tokens,
                "status":if edge.is_some(){"assigned"}else{"excluded"},"reason":status,"edge_source_key":edge,
                "seat":seat,"candidate_edge_keys":candidates.iter().map(|i| &edges[*i].row.source_key).collect::<Vec<_>>(),
                "competing_claim_keys":active.iter().map(|i| &runs[*i].claim.source_key).collect::<Vec<_>>()}));
        }
    }
    // Seats without a retained run still contribute exclusion evidence and coverage.
    for (seat, usages) in &usage_by_seat {
        for u in usages {
            usage_evidence
                .entry(u.source_key.clone())
                .or_insert_with(|| {
                    json!({"source_key":u.source_key,
                "tokens":amount(u),"status":"excluded","reason":"no_seat_runs","seat":seat})
                });
        }
    }
    for &i in &selected_indices {
        let edge = &mut edges[i];
        if let Some(scope) = seat_coverage.get(edge.to) {
            edge.coverage.merge(scope);
        }
    }
    let mut rigs = BTreeMap::<&str, BTreeSet<&str>>::new();
    for identity in input.identities {
        rigs.entry(&identity.seat)
            .or_default()
            .insert(&identity.rig);
    }
    let ring = |from: &str, to: &str| {
        let a = rigs.get(from);
        let b = rigs.get(to);
        let identity_evidence = if a.is_none() || b.is_none() {
            "missing"
        } else if a.is_some_and(|r| r.len() != 1) || b.is_some_and(|r| r.len() != 1) {
            "conflicting"
        } else {
            "proven"
        };
        let a = a.filter(|r| r.len() == 1).and_then(|r| r.first());
        let b = b.filter(|r| r.len() == 1).and_then(|r| r.first());
        let boundary = match (a, b) {
            (Some(a), Some(b)) if **a != *"kernel" && **b != *"kernel" => {
                if a == b {
                    "in_ring"
                } else {
                    "cross_rig_seam"
                }
            }
            _ => "supervisory_plane",
        };
        (boundary, identity_evidence)
    };
    let mut pairs = BTreeMap::<(&str, &str), Vec<usize>>::new();
    let mut chains = BTreeMap::<&str, Vec<usize>>::new();
    for &i in &selected_indices {
        pairs
            .entry((&edges[i].row.source_seat, edges[i].to))
            .or_default()
            .push(i);
        chains.entry(&edges[i].root).or_default().push(i);
    }
    let summarize = |indices: &[usize]| {
        let mut coverage = Coverage::default();
        let mut assigned = BTreeMap::new();
        for &i in indices {
            coverage.merge(&edges[i].coverage);
            assigned.extend(edges[i].assigned.clone());
        }
        let sum = assigned
            .values()
            .try_fold(0u64, |sum, n| sum.checked_add(*n))
            .filter(|n| *n <= i64::MAX as u64);
        if sum.is_none() {
            coverage.partial = true;
        }
        (coverage, sum, assigned)
    };
    let mut pair_status = BTreeMap::new();
    let mut pair_values = Vec::new();
    for ((from, to), indices) in &pairs {
        let count = indices.iter().filter(|i| !edges[**i].recovery).count();
        let returns = indices
            .iter()
            .filter(|i| edges[**i].is_return && !edges[**i].recovery)
            .count();
        let rate = (count > 0).then(|| returns as f64 / count as f64);
        let lineage_complete = indices.iter().all(|i| edges[*i].lineage == "complete");
        let (coverage, tokens, assigned) = summarize(indices);
        let rate_status =
            rate.and_then(|r| threshold(r, input.thresholds.return_rate, lineage_complete));
        pair_status.insert((*from, *to), rate_status);
        let (boundary, identity) = ring(from, to);
        let waits: Vec<_> = indices
            .iter()
            .filter(|i| !edges[**i].recovery)
            .filter_map(|i| edges[*i].wait)
            .collect();
        let execs: Vec<_> = indices
            .iter()
            .filter(|i| !edges[**i].recovery)
            .filter_map(|i| edges[*i].exec)
            .collect();
        pair_values.push(json!({"source_seat":from,"dest_seat":to,"count":count,
            "recovery_count":indices.len()-count,"return_count":returns,"return_rate":rate,
            "wait_median_ms":median(&waits),"exec_median_ms":median(&execs),"wait_samples_ms":waits,"exec_samples_ms":execs,
            "rework_tokens":tokens,"token_error":if tokens.is_none(){Some("aggregate_token_overflow")}else{None},"ring_boundary":boundary,"identity_evidence":identity,"coverage":coverage.name(),
            "return_coverage":if lineage_complete {"complete"}else{"partial"},
            "threshold_status":{"pair_return_rate":rate_status,"rework_tokens":assigned_threshold(tokens,&assigned,input.thresholds.rework_tokens,coverage.name()=="complete")},
            "source_keys":indices.iter().map(|i| &edges[*i].row.source_key).collect::<Vec<_>>(),
            "assigned_usage_keys":assigned.keys().collect::<Vec<_>>(),
            "wait_invalid":indices.iter().filter(|i| !edges[**i].recovery && edges[**i].wait.is_none()).count(),
            "exec_invalid":indices.iter().filter(|i| !edges[**i].recovery && edges[**i].exec.is_none()).count(),
            "samples":indices.iter().map(|i| edge_replay(&edges[*i], &runs)).collect::<Vec<_>>() }));
    }
    let mut chain_values = Vec::new();
    let mut chain_status = BTreeMap::new();
    for (root, indices) in &chains {
        let returns = indices.iter().filter(|i| edges[**i].is_return).count() as u64;
        let complete = indices.iter().all(|i| edges[*i].lineage == "complete");
        let status = token_threshold(returns, input.thresholds.chain_returns, complete);
        chain_status.insert(*root, status);
        let metadata: BTreeSet<_> = indices
            .iter()
            .filter_map(|i| edges[*i].row.chain_id.as_deref())
            .collect();
        let issues: BTreeSet<_> = indices
            .iter()
            .flat_map(|i| {
                let (path, _) = ancestry(&edges[*i].row.qitem_id, &items);
                path.into_iter()
                    .flat_map(|id| {
                        items
                            .get(id)
                            .into_iter()
                            .flat_map(|item| item.issues.iter().copied())
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        chain_values.push(json!({"root":root,"chain_id":if metadata.len()==1{metadata.first().copied()}else{None},
            "recorded_chain_ids":metadata,"issue_tags":issues,"return_count":returns,
            "coverage":if complete{"complete"}else{"partial"},"threshold_status":{"chain_returns":status},
            "alerts":if status==Some(true){vec!["chain_returns"]}else{vec![]},
            "source_keys":indices.iter().map(|i| &edges[*i].row.source_key).collect::<Vec<_>>() }));
    }
    // Each selected edge has at most one causal next edge. Forks break adjacency.
    let selected_set: BTreeSet<_> = selected_indices.iter().copied().collect();
    let mut next = BTreeMap::new();
    for &i in &selected_indices {
        let edge = &edges[i];
        if edge.recovery
            || edge.lineage.starts_with("ambiguous")
            || items[edge.row.qitem_id.as_str()].conflict
        {
            continue;
        }
        let Some(child) = edge.child else { continue };
        if children
            .get(edge.row.qitem_id.as_str())
            .is_none_or(|c| c.len() != 1)
            || items[child].recovery
        {
            continue;
        }
        let candidates = edge_by_item.get(child).map(Vec::as_slice).unwrap_or(&[]);
        if candidates.len() != 1 {
            continue;
        }
        let j = candidates[0];
        if selected_set.contains(&j)
            && !edges[j].recovery
            && edge.to == edges[j].row.source_seat
            && edges[j].row.ts >= edge.row.ts
            && !items[child].conflict
            && !edges[j].lineage.starts_with("ambiguous")
        {
            next.insert(i, j);
        }
    }
    let mut loops = BTreeMap::<Vec<String>, Value>::new();
    for &start in &selected_indices {
        if edges[start].recovery {
            continue;
        }
        let mut path = vec![start];
        while path.len() < 4 {
            let Some(&j) = next.get(path.last().unwrap()) else {
                break;
            };
            if path.contains(&j) {
                break;
            }
            path.push(j);
            if edges[j].to != edges[start].row.source_seat {
                continue;
            }
            let keys: Vec<_> = path
                .iter()
                .map(|i| edges[*i].row.source_key.clone())
                .collect();
            let (coverage, tokens, assigned) = summarize(&path);
            let duration = (edges[j].row.ts - edges[start].row.ts).as_seconds_f64();
            let pair_rate = any_status(
                path.iter()
                    .map(|i| pair_status[&(edges[*i].row.source_seat.as_str(), edges[*i].to)]),
            );
            let chain = chain_status
                .get(edges[start].root.as_str())
                .copied()
                .flatten();
            let token = assigned_threshold(
                tokens,
                &assigned,
                input.thresholds.rework_tokens,
                coverage.name() == "complete",
            );
            let time = threshold(duration, input.thresholds.loop_duration_secs as f64, true);
            let mut seats = vec![edges[start].row.source_seat.as_str()];
            seats.extend(path.iter().map(|i| edges[*i].to));
            let issue_tags: BTreeSet<_> = path
                .iter()
                .flat_map(|i| {
                    items[edges[*i].row.qitem_id.as_str()]
                        .issues
                        .iter()
                        .copied()
                })
                .collect();
            loops.insert(keys.clone(),json!({"loop_kind":match path.len(){2=>"ping_pong",3=>"triangle",_=>"quad_cycle"},
                "seats":seats,"chain_id":edges[start].row.chain_id,"root":edges[start].root,"issue_tags":issue_tags,
                "return_count":path.iter().filter(|i| edges[**i].is_return).count(),"duration_secs":duration,
                "rework_tokens":tokens,"token_error":if tokens.is_none(){Some("aggregate_token_overflow")}else{None},"coverage":coverage.name(),"source_keys":keys,"assigned_usage_keys":assigned.keys().collect::<Vec<_>>(),
                "threshold_status":{"pair_return_rate":pair_rate,"chain_returns":chain,"rework_tokens":token,"loop_duration":time},
                "exceeds_threshold":any_status([pair_rate,chain,token,time]) }));
        }
    }
    let (coverage, _, _) = summarize(&selected_indices);
    let seats: BTreeSet<_> = pairs.keys().flat_map(|(a, b)| [*a, *b]).collect();
    let value = json!({"definition":FORMULA_VERSION,"formula_version":FORMULA_VERSION,
        "scope":"retained accepted history; sparse absent pairs imply zero counts, null ratios/medians and no observed token data; loop witnesses overlap and must not be summed",
        "window":{"from":input.window.from,"to":input.window.to},"thresholds":input.thresholds,
        "seats":seats,"pairs":pair_values,"chains":chain_values,"vortex_loops":loops.into_values().collect::<Vec<_>>(),
        "coverage":coverage.name(),"evidence_census":{"usage_unique":usage.len(),"usage_conflicts":usage_conflicts,
            "queue_conflicts":queue_conflicts,"assigned_usage":usage_evidence.values().filter(|v|v["status"]=="assigned").count(),
            "excluded_usage":usage_evidence.values().filter(|v|v["status"]=="excluded").count()},
        "history_limit":"conflicts discarded before storage or collapsed by redaction cannot be reconstructed" });
    let replay = if input.capture_replay {
        json!({"formula_version":FORMULA_VERSION,"window":{"from":input.window.from,"to":input.window.to},
        "thresholds":input.thresholds,"queue":queue,"metadata":input.metadata,"destination_snapshots":input.snapshots,
        "identities":input.identities,"activity":input.activity,"usage":usage,
        "conflicting_queue_variants":input.queue.iter().filter(|q|queue_conflicts.contains(&q.source_key)).collect::<Vec<_>>(),
        "conflicting_usage_variants":input.usage.iter().filter(|u|usage_conflicts.contains(&u.source_key)).collect::<Vec<_>>(),
        "usage_disposition":usage_evidence.into_values().collect::<Vec<_>>(),
        "edges":selected_indices.iter().map(|i| edge_replay(&edges[*i],&runs)).collect::<Vec<_>>(),
        "runs":runs.iter().map(|r|json!({"claim_source_key":r.claim.source_key,"end_source_key":r.end.map(|e|&e.source_key),
            "seat":r.seat,"successful":r.success,"ambiguous":r.ambiguous})).collect::<Vec<_>>() })
    } else {
        Value::Null
    };
    Evaluation { value, replay }
}

fn edge_replay(edge: &Edge<'_>, runs: &[Run<'_>]) -> Value {
    let run = edge.run.map(|i| &runs[i]);
    json!({"source_key":edge.row.source_key,"qitem_id":edge.row.qitem_id,"from":edge.row.source_seat,"to":edge.to,
        "ts":edge.row.ts,"root":edge.root,"lineage_status":edge.lineage,"productive_return":edge.is_return,
        "recovery":edge.recovery,"receiver_qitem_id":edge.child,"claim_source_key":run.map(|r|&r.claim.source_key),
        "terminal_source_key":run.filter(|r|r.success).and_then(|r|r.end).map(|r|&r.source_key),
        "wait_ms":edge.wait,"exec_ms":edge.exec,"invalid_reasons":edge.invalid,
        "assigned_usage_keys":edge.assigned.keys().collect::<Vec<_>>()})
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeDelta, TimeZone};

    fn at(seconds: i64) -> chrono::DateTime<chrono::Utc> {
        chrono::Utc
            .timestamp_opt(1_700_000_000 + seconds, 0)
            .unwrap()
    }
    fn row(id: usize, ts: i64, state: &str, key: &str, seat: &str) -> QueueFact {
        QueueFact {
            source_key: key.into(),
            transition_id: ts,
            qitem_id: format!("i{id}"),
            ts: at(ts),
            state: state.into(),
            source_seat: seat.into(),
            dest_seat: seat.into(),
            closure_reason: (state == "done").then(|| "no-follow-on".into()),
            closure_target: None,
            chain_id: Some("i0".into()),
            missions: vec![],
        }
    }
    struct Fixture {
        queue: Vec<QueueFact>,
        meta: Vec<ItemEvidence>,
        usage: Vec<UsageFact>,
        ids: Vec<IdentityFact>,
        activity: Vec<ActivityFact>,
    }
    impl Fixture {
        fn path(seats: &[&str]) -> Self {
            let mut f = Self {
                queue: vec![],
                meta: vec![],
                usage: vec![],
                ids: vec![],
                activity: vec![],
            };
            for (id, seat) in seats.iter().enumerate() {
                let start = id as i64 * 10;
                f.queue.push(row(
                    id,
                    start,
                    "pending",
                    &format!("events:queue.created:i{id}"),
                    seat,
                ));
                f.queue.push(row(
                    id,
                    start + 1,
                    "in-progress",
                    &format!("events:queue.claimed:i{id}"),
                    seat,
                ));
                let mut end = row(
                    id,
                    start + 9,
                    "done",
                    &format!("queue_transitions:i{id}"),
                    seat,
                );
                if let Some(to) = seats.get(id + 1) {
                    end.closure_reason = Some("handed_off_to".into());
                    end.closure_target = Some((*to).into());
                }
                f.queue.push(end);
                f.meta.push(ItemEvidence {
                    source_key: format!("queue_transitions:i{id}"),
                    qitem_id: format!("i{id}"),
                    parent_qitem_id: (id > 0).then(|| format!("i{}", id - 1)),
                    is_recovery: false,
                    recovery_reason: None,
                    issue_tags: vec!["issue:shared".into()],
                });
            }
            for seat in seats.iter().copied().collect::<BTreeSet<_>>() {
                f.ids.push(IdentityFact {
                    source_key: format!("identity:{seat}"),
                    harness: "codex".into(),
                    native_session: seat.into(),
                    rig: "r".into(),
                    seat: seat.into(),
                    ts: None,
                });
                f.activity.push(ActivityFact {
                    source_key: format!("activity:{seat}"),
                    seat: seat.into(),
                    ts: at(0),
                    state: "running".into(),
                    seq: 0,
                });
            }
            f
        }
        fn evaluate(&self) -> Evaluation {
            self.window(at(0), at(100))
        }
        fn window(
            &self,
            from: chrono::DateTime<chrono::Utc>,
            to: chrono::DateTime<chrono::Utc>,
        ) -> Evaluation {
            let edges = metrics::handoff_edges(&self.queue);
            evaluate(Input {
                queue: &self.queue,
                metadata: &self.meta,
                edges: &edges,
                usage: &self.usage,
                identities: &self.ids,
                activity: &self.activity,
                snapshots: &BTreeMap::new(),
                window: Window { from, to },
                thresholds: FrictionThresholds::default(),
                capture_replay: true,
            })
        }
        fn usage(&mut self, seat: &str, ts: i64, key: &str, n: i64) {
            let u:UsageFact=serde_json::from_value(json!({"source_key":key,"harness":"codex","native_session":seat,"ts":at(ts),"seq":ts,
                "input_uncached":n,"output":0,"cache_read":0,"cache_write":0,"reasoning":999,"generation":1})).unwrap();
            self.usage.push(u);
        }
    }
    fn pair<'a>(v: &'a Value, from: &str, to: &str) -> &'a Value {
        v["pairs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["source_seat"] == from && p["dest_seat"] == to)
            .unwrap()
    }

    #[test]
    fn repeated_paths_enumerate_physical_windows_and_prewindow_returns() {
        let f = Fixture::path(&["A", "B", "A", "B", "A"]);
        let v = f.evaluate().value;
        assert_eq!(v["vortex_loops"].as_array().unwrap().len(), 4);
        assert_eq!(pair(&v, "B", "A")["return_count"], 2);
        let v = f.window(at(25), at(100)).value;
        assert_eq!(pair(&v, "A", "B")["return_count"], 1);
        assert_eq!(v["vortex_loops"].as_array().unwrap().len(), 1);
    }
    #[test]
    fn hub_nonhub_triangle_and_self_handoff() {
        for (seats, kind) in [
            (vec!["A", "B", "C", "A"], "triangle"),
            (vec!["A", "B", "C", "D", "A"], "quad_cycle"),
        ] {
            let v = Fixture::path(&seats).evaluate().value;
            assert_eq!(v["vortex_loops"][0]["loop_kind"], kind);
        }
        assert!(Fixture::path(&["A", "A"]).evaluate().value["vortex_loops"]
            .as_array()
            .unwrap()
            .is_empty());
    }
    #[test]
    fn shared_chain_and_tags_cannot_repair_missing_conflicting_or_cyclic_parents() {
        let mut f = Fixture::path(&["A", "B", "A"]);
        f.meta[1].parent_qitem_id = None;
        assert!(f.evaluate().value["vortex_loops"]
            .as_array()
            .unwrap()
            .is_empty());
        assert_eq!(pair(&f.evaluate().value, "B", "A")["return_count"], 0);
        f.meta[1].parent_qitem_id = Some("i0".into());
        let mut conflict = f.meta[1].clone();
        conflict.parent_qitem_id = Some("i2".into());
        f.meta.push(conflict);
        assert!(f.evaluate().value["vortex_loops"]
            .as_array()
            .unwrap()
            .is_empty());
        f.meta.pop();
        f.meta[0].parent_qitem_id = Some("i1".into());
        assert!(f.evaluate().replay["edges"][0]["lineage_status"]
            .as_str()
            .unwrap()
            .contains("cycle"));
        assert!(f.evaluate().value["vortex_loops"]
            .as_array()
            .unwrap()
            .is_empty());
        assert_eq!(pair(&f.evaluate().value, "B", "A")["return_count"], 0);
    }
    #[test]
    fn forks_and_recovery_break_adjacency_without_erasing_pair_rows() {
        let mut f = Fixture::path(&["A", "B", "A"]);
        f.meta[1].is_recovery = true;
        let v = f.evaluate().value;
        assert_eq!(pair(&v, "B", "A")["count"], 0);
        assert_eq!(pair(&v, "B", "A")["recovery_count"], 1);
        assert!(pair(&v, "B", "A")["wait_median_ms"].is_null());
        assert!(v["vortex_loops"].as_array().unwrap().is_empty());
        f.meta[1].is_recovery = false;
        f.queue
            .push(row(9, 10, "pending", "events:queue.created:fork", "B"));
        f.meta.push(ItemEvidence {
            source_key: "events:queue.created:fork".into(),
            qitem_id: "i9".into(),
            parent_qitem_id: Some("i0".into()),
            is_recovery: false,
            recovery_reason: None,
            issue_tags: vec![],
        });
        assert!(f.evaluate().value["vortex_loops"]
            .as_array()
            .unwrap()
            .is_empty());
    }
    #[test]
    fn first_interrupted_claim_never_uses_later_reclaim_and_preserves_boundary() {
        let mut f = Fixture::path(&["A", "B"]);
        f.queue
            .push(row(1, 12, "blocked", "queue_transitions:block", "B"));
        f.queue
            .push(row(1, 15, "in-progress", "events:queue.claimed:again", "B"));
        let e = f.evaluate();
        let p = pair(&e.value, "A", "B");
        assert_eq!(p["wait_median_ms"], 1000.0);
        assert!(p["exec_median_ms"].is_null());
        assert_eq!(
            e.replay["runs"][1]["end_source_key"],
            "queue_transitions:block"
        );
    }
    #[test]
    fn same_time_cross_table_interruption_is_unordered() {
        let mut f = Fixture::path(&["A", "B"]);
        f.queue
            .push(row(1, 11, "blocked", "queue_transitions:block", "B"));
        assert!(pair(&f.evaluate().value, "A", "B")["exec_median_ms"].is_null());
    }
    #[test]
    fn true_submillisecond_pickup_and_execution() {
        let mut f = Fixture::path(&["A", "B"]);
        f.queue[4].ts = at(10) + TimeDelta::microseconds(250);
        f.queue[5].ts = at(10) + TimeDelta::microseconds(1500);
        let v = f.evaluate().value;
        assert_eq!(pair(&v, "A", "B")["wait_median_ms"], 0.25);
        assert_eq!(pair(&v, "A", "B")["exec_median_ms"], 1.25);
        assert_eq!(median(&[0.25, 0.75]), Some(0.5));
    }
    #[test]
    fn tokens_unique_half_open_competing_and_open_runs() {
        let mut f = Fixture::path(&["A", "B", "A"]);
        f.usage("A", 22, "seven", 7);
        f.usage("A", 29, "terminal", 11);
        f.usage.push(f.usage[0].clone());
        let e = f.evaluate();
        let p = pair(&e.value, "B", "A");
        assert_eq!(p["rework_tokens"], 7);
        assert_eq!(p["coverage"], "complete");
        assert_eq!(p["assigned_usage_keys"], json!(["seven"]));
        let mut competing = row(9, 21, "in-progress", "events:queue.claimed:background", "A");
        f.queue.push(row(
            9,
            20,
            "pending",
            "events:queue.created:background",
            "A",
        ));
        competing.chain_id = None;
        f.queue.push(competing);
        let e = f.evaluate();
        assert_eq!(pair(&e.value, "B", "A")["coverage"], "partial");
        assert_eq!(pair(&e.value, "B", "A")["rework_tokens"], 0);
        f.queue
            .push(row(9, 28, "done", "queue_transitions:background", "A"));
        assert_eq!(pair(&f.evaluate().value, "B", "A")["coverage"], "ambiguous");
    }
    #[test]
    fn conflicting_usage_overflow_and_empty_data_are_explicit() {
        let mut f = Fixture::path(&["A", "B", "A"]);
        f.usage("A", 22, "u", 7);
        let mut duplicate = f.usage[0].clone();
        duplicate.output = 1;
        f.usage.push(duplicate);
        let e = f.evaluate();
        assert_eq!(pair(&e.value, "B", "A")["coverage"], "ambiguous");
        assert_eq!(pair(&e.value, "B", "A")["rework_tokens"], 0);
        f.usage.truncate(1);
        f.usage[0].input_uncached = i64::MAX;
        f.usage[0].output = 1;
        assert!(amount(&f.usage[0]).is_none());
        assert_eq!(
            f.evaluate().replay["usage_disposition"][0]["reason"],
            "invalid_or_overflowing_token_components"
        );
        f.usage.clear();
        assert_eq!(pair(&f.evaluate().value, "B", "A")["coverage"], "no_data");
    }
    #[test]
    fn unknown_and_conflicting_rigs_never_assert_in_ring() {
        let mut f = Fixture::path(&["A", "B"]);
        f.ids.clear();
        let v = f.evaluate().value;
        assert_eq!(pair(&v, "A", "B")["ring_boundary"], "supervisory_plane");
        assert_eq!(pair(&v, "A", "B")["identity_evidence"], "missing");
    }
    #[test]
    fn strict_thresholds_and_duration_lexemes() {
        assert_eq!(threshold(0.1, 0.1, true), Some(false));
        assert_eq!(threshold(0.3, 0.3, true), Some(false));
        assert_eq!(token_threshold(7, 7, false), None);
        assert_eq!(token_threshold(7, 6, false), Some(true));
        assert_eq!(token_threshold(7, 7, true), Some(false));
        assert_eq!(parse_friction_duration("10m"), Ok(600));
        assert_eq!(parse_friction_duration("600s"), Ok(600));
        for value in ["600", "-1s", "1.5m", "NaNs", "18446744073709551615m", "+1s"] {
            assert!(parse_friction_duration(value).is_err());
        }
    }
    #[test]
    fn unrelated_malformed_usage_preserves_complete_pair_and_overflow_is_named() {
        let mut f = Fixture::path(&["A", "B", "A"]);
        f.usage("A", 22, "u", 7);
        f.usage("unlinked", 22, "bad", -1);
        assert_eq!(pair(&f.evaluate().value, "B", "A")["coverage"], "complete");
        f.usage[0].input_uncached = i64::MAX;
        f.usage("A", 23, "u2", 1);
        let e = f.evaluate();
        let p = pair(&e.value, "B", "A");
        assert!(p["rework_tokens"].is_null());
        assert_eq!(p["token_error"], "aggregate_token_overflow");
        assert_eq!(p["coverage"], "partial");
        assert_eq!(p["threshold_status"]["rework_tokens"], true);
    }

    #[test]
    fn public_report_trace_share_value_and_custom_friction_preserves_old_fields() {
        let path = std::env::temp_dir().join(format!(
            "dendro-friction-{}-{}.duckdb",
            std::process::id(),
            at(0).timestamp()
        ));
        let store = crate::AnalyticsStore::open_path(&path, "fixture").unwrap();
        for row in Fixture::path(&["A", "B"]).queue {
            store.insert_queue(&row).unwrap();
        }
        store.checkpoint().unwrap();
        drop(store);
        let from = at(0).to_rfc3339();
        let to = at(100).to_rfc3339();
        let report = crate::report_json(&path, "handoffs", None, &from, &to).unwrap();
        let decisions = crate::report_json(&path, "decisions", None, &from, &to).unwrap();
        let trace = crate::trace_json(&path, "handoff_friction", None, &from, &to).unwrap();
        assert_eq!(report["payload"]["handoff_friction"], trace["value"]);
        assert_eq!(decisions["payload"]["handoff_friction"], trace["value"]);
        let mut customized = crate::report_json_with_options(
            &path,
            "handoffs",
            None,
            &from,
            &to,
            crate::LoopThresholds::default(),
            FrictionThresholds {
                return_rate: 0.3,
                ..FrictionThresholds::default()
            },
        )
        .unwrap();
        let mut original = report;
        original["payload"]
            .as_object_mut()
            .unwrap()
            .remove("handoff_friction");
        customized["payload"]
            .as_object_mut()
            .unwrap()
            .remove("handoff_friction");
        assert_eq!(original, customized);
        assert!(customized["payload"].get("loop_thresholds").is_none());
        assert!(customized["payload"].get("handoff_loops").is_none());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn conflicting_edge_key_is_not_selected_arbitrarily() {
        let mut f = Fixture::path(&["A", "B"]);
        let mut conflict = f.queue[2].clone();
        conflict.closure_target = Some("C".into());
        f.queue.push(conflict);
        let result = f.evaluate();
        assert!(result.value["pairs"].as_array().unwrap().is_empty());
        assert_eq!(
            result.value["evidence_census"]["queue_conflicts"],
            json!(["queue_transitions:i0"])
        );
    }

    #[test]
    fn persisted_overflow_refuses_report_and_trace_without_panicking() {
        let path = std::env::temp_dir().join(format!(
            "dendro-friction-overflow-{}.duckdb",
            std::process::id()
        ));
        let store = crate::AnalyticsStore::open_path(&path, "fixture").unwrap();
        let mut f = Fixture::path(&["A", "B", "A"]);
        f.usage("A", 22, "a", i64::MAX);
        f.usage("A", 23, "b", 1);
        store
            .commit_file_batch(
                "fixture",
                "codex",
                "synthetic.jsonl",
                1,
                0,
                "synthetic",
                None,
                &f.usage,
                &[],
            )
            .unwrap();
        store.checkpoint().unwrap();
        drop(store);
        let from = at(0).to_rfc3339();
        let to = at(100).to_rfc3339();
        let report = crate::report_json(&path, "handoffs", None, &from, &to).unwrap_err();
        assert_eq!(
            report.to_string(),
            "invalid or overflowing persisted token totals"
        );
        let trace = crate::trace_json(&path, "handoff_friction", None, &from, &to).unwrap_err();
        assert_eq!(
            trace.to_string(),
            "invalid or overflowing persisted token totals"
        );
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn metadata_loader_preserves_conflicts_without_changing_legacy_aggregation() {
        let store = crate::AnalyticsStore::open_in_memory("fixture").unwrap();
        store
            .insert_queue(&row(1, 1, "pending", "one", "A"))
            .unwrap();
        store
            .insert_queue(&row(1, 2, "pending", "two", "A"))
            .unwrap();
        store
            .connection()
            .execute(
                "UPDATE queue_facts SET parent_qitem_id = ? WHERE source_key = ?",
                duckdb::params!["p1", "one"],
            )
            .unwrap();
        store
            .connection()
            .execute(
                "UPDATE queue_facts SET parent_qitem_id = ? WHERE source_key = ?",
                duckdb::params!["p2", "two"],
            )
            .unwrap();
        let raw = store.load_queue_metadata_evidence().unwrap();
        assert_eq!(raw.len(), 2);
        assert_eq!(raw[0].parent_qitem_id.as_deref(), Some("p1"));
        assert_eq!(raw[1].parent_qitem_id.as_deref(), Some("p2"));
        assert_eq!(
            store.load_queue_item_meta().unwrap()["i1"]
                .parent_qitem_id
                .as_deref(),
            Some("p2")
        );
    }
}
