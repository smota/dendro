# T14 independent expected-value derivation (draft)

Authorship is independent of the future Grok builder. No accounting implementation output was read, copied or executed. Accepted domain_v1 files were inspected as input contracts only; no methods were invoked to compute expectations. The older broad draft and other oracle values were not read or altered.

Run `python .agentflow/oracles/v080/T14/fixture-recipes.py --check` for exact arithmetic and artifact consistency. Without `--check`, the executable recipes regenerate only these task-owned draft artifacts and a fresh factual authorship receipt. This is oracle arithmetic verification, not product conformance or independent review.

## Contract sources

- tasks.json: T14-A1/A2, ownership, stop conditions and identity contract.
- DESIGN.md: section2 canonical quantities; closed WorkEdge contract; corrections and coverage; section4 accounting/time/cost/outcomes; I02/I03/I04/I10/I12/I18/I20/I21.
- ADVERSARIAL.md: F01-F15 and F21-F24, H3 atomic window selection and H4 decimal normalization. F13 policy authority and F24 hostile adapter/security specifics are outside numeric T14 ownership; no kernel/adapter/policy implementation expectations are claimed.
- domain_v1 quantity/revision/work/usage/common declarations: Decimal versus Ratio, units/grain, assignment closure, typed boundaries and append-only revisions. Input bytes are bound in the JSON/receipt.

## Construction and scope

See the JSON conventions for executable fixture scaffolding, typed synthetic IDs, stipulated evidence, clock/window units and adapter obligations. Cases are structured recipes, not raw serde Record JSON. The future adapter must create complete valid typed envelopes and compare only asserted result fields. It must not turn missingness or excluded amounts into measured zeros.

No generic ambiguous reparenting, nonterminating currency wire value or guessed native ABI is authored. The explicit questions below park those claims; unambiguous mathematics remains drafted.

## Exact workings

### T14-01-diamond (F03, T14-A1)

Reach(P)={P,A,B,D}; Reach(A)={A,D}; Reach(B)={B,D}. Each observation occurs once in its set. 1+2+2=5 repeats D and is not fleet usage.

Expected: `{"fleet_direct": 4, "forbidden_caller_sum": 5, "inclusive": {"A": 2, "B": 2, "D": 1, "P": 4}, "inclusive_nonadditive": true}`

Exact checks: `1+1+1+1 == 4`; `1+2+2 == 5`

### T14-02-two-roots (F03)

Root observation sets {r1,d} and {r2,d} intersect at d; union is {r1,r2,d}.

Expected: `{"forbidden_sum": 19, "inclusive": {"R1": 9, "R2": 10}, "union_direct": 12}`

Exact checks: `2+7 == 9`; `3+7 == 10`; `2+3+7 == 12`; `9+10 == 19`

### T14-03-shared-no-weights (F04)

Two callers provide no ownership weights. Shared amount remains 1000, never 500 per caller.

Expected: `{"owned": {"A": null, "B": null}, "ownership": "unknown", "shared": 1000}`

### T14-04-partial-weights (F04)

1000*3/5=600; 1000*3/10=300; residual=1000*(1-9/10)=100.

Expected: `{"allocated": {"A": "600/1", "B": "300/1"}, "conserved": "1000/1", "residual": "100/1"}`

Exact checks: `1000*F(3,5)==600`; `1000*F(3,10)==300`; `1000*(1-F(9,10))==100`; `600+300+100==1000`

### T14-05-omitted-claimant (F04)

25/100+40/100=13/20. Residual 100*7/20=35 stays unallocated; C has no weight.

Expected: `{"C_owned": null, "allocated": {"A": "25/1", "B": "40/1"}, "residual": "35/1"}`

Exact checks: `F(25,100)+F(40,100)==F(13,20)`; `100*(1-F(13,20))==35`; `25+40+35==100`

### T14-06-overweight (F04)

11/10>1 and 6/5>1; neither set admits a conserved nonnegative residual.

Expected: `{"allocated": null, "refusal": "weight_sum_exceeds_one", "source_tokens": 100}`

Exact checks: `F(7,10)+F(4,10)>1`; `F(70,100)+F(50,100)>1`

### T14-07-fractional-conservation (F04)

1/3+1/6=1/2; residual=1/2; exact sum=1. Display does not replace canonical rational shares.

