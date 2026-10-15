//! Trace projection of the exact detector evaluation; never a second detector.
use crate::metrics::{binding_of, LoopEvaluation, SessionBinding};
use crate::privacy::{contains_secret, redact};
use crate::{CodexObservation, SourceCoverage, ToolFact};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};

const DEFINITION: &str = r#"{
  "failures": 3,
  "window_seconds": 600,
  "episode": "whole_maximal_failure_run",
  "equality": "stored_keyed_lexical_fingerprint_and_tool",
  "witness": "first_qualifying_consecutive_triple_in_run_order",
  "span_seconds": "canonical_signed_decimal_end_minus_start",
  "native_span_predicate": "span<=600",
  "codex_span_predicate": "0<=span<=600",
  "unknown_endpoint": "cannot_qualify",
  "unknown_middle_timestamp": "retained",
  "native_order": [
    "native_seq",
    "input_rank"
  ],
  "codex_order": {
    "base": [
      "source_id",
      "generation",
      "native_seq",
      "input_rank"
    ],
    "timed_segments": "stable_timestamp_sort",
    "untimed_anchors": "fixed"
  },
  "codex_membership": "uniquely_linked_CommandExecution_failure_with_known_fingerprint",
  "codex_seat_calls": "run_breakers",
  "scope_filter": "half_open_timestamp_filter_before_ordering_with_untimed_retained"
}
"#;

fn limitation(
    code: &str,
    source: Option<&str>,
    observation: Option<&str>,
    report: Option<usize>,
) -> Value {
    json!({"code":code,"source_ref":source,"observation_id":observation,"report_index":report,
    "detail":match code {
        "native_sql_ties"=>"Captured input enumeration is replayable; independent full SQL ties have no total order guarantee.",
        "producing_revision_not_recorded"=>"Producing-ingest revision is not recorded; latest prefix is not per-row history.",
        "latest_metadata_unavailable"=>"Latest source metadata is unavailable; stored row identity is retained.",
        "current_source_not_checked"=>"Current source filesystem state was not checked.",
        "reader_not_read"=>"Persisted reader coverage is not complete for this source or unit.",
        "linkage_excluded"=>"Codex observation has no unique trusted seat binding.",
        _=>"Essential stored evidence cannot be safely exported; this partition is insufficient.",
    }})
}

pub(crate) fn span_seconds(
    start: chrono::DateTime<chrono::Utc>,
    end: chrono::DateTime<chrono::Utc>,
) -> String {
    // Use the same chrono duration as the detector, including leap-second
    // semantics; do not subtract independently normalized Unix timestamps.
    let delta = end - start;
    let nanos = i128::from(delta.num_seconds()) * 1_000_000_000 + i128::from(delta.subsec_nanos());
    let sign = if nanos < 0 { "-" } else { "" };
    let abs = nanos.abs();
    let whole = abs / 1_000_000_000;
    let fraction = abs % 1_000_000_000;
    if fraction == 0 {
        format!("{sign}{whole}")
    } else {
        format!(
            "{sign}{whole}.{}",
            format!("{fraction:09}").trim_end_matches('0')
        )
    }
}

// All arguments are captured under the caller's single read transaction.
pub(crate) struct EvidenceInput<'a> {
    pub native: &'a [ToolFact],
    pub codex: &'a [CodexObservation],
    pub trusted_indices: &'a [usize],
    pub native_evaluation: &'a LoopEvaluation,
    pub codex_evaluation: &'a LoopEvaluation,
    pub source_ids: &'a HashMap<String, Option<String>>,
    pub prefixes: &'a HashMap<String, (i64, i64, String)>,
    pub coverage: &'a [SourceCoverage],
    pub bindings: &'a BTreeMap<(String, String), SessionBinding>,
    pub rig: Option<&'a str>,
    pub from: &'a str,
    pub to: &'a str,
    pub failures: usize,
    pub window_secs: i64,
}

