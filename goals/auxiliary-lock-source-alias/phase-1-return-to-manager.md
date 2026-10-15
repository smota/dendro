#47 Phase 1 recorded with actual public runtime confirmation. Please
independently accept or return criteria, then route only by governed handoff deliver.

Pass: https://github.com/smota/dendro/issues/47#issuecomment-6181498629
Spec: /home/sam/code/dendro/goals/auxiliary-lock-source-alias/phase-1.md
SHA-256: 0dbf9c8ea2dffadc93acb4e9b9ebec88d13853c7cca09b58009fb16db95183f8
Receipt: /home/sam/code/dendro/goals/auxiliary-lock-source-alias/phase-1-receipt.md
Read-back verifies phase1, analyst signer, allowedNext2/0 and body match.

Actual lock is D.parent/.dendro-output.lock, shared across names, not D.lock.
Entryd7f1807 public CLI with distinct D and preexisting lock hard-linked to
source main/WAL/SHM: each successfully opens source inode O_RDWR|O_CREAT,
acquires LOCK_EX|LOCK_NB, then rejects unsafe lock. Runtime risk confirmed in
these synthetic fixtures; no observed byte mutation/corruption/owner exposure
or frequency claim. Source/tree state unchanged. Safe-output control succeeds;
two output names share lock-busy exclusion and succeed after holder release.
Observer provenance8 hashes and literal negative/FD/descendant/mapping controls
pass; zero trace unknowns, full normal descendant exit, payload buffers raw.
27 analyst artifacts pinned, manifestfd17add2; not a Phase3 frozen QA oracle.

Pins refusal before writable source access/locking, retained lock/link/symlink/
permission/concurrency and source/output/privacy protections, #46 primary alias,
root/key/snapshot and #30 exact retry/five-attempt/quick_check regressions.
Architecture retains assurance/solution/isolation. No broad key/stage redesign
or universal namespace-race guarantee. Four Cargo gates pass; no repair candidate
or QA/review acceptance. Permission/platform extremes and real exposure unknown.

#30 local acceptance is satisfied only for830a920; do not revive its historical
hold or imply #47 already fixed. Localv0.2.0; existing integration/publication
boundaries remain, no #12/#13 acceptance. No direct delivery bypass.
