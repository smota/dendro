//! Additive Wave 1 analytics. Existing K2/K3/K5 and R1 metrics remain authoritative.
use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use serde_json::{json, Value};

use crate::metrics::{
    self, ClosedItem, HandoffEdge, MissionBucket, SeatRole, SessionBinding, Window,
};
use crate::{DestinationSnapshot, IdentityFact, QueueFact, QueueItemMeta, UsageFact};

type Snapshots = BTreeMap<String, Vec<DestinationSnapshot>>;
type Models = BTreeMap<String, (Option<String>, Option<String>)>;
type Bindings = BTreeMap<(String, String), SessionBinding>;

pub struct Input<'a> {
    pub queue: &'a [QueueFact],
    /// Unscoped accepted history for ancestors outside the selected worker rig.
    pub history_queue: &'a [QueueFact],
    pub meta: &'a BTreeMap<String, QueueItemMeta>,
    pub edges: &'a [HandoffEdge],
    pub closures: &'a [(Option<String>, ClosedItem)],
    pub identities: &'a [IdentityFact],
    pub usage: &'a [UsageFact],
    pub models: &'a Models,
    pub bindings: &'a Bindings,
    pub snapshots: &'a Snapshots,
    pub roles: &'a BTreeMap<String, SeatRole>,
    pub window: Window,
}

#[derive(Default, Serialize)]
struct RecoveryCounts {
    productive_closed_items: usize,
    recovery_closed_items: usize,
    productive_handoffs: usize,
    recovery_handoffs: usize,
    productive_returns: usize,
    recovery_by_reason: BTreeMap<String, ReasonCounts>,
}

#[derive(Default, Serialize)]
struct ReasonCounts {
    closed_items: usize,
    handoffs: usize,
}

impl RecoveryCounts {
    fn value(&self) -> Value {
        let mut value = json!(self);
        value["recovery_tax_ratio"] = json!(ratio(
            self.recovery_closed_items,
            self.productive_closed_items + self.recovery_closed_items
        ));
        value["productive_return_rate"] =
            json!(ratio(self.productive_returns, self.productive_handoffs));
        value
    }
}

fn ratio(numerator: usize, denominator: usize) -> Option<f64> {
    (denominator != 0).then(|| numerator as f64 / denominator as f64)
}

fn recovery_reason<'a>(meta: &'a BTreeMap<String, QueueItemMeta>, item: &str) -> Option<&'a str> {
    meta.get(item)
        .filter(|m| m.is_recovery || m.recovery_reason.is_some())
        .map(|m| m.recovery_reason.as_deref().unwrap_or("unspecified"))
}

fn recovery(input: &Input<'_>) -> Value {
    let mut total = RecoveryCounts::default();
    let mut seats = BTreeMap::<String, RecoveryCounts>::new();
    for (seat, item) in input.closures {
        let Some(seat) = seat else { continue };
        let counts = seats.entry(seat.clone()).or_default();
        for c in [&mut total, counts] {
            if let Some(reason) = recovery_reason(input.meta, &item.qitem) {
                c.recovery_closed_items += 1;
                c.recovery_by_reason
                    .entry(reason.into())
                    .or_default()
                    .closed_items += 1;
            } else {
                c.productive_closed_items += 1;
            }
        }
    }
    for edge in input.edges {
        let counts = seats.entry(edge.from_seat.clone()).or_default();
        for c in [&mut total, counts] {
            if let Some(reason) = recovery_reason(input.meta, &edge.qitem_id) {
                c.recovery_handoffs += 1;
                c.recovery_by_reason
                    .entry(reason.into())
                    .or_default()
                    .handoffs += 1;
            } else {
                c.productive_handoffs += 1;
                c.productive_returns += usize::from(edge.is_return);
            }
        }
    }
    let mut value = total.value();
    value["definition"] = json!("Recovery tax = recovery closed items / all K2 closed items with a proven destination; productive return rate excludes recovery edges from numerator and denominator.");
    value["by_seat"] = json!(seats
        .into_iter()
        .map(|(seat, c)| {
            let mut v = c.value();
            v["seat"] = json!(seat);
            v
        })
        .collect::<Vec<_>>());
    value
}

/// Use recorded identities only. Missing or conflicting rig evidence is external.
fn seat_rigs(identities: &[IdentityFact]) -> BTreeMap<&str, BTreeSet<&str>> {
    let mut rigs = BTreeMap::<&str, BTreeSet<&str>>::new();
    for row in identities {
        rigs.entry(&row.seat).or_default().insert(&row.rig);
    }
    rigs
}

fn failed_nudge(status: &str) -> bool {
    let status = status.to_ascii_lowercase();
    [
        "fail",
        "error",
        "unroutable",
        "undelivered",
        "unreachable",
        "not_found",
        "not-found",
        "no-session",
        "no_session",
        "timeout",
        "timed-out",
        "missing",
        "rejected",
    ]
    .iter()
    .any(|part| status.contains(part))
}

fn chain_key(input: &Input<'_>, item: &str) -> String {
    if let Some(chain) = input
        .queue
        .iter()
        .filter(|q| q.qitem_id == item)
        .find_map(|q| q.chain_id.clone())
    {
        return chain;
    }
    let mut id = item;
    let mut seen = BTreeSet::new();
    while seen.insert(id) && seen.len() <= 128 {
        match input
            .meta
            .get(id)
            .and_then(|m| m.parent_qitem_id.as_deref())
        {
            Some(parent) => id = parent,
            None => return id.into(),
        }
    }
    item.into()
}

/// Explicit successor evidence also works when creation/claim precedes the
/// sender's terminal write. Several matching successors are ambiguous.
fn successors<'a>(input: &'a Input<'_>, edge: &HandoffEdge) -> BTreeSet<&'a str> {
    input
        .meta
        .iter()
        .filter(|(_, m)| m.parent_qitem_id.as_deref() == Some(&edge.qitem_id))
        .filter(|(id, _)| {
            input.queue.iter().any(|q| {
                &q.qitem_id == *id
                    && q.source_key.starts_with("events:queue.created:")
                    && q.dest_seat == edge.to_seat
            })
        })
        .map(|(id, _)| id.as_str())
        .collect()
}

