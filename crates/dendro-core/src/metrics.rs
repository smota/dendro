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
    /// Ambiguous, with its visible reason (K3).
    Ambiguous(&'static str),
}

pub const AMBIGUOUS_INSUFFICIENT_HISTORY: &str = "insufficient history";
pub const AMBIGUOUS_CONFLICTING_MISSIONS: &str = "conflicting missions";
pub const AMBIGUOUS_TAG_CHANGE: &str = "detectable tag change";

/// The item's state at `ts` from all of its queue records: created (pending),
/// claimed (in-progress), unclaimed (pending, K5) and transitions.
fn item_state_at<'a>(item: &[&'a QueueFact], ts: DateTime<Utc>) -> Option<&'a QueueFact> {
    item.iter()
        .copied()
        .filter(|row| row.ts <= ts)
        .max_by_key(|row| (row.ts, row.transition_id))
}

/// K3 mission attribution of a usage record of `seat` at `ts`. An item is the
/// seat's when it is in-progress at `ts` and its K5 destination then is the
/// seat; the actor is never the destination and the current destination is
/// never painted backward. An in-progress item whose destination cannot be
/// proven, but whose claim or snapshot names this seat, makes the usage
/// ambiguous with "insufficient history". Missions are the item's `mission:`
/// tags as set at creation; stored readings that disagree are a detectable
/// tag change. Reason order: insufficient history, tag change, conflicting.
pub fn mission_at(
    seat: &str,
    queue: &[QueueFact],
    snapshots: &BTreeMap<String, Vec<DestinationSnapshot>>,
    ts: DateTime<Utc>,
) -> MissionBucket {
    let mut items: BTreeMap<&str, Vec<&QueueFact>> = BTreeMap::new();
    for fact in queue {
        items.entry(fact.qitem_id.as_str()).or_default().push(fact);
    }
    let mut owned = Vec::new();
    let mut insufficient = false;
    for (qitem, facts) in &items {
        let Some(state) = item_state_at(facts, ts) else {
            continue;
        };
        if state.state != "in-progress" {
            continue;
        }
        let mut probe = state.clone();
        probe.ts = ts;
        match destination_at(&probe, queue, snapshots) {
            Ok((destination, _, _)) if destination == seat => owned.push(*qitem),
            Ok(_) => {}
            Err(_) => {
                let named = facts.iter().any(|row| {
                    row.source_key.starts_with("events:queue.claimed:")
                        && row.dest_seat == seat
                        && row.ts <= ts
                }) || snapshots
                    .get(*qitem)
                    .and_then(|rows| rows.last())
                    .is_some_and(|snapshot| snapshot.destination == seat);
                insufficient |= named;
            }
        }
    }
    if insufficient {
        return MissionBucket::Ambiguous(AMBIGUOUS_INSUFFICIENT_HISTORY);
    }
    if owned.is_empty() {
        return MissionBucket::None;
    }
    let mut missions = Vec::new();
    let mut conflicting = false;
    for qitem in owned {
        let Some(readings) = snapshots.get(qitem).filter(|rows| !rows.is_empty()) else {
            return MissionBucket::Ambiguous(AMBIGUOUS_INSUFFICIENT_HISTORY);
        };
        if readings
            .windows(2)
            .any(|pair| pair[0].missions != pair[1].missions)
        {
            return MissionBucket::Ambiguous(AMBIGUOUS_TAG_CHANGE);
        }
        let tags = &readings[readings.len() - 1].missions;
        conflicting |= tags.len() > 1;
        missions.extend(tags.iter().cloned());
    }
    missions.sort();
    missions.dedup();
    match missions.len() {
        _ if conflicting => MissionBucket::Ambiguous(AMBIGUOUS_CONFLICTING_MISSIONS),
        0 => MissionBucket::None,
        1 => MissionBucket::Mission(missions.remove(0)),
        _ => MissionBucket::Ambiguous(AMBIGUOUS_CONFLICTING_MISSIONS),
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
    /// The first later edge, on the same item or chain, that brought the work
    /// back to `from_seat`, and its time. This answers the delivery; it is not
    /// `is_return` and never counts as an M2 return.
    pub returned_by: Option<(String, DateTime<Utc>)>,
    pub ts: DateTime<Utc>,
    pub source_key: String,
}

impl HandoffEdge {
    /// K6: for M1 a delivery is returned only when the return is inside the
    /// same half-open window. A later return never changes this window.
    pub fn returned_within(&self, window: Window) -> bool {
        self.returned_by
            .as_ref()
            .is_some_and(|(_, at)| *at >= window.from && *at < window.to)
    }

    /// Returns are found within the chain of record, else within the item.
    pub fn chain_key(&self) -> &str {
        self.chain_id.as_deref().unwrap_or(&self.qitem_id)
    }
}

/// The creation of a non-root chain item. Chain and item ids are sealed alike,
/// so a differing chain id means the item has a predecessor.
fn is_successor_creation(fact: &QueueFact) -> bool {
    fact.source_key.starts_with("events:queue.created:")
        && fact
            .chain_id
            .as_deref()
            .is_some_and(|chain| chain != fact.qitem_id)
}

/// The receiver's first claim after a handoff, on any item of the same chain
/// (OpenRig hands work over on a new item), else on the same item.
fn receiver_claim<'a>(queue: &'a [QueueFact], edge: &HandoffEdge) -> Option<&'a QueueFact> {
    let at = edge_time(queue, edge);
    queue
        .iter()
        .filter(|row| {
            row.chain_id.as_deref().unwrap_or(&row.qitem_id) == edge.chain_key()
                && row.dest_seat == edge.to_seat
                && row.state == "in-progress"
                && row.ts >= at
        })
        .min_by_key(|row| (row.ts, row.transition_id))
}