pub(crate) fn build(input: EvidenceInput<'_>) -> Value {
    let EvidenceInput {
        native,
        codex,
        trusted_indices,
        native_evaluation,
        codex_evaluation,
        source_ids,
        prefixes,
        coverage,
        bindings,
        rig,
        from,
        to,
        failures,
        window_secs,
    } = input;
    let mut ranks = vec![None; native.len() + codex.len()];
    let mut counters = HashMap::new();
    for &i in &native_evaluation.order {
        let fact = &native[i];
        let rank = counters
            .entry((&fact.harness, &fact.native_session))
            .or_insert(0usize);
        ranks[i] = Some(*rank);
        *rank += 1;
    }
    for &i in &codex_evaluation.order {
        let pos = trusted_indices[i];
        let fact = &codex[pos].fact;
        let rank = counters
            .entry((&fact.harness, &fact.native_session))
            .or_insert(0usize);
        ranks[native.len() + pos] = Some(*rank);
        *rank += 1;
    }
    let mut sources: Vec<Value> = Vec::new();
    let mut refs: HashMap<(String, Option<i64>), usize> = HashMap::new();
    let (observations, mut limitations, unsupported, owners) = {
        let mut get_source = |id: &str, generation: Option<i64>| {
            let key = (id.to_string(), generation);
            if let Some(&index) = refs.get(&key) {
                return index;
            }
            let index = sources.len();
            let reference = format!("s{index}");
            let prefix=prefixes.get(id).map(|(g,offset,revision)|json!({"generation":g,"byte_offset":offset,"revision":revision}));
            let mut limits = vec![
                limitation(
                    "producing_revision_not_recorded",
                    Some(&reference),
                    None,
                    None,
                ),
                limitation("current_source_not_checked", Some(&reference), None, None),
            ];
            if prefix.is_none() {
                limits.push(limitation(
                    "latest_metadata_unavailable",
                    Some(&reference),
                    None,
                    None,
                ));
            }
            sources.push(json!({"source_ref":reference,"source_id":id,"generation":generation,
            "metadata_status":if prefix.is_some(){"available"}else{"unavailable"},"latest_committed_prefix":prefix,
            "producing_revision":null,"producing_revision_status":"not_recorded","current_state":"not_checked","coverage":null,"limitations":limits}));
            refs.insert(key, index);
            index
        };
        let mut observations = Vec::new();
        let mut limitations = vec![limitation("native_sql_ties", None, None, None)];
        let mut unsupported = HashSet::new();
        for i in 0..native.len() + codex.len() {
            let (fact, id) = if i < native.len() {
                (
                    &native[i],
                    source_ids
                        .get(&native[i].source_key)
                        .and_then(|s| s.as_deref()),
                )
            } else {
                let row = &codex[i - native.len()];
                (&row.fact, Some(row.source_id.as_str()))
            };
            let observation = format!("o{i}");
            let safe = |s: &str| !s.is_empty() && !contains_secret(s);
            let source_id = id.filter(|s| safe(s));
            let source_key = safe(&fact.source_key).then_some(fact.source_key.as_str());
            let generation = (fact.generation >= 0).then_some(fact.generation);
            let fp = fact.fingerprint.as_deref().filter(|s| {
                s.len() == 64
                    && s.bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            });
            let essential = source_id.is_some()
                && source_key.is_some()
                && generation.is_some()
                && safe(&fact.harness)
                && safe(&fact.native_session)
                && safe(&fact.name)
                && (fact.fingerprint.is_none() || fp.is_some());
            let source_ref = match (source_id, generation) {
                (Some(id), g) => Some(format!("s{}", get_source(id, g))),
                _ => None,
            };
            let bound = binding_of(bindings, &fact.harness, &fact.native_session);
            let excluded = if fact.harness == "codex" && bound.seat().is_none() {
                Some(bound.status())
            } else {
                None
            };
            if !essential {
                unsupported.insert((fact.harness.clone(), fact.native_session.clone()));
                limitations.push(limitation(
                    "essential_evidence_unavailable",
                    source_ref.as_deref(),
                    Some(&observation),
                    None,
                ));
            }
            if excluded.is_some() {
                limitations.push(limitation(
                    "linkage_excluded",
                    source_ref.as_deref(),
                    Some(&observation),
                    None,
                ));
            }
            observations.push(json!({"observation_id":observation,"source_id":source_id,"source_key":source_key,"generation":generation,
            "harness":redact(&fact.harness),"unit":if fact.harness=="codex"&&fact.name==crate::CODEX_COMMAND{"command_execution"}else{"tool_call"},
            "session":redact(&fact.native_session),"seat":bound.seat().map(redact),"linkage_status":bound.status(),"tool":redact(&fact.name),
            "timestamp":fact.ts.map(|t|t.to_rfc3339()),"outcome":match fact.failed{Some(true)=>"failure",Some(false)=>"success",None=>"unknown"},
            "input_fingerprint":fp,"native_seq":fact.seq,"input_rank":i,"detector_rank":ranks[i],"exclusion_reason":excluded,"source_ref":source_ref}));
        }
        // Keep this closure alive only until all coverage-only source entries exist.
        let mut owners = Vec::new();
        for source in coverage {
            if let Some((generation, _, _)) = prefixes.get(&source.source_id) {
                owners.push((get_source(&source.source_id, Some(*generation)), source));
            }
        }
        (observations, limitations, unsupported, owners)
    };
    for (owner, source) in owners {
        let bound = source
            .native_session
            .as_deref()
            .map(|s| binding_of(bindings, &source.harness, s));
        let seat = bound.and_then(SessionBinding::seat);
        let in_scope = rig.is_none() || bound.is_some_and(|b| b.rig() == rig);
        sources[owner]["coverage"] = json!({"harness":source.harness,"session":source.native_session,"linkage_status":bound.map(SessionBinding::status).unwrap_or("unlinked"),"seat":seat,
            "in_scope":in_scope,"unresolved":source.native_session.is_none()||seat.is_none(),"reader_complete":source.complete});
        if !source.complete {
            let reference = sources[owner]["source_ref"].as_str().unwrap().to_string();
            sources[owner]["limitations"]
                .as_array_mut()
                .unwrap()
                .push(limitation("reader_not_read", Some(&reference), None, None));
        }
    }
    // Codex tool-call reading remains unsupported regardless of boundary evidence.
    limitations.push(limitation("reader_not_read", None, None, None));
    let mut episodes = Vec::new();
    let mut report_index = 0;
    for (evaluation, is_codex) in [(native_evaluation, false), (codex_evaluation, true)] {
        let global = |i: usize| {
            if is_codex {
                native.len() + trusted_indices[i]
            } else {
                i
            }
        };
        for episode in &evaluation.episodes {
            let p = &episode.projection;
            if unsupported.contains(&(p.harness.clone(), p.session.clone())) {
                limitations.push(limitation(
                    "essential_evidence_unavailable",
                    None,
                    None,
                    Some(report_index),
                ));
            } else {
                let members: Vec<_> = episode
                    .members
                    .iter()
                    .map(|&i| format!("o{}", global(i)))
                    .collect();
                let witness: Vec<_> = episode
                    .witness
                    .iter()
                    .map(|&i| format!("o{}", global(i)))
                    .collect();
                let fact = |i: usize| {
                    if is_codex {
                        &codex[trusted_indices[i]].fact
                    } else {
                        &native[i]
                    }
                };
                let span = span_seconds(
                    fact(episode.witness[0]).ts.unwrap(),
                    fact(*episode.witness.last().unwrap()).ts.unwrap(),
                );
                episodes.push(json!({"report_index":report_index,"members":members,"qualifying_triple":witness,"span_seconds":span}));
            }
            report_index += 1;
        }
    }
    let mut def: Value = serde_json::from_str(DEFINITION).expect("static loop definition");
    if (failures, window_secs) != (3, 600) {
        def["failures"] = json!(failures);
        def["window_seconds"] = json!(window_secs);
        if failures != 3 {
            def["witness"] = json!(format!(
                "first_qualifying_consecutive_window_of_{failures}_in_run_order"
            ));
        }
        def["native_span_predicate"] = json!(format!("span<={window_secs}"));
        def["codex_span_predicate"] = json!(format!("0<=span<={window_secs}"));
    }
    json!({"formula_version":"k11-tool-loop-trace-v1","definition":def,
        "scope":{"rig":rig.map(redact),"from":from,"to":to,"interval":"[from,to)","untimed":"retained","consistency":"single_read_transaction",
            "replay":"captured_evaluation","independent_query_ties":"not_guaranteed","input_rank_basis":"scoped_native_then_scoped_codex","detector_rank_partition":["harness","session"]},
        "status":if unsupported.is_empty(){"complete"}else{"insufficient_evidence"},"limitations":limitations,"observations":observations,"sources":sources,"episodes":episodes})
}

