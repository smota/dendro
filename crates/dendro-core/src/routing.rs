//! Effort observations for seat windows whose recorded tools are exclusively routing.
//! This does not attribute a model turn's tokens to a particular command.
use crate::{
    metrics::{self, SessionBinding, Window},
    ToolCategory, ToolFact, UsageFact,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
type Models = BTreeMap<String, (Option<String>, Option<String>)>;
type Bindings = BTreeMap<(String, String), SessionBinding>;
fn covered(coverage: &Value, harness: &str) -> bool {
    coverage.as_array().into_iter().flatten().any(|r| {
        r["harness"] == harness
            && r["sources_total"].as_u64().is_some_and(|n| n > 0)
            && r["sources_read"] == r["sources_total"]
            && ["input", "linkage", "timestamp"]
                .iter()
                .all(|k| r["unknowns"][*k].as_u64() == Some(0))
    })
}
/// Inputs must be the same filtered facts and read snapshot as the report.
pub fn evaluate(
    usage: &[UsageFact],
    tools: &[ToolFact],
    models: &Models,
    categories: &BTreeMap<String, ToolCategory>,
    bindings: &Bindings,
    window: Window,
    coverage: &Value,
) -> Value {
    let mut seat_tools = BTreeMap::<String, Vec<&ToolFact>>::new();
    for t in tools {
        if t.ts.is_some_and(|ts| ts >= window.from && ts < window.to) {
            if let Some(seat) = metrics::binding_of(bindings, &t.harness, &t.native_session).seat()
            {
                seat_tools.entry(seat.into()).or_default().push(t);
            }
        }
    }
    let mut seat_usage = BTreeMap::<String, Vec<&UsageFact>>::new();
    for u in usage {
        if u.ts.is_some_and(|ts| ts >= window.from && ts < window.to) {
            if let Some(seat) = metrics::binding_of(bindings, &u.harness, &u.native_session).seat()
            {
                seat_usage.entry(seat.into()).or_default().push(u);
            }
        }
    }
    let mut evidence = Vec::new();
    let mut evaluated = 0;
    let mut incomplete = 0;
    for (seat, usage) in seat_usage {
        let Some(tools) = seat_tools.get(&seat).filter(|t| !t.is_empty()) else {
            incomplete += 1;
            continue;
        };
        let complete = usage.iter().all(|u| {
            covered(coverage, &u.harness)
                && models
                    .get(&u.source_key)
                    .is_some_and(|(_, effort)| effort.is_some())
                && [u.input_uncached, u.output, u.cache_read, u.cache_write]
                    .iter()
                    .all(|n| *n >= 0)
        }) && tools
            .iter()
            .all(|t| covered(coverage, &t.harness) && categories.contains_key(&t.source_key));
        if !complete {
            incomplete += 1;
            continue;
        }
        evaluated += 1;
        if !tools.iter().all(|t| {
            matches!(
                categories[&t.source_key],
                ToolCategory::CoordQueue | ToolCategory::CoordStatus
            )
        }) {
            continue;
        }
        let mut keys = BTreeSet::new();
        let mut seen = BTreeSet::new();
        let mut tokens = Some(0u64);
        let mut saw_xhigh = false;
        for u in usage {
            keys.insert(u.source_key.clone());
            if !seen.insert(&u.source_key) {
                continue;
            }
            if models
                .get(&u.source_key)
                .and_then(|(_, effort)| effort.as_deref())
                == Some("xhigh")
            {
                saw_xhigh = true;
                for amount in [u.input_uncached, u.output, u.cache_read, u.cache_write] {
                    tokens = tokens.and_then(|n| n.checked_add(amount as u64));
                }
            }
        }
        keys.extend(tools.iter().map(|t| t.source_key.clone()));
        let Some(tokens) = tokens else {
            incomplete += 1;
            continue;
        };
        evidence.push(json!({"kind":"routing_only_observed_seat_window","coverage":"complete","seat":seat,"effort":if saw_xhigh{"xhigh"}else{"non_xhigh"},"tokens":tokens,"source_keys":keys,"window":{"from":window.from,"to":window.to},"definition":"xhigh usage in a seat-window with exclusively recorded routing tools; not per-turn command attribution or causation"}));
    }
    json!({"routing_effort_status":if evaluated>0&&incomplete==0{"observed"}else if evaluated>0{"partial"}else{"no_data"},"routing_effort_evidence":evidence,"routing_effort_coverage":{"evaluated_seat_windows":evaluated,"incomplete_seat_windows":incomplete,"definition":"Complete retained input, linkage and timestamps, explicit categories and effort required. Unknown reader coverage is not evidence of routing-only activity."}})
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (
        UsageFact,
        ToolFact,
        Models,
        BTreeMap<String, ToolCategory>,
        Bindings,
        Window,
        Value,
    ) {
        let t = crate::parse_utc_time("2026-01-01T00:00:00Z").unwrap();
        (
            UsageFact {
                source_key: "u".into(),
                harness: "codex".into(),
                native_session: "s".into(),
                message_id: None,
                ts: Some(t),
                seq: 0,
                input_uncached: 10,
                output: 2,
                cache_read: 20,
                cache_write: 0,
                reasoning: 2,
                currency_total: None,
                generation: 0,
                predecessor_source_key: None,
                predecessor_reason: None,
                prior_input_tokens: None,
                prior_cached_input_tokens: None,
                prior_cache_write_input_tokens: None,
                prior_output_tokens: None,
                prior_reasoning_output_tokens: None,
            },
            ToolFact {
                source_key: "t".into(),
                harness: "codex".into(),
                native_session: "s".into(),
                ts: Some(t),
                seq: 0,
                name: "shell".into(),
                fingerprint: None,
                failed: None,
                structured_write: false,
                generation: 0,
            },
            BTreeMap::from([("u".into(), (Some("model".into()), Some("xhigh".into())))]),
            BTreeMap::from([("t".into(), ToolCategory::CoordQueue)]),
            BTreeMap::from([(
                ("codex".into(), "s".into()),
                SessionBinding::Linked {
                    rig: "r".into(),
                    seat: "a".into(),
                },
            )]),
            Window {
                from: t,
                to: t + chrono::Duration::days(1),
            },
            json!([{"harness":"codex","sources_total":1,"sources_read":1,"unknowns":{"input":0,"linkage":0,"timestamp":0}}]),
        )
    }
    #[test]
    fn explicit_complete_window_conserves_disjoint_tokens_and_evidence() {
        let (u, t, m, c, b, w, coverage) = fixture();
        let v = evaluate(&[u.clone(), u], &[t], &m, &c, &b, w, &coverage);
        assert_eq!(v["routing_effort_status"], "observed");
        assert_eq!(v["routing_effort_evidence"][0]["tokens"], 32);
        assert_eq!(
            v["routing_effort_evidence"][0]["source_keys"],
            json!(["t", "u"])
        );
    }
    #[test]
    fn unknown_or_engineering_does_not_prove_routing() {
        let (u, t, m, mut c, b, w, coverage) = fixture();
        let v = evaluate(
            std::slice::from_ref(&u),
            std::slice::from_ref(&t),
            &m,
            &c,
            &b,
            w,
            &json!([]),
        );
        assert_eq!(v["routing_effort_status"], "no_data");
        c.insert("t".into(), ToolCategory::EngCodeEdit);
        let v = evaluate(&[u], &[t], &m, &c, &b, w, &coverage);
        assert_eq!(v["routing_effort_status"], "observed");
        assert_eq!(v["routing_effort_evidence"], json!([]));
    }
}
