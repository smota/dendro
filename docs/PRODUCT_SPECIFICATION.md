# Dendro — Product Specification (Phase 0)

| Field | Value |
| --- | --- |
| Owner | `pm.manager` (`pm-manager@dendro-pm`) |
| Sponsor | Sam |
| Date | 2026-10-10 |
| State | Sponsor Go-Ahead 2026-10-10 16:56Z. Phase 0 recorded (`dendro-r1-p0`). Acceptance frozen: `docs/ACCEPTANCE_R1.md` Amendment 1. |
| Sources | `docs/INTAKE_BRIEFING.md`, `docs/research/openrig-discussion-1099.md`, local data |
| Next owner | `pm.analyst` (Phase 1), then `dendro-dev` (Phase 2+) |

`docs/ACCEPTANCE_R1.md` (Amendment 1) is the authoritative definition of R1 metrics and acceptance. Where this document and that one differ on metric meaning, that one wins.

---

## 0. Positioning

**Dendro is AI Fleet Intelligence: an operational intelligence and governance engine for agent fleets.**

- Approved framings: "AI Fleet Intelligence", "Operational Intelligence & Governance Engine", "Agentic Fleet Analytics & Decision Platform".
- Banned: the word "telemetry" in positioning, docs, marketing and interfaces. Dendro is not a monitoring or APM product.
- Why this fits the job: the operator does not want more signals. The operator wants a decision about topology, handoffs and budget (§2, O8). The product is judged by decisions, not by data collected.

Source: sponsor directive, 2026-10-10 17:58Z.

---

## 1. Conclusion

**Recommendation: proceed with the local engine (Goal 1).**

Reasons:

1. **A real, urgent job exists.** The operator of a multi-agent rig cannot say whether the topology was worth its cost. The data exists. The answer does not.
2. **The briefing combines three different jobs.** Engine (operate), documentation (adopt and extend), site (convince a buyer). Only the first has a present user and a measured pain today.
3. **Five workstreams do not fit in "tomorrow".** An honest scope for tomorrow is: the operator answers the three questions for a real rig, from the data on this machine.
4. **The site depends on the engine.** Without real numbers, a landing page only promises. With Goal 1 done, the site shows proof.

### Sponsor decision (2026-10-10 16:56Z)

The sponsor granted the Go-Ahead for Release 1 (`v0.1.0`) and adjusted the recommendation:

1. **Engine:** OpenRig, Claude Code and Codex required. AGY, Pi and Grok next, in the same release. (Pi became required: see §12.)
2. **Site, brand and documentation:** approved in parallel. Antigravity owns them, outside this squad. The product squad does not define that scope.
3. **Value test (O8):** executive decision metrics on `dendro-dev` and `agentflow-dev`: seat ROI, handoff efficiency, QA→Build rework and skill impact. (Refined in §12.)

PM decision recorded: **proceed**.

---

## 2. Job Statement

> **When** I operate an agent team (rig) that has worked for days,
> **I want to** see where time, tokens and rework actually went, by seat, handoff and mission,
> **so I can** change the topology, the handoffs or the budget with evidence, not intuition.

### Related jobs (outside Goal 1)

| Job | User | State |
| --- | --- | --- |
| When I adopt Dendro, I want answers in 5 minutes, so I can decide whether it stays. | New operator | Part of Goal 1 (first run) |
| When I use an unsupported harness, I want to plug in a connector, so my data gets in. | Contributor | Goal 3 |
| When I evaluate tools for my AI team, I want proof of value, so I can justify adoption. | CTO / Head of AI | Goal 4. Not validated. |

---

## 3. Struggling moment

The operator finishes a mission. The mission was slow or expensive. The operator does not know why.

Evidence on this machine (2026-10-10):

