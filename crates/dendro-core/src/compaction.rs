//! Context depth, compaction hazards and cache inherited across slices (#54).
//! Usage buckets are already per-turn deltas, including Codex. Cumulative
//! predecessor counters must never be added to a turn's context size.
use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{json, Value};

use crate::metrics::{self, SessionBinding, Window};
use crate::turnaround::{lineage, Lineage};
use crate::{DestinationSnapshot, QueueFact, QueueItemMeta, UsageFact};

pub struct Input<'a> {
    /// Accepted history, including turns before the report window.
    pub usage: &'a [UsageFact],
    pub queue: &'a [QueueFact],
    /// Unscoped queue history permits lineage across rig boundaries.
    pub history_queue: &'a [QueueFact],
    pub meta: &'a BTreeMap<String, QueueItemMeta>,
    pub snapshots: &'a BTreeMap<String, Vec<DestinationSnapshot>>,
    pub bindings: &'a BTreeMap<(String, String), SessionBinding>,
    pub rig: Option<&'a str>,
    pub window: Window,
}

pub fn turn_context_tokens(u: &UsageFact) -> i64 {
    u.input_uncached + u.cache_read + u.cache_write
}

#[derive(Debug, Serialize)]
pub struct CompactionEvent {
    pub seat: Option<String>,
    pub rig: Option<String>,
    pub harness: String,
    pub session: String,
    pub ts: DateTime<Utc>,
    pub pre_drop_context_tokens: i64,
    pub post_drop_context_tokens: i64,
    pub dropped_tokens: i64,
    pub drop_ratio: f64,
    pub active_item_id: Option<String>,
    pub active_item_ids: Vec<String>,
    pub hazard_class: &'static str,
    pub pre_source_key: String,
    pub post_source_key: String,
    pub queue_source_keys: BTreeSet<String>,
}

#[derive(Debug, Serialize)]
pub struct CrossSliceBleedEvent {
    pub seat: String,
    pub rig: Option<String>,
    pub harness: String,
    pub session: String,
    pub from_slice: String,
    pub to_slice: String,
    pub entry_ts: DateTime<Utc>,
    pub prior_slice_peak_context_tokens: i64,
    pub prior_slice_exit_context_tokens: i64,
    pub entry_context_tokens: i64,
    pub inherited_cache_read_tokens: i64,
    pub cumulative_bleed_tokens: i64,
    pub source_keys: BTreeSet<String>,
}

const BRACKETS: [&str; 3] = ["under_100k", "100k_to_250k", "over_250k"];

#[derive(Default, Serialize)]
struct Depth {
    turn_count: usize,
    total_tokens: i64,
    input_context_tokens: i64,
    source_keys: Vec<String>,
}

