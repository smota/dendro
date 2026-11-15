# Dendro Data Dictionary: Schema 5, Reference Insights & Intelligence Catalog

Status: **Canonical Schema 5 & Intelligence Reference**  
Target Engine: **DuckDB 2.0.0 LTS (`~1.20000.0`, storage version 69)**  
Product Schema Version: **`MIGRATION_V5_TARGET_SCHEMA_VERSION = 5`**  
Governing Standard: `DESIGN.md`, `ADDENDUM-DUCKDB2-AND-INTELLIGENCE.md`

---

## 1. Schema 5 Engine & Storage Foundation

Dendro analytics runs on **DuckDB 2.0.0 LTS** (`~1.20000.0`, storage version 69).
- **Target Schema Version:** Schema 5 (`MIGRATION_V5_TARGET_SCHEMA_VERSION = 5`), tracked in `dendro_meta.schema_version`.
- **DuckDB Storage Version:** Storage version 69 for live database cutover; historical backups (`dendro.duckdb.pre-v200.bak`) retain storage version 68 for compatibility with v0.8.0 DuckDB 1.5.6.
- **Single-Owner Concurrency:** The trusted daemon (`dendro serve`) is the single owner opening raw DuckDB / `quack` connections. All client processes (CLI, TUI, HTTP, Nao, and agent hooks) communicate via typed RPC over private transport (Unix domain socket mode `0700` or Windows Named Pipe with owner/SYSTEM/Administrators DACL). Arbitrary SQL execution is strictly forbidden.
- **Privacy & Security Constraints:**
  - No raw prompts, responses, tool parameter values, or credential secrets are ever persisted.
  - Token prefixes (`sk-`, `ghp_`, `gho_`, `xai-`, Bearer tokens) are redacted before storage.
  - Unknown keys in structured metadata are HMAC-masked or dropped.
  - Directories are secured at `0700` and files at `0600` (POSIX) or restricted DACL (Windows).

---

## 2. Canonical Schema 5 Tables

### 2.1. `harness_envelope` (Schema 5 New)
Heterogeneous, shredded, body-free harness metadata (MCP capability flags, tool argument types, and provider counters).
```sql
CREATE TABLE harness_envelope (
    observation_id VARCHAR PRIMARY KEY,
    harness VARCHAR NOT NULL,
    harness_version VARCHAR NOT NULL,
    captured_at TIMESTAMP NOT NULL,
    envelope_shape VARIANT
);
```
- **Grain:** One row per observed harness invocation envelope.
- **`envelope_shape` (`VARIANT`):** DuckDB 2.0 native `VARIANT` shredded into columnar sub-columns on write. Queried via `variant_keys` and `variant_type`.
- **Allowlist & Key Sanitization:** Keys must be present in the versioned harness key allowlist. Unknown keys are dropped or HMAC-replaced with the private HMAC key. Secret-shaped key names are refused.
- **Shape Bounds:** Nesting depth $\le 8$, encoded size $\le 4096$ bytes, $\le 64$ keys, string tokens $\le 64$ bytes, arrays $\le 16$ items. Values are booleans, numbers, closed tokens, or key paths—never prompt text or tool values.

### 2.2. `_dendro_tripwire_outbox` (Schema 5 New)
Transactional notification outbox for active operational tripwires (`I22` vortex detection).
```sql
CREATE TABLE _dendro_tripwire_outbox (
    id VARCHAR PRIMARY KEY,
    insight_id VARCHAR NOT NULL,
    subject_key VARCHAR NOT NULL,
    turn_k BIGINT NOT NULL,
    watermark BIGINT NOT NULL,
    payload_kind VARCHAR NOT NULL,
    created_at TIMESTAMP NOT NULL,
    dispatched_at TIMESTAMP
);
```
- **Grain:** One row per session turn triggering a tripwire notification.
- **Trigger Execution:** Maintained by an $O(1)$ `AFTER INSERT` statement trigger on fact tables. The database trigger increments the session watermark and inserts a lightweight dirty-session row (`payload_kind = 'dirty_session'`).
- **Trigger Bounds:** Triggers strictly perform no historical scans, no quantile calculations, and no table joins. Heavy statistical analysis (`I22`) runs asynchronously in the Rust dispatcher post-commit.
- **Advisory Only:** Alerts are operator-visible, pull-only metadata. The dispatcher never injects alerts into an agent's context, system prompt, or tool output.

