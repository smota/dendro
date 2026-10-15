# Auxiliary output-lock/source identity observation — product referral

2026-10-14 UTC. pm.analyst / pm-analyst@dendro-pm, Codex / codex-cli /
local-cli / current-session. Incoming qitem-20261014222409-8936430d9c70a80a,
from orch.arch. This is a backlog referral, not a phase transition, scope
amendment, implementation authorization or acceptance verdict.

## Disposition

Retain and route to pm.manager as a separate source-protection observation for
backlog triage. Do not add it to #46 or #30 without a new explicit product
record and governed delivery route. Do not characterize it as repaired or as
an accepted exception to general source read-only protection.

Accepted #46 Phase 1 remains unchanged:
https://github.com/smota/dendro/issues/46#issuecomment-6179510551.
Spec goals/source-output-alias/phase-1.md SHA-256
69e6994ba18d182e416b11cae7465f5917383ef5335327af508040f5ec8d08f8.
Its bounded primary --db identity/containment contract explicitly excludes
redesign of auxiliary-file identity policy. Its distinct-output success cases
require separate safe auxiliary artifacts. This new observation instead uses
a distinct primary destination with a separately unsafe preexisting lock path.
Those are different preconditions; the auxiliary exclusion does not waive the
in-scope primary-alias source-operation/state requirements.

## Evidence and bounded claim

Independent reviewer advisory:
.agent-runs/review/46-prebuild/advice.md
SHA-256 9e4a637da5bd048bf1438d10ff14e0e77eb99050746f2f4de5d52642be98641f.
Architect proposed plan:
.agent-runs/issues/46/council/repair-plan-v1.md
SHA-256 0c2e8b7d927ac6c9df72bff7d23cc172b4046c188718482ef3b7345935e29b3b.
These are analysis/advisory records, not a frozen oracle or candidate verdict.

Analyst inspected pinned HEAD 1e4909bee5eefe924cff6c1c1c87ce51f32bfd46,
with_output_lock in crates/dendro-core/src/output.rs. The function checks that
an existing lock path is a regular file, then opens it with read/write/create
capability and truncate(false). It acquires the lock before checking the held
and named device/inode identity and requiring nlink == 1. A preexisting hard
link is a regular file; the later nlink rejection does not establish that the
earlier open was read-only. This confirms the source ordering and a potential
source-write-capable access path when the auxiliary lock is separately aliased.
It does not establish an observed source-content write or mutation.

Hypothesized public trigger for future synthetic proof: primary analytics D
is distinct from selected source S, while D's parent output-lock pathname is
pre-created as a hard link to S. Later rejection can still occur after the
source inode was opened write-capable. Exact supported filesystem/permission
preconditions and runtime operations remain unverified here. No owner-state
reproducer, damaging control, candidate execution or source/key access occurred.
Static inspection is not evidence of actual owner exposure or exploit frequency.

## Next owner and bounded future work

pm.manager owns retention/triage and whether to propose a separate backlog issue.
The next product record should frame safe auxiliary-artifact refusal without
expanding to a universal filesystem-race guarantee. Architecture would own
assurance and technical scope. Before any repair acceptance, independent QA
would need synthetic public command/file-identity evidence, an observer that
captures write-capable attempted opens by inode across aliases, and source/
output-state manifests. Hash equality or the eventual unsafe-lock error alone
cannot prove protection. No owner keys or live-source mutation tests.

No priority, release slot, new scope or fix is approved by this referral.
An unverified runtime consequence remains unknown; the static ordering risk
must not disappear from custody merely because it is outside #46. PM can
accept the referral, route further bounded investigation, or record another
explicit disposition; implementation requires its own accepted outcome.

#46's current analysis/repair continues under its existing authority and gates;
no new gate is imposed by this advisory. #30 remains HOLD until its existing
#46 dependency and composed-candidate verification are satisfied. Neither
primary source protection nor the retained quick_check classification is waived.
No phase 1 amendment, code change, publication/integration authorization,
#12/#13 acceptance or message to a builder is made here.
