#![deny(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::facts::{ActivityFact, QueueFact, QueueItemMeta, SeatBound};
use crate::metrics::{self, Bands, Window};

/// Bucket in the fleet concurrency distribution for a specific count of
/// simultaneously active seats k in 0..=N.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConcurrencyBucket {
    /// Number of simultaneously active seats (k).
    pub active_seats: usize,
    /// Total duration in seconds during the window with exactly this concurrency level.
    pub secs: i64,
    /// Proportion of the report window with this concurrency level.
    pub share: f64,
    /// AC4 conservation flag: true when the total seconds across all buckets equal window_secs.
    pub conserved: bool,
}

/// Fleet concurrency distribution and aggregate parallelism metrics.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FleetConcurrency {
    /// Total duration of the report window in seconds (`(window.to - window.from).num_seconds()`).
    pub window_secs: i64,
    /// Total number of scoped seats (N).
    pub seat_count: usize,
    /// Total active seat-seconds across all seats (`sum(k * secs(k)) == sum(active_secs)`).
    pub total_active_seat_secs: i64,
    /// Effective parallelism: average number of active seats (`total_active_seat_secs / window_secs`).
    pub effective_parallelism: Option<f64>,
    /// Effective parallelism in milli-seats (scaled by 1000, rounded).
    pub effective_parallelism_milli: u64,
    /// Fleet duty cycle: fraction of maximum possible seat-time utilized (`total_active_seat_secs / (N * window_secs)`).
    pub fleet_duty_cycle: Option<f64>,
    /// Fleet duty cycle in basis points (scaled by 10000, rounded).
    pub fleet_duty_cycle_bps: u64,
    /// Histogram of duration buckets for k in 0..=N simultaneously active seats.
    pub histogram: Vec<ConcurrencyBucket>,
    /// AC4 conservation: true when sum of bucket seconds equals window_secs.
    pub conserved: bool,
}

/// AC7 provenance evidence linking an unparked parent container to a child task in progress.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerHoldEvidence {
    pub parent_qitem_id: String,
    pub parent_source_key: String,
    pub child_qitem_id: String,
    pub child_seat: String,
    pub child_source_key: String,
    pub secs: i64,
}

/// Disambiguation of `idle_with_work` time into unparked container holding vs leaf stalling.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeatContainerDisambiguation {
    /// Seconds spent holding an in-progress container while a child task runs on another seat.
    pub unparked_container_secs: i64,
    /// Seconds spent idle with work without any active child running elsewhere.
    pub leaf_stall_secs: i64,
    /// Traceable evidence linking parent and child items with provenance keys.
    pub container_hold_evidence: Vec<ContainerHoldEvidence>,
}

/// Determines whether `parent_id` is a direct or transitive ancestor of `child_id`
/// according to `QueueItemMeta.parent_qitem_id`.
fn is_transitive_parent(
    meta: &BTreeMap<String, QueueItemMeta>,
    child_id: &str,
    parent_id: &str,
) -> bool {
    let mut current = child_id;
    let mut visited = BTreeSet::new();
    visited.insert(current);
    while visited.len() < 128 {
        if let Some(item_meta) = meta.get(current) {
            if let Some(p) = &item_meta.parent_qitem_id {
                if p == parent_id {
                    return true;
                }
                if !visited.insert(p.as_str()) {
                    break;
                }
                current = p.as_str();
            } else {
                break;
            }
        } else {
            break;
        }
    }
    false
}

