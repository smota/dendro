# Refuse analytics output aliases before write-capable access to sources

QA on #30 candidate e34b2c1ee7cd99a8dd6f41988ed9edcedc80d52c found that
`dendro run --db SOURCE --root openrig=SOURCE` opens the original source WAL
and SHM O_RDWR|O_CREAT, truncates SHM and maps it shared writable before refusing
the file as an analytics database. The pinned baseline reproduces the defect.
This violates R1 AC12 source read-only protection and #30's unchanged-source
invariant. AC12's historical SHM-only exception never permits WAL writes.

When I accidentally select a live source as analytics output, I want Dendro
to refuse before write-capable access or modification of that source and its
sidecars, so even a failed ingest cannot disturb the agent fleet's source data.

Given output aliases a source, normal public ingest must refuse without ready
JSON, source write-capable opens, create/delete/truncate/shared-write mappings,
or changes attributable to Dendro. Pin direct alias and the equivalent path
identities recognized by the existing alias/containment contract; analyst must
derive the exact boundary instead of assuming spelling equality is sufficient.
Use synthetic fixtures and source-operation observation plus state evidence;
matching hashes alone are insufficient. An ordinary distinct output remains
usable. Preserve existing root-binding refusal, key/privacy rules and snapshot
policy. This is source protection, not a new retry policy or metric.

Separate defect delivery: do not hide the repair in #30's two-file wording
candidate or waive its invariant because baseline fails too. #30 cannot receive
an unqualified QA/readiness/product pass until the source-protection dependency
has independent QA/review and the resulting composed candidate is reverified.
No fabricated phase5->phase1 return; #30 remains pending candidate QA.

Evidence: .agent-runs/issues/30/qa-e34b2c1/alias-custody-finding.json
SHA-256 27d2e682c175eb8f15104949ecb0ea9cbd6323346330ff021def8625e2d021e9
Candidate syscall trace: .agent-runs/issues/30/qa-e34b2c1/observer/root-output-alias.syscalls
SHA-256 7a284b27bfc5fa66f85e0f19919d9f88ea139c09e8382320749e802a68aba0d7
Architect request: .agent-runs/issues/30/source-alias-disposition-request.md
PM inspected candidate trace lines418–425 and current/candidate preflight order.
Only synthetic source evidence inspected; no owner source/key accessed.

Architect owns root-cause solution, assurance, isolation and implementation plan;
QA/reviewer independently verify the candidate. No product implementation list.
v0.2.0 minor local only, no push/PR/merge/tag/publication authorization. No
#12/#13 acceptance, generic filesystem hardening, race-free universal guarantee,
or change to unrelated snapshots/diagnostics. Existing holds remain.
