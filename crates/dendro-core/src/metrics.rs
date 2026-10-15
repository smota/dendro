use std::collections::{BTreeMap, BTreeSet};

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
            // Whole seconds counted from the span start, so the segments sum
            // to exactly the span's observed seconds (AC4 conservation);
            // truncating each segment on its own loses the fractions.
            let secs = (b - span.start).num_seconds() - (a - span.start).num_seconds();
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

/// The precedence of ambiguity reasons, printed verbatim in the report and
/// the trace (K3/K5, AC7).
pub const MISSION_DEFINITION: &str = "Mission attribution (K3/K5): a usage record gets the mission of the items its seat has in progress at that instant. An item is the seat's when its K5 destination then is the seat; the actor is never the destination. Missions are the item's mission: tags as set at creation. Every applicable ambiguity reason is recorded; the primary reason is the first that applies in this fixed order: insufficient history > detectable tag change > conflicting missions. Only the primary reason is counted in the ambiguous shares, so the shares sum exactly to the ambiguous band; secondary reasons are evidence only and overlap.";

/// One in-progress item behind a usage record's attribution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MissionItem {
    pub qitem: String,
    /// The seat's own item (`owned`) or an unprovable item naming the seat.
    pub relation: &'static str,
    /// K5 destination evidence, or the reason it cannot be proven.
    pub destination_evidence: String,
    /// Mission tags of each stored reading, oldest first.
    pub tag_readings: Vec<Vec<String>>,
}

/// The full attribution of one usage record: bucket, every applicable reason
/// in precedence order (the first is primary), and the items behind it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissionEvidence {
    pub bucket: MissionBucket,
    pub reasons: Vec<&'static str>,
    pub items: Vec<MissionItem>,
}

/// K3 mission attribution of a usage record of `seat` at `ts`, with every
/// applicable reason computed before the primary is chosen.
pub fn mission_evidence(
    seat: &str,
    queue: &[QueueFact],
    snapshots: &BTreeMap<String, Vec<DestinationSnapshot>>,
    ts: DateTime<Utc>,
) -> MissionEvidence {
    let mut by_item: BTreeMap<&str, Vec<&QueueFact>> = BTreeMap::new();
    for fact in queue {
        by_item
            .entry(fact.qitem_id.as_str())
            .or_default()
            .push(fact);
    }
    let readings_of = |qitem: &str| -> Vec<Vec<String>> {
        snapshots
            .get(qitem)
            .map(|rows| rows.iter().map(|row| row.missions.clone()).collect())
            .unwrap_or_default()
    };
    let (mut insufficient, mut tag_change, mut conflicting) = (false, false, false);
    let mut items = Vec::new();
    let mut missions = Vec::new();
    for (qitem, facts) in &by_item {
        let Some(state) = item_state_at(facts, ts) else {
            continue;
        };
        if state.state != "in-progress" {
            continue;
        }
        let mut probe = state.clone();
        probe.ts = ts;
        match destination_at(&probe, queue, snapshots) {
            Ok((destination, evidence, _)) if destination == seat => {
                let tag_readings = readings_of(qitem);
                match tag_readings.last() {
                    None => insufficient = true,
                    Some(latest) => {
                        tag_change |= tag_readings.windows(2).any(|pair| pair[0] != pair[1]);
                        conflicting |= latest.len() > 1;
                        missions.extend(latest.iter().cloned());
                    }
                }
                items.push(MissionItem {
                    qitem: qitem.to_string(),
                    relation: "owned",
                    destination_evidence: evidence,
                    tag_readings,
                });
            }
            Ok(_) => {}
            Err(reason) => {
                let named = facts.iter().any(|row| {
                    row.source_key.starts_with("events:queue.claimed:")
                        && row.dest_seat == seat
                        && row.ts <= ts
                }) || snapshots
                    .get(*qitem)
                    .and_then(|rows| rows.last())
                    .is_some_and(|snapshot| snapshot.destination == seat);
                if named {
                    insufficient = true;
                    items.push(MissionItem {
                        qitem: qitem.to_string(),
                        relation: "unprovable item naming this seat",
                        destination_evidence: reason.to_string(),
                        tag_readings: readings_of(qitem),
                    });
                }
            }
        }
    }
    missions.sort();
    missions.dedup();
    conflicting |= missions.len() > 1;
    let reasons: Vec<&'static str> = [
        (insufficient, AMBIGUOUS_INSUFFICIENT_HISTORY),
        (tag_change, AMBIGUOUS_TAG_CHANGE),
        (conflicting, AMBIGUOUS_CONFLICTING_MISSIONS),
    ]
    .into_iter()
    .filter_map(|(applies, reason)| applies.then_some(reason))
    .collect();
    let bucket = match (reasons.first(), missions.len()) {
        (Some(primary), _) => MissionBucket::Ambiguous(primary),
        (None, 1) => MissionBucket::Mission(missions.remove(0)),
        (None, _) => MissionBucket::None,
    };
    MissionEvidence {
        bucket,
        reasons,
        items,
    }
}

/// The bucket of [`mission_evidence`].
pub fn mission_at(
    seat: &str,
    queue: &[QueueFact],
    snapshots: &BTreeMap<String, Vec<DestinationSnapshot>>,
    ts: DateTime<Utc>,
) -> MissionBucket {
    mission_evidence(seat, queue, snapshots, ts).bucket
}

/// How one harness session is bound to a seat. One rule everywhere (links,
/// tokens, missions, intakes, rig scope): only identity events with the exact
/// harness and native session count. Events that agree on (rig, seat) are one
/// binding; events that disagree are a collision and never link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionBinding {
    Linked { rig: String, seat: String },
    Unlinked,
    Collision,
}

