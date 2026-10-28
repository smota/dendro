# Extensibility & Bring-Your-Own-Component Guide

Dendro is built from the ground up as a modular, local-first intelligence engine for AI coding agents and autonomous fleets. Whether you are adding support for a new AI coding harness, integrating an autonomous multi-agent orchestration runtime, expanding the 5-layer capability taxonomy, or building bespoke diagnostic views, Dendro provides strict, bounded extension contracts designed for high throughput, memory safety, and analytical integrity.

---

## Out-of-the-Box Coverage Today

Dendro ingests and correlates execution telemetry across three distinct operational tiers:

```
┌────────────────────────────────────────────────────────────────────────┐
│                   DENDRO CONNECTED INTELLIGENCE STUDIO                 │
└────────────────────────────────────┬───────────────────────────────────┘
                                     │
     ┌───────────────────────────────┼───────────────────────────────┐
     ▼                               ▼                               ▼
┌─────────────────────────┐  ┌─────────────────────────┐  ┌─────────────────────────┐
│   Native AI Clients     │  │   Export Adapters       │  │ Orchestration Runtimes  │
├─────────────────────────┤  ├─────────────────────────┤  ├─────────────────────────┤
│ • Claude Code           │  │ • Cursor CLI            │  │ • OpenRig (~/.openrig)  │
│ • OpenAI Codex          │  │ • Windsurf (Cascade)    │  │ • AgentFlow SDLC        │
│ • Google Antigravity    │  │ • Block Goose           │  │ • Meshloop Graph        │
│ • Grok                  │  │ • Devin (ATIF-v1.7)     │  │                         │
│ • Pi                    │  │ (See formats.md)        │  │ (Multi-seat topology)   │
└─────────────────────────┘  └─────────────────────────┘  └─────────────────────────┘
```

1. **Native AI Coding Clients:**
   Direct zero-copy streaming ingestion from local session logs for **Claude Code** (`~/.claude`), **OpenAI Codex** (`~/.codex`), **Google Antigravity** (`~/.gemini`), **xAI Grok** (`~/.grok`), and **Pi** (`~/.pi`).
2. **Verified Editor & Autonomous Agent Export Adapters:**
   Explicit export readers for **Cursor** (`stream-json`), **Windsurf / Cascade** (`post_cascade_response_with_transcript` NDJSON), **Block Goose** (legacy session JSONL), and **Devin** (`ATIF-v1.7`). For full format specifications and limitations, see [**`docs/extensions/formats.md`**](formats.md).
3. **Fleet Orchestration Runtimes:**
   Correlation engines that bind isolated client sessions to multi-agent fleet topology: **OpenRig** (`~/.openrig`), **AgentFlow SDLC**, and **Meshloop**, linking individual terminal sessions to lasting agent seats, durable task queues, and cross-seat delegation trees.

---

## The 4 Ways to Extend Dendro

### 1. Bring a New AI Coding Client or Harness (`HarnessConnector`)

All harness connectors implement the core `HarnessConnector` trait defined in `crates/dendro-core/src/traits.rs`.

#### Trait Definition

```rust
use anyhow::Result;
use async_trait::async_trait;
use dendro_core::{CanonicalEvent, Watermark};
use std::path::{Path, PathBuf};

#[async_trait]
pub trait HarnessConnector: Send + Sync {
    /// Returns the unique canonical name of the harness (e.g. "myharness").
    fn name(&self) -> &'static str;

    /// Discovers candidate log or export files within an authorized root directory.
    fn discover_files(&self) -> Result<Vec<PathBuf>>;

    /// Incrementally extracts canonical events from a source file given an optional watermark.
    /// Returns extracted events and an updated, non-rewound Watermark.
    fn extract_incremental(
        &self,
        path: &Path,
        watermark: Option<&Watermark>,
    ) -> Result<(Vec<CanonicalEvent>, Watermark)>;
}
```

#### Complete Runnable Example

A complete runnable example is available at [`examples/custom_connector.rs`](../../examples/custom_connector.rs). Run it from the workspace root:

```sh
cargo run --locked -p dendro-connectors --example custom_connector -- docs/extensions/fixtures target/custom-example.key
```

#### Implementation Lifecycle Contract

1. **Deterministic Discovery (`ExportRoot::discover`):**
   Use `dendro_connectors::sdk::ExportRoot::discover`. It selects direct regular `.jsonl` files deterministically, enforces path containment, skips symlinks, and never recurses through arbitrary parent or configuration directories.
2. **Incremental Bounded Batch Reading (`ExportRoot::read_batch`):**
   Seek to the accepted watermark offset, read up to 128 complete lines, bound each line to 1 MiB, and retain partial trailing lines for subsequent turns without dropping bytes.
3. **Projecting Core Telemetry Facts:**
   Parse raw events into normalized DuckDB relational facts:
   - `usage_facts`: Model token deltas (`prompt_tokens`, `completion_tokens`, `cached_tokens`), duration, and cost USD. Never add both a total and its components. Missing counters mean `null`, never fabricated zero.
   - `tool_facts`: Tool invocations mapped to fixed behavioral archetypes (`read_file`, `write_file`, `edit_file`, `shell`, `mcp_tool`, `other`) with execution duration and exit outcomes (`success`, `failure`, `timeout`, `veto`). Never store unbounded raw arguments or code payloads in fact tables.
   - `message_facts`: Turn index, role (`user`, `assistant`, `system`), and turn timestamp.
   - `security_facts`: Sandbox boundary violations, credential regex alerts (`sk-`, `ghp_`, `Bearer`), or permission approval prompts.
4. **Watermarking & Prefix Integrity:**
   Store file path hashes and byte offsets in `_dendro_watermarks`. Before consuming appended lines, verify the prefix hash against the previously recorded watermark. Refuse modified or truncated prefixes.

