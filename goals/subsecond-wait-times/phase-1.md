# Phase 1 — #34 fractional handoff waits

Status: pass, product proof contract; not candidate QA or review acceptance.
Signer: pm.analyst / pm-analyst@dendro-pm, Codex / codex-cli / local-cli /
current-session. Date 2026-10-14. Key: 34-phase1-fractional-wait-contract.
Goal: https://github.com/smota/dendro/issues/34.
Phase 0: https://github.com/smota/dendro/issues/34#issuecomment-6178078300.
Specification owner pm.analyst; product acceptance owner pm.manager.

## Outcome and existing public journeys

When I compare handoff waits between seat pairs, I want samples and medians
calculated from recorded timestamp differences before presentation rounding,
so I can see measured waits without whole-second truncation.

After normal local ingest, the operator uses these existing JSON journeys with
the same database, rig and UTC window. Paths, rig and times are operator inputs:

```sh
dendro report handoffs --db dendro.duckdb --rig dendro-dev --from 2026-10-14T00:00:00Z --to 2026-10-15T00:00:00Z
dendro trace --metric-id medians --db dendro.duckdb --rig dendro-dev --from 2026-10-14T00:00:00Z --to 2026-10-15T00:00:00Z
```

Report values are in `payload.pair_medians`. The same rows occur at trace
`value` and `inputs.pair_medians`. `wait_samples` and `wait_median` measure
seconds; `intake_samples` and `intake_median` still measure tokens. `from` and
`to` remain seat identifiers. Omitting `--rig` uses the existing unscoped
journey. Shared payload copies on other existing report kinds must agree.
No new report command or human-readable renderer is required.

## Duration, precision and presentation rule

For each currently eligible handoff and its currently selected receiver claim,
compute `wait = claim timestamp - handoff timestamp` in seconds. Normalize
accepted timezone offsets to UTC before subtraction. Preserve recorded
fractions through duration calculation and median calculation; do not truncate,
round, bucket, or convert either endpoint to whole seconds first. No report
window origin or unrelated activity-event boundary enters this subtraction.

The end-to-end precision promised by this bounded correction is one microsecond
on accepted, valid source endpoints: whole seconds, milliseconds and six decimal
fractional digits must survive. An even median may be half a microsecond and
must survive too. This is a precision guarantee, not an assertion that the
source clock is accurate to a microsecond. Existing OpenRig source clock formats
and parsing remain; this does not promise nanosecond storage or invent more
precision than the source provides. Fractions beyond six fractional decimal
places in seconds are outside this contract's exact-preservation guarantee.
The reproduced 999-nanosecond storage loss is an explicit retained limit, not
proof that those timestamps have microsecond accuracy. No schema widening or
new timestamp-parser policy is requested here.

Presentation choice: existing public JSON fields expose computational values
as finite JSON numbers, with no deliberate fixed-decimal presentation rounding.
Use a normal round-trip number representation; insignificant trailing zeroes or
scientific notation do not change the numeric result. Thus 24.734 seconds is
numerically 24.734, not 24, 24.7 or 24.73. A 0.0000005-second median is not
rounded to zero or a whole microsecond. No formatting string or unit suffix is
inserted into numeric arrays. Binary floating-point representation error may
be distinguished from deliberate precision loss; for the small expected
vectors below, independently parsed numeric answers must be within 1e-12
seconds of the stated exact decimal. This tolerance is not permission to
quantize endpoints, samples or medians. Architecture owns numerical types and
algorithm, with evidence for supported-range behavior.

There is consequently no decimal display tie to round up, down or to even.
A median with odd sample count is its sorted central computational value; with
even count, it is the arithmetic mean of the two central computational values.
Duplicates retain their multiplicity; the sample array itself is not sorted or
reordered for display by this change. Never feed rounded display values back
into the median. A valid equal-time claim gives a zero sample and zero median.
No eligible sample gives `wait_samples: []`, matching empty sample keys, and
`wait_median: null`; no data is never fabricated as zero or infinity.