pub fn handoff_edges(queue: &[QueueFact]) -> Vec<HandoffEdge> {
    let mut seen: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    let mut edges = Vec::new();
    let mut ordered = queue.to_vec();
    ordered.sort_by(|a, b| a.ts.cmp(&b.ts).then(a.transition_id.cmp(&b.transition_id)));
    for fact in ordered {
        // A handoff opens the next item of the chain, often before the closing
        // transition is written. That creation is the handoff itself, so the
        // receiver has not touched the chain yet.
        if is_successor_creation(&fact) {
            continue;
        }
        let key = fact
            .chain_id
            .clone()
            .unwrap_or_else(|| fact.qitem_id.clone());
        let touch = seen.entry(key).or_default();
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
            returned_by: None,
            ts: fact.ts,
            source_key: fact.source_key.clone(),
        });
    }
    for index in 0..edges.len() {
        let returned_by = edges[index + 1..]
            .iter()
            .find(|later| {
                later.chain_key() == edges[index].chain_key()
                    && later.to_seat == edges[index].from_seat
            })
            .map(|later| (later.source_key.clone(), later.ts));
        edges[index].returned_by = returned_by;
    }
    edges
}

pub const K2_CLOSED_ITEM: &str = "Closed item: a queue item whose destination is this seat, counted once, with at least one successful terminal transition inside the half-open window [from, to): state done (closure reason no-follow-on, handed_off_to, escalation, or none) or state handed-off. Excluded: canceled, denied, failed, blocked, unknown or unlisted states, and done with closure reason denied or canceled. Tokens per closed item = seat tokens in the window / closed items; zero closed items = no data.";

pub const K2_DESTINATION_RULE: &str = "K5: destination at the closing transition = the latest queue.created destinationSession or qitem.fallback_routed rerouteDestination at or before it. The current queue_items.destination_session (snapshot cutoff queue_items.ts_updated) is used only when the item's queue.created is in the event history and there is no reroute mark (fallback-routed: transition note, qitem.fallback_routed event, fallback-from: chain_of_record entry, or differing stored snapshots). A reroute mark without a reroute event, or a creation outside the history, is insufficient history: the item counts for no seat and is listed under closed items with insufficient history.";

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

/// K5 destination-bearing records: `queue.created` and `qitem.fallback_routed`.
/// `queue.claimed` carries the same value and only confirms it.
fn is_destination_record(fact: &QueueFact) -> bool {
    fact.source_key.starts_with("events:queue.created:")
        || fact.source_key.starts_with("events:qitem.fallback_routed:")
}

/// Facts read from OpenRig events rather than `queue_transitions`. None is a closure.
fn is_event_record(fact: &QueueFact) -> bool {
    fact.source_key.starts_with("events:")
}

pub const INSUFFICIENT_HISTORY: &str = "insufficient history";
pub const NO_CREATION_IN_HISTORY: &str =
    "insufficient history: the item's queue.created is outside the event history";
pub const UNTIMED_REROUTE_MARK: &str = "insufficient history: a reroute mark without a timed qitem.fallback_routed event, so the reroute cannot be placed before or after the closure";
/// Two readings of queue_items.destination_session at different cutoffs show
/// the destination changed. Without a reroute event the change has no time,
/// so it conflicts with any single destination for the closure (K5 needs proof
/// of no change to use the current value).
pub const CONFLICTING_SNAPSHOTS: &str = "insufficient history: stored destination snapshots disagree, which is conflicting historical evidence of an untimed change";

/// K5: the item's destination at `fact`, with the source evidence for it.
/// History first: the latest created or reroute event at or before `fact`.
/// The current snapshot only with proof of no change: the item's creation is in
/// the event history and the source shows no reroute mark. A reroute mark
/// without a reroute event leaves no usable destination.
pub fn destination_at(
    fact: &QueueFact,
    queue: &[QueueFact],
    snapshots: &BTreeMap<String, Vec<DestinationSnapshot>>,
) -> Result<(String, String, Option<String>), &'static str> {
    let item: Vec<&QueueFact> = queue
        .iter()
        .filter(|row| row.qitem_id == fact.qitem_id)
        .collect();
    let rerouted_by_event = item
        .iter()
        .any(|row| row.source_key.starts_with("events:qitem.fallback_routed:"));
    let created = item
        .iter()
        .any(|row| row.source_key.starts_with("events:queue.created:"));
    let item_snapshots = snapshots
        .get(&fact.qitem_id)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    // Timed reroute events resolve the destination at any instant, so marks
    // only block when no such event exists.
    if !rerouted_by_event {
        if item_snapshots
            .iter()
            .any(|snapshot| snapshot.reroute_marked)
        {
            return Err(UNTIMED_REROUTE_MARK);
        }
        if item_snapshots
            .windows(2)
            .any(|pair| pair[0].destination != pair[1].destination)
        {
            return Err(CONFLICTING_SNAPSHOTS);
        }
    }
    if let Some(row) = item
        .iter()
        .filter(|row| is_destination_record(row) && row.ts <= fact.ts)
        .max_by_key(|row| (row.ts, row.transition_id))
    {
        return Ok((row.dest_seat.clone(), row.source_key.clone(), None));
    }
    if !created {
        return Err(NO_CREATION_IN_HISTORY);
    }
    // Reached only when the creation is recorded after this transition. A
    // reroute event then leaves the current value unproven.
    if rerouted_by_event {
        return Err(UNTIMED_REROUTE_MARK);
    }
    let latest = item_snapshots.last().ok_or(NO_CREATION_IN_HISTORY)?;
    Ok((
        latest.destination.clone(),
        "destination_snapshot".to_string(),
        Some(latest.cutoff.clone()),
    ))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClosedItem {
    pub qitem: String,
    pub transition: String,
    pub at: String,
    pub state: String,
    pub closure_reason: Option<String>,
    pub destination_evidence: String,
    pub snapshot_cutoff: Option<String>,
}

