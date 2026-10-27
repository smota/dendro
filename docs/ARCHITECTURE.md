# Dendro System Architecture Blueprint

Status: **Canonical vNext Architecture Blueprint**  
Engine: **DuckDB 2.0.0 LTS (`~1.20000.0`, storage version 69)**  
Product Schema: **Schema 5 (`MIGRATION_V5_TARGET_SCHEMA_VERSION = 5`)**  
Reference Standard: `DESIGN.md`, `ADDENDUM-DUCKDB2-AND-INTELLIGENCE.md`

---

## 1. Design Principles

Dendro is a local-first AI Fleet Intelligence engine written in **Rust**.

- **Operational Intelligence is the evidence layer:** Dendro measures seats, handoffs, tool loops, token accounting, and delegation graphs across local agent runs.
- **Fleet Governance stays with the operator:** Dendro computes evidence, advisory diagnostics, and anomaly indicators. It never autonomously alters agent prompts, kills sessions, or modifies execution routing.
- **Strict Privacy Boundaries:** No user prompts, agent thoughts, response texts, arguments, or secrets (`sk-`, `ghp_`, Bearer tokens) are persisted to DuckDB or exported to dashboards.
- **Single-Owner Concurrency:** The trusted daemon (`dendro serve`) acts as the exclusive read-write owner of the analytics database (`dendro.duckdb`). All client tools (CLI, TUI, HTTP, Nao conversational interface) communicate exclusively via typed, capability-bounded RPC. Raw SQL injection and unauthenticated network exposure are rejected by design.

---

## 2. Four-Domain Architecture & Domain Topology

Dendro connects execution evidence across four domains. Platform adapters link those records to the operator’s fleet topology:

```
┌────────────────────────────────────────────────────────────────────────┐
│                        DENDRO FLEET INTELLIGENCE                       │
│                                                                        │
│   ┌─────────────────────┐               ┌───────────────────────────┐  │
│   │    Model Domain     │               │      Harness Domain       │  │
│   │  Invocations (I01)  │               │    Capabilities (I06)     │  │
│   │    Tokens (I02)     │               │  Context Pressure (I07)   │  │
│   │  Inclusive (I03)    │               │     Tool Loops (I08)      │  │
│   │     Cost (I04)      │               │   Trajectory Vortex (I22) │  │
│   │  Routing Sim (I25)  │               │    Schema Drift (I23)     │  │
│   └──────────┬──────────┘               └─────────────┬─────────────┘  │
│              │                                        │                │
│              └───────────────────┬────────────────────┘                │
│                                  │                                     │
│   ┌──────────────────────────────┴──────────────────────────────┐      │
│   │             Coordination & Lineage Domain                   │      │
│   │  Delegation Chains (I09) · Duration (I10) · Queue (I12)     │      │
│   │      Acceptance (I19) · Descriptive Comparison (I21)        │      │
│   └──────────────────────────────┬──────────────────────────────┘      │
│                                  │                                     │
│   ┌──────────────────────────────┴──────────────────────────────┐      │
│   │             Environment & Security Domain                   │      │
│   │  Placement (I13) · Resources (I14) · Effects (I15)          │      │
│   │  Isolation (I16) · Authority (I17) · Blast Radius (I24)     │      │
│   └──────────────────────────────┬──────────────────────────────┘      │
│                                  │                                     │
│   ┌──────────────────────────────┴──────────────────────────────┐      │
│   │           Platform Adapters (OpenRig, AgentFlow)            │      │
│   │  Rig/Seat Binding, Queue Handoffs, Multi-Agent Topology     │      │
│   └─────────────────────────────────────────────────────────────┘      │
└────────────────────────────────────────────────────────────────────────┘
```

1. **Model Domain:** Token accounting (`cache_read`, `cache_write`, `input_uncached`, `output_tokens`, `reasoning`), call frequency, provider pricing tiers, and counterfactual model routing simulation.
2. **Harness Domain:** Capabilities, tool invocations, loop detection, heterogeneous envelopes via `VARIANT`, and behavioral vortex trajectory analysis.
3. **Coordination & Lineage Domain:** Inter-agent delegation graphs, queue item lifecycles, wall-clock concurrency, and work acceptance yields.
4. **Environment & Security Domain:** Host resource footprint (CPU, RSS, disk I/O), sandboxing posture, permission boundaries, credential scrubbing, and security blast radius.
5. **Platform Adapters (OpenRig, AgentFlow, Meshloop):** Connectors providing correlation between raw harness sessions and orchestrator roles, seats, missions, and queue items.

---

## 3. Storage Layer: Schema 5 & DuckDB 2.0.0 LTS