/// Sweep-line fleet concurrency distribution across the report window `[window.from, window.to)`.
///
/// Sweeps over all event boundaries using whole-second cumulative offset arithmetic from
/// `window.from` (`(b - window.from).num_seconds() - (a - window.from).num_seconds()`) so that
/// the durations for k in 0..=N simultaneously active seats sum exactly to `window_secs`.
pub fn fleet_concurrency(
    window: Window,
    seats: &[String],
    events: &[ActivityFact],
    bounds: &[SeatBound],
) -> FleetConcurrency {
    let window_secs = (window.to - window.from).num_seconds();
    let seat_count = seats.len();

    if window_secs <= 0 {
        let histogram = (0..=seat_count)
            .map(|k| ConcurrencyBucket {
                active_seats: k,
                secs: 0,
                share: 0.0,
                conserved: true,
            })
            .collect();
        return FleetConcurrency {
            window_secs: 0,
            seat_count,
            total_active_seat_secs: 0,
            effective_parallelism: None,
            effective_parallelism_milli: 0,
            fleet_duty_cycle: None,
            fleet_duty_cycle_bps: 0,
            histogram,
            conserved: true,
        };
    }

    // For each seat, compute its active integer-second intervals [start_sec, end_sec)
    // inside [0, window_secs) using the exact per-span whole-second offset arithmetic
    // of `metrics::activity_bands`, so each seat's active duration in the sweep-line
    // equals its canonical `active_secs` even when lifecycle bounds have sub-second offsets.
    let mut seat_active_intervals: Vec<Vec<(i64, i64)>> = Vec::with_capacity(seat_count);
    let mut interval_marks: Vec<i64> = vec![0, window_secs];

    for seat in seats {
        let seat_lives: Vec<SeatBound> =
            bounds.iter().filter(|b| b.seat == *seat).cloned().collect();
        let spans = metrics::observation_spans(window, &seat_lives);
        let mut mine: Vec<&ActivityFact> = events.iter().filter(|e| e.seat == *seat).collect();
        mine.sort_by(|a, b| a.ts.cmp(&b.ts).then(a.seq.cmp(&b.seq)));
        let has_observed = mine
            .iter()
            .any(|e| spans.iter().any(|s| e.ts >= s.start && e.ts < s.end));

        let mut intervals = Vec::new();
        if has_observed {
            for span in &spans {
                let span_base = (span.start - window.from)
                    .num_seconds()
                    .clamp(0, window_secs);
                let life: Vec<&ActivityFact> = mine
                    .iter()
                    .copied()
                    .filter(|e| span.launched.is_none_or(|l| e.ts >= l))
                    .collect();
                let mut marks: Vec<DateTime<Utc>> = vec![span.start, span.end];
                for e in &life {
                    if e.ts > span.start && e.ts < span.end {
                        marks.push(e.ts);
                    }
                }
                marks.sort();
                marks.dedup();
                for pair in marks.windows(2) {
                    let (a, b) = (pair[0], pair[1]);
                    let off_a = (a - span.start).num_seconds();
                    let off_b = (b - span.start).num_seconds();
                    if off_b <= off_a {
                        continue;
                    }
                    let state = life
                        .iter()
                        .rev()
                        .find(|e| e.ts <= a)
                        .map(|e| e.state.as_str())
                        .unwrap_or("unknown");
                    if state == "running" {
                        let s = (span_base + off_a).clamp(0, window_secs);
                        let e = (span_base + off_b).clamp(s, window_secs);
                        if e > s {
                            intervals.push((s, e));
                            interval_marks.push(s);
                            interval_marks.push(e);
                        }
                    }
                }
            }
        }
        seat_active_intervals.push(intervals);
    }

    interval_marks.sort();
    interval_marks.dedup();

    // Initialize histogram buckets for k in 0..=seat_count.
    let mut bucket_secs = vec![0i64; seat_count + 1];

    // Sweep across all adjacent integer-second marks [s, e) in [0, window_secs).
    for pair in interval_marks.windows(2) {
        let (s, e) = (pair[0], pair[1]);
        let secs = e - s;
        if secs <= 0 {
            continue;
        }
        let active_seats = seat_active_intervals
            .iter()
            .filter(|intervals| intervals.iter().any(|&(is, ie)| s >= is && s < ie))
            .count();
        let k = active_seats.min(seat_count);
        bucket_secs[k] += secs;
    }

    let sum_secs: i64 = bucket_secs.iter().sum();
    let conserved = sum_secs == window_secs;

    let total_active_seat_secs: i64 = bucket_secs
        .iter()
        .enumerate()
        .map(|(k, &s)| (k as i64) * s)
        .sum();

    let effective_parallelism = if window_secs > 0 {
        Some(total_active_seat_secs as f64 / window_secs as f64)
    } else {
        None
    };

    let effective_parallelism_milli = if window_secs > 0 {
        ((total_active_seat_secs as f64 / window_secs as f64) * 1000.0).round() as u64
    } else {
        0
    };

    let fleet_duty_cycle = if seat_count > 0 && window_secs > 0 {
        Some(total_active_seat_secs as f64 / (seat_count as i64 * window_secs) as f64)
    } else {
        None
    };

    let fleet_duty_cycle_bps = if seat_count > 0 && window_secs > 0 {
        ((total_active_seat_secs as f64 / (seat_count as i64 * window_secs) as f64) * 10000.0)
            .round() as u64
    } else {
        0
    };

    let histogram: Vec<ConcurrencyBucket> = bucket_secs
        .into_iter()
        .enumerate()
        .map(|(k, secs)| {
            let share = if window_secs > 0 {
                secs as f64 / window_secs as f64
            } else {
                0.0
            };
            ConcurrencyBucket {
                active_seats: k,
                secs,
                share,
                conserved,
            }
        })
        .collect();

    FleetConcurrency {
        window_secs,
        seat_count,
        total_active_seat_secs,
        effective_parallelism,
        effective_parallelism_milli,
        fleet_duty_cycle,
        fleet_duty_cycle_bps,
        histogram,
        conserved,
    }
}

