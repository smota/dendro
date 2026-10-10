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

pub fn observation_span(
    window: Window,
    created: Option<DateTime<Utc>>,
    stopped: Option<DateTime<Utc>>,
) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
    let start = created.filter(|c| *c > window.from).unwrap_or(window.from);
    let end = stopped.filter(|s| *s < window.to).unwrap_or(window.to);
    if end <= start {
        None
    } else {
        Some((start, end))
    }
}

pub fn activity_bands(
    seat: &str,
    events: &[ActivityFact],
    queue: &[QueueFact],
    window: Window,
    created: Option<DateTime<Utc>>,
    stopped: Option<DateTime<Utc>>,
) -> Bands {
    let Some((start, end)) = observation_span(window, created, stopped) else {
        return Bands {
            active_secs: 0,
            idle_with_work_secs: 0,
            idle_without_work_secs: 0,
            waiting_human_secs: 0,
            unknown_secs: 0,
        };
    };
    let mut mine: Vec<&ActivityFact> = events.iter().filter(|e| e.seat == seat).collect();
    mine.sort_by(|a, b| a.ts.cmp(&b.ts).then(a.seq.cmp(&b.seq)));
    let in_window: Vec<&&ActivityFact> = mine
        .iter()
        .filter(|e| e.ts >= start && e.ts < end)
        .collect();
    let mut bands = Bands {
        active_secs: 0,
        idle_with_work_secs: 0,
        idle_without_work_secs: 0,
        waiting_human_secs: 0,
        unknown_secs: 0,
    };
    if in_window.is_empty() {
        add_band(&mut bands, "unknown", false, (end - start).num_seconds());
        return bands;
    }
    let carry = mine
        .iter()
        .rev()
        .find(|e| e.ts < start)
        .map(|e| e.state.as_str())
        .unwrap_or("unknown");
    let mut state = carry;
    let mut points: Vec<(DateTime<Utc>, &str)> = vec![(start, state)];
    for event in &in_window {
        points.push((event.ts, event.state.as_str()));
    }
    points.push((end, state));
    // Rebuild with demand splits inside idle.
    let mut marks: Vec<DateTime<Utc>> = vec![start, end];
    for event in &in_window {
        marks.push(event.ts);
    }
    for q in queue {
        if q.ts > start && q.ts < end && (q.dest_seat == seat || q.source_seat == seat) {
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
        let current = mine
            .iter()
            .rev()
            .find(|e| e.ts <= a)
            .map(|e| e.state.as_str())
            .unwrap_or("unknown");
        state = current;
        let demand = idle_demand(seat, queue, a);
        add_band(&mut bands, state, demand, secs);
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
    pub source_key: String,
}

pub fn handoff_edges(queue: &[QueueFact]) -> Vec<HandoffEdge> {
    let mut seen: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    let mut edges = Vec::new();
    let mut ordered = queue.to_vec();
    ordered.sort_by(|a, b| a.ts.cmp(&b.ts).then(a.transition_id.cmp(&b.transition_id)));
    for fact in ordered {
        if fact.closure_reason.as_deref() != Some("handed_off_to") {
            if !seen.contains_key(&fact.qitem_id) {
                seen.insert(fact.qitem_id.clone(), vec![fact.dest_seat.clone()]);
            } else if let Some(seats) = seen.get_mut(&fact.qitem_id) {
                if !seats.contains(&fact.dest_seat) {
                    seats.push(fact.dest_seat.clone());
                }
            }
            continue;
        }
        let to = fact
            .closure_target
            .clone()
            .unwrap_or_else(|| fact.dest_seat.clone());
        let prior = seen.get(&fact.qitem_id).cloned().unwrap_or_default();
        let is_return = prior.iter().any(|seat| seat == &to);
        edges.push(HandoffEdge {
            from_seat: fact.source_seat.clone(),
            to_seat: to.clone(),
            qitem_id: fact.qitem_id.clone(),
            chain_id: fact.chain_id.clone(),
            is_return,
            source_key: fact.source_key.clone(),
        });
        seen.entry(fact.qitem_id.clone()).or_default().push(to);
    }
    edges
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
        events.sort_by(|a, b| a.seq.cmp(&b.seq));
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
                && run.last().is_none_or(|prev| {
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
    if values.len() % 2 == 0 {
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
            None,
            None,
        );
        assert_eq!(bands.unknown_secs, 60);
        assert_eq!(bands.active_secs, 0);
        assert_eq!(bands.sum(), 60);
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
            None,
            None,
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
                source_key: "a".into(),
            },
            HandoffEdge {
                from_seat: "dev.qa".into(),
                to_seat: "rev-review".into(),
                qitem_id: "i".into(),
                chain_id: Some("c".into()),
                is_return: false,
                source_key: "b".into(),
            },
            HandoffEdge {
                from_seat: "rev-review".into(),
                to_seat: "dev-build".into(),
                qitem_id: "i".into(),
                chain_id: Some("c".into()),
                is_return: true,
                source_key: "c".into(),
            },
        ];
        let (count, chains, paths) = m3_rework(&edges);
        assert_eq!(count, 1);
        assert_eq!(chains, 1);
        assert_eq!(paths.len(), 1);
    }
}