The medians trace formula must identify wait units as seconds, the
claim-minus-handoff calculation before rounding, and the odd/even median rule.
Its formula-version provenance must identify this changed wait rule; architecture
chooses the stable version identifier. Intake-token arithmetic does not change.
Documentation must state this numeric/presentation rule and retained precision
limit. No additional display fields or fixed-width output are required.

## AC4 compatibility disposition

AC4 already conserves integer observation seconds by subtracting elapsed
whole-second prefixes anchored at each observation span's start. It does not
truncate each segment independently. Reproduced source inspection shows that
this differs from both fractional wait calculation and a universal round-to-one-
decimal rule. Applying the AC4 integer prefix quantizer to each individual wait
would retain the defect this issue exists to remove.

The shared principle is elapsed timestamp duration before lossy presentation,
with no accumulation of discarded fractional segments. This issue changes
only wait samples and the medians derived from them. Keep AC4's five bands,
life/window clipping, demand/state boundaries, whole-second allocations and
exact band-sum conservation unchanged. Activity fields remain whole seconds;
wait fields may contain fractions. Do not claim these different metrics share
an identical quantizer. This is the explicit reconciliation of Phase 0's
"same duration rule" intent with the verified AC4 implementation; the manager
must accept this disposition before delivery.

## Membership, endpoints, ties and filters remain fixed

Preserve the existing handoff definition, historical attribution, chains,
receiver-claim selection and source keys. Select the first eligible receiver
claim on the existing item/chain and destination, at or after the handoff,
ordered by source time and existing source-order tie break. Do not substitute
an actor for destination, infer a link, change same-time event/transition
selection, deduplicate equal-duration independent handoffs, or alter intake
claim/transition resolution. Claim-source ties retain their existing selected
source key. Zero duration remains eligible.

`[from,to)` is UTC and continues to select handoffs by the handoff event time.
A handoff at `from` enters; one at `to` does not. Receiver lookup uses the
existing committed queue history, including eligible claims at or after `to`;
this issue does not cap a wait at the window end or introduce censoring.
Rig/unscoped filtering and attribution remain unchanged. A later ingest can
provide a previously missing claim under existing behavior; the formula does
not manufacture one from ingestion time or the current wall clock.

A handoff with no usable selected claim contributes no numeric sample and
increments that pair's existing `wait_invalid`, exactly as today. This includes
missing/unparseable claim timestamps after the existing source-specific fallback
rules, a wrong destination/chain, and a claim strictly before the handoff with
no later eligible claim. Such a negative difference must never be clamped to
zero, made positive, or admitted to the median. A valid handoff with one valid
and one missing-claim peer retains its one valid sample and one invalid count.
If a missing/unparseable handoff timestamp prevents establishing a handoff edge,
there is no trusted wait or new attributed pair from that row; do not invent an
epoch handoff or zero-duration sample. Existing source exclusion/reporting is
preserved; no new invalid-handoff counter is implied.

Keep `wait_sample_keys` positionally aligned with `wait_samples`, with unchanged
handoff/claim source keys and source generations. The trace continues to expose
the existing evidence, formula and source revisions; no raw source body,
command, prompt, key or secret is added. Unknown or unsupported endpoints do
not authorize guessed timestamps or joins.

## Independently timestamp-derived expected examples

All endpoints below are synthetic UTC timestamps on 2026-10-14 in one valid
pair, with independently eligible claims and distinct sample keys. Values are
mathematical decimal seconds; these expected answers do not call candidate
metrics or generate expected data from candidate output. Endpoints are written
as time-of-day; source fixtures must include full dates/timezones.