---

### 2. Bring a Fleet Orchestration Runtime Adapter

While single-session harnesses capture *what one agent executed*, orchestration runtimes capture *how multiple agents collaborate*.

An orchestration adapter maps multi-agent coordination records into Dendro's topology tables:

1. **Identity & Seats (`identity_facts`):**
   Map native harness session IDs (UUIDs or hashes) to lasting organizational identities:
   - `rig`: Team / workspace identifier (e.g. `test-rig`, `marketing-squad`).
   - `seat`: Durable functional role (e.g. `orch-arch@test-rig`, `dev-build@test-rig`).
   - `occupant_model`: The specific model occupying the seat during that turn (e.g. `claude-3-7-sonnet`, `gemini-2.5-pro`).
2. **Durable Queue Handoffs (`queue_facts`):**
   Capture work transitions across seats:
   - `task_id`: Unique persistent work item ID.
   - `from_seat` $\to$ `to_seat`: Delegator and receiver addresses.
   - `handover_status`: `ENQUEUED`, `CLAIMED`, `ACCEPTED`, `REJECTED`, `CLOSED`.
   - `latency_ms`: Queue wait time and turn turnaround latency.
3. **Parent-Child Session Edges (`session_edges`):**
   Ingest parent-child subagent trees (`parent_session_id`, `child_session_id`, `invocation_role`). This feeds the **Authority Blast Radius (`I24`)** and **Trajectory Vortex (`I22`)** consoles.

---

### 3. Extend the 5-Layer Capability Matrix

Dendro's Capability Studio organizes agent actions into 5 distinct architectural tiers:

```
┌────────────────────────────────────────────────────────┐
│  Layer 5: Lifecycle Hooks & Guardrails (Security)      │
├────────────────────────────────────────────────────────┤
│  Layer 4: MCP Protocol & Servers (External Tools)      │
├────────────────────────────────────────────────────────┤
│  Layer 3: Skills Architecture (Declared vs Invoked)    │
├────────────────────────────────────────────────────────┤
│  Layer 2: Harness Primitives (read_file, shell, etc.)  │
├────────────────────────────────────────────────────────┤
│  Layer 1: Behavioral Tool Families (10 Archetypes)     │
└────────────────────────────────────────────────────────┘
```

When integrating a new capability or tool provider:
1. **Tool Families (Layer 1):** Map custom tool identifiers into one of the 10 canonical behavioral archetypes:
   - `inspector_diff` (code diffs, git status, read inspections)
   - `editor_writer` (file writing, targeted editing, refactoring)
   - `terminal_exec` (bash execution, build commands, test runs)
   - `context_graph` (semantic retrieval, memory lookups, vector queries)
   - `delegator_spawn` (subagent spawning, task handoffs)
   - `web_browser` (DOM navigation, visual screenshots, page evaluations)
   - `policy_validator` (linting, schema validation, compliance checks)
   - `mcp_protocol` (dynamic JSON-RPC tools over stdio/SSE)
   - `notification_comm` (chat messages, user confirmations)
   - `system_control` (daemon management, process signaling)
2. **Skills Funnel (Layer 3 & Insight I06):**
   Record declared skills in `skill_declarations` and map actual executions to `skill_facts`. The studio automatically calculates activation yield rates, flagging dormant skills that waste prompt tokens.
3. **MCP Protocols (Layer 4):**
   Record tool calls under namespaced formats (`namespace:server_name:tool_name`) so the studio separates core runtime primitives from third-party MCP capabilities.

---

### 4. Add a New Diagnostic Insight or Console View

Dendro pairs DuckDB SQL analytical views with an Apple-grade Connected Intelligence Studio.

#### Step 1: Define the Semantic Query & Insight Rule
1. Add an insight definition file under `templates/nao/dashboards/` or extend `crates/dendro-core/src/analytics/`:
   ```yaml
   insight_id: "I26"
   title: "Subagent Recursion Depth & Thrashing"
   severity: "WARNING"
   query: |
     SELECT
       parent_session_id,
       COUNT(child_session_id) AS child_count,
       MAX(depth) AS max_depth,
       AVG(duration_ms) AS avg_duration_ms
     FROM session_edges
     GROUP BY parent_session_id
     HAVING MAX(depth) > 3;
   ```
2. Register the insight with Dendro's conversational query engine (`dendro ask "show deep recursion loops"`).

#### Step 2: Build the High-Contrast Glass Console View
1. Create a responsive HTML5/SVG console under `site/dashboards/views/my_new_insight.html`.
2. Follow Dendro's design system tokens in `site/dashboards/style.css`:
   - Surface colors: Dark luxury background (`#0A0D12`), subtle frosted borders (`rgba(255, 255, 255, 0.08)`).
   - Accents: Emerald (`#10B981`) for normal health, Amber (`#F59E0B`) for warnings, Crimson (`#EF4444`) for critical anomalies, Cyan (`#06B6D4`) for temporal controls.
   - Dynamic Time Machine integration: Listen to `?window=` query parameters (`7d`, `30d`, `all`, `custom`) and hydrate via `fetch('/api/v1/...')` or embedded DuckDB JSON exports.
3. Register the console tile in `site/dashboards/index.html` inside `#views-container` (Apple Bento Console Dock).

---

## Conformance & Quality Gates

Before submitting any connector or extension:

```sh
# 1. Check workspace compilation including connector examples
cargo check --locked --workspace --examples

# 2. Run unit and integration test suite
cargo test --locked --workspace

# 3. Ensure zero Clippy warnings across all targets
cargo clippy --locked --workspace --all-targets -- -D warnings

# 4. Enforce canonical code formatting
cargo fmt --check

# 5. Run UI test suite
node --test site/site.test.cjs
```
