# Dendro — Product Site Builder Handover & Architecture Specification
**The Definitive Guide for Revamping the Dendro Product Showcase (`site/index.html` & Move the Needle `/dendro/`)**  
*Document Version:* `0.8.0` | *Target Engine:* Dendro v0.8.0 | *Audience:* Site Builder Agent, Design Engineers, Technical Marketers

---

## Table of Contents
1. [Executive Mission & Product Positioning ("What is Dendro?")](#1-executive-mission--product-positioning-what-is-dendro)
2. [Brand Language, Tone & Vocabulary Rules (Do's & Don'ts)](#2-brand-language-tone--vocabulary-rules-dos--donts)
3. [The Value Proposition & Competitive Differentiation ("Why Dendro Stands Alone")](#3-the-value-proposition--competitive-differentiation-why-dendro-stands-alone)
4. [The 8 Flagship Interactive Product Surfaces (The "WoW" Factors)](#4-the-8-flagship-interactive-product-surfaces-the-wow-factors)
5. [Ready-to-Use Visual Resources, Real Telemetry Stats & Design Tokens](#5-ready-to-use-visual-resources-real-telemetry-stats--design-tokens)
6. [Recommended Landing Page Information Architecture (9-Section Blueprint)](#6-recommended-landing-page-information-architecture-9-section-blueprint)
7. [Hard Guardrails: Test Invariants (`site/site.test.cjs`) & Epistemic Honesty Rules](#7-hard-guardrails-test-invariants-sitesitetestcjs--epistemic-honesty-rules)

---

## 1. Executive Mission & Product Positioning ("What is Dendro?")

### 1.1 Core Identity
**Dendro** is the **Local-First Flight Recorder, Causal X-Ray & Operational Intelligence Studio for Multi-Agent AI Fleets**.

When software engineers and engineering organizations deploy autonomous coding agents—whether running across multiple terminal panes, isolated git worktrees, or coordinated team topologies—execution becomes an opaque black box. Terminals scroll rapidly, git diffs appear, and token bills accumulate. Yet, operators are left blind to what actually happened:
- *Did those six agents converge cleanly, or did two roles spend four hours bouncing the same task back and forth in a ping-pong rework loop?*
- *Did a session stall because of a complex algorithmic problem, or was an agent sitting idle waiting 45 minutes for a human prompt while holding locks?*
- *Why did token burn spike 400% on a routine refactor? Was it cache-invalidation bleed, a runaway tool retry loop, or an inefficient sub-agent fanout?*

**Dendro reads the local session history your AI tools already write on your machine—turning scattered agent runs into a unified flight recorder and operational intelligence studio.** See where your budget goes, which tools and skills actually deliver, and where multi-agent handoffs slow down. Its open, extensible architecture lets you bring your own coding client, orchestration runtime, or capability mapper. Keep implementation details in the technical reference; lead product copy with what the user can see and improve.

```
       ┌─────────────────────────────────────────────────────────────┐
       │                 THE AUTONOMOUS AGENT STACK                  │
       ├─────────────────────────────────────────────────────────────┤
       │ 1. MODEL LAYER               Tokens, Cache Yield, Pricing   │
       │    ▲                                                        │
       │ 2. HARNESS LAYER             Tools, Context, Loops, Drift   │
       │    ▲                                                        │
       │ 3. COORDINATION LAYER        Handoffs, Lineage, Subagents   │
       │    ▲                                                        │
       │ 4. ENVIRONMENT & SECURITY    Sandboxes, OS CPU/RSS, Grants  │
       └─────────────────────────────────────────────────────────────┘
                                      │
                         Dendro Passive Flight Recorder
                                      ▼
       ┌─────────────────────────────────────────────────────────────┐
       │                   LOCAL DUCKDB ENGINE                       │
       │     (Zero Prompts · Zero Secrets · Sub-15ms Queries)        │
       └─────────────────────────────────────────────────────────────┘
                                      │
            ┌─────────────────────────┴─────────────────────────┐
            ▼                                                   ▼
┌───────────────────────┐                           ┌───────────────────────┐
│  RATATUI TUI COCKPIT  │                           │  CONNECTED WEB STUDIO │
│   (`dendro dashboard`)  │                           │   (`dendro dashboard`)│
└───────────────────────┘                           └───────────────────────┘
```

### 1.2 Supported Coding Clients, Export Adapters & Fleet Orchestration Runtimes

Keep three categories visible in product copy. AI coding clients execute the work; fleet orchestration runtimes coordinate multi-agent teams and handoffs across those clients.

| Stack component | Supported out of the box today | Coverage and extension path |
| --- | --- | --- |
| **Coding Clients** | **Claude Code, OpenAI Codex, Google Antigravity, Grok, and Pi** | Read their authorized local session history. Use full recognized names in prose. |
| **Editor/Agent Export Adapters** | Verified **Cursor, Windsurf, Goose, and Devin** exports | Coverage is limited to the documented [Supported Export Formats](docs/extensions/formats.md). |
| **Fleet Orchestration Runtimes** | **OpenRig**: rig topologies, persistent seats, durable queues, and handoff chains | Other runtimes, such as **AgentFlow** and **Meshloop**, also coordinate teams and handoffs; connect their available records through the extension contract rather than implying standalone adapters already ship. |

Use **Google Antigravity** throughout prose. OpenRig, AgentFlow, and Meshloop belong in the runtime category, separately from coding clients and export adapters. Present OpenRig `Rig` and `Seat` bindings as an optional integration within the broader fleet, session, actor, tool, and capability model.

For the **Extensible by Design — Bring Your Own Stack** section, show this support matrix and link directly to [Bring Your Own Client or Runtime](docs/extensions/README.md), [Supported Export Formats](docs/extensions/formats.md), and the [Contributing Guide](CONTRIBUTING.md). Include capability mappers in the extension invitation.

### 1.3 The Core Brand Metaphor: Dendrochronology Meets Apple Instruments & Apple Health
The name *Dendro* derives from **dendrochronology**—the scientific method of dating and interpreting environmental history through the growth rings of trees. 

Trees do not lie. A single cross-section of trunk reveals years of climate: wide rings mark periods of rapid, healthy growth; compressed rings reveal harsh droughts; dark, jagged notches (**fire scars**) permanently record forest fires, insect infestations, and physical trauma.

Dendro brings this exact empirical discipline to artificial intelligence:
- **Living Growth Rings:** 4 concentric, colored rings tiling 48 radial clock sectors representing a time window. Each ring maps to one architectural layer: Model (Warm Amber), Harness (Cyan), Coordination (Emerald), and Environment & Security (Violet).
- **Heartwood Core Readout:** The center of the concentric trunk. Scrubbing or hovering over any sector instantly displays layer name, sector timestamp, active tool/actor, and precise telemetry under the cursor.
- **Fire Scars:** Luminous anomaly notches along the rings that indicate tool retry loops (`I08`), context drops (`I07`), schema drift events (`I23`), or handoff rejections (`I19`).
- **Tactile Time Machine Scrubber:** A precision dual-handle scrubber with a 4-layer density histogram, sliding preset windows (`Today`, `Last Week · 7d`, `30d`, `This Month`, `All Time`), custom calendar popovers, and rounded date labels (`Oct 20 – Oct 27, 2026`).
- **4-Layer Exploding Stack X-Ray (`⌘E`):** With a single keystroke or click, the 2D timeline lifts into an interactive 3D isometric stack. Causal Bezier threads weave vertically from a Model token spike, down through a Harness tool loop, into a Coordination queue wait, down to the host OS process RSS footprint.

---

## 2. Brand Language, Tone & Vocabulary Rules (Do's & Don'ts)

### 2.1 Voice & Tone
- **Instrument-Grade & Empirical:** Tone should feel like Apple Instruments, Linear, or Stripe developer infrastructure. Confident, restrained, mathematically honest, beautifully crafted.
- **Root-Cause Driven:** Never speak in vague generalities ("improve AI workflows"). Speak in concrete engineering mechanics: "isolate a 14-minute handoff stall between Builder and Reviewer", "unmask a 96.9% cache hit efficiency vs. uncached token bleed".
- **Anti-Hype & Anti-Slop:** Absolutely forbidden to use AI vaporware cliches: "magic", "supercharge", "10x engineer", "revolutionary AI copilot", "seamless synergy". Dendro is a measurement instrument, not a chatbot.

### 2.2 What to Emphasize on the Landing Page
1. **User Outcomes Over Raw Busyness:** *"Activity is easy to fake. Flow is what ships software."* Six open terminals look impressive, but if four are waiting and two are looping, productivity is zero.
2. **Cross-Layer Causal Visibility:** Break down the artificial barrier between prompt cost, tool runtime, team handoffs, and sandbox security.
3. **Instant Multi-Dimensional Slicing:** Zero-latency pivots across Harness, Capability, Tool Family, Date, Team/Workspace, and Outcome.
4. **Zero-Configuration Local Discovery:** Zero agents to install, zero proxies to configure, zero network round-trips. Just point `dendro` at your machine and read history immediately.
5. **Local Privacy:** Source logs are read-only. No prompts, agent thoughts, code completions, or secrets (`sk-`, `ghp_`, Bearer tokens) are ever captured or stored. Everything runs locally on your machine—nothing is proxied or sent to a third-party cloud.

### 2.3 User Value Pillars

Lead with the user's daily reality: you see the final diff or monthly bill, but cannot explain a 40-minute task, a looping sub-agent, or a custom skill that never ran. Carry these four outcomes into the site:

- **Stop paying for invisible waste.** You can spot retry loops, uncached context refills, and runaway sub-agents before they drain your token budget.
- **Know which tools, skills, and MCP servers actually deliver.** You can see which capabilities help finish work, which fail silently, and which sit unused.
- **Pinpoint where multi-agent workflows stall.** You can follow available evidence through model time, tool failures, sub-agent handoffs, and human approval gates.
- **Test routing changes before you touch production.** You can simulate fast-tier models and context limits against recorded work before changing configuration.

Transition into **One studio across every layer of your stack**. Present the four architectural layers and five capability layers in separate tables with columns **Layer | What You See | What You Can Decide & Improve**; use the [README](README.md#architecture) as the copy reference.

### 2.4 Headline Vocabulary Matrix: Do's and Don'ts

| Feature / Concept | ❌ DO NOT USE (Forbidden in Headlines) | ✅ ALWAYS USE (Authorized Headline Language) |
| :--- | :--- | :--- |
| **Product Category** | "DuckDB telemetry dashboard", "AI observability log parser" | **"The Local-First Flight Recorder & Operational Intelligence Studio"** |
| **Actor Model** | "Multi-Rig monitor", "Seat occupancy tracker" | **"Fleet Intelligence across Sessions, Actors, and Harnesses"** *(Frame Rig/Seat as an adapter)* |
| **Session Analysis** | "Audit log viewer", "Chat history analyzer" | **"Causal X-Ray & Living Growth Rings"** |
| **Token Tracking** | "Token cost calculator", "LLM billing tracker" | **"5-Part Token & Cache Accounting"** *(96.9% cache read yield, uncached bleed)* |
| **Failure Detection** | "Bug finder", "AI hallucination detector" | **"Fire Scars & Trajectory Loop Tripwires"** *(Empirical distance, not speculative causes)* |
| **Lineage Tracking** | "Subagent tree viewer" | **"Multi-Hop Sub-Agent Lineage & Handoff Friction"** |
| **Security & Sandbox** | "Permission checker" | **"Authority Blast Radius & Process Footprint X-Ray"** |
| **Productivity Claims** | "Saves 20 hours/week", "Guaranteed 40% ROI" | **"Five Disjoint Seat Activity Bands: Measure Flow, Handoff Wait & Rework"** |

### 2.5 Product Owner Rule: Promote, Engage, and Inform — Never Over-Explain Screenshots or Dump Database Numbers
- **The Core Rule:** **"Promote, Engage, and Inform — Never Over-Explain Screenshots or Dump Database Numbers."**
- **Screenshots Speak for Themselves:** High-DPI product captures demonstrate real craft and operational truth. Captions, cards, and narrative copy must sell the *insight and decision unlocked*—never recite raw pixel counts (`6,848.31M`, `84,555`, `39,916,624`), internal row counts, or defensive SQL fallback modes (`Mode A / Mode B`, `intelligence_fallback`, `0.75 * observed`).
- **Focus on Decisions and Capabilities:**
  - *Living Growth Rings:* Reconstruct fleet execution across four layers, pin down anomalous fire scars, and navigate timeline history with the tactile scrubber.
  - *Exploding Stack X-Ray (`⌘E`):* Pull apart execution layers in 3D isometric space to trace cross-layer root causes in seconds.
  - *5-Layer Capability Matrix:* Reveal what capabilities agents actually execute across behavioral tool families, harness primitives, skills, MCP servers, and guardrails.
  - *Trajectory Vortex (`I22`):* Catch runaway prompt and tool retry loops early before they burn budget.
  - *Schema Drift (`I23`):* Spot breaking tool and harness changes instantly across agent updates.
  - *Authority Blast Radius (`I24`):* Map credential reachability and sandbox exposure with verified evidence.
  - *Counterfactual Routing (`I25`):* Test smarter model routing scenarios safely without touching live traffic.
- **Never Over-Explain:** Do not drag visitors into internal implementation details, heuristic formulas, or defensive caveats. State the analytical boundary cleanly and let the product's craft shine.

---

## 3. The Value Proposition & Competitive Differentiation ("Why Dendro Stands Alone")

### 3.1 3-Way Competitive Landscape Matrix

| Dimension | Cloud LLM Tracers<br>*(LangSmith, Arize, Helicone, Datadog)* | Raw Local Logs<br>*(JSONL files, `grep`, `jq`, bash)* | Dendro Local-First Studio<br>*(v0.8.0)* |
| :--- | :--- | :--- | :--- |
| **Data Privacy & Storage** | Transmits prompts, outputs, and code to third-party US cloud servers. | Local, but unindexed, uncompressed, and sprawling across dirs. | **100% Local-First DuckDB.** Zero prompts, thoughts, or secrets ever persisted. |
| **Execution Overhead** | Sits in the runtime network path. Adds 30–120ms latency to every prompt. | Zero runtime latency (passive file append). | **Zero overhead.** Out-of-band passive flight recording. Never touches the prompt wire. |
| **Multi-Agent Lineage** | Truncates at single LLM spans. Blind to terminal handoffs & worktrees. | Manual cross-referencing of timestamps and UUIDs across files. | **Full Causal Graph (`I09`).** Multi-hop sub-agent lineage, queue wait, and returns. |
| **Cross-Layer Breadth** | Model layer only (tokens and latency). Blind to OS and tools. | Blind to structure; requires complex custom scripts to join. | **4-Layer Unified Stack:** Model + Harness + Coordination + Environment/OS. |
| **Query & Slicing Speed** | Slow cloud UI; rate-limited queries; aggregation lag. | Painful `jq` scripts taking minutes over gigabyte logs. | **Sub-15ms local queries** via vectorized columnar DuckDB and Ratatui TUI. |
| **Loop & Anomaly Physics** | Post-facto cost alerts after token limits have already burned. | Discovered only when terminal freezes or budget limit is hit. | **Pre-Loop Distance tripwires (`I22`)** & Schema Drift detection (`I23`). |
| **Cost & Licensing** | $500–$3,000/mo SaaS subscription seat fees + usage egress. | Free, but high engineering toil. | **100% Free for Internal Use (FSL-1.1-Apache-2.0).** Runs on your own hardware forever. |

---

### 3.2 The Dual Taxonomy Breakthrough
Dendro's architectural breakthrough is its **Dual Taxonomy**: it splits telemetry across **4 Canonical Architectural Layers** and classifies agent actions through **Dendro's 5-Layer Execution & Capability Intelligence**.

```
┌────────────────────────────────────────────────────────────────────────────────────────┐
│                        DENDRO DUAL TAXONOMY ARCHITECTURE                               │
├────────────────────────────────────────────────────────────────────────────────────────┤
│ 1. FOUR CANONICAL ARCHITECTURAL LAYERS                                                │
│    ├── 1. MODEL LAYER (#F2A541 Warm Amber)                                             │
│    │      • 5-Part Token Breakdown: input_uncached, cache_write, cache_read, output,   │
│    │        reasoning (nested inside output, never double-counted).                   │
│    │      • Cache Read Yield: ratio-of-sums vs. mean-of-ratios (96.9% benchmark).     │
│    │      • Counterfactual Routing (I25): Fast-tier cache-bleed simulation.           │
│    │                                                                                  │
│    ├── 2. HARNESS LAYER (#06B6D4 Cyan)                                                │
│    │      • Tool Execution Loops (I08): Action failures, retries, thrashing.          │
│    │      • Context Window Pressure (I07): Compaction thresholds and drop markers.    │
│    │      • Trajectory Vortex (I22): 32-dim phase-space attractor loop tripwires.     │
│    │      • Schema Drift (I23): Key distribution L1 distance across harness versions. │
│    │                                                                                  │
│    ├── 3. COORDINATION LAYER (#10B981 Emerald)                                        │
│    │      • Sub-Agent Lineage Graphs (I09): Spawn, delegate, join, return trees.      │
│    │      • Work Handoff Friction: Queue wait times (I12) and rework bounces.         │
│    │      • 5 Disjoint Activity Bands: active, idle_with_work, idle_without_work,     │
│    │        waiting_on_human, unknown.                                                │
│    │      • Governance Acceptance Yield (I19): Accepted / rule-eligible evaluated.    │
│    │                                                                                  │
│    └── 4. ENVIRONMENT & SECURITY LAYER (#8B5CF6 Violet)                               │
│           • Process Footprint (I14): OS process CPU deltas and max sampled RSS bytes. │
│           • Sandbox Isolation (I16): Declared vs. observed runtime isolation.         │
│           • Secret Scrubbing (I18): Pattern-based sanitization of sk-, ghp_, tokens.  │
│           • Authority Blast Radius (I24): 8-hop dependency exposure reachability.     │
├────────────────────────────────────────────────────────────────────────────────────────┤
│ 2. DENDRO 5-LAYER EXECUTION & CAPABILITY INTELLIGENCE                                 │
│    ├── Layer 1: 10 Behavioral Tool Families (Shell, Read, Edit, Search, Spawn, etc.)  │
│    ├── Layer 2: Native Harness Primitives (Bash, Read, Edit, Write, Grep, Glob, Task) │
│    ├── Layer 3: Skills Funnel (FUNNEL I06: Declared vs. Loaded vs. Invoked)           │
│    ├── Layer 4: MCP Servers & Tools (Routing, latencies, failure rates)               │
│    └── Layer 5: Lifecycle Hooks & Guardrails (Pre/post execution policies)           │
└────────────────────────────────────────────────────────────────────────────────────────┘
```

---

## 4. The 8 Flagship Interactive Product Surfaces (The "WoW" Factors)

The site builder must highlight these 8 interactive surfaces across the product website and showcase:

### 1. The Living Growth Rings (`#dendro-growth-rings`)
- **Visual Mechanics:** A 420px concentric circular SVG rendering 4 layered rings tiled into 48 radial clock sectors.
- **Interactive Experience:** Rotary click-wheel scrub interaction. Dragging around the ring moves the playhead across the time window.
- **Heartwood Center HUD:** The center circle acts as a high-density digital readout showing active timestamp, current layer under cursor, dominant actor, and specific telemetry counts.
- **Fire Scar Anomaly Notches:** Visual luminous notches cut directly into the concentric bands marking detected loop anomalies, schema breaks, or failed runs.
- **Epistemic Honesty:** Unobserved or quiet time is explicitly rendered with diagonal hatch lines (`url(#container-hatch)`), never fabricated as zero activity.

### 2. Tactile Time Machine Scrubber
- **Visual Mechanics:** Sits directly above the dashboards. Combines a 4-layer stacked density histogram sparkline with dual draggable boundary handles.
- **Sliding Window Presets:** Single-click preset pills: `Today · 24h`, `Last Week · 7d`, `30 Days`, `This Month`, `Last Month`, `All Time`.
- **Custom Date Popover:** Calendar input allowing arbitrary `YYYY-MM-DD..YYYY-MM-DD` slices.
- **Clean Rounded Date Labels:** Formats timestamps as readable human spans (e.g., `Oct 20 – Oct 27, 2026`) rather than raw ISO strings.
- **Cross-View State Persistence:** Slices persist seamlessly across all 12 views via URL query parameters (`?window=7d`) and `localStorage`.

### 3. Apple Health "Story Highlights"
- **Visual Mechanics:** Located directly below the Growth Rings. 3 high-contrast, clean summary cards inspired by Apple Health narrative trends.
- **Cards & Micro-Charts:**
  1. *Model Cache Efficiency Ring:* Circular gauge showing `96.9%` cache read efficiency, pairing uncached input vs. cache read tokens in plain English.
  2. *24h Multi-Harness Rhythm Strip:* Multi-colored pulse strip tracking turn frequency and message pace across Claude Code, OpenAI Codex, Google Antigravity, Grok, and Pi.
  3. *7-Column Loop Conditions Strip:* Weather-style column strip showing daily loop conditions (clear, rework, or vortex trips) across the fleet.

### 4. 4-Layer "Exploding Stack X-Ray" (`⌘E`)
- **Visual Mechanics:** The crown jewel interactive visualization. Pressing `⌘E` (or clicking *Explode 3D Stack*) lifts flat 2D timeline swimlanes into an isometric 3D perspective stack.
- **Causal Bezier Threads:** Illuminated vertical cubic Bezier curves connect events across the 4 planes. A single click highlights how a token burst in the Model Layer triggered 14 tool retries in the Harness Layer, which caused a 30-minute queue block in Coordination, driving 2.4 GB RSS memory consumption in Environment.
- **Inspectable Nodes:** Clicking any node reveals safe evidence IDs, L0–L3 disclosure levels, and underlying insight metrics.

### 5. Dendro's 5-Layer Execution & Capability Intelligence Studio
- **Visual Mechanics:** A sleek, dark Bento grid combining a **Proportion Spectrum Bar** with 5 switchable capability tabs.
- **10 Behavioral Tool Families Grid:**
  1. `Shell / Exec` — Sandboxed CLI execution
  2. `Read / Inspect` — Code and diff inspection
  3. `Edit / Patch` — File modifications
  4. `Search / Grep` — Codebase symbol/regex search
  5. `Subagent Spawn` — Delegated worker processes
  6. `MCP Call` — Model Context Protocol tools
  7. `Skill Load` — Procedural skill activations
  8. `Web / Fetch` — Network requests
  9. `Task / Queue` — Handoffs and task updates
  10. `Governance / Gate` — Reviewer approval gates
- **Skills Funnel Badge (`FUNNEL I06`):** Reports declared skills vs. loaded skills vs. verified effective invoked skills.

### 6. Multi-Dimensional Group-By & Rollup Matrix
- **Visual Mechanics:** Instant columnar pivot engine embedded in the Hub and across all 12 views.
- **Zero-Latency Group-By Modes:**
  - `By Harness / Tool` (Calls, tokens, execution share)
  - `By Capability / Tool Family` (Execution breakdown)
  - `By Date (Daily)` (Historical volume progression)
  - `By Workspace / Team` (Multi-project attribution)
  - `By Status / Outcome` (Clean completions vs. rework)
  - `Raw Records` (Evidenced telemetry rows)

### 7. 4 Bespoke Active Intelligence Consoles (`I22`–`I25`)
Four specialized scientific visualization consoles:
- **`I22` Trajectory Vortex:** *Orbital Phase-Space Attractor.* Plots multi-turn trajectories in an elliptical phase space. Loops pull trajectories away from the stable center into expanding vortex orbits before token budgets burn.
- **`I23` Schema Drift:** *DNA Fingerprint Ribbon.* Visualizes key distribution drift across harness versions as a biological DNA gel-strip, flagging added, removed, and retyped envelope keys.
- **`I24` Authority Blast Radius:** *Radial Propagation Ripple Graph.* Computes 8-hop dependency exposure from a failed or revoked permission grant, visualizing affected downstream sessions in concentric ripples.
- **`I25` Counterfactual Model Routing:** *Split-Beam Route Comparator.* Interactive dual-beam comparator allowing operators to slide context-pressure thresholds and simulate the financial and latency impact of routing sessions to faster, cheaper model tiers.

### 8. Ratatui Terminal Cockpit (`dendro dashboard`) & Headless Services
- **6-Screen TUI Cockpit:** A pure-Rust terminal interface built with Ratatui, featuring 6 navigation tabs (`Overview`, `Layers`, `Handoffs`, `Capabilities`, `Consoles`, `Inspection`), responsive layouts, and zero-flicker rendering.
- **Local Services:** `dendro serve` provides trusted local IPC. `dendro dashboard --serve 127.0.0.1:4242` serves the connected web studio, SSE streams (`/api/events`), and JSON query projections (`/api/query`).
- **Warm Pre-Flight Advisor (`dendro advise`):** Sub-15ms pull-only advisory oracle that answers session health checks before an agent begins work.
- **Deterministic Semantic Queries (`dendro ask`):** In-process Nao semantic model router translating plain-English queries into deterministic DuckDB aggregations across 6 bounded intents without fuzzy LLM hallucinations.

---

## 5. Ready-to-Use Visual Resources, Real Telemetry Stats & Design Tokens

### 5.1 Real High-DPI Screenshot Catalog
All 8 production screenshots are available at `/home/sam/code/dendro/site/assets/screenshots/*.png` (and `/home/sam/code/dendro/docs/assets/screenshots/*.png`). They are crisp, high-DPI (1480px width) captures of the live engine operating over real multi-agent fleets:

| Filename | Resolution | Real Telemetry & Stats Visible in Capture | Recommended Landing Page Role & Caption |
| :--- | :--- | :--- | :--- |
| `hub-living-growth-rings.png` | **1480 × 1300** | • **Oct 20 – Oct 27, 2026** (7d slice)<br>• **4 Concentric Rings** (Model, Harness, Coordination, Env)<br>• **0 Vortex Loops**, **0 Schema 5 Drift**<br>• Status: **AUTONOMOUS FLEET HEALTH · NOMINAL**<br>• Apple Health 3-Story Highlight Strip | **Hero Interactive Preview / Core Feature #1**<br>*"The Living Growth Rings: Reconstruct fleet history with the tactile time scrubber and surface critical bottlenecks instantly through health story highlights."* |
| `hub-stack-xray-3d.png` | **1480 × 1520** | • **`⌘E` 3D Exploded Stack Mode**<br>• Causal Bezier correlation threads linking layers<br>• Multi-Dimensional Group-By Explorer<br>• Inspectable node attributes | **Feature Showcase #2 / Architecture Deep-Dive**<br>*"The 4-Layer Exploding Stack: Pull apart execution across all four architectural layers in flat or 3D isometric views to trace root causes in seconds."* |
| `hub-capability-studio.png` | **1480 × 1680** | • **5-Layer Capability Matrix** badge<br>• **FUNNEL I06** badge (`harness.capability_funnel`)<br>• Fleet Execution Spectrum Bar<br>• 10 Behavioral Tool Families + `ToolArchetype` tiles<br>• Declared vs. Invoked vs. Dormant Skills | **Capability Section / Tool Execution Intelligence**<br>*"5-Layer Capability Matrix: Deconstruct fleet execution across all five capability layers to see which tools, skills, and MCP servers drive real outcomes."* |
| `console-trajectory-vortex.png` | **1480 × 1520** | • **I22 Active Console** (`harness.trajectory_vortex`)<br>• Completed multi-turn trajectories analyzed<br>• Phase-space attractor turn-length & self-transition tracking<br>• Real-time loop tripwire defense | **Active Consoles Grid / Loop Defense**<br>*"Trajectory Vortex (`I22`): Catch runaway agent loops early before they burn budget."* |
| `console-schema-drift.png` | **1480 × 1500** | • **I23 Active Console** (`harness.schema_drift`)<br>• Historical metadata cohort progression<br>• L1 distance banding across Claude Code, OpenAI Codex, Google Antigravity, Grok, and Pi<br>• Breaking structural mutation alerts | **Active Consoles Grid / Fleet Reliability**<br>*"Schema Drift (`I23`): Spot breaking tool and harness changes instantly across version updates."* |
| `console-blast-radius.png` | **1480 × 1520** | • **I24 Active Console** (`security.authority_blast_radius`)<br>• Multi-hop lineage dependency trees walked<br>• Radial propagation ripple graph<br>• Evidenced credential reachability boundary | **Active Consoles Grid / Security & Sandboxing**<br>*"Authority Blast Radius (`I24`): Map security reachability and blast radius with verified evidence."* |
| `console-counterfactual-routing.png` | **1480 × 1680** | • **I25 Active Console** (`model.counterfactual_routing`)<br>• Split-Beam Route Comparator<br>• Fast-tier simulation vs. observed baseline<br>• Safe context threshold explorer | **Active Consoles Grid / FinOps & Optimization**<br>*"Counterfactual Routing (`I25`): Test smarter model routing scenarios safely without touching live routers."* |
| `console-token-economics.png` | **1480 × 1700** | • Multi-agent session telemetry<br>• 5-part token breakdown (input, cache write, cache read, output, reasoning)<br>• 96.9% cache read efficiency (ratio of sums)<br>• Multi-dimensional group-by pivots across actor, model, and task | **Operational Economics / FinOps Section**<br>*"Operational Economics: Honest 5-part token breakdown and 96.9% cache read yield across multi-agent workflows."* |

### 5.2 Supporting Media Assets
- **Logo / Mark:** `site/assets/mark.svg` (SVG mark), `site/assets/logo.jpg` (High-res brand asset)
- **Video & Motion:**
  - `site/assets/dendro-demo.mp4` & `docs/assets/dendro-demo.mp4` (Full interactive demo walkthrough)
  - `site/assets/dendro-preview.gif` & `docs/assets/dendro-preview.gif` (Quick animated preview loop)
- **Documentation Guides:** [Visual Product Tour](docs/VISUAL_PRODUCT_TOUR.md) and [Studio Walkthrough](docs/VIDEO_WALKTHROUGH.md). Place the linked GIF/video at the start of the visual tour and high on the landing page. Use the current Connected Intelligence Studio recording; media production is owned separately from this handover.
- **Shared Stylesheet:** `site/dashboards/style.css` (Canonical design tokens and dark mode styling)

### 5.3 Design System Palette & CSS Tokens

```css
:root {
  /* Canvas & Obsidian Surfaces */
  --bg-obsidian: #0A0D12;       /* Base dark canvas background */
  --bg-canvas: #0B0D0E;         /* Deepest viewport canvas */
  --bg-surface: #11161F;        /* Elevated card / panel background */
  --bg-surface-elevated: #161C27; /* Popovers, tooltips, modals */
  --border-subtle: #1F2735;     /* Structural gridlines */
  --border-highlight: #2A3649;  /* Hover / active borders */
  
  /* Text & Typography */
  --text-primary: #F3EDE2;      /* Warm white primary headings */
  --text-secondary: #9BA3AF;    /* Muted body text and metadata */
  --text-muted: #64748B;        /* Footnotes, inactive labels */
  --font-mono: 'JetBrains Mono', 'Fira Code', monospace;
  --font-sans: 'Inter', system-ui, -apple-system, sans-serif;
  
  /* The 4 Architectural Layer Colors */
  --layer-model: #F2A541;       /* Warm Amber (Tokens, Cost, Cache) */
  --layer-harness: #06B6D4;     /* Electric Cyan (Tools, Loops, Context) */
  --layer-coord: #10B981;       /* Emerald Green (Handoffs, Lineage, Queues) */
  --layer-env: #8B5CF6;         /* Vivid Violet (OS Resources, Sandbox, Security) */
  
  /* Operational Status Badges */
  --status-ok: #10B981;         /* Nominal / Computed */
  --status-partial: #F59E0B;    /* Threshold exceeded / Partial */
  --status-insufficient: #64748B; /* Insufficient evidence / sample */
  --status-refused: #EF4444;    /* Refused / Missing baseline */
  --status-not-applicable: #475569; /* Not assessed / Not applicable */
  
  /* Glassmorphism & Elevation */
  --glass-bg: rgba(17, 22, 31, 0.75);
  --glass-border: 1px solid rgba(255, 255, 255, 0.08);
  --glass-shadow: 0 20px 40px -15px rgba(0, 0, 0, 0.6);
}
```

---

## 6. Recommended Landing Page Information Architecture (9-Section Blueprint)

This section outlines the recommended architecture for a world-class revamp of `site/index.html`:

```
┌────────────────────────────────────────────────────────────────────────┐
│ 1. GLOBAL NAVIGATION BAR                                              │
│    Brand Mark + "v0.8.0" Pill + Nav Links + "ILLUSTRATIVE SNAPSHOT"    │
├────────────────────────────────────────────────────────────────────────┤
│ 2. HERO SECTION & LIVE SCREENSHOT SWITCHER                             │
│    Headline + Lead + Primary CTA Buttons + 8-Tab Screenshot Stage      │
├────────────────────────────────────────────────────────────────────────┤
│ 3. THE PROBLEM: "ACTIVITY IS EASY TO FAKE. FLOW SHIPS SOFTWARE."       │
│    The Illusion of Busyness vs. Five Disjoint Activity Bands           │
├────────────────────────────────────────────────────────────────────────┤
│ 4. THE METAPHOR: LIVING GROWTH RINGS & TIME MACHINE                   │
│    420px Concentric Ring Demo + Apple Health Story Highlights          │
├────────────────────────────────────────────────────────────────────────┤
│ 5. THE 4-LAYER EXPLODING STACK X-RAY (`⌘E`)                            │
│    Interactive 3D Perspective Exploder + Causal Bezier Threads         │
├────────────────────────────────────────────────────────────────────────┤
│ 6. CAPABILITY & EXECUTION STUDIO (5-Layer Capability Matrix)          │
│    10 Tool Families Bento Grid + Skills Funnel (I06) + MCP Monitoring  │
├────────────────────────────────────────────────────────────────────────┤
│ 7. ACTIVE INTELLIGENCE CONSOLES (I22–I25)                             │
│    2×2 Interactive Showcase of Vortex, Drift, Radius & Routing         │
├────────────────────────────────────────────────────────────────────────┤
│ 8. CAPACITY SCENARIO CALCULATOR                                        │
│    Interactive Sliders & Real-Time Math (Zero Cloud Benchmarks)        │
├────────────────────────────────────────────────────────────────────────┤
│ 9. 60-SECOND QUICKSTART & TERMINAL ECOSYSTEM                          │
│    One-Line Install + TUI Cockpit Preview + Loopback Trust Philosophy  │
└────────────────────────────────────────────────────────────────────────┘
```

### Detailed Section Breakdown

#### Section 1: Global Navigation Bar
- **Left:** Dendro Mark (`site/assets/mark.svg`) + "Dendro" + `v0.8.0` product badge.
- **Center:** Navigation links: `Overview`, `How It Works`, `Dashboards` (`dashboards/index.html`), `Fleet Explorer` (`viewer.html`), `GitHub`.
- **Right:** Status pill: `● ILLUSTRATIVE SNAPSHOT` (or live sync badge when attached) + Quickstart CTA button.

#### Section 2: Hero Section & Interactive Screenshot Switcher
- **Eyebrow:** `THE LOCAL-FIRST FLIGHT RECORDER FOR MULTI-AGENT AI FLEETS · FSL-1.1-APACHE-2.0`
- **Headline:** `Read the growth rings of your AI workflows.`
- **Subheadline:** `Six open agent terminals look productive—even when four are waiting on a handoff and two are trapped in a loop. Dendro turns existing local session logs into clear, causal answers about where time, handoffs, and token budget actually go.`
- **Action Buttons:**
  - `Launch Fleet Explorer →` (`viewer.html`)
  - `View Connected Dashboards` (`dashboards/index.html`)
  - `Quickstart Guide` (`#quickstart`)
- **Coding Client Pills:** Claude Code · OpenAI Codex · Google Antigravity · Grok · Pi.
- **Export Adapter Pills:** Cursor · Windsurf · Goose · Devin; link to verified format coverage.
- **Fleet Orchestration Runtime Row:** OpenRig (built-in integration). Explain AgentFlow and Meshloop as other coordination runtimes with extension paths.
- **Extensibility CTA:** Bring Your Own Stack; link to the extension guide, supported formats, and contributing guide.
- **Walkthrough:** Put the [animated preview](docs/assets/dendro-preview.gif), linked to the [full video](docs/assets/dendro-demo.mp4) and [walkthrough](docs/VIDEO_WALKTHROUGH.md), before the screenshot stage.
- **Hero Visual:** An **Interactive 8-Tab Screenshot Stage**. Visitors can click across 8 tabs (*Growth Rings*, *Stack X-Ray*, *Capability Studio*, *Trajectory Vortex*, *Schema Drift*, *Blast Radius*, *Counterfactual Routing*, *Token Economics*) to view full high-DPI screenshots paired with interactive callouts and live telemetry stats.

#### Section 3: The Core Shift: "Activity is Easy to Fake. Flow Ships Software."
- **Contrast Grid:**
  - *Without Dendro (The Illusion of Busyness):* Terminals scroll fast while work quietly stalls; ping-pong rework loops burn budgets; handoff delays go unnoticed; cloud invoices arrive weeks late without attribution.
  - *With Dendro (Retrospective Causal Truth):* Five disjoint seat activity bands separate actual work from handoff waits and rework; multi-hop lineage exposes friction; 100% private local discovery.
- **The Five Disjoint Activity Bands Diagram:** Visual bar breakdown showing `Active Work` (Emerald), `Waiting on You` (Cyan), `Handoff Wait` (Amber), `Stalled / Rework` (Red), and `Cleanly Parked` (Slate).

#### Section 4: The Core Metaphor: Living Growth Rings & Time Machine
- **Interactive SVG Mockup:** 420px Growth Rings with rotatable hover sectors.
- **Time Machine Controls:** Sliding window presets (`Today`, `7d`, `30d`, `All`), custom date popover, and rounded calendar labels (`Oct 20 – Oct 27, 2026`).
- **Apple Health Story Highlights Cards:** 
  1. *Cache Yield:* 96.9% cache efficiency.
  2. *Harness Rhythm:* 24h pacing strip.
  3. *Loop Weather:* Clear days vs. vortex trips.

#### Section 5: The 4-Layer Exploding Stack X-Ray (`⌘E`)
- **Interactive Stack Exploder Visual:** Interactive toggle allowing visitors to flatten or explode the 4 layers in 3D isometric perspective.
- **Causal Bezier Story:** Visualizing vertical causality:
  `Model Invocations → Harness Tool Loops → Coordination Queue Stalls → OS Memory Footprint`.
- **4 Layer Spec Cards:** Model (`#F2A541`), Harness (`#06B6D4`), Coordination (`#10B981`), Environment & Security (`#8B5CF6`).

#### Section 6: Capability & Execution Studio (Dendro's 5-Layer Execution & Capability Intelligence)
- **Proportion Spectrum Bar:** Colored composition of observed fleet actions.
- **10 Behavioral Tool Families Bento:** Interactive grid with icons and real counts for `Shell/Exec`, `Read/Inspect`, `Edit/Patch`, `Search/Grep`, `Subagent Spawn`, `MCP Call`, `Skill Load`, `Web/Fetch`, `Task/Queue`, and `Governance/Gate`.
- **Skills Funnel (`FUNNEL I06`):** Explaining the critical distinction between declared skills, loaded skills, and verified effective skills.

#### Section 7: The 4 Active Intelligence Consoles (`I22`–`I25`)
- **2×2 Scientific Showcase Grid:**
  1. *Trajectory Vortex (`I22`):* Orbital phase-space physics detecting loops before token burn.
  2. *Schema Drift (`I23`):* Biological DNA gel-strip tracking structural envelope shifts.
  3. *Authority Blast Radius (`I24`):* 8-hop radial ripple graph of credential dependency exposure.
  4. *Counterfactual Routing (`I25`):* Split-beam cache-bleed comparator.
- **Active Telemetry Badge:** Clear status pill indicating live local telemetry without defensive implementation jargon.

#### Section 8: Capacity Scenario Calculator
- **Interactive Live Sliders:** Matches the exact mathematical model in `site/app.js`:
  - Number of Active Actors (`seatsInput`, e.g., 4)
  - Monthly Missions (`missionsInput`, e.g., 25)
  - Avg Tokens per Mission (`tokensPerMissionInput`, e.g., 400k)
  - Estimated Handoff Rework Rate (`reworkRateInput`, e.g., 25%)
  - Blended Cost per 1M Tokens (`tokenCostInput`, e.g., $5.00)
  - Engineering Hours per Rework Loop (`hoursInput`, e.g., 2.0h)
  - Target Rework Reduction (`reductionInput`, e.g., 20%)
- **Real-Time Output Cards:**
  - Expected Handoff Returns (`reworkChainsVal`)
  - Diverted Rework Tokens (`reworkTokensVal`)
  - Recoverable Token Spend (`reworkCostVal`)
  - Engineering Hours Reclaimed (`hoursSavedVal`)
  - Fleet Capacity Unlocked (`capacityVal`)
  - Missions per Actor Load (`seatLoadVal`)
- **Disclaimer Callout:** *"All calculations represent operator-supplied assumptions. Dendro never uses hidden cloud averages, synthetic ROI formulas, or speculative productivity benchmarks."*

#### Section 9: 60-Second Quickstart & Terminal Ecosystem
- **Build and Open the Studio:** Follow the current [README quickstart](README.md#quickstart). Show the clone/build commands, `dendro run`, and `dendro dashboard --serve 127.0.0.1:4242 --site-dir ./site`. State that source build time depends on the machine.
- **Terminal Alternative:** `dendro dashboard --auto-ingest` opens the cockpit; `dendro status` checks coverage and `dendro ask "where do tokens go?"` explores recorded work.
- **Advanced Usage:** Link to the [Operator Guide](docs/adoption/README.md) and [CLI Reference](docs/CLI_R1.md) for source roots, storage, fixed windows, exports, and daemon setup.
- **Footer:** GitHub link, FSL-1.1-Apache-2.0 notice, Move the Needle umbrella links, and software version `v0.8.0`.

---

## 7. Hard Guardrails: Test Invariants (`site/site.test.cjs`) & Epistemic Honesty Rules

The Site Builder Agent **must not break** any existing test assertions. Every test in `site/site.test.cjs` is an immutable CI gate.

### 7.1 Required Test Invariants & DOM Contract

```javascript
// Verification script: node --test site/site.test.cjs
```

1. **Capacity Scenario Calculator DOM IDs (Required by `site/app.js` and `site.test.cjs`):**
   - **Input IDs:**
     - `seatsInput`
     - `missionsInput`
     - `tokensPerMissionInput`
     - `reworkRateInput`
     - `tokenCostInput`
     - `hoursInput`
     - `reductionInput`
   - **Output Value IDs:**
     - `seatsVal`
     - `missionsVal`
     - `tokensPerMissionVal`
     - `reworkRateVal`
     - `tokenCostVal`
     - `hoursVal`
     - `reductionVal`
     - `reworkChainsVal`
     - `reworkTokensVal`
     - `reworkCostVal`
     - `hoursSavedVal`
     - `capacityVal`
     - `seatLoadVal`
   - **Scenario Math Integrity:** Must preserve the exact fractional calculations of `modelScenario()`:
     ```javascript
     returned = missions * reworkPercent / 100;
     tokenMillions = returned * tokensK / 1000;
     cost = tokenMillions * costPerMillion;
     hours = returned * hours * reduction / 100;
     capacity = reworkPercent * reduction / 100;
     missionsPerSeat = missions / seats;
     ```

2. **Offline Zero-CDN Rule (Strict Airgap Compatibility):**
   - **No Remote Dependencies:** Absolutely NO external `<script>`, `<link>`, `<img>`, or `<iframe>` referencing `http://` or `https://`.
   - **No Google Fonts:** `site/index.html` and `site/viewer.html` must **never** reference `fonts.googleapis.com` or `fonts.gstatic.com`. Use system fonts and local font stacks.
   - **Local Media Preservation:** `site/assets/logo.jpg`, `site/assets/dendro-demo.mp4`, and `site/assets/dendro-preview.gif` must exist and maintain file sizes `> 0`.

3. **Mandatory Navigation Links & Versioning:**
   - `site/index.html` must contain a direct link to `dashboards/index.html`.
   - `site/index.html` must display the canonical version string `v0.8.0`.
   - `site/viewer.html` must preserve `<script id="dendro-data" type="application/json">null</script>`.

4. **Obsidian Palette Consistency:**
   - Both `site/dashboards/style.css` and `site/viewer.html` must use:
     - `--bg-obsidian: #0A0D12`
     - `--bg-surface: #11161F`
   - Neither file may contain legacy color hexes `#090d16` or `#0f1927`.

### 7.2 Mandatory Epistemic Honesty Rules

Dendro's primary brand asset is its absolute mathematical and philosophical honesty. The landing page must scrupulously enforce these rules:

1. **`Null ≠ Zero` Rule:**
   - A missing table, unobserved interval, or unmaterialized metric is `null` or `not_observable`, **never zero**.
   - Zero represents an evidenced, observed count of zero events. An unobserved interval is rendered with diagonal hatch marks or labeled `insufficient_evidence`.
   - The UI displays five distinct status badges: `ok`, `partial`, `insufficient_evidence`, `refused`, and `not_applicable`.

2. **`Tokens ≠ Productivity` Rule:**
   - Never correlate high token volume with high engineering output. A 20M token session might represent a stuck agent thrashing in a git rebase loop.
   - Always frame token accounting as **operational cost attribution**, not developer value or productivity.

3. **`Correlation ≠ Causation` in Stack X-Ray:**
   - Vertical Bezier threads connecting Model tokens, Harness tool loops, Coordination queue waits, and OS RSS memory represent **temporal and causal correlation**, not mathematical proof of causation.

4. **Clear Telemetry Attribution for Consoles `I22`–`I25`:**
   - Consoles clearly distinguish live telemetry from simulation models without defensive jargon.
   - `I25` must explicitly state: **`SIMULATED TIER TOTAL (COUNTERFACTUAL)`** in normalized units, accompanied by the disclaimer: *"Simulation proxy, not a live routing mutation or financial guarantee."*

5. **`Scenario Calculator ≠ Predicted ROI`:**
   - The capacity calculator must be clearly labeled as an **Operator Assumption Explorer**, never as an automated benchmark or guaranteed return on investment.

6. **100% Privacy & Zero-Exfiltration Guarantee:**
   - Prompts, agent thoughts, response texts, tool arguments, tool outputs, and error bodies are **never persisted**.
   - Known token prefixes (`sk-`, `ghp_`, `gho_`, `xai-`, Bearer tokens) are aggressively redacted before storage.

---

### 7.3 Complete Verification Command Suite

Before claiming completion of any site revamp, the Site Builder Agent must run and pass all three verification suites:

```bash
# 1. Site contract, DOM IDs, zero-CDN, and offline viewer integrity tests
node --test site/site.test.cjs

# 2. Rust CLI and documentation parity tests
cargo test -p dendro-cli --test cli_doc_parity

# 3. DuckDB semantic models and Nao view verification
cargo test -p dendro-core --test semantic
```

---

*Handover document authored by **Product Site Handover Architect**. Ready for immediate consumption by the Site Builder Agent.*
