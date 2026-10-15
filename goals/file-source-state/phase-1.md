# Phase 1 — #32 truthful file-source metadata state

Status: pass, product acceptance specification; not a candidate QA/review verdict.
pm.analyst / pm-analyst@dendro-pm, Codex / codex-cli / local-cli /
current-session. 2026-10-15 UTC. Key: 32-phase1-metadata-file-source-state.
Goal https://github.com/smota/dendro/issues/32.
Phase 0 https://github.com/smota/dendro/issues/32#issuecomment-6183940065.

## Outcome and existing public journey

When I inspect a historical metric after pruning or replacement of session
files, I want its trace to distinguish retained facts from the latest observed
condition of each file source, so an old path is not mistaken for currently
available evidence and historical totals are not discarded.

Use existing commands with identical database/rig/UTC filters:

```sh
dendro report tokens --db dendro.duckdb --rig dendro-dev --from 2026-10-15T00:00:00Z --to 2026-10-16T00:00:00Z
dendro trace --metric-id tokens --db dendro.duckdb --rig dendro-dev --from 2026-10-15T00:00:00Z --to 2026-10-16T00:00:00Z
```

Other existing trace selectors receive the same file-state semantics in their
common inputs. No new command, selector, UI or required safety flag. Reports
remain computed from persisted analytics and do not gain filesystem reads or
new metric meanings. A trace performs bounded metadata-only observation of
eligible retained Claude/Codex/Pi paths; unsafe, unavailable or unknown references
remain explicit. It never ingests, backfills, repairs or opens source content.

## One state model, with three distinct questions

Add canonical `inputs.file_source_states`, an array keyed by actual stored
source_id and referenced generation. Emit entries for file sources in the
existing selected session/coverage scope and retained source-generation
references used by trace facts/evidence, including sources with no current
usage/loop episode. Unscoped traces include their existing unresolved sources;
rig-scoped traces must not assign excluded/unlinked sources to a rig or probe
out-of-scope paths to answer a global coverage count. Any already exported
out-of-scope coverage-only reference stays explicitly not_checked/out_of_scope.
No sources means an empty array, never a fabricated present source.

Each entry contains these required fields:

| Field | Observable meaning |
| --- | --- |
| `file_state_ref` | Unique safe response reference |
| `source_id`, `harness`, `generation` | Actual retained identities/generation, nullable unavailable identity marked explicitly; only Claude/Codex/Pi in this feature |
| `display_path` | Existing sanitized path or null; never a reconstructed secret/raw path |
| `latest_recorded_generation` | Actual latest committed generation for that source_id or null |
| `generation_state` | latest_recorded, superseded or unknown, from persisted generation evidence |
| `path_state` | present, gone, unavailable or not_checked, from the current path observation below |
| `metadata_comparison` | matches_recorded, differs_recorded, baseline_unavailable or not_compared |
| `state` | Summary present, changed, superseded, gone, unavailable or not_checked using the precedence below |
| `observation` | metadata_only method, checked_at UTC or null, safe reason, observed metadata or null, comparison basis/version and limitations |
| `baseline` | Recorded ingest-associated metadata/generation/observation time, or null with explicit unavailability; not a historical content revision |

A usable observation records regular-file type, device, inode, full file size
and nanosecond mtime. Exact metadata-v1 comparison uses these fields; architecture
freezes their lossless JSON representation before build. Optional other metadata
must not silently change this version's state predicate. None is a content hash
or assertion of source-clock accuracy. observed size is the full file size, not
committed byte_offset. An incomplete trailing line can make that offset smaller
than the file even immediately after successful ingest.

Canonical state is reused by references, not separately re-probed or recomputed:
add `file_state_ref` to applicable existing usage/source rows and #33 loop-source
entries. Existing `tool_loop_trace.sources[*].current_state` projects this entry's
summary for applicable file references. Remove/replace only corresponding stale
current_source_not_checked annotations when an actual check now occurred; retain
unrelated history/coverage limits. Existing source_ref/source_id/generation,
producing_revision and metric/rank/member/witness fields retain their meanings.
This is the declared file-state annotation change to #33, not a new loop rule.
OpenRig source entries retain their existing model and scope.

## Path state and observation basis

For eligible in-scope exact stored file references, observe metadata once per
physical pathname reference per trace and reuse the result across generations/
rows. Record checked_at and a metadata_only basis. Metadata is a point-in-time
observation separate from the read-only analytics snapshot; it is not a lock or
an atomic whole-filesystem snapshot. A changing path may become unavailable
immediately afterward. Detected uncertainty during observation is explicit.

