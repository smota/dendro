# Phase 0 — #32 file-source state in trace

Decision: proceed to analysis. pm.manager / pm-manager@dendro-pm, Codex /
codex-cli / local-cli / current-session, 2026-10-15 UTC.
Key: 32-phase0-file-source-state
Goal: https://github.com/smota/dendro/issues/32
Requester: steer.lead@kernel
Intake: qitem-20261015034642-e4911d6304c668cd
Brief: /home/sam/code/agentflow-sdlc/.agent-runs/briefs/issue-32-brief.md

## Job and evidence

When I inspect a historical metric after session-file pruning or replacement,
I want the trace to distinguish retained facts from the latest known condition
of their file source, so I can understand whether that source remains available
without mistaking its old path for current evidence or losing historical totals.

Read live issue32 (revision cb2e53ef2d601459ad50e1fcdeb3141123822c18918b684a319078251031f4a8),
steering brief, AC7/K9, current source-state storage and trace documentation.
Entry repository HEAD a1db495b5d131112c6f179e465b8e53411e65480 is observed;
not presumed identical to any separately accepted candidate. Current documented
trace source state is not_checked; historical facts and latest committed prefix
metadata are distinct. #33 local acceptance is bound to b975e1c and does not
claim filesystem presence or historical producing-revision authenticity.
Issue F3 reports the gap and pruning motivation; frequency, commercial impact,
historical sizing and steering confidence are not independent runtime proof.

Push: retained paths conceal deletion/replacement. Pull: explicit source state
alongside preserved history. Anxiety: a presence check could leak content,
mutate a source or imply stronger identity than metadata proves. Inertia:
existing facts remain useful without checking current source availability.

## Outcome and boundaries

Existing public trace journeys for Claude, Codex and Pi file sources expose
latest known present/changed/superseded/gone-or-unavailable state with generation,
observation basis and temporal scope. Distinguish current pathname availability,
persisted generation history and historical source references. Preserve facts,
all metric totals, filters, loop meanings and independent captured replay.
Changing/removing a source without analytics ingest must not change historical
metrics. Reingest can add facts under existing rules; do not conflate that with
invariant totals on the same analytics snapshot.

Analyst pins exact public fields, report compatibility, and whether/check-when
filesystem metadata is observed. Define what evidence establishes supersession;
do not infer lineage from neighboring files, matching names or a new session
alone. Separate absent paths from permission/metadata errors and unchecked or
unresolvable/redacted paths. Metadata equality cannot prove content equality;
same-size/mtime replacements, races and historical unavailable identity must
retain truthful limitations. Do not synthesize a generation from a stat call.

Privacy is path metadata only: no source content, hashes requiring content reads,
raw buffers, raw logs, keys or credentials. Metadata observation must preserve
source/analytics state and containment/symlink defenses; no creating opens,
source locks, cleanup, backfill or hidden ingest. Preserve a supported existing
analytics DB and detached-source/missing-key trace behavior, with explicit
unknown/unavailable states rather than failure or fabricated presence where
that is the accepted path. Architecture owns implementation and assurance.

Issue notes request alignment with #13. Reuse compatible terminology where
supported, but this file-source outcome does not authorize OpenRig retention,
multi-generation sealed replay, #13 implementation or #13 acceptance. Analyst
must return any concrete shared-model dependency before scope expansion.
Prior #30/#34/#40/#46/#47 and #33 acceptances stay candidate-bound.

## Proof and next owner

Freeze independent synthetic expected cases before build: unchanged, append,
truncate, delete, replacement/retained generations, lineage without evidence,
metadata errors, symlink/containment and missing-key/detached-source cases.
Compare exact historical report/trace values before/after source changes and
independently verify metadata-only operations and zero protected mutations.
Include old DB availability and stale/unknown classifications. Required Cargo
gates and independent QA5/review6 remain; PM framing is no runtime verification.

Next pm.analyst: define testable acceptance, public journey, assumptions,
evidence limits and scope; record Phase1 and return pass/digest to manager.
Only accepted Phase1 crosses governed delivery seam. One local v0.2.0 minor
outcome, no main edit, push, PR opening, merge, tag, integration or publication.
No performance guarantee, UI/browser or detector change. Current autonomous
policy applies without invented person gates or source-safety waivers.