#[cfg(test)]
mod tests {
    use super::*;
    fn time(s: &str) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339(s)
            .unwrap()
            .with_timezone(&chrono::Utc)
    }
    #[test]
    fn exact_spans_preserve_signed_subseconds_and_wide_ranges() {
        let zero = time("2026-10-15T00:00:00Z");
        assert_eq!(
            span_seconds(zero, time("2026-10-15T00:10:00.000001Z")),
            "600.000001"
        );
        assert_eq!(
            span_seconds(time("2026-10-15T00:00:00.000001Z"), zero),
            "-0.000001"
        );
        assert_eq!(span_seconds(zero, zero), "0");
        assert_eq!(
            span_seconds(
                time("0001-01-01T00:00:00.000001Z"),
                time("9999-01-01T00:00:00.000002Z")
            ),
            "315506361600.000001"
        );
    }
    #[test]
    fn exact_spans_use_detector_chrono_duration_for_leap_seconds() {
        let leap = time("2016-12-31T23:59:60.500Z");
        let next = time("2017-01-01T00:00:00Z");
        let delta = next - leap;
        let got = span_seconds(leap, next);
        // Independent small-range chrono nanoseconds are exact here.
        let ns = delta.num_nanoseconds().unwrap();
        assert_eq!(ns, -500_000_000);
        assert_eq!(got, "-0.5");
    }
}
