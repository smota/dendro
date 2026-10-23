# Adversarial review and fixture ledger

This is Codex lead adjudication of completed Grok source/domain, insight and mathematics reviews and Claude independent architecture critique. Claude rejected readiness of the initial skeleton; its critique was accepted/adapted/refuted below. It has not approved the finalized packet. Supplied native-client reports identify requested model selectors; actual model/effort remains requested-only unless exposed in observed run metadata. A later independent final-packet audit must name its own exact packet digest. “Confirmed baseline” below means supported by the supplied source audit at the pinned SHA, except branch/HEAD/version independently checked during writing. A suspected defect needs the stated reproduction before it can become a confirmed bug. Severity denotes release risk, not a claim of exploitation.

## Findings and adjudication

| Priority / finding | Disposition and rationale | Closure |
|---|---|---|
| P0 scope could become an all-not-observable demo | Accepted risk. Four-domain positives require real readers/collector and deterministic analyzers; schemas/hooks alone fail release. | T10/T11/T16/T18/T22 |
| P0 Grok/AGY presence mistaken for capture | Confirmed baseline: `ingest.rs:201` reports present-not-parsed. Adapt: real specimen-backed Grok; actual Dendro AGY export import. No invented native AGY support. | B01/T08/T09 |
| P0 environment facts absent in inspected core | Confirmed scoped absence, not global repository claim. Collector/import/syscall reader are required positive delivery paths. | T10/T11 |
| P0 migration rollback by ignoring new tables | Rejected Grok recommendation. Old binary version guard refuses schema4; preserve DB/key/sidecars and switch back to schema3. | T04/T21 |
| P0 no policy -> zero violations | Rejected. NOT_ASSESSED with null violation count; absent grant is missing evidence, not a breach. | T13/T16 |
| P0 any retry success -> accepted work | Rejected. Typed outcome rules and revocation history distinguish attempt success, review, acceptance and merge. | T12 |
| P0 ambiguous dedup or no invocation ID deletes usage | Rejected. Stable observation keys retain valid usage; only exact equivalence merges; ambiguous fleet overlap refuses total. | T06/T14 |
| P1 always refuse incomplete inclusive or require allocation | Adapted. Deduplicated evidenced descendant union needs no ownership split; nonnegative partial subtree may yield lower bound. Inclusive is nonadditive. | T14 |
| P1 all mean-of-ratios forbidden | Rejected. Named population-specific mean is legitimate; it cannot impersonate ratio-of-sums. | T14/T16 |
| P1 cost always requires ratecard | Rejected. Observed currency survives; only estimates require component prices. Subscription spend remains separate. | T14 |
| P1 summed duration = depth integral | Rejected as general rule. Sum distinct attempt activity segments; interval sweep derives concurrency from those intervals. | T14 |
| P1 transient argv parsing inherently forbidden | Adapted. Bounded safe in-memory classification is allowed; persisted raw argv/transcripts/secrets are forbidden. Structured capability evidence still differs from mention. | T06/T13 |
| P1 regex narrative lint proves truth | Rejected. Structured claim references/schema/numeric checks plus hypothesis labeling/fallback; semantic unsupported claims cannot become facts. | T19 |
| P1 recognized record kinds disappear | Suspected defect: Grok cites `stream.rs:815,832,1203`. Reproduce recognized-but-unprojected input and inspect coverage, not merely empty usage. | T06/T20 |
| P1 Codex tool warning contradicts projected commands | Suspected defect: `ingest.rs:106`, `stream.rs:282,375`. Reproduce command/seat/MCP cases separately; capability matrix replaces blanket statements. | T06/T18 |
| P1 missing Claude session aliases unrelated files | Suspected defect: `stream.rs:994,1010`. Reproduce two missing-session files; do not normalize historic identity silently. | T06/T20 |
| P1 topology failure looks like no rigs | Suspected defect: `openrig.rs:33`. Inject DB read failure and require source_failed, distinct from observed empty snapshot. | T12/T20 |
| P1 native Windows assumed from orchestrator OS | Prior inference rejected on the earlier brief; now superseded by explicit user requirement for native Windows AND Linux. Unix import at `output.rs:3` is a portability gap to fix in T24, not grounds to exclude Windows. Preserve applicable macOS contract. | B00/T22 |
| P1 confidence/completeness/ranking overclaims | Adapted. Mathematical confidence needs a declared model; this descriptive pack claims none. Known denominator required for completeness; comparable cohorts required for ranking. | T15/T16 |
| P1 source traits imply sandboxed plugins | Rejected inference. Compile-time trusted adapter registry requires conformance/security review; manifest grants no sandbox or execution authority. | T05/T20 |