fn boundaries(input: &Input<'_>) -> Value {
    let rigs = seat_rigs(input.identities);
    let rig = |seat: &str| {
        rigs.get(seat)
            .filter(|r| r.len() == 1)
            .and_then(|r| r.first().copied())
    };
    let mut counts = BTreeMap::from([
        ("in_ring", 0usize),
        ("cross_rig_seam", 0),
        ("supervisory_plane", 0),
    ]);
    let edges: Vec<_> = input
        .edges
        .iter()
        .map(|e| {
            let boundary = match (rig(&e.from_seat), rig(&e.to_seat)) {
                (Some(a), Some(b)) if a != "kernel" && b != "kernel" => {
                    if a == b {
                        "in_ring"
                    } else {
                        "cross_rig_seam"
                    }
                }
                _ => "supervisory_plane",
            };
            *counts.get_mut(boundary).unwrap() += 1;
            let children = successors(input, e);
            let receiver_item = if children.len() == 1 {
                children.first().copied()
            } else {
                None
            };
            let nudge = receiver_item
                .and_then(|id| input.meta.get(id))
                .and_then(|m| m.last_nudge_result.as_deref());
            let source_nudge = input
                .meta
                .get(&e.qitem_id)
                .and_then(|m| m.last_nudge_result.as_deref());
            json!({"source_key": e.source_key, "qitem_id": e.qitem_id, "from": e.from_seat,
            "to": e.to_seat, "boundary": boundary, "receiver_qitem_id": receiver_item,
            "source_item_last_nudge_result": source_nudge, "last_nudge_result": nudge,
            "failed_nudge": nudge.is_some_and(failed_nudge)})
        })
        .collect();
    let items: BTreeSet<_> = input
        .queue
        .iter()
        .filter(|q| q.ts >= input.window.from && q.ts < input.window.to)
        .map(|q| &q.qitem_id)
        .collect();
    let failures: Vec<_> = items
        .into_iter()
        .filter_map(|id| {
            let status = input.meta.get(id)?.last_nudge_result.as_deref()?;
            failed_nudge(status).then(|| json!({"qitem_id": id, "last_nudge_result": status}))
        })
        .collect();
    let mut value = json!({"edges": edges, "failed_nudges": failures.len(), "failed_nudge_items": failures,
        "nudge_scope": "latest observed item status, once per item with activity in the report window; not a delivery-attempt history"});
    for (name, count) in counts {
        value[name] = json!(count);
        value[format!("{name}_share")] = json!(ratio(count, input.edges.len()));
    }
    value
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Execution {
    pub exec_samples: Vec<f64>,
    pub exec_sample_keys: Vec<(String, String)>,
    pub exec_invalid: usize,
    pub exec_median: Option<f64>,
}

impl Execution {
    fn add(&mut self, claim: &QueueFact, end: &QueueFact) {
        let seconds = (end.ts - claim.ts).as_seconds_f64();
        if seconds < 0.0 {
            self.exec_invalid += 1;
            return;
        }
        self.exec_samples.push(seconds);
        self.exec_sample_keys
            .push((claim.source_key.clone(), end.source_key.clone()));
    }
    fn finish(&mut self) {
        let mut samples = self.exec_samples.clone();
        samples.sort_by(f64::total_cmp);
        self.exec_median = if samples.is_empty() {
            None
        } else {
            let mid = samples.len() / 2;
            Some(if samples.len().is_multiple_of(2) {
                (samples[mid - 1] + samples[mid]) / 2.0
            } else {
                samples[mid]
            })
        };
    }
}

fn terminal(row: &QueueFact) -> bool {
    row.state == "handed-off"
        || (row.state == "done"
            && matches!(
                row.closure_reason.as_deref(),
                None | Some("no-follow-on" | "handed_off_to" | "escalation")
            ))
}

struct Run<'a> {
    claim: &'a QueueFact,
    end: Option<&'a QueueFact>,
}

/// Repeated in-progress observations do not restart a claim. Any intervening
/// state ends it, so a later closure cannot bridge an unclaim or cancellation.
fn runs(queue: &[QueueFact]) -> Vec<Run<'_>> {
    let mut items = BTreeMap::<&str, Vec<&QueueFact>>::new();
    for row in queue {
        items.entry(&row.qitem_id).or_default().push(row);
    }
    let mut result = Vec::new();
    for rows in items.values_mut() {
        rows.sort_by_key(|r| (r.ts, r.transition_id));
        let mut claim = None;
        for &row in rows.iter() {
            if row.state == "in-progress" {
                if claim.is_none() {
                    claim = Some(row);
                }
            } else if let Some(start) = claim.take() {
                result.push(Run {
                    claim: start,
                    end: terminal(row).then_some(row),
                });
            }
        }
        if let Some(claim) = claim {
            result.push(Run { claim, end: None });
        }
    }
    result
}

fn execution(input: &Input<'_>) -> (Value, BTreeMap<(String, String), Execution>) {
    let runs = runs(input.queue);
    let owner = |row: &QueueFact| {
        metrics::destination_at(row, input.queue, input.snapshots)
            .map(|v| v.0)
            .unwrap_or_else(|_| {
                if row.source_key.starts_with("events:queue.claimed:") {
                    row.dest_seat.clone()
                } else {
                    "unknown".into()
                }
            })
    };
    let mut seats = BTreeMap::<String, Execution>::new();
    for run in &runs {
        let in_window = |r: &QueueFact| r.ts >= input.window.from && r.ts < input.window.to;
        let Some(end) = run.end else {
            if in_window(run.claim) {
                seats.entry(owner(run.claim)).or_default().exec_invalid += 1;
            }
            continue;
        };
        if in_window(end) {
            seats
                .entry(owner(run.claim))
                .or_default()
                .add(run.claim, end);
        }
    }
    // Successful closures without a preceding claim are explicit invalid samples.
    for row in input.queue.iter().filter(|r| {
        !r.source_key.starts_with("events:")
            && terminal(r)
            && r.ts >= input.window.from
            && r.ts < input.window.to
    }) {
        if !runs
            .iter()
            .any(|r| r.end.is_some_and(|end| end.source_key == row.source_key))
        {
            seats.entry(owner(row)).or_default().exec_invalid += 1;
        }
    }
    let mut pairs = BTreeMap::<(String, String), Execution>::new();
    for edge in input.edges {
        let entry = pairs
            .entry((edge.from_seat.clone(), edge.to_seat.clone()))
            .or_default();
        let children = successors(input, edge);
        let receiver = if children.len() > 1 {
            None
        } else {
            runs.iter()
                .filter(|r| {
                    owner(r.claim) == edge.to_seat
                        && if let Some(child) = children.first() {
                            r.claim.qitem_id == *child
                        } else {
                            r.claim.ts >= edge.ts
                                && chain_key(input, &r.claim.qitem_id)
                                    == chain_key(input, &edge.qitem_id)
                        }
                })
                .min_by_key(|r| (r.claim.ts, r.claim.transition_id))
        };
        if let Some(Run {
            claim,
            end: Some(end),
        }) = receiver
        {
            entry.add(claim, end);
        } else {
            entry.exec_invalid += 1;
        }
    }
    let mut waits = BTreeMap::<String, (Vec<f64>, Vec<(String, String)>, i64)>::new();
    for pair in metrics::pair_medians(input.edges, input.queue, &[]) {
        seats.entry(pair.to_seat.clone()).or_default();
        let wait = waits.entry(pair.to_seat).or_default();
        wait.0.extend(pair.wait_samples);
        wait.1.extend(pair.wait_sample_keys);
        wait.2 += pair.wait_invalid;
    }
    for e in seats.values_mut().chain(pairs.values_mut()) {
        e.finish();
    }
    let rows: Vec<_> = seats
        .into_iter()
        .map(|(seat, e)| {
            let (samples, keys, invalid) = waits.remove(&seat).unwrap_or_default();
            let mut wait = Execution {
                exec_samples: samples.clone(),
                ..Execution::default()
            };
            wait.finish();
            let mut v = json!(e);
            v["seat"] = json!(seat);
            v["wait_samples"] = json!(samples);
            v["wait_sample_keys"] = json!(keys);
            v["wait_invalid"] = json!(invalid);
            v["wait_median"] = json!(wait.exec_median);
            v
        })
        .collect();
    (
        json!({"by_seat": rows, "definition": "seconds from an in-progress claim to the next successful terminal transition on the same item; repeated in-progress observations are one claim; interrupted or missing claims are invalid. Seat samples close in the report window. Pair samples follow the first receiver claim on the handoff chain, using full accepted history."}),
        pairs,
    )
}