Expected: `{"allocated": {"A": "1/3", "B": "1/6"}, "display": {"A": "0.33", "B": "0.17", "residual": "0.50"}, "residual": "1/2", "total": "1/1"}`

Exact checks: `F(1,3)+F(1,6)+F(1,2)==1`; `half_even(F(1,3),2)==F(33,100)`; `half_even(F(1,6),2)==F(17,100)`

### T14-08-equivalent-pair (F06)

Explicit equivalence collapses {o1,o2} to one 100-token equivalence class; 2-1=1 duplicate.

Expected: `{"canonical_tokens": 100, "duplicate_rows": 1}`

Exact checks: `2-1==1`

### T14-09-overlap (F06)

Known equivalence resolves only o1/o2. Unknown intersection with o3 prevents a canonical fleet sum.

Expected: `{"canonical_fleet": null, "duplicate_rows": 1, "equivalent_pair_tokens": 100, "o3_source_tokens": 100, "refusal": "ambiguous_overlap"}`

### T14-10-namespaces (F07)

The authority component differs; full keys are unequal. Disjoint direct values add 100+200.

Expected: `{"direct_tokens": 300, "distinct_observations": 2}`

Exact checks: `100+200==300`

### T14-11-generation-replacement (F06, F09)

Equal bytes/native keys in generations 1 and 2 prove neither equivalence nor reset; do not silently choose 40 or 80.

Expected: `{"canonical_fleet": null, "inferred_reset": false, "refusal": "ambiguous_overlap", "retained_source_values": [40, 40]}`

### T14-12-usage-only (T14-A1)

Usage amount is evidenced independently. An absent invocation link does not erase 17 or prove total invocation count.

Expected: `{"direct_tokens": 17, "linked_invocation_count": 0, "total_invocation_count": null}`

### T14-13-inclusion (F08)

Input already contains cache-read; 100+20=120. Cache fraction=40/100=2/5, not extra usage.

Expected: `{"cache_fraction": "2/5", "derived_total": 120}`

Exact checks: `100+20==120`; `F(40,100)==F(2,5)`

### T14-14-unknown-inclusion (F08)

Unknown intersection size prevents union cardinality. Literal 120 is a separate source-grain statement, not a derived total.

Expected: `{"cache_fraction": null, "derived_total": null, "literal_definition": "definition_unknown", "literal_reported_total": 120, "refusal": "unknown_inclusion", "retained_components": [100, 40, 20]}`

### T14-15-mixed-knownness (F08)

Known subset is 100; no quantity is supplied for legacy input, so full total cannot be 100 by substitution.

Expected: `{"full_input": null, "observed_subset_input": 100}`

### T14-16-cumulative (F09)

The first baseline is unknown; only the difference 150-100=50 is observed incremental consumption.

Expected: `{"complete_consumption": null, "first_cumulative": 100, "first_delta": null, "second_delta": 50}`

Exact checks: `150-100==50`

### T14-17-reset (F09)

An explicit since-zero reset initializes a new epoch at zero; current-minus-zero=20.

Expected: `{"new_epoch_delta": 20}`

Exact checks: `20-0==20`

### T14-18-decrease-only (F09)

20<150 without reset evidence. Negative difference -130 is not consumed usage and does not prove an epoch change.

Expected: `{"delta": null, "retained_current": 20, "status": "conflict_or_unknown"}`

Exact checks: `20-150==-130`

### T14-19-reset-baseline-only (F09)

Reset evidence without since-zero initialization does not establish pre-reading consumption in the new epoch.

Expected: `{"new_epoch_baseline": 20, "new_epoch_delta": null}`

### T14-20-retry (F14, T14-A1)

Work membership selects both distinct attempts: 10+4=14. Retry is not descendant reachability; success is not acceptance.

Expected: `{"attempt_count": 2, "attempt_successes": 1, "inclusive_a1_tokens": 10, "work_acceptance": null, "work_direct_tokens": 14}`

Exact checks: `10+4==14`

### T14-21-resume (T14-A1)

Continuation keeps one attempt. Disjoint lengths (3-0)+(8-5)=6; observations 2+7=9.

Expected: `{"attempt_count": 1, "direct_tokens": 9, "segment_count": 2, "summed_attempt_us": 6, "wall_us": 6}`

Exact checks: `(3-0)+(8-5)==6`; `2+7==9`

### T14-22-correction-cutoffs (F22)

