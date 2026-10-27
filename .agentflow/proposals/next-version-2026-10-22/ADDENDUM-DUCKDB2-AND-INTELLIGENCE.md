# Dendro vNext addendum: DuckDB 2.0.0 LTS, extensions, active intelligence, presentation and documentation

Status: **normative vNext scope, authored 2026-10-27 and amended the same day for the second-model gate.** This page proposes delivery. It performs no migration, crate bump, schema change, or product adoption. Goal 9 in `docs/BACKLOG.md` is the backlog home. `DESIGN.md` remains the meaning of facts and of insights I01–I21. `SCOPE-TRIMMED.md` (2026-10-24) remains the v0.8.0 build decision and is not reopened. `tasks.json` remains the v0.8.0 DAG. This addendum does not edit that inventory (35 tasks, 28 release requirements, 69 acceptance criteria).

Requirement IDs `R29`–`R33` and acceptance IDs `A01`–`A15` belong to this addendum. They are not the assumption IDs `A09-01`–`A09-05` in `.agent-runs/goals/goal-9-acceptance-contracts-v1.md`. Where those assumptions still bind, section 10 says so.

Upstream engine facts below are taken from the DuckDB 2.0.0 LTS announcement and the 2.0 storage, migration, FTS, and graph-query docs (published 2026-10-21, codename “Cinnamon Teal”, *Spatula cyanoptera*, LTS through at least October 2027). Exact SQL that an admission has not yet executed against the pinned library stays an admission freeze, in the same sense as `DESIGN.md` section 8: versions are targets until B-style evidence exists. Numeric budgets are design targets. Product validation remains future work.

## 0. How to read this page

| Question | Where it is answered |
|---|---|
| What does v0.8.0 already mean? | `DESIGN.md`, `SCOPE-TRIMMED.md`, Goal 8 in `docs/BACKLOG.md` |
| What must vNext add? | Sections 3–7 (`R29`–`R33`) |
| What proves an acceptance ID? | Section 8 |
| What must a later PR not “fix” by drifting? | Section 9, then `A13` and `A15` |
| What stays out? | Section 10 |

Proposed paths and test names are reservations for authorized delivery. They are not claims that those files exist today.

## 1. Baseline this addendum admits

| Fact | Evidence |
|---|---|
| Workspace crate requirement `duckdb = { version = "1.1", features = ["bundled", "json"] }` resolves to locked `duckdb` / `libduckdb-sys` `1.10506.0` (DuckDB 1.5.6). | `Cargo.toml`, `Cargo.lock` |
| Product publication in v0.8.0 is schema 4 via `MIGRATION_TARGET_SCHEMA_VERSION = 4`. `facts::SCHEMA_VERSION` remains `3` so a historical opener can still name the pre-migration database. The reader admits schema 3 or target 4. | `crates/dendro-core/src/facts.rs`, `crates/dendro-core/src/migration_v4/mod.rs`, `crates/dendro-core/src/storage.rs` |
| Private open sets `temp_directory`, `extension_directory`, `autoinstall_known_extensions=false`, and `autoload_known_extensions=false` before `open`, including WAL replay, then `allowed_configs=['memory_limit','threads','preserve_insertion_order']` and `lock_configuration=true`. | `crates/dendro-core/src/storage/library_privacy.rs` |
| Linux private data directory mode is `0700`. Windows uses the private DACL (owner, SYSTEM, Administrators). | Goal 8 `B02 / T24`; `DESIGN.md` section 8 |
| v0.8.0 shipped six reference insights: `I02`, `I04`, `I08`, `I09`, `I14`, `I18`. The other fifteen remain Goal 9. | Goal 8 `T15 / T16`; Goal 9 P2 |
| Answer status is `Computed` / `Partial` / `Refused`, with at most five sealed evidence keys. Presenters do not recompute measures. | `DESIGN.md` section 5; `docs/contributing/insights-v1.md` |
| Lineage walks already live in Rust `session_lineage`. | `crates/dendro-core/src/session_lineage.rs` |
| Dashboard home and eight views are Obsidian Edition pages. The live badge string today is `ILLUSTRATIVE SNAPSHOT · LIVE SYNC READY`. Surfaces are `#090d16` / `#0f1927`. Viewer allowlists are a closed `Set` of the six shipped insight IDs. | `site/dashboards/style.css`, `site/dashboards/index.html`, `site/viewer.html` |
| Nao views shipped in v0.8.0: `v_message_mix`, `v_subagent_lineage`, `v_security_posture`, `v_process_resources`, plus the earlier scorecard views. They are all-time. Half-open windows stay the existing Goal 9 “Nao windows” row. | `docs/NAO_SEMANTIC.md`, `templates/nao/prompts.md` |
| DuckDB 1.5.x files use storage version 68. DuckDB 2.0 creates storage version 69 by default. A version-69 file cannot be opened by 1.x. DuckDB 2.0 can still read a version-68 file when it does not rewrite it. | DuckDB storage docs for 2.0 |

Two numbers must stay distinct. **Product schema 5** is Dendro’s migration target. **Storage version 69** is DuckDB’s on-disk format. A receipt that reports one of them has not reported the other.

## 2. Requirement index

| ID | Acceptance | Title | Depends on |
|---|---|---|---|
| `R29` | `A01` `A02` `A03` | DuckDB 2.0.0 LTS engine, schema 5, single-owner quack | v0.8.0 schema-4 publication |
| `R30` | `A04` `A05` `A06` `A07` | Sanitized `VARIANT`, tripwire triggers, `duckpgq`, `fts`, trajectory distance, DuckLake 1.1 | `A02`; `A06` also needs the pinned `duckpgq` blob |
| `R31` | `A08` `A09` `A10` `A11` `A12` | I01–I25 and `dendro advise` | `A08` may proceed on schema 4 for I01–I21; `A10` needs `A04`–`A07` |
| `R32` | `A13` `A14` | Presentation parity and the Obsidian visual contract | The fact or insight being marked done |
| `R33` | `A15` | Documentation, Nao catalog, and version-stamp parity | The same PR as the change it describes |

`R32` and `R33` are release gates on top of `R29`–`R31`. An analyzer that computes the right number and skips the surfaces or the docs is not done.

## 3. `R29` — DuckDB 2.0.0 LTS engine and sealed storage

vNext must run DuckDB 2.0.0 LTS on native Windows and native Linux. WSL does not satisfy the Windows lane. macOS support that v0.8.0 already has is preserved. New sensor behavior is not invented for macOS.

DuckDB’s own announcement says the Rust client can lag the CLI and Python client. `A01` stays blocked until the published or vendored crate digest exists. The requirement is not waived by building against a newer C++ library while the Rust crate remains `1.10506.0`.