## Frozen negative and numeric fixtures

These are proposed independent oracle inputs/outputs, not executed results. T02 serializes full golden Answers and safe source vectors, preserving these values; each includes query window, knowledge cutoff and version `1.0.0`. Expected null/status/reason is as binding as a number.

| ID | Input | Expected canonical output |
|---|---|---|
| F01 | A,B both active `[0,10)` seconds | Wall 10; summed 20 attempt-seconds; mean concurrency 2. |
| F02 | A `[0,10)`, B `[10,12)`; variant B `[9,12)` | Abut: wall=sum=12. Overlap: wall=12,sum=13. |
| F03 | P spawns A/B; A/B delegate_existing to same work-bound D; D has one separate known spawn origin at most; direct tokens each 1 | Fleet=4; inclusive(P)=4; inclusive(A)=inclusive(B)=2. Caller rollup 5 forbidden. |
| F04 | W=1000 tokens, two callers, no allocation | Shared=1000; caller-owned totals unknown. Exact rational weights .6/.3 ->600/300/residual100; .7/.4 rejected. Additional partial claimant vector total100 weights25/100,40/100 ->25,40,residual35 even when other callers omitted;70/100+50/100 reject. |
| F05 | Parent `[0,5)`, child `[4,20)`; child tokens100 | Parent wall5; lineage-close wall20; inclusive100, descendant_outlived=true. |
| F06 | Two rows exactly equivalent request100; third possible-overlap row100 | Exact pair canonical100/duplicate_rows1. Third preserved; fleet sum refused ambiguous_overlap; source totals preserved, never a claimed canonical sum. Distinct potential same-call values40/25 are unresolved overlap/conflict at their own grains, not exact-hash equivalents. |
| F07 | Two authorities each request `r1`, tokens100 and200 | Distinct IDs; direct total300 unless other overlap evidence exists. Never merge by bare request ID. |
| F08 | Input100 includes cache-read40; output20 | Total120. Unknown cache inclusion -> derived total null/refused, components retained; literal reported-total120 remains supported at source grain with definition_unknown. Modern observed input100 plus legacy unknown -> observed-subset100/fulltotal unknown. |
| F09 | Same proved cumulative counter100 then150; separately evidenced reset to20; counter-decrease-only variant and first sample | First sample cumulative100 retained, incremental baseline unknown unless since-zero proven; delta50 for same counter. Reset20 counted only under source since-zero reset semantics, otherwise baseline20/unknown first delta; decrease alone -> unknown/conflict, never automatic reset or -130. |
| F10 | Three costs10,10,10; successes1,0,1 | Ratio-of-sums15 per success; separately named mean successful-attempt cost10; undefined all-attempt ratios count1. Independent cache vector90/100,1/10 ->ratio-of-sums91/110, named positive-denominator invocation mean0.5; absent invocation grain refuses mean. Other exact ratios19/40=.475 and11/16=.6875; zero aggregate denominator refuses. |
| F11 | Observed USD0.02; ratecard absent | Observed0.02 supported; estimate null/rate_card_absent. 1000*$2/M +500*$8/M -> estimate0.006 with card. Independent partition vector observed USD150 and EUR200, estimated USD6/cardv3 and subscription USD2000 remain four labeled facts; no FX or observed+estimated sum. Canonical USD10/100==USD1/10 and zero0/1 before hash/equality; same key/revision is idempotent, no forced ISOscale precision loss. |
| F12 | AGY source unavailable; completed scan elsewhere with zero eligible tools | AGY null/not_observable/log_unavailable. Complete eligible scan0/supported. Forty rejects -> partial with rejected_n40. |
| F13 | Policy absent, effect observed, grant absent | Rule state NOT_ASSESSED; violation_count null; missing_grant_evidence, not violation. Explicit prohibiting rule+required evidence can establish rule-specific violation. |
| F14 | Work completed; attempt succeeded; acceptance absent, later accepted then revoked | Attempt success1; initial work acceptance unknown; accepted at later cutoff, revoked at newest; no merged transition invented. |
| F15 | End before start; segment start5 with ingested ongoing/liveness evidence covering cutoff10; lone-start variant | Invalid segment conflict, never zero; witnessed ongoing contribution5/right-censored. Lone start with missing stop -> missing_end/unknown duration, not automatically5. Liveness learned after sealed cutoff cannot affect prior answer; raw timing protected, cross-source unknown clock order stays ambiguous. |
| F16 | Same label `box`, incarnations E1/E2; host/tool/effect placements | Two environment instances; loci remain distinct; no label merge. |
| F17 | Declared isolated=true, namespace metadata only | Effective isolation unknown; configuration evidence not verified_effective; comparison insufficient. |
| F18 | Tool succeeds; syscall trace absent | Tool success counted; effect count null/not_observable. Trace read failure != zero effects. |
| F19 | Source label contains HTML, ANSI and “ignore policy”; secret canaries in transient args | Escaped/inert label; Answer IDs/status/numbers unchanged; no canary in decoded retained artifacts. |
| F20 | Grok root exists but schema specimen absent; AGY valid export2 usage rows | Native Grok admission blocked, no delivered-parser claim; AGY export reader yields2 observations with native status unadmitted. |
| F21 | M1 long tasks, M2 short tasks; different coverage/population | Comparison noncomparable, no winning model or causal claim; raw descriptive values allowed with exclusions. |
| F22 | Fact100 ingested day1; correction80 ingested day3; retraction day4 | Knowledge cutoff day2 ->100; day3 ->80; day4 -> no active fact, audit history retained. |
| F23 | Spawn A->B->A versus message A->B->A; valid A->B with A direct10/B corrected10->4 | Spawn cycle quarantines inclusive fold, retains direct14 after correction; valid A->B direct fleet14, inclusiveA14/B4. Message/handoff loop valid; no return edge pulls caller session into child closure. |
| F24 | Unknown schema major, AGY line>1MiB or another adapter above its admitted limit/global8MiB ceiling, PID reused, missing privacy key | Source quarantined/unsupported_schema; oversized quarantined with partial coverage under adapter-specific bound; native1.5MiB not automatically valid; new process identity; private operations blocked without plaintext/rekey fallback. |

