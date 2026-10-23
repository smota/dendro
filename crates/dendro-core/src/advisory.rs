//! Read-only advisory intervals over retained queue, activity and lifecycle evidence.
use std::collections::{BTreeMap, BTreeSet};

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::metrics::{self, Window};
use crate::vortex::ItemEvidence;
use crate::{ActivityFact, DestinationSnapshot, QueueFact, SeatBound};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvisoryOptions {
    pub container_hold_secs: u64,
    pub leaf_stall_secs: u64,
    pub detached_target_secs: u64,
    pub ceremony_ratio: f64,
    pub serialization_ratio: f64,
    pub parallelism: f64,
    pub minimum_seats: usize,
    pub container_ratio: f64,
    pub recovery_ratio: f64,
    pub context_drop_ratio: f64,
    pub cache_bleed_tokens: u64,
    pub builder_duty_ratio: f64,
    pub peer_saturation_ratio: f64,
    pub builder_seats: Vec<String>,
}
impl Default for AdvisoryOptions {
    fn default() -> Self {
        Self {
            container_hold_secs: 900,
            leaf_stall_secs: 1800,
            detached_target_secs: 300,
            ceremony_ratio: 0.4,
            serialization_ratio: 0.75,
            parallelism: 1.2,
            minimum_seats: 4,
            container_ratio: 0.5,
            recovery_ratio: 0.1,
            context_drop_ratio: 0.5,
            cache_bleed_tokens: 100_000,
            builder_duty_ratio: 0.05,
            peer_saturation_ratio: 0.75,
            builder_seats: vec![],
        }
    }
}
impl AdvisoryOptions {
    pub fn validate(&self) -> Result<()> {
        if self
            .builder_seats
            .iter()
            .any(|seat| crate::contains_secret(seat))
        {
            return Err(anyhow!(
                "builder seat configuration contains a secret-shaped value"
            ));
        }
        for value in [
            self.ceremony_ratio,
            self.serialization_ratio,
            self.container_ratio,
            self.recovery_ratio,
            self.context_drop_ratio,
            self.builder_duty_ratio,
            self.peer_saturation_ratio,
        ] {
            if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                return Err(anyhow!("advisory ratios must be finite and in [0,1]"));
            }
        }
        if self.context_drop_ratio < 0.5 {
            return Err(anyhow!("spotlight-context-drop-ratio must be at least 0.5: canonical producer floor is 50% and 50000 tokens"));
        }
        if !self.parallelism.is_finite() || self.parallelism < 0.0 || self.minimum_seats == 0 {
            return Err(anyhow!("spotlight parallelism must be finite and nonnegative; minimum seats must be positive"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ActivityPoint {
    pub seat: String,
    pub ts: DateTime<Utc>,
    pub source_key: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct UnresolvedActivity {
    pub source_key: String,
    pub seat: Option<String>,
    pub reason: &'static str,
}

pub struct Input<'a> {
    pub queue: &'a [QueueFact],
    pub metadata: &'a [ItemEvidence],
    pub activity: &'a [ActivityFact],
    pub bounds: &'a [SeatBound],
    pub local_activity: &'a [ActivityPoint],
    pub unresolved_activity: &'a [UnresolvedActivity],
    pub snapshots: &'a BTreeMap<String, Vec<DestinationSnapshot>>,
    pub seats: &'a BTreeSet<String>,
    pub nudge_metadata: &'a BTreeMap<String, crate::QueueItemMeta>,
    pub window: Window,
    pub options: &'a AdvisoryOptions,
}

fn state_at<'a>(rows: &[&'a QueueFact], at: DateTime<Utc>) -> Option<&'a QueueFact> {
    let latest = rows
        .iter()
        .filter(|r| r.ts <= at)
        .max_by_key(|r| (r.ts, r.transition_id))?;
    if rows.iter().any(|r| {
        r.source_key == latest.source_key
            && (r.state != latest.state
                || r.dest_seat != latest.dest_seat
                || r.source_seat != latest.source_seat)
    }) {
        return None;
    }
    if rows.iter().any(|r| {
        r.ts == latest.ts
            && r.state != latest.state
            && (r.source_key.starts_with("events:") != latest.source_key.starts_with("events:")
                || r.transition_id == latest.transition_id)
    }) {
        return None;
    }
    Some(*latest)
}
fn ancestor(child: &str, parent: &str, parents: &BTreeMap<&str, BTreeSet<&str>>) -> Option<bool> {
    let mut id = child;
    let mut seen = BTreeSet::new();
    for _ in 0..128 {
        if !seen.insert(id) {
            return None;
        }
        let Some(p) = parents.get(id) else {
            return Some(false);
        };
        if p.len() > 1 {
            return None;
        }
        let Some(next) = p.first() else {
            return Some(false);
        };
        if *next == parent {
            return Some(true);
        }
        id = next;
    }
    None
}
fn failed(status: &str) -> bool {
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
        "missing",
        "rejected",
    ]
    .iter()
    .any(|p| status.contains(p))
}

