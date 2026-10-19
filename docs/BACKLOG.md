# Dendro — Product Backlog & Release Hierarchy

This document serves as the authoritative, human-readable index of **Dendro's Multi-Goal Product Backlog**, maintained autonomously under the **AgentFlow SDLC** governance framework.

## Backlog Fulfillment — 2026-10-19 (delivered on `main`)

All Goal 1–Goal 5 backlog slices, post-checkpoint hardening (#59), and Rust 1.98 strict Clippy validation (#66) are delivered on `main` via [PR #56](https://github.com/smota/dendro/pull/56), [PR #57](https://github.com/smota/dendro/pull/57), [PR #58](https://github.com/smota/dendro/pull/58), and [PR #68](https://github.com/smota/dendro/pull/68). All 20 backlog issues and parent epics (#7–#10, #14–#21, #24–#29, #59, #66) are closed with verified receipts.

Verification summary: all four required Rust gates pass (`cargo check/test/clippy --workspace --locked`, Clippy with `-D warnings`, and `cargo fmt --check`). Workspace tests: **379 passed, 0 failed, 1 explicitly ignored real-log corpus test** across 22 suites. Site tests: **14 total (13 passed, 1 optional fixture skipped)**. The 5,500-fact benchmark, bounded large-payload regression, portfolio tests (including dangling-symlink `hold_dir` refusal), routing pipeline regressions, legacy semantic sidecar compatibility, failed-export partial file cleanup, terminal UI escape/C1 hygiene, and `dendro simulate topology` CLI + library tests are included. Independent read-only Claude reviews approved both the PR #58 checkpoint (`a49188b` / `c2249ba`) and the PR #68 completion wave (`04cad5b`). Local exact commands/logs: `.agent-runs/backlog-completion/checkpoint/`.

| Stream | Delivered content | Verification & truthful scope boundary |
| --- | --- | --- |
| #19–21 connectors / epic #8 | SDK example, four bounded verified-export adapters, staged/sealed replay; Claude approved connector fixes | Integrated review approved; native private stores/cloud webhooks explicitly unsupported |
| #26/#15/#25 advisory/governance | Strict parent-backed idle intervals, metric-specific local/opt-in publish drafts, CI assertion exits | Claude approved corrections and integrated source |
| #29 spotlights | A1–A6, explicit builder roster, scoped routing-only seat-window producer, portfolio supervisory evidence | Claude approved A5 correction; no per-turn causation claim |
| #27 portfolio | Independent DB aggregation, source-local mapping, unknown coverage, B1–B4 hold evidence | Integrated review approved; no live hold/residence or atomic cross-DB snapshot claim |
| #16–18 / epic #7 | Canonical views, bounded local English intents, safe native Nao workspace; typed materializer | Claude approved semantic/storage source; organic Nao operator/provider roundtrip not performed |
| #14/#28 terminal/export | Human formats, ANSI distributions, keyboard explorer, offline aliased HTML with actual classified intervals | Integrated review approved; aggregate counts are not temporal or token-transfer evidence |
| #24 / epic #10 | Pure Rust historical simulator; `dendro simulate topology` CLI wired (#68); focused + CLI tests pass | Observational what-if assumptions only; no causal prediction or live rig mutation claim |
| #59 / #66 hardening | Legacy semantic sidecar signature, failed-export partial file cleanup, TUI escape/C1 hygiene, dangling-symlink `hold_dir` refusal, Rust 1.98 strict Clippy (#68) | Implemented, verified by focused regression tests + strict workspace gates, and Claude-approved |
| Documentation / epic #9 | Truthful commands, calculator assumptions, historical media labels | Delivered (#22, #23, #28) |

A 5,500-fact integrated run exposed ~53GB JSON materialization growth. Its test alone was stopped, and its ~10GB temporary spill was removed while logs were retained; the synthetic DB is disposable and its regression fixture is committed. The replacement uses typed appenders, a 1GB refresh cap and a 5,500-fact + 16MB extension regression under 512MB. Do not reuse the failed run as passing performance evidence. Warm query timings and cold setup are separate; there is no sub-millisecond claim.

For future reference, local `AUTHORITY.md`, `DECISIONS.md`, `PROGRESS.md`, `PROGRAM.json` and `checkpoint/STATE.json` live under `/home/sam/code/dendro/.agent-runs/backlog-completion/`. Do not manufacture missing operator/source evidence.

Clean delivery worktrees may be removed after merge. Source history is recoverable from local `merge-review/delivery-history.bundle`; ignored proof documents are archived under `merge-review/archived-proofs/`, with manifests beside them. Historical dirty/unmerged worktrees and the primary clone's owner-modified `AGENTS.md` are preserved (SHA256 `9115f41d1970fd48c613a08be54d82b433195ec06ce8405b608340200dc7993e`). Never force-remove historical worktrees. Serialize Cargo, use shared target `/home/sam/code/dendro/target`, and touch the selected worktree's local Rust sources before switching builds: a cross-worktree freshness collision was observed. No tests are an organic operator usability study.

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
* **Status:** Delivered on `main` (Milestone v0.2.0; organic live Nao operator/provider roundtrip unfabricated)
* **Objective:** Enable plain English querying of fleet dynamics over local DuckDB without rigid dashboards.

| Issue | Title | Capabilities | Status |
| :--- | :--- | :--- | :--- |
| **[#7](https://github.com/smota/dendro/issues/7)** | **Goal 2 (Epic)** | Conversational & Semantic Analytics Architecture | Delivered (local semantic views, intents, Nao workspace init) |
| **[#12](https://github.com/smota/dendro/issues/12)** | `feat(analytics): Rework Vortex Detection` | Automatic circular feedback loop detection & friction heatmaps | Delivered (#56) |
| **[#14](https://github.com/smota/dendro/issues/14)** | `feat(tui): Interactive Seat Ribbon Timeline` | Terminal explorer (`dendro ui`, `report --viz`) over canonical facts | Delivered (#58) |
| **[#16](https://github.com/smota/dendro/issues/16)** | `feat(nao): 4 Semantic Materialized Views` | `v_seat_idle_ranking`, `v_handoff_rework_analysis`, `v_mission_token_burn`, `v_executive_governance` | Delivered (#58) |
| **[#17](https://github.com/smota/dendro/issues/17)** | `feat(nao): 6 Interrogation Patterns` | Compare, Rank, Explain, Verify, Trace, HonestLimits (`dendro ask`) | Delivered (#58) |
| **[#18](https://github.com/smota/dendro/issues/18)** | `test(nao): Canonical View Benchmark & Conservation` | Typed materializer, 5,500-fact + 16MB extension regression under 512MB | Delivered (#58) |
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
* **Status:** Delivered on `main` (verified portable export formats; private native stores/cloud webhooks explicitly unsupported)
* **Objective:** Expand the `HarnessConnector` modular architecture beyond Claude/Codex/Pi to modern editor and autonomous harnesses.

| Issue | Title | Capabilities | Status |
| :--- | :--- | :--- | :--- |
| **[#8](https://github.com/smota/dendro/issues/8)** | **Goal 3 (Epic)** | Harness Connectors & Extensibility Specification | Delivered (#58) |
| **[#19](https://github.com/smota/dendro/issues/19)** | `feat(connectors): Cursor & Windsurf` | Portable verified-export session log connectors | Delivered (#58) |
| **[#20](https://github.com/smota/dendro/issues/20)** | `feat(connectors): Devin & Goose` | Portable verified-export autonomous runtime stream readers | Delivered (#58) |
| **[#21](https://github.com/smota/dendro/issues/21)** | `docs(connectors): Custom Harness SDK` | Tutorial & runnable example for in-house harness integration | Delivered (#58) |
| **[#27](https://github.com/smota/dendro/issues/27)** | `feat(analytics): Cross-Project Fleet Portfolio View` | Multi-DB aggregation (`dendro portfolio`), B1–B4 hold evidence | Delivered (#58) |

---

## 🌐 Goal 4 (Epic #9): Public Presence, Branding & Waste Calculator
* **Status:** Delivered on `main`
* **Objective:** Deliver market positioning, high-conversion landing page, and economic waste calculator.

| Issue | Title | Capabilities | Status |
| :--- | :--- | :--- | :--- |
| **[#9](https://github.com/smota/dendro/issues/9)** | **Goal 4 (Epic)** | Public Presence, Branding & Commercial Materials | Delivered (#57, #58) |
| **[#22](https://github.com/smota/dendro/issues/22)** | `feat(site): Interactive 3-Layer Visualizer` | Interactive Sankey and timeline explorer on landing page | Delivered (#57) |
| **[#23](https://github.com/smota/dendro/issues/23)** | `feat(calc): Dynamic Cost-of-Inaction Calc`| Multi-harness empirical waste estimator with financial sliders | Delivered (#57) |

---

## ⚡ Goal 5 (Epic #10): Optimization Scenarios & Autonomous Fleet Tuning
* **Status:** Delivered on `main`
* **Objective:** Turn Dendro diagnoses into evidence-based optimization advice. Dendro observes and advises; it never mutates agent execution (anti-goal, sponsor 2026-10-10 18:56Z).

| Issue | Title | Capabilities | Status |
| :--- | :--- | :--- | :--- |
| **[#10](https://github.com/smota/dendro/issues/10)** | **Goal 5 (Epic)** | Autonomous Fleet Tuning & Closed-Loop Governance | Delivered |
| **[#15](https://github.com/smota/dendro/issues/15)** | `feat(governance): Closed-Loop Backlog Feeder`| `dendro issue-draft` with quantified dollar cost-of-inaction | Delivered (#58) |
| **[#24](https://github.com/smota/dendro/issues/24)** | `feat(tuning): Topology What-If Simulator` | `dendro simulate topology` analytical modeling over canonical facts | Delivered |
| **[#25](https://github.com/smota/dendro/issues/25)** | `feat(governance): Automated CI Quality Gate`| `dendro assert` breaking builds on rework spikes (>15%) | Delivered (#58) |
| **[#26](https://github.com/smota/dendro/issues/26)** | `feat(advisory): Idle-with-Claimed-Work Detection` | Report-only anomaly finding; intervention belongs to the OpenRig watchdog (sponsor 22:54Z) | Delivered (#58) |
| **[#28](https://github.com/smota/dendro/issues/28)** | `feat(viz): Fleet Flamegraphs & Token Flux Sankey` | Offline aliased HTML (`dendro export`) and terminal charts (`--viz`) | Delivered (#58) |
| **[#29](https://github.com/smota/dendro/issues/29)** | `feat(ux): Hypothesis Engine & Anomaly Spotlight` | Advisory anomaly spotlights A1–A6 (`report spotlights`) | Delivered (#58) |