struct Distribution(BTreeMap<&'static str, Depth>);

impl Default for Distribution {
    fn default() -> Self {
        Self(
            BRACKETS
                .into_iter()
                .map(|b| (b, Depth::default()))
                .collect(),
        )
    }
}

impl Distribution {
    fn add(&mut self, u: &UsageFact) {
        let context = turn_context_tokens(u);
        let bracket = if context < 100_000 {
            BRACKETS[0]
        } else if context <= 250_000 {
            BRACKETS[1]
        } else {
            BRACKETS[2]
        };
        let d = self.0.get_mut(bracket).expect("all brackets initialized");
        d.turn_count += 1;
        d.total_tokens += context + u.output;
        d.input_context_tokens += context;
        d.source_keys.push(u.source_key.clone());
    }

    fn value(&self) -> Value {
        let turns: usize = self.0.values().map(|d| d.turn_count).sum();
        let tokens: i64 = self.0.values().map(|d| d.total_tokens).sum();
        let mut result = json!(self.0);
        for bracket in BRACKETS {
            let d = &self.0[bracket];
            result[bracket]["turn_share"] =
                json!((turns != 0).then(|| d.turn_count as f64 / turns as f64));
            result[bracket]["token_share"] =
                json!((tokens != 0).then(|| d.total_tokens as f64 / tokens as f64));
        }
        result
    }
}

#[derive(Default)]
struct Active {
    items: Vec<String>,
    slice: Option<String>,
    source_keys: BTreeSet<String>,
}

/// Use the same point-in-time state and K5 destination proof as turnaround/K3.
/// Conflicting slice tags are not resolved by arbitrarily selecting an item.
fn active_at(
    input: &Input<'_>,
    lineages: &BTreeMap<String, Lineage>,
    seat: &str,
    ts: DateTime<Utc>,
) -> Active {
    let evidence = metrics::mission_evidence(seat, input.queue, input.snapshots, ts);
    let mut active = Active::default();
    let (mut missions, mut issues) = (BTreeSet::new(), BTreeSet::new());
    let mut evidence_items = BTreeSet::new();
    for item in evidence.items.iter().filter(|i| i.relation == "owned") {
        active.items.push(item.qitem.clone());
        evidence_items.insert(item.qitem.as_str());
        if let Some(l) = lineages.get(&item.qitem) {
            missions.extend(l.missions.iter().map(String::as_str));
            issues.extend(l.issues.iter().map(String::as_str));
            evidence_items.extend(l.ancestors.iter().map(String::as_str));
        }
    }
    // Include state/claim, K5 destination and lineage locators. Future transitions
    // are excluded so an event's explanation reflects its own instant.
    active.source_keys.extend(
        input
            .history_queue
            .iter()
            .filter(|q| q.ts <= ts && evidence_items.contains(q.qitem_id.as_str()))
            .map(|q| q.source_key.clone()),
    );
    for id in &evidence_items {
        for snapshot in input.snapshots.get(*id).into_iter().flatten() {
            active.source_keys.insert(format!(
                "queue_items:{id}:{}:{}:{}",
                snapshot.destination,
                snapshot.cutoff,
                json!(snapshot.missions)
            ));
        }
    }
    // Issues are more specific than missions: two issues under one mission
    // must still constitute a slice change. Ambiguous issues stay unresolved.
    active.slice = if issues.len() == 1 {
        issues.first().map(|i| format!("issue:{i}"))
    } else if issues.is_empty() && missions.len() == 1 {
        missions.first().map(|m| format!("mission:{m}"))
    } else {
        None
    };
    active
}

fn is_drop(pre: i64, post: i64) -> bool {
    pre >= 50_000 && post < pre && pre - post >= 50_000 && pre - post >= pre / 2 + pre % 2
}

struct Slice<'a> {
    name: String,
    last: &'a UsageFact,
    peak: i64,
    keys: BTreeSet<String>,
}

#[derive(Default, Serialize)]
struct CompactionCounts {
    mid_item_hazard_count: usize,
    boundary_reset_count: usize,
}

#[derive(Default, Serialize)]
struct BleedTotals {
    total_inherited_entry_tokens: i64,
    total_cumulative_bleed_tokens: i64,
}

