# #47 product acceptance — local auxiliary-lock repair

Decision accept exact local delivery; no rework requested. Integration and
publication remain held. pm.manager / pm-manager@dendro-pm, Codex / codex-cli /
local-cli / current-session, 2026-10-15 UTC.

Goal https://github.com/smota/dendro/issues/47
Contract https://github.com/smota/dendro/issues/47#issuecomment-6181498629
Spec SHA-2560dbf9c8ea2dffadc93acb4e9b9ebec88d13853c7cca09b58009fb16db95183f8
Phase8 https://github.com/smota/dendro/issues/47#issuecomment-6182020361
Verified goal revision98fd53d9c0f30575ef68b4847dcfdfdc5ee38024a4932bb368980bddcc4e728b
Candidate7a7f934299712e278cfd8f17892765db6214832f
Tree180941d7199d6c3231a289d87d831b8d9b0fd4e0
Base d7f180715440d0389101626cfc1eedbb0ce8490b
Branch fix/auxiliary-lock-source-alias
Worktree /home/sam/code/dendro/.worktrees/auxiliary-lock-source-alias
Release SHA-25673c77e82551e287f292a3b23cbdd80540e17fe19dbd48dd95e8a08f3391aeba5

## Accepted outcome and independent proof

Preexisting regular multiply linked output locks are refused before writable
open or protected-source lock acquisition. Only genuine absence permits lock
creation; opened/named/preexisting identities are validated before exclusion
and again under the owned guard. Directory-wide nonblocking exclusion, ordinary
new0600 and existing single-link permissions/reuse remain. No unlink/recreate,
source scan, policy/schema/parser/metric/retry/key redesign introduced.

Verified live Phase8 and distinct builder/QA/reviewer passes, clean HEAD/tree,
parent and spec/release/oracle/QA-manifest digests. Read full independent QA and
review reports plus local draft. Product acceptance relies on independent
evidence, not new PM test execution, self-review or a requester journey witness.

QA and reviewer independently executed35 auxiliary cases and62 primary cases,
all passing with0unknown. The27 auxiliary refusals have0original regular content/
writable/creating opens, protected lock/prohibited effects, key/stage/ingest
progression and no observable source/link/output/tree changes. Safe D1/D2 use
one physical lock; both remain busy under the holder and a new run succeeds
after release. Helper errors/mismatches have inspected source and meaningful
unit proofs, not fictional deterministic public races. EACCES/unreadable entries
retain explicit markers; inaccessible bytes are not claimed directly hashed.

70 reports and8 medians traces preserve frozen baseline results under declared
normalization. Fresh98 wording/control overlay checks retain #30's exact hint,
actual five-copy/EOF/size/sync/close/cleanup and static source protection. Original
alias-class false remains visible, with only the previously authorized one-
pointer overlay; no prior HOLD revived or old unsafe candidate accepted.
Decoded privacy scans enumerate51persisted columns with0fake-canary hits.

Builder actual Cargo check/test/clippy/fmt/release receipts are independently
verified. QA/reviewer honored no-rebuild handoffs and each executed hashed
compiled workspace tests:149 passed, one existing benchmark ignored. This is
direct tests plus verified builder Cargo/build, not fresh independent Cargo4.
Original833-file oracle and628-file QA evidence remain preserved.

Oracle f36af66e582e5b7a3384822c9220a7a38d5cf4d7169ebfe7c141f9dcbb9a9b06
QA manifest367813759b10ad508fb905426b1dae097ab3ea71fc178d964f601f110724dbc9
QA https://github.com/smota/dendro/issues/47#issuecomment-6181925520
Review https://github.com/smota/dendro/issues/47#issuecomment-6182003823
QA .agent-runs/issues/47/qa-7a7f934/QA_REPORT.md
Review .agent-runs/review/47-7a7f934/verdict.md
Draft .agent-runs/issues/47/pr-draft.md

## Retained limits and supervisory disposition

Bounded static preexisting regular two-name hardlinks only. A single-link source
itself named as lock, malicious concurrent namespace/mount replacement and
arbitrary key/stage aliases remain excluded/unproved. Readonly metadata checks
cannot undo a writable open obtained in a race; documentation discloses that.
Default proof uses isolated HOME-absent child/synthetic OPENRIG_HOME, not owner-
present default coverage. Literal trace controls do not promise all platforms/
syscalls. Memory/spill/export, real-owner exposure/frequency and performance
remain unproved. Retained quick_check exhaustion classification remains.

Prior #30/#46 acceptances remain bound to their named candidates; no historical
hold revival, global source-safety waiver or #12/#13 acceptance. v0.2.0 minor
local only. Integration outstanding; no push, PR opening, merge, tag or
publication. Supervisory steering owns later integration/release/requester
journey disposition; generated next4 is a possible rework edge, not a rebuild
instruction for this accepted candidate. Current autonomous implementation
authority does not waive the retained release boundaries.

## Return receipt

Acceptance recorded issuecomment-6182029307. PM readiness return closed
transactionally into supervisory relay qitem-20261015010844-6136c037 at the
registered operator fallback. Architect notified of acceptance/no rework.
Supervisory pickup and integration remain unestablished; no release authorized.
