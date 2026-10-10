# AGENTS.md — Engineering Standards, Security & Autonomous Operations

Welcome to **Dendro**. This repository is governed for fully autonomous, multi-agent software delivery.

Every agent operating in this repository (Claude, Codex, Grok, Pi, AGY) must adhere to the engineering standards and behavioral guidelines described below.

---

## 1. Project Context & Mission

Dendro is a local-first telemetry, observability, and correlation engine for multi-agent fleets written in **Rust**.
* **Storage:** DuckDB (`dendro.duckdb`) colunar engine.
* **Orchestrator Integration:** OpenRig (`~/.openrig`) correlating Rigs, Seats, and Missions with low-level execution logs.
* **Harnesses:** Claude Code, Codex, AGY, Grok, Pi.
* **Conversational Layer:** Nao (`getnao/nao`) via pre-configured semantic models.
* **License:** MIT.

---

## 2. Autonomous Delivery & Governance (AgentFlow SDLC)

* **Posture:** **100% Autonomous (`autonomous`)**.
* **Zero Human Blocking:** All changes, including critical scope and high-assurance contracts, proceed without human intervention or manual sign-off.
* **Strict Four-Eyes Principle (No Self-Review):** Self-review is strictly forbidden (`allowsSelfReview: false`). Every implementation created by a builder agent (Grok) MUST be independently audited and reviewed by another agent (Codex in `rev.review`).
* **Proof Over Promises:** Validate every change with automated verification (`cargo check`, `cargo test`, `cargo clippy`). Record reproducible evidence into the SDLC run logs.
* **Squad Topology:**
  * **`dendro-dev` (Delivery Squad):**
    * Lead Architect & Coordinator: **Codex** (`orch.arch`)
    * Builders (Jr, Pleno, Sr): **Grok** via Pi (`dev.build-jr`, `dev.build`, `dev.build-sr`)
    * QA & Verification: **Codex** (`dev.qa`)
    * Independent Reviewer: **Codex** (`rev.review`)
  * **`dendro-pm` (Product Squad):**
    * Product Manager: **Claude Code** (`pm.manager`)
    * Business Analyst: **Claude Code** (`pm.analyst`)

---

## 3. Rust Engineering Standards: Performance & Security

### 3.1. Performance Best Practices

1. **Memory Efficiency & Zero-Copy:**
   * Favor borrowed references (`&str`, `&[u8]`) and `Cow<'a, str>` over eager cloning (`.clone()`, `.to_string()`).
   * When parsing high-volume JSONL session logs, stream line-by-line using buffered readers (`BufReader::lines()`). Do not read entire gigabyte files into memory at once.
2. **Watermarking & Incremental Processing:**
   * Always record byte offsets and timestamps in `_dendro_watermarks`.
   * On subsequent runs or file watcher events, perform `seek(SeekFrom::Start(offset))` to consume only appended bytes.
3. **Database Performance (DuckDB & SQLite):**
   * Reuse connections and prepared statements.
   * Group writes into transactional batches (`conn.execute_batch` or wrapped transactions) rather than executing individual single-row inserts in loops.
4. **Async & Concurrency (Tokio):**
   * Never block the Tokio async runtime with CPU-heavy loops or blocking file I/O; use `tokio::task::spawn_blocking` when interacting with blocking synchronous libraries.
   * Use bounded channels (`tokio::sync::mpsc::channel(buffer_size)`) to ensure backpressure and prevent unbounded memory growth.
   * Debounce file events (default: 1.5s - 2s) to prevent thrashing during rapid agent token streaming.

---

### 3.2. Security Best Practices

1. **Safe Rust Policy:**
   * Prohibit `unsafe` code blocks across all workspace crates (`#![deny(unsafe_code)]`). Any exception requires an audited rationale.
2. **SQL Injection Prevention:**
   * **Mandatory parameterized queries:** Always use `duckdb::params![...]` and `rusqlite::params![...]`.
   * **Never** construct SQL statements through string concatenation or interpolation (`format!("SELECT ... WHERE id = '{}'", input)`).
3. **Path Traversal & Symlink Defense:**
   * When discovering or opening session files in `~/.claude/`, `~/.codex/`, `~/.openrig/`, validate path containment to prevent directory traversal outside authorized roots.
   * Check for symlink cycles and verify canonical paths.
4. **Secret & Credential Sanitization:**
   * Session logs may inadvertently contain environment variables, API keys, or OAuth tokens from user commands.
   * Redact known token prefixes (`sk-`, `ghp_`, `gho_`, `xai-`, Bearer tokens) before persisting commands or transcripts to DuckDB.
   * Never log, commit, or export auth files (`~/.claude/settings.json`, `~/.codex/auth.json`, etc.).

---

## 4. Verification & Quality Gates

Before finalizing any change:
```bash
cargo check --workspace
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo fmt --check
```
