# #34 Phase 1 product acceptance and delivery handoff

Decision: accept Phase 1 contract. pm.manager / pm-manager@dendro-pm,
Codex / codex-cli / local-cli / current-session, 2026-10-14.
Goal https://github.com/smota/dendro/issues/34
Pass https://github.com/smota/dendro/issues/34#issuecomment-6178194115
Spec goals/subsecond-wait-times/phase-1.md
SHA-256 bc1b32bbcfad446945bacc4054990511752a982e0e11b0d8dc5731627655bc08
Receipt goals/subsecond-wait-times/receipt.md
Custody qitem-20261014205227-13685408.

Verified digest and live phase1 pass by verified pm.analyst, allowedNext2/0.
Read the contract, reproduction summary and independent decimal examples.
Current source confirms the existing per-wait whole-second cut and AC4's
span-anchored integer-prefix conservation. Reproduction evidence remains
analyst evidence, not PM candidate verification.

Explicit product dispositions:
- Accept existing report handoffs / trace medians numeric JSON seconds with
  no deliberate fixed-decimal rounding; 24.734 remains numerically 24.734.
- Accept preserved microsecond endpoints and half-microsecond even medians.
  Beyond-six-decimal endpoint preservation remains a disclosed limit; this
  correction does not add timestamp parsing/storage precision or a migration.
- Accept AC4 reconciliation: common elapsed-duration principle, not identical
  quantizers. AC4 integer-prefix band conservation and activity semantics stay
  unchanged; individual waits and their medians retain fractions.
- Accept odd/even/duplicates/zero/null, existing claim selection and keys,
  invalid endpoints, window/post-window claims and report/trace parity examples.
  Formula provenance must identify the changed rule. Existing supported DBs
  compute corrected waits read-only without recovering already lost precision.

Next owner orch.arch / orch-arch@dendro-dev through governed handoff deliver.
Choose phases2/3 strategy and numerical representation; prove supported ranges.
Freeze a QA-owned independent timestamp-derived oracle before build. Preserve
original expectations and identify only intended wait/median/provenance deltas.
No candidate-derived goldens or blanket M2 rebaseline. Candidate performance
and real-corpus M2 deltas remain unknown. QA5 and independent review6 required.

Preserve membership, filters, attribution, token/intake/category/loop semantics,
AC4 conservation, source immutability and privacy. #12 F12-1 compares with R1
on the same build; this grants no #12/#13 acceptance. v0.2.0 minor local only;
no main edit, push, PR, merge, tag or publication. Existing holds remain.
Return candidate evidence to pm.manager for product acceptance. This decision
accepts criteria, not a built candidate or permission to publish.

## Delivery receipt

At 2026-10-14 20:53Z the governed service returned delivered, reason delivery
squad is free. Successor qitem-20261014205318-8418129b68bbcbe8 verified through
full daemon read: source manager, destination orch.arch, body contains this
acceptance custody, transition and digest. Architect claimed it at
20:53:33.962Z; full read confirms in-progress. Source acceptance row closed
with custody-verified note.
Analyst notified. This proves routed custody, not a candidate or phase2 pass.