### 2.3. `harness_trajectory` (Schema 5 New)
Fixed-dimension embeddings representing agent capability/outcome state transitions.
```sql
CREATE TABLE harness_trajectory (
    observation_id VARCHAR PRIMARY KEY,
    harness VARCHAR NOT NULL,
    native_session VARCHAR NOT NULL,
    turn_k BIGINT NOT NULL,
    trajectory_vector FLOAT[32] NOT NULL,
    created_at TIMESTAMP NOT NULL
);
```
- **Grain:** One row per agent turn $k$ within a session.
- **`trajectory_vector` (`FLOAT[32]`):** 32-element floating-point vector sketch encoding closed capability class and outcome transition states.
- **Distance Metric:** Evaluated via exact `array_cosine_distance`. Persistent on-disk HNSW is forbidden for canonical state.

### 2.4. `_dendro_pricing_tiers` (Schema 5 New)
Operator-pinned reference pricing and model tier parameters for counterfactual simulation (`I25`).
```sql
CREATE TABLE _dendro_pricing_tiers (
    tier_id VARCHAR PRIMARY KEY,
    profile_pin VARCHAR NOT NULL,
    cohort_pin VARCHAR NOT NULL,
    currency VARCHAR NOT NULL,
    input_rate DOUBLE,
    output_rate DOUBLE
);
```
- **Grain:** One row per model tier pricing profile.
- **Integrity:** Pinned by cryptographic digest. Used by `I25` (`model.counterfactual_routing`). If unpinned or empty, counterfactual simulation safely returns `Refused { reason: "unpinned_tier_profile" }`.

### 2.5. `_dendro_redacted_diagnostics` (Schema 5 New)
Curated remediation taxonomy and redacted diagnostic tokens indexed for full-text search.
```sql
CREATE TABLE _dendro_redacted_diagnostics (
    diagnostic_id VARCHAR PRIMARY KEY,
    category VARCHAR NOT NULL,
    redacted_label VARCHAR NOT NULL,
    remediation_hint VARCHAR NOT NULL,
    updated_at TIMESTAMP NOT NULL
);
```
- **Full-Text Search:** Indexed using DuckDB `fts` extension (`PRAGMA create_fts_index('_dendro_redacted_diagnostics', 'diagnostic_id', 'redacted_label', 'remediation_hint', incremental := 1)`).
- **Privacy:** Never indexes prompt text, user code, file paths, arguments, or raw messages.

---

## 3. Canonical Fact & Metadata Tables

### 3.1. `usage_facts`
Detailed token consumption and accounting per session turn.
```sql
CREATE TABLE usage_facts (
    source_key VARCHAR PRIMARY KEY,
    harness VARCHAR NOT NULL,
    native_session VARCHAR NOT NULL,
    message_id VARCHAR,
    ts TIMESTAMP,
    seq BIGINT NOT NULL,
    input_uncached BIGINT NOT NULL,
    output_tokens BIGINT NOT NULL,
    cache_read BIGINT NOT NULL,
    cache_write BIGINT NOT NULL,
    reasoning BIGINT NOT NULL,
    currency_total DOUBLE,
    generation BIGINT NOT NULL,
    source_id VARCHAR NOT NULL,
    predecessor_source_key VARCHAR,
    predecessor_reason VARCHAR,
    prior_input_tokens BIGINT,
    prior_cached_input_tokens BIGINT,
    prior_cache_write_input_tokens BIGINT,
    prior_output_tokens BIGINT,
    prior_reasoning_output_tokens BIGINT,
    model VARCHAR,
    effort VARCHAR
);
```
- **Grain:** One record per message or turn usage emission.
- **Token Invariants:** `tokens = input_uncached + output_tokens + cache_read + cache_write`. `reasoning` is a non-additive subset of `output_tokens`.

