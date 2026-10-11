# Dendro — Product Backlog & Release Hierarchy

This document serves as the authoritative, human-readable index of **Dendro's Multi-Goal Product Backlog**, maintained autonomously under the **AgentFlow SDLC** governance framework.

---

## 🗺️ Multi-Goal Product Roadmap Overview

```
 ┌────────────────────────────────────────────────────────────────────────┐
 │ GOAL 1 (v0.1.0): Local AI Fleet Intelligence & Decision Engine (Rust)  │
 │ Status: Phase 4 Implementation in Progress (PR #11)                    │
 └──────────────────────────────────┬─────────────────────────────────────┘
                                    │
       ┌────────────────────────────┼────────────────────────────┐
       ▼                            ▼                            ▼
 ┌───────────────┐          ┌───────────────┐          ┌───────────────┐
 │ GOAL 2 (v0.2) │          │ GOAL 3 (v0.3) │          │ GOAL 4 (v0.4) │
 │ Conversational│          │ Ecosystem     │          │ Public Presence│
 │ Analytics Nao │          │ Connectors    │          │ & ROI Calc    │
 └───────┬───────┘          └───────┬───────┘          └───────┬───────┘
         │                          │                          │
         └──────────────────────────┼──────────────────────────┘
                                    ▼
       ┌─────────────────────────────────────────────────────────┐
       │ GOAL 5 (v1.0): Optimization & Autonomous Fleet Tuning   │
       └─────────────────────────────────────────────────────────┘
```

---