| Path evidence | path_state and reason |
| --- | --- |
| Exact eligible local path resolves without unsafe symlink traversal to a regular file | present; observed metadata supplied |
| Genuine NotFound at the eligible path or a necessary absent ancestor, without an unresolved symlink/permission ambiguity | gone; path_not_found, observed metadata null |
| PermissionDenied or other metadata error, including ENOTDIR, loop or unresolved filesystem evidence | unavailable; metadata_error/identity_unavailable, not gone |
| Final or ancestor symlink requiring following outside the permitted exact-file boundary, including dangling/cyclic links | unavailable; symlink_not_followed, no target probe |
| Missing/unusable/redacted/relative-unresolvable path or insufficient authorization/containment evidence | not_checked; path_unresolvable/redacted_path/containment_unverified |
| Source not in current observation scope | not_checked; out_of_scope |

Do not recover a raw path from opaque source_id/key, a redaction marker, prefix
hash, neighboring file, environment default or key. A stored usable file path
is an exact-file metadata reference, not permission for recursive discovery or
for following a replacement symlink into another tree. Preserve containment/
symlink-cycle defenses; paths with uncertain meaning are not guessed. Do not
stat credential paths to reconstruct file references. Use safe reason codes,
not unsanitized OS errors or secret-bearing path echoes.

Gone means the retained pathname was not found at the observation time, not
proof that its bytes were destroyed everywhere or that another path is its
successor. No search for a moved copy or lineage inference is implied.

An inaccessible or detached path must not make a supported historical trace fail.
Detached ordinary sources get gone when absence is actually established; an
uncheckable reference gets not_checked/unavailable instead. These differ from
present and from an unchecked stale historical path. No unchecked response
may claim the file is currently available or unchanged.

## Metadata comparison, generations and supersession

Baseline metadata for future normal ingests is a recorded observation associated
with the actual accepted file source_id/generation and observation time, retained
with that generation where available. It uses existing authorized ingest, not a
trace-time reread or new content-hash operation. If the association cannot be
established honestly, retain baseline_unavailable. Do not claim it is an exact
historical producing-byte revision. Architecture owns persistence/schema and
compatibility choices; this contract prescribes no storage edit list.

If a regular path is present and a trustworthy comparable recorded baseline
exists, compare metadata-v1: all equal gives matches_recorded; a type/identity/
size/mtime difference gives differs_recorded. Without such a baseline, use
baseline_unavailable. Non-present/unobserved paths use not_compared. A baseline
for another generation is not silently substituted for this generation.

Current source_files records at entry have display_path, generation, committed
byte_offset/prefix revision and native session, not a device/inode/full-size/
mtime baseline. Thus an old supported DB may establish present/gone without
claiming metadata equivalence to ingest. Do not treat byte_offset as original
file size, stat time as a new generation, or the stored prefix digest as a new
content comparison. Missing baseline/history is a disclosed limit, not a failed
trace or a requirement to ingest before reading history.

For an actual retained generation G of source_id S:

- latest_recorded when the persisted latest committed generation of S equals G.
- superseded only when that persisted latest generation of the SAME actual S is
  greater than G. The trace must cite those two generation values as evidence.
- unknown when that relationship is unavailable/inconsistent; do not guess from
  maximum visible rows, timestamps, names or a new session/file elsewhere.

No cross-source lineage is inferred. A renamed/pruned file with a new file at a
different path, even with the same session name or equal metadata, does not
establish a replacement relationship. A new inode at the old path is a changed
pathname identity; it does not itself advance an ingest generation. A recorded
newer generation can establish supersession even if that pathname is now gone
or inaccessible. A new ingest retains its existing fact/generation semantics;
this feature does not retire, delete, relink or reseal historical facts.

Summary `state` precedence:

1. Proven generation_state superseded -> superseded, while path_state and its
   current observation remain separately visible.
2. Otherwise gone/unavailable/not_checked path_state -> that same state.
3. Otherwise present path plus differs_recorded -> changed.
4. Otherwise present -> present, with metadata_comparison explicitly distinguishing
   matches_recorded from baseline_unavailable. Present alone never means content
   unchanged or authenticated to an ingested generation.