/// K2 membership for every seat: each item once, at its first qualifying
/// transition inside the window, with its destination or `None` when unknown.
/// For `None` (K5 insufficient history) the item counts for no seat, and
/// `destination_evidence` names the reason.
pub fn k2_closures(
    queue: &[QueueFact],
    snapshots: &BTreeMap<String, Vec<DestinationSnapshot>>,
    window: Window,
) -> Vec<(Option<String>, ClosedItem)> {
    let mut ordered: Vec<&QueueFact> = queue.iter().filter(|fact| !is_event_record(fact)).collect();
    ordered.sort_by(|a, b| a.ts.cmp(&b.ts).then(a.transition_id.cmp(&b.transition_id)));
    let mut seen = std::collections::BTreeSet::new();
    let mut closures = Vec::new();
    for fact in ordered {
        if fact.ts < window.from || fact.ts >= window.to || !closed_item_transition(fact) {
            continue;
        }
        if !seen.insert(fact.qitem_id.as_str()) {
            continue;
        }
        let (destination, evidence, cutoff) = match destination_at(fact, queue, snapshots) {
            Ok((destination, evidence, cutoff)) => (Some(destination), evidence, cutoff),
            Err(reason) => (None, reason.to_string(), None),
        };
        closures.push((
            destination,
            ClosedItem {
                qitem: fact.qitem_id.clone(),
                transition: fact.source_key.clone(),
                at: fact.ts.to_rfc3339(),
                state: fact.state.clone(),
                closure_reason: fact.closure_reason.clone(),
                destination_evidence: evidence,
                snapshot_cutoff: cutoff,
            },
        ));
    }
    closures
}

