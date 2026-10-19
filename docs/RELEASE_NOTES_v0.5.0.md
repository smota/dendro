# Dendro v0.5.0 Release Notes (Goals 1–5 Complete)

Release-candidate documentation and independent verification receipts for **Dendro v0.5.0**, completing Goals 2–5 atop the released v0.1.1 (R1) core engine. Tagging and external publication remain governed by operator release authorization.

---

## Release Candidate Identity & Provenance

| Field | Value |
| :--- | :--- |
| **Version** | `0.5.0` (`workspace.package.version` across `dendro-core`, `dendro-connectors`, `dendro-resolver`, `dendro-watcher`, `dendro-cli`) |
| **Base Commit (`main`)** | `25e2c65` (incorporating PR #56, PR #57, PR #58 `81f5a90`, PR #68 `b15397e`, PR #69 `25e2c65`) |
| **Release Binary (`target/release/dendro`)** | SHA256 `555c2ed059b36f0bb250b0a90e6688e63d099c5995201fafb471845ee628bd23` (`dendro 0.5.0`) |
| **Toolchain** | `rustc 1.98.0 (88d9e12ae 2026-08-18)` / `cargo 1.98.0 (797e8a9bc 2026-08-05)` |
| **Base Analytical Contract** | `docs/ACCEPTANCE_R1.md` (`cc365c88…`), Clarifications Register K1–K11 (`44a3032e…`), `docs/ACCEPTANCE_G2_STAGED.md` |

---

## What Ships in v0.5.0 (Since v0.1.1)

### 1. Goal 2 — Conversational & Semantic Analytics, Sealed Replay & Terminal Explorer (`#7`, `#12`–`#14`, `#16`–`#18`, `#30`–`#34`, `#52`–`#55`)
- **Rework Vortex & Pair Friction Matrix (`#12`):** Pair-level handoff/return breakdown, M3 rework chain attribution, handoff-loop detection (`>2` passes on the same pair), and configurable friction alert thresholds (`--friction-return-rate`, `--friction-chain-returns`, `--friction-rework-tokens`, `--friction-loop-duration`).
- **Live-Producer Snapshotting & Sealed Replay (`#13`, `#31`, `#32`):** Atomic byte-cutoff manifests (`dendro run --write-manifest <path>` and `dendro run --sealed <path>`), unified source-state disclosures (`available`, `gone`, `replaced`, `absent`), and single-OpenRig-root-per-DB enforcement.
- **Canonical Semantic Views & Local Interrogation (`#16`, `#17`, `#18`):**
  - `dendro materialize` populates four typed DuckDB semantic views (`v_seat_idle_ranking`, `v_handoff_rework_analysis`, `v_mission_token_burn`, `v_executive_governance`) using typed appenders under a 1 GB refresh cap.
  - `dendro ask` answers six deterministic English interrogation patterns (`Compare`, `Rank`, `Explain`, `Verify`, `Trace`, `HonestLimits`) with exact token/handoff conservation against base tables.
  - `dendro init-nao` creates a local-only Nao workspace (`0700`/`0600`) without overwriting existing directories.
  - Legacy sidecar compatibility (`#59`): sidecar databases missing optional tables (`file_source_baselines`, `openrig_row_records`, `mutable_source_disclosures`) refresh and invalidate cleanly using parameterized `information_schema.tables` probes.
- **Interactive Terminal Explorer (`#14`):** `dendro ui` (24h/7d trailing or fixed `--from`/`--to` window, plus `--once`) and `dendro report --viz`, with keyboard navigation, single-`ESC` reset on navigation, and C1 control-character escaping (`\uXXXX`).
- **Extended Fleet Signals (`#52`–`#55`):** Ceremony vs. engineering tool categories, $k$-seat concurrency histograms, mid-phase compaction hazard detection (`>50%` context drop while `in-progress`), recovery item separation (`qitem-recovery-*`), ring boundary classification (`in_ring`/`out_of_ring`), and `(model, effort)` breakdown.

### 2. Goal 3 — Portable Harness Connectors, SDK & Cross-Project Portfolio (`#8`, `#19`–`#21`, `#27`)
- **Verified-Export Stream Connectors (`#19`, `#20`):** Incremental watermark-backed connectors for **Cursor**, **Windsurf**, **Devin**, and **Goose** portable verified-export session logs, enforcing secret redaction and path containment.
- **Custom Harness Developer SDK (`#21`):** Documented in `docs/extensions/README.md` and `docs/extensions/formats.md` with tutorial and example (`crates/dendro-connectors/examples/custom_harness.rs`).
- **Cross-Project Fleet Portfolio (`#27`):** `dendro portfolio` aggregates independently ingested analytics databases (`--sources` or `--source-dir`) in `json`, `ndjson`, or `table` formats with B1–B4 hold evidence and strict non-symlink `hold_dir` containment (`#59`).

### 3. Goal 4 — Interactive 3-Layer Visualizer, Offline HTML Export & Waste Calculator (`#9`, `#22`, `#23`, `#28`)
- **Interactive Landing Visualizer & Dynamic Cost-of-Inaction Calculator (`#22`, `#23`):** Shipped under `site/` with deterministic Sankey/timeline rendering and empirical financial waste sliders.
- **Scrubbed Offline HTML Export (`#28`, `#59`):** `dendro export --format html --output <path>` writes a self-contained, aliased `0600` HTML viewer and unlinks partial files on write failure without touching pre-existing files.

### 4. Goal 5 — Advisory Governance, Anomaly Spotlights & Topology Simulator (`#10`, `#15`, `#24`–`#26`, `#29`, `#66`)
- **Closed-Loop Backlog Feeder (`#15`):** `dendro issue-draft` generates local Markdown operational-debt drafts with quantified cost-of-inaction (and opt-in `--publish --repo OWNER/REPO`).
- **Automated CI Quality Gate (`#25`):** `dendro assert` evaluates M3 rework rate and idle-with-claimed-work thresholds with deterministic CI exit codes (`0` pass, `2` breach, `3` insufficient evidence).
- **Idle-with-Claimed-Work Advisory (`#26`) & Anomaly Spotlights A1–A6 (`#29`):** `dendro report spotlights` surfaces six proactive operational hypotheses (ceremony tax, serialization bottleneck, container hold, recovery churn, context-drop hazard, builder saturation) as report-only evidence.
- **Historical Topology What-If Simulator (`#24`):** `dendro simulate topology` models seat removal, replacement seat routing, direct handoffs, tier multipliers (`--tier-policy`), boundary resets (`--boundary-reset-threshold`), and model-family diversity (`--enforce-model-family-diversity`) over canonical decision reports while conserving historical engineering demand.

---

## Release-Readiness Verification Receipts

### 1. Deterministic Quality & Security Gates
- `cargo fmt --check`: **pass**
- `cargo check --workspace --locked`: **pass**
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: **pass (`0` warnings)**
- `cargo test --workspace --locked`: **379 passed, 0 failed, 1 ignored** across 22 test suites (including the 5,500-fact + 16 MB extension materialization benchmark under 512 MB RSS).
- `node --test site/site.test.cjs`: **13 passed, 0 failed, 1 optional fixture skipped**.
- `#![deny(unsafe_code)]`: enforced across **all 5 workspace crates** (`dendro-core`, `dendro-connectors`, `dendro-resolver`, `dendro-watcher`, `dendro-cli`); `0` `unsafe` blocks in workspace source.
- SQL parameterization: `0` dynamic `format!()` SQL queries in production library/binary code; all runtime SQL uses static statements or `duckdb::params!` / `rusqlite::params!`.

### 2. Real-Log Corpus Benchmark (AC1 — `ac1_real_logs_finish_within_five_minutes`)
- **Measured elapsed time:** **96.24 s** (`96,243 ms`), leaving **203.76 s of margin** below the 300 s AC1 ceiling.
- **Corpus ingested (`--release` binary):**
  - Claude Code: `138` session files
  - Codex: `127` session files
  - Pi: `126` session files
  - OpenRig SQLite snapshot: `71,686` events

### 3. Release Binary 12-Subcommand CLI Smoke Sweep
Executed with `/home/sam/code/dendro/target/release/dendro` (`0.5.0`, SHA256 `555c2ed059b36f0bb250b0a90e6688e63d099c5995201fafb471845ee628bd23`):
- `dendro --version` → `dendro 0.5.0`
- `dendro report decisions` / `dendro report spotlights` / `dendro report decisions --viz` → pass
- `dendro trace --metric-id tokens` → pass
- `dendro materialize` / `dendro ask` / `dendro init-nao` → pass
- `dendro export --format html` → pass (`0600` file mode verified)
- `dendro portfolio` → pass
- `dendro simulate topology` → pass (`status: projected` / `status: no_data`, `conserved: true`)
- `dendro issue-draft` / `dendro assert` (exit code `3` on insufficient-evidence fixture) / `dendro ui --once` → pass

---

## Explicit Known Limits & Truthful Scope Boundaries

1. **Observational Intelligence Only:** Dendro opens sources read-only and never mutates live OpenRig state, agent seats, or runtime routing.
2. **Topology Simulation (`dendro simulate topology`):** Computes deterministic accounting projections over recorded historical tokens and handoffs under explicit operator assumptions. It is not a causal predictor of future LLM productivity or hazard elimination.
3. **Conversational / Nao Layer (`dendro materialize`, `dendro ask`, `dendro init-nao`):** Local DuckDB semantic views, deterministic English intent answering, conservation checks, and local Nao workspace generation are verified in automated tests. Live external cloud-LLM Nao operator sessions are not fabricated in local CI.
4. **Editor & Autonomous Connectors (Cursor, Windsurf, Devin, Goose):** Ingest verified portable JSONL/JSON session exports under authorized local roots; private proprietary cloud databases and remote webhooks are explicitly out of scope.
5. **Cross-Project Portfolio (`dendro portfolio`):** Aggregates independent per-project DuckDB databases and local hold manifests; it does not claim cross-database atomic point-in-time snapshots or live unrecorded hold residence durations.
6. **Codex Tool Calls (inherited from v0.1.1 Known Limit 11):** Codex structured tool-call loop counts remain marked `not read` where Codex logs omit structured file-write/tool-result pairs.
