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
    header_conflict BOOLEAN NOT NULL DEFAULT FALSE,
    generation BIGINT NOT NULL
);
```
- **Grain:** One row per session identity `(source_id, harness, native_session)` observed during ingestion (`source_key = "{source_id}:{harness}:{native_session}"`), upserted across generations. Only an explicit header conflict (`header_conflict = TRUE`, keyed by `source_id`) permanently locks a session row to `'unlinked'` across subsequent generations and appends; sessions unlinked for configuration or input reasons (`invalid_key_material`, `missing_org`, `missing_endpoint_scope`, missing/invalid `team` or `epoch`) and fallback `{harness}-file:` / `claude-session` rows are recomputed and replaced on subsequent ingests, and a fallback row created during a zero-byte or partial line-1 first pass is retired once the real session header appears.
- **Compute-Before-Seal Invariant:** `link_id` is computed from the **raw** harness-minted session identifier (`native_session`) **before** `seal_session` is applied. `session_facts.native_session` stores the sealed session token (`session:<hex>`, identical to `usage_facts.native_session` and `tool_facts.native_session`); the raw `native_session` is never persisted anywhere in the database.
- **Closed Harness Vocabulary (`I01`):** `link-id-v1` canonicalizes harness aliases strictly (`antigravity`/`agy` $\to$ `antigravity`, `claude`/`claude-code` $\to$ `claude`, `codex` $\to$ `codex`, `grok` $\to$ `grok`, `pi` $\to$ `pi`).
- **Origin Rule (Parser Provenance):** `origin` is assigned by Dendro's parser, never read from transcript content. Native harness-log parsers assign `origin = "harness"`; export or any non-native path sets a non-harness origin, which resolves to `link_id = 'unlinked'` with `unlinked_reason = 'non_harness_origin'`.
- **Synthetic, Fallback & Header-less Sessions (Parser Provenance):** Sessions are marked unlinked when the parser assigns a fallback or synthetic identifier (per-file fallback `{harness}-file:{source_id}` on header-less Codex/Pi/Grok files, `claude-session` fallback when `sessionId` is absent on a Claude record, or files with conflicting session headers across the file or prior generations), resolving to `link_id = 'unlinked'` (`missing_native_session` when link key material is configured, or `invalid_key_material` when unconfigured).
- **Closed `unlinked_reason` Vocabulary:** `NULL` when `link_id` is a 64-character lowercase hex digest (`^[0-9a-f]{64}$`); when `link_id = 'unlinked'`, one of:
  - `invalid_key_material`: No `K_link` configured, `K_link` shorter than 32 bytes / invalid hex, or forbidden `K_org` variable present on the endpoint path.
  - `non_harness_origin`: Caller/parser `origin` is anything other than `"harness"`.
  - `missing_org`: Missing or whitespace-only `org` scope.
  - `missing_endpoint_scope`: Missing or whitespace-only `endpoint_scope`.
  - `missing_harness`: Missing or whitespace-only harness identifier.
  - `unknown_harness`: Harness identifier outside the closed `I01` vocabulary.
  - `conflicting_harness`: Disagreement between `agent.harness` and wire `client_id`.
  - `missing_native_session`: Missing, whitespace-only, or parser-fallback/header-less `native_session`.
  - `invalid_field_characters`: Scope slug (`org`, `endpoint_scope`) violates `^[\x21-\x7E]{1,1024}$`, or `native_session` exceeds 1024 UTF-8 bytes, has leading/trailing Unicode `White_Space`, or contains NUL / control (`Cc`) characters.
- **Pre-D3 Databases & Downstream Export (`D4b`):** `session_facts` is new in D3. Opening a pre-D3 database adds `session_facts` without touching existing tables. Pre-D3 sessions have no row in `session_facts` (`NULL` when joined = *"not computed; full rebuild computes it"*). Downstream export (`D4b`) **MUST** treat a missing `session_facts` row or `NULL` link as `'unlinked'`. See [`docs/link-id-config.md`](link-id-config.md) for endpoint configuration.

### 3.14. Per-Batch Atomicity, Watermarks & Replay Guarantee (`MTN-20 / D4a`)
- **Single-Transaction Batch Commit (`AnalyticsStore::commit_ingest_batch`):** For every per-file ingestion batch across native harness streams (`stream.rs`) and export connectors (`exports.rs`), all emitted fact families (`file_source_baselines`, `codex_counters`, `usage_facts`, `tool_facts`, `message_facts`, `skill_facts`, `session_edges`, `security_facts`, `session_facts` + fallback retirements, `git_evidence_facts` + `git_evidence_coverage`, `warnings`) and the covering `source_files` watermark + `dendro-internal:reader:<source_id>:<generation>` checkpoint commit atomically in one DuckDB transaction. OpenRig per-snapshot ingestion (`ingest_openrig` in `ingest.rs`) likewise commits `openrig_row_records`, `mutable_source_disclosures`, OpenRig fact tables, and the `source_files` revision watermark in one transaction via `with_openrig_fact_batch` + `commit_ingest_batch`.
- **Stage-and-Insert Appenders:** Inside the transaction, `usage_facts` and `tool_facts` bulk-load into connection-scoped temporary tables (`usage_stage`, `tool_stage`) whose creation (`CREATE TEMP TABLE`) and drop (`DROP TABLE`) are transactional inside `tx`, populated via `tx.appender(...)`, and merge via `INSERT INTO ... SELECT ... ON CONFLICT (source_key) DO NOTHING` before dropping the staging table. This retains typed appender throughput and the 1 GB connection memory ceiling while ensuring uncommitted rows and staging tables roll back if the transaction aborts.
- **Per-Batch `session_facts` & `native_session` Durability:** Every committed batch persists all facts derived from the lines covered by its watermark, including `session_facts` observed up to that watermark, the sticky `header_conflict` latch, `retired_fallback_keys`, `git_evidence_facts` / `git_evidence_coverage` (`IngestBatchStage::AfterGitEvidence`), and `source_files.native_session` / the reader-marker session (written on every batch once known from batch 1, never overwritten with `NULL` or `''` on single-session sources). Intermediate multi-batch states are safe because each committed state reflects the exact lines covered by its watermark, and a later header conflict can only flip a session row from linked to `'unlinked'` (never the reverse), enforced by the D3 sticky-conflict upsert. On resume, in-memory per-pass session state (`header_conflict` and existing session rows for `source_id`) is seeded from the store so a resumed pass converges to the exact same final state as an uninterrupted pass.
- **Intentional Separate Writers:** Staged discovery and unreadable-file warnings (`source <name> missing`, `native_entry_unreadable`, `native_file_unreadable`, `export_discovery_failed`), export refusal warnings (`ExportRefused`), live watch process samples (`AnalyticsStore::commit_process_samples`), and post-loop OpenRig seat-lifetime replacement (`AnalyticsStore::replace_seat_bounds`) operate outside per-file batch watermarks in their own bounded transactions.
- **Deterministic Crash Replay:** If any write stage or `BeforeCommit` fails, the transaction rolls back completely, leaving `source_files` and all fact/warning tables unchanged for that batch. Re-running ingestion from the preserved watermark converges to the exact same row set and content as an uninterrupted run.

### 3.15. `git_evidence_facts` & `git_evidence_coverage` (`fact-export-v1` Historical Git Bridge, `MTN-21 / D4c`)
Historical git provenance and missingness facts derived strictly from ingested session records (`claude`, `codex`, `pi`, `grok`), without opening any `.git` repository, spawning `git` processes, or performing network lookups.

```sql
CREATE TABLE IF NOT EXISTS git_evidence_facts (
    source_key VARCHAR PRIMARY KEY,
    source_id VARCHAR NOT NULL,
    harness VARCHAR NOT NULL,
    canonical_harness VARCHAR NOT NULL,
    commit_sealed VARCHAR NOT NULL,
    parent_sealed VARCHAR NOT NULL DEFAULT '[]',
    merge_status VARCHAR NOT NULL,
    is_merge BOOLEAN,
    merge_missing_reason VARCHAR,
    default_branch_status VARCHAR NOT NULL,
    on_default_branch BOOLEAN,
    observed_at TIMESTAMP,
    observed_at_status VARCHAR NOT NULL,
    session_match_status VARCHAR NOT NULL,
    native_session VARCHAR,
    export_status VARCHAR NOT NULL,
    pseudonym_key_id VARCHAR,
    commit_hmac VARCHAR,
    parent_hmacs VARCHAR NOT NULL DEFAULT '[]',
    evidence_source VARCHAR NOT NULL,
    generation BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS git_evidence_coverage (
    source_id VARCHAR PRIMARY KEY,
    harness VARCHAR NOT NULL,
    generation BIGINT NOT NULL,
    observed BIGINT NOT NULL DEFAULT 0,
    unresolved_abbreviated BIGINT NOT NULL DEFAULT 0,
    unverified_object_format BIGINT NOT NULL DEFAULT 0,
    missing_parents BIGINT NOT NULL DEFAULT 0,
    ambiguous BIGINT NOT NULL DEFAULT 0,
    unmatched BIGINT NOT NULL DEFAULT 0,
    default_branch_not_observed BIGINT NOT NULL DEFAULT 0,
    unparsed_compound_git_calls BIGINT NOT NULL DEFAULT 0
);
```

- **Grain & Per-Source Storage (`P3`, `P6`):**
  - `git_evidence_facts`: One row per `(source_id, commit_sealed)` (`source_key = "{source_id}:git:{commit_sealed}"`), written exclusively inside `AnalyticsStore::commit_ingest_batch` (`IngestBatchStage::AfterGitEvidence`). Re-ingesting or rewriting one source deletes only prior generations of that `source_id` and never deletes or rewrites another source's evidence; rows are never merged across sources at write time.
  - `git_evidence_coverage`: One row per `source_id`, recording per-source observation and missingness counters for the latest generation:
    - **Per-unique-SHA counters** (counted once per unique canonical commit SHA observed in the source generation): `observed`, `missing_parents`, `ambiguous`, `unmatched`, and `default_branch_not_observed`.
    - **Per-occurrence counters** (counted on every rejected occurrence): `unresolved_abbreviated` (abbreviated commit SHAs of length `7..=39` lowercase hex), `unverified_object_format` (64-hex digests observed without `git rev-parse --show-object-format` $\to$ `sha256` recorded in the same session and working directory), and `unparsed_compound_git_calls` (tool calls that reference `git` in a compound, piped, redirected, env-prefixed, or global-option shape rejected under `R1`).
- **Single-Command Allow-List & Output Shapes (`P1`, `R1`–`R4`, `R7`):**
  - **Single-command-only rule (`R1`):** A tool call's output is parsed **only** if the entire command (after unwrapping at most `MAX_SHELL_WRAPPER_DEPTH = 4` levels of `sh`/`bash`/`zsh`/`dash` `-c`/`-lc`) is **exactly one** allow-listed `git` invocation, optionally preceded by a single `cd <path> &&` (which sets the effective `cwd` for that call only). Any other shape contributes nothing and (when it mentions `git`) increments `git_evidence_coverage.unparsed_compound_git_calls`: multiple `git` segments, `;`, `&&`/`||` chains beyond the leading `cd`, pipes (`|`), subshells (`$(...)`, backticks, `(...)`), redirections (`>`, `<`), environment assignments (including `GIT_DIR=...` / `GIT_WORK_TREE=...`), or `git` global options (`-C`, `--git-dir`, `--work-tree`, `-c`, `--namespace`, `--bare`, `--super-prefix`, `--exec-path`).
  - **Log/show/rev-list/cat-file shapes (`R2`):**
    - `git log` with `--format=%H` / `--pretty=%H` / `--pretty=format:%H` or **exactly** `%H %P` (single space): allowed only with optional `-n N` / `-N` / `--max-count=N` / `--max-count N` and optional revisions that are `HEAD` or simple branch names (no `rev:path`, no `--`, no file paths, and no `-p`/`--patch`/`--stat`/`--name-only`/`--name-status`/`--numstat`/`--shortstat`/`--dirstat`/`--raw`/`--graph`/`--all`/`--branches`/`--tags`/`--remotes`/`--glob`/`--walk-reflogs`/`-g`/`--oneline`).
    - `git show`: allowed **only** when `-s` or `--no-patch` is present alongside format `%H` or `%H %P` and at most one revision that is `HEAD` or a simple branch/commit name (never `rev:path` or `--`).
    - `git rev-list --parents`: requires a single revision that is `HEAD` or a simple branch name, with optional `-n N` / `-N` / `--max-count` (no `--objects`, `--all`, `--branches`, `--tags`, `--remotes`, `--glob`, `--header`, `--pretty`, `--format`, `--graph`, `--`, or `rev:path`).
    - `git cat-file -p <FULL-SHA>`: allowed only when `<FULL-SHA>` is a canonical 40-hex or 64-hex commit SHA (`git cat-file -p HEAD` and non-SHA targets contribute nothing).
    - **All-or-nothing strict line matching (`R2`):** Output lines must match the expected shape **exactly** with **no leading whitespace** (only trailing `\r`/`\n` is stripped). Any non-conforming line in the output of `git rev-parse HEAD`, `git log`, `git show`, or `git rev-list --parents` causes the **entire** tool output to contribute nothing.
  - **`git rev-parse` shapes (`R3`):**
    - Only the exact argument `HEAD` (commit identity), `--show-object-format` (`P2`), or `--abbrev-ref HEAD` (current branch). Raw hex strings (such as the empty tree hash), tags (`v1.0`), `HEAD~1`, `--short`, and all other refs or flags contribute nothing.
  - **`HEAD`-only `on_default_branch` attribution (`R4`):**
    - `default_branch_status = 'observed'` (`on_default_branch = TRUE | FALSE`) is recorded **only** for a commit established as `HEAD` by `git rev-parse HEAD`, or by `git log -1` / `-n 1` / `--max-count=1` with no revision argument (or revision `HEAD`), **and** only when, at that observation time and for the **same session and same normalized `cwd`**:
      1. The current branch is known from a record or command (`git rev-parse --abbrev-ref HEAD`, `git branch --show-current`, `git status`, `git checkout`/`switch`) in that exact `(session, cwd)` (never another `cwd`'s value, and never a session-wide fallback).
      2. The default branch is known from `git symbolic-ref refs/remotes/origin/HEAD` or `git remote show origin` in that exact `(session, cwd)`.
      3. `HEAD` is not detached in that `(session, cwd)`: any `git checkout` / `git switch` with `--detach` or targeting a SHA / `HEAD~N` / `HEAD^` invalidates the current branch in that `(session, cwd)` until a branch is re-observed.
    - Every other commit (`git log` without `-1`, `git log` on a non-`HEAD` revision, `git rev-list --parents`, `git cat-file -p <FULL-SHA>`, `git show`) receives `default_branch_status = 'not_observed'` (`on_default_branch = NULL`).
  - **Per-Harness Shell Tool Allow-List & Bounded LRU (`R7`):**
    - Shell tool detection (`dendro_core::is_shell_execution_tool_for_harness`) uses an explicit per-harness allow-list (`Claude`: `Bash`/`bash`; `Codex`: `CommandExecution`/`shell`/`local_shell`/`local_shell_call`/`exec_command`/`shell_command`/`container.exec`/`bash`/`Bash`; `Pi`: `bash`/`Bash`/`sh`; `Grok`: `bash`/`Bash`/`shell`/`sh`/`run_command`/`execute_command`; `Antigravity`: `run_command`/`bash`/`Bash`/`shell`/`sh`). Unknown or absent tool names or harnesses are never treated as shells.
    - Tool results are matched to their call **only** by explicit `call_id` (unmatched results are ignored). All tool calls are registered in a per-source bounded LRU (`MAX_PENDING_CALLS_PER_SOURCE = 512`) so non-git tool calls evict stale entries and never alias unrelated outputs.
  - **Hostile-Input Bounds:** Linear-time slice scans with zero regular expressions and strict caps (`MAX_PENDING_CALLS_PER_SOURCE = 512`, `MAX_OBSERVED_COMMITS_PER_SOURCE = 4096`, `MAX_PARENTS_PER_COMMIT = 16`, `MAX_SHELL_WRAPPER_DEPTH = 4`, `MAX_GIT_COMMAND_BYTES = 16384`, `MAX_GIT_OUTPUT_BYTES = 65536`, `MAX_GIT_LINES_PER_RECORD = 512`). Non-zero exit codes / `is_error = true` tool results are ignored.
- **Object-Format Verification & Parent Length Matching (`P2`):**
  - `40`-character lowercase hex is accepted as SHA-1 unless the same `(session, cwd)` recorded `git rev-parse --show-object-format` = `sha256`.
  - `64`-character lowercase hex is accepted **only** when `git rev-parse --show-object-format` = `sha256` was recorded for that same `(session, cwd)`; otherwise it is counted in `git_evidence_coverage.unverified_object_format` and ignored.
  - Within a single commit observation, every parent SHA length must equal the commit SHA length; otherwise all parents for that observation are dropped (`merge_status = 'not_observed'`, `is_merge = NULL`, `merge_missing_reason = 'mixed_parent_object_format'`).
- **Merge Status & Parent Provenance (`merge_status`, `is_merge`, `merge_missing_reason`):**
  - `merge_status = 'observed'` only when an allow-listed parent command (`git log`/`git show -s` with `%H %P`, `git rev-list --parents`, or `git cat-file -p <FULL-SHA>`) records the commit's parents in full canonical form of matching object-format length (at most `MAX_PARENTS_PER_COMMIT = 16`), preserving local recorded parent order in `parent_sealed` (JSON array of sealed commit tokens) and `parent_hmacs` (JSON array of 64-hex `fact-export.v1/commit` HMACs when exportable). When `merge_status = 'observed'`, `is_merge` is `TRUE` if `len(parents) > 1` and `FALSE` if `len(parents) <= 1` (both single-parent commits and root commits with `0` parents have `merge_status = 'observed'` and `is_merge = FALSE`).
  - Otherwise, `merge_status = 'not_observed'`, `is_merge = NULL`, `parent_sealed = '[]'`, `parent_hmacs = '[]'`, `git_evidence_coverage.missing_parents` is incremented, and `merge_missing_reason` is set to one of:
    - `'parents_not_recorded'` (`MERGE_REASON_PARENTS_NOT_RECORDED`): commit was observed only without parent hashes (e.g. `git rev-parse HEAD` or `git log --format=%H`).
    - `'abbreviated_parents'` (`MERGE_REASON_ABBREVIATED_PARENTS`): parent token was an abbreviated hex SHA (`7..=39` chars) or exceeded `MAX_PARENTS_PER_COMMIT`.
    - `'mixed_parent_object_format'` (`MERGE_REASON_MIXED_PARENT_OBJECT_FORMAT`): a parent SHA's hex length (`40` vs `64`) differed from the commit SHA's hex length.
    - `'conflicting_parents'` (`MERGE_REASON_CONFLICTING_PARENTS`): multiple sources observed the same `commit_sealed` with different non-empty `parent_sealed` lists (`R5`).
- **Historical `observed_at` Timestamp (`observed_at`, `observed_at_status`):**
  - `observed_at` is the historical record timestamp when the commit was first observed (`observed_at_status = 'observed'`), or `NULL` with `observed_at_status = 'not_observable'` when the record lacks a valid timestamp. Current wall-clock time (`now()`) and current repository `HEAD` are never substituted.
- **Cross-Source Read Derivation & Coherent Single-Row Selection (`R5`, `P3`):**
  - Each source stores its own local row (`AnalyticsStore::load_git_evidence_by_source()`).
  - `AnalyticsStore::load_git_evidence()` (via `dendro_core::resolve_git_evidence_across_sources` / `resolve_git_evidence_across_sources_with_key_id`) deterministically derives one cross-source row per `commit_sealed` at read time:
    - **Session attribution (`session_match_status`, `native_session`):**
      - `'matched'`: Commit has **exactly one** distinct non-empty `native_session` across all ingested sources (and no source observed an intra-source session ambiguity). `"matched"` holds **if and only if** `native_session` is non-null; when multiple sessions exist, the reader never picks one.
      - `'ambiguous'`: Commit is observed in more than one distinct session across sources (or within a source / in a file with `header_conflict = TRUE`). `native_session` is `NULL`.
      - `'unmatched'` / `'unknown'`: Commit is observed only in records without a harness-minted session. `native_session` is `NULL`.
    - **Parent consistency & single-row export coherence (`R5`):**
      - If two or more sources report `merge_status = 'observed'` for the same `commit_sealed` with **different** `parent_sealed` lists, the resolved row sets `merge_status = 'not_observed'`, `is_merge = NULL`, `merge_missing_reason = 'conflicting_parents'`, `parent_sealed = []`, and `parent_hmacs = []`.
      - Exported fields (`export_status`, `pseudonym_key_id`, `commit_hmac`, `parent_hmacs`, `merge_status`, `is_merge`, `merge_missing_reason`, `parent_sealed`) are derived from **one coherent source row** — never by combining `parent_sealed` from one source row with `commit_hmac` or `parent_hmacs` from another.
      - When HMAC rows carry different `pseudonym_key_id` values across sources, only rows matching the currently configured `FactExportConfig::key_id()` are eligible to be `'exportable'`; rows under any other key ID are treated as `'non_exportable'`.
      - A row is `'exportable'` only if that single chosen row has a valid `commit_hmac`, a matching `pseudonym_key_id`, and (when `merge_status = 'observed'`) `parent_hmacs.len() == parent_sealed.len()` under that same `pseudonym_key_id`.
- **Local-Only Persistence & Downstream `D4b` Handoff (`P4`, `P5`):**
  - Raw commit SHAs, branch names, working directories, and commit messages are **never** persisted to DuckDB. `commit_sealed` and `parent_sealed` always seal canonical SHAs with `hmac_domain(tool_key, "git_commit", canonical_sha)` (`commit:<64-hex>`).
  - `D4c` persists **only** local facts plus `pseudonym_key_id`, `commit_hmac` (`fact-export.v1/commit`), and `parent_hmacs` when `dendro_core::FactExportConfig::state()` is `FactExportKeyState::Ready` (`export_status = 'exportable'`). Without valid fact-export key material (`FactExportKeyState::Unconfigured`, `FactExportKeyState::InvalidKeyMaterial`, or `FactExportKeyState::InvalidKeyId`), rows are stored with `export_status = 'non_exportable'`, `pseudonym_key_id = NULL`, `commit_hmac = NULL`, and `parent_hmacs = '[]'`, and are upgraded in place on the next ingest once key material is configured.
  - **Contract `source_id`, `session_key`, and `git_evidence_key` are not persisted by `D4c`:** Downstream `D4b` (`fact-export-v1` exporter) reads cross-source resolved rows via `AnalyticsStore::load_git_evidence()`, filters to `export_status = 'exportable'`, maps local `'not_observed'` values of `merge_status` and `default_branch_status` to `'unknown'`, and computes `source_id` (`derive_export_source_id`), `session_key` (`derive_export_session_key` when `session_match_status == 'matched'`), and `git_evidence_key` (`derive_export_git_evidence_key`) at export time from a provisioned logical source ID (see [`docs/link-id-config.md`](link-id-config.md#4-fact-export-v1-pseudonym-key-configuration-mtn-21--d4c--d4b)).

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

---

## 6. Field-Level Connector Fidelity & Insight Denominator Matrix (`dendro#134`)

Every connector in `CONNECTOR_DECLARATIONS` (`crates/dendro-core/src/insight_v1/fidelity.rs`)
declares what it supplies across the closed 28-field `Field` enum (`input_tokens`,
`output_tokens`, `cache_read_tokens`, `cache_write_tokens`, `reasoning_tokens`,
`observed_cost`, `message_id`, `native_session`, `turn_timestamp`, `model`, `effort`,
`context_window_usage`, `tool_call_name`, `tool_outcome`, `tool_fingerprint`,
`session_edges`, `skill_declarations`, `skill_invocations`, `activity_intervals`,
`queue_transitions`, `process_identity`, `process_resource_samples`,
`privileged_effect_trace`, `security_policy`, `security_command_check`,
`source_coverage`, `trajectory_vector`, `harness_envelope_shape`). Each declaration
maps a `(connector, Field)` pair to a `FieldFidelity`:
- `Observed`: parsed directly from native session or verified export records.
- `Derived`: deterministically computed from observed fields during ingestion (e.g., Codex cumulative-delta token accounting and `input_tokens = input_raw - cache_read - cache_write`, rejecting negative values with a parse error at `crates/dendro-connectors/src/stream.rs:1469-1476`, or heuristic `$skill` / `skills/<name>/SKILL.md` path matching in Codex and Pi).
- `Partial(PartialFidelityReason)`: supplied only under an explicit typed condition (e.g., Codex `tool_outcome` is `Partial(ExplicitSignalOnly)` when an explicit exit code or error object is present; Claude `effort` is `Partial(ParsedWhenPresentNotObservedInCaptures)`).
- `Absent`: not supplied by the connector's parser (the honest default).

Every registered insight (`I01`–`I25`) declares its `required_fields` and `optional_fields`
directly on its registry entry in `crates/dendro-core/src/insight_v1/reference/mod.rs`.
Evaluating `(insight, connector_set)` via `evaluate` / `evaluate_catalog` or `evaluate_denominator`
produces a closed two-state `DenominatorEvaluation`:
- `Computable { scope: DenominatorScope }`: at least one connector supplies all required fields. Connectors lacking any required field are placed in `scope.excluded` with their missing fields, filtered out via the included-connector allow-list (`allows_harness`), and disclosed in `coverage` (`excluded: <connector> — missing <fields>`, or `excluded: unregistered harness` for unregistered harness values).
- `NotComputable { missing, by_connector, .. }`: no connector in the evaluated set supplies all required fields. The insight refuses with `InsightStatus::Refused { reason: "not_computable: missing ..." }` (mapped to display badge `insufficient_evidence`), preserving its original coverage text and appending the missing-field refusal and exclusion list, never emitting a guessed or zero value. See `docs/providers/provider-matrix.md` for the full generated field-fidelity and denominator matrix.
- **Single Source of Truth & Deferred Cross-Connector Joins:** `CONNECTOR_DECLARATIONS` is the single source of truth for denominator eligibility across both scoped and unscoped evaluation paths; there are no runtime overrides or ad-hoc table presence carve-outs. Every fact row is filtered strictly by its own harness (`allows_harness`), and unattributable rows are excluded and counted in `coverage` (`excluded: N unattributed rows`) whenever an allow-list is active. Cross-connector joins (such as combining `openrig` context, skill declarations, or queue transitions with individual harness rows) are deferred from the MVP and require an explicit declared-join design.
