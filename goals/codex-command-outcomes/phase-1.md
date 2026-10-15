# Phase 1 — #40 Codex command outcomes

Status: pass (product acceptance specification, not a candidate verification).
Owner and signer: pm.analyst, pm-analyst@dendro-pm. Runtime: Codex;
executor codex-cli; transport local-cli; current-session boundary.
Date: 2026-10-14. Issue: https://github.com/smota/dendro/issues/40.
Phase 0: https://github.com/smota/dendro/issues/40#issuecomment-6175637884.
Key: 40-phase-1-reverse-seam-amendment-1. No implementation branch or code changes.
Amendment 1 responds to the verified Phase 2 return:
https://github.com/smota/dendro/issues/40#issuecomment-6176239200.
It supersedes the original Phase 1 specification bb3474fe for the clarified
behavior below; the original transition remains historical evidence.

## Outcome and public journey

When Codex command failures never appear in tool loops, and the only notice is a
warning on ingest, I want the loops report itself to show what was read and to
count Codex command failures from their own exit codes, so I can see a repeated
failure without mistaking “not read” for “no loop” and without trusting a guessed
join.

The operator ingests existing local sources with `dendro run`, then uses the
existing public JSON journey:

```sh
dendro report decisions --db dendro.duckdb --rig dendro-dev --from 2026-10-14T00:00:00Z --to 2026-10-15T00:00:00Z
```

The database, rig and UTC period are operator selections. Omitting `--rig`
selects the existing unscoped journey. `tool_loops` is a payload field, not a
report kind or a new command. In this same JSON response,
`payload.tool_loops` and its sibling `payload.outcome_coverage` show episodes
and coverage for each harness and outcome unit. The operator can distinguish an uncovered unit,
an unknown input or outcome, a covered unit with no detected episode, and a
detected command-execution episode. They need not consult ingest warnings to
understand coverage. This is one outcome and one eventual delivery handoff.

Release intent: v0.2.0, minor, local build. Entry baseline: released v0.1.1.
No publication authorization. This is outside #12/#13 and is not a #13 gate.

## Definition amendment scoped to #40

This extends R1 §4 and K11 for a distinct Codex observation unit. It does not
redefine a Claude or Pi tool call or reclassify a Codex seat call.

Coverage placement: `payload.outcome_coverage` is an additive array beside the
existing `payload.tool_loops` array. Preserve each episode's `session`, `tool`
and `count` fields and add `harness`, `unit` and `seat`; do not wrap or replace
the existing array. A Codex episode has `harness: "codex"`,
`unit: "command_execution"`, `tool: "CommandExecution"`, and its uniquely
resolved seat. Claude/Pi episodes have unit `tool_call` and preserve their
existing meanings/counts. There is no combined cross-unit loop total. No new
text renderer is required; the JSON unit explicitly labels the observation.
The same additions may appear wherever the existing shared payload is emitted.

Each coverage entry has `harness`, `unit`, `status`, `sources_total`,
`sources_read`, and `unknowns`. Status is exactly `read` or `not read`; unit is
`tool_call` or `command_execution`. Always emit the Claude/tool_call,
Pi/tool_call, Codex/tool_call and Codex/command_execution entries, including
zero-source and zero-episode responses. `unknowns` exposes separate counts for
`input`, `outcome`, `timestamp` and `linkage`; one observation can contribute to
several counts, so these counts must not be summed as unique executions.
Unknown counters describe observed command items within the loop report's
scope; incomplete source coverage is an additional gap, not a zero-unknown
claim about unread bytes. For unsupported tool-call outcomes use null unknown
counts, not a fabricated zero. Claude/Pi unknown projections must not change
their existing episode semantics.

