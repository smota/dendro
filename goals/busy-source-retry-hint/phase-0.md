# Phase 0 — #30 busy-source retry guidance

Decision: proceed to analysis. pm.manager / pm-manager@dendro-pm,
Codex / codex-cli / local-cli / current-session, 2026-10-14.
Key 30-phase0-busy-source-retry-hint.
Goal https://github.com/smota/dendro/issues/30
Requester steer.lead@kernel; intake qitem-20261014213204-927e9e65b4b72cdb.
Brief /home/sam/code/agentflow-sdlc/.agent-runs/briefs/issue-30-brief.md

## Job and evidence

When ingest refuses because a live OpenRig source keeps changing during all
snapshot attempts, I want the refusal to tell me I can retry shortly or during
a quieter period, so I have a next step without weakening the read-only guard.

Current source contains the refusal stating that the source changed during
each snapshot attempt and nothing was ingested, without retry guidance. The
issue reports this from R1 AC12 cycle2. Prior PM classification documents15/15
successful runs at5/10/25/26/50 commits per second, with1–3 attempts, on the
pinned historical candidate. These are finite historical probe results, not
current performance, real-rig refusal frequency or a retry-success guarantee.
The real-rig review question in G2 section10 item6 remains unanswered on the
sources read. Steering's current brief authorizes the bounded wording task;
this does not resolve or bypass that separate observation question.

Push: a refusal lacks actionable recovery guidance. Pull: the same diagnostic
names a safe next action. Anxiety: guidance could falsely imply automatic
retry, guaranteed success or permission to alter live sources. Inertia: the
existing refusal already protects sources. Emotional/social value unverified.

## Outcome and boundary

The existing busy-source refusal adds short English guidance to retry in a
moment or retry when the rig is quieter. Preserve the causal diagnosis and
truthful ingestion disposition. The wording must not imply that Dendro will
automatically retry indefinitely or that waiting guarantees success. Do not
tell the operator to stop agents, edit sources, delete the analytics database,
change accounts, locks or credentials, or relax read-only protection.

This changes diagnostic wording only. Preserve production snapshot attempt
count5, retry policy, copy/consistency and SQLite integrity checks, exit/error
behavior, source containment, sanitization, source immutability and ingestion
transaction behavior. Other refusal causes must retain their meaning; do not
classify corruption, permission or missing-source failures as transient busy
sources. No metrics, schema, parser, performance or source-control changes.

Analyst pins the real public command/diagnostic journey, exact or semantically
checkable guidance, production attempt boundary, failed-ingest/source state
evidence, stable-source success and unrelated-refusal examples. Privacy proof
uses synthetic canaries; no owner credentials/source payloads in artifacts.
Do not prescribe files to edit. Phase3 freezes independent expected diagnostics
and narrowly documents the intended string delta, preserving originals rather
than deriving goldens from candidate output.

## Release and next owner

v0.2.0 minor, local build only, under staged other-item scope. Not a #12/#13
acceptance or new rate guarantee. Existing integration/publication holds remain;
no main edit, push, PR opening, merge, tag or publication is assigned here.

Read issue, steering brief, current refusal source, historical PM busy-rate
classification, G2 review agenda and staged other-item scope. Current end-to-end
reproduction and real-rig frequency remain unknown; analyst/delivery must prove
the relevant diagnostic journey without operating or mutating owner sources.

Next owner pm.analyst: write a checkable Phase1 contract and return its pass,
digest, evidence and unknowns to pm.manager for acceptance. Only governed
handoff deliver may send accepted criteria into delivery when free. Delivery
owns phases2/3 strategy/proof, build4, QA5, independent review6, docs7/readiness8.
