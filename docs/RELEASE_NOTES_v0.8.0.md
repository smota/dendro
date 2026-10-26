# Dendro v0.8.0 Release Notes — Storage Privacy, Lineage, Process & Security Facts, and Insights

**Release Date:** TBD (not tagged)
**Tag:** `v0.8.0`
**Previous Release:** `v0.7.0`

---

## Executive Summary

**Dendro v0.8.0** expands Fleet Intelligence across model, harness, agentic-platform (OpenRig), environment, and security layers on native Windows and Linux. Storage defaults to a private per-user data folder with DuckDB spill/WAL guards and schema 3→4 migration (`dendro migrate` / `dendro rollback`). Ingest routes through a builtin capture registry with a native Grok CLI session reader, bounded input ceilings, and per-file `complete` / `partial` / `refused` coverage. Five new additive fact tables capture text-free message metadata, unprivileged process identities and resource samples (`dendro watch` and `dendro run-agent`), sub-agent session lineage edges traced to root callers, and closed-vocabulary security and dangerous-command facts. Accounting separates observed from estimated cost, keeps estimates unknown while the pricing table ships empty, and enforces conserved fleet totals. A deterministic insight layer (`dendro insights`) computes six reference insights (`I02`, `I04`, `I08`, `I09`, `I14`, `I18`), surfaced across the TUI, status, offline dashboard, and Nao semantic views.

Analytical rules are unchanged. Missing observations stay unknown and are never shown as `0`. Source logs stay read-only. `#![deny(unsafe_code)]` and parameterized DuckDB queries stay in force. The workspace package version is `0.8.0`.

---

## Highlights

### 1. Private default data folder, storage guards, and schema 3→4 migration

- **Private per-user default database:** `%LOCALAPPDATA%\Dendro\dendro.duckdb` on Windows and `$XDG_DATA_HOME/dendro/dendro.duckdb` (`~/.local/share/dendro`, mode `0700`) on Linux, created private on first use. An existing folder that is not private is refused and never re-permissioned. `dendro storage-init` with no flags creates or verifies the default folder (`--storage-root` is unchanged).
- **Private DuckDB runtime and recovery:** DuckDB `temp_directory`, `extension_directory`, and extension autoinstall/autoload are applied via `duckdb::Config` at open time before WAL replay, followed by `lock_configuration` and verification. DuckDB uses the bundled `json` feature with no ambient extension loading. Storage recovery uses direct journal lookup (removing the ~2,048-publication scan limit), selected-storage binding and cleanup, and Linux mount/publication parity (`renameat2` `NOREPLACE`/`EXCHANGE`).
- **Schema 3→4 migration and rollback:** `dendro migrate` and `dendro rollback` perform schema 3→4 migration with publication, rollback, and loss receipts.

### 2. Capture registry, native Grok reader, input ceilings, and truthful coverage

- **Capture registry dispatch:** `dendro run` routes every reader through `Registry::builtin()` using production descriptors for `claude`, `codex`, `pi`, `grok`, `cursor`, `windsurf`, `goose`, and `devin` (`agy` is registered as pending).
- **Native Grok CLI reader:** Reads `~/.grok/sessions/<cwd>/<session>/updates.jsonl` (`USERPROFILE` on Windows, `HOME` on Linux) with byte-offset watermarking so re-runs read appended lines only. Stores one usage row per `modelUsage` entry (`cachedReadTokens` inside `inputTokens`, `reasoningTokens` inside output) and joins `tool_call` to `tool_call_update` by call id. Thought/message chunks, hooks, compaction, sub-agent spawn/finish, and background tasks are counted. Only token integers, timestamps, tool names, and a keyed fingerprint of `rawInput` are persisted; sibling files holding prompts are never read.
- **Input ceilings:** Native readers enforce bounded record reads (8 MiB per record; oversize lines are quarantined and skipped), JSON nesting depth ≤ 64, and ≤ 1024 decoded records per batch with continuation. Descriptor limits may only tighten global ceilings.
- **Truthful coverage:** Per-file coverage is recorded as `coverage=complete|partial|refused` with projected and unprojected counts and reasons in ingest notes. One bad export file is refused while the rest of the run continues; a missing or failed source is reported `refused`, never `0`.