### 3.2. `tool_facts`
Tool execution and failure tracking.
```sql
CREATE TABLE tool_facts (
    source_key VARCHAR PRIMARY KEY,
    harness VARCHAR NOT NULL,
    native_session VARCHAR NOT NULL,
    ts TIMESTAMP,
    seq BIGINT NOT NULL,
    tool_name VARCHAR NOT NULL,
    fingerprint VARCHAR,
    failed BOOLEAN,
    structured_write BOOLEAN NOT NULL,
    generation BIGINT NOT NULL,
    source_id VARCHAR NOT NULL,
    category VARCHAR NOT NULL DEFAULT 'other'
);
```
- **Grain:** One row per tool invocation. Command arguments and raw outputs are omitted; only deterministic hashed fingerprints are retained.
- **Shared Watermark-Gated Tool Scan (`ToolScan`, D1 / F2):**
  - Claude, Codex, Pi, and Grok tool calls are observed through the shared per-generation `ToolScan` path in [`crates/dendro-connectors/src/stream.rs`](../crates/dendro-connectors/src/stream.rs), guarded by the per-file ingestion watermark (`new_from`).
  - **Real Codex Tool Names & Payload Types:** Codex rollout records (`response_item` with `payload.type` in `function_call`, `custom_tool_call`, or `tool_call` [not observed in G0a captures]) record the real tool name from `payload.name` (e.g. `exec`, `exec_command`, `spawn_agent`, `wait_agent`, `apply_patch`), never the legacy constant `CodexSeatCall`. `response_item` with `payload.type = "local_shell_call"` records a call named `local_shell` (fingerprinting and classifying `action.command` under the same privacy rules as other tool inputs without storing or exporting command text) and settles against `local_shell_call_output`. Native `event_msg` `item_completed` `CommandExecution` items continue to record `CommandExecution` under the same watermark guard and are included in `calls`, `new_calls`, and `prefix_skipped`.
  - **Per-File Fallback Sessions:** When a Codex file has no `session_meta` header on line 1, `tool_facts.native_session` uses the sealed per-file fallback `codex-file:{source_id}` (`source_session`), matching the session used by the same file's other facts and keeping each header-less file isolated in `evaluate_codex_loops` without guessing or sharing sessions across files.
  - **Replay Safety, Call-ID Matching & Deduplication:** Within a file generation, calls and results match strictly on `(native_session, call_id)` (never falling back to message `id`, which occupies a different identifier space). A call without a `call_id` is recorded and counted as call-only and is never matched by a result. Calls inside the already-committed watermark prefix (`line_no <= new_from`) do not emit new `tool_facts` rows on replay or re-ingest and increment `prefix_skipped`; repeated call blocks with an already-seen `(native_session, call_id)` increment `duplicate_call_id`.
  - **Outcome Settlement & Late/Unmatched Results:** A `function_call_output`, `custom_tool_call_output`, `tool_call_output` (not observed in G0a captures), or `local_shell_call_output` settles `tool_facts.failed` only when an explicit outcome signal is present (`is_error`/`isError`, explicit `status`, `exit_code`, non-empty `error`, or an unambiguous `Exit code: N` / `Process exited with code N` line in the `exec_command` header section before `Output:`); outputs without an explicit signal leave `tool_facts.failed = NULL` (unknown). A matched result is counted in `late_results` only when the result's line is past the watermark (`line_no > new_from`) and its call was recorded in an earlier pass or batch (`call.line <= new_from`), so parallel in-batch calls and pure replays of an unchanged log report `late_results = 0`. Outputs arriving before their call within a batch or across batches in the same pass still attach once the call is observed. Outputs with no matching call in `(native_session, call_id)` are counted as `unmatched_results` and never invent a call.
  - **Observable Tool-Scan Measures (`FileStats.tool_scan: ToolScanCounts`):** Each ingest pass exposes `calls` (unique calls across the generation, including `CommandExecution`), `new_calls` (calls emitted past the watermark cutoff, including `CommandExecution`), `results` (matched result records), `late_results` (matched results past the watermark whose call was recorded in an earlier pass or batch), `unmatched_results` (results with no matching call), `prefix_skipped` (calls at or before the watermark), and `duplicate_call_id` (repeated `call_id` records suppressed within the session).
  - **Migration / Release Note (Pre-D1 Databases):** Databases ingested before D1 retain their existing `CodexSeatCall` rows in `tool_facts` until the source file changes generation (is replaced or truncated). When an existing pre-D1 Codex session file grows by append without changing generation, its already-committed prefix rows keep `CodexSeatCall` while newly appended calls in that same generation are recorded under their real tool names (mixed naming within that generation until a fresh database or new file generation).