Dendro analytics uses **DuckDB 2.0.0 LTS** (`~1.20000.0`, storage version 69) with product schema target **Schema 5** (`MIGRATION_V5_TARGET_SCHEMA_VERSION = 5`).

### 3.1. Canonical Schema 5 Tables
The storage engine maintains the following authoritative canonical tables:

1. **`harness_envelope` (Schema 5 New):**
   Stores heterogeneous harness metadata using native DuckDB `VARIANT` sub-columns.
   Columns: `observation_id VARCHAR PRIMARY KEY`, `harness VARCHAR NOT NULL`, `harness_version VARCHAR NOT NULL`, `captured_at TIMESTAMP NOT NULL`, `envelope_shape VARIANT`.
   Unknown keys are dropped or HMAC-masked. Depths $> 8$ or sizes $> 4096$B are rejected.
2. **`_dendro_tripwire_outbox` (Schema 5 New):**
   Transactional outbox for active operational tripwires (`I22`).
   Columns: `id VARCHAR PRIMARY KEY`, `insight_id VARCHAR NOT NULL`, `subject_key VARCHAR NOT NULL`, `turn_k BIGINT NOT NULL`, `watermark BIGINT NOT NULL`, `payload_kind VARCHAR NOT NULL`, `created_at TIMESTAMP NOT NULL`, `dispatched_at TIMESTAMP`.
   Populated by an $O(1)$ statement trigger on write; processed asynchronously post-commit.
3. **`harness_trajectory` (Schema 5 New):**
   Vector sketches of capability and outcome state transitions.
   Columns: `observation_id VARCHAR PRIMARY KEY`, `harness VARCHAR NOT NULL`, `native_session VARCHAR NOT NULL`, `turn_k BIGINT NOT NULL`, `trajectory_vector FLOAT[32] NOT NULL`, `created_at TIMESTAMP NOT NULL`.
   Distance measured via exact `array_cosine_distance`.
4. **`_dendro_pricing_tiers` (Schema 5 New):**
   Operator-pinned model pricing profiles for counterfactual simulation (`I25`).
   Columns: `tier_id VARCHAR PRIMARY KEY`, `profile_pin VARCHAR NOT NULL`, `cohort_pin VARCHAR NOT NULL`, `currency VARCHAR NOT NULL`, `input_rate DOUBLE`, `output_rate DOUBLE`.
5. **`_dendro_redacted_diagnostics` (Schema 5 New):**
   Curated diagnostic taxonomy and remediation hints indexed for incremental full-text search.
   Columns: `diagnostic_id VARCHAR PRIMARY KEY`, `category VARCHAR NOT NULL`, `redacted_label VARCHAR NOT NULL`, `remediation_hint VARCHAR NOT NULL`, `updated_at TIMESTAMP NOT NULL`.
6. **`usage_facts`:**
   Atomic token consumption, reasoning output, cache efficiency, and source attribution per turn.
7. **`tool_facts`:**
   Tool execution records, category classification, failure tracking, and hashed input fingerprints.
8. **`message_facts`:**
   Message direction, role, turn sequences, and modality indicators (without prompt/response text).
9. **`identity_facts`:**
   Deterministic correlation between native harness session IDs and rig/seat identities.
10. **`activity_facts`:**
    Agent activity states (`active`, `idle_with_work`, `idle_without_work`, `waiting_on_human`).
11. **`queue_facts`:**
    Durable queue item lifecycles, handoff pairs, recovery events, and mission linkages.
12. **`skill_facts`:**
    Declared vs invoked skill evaluation per seat.
13. **`security_facts`:**
    Explicit security observations: tool authorizations, permission grants, and sandbox boundaries.
14. **`session_edges`:**
    Directed parent-child session relationships (`spawn`, `delegate`, `handoff`, `return`, `join`, `retry`).
15. **`process_identities`:**
    Operating system process metadata (PID, executable name, platform, isolation hint, start time).
16. **`process_samples`:**
    Periodic resource consumption snapshots (CPU time, RSS bytes, disk I/O).
17. **`file_source_baselines`:**
    Cryptographic and inode baselines for incremental ingestion and sealed replay verification.
18. **`warnings`:**
    Structured record of format anomalies, missing headers, or unrecognized logs.
19. **`dendro_meta`:**
    Database metadata storing the active `schema_version` and encryption/pseudonymization `key_id`.
20. **`dendro_semantic_state`:**
    Cache invalidation flag used to signal when Nao views require materialization.

---

## 4. Advanced Analytical Subsystems