### 3. Message, process, lineage, and security facts

All five new schema-4 fact tables (`message_facts`, `process_identities`, `process_samples`, `session_edges`, `security_facts`) are created lazily with an exact-column allow-list and sealed keys; no message text, command lines, raw paths, or secrets land in `VARCHAR` columns.

- **Message metadata (`message_facts`):** Projects text-free message facts (`role`, `direction`, `modality`, `source order`, `grain`) from Claude, Pi, Codex, and Grok using the typed `domain_v1` vocabulary.
- **Process & environment facts (`process_identities`, `process_samples`):**
  - `dendro watch [--interval 1s] [--duration] [--once] [--harness …] [--flush-every 60s]` samples the user's harness processes (`claude`, `codex`, `grok`, `pi`) and their descendants via `sysinfo` 0.39 (`system` feature only), bounded to ≤ 64 tracked processes and ≤ 600 samples per process per watch, with PID reuse detection by start time and retry on flush failure (restoring failed batches, stopping after 5 consecutive failures or exceeded watch bounds, and retrying final flush 3×).
  - `dendro run-agent [--harness NAME] -- <command> [args...]` launches a harness with inherited stdio, cwd, and env, pins its PID as the sampler root (`attribution = launched` vs `discovered` from `watch`; `null` on descendants), handles Ctrl-C via the shared console/foreground process group, and exits with the child's exit code.
  - Stores executable name only (never argv, environ, or raw paths), CPU time (µs), RSS / working set, I/O bytes, start/exit times, cwd as a keyed fingerprint, and Linux isolation hints (`/.dockerenv`, cgroup markers, WSL via `/proc/version`; otherwise `unknown`). Unreadable values are stored as `NULL` with a visibility limit (`permission`, `exited`, `PID reuse`, `bounds`), never `0`.
- **Sub-agent lineage (`session_edges`):** Records `(parent, child, relation, status, evidence)` from explicit source fields only: Grok `subagent_spawned` / `subagent_finished` (`parent_session_id`, `child_session_id`), Claude Code sidechains (`isSidechain: true`, where `sessionId` is parent and `agentId` is child), and Codex `payload.source.subagent.thread_spawn.parent_thread_id` (Pi has no parent field). Messaging or turn links (`sender_thread_id`, `receiver_thread_ids`, `parent_turn_id`, `parentUuid`) are counted as `lineage_not_explicit` and never projected as edges. Root-caller walks in `dendro-core` handle chains, diamonds, cycles (stopped and flagged), and missing parents (flagged, never invented). Coverage summaries gain `edges=<n>`.
- **Security facts (`security_facts`):** Projects explicit policy fields per change (not per line) using closed vocabularies (`other` for unknown): Claude Code (`permissionMode`, `permissionDecision.decision`, `sandboxed`, `dangerouslyDisableSandbox` → `sandbox_override_requested`), Codex `turn_context` (`approval_policy`, `sandbox_policy.type`, `permission_profile`, and the count of `approved_command_prefixes`), and Pi bash command categories (Grok has no permission fields). A deterministic dangerous-command classifier records only the category (`recursive_delete`, `pipe_to_shell`, `force_push`, `chmod_world_writable`, `sudo`, `disk_write_device`, `credential_file_access`, `network_exfil_pattern`), never the command text.

### 4. Observed vs estimated accounting and conserved totals

- **Grok observed cost:** Converts `costUsdTicks` to observed USD using `GROK_USD_TICKS_PER_USD = 10^10`. Multi-model turns without per-model cost attribution leave each model row `None`, emit warning `cost_not_attributable`, and store the turn cost once on a zero-token, model-less row.
- **Observed vs estimated separation:** Estimated cost reads from a pricing table that ships empty, so estimates are `null` (`unknown`), never `0`, and are never mixed with observed cost. Any total over rows with unknown cost reports `partial` or `unknown` with a count and `usd: null` rather than a smaller number.
- **Conservation:** Fleet totals count each usage observation once, and exclusive session totals sum to the fleet total. Per-root inclusive totals (including sub-agents via `session_edges`) are labelled non-additive. `accounting_v1::check_total_admission` admits these defined totals and refuses shared/legacy attribution, cumulative counters, duplicates, mixed currencies, and source-reported totals. Report JSON adds `observed_cost`, `estimated_cost`, `cost_rule`, and per-session `inclusive`.

