# #47 Phase 1 analyst receipt

2026-10-15 UTC, pm.analyst / pm-analyst@dendro-pm, Codex / codex-cli /
local-cli / current-session. Claimed qitem-20261015002152-8740320e.

Pass: https://github.com/smota/dendro/issues/47#issuecomment-6181498629
Spec: goals/auxiliary-lock-source-alias/phase-1.md
SHA-256: 0dbf9c8ea2dffadc93acb4e9b9ebec88d13853c7cca09b58009fb16db95183f8
Key: 47-phase1-auxiliary-lock-safety.
Read-back: currentPhase 1, allowedNext 2/0, verified analyst signer; body matches
except reader-stripped final newline. phase-1-readback.json records revision.

Read current framing/brief and source, original observation, #46/#30 exact local
acceptances. Corrected illustrative D.lock to actual D.parent/.dendro-output.lock.
Built entry d7f180715440d0389101626cfc1eedbb0ce8490b; binary SHA-256
0262540a9f54f70ef2218470ec8aba6f41182c33c00190f8844817ca2ac89d9e.
No product code changed; all authored artifacts remain in this goal folder.

Public runtime defect confirmed on three synthetic fixtures: distinct D, actual
shared-directory lock hard-linked to main, WAL or SHM. Each has matching source/
lock dev/inode and nlink >=2; trace shows one successful O_RDWR|O_CREAT source
open through lock, followed by LOCK_EX|LOCK_NB, then exit 1 unsafe-lock error.
No source write/truncate/shared writable map or measured source/tree change
observed. Thus unsafe capability/locking is confirmed, corruption/owner exposure
or frequency is not. Safe distinct-output control succeeds, source unchanged,
zero write-capable source opens. All have normal descendant exit coverage,
zero observer unknowns and no CLI canary leakage.

Eight observer provenance artifact hashes verified; existing payload-free #46
observer imported unchanged with bytecode disabled. Literal failed-open, inode/
descendant/FD, unresolved/unfinished/argv and mapping-upgrade controls pass.
Producer synthetic/quiescent, FDs not inherited; source manifests before teardown.
No owner source/key read, privilege/profile/service change or payload capture.
No decoded-storage privacy pass is claimed by these analyst probes.
Additional safe shared-lock public control: two D names in same directory each
lock-busy exit 1 while holder owns lock, ordinary invocation succeeds after release.

Pinned 27 analyst evidence artifacts in investigation-evidence-manifest.json,
SHA-256 fd17add26ddf9e9065b1dadeeebe9cc387a230eaf198032fc114d6618c877546.
Runtime commands/receipts/traces/operations are under baseline-probe/;
locking-controls/summary.json records hold/release control.
These are current-entry evidence, not a frozen independent QA oracle or a
repaired candidate. Phase 3/QA own independent expectation freeze before build.

Contract pins safe refusal before writable source access/source-inode locking,
source/output state, same hard-link identity family, normal directory-wide
exclusion, retained lock/symlink/link/permission policies and all root/key/
snapshot/#46/#30 regressions. Architecture owns assurance/solution/isolation.
No arbitrary key/stage/recovery redesign or universal malicious-race guarantee.
Candidate repair/permission-platform extremes and real-owner exposure unknown.

All four required workspace checks pass: cargo check --workspace,
cargo test --workspace, cargo clippy --workspace -- -D warnings, cargo fmt --check.
Command exits/logs: phase-1-checks.json, phase-1-cargo-*.log. Existing workspace
checks do not substitute for repaired-candidate independent QA/review.

Next owner pm.manager independently accepts/returns criteria, then only governed
handoff deliver may send accepted Phase 1 to delivery. No code/commit/builder
contact, integration or publication. v0.2.0 local only; no #12/#13 pass.
#30 local acceptance remains exact830a920; old historical HOLD is not revived.
That accepted composition does not certify #47's auxiliary safety.

Durable custody: qitem-20261015002152-8740320e closed as handed_off_to
pm-manager@dendro-pm; successor qitem-20261015002635-d14740dd.