`read` means applied coverage of the persisted complete source prefix for the
current source generation, not merely an installed reader capability. It does
not promise known outcome/input/time/linkage on every item. Missing fields on a
recognized CommandExecution are observed unknowns; unsupported source shapes
cannot be silently counted as covered. Codex/tool_call always says `not read`
and `sources_read: 0`; CommandExecution is never its outcome proxy. For other
units, `read` requires at least one in-scope source and completion on every
in-scope current generation: `sources_read == sources_total > 0`. Otherwise
status is `not read`. Counts distinguish partial coverage (1/2), no coverage
(0/2), and absence of sources (0/0). Covered Claude/Pi sources say `read` without
reinterpreting their outcomes.

Coverage is source based, independent of whether a command or episode lies in
the selected time window. Rig scope includes only uniquely linked sessions in
that rig. In the unscoped report, unresolved/colliding linkage is visible as
unknown and cannot produce a trusted Codex episode. Rig-scoped output discloses
that unresolved sources were excluded and their count, without attributing
them to the selected rig; architecture chooses the additive disclosure field.
Source counts/coverage refer to the committed snapshot, not unread future
appends. Do not enumerate source paths or expose command content for coverage.

A report-only open of a pre-reader database must succeed with legacy Codex
coverage `not read` and no claim that its absent command rows mean zero loops.
It must not mutate or backfill sources. On the next normal ingest, process the
previously watermarked complete Codex prefix as well as new complete lines.
Completion and resulting facts must become visible together or neither does;
a failed batch cannot leave a false `read` mark or partial duplicate history.
Backfill must not reset source generation, recount usage/tokens, or change
intake endpoints. Unchanged reingest is idempotent. New/replaced generations
must establish their own completeness, never inherit the previous generation's
mark. An incomplete trailing line is not a completed observation; completing
it on append must converge to the complete-input result. The architect owns
storage/schema/reader-version representation and compatibility assessment.

An empty supported source can be read: successful complete-prefix processing
marks completion even when there are zero command facts. The same applies to a
supported source with only successes or supported envelopes with no commands.
A source containing only malformed/unsupported records is not proof of coverage:
it remains `not read` with a visible source/read gap. A mixed source with a
malformed or unsupported observation that could conceal a command also cannot
claim complete coverage; supported non-command records are not such a gap.
Recognized command items with invalid input or exit fields can be fully read
with unknown counters.

The new observation is the native `event_msg` / `item_completed` item of type
`CommandExecution`. Its own integer `exit_code` settles its outcome: zero is
success; any nonzero integer is failure. Missing, null, boolean, string, fractional, or
otherwise invalid codes are unknown and break a failure run. Item `status`,
stdout, stderr, formatted output, prompts and other text cannot settle it.
An exec call can contain multiple executions; count executions individually.
Never join an execution to `custom_tool_call` or `function_call` by time, turn,
adjacency, process identifier, or an invented call identifier.

For this unit, the fixed tool identity is `CommandExecution`. Input is the
item's own `command`, not output, parsed command descriptions, cwd, an exec
wrapper's input, or a prompt. For the native command array, equality uses its
exact lexical JSON slice, trimmed only at the outside of the whole slice.
Preserve array order, element boundaries, internal whitespace, quoting and
escapes. Do not reserialize, sort, join arguments, or trim individual elements.
If a supported source has a command string, use that string's body with only
outer whitespace trimmed; do not silently equate strings and arrays. A command
array must contain at least one element, all of them strings.
`[]`, `[null]`, objects, numbers, null, a missing command or an unavailable
exact slice have unknown equality and break a run. `[""]` is a valid distinct
lexical input; `[" "]` is also valid and does not equal `[""]`. This judges
observed equality, not whether a command can execute. If command strings are
independently supported, `""` and whitespace-only bodies are unknown after
outer trimming; `" x "` equals `"x"`, but neither equals `["x"]`. String support
is optional, and unsupported strings remain unknown. Unknown equality never
uses a shared empty key or a guessed match. Delivery must prove any additional
source shape before claiming it is covered.