At cutoff2 only f1 is visible. At3 f2 supersedes f1. At4 f3 removes f2. Empty active set is not erased history.

Expected: `{"active_fact_counts": [1, 1, 0], "active_values": [[100], [80], []], "audit_fact_count": 3}`

### T14-23-reparent-correction (F22, F23)

This is an authoritative correction of the same historical relationship, not a new creator. Cutoff2 has only P->D; cutoff3 only Q->D. Fleet observation set remains {p,q,d}.

Expected: `{"at_2": {"D": 7, "P": 9, "Q": 3}, "at_3": {"D": 7, "P": 2, "Q": 10}, "fleet_at_both": 12}`

Exact checks: `2+7==9`; `3+7==10`; `2+3+7==12`

### T14-24-conflicting-creators (F23)

Two independent creator claims violate one-creator cardinality. No latest-wins reparenting; direct facts survive.

Expected: `{"affected_inclusive": null, "fleet_direct": 12, "refusal": "conflicting_creators"}`

Exact checks: `2+3+7==12`

### T14-25-cycle (F23)

A->B->A is not an admissible lineage DAG. Independent direct values remain 10+4=14.

Expected: `{"fleet_direct": 14, "inclusive_A": null, "inclusive_B": null, "refusal": "cycle"}`

Exact checks: `10+4==14`

### T14-26-return-does-not-expand (F23)

Only spawn expands closure. B return A is a receipt, so Reach(B)={B}; no caller-session import.

Expected: `{"fleet_direct": 14, "inclusive": {"A": 14, "B": 4}}`

Exact checks: `10+4==14`

### T14-27-incomplete-closure (T14-A1)

The unique observed nonnegative subset contributes 8+3=11; missing descendants cannot reduce it.

Expected: `{"complete_inclusive": null, "inclusive_observed_lower_bound": 11, "uncertainty": "observed_lower_bound"}`

Exact checks: `8+3==11`

### T14-28-handoff-straddle (T14-A1)

A transfer at5 does not split atomic 100-token evidence into equal halves. Worker-grain observation survives.

Expected: `{"after_owned": null, "before_owned": null, "shared_unattributed": 100, "worker_direct": 100}`

### T14-29-same-revision (F11, F22)

Strip one trailing zero from 10/100 ->1/10; numeric canonical content is identical, no new usage.

Expected: `{"canonical_decimal": {"coefficient": 1, "denominator": 10}, "digest_relation": "equal", "duplicate_consumption": 0, "idempotent": true}`

Exact checks: `F(10,100)==F(1,10)`

### T14-30-divergent-revision (F22)

Same revision differs in canonical amount. Neither 40,25 nor their sum is an authorized winner.

Expected: `{"retained_values": [40, 25], "selected_value": null, "status": "conflict"}`

### T14-31-window-moved-correction (F22)

At3 the active revision b lies outside [0,5); filtering b cannot resurrect superseded a.

Expected: `{"at_2_included_ids": ["a"], "at_2_tokens": 100, "at_3_active_id": "b", "at_3_included_ids": []}`

### T14-32-observed-unpriced (F11)

Observed two cents needs no price inference. Missing rates invalidate only estimated cost.

Expected: `{"estimate": null, "estimate_refusal": "rate_card_absent", "observed": {"coefficient": 2, "currency": "USD", "denominator": 100}}`

### T14-33-priced-components (F11)

1000*2/1000000+500*8/1000000=2000/1000000+4000/1000000=0.006. Canonical Decimal is 6/1000; reduced ratio 3/500 describes the same number, not Decimal wire.

Expected: `{"estimated_USD": {"coefficient": 6, "denominator": 1000}, "exact_reduced": "3/500"}`

Exact checks: `1000*F(2,1000000)+500*F(8,1000000)==F(3,500)`; `F(6,1000)==F(3,500)`

### T14-34-cost-partitions (F11)

Currency, evidence mode and subscription period define distinct measures. No operation authorizes 2356 as spend.

Expected: `{"grand_total": null, "partitions": [["observed", "USD", 150], ["observed", "EUR", 200], ["estimated", "USD", 6], ["subscription", "USD", 2000]]}`

### T14-35-observed-precedence (F11)

A has an observed comparable cost; its estimate remains labeled and is not added. B has only an unmatched estimate.

Expected: `{"combined_observed_plus_estimated": null, "matched_estimate_USD": "3/100", "preferred_observed_USD": "1/50", "unmatched_estimated_USD": "1/100"}`