Matching metadata cannot prove matching bytes. Same inode/type/size/mtime with
changed content can be invisible to metadata-v1. New inode with the same size/
mtime differs by identity and is changed when a baseline exists. Filesystem
races, inode reuse and timestamp granularity retain explicit limitations; no
universal identity/content guarantee or secretly added content verification.

## Historical metric and read-only guarantees

Changing/appending/truncating/replacing/removing source files WITHOUT ingest
must not alter persisted analytics, metric input eligibility, filters, detector
order/predicates, totals or historical attribution. Metadata observations are
annotations only, never filters or triggers to discard/recompute facts. Preserve
#33's captured-evaluation replay and its independent-query full-SQL-tie limit;
do not reintroduce a universal cross-invocation order guarantee. Exact before/
after metric comparisons use fixed deterministic fixtures; fully tied native
cases must preserve the existing permitted enumeration behavior, not add sorting.

Trace/report value, token categories/totals, loops/coverage, member/witness/order,
handoff/intake/activity/median values and historical source keys/generations stay
unchanged for the same analytics snapshot. Only the declared annotation/reference/
observation-time fields and corresponding source-state limitations may differ.
The metrics share their existing consistent analytics evaluation; filesystem
observations do not pretend to belong to the original ingestion instant.

Normal reingest is a separate authorized operation and can add/reset facts or
advance generations under existing ingestion rules. Before/after reingest is
not the same-snapshot invariant. Do not modify those rules to manufacture a
superseded example or reconstruct old metadata/content. Prior producing-ingest
revisions remain unavailable where unrecorded; latest metadata is not history.

State observation and trace remain read-only: no source-content or local-key
opens/reads, prefix rehashes, source locks, writable mappings, create/truncate/
unlink/rename, cleanup/recovery, backfill, migrations or analytics writes.
Metadata namespace operations and metadata-only directory handles are permitted;
opening the original regular file for content is not needed for this feature.
Preserve original main/WAL/SHM/source/key state and existing primary/auxiliary
alias safety. No network or real-owner mutation experiment. A missing local key
must not stop supported report-only traces or cause a replacement key to appear.

## Independent expected examples

Separate synthetic fixtures supply known metadata baseline B at accepted G1.
Metadata-v1 fields, safe paths, raw source bytes and expected outputs are literal
independent fixtures; checker state expectations cannot derive from candidate
classification. Source setup changes occur outside the traced public process.

| Fixture at trace time, no intervening ingest unless stated | Expected state / evidence |
| --- | --- |
| Regular source, metadata equals recorded B, latest G1 | present; path present, matches_recorded, latest_recorded G1 |
| Source append changes full size/mtime after B | changed; path present, differs_recorded; generation remains G1 |
| Source truncated after B | changed with differing size; no metric/fact removal |
| Same path replaced by a different inode, size and mtime preserved | changed by device/inode evidence; no inferred G2 |
| Same inode/type/size/mtime restored after a content edit | present, matches_recorded, explicit cannot_verify_content limitation; no content read to detect it |
| G1 source path deleted, genuine NotFound | gone/path_not_found; G1 facts and metrics retained |
| Parent/root detached, exact retained path genuinely absent | gone; no data loss; supported missing-key trace succeeds |
| Permission or metadata I/O failure | unavailable; not gone; historical trace still usable |
| Symlink to source or unrelated target, dangling link, parent link/cycle | unavailable/symlink_not_followed; no target-content read or out-of-bound probe |
| Redacted/unresolvable source path | not_checked with reason; no raw-path reconstruction/probe |
| Existing supported DB, no stat baseline, regular eligible path present | present, baseline_unavailable; never matches_recorded |
| Existing supported DB, no stat baseline, genuine path absence | gone; baseline absence does not prevent established absence |
| G2 actually committed for same source_id, trace retains G1 facts | G1 superseded, latest_recorded_generation 2; G2 latest_recorded; current path_state remains separate |
| G1 superseded by recorded G2 and current path now absent | summary superseded AND path_state gone; both evidence bases visible |
| Different new filename/session without a recorded same-source generation relationship | old source may be gone; new file does not make old source superseded |
| Old row has no usable path/latest metadata | not_checked/unknown; actual known row generation retained, unavailable fields null |
| No retained file sources in scope | empty file_source_states; no fabricated presence |

Baseline mismatch is tested against full recorded file size, not committed
prefix length: a stable source with an unconsumed trailing partial line must
not be mislabeled changed merely because its total size exceeds byte_offset.
After genuine reingest with an honestly updated baseline, metadata comparison
uses that recorded generation/observation, never retroactively updates G1's
historical baseline to make old facts look newly produced.

