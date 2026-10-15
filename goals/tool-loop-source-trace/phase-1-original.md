# Phase 1 — #33 source-backed tool-loop trace

Status: pass, product acceptance specification; not a candidate QA/review verdict.
pm.analyst / pm-analyst@dendro-pm, Codex / codex-cli / local-cli /
current-session. 2026-10-15 UTC. Key: 33-phase1-source-backed-loop-trace.
Goal https://github.com/smota/dendro/issues/33.
Phase 0 https://github.com/smota/dendro/issues/33#issuecomment-6182101414.

## Outcome and exact additive public journey

When a report shows repeated tool or command failures, I want its trace to show
safe source-backed observations and a qualifying failure span, so I can
independently reproduce the episode/count without opening raw logs or trusting
an unexplained aggregate or claimed membership list.

Use existing public commands with identical DB, rig and UTC filters:

```sh
dendro report tokens --db dendro.duckdb --rig dendro-dev --from 2026-10-15T00:00:00Z --to 2026-10-16T00:00:00Z
dendro trace --metric-id tokens --db dendro.duckdb --rig dendro-dev --from 2026-10-15T00:00:00Z --to 2026-10-16T00:00:00Z
```

The report already exposes `payload.tool_loops`, `payload.outcome_coverage` and
`payload.outcome_coverage_exclusions`. No `report tool_loops` command exists or
is added. The trace keeps its existing token `value`, formula/version, envelope
and inputs. Add these sibling fields to the common trace `inputs` object:

- `tool_loops`: exact copy of the same-snapshot/filter report episode array.
- `outcome_coverage` and `outcome_coverage_exclusions`: exact report copies.
- `tool_loop_trace`: the evidence object defined below.

This makes the existing token trace sufficient; no new metric selector, report
kind, UI or raw transcript browser is required. Common evidence may appear on
other existing trace selectors, with identical scope and meaning. Existing
report arrays/counts/units and existing trace fields are not wrapped, renamed
or reinterpreted. No combined cross-unit loop total is introduced.

## Evidence object and input completeness

`inputs.tool_loop_trace` has exactly these required top-level members; additional
safe metadata may be additive without changing their meaning:

| Field | Required observable meaning |
| --- | --- |
| `formula_version` | `k11-tool-loop-trace-v1`, the evidence/rule contract, not a new detector definition |
| `definition` | Plain description of whole-run episodes, fixed 3 consecutive failures / inclusive 600-second sliding triple, harness ordering, breaks and unknown treatment |
| `scope` | Same UTC from/to, optional rig, committed analytics snapshot/source provenance and applied filters |
| `status` | `complete` or `insufficient_evidence` for reconstruction of the reported stored-observation result; not a replacement for reader coverage |
| `limitations` | Explicit safe reasons for missing provenance/order/equality evidence, unread/unsupported units, and inherited source-state limits |
| `observations` | Complete safe scoped observation stream below, including nonmembers and run breakers |
| `sources` | Safe source/generation references and committed ingestion provenance, including unavailable/not-checked states |
| `episodes` | Episode membership/witness claims cross-referenced to observations and report rows |

`observations` includes every persisted observation that participates in the
current report's scoped loop evaluation, not just failures or detected members.
Include successes, unknown outcomes/input, other tools and actual Codex seat
calls that break runs. Include applicable stored Codex observations excluded
from trusted membership by unresolved/colliding linkage in an unscoped trace,
with explicit exclusion reason. Do not emit records excluded from a selected
rig as though they belonged to that rig; retain the existing exclusion counts.
Result/output messages are not fabricated extra calls or dumped as evidence.
No silent truncation of the stream is permitted under a `complete` status.

Each observation carries these named fields:

| Field | Meaning |
| --- | --- |
| `observation_id` | Unique safe response reference, unambiguous within this trace |
| `source_id`, `source_key`, `generation` | Actual stored source identity, record key and producing file generation; null plus an explicit limitation if unavailable |
| `harness`, `unit`, `session`, `seat`, `linkage_status` | Report-consistent identities, distinct tool_call/command_execution units, nullable unresolved seat and recorded linkage status |
| `tool` | Existing safe tool label; Codex command unit uses fixed CommandExecution, actual seat calls are distinct boundary observations |
| `timestamp` | Stored source timestamp in UTC, or null; never an imputed time |
| `outcome` | `failure`, `success` or `unknown`, from the stored supported native outcome; a Codex call boundary remains unknown |
| `input_fingerprint` | Existing keyed opaque equality value or null for unknown input; no raw input or new unkeyed hash |
| `native_seq`, `input_rank`, `detector_rank` | Stored native sequence, snapshot-relative input enumeration rank, and effective within-session detector order; excluded observations have null detector_rank |
| `exclusion_reason` | Null for included observations; explicit safe reason for excluded membership such as unresolved Codex seat |

