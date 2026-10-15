# Phase 1 — #30 actionable busy-source refusal

Status: pass, product acceptance contract; not candidate verification.
Signer pm.analyst / pm-analyst@dendro-pm, Codex / codex-cli / local-cli /
current-session, 2026-10-14. Key: 30-phase1-busy-source-wording.
Goal https://github.com/smota/dendro/issues/30.
Phase 0 https://github.com/smota/dendro/issues/30#issuecomment-6178975732.

## Outcome and existing public journey

When ingest refuses because a live OpenRig source keeps changing during all
snapshot attempts, I want the refusal to tell me I can retry shortly or during
a quieter period, so I have a next step without weakening the read-only guard.

The operator uses the existing one-shot CLI ingest, with their selected paths:

```sh
dendro run --db dendro.duckdb --root openrig=/absolute/source/openrig.sqlite
```

Default root selection and `--frozen` keep their existing behavior. No new
command, flag, prompt, report, retry schedule or agent operation is introduced.
When the existing bounded snapshot-exhaustion path refuses ingestion, the
English diagnostic becomes exactly:

> the OpenRig source changed during each of 5 snapshot attempts; nothing was ingested. Retry in a moment, or retry when the rig is quieter.

Keep the existing causal clause and no-ingestion disposition, add the sentence
boundary and this short manual next step. Ordinary CLI error framing such as
`Error: ` and the final newline is outside the diagnostic literal and remains
unchanged. The error remains on stderr, with exit 1 for this existing CLI path,
no successful ready JSON and no claim that a retry will succeed. The operator
chooses whether/when to invoke ingest again. Do not imply automatic indefinite
retry or tell the operator to stop agents, alter live sources, delete analytics,
change accounts/credentials/locks, or disable protection.

## Bounded behavior and truthful state

This is wording only. Keep the production maximum of five snapshot attempts,
existing retry/consistency policy, cancellation, copy order, source metadata/WAL
checks and private-copy SQLite integrity checks. Do not add attempts, waiting,
backoff, a rate threshold or a fallback that reads the live source directly.
A helper test using a deliberately different attempt limit may interpolate that
existing limit in its diagnosis; it must not change the production value five.

Nothing was ingested means no failed-run facts are committed/promoted into the
published analytics database. With an existing output, its analytics state is
unchanged by the refused ingest. With a fresh output, no ready analytics
database is published. Private failed snapshots/stages are cleaned according
to existing lifecycle behavior. It does not mean no filesystem activity:
existing output-directory, lock and first-run key-sidecar behavior is preserved.
Do not inspect, print or copy an owner key to prove this distinction.

Keep read-only access to original sources, containment and symlink checks,
source-root binding, sanitization and the transactional publish boundary.
Original source main/WAL/SHM bytes and metadata must remain unchanged by Dendro;
a synthetic producer may intentionally change its own fixture, whose writes
must be distinguished from Dendro's. No source file, raw record, command,
prompt, secret, key material or source path is added to this diagnostic.
No schema, parser, report, metric, category, attribution or ingestion-count
change is authorized.

Do not append this hint to separately emitted errors for a corrupt database,
permission denial, output lock, containment/root refusal or other failures.
Do not change discovery's existing missing-root warning/skip behavior into a
busy-source error. A stable valid source still succeeds normally, without the
busy refusal/hint, and a later manual retry is a new ordinary invocation.
Cancellation/interrupt and cleanup-error behavior remain unchanged.

## Existing integrity-path limit requiring explicit acceptance

Source inspection finds that a private copy returning non-OK from quick_check
can also exhaust the existing loop and reach the same terminal diagnostic.
Errors returned by quick_check can instead exit separately; the corrupt-file
control below takes that separate path. The existing terminal text is therefore
not independent proof that every failed attempt observed a changing source.

This wording contract preserves the existing terminal path and classification;
it does not add retry advice to other error paths or silently introduce new
corruption detection/routing. The manager must explicitly accept that retained
limit with this wording-only scope. Any proposal to distinguish additional
underlying causes needs separate product disposition before code, not an
unrecorded behavior fix inside #30. QA must not claim the separate corrupt-file
control proves all integrity failures are distinct from exhaustion.

## Observable acceptance and expected answers

1. Given a contained synthetic source whose observed state changes on every
   production snapshot attempt, when normal `dendro run` exhausts its five
   attempts, then exit is 1, the stderr diagnostic has the exact text above,
   stdout has no ready/success result, and it offers only a manual retry.
   Five is the existing production limit, not inferred solely from the text.
2. Given an existing analytics database and the same refusal, when the run ends,
   then published analytics state/bytes are unchanged and no partial private
   stage is left after successful cleanup. Given a fresh output, no database is
   published. Existing sidecar/lock behavior is not reclassified as ingestion.
3. Given a valid stable synthetic SQLite source, when ingest runs, then it
   succeeds with the existing ready JSON, no busy guidance, a coherent private
   source copy and the same facts/provenance as the baseline after declared
   nondeterministic-field normalization. An unchanged rerun remains idempotent.