### `A01` — Engine and crate pin, with shadow validation

The workspace requirement becomes:

```toml
duckdb = { version = "~1.20000.0", features = ["bundled", "json"] }
```

`~1.20000.0` is the Cargo-compatible encoding of the DuckDB 2.0.0 line used by `duckdb-rs` (the same encoding that made DuckDB 1.5.6 lock as `1.10506.0`). Features stay `bundled` and `json`. No feature may download extensions. A lockfile change inside that tilde, and any proposed pin outside it, reruns the shadow suite. A crate whose runtime version string is not `v2.0.0` fails admission even if the Cargo version parses.

`Cargo.toml` must stop using the floating `1.1` requirement. That range is what selected `1.10506.0`.

Shadow validation (`shadow-v200`) is mandatory before the default database is cut over:

1. Copy a schema-4 database that was written by `1.10506.0`. Do not open the user’s live file.
2. Open the copy with the 2.0 library while `storage_compatibility_version` keeps the file at storage version 68. The copy must still open with `1.10506.0` afterward.
3. Compute the six shipped insights (`I02`, `I04`, `I08`, `I09`, `I14`, `I18`) on both libraries. Status, measures, refusal reasons, and sealed evidence keys must match.
4. A second copy is the migration rehearsal in `A02`. The shadow copy is not that rehearsal.

The C++17 compiler requirement is an admission input for both native lanes. It does not by itself prove the crate pin.

### `A02` — Storage version 69, schema 5, and rollback

Product migration is schema 4 → schema 5, in a new module reserved as `migration_v5`, following the schema 3 → 4 rules in `DESIGN.md` section 8.

- Checkpoint and close the owned database. A dirty, uncheckpointed WAL fails the migration before any backup is created. Nothing is copied off a live writer.
- Create the pre-upgrade backup as an immutable versioned private directory named `dendro.duckdb.pre-v200.bak`. Unix creates it with `O_CREAT | O_EXCL`. Windows uses `CREATE_NEW`. If that name already exists, migration refuses. It does not overwrite, truncate, or replace the existing backup on a re-run.
- The backup is one atomic set. The set contains the checkpointed main database, the WAL that belongs to that checkpoint, the privacy-key sidecar, and the provenance sidecar files that sit beside the database. When checkpoint has removed the WAL, the manifest records `wal: absent` and the other members are still required. The migration receipt records a SHA-256 of every member.
- This set follows `DESIGN.md:188`. It is an immutable versioned private directory. Cutover and rollback select it by atomically replacing a regular pointer manifest on the same volume. The manifest names that directory's identity and digest. The migration never replaces a directory or junction, and it never deletes the live database to force a rename. A sharing violation or an interrupted copy leaves the old pointer in place.
- The v0.8.0 binary must still open the backup's main database. The new binary must refuse to treat the backup directory as the live schema-5 database.
- Cutover creates a schema-5 database at DuckDB storage version 69. Inspection of the storage version uses the statement frozen in this admission against the pinned library (the 2.0 docs expose `storage_compatibility_version` and `ATTACH ... (STORAGE_VERSION ...)`). A receipt that only records “2.0” fails.
- Opening the version-69 file with `1.10506.0` must fail cleanly. The test asserts the failure and asserts that every backup member's bytes are unchanged.
- Rollback stops writers, selects `dendro.duckdb.pre-v200.bak` through the pointer manifest and the v0.8.0 binary, and leaves schema-5 facts off the restored database. The loss receipt names the watermark, the excluded schema-5 counts, and whether source logs can replay them. Rollback does not delete the failed schema-5 file and does not rewrite the backup.
- An interrupted migration leaves the live pointer on the schema-4 database.

During the transition window, every open that must not rewrite a version-68 file sets `storage_compatibility_version` to the v1.5.6 compatibility value **before** `lock_configuration`. After cutover, new files are version 69. The setting is not left on the post-lock `allowed_configs` list for ordinary sessions. `autoinstall_known_extensions=false`, `autoload_known_extensions=false`, and `lock_configuration=true` remain in force, still applied so they hold during WAL replay.

Privacy layout, enforced by `library_privacy` and the platform private-IO crate:

| Object | Linux | Windows |
|---|---|---|
| Private directory, spill, extension directory, socket directory | mode `0700`, owner only; group/other access fails closed | Existing private DACL. SYSTEM and Administrators stay, per Goal 9 anti-goal 1 |
| Database, WAL, backup-set members, receipt, privacy-key sidecar, provenance sidecars, quack token sidecar | mode `0600` | Same private DACL, no extra principal |

Tests must observe those modes under the CI umask without changing the process umask (Goal 9 test-hardening row). An existing non-private directory is still refused and left untouched.

### `A03` — Single owner through the quack server

`dendro serve`, the single-owner daemon, is the only process that opens a raw DuckDB connection or a raw `quack` SQL session. `quack` stays inside that trusted daemon boundary. The daemon is also the only read-write opener of the live database.

External clients never receive a raw SQL `quack` session and never receive a DuckDB connection. The client set is the `dendro` CLI, the TUI, the HTTP server, Nao, and live agent hooks. They talk only to the daemon over a typed, parameterized RPC/MCP transport. Request handlers, including HTTP handlers embedded in the same binary, do not hold a SQL session.

The daemon itself may use the DuckDB 2.0 `quack` protocol inside that boundary. Session start is the documented `CONNECT` / `ATTACH 'quack:…'` sequence. Session end uses the documented closer. The 2.0 announcement specifies `CONNECT` and does not name a `DISCONNECT` statement. Admission freezes the closer from the pinned quack docs. If that closer is `DETACH` or another statement, the test still proves the daemon session ends and the server releases it. A raw `quack_serve(token => '…')` example from the announcement is not the product configuration, and it is never handed to a client.

Transport bind. File mode makes the socket private to the user. It does not define what a same-user client is allowed to ask.

- Unix: a Unix domain socket inside the private directory, mode `0700`. It must not be reachable through a group or other bit.
- Windows: a Named Pipe with the private security descriptor (owner, SYSTEM, Administrators). No extra principal.
- Unauthenticated loopback TCP is forbidden, including `127.0.0.1`. Any local user can connect to that socket. `0.0.0.0` is forbidden. `allow_other_hostname` is forbidden.
- A TLS reverse proxy in front of quack is out of scope. Quack is not a network service. Operator push channels, including dashboard notifications, use this same transport. They do not open a second TCP listener.

The quack token is generated locally, stored in the `0600` sidecar, and stays inside the daemon. It is never placed in argv, logs, answers, dashboard HTML, MCP tool text, or a client credential. A client credential authorizes one typed capability. It cannot be upgraded to SQL.

