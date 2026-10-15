# Dendro — Product Backlog & Release Hierarchy

This document serves as the authoritative, human-readable index of **Dendro's Multi-Goal Product Backlog**, maintained autonomously under the **AgentFlow SDLC** governance framework.

---

## 🗺️ Multi-Goal Product Roadmap Overview

```
 ┌────────────────────────────────────────────────────────────────────────┐
 │ GOAL 1 (v0.1.0): Local AI Fleet Intelligence & Decision Engine (Rust)  │
 │ Status: RELEASED as v0.1.1 (2026-10-12, tag v0.1.1)                    │
 └──────────────────────────────────┬─────────────────────────────────────┘
                                    │
       ┌────────────────────────────┼────────────────────────────┐
       ▼                            ▼                            ▼
 ┌───────────────┐          ┌───────────────┐          ┌───────────────┐
 │ GOAL 2 (v0.2) │          │ GOAL 3 (v0.3) │          │ GOAL 4 (v0.4) │
 │ Conversational│          │ Ecosystem     │          │ Public Presence│
 │ Analytics Nao │          │ Connectors    │          │ & Waste Calc  │
 └───────┬───────┘          └───────┬───────┘          └───────┬───────┘
         │                          │                          │
         └──────────────────────────┼──────────────────────────┘
                                    ▼
       ┌─────────────────────────────────────────────────────────┐
       │ GOAL 5 (v0.5): Optimization & Autonomous Fleet Tuning   │
       └─────────────────────────────────────────────────────────┘
```

---