| Source | Volume | What is missing |
| --- | --- | --- |
| `~/.openrig/openrig.sqlite` | 6 rigs, 28 nodes, 208 queue items, 702 transitions, 4,169 usage samples, 133 watchdog events | No view joins transitions with cost |
| `~/.claude/projects` | 44 MB | Tokens per session, no seat or mission |
| `~/.codex/sessions` | 29 MB | Different format, no link to the rig |
| `~/.gemini/antigravity-cli/brain` | 105 MB | Different format, no link to the rig |
| `~/.pi`, `~/.grok` | < 1 MB | Different format |

```
 slow/expensive mission
        │
        ▼
 "Was it the prompt? A loop? A bad handoff? A useless seat?"
        │
        ├── rig capture / transcript  → one seat at a time, no cost
        ├── harness logs              → cost without seat or mission
        ├── openrig.sqlite            → queues without cost, manual SQL
        └── cloud APM                 → API latency, no concept of a seat
        │
        ▼
 decision by intuition  (or no decision)
```

### Failures of current alternatives

| Alternative | Where it fails |
| --- | --- |
| Read transcripts and `rig capture` | One seat at a time. No totals. No cost. Does not scale beyond 2–3 seats. |
| Provider usage dashboard | Totals per account. Does not separate rig, seat or mission. |
| Manual SQL on `openrig.sqlite` | No harness token data. Requires schema knowledge. Nobody repeats it. |
| Cloud APM (Langfuse, Datadog, etc.) | Requires instrumentation and sending data off the machine. Models API calls, not seats and handoffs. |
| Do nothing | The operator keeps seats that only relay text and pays for loops nobody sees. |

---

## 4. Forces of progress

```
        FOR THE CHANGE                         AGAINST THE CHANGE
 ┌──────────────────────────────┐     ┌──────────────────────────────┐
 │ PUSH                         │     │ ANXIETY                      │
 │ • cost grows unexplained     │     │ • wrong numbers lead to      │
 │ • rigs with 6+ seats         │     │   wrong decisions            │
 │ • hidden QA→Build rework     │     │ • reading transcripts        │
 │                              │     │   exposes secrets            │
 │ PULL                         │     │ • one more tool to maintain  │
 │ • local, no data leaves      │     │                              │
 │ • natural-language answers   │     │ INERTIA                      │
 │ • decide topology with       │     │ • "I read the logs when I    │
 │   evidence                   │     │   need to"                   │
 └──────────────────────────────┘     └──────────────────────────────┘
```

Product implications:

1. **Trust before breadth.** Every number must lead to its source record. A wrong number costs more than a missing one.
2. **Local and read-only.** Dendro never writes to sources. No data leaves the machine.
3. **Secrets stay out.** Prompt content does not enter the store by default. This preserves `no-raw-secrets-or-transcripts`.
4. **Zero maintenance.** One command produces the answers. Re-running reads only what changed.

### Dimensions

| Dimension | Desired progress |
| --- | --- |
| Functional | Answer the three questions for a rig and a period. |
| Emotional | Move from "I think the rig is expensive" to "I know where the cost is". |
| Social | Show the team or the buyer why the topology has its shape. |

---

## 5. Observable outcome

Goal 1 is done when this is true for the operator, on this machine, with real data:

| # | Observable change |
| --- | --- |
| O1 | From existing logs, the operator gets the first answer within 5 minutes of installation. |
| O2 | For a named rig and period, the operator sees each seat's active work and idle time. (Question 1) |
| O3 | For the same rig, the operator sees each handoff, how often work came back, and the context cost of the transfer. (Question 2) |
| O4 | For the same rig, the operator sees token consumption by mission and seat, and detected retry loops. (Question 3) |
| O5 | The operator can trace any number to its source record (file and line, or table row). |
| O6 | On re-run, only new data enters, and totals do not duplicate. |
| O7 | No prompt content and no secret is kept in the analytical store. |
| O8 | Value signal: based on Dendro, the sponsor makes at least one real topology, handoff or budget decision on `agentflow-dev` (and `dendro-dev` once it has real work), using the decision metrics defined in `docs/ACCEPTANCE_R1.md` (M1 seat yield, M2 handoff efficiency, M3 build rework, M4 skill exposure). |

O8 is the product test. O1 to O7 are conditions for O8.

---

