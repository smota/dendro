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

## In flight — #41 headroom

Decision 2026-10-14: **proceed**. Phase 0 pass: `https://github.com/smota/dendro/issues/41#issuecomment-6173744517`. Restore spare time inside O1 / AC1 (reference first run ≤ 225 s, sealed and unsealed). Numbers stay identical. Not a G2 interface. v0.2.0, local build, no publication. Land before or with #13.

Phase 8 decision 2026-10-14 17:53Z: **pause publication**. Candidate `19fc70a6fc4e2d2be97662747a32080ec913ce5d` on `work/perf-ingest-headroom` meets the amended #41 contract on one reference run (107.112 s, review pass). Integration is not delivered. No push, pull request, merge, tag, or publication. #13 F13-8 stays unpassed. Proofs stay local. Relay: requester `antigravity`.

## In flight — #31 one OpenRig root

Decision 2026-10-14: **proceed**. Close K9 (d): a second OpenRig root must be refused before any number changes. Not multi-root (#27). v0.2.0, local build, no publication. Land before or with #13 (G-A6). Commits `cb800e5` and `0eeb09f` on main are unaccepted candidates. The v0.1.1 tag does not contain them.

Phase 8 decision 2026-10-14 18:32Z: **pause publication**. Candidate `50bca08c5cecc7fffd5fb4a4e2bb3c0ea85dd8dd` on `fix/one-openrig-root-hint` meets the frozen Phase 1 contract (R1–R6). The bound-root refusal names both roots and says to use a fresh analytics database. Release notes no longer call that guard part of v0.1.1. Zero source opens stay unknown. Integration is not delivered. No push, pull request, merge, tag, or publication. #13 and #27 are not accepted by this pass.

## In flight — #40 Codex command outcomes

Decision 2026-10-14: **refine**. Codex seat calls stay not read. Do not guess a join. Count failures only from a CommandExecution item's own integer exit code, and label that unit as a command execution, not a tool call. The loops report shows `read` or `not read` per harness. No raw command, prompt, or secret stored. v0.2.0, local build, no publication. Not a #13 gate.

Phase 8 decision 2026-10-14: **accept local result; pause integration and publication**. Candidate `6a9eecc6fdadff5629accc7fa09f0d208a43f340` on `feature/codex-command-outcomes` meets Amendment 1 through independent QA and review (69 frozen cases; 132 tests passed, one ignored). Reference first run 66.454055 s meets existing AC1 on the pinned corpus. Native arrays only; older-reader/new-database semantics unsupported. Oracle correction is intake 23 → 20 only, with original bytes preserved and independent approval. Integration remains outstanding. No push, PR, merge, tag or publication; no #13 acceptance. Full decision: `goals/codex-command-outcomes/product-acceptance.md`.

## Local result — #34 fractional waits

Phase 8 decision 2026-10-14: **accept local result; pause integration and publication**. Candidate `51998a6af30155919632b224af6cb4698a81bfb9` on `fix/fractional-handoff-waits` meets the fractional wait/median contract through independent QA/review (32 cases, 64 paired journeys; 136 tests passed, one ignored). Independent Decimal proof covers 265 samples/42 medians; all nonwait fields remain exact across 26 corpus surfaces. AC4 conservation stays unchanged. Float magnitude and finer-than-microsecond persistence limits remain; fresh-ingest performance is unproved. Integration outstanding; no push, PR, merge, tag or publication, no #12/#13 acceptance. Full decision: `goals/subsecond-wait-times/product-acceptance.md`.

## Held — #30 wording and #46 source-output alias protection

Decision 2026-10-14: **hold unqualified #30 QA/product acceptance**. Independent synthetic QA found original source WAL write-capable opens and SHM mutation when analytics output aliases the source, before refusal. Baseline parity does not waive R1 AC12/#30 source protection. Separate defect #46 proceeds to analysis; #30 wording scope stays unchanged. Architect owns assurance and isolated repair plan. Independent defect QA/review and composed-candidate reverification are prerequisites for an unqualified #30 pass. No fictional lifecycle return, #12/#13 acceptance or publication/integration authorization. Disposition: `goals/busy-source-retry-hint/source-protection-disposition.md`; new framing: `goals/source-output-alias/phase-0.md`.

#46 local product acceptance 2026-10-14 UTC: candidate `28cb41a21fd9cf801b00f7e90cedc3c115a83801` meets its static primary-output alias contract through independent QA/review (62 cases each, 50 safe refusals, zero unknowns; reviewer Cargo4, 142 passed/one ignored). Exact #30 composition remains pending; #30 HOLD stays. #47, race/default-route/platform/privacy limits and publication boundaries remain. Full acceptance: `goals/source-output-alias/product-acceptance.md`.

#30 local product acceptance 2026-10-14 UTC: exact composed `830a9209600e5437ffe29d6079421efe0eebc0e9` satisfies wording and repaired source-protection criteria through fresh independent QA/review (98 wording checks, 62 repair cases, 30 reports; Cargo4, 142 passed/one ignored). This satisfies the local acceptance hold for the composed candidate only; unsafe original `e34b2c1` and findings stay preserved/unaccepted. Retained integrity/race/default/privacy limits and #47 remain separate. Integration/publication held; no #12/#13 acceptance. Full decision: `goals/busy-source-retry-hint/product-acceptance.md`.

## Selected — #47 auxiliary output lock

2026-10-15 UTC: **proceed to synthetic investigation and conditional bounded repair** under steering intake. Runtime write-capable source access remains unverified; static ordering alone is not a candidate/runtime verdict. Use actual parent `.dendro-output.lock` path, preserve normal lock protections and prior #30/#46 candidate-bound outcomes. Phase0 recorded; analyst owns Phase1. Local v0.2.0 only, integration/publication holds unchanged. Framing: `goals/auxiliary-lock-source-alias/phase-0.md`.

#47 local acceptance2026-10-15 UTC: exact `7a7f934299712e278cfd8f17892765db6214832f` meets the bounded preexisting hard-link safe-refusal contract. Independent QA/review each pass35 auxiliary+62primary cases/0unknown,70reports+8traces and98wording checks. Verified builder Cargo/build and direct compiled tests149passed/one ignored remain method-distinct. Race/single-link-source/arbitrary auxiliary/default/platform/privacy limits remain; prior #30/#46 histories unchanged. Integration/publication held, no #12/#13 acceptance. Full decision: `goals/auxiliary-lock-source-alias/product-acceptance.md`.

## Selected — #33 loop source trace

2026-10-15 UTC: **proceed to analysis** for trace-only independent episode/count recomputation with safe source keys/generations, ordered equality/outcome/time evidence, run breaks and qualifying triple/span. Preserve K11 and accepted #40 units/anchors/coverage, loop/report meanings and AC10; no raw-input trace or guessed join. Analyst owns exact public contract. Local v0.2.0 only, existing integration/publication boundaries remain. Framing: `goals/tool-loop-source-trace/phase-0.md`.

#33 local acceptance 2026-10-15 UTC: exact `b975e1c1dd23367bcb024c2462fc667deee81ce1` meets accepted Amendment 1 for independent captured-evaluation loop replay. Distinct QA/review each ran the final-release suite (61 native cases, 12 states, 16 journeys, 35 lock/62 alias controls and 98 manual-five checks); authenticated builder Cargo4 gives 156 passed/one ignored. Effective oracle preserves base725 with explicitly approved guarded two-pointer physical-revision comparison correction; original failures remain. Native full-tie parity, producing history, source authenticity and universal privacy/spill guarantees remain excluded. Integration/publication held; no #12/#13 or prior candidate acceptance expansion. Decision: https://github.com/smota/dendro/issues/33#issuecomment-6183898818; full record: `goals/tool-loop-source-trace/product-acceptance.md`.

## Selected — #32 file-source state

2026-10-15 UTC: **proceed to analysis** for present/changed/superseded/unavailable file-source trace annotations after pruning or replacement, preserving historical metrics on the same analytics snapshot. Metadata-only observations must distinguish evidence from unknowns and cannot prove content identity or infer lineage. Analyst owns exact fields, timing, errors, detached-source compatibility and synthetic expected vectors. Phase0: https://github.com/smota/dendro/issues/32#issuecomment-6183940065. File-source scope only; #13 alignment dependency must be surfaced, no #13 acceptance. Local v0.2.0, existing integration/publication boundaries unchanged. Framing: `goals/file-source-state/phase-0.md`.