impl SessionBinding {
    pub fn status(&self) -> &'static str {
        match self {
            SessionBinding::Linked { .. } => "linked",
            SessionBinding::Unlinked => "unlinked",
            SessionBinding::Collision => "collision",
        }
    }

    pub fn seat(&self) -> Option<&str> {
        match self {
            SessionBinding::Linked { seat, .. } => Some(seat),
            _ => None,
        }
    }

    pub fn rig(&self) -> Option<&str> {
        match self {
            SessionBinding::Linked { rig, .. } => Some(rig),
            _ => None,
        }
    }
}

/// Every (harness, native session) named by an identity event, with its binding.
/// A session absent from the map is unlinked.
pub fn session_bindings(identities: &[IdentityFact]) -> BTreeMap<(String, String), SessionBinding> {
    let mut seen = BTreeMap::<(String, String), BTreeSet<(String, String)>>::new();
    for fact in identities {
        seen.entry((fact.harness.clone(), fact.native_session.clone()))
            .or_default()
            .insert((fact.rig.clone(), fact.seat.clone()));
    }
    seen.into_iter()
        .map(|(session, bound)| {
            let binding = if bound.len() == 1 {
                let (rig, seat) = bound.into_iter().next().unwrap();
                SessionBinding::Linked { rig, seat }
            } else {
                SessionBinding::Collision
            };
            (session, binding)
        })
        .collect()
}

pub fn binding_of<'a>(
    bindings: &'a BTreeMap<(String, String), SessionBinding>,
    harness: &str,
    native: &str,
) -> &'a SessionBinding {
    bindings
        .get(&(harness.to_string(), native.to_string()))
        .unwrap_or(&SessionBinding::Unlinked)
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

/// `linked_seats`: seats with at least one session singly linked to them by
/// the binding rule (F2 table row 3), whether or not it used tokens.
pub fn handoff_intakes(
    edges: &[HandoffEdge],
    queue: &[QueueFact],
    tools: &[ToolFact],
    usages: &[UsageFact],
    linked_seats: &BTreeSet<String>,
    seat_of: impl Fn(&str, &str) -> Option<String>,
) -> Vec<IntakeRow> {
    edges
        .iter()
        .map(|edge| intake_for_edge(edge, queue, tools, usages, linked_seats, &seat_of))
        .collect()
}

fn is_transition_row(fact: &QueueFact) -> bool {
    fact.source_key.starts_with("queue_transitions:")
}

/// The receiver's own claim row in `queue_transitions` for this claim, which
/// the `queue.claimed` event only identifies (F2 table): the receiver's
/// in-progress transition on the claimed item at exactly the event's time,
/// else the only one in the event's whole second. None means an events-only
/// claim; two or more candidates without a unique exact match are ambiguous.
enum ClaimRow<'a> {
    Row(&'a QueueFact),
    EventsOnly,
    Ambiguous,
}

fn claim_row<'a>(queue: &'a [QueueFact], claim: &'a QueueFact, receiver: &str) -> ClaimRow<'a> {
    if is_transition_row(claim) {
        return ClaimRow::Row(claim);
    }
    let same_second: Vec<&QueueFact> = queue
        .iter()
        .filter(|row| {
            is_transition_row(row)
                && row.qitem_id == claim.qitem_id
                && row.source_seat == receiver
                && row.state == "in-progress"
                && row.ts.timestamp() == claim.ts.timestamp()
        })
        .collect();
    let exact: Vec<&QueueFact> = same_second
        .iter()
        .copied()
        .filter(|row| row.ts == claim.ts)
        .collect();
    match (exact.as_slice(), same_second.as_slice()) {
        ([row], _) | ([], [row]) => ClaimRow::Row(row),
        (_, []) => ClaimRow::EventsOnly,
        _ => ClaimRow::Ambiguous,
    }
}