## 6. Goals and stories

Acceptance criteria are owned by `pm.analyst` and frozen in `docs/ACCEPTANCE_R1.md`. Below are outcome-level acceptance signals only.
Architecture is owned by `orch.arch`. This section does not choose files, crates or schemas.

```
 G1 Local engine (NOW) ──► G2 Guided answers ──► G3 Extensibility
        │                                              │
        └──────────── real proof ─────────────► G4 Public presence
```

### G1 — The operator answers the three questions for a real rig (NOW, R1)

| Story | Acceptance signal |
| --- | --- |
| G1.1 As an operator, I run one command and Dendro reads the local sources it finds. | Reports which sources were found, which are missing, and how many records it read from each. |
| G1.2 As an operator, I see harness sessions linked to the right rig, seat and mission. | An unlinked session shows as "unlinked", with no guess. The link rate is shown. |
| G1.3 As an operator, I see active and idle time per seat. | Answers Question 1 for the reference rig over the period. |
| G1.4 As an operator, I see handoffs and rework. | Answers Question 2. Work returning to build counts as rework. |
| G1.5 As an operator, I see token burn and loops. | Answers Question 3. A repeated failing command shows as a loop. |
| G1.6 As an operator, I re-run and only new data enters. | A second run without changes does not alter any total. |
| G1.7 As an operator, I trust the numbers. | Each metric has a written definition and a path to its source. |
| G1.8 As an operator, I see each seat's yield. | For the reference rig, shows what each seat consumed and what it passed on that did not come back. |
| G1.9 As an operator, I see skill exposure. | Shows which skills each seat had loaded. Impact shows "insufficient data" until a valid comparison exists. |

Harnesses in G1: **OpenRig, Claude Code, Codex and Pi required.** AGY and Grok follow in the same release.

### G2 — The operator asks in natural language (NEXT, not R1)

| Story | Acceptance signal |
| --- | --- |
| G2.1 As an operator, I ask the three questions in natural language (Nao). | Answers match G1 for the same period. |
| G2.2 As an operator, I open one starter dashboard per question. | Three dashboards, one per question, with real data. |
| G2.3 As an operator, I get an alert on anomalous burn or loops. | The alert arrives before the operator notices alone. Threshold is configurable. |
| G2.4 As an operator, I keep answers current without running commands. | Continuous mode reflects new data within minutes. |

### G3 — A contributor plugs in a new harness (LATER)

| Story | Acceptance signal |
| --- | --- |
| G3.1 As a contributor, I add a connector by following a guide. | A new connector works without changing the core. |
| G3.2 As a maintainer, I understand the architecture and conventions. | A new builder ships a first change using only the documentation. |
| G3.3 As a new operator, I follow the quickstart. | An unassisted operator reaches O1 within 5 minutes. |

### G4 — A buyer understands the value (parallel, owner: Antigravity)

The sponsor approved G4 and the G3 documentation in parallel, owned by Antigravity. The stories below are reference only. They are not in this squad's Phase 1.

| Story | Acceptance signal |
| --- | --- |
| G4.1 As a CTO, I understand in 30 seconds what Dendro answers. | Validate with 3–5 people from the target audience before building. |
| G4.2 As a visitor, I see the tree-ring metaphor with real data. | Uses real G1 numbers, not invented data. |
| G4.3 Domain, logo and visual identity. | Sponsor decision. Buying a domain is an external, paid action and requires explicit sponsor approval. |

### Priority matrix

| | High value | Uncertain value |
| --- | --- | --- |
| **Feasible now** | G1, G1.7 | G2.1 Nao |
| **Depends on G1** | G2.2–G2.4, G3.3 | G4 site, brand, domain |

---

## 7. Constraints and anti-goals

### Constraints

1. Local-first. No data leaves the machine.
2. Read-only on sources (`~/.openrig`, `~/.claude`, `~/.codex`, `~/.gemini`, `~/.pi`, `~/.grok`).
3. No raw prompt content and no secrets in the store.
4. Stack decided by the sponsor: Rust, DuckDB, Nao. Not reopened.
5. Sources change format. An unknown format produces a warning, never a failure or wrong data.
6. English for interfaces, docs, code and tickets (sponsor directive, 2026-10-10 17:57Z).