/// Compute every projection from a single accepted snapshot. Events are scoped
/// by their entry/drop timestamp; depth counts include only in-window turns.
pub fn evaluate(input: Input<'_>) -> Value {
    let lineages = lineage(input.history_queue, input.meta, input.snapshots);
    let mut sessions = BTreeMap::<(&str, &str), Vec<&UsageFact>>::new();
    for u in input.usage {
        let bound = metrics::binding_of(input.bindings, &u.harness, &u.native_session);
        if u.ts.is_some_and(|t| t < input.window.to)
            && input.rig.is_none_or(|r| bound.rig() == Some(r))
        {
            sessions
                .entry((&u.harness, &u.native_session))
                .or_default()
                .push(u);
        }
    }
    let mut compactions = Vec::new();
    let mut bleeds: Vec<CrossSliceBleedEvent> = Vec::new();
    let mut overall = Distribution::default();
    let mut seat_depth = BTreeMap::<&str, Distribution>::new();
    let mut session_depth = Vec::new();
    for ((harness, session), turns) in &mut sessions {
        turns.sort_by_key(|u| (u.ts, u.seq, &u.source_key));
        let bound = metrics::binding_of(input.bindings, harness, session);
        let mut previous: Option<&UsageFact> = None;
        let mut slice: Option<Slice<'_>> = None;
        let mut bleed_index: Option<usize> = None;
        let mut depth = Distribution::default();
        let mut window_turns = 0;
        for &u in turns.iter() {
            let ts = u.ts.expect("timestamped history");
            let in_window = ts >= input.window.from;
            let context = turn_context_tokens(u);
            let active = bound
                .seat()
                .map(|s| active_at(&input, &lineages, s, ts))
                .unwrap_or_default();
            let drop = previous.is_some_and(|p| is_drop(turn_context_tokens(p), context));
            if drop {
                // Any compaction ends accumulation; only a boundary reset breaks
                // slice continuity and suppresses the next transition event.
                bleed_index = None;
                if active.items.is_empty() {
                    slice = None;
                }
                if in_window {
                    let p = previous.expect("drop has a predecessor");
                    let pre = turn_context_tokens(p);
                    compactions.push(CompactionEvent {
                        seat: bound.seat().map(str::to_owned),
                        rig: bound.rig().map(str::to_owned),
                        harness: (*harness).into(),
                        session: (*session).into(),
                        ts,
                        pre_drop_context_tokens: pre,
                        post_drop_context_tokens: context,
                        dropped_tokens: pre - context,
                        drop_ratio: (pre - context) as f64 / pre as f64,
                        active_item_id: active.items.first().cloned(),
                        active_item_ids: active.items.clone(),
                        hazard_class: if active.items.is_empty() {
                            "boundary_reset"
                        } else {
                            "mid_item_hazard"
                        },
                        pre_source_key: p.source_key.clone(),
                        post_source_key: u.source_key.clone(),
                        queue_source_keys: active.source_keys.clone(),
                    });
                }
            }
            if let (Some(seat), Some(name)) = (bound.seat(), active.slice) {
                if let Some(prior) = &slice {
                    if prior.name != name {
                        let exit = turn_context_tokens(prior.last);
                        let mut keys = prior.keys.clone();
                        keys.extend(active.source_keys.iter().cloned());
                        keys.insert(prior.last.source_key.clone());
                        keys.insert(u.source_key.clone());
                        bleeds.push(CrossSliceBleedEvent {
                            seat: seat.into(),
                            rig: bound.rig().map(str::to_owned),
                            harness: (*harness).into(),
                            session: (*session).into(),
                            from_slice: prior.name.clone(),
                            to_slice: name.clone(),
                            entry_ts: ts,
                            prior_slice_peak_context_tokens: prior.peak,
                            prior_slice_exit_context_tokens: exit,
                            entry_context_tokens: context,
                            inherited_cache_read_tokens: u.cache_read.min(exit),
                            cumulative_bleed_tokens: 0,
                            source_keys: keys,
                        });
                        bleed_index = Some(bleeds.len() - 1);
                    }
                }
                if let Some(index) = bleed_index {
                    let event = &mut bleeds[index];
                    event.cumulative_bleed_tokens +=
                        u.cache_read.min(event.prior_slice_exit_context_tokens);
                    event.source_keys.insert(u.source_key.clone());
                    event.source_keys.extend(active.source_keys.iter().cloned());
                }
                match &mut slice {
                    Some(s) if s.name == name => {
                        s.last = u;
                        s.peak = s.peak.max(context);
                        s.keys.extend(active.source_keys);
                    }
                    _ => {
                        slice = Some(Slice {
                            name,
                            last: u,
                            peak: context,
                            keys: active.source_keys,
                        })
                    }
                }
            }
            if in_window {
                window_turns += 1;
                overall.add(u);
                depth.add(u);
                if let Some(seat) = bound.seat() {
                    seat_depth.entry(seat).or_default().add(u);
                }
            }
            previous = Some(u);
        }
        if window_turns != 0 {
            session_depth.push(json!({"harness": harness, "session": session,
                "seat": bound.seat(), "rig": bound.rig(), "brackets": depth.value()}));
        }
    }
    compactions.sort_by(|a, b| {
        (a.ts, &a.harness, &a.session, &a.post_source_key).cmp(&(
            b.ts,
            &b.harness,
            &b.session,
            &b.post_source_key,
        ))
    });
    bleeds.retain(|e| e.entry_ts >= input.window.from);
    bleeds.sort_by(|a, b| {
        (a.entry_ts, &a.harness, &a.session).cmp(&(b.entry_ts, &b.harness, &b.session))
    });
    let mut counts = CompactionCounts::default();
    let mut seat_counts = BTreeMap::<&str, CompactionCounts>::new();
    for e in &compactions {
        let add = |c: &mut CompactionCounts| {
            if e.hazard_class == "mid_item_hazard" {
                c.mid_item_hazard_count += 1;
            } else {
                c.boundary_reset_count += 1;
            }
        };
        add(&mut counts);
        if let Some(seat) = e.seat.as_deref() {
            add(seat_counts.entry(seat).or_default());
        }
    }
    let mut totals = BleedTotals::default();
    let mut seat_bleed = BTreeMap::<&str, BleedTotals>::new();
    for e in &bleeds {
        for t in [&mut totals, seat_bleed.entry(&e.seat).or_default()] {
            t.total_inherited_entry_tokens += e.inherited_cache_read_tokens;
            t.total_cumulative_bleed_tokens += e.cumulative_bleed_tokens;
        }
    }
    json!({
        "context_compactions": {
            "events": compactions, "mid_item_hazard_count": counts.mid_item_hazard_count,
            "boundary_reset_count": counts.boundary_reset_count,
            "by_seat": seat_counts.iter().map(|(seat, c)| json!({"seat": seat,
                "mid_item_hazard_count": c.mid_item_hazard_count, "boundary_reset_count": c.boundary_reset_count})).collect::<Vec<_>>(),
            "definition": "Consecutive turns within (harness, native_session), ordered by (ts, seq, source_key); a drop of at least 50,000 tokens and 50%, reported at the post-drop turn in [from,to). Context = input_uncached + cache_read + cache_write; Codex prior counters are cumulative and are not added. Primary active item is the lexicographically first proven K5 owner."
        },
        "context_depth_brackets": {
            "overall": overall.value(),
            "by_seat": seat_depth.iter().map(|(seat, d)| json!({"seat": seat, "brackets": d.value()})).collect::<Vec<_>>(),
            "by_session": session_depth,
            "definition": "In-window timestamped turns: under_100k <100000, 100k_to_250k 100000..=250000, over_250k >250000. Token shares use all four token buckets within each group; zero denominators are null. Unlinked turns appear in overall and by_session."
        },
        "cross_slice_cache_bleed": {
            "events": bleeds, "total_inherited_entry_tokens": totals.total_inherited_entry_tokens,
            "total_cumulative_bleed_tokens": totals.total_cumulative_bleed_tokens,
            "by_seat": seat_bleed.iter().map(|(seat, t)| json!({"seat": seat,
                "total_inherited_entry_tokens": t.total_inherited_entry_tokens,
                "total_cumulative_bleed_tokens": t.total_cumulative_bleed_tokens})).collect::<Vec<_>>(),
            "definition": "Events enter a new resolved slice in [from,to) within one linked native session. A single issue takes precedence over a single mission; conflicting tags remain unresolved. Boundary compactions break slice continuity; any compaction stops ongoing accumulation. Cumulative tokens cover resolved turns of the destination slice until the next slice or compaction, clipped at to. Each turn contributes min(cache_read, prior slice exit context)."
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::IdentityFact;

    fn at(second: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(second, 0).unwrap()
    }

    pub(super) fn usage(key: &str, second: i64, context: i64, cached: i64) -> UsageFact {
        serde_json::from_value(json!({
            "source_key": key, "harness": "codex", "native_session": "persistent",
            "ts": at(second), "seq": second, "input_uncached": context - cached,
            "cache_read": cached, "cache_write": 0, "output": 1000,
            "reasoning": 500, "generation": 1
        }))
        .unwrap()
    }

    #[derive(Default)]
    struct Fixture {
        usage: Vec<UsageFact>,
        queue: Vec<QueueFact>,
        snapshots: BTreeMap<String, Vec<DestinationSnapshot>>,
        meta: BTreeMap<String, QueueItemMeta>,
        bindings: BTreeMap<(String, String), SessionBinding>,
    }

    impl Fixture {
        fn linked() -> Self {
            Self {
                bindings: metrics::session_bindings(&[IdentityFact {
                    source_key: "identity".into(),
                    harness: "codex".into(),
                    native_session: "persistent".into(),
                    seat: "builder".into(),
                    rig: "worker".into(),
                    ts: Some(at(0)),
                }]),
                ..Self::default()
            }
        }

        fn item(&mut self, id: &str, start: i64, end: i64, missions: &[&str]) {
            for (second, state, key) in [
                (start - 1, "pending", format!("events:queue.created:{id}")),
                (start, "in-progress", format!("claim:{id}")),
                (end, "done", format!("done:{id}")),
            ] {
                self.queue.push(QueueFact {
                    source_key: key,
                    transition_id: second,
                    qitem_id: id.into(),
                    ts: at(second),
                    state: state.into(),
                    source_seat: "actor".into(),
                    dest_seat: "builder".into(),
                    closure_reason: None,
                    closure_target: None,
                    chain_id: None,
                    missions: missions.iter().map(|m| (*m).into()).collect(),
                });
            }
            self.snapshots.insert(
                id.into(),
                vec![DestinationSnapshot {
                    destination: "builder".into(),
                    cutoff: at(end).to_rfc3339(),
                    reroute_marked: false,
                    missions: missions.iter().map(|m| (*m).into()).collect(),
                }],
            );
        }

        fn evaluate(&self, from: i64, to: i64) -> Value {
            evaluate(Input {
                usage: &self.usage,
                queue: &self.queue,
                history_queue: &self.queue,
                snapshots: &self.snapshots,
                meta: &self.meta,
                bindings: &self.bindings,
                rig: None,
                window: Window {
                    from: at(from),
                    to: at(to),
                },
            })
        }
    }

    #[test]
    fn mid_item_drop_uses_history_and_preserves_all_active_item_locators() {
        let mut f = Fixture::linked();
        f.item("a", 1, 20, &["slice"]);
        f.item("b", 2, 20, &["slice"]);
        f.usage = vec![
            usage("pre:1", 5, 325_000, 300_000),
            usage("post:2", 10, 34_000, 30_000),
        ];
        let v = f.evaluate(10, 11);
        let e = &v["context_compactions"]["events"][0];
        assert_eq!(e["pre_drop_context_tokens"], 325_000);
        assert_eq!(e["post_drop_context_tokens"], 34_000);
        assert_eq!(e["dropped_tokens"], 291_000);
        assert_eq!(e["drop_ratio"], json!(291_000.0 / 325_000.0));
        assert_eq!(e["seat"], "builder");
        assert_eq!(e["rig"], "worker");
        assert_eq!(e["hazard_class"], "mid_item_hazard");
        assert_eq!(e["active_item_id"], "a");
        assert_eq!(e["active_item_ids"], json!(["a", "b"]));
        assert_eq!(e["pre_source_key"], "pre:1");
        assert_eq!(e["post_source_key"], "post:2");
        let keys = e["queue_source_keys"].as_array().unwrap();
        for key in [
            "claim:a",
            "claim:b",
            "events:queue.created:a",
            "events:queue.created:b",
        ] {
            assert!(keys.contains(&json!(key)));
        }
        assert!(!keys.contains(&json!("done:a")));
        assert_eq!(v["context_compactions"]["mid_item_hazard_count"], 1);
        assert_eq!(
            v["context_compactions"]["by_seat"][0]["mid_item_hazard_count"],
            1
        );
        assert_eq!(
            v["context_depth_brackets"]["overall"]["under_100k"]["turn_count"],
            1
        );
        assert_eq!(
            v["context_depth_brackets"]["overall"]["over_250k"]["turn_count"],
            0
        );
        assert!(f.evaluate(0, 10)["context_compactions"]["events"]
            .as_array()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn completed_unclaimed_and_unlinked_turns_are_boundary_resets() {
        let mut f = Fixture::linked();
        f.item("a", 1, 10, &["A"]);
        f.usage = vec![usage("pre", 5, 325_000, 0), usage("post", 10, 34_000, 0)];
        let v = f.evaluate(0, 20);
        assert_eq!(v["context_compactions"]["boundary_reset_count"], 1);
        assert_eq!(
            v["context_compactions"]["events"][0]["active_item_id"],
            Value::Null
        );
        assert_eq!(
            v["context_compactions"]["events"][0]["active_item_ids"],
            json!([])
        );
        f.queue
            .iter_mut()
            .find(|q| q.state == "done")
            .unwrap()
            .state = "pending".into();
        assert_eq!(
            f.evaluate(0, 20)["context_compactions"]["boundary_reset_count"],
            1
        );
        f.bindings.clear();
        f.queue.retain(|q| q.state != "pending");
        let v = f.evaluate(0, 20);
        assert_eq!(v["context_compactions"]["boundary_reset_count"], 1);
        assert_eq!(v["context_compactions"]["events"][0]["seat"], Value::Null);
        assert_eq!(v["context_depth_brackets"]["by_seat"], json!([]));
        assert_eq!(
            v["context_depth_brackets"]["by_session"][0]["brackets"]["over_250k"]["turn_count"],
            1
        );
    }

    #[test]
    fn drop_thresholds_are_inclusive_and_require_both_absolute_and_relative_drop() {
        for (pre, post, expected) in [
            (100_000, 50_000, true),
            (100_001, 50_000, true),
            (100_001, 50_001, false),
            (99_999, 50_000, false),
            (50_000, 0, true),
            (49_999, 0, false),
            (325_000, 200_000, false),
            (325_000, 325_000, false),
            (325_000, 400_000, false),
        ] {
            assert_eq!(is_drop(pre, post), expected, "{pre} -> {post}");
        }
    }

    #[test]
    fn codex_deltas_do_not_add_prior_cumulative_counters_and_reset_uses_previous_turn() {
        let mut f = Fixture::linked();
        let mut pre = usage("pre", 5, 325_000, 300_000);
        pre.input_uncached -= 5_000;
        pre.cache_write = 5_000;
        pre.predecessor_reason = Some("prior".into());
        pre.prior_input_tokens = Some(10_000_000);
        pre.prior_cached_input_tokens = Some(9_000_000);
        pre.prior_cache_write_input_tokens = Some(500_000);
        let mut post = usage("post", 10, 34_000, 30_000);
        post.predecessor_reason = Some("reset".into());
        post.prior_input_tokens = Some(11_000_000);
        f.usage = vec![pre, post];
        let v = f.evaluate(0, 20);
        assert_eq!(
            v["context_compactions"]["events"][0]["pre_drop_context_tokens"],
            325_000
        );
        assert_eq!(
            v["context_compactions"]["events"][0]["post_drop_context_tokens"],
            34_000
        );
        assert_eq!(turn_context_tokens(&f.usage[0]), 325_000);
    }

    #[test]
    fn ordering_uses_timestamp_then_sequence_then_source_and_never_crosses_sessions() {
        let mut f = Fixture::linked();
        let mut a = usage("a", 5, 325_000, 0);
        let mut b = usage("b", 5, 34_000, 0);
        a.seq = 1;
        b.seq = 2;
        f.usage = vec![b.clone(), a.clone()];
        assert_eq!(
            f.evaluate(0, 20)["context_compactions"]["boundary_reset_count"],
            1
        );
        b.seq = 1;
        f.usage = vec![b.clone(), a.clone()];
        assert_eq!(
            f.evaluate(0, 20)["context_compactions"]["events"][0]["pre_source_key"],
            "a"
        );
        b.native_session = "other".into();
        f.usage = vec![a.clone(), b.clone()];
        assert_eq!(
            f.evaluate(0, 20)["context_compactions"]["boundary_reset_count"],
            0
        );
        b.native_session = "persistent".into();
        b.harness = "claude".into();
        f.usage = vec![a, b];
        assert_eq!(
            f.evaluate(0, 20)["context_compactions"]["boundary_reset_count"],
            0
        );
    }

    #[test]
    fn brackets_cover_exact_edges_and_shares_conserve_tokens_without_reasoning_double_count() {
        let mut f = Fixture::linked();
        f.usage = [99_999, 100_000, 250_000, 250_001]
            .into_iter()
            .enumerate()
            .map(|(i, n)| usage(&format!("u:{i}"), i as i64, n, 1000))
            .collect();
        let v = f.evaluate(0, 4);
        let d = &v["context_depth_brackets"]["overall"];
        for (bracket, turns, input, total) in [
            ("under_100k", 1, 99_999, 100_999),
            ("100k_to_250k", 2, 350_000, 352_000),
            ("over_250k", 1, 250_001, 251_001),
        ] {
            assert_eq!(d[bracket]["turn_count"], turns);
            assert_eq!(d[bracket]["input_context_tokens"], input);
            assert_eq!(d[bracket]["total_tokens"], total);
            assert_eq!(d[bracket]["turn_share"], json!(turns as f64 / 4.0));
            assert_eq!(d[bracket]["token_share"], json!(total as f64 / 704_000.0));
            assert_eq!(
                d[bracket]["source_keys"].as_array().unwrap().len(),
                turns as usize
            );
        }
        assert_eq!(v["context_depth_brackets"]["by_seat"][0]["brackets"], *d);
        assert_eq!(v["context_depth_brackets"]["by_session"][0]["brackets"], *d);
        assert_eq!(
            f.evaluate(10, 20)["context_depth_brackets"]["overall"]["under_100k"]["turn_share"],
            Value::Null
        );
    }

    #[test]
    fn bleed_uses_exit_cap_not_peak_and_stops_at_compaction_or_next_slice() {
        let mut f = Fixture::linked();
        f.item("a", 1, 8, &["A"]);
        f.item("b", 8, 20, &["B"]);
        f.item("c", 20, 30, &["C"]);
        f.usage = vec![
            usage("a:peak", 2, 300_000, 100_000),
            usage("a:exit", 7, 240_000, 200_000),
            usage("b:entry", 8, 260_000, 200_000),
            usage("b:next", 10, 300_000, 290_000),
            usage("b:reset", 12, 34_000, 30_000),
            usage("b:after", 15, 40_000, 35_000),
            usage("c:entry", 20, 50_000, 45_000),
        ];
        let v = f.evaluate(8, 30);
        let e = &v["cross_slice_cache_bleed"]["events"][0];
        assert_eq!(e["from_slice"], "mission:A");
        assert_eq!(e["to_slice"], "mission:B");
        assert_eq!(e["prior_slice_peak_context_tokens"], 300_000);
        assert_eq!(e["prior_slice_exit_context_tokens"], 240_000);
        assert_eq!(e["entry_context_tokens"], 260_000);
        assert_eq!(e["inherited_cache_read_tokens"], 200_000);
        assert_eq!(e["cumulative_bleed_tokens"], 440_000);
        let keys = e["source_keys"].as_array().unwrap();
        for key in ["a:exit", "b:entry", "b:next", "claim:a", "claim:b"] {
            assert!(keys.contains(&json!(key)));
        }
        assert!(!keys.contains(&json!("b:reset")));
        let e2 = &v["cross_slice_cache_bleed"]["events"][1];
        assert_eq!(e2["prior_slice_exit_context_tokens"], 40_000);
        assert_eq!(e2["inherited_cache_read_tokens"], 40_000);
        assert_eq!(
            v["cross_slice_cache_bleed"]["total_inherited_entry_tokens"],
            240_000
        );
        assert_eq!(
            v["cross_slice_cache_bleed"]["total_cumulative_bleed_tokens"],
            480_000
        );
        assert_eq!(
            v["cross_slice_cache_bleed"]["by_seat"][0]["total_cumulative_bleed_tokens"],
            480_000
        );
        assert_eq!(
            f.evaluate(8, 10)["cross_slice_cache_bleed"]["events"][0]["cumulative_bleed_tokens"],
            200_000
        );
        assert_eq!(
            f.evaluate(10, 20)["cross_slice_cache_bleed"]["events"],
            json!([])
        );
    }

    #[test]
    fn intervening_boundary_reset_suppresses_bleed_even_before_window() {
        let mut f = Fixture::linked();
        f.item("a", 1, 8, &["A"]);
        f.item("b", 12, 20, &["B"]);
        f.usage = vec![
            usage("a", 5, 240_000, 200_000),
            usage("reset", 10, 34_000, 30_000),
            usage("b", 15, 260_000, 200_000),
        ];
        let v = f.evaluate(12, 20);
        assert_eq!(v["cross_slice_cache_bleed"]["events"], json!([]));
        assert_eq!(v["context_compactions"]["events"], json!([]));
        f.usage.remove(1);
        assert_eq!(
            f.evaluate(12, 20)["cross_slice_cache_bleed"]["total_inherited_entry_tokens"],
            200_000
        );
        f.bindings.clear();
        assert_eq!(
            f.evaluate(12, 20)["cross_slice_cache_bleed"]["events"],
            json!([])
        );
    }

    #[test]
    fn issue_lineage_overrides_shared_mission_and_includes_parent_locators() {
        let mut f = Fixture::linked();
        f.item("root", 0, 1, &["shared"]);
        f.item("a", 1, 8, &[]);
        f.item("b", 8, 20, &[]);
        f.meta.insert(
            "root".into(),
            QueueItemMeta {
                issue_tags: vec!["54".into()],
                ..QueueItemMeta::default()
            },
        );
        f.meta.insert(
            "a".into(),
            QueueItemMeta {
                parent_qitem_id: Some("root".into()),
                ..QueueItemMeta::default()
            },
        );
        f.meta.insert(
            "b".into(),
            QueueItemMeta {
                parent_qitem_id: Some("root".into()),
                issue_tags: vec!["55".into()],
                ..QueueItemMeta::default()
            },
        );
        f.usage = vec![
            usage("a", 5, 240_000, 200_000),
            usage("b", 10, 260_000, 200_000),
        ];
        let v = f.evaluate(0, 20);
        let e = &v["cross_slice_cache_bleed"]["events"][0];
        assert_eq!(e["from_slice"], "issue:54");
        assert_eq!(e["to_slice"], "issue:55");
        assert!(e["source_keys"]
            .as_array()
            .unwrap()
            .contains(&json!("events:queue.created:root")));
        f.meta.get_mut("b").unwrap().issue_tags.push("56".into());
        assert_eq!(
            f.evaluate(0, 20)["cross_slice_cache_bleed"]["events"],
            json!([])
        );
    }

    #[test]
    fn k5_reroute_and_unprovable_destinations_do_not_attribute_hazards_to_actor() {
        let mut f = Fixture::linked();
        f.item("a", 1, 20, &["A"]);
        f.usage = vec![usage("pre", 5, 325_000, 0), usage("post", 10, 34_000, 0)];
        let mut reroute = f.queue[1].clone();
        reroute.source_key = "events:qitem.fallback_routed:a".into();
        reroute.ts = at(8);
        reroute.transition_id = 8;
        reroute.dest_seat = "other-seat".into();
        f.queue.push(reroute);
        assert_eq!(
            f.evaluate(0, 20)["context_compactions"]["boundary_reset_count"],
            1
        );
        f.queue.pop();
        f.queue
            .retain(|q| !q.source_key.starts_with("events:queue.created:"));
        assert_eq!(
            f.evaluate(0, 20)["context_compactions"]["boundary_reset_count"],
            1
        );
    }
}