### 3.3. `message_facts`
Message metadata tracking turn direction and order without storing text.
```sql
CREATE TABLE message_facts (
    source_key VARCHAR PRIMARY KEY,
    harness VARCHAR NOT NULL,
    native_session VARCHAR,
    ts TIMESTAMP,
    seq BIGINT NOT NULL,
    role VARCHAR NOT NULL,
    direction VARCHAR NOT NULL,
    modalities VARCHAR NOT NULL,
    grain VARCHAR NOT NULL,
    generation BIGINT NOT NULL,
    source_id VARCHAR NOT NULL
);
```
- **Grain:** One row per message event. Role is `user`, `assistant`, or `system`.

### 3.4. `identity_facts`
Deterministic correlation linking native harness sessions to rig and seat identities.
```sql
CREATE TABLE identity_facts (
    source_key VARCHAR PRIMARY KEY,
    harness VARCHAR NOT NULL,
    native_session VARCHAR NOT NULL,
    rig VARCHAR NOT NULL,
    seat VARCHAR NOT NULL,
    ts TIMESTAMP
);
```
- **Grain:** One row per session-to-seat identity binding. Collision detection marks sessions unlinked when conflicting identities appear.

### 3.5. `activity_facts`
Agent state transitions (`active`, `idle_with_work`, `idle_without_work`, `waiting_on_human`).
```sql
CREATE TABLE activity_facts (
    source_key VARCHAR PRIMARY KEY,
    seat VARCHAR NOT NULL,
    ts TIMESTAMP NOT NULL,
    state VARCHAR NOT NULL,
    seq BIGINT NOT NULL
);
```
- **Grain:** One row per seat state change.

### 3.6. `queue_facts`
Work item lifecycle and handoff transitions across agents.
```sql
CREATE TABLE queue_facts (
    source_key VARCHAR PRIMARY KEY,
    transition_id BIGINT NOT NULL,
    qitem_id VARCHAR NOT NULL,
    ts TIMESTAMP NOT NULL,
    state VARCHAR NOT NULL,
    source_seat VARCHAR NOT NULL,
    dest_seat VARCHAR NOT NULL,
    closure_reason VARCHAR,
    closure_target VARCHAR,
    chain_id VARCHAR,
    missions VARCHAR NOT NULL,
    is_recovery BOOLEAN NOT NULL DEFAULT FALSE,
    recovery_reason VARCHAR,
    last_nudge_result VARCHAR,
    issue_tags VARCHAR NOT NULL DEFAULT '[]',
    phase_tags VARCHAR NOT NULL DEFAULT '[]',
    parent_qitem_id VARCHAR
);
```
- **Grain:** One row per queue item state transition.

### 3.7. `skill_facts`
Declared vs invoked skill tracking for agent capability evaluation.
```sql
CREATE TABLE skill_facts (
    source_key VARCHAR PRIMARY KEY,
    seat VARCHAR NOT NULL,
    skill VARCHAR NOT NULL,
    invoked BOOLEAN NOT NULL
);
```
- **Grain:** One row per skill declaration or invocation check.

### 3.8. `security_facts`
Explicit security policy events (grants, refusals, sandbox boundaries).
```sql
CREATE TABLE security_facts (
    source_key VARCHAR PRIMARY KEY,
    source_id VARCHAR NOT NULL,
    harness VARCHAR NOT NULL,
    native_session VARCHAR,
    ts TIMESTAMP,
    kind VARCHAR NOT NULL,
    value VARCHAR NOT NULL,
    source_field VARCHAR NOT NULL,
    generation BIGINT NOT NULL
);
```
- **Grain:** One row per security observation. Values use closed vocabularies; command text is never stored.

### 3.9. `session_edges`
Explicit parent $\to$ child session lineage edges.
```sql
CREATE TABLE session_edges (
    source_key VARCHAR PRIMARY KEY,
    source_id VARCHAR NOT NULL,
    harness VARCHAR NOT NULL,
    parent_session VARCHAR NOT NULL,
    child_session VARCHAR NOT NULL,
    relation VARCHAR NOT NULL,
    status VARCHAR NOT NULL,
    observed_at TIMESTAMP,
    evidence_kind VARCHAR NOT NULL,
    generation BIGINT NOT NULL
);
```
- **Grain:** One row per discovered session linkage. Relations include `spawn`, `delegate`, `handoff`, `return`, `join`, `retry`. Status must be `evidenced` for graph reachability traversal.

