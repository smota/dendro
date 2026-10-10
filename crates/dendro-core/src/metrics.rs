use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{json, Value};

use crate::facts::*;

#[derive(Debug, Clone, Copy)]
pub struct Window {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Bands {
    pub active_secs: i64,
    pub idle_with_work_secs: i64,
    pub idle_without_work_secs: i64,
    pub waiting_human_secs: i64,
    pub unknown_secs: i64,
}

impl Bands {
    pub fn sum(&self) -> i64 {
        self.active_secs
            + self.idle_with_work_secs
            + self.idle_without_work_secs
            + self.waiting_human_secs
            + self.unknown_secs
    }
}

/// One observed part of the window: `[start, end)`, and the launch that began it.
/// A launch before the window still bounds which earlier states may carry in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObservedSpan {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub launched: Option<DateTime<Utc>>,
}

/// Clips the window to the seat's recorded lives (K4). No lives means no
/// lifecycle evidence, so the whole window is observed.
pub fn observation_spans(window: Window, lives: &[SeatBound]) -> Vec<ObservedSpan> {
    if lives.is_empty() {
        return vec![ObservedSpan {
            start: window.from,
            end: window.to,
            launched: None,
        }];
    }
    let mut spans: Vec<ObservedSpan> = lives
        .iter()
        .filter_map(|life| {
            let start = life
                .created_at
                .filter(|c| *c > window.from)
                .unwrap_or(window.from);
            let end = life
                .stopped_at
                .filter(|s| *s < window.to)
                .unwrap_or(window.to);
            (end > start).then_some(ObservedSpan {
                start,
                end,
                launched: life.created_at,
            })
        })
        .collect();
    spans.sort_by_key(|span| span.start);
    // Lives of one seat from two sources may overlap; count each second once.
    let mut merged: Vec<ObservedSpan> = Vec::new();
    for span in spans {
        match merged.last_mut() {
            Some(last) if span.start <= last.end => {
                if span.end > last.end {
                    last.end = span.end;
                }
                last.launched = match (last.launched, span.launched) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    _ => None,
                };
            }
            _ => merged.push(span),
        }
    }
    merged
}

pub fn observed_secs(spans: &[ObservedSpan]) -> i64 {
    spans
        .iter()
        .map(|span| (span.end - span.start).num_seconds())
        .sum()
}

pub fn activity_bands(
    seat: &str,
    events: &[ActivityFact],
    queue: &[QueueFact],
    window: Window,
    lives: &[SeatBound],
) -> Bands {
    let spans = observation_spans(window, lives);
    let mut bands = Bands {
        active_secs: 0,
        idle_with_work_secs: 0,
        idle_without_work_secs: 0,
        waiting_human_secs: 0,
        unknown_secs: 0,
    };
    let mut mine: Vec<&ActivityFact> = events.iter().filter(|e| e.seat == seat).collect();
    mine.sort_by(|a, b| a.ts.cmp(&b.ts).then(a.seq.cmp(&b.seq)));
    let any_observed_event = mine.iter().any(|e| {
        spans
            .iter()
            .any(|span| e.ts >= span.start && e.ts < span.end)
    });
    if !any_observed_event {
        add_band(&mut bands, "unknown", false, observed_secs(&spans));
        return bands;
    }
    for span in &spans {
        // A state never outlives a stop: only this life's events can carry in.
        let life: Vec<&ActivityFact> = mine
            .iter()
            .copied()
            .filter(|e| span.launched.is_none_or(|launched| e.ts >= launched))
            .collect();
        let mut marks: Vec<DateTime<Utc>> = vec![span.start, span.end];
        for event in &life {
            if event.ts > span.start && event.ts < span.end {
                marks.push(event.ts);
            }
        }
        for q in queue {
            if q.ts > span.start
                && q.ts < span.end
                && (q.dest_seat == seat || q.source_seat == seat)
            {
                marks.push(q.ts);
            }
        }
        marks.sort();
        marks.dedup();
        for pair in marks.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let secs = (b - a).num_seconds();
            if secs <= 0 {
                continue;
            }
            let state = life
                .iter()
                .rev()
                .find(|e| e.ts <= a)
                .map(|e| e.state.as_str())
                .unwrap_or("unknown");
            let demand = idle_demand(seat, queue, a);
            add_band(&mut bands, state, demand, secs);
        }
    }
    bands
}

fn idle_demand(seat: &str, queue: &[QueueFact], at: DateTime<Utc>) -> bool {
    let mut latest: std::collections::BTreeMap<&str, &QueueFact> =
        std::collections::BTreeMap::new();
    for fact in queue.iter().filter(|q| q.ts <= at) {
        latest.insert(fact.qitem_id.as_str(), fact);
    }
    latest
        .values()
        .any(|q| q.dest_seat == seat && matches!(q.state.as_str(), "pending" | "in-progress"))
}

