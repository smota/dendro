# Phase 0 — #34 sub-second handoff waits

Decision: proceed to analysis. Owner/signature pm.manager,
pm-manager@dendro-pm, Codex / codex-cli / local-cli / current-session,
2026-10-14. Key: 34-phase0-subsecond-waits.

Goal: https://github.com/smota/dendro/issues/34
Requester: steer.lead@kernel. Intake qitem-20261014204506-4934f1b0cc81c4eb.
Brief: /home/sam/code/agentflow-sdlc/.agent-runs/briefs/issue-34-brief.md

## Job and evidence

When I compare handoff waits between seat pairs, I want samples and medians
calculated from the recorded timestamp differences before presentation rounding,
so I can see the measured wait rather than a value shortened to whole seconds.

The issue and documented QA observation O1 report that 24.734 seconds becomes
24 seconds and that medians use truncated samples. This is a disclosed shipped
limit; the earlier statement that it changed no decision is historical triage,
not evidence about every operator decision. R1 M2 already promises median wait
by seat pair, and AC7 requires the samples behind medians.

Push: documented loss of fractional waits. Pull: truthful samples and medians
under one consistent duration rule. Anxiety: silently changing expected M2
values or introducing different calculations across reports. Inertia: existing
reports remain useful with their disclosed limit. No new demand, commercial
claim or emotional/social adoption evidence is inferred.

## Outcome and boundary

The existing public handoff report and trace journeys expose wait durations
with sub-second precision. Compute timestamp differences and medians without
truncating samples first. Presentation rounding happens after calculation and
has an explicit, consistent rule. Use the duration semantics already accepted
for AC4 conservation; do not redefine its activity states or observation window.

Analyst must pin existing public journeys, units, source timestamp precision,
rounding/tie examples, odd/even medians, sub-second/zero waits, missing or invalid
endpoint treatment, filters and trace parity. Distinguish computational values
from displayed values; a rounded display must not feed the median calculation.
Exact numbers and any unsupported timestamp precision need source evidence.
Do not guess missing times or links, widen window/attribution rules or change
token/intake/category counts. Architecture owns numerical representation.

Changing wait samples necessarily changes some M2 medians. Phase 3 must freeze
independent timestamp-derived expectations, document each intended difference
from the baseline and preserve original oracle evidence. No candidate-derived
golden regeneration or blanket re-baseline is authorized.

This is one bounded correction, not a new report/interface, threshold, latency
dashboard, duration attribution policy or #12/#13 acceptance. G2 staged scope
already names #34: #12 F12-1 compares with R1 on the same build after this change.
That dependency must remain visible; do not declare #12/#13 passed here.

## Release and next owner

v0.2.0, minor, local build only. No push, PR opening, merge, tag or publication.
No main edit or implementation is assigned by this product framing. Existing
publication holds remain; integration decisions stay with supervisory steering.

Read: issue/steering brief, R1 M2 and AC7, specification's O1 triage, G2 staged
other-items/dependency note. Exact present output and AC4 numerical mechanism
are analyst/delivery verification needs, not asserted as reproduced here.

Next owner pm.analyst / pm-analyst@dendro-pm: write a checkable Phase 1 contract
with outcome, public journey, constraints, exclusions, references and unknowns;
no file-by-file edit list. Return to this manager for acceptance. Only the
governed handoff service may move the accepted Phase 1 pass to delivery when
the squad is available. Delivery retains phases 2/3 strategy and proof plan,
QA5, independent review6, docs7 and readiness8 under project policy.
