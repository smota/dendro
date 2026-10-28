# Dendro v0.9.0 Release Notes — Connected Intelligence Studio, 5-Layer Capability Matrix & Active Intelligence Consoles

**Release Date:** 2026-10-28  
**Tag:** `v0.9.0`  
**Previous Release:** `v0.8.0`  

---

## Executive Summary

**Dendro v0.9.0** delivers an Apple-grade Connected Intelligence Studio, comprehensive 5-layer capability intelligence, 4 active intelligence consoles (`I22`–`I25`), offline fleet explorer fixtures, and single-owner trusted daemon services for multi-agent engineering fleets.

With this release, Dendro transitions from a local flight recorder into an integrated, causal operational intelligence platform. Fleets running across Claude Code, OpenAI Codex, Google Antigravity, Grok, Pi, and orchestrators like OpenRig and AgentFlow can be inspected in real time, scrubbed across arbitrary historical intervals, and evaluated with rigorous factual accountability. In addition, the repository has transitioned to the Functional Source License (`FSL-1.1-Apache-2.0`).

---

## Highlights

### 1. Apple-Grade Connected Intelligence Studio (`dendro dashboard --serve`)

The studio introduces an immersive, fluid visual surface inspired by Apple design language and professional audio/video timelines:

- **The Living Growth Rings:** A centralized dynamic radial visualization displaying multi-agent fleet composition, token velocity, and health concentric rings that pulse with activity and react to time scrubs.
- **Tactile Time Machine Scrubber:** Fluid scrub timeline with one-touch presets (`1h`, `6h`, `24h`, `7d`, `30d`, `All Time`) alongside a dual-calendar Custom Date Range Picker that filters all telemetry, traces, and metrics consistently across the interface.
- **Apple Health Story Highlights:** 3 diagnostic metric cards highlighting top cost drivers, capability utilization bottlenecks, and runaway trajectory warnings with direct jump links to underlying causal layers.
- **4-Layer Exploding Stack X-Ray (`⌘E` / `Ctrl+E`):** An interactive 3D perspective breakdown slicing fleet telemetry into its 4 foundational strata:
  1. *Model Layer* (Tokens, costs, cache hit rates)
  2. *Harness Layer* (Sessions, agents, turns, tool execution)
  3. *Coordination Layer* (OpenRig pods, seats, handoffs, message lineages)
  4. *Environment & Security Layer* (Process resource consumption, sandbox policies, authority boundaries)

### 2. 5-Layer Capability & Execution Intelligence Matrix

Dendro v0.9.0 categorizes every agentic operation across 5 discrete capability tiers:

- **10 Behavioral Tool Families:** Deep classification across read, write, edit, command execution, web search, MCP calls, sub-agent invocation, scheduling, task management, and question asking.
- **Harness Primitives:** Native primitive classification for prompt turns, streaming chunks, tool execution, and context compactions.
- **Skills Declared vs. Invoked (`FUNNEL I06`):** Identifies loaded skills, evaluates activation rates, and reveals "zombie skills"—skills declared in prompts or configurations that consume context tokens without ever executing.
- **Model Context Protocol (MCP) Servers:** Tracks tool discovery, call latency, throughput, and error rates per MCP server endpoint.
- **Lifecycle Hooks:** Monitors pre-tool hooks, post-turn validators, and environmental traps.

### 3. 4 Active Intelligence Consoles (`I22`–`I25`) & Multi-Dimensional Rollups

Expands Dendro's analytical engine with four dedicated forensic consoles:

- **`I22` Trajectory Vortex Console:** Pinpoints cyclic loops, runaway prompt expansions, and unproductive agent reasoning spirals with visual spiral intensity diagrams.
- **`I23` Schema Drift Console:** Tracks protocol migrations, unexpected payload alterations, and deserialization warnings across evolving agent runtimes.
- **`I24` Authority Blast Radius Console:** Maps tool capabilities to security scopes, pinpointing filesystem escapes, unconstrained shell access, and credential leakage risks.
- **`I25` Counterfactual Model Routing:** Simulates fleet cost, latency, and token distributions if tasks had been dispatched to alternative models (e.g., Gemini Flash vs. Claude Opus vs. Grok Fast).
- **Multi-Dimensional Group-By Rollups:** Flexible slicing and aggregation across all 12 studio views by model, harness, provider, seat, pod, and status.

### 4. Offline Fleet Explorer Demo Dataset & Format Specification

- **Portable Fleet Package (`site/fixtures/fleet-explorer-demo.json`):** A zero-dependency, self-contained offline dataset enabling instant local exploration of fleet telemetry without requiring a running DuckDB daemon.
- **Formal Format Specification (`docs/OFFLINE_FLEET_EXPLORER_FORMAT.md`):** Complete schema documentation for exporting, archiving, and sharing deterministic fleet explorer packages.

### 5. Trusted Single-Owner Daemon (`dendro serve`) & Pre-Flight Advisor (`dendro advise`)

- **Single-Owner Daemon (`dendro serve`):** Background telemetry collector and local HTTP endpoint bound strictly to the local user session, guaranteeing private storage isolation and race-free DuckDB access.
- **Pre-Flight Advisor (`dendro advise`):** System diagnostics CLI command validating local platform prerequisites, directory permissions (`0700`), process sampling capability, and harness log readability.

### 6. License Update (`FSL-1.1-Apache-2.0`)

- Updated workspace licensing to the **Functional Source License 1.1** with Apache 2.0 conversion after two years (`LICENSE` and `NOTICE`).
- Added official status badges to `README.md` including the OpenRig integration badge (`Built-with: Openrig`).

---

## Upgrade & Parity Notes

- **Workspace Version:** All workspace members (`dendro-core`, `dendro-connectors`, `dendro-resolver`, `dendro-watcher`, `dendro-platform-policy`, `dendro-windows-private-io`, `dendro-cli`) are synchronized at `0.9.0`.
- **Database Schema:** Schema 5 remains the canonical storage format. Ingestion automatically initializes and upgrades local DuckDB instances without data loss.
- **Verification:** Passes all workspace tests (`cargo test --workspace`), doc parity checks (`cli_doc_parity`), Clippy (`cargo clippy --workspace -- -D warnings`), code formatting (`cargo fmt --check`), and web test suites (`node --test site/site.test.cjs`).
