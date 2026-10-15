#34 Phase 1 proof contract recorded. Please independently accept or return the
criteria, then decide delivery availability through the governed handoff service.

Pass: https://github.com/smota/dendro/issues/34#issuecomment-6178194115
Spec: /home/sam/code/dendro/goals/subsecond-wait-times/phase-1.md
SHA-256: bc1b32bbcfad446945bacc4054990511752a982e0e11b0d8dc5731627655bc08
Receipt: /home/sam/code/dendro/goals/subsecond-wait-times/receipt.md
Read-back verifies phase 1 pass, analyst signer and allowedNext 2/0.

Reproduced the current public report/trace defect on synthetic sources at
13423047: 24.734 becomes 24; fractional odd/even examples become zero.
Source unchanged; microsecond endpoint precision survives, nanoseconds do not.
Contract uses existing report handoffs / trace medians numeric JSON fields in
seconds without fixed-decimal display rounding. Pin exact microsecond waits,
half-microsecond even medians and independent decimal expected examples;
preserve membership, sample keys, invalid counts, filters, attribution and all
unrelated metrics. AC4's existing integer-prefix conservation stays unchanged;
please explicitly accept this reconciliation of the same-duration intent and
the retained finer-than-microsecond precision limit.

Four current-workspace Cargo checks pass. No candidate built or QA/review pass.
Architecture retains representation; Phase 3 freezes independent QA-owned oracle
with original R1/M2 evidence and narrow intended deltas. Candidate performance
and full-corpus M2 deltas unknown. #12 F12-1 uses R1 on the same build; no #12/#13
pass. Local v0.2.0 only; preserve integration/publication holds. No direct delivery
contact: only agentflow-sdlc handoff deliver may send accepted criteria onward.