| Case | Handoff -> receiver claim, for each sample | Exact samples (s) | Exact median (s) |
| --- | --- | --- | --- |
| Disclosed defect | 00:00:00.000000 -> 00:00:24.734000 | [24.734] | 24.734 |
| Fractional endpoints across second | 00:00:00.900000 -> 00:00:01.100000 | [0.2] | 0.2 |
| Odd, unsorted input | 00:00:00 -> 00:00:00.154; 00:00:00 -> 00:00:00.049; 00:00:00 -> 00:00:00.051 | [0.154,0.049,0.051] | 0.051 |
| Even, round-after-computing adversary | 00:00:00 -> 00:00:00.049; 00:00:00 -> 00:00:00.149 | [0.049,0.149] | 0.099 |
| Duplicate central values | 00:00:00 -> 00:00:00.1; 00:00:00 -> 00:00:00.1; 00:00:00 -> 00:00:00.9 | [0.1,0.1,0.9] | 0.1 |
| Whole-second regression | 00:00:00 -> 00:00:01; 00:00:00 -> 00:00:03; 00:00:00 -> 00:00:05; 00:00:00 -> 00:00:07 | [1,3,5,7] | 4 |
| Equal timestamps | 00:00:00.100000 -> 00:00:00.100000 | [0] | 0 |
| Microsecond | 00:00:00.000000 -> 00:00:00.000001 | [0.000001] | 0.000001 |
| Half-microsecond even result | 00:00:00 -> 00:00:00; 00:00:00 -> 00:00:00.000001 | [0,0.000001] | 0.0000005 |

Exact decimal arithmetic derivations are retained in
`analyst-decimal-examples.json`; these are analyst expectations, not the
Phase 3 frozen QA oracle.

The even adversary's 0.099 must not become 0 (whole-second truncation), or 0.1
from pre-rounding samples to tenths. No fixed-decimal display tie is invoked.
Equivalent-offset example: 02:00:00.900000+02:00 ->
00:00:01.100000Z is 0.2 seconds on the same date.

| Endpoint/membership case | Expected answer |
| --- | --- |
| One valid handoff, no usable receiver claim | samples [], keys [], invalid 1, median null |
| One valid 0.25 s sample and one missing claim | samples [0.25], one aligned key pair, invalid 1, median 0.25 |
| Unparseable claim with no usable fallback | invalid 1; no sample, no median contribution |
| Only claim is 1 microsecond before handoff | invalid 1; no negative or zero sample |
| Claim for another destination/chain only | invalid 1; no guessed match |
| Handoff timestamp missing/unparseable, no usable native fallback | no trusted edge from that row, no fabricated zero sample |
| Two eligible claims at equal timestamps | selected existing source-order winner/key; one sample, not two |
| No eligible handoffs | no fabricated pair row or zero median |

For the window `[00:00:00Z,00:01:00Z)`, a handoff at 00:00:00Z followed by a
claim at 00:01:00.250Z has wait 60.25 s. A handoff at 00:01:00Z is excluded.
A handoff at 00:00:59.900Z with claim at 00:01:00Z has wait 0.1 s, not zero
and not a missing claim. Same input snapshot and filters give identical report
and trace sample arrays, selected keys, invalid counts and medians.

## Reproduced entry behavior and compatibility

Source HEAD inspected and built for the synthetic probe:
13423047ba4f4094e49a9267fcf2278be0da43ab. No product code was edited.
`baseline-probe.py` generated a private synthetic OpenRig SQLite source under
this goal directory, ran normal ingest with only that explicit root, then
`report handoffs` and `trace --metric-id medians` for `probe-rig` and the
00:00:00–00:01:00 UTC window. No owner data or credentials were read.