Exact checks: `F(2,100)==F(1,50)`

### T14-36-unknown-currency (F11)

A scalar without evidenced currency cannot be silently USD. No expected typed Money or FX value is asserted.

Expected: `{"currency_total": null, "refusal": "currency_unknown"}`

### T14-37-missing-component-rate (F11)

Known input component is 1000*2/1000000=1/500; unknown output price prevents a full estimate.

Expected: `{"full_estimate": null, "priced_input_subset_USD": "1/500", "refusal": "component_rate_missing"}`

Exact checks: `1000*F(2,1000000)==F(1,500)`

### T14-38-round-once (F11)

0.005+0.005=0.010 ->canonical0.01 ->display0.01. Each 0.005 independently rounds to0.00; rounding before aggregation loses the cent.

Expected: `{"canonical_sum": {"coefficient": 1, "denominator": 100}, "display": "0.01", "forbidden_sum_of_rounded": "0.00"}`

Exact checks: `F(5,1000)+F(5,1000)==F(1,100)`; `half_even(F(5,1000),2)==0`; `half_even(F(1,100),2)==F(1,100)`

### T14-39-half-even-ties (F11)

Scaled ties are2.5,3.5,-2.5,-3.5; choose the even integer2,4,-2,-4. Signed decimal display is tested without interpreting negative amounts as nonnegative usage.

Expected: `{"display": ["0.02", "0.04", "-0.02", "-0.04"]}`

Exact checks: `half_even(F(25,1000),2)==F(2,100)`; `half_even(F(35,1000),2)==F(4,100)`; `half_even(F(-25,1000),2)==F(-2,100)`; `half_even(F(-35,1000),2)==F(-4,100)`

### T14-40-zero-and-precision (F11)

Zero is0/1.123456789 has no trailing decimal zero, so preserve all nine source decimal places;100/1000 strips two zeros.

Expected: `{"canonical_decimals": [[0, 1], [123456789, 1000000000], [1, 10]]}`

Exact checks: `F(0,1000000)==0`; `F(100,1000)==F(1,10)`

### T14-41-count-overflow (T14-A1)

2^64-1+1=2^64 exceeds Count u64; never wrap to0.

Expected: `{"canonical_count": null, "mathematical_sum": "18446744073709551616", "refusal": "overflow"}`

Exact checks: `(2**64-1)+1==2**64`

### T14-42-full-overlap (F01)

Union is[0,10M); two distinct attempt lengths sum20M.20M/10M=2.

Expected: `{"mean_concurrency": "2/1", "summed_attempt_us": 20000000, "wall_us": 10000000}`

Exact checks: `10000000+10000000==20000000`; `F(20000000,10000000)==2`

### T14-43-abut (F02)

At10M A has ended and B starts. Half-open endpoints do not create overlap;10M+2M=12M.

Expected: `{"mean_concurrency": "1/1", "peak": 1, "summed_attempt_us": 12000000, "wall_us": 12000000}`

Exact checks: `10000000+2000000==12000000`

### T14-44-overlap-sweep (F02)

Union[0,12M); lengths10M+3M=13M. Sweep integral9M*1+1M*2+2M*1=13M.

Expected: `{"mean_concurrency": "13/12", "peak": 2, "summed_attempt_us": 13000000, "sweep": [[0, 9000000, 1], [9000000, 10000000, 2], [10000000, 12000000, 1]], "wall_us": 12000000}`

Exact checks: `10000000+3000000==13000000`; `9000000+2*1000000+2000000==13000000`; `F(13000000,12000000)==F(13,12)`

### T14-45-distinct-vs-replay (F01)

Replay of segment s1 is not another execution. A and B remain distinct active attempts even with equal intervals.

Expected: `{"distinct_segments": 2, "mean_concurrency": "2/1", "summed_attempt_us": 20, "wall_us": 10}`

Exact checks: `10+10==20`

### T14-46-descendant-outlives (F05)

Parent length5M; child length16M; connected union[0,20M) length20M; summed=5M+16M=21M.

Expected: `{"descendant_outlived": true, "inclusive_parent_tokens": 100, "lineage_summed_us": 21000000, "lineage_wall_us": 20000000, "parent_wall_us": 5000000}`

Exact checks: `5000000+16000000==21000000`; `20000000-4000000==16000000`