4. Given the synthetic source stops changing, when the operator invokes a new
   run, then the same normal success contract applies. The hint itself performs
   no retry and guarantees no outcome if the source continues changing.
5. Given a known non-SQLite synthetic source, when the existing integrity-open
   error is emitted, then its nonzero error/exit behavior remains and the busy
   retry sentence is absent. Existing missing-root, output/root-containment and
   permission-error controls retain their own dispositions and diagnostics;
   do not mislabel them as busy. No privilege change is needed for a control.
6. Given fake secret and content canaries in synthetic source payloads and
   unexpected fields, when stable/refused ingest and diagnostic paths run,
   then they are absent from diagnostic/output and persistable analytics text
   under existing privacy rules. Preserve source contents and inspect applicable
   source main/WAL/SHM access using existing verification techniques; byte hashes
   alone do not establish absence of write-capable opens. No owner payloads or
   credentials enter artifacts. Existing sanitization/containment refusals stand.
7. Given a pinned baseline and candidate on the same synthetic inputs, when
   the journeys above run, then the intended difference is solely the specified
   diagnostic sentence/punctuation and associated docs/expectations. Snapshot
   attempt policy, result/exit channels, analytics publication, source access,
   stable-ingest facts and every unrelated diagnostic/metric remain unchanged.

Expected terminal body, for five attempts:
`the OpenRig source changed during each of 5 snapshot attempts; nothing was ingested. Retry in a moment, or retry when the rig is quieter.`

The original body ends at `nothing was ingested`, without the final period.
Keep that original oracle evidence; do not replace it with candidate-derived
output and call the result an independent expected answer.

## Reproduced entry evidence and limits

Read and built source HEAD 1e4909bee5eefe924cff6c1c1c87ce51f32bfd46.
`baseline-probe.py` creates a new synthetic SQLite source under this goal
folder, runs stable ingest, then changes only its mtime from a fixture producer
while running the public CLI against existing and fresh output locations.
Source size is 16,822,272 bytes; this is a synthetic race fixture, not a commit-
rate, latency, throughput or real-rig observation. Dendro never controls an
agent or owner source in this proof.

Both exhausted public runs: exit 1, stdout empty, stderr:
`Error: the OpenRig source changed during each of 5 snapshot attempts; nothing was ingested`.
The original message lacks retry guidance. Existing analytics SHA-256 is
unchanged across the refused run; the fresh database is absent; no private
stage remains. Synthetic source bytes are unchanged across the whole probe;
mtime changes are intentional producer writes and are not attributed to
Dendro. Stable controls before and after succeed with one reported attempt.
A non-SQLite synthetic control exits 1 with `file is not a database` and no busy
message; its source bytes remain unchanged. Synthetic privacy canaries were
placed in an unexpected payload field; the captured CLI outputs do not contain
them. No decoded-storage or syscall-observer privacy pass is claimed here.

Full commands, public stdout/stderr, source/binary hashes and aggregate state
checks: `baseline-probe.py`, `baseline-probe/summary.json` and sibling outputs.
This reproduces current behavior, not a #30 candidate. Source inspection verifies
production MAX_ATTEMPTS = 5, default route into that limit, private-copy checks
and cleanup/publish behavior; a helper's existing exhaustion test uses 3.

Historical PM/QA evidence records 15/15 successful trials at 5/10/25/26/50
commits per second on the pinned earlier candidate, with 1–3 attempts. Originals
and the accepted historical limit remain historical evidence. This synthetic
probe neither repeats that rate experiment nor establishes real-rig refusal
frequency. The G2 review question remains open. No throughput, retry-success or
real-rig frequency claim is accepted by this contract.

## Proof ownership, constraints and return

Phase 3 freezes independent expected diagnostics and fixture/source/baseline
hashes before build, preserving original oracle evidence and documenting only
the intended string delta. QA/reviewer own independent proof; the builder must
not regenerate expected text from candidate output. Reuse bounded synthetic
controls; no production-only fault hook, real-rig stress run, parser change or
extra rate benchmark is requested. Candidate execution and syscall/decoded-
storage proof remain unverified. Four required workspace Cargo gates remain;
QA5 and independent review6 retain candidate acceptance authority.

Release intent v0.2.0 minor, local only. One wording correction; no #12/#13
acceptance, metric, schema, performance or retry-policy expansion. Integration
and publication holds remain with their owners. No product code, file-by-file
implementation plan, main edit, push, PR opening, merge, tag or publication.

Return this pass, digest, evidence and unknowns to pm.manager for independent
acceptance, including the retained integrity-path limit. Only governed
`agentflow-sdlc handoff deliver` may send accepted criteria into delivery when
free. No direct delivery contact is authorized by this analyst contract.

References: Phase 0 and steering brief, issue #30, current CLI snapshot/refusal
and staged publication paths, existing snapshot tests and AC12 verification
script, PRODUCT_SPECIFICATION historical busy-rate disposition, R1 AC10/AC12
and clarification K9, G2 staged other-items and real-rig review agenda item 6.