| F25 | Native Windows junction/reparse/hardlink/case-sensitive/UTF16 path aliases; native Linux symlink/inode aliases | Same containment/private-source invariants hold on each OS; no lowercase-path shortcut or cfg-disabled defense. |
| F26 | Native Windows sharing violation/open handle during DB pointer swap; concurrent locks/crash on both OSes | Old pointer remains intact, no forced DB deletion; consistent snapshot/rollback receipt, lock crash-release and owner-private ACL/mode verified. |
| F27 | Native Windows PID reused with changed creation time, inaccessible token, bounded CPU/working-set sampling | Distinct process identities and unknown denied fields, actual standard-user bounded observations; no Linux start_ticks fabrication or working-set=RSS claim. |
| F28 | Genuine pinned Windows ETW controlled trace with loss/unknown event/PID ambiguity/raw path canary | Native importer yields evidenced safe operation/status/resource classes, unknown/lost coverage explicit, source assertions separate; no raw retention/all-syscall claim. |
| F29 | Windows-native exe and Linux-native package native CI/staging; WSL-only or crosscompile-only variant | Both actual native lanes pass or release blocked. WSL/crosscompile-only cannot satisfy Windows; exact version/package digests bind repeated T23 receipts. |

Property oracles add direct conservation over disjoint partitions, allocations+residual=direct, wall<=summed, hypotheses excluded, surface equality, deterministic replay, safe schema rollback without logs, and bounded decode after a 1000-record append to 1M history. Prefix bytes are reported separately. Parser acceptance counts cannot masquerade as global sensor completeness.