### T14-47-duration-window (F02, T14-A2)

Intersect duration intervals geometrically: lengths3+3=6, union[5,10) length5.

Expected: `{"clipped": [[5, 8], [7, 10]], "mean_concurrency": "6/5", "summed_attempt_us": 6, "wall_us": 5}`

Exact checks: `(8-5)+(10-7)==6`; `10-5==5`; `F(6,5)==F(6,5)`

### T14-48-censored (F15)

Visible liveness proves survival through10, yielding10-5=5 observed duration; completed duration remains unknown.

Expected: `{"completed_duration": null, "observed_contribution_us": 5, "right_censored": true, "status": "partial"}`

Exact checks: `10-5==5`

### T14-49-lone-start (F15)

A start alone proves no ongoing activity through10; do not replace missing end with cutoff.

Expected: `{"duration_us": null, "reason": "missing_end", "right_censored": false}`

### T14-50-late-liveness (F15)

Receipt11 is outside knowledge cutoff10. Later evidence cannot rewrite sealed earlier knowledge.

Expected: `{"duration_us": null, "reason": "missing_end", "used_liveness": false}`

### T14-51-invalid-and-empty (F15)

10->5 is a conflicting interval, not length0. Valid[5,5) is empty;0/0 has no concurrency ratio.

Expected: `{"empty": {"mean_concurrency": null, "reason": "zero_denominator", "summed_attempt_us": 0, "wall_us": 0}, "reversed": {"duration_us": null, "reason": "invalid_interval"}}`

Exact checks: `5-5==0`

### T14-52-unknown-clocks (F15)

Local lengths are10 each. Equal numeric coordinates on unrelated clocks do not establish overlap or a fleet wall.

Expected: `{"combined_concurrency": null, "combined_wall_us": null, "reason": "incomparable_clocks", "source_durations_us": {"A": 10, "B": 10}}`

Exact checks: `10-0==10`

### T14-53-clock-bounds (F15)

A possible[8,12],B possible[11,15] overlap;12<=11 is false and8>15 is false. Nominal ordering is not proved.

Expected: `{"A_no_later_than_B": null, "status": "ambiguous"}`

Exact checks: `10+2==12`; `13-2==11`; `not (12<=11)`; `not (8>15)`

### T14-54-clock-disjoint-and-local (F15)

Latest A=12<=earliest B=13 proves order. Separately sequence4<=5 proves source-local order, not causal influence.

Expected: `{"disjoint_A_no_later": true, "inferred_causality": false, "local_A_no_later": true}`

Exact checks: `10+2<=15-2`; `4<=5`

### T14-55a-atomic-window (T14-A2)

Timestamp takes precedence when present:0<=t<5. Otherwise require full interval containment. No included observation means exclusion, not an assertion that source usage was measured zero. Apply the same whole-observation decision to100 tokens,USD0.02 and one event; no proration.

Expected: `{"coverage": "partial", "excluded_observation_count": 1, "excluded_source_tokens": 100, "included_amounts": null, "included_ids": [], "reason": "straddle_unsplit"}`

### T14-55b-atomic-window (T14-A2)

Timestamp takes precedence when present:0<=t<5. Otherwise require full interval containment. No included observation means exclusion, not an assertion that source usage was measured zero. Apply the same whole-observation decision to100 tokens,USD0.02 and one event; no proration.

Expected: `{"coverage": "complete_for_declared_selection", "excluded_observation_count": 0, "excluded_source_tokens": 0, "included_amounts": {"event_count": 1, "observed_USD": "1/50", "tokens": 100}, "included_ids": ["o1"], "reason": null}`

### T14-55c-atomic-window (T14-A2)

Timestamp takes precedence when present:0<=t<5. Otherwise require full interval containment. No included observation means exclusion, not an assertion that source usage was measured zero. Apply the same whole-observation decision to100 tokens,USD0.02 and one event; no proration.

Expected: `{"coverage": "complete_for_declared_selection", "excluded_observation_count": 1, "excluded_source_tokens": 100, "included_amounts": null, "included_ids": [], "reason": "timestamp_outside_window"}`

### T14-55d-atomic-window (T14-A2)

Timestamp takes precedence when present:0<=t<5. Otherwise require full interval containment. No included observation means exclusion, not an assertion that source usage was measured zero. Apply the same whole-observation decision to100 tokens,USD0.02 and one event; no proration.

