# Dendro — Product Backlog & Release Hierarchy

This document serves as the authoritative, human-readable index of **Dendro's Multi-Goal Product Backlog**, maintained autonomously under the **AgentFlow SDLC** governance framework.

**Synced:** 2026-10-29. **Release preparation:** v0.9.1 (workspace `0.9.1`; newest git tag is `v0.9.0`). **Active backlog:** v1.0.0 / vNext (Goal 10).

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
       └────────────────────────────┬────────────────────────────┘
                                    │
                                    ▼
       ┌─────────────────────────────────────────────────────────┐
       │ GOAL 6 (v0.6.0): Operator Usability & Smart Defaults    │
       │ Status: RELEASED (2026-10-19, tag v0.6.0)               │
       └────────────────────────────┬────────────────────────────┘
                                    │
                                    ▼
       ┌─────────────────────────────────────────────────────────┐
       │ GOAL 7 (v0.7.0): 6-Screen Ratatui Cockpit               │
       │ Status: DELIVERED on main (2026-10-20, PR #73)          │
       └────────────────────────────┬────────────────────────────┘
                                    │
                                    ▼
       ┌─────────────────────────────────────────────────────────┐
       │ GOAL 8 (v0.8.0): Schema 4, Privacy, Insights            │
       │ Status: DELIVERED on main (2026-10-26, PRs #74-#90)     │
       └────────────────────────────┬────────────────────────────┘
                                    │
                                    ▼
       ┌─────────────────────────────────────────────────────────┐
       │ GOAL 9 (v0.9.0 / vNext): Capture, Insights, Platforms   │
       │ Status: ACTIVE BACKLOG                                  │
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

---

## 🧭 Goal 6 (v0.6.0): Operator Usability, Zero-Ceremony Dashboards & Smart Defaults
* **Status:** Delivered in **v0.6.0** (2026-10-19)
* **Council Governance:** Convened a 5-member multi-model Usability Council across **AGY**, **Codex**, and **Claude**:
  1. **User Persona A — Pragmatic Solo/Small-Team Operator** (AGY `research` subagent): Diagnosed cold-start DB failures (`Error: analytics database could not be opened`), mandatory manual RFC3339 `--from`/`--to` flags, manual `dendro materialize` plumbing before `dendro ask`, and brittle 6-string matching in `dendro ask`.
  2. **User Persona B — Engineering Lead & Multi-Rig Fleet Supervisor** (AGY `research` subagent): Diagnosed multi-step ceremony to inspect dashboards, trailing-window mismatch on historical DBs (`--window 7d` anchored to `Utc::now()` showing `100% UNKNOWN` on recorded corpora), and missing fleet status preflight.
  3. **UX Specialist** (Codex `low` effort): Designed progressive disclosure (`dendro` -> `status`, `dendro dashboard`, `dendro ask "<question>"`), actionable error recovery hints, and in-TUI `?`/`h` keyboard help overlay.
  4. **Information Architect** (Codex `low` effort): Unified the CLI command taxonomy and universal `--window 1h|24h|7d|30d|all` resolution anchored to recorded database timestamp bounds (`[min_ts, max_ts]`) with explicit `window_resolution` disclosure.
  5. **Product Owner** (Claude `sonnet` `medium` effort): Scoped and prioritized the v0.6.0 release slices and non-negotiable analytical guardrails (no fabricated metrics, preserved `#![deny(unsafe_code)]`, parameterized SQL, and backward compatibility with all v0.5.0 contracts).

### v0.6.0 Usability Workplan & Delivery Matrix

| Slice | Title | Capabilities Delivered | Status |
| :--- | :--- | :--- | :--- |
| **U6.1** | `feat(cli): Zero-Config Entry & Status Preflight` | Bare `dendro` defaults to `dendro status`; reports DB readiness (`ready`, `empty`, `not_initialized`), redacted rigs/seats, `usage_rows`/`activity_rows`/`queue_rows`, rig-scoped `[min_timestamp, max_timestamp]` bounds, `semantic_fresh`, and next steps; missing DB errors include actionable `dendro run` / `dendro dashboard --auto-ingest` guidance | Delivered (v0.6.0) |
| **U6.2** | `feat(window): Universal Smart Window Resolution` | `--window 1h\|24h\|7d\|30d\|all` across `dashboard`, `ui`, `report`, `trace`, `materialize`, `export`, `ask`, and `simulate topology`; anchors relative windows to rig-scoped DB `[min_timestamp, max_timestamp + 1s)` when data exists and discloses `window_resolution` in JSON and TUI headers | Delivered (v0.6.0) |
| **U6.3** | `feat(dashboard): One-Command Dashboard & Auto-Ingest` | `dendro dashboard` opens the interactive TUI (or `--once` snapshot / `--html [path]` offline web bundle) over the active DB window; `--auto-ingest` on `dashboard` and `ui` automatically ingests local logs when the DB is missing or not `ready` | Delivered (v0.6.0) |
| **U6.4** | `feat(ask): Auto-Materializing Conversational Ask & Natural Aliases` | Positional `dendro ask "<question>"` (plus `--question` compatibility), automatic `semantic::refresh` when `dendro_semantic_snapshot` is missing or stale (unless `--no-refresh`), `--list-intents`, and natural whole-phrase aliases (`"highest idle"`, `"where do tokens go?"`, `"rework"`, `"verify"`, `"trace tokens"`, `"trace handoffs"`, `"known limits"`) | Delivered (v0.6.0) |
| **U6.5** | `feat(tui): In-App Keyboard Help Overlay` | Pressing `?` or `h` inside `dendro ui` / `dendro dashboard` opens a self-contained keyboard & panel navigation guide | Delivered (v0.6.0) |

---

## 🎛️ Goal 7 (v0.7.0): Operator Surface & 6-Screen Ratatui Cockpit

* **Status:** Delivered on `main` 2026-10-20 in [PR #73](https://github.com/smota/dendro/pull/73). [PR #71](https://github.com/smota/dendro/pull/71) is the v0.6.0 usability merge (tag `v0.6.0`) and [PR #72](https://github.com/smota/dendro/pull/72) is its docs update; both stay under Goal 6. Release notes name tag `v0.7.0`; that git tag is not in the repository. Package rules from v0.6.0 stand: missing stays unknown, logs are read-only, `#![deny(unsafe_code)]`, parameterized DuckDB.
* **Objective:** Make the operator surface match evidence v0.6.0 already stored. Bare `dendro` is a sectioned fleet brief. `dendro dashboard` and `dendro ui` are a six-screen Ratatui cockpit. Ask routing is two explicit tiers. Pi ingest reads the OpenRig tree and the standalone `~/.pi/agent/sessions` tree, and status says which is which.

### v0.7.0 Operator Surface & Delivery Matrix

| Slice | Title | Capabilities Delivered | Status |
| :--- | :--- | :--- | :--- |
| **V7.1** | `feat(status): Sectioned Fleet Brief` | No-arg `dendro` runs `dendro status` with `=== DENDRO FLEET INTELLIGENCE ===`, `DATABASE & COVERAGE`, `DISCOVERED HARNESS SOURCES`, `FLEET TOPOLOGY (RIGS & AGENT SEATS)`, and `QUICKSTART & COMMANDS`. States are `NOT INITIALIZED`, `READY`, and `EMPTY` with usage, activity, and queue counts. `dendro status --format json` exposes `pi_breakdown` (`openrig_files`, `standalone_files`, `linked_usage`, `standalone_usage`) and `harnesses.pi`. Seats print per rig. `dendro --help` leads with status, run, cockpit, and ask, then reports, governance, and cache, and lists report kinds `links`, `activity`, `handoffs`, `tokens`, `decisions`, `advisory`, `anomalies`, `spotlights` | Delivered (#73) |
| **V7.2** | `feat(hygiene): Entity Classifier for Status and Cockpit` | One classifier feeds status and the cockpit: agent seat `<role>@<rig>` when the rig is not a raw ULID; queue endpoint `qitem-…`; system or gate `external:…`, `system:…`, `…@system`, or exactly `system`, `steerer`, `steer-lead`; every other id is unlinked, including a 26-character Crockford ULID starting with `01`. Named rigs are listed. Ephemeral ULID rigs are a count (`+N ephemeral ULID`). Cockpit `x` shows or hides queue endpoints and gates. The default view stays on canonical agent seats | Delivered (#73) |
| **V7.3** | `feat(tui): Six-Screen Ratatui Cockpit, TachyonFX, Porter-Duff Backdrop` | `dendro dashboard` and `dendro ui` render Pulse `1`, Seats & Timeline `2`, Handoffs & Flow `3`, Tokens & Cache `4`, Topology Simulator `5`, and Trace & Spotlights `6`. TachyonFX plays screen transitions. Modal overlays sit on a Porter-Duff RGBA backdrop (the OpenTUI-style compositor in `dendro-cli`). Keys: `/` filter seats, `x` hygiene, `m` ribbon or timeline, `v` cycle the handoff heatmap (return rate, count, wait median, exec median), `r` cycle named rigs, `w` cycle `1h` / `24h` / `7d` / `30d` / `all`, `:` or `a` ask Nao, `i` preview an issue draft, `?` or `h` help, `q` quit. `--once`, `--html`, `--auto-ingest`, and `--window` behave as in v0.6.0 | Delivered (#73) |
| **V7.4** | `feat(ask): Two-Tier Nao Routing` | `dendro ask` refuses unknown questions. Tier 1 is an exact match on the normalized phrase (case, whitespace, trailing `?` or `.`). Tier 2 scores tokens for Rank, Explain, Compare, Verify, Trace, and HonestLimits. Unsupported qualifiers refuse first (mission or calendar words, SQL verbs, forecast or savings language, secrets, any token containing `@`). A miss returns `suggest_closest_intents`. Default format is text on a terminal and json otherwise. `--list-intents` prints JSON and does not need a database | Delivered (#73) |
| **V7.5** | `feat(ingest): Dual-Root Pi Ingest` | Default roots are `$OPENRIG_HOME/state/pi` (or `~/.openrig/state/pi`) and `~/.pi/agent/sessions`. Two distinct `--root pi=<path>` values are both kept; the same path twice is deduplicated. A file under `.pi/agent/sessions` is a standalone file; other Pi files are OpenRig files. Usage whose `native_session` is in Pi `identity_facts` is linked usage; the rest is standalone usage. Session identity remains the v0.5.0 first-line header rule (K10). Standalone usage is not promoted to a seat | Delivered (#73) |

---

## 🏗️ Goal 8 (v0.8.0): Four-Domain Foundation, Schema 4, Private Storage, Capture Registry & Reference Insights

* **Status:** Delivered on `main` 2026-10-26 via [PR #74](https://github.com/smota/dendro/pull/74) through [PR #90](https://github.com/smota/dendro/pull/90). Workspace version is `0.8.0`. [PR #90](https://github.com/smota/dendro/pull/90) records that the `v0.8.0` git tag and GitHub release were left for a separate go-ahead, so `docs/RELEASE_NOTES_v0.8.0.md` still says the release date is TBD. Scope for this goal is the trimmed v0.8.0 page `.agentflow/proposals/next-version-2026-10-22/SCOPE-TRIMMED.md` (2026-10-24).
* **Objective:** Fleet intelligence on native Windows and Linux, with schema 4 on a private per-user DuckDB, a builtin capture registry, text-free message facts, unprivileged process samples, sub-agent lineage to the root caller, closed-vocabulary security facts, conserved accounting, and six reference insights on the operator surfaces. Missing observations stay unknown.

### v0.8.0 Foundation & Delivery Matrix

| Slice | Title | Capabilities Delivered | Status |
| :--- | :--- | :--- | :--- |
| **T03 / T12** | `domain_v1` vocabulary and session lineage | `domain_v1` is the typed vocabulary for later facts (`message_facts` uses it for `role`, `direction`, `modality`, source order, and `grain`). `lineage_v1` writes `session_edges` as `(parent, child, relation, status, evidence)` from explicit source fields only: Grok `subagent_spawned` / `subagent_finished`, Claude Code sidechains (`isSidechain: true`), and Codex `payload.source.subagent.thread_spawn.parent_thread_id` (Pi has no parent field). Non-explicit links count as `lineage_not_explicit`. Root-caller walks handle chains, diamonds, cycles, and missing parents. Coverage summaries gain `edges=<n>` | Delivered (#74, #76, #85) |
| **T04 / T21** | `migration_v4` schema 3→4, rollback, and release smoke | `dendro migrate` and `dendro rollback` publish schema 3→4 with rollback and loss receipts. Five additive schema-4 tables (`message_facts`, `process_identities`, `process_samples`, `session_edges`, `security_facts`) are created lazily with an exact-column allow-list and sealed keys. [PR #90](https://github.com/smota/dendro/pull/90) runs the Linux historical smoke: build v0.7.0, ingest, then v0.8 `migrate`, `run`, `insights --json`, `status`, and `rollback`, and checks that v0.7 `report tokens` matches the pre-migration report. A blocking `native-windows` CI job covers fmt, check, clippy, and test. The hosted Windows cross-volume job stays non-blocking | Delivered (#76, #90) |
| **B02 / T24** | `platform_v1` and `dendro-windows-private-io` | Native platform safety (`platform_v1`, crate `dendro-windows-private-io`) plus private storage. Defaults are `%LOCALAPPDATA%\Dendro\dendro.duckdb` on Windows and `$XDG_DATA_HOME/dendro/dendro.duckdb` (`~/.local/share/dendro`, mode `0700`) on Linux. An existing non-private folder is refused and left as it is. `dendro storage-init` with no flags creates or verifies the default folder. DuckDB `temp_directory`, `extension_directory`, and extension autoinstall/autoload are set at open time before WAL replay, then `lock_configuration`. Bundled `json` only. Linux publication uses `renameat2` `NOREPLACE` / `EXCHANGE`. Opening an ordinary or legacy database path is refused before open, with the default private path and an exact copy command (`Copy-Item` or `cp`, including `.wal`). Nothing is copied automatically | Delivered (#74, #76, #77, #78) |
| **T05 / T18** | `capture_registry_v1` on the live path | `dendro run` routes every reader through `Registry::builtin()` for `claude`, `codex`, `pi`, `grok`, `cursor`, `windsurf`, `goose`, and `devin`. `agy` is registered as `pending`. Per-file coverage is `coverage=complete`, `partial`, or `refused`. One bad export file is refused while the rest of the run continues. A missing or failed source is `refused` | Delivered (#81, #89) |
| **T06 / T07** | Claude, Codex, Pi, and export bounds, plus `message_facts` | Input ceilings are 8 MiB per record, JSON nesting depth ≤ 64, and ≤ 1024 decoded records per batch. Descriptor limits may only tighten those ceilings. `message_facts` stores text-free metadata (`role`, `direction`, `modality`, source order, `grain`) from Claude, Pi, Codex, and Grok | Delivered (#81) |
| **T08** | Native Grok CLI session reader | Reads `~/.grok/sessions/<cwd>/<session>/updates.jsonl` (`USERPROFILE` / `HOME`) with byte-offset watermarking. One usage row per `modelUsage` (`cachedReadTokens` inside `inputTokens`, `reasoningTokens` inside output). Joins `tool_call` to `tool_call_update` by call id. Persists token integers, timestamps, tool names, and a keyed fingerprint of `rawInput` | Delivered (#80) |
| **T10 / T25** | Linux and Windows process and resource sampler | `dendro watch [--interval 1s] [--duration] [--once] [--harness …] [--flush-every 60s]` samples `claude`, `codex`, `grok`, `pi`, and descendants via `sysinfo` 0.39 (`system` feature only): ≤ 64 tracked processes, ≤ 600 samples per process per watch. `dendro run-agent [--harness NAME] -- <command> [args...]` pins the child PID as the sampler root (`attribution = launched` vs `discovered`; `null` on descendants) and exits with the child's exit code. Stores the executable name only. Unreadable values are `NULL` with a visibility limit. Linux isolation hints cover `/.dockerenv`, cgroup markers, and WSL via `/proc/version`; otherwise `unknown` | Delivered (#82, #83, #84) |
| **T13** | `security_facts` and `dangerous_command` | Explicit policy fields only, closed vocabularies (`other` for unknown). Claude: `permissionMode`, `permissionDecision.decision`, `sandboxed`, `dangerouslyDisableSandbox` → `sandbox_override_requested`. Codex `turn_context`: `approval_policy`, `sandbox_policy.type`, `permission_profile`, and a count of `approved_command_prefixes`. Pi bash command categories. Grok has no permission fields. Dangerous-command categories are `recursive_delete`, `pipe_to_shell`, `force_push`, `chmod_world_writable`, `sudo`, `disk_write_device`, `credential_file_access`, and `network_exfil_pattern` | Delivered (#86) |
| **T14** | `accounting_v1` conservation | Observed cost is separate from estimated cost. Grok observed cost uses `costUsdTicks` via `GROK_USD_TICKS_PER_USD = 10^10`. A multi-model turn without per-model cost leaves each model row `None` and warns `cost_not_attributable`. The pricing table ships empty, so estimates are `null` (`unknown`). Totals over unknown cost are `partial` or `unknown` with `usd: null`. Fleet totals count each usage observation once. Exclusive session totals sum to the fleet total. Per-root inclusive totals are labelled non-additive. `accounting_v1::check_total_admission` admits these defined totals and refuses shared or legacy attribution, cumulative counters, duplicates, mixed currencies, and source-reported totals. Report JSON adds `observed_cost`, `estimated_cost`, `cost_rule`, and per-session `inclusive` | Delivered (#87) |
| **T15 / T16** | Six reference insights | `dendro insights [--json] [--harness H] [--session S]` returns `Computed`, `Partial { reason }`, or `Refused { reason }`, with at most 5 sealed evidence keys. Shipped: `I02` `model.tokens.direct`, `I04` `model.cost`, `I08` `harness.action_failure`, `I09` `work.lineage_shape`, `I14` `environment.resources`, `I18` `coverage.source`. The catalog in `DESIGN.md` still defines I01–I21; the other fifteen are Goal 9 | Delivered (#88) |
| **T17 / T19** | Insights screen, viewer, four Nao views, Obsidian entryway | TUI screen `[7]` Insights with L0–L3 disclosure (`domain` → `insight` → `measures` → `DetailJson`) and observed vs estimated cost on the Tokens screen. Dashboard payload adds `insights` (evidence cleared), `lineage` (`I09`), `environment` (`I14`), and `security`, rendered by offline `site/viewer.html` via `textContent`. The multi-view dashboard entryway is the Obsidian Edition telemetry page `site/dashboards/index.html` ("Fleet Intelligence Telemetry · Obsidian Edition"). `dendro status` reports row counts for the five new fact tables (`not created` in text when the lazy tables are absent; JSON still uses `rows: 0` in that case). Nao views `v_message_mix`, `v_subagent_lineage`, `v_security_posture`, and `v_process_resources` are counts, max, and group-by only (CPU and cost stay in `I14` and `I04`) and are all-time in v0.8.0 | Delivered (#89) |

---

## 🚧 Goal 9 (v0.9.0 / vNext): Active Backlog

* **Status:** Active. Nothing in this goal is on `main` as a v0.9.0 deliverable. Open product defect: [#91](https://github.com/smota/dendro/issues/91). Normative contract for the DuckDB 2.0, intelligence, presentation, and documentation rows: [`.agentflow/proposals/next-version-2026-10-22/ADDENDUM-DUCKDB2-AND-INTELLIGENCE.md`](../.agentflow/proposals/next-version-2026-10-22/ADDENDUM-DUCKDB2-AND-INTELLIGENCE.md) (2026-10-27). That page does not mark `A01`–`A15` done.
* **Objective:** Close the Windows publication failure and the private-path test gaps, then pick up capture and views that v0.8.0 left behind, the other fifteen reference insights from `DESIGN.md`, four active-intelligence analyzers `I22`–`I25`, a DuckDB 2.0.0 LTS / schema-5 migration, and a second platform binding beside OpenRig. New facts, insights, and Nao views are done only together with the presentation gate (`R32`) and the doc-sync gate (`R33`).

### v0.9.0 / vNext Workplan

| Priority | ID | Title | Scope | Status |
| :--- | :--- | :--- | :--- | :--- |
| **P0** | [#91](https://github.com/smota/dendro/issues/91) | Windows `publish_replace` `ERROR_SHARING_VIOLATION` | `dendro-windows-private-io` test `tests::native_staging_rework` fails on GitHub-hosted `windows-2022` (`native-windows`, PR #90). After `create_source` and `write_all`, `publish_replace` on a folder whose SDDL grants the user, SYSTEM, and Administrators returns Win32 code 32 ("The process cannot access the file because it is being used by another process."). Deterministic on the elevated `runneradmin` runner (3 failures); passes locally. Handles in the test use share mode 7. A bounded retry (8 attempts, about 5s) was reverted (`ce2f6e3`, then `17c9300`). The same grant pattern exists on ordinary `%LOCALAPPDATA%` folders, so a SYSTEM holder can fail a real user's publish the same way | Active |
| **P0** | test hardening | Umask and publication-mock hardening | Private-directory tests must assert mode `0700` under the CI umask (077 is called out in `dendro-cli` sealed tests) without changing the process umask. `platform_v1` and `migration_v4` already refuse ambient umask changes and refuse to re-permission an existing folder. Publication tests need to drive real `publish_replace` so a sharing violation like #91 is visible, including Linux `renameat2` `NOREPLACE` / `EXCHANGE` parity | Active |
| **P1** | **T11** | Linux `strace` → `syscall_v1` | Deferred to v0.9 by `SCOPE-TRIMMED.md` (2026-10-24) until a real privileged producer specimen exists. Import is transient and bounded: identify call, return, PID, and time; keep allowlisted operation, status, count, and resource class; discard argument bodies. `DESIGN.md` requires a short-lived synthetic authorized Linux process through the admitted trace reader. Unprivileged process and resource facts already ship as T10 (Goal 8). Feeds `I15` `environment.effects`. Specimen gate B01-S (real Linux strace profile admission) was cut from v0.8.0 because it only existed to admit this importer | Deferred (v0.9.0) |
| **P1** | **T26** | Windows ETW → `windows_events_v1` | Deferred to v0.9 by the same scope page, for the same specimen reason. `DESIGN.md` requires a genuine controlled Windows trace for a bound synthetic process and a frozen provider profile. The ETW profile is a separate event grammar from Linux `syscall_v1`. Specimen gates B01-W / B02-W were cut from v0.8.0 because they only existed to admit this importer | Deferred (v0.9.0) |
| **P1** | **T09** | Native AGY adapter | Cut from v0.8.0 in `SCOPE-TRIMMED.md` ("AGY is not available") and left registered as `pending` in `Registry::builtin()`. `DESIGN.md` specifies the AGY export envelope (contract/version, source instance, record key, time, kind, typed payload; 1 MiB per line; batches ≤ 128) | Cut from v0.8.0 (ParserPending; unscheduled pending AGY format/specimens) |
| **P1** | Nao windows | Half-open `[from, to)` on the four v0.8.0 semantic views | `v_message_mix`, `v_subagent_lineage`, `v_security_posture`, and `v_process_resources` report all-time counts in v0.8.0 (`docs/RELEASE_NOTES_v0.8.0.md`). Apply the `DESIGN.md` half-open window: include a whole atomic count when its evidenced attribution timestamp is inside `[from, to)`, and exclude it when the timestamp is outside. CPU and cost stay on `I14` and `I04` | Active (v0.8.0 known limit) |
| **P1** | **R29** | DuckDB 2.0.0 LTS, schema 5, quack owner | `A01`–`A03`. Pin `duckdb = "~1.20000.0"` (runtime `v2.0.0`) with shadow validation against locked `1.10506.0`. Schema 4→5 backup is the exclusive immutable directory `dendro.duckdb.pre-v200.bak` (main, WAL, privacy key, provenance) selected by the `DESIGN.md:188` pointer manifest. Storage version 69 after cutover. `dendro serve` is the only raw DuckDB or quack session. CLI, TUI, HTTP, Nao, and live hooks use typed RPC on a `0700` Unix socket or a private Windows named pipe. No unauthenticated loopback TCP. Same-user isolation is the capability check, not the file mode | Implemented (vNext: `engine_v200.rs`, `migration_v5.rs`, `quack_owner.rs`) |
| **P1** | **R30** | `VARIANT`, triggers, graph, search, fleet partitions | `A04`–`A07`. Body-free `harness_envelope.envelope_shape` with a per-harness key allowlist (unknown keys dropped or HMAC-replaced). Trigger bodies are O(1) dirty-session notifications on `_dendro_tripwire_outbox`. `I22` runs in the Rust analyzer after commit. `duckpgq` reachability uses only `status = evidenced` and is dependency exposure, not security authority. `session_lineage.rs` is the fallback. `fts` on redacted labels only. Exact `array_cosine_distance` for `FLOAT[32]`. DuckLake 1.1 local pseudonymized Parquet | Implemented (vNext: `envelope_variant.rs`, `tripwire_outbox.rs`, `lineage_graph.rs`, `retrieval_extensions.rs`) |
| **P1** | **R31** | I01–I25 and `dendro advise` | `A08` gates exactly I01–I21. `A10` expands that set to exactly I01–I25. `I22` `harness.trajectory_vortex` (Rust analyzer, not the SQL trigger), `I23` `harness.schema_drift`, `I24` `security.authority_blast_radius`, `I25` `model.counterfactual_routing` from an operator-pinned tier table. Advise answers cohort, skill-effectiveness, delegation-loop, and environment-readiness questions in a warm p95 target of 15 ms. Tripwires are pull-only operator advisories and do not enter agent prompts. `AdviseUncertainty` is not a display badge | Implemented (vNext: `insight_v1.rs`, `advise.rs`, `tripwire_v1.rs`) |
| **P1** | **R32** | Presentation parity | `A13`–`A14`. Dashboards grouped as Model, Harness, Coordination & Lineage, and Environment & Security, with OpenRig badged as a platform adapter. Insight IDs match across the registry, `presentation.rs`, `ALLOWED_INSIGHT_IDS`, the Compiled Insights Strip, and the TUI. TUI L0–L3 keep the `DESIGN.md:161` levels: L0 status, finding, practical meaning, scope, and missingness; L1 assumptions and alternatives; L2 safe evidence IDs and source versions; L3 formula, population (`what`, `included`, `excluded`), exclusions, and cutoff. `display_status` maps `Refused` plus `not_observable` or `insufficient_sample` to `insufficient_evidence`. Surfaces `#0A0D12` / `#11161F`. Snapshot and live badges are mutually exclusive | Implemented (vNext: `semantic.rs`, `site.test.cjs`) |
| **P1** | **R33** | Docs and Nao catalog sync | `A15`. Schema, insight, Nao, CLI, TUI, and web changes update `docs/NAO_SEMANTIC.md`, `templates/nao/`, `docs/ARCHITECTURE.md`, `docs/DATA_DICTIONARY.md`, `docs/CLI_R1.md`, and this backlog in the same change. `site/site.test.cjs` and `crates/dendro-core/tests/semantic.rs` fail the build on insight and Nao set drift. `semantic.rs` also requires every schema-5 table in the data dictionary and architecture doc. `crates/dendro-cli/tests/cli_doc_parity.rs` requires every `clap` subcommand in `docs/CLI_R1.md` | Implemented (vNext: `cli_doc_parity.rs`, `semantic.rs`, `site.test.cjs`) |
| **P2** | **I01, I03, I05–I07, I10–I13, I15–I17, I19–I21** | Remaining 15 reference insights | v0.8.0 shipped 6 of the 21 `DESIGN.md` version `1.0.0` definitions (`I02`, `I04`, `I08`, `I09`, `I14`, `I18`). Register and compute the other fifteen on captured facts, with the same `Computed` / `Partial` / `Refused` contract. `I15` waits on T11 and T26 and returns `not_observable` until those specimens exist. Names and refusal conditions are in the catalog below. Acceptance is addendum `A08`, and a row is not done without `A13` and `A15` | Implemented (vNext: `insight_v1.rs`, `semantic.rs`) |
| **P3** | platforms | Three-plane abstraction and a second platform connector | Split legacy OpenRig nouns that still sit in `dendro-core` into platform bindings, and add an AgentFlow / Meshloop connector beside the OpenRig binding. v0.8.0 `platform_v1` is the native Windows/Linux IO and publication layer, which is the substrate this split builds on. No second connector is in the v0.8.0 tree. This split does not itself apply schema 5 (`R29` / `R30`) | Active (vNext) |

### P2 catalog (implemented in vNext)

Definitions below are the `DESIGN.md` section 5 rows, implemented and verified on `main` via `crates/dendro-core/tests/insight_v1.rs` and `crates/dendro-core/tests/semantic.rs`. v0.8.0 names for the six shipped insights are in Goal 8.

| ID | Name | Contract | Status |
| :--- | :--- | :--- | :--- |
| **I01** | `model.invocations` | Count distinct evidenced call IDs. Call grain only; usage-only rows counted separately; no admitted call facts → `not_observable` | Implemented (proof: `insight_v1.rs`) |
| **I03** | `model.tokens.inclusive` | Sum over the reachable observation union. Nonadditive. Incomplete nonnegative lineage → lower bound. Ambiguous duplicates refuse the canonical total | Implemented (proof: `insight_v1.rs`) |
| **I05** | `model.configuration_divergence` | Observed bindings differing from contemporaneous desired bindings, or from paired observed bindings. Unknown desired or observed values are excluded and counted. Missing paired population → `not_observable` | Implemented (proof: `insight_v1.rs`) |
| **I06** | `harness.capability_funnel` | Distinct loaded and invoked capability IDs, and the invoked∩loaded / loaded rate. Absent loaded evidence refuses the rate; actions still count | Implemented (proof: `insight_v1.rs`) |
| **I07** | `harness.context_pressure` | Evidenced context-used / context-limit at invocation. Prompt tokens do not substitute. Missing limit refuses the ratio | Implemented (proof: `insight_v1.rs`) |
| **I10** | `work.duration` | Wall union, summed attempt-seconds, and summed/wall. Half-open same-clock segments. Censored contribution is partial. `wall=0` refuses the concurrency ratio | Implemented (proof: `insight_v1.rs`) |
| **I11** | `work.shared_allocation` | Shared usage, weighted beneficiaries, and residual. Rational weights ≥ 0 and sum ≤ 1. Absent policy leaves the shared bucket | Implemented (proof: `insight_v1.rs`) |
| **I12** | `coord.queue_wait` | Sum or median of ready-to-start elapsed intervals, and a separate wall union. Missing ready, start, or cross-clock uncertainty refuses the affected intervals | Implemented (proof: `insight_v1.rs`) |
| **I13** | `environment.placement` | Distinct incarnations and placement intervals by locus, plus same-label rebuild count. Unknown incarnation gets an unresolved count | Implemented (proof: `insight_v1.rs`) |
| **I15** | `environment.effects` | Normalized successful and failed syscall counts by operation and resource class, from admitted T11/T26 traces. An unmatched call has an unknown result. No trace → `not_observable` | Implemented (proof: `insight_v1.rs`) |
| **I16** | `security.isolation` | Paired declared / configured / tested dimension comparisons. A lone declaration is insufficient. Namespace equality is configuration, not a universal verification | Implemented (proof: `insight_v1.rs`) |
| **I17** | `security.authority` | Comparator counts of consistent, mismatch, and insufficient by an explicit rule. No applicable policy → `NOT_ASSESSED` | Implemented (proof: `insight_v1.rs`) |
| **I19** | `outcome.acceptance` | Currently accepted eligible work / rule-eligible evaluated work, with attempt success kept separate. Apply revocations and supersession. No acceptance rule → `not_observable` | Implemented (proof: `insight_v1.rs`) |
| **I20** | `outcome.resource_efficiency` | Matched-population cost / accepted work, and a separate mean of per-work ratios. Zero accepted → null. The named mean excludes undefined terms and reports the count | Implemented (proof: `insight_v1.rs`) |
| **I21** | `comparison.descriptive` | Stratified difference of same-version measures. Match clock basis, grain, accounting, units, and coverage. Biased or unmatched selection → `noncomparable` | Implemented (proof: `insight_v1.rs`) |

### Addendum catalog (`I22`–`I25`, implemented in vNext)

Formulas, refusals, and the `A01`–`A15` proof table are in the addendum linked from the Goal 9 status. Verified on `main` via `crates/dendro-core/tests/insight_v1.rs` and `crates/dendro-core/tests/semantic.rs`.

| ID | Name | Contract | Status |
| :--- | :--- | :--- | :--- |
| **I22** | `harness.trajectory_vortex` | `FLOAT[32]` projection of a closed capability/outcome transition sketch. The SQL trigger only marks the session dirty. The Rust `insight_v1` analyzer, after commit or on demand, trips at turn \(k\) (initial \(k = 3\)) when cosine distance and self-transition rate both clear the frozen cohort bounds. No prompt text. The signal is pull-only and does not stop the agent | Implemented (proof: `insight_v1.rs`, `tripwire_outbox.rs`) |
| **I23** | `harness.schema_drift` | L1 distance of `envelope_shape` key-path and `variant_type` distributions across harness versions, plus added, removed, and retyped key counts. Unknown keys never enter the `VARIANT`. Short samples are reason `insufficient_sample` and badge `insufficient_evidence` | Implemented (proof: `insight_v1.rs`, `envelope_variant.rs`) |
| **I24** | `security.authority_blast_radius` | Bounded `duckpgq` reachability on `status = evidenced` only, from a revoked or failed grant, or an unverified sandbox, to downstream sessions. `spawn` / `delegate` / `handoff` walk parent → child. `return` / `join` walk child → parent. `session_lineage.rs` must match on fixtures. The count is dependency exposure, not inherited or effective authority. PageRank is not an input | Implemented (proof: `insight_v1.rs`, `lineage_graph.rs`) |
| **I25** | `model.counterfactual_routing` | `ASOF JOIN` and `QUANTILE_CONT` cache-bleed inflection, then a fast-tier cost and context simulation from a versioned operator-pinned pricing/tier table. No hardcoded model name. Unpinned cohort or tier profile refuses the simulation and keeps observed `I02`. Labeled counterfactual | Implemented (proof: `insight_v1.rs`) |