Security boundary. Same-user agent subprocesses can open a `0700` socket, because they run as the same uid. Isolation from those subprocesses is the typed RPC capability check, not the file mode.

- CLI, TUI, HTTP, and Nao call typed methods with bound parameters. Nao still selects a typed intent. None of those methods accepts a SQL string.
- Live agent hooks get only a typed, capability-scoped append/advise endpoint. The endpoint accepts structured facts and advisory questions. It rejects an arbitrary SQL string, a statement template, or a request to borrow the daemon's connection.
- Answer reads stay read-only and allowlisted.
- `DESIGN.md` section 7 still prohibits `ATTACH`, `COPY`, file/URL/network readers, and dynamic SQL on answer paths and on `dendro advise`. The daemon may run its own frozen SQL. That SQL is compiled with the binary, reviewed as code, and is not a client-supplied string.
- If the daemon is not running, one trusted `dendro` process may open the file itself, which is today's embedded CLI. That process is then the single owner. It still does not hand a `quack` session or a SQL parameter to another process, hook, or Nao client. A second writer must fail. A live daemon must reject a direct open of the same file.

`#91` (Windows `publish_replace` sharing violation) stays the P0 defect. Quack does not replace that fix. Publishing a folder while the daemon holds the database must fail closed and keep the old pointer.

## 4. `R30` — `VARIANT`, triggers, and admitted extensions

Extensions do not weaken `library_privacy`. `INSTALL … FROM community` at runtime, network install, and `autoinstall` / `autoload` stay off. An extension used by vNext is a hash-pinned blob placed in the private `extension_directory` by an explicit maintenance command. `LOAD` reads that blob. A hash mismatch refuses the feature and leaves the typed facts queryable through the Rust fallback where this page defines one. `lock_configuration=true` still ends the open. Any setting an extension needs (`active_grammar_extensions`) is applied to a frozen value before the lock, never from a client string.

### `A04` — `envelope_shape VARIANT`

Schema 5 adds one table, `harness_envelope`:

| Column | Rule |
|---|---|
| `observation_id` | Primary key, Dendro-issued |
| `harness`, `harness_version` | Closed identifiers already admitted by capture |
| `captured_at` | Evidenced timestamp |
| `envelope_shape` | `VARIANT`, schema-on-read |

The column exists so DuckDB 2.0 can shred a heterogeneous shape into columnar sub-columns. Writers must not store JSON text as the query path. `variant_keys` / `variant_type` are the read API. There is no parallel JSON document column.

The shape is body-free harness metadata: MCP capability flags, tool argument **types**, and provider counters. Values are booleans, numbers, closed tokens, and key paths. They are not prompts, transcripts, argv, paths, secret material, or tool-argument values.

Keys are a second closed gate. Each harness has a versioned key allowlist shipped with the analyzer. A writer accepts a key only when it is on that list. An unknown key is dropped or replaced with an HMAC of the key name under the privacy key. The raw key name is not stored. Value sanitization does not make an arbitrary key safe: a secret or a path sitting in the key name must not enter `envelope_shape`. The `envelope_variant` test places a secret-shaped canary in a key position and asserts that the stored `VARIANT` does not contain that canary.

Bounds, refused as a whole row (no silent truncation): nesting depth ≤ 8, encoded size ≤ 4096 bytes, ≤ 64 keys, string tokens ≤ 64 bytes, arrays ≤ 16 items. A refused envelope does not drop the typed fact the capture already stored. Overflow counts as a coverage refusal.

Depth 64 on ingest (`T06`) is a parse ceiling for hostile input. It is not permission to persist a depth-64 variant.

### `A05` — Triggers and `_dendro_tripwire_outbox`

DuckDB 2.0 `CREATE TRIGGER` is allowed only inside the migration-owned schema, for two jobs:

1. Maintain `_dendro_tripwire_outbox` and the incremental watermarks in the same transaction as the fact write.
2. Keep an opted-in `fts` index in step with its redacted corpus (`A07`).

Triggers are `AFTER` and `FOR EACH STATEMENT` unless admission shows that form cannot express the watermark. `BEFORE … FOR EACH ROW` and `UPDATE … FOR EACH ROW` are unsupported in 2.0 and must not be required. Trigger bodies are frozen SQL in reviewed Rust strings. They perform no network I/O, no `ATTACH`, and no `COPY`.

A database `TRIGGER` body is strictly **O(1) per statement**. It may only increment a per-session counter or watermark, and append one lightweight dirty-session notification row to `_dendro_tripwire_outbox`. It may read at most one row of pre-computed frozen thresholds. It must not scan a historical turn window, compute a cohort centroid or quantile, evaluate a k-turn Markov window, or run a multi-row join inside the write transaction. `I22` (`harness.trajectory_vortex`) does that work in the versioned Rust `insight_v1` analyzer, after commit or on demand. It does not do that work inside the trigger. See `A10`.

Outbox columns: `id`, `insight_id`, `subject_key`, `turn_k`, `watermark`, `payload_kind` (closed enum), `created_at`, `dispatched_at`. A row written by the trigger is a dirty-session notification. `payload_kind` names that notification. The row has no free-text column and no cohort measure. The analyzer attaches sealed evidence IDs and numeric measures to its own answer after it runs. It does not fold that work back into the trigger body.

The Rust dispatcher runs after commit. It reads undispatched notification rows and runs the registered analyzer, or leaves the row for an on-demand read. Operator-visible clients pull the resulting advisory over the `A03` typed transport. A dispatcher failure leaves `dispatched_at` null and does not roll back the fact transaction. Redelivery is at-least-once and idempotent on `id`. A reconciliation query is allowed only as recovery and must be labeled recovery. The steady state does not poll fact tables to discover tripwires.

Tripwire alerts are operator-visible and pull-only advisory metadata (`A12`). The dispatcher must not inject them into a live agent's prompt, tool result, or context window. That keeps Goal 9 anti-goal 4: the intelligence layer is not autonomous live control.

The `tripwire_outbox` test rejects any frozen trigger body that scans a historical table, computes a centroid or quantile, or contains a join. The allowed body is the counter or watermark update, the single-row threshold read, and the notification insert.

### `A06` — `duckpgq` and the Rust fallback

`duckpgq` is the community SQL:2023 property-graph extension. After the pinned blob is loaded, the open sets:

```sql
SET active_grammar_extensions = ['duckpgq'];
```

That array is exactly `duckpgq`. It is not user input. The legacy parser is not enabled to make graph syntax parse.

The graph is a view over tables Dendro already writes. v0.8.0 physical lineage is `session_edges`. v0.8.0 security observations are `security_facts`. Schema 5 does not add a second writable copy of either.