Expected: `{"coverage": "complete_for_declared_selection", "excluded_observation_count": 1, "excluded_source_tokens": 100, "included_amounts": null, "included_ids": [], "reason": "timestamp_outside_window"}`

### T14-55e-atomic-window (T14-A2)

Timestamp takes precedence when present:0<=t<5. Otherwise require full interval containment. No included observation means exclusion, not an assertion that source usage was measured zero. Apply the same whole-observation decision to100 tokens,USD0.02 and one event; no proration.

Expected: `{"coverage": "complete_for_declared_selection", "excluded_observation_count": 0, "excluded_source_tokens": 0, "included_amounts": {"event_count": 1, "observed_USD": "1/50", "tokens": 100}, "included_ids": ["o1"], "reason": null}`

### T14-55f-atomic-window (T14-A2)

Timestamp takes precedence when present:0<=t<5. Otherwise require full interval containment. No included observation means exclusion, not an assertion that source usage was measured zero. Apply the same whole-observation decision to100 tokens,USD0.02 and one event; no proration.

Expected: `{"coverage": "partial", "excluded_observation_count": 1, "excluded_source_tokens": 100, "included_amounts": null, "included_ids": [], "reason": "time_unknown"}`

### T14-55g-atomic-window (T14-A2)

Timestamp takes precedence when present:0<=t<5. Otherwise require full interval containment. No included observation means exclusion, not an assertion that source usage was measured zero. Apply the same whole-observation decision to100 tokens,USD0.02 and one event; no proration.

Expected: `{"coverage": "complete_for_declared_selection", "excluded_observation_count": 0, "excluded_source_tokens": 0, "included_amounts": {"event_count": 1, "observed_USD": "1/50", "tokens": 100}, "included_ids": ["o1"], "reason": null}`

### T14-55h-atomic-window (T14-A2)

Timestamp takes precedence when present:0<=t<5. Otherwise require full interval containment. No included observation means exclusion, not an assertion that source usage was measured zero. Apply the same whole-observation decision to100 tokens,USD0.02 and one event; no proration.

Expected: `{"coverage": "complete_for_declared_selection", "excluded_observation_count": 1, "excluded_source_tokens": 100, "included_amounts": null, "included_ids": [], "reason": "timestamp_outside_window"}`

### T14-56-ratio-populations (F10)

Ratio of sums=(10+10+10)/(1+0+1)=15. Successful-attempt mean=(10+10)/2=10. Named mean of defined per-attempt ratios=(10/1+10/1)/2=10, excludes one10/0.

Expected: `{"cost_per_success": "15/1", "mean_per_attempt_cost_per_success_positive_denominator": "10/1", "mean_successful_attempt_cost": "10/1", "undefined_terms": 1}`

Exact checks: `F(10+10+10,1+0+1)==15`; `F(10+10,2)==10`

### T14-57-cache-ratios (F10)

Sum raw counts:(90+1)/(100+10)=91/110. Mean fractions=(9/10+1/10)/2=1/2. Reducing individual ratios before summing counts would change the measure.

Expected: `{"named_mean_invocation_fraction": "1/2", "ratio_of_sums": "91/110", "undefined_terms": 0}`

Exact checks: `F(90+1,100+10)==F(91,110)`; `(F(90,100)+F(1,10))/2==F(1,2)`

### T14-58-mean-missing-grain (F10)

Aggregate numerator/denominator suffice for91/110 but cannot reconstruct individual invocation fractions.

Expected: `{"named_mean_invocation_fraction": null, "ratio_of_sums": "91/110", "refusal": "invocation_grain_missing"}`

Exact checks: `F(91,110)==F(91,110)`

### T14-59-undefined-and-zero (F10)

0/0 is excluded and counted for the mean. Aggregate9/10 is defined; an all-zero-denominator aggregate is refused.

Expected: `{"all_zero_denominator_ratio": null, "included_terms": 1, "named_mean_positive_denominator": "9/10", "ratio_of_sums": "9/10", "undefined_terms": 1}`

Exact checks: `F(9+0,10+0)==F(9,10)`

### T14-60-missing-denominator (F12, F14)

2 successes lack an eligible outcome denominator.8/(8+2)=4/5 describes encountered parse acceptance only, not sensor completeness.

