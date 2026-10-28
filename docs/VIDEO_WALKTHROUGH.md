# Dendro Connected Intelligence Studio — Video Walkthrough & Tour

**Video Asset:** [`docs/assets/dendro-demo.mp4`](assets/dendro-demo.mp4)  
**Animated Preview:** [`docs/assets/dendro-preview.gif`](assets/dendro-preview.gif)  
**Recorded Surface:** Connected Intelligence Studio (`http://127.0.0.1:4242`)  
**Resolution / Duration:** `1480x960` @ `24 fps` (`30.8s` H.264 MP4 + `960x622` 10 fps optimized palette GIF)

---

## Overview & Analytical Integrity

This walkthrough captures the live **Connected Intelligence Studio** running against real multi-agent fleet telemetry ingested across native AI coding harnesses (Claude Code, OpenAI Codex, Google Antigravity, Grok, Pi), export adapters (Cursor, Windsurf, Goose, Devin ATIF-v1.7), and multi-agent coordination runtimes (OpenRig, AgentFlow, Meshloop).

In accordance with Dendro's analytical standards:
- **Zero Mock / Unedited Live Telemetry:** Every screen displays live data served by the DuckDB-backed HTTP studio engine (`dendro.duckdb`).
- **Explicit Time Anchoring:** The Time Machine Scrubber explicitly anchors every metric (`7d`, `30d`, `this_month`, `last_month`, `all`, and custom date bounds). Missing observation denominators remain `Unavailable` / `null`, never fabricated zero.
- **Fail-Closed Governance:** Insights separate observed historical facts from counterfactual what-if projections (`I25`), preserving an auditable chain of evidence.

---

## Scene-by-Scene Tour

### Part I: The Fleet Intelligence Hub & Living Growth Rings (`00:00 – 00:06.6`)

#### Scene 01 — Top of Hub & 7-Day Window (`00:00 – 00:02.2`)
* **URL:** `http://127.0.0.1:4242/dashboards/index.html?window=7d`
* **Visual Highlights:**
  - **Brand Header & Global Fleet Status:** Displays real-time fleet health (`FLEET OPTIMAL`), active rigs, seats, and session counts.
  - **Time Machine Scrubber:** Segmented control active on `Last Week (7d)`, dynamically slicing telemetry.
  - **Living Growth Rings (SVG):** Concentric rings visualizing multi-layer fleet maturity (Model, Harness, Coordination, Environment).
  - **3 Narrative Story Highlights:** Executive callouts summarizing token cache yield, tool stability, and coordination efficiency.

#### Scene 02 — Fleet-Wide Temporal Slicing (`00:02.2 – 00:04.4`)
* **URL:** `http://127.0.0.1:4242/dashboards/index.html?window=all`
* **Visual Highlights:**
  - Dynamic rehydration of the entire dashboard to `All Time` bounds without page reload.
  - KPI cards reflect full historical volume across all connected rigs and agent seats.

#### Scene 03 — Custom Range Popover Bar (`00:04.4 – 00:06.6`)
* **Action:** Click `Custom Range ▾` (`#btn-custom-range`)
* **Visual Highlights:**
  - Reveals the Apple-grade inline custom date popover bar (`#hub-custom-range-bar`).
  - Allows precise UTC date slicing (`from` and `to`) with zero-overhead DuckDB range querying.

---

### Part II: Multi-Dimensional Rollup & 4-Layer Exploding Stack X-Ray (`00:06.6 – 00:13.2`)

#### Scene 04 — Cross-Layer Stack X-Ray (Flat Swimlanes) (`00:06.6 – 00:08.8`)
* **Section:** `#hub-stack-xray`
* **Visual Highlights:**
  - Flat architectural swimlanes grouping fleet execution across four distinct layers:
    1. **Layer 1: Model** (Token distribution, prompt cache hit rate, model latency).
    2. **Layer 2: Harness** (Tool execution, terminal bash sessions, file mutations).
    3. **Layer 3: Coordination** (Agent seats, delegation trees, queue transitions, handoffs).
    4. **Layer 4: Environment** (Sandboxes, git worktrees, lock files, system resources).
  - Real-time fact volume shares and anomaly flags highlighted per layer.

#### Scene 05 — 3D Isometric Stack Explode (`⌘E`) (`00:08.8 – 00:11.0`)
* **Action:** Click `Explode 3D Stack ⌘E` (`#btn-xray-3d`)
* **Visual Highlights:**
  - Smooth CSS 3D perspective transformation tilting the swimlanes into an isometric architectural stack.
  - Visually demonstrates how low-level token facts stack into harness calls, multi-agent coordination handoffs, and physical environment state.