/// Convenience wrapper for `fleet_concurrency` taking string slices.
pub fn fleet_concurrency_scoped(
    window: Window,
    seats: &[&str],
    events: &[ActivityFact],
    bounds: &[SeatBound],
) -> FleetConcurrency {
    let owned: Vec<String> = seats.iter().map(|s| s.to_string()).collect();
    fleet_concurrency(window, &owned, events, bounds)
}

/// Sub-attributes a seat's `idle_with_work_secs` into `unparked_container_secs` and `leaf_stall_secs`.
///
/// An unparked container hold occurs when seat A is in the `idle_with_work` band, holds an
/// `in-progress` parent queue item, and at the same instant a downstream child item is
/// `in-progress` on another seat B != A linked via:
/// 1. `QueueItemMeta.parent_qitem_id` (direct or transitive),
/// 2. Same `chain_id` or `qitem_parent == qitem_child.chain_id`, or
/// 3. Created/handed-off by seat A (`source_seat == A`) or sharing non-empty `missions` or `issue_tags`.
pub fn disambiguate_container_claims(
    seat: &str,
    events: &[ActivityFact],
    queue: &[QueueFact],
    window: Window,
    lives: &[SeatBound],
    meta: &BTreeMap<String, QueueItemMeta>,
) -> SeatContainerDisambiguation {
    let bands = metrics::activity_bands(seat, events, queue, window, lives);
    disambiguate_container_claims_with_bands(seat, events, queue, window, lives, meta, &bands)
}