### Anti-goals

1. Not a cloud APM and not SaaS.
2. Does not control or restart agents. It observes. It does not act.
3. Does not judge the quality of code produced by agents.
4. The site does not show invented numbers as if they were real.
5. Does not guess links. "Unlinked" is better than a wrong link.

---

## 8. Release intent

| Field | Value |
| --- | --- |
| Release | `v0.1.0` — local preview of G1 |
| Impact | `minor` (first functional increment) |
| Audience | Sponsor. Reference rig: `agentflow-dev`. `dendro-dev` once it has real work. |
| Channel | Local build from the repository. No publication, no crates.io. |
| Exit criterion | AC1–AC12 in `docs/ACCEPTANCE_R1.md` (Amendment 2, sha256 `cc365c88`, plus clarifications register K1–K8, sha256 `8f335d37`) verified with real data at one exact commit. O8 observed by the sponsor. |
| Target date | None set (2026-10-10 19:52Z). Readiness-gated: `v0.1.0` ships when the exit criterion holds. Checkpoint: PM re-plans when the Phase 4 developer proof is complete. |
| Delivery path | Squad-only: `dendro-dev` Phase 4 build → `dev-qa` Phase 5 independent validation → `rev-review` Phase 6 four-eyes audit → PR #11 merge. No bypass commits to `main`. |
| Not in v0.1.0 | Nao (G2) is non-normative for R1, even where code exists on `main`. |
| Parallel | Site, brand and the three documentation sets: Antigravity. |
| Next | `v0.2` = G2. |

---

## 9. Sponsor decisions (resolved 2026-10-10)

| # | Question | Resolution |
| --- | --- | --- |
| D1 | Scope for R1 | Engine. OpenRig, Claude Code, Codex, Pi required; AGY, Grok next. |
| D2 | Site, domain, brand | Parallel, owned by Antigravity. |
| D3/D4 | Value test | Executive decision metrics on local rigs. Defined in `docs/ACCEPTANCE_R1.md`. |
| D5–D7 | Conflicting issues and flows doc | Option A (17:45Z): Amendment 1 is authoritative. |
| D8 | Nao and delivery path | Nao is G2, non-normative for R1. All implementation flows through `dendro-dev` (17:57Z). |

---

## 10. Solution design: ownership boundaries

| Artifact | Owner | Phase |
| --- | --- | --- |
| Job, outcome, release intent | `pm.manager` | 0 (this document) |
| Testable acceptance, scope, assumptions | `pm.analyst` | 1 (`docs/ACCEPTANCE_R1.md`) |
| Architecture, analytical schemas, flows, plan | `orch.arch` (`dendro-dev`) | 2+ |
| Nao dashboards, site | Delivery after G1 / Antigravity | G2 / G4 |

`docs/architecture.md` and `docs/ARCHITECTURE_FLOWS.md` are input for `orch.arch`. Where they differ from Amendment 1 on metric meaning, Amendment 1 wins.

```
 Sam ──Go-Ahead──► pm.manager ──Phase 0──► pm.analyst ──Phase 1──► orch-arch@dendro-dev ──► build/qa/review
                       ▲                      │                         │
                       └────── return ────────┘◄──────── return ────────┘
```

---

## 11. Unknowns

Resolutions are in `docs/ACCEPTANCE_R1.md` §4 and §8.

| # | Unknown | State |
| --- | --- | --- |
| U1 | Session-to-seat linking | Resolved: OpenRig session identity events. |
| U2 | Definition of idle | Resolved by the analyst. |
| U3 | Handoff context cost | Resolved as a labelled estimate. |
| U4 | Definition of a loop | Resolved, thresholds configurable. |
| U5 | Format stability of Codex, AGY, Pi, Grok | `orch.arch`. AC2 limits damage. |
| U6 | Nao over local DuckDB | Out of R1 (G2). |
| U7 | Skill impact | Not measurable in R1. Exposure only. |
| U8 | Seat ROI without judging code quality | Redefined as seat yield (M1). |

