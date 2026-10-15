# Phase0 — #47 auxiliary output-lock source protection

Decision proceed to analysis/reproduction and, if confirmed, bounded repair.
pm.manager / pm-manager@dendro-pm, Codex / codex-cli / local-cli /
current-session, 2026-10-15 UTC. Key47-phase0-auxiliary-output-lock.
Goal https://github.com/smota/dendro/issues/47
Requester steer.lead@kernel; intake qitem-20261015002030-d8a02c697c30ce36.
Brief /home/sam/code/agentflow-sdlc/.agent-runs/briefs/issue-47-brief.md.
This steering selection supersedes the prior backlog-only scheduling decision;
it does not establish a reproduced runtime defect or candidate acceptance.

## Job, evidence and forces

When my analytics destination is distinct but its auxiliary lock aliases a
source, I want safe refusal before write-capable source access, so output
locking cannot disturb the source data even if ingest ultimately refuses.

Origin observation goals/source-output-alias/auxiliary-lock-observation.md,
SHAa12cce45d8a8ebfaff110867791acd5462abcfa237d7ffa23432de285702bf79.
Historical static finding is still present at entryd7f180715440d0389101626cfc1eedbb0ce8490b:
regular-file check then read/write/create open, followed by lock and identity/
nlink rejection. No runtime source access, mutation, owner exposure or frequency
is established by this source ordering alone. No owner-state probe performed.

Important public-path correction: current run uses the analytics destination
parent's .dendro-output.lock, shared by outputs in that directory, not a D.lock
suffix. Analyst must verify exact argv/cwd/root/lock identity in the public
journey before freezing any reproduction. A helper-only hardlink demonstration
does not establish the normal CLI's source access.

Push is a confirmed open-before-link-validation risk. Pull is source-safe
locking/refusal. Anxiety is a late error masking a write-capable source open;
inertia is existing eventual lock rejection and already accepted primary alias
protection. User frequency/commercial/emotional/social evidence remain unknown.

## Outcome and scope

First establish supported static local-path trigger and source operations on
new synthetic fixtures with a distinct primary output and preexisting auxiliary
lock hard-linked to a source. Distinguish attempted write-capable access from
actual byte mutation. A successful or failed writable original-source open is
not safe merely because no content changed. No damaging real/owner experiment,
owner keys, privilege escalation or authenticated environment changes.

If runtime risk is confirmed, the accepted outcome is refusal before auxiliary
locking can open the protected source write-capable, without changing source
state. Existing lock exclusion, identity/link/symlink/permission protections and
normal distinct-output success must remain. Architecture chooses representation,
assurance, isolation and implementation; product prescribes no file-edit list.
This is auxiliary lock protection, not arbitrary key/stage/recovery redesign,
universal malicious namespace-race safety or broader filesystem support.

Analyst pins source/lock identity boundaries and public expected refusal,
operation observation across inode/path aliases and descendants, whole source/
output state and safe concurrency/normal-output controls. Include original
main/WAL/SHM where applicable under existing source guarantees, synthetic
canaries and negative observer controls. Payload-free operation evidence plus
state manifests required; final error or matching hashes alone insufficient.
Missing observer/runtime evidence stays unknown. If the hypothesized path is
not reproduced, report preconditions and limits; do not invent a successful
exploit or launch an unrelated rewrite to force a repair outcome.

Preserve accepted #46 primary-output identity/containment behavior and #30
manual retry/five-attempt/quick_check classification, stable metrics/reports,
root/key/snapshot/publication behavior and existing privacy. Their local
acceptances remain candidate-bound; #30's earlier local acceptance hold was
satisfied by exact830a920, not retroactively by originale34b2c1. Do not resurrect
that historical hold from stale backlog prose or imply #47 was already fixed.

## Next owner and release

pm.analyst writes Phase1 with reproducible public journey, expected safe and
unsafe operations/state, exclusions and unknowns; return pass/digest/evidence
to pm.manager for acceptance. Phase3 later freezes independent oracle before
build; QA/review remain independent. Only governed handoff deliver moves accepted
Phase1 to delivery when available. Current autonomous high-assurance policy
applies without invented person confirmation; no assurance downgrade or gate
bypass. Actual machine rejections must be recorded and resolved honestly.

v0.2.0 minor local only. No main edit, push, PR, merge, tag, integration or
publication; no #12/#13 acceptance or performance/clock/rate guarantee. Existing
release boundaries remain. Read steering brief/issue, origin observation,
current public lock construction/open ordering and prior product acceptances.