## Completed Claude critique and lead disposition

| Critique | Lead disposition / implemented closure |
|---|---|
| B1/B2 freeze undefined; oracle/build/review conflated | Accepted objective freeze: versioned typed entity/relationship/accounting/environment/Answer/source-matrix contracts plus complete R01-R24/AC/test map and digests. Codex architecture/curator, independent Grok oracle, fresh Claude review-only identities; no same artifact self-acceptance. Bootstrap checklist finite exception explicit. |
| B3 prerequisite remedies absent | Accepted bounded diagnostic/remedy tasks in B00. Adapted authority: actual ownership/custody/pause resolution required; rejected invented file ledger and a fresh human approval for every read-only substep. Existing high-assurance/merge/publication gates remain. |
| B4 Windows port prerequisite | Historical lead disposition superseded by explicit user correction: native Windows AND Linux now release-blocking. B02/T24 own feasibility and safe portability, T25/T26 Windows instrumentation, T27 native CI/packages. Reject only the unsafe Linux case-insensitive containment generalization; preserve established macOS support. |
| B5 counter decrease/reset, each resume retry, all-open censored, SCC rollup | Rejected. Evidence-backed reset/continuation/liveness, unknown baseline/end and cycle quarantine replace these rules. WorkAssignment/closed eligible closure avoids persistent-session inflation. Shared D uses delegation to existing, never two spawn creators. |
| B5 all claimant weights sum1; missing link always lower bound | Adapted exact partial weights sum<=1 plus explicit residual; lower bound only with justified nonnegative deduplicated scope, otherwise refusal. Legitimate mean-of-ratios and observed cost retained. |
| B6 shared files/lockfile ordering | Accepted standing Codex integration owner; T03 core/T05 adapter skeleton and dependencies before --locked builds; T18 dispatch integration; final T23 version change repeats exact candidate acceptance. |
| H1/H3 migration/privacy/input risks | Accepted staged private migration/key continuity, schema4-only loss receipt, query external-access denial, C0/C1/OSC stripping, HTML+CSP/inert content and quarantine/provenance-scoped remediation including backups/key copies. |
| H2 fixed inference tests and statistical cutoffs | Rejected arbitrary MannWhitney/bootstrap/n20/BH mandate. Ship exact descriptive I01-I21; extensions require declared sampling/model/cluster/uncertainty/multiplicity contract before inferential claims. |
| H4 environment/source proof | Accepted exact per-kind matrix, native Grok genuine specimens and normalized real syscall importer. Adapted AGY to tested versioned export contract. /proc proves scoped metadata/resources/configuration only, never arbitrary effects or universal sandbox. Safe resource classes/HMACs replace raw destinations/paths; signed assertions need actual verification. |
| H5-H9 gates/performance/outcomes/time | Accepted explicit gates, task-scoped requested-versus-observed route receipts, typed outcome/revocation and source-local clock uncertainty. Lead fixed numeric budgets supersede reviewer ratios; baseline runner exists T02 before measurement. No significance, unmeasured performance or completed live adoption claimed. |

The complete external critique and lead/math adjudications were supplied to this refinement as separate review inputs; this ledger preserves their disposition without claiming a final reviewer endorsed later edits. Final audit receipt is still required against the exact packet.