### 3.10. `process_identities` & `process_samples`
Unprivileged process monitoring without command-line or environment snooping.
```sql
CREATE TABLE process_identities (
    identity_key VARCHAR PRIMARY KEY,
    root_key VARCHAR NOT NULL,
    pid BIGINT NOT NULL,
    parent_pid BIGINT,
    platform VARCHAR NOT NULL,
    start_time_unix_s BIGINT NOT NULL,
    exe_name VARCHAR NOT NULL,
    harness VARCHAR,
    cwd_fingerprint VARCHAR,
    session_link VARCHAR,
    environment_kind VARCHAR NOT NULL,
    isolation_hint VARCHAR NOT NULL,
    first_seen TIMESTAMP NOT NULL,
    last_seen TIMESTAMP NOT NULL,
    ended_at TIMESTAMP,
    visibility_limits VARCHAR NOT NULL,
    attribution VARCHAR
);

CREATE TABLE process_samples (
    watch_id VARCHAR NOT NULL,
    identity_key VARCHAR NOT NULL,
    seq BIGINT NOT NULL,
    ts TIMESTAMP NOT NULL,
    cpu_time_us BIGINT,
    rss_bytes BIGINT,
    rss_unit VARCHAR NOT NULL,
    disk_read_bytes BIGINT,
    disk_write_bytes BIGINT,
    capture_method VARCHAR NOT NULL,
    visibility_limits VARCHAR NOT NULL,
    PRIMARY KEY (watch_id, identity_key, seq)
);
```

### 3.11. `file_source_baselines`
File provenance, modification, and inode integrity baselines for sealed re-runs.
```sql
CREATE TABLE file_source_baselines (
    source_id VARCHAR NOT NULL,
    generation BIGINT NOT NULL,
    metadata_version INTEGER NOT NULL,
    latest_association VARCHAR NOT NULL,
    latest_attempt_at VARCHAR NOT NULL,
    last_stable_observed_at VARCHAR,
    last_stable_type VARCHAR,
    last_stable_device VARCHAR,
    last_stable_inode VARCHAR,
    last_stable_size VARCHAR,
    last_stable_mtime_ns VARCHAR,
    association_reason VARCHAR NOT NULL,
    PRIMARY KEY (source_id, generation)
);
```

### 3.12. `warnings`, `dendro_meta`, `dendro_semantic_state`
- `warnings`: Ingest parsing anomalies (`path VARCHAR, line_no BIGINT, reason VARCHAR`).
- `dendro_meta`: Metadata header (`schema_version INTEGER NOT NULL, key_id VARCHAR NOT NULL`).
- `dendro_semantic_state`: Semantic cache invalidation tracking (`id INTEGER PRIMARY KEY, dirty BOOLEAN NOT NULL`).