## 🌲 Goal 1 (Epic #1): Local AI Fleet Intelligence & Decision Engine (Rust + DuckDB)
* **Status:** In Delivery — [Pull Request #11](https://github.com/smota/dendro/pull/11) (Branch `work/dendro-r1`)
* **Contract:** `docs/ACCEPTANCE_R1.md` (Amendment 1, SHA256 `046c1722`)

| Issue | Title | Capabilities | Status |
| :--- | :--- | :--- | :--- |
| **[#1](https://github.com/smota/dendro/issues/1)** | **Goal 1 (Epic)** | Core Engine Specification & Multi-Crate Architecture | In Delivery ([PR #11](https://github.com/smota/dendro/pull/11)) |
| **[#2](https://github.com/smota/dendro/issues/2)** | `feat(core): Source Discovery and Linking` | OpenRig, Claude, Codex, Pi zero-copy stream readers | In Delivery ([PR #11](https://github.com/smota/dendro/pull/11)) |
| **[#3](https://github.com/smota/dendro/issues/3)** | `feat(analytics): Question 1 — Seat Dynamics` | 5 disjoint activity bands (active, idle-with-work, unknown) | In Delivery ([PR #11](https://github.com/smota/dendro/pull/11)) |
| **[#4](https://github.com/smota/dendro/issues/4)** | `feat(analytics): Question 2 — Handover Flow` | Handoff counting, returns, intake cost window | In Delivery ([PR #11](https://github.com/smota/dendro/pull/11)) |
| **[#5](https://github.com/smota/dendro/issues/5)** | `feat(analytics): Question 3 — Token Burn` | Token attribution by mission/seat, tool loop detection | In Delivery ([PR #11](https://github.com/smota/dendro/pull/11)) |
| **[#6](https://github.com/smota/dendro/issues/6)** | `feat(storage): DuckDB Engine & Privacy` | Local storage, key-salt hashing, symlink guards | In Delivery ([PR #11](https://github.com/smota/dendro/pull/11)) |
| **[#13](https://github.com/smota/dendro/issues/13)** | `feat(engine): Live-Producer Snapshotting` | Atomic byte-cutoff manifests for live producer runs | Milestone v0.2.0 / Sealed Replay |

---

## 💬 Goal 2 (Epic #7): Conversational & Semantic Analytics with Nao
* **Status:** Backlog / Milestone v0.2.0
* **Objective:** Enable plain English querying of fleet dynamics over local DuckDB without rigid dashboards.

| Issue | Title | Capabilities | Status |
| :--- | :--- | :--- | :--- |
| **[#7](https://github.com/smota/dendro/issues/7)** | **Goal 2 (Epic)** | Conversational & Semantic Analytics Architecture | Backlog |
| **[#12](https://github.com/smota/dendro/issues/12)** | `feat(analytics): Rework Vortex Detection` | Automatic circular feedback loop detection & friction heatmaps | Backlog |
| **[#14](https://github.com/smota/dendro/issues/14)** | `feat(tui): Interactive Seat Ribbon Timeline` | Ratatui TUI with stacked ribbons & curiosity breadcrumbs | Backlog |
| **[#16](https://github.com/smota/dendro/issues/16)** | `feat(nao): 4 Semantic Materialized Views` | `v_seat_idle`, `v_handoff_rework`, `v_token_burn`, `v_governance` | Backlog (Benchmarked <20ms prototype) |
| **[#17](https://github.com/smota/dendro/issues/17)** | `feat(nao): 6 Interrogation Patterns` | Compare, Rank, Explain, Verify, Trace, Limit patterns | Backlog |
| **[#18](https://github.com/smota/dendro/issues/18)** | `test(nao): Sub-Millisecond Benchmark Suite`| Automated regression assertion gate for query latency | Backlog (Prototype suite ready) |

---

## 🔌 Goal 3 (Epic #8): Extensibility, Harness Connectors & Ecosystem Guides
* **Status:** Backlog
* **Objective:** Expand the `HarnessConnector` modular architecture beyond Claude/Codex/Pi to modern editor and autonomous harnesses.

| Issue | Title | Capabilities | Status |
| :--- | :--- | :--- | :--- |
| **[#8](https://github.com/smota/dendro/issues/8)** | **Goal 3 (Epic)** | Harness Connectors & Extensibility Specification | Backlog |
| **[#19](https://github.com/smota/dendro/issues/19)** | `feat(connectors): Cursor & Windsurf` | SQLite and cascade session log connectors | Backlog |
| **[#20](https://github.com/smota/dendro/issues/20)** | `feat(connectors): Devin & Goose` | Autonomous agent runtime stream readers | Backlog |
| **[#21](https://github.com/smota/dendro/issues/21)** | `docs(connectors): Custom Harness SDK` | Tutorial & runnable example for in-house harness integration | Backlog |

---

## 🌐 Goal 4 (Epic #9): Public Presence, Branding & ROI Waste Calculator
* **Status:** Active Parallel Stream (Lead: Antigravity PO)
* **Objective:** Deliver market positioning, high-conversion landing page, and economic waste calculator.

| Issue | Title | Capabilities | Status |
| :--- | :--- | :--- | :--- |
| **[#9](https://github.com/smota/dendro/issues/9)** | **Goal 4 (Epic)** | Public Presence, Branding & Commercial Materials | In Progress |
| **[#22](https://github.com/smota/dendro/issues/22)** | `feat(site): Interactive 3-Layer Visualizer` | Interactive Sankey and timeline explorer on landing page | Backlog |
| **[#23](https://github.com/smota/dendro/issues/23)** | `feat(calc): Dynamic Cost-of-Inaction Calc`| Multi-harness empirical waste estimator with financial sliders | Implemented (v1 on site) |

---

## ⚡ Goal 5 (Epic #10): Optimization Scenarios & Autonomous Fleet Tuning
* **Status:** Strategic Roadmap
* **Objective:** Transition Dendro from diagnostic observability into a closed-loop tuning and self-healing engine.

| Issue | Title | Capabilities | Status |
| :--- | :--- | :--- | :--- |
| **[#10](https://github.com/smota/dendro/issues/10)** | **Goal 5 (Epic)** | Autonomous Fleet Tuning & Closed-Loop Governance | Roadmap |
| **[#15](https://github.com/smota/dendro/issues/15)** | `feat(governance): Closed-Loop Backlog Feeder`| `dendro issue-draft` with quantified dollar cost-of-inaction | Backlog |
| **[#24](https://github.com/smota/dendro/issues/24)** | `feat(tuning): Topology What-If Simulator` | Analytical modeling of seat addition/removal impact | Backlog |
| **[#25](https://github.com/smota/dendro/issues/25)** | `feat(governance): Automated CI Quality Gate`| `dendro assert` breaking builds on rework spikes (>15%) | Backlog |
| **[#26](https://github.com/smota/dendro/issues/26)** | `feat(self-healing): Watchdog Nudge Reclaim` | Autonomous unclaim and recovery for stalled queue items | Backlog |