| Graph name in queries | Physical source |
|---|---|
| Sessions / work-item nodes | Admitted session and work identities. Where no work-item grain exists, the node is the session and the work-item count is `not_observable` |
| `work_edges` | Projection of `session_edges` |
| `security_decisions` | Projection of grant, refusal, and sandbox rows in `security_facts` |

Edge kinds included are every kind `relationship-contract-v1` and `I09` already admit. The minimum blast-radius set is `spawn`, `delegate`, `handoff`, `return`, `join`, and `retry`. Kinds already stored and shown in the viewer allowlist (`delegate_existing`, `resume`, `dependency`) stay on the graph and stay labeled. Dropping them would change `I09`.

`duckpgq` reachability and `I24` filter strictly on `status = evidenced`. Edges with status `absent`, `truncated`, or `conflict` stay stored and stay visible as evidence. They are ineligible for reachability, as `DESIGN.md:63` already requires. A query that traverses those statuses fails the fixture.

Traversal direction is fixed. Queries do not pick an opposite direction to enlarge a set.

| Kind | Reachability direction |
|---|---|
| `spawn`, `delegate`, `delegate_existing`, `handoff` | Parent → child (creator, caller, or source → executor or successor) |
| `return` | Child → parent (executor → caller) |
| `join` | Child → parent, as continuation onto the waiting assignment. The stored `relationship-contract-v1` endpoint remains waiting assignment → child. Blast-radius traversal walks that edge back toward the waiter and does not import the child's descendants into the parent's authority set |
| `retry` | Prior attempt → distinct later attempt, as `relationship-contract-v1` already states |

Structural reachability identifies dependency exposure. It does not identify inherited authority or effective security authority. `DESIGN.md:54` forbids parent-to-child authority inheritance. `I24` must carry that non-claim on every answer, including `Computed` answers.

Queries that vNext may issue through frozen SQL: bounded multi-hop reachability, cycle detection, and the downstream set of a revoked or failed grant or an unverified sandbox. Hop limit defaults to 8. An unbounded path query is refused. Cycles are reported. They are not deleted to force a tree. The downstream walk uses the direction table above and the evidenced-only filter.

PageRank, weakly connected components, and clustering coefficients may be computed only as explicitly labeled descriptive statistics. They are not authority, not causality, and not a substitute for the reachability query. `I24` must not call them.

When the extension blob is absent, mismatched, or refused, `session_lineage.rs` answers the same reachability and cycle fixtures inside the hop limit. Divergent results fail the test. The fallback is allowed to refuse a query the extension would also refuse.

### `A07` — `fts`, trajectory distance, and DuckLake 1.1

Three separate mechanisms. None of them is an answer-path `ATTACH`.

**Full text.** The `fts` extension indexes only redacted diagnostic classes, capability names, and curated remediation labels. Corpus identifiers are `NOT NULL` primary keys so `PRAGMA create_fts_index(..., incremental := 1)` is legal. Incremental indexing is the 2.0 opt-in and is the mode Dendro uses. The index is a derivative. Deleting it must not delete facts. Queries are parameterized. There is no index over prompts, paths, argv, or message bodies.

**Trajectory vectors.** Schema 5 stores a `FLOAT[32]` column on the trajectory table reserved as `harness_trajectory`. The canonical comparison is exact `array_cosine_distance`. An in-memory `vss` HNSW index is optional, rebuilt from that column, and discarded with the process. Persistent on-disk HNSW is experimental upstream and must not hold canonical state. A HNSW answer that disagrees with exact distance beyond the frozen tolerance is a test failure, and the exact distance wins. `APPROX NEAREST` is not the canonical query.

**DuckLake 1.1.** An explicit operator command may publish immutable, pseudonymized Parquet partitions through the DuckLake 1.1 extension for cross-machine baseline comparison. “Cross-machine” means an operator can place partitions side by side later. Dendro does not open object stores, does not stream, and does not call another machine. The publisher is the only command allowed to `COPY` or `ATTACH` a DuckLake catalog, and only into the private export directory. Answer paths and `dendro advise` still cannot. Partitions contain the same projected fields as answers: no raw identifiers, no privacy-key sidecar, no prompt text. The catalog is content-addressed. A rewritten partition is a new object, not an update in place.

## 5. `R31` — Beyond-observability intelligence

Insights stay descriptive. They return `Computed`, `Partial`, or `Refused`. They do not emit p-values, leaderboards, or quality scores (Goal 9 assumption `A09-04` and anti-goal 3). Missing facts stay unknown. A zero is an observed zero and is labeled as one.

`I01`–`I21` keep the formulas, exclusions, and refusal conditions in `DESIGN.md` section 5 and in the Goal 9 P2 catalog. This addendum does not restate them in order to edit them. Registering a changed formula is a version bump and a different requirement.

### `A08` — Finish I01–I21

The fifteen insights not shipped in v0.8.0 (`I01`, `I03`, `I05`, `I06`, `I07`, `I10`, `I11`, `I12`, `I13`, `I15`, `I16`, `I17`, `I19`, `I20`, `I21`) are registered at measure/analyzer `1.0.0` and compute on captured facts. The six already delivered stay delivered. At `A08` merge time the exact-set gate is `I01`–`I21`, including the named `I02` cache submeasures. `I22`–`I25` are not required in that set. `A10` is the merge that expands the same gate to exactly `I01`–`I25`. An `A08` build that already lists `I22`–`I25`, or an `A10` build that omits any of `I01`–`I25`, fails its own gate.

`I15` `environment.effects` ships now as the `DESIGN.md` function. With no admitted T11 or T26 trace it returns `not_observable`. It must not be fed synthetic syscalls or fake ETW (anti-goal 2). Trace importers remain their own Goal 9 rows and their specimen gates.

`A08` may merge on the schema-4 / DuckDB 1.5.6 line. It is not accepted as vNext-done until `A13` and `A15` hold for each new ID.

### `A10` — I22–I25

These four analyzers ship at `1.0.0`. They are in the same registry and the same `Computed` / `Partial` / `Refused` contract. Inputs are projected facts. No analyzer reads prompt text.

