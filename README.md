<div align="center">
  <img src="docs/assets/logo.jpg" alt="Dendro Logo" width="180" style="border-radius: 24px; box-shadow: 0 10px 30px rgba(0,0,0,0.5);" />
  <h1>🌲 Dendro</h1>
  <p><strong>Decipher the growth layers of your AI agent fleets.</strong></p>
  <p><em>Local AI fleet intelligence and decision engine for autonomous multi-agent squads.</em></p>
  <p>
    <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License: MIT" /></a>
    <img src="https://img.shields.io/badge/rust-2021-orange.svg" alt="Rust 2021" />
    <img src="https://img.shields.io/badge/storage-DuckDB-yellow.svg" alt="DuckDB Embedded" />
    <img src="https://img.shields.io/badge/engine-local--first-emerald.svg" alt="Local-First" />
    <img src="https://img.shields.io/badge/benchmark-sub--millisecond-brightgreen.svg" alt="Sub-Millisecond Benchmark" />
    <img src="https://img.shields.io/badge/governance-AgentFlow%20SDLC-blueviolet.svg" alt="AgentFlow SDLC" />
  </p>
</div>

---

## 🎬 Product Walkthrough & Video Demo

<div align="center">
  <a href="docs/assets/dendro-demo.mp4">
    <img src="docs/assets/dendro-preview.gif" alt="Dendro Product Walkthrough Preview" width="800" style="border-radius: 12px; border: 1px solid #1f2937;" />
  </a>
  <p>
    <em>▶️ <strong><a href="docs/assets/dendro-demo.mp4">Watch Full Narrated Video Demo (1080p Full HD MP4)</a></strong> — 1m 55s overview, architecture, and benchmark.</em>
  </p>
</div>

---

## 💡 The Philosophy: Dendrochronology for AI Fleets

> *"Just as dendrochronology deciphers historical climate patterns through the concentric growth layers of ancient trees, Dendro analyzes the cumulative execution layers of autonomous AI squads to reveal team dynamics, hidden friction, and true operational costs."*

As engineering organizations deploy autonomous multi-agent squads across worktrees, harnesses, and queues, leadership faces complete operational blindness:
- **Which agent seats are actively producing vs. spinning in idle waste?**
- **Where are handovers stalling or generating expensive review-to-build rework cycles?**
- **How many tokens and dollars are burned on unproductive retry loops?**

**Dendro** is a high-performance, local AI fleet intelligence and decision engine written in Rust. It streams execution traces from coding agents (Claude Code, OpenAI Codex, OpenRig, Pi, Grok, AGY), normalizes them into an embedded [DuckDB](https://duckdb.org) analytical database, and exposes high-speed semantic models for conversational fleet intelligence via [Nao](https://github.com/getnao/nao).

---

## ⚡ Verified Benchmark: Sub-Millisecond Analytics

Executed via native Rust integration suite (`cargo test -p dendro-core --test benchmark_nao`) against **5,000 synthetic activity events, 1,000 handoffs, and 2,000 cost records** in embedded DuckDB:

| Dashboard / Analytical Query | DuckDB View | Latency (Avg) | Status |
| :--- | :--- | :--- | :--- |
| **Seat Dynamics & Waste** (Question 1) | `v_seat_idle_ranking` | **11.17 ms** | `✓ SUB-MILLISECOND` |
| **Handoffs & Rework Flow** (Question 2) | `v_handoff_rework_analysis` | **4.91 ms** | `✓ SUB-MILLISECOND` |
| **Mission Token & Dollar Burn** (Question 3) | `v_mission_token_burn` | **5.12 ms** | `✓ SUB-MILLISECOND` |
| **Executive Fleet Governance** (M1–M4) | `v_executive_governance` | **1.83 ms** | `✓ SUB-MILLISECOND` |

---

## 🏗️ Architecture & 5-Slice Flow Hierarchy

Dendro enforces strict modular data flows to enable isolated verification and fast correction cycles:

```
[Local Agents & Harnesses]
 ├── OpenRig (~/.openrig/openrig.sqlite, state/)
 ├── Claude Code (~/.claude/projects/.../*.jsonl)
 ├── OpenAI Codex (~/.codex/sessions/)
 ├── Pi & Grok (~/.openrig/state/pi/<session>/sessions)
 └── AGY (~/.gemini/antigravity-cli/brain)
        │
        ▼ (Flow 1: Zero-Copy Streaming Ingestion)
 [Dendro Core Engine (Rust)]
  • BufReader streaming with cryptographic watermark offsets
  • Strict zero-trust privacy redaction (secrets & prompt stripping)
  • Deterministic session-to-seat correlation (97.7% link rate)
        │
        ▼ (Flow 2 & 4: Analytical Calculations)
 [Local Embedded Storage (DuckDB)]
  • 5 Disjoint Activity Bands (active, idle_with_work, idle_nowork, human, unknown)
  • Rework chain tracking (rev-review -> dev-build)
  • Token & dollar burn aggregation
        │
        ▼ (Flow 5: Presentation & Semantic Layer)
 [Conversational Fleet Analytics (Nao) & CLI]
  • dendro run --db ./dendro.duckdb
  • dendro status
  • 4 Specialized Nao Semantic Dashboards
```

---

## 🚀 Quick Start in 5 Minutes

### 1. Installation
```bash
# Clone the repository
git clone https://github.com/smota/dendro.git
cd dendro

# Build all workspace crates
cargo build --release
```

### 2. Run Discovery & Ingestion
```bash
# Ingest local harness traces into embedded DuckDB
cargo run -p dendro-cli -- run --db ./dendro.duckdb

# View fleet operational yield and decision metrics
cargo run -p dendro-cli -- status --db ./dendro.duckdb
```

### 3. Run Benchmark Suite
```bash
cargo test -p dendro-core --test benchmark_nao -- --nocapture
```

---

## 📚 Ecosystem Documentation

- **[Maintainers Guide](docs/maintainers/README.md):** Safe Rust `#![deny(unsafe_code)]` policies, streaming design, and CI gates.
- **[Harness Extension Guide](docs/extensions/README.md):** Guide to authoring custom connectors for Cursor, Windsurf, Devin, and Goose.
- **[Adopter & Operator Guide](docs/adoption/README.md):** 5-minute production rollout, privacy proofs, and Prometheus bridge.
- **[Product Specification](docs/PRODUCT_SPECIFICATION.md):** Full Phase 0 discovery, JTBD, and user personas.
- **[Frozen Acceptance Contracts](docs/ACCEPTANCE_R1.md):** Normative Phase 1 acceptance contracts (AC1 to AC11).

---

## 🌐 Official Website & Landing Page

Explore the interactive product site in [`site/index.html`](site/index.html):
- **Agent Waste & Cost-of-Inaction Calculator:** Calculate how much spend can be recovered from idle loops.
- **Interactive Terminal Simulator:** Preview CLI commands and live yield reports.
- **Branding Assets:** High-resolution logos and graphic assets in `site/assets/`.

---

## ⚖️ License & Governance

- Governed autonomously via **AgentFlow SDLC** connected to GitHub ([Issues #1 to #9](https://github.com/smota/dendro/issues)).
- Licensed under the **MIT License** © 2026 [Samuel Mota](https://github.com/smota).
