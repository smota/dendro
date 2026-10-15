# Auxiliary output lock may open a source hard link write-capable before refusal

Disposition: retain as a separate source-protection backlog investigation.
pm.manager / pm-manager@dendro-pm, Codex, 2026-10-14 UTC.
No release slot or implementation authorization; no new #46/#30 gate.

When my analytics destination is distinct but a preexisting auxiliary lock
aliases a source, I want a safe refusal before write-capable source access,
so locking the output cannot disturb the source data.

Static inspection at1e4909bee5eefe924cff6c1c1c87ce51f32bfd46 confirms
with_output_lock opens a regular path read/write/create before checking held
device/inode and nlink. A hard link passes the earlier regular-file check;
later unsafe-lock rejection does not prove the earlier open was read-only.

Hypothesized public trigger: distinct analytics destination D and selected
source S, with D's auxiliary output-lock path already hard-linked to S.
No runtime write-capable source access, source-content mutation, owner exposure,
exploit or occurrence frequency is established. No damaging/owner-source
probe was performed. Supported filesystem/permission prerequisites and the
exact public trigger require bounded synthetic reproduction before declaring
a verified runtime defect or approving a repair.

Origin observation goals/source-output-alias/auxiliary-lock-observation.md
SHA-256 a12cce45d8a8ebfaff110867791acd5462abcfa237d7ffa23432de285702bf79
Independent advisory .agent-runs/review/46-prebuild/advice.md
Architect plan .agent-runs/issues/46/council/repair-plan-v1.md
Referral qitem-20261014222611-ae6d0d6f.
PM checked observation digest and the current source open/check ordering.
These are static/advisory records, not a repair candidate or QA verdict.

Next planning decision belongs to supervisory steering with PM. If selected,
analysis must pin source/lock identity and expected refusal before write-capable
opens, validated inode-aware operation observation and source/output state on
synthetic inputs. Hash equality or final lock rejection alone is insufficient.
Architecture owns assurance/technical scope; QA/review independent proof.
No owner keys, real-source destructive control or universal filesystem-race
promise. No automatic implementation dispatch from this backlog record.

#46 remains confined to its accepted primary --db identity/containment outcome,
with distinct-output fixtures requiring safe auxiliary artifacts. This is a
different precondition expressly excluded from auxiliary identity redesign;
it neither waives general read-only protection nor expands #46. #30's existing
HOLD/#46 dependency/composed-candidate requirements remain unchanged.
No retry/metric/schema expansion, #12/#13 pass, integration or publication.

Backlog issue: https://github.com/smota/dendro/issues/47
Referral closed as no-follow-on: retention/triage complete, no selected active
investigation or build. Analyst notified; existing specialist workflow proceeds.

Later selection2026-10-15: steering intake d8a02c697c30ce36 explicitly selects
bounded synthetic investigation and conditional repair. Historical backlog-only
status above is superseded by Phase0
https://github.com/smota/dendro/issues/47#issuecomment-6181442392.
Runtime defect remains unverified at intake; actual lock is destination parent/
.dendro-output.lock. Prior #30 local hold has since been satisfied by exact
composed830a920; preserve that candidate-bound acceptance and release boundaries.