/// Disambiguates container claims using precomputed activity bands for efficiency.
pub fn disambiguate_container_claims_with_bands(
    seat: &str,
    events: &[ActivityFact],
    queue: &[QueueFact],
    window: Window,
    lives: &[SeatBound],
    meta: &BTreeMap<String, QueueItemMeta>,
    bands: &Bands,
) -> SeatContainerDisambiguation {
    if bands.idle_with_work_secs <= 0 {
        return SeatContainerDisambiguation {
            unparked_container_secs: 0,
            leaf_stall_secs: 0,
            container_hold_evidence: Vec::new(),
        };
    }

    let spans = metrics::observation_spans(window, lives);
    let mut mine: Vec<&ActivityFact> = events.iter().filter(|e| e.seat == seat).collect();
    mine.sort_by(|a, b| a.ts.cmp(&b.ts).then(a.seq.cmp(&b.seq)));

    let any_observed = mine
        .iter()
        .any(|e| spans.iter().any(|s| e.ts >= s.start && e.ts < s.end));
    if !any_observed {
        return SeatContainerDisambiguation {
            unparked_container_secs: 0,
            leaf_stall_secs: 0,
            container_hold_evidence: Vec::new(),
        };
    }

    let mut unparked_container_secs = 0i64;
    let mut leaf_stall_secs = 0i64;
    let mut evidence_map: BTreeMap<(String, String, String, String, String), i64> = BTreeMap::new();

    for span in &spans {
        let life: Vec<&ActivityFact> = mine
            .iter()
            .copied()
            .filter(|e| span.launched.is_none_or(|l| e.ts >= l))
            .collect();

        // Collect marks: span start/end, seat activity transitions, and all queue events.
        let mut marks: Vec<DateTime<Utc>> = vec![span.start, span.end];
        for e in &life {
            if e.ts > span.start && e.ts < span.end {
                marks.push(e.ts);
            }
        }
        for q in queue {
            if q.ts > span.start && q.ts < span.end {
                marks.push(q.ts);
            }
        }
        marks.sort();
        marks.dedup();

        for pair in marks.windows(2) {
            let (a, b) = (pair[0], pair[1]);
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
            let demand = metrics::idle_demand(seat, queue, a);

            // Disambiguation applies only inside the idle_with_work band.
            if state != "idle" || !demand {
                continue;
            }

            // Determine latest state of every queue item at timestamp a.
            let mut latest_by_item: BTreeMap<&str, &QueueFact> = BTreeMap::new();
            for fact in queue.iter().filter(|q| q.ts <= a) {
                match latest_by_item.get(fact.qitem_id.as_str()) {
                    Some(existing) => {
                        if (fact.ts, fact.transition_id) > (existing.ts, existing.transition_id) {
                            latest_by_item.insert(fact.qitem_id.as_str(), fact);
                        }
                    }
                    None => {
                        latest_by_item.insert(fact.qitem_id.as_str(), fact);
                    }
                }
            }

            // Parent candidates: claimed in-progress on seat A.
            let parents: Vec<&QueueFact> = latest_by_item
                .values()
                .copied()
                .filter(|q| q.dest_seat == seat && q.state == "in-progress")
                .collect();

            // Child candidates: in-progress on seat B != A.
            let children: Vec<&QueueFact> = latest_by_item
                .values()
                .copied()
                .filter(|q| q.dest_seat != seat && q.state == "in-progress")
                .collect();

            // Check if any (parent, child) pair matches the linkage criteria.
            // Priority: rule 1 (hierarchical metadata) > rule 2 (chain id) > rule 3 (creator / missions / issue tags).
            let mut best_match: Option<(u8, &QueueFact, &QueueFact)> = None;

            for parent in &parents {
                for child in &children {
                    if child.qitem_id == parent.qitem_id {
                        continue;
                    }

                    // Rule 1: QueueItemMeta.parent_qitem_id == Some(parent) or transitive ancestor.
                    let rule1 = is_transitive_parent(meta, &child.qitem_id, &parent.qitem_id);

                    // Rule 2: Same chain_id or parent == child.chain_id or child == parent.chain_id.
                    let rule2 = (child.chain_id.is_some() && child.chain_id == parent.chain_id)
                        || child.chain_id.as_deref() == Some(&parent.qitem_id)
                        || parent.chain_id.as_deref() == Some(&child.qitem_id);

                    // Rule 3: Created/handed-off by seat A while parent was claimed, for the same mission/issue or direct delegation.
                    // Automatic fallback routing (`qitem.fallback_routed`) records `originalDestination` in `source_seat` and is not causal action by A.
                    let is_causal_by_a = |q: &QueueFact| {
                        q.qitem_id == child.qitem_id
                            && q.source_seat == seat
                            && !q.source_key.contains("fallback_routed")
                            && q.ts >= parent.ts
                            && q.ts <= a
                    };
                    let created_or_handed_off_by_a_while_parent_claimed =
                        queue.iter().any(is_causal_by_a);
                    let shared_missions = !child.missions.is_empty()
                        && child
                            .missions
                            .iter()
                            .any(|m| !m.is_empty() && parent.missions.contains(m));
                    let child_issues = meta.get(&child.qitem_id).map(|m| &m.issue_tags);
                    let parent_issues = meta.get(&parent.qitem_id).map(|m| &m.issue_tags);
                    let shared_issues = match (child_issues, parent_issues) {
                        (Some(c), Some(p)) => {
                            !c.is_empty() && c.iter().any(|t| !t.is_empty() && p.contains(t))
                        }
                        _ => false,
                    };
                    let direct_delegation_from_a = queue
                        .iter()
                        .any(|q| is_causal_by_a(q) && q.dest_seat == child.dest_seat);
                    let rule3 = created_or_handed_off_by_a_while_parent_claimed
                        && (shared_missions || shared_issues || direct_delegation_from_a);

                    let rule_tier = if rule1 {
                        Some(1u8)
                    } else if rule2 {
                        Some(2u8)
                    } else if rule3 {
                        Some(3u8)
                    } else {
                        None
                    };

                    if let Some(tier) = rule_tier {
                        let is_better = match &best_match {
                            None => true,
                            Some((curr_tier, curr_p, curr_c)) => {
                                (tier, &parent.qitem_id, &child.qitem_id)
                                    < (*curr_tier, &curr_p.qitem_id, &curr_c.qitem_id)
                            }
                        };
                        if is_better {
                            best_match = Some((tier, parent, child));
                        }
                    }
                }
            }

            if let Some((_, p, c)) = best_match {
                unparked_container_secs += secs;
                let key = (
                    p.qitem_id.clone(),
                    p.source_key.clone(),
                    c.qitem_id.clone(),
                    c.dest_seat.clone(),
                    c.source_key.clone(),
                );
                *evidence_map.entry(key).or_insert(0) += secs;
            } else {
                leaf_stall_secs += secs;
            }
        }
    }

    // Clamp and conserve exactly to bands.idle_with_work_secs.
    let total_disambiguated = unparked_container_secs + leaf_stall_secs;
    if total_disambiguated != bands.idle_with_work_secs {
        // If sub-second marks caused any discrepancy with whole-second bands, reconcile.
        unparked_container_secs = unparked_container_secs
            .min(bands.idle_with_work_secs)
            .max(0);
        leaf_stall_secs = bands.idle_with_work_secs - unparked_container_secs;
    }

    let container_hold_evidence = evidence_map
        .into_iter()
        .map(
            |(
                (parent_qitem_id, parent_source_key, child_qitem_id, child_seat, child_source_key),
                secs,
            )| {
                ContainerHoldEvidence {
                    parent_qitem_id,
                    parent_source_key,
                    child_qitem_id,
                    child_seat,
                    child_source_key,
                    secs,
                }
            },
        )
        .collect();

    SeatContainerDisambiguation {
        unparked_container_secs,
        leaf_stall_secs,
        container_hold_evidence,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn t(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000 + secs, 0).unwrap()
    }

    fn win(from_secs: i64, to_secs: i64) -> Window {
        Window {
            from: t(from_secs),
            to: t(to_secs),
        }
    }

    fn act(seat: &str, state: &str, secs: i64, seq: i64) -> ActivityFact {
        ActivityFact {
            source_key: format!("act:{seat}:{secs}"),
            seat: seat.to_string(),
            ts: t(secs),
            state: state.to_string(),
            seq,
        }
    }

    fn qfact(id: &str, state: &str, src: &str, dest: &str, secs: i64, trans_id: i64) -> QueueFact {
        QueueFact {
            source_key: format!("q:{id}:{secs}"),
            transition_id: trans_id,
            qitem_id: id.to_string(),
            ts: t(secs),
            state: state.to_string(),
            source_seat: src.to_string(),
            dest_seat: dest.to_string(),
            closure_reason: None,
            closure_target: None,
            chain_id: None,
            missions: Vec::new(),
        }
    }

    #[test]
    fn concurrency_distribution_conservation_and_parallelism() {
        // Window of 100 seconds: t(0) to t(100).
        let window = win(0, 100);
        let seats = vec!["seat-a".to_string(), "seat-b".to_string()];

        // seat-a is running from 10 to 40 (30s)
        // seat-b is running from 20 to 60 (40s)
        // Overlap:
        // [0, 10): 0 active (10s)
        // [10, 20): 1 active (seat-a, 10s)
        // [20, 40): 2 active (seat-a & seat-b, 20s)
        // [40, 60): 1 active (seat-b, 20s)
        // [60, 100): 0 active (40s)
        //
        // Total bucket 0: 10 + 40 = 50s
        // Total bucket 1: 10 + 20 = 30s
        // Total bucket 2: 20s
        // Conservation: 50 + 30 + 20 = 100s == window_secs!
        // Total active seat-seconds: 0*50 + 1*30 + 2*20 = 70s.
        // Parallelism: 70 / 100 = 0.70 (700 milli)
        // Fleet duty cycle: 70 / (2 * 100) = 0.35 (3500 bps)

        let events = vec![
            act("seat-a", "idle", 0, 1),
            act("seat-a", "running", 10, 2),
            act("seat-a", "idle", 40, 3),
            act("seat-b", "idle", 0, 1),
            act("seat-b", "running", 20, 2),
            act("seat-b", "idle", 60, 3),
        ];

        let bounds = Vec::new();
        let fc = fleet_concurrency(window, &seats, &events, &bounds);

        assert_eq!(fc.window_secs, 100);
        assert_eq!(fc.seat_count, 2);
        assert!(fc.conserved);
        assert_eq!(fc.total_active_seat_secs, 70);
        assert_eq!(fc.effective_parallelism, Some(0.70));
        assert_eq!(fc.effective_parallelism_milli, 700);
        assert_eq!(fc.fleet_duty_cycle, Some(0.35));
        assert_eq!(fc.fleet_duty_cycle_bps, 3500);

        assert_eq!(fc.histogram.len(), 3);
        assert_eq!(fc.histogram[0].active_seats, 0);
        assert_eq!(fc.histogram[0].secs, 50);
        assert!((fc.histogram[0].share - 0.50).abs() < 1e-6);
        assert!(fc.histogram[0].conserved);

        assert_eq!(fc.histogram[1].active_seats, 1);
        assert_eq!(fc.histogram[1].secs, 30);
        assert!((fc.histogram[1].share - 0.30).abs() < 1e-6);
        assert!(fc.histogram[1].conserved);

        assert_eq!(fc.histogram[2].active_seats, 2);
        assert_eq!(fc.histogram[2].secs, 20);
        assert!((fc.histogram[2].share - 0.20).abs() < 1e-6);
        assert!(fc.histogram[2].conserved);
    }

    #[test]
    fn concurrency_with_lifecycle_bounds_and_stops() {
        let window = win(0, 100);
        let seats = vec!["seat-a".to_string(), "seat-b".to_string()];

        // seat-a created at 10, stopped at 50
        // seat-b created at 0, no stop
        let bounds = vec![
            SeatBound {
                seat: "seat-a".to_string(),
                created_at: Some(t(10)),
                stopped_at: Some(t(50)),
            },
            SeatBound {
                seat: "seat-b".to_string(),
                created_at: Some(t(0)),
                stopped_at: None,
            },
        ];

        // seat-a runs from 15 to 60 (clipped by stop at 50 -> runs 15..50, 35s)
        // seat-b runs from 40 to 80 (runs 40..80, 40s)
        // Overlap:
        // [0, 15): 0 active (15s)
        // [15, 40): 1 active (seat-a, 25s)
        // [40, 50): 2 active (seat-a & seat-b, 10s)
        // [50, 80): 1 active (seat-b, 30s)
        // [80, 100): 0 active (20s)
        //
        // Bucket 0: 15 + 20 = 35s
        // Bucket 1: 25 + 30 = 55s
        // Bucket 2: 10s
        // Sum = 35 + 55 + 10 = 100s == window_secs!
        // Active seat secs: 0*35 + 1*55 + 2*10 = 75s.

        let events = vec![
            act("seat-a", "running", 15, 1),
            act("seat-a", "idle", 60, 2),
            act("seat-b", "idle", 0, 1),
            act("seat-b", "running", 40, 2),
            act("seat-b", "idle", 80, 3),
        ];

        let fc = fleet_concurrency(window, &seats, &events, &bounds);
        assert_eq!(fc.window_secs, 100);
        assert!(fc.conserved);
        assert_eq!(fc.total_active_seat_secs, 75);
        assert_eq!(fc.histogram[0].secs, 35);
        assert_eq!(fc.histogram[1].secs, 55);
        assert_eq!(fc.histogram[2].secs, 10);
    }

    #[test]
    fn concurrency_fractional_lifecycle_matches_canonical_active_secs() {
        let window = win(0, 3);
        let seats = vec!["seat-a".to_string()];
        let bounds = vec![SeatBound {
            seat: "seat-a".to_string(),
            created_at: Some(t(0) + chrono::Duration::milliseconds(500)),
            stopped_at: Some(t(2)),
        }];
        let events = vec![ActivityFact {
            source_key: "act:seat-a:0.5".to_string(),
            seat: "seat-a".to_string(),
            ts: t(0) + chrono::Duration::milliseconds(500),
            state: "running".to_string(),
            seq: 1,
        }];
        let bands = metrics::activity_bands("seat-a", &events, &[], window, &bounds);
        let fc = fleet_concurrency(window, &seats, &events, &bounds);
        assert_eq!(bands.active_secs, 1);
        assert_eq!(fc.total_active_seat_secs, bands.active_secs);
        assert_eq!(fc.histogram[0].secs, 2);
        assert_eq!(fc.histogram[1].secs, 1);
        assert!(fc.conserved);
    }

    #[test]
    fn container_vs_leaf_claim_disambiguation_rules_and_traceability() {
        let window = win(0, 100);
        let seat = "orch.lead";
        let child_seat = "dev.build";

        // seat orch.lead is idle throughout 0..100
        let events = vec![act(seat, "idle", 0, 1), act(child_seat, "running", 0, 1)];

        // orch.lead holds parent item Q-100 in-progress from t=10 to t=80
        // child item Q-200 is in-progress on dev.build from t=20 to t=50
        //
        // Idle with work for orch.lead is [10, 80) -> 70 seconds.
        // During [10, 20): Q-100 in progress on orch, no child in progress -> leaf_stall (10s)
        // During [20, 50): Q-100 in progress on orch, Q-200 in progress on dev.build -> unparked_container (30s)
        // During [50, 80): Q-100 in progress on orch, Q-200 completed -> leaf_stall (30s)
        // Total unparked_container_secs = 30s
        // Total leaf_stall_secs = 40s
        // Sum = 70s == idle_with_work_secs!

        let queue = vec![
            // Parent item claimed by orch.lead at t=10
            qfact("Q-100", "in-progress", seat, seat, 10, 1),
            // Child item created and claimed by dev.build at t=20
            qfact("Q-200", "in-progress", seat, child_seat, 20, 2),
            // Child item completed at t=50
            qfact("Q-200", "closed", child_seat, child_seat, 50, 3),
            // Parent item closed at t=80
            qfact("Q-100", "closed", seat, seat, 80, 4),
        ];

        let mut meta = BTreeMap::new();
        meta.insert(
            "Q-200".to_string(),
            QueueItemMeta {
                qitem_id: "Q-200".to_string(),
                parent_qitem_id: Some("Q-100".to_string()),
                ..QueueItemMeta::default()
            },
        );

        let disambiguation =
            disambiguate_container_claims(seat, &events, &queue, window, &[], &meta);

        assert_eq!(disambiguation.unparked_container_secs, 30);
        assert_eq!(disambiguation.leaf_stall_secs, 40);
        assert_eq!(
            disambiguation.unparked_container_secs + disambiguation.leaf_stall_secs,
            70
        );

        assert_eq!(disambiguation.container_hold_evidence.len(), 1);
        let ev = &disambiguation.container_hold_evidence[0];
        assert_eq!(ev.parent_qitem_id, "Q-100");
        assert_eq!(ev.parent_source_key, "q:Q-100:10");
        assert_eq!(ev.child_qitem_id, "Q-200");
        assert_eq!(ev.child_seat, "dev.build");
        assert_eq!(ev.child_source_key, "q:Q-200:20");
        assert_eq!(ev.secs, 30);
    }

    #[test]
    fn container_disambiguation_rule2_chain_id_and_rule3_missions() {
        let window = win(0, 100);
        let seat = "orch.lead";
        let child_seat = "dev.build";

        let events = vec![act(seat, "idle", 0, 1)];

        // Test Rule 2 (same chain_id)
        let mut q_parent = qfact("P-1", "in-progress", seat, seat, 0, 1);
        q_parent.chain_id = Some("chain-alpha".to_string());
        let mut q_child = qfact("C-1", "in-progress", child_seat, child_seat, 10, 2);
        q_child.chain_id = Some("chain-alpha".to_string());
        let q_child_done = qfact("C-1", "closed", child_seat, child_seat, 30, 3);
        let q_parent_done = qfact("P-1", "closed", seat, seat, 50, 4);

        let queue = vec![q_parent, q_child, q_child_done, q_parent_done];
        let meta = BTreeMap::new();

        let disambiguation =
            disambiguate_container_claims(seat, &events, &queue, window, &[], &meta);

        // idle_with_work from 0..50 (50s).
        // [0, 10): leaf stall (10s)
        // [10, 30): unparked container via same chain_id (20s)
        // [30, 50): leaf stall (20s)
        assert_eq!(disambiguation.unparked_container_secs, 20);
        assert_eq!(disambiguation.leaf_stall_secs, 30);
        assert_eq!(
            disambiguation.unparked_container_secs + disambiguation.leaf_stall_secs,
            50
        );
        assert_eq!(disambiguation.container_hold_evidence.len(), 1);
        assert_eq!(
            disambiguation.container_hold_evidence[0].child_qitem_id,
            "C-1"
        );

        // Negative test for Rule 3: unrelated item on child_seat NOT created by seat A,
        // even if it shares a mission tag, remains leaf_stall_secs.
        let mut p2 = qfact("P-2", "in-progress", "other", seat, 0, 1);
        p2.missions = vec!["mission:shared".to_string()];
        let mut c2 = qfact("C-2", "in-progress", "other", child_seat, 20, 2);
        c2.missions = vec!["mission:shared".to_string()];
        let disamb_unrelated = disambiguate_container_claims(
            seat,
            &events,
            &[p2.clone(), c2.clone()],
            window,
            &[],
            &meta,
        );
        assert_eq!(disamb_unrelated.unparked_container_secs, 0);
        assert_eq!(disamb_unrelated.leaf_stall_secs, 100);

        // Negative test for automatic fallback routing (`qitem.fallback_routed`) where `source_seat == seat`
        // records `originalDestination` rather than causal creation/handoff by seat A.
        let mut fallback = qfact("C-2", "pending", seat, child_seat, 10, 2);
        fallback.source_key = "events:qitem.fallback_routed:3".to_string();
        fallback.missions = vec!["mission:shared".to_string()];
        let disamb_fallback =
            disambiguate_container_claims(seat, &events, &[p2, fallback, c2], window, &[], &meta);
        assert_eq!(disamb_fallback.unparked_container_secs, 0);
        assert_eq!(disamb_fallback.leaf_stall_secs, 100);
    }
}
