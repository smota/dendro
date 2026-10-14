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

## Codex command outcomes (local v0.2.0 candidate, #40)

The `report decisions` JSON payload keeps `tool_loops` and adds its sibling
`outcome_coverage`. Episodes retain session, tool and count, and add harness,
unit and seat. Claude/Pi units are tool calls. Codex native `CommandExecution`
items are command executions; only their own integer exit codes determine
success or failure. Codex seat-call outcomes remain `not read`, and observed
seat calls interrupt execution failure runs. Outputs never settle an outcome.
Commands never become structured writes or change intake endpoints.

Coverage describes the persisted complete prefix of each current source
generation. Zero sources, unread legacy prefixes, unsupported records, and
observed unknown fields remain distinct. Report-only opens never read sources
or backfill a database. Normal ingest backfills complete Codex prefixes without
recounting usage. Reader metadata, facts, watermark and session identity commit
atomically; metadata is excluded from observation loaders and the unit-aware
census, while privacy scans inspect every physical row.

Command arrays are compared using their original lexical JSON slices and a
keyed HMAC with fixed unit and array tags. String commands are currently unknown.
Raw input and output are never persisted. Ordering uses deterministic source
order, retains untimed anchors, and sorts timed segments between anchors. The
report window is applied first; qualifying endpoint spans must be nonnegative
and at most 600 seconds. A trusted Codex episode requires unique seat linkage.

The new reader can report a legacy schema-3 database with truthful unread
coverage, then backfill it on ingest. Older binaries interpreting a database
with the new observation units or reader metadata are semantically unsupported,
even though the physical schema version is unchanged. This describes a local
candidate; it does not announce a published release or a migration.