### 4.1. Graph Engine (`lineage_graph_v1`)
Lineage analysis uses DuckDB's SQL:2023 Property Graph extension (`duckpgq`):
- Enabled via `SET active_grammar_extensions = ['duckpgq'];`.
- Defines property graph projections over `session_edges` and `security_facts`.
- **Evidenced-Only Invariant:** Traversal filters strictly on `status = 'evidenced'`. Edges with status `absent`, `truncated`, or `conflict` are ineligible for reachability.
- **Fixed Traversal Directions:**
  - Parent $\to$ Child: `spawn`, `delegate`, `delegate_existing`, `handoff`.
  - Child $\to$ Parent: `return`, `join`.
  - Prior Attempt $\to$ Later Attempt: `retry`.
- **Hop Ceiling:** Graph walks are bounded by an 8-hop limit to ensure real-time response.
- **Rust Fallback:** If `duckpgq` is absent, the pure Rust implementation in `crates/dendro-core/src/session_lineage.rs` provides 100% fixture parity.
- **Non-Claim:** Structural reachability measures dependency exposure, not inherited security authority.

### 4.2. Retrieval Subsystem (`retrieval_v1`)
Consists of three complementary mechanisms:
1. **Exact Vector Cosine Distance:** Calculates exact cosine similarity on `FLOAT[32]` vectors stored in `harness_trajectory` using `array_cosine_distance`. Experimental on-disk HNSW is not used for canonical state.
2. **Incremental Full-Text Search (`fts`):** Indexes curated diagnostic classes in `_dendro_redacted_diagnostics` via `PRAGMA create_fts_index(..., incremental := 1)`. User text is never indexed.
3. **DuckLake 1.1 Local Partitions:** Enables exporting immutable, pseudonymized Parquet partitions to a local export directory for cross-fleet analysis without network exposure.

### 4.3. Trusted Daemon (`dendro serve`)
Dendro provides single-owner concurrency via `dendro serve`:
- Only `dendro serve` opens the read-write DuckDB connection or embedded `quack` protocol SQL session.
- Clients connect via local IPC: a Unix domain socket (`0700`) on Linux/macOS or a Named Pipe (private DACL) on Windows.
- Unauthenticated loopback TCP is strictly prohibited.
- Same-user isolation is enforced by typed capability RPC checks rather than ambient process permissions. Client processes cannot execute raw SQL statements.

### 4.4. Advisory & Pre-flight Engine (`dendro advise`)
The advisory subsystem evaluates fleet operational health across four core questions:
1. `model_cohort_baseline`: Token and cost comparability across harness versions and models.
2. `skill_effectiveness_audit`: Verification of declared vs loaded vs verified effective skills.
3. `delegation_loop_risk`: Detection of cycles and excessive retry loops in subagent delegation.
4. `environment_readiness`: Assessment of process placement and isolation hints.
- **Warm Budget:** Answers are computed within a warm $p95 \le 15\text{ ms}$ budget. If the budget is exceeded, `advisory_budget_exceeded` is returned rather than partial advice.
- **Structured Uncertainty:** Encoded via the `AdviseUncertainty` enum (`sample_below_minimum`, `missing_denominator`, `noncomparable_mix`, `counterfactual_only`, `stale`).
- **Strictly Advisory:** Results are pull-only operator metadata; they are never injected into live agent context windows.

### 4.5. Connected Web Intelligence Studio

The browser studio presents the local evidence engine through a loopback server:

```sh
dendro dashboard --serve 127.0.0.1:4242 --site-dir ./site
```

The command serves the studio and typed JSON presentation endpoints on `127.0.0.1`, `localhost`, or `[::1]`. Wildcard and external binds are refused. This presentation interface is distinct from the trusted daemon's local IPC. Browser payloads follow the same zero-content privacy boundary as reports.

| Endpoint | Role |
| --- | --- |
| `/api/query` | Query projections for summaries, rows, insights, and groupings. |
| `/api/events` | Server-Sent Events invalidation stream that prompts a view refresh. |

The **Time Machine** resolves calendar presets and custom date ranges into UTC query bounds. Date-only custom end dates include the selected day; timestamp intervals remain half-open. The selected period travels across connected views. CLI relative windows retain their recorded-data anchoring and `window_resolution` disclosure.

| Surface | Representation | Operator use |
| --- | --- | --- |
| **Living Growth Rings** | `time_buckets` across Model, Harness, Coordination, and Environment. | Locate a change and follow Story Highlights into an investigation. |
| **4-Layer Stack X-Ray** | `xray_lanes` in swimlane or 3D isometric view (`⌘E`). | Trace relationships across execution layers. |
| **Dimension Explorer** | `groupings` over the selected view's available dimensions. | Pivot by harness, capability, date, workspace, or outcome. |
| **5-Layer Capability & Execution Intelligence** | `capability_intelligence`: `tool_families`, `primitives`, `skills`, `mcp`, `hooks`. | Compare available capabilities with observed execution. |

