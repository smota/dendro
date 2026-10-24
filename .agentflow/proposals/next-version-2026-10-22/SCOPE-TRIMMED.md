# Dendro v0.8.0 — trimmed scope (2026-10-24)

Supersedes the schedule in `tasks.json` (35 tasks). DESIGN.md stays the reference for
*what facts mean*; this page decides *what we build and how we prove it*.

## What the user asked for (the bar)

1. Intelligence across model, harness and agentic-platform (OpenRig) layers — not observability/ETL.
2. Environment layer (isolation, processes) and security as first-class facts.
3. Sub-agents traced to their root callers.
4. An extensible insight layer; strict, accessible definitions.
5. Native Windows **and** Linux.

Anything not serving one of these is cut or deferred.

## Done (merged #74, #76)

B00, B01-core, B02, T02, T02-runner, T03, T24, T04 — typed core, native platform safety,
schema 3→4 migration/rollback, storage recovery, native Windows + Linux CI.

## Remaining work: 8 tasks, one PR each

| # | Task | Replaces | Serves |
|---|---|---|---|
| 1 | **Privacy-guard hardening**: guard new-DB creation, refuse unguarded writable `open_existing`, set spill dir before WAL replay | review follow-ups from #76 | 2, 5 |
| 2 | **Capture registry + readers**: registry, Claude/Codex/Pi readers, 4 export readers, Grok parser (fixtures = real local sessions, redacted) | T05, T06, T07, T08, B01-G | 1 |
| 3 | **Environment collectors**: Linux + Windows unprivileged process/resource collection | T10, T25 | 2, 5 |
| 4 | **Lineage**: generic sub-agent lineage + OpenRig adapter + outcomes | T12 | 1, 3 |
| 5 | **Security + accounting**: security facts, privacy boundaries, token/time/cost conservation | T13, T14 | 1, 2 |
| 6 | **Insight layer**: analyzer/answer extension contract + ~6 reference insights (not 21) | T15, T16 | 4 |
| 7 | **End-to-end wiring + surfaces**: register readers, ingest, CLI/TUI/dashboard, Nao answers | T18, T17, T19 | 1, 4 |
| 8 | **Release**: native CI on both OSes, one install/upgrade/rollback smoke test, release notes | T27, T21, T22, T23 | 5 |

## Cut

- **T09 AGY adapter** — AGY is not available.
- **T20 hostile-input conformance suite, T22 integrated perf acceptance, T02-baseline** — folded into
  ordinary tests in each task; latency targets are directional (#75).
- **B01-S/B01-W/B02-W specimen admissions** — only existed to gate syscall/ETW importers (deferred).
- **Process ceremony**: frozen per-task oracle packs, evidence receipts/digests, worker identity
  contracts, oracle-owner/author separation, checkpoint/custody JSON.

## Deferred to v0.9 (user decision, 2026-10-24)

- **T11 syscall-trace import (strace) and T26 Windows ETW event import.** Both need real
  privileged producer specimens before they are trustworthy. Task 3 still delivers
  unprivileged process/resource facts on both OSes.

## How each task is proven

- Gates: `cargo check / test / clippy -D warnings / fmt --check` on Windows and Linux CI.
- Tests that exercise the behaviour, written alongside the code. No separate oracle phase.
- One independent Claude review per PR; findings go in the PR, not in JSON receipts.

## Routing (cost-effective)

| Work | Route |
|---|---|
| Reading / discovery | Haiku sub-agent |
| Build loops | Grok `grok-4.7-build-fast` (medium); Codex when capacity returns |
| Diff review (second model) | Claude Sonnet |
| Architecture calls | Claude (this session), sparingly |
