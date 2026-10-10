<div align="center">
  <h1>🌲 Dendro</h1>
  <p><strong>Reading the rings of your AI agents.</strong></p>
  <p>
    <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License: MIT" /></a>
    <img src="https://img.shields.io/badge/rust-2021-orange.svg" alt="Rust 2021" />
    <img src="https://img.shields.io/badge/storage-DuckDB-yellow.svg" alt="DuckDB" />
    <img src="https://img.shields.io/badge/runtime-OpenRig-purple.svg" alt="OpenRig" />
  </p>
</div>

> *"Just as dendrochronology reveals climate history through tree rings, Dendro reads the session rings of your AI agents to uncover team topology, costs, and handovers."*

**Dendro** is a high-performance, local-first telemetry and analytics engine written in Rust. It unifies execution traces from coding agents (Claude Code, Codex, AGY, Grok, Pi) with runtime orchestrators (like [OpenRig](https://openrig.dev)), normalizes them into [DuckDB](https://duckdb.org), and models semantic layers for natural language exploration with [Nao](https://github.com/getnao/nao).

---

## The Three Core Pillars

Dendro solves the three fundamental observability questions for multi-agent squads:

1. **Seat Optimization:** Are you over-engineering agent seats? Identify idle times, task overlap, and seat convergence speed.
2. **Handover Efficiency:** Is context lost or bloated during handoffs? Track queue transitions, prompt token overhead, and rework rates.
3. **Resource & Cost Mastery:** Where is compute burning vs idling? Monitor token burn by mission, detect bash command retry loops, and surface watchdog alerts.

---

## Architecture

```
[Local Agents & Runtimes]
 ├── OpenRig (~/.openrig/openrig.sqlite, state/)
 ├── Claude Code (~/.claude/projects/.../*.jsonl)
 ├── Codex (~/.codex/sessions/, logs_2.sqlite)
 ├── AGY, Grok, Pi
        │
        ▼ (dendro watch / dendro run)
 [Dendro Engine (Rust)]
  • Watermark & Byte Offset Manager
  • Pluggable Connectors
  • Correlation Resolvers (Stitching Rigs <-> Sessions)
        │
        ▼
 [Local Storage (DuckDB)]
  • fct_sessions
  • fct_tool_calls
        │
        ▼
 [Conversational Analytics (Nao)]
  • "Which seat had the highest idle time this week?"
  • "Compare Claude Code vs Codex tool failure rates"
```

---

## Quick Start

### Build & Run
```bash
# Clone the repository
git clone https://github.com/smota/dendro.git
cd dendro

# Build all workspace crates
cargo build --release

# One-shot batch extraction into local DuckDB
dendro run --db dendro.duckdb

# Continuous background watching
dendro watch --db dendro.duckdb --debounce-secs 2

# Initialize Nao semantic layer
dendro init-nao --target . --db dendro.duckdb
```

---

## Cargo Workspace Structure

* `crates/dendro-core`: Canonical schemas (`CanonicalSession`, `CanonicalToolCall`), traits, and DuckDB storage engine.
* `crates/dendro-connectors`: Pluggable adapters for OpenRig, Claude Code, Codex, AGY, Grok, and Pi.
* `crates/dendro-resolver`: Extensible correlation resolvers (OpenRig explicit bindings & standalone mode).
* `crates/dendro-watcher`: Lightweight debounced file watcher daemon based on `notify`.
* `crates/dendro-cli`: Command-line executable (`dendro run`, `dendro watch`, `dendro init-nao`).

---

## License

MIT © 2026 [smota](https://github.com/smota)