fn add_band(bands: &mut Bands, state: &str, demand: bool, secs: i64) {
    match state {
        "running" => bands.active_secs += secs,
        "needs_input" => bands.waiting_human_secs += secs,
        "idle" if demand => bands.idle_with_work_secs += secs,
        "idle" => bands.idle_without_work_secs += secs,
        _ => bands.unknown_secs += secs,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MissionBucket {
    Mission(String),
    None,
    Ambiguous,
}

pub fn mission_at(seat: &str, queue: &[QueueFact], ts: DateTime<Utc>) -> MissionBucket {
    let mut latest: std::collections::BTreeMap<&str, &QueueFact> =
        std::collections::BTreeMap::new();
    for fact in queue.iter().filter(|q| q.ts <= ts && q.dest_seat == seat) {
        latest.insert(fact.qitem_id.as_str(), fact);
    }
    let open: Vec<&QueueFact> = latest
        .values()
        .copied()
        .filter(|q| q.state == "in-progress")
        .collect();
    if open.is_empty() {
        return MissionBucket::None;
    }
    let mut missions = Vec::new();
    for item in open {
        if item.missions.len() > 1 {
            return MissionBucket::Ambiguous;
        }
        if let Some(m) = item.missions.first() {
            missions.push(m.clone());
        }
    }
    if missions.is_empty() {
        return MissionBucket::None;
    }
    missions.sort();
    missions.dedup();
    if missions.len() == 1 {
        MissionBucket::Mission(missions.remove(0))
    } else {
        MissionBucket::Ambiguous
    }
}

pub fn token_totals(usages: &[UsageFact]) -> (i64, i64, i64, i64) {
    usages.iter().fold((0, 0, 0, 0), |acc, u| {
        (
            acc.0 + u.input_uncached,
            acc.1 + u.output,
            acc.2 + u.cache_read,
            acc.3 + u.cache_write,
        )
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandoffEdge {
    pub from_seat: String,
    pub to_seat: String,
    pub qitem_id: String,
    pub chain_id: Option<String>,
    pub is_return: bool,
    /// The original delivery was later answered by a return to this seat.
    /// This is not `is_return` and must not be counted as an M2 return.
    pub delivery_returned: bool,
    pub source_key: String,
}

pub fn handoff_edges(queue: &[QueueFact]) -> Vec<HandoffEdge> {
    let mut seen: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    let mut edges = Vec::new();
    let mut ordered = queue.to_vec();
    ordered.sort_by(|a, b| a.ts.cmp(&b.ts).then(a.transition_id.cmp(&b.transition_id)));
    for fact in ordered {
        let touch = seen.entry(fact.qitem_id.clone()).or_default();
        if !touch.contains(&fact.source_seat) {
            touch.push(fact.source_seat.clone());
        }
        if fact.closure_reason.as_deref() != Some("handed_off_to") {
            if !touch.contains(&fact.dest_seat) {
                touch.push(fact.dest_seat.clone());
            }
            continue;
        }
        let to = fact
            .closure_target
            .clone()
            .unwrap_or_else(|| fact.dest_seat.clone());
        let is_return = touch.iter().any(|seat| seat == &to);
        if !touch.contains(&to) {
            touch.push(to.clone());
        }
        edges.push(HandoffEdge {
            from_seat: fact.source_seat.clone(),
            to_seat: to,
            qitem_id: fact.qitem_id.clone(),
            chain_id: fact.chain_id.clone(),
            is_return,
            delivery_returned: false,
            source_key: fact.source_key.clone(),
        });
    }
    for index in 0..edges.len() {
        let qitem = edges[index].qitem_id.clone();
        let from = edges[index].from_seat.clone();
        edges[index].delivery_returned = edges[index + 1..]
            .iter()
            .any(|later| later.qitem_id == qitem && later.to_seat == from);
    }
    edges
}

pub const K2_CLOSED_ITEM: &str = "Closed item: a queue item whose destination is this seat, counted once, with at least one successful terminal transition inside the half-open window [from, to): state done (closure reason no-follow-on, handed_off_to, escalation, or none) or state handed-off. Excluded: canceled, denied, failed, blocked, unknown or unlisted states, and done with closure reason denied or canceled. Tokens per closed item = seat tokens in the window / closed items; zero closed items = no data.";

fn closed_item_transition(fact: &QueueFact) -> bool {
    match fact.state.as_str() {
        "handed-off" => true,
        "done" => matches!(
            fact.closure_reason.as_deref(),
            None | Some("no-follow-on") | Some("handed_off_to") | Some("escalation")
        ),
        _ => false,
    }
}

/// A source record of an item's destination: `queue.created` or `queue.claimed`.
/// Transition rows store the handoff target or actor instead, never the item's destination.
fn is_destination_record(fact: &QueueFact) -> bool {
    fact.source_key.starts_with("events:queue.created:")
        || fact.source_key.starts_with("events:queue.claimed:")
}

/// The item's destination at `fact`, and the source evidence for it: the latest
/// created or claimed event at or before the transition, else the item snapshot.
pub fn destination_at(
    fact: &QueueFact,
    queue: &[QueueFact],
    snapshots: &BTreeMap<String, String>,
) -> Option<(String, String)> {
    queue
        .iter()
        .filter(|row| {
            row.qitem_id == fact.qitem_id && is_destination_record(row) && row.ts <= fact.ts
        })
        .max_by(|a, b| a.ts.cmp(&b.ts).then(a.transition_id.cmp(&b.transition_id)))
        .map(|row| (row.dest_seat.clone(), row.source_key.clone()))
        .or_else(|| {
            snapshots
                .get(&fact.qitem_id)
                .map(|dest| (dest.clone(), "destination_snapshot".to_string()))
        })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClosedItem {
    pub qitem: String,
    pub transition: String,
    pub at: String,
    pub state: String,
    pub closure_reason: Option<String>,
    pub destination_evidence: String,
}

/// K2: each item counts once, for the seat that was its destination at its first
/// qualifying transition inside the window. The actor does not matter.
pub fn closed_items(
    seat: &str,
    queue: &[QueueFact],
    snapshots: &BTreeMap<String, String>,
    window: Window,
) -> Vec<ClosedItem> {
    let mut ordered: Vec<&QueueFact> = queue
        .iter()
        .filter(|fact| !is_destination_record(fact))
        .collect();
    ordered.sort_by(|a, b| a.ts.cmp(&b.ts).then(a.transition_id.cmp(&b.transition_id)));
    let mut seen = std::collections::BTreeSet::new();
    let mut items = Vec::new();
    for fact in ordered {
        if fact.ts < window.from || fact.ts >= window.to || !closed_item_transition(fact) {
            continue;
        }
        if !seen.insert(fact.qitem_id.as_str()) {
            continue;
        }
        let Some((destination, evidence)) = destination_at(fact, queue, snapshots) else {
            continue;
        };
        if destination == seat {
            items.push(ClosedItem {
                qitem: fact.qitem_id.clone(),
                transition: fact.source_key.clone(),
                at: fact.ts.to_rfc3339(),
                state: fact.state.clone(),
                closure_reason: fact.closure_reason.clone(),
                destination_evidence: evidence,
            });
        }
    }
    items
}

#[derive(Debug, Clone, PartialEq)]
pub enum Intake {
    Estimate { tokens: i64 },
    NoData,
    Ambiguous,
}

pub fn intake_cost(
    dest_seat: &str,
    claim_at: DateTime<Utc>,
    write_at: Option<DateTime<Utc>>,
    usages: &[UsageFact],
    seat_of: impl Fn(&UsageFact) -> Option<String>,
    other_open: bool,
) -> Intake {
    let Some(end) = write_at else {
        return Intake::NoData;
    };
    if other_open {
        return Intake::Ambiguous;
    }
    let mut tokens = 0;
    let mut any = false;
    for usage in usages {
        let Some(ts) = usage.ts else { continue };
        if seat_of(usage).as_deref() != Some(dest_seat) {
            continue;
        }
        if ts >= claim_at && ts < end {
            any = true;
            tokens += usage.input_uncached + usage.cache_read + usage.cache_write;
        }
    }
    if any {
        Intake::Estimate { tokens }
    } else {
        Intake::NoData
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntakeRow {
    pub qitem_id: String,
    pub to_seat: String,
    pub status: String,
    pub reason: Option<String>,
    pub tokens: Option<i64>,
    pub claim: Option<String>,
    pub write: Option<String>,
    pub samples: Vec<String>,
}

pub fn handoff_intakes(
    edges: &[HandoffEdge],
    queue: &[QueueFact],
    tools: &[ToolFact],
    usages: &[UsageFact],
    seat_of: impl Fn(&str) -> Option<String>,
) -> Vec<IntakeRow> {
    edges
        .iter()
        .map(|edge| intake_for_edge(edge, queue, tools, usages, &seat_of))
        .collect()
}

fn intake_for_edge(
    edge: &HandoffEdge,
    queue: &[QueueFact],
    tools: &[ToolFact],
    usages: &[UsageFact],
    seat_of: &impl Fn(&str) -> Option<String>,
) -> IntakeRow {
    let claim = queue
        .iter()
        .filter(|row| {
            row.qitem_id == edge.qitem_id
                && row.dest_seat == edge.to_seat
                && row.state == "in-progress"
                && row.ts >= edge_time(queue, edge)
        })
        .min_by_key(|row| (row.ts, row.transition_id));
    let Some(claim) = claim else {
        return intake_row(edge, "no_data", Some("no claim"), None, None, None, vec![]);
    };
    let linked = usages
        .iter()
        .any(|usage| seat_of(&usage.native_session).as_deref() == Some(edge.to_seat.as_str()));
    if !linked {
        return intake_row(
            edge,
            "no_data",
            Some("no linked session"),
            None,
            Some(claim.ts.to_rfc3339()),
            None,
            vec![],
        );
    }
    let write = tools
        .iter()
        .filter(|tool| {
            tool.structured_write
                && tool.ts.is_some_and(|ts| ts >= claim.ts)
                && seat_of(&tool.native_session).as_deref() == Some(edge.to_seat.as_str())
        })
        .min_by_key(|tool| (tool.ts, tool.seq));
    let Some(write) = write else {
        return intake_row(
            edge,
            "no_data",
            Some("no structured write"),
            None,
            Some(claim.ts.to_rfc3339()),
            None,
            vec![],
        );
    };
    let write_at = write.ts.unwrap();
    let overlap = queue.iter().any(|row| {
        row.qitem_id != edge.qitem_id
            && row.dest_seat == edge.to_seat
            && row.state == "in-progress"
            && row.ts < write_at
            && !closed_before(queue, row, claim.ts)
    });
    if overlap {
        return intake_row(
            edge,
            "ambiguous",
            Some("overlapping claim"),
            None,
            Some(claim.ts.to_rfc3339()),
            Some(write_at.to_rfc3339()),
            vec![],
        );
    }
    let mut tokens = 0i64;
    let mut samples = Vec::new();
    let mut any = false;
    for usage in usages {
        let Some(ts) = usage.ts else { continue };
        if seat_of(&usage.native_session).as_deref() != Some(edge.to_seat.as_str()) {
            continue;
        }
        if ts >= claim.ts && ts < write_at {
            any = true;
            let part = usage.input_uncached + usage.cache_read + usage.cache_write;
            tokens += part;
            samples.push(usage.source_key.clone());
        }
    }
    if !any {
        return intake_row(
            edge,
            "no_data",
            Some("no timestamped usage"),
            None,
            Some(claim.ts.to_rfc3339()),
            Some(write_at.to_rfc3339()),
            samples,
        );
    }
    intake_row(
        edge,
        "estimate",
        None,
        Some(tokens),
        Some(claim.ts.to_rfc3339()),
        Some(write_at.to_rfc3339()),
        samples,
    )
}

fn edge_time(queue: &[QueueFact], edge: &HandoffEdge) -> DateTime<Utc> {
    queue
        .iter()
        .find(|row| row.source_key == edge.source_key)
        .map(|row| row.ts)
        .unwrap_or(DateTime::<Utc>::UNIX_EPOCH)
}

fn closed_before(queue: &[QueueFact], open: &QueueFact, at: DateTime<Utc>) -> bool {
    queue.iter().any(|row| {
        row.qitem_id == open.qitem_id
            && row.ts <= at
            && row.transition_id >= open.transition_id
            && matches!(row.state.as_str(), "done" | "handed-off" | "canceled")
    })
}

fn intake_row(
    edge: &HandoffEdge,
    status: &str,
    reason: Option<&str>,
    tokens: Option<i64>,
    claim: Option<String>,
    write: Option<String>,
    samples: Vec<String>,
) -> IntakeRow {
    IntakeRow {
        qitem_id: edge.qitem_id.clone(),
        to_seat: edge.to_seat.clone(),
        status: status.to_string(),
        reason: reason.map(str::to_string),
        tokens,
        claim,
        write,
        samples,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PairMedian {
    pub from_seat: String,
    pub to_seat: String,
    pub wait_samples: Vec<f64>,
    pub wait_invalid: i64,
    pub wait_median: Option<f64>,
    pub intake_samples: Vec<f64>,
    pub intake_invalid: i64,
    pub intake_median: Option<f64>,
}

pub fn pair_medians(
    edges: &[HandoffEdge],
    queue: &[QueueFact],
    intakes: &[IntakeRow],
) -> Vec<PairMedian> {
    let mut pairs: BTreeMap<(String, String), PairMedian> = BTreeMap::new();
    for edge in edges {
        let entry = pairs
            .entry((edge.from_seat.clone(), edge.to_seat.clone()))
            .or_insert_with(|| PairMedian {
                from_seat: edge.from_seat.clone(),
                to_seat: edge.to_seat.clone(),
                wait_samples: Vec::new(),
                wait_invalid: 0,
                wait_median: None,
                intake_samples: Vec::new(),
                intake_invalid: 0,
                intake_median: None,
            });
        let handoff_at = edge_time(queue, edge);
        let claim = queue.iter().find(|row| {
            row.qitem_id == edge.qitem_id
                && row.dest_seat == edge.to_seat
                && row.state == "in-progress"
                && row.ts >= handoff_at
        });
        if let Some(claim) = claim {
            entry
                .wait_samples
                .push((claim.ts - handoff_at).num_seconds() as f64);
        } else {
            entry.wait_invalid += 1;
        }
    }
    for intake in intakes {
        let Some(edge) = edges
            .iter()
            .find(|edge| edge.qitem_id == intake.qitem_id && edge.to_seat == intake.to_seat)
        else {
            continue;
        };
        let Some(entry) = pairs.get_mut(&(edge.from_seat.clone(), edge.to_seat.clone())) else {
            continue;
        };
        if intake.status == "estimate" {
            if let Some(tokens) = intake.tokens {
                entry.intake_samples.push(tokens as f64);
            }
        } else {
            entry.intake_invalid += 1;
        }
    }
    for entry in pairs.values_mut() {
        entry.wait_median = even_median(entry.wait_samples.clone());
        entry.intake_median = even_median(entry.intake_samples.clone());
    }
    pairs.into_values().collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolLoop {
    pub session: String,
    pub tool: String,
    pub count: usize,
}

pub fn tool_loops(tools: &[ToolFact], failures: usize, window_secs: i64) -> Vec<ToolLoop> {
    let mut by_session: BTreeMap<String, Vec<&ToolFact>> = BTreeMap::new();
    for tool in tools {
        by_session
            .entry(tool.native_session.clone())
            .or_default()
            .push(tool);
    }
    let mut loops = Vec::new();
    for (session, mut events) in by_session {
        events.sort_by_key(|event| event.seq);
        let mut run: Vec<&ToolFact> = Vec::new();
        let flush = |run: &mut Vec<&ToolFact>, loops: &mut Vec<ToolLoop>, session: &str| {
            if run.len() >= failures {
                if let (Some(first), Some(last)) = (run.first(), run.last()) {
                    if let (Some(start), Some(end)) = (first.ts, last.ts) {
                        if (end - start).num_seconds() <= window_secs {
                            loops.push(ToolLoop {
                                session: session.to_string(),
                                tool: first.name.clone(),
                                count: run.len(),
                            });
                        }
                    }
                }
            }
            run.clear();
        };
        for event in events {
            let continues = event.failed == Some(true)
                && event.fingerprint.is_some()
                && run.last().is_some_and(|prev| {
                    prev.name == event.name && prev.fingerprint == event.fingerprint
                });
            if continues {
                run.push(event);
            } else {
                flush(&mut run, &mut loops, &session);
                if event.failed == Some(true) {
                    run.push(event);
                }
            }
        }
        flush(&mut run, &mut loops, &session);
    }
    loops
}

pub fn m1_yield(tokens: i64, nonreturn_handoffs: i64) -> Option<f64> {
    if nonreturn_handoffs == 0 {
        None
    } else {
        Some(tokens as f64 / nonreturn_handoffs as f64)
    }
}

pub fn even_median(mut values: Vec<f64>) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = values.len() / 2;
    if values.len().is_multiple_of(2) {
        Some((values[mid - 1] + values[mid]) / 2.0)
    } else {
        Some(values[mid])
    }
}

pub fn m2_return_rate(returns: i64, handoffs: i64) -> Option<f64> {
    if handoffs == 0 {
        None
    } else {
        Some(returns as f64 / handoffs as f64)
    }
}

pub fn m3_rework(edges: &[HandoffEdge]) -> (i64, i64, Vec<String>) {
    let mut chains: BTreeMap<String, Vec<&HandoffEdge>> = BTreeMap::new();
    for edge in edges {
        let key = edge
            .chain_id
            .clone()
            .unwrap_or_else(|| edge.qitem_id.clone());
        chains.entry(key).or_default().push(edge);
    }
    let mut rework = 0;
    let mut paths = Vec::new();
    for group in chains.values() {
        let mut saw_gate = false;
        let mut path = Vec::new();
        for edge in group {
            path.push(edge.from_seat.clone());
            path.push(edge.to_seat.clone());
            if edge.from_seat.contains("dev.qa") || edge.from_seat.contains("rev") {
                saw_gate = true;
            }
            if saw_gate
                && (edge.to_seat.starts_with("dev-build") || edge.to_seat.starts_with("dev.build"))
            {
                rework += 1;
                paths.push(path.join(" -> "));
                break;
            }
        }
    }
    (rework, chains.len() as i64, paths)
}

pub fn m4_impact() -> &'static str {
    "insufficient data: a valid skill comparison needs the same role with and without the skill and at least five closed items on each side"
}

pub fn report_value(kind: &str, payload: Value) -> Value {
    json!({
        "metric": kind,
        "definition_version": "acceptance-r1-a1",
        "tie_order": "source timestamp, then source sequence; queue boundaries apply before usage at the same timestamp",
        "payload": payload,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn ts(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000 + secs, 0).unwrap()
    }

    #[test]
    fn no_event_in_window_is_entirely_unknown() {
        let events = vec![ActivityFact {
            source_key: "a".into(),
            seat: "dev.build".into(),
            ts: ts(-10),
            state: "running".into(),
            seq: 1,
        }];
        let bands = activity_bands(
            "dev.build",
            &events,
            &[],
            Window {
                from: ts(0),
                to: ts(60),
            },
            &[],
        );
        assert_eq!(bands.unknown_secs, 60);
        assert_eq!(bands.active_secs, 0);
        assert_eq!(bands.sum(), 60);
    }

    fn life(created: Option<i64>, stopped: Option<i64>) -> SeatBound {
        SeatBound {
            seat: "dev.build".into(),
            created_at: created.map(ts),
            stopped_at: stopped.map(ts),
        }
    }

    fn running_at(secs: i64, seq: i64) -> ActivityFact {
        ActivityFact {
            source_key: format!("a{seq}"),
            seat: "dev.build".into(),
            ts: ts(secs),
            state: "running".into(),
            seq,
        }
    }

    fn window() -> Window {
        Window {
            from: ts(0),
            to: ts(60),
        }
    }

    #[test]
    fn a_state_does_not_carry_across_a_stop_and_relaunch() {
        let lives = [life(None, Some(20)), life(Some(40), None)];
        let events = [running_at(5, 1)];
        let bands = activity_bands("dev.build", &events, &[], window(), &lives);
        let spans = observation_spans(window(), &lives);
        assert_eq!(observed_secs(&spans), 40);
        assert_eq!(
            bands.unknown_secs,
            5 + 20,
            "before the first event, and after relaunch"
        );
        assert_eq!(bands.active_secs, 15);
        assert_eq!(bands.sum(), observed_secs(&spans));
    }

    #[test]
    fn a_launch_before_the_window_still_carries_its_own_state() {
        let lives = [life(Some(-30), None)];
        let events = [running_at(-40, 1), running_at(-10, 2), running_at(30, 3)];
        let bands = activity_bands("dev.build", &events, &[], window(), &lives);
        assert_eq!(bands.active_secs, 60);
        let events = [running_at(-40, 1), running_at(30, 2)];
        let bands = activity_bands("dev.build", &events, &[], window(), &lives);
        assert_eq!((bands.unknown_secs, bands.active_secs), (30, 30));
    }

    #[test]
    fn a_seat_launched_mid_window_is_observed_from_its_launch() {
        let lives = [life(Some(30), None)];
        let bands = activity_bands("dev.build", &[running_at(30, 1)], &[], window(), &lives);
        assert_eq!(bands.active_secs, 30);
        assert_eq!(bands.sum(), 30);
    }

    #[test]
    fn a_seat_stopped_before_the_window_is_not_observed() {
        let lives = [life(Some(-50), Some(-10))];
        let spans = observation_spans(window(), &lives);
        assert!(spans.is_empty());
        let bands = activity_bands("dev.build", &[running_at(-20, 1)], &[], window(), &lives);
        assert_eq!(bands.sum(), 0);
    }

    #[test]
    fn overlapping_lives_from_two_sources_count_once() {
        let lives = [
            life(Some(10), Some(40)),
            life(Some(20), None),
            life(Some(50), None),
        ];
        let spans = observation_spans(window(), &lives);
        assert_eq!(spans.len(), 1);
        assert_eq!(observed_secs(&spans), 50);
        let bands = activity_bands("dev.build", &[running_at(10, 1)], &[], window(), &lives);
        assert_eq!(bands.sum(), 50);
    }

    #[test]
    fn a_seat_without_lifecycle_events_is_observed_for_the_whole_window() {
        let spans = observation_spans(window(), &[]);
        assert_eq!(observed_secs(&spans), 60);
        let bands = activity_bands("dev.build", &[], &[], window(), &[]);
        assert_eq!(bands.unknown_secs, 60);
    }

    #[test]
    fn idle_splits_when_demand_appears() {
        let events = vec![ActivityFact {
            source_key: "a".into(),
            seat: "dev.build".into(),
            ts: ts(0),
            state: "idle".into(),
            seq: 1,
        }];
        let queue = vec![
            QueueFact {
                source_key: "q1".into(),
                transition_id: 1,
                qitem_id: "item".into(),
                ts: ts(10),
                state: "pending".into(),
                source_seat: "orch.arch".into(),
                dest_seat: "dev.build".into(),
                closure_reason: None,
                closure_target: None,
                chain_id: None,
                missions: vec!["m".into()],
            },
            QueueFact {
                source_key: "q2".into(),
                transition_id: 2,
                qitem_id: "item".into(),
                ts: ts(20),
                state: "done".into(),
                source_seat: "dev.build".into(),
                dest_seat: "dev.build".into(),
                closure_reason: Some("no-follow-on".into()),
                closure_target: None,
                chain_id: None,
                missions: vec!["m".into()],
            },
        ];
        let bands = activity_bands(
            "dev.build",
            &events,
            &queue,
            Window {
                from: ts(0),
                to: ts(60),
            },
            &[],
        );
        assert_eq!(bands.idle_with_work_secs, 10);
        assert_eq!(bands.idle_without_work_secs, 50);
        assert_eq!(bands.sum(), 60);
    }

    #[test]
    fn overlapping_missions_are_ambiguous_and_conserve() {
        let queue = vec![
            QueueFact {
                source_key: "a".into(),
                transition_id: 1,
                qitem_id: "i1".into(),
                ts: ts(0),
                state: "in-progress".into(),
                source_seat: "s".into(),
                dest_seat: "dev.build".into(),
                closure_reason: None,
                closure_target: None,
                chain_id: None,
                missions: vec!["one".into()],
            },
            QueueFact {
                source_key: "b".into(),
                transition_id: 2,
                qitem_id: "i2".into(),
                ts: ts(0),
                state: "in-progress".into(),
                source_seat: "s".into(),
                dest_seat: "dev.build".into(),
                closure_reason: None,
                closure_target: None,
                chain_id: None,
                missions: vec!["two".into()],
            },
        ];
        assert_eq!(
            mission_at("dev.build", &queue, ts(5)),
            MissionBucket::Ambiguous
        );
    }

    fn queue(
        id: &str,
        key: &str,
        at: i64,
        state: &str,
        from: &str,
        to: &str,
        reason: Option<&str>,
        target: Option<&str>,
    ) -> QueueFact {
        QueueFact {
            source_key: key.into(),
            transition_id: at,
            qitem_id: id.into(),
            ts: ts(at),
            state: state.into(),
            source_seat: from.into(),
            dest_seat: to.into(),
            closure_reason: reason.map(str::to_string),
            closure_target: target.map(str::to_string),
            chain_id: Some("chain".into()),
            missions: vec!["m".into()],
        }
    }

    #[test]
    fn handoff_return_uses_prior_seats_on_the_item() {
        let queue = vec![
            queue(
                "item",
                "q1",
                0,
                "pending",
                "orch.arch",
                "dev.build",
                None,
                None,
            ),
            queue(
                "item",
                "q2",
                5,
                "handed-off",
                "dev.build",
                "dev.qa",
                Some("handed_off_to"),
                Some("dev.qa"),
            ),
            queue(
                "item",
                "q3",
                9,
                "handed-off",
                "dev.qa",
                "dev.build",
                Some("handed_off_to"),
                Some("dev.build"),
            ),
        ];
        let edges = handoff_edges(&queue);
        assert_eq!(edges.len(), 2);
        assert!(!edges[0].is_return);
        assert!(edges[1].is_return);
        assert_eq!(edges[1].to_seat, "dev.build");
    }

    #[test]
    fn intake_includes_claim_and_excludes_the_write() {
        let usages = vec![
            UsageFact {
                source_key: "u0".into(),
                harness: "claude".into(),
                native_session: "s".into(),
                message_id: None,
                ts: Some(ts(0)),
                seq: 0,
                input_uncached: 2,
                output: 0,
                cache_read: 1,
                cache_write: 0,
                reasoning: 0,
                currency_total: None,
                generation: 1,
                predecessor_source_key: None,
                predecessor_reason: None,
                prior_input_tokens: None,
                prior_cached_input_tokens: None,
                prior_cache_write_input_tokens: None,
                prior_output_tokens: None,
                prior_reasoning_output_tokens: None,
            },
            UsageFact {
                source_key: "u5".into(),
                harness: "claude".into(),
                native_session: "s".into(),
                message_id: None,
                ts: Some(ts(5)),
                seq: 1,
                input_uncached: 3,
                output: 0,
                cache_read: 0,
                cache_write: 1,
                reasoning: 0,
                currency_total: None,
                generation: 1,
                predecessor_source_key: None,
                predecessor_reason: None,
                prior_input_tokens: None,
                prior_cached_input_tokens: None,
                prior_cache_write_input_tokens: None,
                prior_output_tokens: None,
                prior_reasoning_output_tokens: None,
            },
            UsageFact {
                source_key: "u10".into(),
                harness: "claude".into(),
                native_session: "s".into(),
                message_id: None,
                ts: Some(ts(10)),
                seq: 2,
                input_uncached: 9,
                output: 0,
                cache_read: 0,
                cache_write: 0,
                reasoning: 0,
                currency_total: None,
                generation: 1,
                predecessor_source_key: None,
                predecessor_reason: None,
                prior_input_tokens: None,
                prior_cached_input_tokens: None,
                prior_cache_write_input_tokens: None,
                prior_output_tokens: None,
                prior_reasoning_output_tokens: None,
            },
        ];
        let seat = |_u: &UsageFact| Some("dev.qa".to_string());
        assert_eq!(
            intake_cost("dev.qa", ts(0), Some(ts(10)), &usages, seat, false),
            Intake::Estimate { tokens: 7 }
        );
        assert_eq!(
            intake_cost("dev.qa", ts(0), None, &usages, seat, false),
            Intake::NoData
        );
        assert_eq!(
            intake_cost("dev.qa", ts(0), Some(ts(10)), &usages, seat, true),
            Intake::Ambiguous
        );
    }

    fn tool(seq: i64, failed: Option<bool>, name: &str) -> ToolFact {
        ToolFact {
            source_key: format!("t{seq}"),
            harness: "claude".into(),
            native_session: "s".into(),
            ts: Some(ts(seq * 10)),
            seq,
            name: name.into(),
            fingerprint: Some("fp".into()),
            failed,
            structured_write: false,
            generation: 1,
        }
    }

    #[test]
    fn tool_loop_counts_three_or_more_as_one_episode() {
        let two = vec![tool(0, Some(true), "Read"), tool(1, Some(true), "Read")];
        assert!(tool_loops(&two, 3, 600).is_empty());
        let three = vec![
            tool(0, Some(true), "Read"),
            tool(1, Some(true), "Read"),
            tool(2, Some(true), "Read"),
        ];
        let loops = tool_loops(&three, 3, 600);
        assert_eq!(loops.len(), 1);
        assert_eq!(loops[0].count, 3);
        let mut four = three.clone();
        four.push(tool(3, Some(true), "Read"));
        assert_eq!(tool_loops(&four, 3, 600)[0].count, 4);
        let broken = vec![
            tool(0, Some(true), "Read"),
            tool(1, None, "Read"),
            tool(2, Some(true), "Read"),
            tool(3, Some(true), "Read"),
        ];
        assert!(tool_loops(&broken, 3, 600).is_empty());
    }

    #[test]
    fn decision_metrics_keep_zero_denominator_and_even_median() {
        assert_eq!(m1_yield(100, 0), None);
        assert_eq!(m1_yield(100, 4), Some(25.0));
        assert_eq!(even_median(vec![1.0, 3.0, 5.0, 7.0]), Some(4.0));
        assert_eq!(m2_return_rate(0, 0), None);
        assert_eq!(m2_return_rate(1, 4), Some(0.25));
        assert_eq!(
            m4_impact(),
            "insufficient data: a valid skill comparison needs the same role with and without the skill and at least five closed items on each side"
        );
    }

    #[test]
    fn m3_counts_a_return_to_build_after_qa() {
        let edges = vec![
            HandoffEdge {
                from_seat: "dev-build".into(),
                to_seat: "dev.qa".into(),
                qitem_id: "i".into(),
                chain_id: Some("c".into()),
                is_return: false,
                delivery_returned: true,
                source_key: "a".into(),
            },
            HandoffEdge {
                from_seat: "dev.qa".into(),
                to_seat: "rev-review".into(),
                qitem_id: "i".into(),
                chain_id: Some("c".into()),
                is_return: false,
                delivery_returned: false,
                source_key: "b".into(),
            },
            HandoffEdge {
                from_seat: "rev-review".into(),
                to_seat: "dev-build".into(),
                qitem_id: "i".into(),
                chain_id: Some("c".into()),
                is_return: true,
                delivery_returned: false,
                source_key: "c".into(),
            },
        ];
        let (count, chains, paths) = m3_rework(&edges);
        assert_eq!(count, 1);
        assert_eq!(chains, 1);
        assert_eq!(paths.len(), 1);
    }

    #[test]
    fn indirect_return_marks_membership_without_inflating_is_return() {
        let window = Window {
            from: ts(0),
            to: ts(60),
        };
        let queue = vec![
            QueueFact {
                source_key: "t1".into(),
                transition_id: 1,
                qitem_id: "item".into(),
                ts: ts(10),
                state: "handed-off".into(),
                source_seat: "A".into(),
                dest_seat: "B".into(),
                closure_reason: Some("handed_off_to".into()),
                closure_target: Some("B".into()),
                chain_id: None,
                missions: vec![],
            },
            QueueFact {
                source_key: "t2".into(),
                transition_id: 2,
                qitem_id: "item".into(),
                ts: ts(20),
                state: "handed-off".into(),
                source_seat: "B".into(),
                dest_seat: "C".into(),
                closure_reason: Some("handed_off_to".into()),
                closure_target: Some("C".into()),
                chain_id: None,
                missions: vec![],
            },
            QueueFact {
                source_key: "t3".into(),
                transition_id: 3,
                qitem_id: "item".into(),
                ts: ts(30),
                state: "handed-off".into(),
                source_seat: "C".into(),
                dest_seat: "A".into(),
                closure_reason: Some("handed_off_to".into()),
                closure_target: Some("A".into()),
                chain_id: None,
                missions: vec![],
            },
        ];
        let edges = handoff_edges(&queue);
        assert_eq!(edges.iter().filter(|edge| edge.is_return).count(), 1);
        assert!(edges[0].delivery_returned);
        assert!(!edges[1].delivery_returned);
        assert!(!edges[2].delivery_returned);
        // The handoff target is not the item's destination (K2).
        let snapshots = BTreeMap::from([("item".to_string(), "A".to_string())]);
        assert!(closed_items("B", &queue, &snapshots, window).is_empty());
        let closed = closed_items("A", &queue, &snapshots, window);
        assert_eq!(closed.len(), 1);
        assert_eq!(closed[0].transition, "t1");
        assert!(K2_CLOSED_ITEM.contains("zero closed items = no data"));
    }

    fn q(
        key: &str,
        id: i64,
        item: &str,
        at: i64,
        state: &str,
        source: &str,
        dest: &str,
        reason: Option<&str>,
    ) -> QueueFact {
        QueueFact {
            source_key: key.into(),
            transition_id: id,
            qitem_id: item.into(),
            ts: ts(at),
            state: state.into(),
            source_seat: source.into(),
            dest_seat: dest.into(),
            closure_reason: reason.map(str::to_string),
            closure_target: None,
            chain_id: None,
            missions: vec![],
        }
    }

    fn created(item: &str, at: i64, dest: &str) -> QueueFact {
        let mut fact = q(
            &format!("events:queue.created:{item}"),
            0,
            item,
            at,
            "pending",
            "orch",
            dest,
            None,
        );
        fact.transition_id = 1_000 + at;
        fact
    }

    fn members(items: &[ClosedItem]) -> Vec<&str> {
        items.iter().map(|item| item.transition.as_str()).collect()
    }

    #[test]
    fn closed_items_follow_k2_states_reasons_and_window() {
        let window = Window {
            from: ts(0),
            to: ts(60),
        };
        let mut queue = Vec::new();
        for item in "abcdefghijkmno".chars() {
            queue.push(created(&item.to_string(), -100, "seat"));
        }
        // The actor and the stored handoff target are never the destination.
        let other = |key: &str, id, item: &str, state, reason| {
            q(key, id, item, 10, state, "someone", "elsewhere", reason)
        };
        queue.extend([
            other("ok-none", 1, "a", "done", None),
            other("ok-follow", 2, "b", "done", Some("no-follow-on")),
            other("ok-hand", 3, "c", "done", Some("handed_off_to")),
            other("ok-esc", 4, "d", "done", Some("escalation")),
            other("ok-state", 5, "e", "handed-off", Some("handed_off_to")),
            other("bad-cancel", 6, "f", "canceled", None),
            other("bad-denied", 7, "g", "done", Some("denied")),
            other("bad-canceled-reason", 8, "h", "done", Some("canceled")),
            other("bad-failed", 9, "i", "failed", None),
            other("bad-blocked", 10, "j", "blocked", None),
            other("bad-unknown", 11, "k", "unknown", None),
            other("bad-unlisted-reason", 12, "o", "done", Some("superseded")),
            q("too-early", 13, "m", -1, "done", "seat", "seat", None),
            q("at-start", 14, "m", 0, "done", "x", "x", None),
            q("at-end", 15, "n", 60, "done", "seat", "seat", None),
            q("open", 16, "a", 5, "in-progress", "seat", "seat", None),
            q("mirror", 17, "a", 20, "handed-off", "x", "x", None),
            q("repeat", 18, "c", 30, "done", "x", "x", None),
        ]);
        let none = BTreeMap::new();
        let items = closed_items("seat", &queue, &none, window);
        assert_eq!(
            members(&items),
            vec![
                "at-start",
                "ok-none",
                "ok-follow",
                "ok-hand",
                "ok-esc",
                "ok-state"
            ]
        );
        assert!(items.iter().all(|item| item
            .destination_evidence
            .starts_with("events:queue.created:")));
        assert!(closed_items("elsewhere", &queue, &none, window).is_empty());
        assert!(closed_items("someone", &queue, &none, window).is_empty());
        assert!(K2_CLOSED_ITEM.contains("zero closed items = no data"));
    }

    #[test]
    fn closed_item_destination_comes_from_history_then_snapshot() {
        let window = Window {
            from: ts(0),
            to: ts(60),
        };
        let mut claimed = q(
            "events:queue.claimed:9",
            2_000,
            "a",
            5,
            "in-progress",
            "seat-b",
            "seat-b",
            None,
        );
        claimed.source_key = "events:queue.claimed:9".into();
        let queue = vec![
            created("a", -10, "seat-a"),
            claimed,
            q("closed-a", 1, "a", 10, "done", "seat-b", "unknown", None),
            q("closed-b", 2, "b", 10, "done", "actor", "unknown", None),
            q("closed-c", 3, "c", 10, "done", "actor", "unknown", None),
            // A claim after the closure does not move the closure.
            created("d", -10, "seat-a"),
            q("closed-d", 4, "d", 10, "done", "actor", "unknown", None),
            q(
                "events:queue.claimed:99",
                2_001,
                "d",
                20,
                "in-progress",
                "seat-b",
                "seat-b",
                None,
            ),
        ];
        let snapshots = BTreeMap::from([
            ("a".to_string(), "seat-a".to_string()),
            ("b".to_string(), "seat-b".to_string()),
        ]);
        let b = closed_items("seat-b", &queue, &snapshots, window);
        assert_eq!(members(&b), vec!["closed-a", "closed-b"]);
        assert_eq!(b[0].destination_evidence, "events:queue.claimed:9");
        assert_eq!(b[1].destination_evidence, "destination_snapshot");
        let a = closed_items("seat-a", &queue, &snapshots, window);
        assert_eq!(members(&a), vec!["closed-d"]);
        // No destination evidence at all: the item is not anyone's closed item.
        assert!(["seat-a", "seat-b", "actor", "unknown"]
            .iter()
            .all(
                |seat| !members(&closed_items(seat, &queue, &snapshots, window))
                    .contains(&"closed-c")
            ));
    }
}