#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct Lineage {
    pub(crate) missions: BTreeSet<String>,
    pub(crate) issues: BTreeSet<String>,
    phases: BTreeSet<String>,
    pub(crate) ancestors: BTreeSet<String>,
    unresolved: bool,
    mission_unresolved: bool,
}

pub(crate) fn lineage(
    history_queue: &[QueueFact],
    meta: &BTreeMap<String, QueueItemMeta>,
    snapshots: &Snapshots,
) -> BTreeMap<String, Lineage> {
    let mut own = BTreeMap::<String, Lineage>::new();
    let mut parents = BTreeMap::<String, BTreeSet<String>>::new();
    for q in history_queue {
        let own = own.entry(q.qitem_id.clone()).or_default();
        // The snapshots are the creation-tag readings used by K3.
        if let Some(s) = snapshots.get(&q.qitem_id).and_then(|s| s.last()) {
            own.missions.extend(s.missions.iter().cloned());
        } else {
            own.missions.extend(q.missions.iter().cloned());
        }
        if let Some(root) = &q.chain_id {
            if root != &q.qitem_id {
                parents
                    .entry(q.qitem_id.clone())
                    .or_default()
                    .insert(root.clone());
            }
        }
    }
    for (id, meta) in meta {
        let own = own.entry(id.clone()).or_default();
        own.issues.extend(meta.issue_tags.iter().cloned());
        own.phases.extend(meta.phase_tags.iter().cloned());
        if let Some(parent) = &meta.parent_qitem_id {
            // The immediate predecessor is stronger evidence than the chain root.
            parents.insert(id.clone(), BTreeSet::from([parent.clone()]));
        }
    }
    fn resolve(
        id: &str,
        own: &BTreeMap<String, Lineage>,
        parents: &BTreeMap<String, BTreeSet<String>>,
        visiting: &mut BTreeSet<String>,
        memo: &mut BTreeMap<String, Lineage>,
    ) -> Lineage {
        if let Some(value) = memo.get(id) {
            return value.clone();
        }
        let Some(mut value) = own.get(id).cloned() else {
            return Lineage {
                unresolved: true,
                mission_unresolved: true,
                ..Lineage::default()
            };
        };
        if !visiting.insert(id.into()) {
            return Lineage {
                unresolved: true,
                mission_unresolved: true,
                ..Lineage::default()
            };
        }
        // Bound recursion for malformed or adversarial histories.
        if visiting.len() > 128 {
            visiting.remove(id);
            return Lineage {
                unresolved: true,
                mission_unresolved: true,
                ..Lineage::default()
            };
        }
        let mut inherited = Lineage::default();
        for parent in parents.get(id).into_iter().flatten() {
            let p = resolve(parent, own, parents, visiting, memo);
            inherited.missions.extend(p.missions);
            inherited.issues.extend(p.issues);
            inherited.phases.extend(p.phases);
            inherited.ancestors.extend(p.ancestors);
            inherited.ancestors.insert(parent.clone());
            inherited.unresolved |= p.unresolved;
            inherited.mission_unresolved |= p.mission_unresolved;
        }
        if value.missions.is_empty() {
            value.missions = inherited.missions;
            value.mission_unresolved = inherited.mission_unresolved;
        }
        if value.issues.is_empty() {
            value.issues = inherited.issues;
        }
        if value.phases.is_empty() {
            value.phases = inherited.phases;
        }
        value.ancestors = inherited.ancestors;
        value.unresolved = inherited.unresolved;
        visiting.remove(id);
        memo.insert(id.into(), value.clone());
        value
    }
    let mut memo = BTreeMap::new();
    for id in own.keys() {
        resolve(id, &own, &parents, &mut BTreeSet::new(), &mut memo);
    }
    memo
}

fn token_amount(u: &UsageFact) -> i64 {
    u.input_uncached + u.output + u.cache_read + u.cache_write
}

fn model_family(model: &str) -> Option<&'static str> {
    let m = model.to_ascii_lowercase();
    if m.starts_with("gpt-")
        || m.starts_with("chatgpt-")
        || ["o1", "o3", "o4"]
            .iter()
            .any(|p| m == *p || m.starts_with(&format!("{p}-")))
    {
        Some("gpt")
    } else if m.starts_with("claude-") {
        Some("claude")
    } else if m.starts_with("grok-") {
        Some("grok")
    } else if m.starts_with("gemini-") {
        Some("gemini")
    } else {
        None
    }
}

fn build_seat(seat: &str) -> bool {
    seat.split('@')
        .next()
        .unwrap_or(seat)
        .replace('.', "-")
        .starts_with("dev-build")
}