| ID / inputs and formula | Conditions / refusal and exclusions |
|---|---|
| `I22` `harness.trajectory_vortex`. State is the closed pair (capability class, outcome class), overflow bucket `other`. A frozen projection, versioned with the analyzer, writes `FLOAT[32]`. Distance is `array_cosine_distance` to the centroid of completed sessions in the same harness cohort that were not themselves tripped. The analyzer reports a trip at turn \(k\) when the distance is above the frozen cohort quantile and the self-transition rate stays above the frozen threshold for \(k\) consecutive turns. Initial \(k\) is 3. Changing \(k\), the projection, or the quantile bumps the analyzer version. Cohort centroids, quantiles, and the k-turn Markov window run in this Rust `insight_v1` analyzer. The `A05` trigger only increments the watermark and appends the dirty-session notification. The dispatcher runs the analyzer asynchronously after commit. An operator may also run it on demand. | No prompt, argv, or path enters the state. Cohort sample below the frozen minimum → reason `insufficient_sample`, display badge `insufficient_evidence` (`A14`), and no trip. Missing capability class counts in `other` and is reported. The analyzer's answer is the signal. The notification row is only the schedule. Neither one stops the agent. The answer's L1 fields include assumptions and alternative explanations (`A13`), so a trip is not a cause claim. |
| `I23` `harness.schema_drift`. Per harness and harness version, the distribution of `envelope_shape` key paths and `variant_type`. The measure is the L1 distance between aligned distributions, plus the count of keys added, keys removed, and keys whose type changed. | Either side below the frozen minimum sample → `insufficient_evidence`. A refused envelope (`A04`) is excluded and counted. Type change and presence change are separate components. No compatibility verdict is inferred from a key name. |
| `I24` `security.authority_blast_radius`. Bounded reachability on the `A06` graph from a revoked grant, a failed grant, or an unverified sandbox to downstream sessions. Traversal uses only `status = evidenced` and the `A06` direction table. Reports the downstream session count, the hop histogram, and any cycle touched. Artifact counts are included only when a typed artifact identity already exists; otherwise artifacts are `not_observable`. | No applicable decision → `NOT_ASSESSED`, matching `I17`. Hop limit exceeded → `Partial` with the truncated frontier counted. `duckpgq` and `session_lineage` must agree on the fixtures, including fixtures where `absent`, `truncated`, and `conflict` edges are present and must be skipped. PageRank is not an input. Every answer states the non-claim: the count is dependency exposure, not inherited or effective security authority (`DESIGN.md:54`). |
| `I25` `model.counterfactual_routing`. On the admitted invocation series, an `ASOF JOIN` aligns cache-read fraction (`I02`) with the cohort baseline. `QUANTILE_CONT` locates the cache-bleed inflection the frozen rule defines. Candidate tier profiles come from a versioned pricing/tier table the operator pins by digest. The analyzer reads prices, context limits, and model identifiers only from that pinned table. It does not hardcode a model name. A second pass substitutes the pinned fast-tier profile and reports the simulated cost and the simulated context-pressure (`I07`) against the observed run. | Empty price table, an unpinned cohort, or an unpinned tier profile → estimated component `unknown`, simulation `Refused`, observed `I02` still reportable. The analyzer refuses rather than guessing a fast tier. Noncomparable mixes use the `I21` rule and return `noncomparable`. The result is labeled counterfactual. It is not an effect estimate and not a routing change. L1 assumptions and alternative explanations are required (`A13`). |

The `A05` trigger records a dirty session for `I22`. The `insight_v1` analyzer writes the `I22` answer after commit or on demand. `I23`, `I24`, and `I25` do not write the outbox unless a later version says so.

### `A09` — `dendro advise`

`dendro advise` is a CLI command and an MCP server in the trusted process. It answers four questions from canonical facts:

| Question | What it uses | Placebo / risk rule |
|---|---|---|
| `model_cohort_baseline` | `I02`, `I04`, `I21` comparability | A different version or a biased mix is `noncomparable`, not a winner |
| `skill_effectiveness_audit` | Counts along `declared → available → loaded → invoked → verified_effective` | `placebo` only when the skill was loaded or declared available, the sample meets the minimum, and verified-effective count is zero. Missing verification is `insufficient_evidence` |
| `delegation_loop_risk` | `session_lineage` cycles, retry edges, depth | A cycle or a retry chain past the frozen depth is the risk. Centrality is not the risk |
| `environment_readiness` | Placement, resources, isolation facts already admitted | Unknown stays unknown. A declaration alone is not readiness (`I16`) |

Each response carries evidence IDs (≤ 5), sample count, freshness (age of the newest admitted fact against the advisory clock), and one value of the named enum `AdviseUncertainty`: `sample_below_minimum`, `missing_denominator`, `noncomparable_mix`, `counterfactual_only`, `stale`. No other uncertainty prose is allowed. Staleness past the frozen bound sets `AdviseUncertainty::stale` and does not guess a number.

`AdviseUncertainty` is not a UI badge token and is not an `InsightStatus` reason. Badge tokens stay the `A14` set (`ok`, `partial`, `insufficient_evidence`, `refused`, `not_applicable`). A rendered advise card shows the badge from `display_status` applied to the underlying answer, and shows the `AdviseUncertainty` value as its own field. `stale` does not alias `insufficient_evidence`. `sample_below_minimum` does not alias `insufficient_sample`. `advisory_budget_exceeded` (`A11`) is an advisor result code in that same separate family. It is not a badge token.

### `A11` — Pre-flight budget

The design target is 15 ms, warm process, p95, on a sealed fixture whose row counts are frozen in the test. Cold start, migration, and extension load are outside the budget. The advisor reads maintained watermarks and pre-aggregated answers. It does not scan unbounded history on the request path. A query that would exceed the budget returns `advisory_budget_exceeded` and no partial number dressed up as advice.

### `A12` — Advisory is not governance

`dendro advise`, the tripwire dispatcher, and I22–I25 must not change grants, model routing, session lifetime, queue assignment, or storage. A tripwire is operator-visible, pull-only advisory metadata. A dashboard shows it when the operator opens it. A harness alert is that same metadata, fetched by the operator or by an explicit typed pull. It is not inserted into a live agent's prompt, tool result, system message, or context window. Automatic prompt injection is forbidden. An operator policy may separately consume the outbox and stop a session. That policy is off unless the operator turns it on, and it is not this requirement. This preserves Goal 9 anti-goal 4: the intelligence layer does not become live agent control.

MCP tools under `dendro advise` have no SQL parameter, no filesystem parameter, and no “execute this” parameter.

## 6. `R32` — Presentation parity

v0.8.0 delivered `domain_v1`, `insight_v1`, and four Nao views while `site/dashboards/` and parts of the TUI still lead with OpenRig nouns (`rig`, `seat`, four-eyes). That split is the reason this gate exists. Goal 8 stays delivered. vNext cannot be accepted while the home surfaces still present OpenRig as the only organization of the product.

### `A13` — Four surfaces, one answer

No new domain fact, no `insight_v1` analyzer `I01`–`I25`, and no Nao projection `v_*` is marked done unless the same PR wires it into all four places. The serialized Answer is the one `DESIGN.md` section 6 already requires. Presenters display it. They do not recompute it.