Ranks are ordering metadata, not source timestamps/generations or an episode
claim. They must be unambiguous and internally consistent, with no duplicate
included detector ranks in a session. `input_rank` records the actual persisted
input enumeration at this snapshot; it is not invented source chronology.
The trace must describe its ordering basis so the checker can derive effective
order rather than trusting an episode's listed member order. Any unavailable
source position remains unavailable, not guessed from adjacency/call ids.

The safe source reference identifies a source record/generation without opening
or returning the raw file. `sources` supplies the committed prefix revision /
ingest provenance when recorded, identifies what that revision describes, and
uses null/explicit unavailable status when historical producing-ingest metadata
is absent. A latest source revision is never relabeled as an older row's
producing revision. File generation comes from stored facts, not a parsing guess
or a default 1. OpenRig linkage/source revisions retain K9's existing latest-
ingest limitation; this is not per-row OpenRig replay/generation work.

## Preserve rules; expose the evidence needed to replay them

Keep current report scoping before ordering: timed observations inside UTC
[from,to), including from and excluding to, plus the existing untimed loop
observations retained in scoped sessions. Out-of-window timed calls do not
participate or form barriers. Unknown timestamps stay null and visible.

Claude/Pi keep their existing native-sequence detector behavior and counts.
At entry the detector stable-sorts native_seq, retaining its input enumeration
for ties; the trace exposes native_seq and input_rank so this exact order is
reconstructible. `input_rank` is snapshot-relative evidence, not a new tie rule
or a promise of cross-generation physical ordering. Preserve existing seat/
session attribution and linkage meanings, including nullable report seat where
already applicable. Do not change Claude/Pi to Codex's ordering or exclude
existing reported episodes just to simplify trace output.

Codex preserves accepted #40: native command executions have their own integer
exit-code outcome and keyed exact lexical equality; actual seat calls remain
unread tool_call boundaries. No wrapper/result join is inferred. Within each
session, start with source_id/generation/native_seq order, retain untimed
observations as source-position anchors, and timestamp-sort each maximal timed
segment between anchors, using the same native order for ties. Filter first;
never move a timed observation across an untimed anchor. Unique seat linkage
is required for trusted Codex episodes; unresolved/collision observations do
not merge through a null seat. Applied coverage and reader completeness retain
their existing meanings and unknown counters.

Within the accepted harness order, same tool/unit and same known fingerprint
failures in one harness/session/seat form a run. Success, unknown outcome/input,
different input/tool/seat, or an actual intervening other call breaks it under
existing definitions. Sessions/units/harnesses never merge. An untimed failure
stays in its run and count; it cannot be the first/third endpoint of a qualifying
triple. An untimed middle member can qualify with known endpoints. Codex's
anchored endpoint span must be nonnegative. Preserve K11's sliding triples and
one full-run episode, including failures before/after the qualifying triple.
No threshold customization, new normalization or outcome inference is allowed.

If a current detector answer contradicts the accepted definitions or has no
valid source-backed witness, do not alter counts or quietly invent a witness:
report insufficient evidence and return the concrete discrepancy to product/
architecture before broadening this trace change. #33 is not authorization for
an unrelated detector correction or a new interpretation of invalid time order.

## Episode/witness claims and independent checking

Each `episodes` entry contains `report_index` (zero-based index in
inputs.tool_loops), `members` (ordered observation_id references for the entire
run), `qualifying_triple` (exactly three consecutive member references), and
`span_seconds` (unrounded source endpoint difference in seconds). All members
must resolve to included observations with the matching harness/unit/session/
seat, tool, failure outcome and known equality. Different same-label/count
episodes separated by a breaker remain distinct report rows and member sets.

The deterministic witness is the first qualifying consecutive triple encountered
left-to-right in the full run, not the tightest span or always its first three.
Its first/third source timestamps must be known; preserve any untimed middle.
No empty witness, invented endpoint or formatted integer-second truncation.
For supported examples, witness span preserves stored sub-second precision;
600.001 seconds must not be displayed/rechecked as an inclusive 600.