## Reproduced entry gap and evidence

Built entry HEAD a1db495b5d131112c6f179e465b8e53411e65480; debug binary
SHA-256 53dfa2d21c405a9d234ef960149352d94c3cf157b7100c1db63ba2b8233f5d17.
Analyst baseline-probe.py creates only new synthetic Claude, Codex, Pi and
OpenRig sources with resolved sessions. Initial report has 15 tokens and one
count-3 Codex command episode. Without any further ingest, the fixture appends
then truncates Claude, replaces Pi with a same-size/mtime new inode, deletes
Codex, and finally detaches all sources and removes only its synthetic key.

All five report/trace pairs remain exactly equal to initial output, analytics
bytes remain unchanged, and all exported source states remain not_checked.
This reproduces both useful retained-history behavior and the missing live
availability/change annotation. It does not prove candidate metadata observation,
new baseline capture, generation supersession, decoded privacy or permissions.
Sources change only as explicit synthetic fixture setup; no Dendro writes are
inferred from those setup changes. No owner paths/credentials/key content read.
Probe argv/construction/results: baseline-probe.py and baseline-probe/summary.json
with captured public stdout/stderr. Exact baseline metadata availability was
verified from current source schema and commit/read paths, not an old estimate.

## Proof ownership, alignment and return

Phase 3 freezes independent literal metadata/source-generation cases, safe path/
error controls, actual commands/cwd, source/analytics/baseline/candidate hashes,
lossless field representations and only the declared annotation deltas. Native
source-fact oracle and file-stat expected answers are independent of candidate
classification; final checker runs on actual public trace. Preserve originals
and existing #33 captured-input/rank/witness proof rather than regenerate goldens.

Independent QA must observe metadata-only operations and absence of source/key
content opens, writes, locks or mappings by the public process/descendants.
Use validated payload-free instrumentation, inode/alias-aware negative controls,
unfinished/FD/descendant handling and before/after state; unknown observer coverage
is not zero violations. Genuine unprivileged permission/error controls or scoped
injection must state limits; chmod with bypass privileges is not evidence.
All destructive setup is confined to new synthetic sources. Full applicable
decoded privacy covers persisted text/JSON/BLOB and DB/WAL/temp/export/trace/
report/diagnostic surfaces, with safe path canaries and no raw secret/path echo.
Record absent/unexercised surfaces; raw byte grep alone is not decoded proof.

Compare deterministic historical values exactly after only declared annotation/
observation-time normalization, before/after each source setup change with no
ingest, including detached sources/missing key and supported old DBs. Separately
verify normal reingest/append/reset semantics, first/missing/mismatch-key policy,
#30 retry, #34 precision, #40 units/coverage, #46/#47 source protection and #33
trace-only replay at the actual candidate. Cargo4 and independent QA5/review6
remain mandatory; analyst checks are not candidate acceptance.

Align terminology with #13 where compatible. No concrete mandatory #13 dependency
is established by this file-only contract: it annotates retained file references
and recorded generations, not OpenRig retention, sealed multi-generation replay
or cross-source lineage. If the shared design cannot remain within this boundary,
architecture must return the exact conflicting interface/requirement before
expansion; do not use the issue's alignment note as blanket #13 authorization.
Architecture owns design/assurance/storage/compatibility and race limits.

Unknowns: candidate baseline persistence/representation, safe path observation
implementation, full permission/platform/race behavior and performance. No
content-equality, universal inode/race identity, historical producing-revision
authentication or real-machine pruning-frequency guarantee. No source mutation,
new detector/threshold/UI/browser, #13 implementation or #12/#13 acceptance.
One local v0.2.0 minor outcome; no main edit, push, PR, merge, tag, integration
or publication. Earlier acceptances, including #33 b975e1c, stay candidate-bound
and are not this entry HEAD or a new file-state proof.

Return Phase 1 pass/digest, evidence and unknowns to pm.manager for independent
acceptance; only governed handoff deliver may move accepted criteria into
available delivery. No code, file-by-file implementation list or direct delivery
contact. Current autonomous policy applies without invented person gates.

References: current Phase 0/steering brief and issue32/F3; AC7/K8/K9 and source-
privacy contracts; current file-source schema/commit/read and loop trace state;
accepted #33 captured-evaluation/metadata limits and candidate-bound PM receipt.