/// All new projections are computed from the caller's coherent read snapshot.
pub fn evaluate(input: Input<'_>) -> Value {
    let lineage = lineage(input.history_queue, input.meta, input.snapshots);
    let mut sku = BTreeMap::<(Option<String>, Option<String>, Option<String>), i64>::new();
    let mut chain_models = BTreeMap::<(String, String), (BTreeSet<String>, bool)>::new();
    let mut mission_tokens = BTreeMap::<String, i64>::new();
    let mut issue_tokens = BTreeMap::<String, i64>::new();
    let mut phase_tokens = BTreeMap::<String, i64>::new();
    let (mut assigned, mut ambiguous, mut overhead, mut unlinked, mut total) =
        (0i64, 0i64, 0i64, 0i64, 0i64);
    let mut records = Vec::new();
    for u in input.usage {
        let amount = token_amount(u);
        total += amount;
        let binding = metrics::binding_of(input.bindings, &u.harness, &u.native_session);
        let (model, effort) = input.models.get(&u.source_key).cloned().unwrap_or_default();
        *sku.entry((binding.seat().map(str::to_owned), model.clone(), effort))
            .or_default() += amount;
        let Some(seat) = binding.seat() else {
            if matches!(binding, SessionBinding::Collision) {
                ambiguous += amount;
            } else {
                unlinked += amount;
            }
            continue;
        };
        let Some(ts) = u.ts else {
            ambiguous += amount;
            continue;
        };
        let evidence = metrics::mission_evidence(seat, input.queue, input.snapshots, ts);
        let mut missions = BTreeSet::new();
        let mut issues = BTreeSet::new();
        let mut phases = BTreeSet::new();
        let mut unresolved = false;
        let active_chains: BTreeSet<_> = evidence
            .items
            .iter()
            .filter(|i| i.relation == "owned")
            .map(|i| chain_key(&input, &i.qitem))
            .collect();
        for item in &evidence.items {
            if let Some(l) = lineage.get(&item.qitem) {
                missions.extend(l.missions.iter().cloned());
                issues.extend(l.issues.iter().cloned());
                phases.extend(l.phases.iter().cloned());
                unresolved |= l.mission_unresolved;
            }
            if item.relation == "owned" {
                let chain = chain_key(&input, &item.qitem);
                let (families, unknown) = chain_models.entry((chain, seat.into())).or_default();
                if active_chains.len() != 1 {
                    *unknown = true;
                } else if let Some(family) = model.as_deref().and_then(model_family) {
                    families.insert(family.into());
                } else {
                    *unknown = true;
                }
            }
        }
        let bucket = if matches!(evidence.bucket, MissionBucket::Ambiguous(_)) || unresolved {
            ambiguous += amount;
            "ambiguous"
        } else if missions.len() == 1 {
            assigned += amount;
            *mission_tokens
                .entry(missions.first().unwrap().clone())
                .or_default() += amount;
            "assigned_mission"
        } else if evidence.items.is_empty() {
            overhead += amount;
            "task_intake_overhead"
        } else {
            // Active but untagged work is not evidence of task-intake overhead.
            ambiguous += amount;
            "ambiguous"
        };
        if issues.len() == 1 {
            *issue_tokens
                .entry(issues.first().unwrap().clone())
                .or_default() += amount;
        }
        if phases.len() == 1 {
            *phase_tokens
                .entry(phases.first().unwrap().clone())
                .or_default() += amount;
        }
        records.push(
            json!({"source_key": u.source_key, "seat": seat, "tokens": amount,
            "bucket": bucket, "missions": missions, "issues": issues, "phases": phases,
            "items": evidence.items, "unresolved_lineage": unresolved}),
        );
    }
    let mut checks = Vec::new();
    let mut seen = BTreeSet::new();
    for (i, edge) in input
        .edges
        .iter()
        .enumerate()
        .filter(|(_, e)| build_seat(&e.from_seat))
    {
        for later in &input.edges[i..] {
            let chain = chain_key(&input, &edge.qitem_id);
            if chain_key(&input, &later.qitem_id) != chain
                || !matches!(input.roles.get(&later.to_seat), Some(SeatRole::Check(_)))
            {
                continue;
            }
            if !seen.insert((chain.clone(), &edge.from_seat, &later.to_seat)) {
                continue;
            }
            let builder = chain_models.get(&(chain.clone(), edge.from_seat.clone()));
            let reviewer = chain_models.get(&(chain.clone(), later.to_seat.clone()));
            let empty = BTreeSet::new();
            let a = builder.map(|v| &v.0).unwrap_or(&empty);
            let b = reviewer.map(|v| &v.0).unwrap_or(&empty);
            let shared: Vec<_> = a.intersection(b).collect();
            let status = if !shared.is_empty() {
                "same_family"
            } else if a.is_empty()
                || b.is_empty()
                || builder.is_some_and(|v| v.1)
                || reviewer.is_some_and(|v| v.1)
            {
                "insufficient_evidence"
            } else {
                "independent"
            };
            checks.push(json!({"chain": chain, "builder": edge.from_seat, "reviewer": later.to_seat,
                "builder_families": a, "reviewer_families": b, "shared_families": shared, "status": status,
                "source_key": edge.source_key, "review_handoff": later.source_key}));
        }
    }
    let (execution, pairs) = execution(&input);
    let mut visible_items: BTreeSet<_> = input.queue.iter().map(|q| q.qitem_id.clone()).collect();
    for id in visible_items.clone() {
        if let Some(l) = lineage.get(&id) {
            visible_items.extend(l.ancestors.iter().cloned());
        }
    }
    let same_family_chains: BTreeSet<_> = checks
        .iter()
        .filter(|c| c["status"] == "same_family")
        .filter_map(|c| c["chain"].as_str())
        .collect();
    json!({
        "watchdog_recovery": recovery(&input),
        "ring_boundaries": boundaries(&input),
        "exec_turnaround": execution,
        "exec_pairs": pairs.into_iter().map(|((from, to), e)| { let mut v = json!(e); v["from"] = json!(from); v["to"] = json!(to); v }).collect::<Vec<_>>(),
        "model_effort_tokens": sku.into_iter().map(|((seat, model, effort), tokens)| json!({"seat": seat, "model": model, "effort": effort, "tokens": tokens})).collect::<Vec<_>>(),
        "four_eyes_independence": {"same_family_chains": same_family_chains.len(), "checks": checks,
            "definition": "Recorded check roles and contract dev-build seats on a handoff chain; model families come only from in-progress work on that chain in the report window. Unknown models or missing usage cannot prove independence."},
        "issue_lineage": {"items": lineage.into_iter().filter(|(id, _)| visible_items.contains(id)).map(|(qitem_id, l)| {let mut v = json!(l); v["qitem_id"] = json!(qitem_id); v}).collect::<Vec<_>>(),
            "lineage_missions": mission_tokens, "issue_tokens": issue_tokens, "phase_tokens": phase_tokens, "records": records},
        "task_intake_overhead": {"assigned_mission_tokens": assigned, "ambiguous_tokens": ambiguous,
            "task_intake_overhead": overhead, "unlinked_session": unlinked, "total_tokens": total,
            "conserved": assigned + ambiguous + overhead + unlinked == total,
            "definition": "Inherited lineage is additive to K3. Linked usage outside every active item is intake overhead. Active untagged work, unresolved lineage and identity collisions are ambiguous; sessions without a binding are unlinked. Reasoning is already included in output."}
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Duration, Utc};

    fn at(seconds: i64) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-15T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
            + Duration::seconds(seconds)
    }

    fn queue(key: &str, item: &str, second: i64, state: &str, seat: &str) -> QueueFact {
        QueueFact {
            source_key: key.into(),
            transition_id: second,
            qitem_id: item.into(),
            ts: at(second),
            state: state.into(),
            source_seat: seat.into(),
            dest_seat: seat.into(),
            closure_reason: None,
            closure_target: None,
            chain_id: Some("root".into()),
            missions: vec![],
        }
    }

    fn usage(key: &str, seat: &str, second: i64, tokens: i64) -> UsageFact {
        serde_json::from_value(
            json!({"source_key": key, "harness": "codex", "native_session": seat,
            "ts": at(second), "seq": second, "input_uncached": tokens, "output": 0,
            "cache_read": 0, "cache_write": 0, "reasoning": 0, "generation": 1}),
        )
        .unwrap()
    }

    #[derive(Default)]
    struct Fixture {
        queue: Vec<QueueFact>,
        meta: BTreeMap<String, QueueItemMeta>,
        usage: Vec<UsageFact>,
        models: Models,
        identities: Vec<IdentityFact>,
        snapshots: Snapshots,
        roles: BTreeMap<String, SeatRole>,
    }
    impl Fixture {
        fn link(&mut self, seat: &str, rig: &str) {
            self.identities.push(IdentityFact {
                source_key: format!("{seat}:{rig}"),
                harness: "codex".into(),
                native_session: seat.into(),
                rig: rig.into(),
                seat: seat.into(),
                ts: Some(at(0)),
            });
        }
        fn item(&mut self, item: &str, seat: &str, start: i64, end: i64, missions: &[&str]) {
            self.queue.extend([
                queue(
                    &format!("events:queue.created:{item}"),
                    item,
                    start - 1,
                    "pending",
                    seat,
                ),
                queue(&format!("claim:{item}"), item, start, "in-progress", seat),
                queue(&format!("done:{item}"), item, end, "done", seat),
            ]);
            self.snapshots.insert(
                item.into(),
                vec![DestinationSnapshot {
                    destination: seat.into(),
                    cutoff: at(99).to_rfc3339(),
                    reroute_marked: false,
                    missions: missions.iter().map(|s| (*s).into()).collect(),
                }],
            );
        }
        fn handoff(&mut self, item: &str, to: &str) {
            let q = self
                .queue
                .iter_mut()
                .find(|q| q.qitem_id == item && q.state == "done")
                .unwrap();
            q.closure_reason = Some("handed_off_to".into());
            q.closure_target = Some(to.into());
        }
        fn evaluate(&self, from: i64, to: i64) -> Value {
            let window = Window {
                from: at(from),
                to: at(to),
            };
            let bindings = metrics::session_bindings(&self.identities);
            let all_edges = metrics::handoff_edges(&self.queue);
            let edges: Vec<_> = all_edges
                .into_iter()
                .filter(|e| e.ts >= window.from && e.ts < window.to)
                .collect();
            let closures = metrics::k2_closures(&self.queue, &self.snapshots, window);
            let usages: Vec<_> = self
                .usage
                .iter()
                .filter(|u| u.ts.is_some_and(|t| t >= window.from && t < window.to))
                .cloned()
                .collect();
            evaluate(Input {
                queue: &self.queue,
                history_queue: &self.queue,
                meta: &self.meta,
                edges: &edges,
                closures: &closures,
                identities: &self.identities,
                usage: &usages,
                models: &self.models,
                bindings: &bindings,
                snapshots: &self.snapshots,
                roles: &self.roles,
                window,
            })
        }
    }

    #[test]
    fn recovery_is_additive_per_seat_and_uses_metadata_not_sealed_id() {
        let mut f = Fixture::default();
        f.item("productive", "a", 1, 5, &[]);
        f.item("opaque-recovery", "a", 6, 10, &[]);
        f.item("reason-only", "b", 11, 15, &[]);
        f.handoff("productive", "b");
        f.handoff("opaque-recovery", "b");
        f.handoff("reason-only", "a");
        f.meta.insert(
            "opaque-recovery".into(),
            QueueItemMeta {
                qitem_id: "opaque-recovery".into(),
                is_recovery: true,
                recovery_reason: Some("undelivered-wake".into()),
                ..QueueItemMeta::default()
            },
        );
        f.meta.insert(
            "reason-only".into(),
            QueueItemMeta {
                qitem_id: "reason-only".into(),
                recovery_reason: Some("dangling-closure".into()),
                ..QueueItemMeta::default()
            },
        );
        let v = f.evaluate(0, 100);
        let r = &v["watchdog_recovery"];
        assert_eq!(r["productive_closed_items"], 1);
        assert_eq!(r["recovery_closed_items"], 2);
        assert_eq!(r["recovery_tax_ratio"], json!(2.0 / 3.0));
        assert_eq!(r["productive_handoffs"], 1);
        assert_eq!(r["recovery_handoffs"], 2);
        assert_eq!(r["productive_return_rate"], 0.0);
        assert_eq!(
            r["recovery_by_reason"]["undelivered-wake"]["closed_items"],
            1
        );
        assert_eq!(r["by_seat"][0]["productive_closed_items"], 1);
        assert_eq!(r["by_seat"][0]["recovery_closed_items"], 1);
        assert_eq!(r["by_seat"][1]["recovery_closed_items"], 1);
    }

    #[test]
    fn boundary_partition_conserves_edges_and_counts_failed_items_once() {
        let mut f = Fixture::default();
        for (seat, rig) in [
            ("a", "dendro-dev"),
            ("b", "dendro-dev"),
            ("pm", "dendro-pm"),
            ("steer", "kernel"),
        ] {
            f.link(seat, rig);
        }
        for (i, to) in ["b", "pm", "steer", "external"].iter().enumerate() {
            let item = format!("q{i}");
            f.item(&item, "a", i as i64 * 10 + 1, i as i64 * 10 + 5, &[]);
            f.handoff(&item, to);
        }
        f.meta.insert(
            "q1".into(),
            QueueItemMeta {
                last_nudge_result: Some("unroutable".into()),
                ..QueueItemMeta::default()
            },
        );
        let v = f.evaluate(0, 100);
        let b = &v["ring_boundaries"];
        assert_eq!(b["in_ring"], 1);
        assert_eq!(b["cross_rig_seam"], 1);
        assert_eq!(b["supervisory_plane"], 2);
        assert_eq!(b["cross_rig_seam_share"], 0.25);
        assert_eq!(b["failed_nudges"], 1);
        // A rig conflict cannot be resolved from the session name.
        f.link("b", "another");
        assert_eq!(
            f.evaluate(0, 100)["ring_boundaries"]["supervisory_plane"],
            3
        );
    }

    #[test]
    fn execution_uses_receiver_same_item_and_preserves_fractional_samples() {
        let mut f = Fixture::default();
        f.item("send", "a", 1, 5, &[]);
        f.handoff("send", "b");
        f.item("receive", "b", 7, 11, &[]);
        // Real transition rows have actor/target, not the historical destination.
        f.queue
            .iter_mut()
            .find(|q| q.source_key == "claim:receive")
            .unwrap()
            .dest_seat = "unknown".into();
        f.queue
            .iter_mut()
            .find(|q| q.source_key == "done:receive")
            .unwrap()
            .ts = at(11) + Duration::milliseconds(250);
        let v = f.evaluate(0, 100);
        let p = &v["exec_pairs"][0];
        assert_eq!(p["exec_samples"], json!([4.25]));
        assert_eq!(
            p["exec_sample_keys"],
            json!([["claim:receive", "done:receive"]])
        );
        assert_eq!(p["exec_median"], 4.25);
        assert_eq!(p["exec_invalid"], 0);
        let seat = v["exec_turnaround"]["by_seat"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["seat"] == "b")
            .unwrap();
        assert_eq!(seat["exec_median"], 4.25);
        // Claim outside the window still supports a terminal inside it.
        assert_eq!(
            f.evaluate(10, 12)["exec_turnaround"]["by_seat"][0]["exec_median"],
            4.25
        );
        assert!(f.evaluate(0, 11)["exec_turnaround"]["by_seat"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["seat"] != "b" || r["exec_samples"] == json!([])));
    }

    #[test]
    fn execution_reclaims_interruptions_missing_claims_and_even_medians() {
        let mut f = Fixture::default();
        f.item("one", "a", 1, 5, &[]);
        f.queue.push(queue("repeat", "one", 2, "in-progress", "a"));
        f.item("two", "a", 10, 18, &[]);
        f.item("interrupted", "b", 20, 30, &[]);
        f.queue
            .push(queue("unclaim", "interrupted", 21, "pending", "b"));
        f.queue
            .push(queue("reclaim", "interrupted", 25, "in-progress", "b"));
        f.queue.push(queue("missing", "orphan", 40, "done", "c"));
        f.queue.push(queue("open", "open", 50, "in-progress", "d"));
        let v = f.evaluate(0, 100);
        let rows = v["exec_turnaround"]["by_seat"].as_array().unwrap();
        let a = rows.iter().find(|r| r["seat"] == "a").unwrap();
        assert_eq!(a["exec_samples"], json!([4.0, 8.0]));
        assert_eq!(a["exec_median"], 6.0);
        let b = rows.iter().find(|r| r["seat"] == "b").unwrap();
        assert_eq!(b["exec_samples"], json!([5.0]));
        assert_eq!(b["exec_invalid"], 1);
        assert_eq!(
            rows.iter()
                .map(|r| r["exec_invalid"].as_u64().unwrap())
                .sum::<u64>(),
            3
        );
    }

    #[test]
    fn lineage_inherits_missing_tags_and_token_partition_conserves_all_usage() {
        let mut f = Fixture::default();
        f.link("a", "worker");
        f.link("collision", "first");
        f.link("collision", "second");
        f.item("root", "a", 1, 5, &["mission"]);
        f.item("child", "a", 6, 10, &[]);
        f.item("untagged", "a", 20, 25, &[]);
        f.queue
            .iter_mut()
            .filter(|q| q.qitem_id == "untagged")
            .for_each(|q| q.chain_id = None);
        f.meta.insert(
            "root".into(),
            QueueItemMeta {
                issue_tags: vec!["55".into()],
                phase_tags: vec!["build".into()],
                ..QueueItemMeta::default()
            },
        );
        f.meta.insert(
            "child".into(),
            QueueItemMeta {
                parent_qitem_id: Some("root".into()),
                ..QueueItemMeta::default()
            },
        );
        f.usage = vec![
            usage("assigned", "a", 7, 11),
            usage("intake", "a", 15, 13),
            usage("active-untagged", "a", 22, 17),
            usage("unlinked", "external", 7, 19),
            usage("collision", "collision", 7, 23),
        ];
        let v = f.evaluate(0, 100);
        let p = &v["task_intake_overhead"];
        assert_eq!(p["assigned_mission_tokens"], 11);
        assert_eq!(p["task_intake_overhead"], 13);
        assert_eq!(p["ambiguous_tokens"], 40);
        assert_eq!(p["unlinked_session"], 19);
        assert_eq!(p["total_tokens"], 83);
        assert_eq!(p["conserved"], true);
        assert_eq!(v["issue_lineage"]["lineage_missions"]["mission"], 11);
        assert_eq!(v["issue_lineage"]["issue_tokens"]["55"], 11);
        assert_eq!(v["issue_lineage"]["phase_tokens"]["build"], 11);
        assert_eq!(
            v["model_effort_tokens"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| r["tokens"].as_i64().unwrap())
                .sum::<i64>(),
            83
        );
        let bindings = metrics::session_bindings(&f.identities);
        assert_eq!(
            metrics::binding_of(&bindings, "codex", "a").seat(),
            Some("a")
        );
        // K3 remains creation-only, independently of additive lineage.
        assert_eq!(
            metrics::mission_at("a", &f.queue, &f.snapshots, at(7)),
            MissionBucket::None
        );
    }

    #[test]
    fn lineage_explicit_overrides_cycles_and_unknown_parent_are_not_guessed() {
        let mut f = Fixture::default();
        f.link("a", "worker");
        f.item("root", "a", 1, 3, &["parent-mission"]);
        f.item("child", "a", 4, 6, &["own-mission"]);
        f.item("cycle", "a", 7, 9, &[]);
        f.item("missing", "a", 10, 12, &[]);
        f.meta.insert(
            "cycle".into(),
            QueueItemMeta {
                parent_qitem_id: Some("cycle".into()),
                ..QueueItemMeta::default()
            },
        );
        f.meta.insert(
            "missing".into(),
            QueueItemMeta {
                parent_qitem_id: Some("absent".into()),
                ..QueueItemMeta::default()
            },
        );
        f.usage = vec![
            usage("own", "a", 5, 10),
            usage("cycle", "a", 8, 20),
            usage("missing", "a", 11, 30),
        ];
        let v = f.evaluate(0, 100);
        assert_eq!(v["issue_lineage"]["lineage_missions"]["own-mission"], 10);
        assert!(v["issue_lineage"]["lineage_missions"]
            .get("parent-mission")
            .is_none());
        assert_eq!(v["task_intake_overhead"]["ambiguous_tokens"], 50);
        assert_eq!(v["task_intake_overhead"]["conserved"], true);
    }

    #[test]
    fn model_effort_and_four_eyes_use_chain_local_usage_and_recorded_review_roles() {
        let mut f = Fixture::default();
        for seat in ["dev-build", "rev-review"] {
            f.link(seat, "worker");
        }
        f.roles.insert(
            "rev-review".into(),
            SeatRole::Check(vec!["recorded".into()]),
        );
        f.item("root", "dev-build", 1, 5, &["mission"]);
        f.handoff("root", "rev-review");
        f.item("review", "rev-review", 6, 10, &[]);
        f.usage = vec![
            usage("build", "dev-build", 2, 10),
            usage("review", "rev-review", 7, 20),
            usage("overhead", "rev-review", 15, 30),
        ];
        f.models.insert(
            "build".into(),
            (Some("gpt-6.1".into()), Some("high".into())),
        );
        f.models
            .insert("review".into(), (Some("gpt-6".into()), Some("low".into())));
        f.models
            .insert("overhead".into(), (Some("claude-opus-4".into()), None));
        let v = f.evaluate(0, 100);
        assert_eq!(v["four_eyes_independence"]["same_family_chains"], 1);
        assert_eq!(
            v["four_eyes_independence"]["checks"][0]["shared_families"],
            json!(["gpt"])
        );
        assert_eq!(v["model_effort_tokens"].as_array().unwrap().len(), 3);
        f.models.insert(
            "review".into(),
            (Some("claude-opus-4".into()), Some("high".into())),
        );
        assert_eq!(
            f.evaluate(0, 100)["four_eyes_independence"]["checks"][0]["status"],
            "independent"
        );
        f.models.remove("review");
        assert_eq!(
            f.evaluate(0, 100)["four_eyes_independence"]["checks"][0]["status"],
            "insufficient_evidence"
        );
        f.roles.clear();
        assert!(f.evaluate(0, 100)["four_eyes_independence"]["checks"]
            .as_array()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn successor_before_sender_terminal_binds_execution_and_wake_to_receiver() {
        let mut f = Fixture::default();
        f.item("root", "a", 1, 10, &[]);
        f.handoff("root", "b");
        f.item("receiver", "b", 8, 15, &[]);
        f.meta.insert(
            "root".into(),
            QueueItemMeta {
                last_nudge_result: Some("delivered".into()),
                ..QueueItemMeta::default()
            },
        );
        f.meta.insert(
            "receiver".into(),
            QueueItemMeta {
                parent_qitem_id: Some("root".into()),
                last_nudge_result: Some("unroutable".into()),
                ..QueueItemMeta::default()
            },
        );
        let v = f.evaluate(0, 100);
        assert_eq!(v["exec_pairs"][0]["exec_samples"], json!([7.0]));
        assert_eq!(
            v["exec_pairs"][0]["exec_sample_keys"],
            json!([["claim:receiver", "done:receiver"]])
        );
        assert_eq!(
            v["ring_boundaries"]["edges"][0]["receiver_qitem_id"],
            "receiver"
        );
        assert_eq!(
            v["ring_boundaries"]["edges"][0]["last_nudge_result"],
            "unroutable"
        );
        assert_eq!(v["ring_boundaries"]["edges"][0]["failed_nudge"], true);
        // Ambiguous successor evidence must not select one arbitrarily.
        f.item("other-receiver", "b", 9, 20, &[]);
        f.meta.insert(
            "other-receiver".into(),
            QueueItemMeta {
                parent_qitem_id: Some("root".into()),
                ..QueueItemMeta::default()
            },
        );
        let v = f.evaluate(0, 100);
        assert_eq!(v["exec_pairs"][0]["exec_invalid"], 1);
        assert_eq!(v["exec_pairs"][0]["exec_samples"], json!([]));
    }

    #[test]
    fn creation_only_parent_and_parent_only_chain_keep_lineage_without_changing_k3() {
        let mut f = Fixture::default();
        f.link("dev-build", "worker");
        f.link("review", "worker");
        f.roles
            .insert("review".into(), SeatRole::Check(vec!["recorded".into()]));
        f.item("root", "dev-build", 1, 5, &["inherited"]);
        f.handoff("root", "review");
        f.item("child", "review", 6, 10, &[]);
        f.meta.insert(
            "child".into(),
            QueueItemMeta {
                parent_qitem_id: Some("root".into()),
                ..QueueItemMeta::default()
            },
        );
        f.queue
            .iter_mut()
            .filter(|q| q.qitem_id == "root" && q.state == "pending")
            .for_each(|q| q.missions = vec!["inherited".into()]);
        f.queue.iter_mut().for_each(|q| q.chain_id = None);
        // Simulate a retained creation event whose queue_items snapshot is gone.
        f.snapshots.remove("root");
        f.usage = vec![usage("review", "review", 7, 10)];
        let v = f.evaluate(0, 100);
        assert_eq!(v["issue_lineage"]["lineage_missions"]["inherited"], 10);
        assert_eq!(
            metrics::mission_at("review", &f.queue, &f.snapshots, at(7)),
            MissionBucket::None
        );
        assert_eq!(v["four_eyes_independence"]["checks"][0]["chain"], "root");
    }

    #[test]
    fn reports_and_traces_roundtrip_wave1_while_preserving_r1_fields() {
        let mut f = Fixture::default();
        f.link("dev-build", "dendro-dev");
        f.link("review", "dendro-dev");
        f.item("root", "dev-build", 1, 5, &["mission"]);
        f.handoff("root", "review");
        f.item("child", "review", 7, 11, &[]);
        f.meta.insert(
            "child".into(),
            QueueItemMeta {
                qitem_id: "child".into(),
                parent_qitem_id: Some("root".into()),
                is_recovery: true,
                recovery_reason: Some("undelivered-wake".into()),
                issue_tags: vec!["55".into()],
                phase_tags: vec!["review".into()],
                ..QueueItemMeta::default()
            },
        );
        f.usage = vec![
            usage("build", "dev-build", 2, 10),
            usage("review", "review", 8, 20),
            usage("intake", "review", 15, 30),
            usage("external", "outside", 8, 40),
        ];
        f.models
            .insert("build".into(), (Some("gpt-6".into()), Some("high".into())));
        f.models.insert(
            "review".into(),
            (Some("gpt-6.1".into()), Some("low".into())),
        );
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!(".test-issue55-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("analytics.duckdb");
        let store = crate::AnalyticsStore::open_path(&db, "test").unwrap();
        for q in &f.queue {
            store.insert_queue(q).unwrap();
        }
        for i in &f.identities {
            store.insert_identity(i).unwrap();
        }
        for (id, ss) in &f.snapshots {
            for s in ss {
                store
                    .insert_destination_snapshot(
                        id,
                        &s.destination,
                        &s.cutoff,
                        s.reroute_marked,
                        &s.missions,
                    )
                    .unwrap();
            }
        }
        for m in f.meta.values() {
            store.record_queue_item_meta(m).unwrap();
        }
        store
            .insert_seat_role("review", "check", "recorded")
            .unwrap();
        store
            .commit_file_batch(
                "source",
                "codex",
                "fixture.jsonl",
                1,
                0,
                "hash",
                None,
                &f.usage,
                &[],
            )
            .unwrap();
        store
            .record_usage_models(
                &f.models
                    .iter()
                    .map(|(k, (m, e))| (k.clone(), m.clone(), e.clone()))
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        store.checkpoint().unwrap();
        drop(store);
        let from = at(0).to_rfc3339();
        let to = at(100).to_rfc3339();
        for kind in ["handoffs", "tokens", "decisions"] {
            let report = crate::report_json(&db, kind, None, &from, &to).unwrap();
            let p = &report["payload"];
            assert_eq!(p["tokens"], 100);
            assert_eq!(p["mission_tokens"]["mission:mission"], 10);
            assert_eq!(p["mission_tokens"]["no_mission"], 50);
            assert_eq!(p["unlinked_tokens"], 40);
            assert_eq!(p["m1_by_seat"][0]["closed_items"], 1);
            assert_eq!(p["m1_by_seat"][1]["closed_items"], 1);
            assert_eq!(p["pair_medians"][0]["wait_median"], 2.0);
            assert_eq!(p["pair_medians"][0]["exec_median"], 4.0);
            assert_eq!(p["m2_return_rate"], 0.0);
            assert_eq!(p["watchdog_recovery"]["recovery_closed_items"], 1);
            assert_eq!(p["four_eyes_independence"]["same_family_chains"], 1);
            assert_eq!(p["task_intake_overhead"]["assigned_mission_tokens"], 30);
            assert_eq!(p["task_intake_overhead"]["task_intake_overhead"], 30);
            assert_eq!(p["task_intake_overhead"]["unlinked_session"], 40);
            assert_eq!(p["task_intake_overhead"]["conserved"], true);
            for metric in [
                "tokens",
                "medians",
                "watchdog_recovery",
                "ring_boundaries",
                "exec_turnaround",
                "model_effort_tokens",
                "four_eyes_independence",
                "issue_lineage",
                "task_intake_overhead",
            ] {
                let trace = crate::trace_json(&db, metric, None, &from, &to).unwrap();
                assert_eq!(
                    trace["inputs"]["task_intake_overhead"],
                    p["task_intake_overhead"]
                );
                assert_eq!(
                    trace["inputs"]["model_effort_tokens"],
                    p["model_effort_tokens"]
                );
                if let Some(value) = p.get(metric) {
                    assert_eq!(&trace["value"], value);
                }
            }
        }
        let scoped = crate::report_json(&db, "tokens", Some("dendro-dev"), &from, &to).unwrap();
        assert_eq!(scoped["payload"]["tokens"], 60);
        assert_eq!(
            scoped["payload"]["task_intake_overhead"]["unlinked_session"],
            0
        );
        assert_eq!(scoped["payload"]["task_intake_overhead"]["conserved"], true);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn scoped_lineage_reads_cross_rig_ancestors_and_explicit_missions_need_no_parent() {
        let mut f = Fixture::default();
        f.link("worker", "dendro-dev");
        f.link("pm", "dendro-pm");
        f.item("root", "pm", 1, 5, &["parent-mission"]);
        f.item("child", "worker", 6, 10, &[]);
        f.item("explicit", "worker", 12, 16, &["own-mission"]);
        f.meta.insert(
            "child".into(),
            QueueItemMeta {
                parent_qitem_id: Some("root".into()),
                ..QueueItemMeta::default()
            },
        );
        f.meta.insert(
            "explicit".into(),
            QueueItemMeta {
                parent_qitem_id: Some("gone-parent".into()),
                ..QueueItemMeta::default()
            },
        );
        f.usage = vec![
            usage("child", "worker", 7, 10),
            usage("explicit", "worker", 13, 20),
        ];
        let scoped: Vec<_> = f
            .queue
            .iter()
            .filter(|q| q.dest_seat == "worker")
            .cloned()
            .collect();
        let bindings = metrics::session_bindings(&f.identities);
        let window = Window {
            from: at(0),
            to: at(100),
        };
        let v = evaluate(Input {
            queue: &scoped,
            history_queue: &f.queue,
            meta: &f.meta,
            edges: &[],
            closures: &[],
            identities: &f.identities,
            usage: &f.usage,
            models: &f.models,
            bindings: &bindings,
            snapshots: &f.snapshots,
            roles: &f.roles,
            window,
        });
        assert_eq!(v["issue_lineage"]["lineage_missions"]["parent-mission"], 10);
        assert_eq!(v["issue_lineage"]["lineage_missions"]["own-mission"], 20);
        assert_eq!(v["task_intake_overhead"]["assigned_mission_tokens"], 30);
        assert_eq!(v["task_intake_overhead"]["ambiguous_tokens"], 0);
    }

    #[test]
    fn empty_report_has_no_invented_ratios_and_is_conserved() {
        let v = Fixture::default().evaluate(0, 100);
        assert_eq!(v["watchdog_recovery"]["recovery_tax_ratio"], Value::Null);
        assert_eq!(
            v["watchdog_recovery"]["productive_return_rate"],
            Value::Null
        );
        assert_eq!(v["ring_boundaries"]["in_ring_share"], Value::Null);
        assert_eq!(v["task_intake_overhead"]["total_tokens"], 0);
        assert_eq!(v["task_intake_overhead"]["conserved"], true);
    }
}
