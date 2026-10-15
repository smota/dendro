# #30 Phase 1 analyst receipt

2026-10-14, pm.analyst / pm-analyst@dendro-pm, Codex / codex-cli /
local-cli / current-session. Claimed qitem-20261014213319-dec426c1.

Pass: https://github.com/smota/dendro/issues/30#issuecomment-6179032841
Specification: goals/busy-source-retry-hint/phase-1.md
SHA-256: 98feea6dc328abcb4d2c6296b40cd26f1c2e8fb96c31b1ba556f58138e5a9f39
Key: 30-phase1-busy-source-wording.
Read-back confirms phase 1, allowedNext 2/0, verified analyst signer, body
matching after the reader strips final newline; phase-1-readback.json.

Pinned wording-only terminal diagnostic: preserve source-change/5-attempt/no-
ingestion clause and add: Retry in a moment, or retry when the rig is quieter.
Preserve exit 1/stderr/no ready JSON; attempt limit/retry/copy/integrity policy,
source containment/privacy, transactional publish/cleanup and unrelated errors.

Baseline source 1e4909bee5eefe924cff6c1c1c87ce51f32bfd46, built binary
ad7d769a6bd9075bcfce08c50ffdb5281940d6d89206e514dae470e4c21a3af0.
Synthetic public CLI probe reproduces exact old message on existing and fresh
outputs. Existing analytics byte hash stays identical; fresh database absent;
no stage remains. Stable controls before and after succeed. Corrupt-file control
retains separate file-is-not-a-database error. Source bytes unchanged; mtime
changes come only from the fixture producer. This is an artificial race proof,
not real-rig refusal frequency, source-write syscall assurance, or a throughput
benchmark. All nine checks in baseline-probe/checks.json pass. Synthetic output
canaries are absent; decoded-storage/syscall proof remains for candidate QA.
Artifacts: baseline-probe.py, baseline-probe/summary.json and stdout/stderr.
No owner payloads, sources or key material were read.

Material retained limit for explicit manager acceptance: non-OK private-copy
quick_check results can also exhaust the existing loop and reach the same
terminal diagnostic. This pass does not certify every cause of exhaustion as
source mutation or change that classification. Separately emitted corrupt/
permission/missing-root/containment errors keep their existing dispositions.
A broader diagnosis/classification change requires separate disposition and
must not be silently added to this wording-only task.

Four workspace checks pass: cargo check --workspace, cargo test --workspace,
cargo clippy --workspace -- -D warnings, cargo fmt --check. Command exits/logs:
phase-1-checks.json and phase-1-cargo-*.log. These verify entry workspace only;
no #30 candidate built or QA/review accepted. This tenure writes only this goal
directory; no product edits, commits, direct delivery contact or publication.

Next owner pm.manager: independently accept/return criteria including the
retained integrity-path limit. Only governed handoff deliver may send accepted
criteria to a free delivery squad. Phase 3 freezes independent expectations,
preserves original oracle evidence and records only the intended diagnostic
delta. Real-rig refusal frequency, candidate behavior and candidate privacy/
source-access evidence remain unknown. No #12/#13 acceptance, retry-policy or
performance change; existing integration/publication holds remain.

Durable custody: qitem-20261014213319-dec426c1 closed as handed_off_to
pm-manager@dendro-pm, successor qitem-20261014213656-05789bf8.