Capability intelligence spans **Tool Families**, **Harness Primitives**, **Skills**, **MCP Servers & Tools**, and **Lifecycle Hooks & Guardrails**. The behavioral families are Shell / Exec, Read / Inspect, Edit / Patch, Search / Grep, Subagent Spawn, MCP Call, Skill Load, Web / Fetch, Task / Queue, and Governance / Gate. Skill declarations and invocations remain distinct.

#### Analytical sources

Reference projections use `v_trajectory_vortex`, `v_schema_drift`, `v_authority_blast_radius`, and `v_counterfactual_routing`. These provide trajectory distance, versioned envelope drift, evidenced graph reachability, and operator-pinned routing simulations. The retrieval and graph sections above define their analytical contracts.

For live windows without materialized reference rows, connected projections can use labeled heuristics over tool, usage, message, and session-edge facts. A heuristic retains its own method and assumptions; it does not acquire the reference calculation's meaning. Dependency reachability describes exposure, and routing results remain simulations under operator control.

Offline semantic materialization has separate scope rules. The domain views `v_message_mix`, `v_subagent_lineage`, `v_security_posture`, and `v_process_resources` are all-time aggregates with snapshot window labels. The compatibility views `v_mission_token_burn`, `v_handoff_rework_analysis`, `v_seat_idle_ranking`, and `v_executive_governance` filter the materialized interval. See [Nao & Semantic Queries](NAO_SEMANTIC.md) for the complete contract.

---

## 5. Reference Insights Registry (`I01`–`I25`)

Dendro implements 25 reference insights across all four domains:

| ID | Identifier | Domain | Core Meaning |
|:---|:---|:---|:---|
| **I01** | `model.invocations` | Model | Distinct evidenced model invocations. |
| **I02** | `model.tokens` | Model | Cache reads, cache writes, fresh input, and output; reasoning is detail within output. |
| **I03** | `model.tokens.inclusive` | Model | Subgraph token consumption including downstream subagents. |
| **I04** | `model.cost` | Model | Verified financial expenditure from recorded currency or pinned tiers. |
| **I05** | `model.configuration_divergence` | Model | Divergence between declared and observed model parameters. |
| **I06** | `harness.capability_funnel` | Harness | Ratio of invoked capabilities to declared/loaded capabilities. |
| **I07** | `harness.context_pressure` | Harness | Ratio of active context window to declared context limits. |
| **I08** | `harness.tool_loops` | Harness | Repetitive failure episodes within bounded time windows. |
| **I09** | `harness.delegation_chains` | Harness | Multi-hop agent handoff chains and return friction. |
| **I10** | `work.duration` | Coordination | Wall-clock execution time and active concurrency. |
| **I11** | `work.shared_allocation` | Coordination | Resource consumption shared across concurrent missions. |
| **I12** | `coord.queue_wait` | Coordination | Time spent waiting in queue before processing begins. |
| **I13** | `environment.placement` | Environment | Host, container, or VM placement distribution. |
| **I14** | `environment.process_resources` | Environment | Operating system CPU, memory, and disk usage. |
| **I15** | `environment.effects` | Environment | System calls and side effects (requires privileged traces). |
| **I16** | `security.isolation` | Security | Process isolation and sandbox boundary enforcement. |
| **I17** | `security.authority` | Security | Authorization consistency across security boundaries. |
| **I18** | `security.redactions` | Security | Scrubbed credential tokens and sanitization events. |
| **I19** | `outcome.acceptance` | Coordination | Ratio of accepted work units to evaluated attempts. |
| **I20** | `outcome.resource_efficiency` | Coordination | Token and cost expenditure per accepted outcome. |
| **I21** | `comparison.descriptive` | Coordination | Stratified comparison across distinct cohorts or windows. |
| **I22** | `harness.trajectory_vortex` | Harness | Vector cosine distance divergence identifying behavioral loops. |
| **I23** | `harness.schema_drift` | Harness | Structural drift in metadata envelopes across harness versions. |
| **I24** | `security.authority_blast_radius` | Security | Downstream session reachability from failed/revoked permissions. |
| **I25** | `model.counterfactual_routing` | Model | Fast-tier counterfactual simulation based on cache bleed inflections. |

### Progressive Disclosure Contract (L0–L3)
All insight results are serialized into a standardized Answer structure:

- **L0:** Status, finding, practical meaning, scope, and missingness.
- **L1:** Assumptions, caveats, and alternative explanations.
- **L2:** Safe evidence keys ($\le 5$) and component version stamps.
- **L3:** Mathematical formula, population definition (`what`, `included`, `excluded`), exclusions, and cutoff timestamps.

Display status maps deterministically into 5 badge tokens: `ok`, `partial`, `insufficient_evidence`, `refused`, and `not_applicable`.