An independent trace-only checker must, without raw logs, database access,
owner key or candidate detector code:

1. Validate source/observation references and provenance availability, scope,
   ranks, linkage/unit eligibility and advertised limitations/coverage.
2. Derive per-session order from observations and the stated harness ordering,
   independently construct runs from all included observations, and apply K11.
3. Derive the entire episode set/full-run counts and first qualifying witness,
   compare claimed membership/witness/span, and compare the full projected
   episode array/multiset and any displayed aggregate with inputs.tool_loops.
4. Reject duplicate/omitted members, invalid/nonconsecutive/late witness choice,
   wrong spans/counts or cross-session/unit merges, and report inability to prove
   source/order completeness as insufficient evidence, not a successful zero.

The claimed members/witness are redundant, checkable explanations, not the
checker's input oracle. A trace alone cannot cryptographically prove that a
producer omitted no source records; independent QA verifies completeness and
native outcomes against frozen literal synthetic source/expected vectors.
No new signature, raw-input recovery or authenticity service is implied.

## Expected vectors and zero/unknown behavior

F = same known-input failure in one uniquely linked session; B = actual call
boundary; ? = untimed. Lists below are in the applicable detector/source order
unless stated otherwise. Each row is a separate fixture, with literal source
keys/generations and independent expected membership, not candidate goldens.

| Input | Expected complete trace result |
| --- | --- |
| F@0,F@300,F@600 | One count-3 episode, members 1–3, witness 1–3, span 600 |
| F@0,F@300,F@600.001 | No episode; observations retained |
| F@0,F@400,F@700,F@800 | One count-4 episode, witness members 2,3,4, span 400 |
| F@0,F@400,F@700,F@1101 | No episode, failing triple spans 700 and 701 |
| F@0,F@1,F@2,F@3,F@4 | One count-5 episode, first witness 1,2,3, span 2 |
| Codex F@0,F@?,F@2 | One count-3 episode, span 2, null middle time remains visible |
| Codex F@?,F@0,F@1,F@2 | One count-4 episode, witness 2,3,4, span 2 |
| Codex F@500,F@?,F@0 | No episode: anchored endpoints reverse time |
| Codex F@0,F@1,B@?,F@2,F@3 | No episode; untimed barrier is present in observations |
| Three failures, success, three equal failures | Two count-3 episodes with disjoint members; success is visible |
| Two failures separated from two others by unknown input/outcome or different tool/input | No bridging episode; breaking observation is visible |
| Same failures separated only by result/output messages | Same call run; no fabricated extra call/barrier |
| Same apparent input in different sessions/units/seats | No cross-partition episode |
| Codex unresolved/colliding linkage | Observations/exclusion reason visible unscoped; no trusted episode |

Window [0,600): F@0,F@?,F@599 gives one count-3 episode; F@0,F@?,F@600 gives
none because the final endpoint is excluded. Codex timed source order 600,0,300
sorts to 0,300,600 within its timed segment. A timed segment cannot cross an
untimed barrier. Same-time ties retain the accepted source/native ordering.

For zero episodes, the trace still returns all safe scoped inputs/breakers,
coverage and reasons. A complete read success-only source can prove zero
observed episodes; no sources (0/0), an unread legacy prefix, unsupported Codex
seat-call outcomes and missing outcome/input/time/linkage remain distinct gaps.
Reader coverage is not changed to read merely because trace evidence exists.
`status: complete` means all persisted inputs/provenance needed for the reported
observed-observation result are supplied; it never certifies unread bytes.
Coverage limitations must accompany any reported observed zero/partial history.

If source key/generation/order/fingerprint evidence required to substantiate an
existing episode is unavailable, retain report counts unchanged but mark
insufficient_evidence with precise limitations. Never make up identifiers,
force all unknowns into one input group, omit the episode silently, or claim
independent source-backed acceptance of that result. Missing fingerprint as a
known detector input remains a run breaker; missing exportable evidence for a
claimed known input is an insufficiency, not a new detector input.

## Read-only / legacy / source-state contract and privacy

A supported existing analytics DB can supply trace from its persisted facts and
metadata without ingest, source reads, migrations, new fingerprint computation
or owner key access. It must work if source files are unavailable and if the
local key is absent, under K8's existing report-only behavior. Reuse stored keyed
equality values. Unsupported legacy schemas retain their existing refusal;
supported DBs missing provenance get explicit insufficiency, not fake completeness.

