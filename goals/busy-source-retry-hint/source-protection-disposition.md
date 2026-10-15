# #30 source-protection disposition — HOLD

pm.manager / pm-manager@dendro-pm, Codex, 2026-10-14.
Classify the observed source-output alias failure as a defect against existing
source protection. Do not accept with conditions or waive because baseline
also fails. No unqualified QA5/readiness8/product pass at e34b2c1.

Separate repair issue: https://github.com/smota/dendro/issues/46
Framing: goals/source-output-alias/issue.md
The #30 wording scope, exact sentence and accepted quick_check classification
limit remain unchanged. #46 is not an unrecorded addition to its builder scope.

Current #30 phase4 https://github.com/smota/dendro/issues/30#issuecomment-6179299420
remains pending QA. Do not append a fictional phase5->phase1 return. Resume
unqualified #30 verification only after #46 independent QA/review and deliberate
composition with reverified wording/source-protection evidence on the resulting
candidate. Architect owns assurance, plan/isolation and valid lifecycle actions;
this disposition authorizes analysis, not a silent implementation expansion.

PM inspected synthetic evidence of source WAL/SHM O_RDWR|O_CREAT, SHM truncate
and shared-write mapping before refusal; comparison records SHM hash/mtime
changes. AC12's historical SHM-only exception does not allow WAL writes and
does not waive the stronger frozen #30 source invariant. Statement that source
protection was unchanged requires positive evidence, not baseline parity.

Findings .agent-runs/issues/30/qa-e34b2c1/alias-custody-finding.json, digest
27d2e682c175eb8f15104949ecb0ea9cbd6323346330ff021def8625e2d021e9.
Candidate syscall trace digest
7a284b27bfc5fa66f85e0f19919d9f88ea139c09e8382320749e802a68aba0d7.
Architect follow-up records67 file checks/manifest9e15d5c9 and QA_REPORT.md;
preserve all original findings/oracles. No owner source/key accessed by PM.

Delivery may retain completed bounded wording proofs as evidence. They do not
pass this source-protection criterion. No source operation/rate/metric expansion,
#12/#13 pass, push/PR/merge/tag/publication or integration authorized. Holds stay.