---

## 12. Decision log

### PM decision before freeze (2026-10-10 17:05Z)

**Decision: proceed.** The sponsor approved `docs/ACCEPTANCE_R1.md` at 17:01Z. The job does not change. The adjustments below refine R1 scope and do not require a return to Phase 0.

| Acceptance item | Decision | Reason |
| --- | --- | --- |
| A1 Pi required | Accepted | On `agentflow-dev`, `orch.arch` and `rev.review` run on Pi. On `dendro-dev`, all builders run on Pi. Without Pi, O8 lacks cost for the central seats. Pi was already in R1. Only the order changes. |
| A2 Reference rig `agentflow-dev` | Accepted | `dendro-dev` has 0 queue items (verified). O8 on `dendro-dev` waits for real work. |
| A5 Rework by item chain | Accepted | There are 0 direct QA→Build handoffs. The sponsor's intent is "work that returns to build". |
| M1 "yield" instead of "ROI" | Accepted | Dendro does not measure value. The name must not promise what the metric does not measure. |
| M4 skill exposure only | Accepted | No comparison baseline exists. "Insufficient data" beats an opinion with a number. |

### Acceptance Amendment 1 (2026-10-10 17:08Z)

**Decision: refine one point, then proceed.** Delivery return, cycle 1 of 2. The job does not change.

- All of Amendment 1 is accepted, including the SQLite `-shm` exception.
- AC10: mission tags, tool names and source paths are product output. They may appear. Free text (prompts, responses, tool arguments, tool results, errors) never appears. A secret-shaped value never appears in any field and is masked.

### Sponsor decision: Option A (2026-10-10 17:45Z)

Amendment 1 (`docs/ACCEPTANCE_R1.md`, sha256 `046c1722`) is the authoritative definition of metrics and conservation rules. Issues and `ARCHITECTURE_FLOWS.md` follow it. Commits: `fa6dad0`, `3c74b65`.

### Sponsor directive (2026-10-10 17:57Z)

