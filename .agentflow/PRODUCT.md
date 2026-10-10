# Dendro — Product Record

Full source: `docs/PRODUCT_SPECIFICATION.md`. Authoritative acceptance: `docs/ACCEPTANCE_R1.md` (Amendment 1). Owner: `pm.manager`. State: sponsor Go-Ahead 2026-10-10 16:56Z; Phase 0 recorded (`dendro-r1-p0`); acceptance frozen.

## Positioning

Dendro is an **AI Fleet Intelligence** product: an operational intelligence and governance engine for agent fleets. It is not a monitoring, APM or "telemetry" product, and that word is not used in positioning, docs, marketing or interfaces (sponsor directive, 2026-10-10 17:58Z).

## Job Statement

When I operate an agent team (rig) that has worked for days,
I want to see where time, tokens and rework actually went, by seat, handoff and mission,
so I can change the topology, the handoffs or the budget with evidence, not intuition.

## User

Primary: operator of a multi-agent OpenRig rig (sponsor). Secondary: contributor (G3). Buyer (G4): not validated.

## Observable outcome (G1)

- O1: first answer within 5 minutes, from existing local logs.
- O2–O4: answers the seat, handoff and cost questions for a named rig and period.
- O5: every number leads to its source record.
- O6: re-running does not duplicate totals.
- O7: no raw prompts or secrets stored.
- O8: the sponsor makes a real decision on `agentflow-dev` (and `dendro-dev` once it has real work) using M1 seat yield, M2 handoff efficiency, M3 build rework and M4 skill exposure.

## Scope

| Goal | State |
| --- | --- |
| G1 Local engine (OpenRig, Claude Code, Codex, Pi; then AGY, Grok) | R1 — proceed |
| G2 Natural language (Nao), dashboards, alerts | Next. Non-normative for R1. |
| G3 Extensibility | Later. The three documentation sets: parallel, Antigravity |
| G4 Site, brand | Parallel, Antigravity (outside this squad) |

## Anti-goals

Not SaaS or cloud APM. Does not control agents. Does not judge code quality. Does not guess links. The site does not show invented numbers as real.

## Release intent

`v0.1.0`, impact `minor`, local build, no publication. Path: `dendro-dev` Phase 4 → Phase 5 QA → Phase 6 four-eyes review → PR #11 merge. No bypass commits to `main`. English for interfaces, docs, code and tickets.

## PM decision

Proceed. Before freeze (17:05Z): proceed with `docs/ACCEPTANCE_R1.md`. Pi required, reference rig `agentflow-dev`, M1 = seat yield, M4 = skill exposure only. Sponsor Option A (17:45Z): Amendment 1 is authoritative. See §12 of the specification.