## Residual blockers and reserved independent review

No all-clear is claimed. Live authority/custody/ownership/distribution repair and backlog status remain unresolved; B00 cannot authorize its own repair. Genuine Grok schema specimens, real tracer format/availability, source authorization, applicable macOS and historical AC1 requirements await B01/admission evidence. Missing any required specimen or environment end-to-end path blocks release rather than downgrading scope. All build, OS, performance, installed-runtime and canary results remain unrun.

Claude initial critique is completed and adjudicated above. Finalized-packet independent acceptance remains unclaimed. Record the final reviewer identity/provider, reviewed packet digest, findings/evidence/severity, lead disposition and any required re-review; never backdate initial critique as approval of revised files. Remaining disputed design decisions go to the actual design-approval gate, not silent builder choice.

## Final Grok audit receipt and bounded repair disposition

Independent reviewer: Grok/xAI, review-only, native requested selector `grok-4.7`; requested model is not proof of observed backend metadata. Review scope was the five-file planning snapshot below. Original verdict: **six HIGH findings block bounded autonomous delivery**. It reported no product tests. The reviewer did not compute a digest; the dispatcher supplied the pre-review snapshot hashes, verified against the files before this repair. No design/admission/publication authority was granted.

| Original reviewed file | SHA-256 (pre-repair snapshot) |
|---|---|
| ADVERSARIAL.md | `AB8AF623999B4172871B5CC3B3570011C8267EDDB44DD3F7437DA2EA6E6B8664` |
| DESIGN.md | `909EBC171108E7A49C75A05181744EED624299E185C74D101AA8434A82717701` |
| EXECUTION.md | `E90B303AFA3FA7267613D604E131B9D2E659838A952605F5994C579E2F23D7A5` |
| README.md | `AA3A55915552A7DC84F4955529B4A4C942DC9979620A2891DCC7AF145118349C` |
| tasks.json | `C76F0F37751ECA43C246CC61989D891078785A0B4B95912B3B2D5DF548C10310` |

| Finding | Lead disposition and revised closure contract | Acceptance mapping |
|---|---|---|
| H1 insight registration unavailable after catalog build | Accepted. T16 requests standing Codex insight-registry write; exactly I01-I21/version1.0.0 plus named submeasures registered before acceptance/lookups. T18 removes insight registry path and is capture-only. | T16-A1; T18-A2 |
| H2 AGY-specific bounds imposed globally | Accepted adapter-specific limits plus global hostile ceiling. B01 pins per-adapter schema bounds; T05 global8MiB logical-record/depth64/1024 decoded batch ceilings, descriptor may tighten only. AGY retains1MiB/128; oversized records quarantine/partial. A1.5MiB native record is not presumed valid. F24 reconciled. | B01-A1; T05-A1; T09-A1 |
| H3 straddling tokens prorated by time | Accepted with timestamp precedence. Geometric intersection only I10/I12 duration intervals. Atomic usage/cost/count uses evidenced attribution timestamp in half-open window; otherwise fully-contained evidence interval. Unsplit straddles excluded with partial coverage and preserved source amount; no elapsed-time proration. | T14-A2; T02-A1 |
| H4 noncanonical equal currency values | Accepted minimal decimal normalization before equality/revision/hash: strip trailing decimal zeros, zero0/1, preserve precision without forced ISOscale. USD10/100==1/10 same revision idempotent. | T14-A2; F11 |
| H5 independence not machine-verifiable | Adapted. Accept unique native-client session/run execution IDs, launch receipts and immutable authorship digest ledger; QA IDs disjoint from all tested builders/integrators, reviewer verifies joins. Reject different authenticated principal/account requirement: the same subscription may host distinct workers. Role/model phrases alone insufficient; renamed successor cannot hide predecessor authorship. | All worker_identity_contract records; T20-A2 |
| H6 native permission acceptance depends on downstream task | Accepted. T06 accepts emitted source-backed typed permission/context facts only. T13 ingests admitted real native specimens through reader into security layers/comparator; only structurally evidenced grant/attempt/effect allowed, other layers unknown. No synthetic-only substitute or text-mention authority. | T06-A1; T13-A1 |