1. **Obsidian dashboards.** `site/dashboards/index.html` and `site/dashboards/views/*.html` are grouped into four layers: **Model**, **Harness**, **Coordination & Lineage**, **Environment & Security**. **Platform Adapter (OpenRig)** is an explicit badge on views that still use rig, seat, or four-eyes bindings. It is not a fifth domain and not the home grouping.

   | Existing view | vNext home |
   |---|---|
   | `message_mix` | Model and Harness. Body-free copy stays |
   | `cost_and_loops` | Model. Seat columns carry the OpenRig adapter badge |
   | `subagent_lineage` | Coordination & Lineage |
   | `security_posture` | Environment & Security |
   | `process_resources` | Environment & Security |
   | `handoff_flow`, `seat_dynamics`, `executive_governance` | Platform Adapter (OpenRig), linked from home and not presented as the domain model |

   New projections reserved for the new analyzers, created when that analyzer is done: `v_trajectory_vortex`, `v_schema_drift`, `v_authority_blast_radius`, `v_counterfactual_routing`. CPU stays on `I14`. Cost stays on `I04`. The half-open window work stays the existing Nao-windows row and applies to these views too.

2. **Offline viewer.** `site/viewer.html` allowlists are `Set` literals synchronized with `crates/dendro-core/src/presentation.rs`. Insight IDs, measure keys, relation kinds, and security tokens grow only by editing both sets in one change. A regex or slug fallback is a test failure. `site/site.test.cjs` must include a realistic token canary (a secret-shaped string and an unknown insight ID) that the viewer strips.

3. **TUI cockpit.** `crates/dendro-cli/src/tui/` navigates domain first. OpenRig screens remain reachable behind the adapter badge. Progressive disclosure is the `DESIGN.md:161` levels, filled with the Answer fields at `DESIGN.md:127`. vNext does not move a field to a different level.

   | Level | Fields (`DESIGN.md:161`) |
   |---|---|
   | L0 | status, finding, practical meaning, scope, and missingness |
   | L1 | assumptions and alternatives |
   | L2 | safe evidence IDs and source versions |
   | L3 | formula, population (`what`, `included`, `excluded`), exclusions, and cutoff |

   Assumptions and alternative explanations at L1 are required whenever the answer could be over-read. On `I22` they block reading a vortex trip as a cause. On `I25` they block reading the simulation as an effect or as a routing change. Population (`what`, `included`, `excluded`) stays at L3 with the formula, the exclusions, and the cutoff. The canonical digest is unchanged: typed values and status, snapshot and scope, population, versions, coverage, and evidence. Narrative rendering and hypotheses stay outside that digest. Parse-back compares the canonical projection.

   Keyboard focus order, visible focus, labeled controls, and back/escape stay mandatory. Status is not conveyed by color alone. Journeys cover populated, empty, loading, failed-source, refused, and partial.

Insight ID parity is machine-checked. A surface is not done because a reviewer remembers the list. `crates/dendro-core/tests/semantic.rs` and `site/site.test.cjs` each assert exact set equality of insight IDs across all five of these sources:

- the `insight_v1` registry
- admitted IDs in `crates/dendro-core/src/presentation.rs`
- `ALLOWED_INSIGHT_IDS` in `site/viewer.html`
- the Compiled Insights Strip in `site/dashboards/index.html`. That strip is the catalog index used for insight-ID parity (`I01`–`I25`). It is not the home-page headline list. `DESIGN.md:161` caps the home page at at most seven selected headline persona questions. Keeping the strip as the full catalog index preserves that cap and the parity check together.
- the TUI insight index and domain navigation in `crates/dendro-cli/src/tui/` (`insight_domains` and the insight rows)

Those two tests also assert the Nao parity named in `A15`. Schema-table parity is `crates/dendro-core/tests/semantic.rs`. CLI doc parity is `crates/dendro-cli/tests/cli_doc_parity.rs`. A mismatch of one ID fails the build.

4. **Design system.** See `A14`.

### `A14` — Obsidian visual contract

Dashboards, viewer, and TUI share these tokens. Current CSS values `#090d16` and `#0f1927` are the baseline to move, not the target.

| Token | Value |
|---|---|
| Page surface | `#0A0D12` |
| Raised surface | `#11161F` |
| Metrics | Tabular monospace |
| Display status | `ok`, `partial`, `insufficient_evidence`, `refused`, `not_applicable` |

`display_status(status, reason)` is a total function from `InsightStatus` plus `reason` to one primary badge token. It deterministically maps both representations in this paragraph.

`DESIGN.md:127` defines the logical Answer statuses `computed`, `partial`, `insufficient_sample`, `not_applicable`, `not_observable`, `noncomparable`, `threshold_exceeded`, and `refused`. The sentence at that line spells a supported answer as `supported`, which this contract ships as `computed`, and it spells `partial`, `refused`, `not_observable`, `noncomparable`, and `threshold_exceeded` directly. `insufficient_sample` and `not_applicable` are logical outcomes in that same answer vocabulary. They are not extra badge tokens. `NOT_ASSESSED` stays a policy-evaluation state within the answer, never a zero violation count.

The Rust `insight_v1::contract` (`crates/dendro-core/src/insight_v1/contract.rs`, with the contributor rules in `docs/contributing/insights-v1.md`) does not store `not_observable`, `insufficient_sample`, `noncomparable`, or `threshold_exceeded` as `InsightStatus` variants. It encodes those four via `InsightStatus` + `reason`. `InsightStatus` is `Computed`, `Partial { reason }`, or `Refused { reason }`. `reason` is a closed code or absent. `threshold_exceeded` is one of those reason codes. It is not a sixth badge token. The reason, when present, is also the badge subtitle. The subtitle does not change which row of the table matched.

First match wins:

| Order | Condition | Primary token |
|---|---|---|
| 1 | `reason` is `not_observable` or `insufficient_sample` | `insufficient_evidence` |
| 2 | `reason` is `not_applicable` or `NOT_ASSESSED` | `not_applicable` |
| 3 | `reason` is `threshold_exceeded` | `partial` |
| 4 | `reason` is `noncomparable` | `insufficient_evidence` |
| 5 | `status` is `Computed` | `ok` |
| 6 | `status` is `Partial` | `partial` |
| 7 | `status` is `Refused` | `refused` |

Row 1 is why `Refused { reason: not_observable }` and `Refused { reason: insufficient_sample }` display as `insufficient_evidence`, with that reason kept as the subtitle. A short sample uses the reason code `insufficient_sample`. Any other reason falls through to the status row, so every pair has one token. Storage and JSON keep `InsightStatus`. They do not store the badge token in place of the answer.