## 🌲 Goal 1 (Epic #1): Local AI Fleet Intelligence & Decision Engine (Rust + DuckDB)
* **Status:** Released as **v0.1.1** (tag `v0.1.1`, merge `a0671a9`, 2026-10-12). v0.1.0 (PR #11) is untagged.
* **Contract:** `docs/ACCEPTANCE_R1.md` (sha256 `cc365c88`), register K1–K11 (`44a3032e`). Release notes: `docs/RELEASE_NOTES_R1.md`.

| Issue | Title | Capabilities | Status |
| :--- | :--- | :--- | :--- |
| **[#1](https://github.com/smota/dendro/issues/1)** | **Goal 1 (Epic)** | Core Engine Specification & Multi-Crate Architecture | Delivered (v0.1.1) |
| **[#2](https://github.com/smota/dendro/issues/2)** | `feat(core): Source Discovery and Linking` | OpenRig, Claude, Codex, Pi zero-copy stream readers | Delivered (v0.1.1) |
| **[#3](https://github.com/smota/dendro/issues/3)** | `feat(analytics): Question 1 — Seat Dynamics` | 5 disjoint activity bands (active, idle-with-work, unknown) | Delivered (v0.1.1) |
| **[#4](https://github.com/smota/dendro/issues/4)** | `feat(analytics): Question 2 — Handover Flow` | Handoff counting, returns, intake cost window | Delivered (v0.1.1) |
| **[#5](https://github.com/smota/dendro/issues/5)** | `feat(analytics): Question 3 — Token Burn` | Token attribution by mission/seat, tool loop detection | Delivered (v0.1.1) |
| **[#6](https://github.com/smota/dendro/issues/6)** | `feat(storage): DuckDB Engine & Privacy` | Local storage, key-salt hashing, symlink guards | Delivered (v0.1.1) |
| **[#13](https://github.com/smota/dendro/issues/13)** | `feat(engine): Live-Producer Snapshotting` | Atomic byte-cutoff manifests for live producer runs | Delivered (main; publication follows release policy) |

---

## 💬 Goal 2 (Epic #7): Conversational & Semantic Analytics with Nao
* **Status:** Backlog / Milestone v0.2.0
* **Objective:** Enable plain English querying of fleet dynamics over local DuckDB without rigid dashboards.

| Issue | Title | Capabilities | Status |
| :--- | :--- | :--- | :--- |
| **[#7](https://github.com/smota/dendro/issues/7)** | **Goal 2 (Epic)** | Conversational & Semantic Analytics Architecture | Backlog |
| **[#12](https://github.com/smota/dendro/issues/12)** | `feat(analytics): Rework Vortex Detection` | Automatic circular feedback loop detection & friction heatmaps | In implementation — direct candidate; independent QA/review pending |
| **[#14](https://github.com/smota/dendro/issues/14)** | `feat(tui): Interactive Seat Ribbon Timeline` | Ratatui TUI with stacked ribbons & curiosity breadcrumbs | Backlog |
| **[#16](https://github.com/smota/dendro/issues/16)** | `feat(nao): 4 Semantic Materialized Views` | `v_seat_idle`, `v_handoff_rework`, `v_token_burn`, `v_governance` | Backlog (Benchmarked <20ms prototype) |
| **[#17](https://github.com/smota/dendro/issues/17)** | `feat(nao): 6 Interrogation Patterns` | Compare, Rank, Explain, Verify, Trace, Limit patterns | Backlog |
| **[#18](https://github.com/smota/dendro/issues/18)** | `test(nao): Sub-Millisecond Benchmark Suite`| Automated regression assertion gate for query latency | Backlog (Prototype suite ready) |
| **[#30](https://github.com/smota/dendro/issues/30)** | `feat(ux): Busy-source retry hint` | Busy-source refusal suggests a retry | Delivered (main; publication follows release policy) |
| **[#31](https://github.com/smota/dendro/issues/31)** | `fix(ingest): One OpenRig root per DB guard` | Refuse a second OpenRig root on an existing analytics DB (K9 limit d) | Delivered (main; publication follows release policy) |
| **[#32](https://github.com/smota/dendro/issues/32)** | `feat(trace): File source gone/superseded` | Trace marks deleted or replaced file sources (v0.1.0 known limit F3) | Delivered (main; publication follows release policy) |
| **[#33](https://github.com/smota/dendro/issues/33)** | `feat(trace): Tool-loop provenance` | Loop counts traceable to source records under K11 (known limit F5) | Delivered (#51) |
| **[#34](https://github.com/smota/dendro/issues/34)** | `fix(report): Sub-second wait precision` | Exact wait samples and medians (known limit O1) | Delivered (#45) |
| **[#52](https://github.com/smota/dendro/issues/52)** | `feat(analytics): Ceremony vs. engineering tool categories` | Pre-redaction enum classification (`coord_queue`, `coord_status`, `governance_phase`, `eng_*`) | Delivered (main; publication follows release policy) |
| **[#53](https://github.com/smota/dendro/issues/53)** | `feat(analytics): Fleet concurrency & container-vs-leaf claims` | $k$-seat concurrency histogram, effective parallelism, and `unparked_container_secs` | Delivered (main; publication follows release policy) |
| **[#54](https://github.com/smota/dendro/issues/54)** | `feat(analytics): Mid-phase compaction & cross-slice cache bleed` | Auto-compaction hazard detection (`>50%` drop while `in-progress`) and carryover cache tax | Delivered (main; publication follows release policy) |
| **[#55](https://github.com/smota/dendro/issues/55)** | `feat(analytics): Recovery items, ring boundaries & SKU/effort` | Separate `qitem-recovery-*`, `claimed->done` turnaround, `in_ring`/`out_of_ring`, and `(model, effort)` | Delivered (main; publication follows release policy) |

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
| **[#27](https://github.com/smota/dendro/issues/27)** | `feat(analytics): Cross-Project Fleet Portfolio View` | Several OpenRig homes in one view; per-project burn, throughput, rework | Needs discovery |

---

## 🌐 Goal 4 (Epic #9): Public Presence, Branding & Waste Calculator
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
* **Objective:** Turn Dendro diagnoses into evidence-based optimization advice. Dendro observes and advises; it never mutates agent execution (anti-goal, sponsor 2026-10-10 18:56Z).

| Issue | Title | Capabilities | Status |
| :--- | :--- | :--- | :--- |
| **[#10](https://github.com/smota/dendro/issues/10)** | **Goal 5 (Epic)** | Autonomous Fleet Tuning & Closed-Loop Governance | Roadmap |
| **[#15](https://github.com/smota/dendro/issues/15)** | `feat(governance): Closed-Loop Backlog Feeder`| `dendro issue-draft` with quantified dollar cost-of-inaction | Backlog |
| **[#24](https://github.com/smota/dendro/issues/24)** | `feat(tuning): Topology What-If Simulator` | Analytical modeling of seat addition/removal impact | Backlog |
| **[#25](https://github.com/smota/dendro/issues/25)** | `feat(governance): Automated CI Quality Gate`| `dendro assert` breaking builds on rework spikes (>15%) | Backlog |
| **[#26](https://github.com/smota/dendro/issues/26)** | `feat(advisory): Idle-with-Claimed-Work Detection` | Report-only anomaly finding; intervention belongs to the OpenRig watchdog (sponsor 22:54Z) | Backlog |
| **[#28](https://github.com/smota/dendro/issues/28)** | `feat(viz): Fleet Flamegraphs & Token Flux Sankey` | Offline HTML/SVG and terminal charts; waits for the G2 interface decision | Needs discovery |
| **[#29](https://github.com/smota/dendro/issues/29)** | `feat(ux): Hypothesis Engine & Anomaly Spotlight` | Advisory anomaly umbrella; reuses #12, #17, #26 definitions; badges removed | Needs discovery |