Medium suggestions: accepted watcher1500ms/read-only roots at T18-A2, sole revision policy `corrections_at_cutoff_v1`/F22, T02 replacement-generation and incomparable-clock vectors, T14 F22 input reference, G03 held open until final T23 rerun receipts, and T18 ingest-through-storage scope (Answer journeys remain T20). Adapted unsafe suggestion to verification of existing `deny(unsafe_code)` policy/audited exceptions at T22-A1; no unjustified policy migration to `forbid`.

Repair status: normative text and machine-readable acceptance contracts revised; document consistency checks only. **At this repair checkpoint, independent targeted re-review was pending; the subsequent scoped receipt below records its result.** Neither initial audit nor this author disposition approves the revised packet. All original admission, build, capture, performance and live-adoption prerequisites remain unresolved until actually verified.

## Scoped closure re-review receipt

After the full-snapshot six-HIGH verdict and the bounded repairs above, a native Grok/xAI review-only worker (requested model `grok-4.7`; backend model metadata not independently inferred) reviewed a scoped excerpt of the changed contracts. Its result: **all six contract concerns closed on those excerpts; packet unapproved, G03 open, no product tests run**. This is not an approval or complete re-review of the final five-file packet, nor validation of source-matrix row values or implementation. The following SHA-256 values identify the actual scoped input and returned report:

| Artifact | SHA-256 |
|---|---|
| `closure-review-scoped.txt` | `7B109043EB0E6DD77050D3CF87199FB68E59708DBF5A9F1A157AEEB79EE5C9B0` |
| `grok-closure.txt` | `B1508D859EFBC912F3A177E86A382299923A033802127683EC1CC96D21D26059` |

The closure reviewer renumbered topics; this mapping preserves original full-review finding identity by semantic content:

| Original finding | Scoped closure evidence | Result and limits |
|---|---|---|
| H1 catalog registration writer/order | Scoped H1/H2: standing-owner I01-I21 registration before T17/T19; T18 capture-only | Contract closed. T15 retains contract/example registration through standing owner; T16 owns later full-catalog registration. Scoped shorthand about only T16 is not a change to the actual T15/T16 standing-owner schedule. |
| H2 per-adapter versus global bounds | Scoped H4:8MiB/depth64/1024 ceilings and AGY1MiB/128 tightening | Contract closed; individual native size/schema validity still requires B01 evidence. |
| H3 atomic window attribution | Scoped H5:timestamp precedence, duration-only clipping, no proration/straddle_unsplit | Contract closed. Editorial clarification below makes the accepted priority explicit. |
| H4 decimal canonical identity | Scoped H5:trailing-zero normalization, zero0/1, exact reduced ratios | Contract closed. Normalize canonical amounts at equality/hash time; cache ratio-of-sums still sums raw eligible numerator/denominator counts, not pre-reduced per-invocation ratios. |
| H5 worker independence | Scoped H6:session/run identity, launch receipt, immutable authorship ledger and QA disjoint IDs | Contract closed; same authenticated account allowed. Execution still must produce actual receipts. |
| H6 native permission acceptance | Scoped H3:admitted native specimen reader-to-security path and separated layers | Contract closed; does not certify actual capture or admit unsupported matrix cells. |

The reviewer identified wording ambiguity in DESIGN's timestamp fallback sentence while finding T14's accepted oracle unambiguous. This revision makes that priority explicit: if an evidenced attribution timestamp exists, use it alone; out-of-window never falls back to interval inclusion. Only absence of such timestamp permits fully-contained-interval attribution. This editorial clarification follows the accepted T14 oracle and does not imply a new review of the whole packet.