/// Intake (§4, frozen F2 table 4acf5390): from the claim, inclusive, to the
/// receiver's first write action, exclusive. A write action is a receiver
/// transition on the claimed item after the claim row (same table: time, then
/// key), or a structured edit or write in the receiver's linked session at or
/// after the start. Other actors, the claim itself and bash never end it.
fn intake_for_edge(
    edge: &HandoffEdge,
    queue: &[QueueFact],
    tools: &[ToolFact],
    usages: &[UsageFact],
    linked_seats: &BTreeSet<String>,
    seat_of: &impl Fn(&str, &str) -> Option<String>,
) -> IntakeRow {
    let Some(claim) = receiver_claim(queue, edge) else {
        return intake_row(edge, "no_data", Some("no claim"), None, None, None, vec![]);
    };
    let receiver = edge.to_seat.as_str();
    let own_row = match claim_row(queue, claim, receiver) {
        ClaimRow::Row(row) => Some(row),
        ClaimRow::EventsOnly => None,
        ClaimRow::Ambiguous => {
            return intake_row(
                edge,
                "ambiguous",
                Some("ambiguous claim row"),
                None,
                Some(claim.ts.to_rfc3339()),
                None,
                vec![],
            )
        }
    };
    let start = own_row.map_or(claim.ts, |row| row.ts);
    if !linked_seats.contains(receiver) {
        return intake_row(
            edge,
            "no_data",
            Some("no linked session"),
            None,
            Some(start.to_rfc3339()),
            None,
            vec![],
        );
    }
    let queue_end = queue
        .iter()
        .filter(|row| {
            is_transition_row(row)
                && row.qitem_id == claim.qitem_id
                && row.source_seat == receiver
                && match own_row {
                    Some(own) => (row.ts, row.transition_id) > (own.ts, own.transition_id),
                    None => row.ts >= start,
                }
        })
        .map(|row| row.ts)
        .min();
    let receiver_writes = || {
        tools.iter().filter(|tool| {
            tool.structured_write
                && seat_of(&tool.harness, &tool.native_session).as_deref() == Some(receiver)
        })
    };
    let tool_end = receiver_writes()
        .filter_map(|tool| tool.ts)
        .filter(|ts| *ts >= start)
        .min();
    let end = match (queue_end, tool_end) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    };
    let Some(end) = end else {
        // A receiver write without a time cannot be placed in the window.
        let reason = if receiver_writes().any(|tool| tool.ts.is_none()) {
            "no timestamp"
        } else {
            "no classifiable end"
        };
        return intake_row(
            edge,
            "no_data",
            Some(reason),
            None,
            Some(start.to_rfc3339()),
            None,
            vec![],
        );
    };
    let overlap = queue.iter().any(|row| {
        row.qitem_id != claim.qitem_id
            && row.qitem_id != edge.qitem_id
            && row.dest_seat == edge.to_seat
            && row.state == "in-progress"
            && row.ts < end
            && !closed_before(queue, row, start, own_row)
    });
    if overlap {
        return intake_row(
            edge,
            "ambiguous",
            Some("overlapping claim"),
            None,
            Some(start.to_rfc3339()),
            Some(end.to_rfc3339()),
            vec![],
        );
    }
    // A linked receiver with a classifiable end and no usage in the window
    // is a zero estimate, not missing data.
    let mut tokens = 0i64;
    let mut samples = Vec::new();
    for usage in usages {
        let Some(ts) = usage.ts else { continue };
        if seat_of(&usage.harness, &usage.native_session).as_deref() != Some(receiver) {
            continue;
        }
        if ts >= start && ts < end {
            tokens += usage.input_uncached + usage.cache_read + usage.cache_write;
            samples.push(usage.source_key.clone());
        }
    }
    intake_row(
        edge,
        "estimate",
        None,
        Some(tokens),
        Some(start.to_rfc3339()),
        Some(end.to_rfc3339()),
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

/// Whether `a` comes strictly after `b`: later time, or the same time and a
/// later key within one table. A tie across tables has no order (F7).
fn strictly_after(a: &QueueFact, b: &QueueFact) -> bool {
    a.ts > b.ts
        || (a.ts == b.ts
            && is_transition_row(a) == is_transition_row(b)
            && a.transition_id > b.transition_id)
}

/// Whether the other item `open` was closed after it opened and before this
/// claim (F7, PM item 10): closing transition time against the claim row's
/// time, key order only within one table for ties. An item still open at the
/// claim is not closed, so the intake stays ambiguous.
fn closed_before(
    queue: &[QueueFact],
    open: &QueueFact,
    claim_at: DateTime<Utc>,
    claim_row: Option<&QueueFact>,
) -> bool {
    queue.iter().any(|row| {
        row.qitem_id == open.qitem_id
            && matches!(row.state.as_str(), "done" | "handed-off" | "canceled")
            && strictly_after(row, open)
            && match claim_row {
                Some(claim) => strictly_after(claim, row),
                None => row.ts < claim_at,
            }
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
                .push((claim.ts - handoff_at).as_seconds_f64());
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
    pub harness: String,
    pub session: String,
    pub tool: String,
    pub count: usize,
}

/// Tool loops (§4 rules 1–6, K11). A run is consecutive failures of one tool
/// with the same trimmed input in one session (harness, session). It is a
/// loop when any 3 consecutive failures in it span at most `window_secs`
/// exactly, from their 1st to their 3rd; the whole run is one episode with
/// its full count. A three whose 1st or 3rd failure has no timestamp cannot
/// qualify, though the failure stays in the run.
pub fn tool_loops(tools: &[ToolFact], failures: usize, window_secs: i64) -> Vec<ToolLoop> {
    evaluate_tool_loops(tools, failures, window_secs)
        .episodes
        .into_iter()
        .map(|e| e.projection)
        .collect()
}

#[derive(Debug, Clone)]
pub(crate) struct LoopEpisode {
    pub projection: ToolLoop,
    pub members: Vec<usize>,
    pub witness: Vec<usize>,
}

#[derive(Debug, Default)]
pub(crate) struct LoopEvaluation {
    pub order: Vec<usize>,
    pub episodes: Vec<LoopEpisode>,
}

pub(crate) fn evaluate_tool_loops(
    tools: &[ToolFact],
    failures: usize,
    window_secs: i64,
) -> LoopEvaluation {
    let mut by_session: BTreeMap<(String, String), Vec<usize>> = BTreeMap::new();
    for (i, tool) in tools.iter().enumerate() {
        by_session
            .entry((tool.harness.clone(), tool.native_session.clone()))
            .or_default()
            .push(i);
    }
    let window = chrono::Duration::seconds(window_secs);
    let mut result = LoopEvaluation::default();
    for ((harness, session), mut events) in by_session {
        events.sort_by_key(|&i| tools[i].seq);
        result.order.extend_from_slice(&events);
        let mut run: Vec<usize> = Vec::new();
        let flush = |run: &mut Vec<usize>, result: &mut LoopEvaluation| {
            let witness = if failures > 0 {
                run.windows(failures).find(|three| {
                    match (tools[three[0]].ts, tools[*three.last().unwrap()].ts) {
                        (Some(start), Some(end)) => end - start <= window,
                        _ => false,
                    }
                })
            } else {
                None
            };
            if let Some(witness) = witness {
                result.episodes.push(LoopEpisode {
                    projection: ToolLoop {
                        harness: harness.clone(),
                        session: session.clone(),
                        tool: tools[run[0]].name.clone(),
                        count: run.len(),
                    },
                    members: run.clone(),
                    witness: witness.to_vec(),
                });
            }
            run.clear();
        };
        for i in events {
            let event = &tools[i];
            let continues = event.failed == Some(true)
                && event.fingerprint.is_some()
                && run.last().is_some_and(|&prev| {
                    tools[prev].name == event.name && tools[prev].fingerprint == event.fingerprint
                });
            if continues {
                run.push(i);
            } else {
                flush(&mut run, &mut result);
                if event.failed == Some(true) {
                    run.push(i);
                }
            }
        }
        flush(&mut run, &mut result);
    }
    result
}

/// Codex source order has untimed anchors. Timed segments sort independently.
pub fn codex_command_loops(observations: &[CodexObservation]) -> Vec<ToolLoop> {
    evaluate_codex_loops(observations)
        .episodes
        .into_iter()
        .map(|e| e.projection)
        .collect()
}

pub(crate) fn evaluate_codex_loops(observations: &[CodexObservation]) -> LoopEvaluation {
    let mut sessions: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, row) in observations.iter().enumerate() {
        sessions
            .entry(&row.fact.native_session)
            .or_default()
            .push(i);
    }
    let mut result = LoopEvaluation::default();
    for (session, mut rows) in sessions {
        let native = |i: usize| {
            let row = &observations[i];
            (&row.source_id, row.fact.generation, row.fact.seq)
        };
        rows.sort_by(|&a, &b| native(a).cmp(&native(b)));
        let mut start = 0;
        for end in 0..=rows.len() {
            if end == rows.len() || observations[rows[end]].fact.ts.is_none() {
                rows[start..end].sort_by(|&a, &b| {
                    observations[a]
                        .fact
                        .ts
                        .cmp(&observations[b].fact.ts)
                        .then_with(|| native(a).cmp(&native(b)))
                });
                start = end + 1;
            }
        }
        result.order.extend_from_slice(&rows);
        let mut run: Vec<usize> = Vec::new();
        let flush = |run: &mut Vec<usize>, result: &mut LoopEvaluation| {
            let witness = run.windows(3).find(|triple| {
                match (
                    observations[triple[0]].fact.ts,
                    observations[triple[2]].fact.ts,
                ) {
                    (Some(a), Some(b)) => b >= a && b - a <= chrono::Duration::seconds(600),
                    _ => false,
                }
            });
            if let Some(witness) = witness {
                result.episodes.push(LoopEpisode {
                    projection: ToolLoop {
                        harness: "codex".into(),
                        session: session.into(),
                        tool: CODEX_COMMAND.into(),
                        count: run.len(),
                    },
                    members: run.clone(),
                    witness: witness.to_vec(),
                });
            }
            run.clear();
        };
        for i in rows {
            let fact = &observations[i].fact;
            let eligible = fact.name == CODEX_COMMAND
                && fact.failed == Some(true)
                && fact.fingerprint.is_some();
            if !eligible
                || run
                    .last()
                    .is_some_and(|&last| observations[last].fact.fingerprint != fact.fingerprint)
            {
                flush(&mut run, &mut result);
            }
            if eligible {
                run.push(i);
            }
        }
        flush(&mut run, &mut result);
    }
    result
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

    fn at_ms(ms: i64) -> DateTime<Utc> {
        ts(0) + chrono::Duration::milliseconds(ms)
    }

    fn state_at_ms(ms: i64, state: &str, seq: i64) -> ActivityFact {
        ActivityFact {
            source_key: format!("f{seq}"),
            seat: "dev.build".into(),
            ts: at_ms(ms),
            state: state.into(),
            seq,
        }
    }

    #[test]
    fn fractional_marks_conserve_every_observed_second_across_lives() {
        // Two lives, [0.4, 30.9) and [40.25, 60): 30 + 19 observed seconds.
        let lives = [
            SeatBound {
                seat: "dev.build".into(),
                created_at: Some(at_ms(400)),
                stopped_at: Some(at_ms(30_900)),
            },
            SeatBound {
                seat: "dev.build".into(),
                created_at: Some(at_ms(40_250)),
                stopped_at: None,
            },
        ];
        let events = [
            state_at_ms(700, "running", 1),
            state_at_ms(5_600, "idle", 2),
            state_at_ms(12_300, "needs_input", 3),
            state_at_ms(41_100, "running", 4),
            state_at_ms(50_750, "idle", 5),
        ];
        let spans = observation_spans(window(), &lives);
        assert_eq!(observed_secs(&spans), 30 + 19);
        let bands = activity_bands("dev.build", &events, &[], window(), &lives);
        // Seconds are whole offsets from each span start: 0.4 + {0, 5, 11, 30}
        // and 40.25 + {0, 10, 19}. Truncating each segment alone gave 4, 6,
        // 18, 9 and 9: three seconds short.
        assert_eq!(bands.active_secs, 5 + 10);
        assert_eq!(bands.idle_without_work_secs, 6 + 9);
        assert_eq!(bands.waiting_human_secs, 19);
        assert_eq!(bands.unknown_secs, 0);
        assert_eq!(bands.sum(), observed_secs(&spans));
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
    fn every_reason_is_recorded_and_the_primary_follows_the_fixed_order() {
        // One instant with all three reasons: an unprovable claimed item
        // (insufficient history), an owned item whose tag readings changed
        // (tag change) and an owned item with two mission tags (conflicting).
        let mut queue = vec![
            claimed("old", 1, "dev.build"),
            created("drift", 0, "dev.build"),
            claimed("drift", 1, "dev.build"),
            created("two", 0, "dev.build"),
            claimed("two", 1, "dev.build"),
        ];
        let snapshots = BTreeMap::from([
            ("old".to_string(), vec![tagged("dev.build", &["a"])]),
            (
                "drift".to_string(),
                vec![tagged("dev.build", &["a"]), tagged("dev.build", &["b"])],
            ),
            ("two".to_string(), vec![tagged("dev.build", &["a", "c"])]),
        ]);
        let all = mission_evidence("dev.build", &queue, &snapshots, ts(5));
        assert_eq!(
            all.reasons,
            vec![
                AMBIGUOUS_INSUFFICIENT_HISTORY,
                AMBIGUOUS_TAG_CHANGE,
                AMBIGUOUS_CONFLICTING_MISSIONS
            ]
        );
        assert_eq!(
            all.bucket,
            MissionBucket::Ambiguous(AMBIGUOUS_INSUFFICIENT_HISTORY)
        );
        assert_eq!(all.items.len(), 3);
        let item = |id: &str| all.items.iter().find(|item| item.qitem == id).unwrap();
        assert_eq!(item("old").relation, "unprovable item naming this seat");
        assert_eq!(item("drift").relation, "owned");
        assert_eq!(
            item("drift").tag_readings,
            vec![vec!["a".to_string()], vec!["b".to_string()]]
        );
        // Evidence order does not change the result.
        queue.reverse();
        assert_eq!(
            mission_evidence("dev.build", &queue, &snapshots, ts(5)),
            all
        );
        // Without the unprovable item, tag change is primary and conflict secondary.
        let rest: Vec<QueueFact> = queue
            .iter()
            .filter(|f| f.qitem_id != "old")
            .cloned()
            .collect();
        let two = mission_evidence("dev.build", &rest, &snapshots, ts(5));
        assert_eq!(
            two.reasons,
            vec![AMBIGUOUS_TAG_CHANGE, AMBIGUOUS_CONFLICTING_MISSIONS]
        );
        assert_eq!(two.bucket, MissionBucket::Ambiguous(AMBIGUOUS_TAG_CHANGE));
        // Each reason alone.
        let alone = |items: &[&str]| {
            let only: Vec<QueueFact> = queue
                .iter()
                .filter(|f| items.contains(&f.qitem_id.as_str()))
                .cloned()
                .collect();
            mission_evidence("dev.build", &only, &snapshots, ts(5)).reasons
        };
        assert_eq!(alone(&["old"]), vec![AMBIGUOUS_INSUFFICIENT_HISTORY]);
        assert_eq!(alone(&["drift"]), vec![AMBIGUOUS_TAG_CHANGE]);
        assert_eq!(alone(&["two"]), vec![AMBIGUOUS_CONFLICTING_MISSIONS]);
        assert!(MISSION_DEFINITION
            .contains("insufficient history > detectable tag change > conflicting missions"));
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

    /// One failing run at these offsets in milliseconds; None has no timestamp.
    fn failing_run(harness: &str, session: &str, offsets_ms: &[Option<i64>]) -> Vec<ToolFact> {
        offsets_ms
            .iter()
            .enumerate()
            .map(|(seq, offset)| ToolFact {
                source_key: format!("{harness}{session}{seq}"),
                harness: harness.into(),
                native_session: session.into(),
                ts: offset.map(at_ms),
                seq: seq as i64,
                name: "Read".into(),
                fingerprint: Some("fp".into()),
                failed: Some(true),
                structured_write: false,
                generation: 1,
            })
            .collect()
    }

    fn loop_counts(offsets_ms: &[Option<i64>]) -> Vec<usize> {
        tool_loops(&failing_run("claude", "s", offsets_ms), 3, 600)
            .iter()
            .map(|found| found.count)
            .collect()
    }

    #[test]
    fn a_loop_needs_some_three_within_exactly_ten_minutes_k11() {
        let s = |secs: i64| Some(secs * 1000);
        // QA: a fractional excess over 600 s is not truncated away.
        assert_eq!(
            loop_counts(&[s(0), s(300), Some(600_001)]),
            Vec::<usize>::new()
        );
        assert_eq!(
            loop_counts(&[s(0), s(300), s(600)]),
            vec![3],
            "exactly 600 s counts"
        );
        // QA: first to last is not the window; the first three qualify.
        assert_eq!(loop_counts(&[s(0), s(300), s(600), s(1200)]), vec![4]);
        // K11: the window slides; earlier, slower failures stay in the count.
        assert_eq!(loop_counts(&[s(0), s(400), s(700), s(800)]), vec![4]);
        assert_eq!(
            loop_counts(&[s(0), s(400), s(700), s(1101)]),
            Vec::<usize>::new()
        );
        // A three whose 1st or 3rd failure has no timestamp cannot qualify.
        assert_eq!(loop_counts(&[None, s(1), s(2)]), Vec::<usize>::new());
        assert_eq!(loop_counts(&[s(0), s(1), None]), Vec::<usize>::new());
        // A missing middle time does not matter, and it stays in the count.
        assert_eq!(loop_counts(&[s(0), None, s(2)]), vec![3]);
        assert_eq!(loop_counts(&[None, s(0), s(1), s(2)]), vec![4]);
    }

    #[test]
    fn the_same_session_id_in_two_harnesses_is_two_sessions() {
        let s = |secs: i64| Some(secs * 1000);
        let mut tools = failing_run("claude", "same", &[s(0), s(1)]);
        tools.extend(failing_run("pi", "same", &[s(2)]));
        assert!(tool_loops(&tools, 3, 600).is_empty());
        tools.extend(failing_run("pi", "same", &[s(3), s(4)]));
        let loops = tool_loops(&tools, 3, 600);
        assert_eq!(loops.len(), 1);
        assert_eq!(loops[0].count, 3);
    }

    /// F2 table fixtures: build hands `root` to qa on successor `next`; times
    /// are milliseconds after t0 + 100 s ("S").
    struct IntakeCase {
        claim_event_ms: i64,
        claim_row: Option<(i64, i64)>,
        transitions: Vec<(i64, i64, &'static str)>,
        extra_claim_rows: Vec<(i64, i64)>,
        /// Another item for qa: its claim event time, and its closing row.
        other: Option<(i64, Option<(i64, i64)>)>,
        tools: Vec<(Option<i64>, bool)>,
        usage: Vec<(i64, i64)>,
        linked: bool,
    }

    impl IntakeCase {
        fn new() -> Self {
            IntakeCase {
                claim_event_ms: 200,
                claim_row: Some((200, 2)),
                transitions: vec![],
                extra_claim_rows: vec![],
                other: None,
                tools: vec![],
                usage: vec![],
                linked: true,
            }
        }

        fn run(&self) -> IntakeRow {
            let s = |ms: i64| at_ms(100_000 + ms);
            let mut queue = vec![q(
                "queue_transitions:1",
                1,
                "root",
                0,
                "handed-off",
                "build",
                "unknown",
                None,
            )];
            let mut claim = q(
                "events:queue.claimed:9",
                9,
                "next",
                0,
                "in-progress",
                "build",
                "qa",
                None,
            );
            claim.ts = s(self.claim_event_ms);
            claim.chain_id = Some("root".into());
            queue.push(claim);
            let rows = self
                .claim_row
                .iter()
                .chain(self.extra_claim_rows.iter())
                .map(|(ms, id)| (*ms, *id, "qa", "in-progress"));
            let ends = self
                .transitions
                .iter()
                .map(|(ms, id, actor)| (*ms, *id, *actor, "blocked"));
            for (ms, id, actor, state) in rows.chain(ends) {
                let mut row = q(
                    &format!("queue_transitions:{id}"),
                    id,
                    "next",
                    0,
                    state,
                    actor,
                    "unknown",
                    None,
                );
                row.ts = s(ms);
                row.chain_id = Some("root".into());
                queue.push(row);
            }
            if let Some((claimed, closed)) = self.other {
                let mut other = q(
                    "events:queue.claimed:8",
                    8_000,
                    "other",
                    0,
                    "in-progress",
                    "pm",
                    "qa",
                    None,
                );
                other.ts = s(claimed);
                queue.push(other);
                if let Some((ms, id)) = closed {
                    let mut close = q(
                        &format!("queue_transitions:{id}"),
                        id,
                        "other",
                        0,
                        "done",
                        "qa",
                        "unknown",
                        None,
                    );
                    close.ts = s(ms);
                    queue.push(close);
                }
            }
            let edge = HandoffEdge {
                from_seat: "build".into(),
                to_seat: "qa".into(),
                qitem_id: "root".into(),
                chain_id: Some("root".into()),
                is_return: false,
                returned_by: None,
                ts: ts(0),
                source_key: "queue_transitions:1".into(),
            };
            let tools: Vec<ToolFact> = self
                .tools
                .iter()
                .enumerate()
                .map(|(seq, (ms, structured))| ToolFact {
                    source_key: format!("tool{seq}"),
                    harness: "claude".into(),
                    native_session: "qa-session".into(),
                    ts: ms.map(s),
                    seq: seq as i64,
                    name: if *structured { "Write" } else { "Bash" }.into(),
                    fingerprint: None,
                    failed: Some(false),
                    structured_write: *structured,
                    generation: 1,
                })
                .collect();
            let usages: Vec<UsageFact> = self
                .usage
                .iter()
                .enumerate()
                .map(|(seq, (ms, input))| UsageFact {
                    source_key: format!("u{seq}"),
                    harness: "claude".into(),
                    native_session: "qa-session".into(),
                    message_id: None,
                    ts: Some(s(*ms)),
                    seq: seq as i64,
                    input_uncached: *input,
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
                })
                .collect();
            let linked = self.linked;
            let seats: BTreeSet<String> = linked.then(|| "qa".to_string()).into_iter().collect();
            let seat_of = move |_: &str, _: &str| linked.then(|| "qa".to_string());
            let mut rows = handoff_intakes(&[edge], &queue, &tools, &usages, &seats, seat_of);
            rows.remove(0)
        }
    }

    fn outcome(row: &IntakeRow) -> (&str, Option<&str>, Option<i64>) {
        (row.status.as_str(), row.reason.as_deref(), row.tokens)
    }

    #[test]
    fn intake_follows_every_row_of_the_frozen_f2_table() {
        let est = |n| ("estimate", None, Some(n));
        // A, row 1: claim row S.200, receiver update S.500; usage at the end is out.
        let mut a = IntakeCase::new();
        a.transitions = vec![(500, 3, "qa")];
        a.usage = vec![(300, 7), (500, 100)];
        let row = a.run();
        assert_eq!(outcome(&row), est(7));
        assert_eq!(row.claim, Some(at_ms(100_200).to_rfc3339()));
        assert_eq!(row.write, Some(at_ms(100_500).to_rfc3339()));
        // B, row 2: the same with no usage in a non-empty window is zero.
        let mut b = IntakeCase::new();
        b.transitions = vec![(500, 3, "qa")];
        assert_eq!(outcome(&b.run()), est(0));
        // C, row 3: unlinked receiver (or a collision) has no data.
        b.linked = false;
        assert_eq!(
            outcome(&b.run()),
            ("no_data", Some("no linked session"), None)
        );
        // D, row 7: a receiver row before the claim row is not an end.
        let mut d = IntakeCase::new();
        d.transitions = vec![(100, 3, "qa")];
        assert_eq!(
            outcome(&d.run()),
            ("no_data", Some("no classifiable end"), None)
        );
        // E, row 4: the sender's update, or bash alone, is not an end.
        let mut e = IntakeCase::new();
        e.transitions = vec![(600, 3, "build")];
        e.tools = vec![(Some(300), false)];
        e.usage = vec![(300, 7)];
        assert_eq!(
            outcome(&e.run()),
            ("no_data", Some("no classifiable end"), None)
        );
        // F, row 6: same time as the claim row, later key: empty window, zero.
        let mut f = IntakeCase::new();
        f.transitions = vec![(200, 3, "qa")];
        f.usage = vec![(200, 7)];
        assert_eq!(outcome(&f.run()), est(0));
        // Row 7: same time, earlier key: not an end.
        let mut f2 = IntakeCase::new();
        f2.claim_row = Some((200, 5));
        f2.transitions = vec![(200, 4, "qa")];
        assert_eq!(
            outcome(&f2.run()),
            ("no_data", Some("no classifiable end"), None)
        );
        // G, row 8: a structured write exactly at the start: empty, zero.
        let mut g = IntakeCase::new();
        g.tools = vec![(Some(200), true)];
        g.usage = vec![(200, 7)];
        assert_eq!(outcome(&g.run()), est(0));
        // H, row 9: events-only claim at S (whole second), receiver row at S.500.
        let mut h = IntakeCase::new();
        h.claim_event_ms = 0;
        h.claim_row = None;
        h.transitions = vec![(500, 3, "qa")];
        h.usage = vec![(300, 7)];
        let row = h.run();
        assert_eq!(outcome(&row), est(7));
        assert_eq!(row.claim, Some(at_ms(100_000).to_rfc3339()));
        h.usage = vec![];
        assert_eq!(outcome(&h.run()), est(0));
        // I, row 10: events-only claim at S, receiver row at exactly S.
        let mut i = IntakeCase::new();
        i.claim_event_ms = 0;
        i.claim_row = None;
        i.transitions = vec![(0, 3, "qa")];
        i.usage = vec![(0, 7)];
        assert_eq!(outcome(&i.run()), est(0));
        // Claim row matching: an exact-time row wins over another row in the
        // same second; two rows in the second with none exact are ambiguous;
        // a row in another second is not this claim's row (events only).
        let mut m = IntakeCase::new();
        m.claim_event_ms = 200;
        m.claim_row = Some((200, 2));
        m.transitions = vec![(500, 3, "qa")];
        m.extra_claim_rows = vec![(700, 4)];
        assert_eq!(outcome(&m.run()), est(0));
        let mut m2 = IntakeCase::new();
        m2.claim_event_ms = 100;
        m2.claim_row = Some((200, 2));
        m2.extra_claim_rows = vec![(300, 4)];
        m2.transitions = vec![(500, 3, "qa")];
        assert_eq!(
            outcome(&m2.run()),
            ("ambiguous", Some("ambiguous claim row"), None)
        );
        let mut m3 = IntakeCase::new();
        m3.claim_event_ms = 100;
        m3.claim_row = Some((205, 2));
        m3.transitions = vec![(500, 3, "qa")];
        m3.usage = vec![(150, 4), (300, 7)];
        let row = m3.run();
        assert_eq!(outcome(&row), est(7), "the row's own ms, not the event's");
        assert_eq!(row.claim, Some(at_ms(100_205).to_rfc3339()));
        let mut m4 = IntakeCase::new();
        m4.claim_event_ms = 0;
        m4.claim_row = Some((1_200, 2));
        m4.transitions = vec![(1_500, 3, "qa")];
        m4.usage = vec![(1_100, 4)];
        assert_eq!(
            outcome(&m4.run()),
            est(4),
            "a row in the next second is an end, not the claim"
        );
        // F7: another item of the receiver closed before the claim is not an
        // overlap, whatever its event and transition ids; still open, it is.
        // The claim row is key 5; the root handoff already uses key 1.
        let mut o = IntakeCase::new();
        o.claim_row = Some((200, 5));
        o.transitions = vec![(500, 6, "qa")];
        o.usage = vec![(300, 7)];
        let ambiguous = ("ambiguous", Some("overlapping claim"), None);
        o.other = Some((-50_000, Some((-10_000, 4))));
        assert_eq!(outcome(&o.run()), est(7), "closed before the claim");
        o.other = Some((-50_000, None));
        assert_eq!(outcome(&o.run()), ambiguous, "still open");
        o.other = Some((-50_000, Some((300, 9))));
        assert_eq!(outcome(&o.run()), ambiguous, "closed inside the window");
        // Same time as the claim row, same table: key order decides.
        o.other = Some((-50_000, Some((200, 4))));
        assert_eq!(outcome(&o.run()), est(7), "earlier key closes first");
        o.other = Some((-50_000, Some((200, 9))));
        assert_eq!(
            outcome(&o.run()),
            ambiguous,
            "later key closes after the claim"
        );
        // Row 5: the only receiver write has no timestamp.
        let mut t = IntakeCase::new();
        t.tools = vec![(None, true)];
        assert_eq!(outcome(&t.run()), ("no_data", Some("no timestamp"), None));
    }

    #[test]
    fn a_receiver_queue_close_ends_intake_before_a_later_write_qa_fixture() {
        // QA: claim at 1 s (events only), receiver closes at 10 s; usage 5 at
        // 2 s and 9 at 10 s. A Write at 12 s cannot extend the window.
        let mut case = IntakeCase::new();
        case.claim_event_ms = -99_000;
        case.claim_row = None;
        case.transitions = vec![(-90_000, 2, "qa")];
        case.usage = vec![(-98_000, 5), (-90_000, 9)];
        assert_eq!(outcome(&case.run()), ("estimate", None, Some(5)));
        case.tools = vec![(Some(-88_000), true)];
        assert_eq!(outcome(&case.run()), ("estimate", None, Some(5)));
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
    fn fractional_wait_samples_preserve_order_keys_and_medians() {
        let cases: &[(&[i64], f64)] = &[
            (&[24_734_000], 24.734),
            (&[154_000, 49_000, 51_000], 0.051),
            (&[49_000, 149_000], 0.099),
            (&[100_000, 100_000, 900_000], 0.1),
            (&[1_000_000, 3_000_000, 5_000_000, 7_000_000], 4.0),
            (&[0], 0.0),
            (&[1], 0.000001),
            (&[0, 1], 0.0000005),
        ];
        for &(micros, expected_median) in cases {
            let mut queue = Vec::new();
            for (i, &wait) in micros.iter().enumerate() {
                let item = format!("item-{i}");
                let mut handoff = hand(&format!("handoff-{i}"), i as i64, &item, None, "A", "B");
                // Fractional endpoints cross second boundaries for short waits.
                handoff.ts += chrono::Duration::microseconds(900_000);
                let mut claim = q(
                    &format!("claim-{i}"),
                    100 + i as i64,
                    &item,
                    0,
                    "in-progress",
                    "B",
                    "B",
                    None,
                );
                claim.ts = handoff.ts + chrono::Duration::microseconds(wait);
                queue.extend([handoff, claim]);
            }
            let edges = handoff_edges(&queue);
            let rows = pair_medians(&edges, &queue, &[]);
            assert_eq!(rows.len(), 1);
            let row = &rows[0];
            assert_eq!(row.wait_invalid, 0);
            assert_eq!(row.wait_samples.len(), micros.len());
            for (i, (&sample, &wait)) in row.wait_samples.iter().zip(micros).enumerate() {
                assert!((sample - wait as f64 / 1_000_000.0).abs() <= 1e-12);
                assert_eq!(
                    row.wait_sample_keys[i],
                    (format!("handoff-{i}"), format!("claim-{i}"))
                );
            }
            assert!((row.wait_median.unwrap() - expected_median).abs() <= 1e-12);
            assert!(row.intake_samples.is_empty());
            assert_eq!(row.intake_median, None);
        }
    }

    #[test]
    fn fractional_waits_do_not_drop_datetime_extrema() {
        // Computational range proof, not a claim that source parsers accept
        // these extended dates. Total microseconds overflow i64, seconds do not.
        let duration = DateTime::<Utc>::MAX_UTC - DateTime::<Utc>::MIN_UTC;
        assert!(duration.num_microseconds().is_none());
        let mut handoff = hand("full-span", 0, "wide", None, "A", "B");
        handoff.ts = DateTime::<Utc>::MIN_UTC;
        let mut claim = q("wide-claim", 1, "wide", 0, "in-progress", "B", "B", None);
        claim.ts = DateTime::<Utc>::MAX_UTC;
        let queue = vec![handoff, claim];
        let rows = pair_medians(&handoff_edges(&queue), &queue, &[]);
        assert_eq!(rows[0].wait_invalid, 0);
        assert_eq!(rows[0].wait_samples.len(), 1);
        assert_eq!(rows[0].wait_median, Some(duration.as_seconds_f64()));
        assert!(rows[0].wait_median.unwrap().is_finite());
    }

    #[test]
    fn fractional_wait_binary64_resolution_is_magnitude_dependent() {
        let large = chrono::Duration::seconds(1_i64 << 32);
        assert_eq!(
            even_median(vec![
                large.as_seconds_f64(),
                (large + chrono::Duration::microseconds(1)).as_seconds_f64()
            ]),
            Some(large.as_seconds_f64())
        );
        let larger = chrono::Duration::seconds(1_i64 << 34);
        assert_eq!(
            larger.as_seconds_f64(),
            (larger + chrono::Duration::microseconds(1)).as_seconds_f64()
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

#[cfg(test)]
mod loop_evidence_tests {
    use super::*;
    use chrono::TimeZone;
    fn facts(offsets: &[Option<i64>]) -> Vec<ToolFact> {
        offsets
            .iter()
            .enumerate()
            .map(|(i, t)| ToolFact {
                source_key: format!("row{i}"),
                harness: "claude".into(),
                native_session: "s".into(),
                ts: t.map(|s| chrono::Utc.timestamp_opt(s, 0).unwrap()),
                seq: i as i64,
                name: "Read".into(),
                fingerprint: Some("opaque".into()),
                failed: Some(true),
                structured_write: false,
                generation: 7,
            })
            .collect()
    }
    #[test]
    fn full_run_and_first_sliding_witness_are_retained() {
        let rows = facts(&[Some(0), Some(400), Some(700), Some(800), Some(900)]);
        let e = evaluate_tool_loops(&rows, 3, 600);
        assert_eq!(e.order, vec![0, 1, 2, 3, 4]);
        assert_eq!(e.episodes.len(), 1);
        assert_eq!(e.episodes[0].members, vec![0, 1, 2, 3, 4]);
        assert_eq!(e.episodes[0].witness, vec![1, 2, 3]);
        assert_eq!(e.episodes[0].projection.count, 5);
        assert!(evaluate_tool_loops(&rows, 0, 600).episodes.is_empty());
    }
    #[test]
    fn captured_native_ties_can_have_different_valid_results() {
        let mut rows = facts(&[Some(0); 4]);
        for row in &mut rows {
            row.seq = 100;
        }
        rows[3].failed = Some(false);
        let e = evaluate_tool_loops(&rows, 3, 600);
        assert_eq!(e.episodes[0].members, vec![0, 1, 2]);
        rows.swap(2, 3);
        let e = evaluate_tool_loops(&rows, 3, 600);
        assert_eq!(e.order, vec![0, 1, 2, 3]);
        assert!(e.episodes.is_empty());
    }
    #[test]
    fn signed_native_and_anchored_codex_predicates_remain_distinct() {
        let rows = facts(&[Some(500), None, Some(0)]);
        let native = evaluate_tool_loops(&rows, 3, 600);
        assert_eq!(native.episodes[0].witness, vec![0, 1, 2]);
        let codex: Vec<_> = rows
            .into_iter()
            .map(|mut fact| {
                fact.harness = "codex".into();
                fact.name = CODEX_COMMAND.into();
                CodexObservation {
                    source_id: "source".into(),
                    fact,
                }
            })
            .collect();
        assert!(evaluate_codex_loops(&codex).episodes.is_empty());
    }
}