Expected: `{"completeness_percentage": null, "denominator_basis": "encountered_records", "encountered_parse_acceptance": "4/5", "rejected_count": 2, "success_rate": null}`

Exact checks: `F(8,8+2)==F(4,5)`

### T14-61-zero-vs-unavailable (F12)

Only complete supported eligible enumeration establishes zero. An unread or failed source cannot establish emptiness.

Expected: `{"A": {"count": 0, "status": "supported"}, "B": {"count": null, "status": "not_observable"}, "C": {"count": null}}`

### T14-62-population-mismatch (F21, T14-A2)

30/2 is arithmetically possible but lacks an evidenced matched work population; no efficiency claim follows.

Expected: `{"ratio": null, "reason": "unmatched_population_and_grain", "status": "noncomparable"}`

### T14-63-unit-mismatch (F21, T14-A2)

Tokens and bytes have no evidenced conversion;100+200 cannot be reported as300 tokens.

Expected: `{"reason": "unit_mismatch", "retained_values": [100, 200], "sum": null}`

### T14-64-grain-mismatch (F21, T14-A2)

A fragment is not an additional distinct message; missing assembly/equivalence evidence forbids a7-message total.

Expected: `{"message_count": null, "reason": "grain_mismatch", "retained_values": [2, 5]}`

### T14-65-selection-mismatch (F21)

Different selection and coverage prevent the proposed ranking. Raw separately labeled descriptive observations remain possible.

Expected: `{"causal_effect": null, "ranking": null, "statistical_significance": null, "status": "noncomparable"}`

### T14-66-zero-acceptance (F14, T14-A2)

30/0 is undefined; a known zero denominator is distinct from missing acceptance evidence.

Expected: `{"cost_per_accepted_work": null, "infinity": false, "reason": "zero_denominator"}`

### T14-67-acceptance-cutoffs (F14)

At2 no acceptance receipt exists. At3 acceptance is visible. At4 its revocation is visible; successful attempt does not change.

Expected: `{"acceptance_states": ["unknown", "accepted", "revoked"], "attempt_successes": 1, "merged": null}`

### T14-68-exact-other-ratios (F10)

19/40=475/1000 and11/16=6875/10000 exactly; no binary floating approximation.

Expected: `{"exact_decimals": ["0.475", "0.6875"], "reduced": ["19/40", "11/16"]}`

Exact checks: `F(19,40)==F(475,1000)`; `F(11,16)==F(6875,10000)`

## Open contract questions (claimed values omitted)

- **Q1 General reparenting without authoritative relationship revisions**: Does a runtime reparent event mean a new delegate_existing/handoff relation, or an authoritative correction of historical spawn? Specify stable source key, supersedes target and event validity before expecting subtree transfer. Omitted: No generic latest-parent-wins expected vector. Case23 covers only explicit same-chain historical correction; case24 covers conflicting creators.
- **Q2 Money allocation canonical result wire type**: How must a nonterminating rational allocated currency share, e.g. USD0.01 * 1/3 = USD1/300, be serialized? Domain Money.amount is Decimal with power-of-ten denominator; allocation contract requires exact reduced rational shares. Omitted: No claimed Money/Decimal wire for1/300. Fractional token allocations and terminating money calculations are covered.
- **Q3 Future T14 adapter and refusal-code ABI**: What exact public accounting_v1 request/response adapter and refusal-code mapping should execute these recipes? No numeric product implementation was used; labels here are semantic draft labels, not promises of Rust symbols. Omitted: No claimed native conformance run, frozen fixture digest or builder/reviewer acceptance.

## Provenance limits

No T14 dispatcher launch/session receipt was provided or found in the scoped T14 run paths. Native client session ID, PID, start time and independently observed model/effort are unavailable. The model/effort in the receipt are user-requested routing only. The UUID in the receipt is a newly generated oracle author-run identifier, not a native session or proof of a separate process.

`rig whoami --json` failed because rig is not on PATH. `agentflow-sdlc distribution inspect --target . --json` failed because its installed CLI could not import yaml. No dependency/environment repair or route migration was attempted. The explicit bounded user assignment supplies this author role; no OpenRig seat or policy authority is inferred.

Only task-owned JSON, derivation, recipe and receipt bytes are written. No product build/tests, review, freeze, self-acceptance, commit or push is performed. Receipt output hashes exclude the receipt itself to avoid a self-referential digest; the receipt hash is printed by the recipe command.