#[derive(Clone)]
struct Span {
    kind: &'static str,
    seat: String,
    item: String,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    keys: BTreeSet<String>,
    children: BTreeSet<String>,
}
fn emit(mut spans: Vec<Span>, options: &AdvisoryOptions) -> Vec<Value> {
    spans.sort_by_key(|s| (s.seat.clone(), s.item.clone(), s.kind, s.from));
    let mut merged: Vec<Span> = vec![];
    for s in spans {
        if let Some(last) = merged.last_mut() {
            if last.to == s.from
                && last.kind == s.kind
                && last.item == s.item
                && last.seat == s.seat
            {
                last.to = s.to;
                last.keys.extend(s.keys);
                last.children.extend(s.children);
                continue;
            }
        }
        merged.push(s);
    }
    merged.into_iter().map(|s| {
        let seconds=(s.to-s.from).as_seconds_f64();
        let threshold=match s.kind {"unparked_container_hold"=>options.container_hold_secs,"true_leaf_stall"=>options.leaf_stall_secs,_=>options.detached_target_secs};
        json!({"exceeds_threshold":seconds>threshold as f64,"classification":s.kind,"seat":s.seat,"qitem_id":s.item,
            "from":s.from,"to":s.to,"duration_secs":seconds,"threshold_secs":threshold,"source_keys":s.keys,"child_qitem_ids":s.children,
            "recommendation":match s.kind {"unparked_container_hold"=>"Consider parking the parent with --blocked-on while downstream work runs.","true_leaf_stall"=>"No active downstream child is recorded. Inspect the claimed leaf's harness and missing lineage evidence before deciding an action.",_=>"Inspect the recorded target lifecycle and transport route before retrying delivery."}})
    }).collect()
}

