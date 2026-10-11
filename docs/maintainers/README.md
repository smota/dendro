# Dendro — Maintainer Guide

Welcome to the internal maintainer guide for **Dendro**. This guide details the engineering architecture, Rust code conventions, concurrency model, lifecycle governance, and quality standards for the project.

---

## 1. Core Architectural Overview

Dendro is a local-first AI fleet intelligence and decision engine built in modular Rust on embedded **DuckDB**:

```
[Local Session Sources]
 ├── OpenRig (~/.openrig/openrig.sqlite, state/)
 ├── Claude Code (~/.claude/projects/*/*.jsonl)
 ├── Codex (~/.codex/sessions/**/rollout-*.jsonl)
 ├── AGY (~/.gemini/antigravity-cli/brain/**/transcript.jsonl)
 ├── Grok (~/.grok/sessions/**/summary.json)
 └── Pi (~/.pi/agent/sessions/**/*.jsonl)
       │
       ▼
 [dendro-connectors] ───► Implements `HarnessConnector` (zero-copy streaming via BufReader)
       │
       ▼
 [dendro-resolver]   ───► Implements `CorrelationResolver` (binds sessions to Rig, Seat, and Mission)
       │
       ▼
 [dendro-core]       ───► DuckDbStorage (idempotent migrations, transactional batch insertion)
       │
       ▼
 [dendro.duckdb]     ───► Dimensional and columnar analytical tables
```

---

## 2. Rust Engineering Standards

### 2.1 Performance & Memory Efficiency (Zero-Copy)
* **Streaming Large Log Files:** Harness logs (Claude Code, Codex, AGY) can reach hundreds of megabytes. **Never** load entire files into memory with `fs::read_to_string`. Consume streams line-by-line using `BufReader::new(file).lines()`.
* **Incremental Watermarks:** Every ingestion writes byte offsets and last-read timestamps to `_dendro_watermarks`. On subsequent runs, connectors seek directly to the offset (`seek(SeekFrom::Start(offset))`).
* **DuckDB Transactions:** Batch write operations must be wrapped within a single transaction (`BEGIN TRANSACTION` ... `COMMIT`). Avoid individual single-row inserts.

### 2.2 Security & Defense in Depth
* **Strict Safe Rust:** The use of `unsafe` blocks is strictly forbidden across all workspace crates (`#![deny(unsafe_code)]`).
* **SQL Injection Prevention:** Query parameters must use `duckdb::params![...]` or prepared statements. SQL string concatenation (`format!("SELECT ... WHERE id = '{}'")`) fails CI quality gates.
* **Secret Redaction:** Prompts and commands matching known credential patterns across 10 families (`sk-`, `ghp_`, `gho_`, `Bearer `, `xai-`) must be redacted before analytical persistence.

---

## 3. Cargo Workspace Structure

```
crates/
├── dendro-core/         # Canonical schemas (CanonicalSession, CanonicalToolCall), core traits, DuckDbStorage
├── dendro-connectors/   # Specialized parsing adapters per agent harness
├── dendro-resolver/     # Correlation and binding algorithms (OpenRigResolver, StandaloneResolver)
├── dendro-watcher/      # Real-time monitoring via notify-debouncer-mini
└── dendro-cli/          # Binary entry point and CLI commands (run, watch, status, init-nao)
```

---

## 4. Quality Gates & Validation Commands

Before submitting any candidate receipt or handing off to the reviewer:

```bash
# 1. Strict compilation check
cargo check --workspace

# 2. Test suite execution
cargo test --workspace

# 3. Linter with zero warnings tolerated
cargo clippy --workspace -- -D warnings

# 4. Canonical formatting
cargo fmt --check
```

---

## 5. Autonomous Governance & SDLC Protocols

This repository is governed under the **AgentFlow SDLC** in **autonomous** posture:
* **Branch Protection:** Never commit directly to `main`. All implementation occurs in dedicated worktrees under `.worktrees/<feature-name>`.
* **Strict Four-Eyes Gate:** Builders never approve their own pull requests (`allowsSelfReview: false`). Independent review seats in `rev.review` audit the final candidate commit SHA before merge.