### 3.13. `session_facts` (`link-id-v1`, `MTN-19 / D3`)
Cross-product privacy-preserving session link facts (`agent.session.link_id` and `agent.session.unlinked_reason`) conforming byte-exactly to [`docs/contracts/link-id-v1/SPEC.md`](contracts/link-id-v1/SPEC.md) (pinned digest `07dff9562958ec286c1b096f0eb15c91deddb284c26d99904afeba2b882d0ab5`).
```sql
CREATE TABLE IF NOT EXISTS session_facts (
    source_key VARCHAR PRIMARY KEY,
    source_id VARCHAR NOT NULL,
    harness VARCHAR NOT NULL,
    native_session VARCHAR NOT NULL,
    link_id VARCHAR NOT NULL DEFAULT 'unlinked',
    unlinked_reason VARCHAR,
    generation BIGINT NOT NULL
);
```
- **Grain:** One row per `(source_id, generation, native_session)` session observed during ingestion (`source_key = "{source_id}:{generation}:{native_session}"`).
- **Compute-Before-Seal Invariant:** `link_id` is computed from the **raw** harness-minted session identifier (`native_session`) **before** `seal_session` is applied. `session_facts.native_session` stores the sealed session token (`session:<hex>`, identical to `usage_facts.native_session` and `tool_facts.native_session`); the raw `native_session` is never persisted anywhere in the database.
- **Closed Harness Vocabulary (`I01`):** `link-id-v1` canonicalizes harness aliases strictly (`antigravity`/`agy` $\to$ `antigravity`, `claude`/`claude-code` $\to$ `claude`, `codex` $\to$ `codex`, `grok` $\to$ `grok`, `pi` $\to$ `pi`).
- **Origin Rule:** An absent `origin` field (`None` / `null`) or exact `"harness"` means a harness-minted session. Any other `origin` value—including case-mismatched `"Harness"`, empty `""`, `"synthetic"`, `"fallback"`, or `"per-file"`—resolves to `link_id = 'unlinked'` with `unlinked_reason = 'non_harness_origin'`.
- **Synthetic, Fallback & Header-less Sessions:** Any per-file fallback session (`{harness}-file:{source_id}` on header-less Codex/Pi/Grok files), fallback literal (`claude-session`, `codex-session`, `pi-session`, `grok-session`), or file with conflicting session headers resolves to `link_id = 'unlinked'` (`missing_native_session` when link key material is configured, or `invalid_key_material` when unconfigured).
- **Closed `unlinked_reason` Vocabulary:** `NULL` when `link_id` is a 64-character lowercase hex digest (`^[0-9a-f]{64}$`); when `link_id = 'unlinked'`, one of:
  - `invalid_key_material`: No `K_link` configured, `K_link` shorter than 32 bytes / invalid hex, or forbidden `K_org` variable present on the endpoint path.
  - `non_harness_origin`: Explicit `origin` is anything other than `"harness"` (e.g. `"Harness"`).
  - `missing_org`: Missing or whitespace-only `org` scope.
  - `missing_endpoint_scope`: Missing or whitespace-only `endpoint_scope`.
  - `missing_harness`: Missing or whitespace-only harness identifier.
  - `unknown_harness`: Harness identifier outside the closed `I01` vocabulary.
  - `conflicting_harness`: Disagreement between `agent.harness` and wire `client_id`.
  - `missing_native_session`: Missing, whitespace-only, synthetic, fallback, or header-less `native_session`.
  - `invalid_field_characters`: Scope slug (`org`, `endpoint_scope`) violates `^[\x21-\x7E]{1,1024}$`, or `native_session` exceeds 1024 UTF-8 bytes, has leading/trailing Unicode `White_Space`, or contains NUL / control (`Cc`) characters.
- **Backward-Compatible Migration:** Existing pre-D3 databases open without data loss. Pre-D3 `session_facts` tables missing `link_id` or `unlinked_reason` are upgraded via `ALTER TABLE session_facts ADD COLUMN IF NOT EXISTS ...`, with existing rows receiving `'unlinked'` / `NULL` (never a guessed value). See [`docs/link-id-config.md`](link-id-config.md) for endpoint configuration.

---

## 4. The 25 Reference Insights Catalog (`I01`–`I25`)

All reference insights adhere to the immutable contract:
- Status is strictly `Computed`, `Partial { reason }`, or `Refused { reason }`.
- Carries at most 5 sealed evidence keys.
- Display status is mapped deterministically to one of 5 badge tokens: `ok`, `partial`, `insufficient_evidence`, `refused`, `not_applicable`.
- Progressive disclosure exposes levels L0 (status/finding), L1 (assumptions/alternatives), L2 (evidence IDs/versions), L3 (formula/population).