pub fn evaluate(input: Input<'_>) -> Value {
    let mut items = BTreeMap::<&str, Vec<&QueueFact>>::new();
    for q in input.queue {
        items.entry(&q.qitem_id).or_default().push(q);
    }
    for rows in items.values_mut() {
        rows.sort_by_key(|r| (r.ts, r.transition_id));
    }
    let owned_items: BTreeMap<_, Vec<QueueFact>> = items
        .iter()
        .map(|(id, rows)| (*id, rows.iter().map(|q| (*q).clone()).collect()))
        .collect();
    let mut parents = BTreeMap::<&str, BTreeSet<&str>>::new();
    for m in input.metadata {
        if let Some(p) = &m.parent_qitem_id {
            parents.entry(&m.qitem_id).or_default().insert(p);
        }
    }
    let mut findings = vec![];
    let mut omissions = vec![];
    let mut usable_activity = false;
    for seat in input.seats {
        let mine: Vec<_> = input.activity.iter().filter(|a| a.seat == *seat).collect();
        let lives: Vec<_> = input
            .bounds
            .iter()
            .filter(|b| b.seat == *seat)
            .cloned()
            .collect();
        let spans = metrics::observation_spans(input.window, &lives);
        if !mine
            .iter()
            .any(|a| spans.iter().any(|s| a.ts >= s.start && a.ts < s.end))
        {
            omissions.push(json!({"seat":seat,"reason":"no_activity_in_observation_span"}));
            continue;
        }
        usable_activity = true;
        let uncertain: Vec<_> = input
            .unresolved_activity
            .iter()
            .filter(|a| a.seat.as_deref().is_none_or(|s| s == seat))
            .collect();
        if !uncertain.is_empty() {
            omissions.push(
                json!({"seat":seat,"reason":"unresolved_local_activity","evidence":uncertain}),
            );
            continue;
        }
        for life in spans {
            let mut marks = vec![life.start, life.end];
            marks.extend(
                input
                    .queue
                    .iter()
                    .map(|q| q.ts)
                    .filter(|t| *t > life.start && *t < life.end),
            );
            marks.extend(
                mine.iter()
                    .map(|a| a.ts)
                    .filter(|t| *t > life.start && *t < life.end),
            );
            let local: Vec<_> = input
                .local_activity
                .iter()
                .filter(|a| a.seat == *seat && a.ts >= life.start && a.ts < life.end)
                .collect();
            marks.extend(local.iter().map(|a| a.ts).filter(|t| *t > life.start));
            marks.sort();
            marks.dedup();
            let mut intervals = vec![];
            for times in marks.windows(2) {
                let (from, to) = (times[0], times[1]);
                let activity = mine
                    .iter()
                    .filter(|a| a.ts <= from && life.launched.is_none_or(|l| a.ts >= l))
                    .max_by_key(|a| (a.ts, a.seq));
                let Some(activity) = activity.filter(|a| a.state == "idle") else {
                    continue;
                };
                if local
                    .iter()
                    .filter(|a| a.ts <= from)
                    .max_by_key(|a| a.ts)
                    .is_some_and(|event| event.ts >= activity.ts)
                {
                    omissions.push(json!({"seat":seat,"from":from,"to":to,"reason":"no_fresh_idle_after_local_activity"}));
                    continue;
                }
                for (id, rows) in &items {
                    if rows.iter().any(|q| q.ts <= from && q.dest_seat == *seat)
                        && state_at(rows, from).is_none()
                    {
                        omissions.push(json!({"seat":seat,"qitem_id":id,"from":from,"to":to,"reason":"unordered_claim_state"}));
                    }
                }
                let latest: Vec<_> = items
                    .values()
                    .filter_map(|rows| state_at(rows, from))
                    .collect();
                if local.iter().any(|a| a.ts == from) && !intervals.is_empty() {
                    findings.extend(emit(std::mem::take(&mut intervals), input.options));
                }
                for &claim in latest.iter().filter(|q| q.state == "in-progress") {
                    let owned = &owned_items[claim.qitem_id.as_str()];
                    match metrics::destination_at(claim, owned, input.snapshots) {
                        Ok(owner) if owner.0 == *seat => {}
                        Ok(_) => continue,
                        Err(reason) => {
                            if claim.dest_seat == *seat {
                                omissions.push(json!({"seat":seat,"qitem_id":claim.qitem_id,"from":from,"to":to,"reason":reason}));
                            }
                            continue;
                        }
                    }
                    let mut children = BTreeSet::new();
                    let mut keys =
                        BTreeSet::from([claim.source_key.clone(), activity.source_key.clone()]);
                    keys.extend(
                        local
                            .iter()
                            .filter(|a| a.ts == from || a.ts == to)
                            .map(|a| a.source_key.clone()),
                    );
                    let mut unknown = items.values().any(|rows| {
                        rows.iter().any(|r| r.ts <= from) && state_at(rows, from).is_none()
                    });
                    for &child in latest
                        .iter()
                        .filter(|q| q.state == "in-progress" && q.qitem_id != claim.qitem_id)
                    {
                        match ancestor(&child.qitem_id, &claim.qitem_id, &parents) {
                            Some(true) if child.dest_seat != *seat => {
                                children.insert(child.qitem_id.clone());
                                keys.insert(child.source_key.clone());
                            }
                            None => unknown = true,
                            _ => {}
                        }
                    }
                    if unknown {
                        omissions.push(json!({"seat":seat,"qitem_id":claim.qitem_id,"from":from,"to":to,"reason":"ambiguous_child_lineage"}));
                    }
                    let kind = if !children.is_empty() {
                        "unparked_container_hold"
                    } else if unknown {
                        continue;
                    } else {
                        "true_leaf_stall"
                    };
                    intervals.push(Span {
                        kind,
                        seat: seat.clone(),
                        item: claim.qitem_id.clone(),
                        from,
                        to,
                        keys,
                        children,
                    });
                }
            }
            findings.extend(emit(intervals, input.options));
        }
    }
    // Latest metadata is a snapshot, so duration comes from queue/lifecycle endpoints,
    // never from an invented nudge timestamp. A pending record alone is not a failed wake.
    for (id, rows) in &items {
        let Some(first) = rows
            .iter()
            .find(|q| q.source_key.starts_with("events:queue.created:"))
        else {
            continue;
        };
        let Some(meta) = input.nudge_metadata.get(*id) else {
            continue;
        };
        let Some(nudge) = meta.last_nudge_result.as_deref().filter(|s| failed(s)) else {
            continue;
        };
        let mut spans = vec![];
        let owned = &owned_items[id];
        let mut marks = vec![input.window.from.max(first.ts), input.window.to];
        marks.extend(
            rows.iter()
                .map(|q| q.ts)
                .filter(|t| *t > input.window.from && *t < input.window.to),
        );
        marks.extend(
            input
                .bounds
                .iter()
                .filter(|b| input.seats.contains(&b.seat))
                .flat_map(|b| [b.created_at, b.stopped_at])
                .flatten()
                .filter(|t| *t > input.window.from && *t < input.window.to),
        );
        marks.sort();
        marks.dedup();
        for times in marks.windows(2) {
            let (from, to) = (times[0], times[1]);
            if from >= to || from < input.window.from {
                continue;
            }
            let Some(row) = state_at(rows, from) else {
                continue;
            };
            if !matches!(row.state.as_str(), "pending" | "unclaimed") {
                continue;
            }
            let seat = match metrics::destination_at(row, owned, input.snapshots) {
                Ok((seat, _, _)) => seat,
                Err(reason) => {
                    if input.seats.contains(&row.dest_seat) {
                        omissions.push(json!({"qitem_id":id,"seat":row.dest_seat,"reason":reason}));
                    }
                    continue;
                }
            };
            if !input.seats.contains(&seat) {
                continue;
            }
            let stopped = input.bounds.iter().filter(|b| b.seat == seat).any(|b| {
                b.stopped_at.is_some_and(|t| t <= from)
                    && !input.bounds.iter().any(|later| {
                        later.seat == seat
                            && later
                                .created_at
                                .is_some_and(|t| t > b.stopped_at.unwrap() && t <= from)
                    })
            });
            let unreachable = [
                "unreachable",
                "undelivered",
                "unroutable",
                "no-session",
                "no_session",
                "not_found",
            ]
            .iter()
            .any(|s| nudge.to_ascii_lowercase().contains(s));
            if stopped || unreachable {
                spans.push(Span {
                    kind: "detached_target_wake_failure",
                    seat: seat.clone(),
                    item: (*id).into(),
                    from,
                    to,
                    keys: BTreeSet::from([first.source_key.clone(), row.source_key.clone()]),
                    children: BTreeSet::new(),
                });
            }
        }
        findings.extend(emit(spans, input.options));
    }
    for finding in &mut findings {
        if finding["classification"] == "detached_target_wake_failure" {
            if let Some(id) = finding["qitem_id"].as_str().map(str::to_owned) {
                finding["nudge_evidence"] = json!({"last_nudge_result":input.nudge_metadata.get(&id).and_then(|m|m.last_nudge_result.as_deref()),
                    "scope":"latest persisted item metadata; no attempt timestamp recorded", "source_keys":input.metadata.iter().filter(|m|m.qitem_id==id).map(|m|&m.source_key).collect::<Vec<_>>()});
                finding["lifecycle_evidence"] = json!(input
                    .bounds
                    .iter()
                    .filter(|b| Some(b.seat.as_str()) == finding["seat"].as_str())
                    .collect::<Vec<_>>());
            }
        }
    }
    let wake_evaluated = input
        .nudge_metadata
        .iter()
        .filter(|(id, m)| {
            m.last_nudge_result.is_some()
                && items
                    .get(id.as_str())
                    .is_some_and(|rows| rows.iter().any(|q| input.seats.contains(&q.dest_seat)))
        })
        .count();
    let intervals = findings.clone();
    findings.retain(|f| f["exceeds_threshold"] == true);
    findings.sort_by_key(|f| {
        (
            f["seat"].as_str().unwrap_or_default().to_owned(),
            f["qitem_id"].as_str().unwrap_or_default().to_owned(),
            f["from"].as_str().unwrap_or_default().to_owned(),
            f["classification"].as_str().unwrap_or_default().to_owned(),
        )
    });
    json!({"definition":"advisory-idle-claimed-v1","scope":"retained evidence only; no runtime intervention; failed nudge metadata is a latest snapshot, not timed attempt history",
        "window":{"from":input.window.from,"to":input.window.to},"thresholds":input.options,"wake_coverage":if wake_evaluated>0{"observed"}else{"no_data"},"coverage":if !usable_activity && intervals.is_empty(){"no_data"}else if omissions.is_empty(){"observed"}else{"partial"},
        "intervals":intervals,"findings":findings,"omissions":omissions})
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeDelta, TimeZone};
    fn at(n: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000 + n, 0).unwrap()
    }
    fn q(id: &str, n: i64, state: &str, seat: &str, prefix: &str) -> QueueFact {
        QueueFact {
            source_key: format!("{prefix}:{id}:{n}"),
            transition_id: n,
            qitem_id: id.into(),
            ts: at(n),
            state: state.into(),
            source_seat: seat.into(),
            dest_seat: seat.into(),
            closure_reason: None,
            closure_target: None,
            chain_id: Some("shared".into()),
            missions: vec!["shared".into()],
        }
    }
    #[derive(Default)]
    struct Fixture {
        queue: Vec<QueueFact>,
        meta: Vec<ItemEvidence>,
        activity: Vec<ActivityFact>,
        bounds: Vec<SeatBound>,
        local: Vec<ActivityPoint>,
        nudge: BTreeMap<String, crate::QueueItemMeta>,
        unresolved: Vec<UnresolvedActivity>,
    }
    impl Fixture {
        fn parent() -> Self {
            Self {
                queue: vec![
                    q("parent", 0, "pending", "A", "events:queue.created"),
                    q("parent", 1, "in-progress", "A", "events:queue.claimed"),
                    q("parent", 100, "done", "A", "queue_transitions"),
                ],
                activity: vec![ActivityFact {
                    source_key: "activity:A:2".into(),
                    seat: "A".into(),
                    ts: at(2),
                    state: "idle".into(),
                    seq: 2,
                }],
                ..Self::default()
            }
        }
        fn evaluate(&self) -> Value {
            self.options(AdvisoryOptions {
                container_hold_secs: 10,
                leaf_stall_secs: 10,
                detached_target_secs: 10,
                ..AdvisoryOptions::default()
            })
        }
        fn options(&self, options: AdvisoryOptions) -> Value {
            evaluate(Input {
                queue: &self.queue,
                metadata: &self.meta,
                activity: &self.activity,
                bounds: &self.bounds,
                local_activity: &self.local,
                unresolved_activity: &self.unresolved,
                snapshots: &BTreeMap::new(),
                seats: &BTreeSet::from(["A".into()]),
                nudge_metadata: &self.nudge,
                window: Window {
                    from: at(0),
                    to: at(100),
                },
                options: &options,
            })
        }
        fn child(&mut self) {
            self.queue.extend([
                q("child", 3, "pending", "B", "events:queue.created"),
                q("child", 4, "in-progress", "B", "events:queue.claimed"),
                q("child", 90, "done", "B", "queue_transitions"),
            ]);
            self.meta.push(ItemEvidence {
                source_key: "events:queue.created:child:3".into(),
                qitem_id: "child".into(),
                parent_qitem_id: Some("parent".into()),
                is_recovery: false,
                recovery_reason: None,
                issue_tags: vec![],
            });
        }
    }
    #[test]
    fn classifies_leaf_container_and_strict_equality() {
        let mut f = Fixture::parent();
        let r = f.evaluate();
        assert_eq!(r["findings"][0]["classification"], "true_leaf_stall");
        assert_eq!(r["findings"][0]["duration_secs"], 98.0);
        assert!(f.options(AdvisoryOptions {
            leaf_stall_secs: 98,
            ..AdvisoryOptions::default()
        })["findings"]
            .as_array()
            .unwrap()
            .is_empty());
        f.child();
        let r = f.evaluate();
        assert_eq!(r["findings"].as_array().unwrap().len(), 1);
        assert_eq!(
            r["findings"][0]["classification"],
            "unparked_container_hold"
        );
        assert_eq!(r["findings"][0]["duration_secs"], 86.0);
    }
    #[test]
    fn shared_tags_never_prove_child_and_conflicts_do_not_prove_leaf() {
        let mut f = Fixture::parent();
        f.child();
        f.meta.clear();
        assert_eq!(
            f.evaluate()["findings"][0]["classification"],
            "true_leaf_stall"
        );
        f.child();
        let mut conflict = f.meta[0].clone();
        conflict.parent_qitem_id = Some("other".into());
        f.meta.push(conflict);
        let r = f.evaluate();
        assert!(r["findings"].as_array().unwrap().is_empty());
        assert_eq!(r["coverage"], "partial");
    }
    #[test]
    fn local_activity_reset_interruption_and_stop_bound_continuous_idle() {
        let mut f = Fixture::parent();
        f.local.push(ActivityPoint {
            seat: "A".into(),
            ts: at(50),
            source_key: "usage:50".into(),
        });
        let r = f.evaluate();
        assert_eq!(r["findings"].as_array().unwrap().len(), 1);
        assert_eq!(r["findings"][0]["duration_secs"], 48.0);
        assert_eq!(r["coverage"], "partial");
        f.activity.push(ActivityFact {
            source_key: "idle:fresh".into(),
            seat: "A".into(),
            ts: at(60),
            state: "idle".into(),
            seq: 60,
        });
        assert_eq!(f.evaluate()["findings"][1]["duration_secs"], 40.0);
        f.queue
            .push(q("parent", 20, "blocked", "A", "queue_transitions"));
        assert_eq!(f.evaluate()["findings"][0]["duration_secs"], 18.0);
        f.bounds.push(SeatBound {
            seat: "A".into(),
            created_at: Some(at(0)),
            stopped_at: Some(at(10)),
        });
        assert!(f.evaluate()["findings"].as_array().unwrap().is_empty());
    }
    #[test]
    fn detached_requires_failed_transport_or_stopped_lifecycle() {
        let mut f = Fixture::default();
        f.queue
            .push(q("pending", 0, "pending", "A", "events:queue.created"));
        f.nudge.insert(
            "pending".into(),
            crate::QueueItemMeta {
                last_nudge_result: Some("failed".into()),
                ..crate::QueueItemMeta::default()
            },
        );
        assert!(f.evaluate()["findings"].as_array().unwrap().is_empty());
        f.bounds.push(SeatBound {
            seat: "A".into(),
            created_at: Some(at(-10)),
            stopped_at: Some(at(20)),
        });
        let r = f.evaluate();
        assert_eq!(
            r["findings"][0]["classification"],
            "detached_target_wake_failure"
        );
        assert_eq!(r["findings"][0]["duration_secs"], 80.0);
    }
    #[test]
    fn unknown_activity_and_tied_states_do_not_invent_idle() {
        let mut f = Fixture::parent();
        f.activity.clear();
        assert!(f.evaluate()["findings"].as_array().unwrap().is_empty());
        f = Fixture::parent();
        let mut tied = q("parent", 1, "blocked", "A", "queue_transitions");
        tied.transition_id = 999;
        f.queue.push(tied);
        assert!(f.evaluate()["findings"].as_array().unwrap().is_empty());
        assert_eq!(f.evaluate()["coverage"], "partial");
        let mut missing = Fixture::parent();
        missing.queue.remove(0);
        assert!(missing.evaluate()["findings"]
            .as_array()
            .unwrap()
            .is_empty());
        assert_eq!(missing.evaluate()["coverage"], "partial");
    }
    #[test]
    fn subsecond_threshold_and_window_are_exact() {
        let mut f = Fixture::parent();
        f.activity[0].ts = at(89) + TimeDelta::milliseconds(999);
        assert_eq!(f.evaluate()["findings"][0]["duration_secs"], 10.001);
    }
    #[test]
    fn public_empty_report_trace_and_formats_are_read_only() {
        let path =
            std::env::temp_dir().join(format!("dendro-advisory-{}.duckdb", std::process::id()));
        let store = crate::AnalyticsStore::open_path(&path, "fixture").unwrap();
        store.checkpoint().unwrap();
        drop(store);
        let before = std::fs::read(&path).unwrap();
        let from = at(0).to_rfc3339();
        let to = at(100).to_rfc3339();
        let report =
            crate::pipeline::report_json_physical(&path, "advisory", None, &from, &to).unwrap();
        let trace =
            crate::pipeline::trace_json_physical(&path, "advisory_idle_claimed", None, &from, &to)
                .unwrap();
        assert_eq!(report["payload"]["advisory_idle_claimed"], trace["value"]);
        let anomalies =
            crate::pipeline::report_json_physical(&path, "anomalies", None, &from, &to).unwrap();
        assert_eq!(
            anomalies["payload"]["anomaly_spotlights"]["detectors"]
                .as_array()
                .unwrap()
                .len(),
            6
        );
        assert!(
            crate::spotlight::markdown(&anomalies["payload"]["anomaly_spotlights"])
                .starts_with("## Fleet Anomalies & Diagnostic Spotlights")
        );
        assert_eq!(before, std::fs::read(&path).unwrap());
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn changing_children_union_one_continuous_hold_and_unknown_activity_suppresses_claims() {
        let mut f = Fixture::parent();
        for (id, start, end) in [("c1", 3, 10), ("c2", 10, 17), ("c3", 17, 24)] {
            f.queue.extend([
                q(id, start - 1, "pending", "B", "events:queue.created"),
                q(id, start, "in-progress", "B", "events:queue.claimed"),
                q(id, end, "done", "B", "queue_transitions"),
            ]);
            f.meta.push(ItemEvidence {
                source_key: format!("events:queue.created:{id}"),
                qitem_id: id.into(),
                parent_qitem_id: Some("parent".into()),
                is_recovery: false,
                recovery_reason: None,
                issue_tags: vec![],
            });
        }
        let result = f.evaluate();
        let hold = result["findings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["classification"] == "unparked_container_hold")
            .unwrap();
        assert_eq!(hold["duration_secs"], 21.0);
        assert_eq!(hold["child_qitem_ids"].as_array().unwrap().len(), 3);
        f.unresolved.push(UnresolvedActivity {
            source_key: "untimed:usage".into(),
            seat: None,
            reason: "missing_timestamp",
        });
        let result = f.evaluate();
        assert!(result["findings"].as_array().unwrap().is_empty());
        assert_eq!(result["coverage"], "partial");
    }
    #[test]
    fn advisory_markdown_and_ndjson_use_duration_schema() {
        let value = Fixture::parent().evaluate();
        let md = crate::spotlight::markdown(&value);
        assert!(md.starts_with("## Idle With Claimed Work Advisories"));
        assert!(md.contains("observed 98.0"));
        assert!(md.contains("threshold > 10"));
        assert!(!md.contains("null"));
        let rows = crate::spotlight::ndjson_records(&value);
        assert!(rows[0].get("detectors").is_none());
        assert_eq!(rows[1]["finding"]["duration_secs"], 98.0);
    }
    #[test]
    fn persisted_untimed_usage_is_not_silently_dropped_from_idle_coverage() {
        let path = std::env::temp_dir().join(format!(
            "dendro-advisory-untimed-{}.duckdb",
            std::process::id()
        ));
        let store = crate::AnalyticsStore::open_path(&path, "fixture").unwrap();
        let f = Fixture::parent();
        for row in &f.queue {
            store.insert_queue(row).unwrap();
        }
        for row in &f.activity {
            store.insert_activity(row).unwrap();
        }
        store
            .insert_identity(&crate::IdentityFact {
                source_key: "identity:A".into(),
                harness: "codex".into(),
                native_session: "A".into(),
                rig: "test".into(),
                seat: "A".into(),
                ts: None,
            })
            .unwrap();
        let u:crate::UsageFact=serde_json::from_value(json!({"source_key":"usage:untimed","harness":"codex","native_session":"A","seq":1,"ts":null,"input_uncached":1,"output":0,"cache_read":0,"cache_write":0,"reasoning":0,"generation":1})).unwrap();
        store
            .commit_file_batch(
                "usage-fixture",
                "codex",
                "synthetic",
                1,
                0,
                "fixture",
                None,
                &[u],
                &[],
            )
            .unwrap();
        store.checkpoint().unwrap();
        drop(store);
        let r = crate::pipeline::report_json_with_analysis_physical(
            &path,
            "advisory",
            None,
            &at(0).to_rfc3339(),
            &at(100).to_rfc3339(),
            crate::LoopThresholds::default(),
            crate::FrictionThresholds::default(),
            AdvisoryOptions {
                leaf_stall_secs: 10,
                ..AdvisoryOptions::default()
            },
        )
        .unwrap();
        assert_eq!(r["payload"]["advisory_idle_claimed"]["coverage"], "partial");
        assert!(r["payload"]["advisory_idle_claimed"]["findings"]
            .as_array()
            .unwrap()
            .is_empty());
        std::fs::remove_file(path).unwrap();
    }
}