Comparison must use a keyed opaque fingerprint of the exact trimmed input and
unit identity, with the existing AC10 privacy property: without the local key,
a known input cannot reproduce the stored fingerprint. Compare before display
redaction, so two different secret-bearing inputs remain distinct even if
their redacted displays would match. Raw input is transient only. Do not store
or emit it, a reversible encoding, key material, command output, or a prompt.
The architect owns the algorithm; a plain unkeyed input hash is insufficient.
K8's first-run and missing-key behavior remains unchanged.

Apply K11: same unit/tool, same exact trimmed input, same seat, one
(harness, session). Codex-specific ordering starts from deterministic source
order within that session: stable source identity, generation, line and item
position, without truncated indices or guessed temporal joins. An untimed
observation anchors its source position. Sort each maximal contiguous segment
of timed observations between those anchors by timestamp, then source order
for ties. Do not move timed observations across an untimed anchor. This is an
explicit limit to global timestamp ordering; Claude/Pi ordering is unchanged.
A success, unknown outcome/input, different input/tool/seat, or intervening
other tool call in that session breaks the run. Observed Codex seat tool calls
remain other calls for this boundary; they are not matched to executions or
deleted by a guessed join. Result/output messages are not extra calls.
Sessions never merge, even for one seat. Unknown seat linkage is not permission
to merge unrelated seats.

A run qualifies when any three consecutive failures have first-to-third span
at most 600 seconds, inclusive. Slide the triple through the run. The full run
is one episode with its full failure count, including failures before and
after the qualifying triple. Do not split overlapping triples into episodes.
A failure lacking a timestamp stays in its run and count, but cannot be the
first or third timestamp of a qualifying triple. An untimed middle member is
allowed when both endpoints are timed. Across an untimed anchor, a triple's
first-to-third span must be nonnegative as well as at most 600 seconds; reversed
endpoint times never qualify. Never guess a time.

Apply the existing UTC `[from, to)` loop filter before this ordering: timed
observations at `from` are included, those at `to` are excluded; untimed
observations in the scoped session are retained, including seat-call barriers.
Do not impute an untimed observation into a time bucket. Count it separately as
unknown timestamp even when it participates in an episode. Out-of-window timed
observations do not participate or form barriers. Full-run count means the full
run in this filtered observation set, not failures outside the report period.
No timed qualifying endpoints means no episode, not evidence of zero failures.
A Codex episode requires unique seat linkage; unknown or colliding seats remain
visible as unknown in the unscoped report and never combine through a null seat.

## Observable acceptance

1. Given an ingested mixed Claude, Pi and Codex source set, when the operator
   runs the specified `report decisions` command, then its sibling
   `payload.tool_loops` and `payload.outcome_coverage` show the entries above,
   including both Codex units even when there are no detected episodes.
   A Codex episode has unit `command_execution`; it is not aggregated or presented
   as a directly comparable Claude/Pi tool-call episode.
2. Given native command items with integer codes 0, 1, -1 and 101 and items with
   missing, null, boolean and text codes, when the sources are read, then only
   the three nonzero integers are failures; zero is success and the rest are
   unknown. Misleading status/output text never changes those outcomes.
3. Given three equal command inputs failing at 0, 300 and 600 seconds in one
   resolved seat/session, when the report is read, then one episode has count 3.
   At 0, 300 and 601 seconds, there is no qualifying episode. At 0, 400, 700,
   800 there is one episode, count 4; at 0, 400, 700, 1101 there is none.
4. Given a qualifying failure run extended by equal failures, when reported,
   then it remains one episode with the full count. Given a success, unknown
   outcome, unknown input, different input, different tool or intervening seat
   tool call, then the run breaks. Given equal failures in different sessions
   or seats, then they do not combine. Missing endpoint timestamps do not
   qualify a triple; other valid triples in the same run can still qualify.
5. Given inputs whose exact lexical slices differ only outside the whole input,
   when grouped, then they match. Given different array elements, internal
   spaces, order, boundaries, JSON escapes or lexical spacing, then they do not
   match. Two different values containing fake secret tokens that redact alike
   remain different groups. Missing input never creates a loop by itself.