`AdviseUncertainty` (`A09`) is a different enum. Its labels are not inputs to this table and are not badge tokens.

A page bound to fixture data badges `ILLUSTRATIVE SNAPSHOT`. A page bound to the local daemon badges `LIVE DUCKDB`. One page must not show both. The current combined string `ILLUSTRATIVE SNAPSHOT · LIVE SYNC READY` is retired when this gate lands.

No external CDN, font host, script host, or remote image. `site/site.test.cjs` already rejects remote dependencies and must keep doing so. Inline script is static site code, not string-built from source content (`DESIGN.md` section 7).

## 7. `R33` — Documentation and catalog parity

### `A15` — Same change, same docs, machine-checked

A PR that touches schema, insights, Nao views, CLI, TUI, or web surfaces updates, in that same change:

| Artifact | Parity rule |
|---|---|
| `docs/NAO_SEMANTIC.md` | Every `v_*` the binary can serve, including window behavior |
| `templates/nao/semantic_models.yaml`, `templates/nao/nao_config.yaml`, `templates/nao/prompts.md`, `templates/nao/dashboards/*.yaml` | Same view names and question phrases as the Rust matcher |
| `crates/dendro-core` semantic SQL and `semantic.rs` | 1:1 with the YAML and with `site/dashboards/views/` |
| `docs/ARCHITECTURE.md`, `docs/DATA_DICTIONARY.md`, `docs/CLI_R1.md`, `docs/BACKLOG.md` | Schema 5 objects, new commands, and insight IDs once they exist |
| `site/site.test.cjs`, `crates/dendro-core/tests/semantic.rs`, and `crates/dendro-cli/tests/cli_doc_parity.rs` | The checks below. `cli_doc_parity.rs` is the reserved CLI doc test |

The parity tests fail the build when any of these drift. `crates/dendro-core/tests/semantic.rs` and `site/site.test.cjs` both run the insight and Nao checks. Schema-table coverage is `crates/dendro-core/tests/semantic.rs`. The CLI doc check has its own file because `clap` lives in `dendro-cli`. No check is left to a reviewer's memory.

Version stamps in workspace `Cargo.toml`, `site/index.html`, `site/dashboards/index.html`, and the TUI header still have to match. Closed vocabularies still have to match: relation kinds, security tokens, `AdviseUncertainty`, and the five display-status tokens.

**Insight ID parity.** Exact set equality across (a) the `insight_v1` registry, (b) admitted IDs in `crates/dendro-core/src/presentation.rs`, (c) `ALLOWED_INSIGHT_IDS` in `site/viewer.html`, (d) the Compiled Insights Strip in `site/dashboards/index.html`, and (e) the TUI insight index and domain navigation in `crates/dendro-cli/src/tui/`. Responsible tests: `crates/dendro-core/tests/semantic.rs` and `site/site.test.cjs`.

**Nao parity.** Exact set equality of view names, and of the question and intent catalog, across `crates/dendro-core/src/semantic_views.sql`, `crates/dendro-core/src/semantic.rs` (`VIEWS`, `supported_intents()`, and the `semantic_views` catalog), `templates/nao/semantic_models.yaml`, `templates/nao/nao_config.yaml`, `templates/nao/prompts.md`, `templates/nao/dashboards/*.yaml`, `site/dashboards/views/*.html`, and `docs/NAO_SEMANTIC.md`. Responsible tests: `crates/dendro-core/tests/semantic.rs` and `site/site.test.cjs`.

**Schema parity.** Every canonical table in schema 5 appears in `docs/DATA_DICTIONARY.md` and in `docs/ARCHITECTURE.md`. Responsible test: `crates/dendro-core/tests/semantic.rs`.

**CLI doc parity.** Every `clap` subcommand declared in `crates/dendro-cli` appears in `docs/CLI_R1.md`. Responsible test: `crates/dendro-cli/tests/cli_doc_parity.rs`. That file is a reservation for delivery. It does not exist today.

A documentation-only PR that does not touch those surfaces still keeps `node --test site/site.test.cjs` green. This authoring change is in that class.

## 8. How each acceptance ID is proved

Gates for implementation PRs: `cargo fmt --check`, `cargo check`, `cargo test`, and `cargo clippy -D warnings` on native Windows and native Linux. `node --test site/site.test.cjs` for every presentation or catalog change. One independent review per PR. Findings live in the PR.

| ID | Reserved proof |
|---|---|
| `A01` | `cargo test -p dendro-core --test engine_v200`: lockfile pin, runtime `v2.0.0`, shadow match of the six shipped insights, version-68 file still opens with `1.10506.0` |
| `A02` | `cargo test -p dendro-core --test migration_v5`: `O_CREAT \| O_EXCL` / `CREATE_NEW` refuses a re-run that would overwrite `dendro.duckdb.pre-v200.bak`; backup set contains main, WAL or recorded absence, privacy-key sidecar, and provenance sidecars; member digests; pointer manifest selects the immutable directory; version 69 refused by `1.10506.0`; rollback leaves the backup bytes unchanged; `0700`/`0600` under CI umask; privacy settings still locked across WAL replay |
| `A03` | `cargo test -p dendro-cli --test quack_owner`: the daemon is the only raw DuckDB or `quack` session; CLI, TUI, HTTP, Nao, and a same-user hook process receive no SQL session; hook calls are limited to the typed append/advise capability and a SQL string is rejected; token absent from logs and from client credentials; Unix socket `0700`; Windows private named pipe; the test fails if any loopback TCP listener is bound. File mode `0700` alone must not be the assertion that a same-user child is contained |
| `A04` | `cargo test -p dendro-core --test envelope_variant`: shred-readable shape, body canaries refused, a secret-shaped canary in a **key** position absent from the stored `VARIANT`, unknown keys dropped or HMAC-replaced, over-depth row refused, typed facts kept |
| `A05` | `cargo test -p dendro-core --test tripwire_outbox`: in-transaction notification is O(1); trigger body rejected when it scans history, computes a centroid or quantile, or joins; post-commit dispatcher runs `I22` outside the trigger; rollback does not enqueue; rediscovery is idempotent; no alert text is written into an agent prompt |
| `A06` | `cargo test -p dendro-core --test lineage_graph`: evidenced-only reachability; `absent`, `truncated`, and `conflict` edges skipped; direction fixtures for parent→child `spawn` / `delegate` / `handoff` and child→parent `return` / `join`; cycles; hop cap; extension-off fallback matches; PageRank absent from `I24`; answer carries the dependency-exposure non-claim |
| `A07` | `cargo test -p dendro-core --test retrieval_extensions`: FTS corpus canary rejects prompt text; exact distance beats HNSW; on-disk HNSW unused; DuckLake export has no raw identifiers and is refused from `dendro advise` |
| `A08` | `cargo test -p dendro-core insight_v1`: at `A08` merge the registry is exactly I01–I21; each of the fifteen computes or refuses on captured fixtures; `I15` is `not_observable` without a specimen |
| `A09` | `cargo test -p dendro-cli --test advise`: four questions, placebo versus insufficient verification, MCP schema has no SQL parameter, uncertainty values are only `AdviseUncertainty` and do not overlap badge tokens |
| `A10` | Fixture per I22–I25 covering the refusal column of the table in section 5. The registry exact-set gate is now I01–I25. `I22` fixtures run the Rust analyzer, not a SQL trigger body. `I25` refuses when the cohort or the tier profile is unpinned |
| `A11` | Warm p95 ≤ 15 ms on the frozen fixture; over-budget path returns `advisory_budget_exceeded` |
| `A12` | Advise and dispatcher tests assert no writes to policy, routing, grants, or session lifetime, and assert that a tripwire is not inserted into agent context |
| `A13` | Dashboard layer headings, adapter badges, five-way insight ID equality (registry, `presentation.rs`, `ALLOWED_INSIGHT_IDS`, Compiled Insights Strip, TUI), TUI L0–L3 matching `DESIGN.md:161` (L0: status, finding, practical meaning, scope, and missingness; L1: assumptions and alternatives; L2: safe evidence IDs and source versions; L3: formula, population (`what`, `included`, `excluded`), exclusions, and cutoff), shared Answer digest |
| `A14` | `#0A0D12` / `#11161F`, exclusive snapshot versus live badge, no remote URL in `site/`, `display_status` row fixtures including `Refused` + `not_observable`, `Refused` + `insufficient_sample`, and `threshold_exceeded` |
| `A15` | `crates/dendro-core/tests/semantic.rs`, `site/site.test.cjs`, and `crates/dendro-cli/tests/cli_doc_parity.rs`, covering insight IDs, Nao catalogs, schema-5 tables in `docs/DATA_DICTIONARY.md` and `docs/ARCHITECTURE.md`, and every `clap` subcommand in `docs/CLI_R1.md` |