/// K2: the items whose destination was this seat. The actor does not matter.
pub fn closed_items(
    seat: &str,
    queue: &[QueueFact],
    snapshots: &BTreeMap<String, Vec<DestinationSnapshot>>,
    window: Window,
) -> Vec<ClosedItem> {
    k2_closures(queue, snapshots, window)
        .into_iter()
        .filter(|(destination, _)| destination.as_deref() == Some(seat))
        .map(|(_, item)| item)
        .collect()
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
    let Some(claim) = receiver_claim(queue, edge) else {
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
        row.qitem_id != claim.qitem_id
            && row.qitem_id != edge.qitem_id
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
    /// (handoff source key, claim source key) for each wait sample, in order.
    pub wait_sample_keys: Vec<(String, String)>,
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
                wait_sample_keys: Vec::new(),
                wait_invalid: 0,
                wait_median: None,
                intake_samples: Vec::new(),
                intake_invalid: 0,
                intake_median: None,
            });
        let handoff_at = edge_time(queue, edge);
        if let Some(claim) = receiver_claim(queue, edge) {
            entry
                .wait_samples
                .push((claim.ts - handoff_at).num_seconds() as f64);
            entry
                .wait_sample_keys
                .push((edge.source_key.clone(), claim.source_key.clone()));
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

/// K7: build seats are the contract-named `dev-build*` members, by name.
fn is_build_seat(seat: &str) -> bool {
    seat.split('@')
        .next()
        .unwrap_or(seat)
        .replace('.', "-")
        .starts_with("dev-build")
}

/// A seat's M3 role from recorded evidence (K7). Names never decide a check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SeatRole {
    Check(Vec<String>),
    Other,
    Unknown(&'static str),
}

pub const ROLE_NOT_RECORDED: &str = "no recorded role evidence";
pub const ROLE_CONFLICTING: &str = "conflicting recorded role evidence";

/// Resolves stored (seat, role, evidence) observations into one role per seat.
pub fn seat_roles(rows: &[(String, String, String)]) -> BTreeMap<String, SeatRole> {
    let mut by_seat: BTreeMap<&str, Vec<&(String, String, String)>> = BTreeMap::new();
    for row in rows {
        by_seat.entry(row.0.as_str()).or_default().push(row);
    }
    by_seat
        .into_iter()
        .map(|(seat, rows)| {
            let check: Vec<String> = rows
                .iter()
                .filter(|row| row.1 == "check")
                .map(|row| row.2.clone())
                .collect();
            let other = rows.iter().any(|row| row.1 == "other");
            let role = match (check.is_empty(), other) {
                (false, true) => SeatRole::Unknown(ROLE_CONFLICTING),
                (false, false) => SeatRole::Check(check),
                (true, _) => SeatRole::Other,
            };
            (seat.to_string(), role)
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Rework {
    pub chain: String,
    pub path: String,
    /// The checking seats the work passed before it came back, with evidence.
    pub checks: Vec<(String, Vec<String>)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnknownChain {
    pub chain: String,
    pub path: String,
    /// Role-unknown seats passed before a return to build, with the reason.
    pub unknown_seats: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct M3 {
    pub rework: Vec<Rework>,
    pub unknown: Vec<UnknownChain>,
    /// Chains in the window, without the unknown ones (K7 denominator).
    pub chains: i64,
}

/// M3 (K7): a chain is rework when work comes back to a build seat after it
/// passed a checking seat. A chain passes a seat when that seat was the
/// destination of one of its items, so the sender of a handoff counts. A
/// return after passes through role-unknown seats only makes an unknown chain,
/// left out of both the count and the denominator. Each chain counts once.
pub fn m3_rework(edges: &[HandoffEdge], roles: &BTreeMap<String, SeatRole>) -> M3 {
    let role_of = |seat: &str| {
        roles
            .get(seat)
            .cloned()
            .unwrap_or(SeatRole::Unknown(ROLE_NOT_RECORDED))
    };
    let mut chains: BTreeMap<&str, Vec<&HandoffEdge>> = BTreeMap::new();
    for edge in edges {
        chains.entry(edge.chain_key()).or_default().push(edge);
    }
    let mut result = M3 {
        rework: Vec::new(),
        unknown: Vec::new(),
        chains: 0,
    };
    for (chain, group) in &chains {
        let mut path = vec![group[0].from_seat.clone()];
        let mut checks: Vec<(String, Vec<String>)> = Vec::new();
        let mut unknowns: Vec<(String, String)> = Vec::new();
        let mut unknown_return = None;
        let pass = |seat: &str,
                    checks: &mut Vec<(String, Vec<String>)>,
                    unknowns: &mut Vec<(String, String)>| {
            if is_build_seat(seat) {
                return;
            }
            match role_of(seat) {
                SeatRole::Check(evidence) => {
                    if !checks.iter().any(|(name, _)| name == seat) {
                        checks.push((seat.to_string(), evidence));
                    }
                }
                SeatRole::Other => {}
                SeatRole::Unknown(reason) => {
                    if !unknowns.iter().any(|(name, _)| name == seat) {
                        unknowns.push((seat.to_string(), reason.to_string()));
                    }
                }
            }
        };
        let mut rework = None;
        for edge in group {
            pass(&edge.from_seat, &mut checks, &mut unknowns);
            path.push(edge.to_seat.clone());
            if is_build_seat(&edge.to_seat) {
                if !checks.is_empty() {
                    rework = Some(Rework {
                        chain: chain.to_string(),
                        path: path.join(" -> "),
                        checks: checks.clone(),
                    });
                    break;
                }
                if !unknowns.is_empty() && unknown_return.is_none() {
                    unknown_return = Some(UnknownChain {
                        chain: chain.to_string(),
                        path: path.join(" -> "),
                        unknown_seats: unknowns.clone(),
                    });
                }
            }
            pass(&edge.to_seat, &mut checks, &mut unknowns);
        }
        match (rework, unknown_return) {
            (Some(rework), _) => {
                result.rework.push(rework);
                result.chains += 1;
            }
            (None, Some(unknown)) => result.unknown.push(unknown),
            (None, None) => result.chains += 1,
        }
    }
    result
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

    fn tagged(destination: &str, missions: &[&str]) -> DestinationSnapshot {
        DestinationSnapshot {
            destination: destination.into(),
            cutoff: "t".into(),
            reroute_marked: false,
            missions: missions.iter().map(|m| m.to_string()).collect(),
        }
    }

    fn claimed(item: &str, at: i64, seat: &str) -> QueueFact {
        let mut fact = q(
            &format!("events:queue.claimed:{item}{at}"),
            0,
            item,
            at,
            "in-progress",
            seat,
            seat,
            None,
        );
        fact.transition_id = 5_000 + at;
        fact
    }

    #[test]
    fn two_in_progress_items_with_different_missions_are_ambiguous() {
        let queue = vec![
            created("i1", 0, "dev.build"),
            claimed("i1", 1, "dev.build"),
            created("i2", 0, "dev.build"),
            claimed("i2", 2, "dev.build"),
        ];
        let snapshots = BTreeMap::from([
            ("i1".to_string(), vec![tagged("dev.build", &["one"])]),
            ("i2".to_string(), vec![tagged("dev.build", &["two"])]),
        ]);
        assert_eq!(
            mission_at("dev.build", &queue, &snapshots, ts(5)),
            MissionBucket::Ambiguous(AMBIGUOUS_CONFLICTING_MISSIONS)
        );
        // Same mission on both: that mission.
        let same = BTreeMap::from([
            ("i1".to_string(), vec![tagged("dev.build", &["one"])]),
            ("i2".to_string(), vec![tagged("dev.build", &["one"])]),
        ]);
        assert_eq!(
            mission_at("dev.build", &queue, &same, ts(5)),
            MissionBucket::Mission("one".into())
        );
    }

    #[test]
    fn k3_mission_follows_in_progress_intervals_and_destination() {
        let snapshots = BTreeMap::from([
            ("i1".to_string(), vec![tagged("dev.build", &["one"])]),
            (
                "two-tags".to_string(),
                vec![tagged("dev.build", &["one", "two"])],
            ),
            ("untagged".to_string(), vec![tagged("dev.build", &[])]),
        ]);
        let mut unclaim = q(
            "events:queue.unclaimed:9",
            0,
            "i1",
            20,
            "pending",
            "dev.build",
            "dev.build",
            None,
        );
        unclaim.transition_id = 6_020;
        let queue = vec![
            created("i1", 0, "dev.build"),
            claimed("i1", 10, "dev.build"),
            unclaim,
            claimed("i1", 30, "dev.build"),
            done("closed", 1, "i1", 40),
        ];
        let at = |t| mission_at("dev.build", &queue, &snapshots, ts(t));
        assert_eq!(at(5), MissionBucket::None, "pending, not in progress");
        assert_eq!(
            at(10),
            MissionBucket::Mission("one".into()),
            "claim applies at its instant"
        );
        assert_eq!(at(25), MissionBucket::None, "unclaim ends the interval");
        assert_eq!(at(35), MissionBucket::Mission("one".into()));
        assert_eq!(at(45), MissionBucket::None, "closed");
        // The actor of a transition is not the destination.
        let actor = vec![
            created("i1", 0, "dev.build"),
            q(
                "t-actor",
                1,
                "i1",
                5,
                "in-progress",
                "orch",
                "unknown",
                None,
            ),
        ];
        assert_eq!(
            mission_at("orch", &actor, &snapshots, ts(8)),
            MissionBucket::None
        );
        assert_eq!(
            mission_at("dev.build", &actor, &snapshots, ts(8)),
            MissionBucket::Mission("one".into())
        );
        let two = vec![
            created("two-tags", 0, "dev.build"),
            claimed("two-tags", 1, "dev.build"),
        ];
        assert_eq!(
            mission_at("dev.build", &two, &snapshots, ts(5)),
            MissionBucket::Ambiguous(AMBIGUOUS_CONFLICTING_MISSIONS),
            "one item with two mission tags"
        );
        let untagged = vec![
            created("untagged", 0, "dev.build"),
            claimed("untagged", 1, "dev.build"),
        ];
        assert_eq!(
            mission_at("dev.build", &untagged, &snapshots, ts(5)),
            MissionBucket::None,
            "an untagged item is no mission"
        );
    }

    #[test]
    fn k3_unprovable_history_or_tags_are_ambiguous_with_their_reason() {
        // No creation in history; the claim names the seat: insufficient history.
        let queue = vec![claimed("old", 1, "dev.build")];
        let snapshots = BTreeMap::from([("old".to_string(), vec![tagged("dev.build", &["one"])])]);
        assert_eq!(
            mission_at("dev.build", &queue, &snapshots, ts(5)),
            MissionBucket::Ambiguous(AMBIGUOUS_INSUFFICIENT_HISTORY)
        );
        assert_eq!(
            mission_at("someone-else", &queue, &snapshots, ts(5)),
            MissionBucket::None,
            "an unprovable item that names another seat does not touch this one"
        );
        // Owned, but no snapshot to read tags from.
        let no_tags = vec![
            created("bare", 0, "dev.build"),
            claimed("bare", 1, "dev.build"),
        ];
        assert_eq!(
            mission_at("dev.build", &no_tags, &BTreeMap::new(), ts(5)),
            MissionBucket::Ambiguous(AMBIGUOUS_INSUFFICIENT_HISTORY)
        );
        // Stored readings disagree on the mission tags.
        let changed = BTreeMap::from([(
            "bare".to_string(),
            vec![tagged("dev.build", &["one"]), tagged("dev.build", &["two"])],
        )]);
        assert_eq!(
            mission_at("dev.build", &no_tags, &changed, ts(5)),
            MissionBucket::Ambiguous(AMBIGUOUS_TAG_CHANGE)
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
                returned_by: None,
                ts: ts(0),
                source_key: "a".into(),
            },
            HandoffEdge {
                from_seat: "dev.qa".into(),
                to_seat: "rev-review".into(),
                qitem_id: "i".into(),
                chain_id: Some("c".into()),
                is_return: false,
                returned_by: None,
                ts: ts(0),
                source_key: "b".into(),
            },
            HandoffEdge {
                from_seat: "rev-review".into(),
                to_seat: "dev-build".into(),
                qitem_id: "i".into(),
                chain_id: Some("c".into()),
                is_return: true,
                returned_by: None,
                ts: ts(0),
                source_key: "c".into(),
            },
        ];
        let roles = BTreeMap::from([
            (
                "dev.qa".to_string(),
                SeatRole::Check(vec!["agentflow-seat:dev.qa".into()]),
            ),
            (
                "rev-review".to_string(),
                SeatRole::Check(vec!["agentflow-seat:rev.review".into()]),
            ),
        ]);
        let m3 = m3_rework(&edges, &roles);
        assert_eq!(m3.rework.len(), 1);
        assert_eq!(m3.chains, 1);
        assert_eq!(m3.rework[0].chain, "c");
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
        assert!(edges[0].returned_within(window));
        assert_eq!(edges[0].returned_by.as_ref().unwrap().0, "t3");
        assert!(!edges[1].returned_within(window));
        assert!(!edges[2].returned_within(window));
        // The handoff target is not the item's destination (K2).
        let snapshots = BTreeMap::from([(
            "item".to_string(),
            vec![DestinationSnapshot {
                destination: "A".into(),
                cutoff: "t".into(),
                reroute_marked: false,
                missions: Vec::new(),
            }],
        )]);
        // A snapshot without the creation in history proves nothing (K5).
        assert!(closed_items("A", &queue, &snapshots, window).is_empty());
        let mut with_history = queue.clone();
        with_history.push(created("item", -100, "A"));
        assert!(closed_items("B", &with_history, &snapshots, window).is_empty());
        let closed = closed_items("A", &with_history, &snapshots, window);
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

    fn snapshot(destination: &str, cutoff: &str, marked: bool) -> DestinationSnapshot {
        DestinationSnapshot {
            destination: destination.into(),
            cutoff: cutoff.into(),
            reroute_marked: marked,
            missions: Vec::new(),
        }
    }

    fn rerouted(item: &str, at: i64, from: &str, to: &str) -> QueueFact {
        let mut fact = q(
            &format!("events:qitem.fallback_routed:{item}{at}"),
            3_000 + at,
            item,
            at,
            "pending",
            from,
            to,
            None,
        );
        fact.transition_id = 3_000 + at;
        fact
    }

    fn done(key: &str, id: i64, item: &str, at: i64) -> QueueFact {
        q(key, id, item, at, "done", "actor", "unknown", None)
    }

    #[test]
    fn k5_destination_comes_from_created_and_reroute_history() {
        let window = Window {
            from: ts(0),
            to: ts(60),
        };
        let queue = vec![
            // Reroute after the closure: the closure keeps the created destination.
            created("a", -10, "seat-a"),
            done("closed-a", 1, "a", 10),
            rerouted("a", 20, "seat-a", "seat-b"),
            // Reroute before the closure: the closure follows the reroute.
            created("b", -10, "seat-a"),
            rerouted("b", 5, "seat-a", "seat-b"),
            done("closed-b", 2, "b", 10),
            // A claim only confirms; it never moves the destination.
            created("c", -10, "seat-a"),
            q(
                "events:queue.claimed:c",
                4_000,
                "c",
                5,
                "in-progress",
                "seat-b",
                "seat-b",
                None,
            ),
            done("closed-c", 3, "c", 10),
        ];
        // Snapshots show the current value; history wins over them.
        let snapshots = BTreeMap::from([
            ("a".to_string(), vec![snapshot("seat-b", "t9", true)]),
            ("b".to_string(), vec![snapshot("seat-b", "t9", true)]),
            ("c".to_string(), vec![snapshot("seat-a", "t9", false)]),
        ]);
        let a = closed_items("seat-a", &queue, &snapshots, window);
        assert_eq!(members(&a), vec!["closed-a", "closed-c"]);
        assert_eq!(a[0].destination_evidence, "events:queue.created:a");
        let b = closed_items("seat-b", &queue, &snapshots, window);
        assert_eq!(members(&b), vec!["closed-b"]);
        assert_eq!(b[0].destination_evidence, "events:qitem.fallback_routed:b5");
        assert!(k2_closures(&queue, &snapshots, window)
            .iter()
            .all(|(destination, _)| destination.is_some()));
    }

    /// A handoff on `item` of `chain`: the actor hands the work to `to`.
    fn hand(
        key: &str,
        at: i64,
        item: &str,
        chain: Option<&str>,
        actor: &str,
        to: &str,
    ) -> QueueFact {
        let mut fact = q(key, at, item, at, "done", actor, to, Some("handed_off_to"));
        fact.closure_target = Some(to.into());
        fact.chain_id = chain.map(str::to_string);
        fact
    }

    fn in_chain(mut fact: QueueFact, chain: &str) -> QueueFact {
        fact.chain_id = Some(chain.into());
        fact
    }

    fn edge<'a>(edges: &'a [HandoffEdge], key: &str) -> &'a HandoffEdge {
        edges.iter().find(|edge| edge.source_key == key).unwrap()
    }

    #[test]
    fn returns_follow_the_chain_across_items() {
        let window = Window {
            from: ts(0),
            to: ts(100),
        };
        // OpenRig: each handoff closes the item and opens the next one in the chain.
        let queue = vec![
            in_chain(created("i1", 0, "A"), "i1"),
            in_chain(created("i2", 9, "B"), "i1"), // successor creation, before its handoff
            hand("h1", 10, "i1", Some("i1"), "A", "B"),
            in_chain(created("i3", 19, "C"), "i1"),
            hand("h2", 20, "i2", Some("i1"), "B", "C"),
            in_chain(created("i4", 29, "A"), "i1"),
            hand("h3", 30, "i3", Some("i1"), "C", "A"), // indirect return to A
            in_chain(created("i5", 39, "C"), "i1"),
            hand("h4", 40, "i4", Some("i1"), "A", "C"), // direct return to C
        ];
        let edges = handoff_edges(&queue);
        assert!(
            !edge(&edges, "h1").is_return,
            "a successor creation is not a touch"
        );
        assert!(!edge(&edges, "h2").is_return);
        assert!(edge(&edges, "h3").is_return, "indirect A->B->C->A");
        assert!(edge(&edges, "h4").is_return, "direct C->A->C");
        // M1: deliveries answered by a later return to their sender, on any chain item.
        assert_eq!(edge(&edges, "h1").returned_by.as_ref().unwrap().0, "h3");
        assert_eq!(edge(&edges, "h2").returned_by, None, "B never gets it back");
        assert_eq!(edge(&edges, "h3").returned_by.as_ref().unwrap().0, "h4");
        assert!(edge(&edges, "h1").returned_within(window));
        // No ratio inflation: two returns among four handoffs, three of four
        // deliveries counted, with each set kept apart.
        assert_eq!(edges.iter().filter(|edge| edge.is_return).count(), 2);
        assert_eq!(
            edges
                .iter()
                .filter(|edge| edge.returned_within(window))
                .count(),
            2
        );
    }

    #[test]
    fn k7_m3_uses_recorded_roles_and_keeps_unknown_chains_apart() {
        let (build, build_sr, qa, review, orch, starter_qa, stranger) = (
            "dev-build@dendro-dev",
            "dev-build-sr@dendro-dev",
            "dev-qa@dendro-dev",
            "rev-review@dendro-dev",
            "orch-arch@dendro-dev",
            "dev-review@starter",
            "rev-review@unrecorded",
        );
        let roles = seat_roles(&[
            (qa.into(), "check".into(), "agentflow-seat:dev.qa".into()),
            (
                review.into(),
                "check".into(),
                "agentflow-seat:rev.review".into(),
            ),
            (
                orch.into(),
                "other".into(),
                "agentflow-seat:orch.arch".into(),
            ),
            (
                starter_qa.into(),
                "check".into(),
                "spec:local:agents/development/qa|qa".into(),
            ),
            ("split@rig".into(), "check".into(), "spec:a|qa".into()),
            (
                "split@rig".into(),
                "other".into(),
                "spec:b|implementer".into(),
            ),
        ]);
        // Each step is a new item of the chain, as OpenRig records handoffs.
        let chain = |name: &str, seats: &[&str]| -> Vec<QueueFact> {
            seats
                .windows(2)
                .enumerate()
                .map(|(step, pair)| {
                    hand(
                        &format!("{name}-{step}"),
                        step as i64 * 10,
                        &format!("{name}-item{step}"),
                        Some(name),
                        pair[0],
                        pair[1],
                    )
                })
                .collect()
        };
        let mut queue = Vec::new();
        queue.extend(chain("direct", &[build, qa, build]));
        queue.extend(chain("via-review", &[build, qa, review, build_sr]));
        queue.extend(chain("via-orch", &[build, qa, orch, build]));
        queue.extend(chain("sender-qualifies", &[orch, qa, build])); // qa sends the return
        queue.extend(chain(
            "starter-qa",
            &["dev-build@starter", starter_qa, "dev-build@starter"],
        ));
        queue.extend(chain("no-check", &[build, orch, build]));
        queue.extend(chain("check-after", &[build, orch, build, qa]));
        queue.extend(chain("unknown-only", &[build, stranger, build]));
        queue.extend(chain("conflict-only", &[build, "split@rig", build]));
        queue.extend(chain(
            "unknown-then-check",
            &[build, stranger, build, qa, build],
        ));
        queue.extend(chain("unknown-no-return", &[build, stranger, orch]));
        let edges = handoff_edges(&queue);
        let m3 = m3_rework(&edges, &roles);
        let rework: Vec<&str> = m3.rework.iter().map(|r| r.chain.as_str()).collect();
        assert_eq!(
            rework,
            vec![
                "direct",
                "sender-qualifies",
                "starter-qa",
                "unknown-then-check",
                "via-orch",
                "via-review"
            ]
        );
        let unknown: Vec<(&str, &str)> = m3
            .unknown
            .iter()
            .map(|u| (u.chain.as_str(), u.unknown_seats[0].1.as_str()))
            .collect();
        assert_eq!(
            unknown,
            vec![
                ("conflict-only", ROLE_CONFLICTING),
                ("unknown-only", ROLE_NOT_RECORDED)
            ]
        );
        // Denominator: 11 chains minus 2 unknown.
        assert_eq!(m3.chains, 9);
        let starter = m3.rework.iter().find(|r| r.chain == "starter-qa").unwrap();
        assert_eq!(
            starter.checks,
            vec![(
                starter_qa.to_string(),
                vec!["spec:local:agents/development/qa|qa".to_string()]
            )],
            "a QA spec, not the name, makes dev-review@starter a check"
        );
        assert_eq!(
            m3.rework.iter().find(|r| r.chain == "via-review").unwrap().path,
            "dev-build@dendro-dev -> dev-qa@dendro-dev -> rev-review@dendro-dev -> dev-build-sr@dendro-dev"
        );
        // A name that looks like a review seat is not evidence.
        assert_eq!(roles.get(stranger), None);
        assert!(is_build_seat("dev.build-jr") && !is_build_seat("orch-arch"));
    }

    #[test]
    fn an_unavailable_chain_finds_returns_within_the_item_only() {
        let queue = vec![
            hand("h1", 10, "i1", None, "A", "B"),
            hand("h2", 20, "i2", None, "B", "A"),
        ];
        let edges = handoff_edges(&queue);
        assert!(
            !edge(&edges, "h2").is_return,
            "different items, no chain: no guess"
        );
        assert_eq!(edge(&edges, "h1").returned_by, None);
        let same_item = vec![
            hand("h1", 10, "i1", None, "A", "B"),
            hand("h2", 20, "i1", None, "B", "A"),
        ];
        let edges = handoff_edges(&same_item);
        assert!(edge(&edges, "h2").is_return);
    }

    #[test]
    fn k6_m1_counts_a_return_only_inside_the_same_window() {
        let queue = vec![
            hand("out", 10, "i1", Some("c"), "A", "B"),
            hand("back", 60, "i2", Some("c"), "B", "A"),
        ];
        let edges = handoff_edges(&queue);
        let delivery = edge(&edges, "out");
        let at = |from: i64, to: i64| Window {
            from: ts(from),
            to: ts(to),
        };
        assert!(delivery.returned_within(at(0, 61)), "return before to");
        assert!(
            !delivery.returned_within(at(0, 60)),
            "return exactly at to is outside"
        );
        assert!(
            !delivery.returned_within(at(0, 30)),
            "return after to, before the cutoff"
        );
        assert!(
            delivery.returned_within(at(10, 61)),
            "delivery exactly at from"
        );
    }

    #[test]
    fn the_wait_and_intake_claim_is_on_the_next_chain_item() {
        let mut claim = in_chain(
            q(
                "events:queue.claimed:7",
                7,
                "i2",
                25,
                "in-progress",
                "B",
                "B",
                None,
            ),
            "c",
        );
        claim.transition_id = 4_007;
        let queue = vec![
            hand("h1", 10, "i1", Some("c"), "A", "B"),
            claim.clone(),
            in_chain(
                q(
                    "events:queue.claimed:8",
                    8,
                    "other",
                    15,
                    "in-progress",
                    "B",
                    "B",
                    None,
                ),
                "elsewhere",
            ),
        ];
        let edges = handoff_edges(&queue);
        let medians = pair_medians(&edges, &queue, &[]);
        assert_eq!(medians[0].wait_samples, vec![15.0]);
        assert_eq!(
            medians[0].wait_sample_keys,
            vec![("h1".to_string(), "events:queue.claimed:7".to_string())]
        );
        assert_eq!(medians[0].wait_median, Some(15.0));
        assert_eq!(
            receiver_claim(&queue, &edges[0]).map(|row| row.source_key.as_str()),
            Some(claim.source_key.as_str())
        );
    }

    #[test]
    fn k5_untimed_mark_blocks_but_a_timed_reroute_after_the_closure_does_not() {
        let window = Window {
            from: ts(0),
            to: ts(60),
        };
        // Same closure time, same marked snapshot; only the reroute event differs.
        let marked = BTreeMap::from([
            ("u".to_string(), vec![snapshot("seat-b", "t9", true)]),
            ("t".to_string(), vec![snapshot("seat-b", "t9", true)]),
        ]);
        let queue = vec![
            created("u", -10, "seat-a"),
            done("untimed", 1, "u", 10),
            created("t", -10, "seat-a"),
            done("timed", 2, "t", 10),
            rerouted("t", 20, "seat-a", "seat-b"),
        ];
        let closures = k2_closures(&queue, &marked, window);
        assert_eq!(closures[0].0, None);
        assert_eq!(closures[0].1.destination_evidence, UNTIMED_REROUTE_MARK);
        assert_eq!(closures[1].0.as_deref(), Some("seat-a"));
        assert_eq!(closures[1].1.destination_evidence, "events:queue.created:t");
        // Disagreeing snapshots are likewise resolved by a timed reroute event.
        let drifted = BTreeMap::from([(
            "t".to_string(),
            vec![
                snapshot("seat-a", "t1", false),
                snapshot("seat-b", "t2", false),
            ],
        )]);
        let resolved = k2_closures(&queue[2..], &drifted, window);
        assert_eq!(resolved[0].0.as_deref(), Some("seat-a"));
    }

    #[test]
    fn k5_without_proof_of_no_change_is_insufficient_history() {
        let window = Window {
            from: ts(0),
            to: ts(60),
        };
        let queue = vec![
            // Creation outside the history: the snapshot alone is not proof.
            done("no-created", 1, "n", 10),
            // A reroute mark (note or chain) without a reroute event.
            created("m", -10, "seat-a"),
            done("marked", 2, "m", 10),
            // Two stored snapshots that disagree prove a change.
            created("d", -10, "seat-a"),
            done("drifted", 3, "d", 10),
            // Control: creation in history, no mark, the snapshot agrees.
            created("k", -10, "seat-a"),
            done("clean", 4, "k", 10),
        ];
        let snapshots = BTreeMap::from([
            ("n".to_string(), vec![snapshot("seat-a", "t1", false)]),
            ("m".to_string(), vec![snapshot("seat-a", "t1", true)]),
            (
                "d".to_string(),
                vec![
                    snapshot("seat-a", "t1", false),
                    snapshot("seat-b", "t2", false),
                ],
            ),
            ("k".to_string(), vec![snapshot("seat-a", "t1", false)]),
        ]);
        let closures = k2_closures(&queue, &snapshots, window);
        let insufficient: Vec<&str> = closures
            .iter()
            .filter(|(destination, _)| destination.is_none())
            .map(|(_, item)| item.transition.as_str())
            .collect();
        assert_eq!(insufficient, vec!["no-created", "marked", "drifted"]);
        let reasons: Vec<&str> = closures
            .iter()
            .filter(|(destination, _)| destination.is_none())
            .map(|(_, item)| item.destination_evidence.as_str())
            .collect();
        assert_eq!(
            reasons,
            vec![
                NO_CREATION_IN_HISTORY,
                UNTIMED_REROUTE_MARK,
                CONFLICTING_SNAPSHOTS
            ]
        );
        assert!(reasons
            .iter()
            .all(|reason| reason.starts_with(INSUFFICIENT_HISTORY)));
        for seat in ["seat-a", "seat-b", "actor", "unknown"] {
            let seat_members = members(&closed_items(seat, &queue, &snapshots, window))
                .into_iter()
                .map(str::to_string)
                .collect::<Vec<_>>();
            for key in &insufficient {
                assert!(!seat_members.contains(&key.to_string()), "{seat} {key}");
            }
        }
        assert_eq!(
            members(&closed_items("seat-a", &queue, &snapshots, window)),
            vec!["clean"]
        );
    }
}