#### Scene 06 — Fleet Telemetry Rollup & Dimension Explorer (`00:11.0 – 00:13.2`)
* **Section:** `#hub-dimension-explorer`
* **Action:** Switch to `By Architectural Layer` tab (`#dim-tab-layer`)
* **Visual Highlights:**
  - Interactive multi-dimensional rollup table showing recorded facts, volume share percentages, active anomalies, and primary insight focus per layer.
  - Seamless toggle between *By Harness / Tool*, *By Architectural Layer*, and *By Date / Time Bucket*.

---

### Part III: 5-Layer Capability Studio & Bento Console Dock (`00:13.2 – 00:19.8`)

#### Scene 07 — Layer 1: Tool Families & 10 Archetypes (`00:13.2 – 00:15.4`)
* **Section:** `#hub-capability-studio`
* **Visual Highlights:**
  - Spectrum bar illustrating execution proportions across the fleet.
  - Archetype Bento Grid categorizing 42.2K observed calls into 10 behavioral archetypes (`inspector_diff`, `editor_writer`, `terminal_exec`, `context_graph`, etc.).

#### Scene 08 — Layer 3: Skills Capability Funnel (Insight I06) (`00:15.4 – 00:17.6`)
* **Action:** Click `Skills (Declared vs Invoked)` tab (`#tab-btn-skills`)
* **Visual Highlights:**
  - Funnel header: 18 declared skills &middot; 12 actively invoked &middot; 6 dormant in observation window.
  - Bento cards displaying activation frequency, invocation rate, and dormant skill indicators to prevent agent prompt bloating.

#### Scene 09 — Bento Console Dock & Diagnostic Lenses (`00:17.6 – 00:19.8`)
* **Action:** Click `MCP Servers & Tools` tab (`#tab-btn-mcp`), then scroll to `#views-container`
* **Visual Highlights:**
  - MCP Namespace metrics (e.g. `holoself`, `mtn-context-graph`) with tool counts and invocation health.
  - Apple Bento Console Dock (`#views-container`): High-contrast glass tiles linking directly to specialized intelligence views with active anomaly badges.
  - Diagnostic Lenses (`#persona-questions`): Filterable questions grouped by persona (Staff Engineer, AI Architect, Fleet FinOps).

---

### Part IV: Active Intelligence Consoles & Token Economics (`00:19.8 – 00:30.8`)

#### Scene 10 — Trajectory Vortex Console (Insight I22) (`00:19.8 – 00:22.0`)
* **URL:** `http://127.0.0.1:4242/dashboards/views/trajectory_vortex.html?window=all`
* **Visual Highlights:**
  - **Orbital Phase-Space Attractor:** 2D phase-space canvas mapping agent turns, looping traps, and convergence orbits.
  - Dynamic friction matrix detailing loop duration, turn velocity, and recovery interventions.

#### Scene 11 — Schema Drift Console (Insight I23) (`00:22.0 – 00:24.2`)
* **URL:** `http://127.0.0.1:4242/dashboards/views/schema_drift.html?window=all`
* **Visual Highlights:**
  - **Schema DNA Fingerprint Ribbon:** Visualizes key additions, removals, and type mutations across tool payloads over time.
  - Explicit warning states for short samples (`insufficient_sample`) to prevent premature statistical conclusions.

#### Scene 12 — Authority Blast Radius Console (Insight I24) (`00:24.2 – 00:26.4`)
* **URL:** `http://127.0.0.1:4242/dashboards/views/authority_blast_radius.html?window=all`
* **Visual Highlights:**
  - **Radial Propagation Ripple Graph:** Maps the blast radius of agent tool execution across files, shell commands, and system authority.
  - Highlights high-risk touchpoints, credential accesses, and cross-worktree modifications.

#### Scene 13 — Counterfactual Routing Console (Insight I25) (`00:26.4 – 00:28.6`)
* **URL:** `http://127.0.0.1:4242/dashboards/views/counterfactual_routing.html?window=all`
* **Visual Highlights:**
  - **Split-Beam Route Comparator:** Compares actual model routing decisions against counterfactual alternatives.
  - Interactive Inflection Slider demonstrating cost vs latency trade-offs without speculative extrapolation.

#### Scene 14 — Model Token & Cost Accounting (Insights I02 / I04) (`00:28.6 – 00:30.8`)
* **URL:** `http://127.0.0.1:4242/dashboards/views/cost_and_loops.html?window=all`
* **Visual Highlights:**
  - Multi-dimensional breakdown of token consumption: prompt tokens, completion tokens, and prompt cache hit rate (highlighting a **96.9% cache yield**).
  - Cost attribution by actor, seat, and task with rigorous distinction between measured provider counters and estimated rates.