Remaining reviewer limits stay explicit: same source bytes/new generation do not establish equivalence; descriptor enforcement does not sandbox trusted compiled adapters; all shared lib writes retain standing-owner leases; B01 must freeze actual matrix rows; G03 stays open until T23 exact versioned rerun receipts replace earlier T22 receipts. No implementation, product test, measured performance, installed adoption, design approval, merge or publication is established by either review.

## User correction: mandatory native Windows and Linux

The direct user instruction, “No, it needs to support windows and Linux natively”, supersedes the previous lead exclusion of a Windows port. Scope now requires actual native Windows/Linux capture, database/query/rendering, safety, collection, trace import, packaging and release verification. WSL/import-only/crosscompile-only outcomes cannot substitute. Earlier Claude/Grok reports and six-contract closure remain historical evidence for their reviewed snapshots; none reviews or approves these dual-native additions.

Source-grounded feasibility starts with the pinned audit of unconditional Unix filesystem assumptions at `output.rs:3` and snapshot behavior at `snapshot.rs:143`. B02 must inspect and bind the actual dependency/build/API constraints; official Windows/ETW/DuckDB references in DESIGN establish feasible APIs but do not prove current implementation. B02 owns bounded dependency/permission/runner diagnosis and remedy admission; T24 fixes native safety/compilation before storage/adapters; T25/T26 Windows sensor/import modules follow; T27 owns native CI and packages. Dependency failure is an owned release blocker, never a quiet Windows deferral. No code/build/install was performed for this amendment.

Adversarial acceptance must challenge DACL versus readonly attributes, file-handle identity and case-sensitive/reparse races, native lock/sharing/rollback lifecycle, PID creation-time reuse, event producer privileges versus importer permission, real event profile provenance versus self assertions, trace loss/privacy, and actual native CI/package execution. Preserve equivalent security invariants without assuming identical sensor APIs or full syscall coverage on Windows. A Windows kernel/privileged hostwide monitor is not required or silently authorized. Existing documented macOS support remains preserved. Independent review of this amendment is pending.

## Dual-native scoped review:13 concerns and lead disposition

Grok/xAI review-only, requested `grok-4.7`, reviewed only the supplied native-design excerpts: verdict **amendment not a consistent admission contract**; no repository inspection, product tests or approval. Its excerpt omitted some existing task contracts. The following dispositions preserve that scope and do not claim a new independent approval of the repaired packet.