Source presence/state is `not_checked` when current filesystem state has not
been verified; distinguish any actually recorded changed/gone/superseded state
from unknown/unavailable historical evidence. Do not claim unchanged/current
bytes, universal gone-source detection or source replay from metadata alone.
Retain file-source append/generation evidence and K9's OpenRig reset/retention
limits; #13 recovery/reconciliation is not silently implemented here. Identical
snapshot/filter reads are reproducible; append/replacement changes results only
under existing ingestion/metric semantics with truthful generation references.

No trace field contains raw tool input/command, prompt/output, source record
body, reversible encoding, unkeyed input digest or key material. Identifiers/
labels/source references retain existing sealing/sanitization and must remain
distinct when their raw secret-bearing values would redact alike. Exact keyed
lexical equality and unit/representation separation remain unchanged. No key
rotation or missing-key behavior change is introduced to generate trace IDs.
Response-local IDs/order ranks may reference safe stored facts without a key.

## Reproduced entry evidence and proof ownership

Built entry HEAD 65a75a9e52ed8bfee52d119d278391ad3b39dca4, debug binary
SHA-256 f1d250493d6c707280144a5559b92d580aa4de82a4010aa96a66485791a598d0.
Analyst baseline-probe.py uses only synthetic Codex/OpenRig roots and a fresh
analytics output. It emits native failures at 0,400,700,800, an actual seat call
at 900, then failures at 1000,1100, with one resolved session/seat and fake
privacy canaries in command/boundary input. Normal public ingest succeeds.
The report shows one Codex command_execution episode, count 4; the token trace
has neither inputs.tool_loops nor loop observations/equality/member/witness
evidence. It cannot independently establish that episode from its existing
inputs. This reproduces the present public gap, not a candidate pass.

Source hashes remain unchanged after ingest/report/trace; canaries are absent
from captured report/trace. Commands, source/binary hashes, safe public outputs
and aggregate result: baseline-probe.py and baseline-probe/summary.json plus
siblings. No owner logs/key read, real-corpus census or decoded-storage privacy
pass is claimed. Existing detector/source/fingerprint/storage code was inspected;
implementation representation, provenance joins and performance remain delivery
unknowns. A known row-generation field does not prove every historical producing-
ingest revision is recorded.

Phase 3 freezes independent literal-byte source fixtures and a trace-only
checker before build, including expected ordered nonmembers/breaks, source/
generation refs, witnesses/spans and report projections. Checker must not import
candidate parser/metrics or require raw logs/key at replay time. Source-fixture
construction/proof is separate from trace-only replay. Pin snapshot/filters,
baseline/candidate binaries and normalization; preserve original oracle evidence.
Include deliberately invalid trace copies (missing breaker, forged membership,
wrong/nonconsecutive/late witness, span/count/generation changes) and require the
checker to reject or disclose insufficient evidence as applicable.

Independent QA runs actual public mixed Claude/Pi/Codex journeys, supported
report-only/missing-key/source-unavailable/legacy controls, append/reingest and
all vectors. Full decoded privacy across persisted text/DB/WAL/temp and report/
trace/stdout/stderr/log/error/export surfaces uses synthetic canaries and controlled
synthetic-key separation; no owner key. Preserve token/intake/handoff/activity/
median/loop counts, applied coverage and source safety. Only declared trace
additions/provenance change. Cargo4 plus independent QA5/review6 remain gates.
Architecture owns assurance and shared detector/trace design; do not fork the
rules or modify metrics to make an explanation fit.

## Return and release boundary

One local v0.2.0 minor outcome, no threshold/configuration, handoff-loop (#12),
new report/UI, metric/parser definition, raw transcript access, source mutation,
#13 replay expansion, schema prescription, performance guarantee or #12/#13
acceptance. No main edit, push, PR, merge, tag, integration or publication.
Prior #30/#40/#46/#47 product decisions remain bound to their exact candidates;
no historical hold is revived or source protection waived by this trace work.

Return this pass/digest, evidence and unknowns to pm.manager for independent
acceptance. Only governed handoff deliver may send accepted Phase 1 to available
delivery. No code, file-by-file implementation list or direct delivery contact
is authorized by this analyst specification. Current autonomous policy applies
without invented person confirmation; architecture chooses assurance/solution.

References: current Phase 0/steering brief, issue/F5; R1 AC6/AC7/AC10 and K8/K9/
K10/K11; accepted #40 amended unit/coverage/anchor contract; current public
report/trace, stored tool/native sequence/generation and keyed privacy behavior.
