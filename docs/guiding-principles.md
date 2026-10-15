# Dendro guiding principles

Dendro provides local-first fleet intelligence; fleet governance belongs to the operator. Existing Rust safety, parameterized-query, bounded-processing and credential-redaction standards remain in `AGENTS.md`.

Autonomy operates within the configured review and action boundaries. Independent review and human approval remain mandatory where `sdlc.config.json` requires them. A role name, preferred model or heartbeat grants no authority.

Desired configuration, installed configuration and actual runtime evidence are different facts. Verify current occupant identity, task ownership, source history and runtime readiness before admitting work. Unknown evidence remains unknown.

Preserve existing candidate/oracle digests, run/epoch/fence, durable custody and publication holds during recovery. Reconcile interrupted operations before redispatch; never initialize a replacement ledger to bypass unresolved history.

Use project-scoped dependencies and isolated worktrees. Preserve unrelated modifications. Apply reviewed adoption changes transactionally with an exact rollback baseline. The current backlog pause remains in force until the operator explicitly resumes it.