| ID | Name | Domain | Physical Facts | Contract & Output Measures |
|:---|:---|:---|:---|:---|
| **I01** | `model.invocations` | Model | `usage_facts` | Distinct qualified invocation count at call grain; dedupes on `(canonical_harness, native_session, source_id, message_id)`. `usage_only` and `unqualified` rows are counted separately. See identity and missingness below. |
| **I02** | `model.tokens` | Model | `usage_facts` | Inclusive sum of tokens: `cache_read`, `cache_write`, `input_uncached`, `output_tokens`, and non-additive `reasoning`. |
| **I03** | `model.tokens.inclusive` | Model | `usage_facts`, `session_edges` | Sum over reachable observation graph union. Incomplete lineage yields lower bound. Ambiguous duplicates refuse total. |
| **I04** | `model.cost` | Model | `usage_facts`, `_dendro_pricing_tiers` | Monetary burn calculated from currency figures or pinned pricing tiers. Excludes estimated or unverified rates. |
| **I05** | `model.configuration_divergence` | Model | `usage_facts`, `harness_envelope` | Divergence between contemporaneous observed and desired model/effort configurations. Missing pairs return `not_observable`. |
| **I06** | `harness.capability_funnel` | Harness | `skill_facts`, `tool_facts` | Funnel ratio of distinct loaded vs invoked capabilities: $\text{invoked} \cap \text{loaded} / \text{loaded}$. |
| **I07** | `harness.context_pressure` | Harness | `usage_facts`, `_dendro_pricing_tiers` | Evidenced context ratio: $\text{context\_used} / \text{context\_limit}$. Missing limit safely refuses ratio. |
| **I08** | `harness.tool_loops` | Harness | `tool_facts` | Consecutive tool call failure loops (e.g. 3 failures within 600 seconds) and retry spans. |
| **I09** | `harness.delegation_chains` | Harness | `session_edges`, `queue_facts` | Multi-hop agent handoffs, chain return rates, and delegation depth. |
| **I10** | `work.duration` | Coordination | `activity_facts`, `queue_facts` | Wall-clock union, active seconds, and concurrency multiplier. Half-open intervals $[from, to)$. |
| **I11** | `work.shared_allocation` | Coordination | `queue_facts`, `usage_facts` | Shared token and duration usage weighted across co-beneficiary missions. Weights sum $\le 1.0$. |
| **I12** | `coord.queue_wait` | Coordination | `queue_facts` | Elapsed interval between queue item creation (`ready`) and first agent processing (`start`). |
| **I13** | `environment.placement` | Environment | `process_identities` | Incarnations and placement intervals by host/platform locus; rebuild counts. |
| **I14** | `environment.process_resources` | Environment | `process_samples` | Monitored process resource burn: CPU user/system time, RSS memory, disk I/O bytes. |
| **I15** | `environment.effects` | Environment | `process_samples` | System-level effects. Returns `not_observable` without privileged trace specimens (T11/T26). |
| **I16** | `security.isolation` | Security | `process_identities`, `security_facts` | Declared vs observed process isolation and environment sandboxing checks. |
| **I17** | `security.authority` | Security | `security_facts` | Consistency of security grants, permissions, and tool execution checks. Unassessed policies yield `NOT_ASSESSED`. |
| **I18** | `security.redactions` | Security | `warnings`, `security_facts` | Scrubbed credential counters and sanitization incident counts across ingestion streams. |
| **I19** | `outcome.acceptance` | Coordination | `queue_facts` | Accepted work ratio: accepted items / rule-eligible evaluated items. |
| **I20** | `outcome.resource_efficiency` | Coordination | `usage_facts`, `queue_facts` | Matched-population resource burn per completed and accepted outcome unit. |
| **I21** | `comparison.descriptive` | Coordination | All Fact Tables | Stratified descriptive comparison of identical metrics across two disjoint time windows or cohorts. Noncomparable mixes yield `noncomparable`. |
| **I22** | `harness.trajectory_vortex` | Harness | `harness_trajectory`, `_dendro_tripwire_outbox` | Vector cosine distance to cohort completed centroid. Trips when distance clears quantile and self-transition rate persists for $k$ consecutive turns. |
| **I23** | `harness.schema_drift` | Harness | `harness_envelope` | L1 distance between `envelope_shape` key distributions across harness versions. Reports added, removed, and type-altered keys. |
| **I24** | `security.authority_blast_radius` | Security | `session_edges`, `security_facts` | Bounded graph reachability (`duckpgq`) from revoked/failed grants. Traverses only `status = evidenced`. Carries dependency exposure non-claim. |
| **I25** | `model.counterfactual_routing` | Model | `usage_facts`, `_dendro_pricing_tiers` | Counterfactual fast-tier simulation via `ASOF JOIN` and `QUANTILE_CONT` cache inflection. Refuses if cohort/tier profile is unpinned. |

### I01 invocation identity and missingness

`I01.distinct_calls` counts distinct tuples of `(canonical_harness, native_session,
message_id)` from scoped `usage_facts`, matching the store's own call identity
(`message_seen`). Repeated observations of the same tuple count once, including across
different `source_id` values; the same message ID in different sessions or harnesses
counts separately. This is local invocation qualification;
it does not compute a session `link_id` or introduce machine, actor, or principal keys.