`A11` is a design target until that test is measured. A missed budget blocks `A09` acceptance. It does not block registering an insight under `A08`.

## 9. Deltas a later editor must not smear back into v0.8.0

| Topic | v0.8.0 contract | vNext contract |
|---|---|---|
| Engine | Locked `1.10506.0` via requirement `1.1` | `~1.20000.0`, runtime `v2.0.0` |
| On-disk DuckDB | Storage 68 | Storage 69 after cutover; 68 only for shadow and backup |
| Product schema | Target 4 | Target 5. Backup `dendro.duckdb.pre-v200.bak` is an exclusive immutable directory set (main, WAL, privacy key, provenance), selected by the `DESIGN.md:188` pointer manifest |
| Insights | Six shipped; fifteen backlog | `A08` registers exactly I01–I21. `A10` expands that set to exactly I01–I25. I01–I21 formulas stay unchanged |
| Writers | Embedded open | `dendro serve` is the only raw DuckDB or `quack` session. Clients use typed RPC on a `0700` Unix socket or a private Windows named pipe. No unauthenticated loopback TCP |
| Graph | `session_lineage` only | `duckpgq` plus the same Rust module. Reachability is evidenced edges only, with the `A06` directions, and is not security authority |
| Disclosure labels | `DESIGN.md` L0–L3 | The `DESIGN.md:161` levels: L0 status, finding, practical meaning, scope, and missingness; L1 assumptions and alternatives; L2 safe evidence IDs and source versions; L3 formula, population (`what`, `included`, `excluded`), exclusions, and cutoff |
| Dashboard chrome | OpenRig-first home, surfaces `#090d16` / `#0f1927` | Four layers, adapter badge, `#0A0D12` / `#11161F` |
| Scope page | `SCOPE-TRIMMED.md` decides what v0.8.0 built | This page decides what vNext builds |

`SCOPE-TRIMMED.md` cut the insight set to about six **for v0.8.0**. Goal 9 already put the other fifteen back on the backlog. This addendum is that backlog’s contract for those fifteen, plus I22–I25, plus the engine and the two parity gates. It does not mean v0.8.0 shipped unfinished.

## 10. Non-goals and holds that still bind

- No runtime `INSTALL` of extensions, no change to `autoinstall_known_extensions=false` or `autoload_known_extensions=false`, no unlocking configuration for client sessions.
- No raw quack session and no raw SQL exposed to the CLI, TUI, HTTP server, Nao, or live agent hooks. Same-user containment is the typed RPC capability, not the socket mode alone.
- No unauthenticated loopback TCP for the daemon transport.
- No historical scan, cohort centroid, quantile, or multi-row join inside a trigger body. `I22` runs in the Rust analyzer after commit or on demand.
- No automatic insertion of a tripwire into a live agent's prompt or context.
- No `VARIANT` key outside the harness allowlist. Unknown keys are dropped or HMAC-replaced. A raw key name is never stored.
- No `I25` simulation from a hardcoded model name, an unpinned cohort, or an unpinned tier profile.
- No blast-radius traversal of `absent`, `truncated`, or `conflict` edges, and no reading of structural reachability as inherited or effective authority.
- No prompt, transcript, argv, path, or secret in `VARIANT`, FTS, vectors, tripwire payloads, Parquet partitions, or advise responses.
- No persistent on-disk HNSW as canonical state.
- No causal claim from PageRank, from cosine distance, or from the `I25` simulation.
- No p-values, rankings, or quality scores.
- No synthetic strace or ETW. `I15` waits on real specimens.
- No weakening of the Windows private DACL to chase `#91`.
- No use of this migration to smuggle schema changes into the three-plane platform split. Assumption `A09-05` still says that split does not reshape tables. Schema 5 is `R29` / `R30` only.
- No cloud APM, remote streaming exporter, or non-local database. DuckLake partitions are local files.
- No automatic session kill from `I22`. Stopping an agent requires a separate operator policy, default off.
- No edit to the v0.8.0 `tasks.json` counts from this document.
- AGY remains the cut `T09` row until a specimen exists.
- Answer-path prohibition on `ATTACH`, `COPY`, file/URL readers, and dynamic SQL stays, except the single DuckLake publisher in `A07`.

## 11. What this authoring change itself proves

This file, the Goal 9 registration in `docs/BACKLOG.md`, and the link in `README.md` are the specification. They mark no acceptance ID done. The site suite is run because `A15` requires document changes beside `site/` to leave `node --test site/site.test.cjs` passing. That run does not validate DuckDB 2.0.