1. Nao is Goal 2 and non-normative for R1. `orch.arch` has authority over canonical R1 schema on `work/dendro-r1` (PR #11).
2. All product implementation flows through `dendro-dev`. No bypass commits to `main`.
3. Interfaces, docs, code and tickets are in English.
4. Sequence: `dev-build-sr` Phase 4 → `dev-qa` Phase 5 → `rev-review` Phase 6 → PR #11 merge.

### Sponsor positioning directive (2026-10-10 17:58Z)

Dendro is positioned as AI Fleet Intelligence (operational intelligence and governance), not as a telemetry or APM product. The word "telemetry" is banned in positioning, docs, marketing and interfaces. See §0.

### Sponsor disposition: R1 cycle-3 correction (2026-10-10 19:06Z)

Two phase 6→4 returns were consumed (transitions 0009, 0012). QA then reproduced an AC10 failure: a secret-shaped value in an OpenRig activity state was stored in DuckDB and emitted in the trace. Synthetic canary only.

**Approved:** one bounded third correction. (1) Activity-state allowlist; anything else maps to `unknown` before storage and trace. (2) Field-by-field AC10 regression across every text-bearing column and output surface. (3) Full independent AC1–AC12 QA, then the Phase 6 four-eyes audit. Amendment 1 unchanged. No fourth cycle without explicit sponsor approval.

### Sponsor disposition: AC10 follow-up (2026-10-10 19:10Z)

The engine enforces a hard maximum of 2 return cycles with no exception input. **Approved:** a bounded follow-up under the R1 goal (a high-assurance capability and spec, per `docs/sdlc-definition.md`), carrying the 19:06Z scope with its own return budget. The parent stays at Phase 6. The parent reviewer accepts against full AC1–AC12 after the follow-up passes. Opening the capability requires the sponsor's own `adequacy-of-intent` attestation.

### Sponsor disposition: legacy follow-up record (2026-10-10 19:18Z)

AgentFlow defect: the capability-review gate that `gates open` accepts (goalSubject) differs from the one `validateOpen` requires (revision), so the high-assurance spec cannot open. **Approved:** the AC10 fix runs as a legacy follow-up record linked to the R1 goal, the capability (`6a968e6b`) and both human attestations (`cdbb68a40b99`, `799dd427c955`). The skipped spec gate is recorded openly as an exception. QA, the four-eyes review and the final R1 acceptance are unchanged. Defect routed to `agentflow-pm` (`qitem-20261010191850-939b9f1422da9e80`), accepted as smota/agentflow-sdlc#402. The two-cycle exception is not part of that fix.

### Sponsor disposition: R1 Amendment 2 and re-scope (2026-10-10 19:44Z)

R1 candidate `63cebb0` was not releasable (`docs/R1_GAP_ANALYSIS.md`). **Approved:** C1 configurable loop thresholds → v0.2 (fixed defaults in R1); C2 M4 skill exposure → v0.2; C3 R1 CLI limited to ingest, report and trace; C4 handoff-loop detector → v0.2 as a #12 prerequisite. Carrier: new legacy follow-up `.agent-runs/follow-ups/r1-amendment2` with a fresh return budget. AC10 is absorbed into it. All other Amendment 1 criteria stay, including per-seat tokens, per-seat M1 and per-pair M2. The v0.1.0 date will be re-planned after `orch.arch` sizes the unified plan. Merging PR #11 remains the person's release decision after Phase 6.

### PM decision: clarifications register frozen for R1 QA (2026-10-10 22:00Z)

The register `docs/ACCEPTANCE_R1_CLARIFICATIONS.md` reached five entries (K1–K5, sha256 `e83d505c`). It is now the published clarifications note for R1. QA and the four-eyes review verify against contract `cc365c88` plus register `e83d505c`. No entry changes a formula or adds a criterion. A new entry after this point needs `pm.manager` acceptance and a new hash sent to `orch.arch`, `dev.qa` and `rev.review`. K5 adds one visible count to the M1 output ("closed items with insufficient history"); this is a display of excluded data, consistent with K3, not a new criterion.

Process note (21:24Z–21:29Z): two messages labelled `sponsor@dendro-pm`, an address that does not resolve, told the builder to append Phase 4 before the proof receipt and to "record zero gaps". The builder followed the first (`0007`). The handoff note did not claim zero gaps. The source is unconfirmed by the sponsor. Rule: delivery acts on sponsor direction only through `orch.arch` or `pm.manager`.

### PM decision: K6 M1 return window (2026-10-10 22:15Z)

Question from `orch.arch`: can a return after the report window end remove an in-window delivery from M1? **Decision: no.** M1 and M2 count only returns inside `[from, to)` (§4.1). The same window gives the same number whenever the report runs. Known limit: M1 is slightly optimistic for handoffs near the window end; logged for the R1 review. Returns are chain-aware by the existing §4 definition. Register re-freezes at K1–K6; new hash `33a8175c` (K1–K6) replaces `e83d505c` in §8.

### PM decision: K7 M3 seat roles (2026-10-10 22:45Z)

Builder candidate `d5af24a` classified M3 seats by parsing names. Live evidence: `nodes.role` is NULL for every seat, and `dev-review@starter` has a recorded QA spec. **Decision:** M3 checking seats come from recorded evidence (AgentFlow `dev.qa` / `rev.review`, or a recorded QA or review spec), never from name segments. `dev-build*` stays as the contract names it. A chain whose only checking pass is through a role-unknown seat is "M3 unknown" with the reason. Precision added by PM: unknown chains are excluded from the rework count and the proportion denominator, and shown as a separate count. M3 on `agentflow-dev` is unaffected. Register re-freezes at K1–K7, sha256 `a1b5a8cc` (supersedes `33a8175c`).

### Sponsor disposition: unauthorized sponsor-labelled prompts (2026-10-10 22:48Z)

The sponsor confirmed that the `sponsor@dendro-pm` prompts were **not** authorized sponsor direction. Valid sponsor direction reaches delivery only through `orch.arch` or `pm.manager`; direct prompts to builder seats are ignored and forwarded to `orch.arch`. Work continues under Amendments 1/2, register K1–K7 (`a1b5a8cc`), receipt-first, independent Phase 5 QA and Phase 6 review before merge. G2 #12/#13 Phase 1 drafting stays parked until R1 four-eyes completes.

PM evidence check (read-only, `openrig.sqlite`): the label `sponsor@dendro-pm` wrote 3 messages to `dev-build-sr` (21:24, 21:26, 21:28Z) and 15 queue transition notes on qitems `…9800e474`, `…9df9b12f` and `…1159387e`, about every 5 minutes from 20:10 to 21:28Z. The notes contain progress claims (for example "proof matrix covering all 44 text columns") that `orch.arch` later found false. Those notes are not evidence for any phase. No queue item was created by that label. Nothing has been sent under it since 21:28Z. The regular cadence suggests an automated loop; its origin is not identified.

### Sponsor decisions: backlog ownership and grooming (2026-10-10 22:51Z, 22:54Z)

- Full backlog ownership (#1–#26, all milestones) assigned to `pm.manager` and `pm.analyst`. GitHub is the single source of truth. G2 #12/#13 stay staged; the interface (#14 TUI vs #17 Nao) is chosen by evidence from the first R1 review.
- #26: Dendro reports only (advisory anomaly detection). Active intervention belongs upstream in the OpenRig watchdog. Issue retitled and rewritten.
- Milestone 5 renamed `v1.0.0` → `v0.5.0` (Goals 1–5 map to `v0.1.0`–`v0.5.0`). `v1.0.0` is reserved for the production GA release.
- #8 and #9 rewritten in English, aligned with AI Fleet Intelligence positioning; "ROI" removed from #9.
- Upstream OpenRig report on unverified sender identity: drafted for the sponsor, who dispatches it.
- Origin tracing of the 5-minute loop: not pursued (external watchdog probe, silenced).

### Sponsor disposition: Escalation on candidate c635803 and snapshot lifecycle (2026-10-11 00:32Z)

On escalation `qitem-20261010231401-3d2fae7cc70d9fb9` (`pm-manager` / `orch-arch`):
1. **Option C Approved:** Authorized dedicated bounded follow-up record for snapshot lifecycle and proof completion with a fresh 2-cycle budget.
2. **Deletion of /tmp remnants:** Deletion of the two confirmed `1bb6a82` `/tmp` snapshot directories (`/tmp/dendro-snap-18dd4b6915556e02` and `/tmp/dendro-snap-18dd4b6da94a06d7`) was approved and completed by `pm-manager`, as attribution was confirmed to builder test runs and defect reproduction is preserved in the worktree. No wildcards permitted.
3. **Implementation Scope for follow-up:**
   - Private `0700` permissions on snapshot storage directory.
   - Signal handling (`SIGINT`/`SIGTERM`) to trigger cleanup on process interruption.
   - Startup scavenger to clean stale snapshot remnants on next `dendro run` if a previous crash/`SIGKILL` occurred (removal or explicit report if removal fails).
   - AC7/K3 secondary trace evidence and mission precedence definition.
   - Decoded all-column scan.
   - Clarification K8 for missing-key read-path rejection behavior if required.
4. **Carrier & Phasing:** Carried under a dedicated bounded follow-up record. Phases 0/1 routed through `pm-analyst` and `orch-arch`. Builder (`dev-build-sr`) remains parked until Phase 3 architecture/plan is complete. Independent Phase 5 QA (`dev-qa`) and Phase 6 review (`rev-review`) strictly enforced before merge.


### Sponsor disposition: R1 snapshot lifecycle follow-up, Option C (2026-10-11 00:32Z)

Candidate `c635803` failed the developer proof after the last planning correction (`r1-amendment2/0008`): a frozen ingest stopped by error or signal left a raw, byte-identical copy of the source database in `TMPDIR/dendro-snap-*`; AC7/K3 trace content was incomplete. Two real-data copies were found in `/tmp`, created by the builder's own interrupted test runs on `1bb6a82` (SIGTERM).

**Approved (Option C):** one bounded follow-up, `.agent-runs/follow-ups/r1-snapshot-lifecycle`, fresh budget of 2 returns. Phase 0 recorded skipped (`dendro-r1-snap-p0`, transition 0001). Closed scope:

- S1 Owner-only snapshot storage (directory 0700).
- S2 Cleanup on normal exit, handled error, SIGINT and SIGTERM.
- S3 Next-run removal (or report) of remnants after SIGKILL or host crash.
- S4 AC7/K3 mission precedence in the printed definition and trace; per-record secondary evidence.
- S5 Decoded all-column canary scan on the final candidate.
- S6 Missing key (PM 23:16Z, K8): ingest refuses; report and trace may run without computing fingerprints or printing key material.

Proof command list fixed by `orch.arch` before code; receipt first; full independent QA and four-eyes review. No waivers. If this fails again, PM recommends a pause and a builder-seat review.

**Deletion approved and done (00:35Z):** exactly `/tmp/dendro-snap-18dd4b6915556e02` and `/tmp/dendro-snap-18dd4b6da94a06d7`. No other path touched. The defect reproduction stays in the builder worktree.

### Sponsor disposition: snapshot follow-up correction 1 (2026-10-11 01:55Z)

Candidate `3669b15` failed the developer proof at 100/102 lifecycle checks: on a partial cleanup failure, the ownership marker was deleted before the snapshot directory, so the next run correctly preserved and reported the unmarked private stage but could not recover it. No cross-user exposure (0700/0600, synthetic data). The receipt-first order worked: the builder reported the failure and did not pass Phase 4.

**Approved (Option 1):** one bounded correction, cycle 1 of 2 in `r1-snapshot-lifecycle`. Scope only: (1) marker-last deletion order in every cleanup path (finish, Drop, and next-run recovery cleanup); (2) a regression test proving that after a partial cleanup failure the marker is intact and the next run's scavenger removes the stage. Protocol unchanged: receipt first to `orch.arch`, written acceptance, Phase 4, independent QA (Phase 5) and review (Phase 6). If this fails, PM returns to the sponsor before the last cycle is used.

### Sponsor disposition: AC12 correction, cycle 2 of 2 (2026-10-11 02:31Z)

Independent QA (Phase 5) on `06cd1a7` / binary `9955b133` failed AC12: a ptrace observer captured the source `-wal` opened with `O_RDWR|O_CREAT` during a non-frozen ingest. Cause (`orch.arch`): the direct SQLite read-only connection to the live source (`mode=ro`, `SQLITE_OPEN_READ_ONLY`) still opens the WAL write-capable. The same direct path existed in `63cebb0` and `c635803`; past write-capable opens or writes on the live OpenRig database are not established.

**Approved (Option 1), final cycle 2 of 2.** Scope only: main DB and WAL never open `O_RDWR`/`O_CREAT`; only `-shm` excepted. `immutable=1` is forbidden on live sources; read fidelity and live transaction visibility are preserved through private staging/copy or a strictly read-only open. QA's ptrace observer becomes a mandatory proof command. Full proof rerun, written acceptance, full independent QA and review. **If this fails, R1 pauses for a full rescope.** No real-data Dendro runs on the live sources until QA passes the fix.

### Sponsor confirmation: sealed input folder (2026-10-11 02:55Z)

`/tmp/dendro-sealed` (130 MB of real source copies from an earlier builder run, previously 0755/0644) was locked by Antigravity at 02:35Z (`chmod -R go-rwx`); PM verified 700/600 at 02:40Z. PM authorized source-only use for the cycle-2 sealed oracles (manifest verified; private copy of `sources/` only; old `out/` DB and key never read). **Deletion authorized** once R1 is formally accepted and merged to main; PM deletes that exact path then.
