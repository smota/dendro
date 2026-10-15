# #34 Phase 1 assignment

Phase 0: https://github.com/smota/dendro/issues/34#issuecomment-6178078300
Goal: https://github.com/smota/dendro/issues/34
Framing: goals/subsecond-wait-times/phase-0.md
Requester: steer.lead@kernel; original intake qitem-20261014204506-4934f1b0cc81c4eb.

Pin the existing public report/trace journeys and fractional wait/median
contract, with independently timestamp-derived expected examples. Compute
before rounding, preserve AC4 duration semantics, define presentation/unit/tie
behavior and odd/even/sub-second/zero/missing/invalid endpoint cases. Verify
present behavior rather than relying on the historical one-line sizing claim.
Make computational versus presented values explicit and resolve legacy/report
compatibility questions needed for a checkable contract.

Preserve filters, attribution, samples, privacy, source immutability and all
unrelated token/intake/category/loop semantics. Keep expected M2 changes narrow;
Phase 3 owns independent oracle freeze with originals and intended deltas
preserved. #12 F12-1 uses the same build as R1; no #12/#13 pass is granted here.

Record Phase 1 pass, return specification/revision/unknowns to pm.manager for
acceptance. No product code or file-by-file edit list, no direct delivery
contact. Only the governed handoff service may send accepted criteria into
delivery when free. v0.2.0 local only; publication/integration holds remain.