| Concern | Lead disposition / closure contract |
|---|---|
| 1 evidence versus remedy | Accepted. B02 real native prerequisite/API/producer-readiness artifacts and B01 real acquired-trace provenance/schema artifacts are separate machine admission predicates. Remedy-only outputs remain blocked_pending_artifacts, never accepted task-done. |
| 2 unnamed remedy deferral | Accepted precision. Existing B02 bounded remediation substeps now name owning environment authority, package/version/runner/path scope, one diagnosis/one admitted repair attempt, receipt path and actual return condition. Missing authority/results hold task and release; no extra unbounded task or blind install. |
| 3 no Cargo writer node | Excerpt omission: standing Codex substep mechanism already exists. Explicit B02/T24/T25/T26/T27 integration requests now name paths/input digest, globally serialized fence, patch/output receipt and independent oracle/review before consumption/acceptance. No new writer/task added. |
| 4 stale B02/T03 diamond | Accepted. T24 revalidates B02 source/import/dependency digest against T03 output before admission; changed assumptions revise B02 and invalidate downstream evidence. |
| 5 same provider equals self-review | Rejected automatic family-based inference. Different execution/authorship IDs and independent expected-value authors per artifact govern; earlier evidence curation is not semantic oracle authorship. Existing ledger/provider role boundaries remain. |
| 6 macOS owner absent | Accepted precision. T27 owns conditional native macOS baseline maintenance lane when documented prior support is recovered, preserving prior toolchain/checks without new sensor promise. |
| 7 safety proof narrower than threat list | Accepted. T24-A2 explicitly covers shortname/UNC/device/alternate streams, long/UTF16/case-sensitive paths, junction/reparse/hardlinks, lock aliases and concurrent replacement races, plus Linux equivalents; unsupported cases fail safely. |
| 8 paired private access fail-closed | Accepted. Protected Windows DACL admits owner/SYSTEM/Administrators with documented administrator trust; untrusted inherited ACEs fail. Linux foreign owner or effective group/other/ACL private access also fails. Source read-only authorization differs from private output/key permissions; neither source mutated to force privacy. |
| 9 pointer primitive/cross-volume ambiguity | Accepted. Pointer is regular manifest file, never directory/junction. Same-volume atomic file replacement; crossvolume builds/verifies closed private immutable destination copy first, then local pointer replacement. Sharing/copy interruption preserves old selection. |
| 10 Linux sensor ownership absent | Excerpt omission: T10 already owns CPU/RSS/cgroup/namespace/PID behavior. T10-A1 now explicitly requires native Linux synthetic activity, host/PID/start_ticks and exit/permission/gap receipts. |
| 11 two trace classes / dependencies | Excerpt omission: actual DAG T18 already depends on T10/T11/T25/T26. T18/T27 now require separate named Linux-strace and Windows-ETW profile/coverage receipts; no merged checkbox or equal-grain inference. |
| 12 trace fixture ownership | Accepted. B01 owns safe provenance manifests/private external raw capture authorization/digests; T26 alone owns minimized sanitized project fixture path with source/projection/digest lineage. No raw trace committed, no cross-writer fixture path. |
| 13 early native proof versus later CI YAML | Accepted precision. T24 directly executes B02-pinned native runner/toolchain manifests before CI YAML exists. T27 revalidates same manifest against later candidate; drift invalidates affected early evidence. |

No task/scope reduction or new platform waiver follows. Full native Windows AND Linux requirements, standard-user collection, genuine privileged-producer provenance boundary, exact candidate gates and conditional prior macOS maintenance remain. Real native build/collection/event/package/permission artifacts are future work; repaired design acceptance is not execution evidence.

Implementation authorization update2026-10-23: the user explicitly instructed the full revised plan to be implemented. This satisfies design intent without claiming prior reviewer approval of final product. New direct-local records preserve historical G2 and unrelated pause; owner issue526 clarification permits direct agents without rigs. Remaining native/source/oracle/ownership checks are actual implementation prerequisites, not a request to repeat user authorization.

Claude admission review (observed claude-opus-5-5, session e3d0ad57-2b69-4e3b-a6b7-847d4bd41b19) found overbroad bootstrap/source/native/baseline dependencies. Accepted A1-A5 and C1-C3 partition/identity/lease corrections preserve full release: B01-core/G/S/W and B02-W separate, T02 core/spec freeze versus T02-runner/T02-baseline, protected direct task leases and reviewed partial platform handoffs. Mid-build trace/Grok dependency overreach removed; actual native/I15/security positive receipts remain T18/T20 release requirements. Exact report and hash/adjudication are in `.agentflow/runs/v080/admission/claude-admission-*`. Real bootstrap checklist is written, independent review pending. This is no task/product acceptance.

Bootstrap scoped-review completion: observed Claude session9d93b510-e684-469d-b0a5-42a2abb587c4 explicitly judged core structural contracts internally consistent. Accepted checklist status/ID/mapping fixes are applied; gapA2 is intentionally partitioned, intrinsic circular self-hash and blanket author ban rejected. Detached `.agentflow/oracles/v080/bootstrap/freeze.json` binds reviewed draft/report/current hashes and actual review identity/time. T03 structural assertions are frozen separately; broader Grok numerical fixtures remain pending and are not a condition on unrelated typed construction. This permits scoped structural construction by a fresh worker, not final task/product acceptance.