### 5. Insight contract (`dendro insights`) and six reference insights

`dendro insights [--json] [--harness H] [--session S]` evaluates registered insights over a read-only scoped input using fixed, parameterized queries (no caller SQL, no runtime plugins). Each insight returns `Computed`, `Partial { reason }`, or `Refused { reason }` with population, measures (`None` = unknown, never `0`), a coverage note, and at most 5 sealed evidence keys.

| ID | Name | What it computes |
| --- | --- | --- |
| `I02` | `model.tokens.direct` | Disjoint token components per model, with `cache_read_ratio_of_sums` and `cache_read_mean_of_ratios` as separate measure IDs |
| `I04` | `model.cost` | Observed vs estimated USD kept separate; partial or unknown totals are never reported as a smaller sum |
| `I08` | `harness.action_failure` | Failed vs settled terminal actions, with unsettled actions counted separately |
| `I09` | `work.lineage_shape` | Edge kinds, diamonds, roots, and depth; cycles are flagged and unresolved parents are never invented |
| `I14` | `environment.resources` | Per-process CPU and disk deltas summed per tree, max RSS per unit, and `launched` vs `discovered` attribution |
| `I18` | `coverage.source` | Read, projected, unprojected-by-reason, and refused counts per source |

### 6. CLI, TUI, dashboard, status, and Nao surfaces

All operator surfaces render canonical results (`insight_v1::evaluate` and existing reports) without recomputing numbers, labelling unknown, partial, and refused values rather than displaying `0`:

- **TUI cockpit:** Adds screen `[7]` Insights with L0–L3 progressive disclosure (`domain` → `insight` → `measures` → `DetailJson`) and displays observed vs estimated cost on the Tokens screen.
- **Dashboard and status:** Public dashboard payload adds `insights` (with evidence cleared), `lineage` (`I09`), `environment` (`I14`), and `security` (closed-vocabulary counts), rendered by offline `site/viewer.html` via `textContent` with no `fetch` or external scripts. `dendro status` reports row counts for the five new fact tables (`not created` in text when absent).
- **Nao semantic layer:** Adds semantic views and models `v_message_mix`, `v_subagent_lineage`, `v_security_posture`, and `v_process_resources` (counts, max, and group-by only; CPU and cost remain in `I14` and `I04`), tolerating absent lazy tables.

---

## Breaking changes

- **Databases in ordinary (non-private) folders and legacy locations are refused:** Opening `./dendro.duckdb` in an ordinary working directory or passing `--db` in a non-private folder is refused before any database open with an error naming the default private folder (`%LOCALAPPDATA%\Dendro\dendro.duckdb` on Windows or `$XDG_DATA_HOME/dendro/dendro.duckdb` / `~/.local/share/dendro/dendro.duckdb` on Linux) and the exact copy command (PowerShell `Copy-Item` on Windows or `cp` on Linux, including the `.wal` file when present). Nothing is copied or moved automatically.

---

## Known limits

- **Nao new views are all-time counts:** `v_message_mix`, `v_subagent_lineage`, `v_security_posture`, and `v_process_resources` report all-time counts (time window not applied).
- **`strace`/ETW import deferred to v0.9:** Linux `strace` syscall-trace import and Windows ETW event import are deferred to v0.9 until privileged producer specimens are available.
- **AGY reader not included:** The AGY adapter is cut from v0.8.0 and remains registered only as `pending`.
- **Claude sidechain usage keyed on parent session:** Claude sidechain usage rows are keyed by `sessionId` and sit on the parent session row, so per-session parent totals include sidechain usage (fleet totals still count each child observation once).
- **Multi-model Grok cost marked partial:** Per-model rows of a multi-model Grok turn without per-model cost attribution have `None` cost, so fleet or model observed cost over those rows is marked `partial` / `unknown`.
- **Status shows `rows: 0` for absent tables:** When the five lazy schema-4 fact tables have not yet been created, status reports `rows: 0` (alongside `not created` in text).
- **Windows cross-volume CI job non-blocking:** The hosted Windows cross-volume CI job is non-blocking.