6. Given fake secret and unique content canaries in command input, outputs,
   prompts and unexpected fields, when ingest, reports, diagnostics, traces and
   exports run, then no content canary or fake secret appears in analytics
   storage (including temporary/WAL artifacts) or output. No known command can
   reproduce its fingerprint without the local key. Sources retain their hashes.
   This proves privacy behavior; it does not add loop-to-source tracing.
7. Given a pinned source set on v0.1.1 and the candidate, when the same public
   journeys run with the same filters, then Claude/Pi tool-call meanings and
   counts, all token categories/totals and intake endpoints are unchanged.
   Codex commands never become structured writes or end an intake window.
   Reingesting unchanged sources and replaying complete inputs adds no duplicate
   executions or episodes. Split append processing matches complete input.

## Amendment 1 boundary examples

These are product expected answers for QA's independent literal-byte oracle,
not implementation tests or a candidate pass. F means an equal-input nonzero
integer CommandExecution failure in one uniquely linked Codex session; B means
an observed seat-call barrier. Lists are in source order; times are seconds
relative to a fixed UTC origin; `?` means missing time. Unless stated otherwise,
all timed observations pass the report filter.

| Source observations | Expected episode result |
| --- | --- |
| F@600, F@0, F@300 | Timed segment sorts to 0,300,600; one episode, count 3 |
| F@0, F@0, F@0 | Source-order ties; one episode, count 3 |
| F@0, F@?, F@2 | Anchor stays in the middle; one episode, count 3; timestamp unknown count 1 |
| F@?, F@0, F@2 | Missing first endpoint; no episode |
| F@0, F@2, F@? | Missing third endpoint; no episode |
| F@?, F@0, F@1, F@2 | Last timed triple qualifies; one episode, full count 4 |
| F@500, F@?, F@0 | Anchored reversed endpoints; no qualifying triple |
| F@0, F@1, B@?, F@2, F@3 | Untimed barrier stays between segments; no episode |
| F@600, F@0, B@?, F@300 | First segment sorts to 0,600; barrier is retained; no episode |
| F@0, F@300, F@600.001 | Above the inclusive 600-second bound; no episode |
| F@?, F@?, F@? | No timed endpoints; no episode, timestamp unknown count 3 |

Window example `[0,600)`: F@0,F@?,F@599 produces one episode, count 3;
F@0,F@?,F@600 produces none because the final endpoint is excluded. For
F@0,B@600,F@300,F@599 in that window, the out-of-window barrier is excluded and
the remaining timed failures produce one count-3 episode. Unchanged repeated
reports over the same snapshot produce the same order and answer.

Input examples use literal JSON slices: three `[]` failures produce no episode
and unknown-input count 3; three `[""]` failures at 0,1,2 produce one count-3
episode; `["x"]`, `[ "x" ]`, `["\u0078"]` are three distinct lexical inputs,
so one failure of each produces no episode. A missing/invalid input between
two equal failures breaks the run. A result/output message between equal
failures does not break it; an actual custom/function call does. Three equal
failures with unresolved or colliding linkage yield no trusted episode and
linkage unknown count 3 in the unscoped report.

| Committed snapshot / action | Codex command-execution coverage and result |
| --- | --- |
| No Codex sources | not read, 0/0; empty command episode set is absence of sources |
| Legacy watermarked Codex source, report only | not read, 0/1; no mutation or inferred zero failures |
| Legacy prefix has a qualifying triple; normal ingest completes backfill | read, 1/1; same count-3 episode as fresh ingest; token/intake values unchanged |
| One current generation read, one legacy generation unread | not read, 1/2; any observed episode does not hide the unread source |
| Both complete after normal ingest | read, 2/2; unchanged reingest/replay adds no duplicates |
| Supported empty source successfully processed | read, 1/1; zero episodes, zero observed unknown command fields |
| Supported source has successes only | read, 1/1; zero episodes, observed failures zero |
| Supported CommandExecution items have invalid exits | read, 1/1; unknown-outcome count reflects items; no inferred failures |
| Malformed/unsupported-only source | not read, 0/1; source gap visible |
| Generation replaced but not processed | not read, 0/1; no inherited completion |
| Backfill batch fails before commit | prior snapshot/coverage unchanged; retry converges to fresh ingest |