Observed at this baseline: the 24.734 sample/median is 24; odd samples
0.049,0.051,0.154 all become zero and median zero; even samples 0.049,0.149
both become zero and median zero. Zero remains zero. Missing/invalid/pre-handoff
claims each have no sample, invalid 1 and null median. Public trace demand
rows preserve .000001 seconds, while a source .000000999 endpoint becomes a
whole-second timestamp. The schema's TIMESTAMP precision and this public probe
support the microsecond guarantee and the retained finer-precision limit.
Report and trace pair-median rows match. The generated source SHA-256 is
unchanged after ingest/report/trace. Binary/source hashes, raw synthetic public
outputs and aggregate observations are retained in `baseline-probe/summary.json`
and its sibling files. This is current-behavior evidence, not candidate proof.

Compatibility: keep existing JSON arrays, number types, key alignment, null
no-data convention, pair identifiers and count/invalid meanings. Wait values
intentionally gain fractions; numeric clients must not expect integer waits.
Whole-second input values remain numerically equal. A report-only read of a
currently supported analytics database with retained microsecond timestamps
must compute corrected waits without reingest or writes. The new reader does
not recover precision already lost before those timestamps were persisted.
Existing refusal of unsupported database schemas remains; this slice does not
introduce a database migration or relax source-root/key rules. Reports and
traces keep their existing read-only/missing-local-key behavior.

## Proof ownership, regressions and scope

Phase 3 freezes an independent oracle before build: literal synthetic endpoint
fixtures, exact decimal duration/median answers, source membership/key answers,
commands/filters, baseline commit/binary hashes and normalization rules. QA owns
the independent expected answers and their provenance. Preserve original R1/M2
oracle artifacts; record each intended wait sample/median delta separately.
The builder cannot silently regenerate goldens from candidate parsing/metrics.
The expected change is wait fractions and derived M2 medians plus their formula/
documentation provenance, not a blanket M2 rebaseline.

Candidate verification must exercise both existing public journeys, not only a
median helper. Cover all examples above, whole-second regression, sample/key
alignment, stable ties, handoff-window boundaries and post-window claims,
report-only existing-DB behavior, unchanged rerun/new append, and source hashes.
On a pinned common input snapshot compare entry baseline and candidate:
only intended wait values/provenance change. Preserve handoff counts/returns,
selected samples and keys, invalid counts, rig/seat/chain attribution, token
categories/totals, intake windows/costs, activity bands/conservation, loop units/
counts/coverage, privacy/sanitization and source immutability. Fake canaries
remain absent from analytics output/storage under the existing privacy gates.
Do not inspect an owner key. Required current-workspace cargo check, cargo test,
cargo clippy with -D warnings and cargo fmt --check remain verification gates.
QA5 and independent review6 retain candidate acceptance authority.

Unknowns for delivery: candidate representation/performance and full-corpus
M2 deltas have not been measured. Synthetic evidence establishes current
behavior, not every OpenRig version's native clock resolution. Phase 3 must
pin applicable source formats and all intended oracle deltas; precision beyond
microseconds is not accepted as exact by this contract. No new performance
threshold, parser, dashboard, latency target, timestamp attribution policy or
source-generation recovery is added.

This is one local v0.2.0 correction. G2 #12 F12-1 compares R1 and #12 on the
same build containing #34; this contract grants no #12/#13 pass. Excluded:
new report/UI, token/intake/category/loop changes, AC4 changes, guessed links,
main edits, push, PR opening, merge, tag or publication. Existing integration
and publication holds remain with their owners.

Return this Phase 1 pass, spec digest, evidence and unknowns to pm.manager for
independent product acceptance. Only the governed `agentflow-sdlc handoff deliver`
service may send accepted criteria into delivery when free. No direct delivery
contact or implementation plan is authorized by this analyst contract.

References read: Phase 0 and steering brief; R1 §4/§4.1, M2, AC4/AC5/AC7/AC9;
clarifications K4/K5/K6/K8/K9; PRODUCT_SPECIFICATION AC4 disposition and O1 triage;
ACCEPTANCE_G2_STAGED other-items/F12-1 dependency; current public CLI, timestamp
parsing/storage and report/median calculation; AC4 prefix conservation source.