Harness aliases follow the frozen `link-id-v1` vocabulary in
[`smota/agent-otel-bridge`](https://github.com/smota/agent-otel-bridge/blob/main/docs/contracts/fact-export-v1/link-id-v1/vocabulary.json):
`claude-code` and `claude` map to `claude`; `agy` and `antigravity` map to
`antigravity`; `codex`, `grok`, and `pi` map to themselves. Input must be ASCII,
ASCII case-insensitive, with no trimming. Unknown aliases (including model/provider
names) do not qualify. Session and message identifiers remain byte-exact;
they are never concatenated into an ambiguous delimiter-separated key.

`I01.usage_only` counts rows with absent, empty, or whitespace-only message IDs.
Among rows with message IDs, `I01.unqualified` counts each row with a missing or
unknown qualifier separately, without deduplicating or adding it to
`distinct_calls`. Empty/whitespace-only values and literal `unknown` (ASCII
case-insensitive) are treated as missing session qualifiers (a real session id
spelled `unknown` is therefore unqualified). When `unqualified > 0` alongside qualified
calls the status is `partial` with reason `unqualified_rows`; otherwise `computed`.
Insight version is `1.1.0`. The two row counts are disjoint; the population
includes all scoped usage rows. With no qualified call the result refuses with
`not_observable`; an empty population also refuses. Existing qualified single-session
counts are unchanged. The additive `unqualified` measure exposes missingness rather
than guessing invocation identities.

---

## 5. Extensions & Subsystems

### 5.1. Lineage Graph (`lineage_graph_v1`)
Lineage analysis is powered by DuckDB's SQL:2023 Property Graph Extension (`duckpgq`):
- **Grammar Configuration:** `SET active_grammar_extensions = ['duckpgq'];`.
- **Physical Tables:** Views over `session_edges` (`work_edges`) and `security_facts` (`security_decisions`).
- **Edge Eligibility:** Strictly restricted to `status = 'evidenced'`. Edges with status `absent`, `truncated`, or `conflict` are never traversed for reachability.
- **Fixed Traversal Directions:**
  - `spawn`, `delegate`, `delegate_existing`, `handoff`: Parent $\to$ Child.
  - `return`, `join`: Child $\to$ Parent.
  - `retry`: Prior attempt $\to$ Later attempt.
- **Hop Ceiling:** Bounded traversal with default ceiling of 8 hops to prevent cycle traps.
- **Rust Fallback:** When the `duckpgq` extension is unavailable, `crates/dendro-core/src/session_lineage.rs` executes the identical deterministic graph walk.

### 5.2. Retrieval Subsystem (`retrieval_v1`)
Consists of three complementary mechanisms:
1. **Vector Cosine Distance:** Uses DuckDB's native `array_cosine_distance` over `FLOAT[32]` vectors stored in `harness_trajectory`. Experimental on-disk HNSW is never used for canonical state.
2. **Incremental Full-Text Search (`fts`):** Indexes curated diagnostic strings in `_dendro_redacted_diagnostics` via `PRAGMA create_fts_index(..., incremental := 1)`. User prompts, file paths, and argument values are excluded from indexing.
3. **DuckLake 1.1 Export:** Generates immutable, pseudonymized Parquet partitions in the local export directory for cross-machine baseline analysis. Partitions never contain raw identifiers or prompt text.

### 5.3. Daemon (`dendro serve`)
Single-owner trusted daemon process:
- Sole holder of the read-write DuckDB connection and embedded `quack` protocol SQL session.
- Client requests connect over a private Unix domain socket (`0700`) or Windows Named Pipe (restricted DACL).
- Arbitrary SQL execution is refused; all client requests are strictly typed RPC commands. Unauthenticated loopback TCP listeners are forbidden.

### 5.4. Advisory Engine (`dendro advise`)
Operator-visible intelligence advisor and Model Context Protocol (MCP) server:
- **Questions Answered:**
  1. `model_cohort_baseline`: Comparability of token/cost performance across harness versions.
  2. `skill_effectiveness_audit`: Verification of declared vs loaded vs effective skill execution.
  3. `delegation_loop_risk`: Detection of cycles and excessive retry loops in subagent delegation.
  4. `environment_readiness`: Inspection of placement, isolation hints, and resource ceilings.
- **Budget & Performance:** Evaluates responses within a warm $p95 \le 15\text{ ms}$ budget. Returns `advisory_budget_exceeded` if the threshold is exceeded.
- **Uncertainty Taxonomy:** Structured uncertainty expressed through the `AdviseUncertainty` enum (`sample_below_minimum`, `missing_denominator`, `noncomparable_mix`, `counterfactual_only`, `stale`).
- **Governance Seam:** Advisory is strictly informational for the operator. Advice is never injected into agent prompts and never executes autonomous lifecycle changes.