## Proof ownership and Council disposition

Read all four Council artifacts named by the reverse-seam transition:
`.agent-runs/issues/40/council/builder-advice.md`, `qa-advice.md`,
`reviewer-return.md`, and `.agent-runs/review/40-council/advice.md`.
Their findings are advisory, not QA or review acceptance. Accepted here: existing
report payload, additive shape, anchored missing-time policy, applied historical
coverage with backfill and atomic completion, explicit empty/unknown boundaries.
Storage, schema, fingerprint algorithm reuse, compatibility/assurance selection
and technical plan remain the architect's responsibilities. No new time/RSS
threshold is introduced as a product gate.

Before build, QA must pin an immutable, independently calculated literal-byte
fixture/oracle covering the seven journeys, these examples, baseline filters
and declared report additions. Builder must not derive or regenerate expected
answers through the candidate parser/metrics. Later candidate verification must
include fresh/split/partial/backfill/failed-batch/replay parity, synthetic privacy
canaries and controlled synthetic key separation, and pinned v0.1.1 Claude/Pi,
token and intake regression. Do not read an owner key. Missing evidence remains
unknown, never an inferred pass. Manager independently accepts this product
amendment before the authorized delivery handoff.

## Evidence and uncertainty

Read: Phase 0, R1 §4/AC6/AC10, clarifications K8/K11, the G2 staged scope,
the product record, prior builder sizing and Q4 lexical-input evidence.

Predecessor's Phase 1 receipt records reproduction on the private sealed corpus,
streamed one line at a time (not rerun during this amendment):
23 Codex files verified against manifest SHA-256
`40c65db7ec8dcac83cdc06332979b44cae0a001a438db042081c6926f6528ff3`.
Counts: 568 exec calls, 8 function calls, 957 CommandExecution items;
957 integer exits, 41 failures, 916 successes. All 957 have native command
arrays containing only strings and timestamps. None of these items has a
`call_id` field. This verifies this corpus, not every Codex version or a global
absence of possible joins. No raw commands, outputs or credentials were printed.
Corpus location: `.agent-runs/qa/cycle2-sealed-input`; prior sizing:
`.agent-runs/scratch/dendro-r1/builder-sizing-pi-codex-tools-d5ebb07.md`.

The prior sizing's zero-loop claim is not reproduced here. Its compact-JSON
comparison is not proof of exact lexical equality. Delivery must independently
prove outcomes, equality, ordering, episode counts, privacy and regression
behavior on a pinned candidate and source set; zero real loops alone is not a
pass. Candidate performance, schema impact and other Codex source versions are
unknown. Missing grouping evidence means unknown equality for those observations;
it does not by itself mean the recognized source prefix was not read.
No missing grouping key was found among these 957 observations.

## Constraints, anti-goals and next owner

Preserve local-first operation, source containment, safe Rust, bounded streaming
and watermarking, parameterized database access, secret sanitization and existing
verification gates. Read sources without controlling agents or modifying owner
state. Preserve R1 and G2 definitions outside this explicit unit extension.

Excluded: #33 loop-to-source trace, #12 handoff loops, #13 acceptance, #27
multi-root, token/intake changes, new dashboards, threshold customization,
outcome inference from text, guessed joins, push, PR, merge, tag or publication.

Return this specification and its Phase 1 transition to pm.manager for the
custody and availability decision. No direct delivery message. Only the
authorized `agentflow-sdlc handoff deliver` route may move it to delivery when
the squad is free. Phases 2/3 choose the technical plan; QA and independent
review prove the candidate. A Phase 1 pass supplies criteria, not a QA verdict.
