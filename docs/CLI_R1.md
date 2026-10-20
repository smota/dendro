# Dendro CLI

Dendro is an AI Fleet Intelligence tool. Reports are Operational Intelligence. Fleet Governance decisions stay with the operator.

The sealed acceptance contract is Amendment 1. This page only describes the commands.

```text
dendro run --db dendro.duckdb
dendro report <links|activity|handoffs|tokens|decisions> --db dendro.duckdb --rig <rig> --from <utc> --to <utc>
dendro trace --db dendro.duckdb --metric-id <id>
```

`report` and `trace` print JSON. A missing M1 denominator is JSON null, which means no data, not zero. M4 impact stays insufficient data. Nao and watch are outside this R1 proof.

A first-ingest timing result does not by itself prove AC1. AC1 also requires the first report.

## Private staging and snapshots

`dendro run` holds a lock file, `.dendro-output.lock`, next to the analytics database for the whole run. A second run in the same directory is refused while the lock is held. The run works in one private stage directory in that same directory, `.dendro-stage-<16 hex>`. The stage has mode 0700 and a fixed marker file, `.dendro-stage`. Every run, with or without `--frozen`, reads OpenRig from a private copy and never opens a SQLite connection on the original database (AC12). The flag is kept for compatibility and changes nothing. Dendro copies the `-wal` and then the main file into the stage's `snapshot/` directory with read-only handles. It never opens the original `-shm`. The copy is accepted only if the original's identity, size, mtime and WAL header did not change during the copy and the copy passes `PRAGMA quick_check`. Otherwise Dendro retries. After 5 changed attempts the run fails and ingests nothing. The run output reports the accepted source revision, the number of attempts and the measured copy and check times, and reports show the original path and revision under `openrig_sources`. Dendro creates those copies, the marker and the staged copy of an existing analytics database with mode 0600. Files DuckDB creates in the stage follow the process umask; the 0700 stage keeps other users out. Raw snapshots never go to `TMPDIR`.

- A run removes its stage when it ends normally or with an error. If removal fails, the run exits nonzero and names the stage.
- On SIGINT or SIGTERM the run stops at the next line, row or copy chunk, closes its databases, removes its stage and exits with 130 or 143. If it cannot stop within 30 seconds, or a second signal arrives, it exits without deleting files that may still be written and says that cleanup failed.
- After SIGKILL or a host crash, the next run in that directory removes the old stage while it holds the lock. It removes only real directories owned by the user with a valid marker and expected contents. It keeps and lists everything else, including look-alike names, symlinks and directories without a marker. The `staging` field of the run's JSON output lists what was removed or kept.

## Missing key

An existing analytics database without its `.key` sidecar refuses ingest and stays unchanged. `report` and `trace` still read it. They create no key, compute no new fingerprint and print no key material. A first run with no database creates the key, as before (K8).

## Pi and Codex session ids

A Pi or Codex session file is one native session, named by the id on its first line: the Pi `session` header id or the Codex `session_meta` id (K10). Every usage line in the file counts under that id, so two sessions that reuse a local message id never merge. A later header, or a Codex line `session_id`, that names a different session does not move tokens; the run records the warning "conflicting session header". A file without a valid first-line header counts under its own id `pi-file:<source>` or `codex-file:<source>`, stays unlinked, and records "missing session header". A later id never rescues it.

Analytics databases written before this change counted every Pi or Codex file under one shared session. Dendro refuses them as an older schema version. Ingest again into a new analytics database; source files are never changed.

## Source summary (AC1)

The `sources` field of `dendro run` lists each configured root. Default configured roots for Pi include both OpenRig-managed sessions (`$OPENRIG_HOME/state/pi`) and standalone user sessions (`$HOME/.pi/agent/sessions`). A Claude, Codex or Pi root reads `<harness> files=<n> records_read=<r> usage_records=<u>`. A record is one complete, non-blank JSONL line read by this run. Lines below the stored watermark were read by an earlier run and are not counted again, and a trailing line without its newline is not a record yet. `usage_records` counts the usage facts stored by this run after duplicate messages are skipped, so it can be lower than `records_read`. The OpenRig root reads `openrig events=<n> ...`, where `events` counts the rows of the `events` table read from the private copy. The queue tables are read too, for chain and destination evidence, and are not counted. A root that does not exist is listed as `<harness> missing`, records the warning `source <harness> missing`, and the other roots still run.

## Token totals per session and seat (AC6)

`report tokens` and `trace --metric-id tokens` carry the same token fields. Every figure uses the half-open window `[from, to)`.

- `token_categories`: the four categories (`input_uncached`, `output`, `cache_read`, `cache_write`) and their sum `tokens`, for all usage in scope.
- `seat_token_categories`: the same per linked seat. A seat's `tokens` equals its `m1_by_seat` tokens and its `mission_by_seat` total.
- `unlinked_tokens`: usage from sessions with no seat. `unlinked_token_categories` splits it into `unlinked` (no identity event) and `collision` (identity events name different seats).
- `sessions`: one row per discovered session, with `harness`, the sealed `session` id, `status` (`linked`, `unlinked` or `collision`), `rig` and `seat` when linked, the four categories, `tokens`, `period_records`, `discovered_by` (`usage`, `zero-usage file`, `identity`) and `untimestamped`.
- `untimestamped`: the session's records with no time. They are counted per session only and never enter a period figure. `untimestamped_count` and `untimestamped_tokens_all_time` give the in-scope totals.

A session is linked only when every identity event for its exact harness and native id names the same rig and seat. Links, token totals, missions, intake and `--rig` all use this one rule. With `--rig`, the report keeps only sessions linked to that rig, with their untimestamped records, so a collision never enters through one of its candidate seats.

Reasoning is part of output. It is shown as `reasoning` and never added to the four categories again. Claude does not record it, so Claude sessions show `not recorded`.

## Tool-loop source evidence (#33, local candidate)

Every metric trace adds `tool_loops`, `outcome_coverage`,
`outcome_coverage_exclusions` and `tool_loop_trace` under `inputs`. The report
payload and existing trace fields retain their definitions. `tool_loop_trace`
uses formula version `k11-tool-loop-trace-v1`: observations carry actual stored
source keys, source ids, generations, keyed input fingerprints and captured
input/detector ranks. Each episode lists its entire maximal failure run and the
first consecutive qualifying triple. Its `span_seconds` is an exact signed
decimal string. Native Claude/Pi spans may be negative; Codex spans must also be
nonnegative. The fixed rule remains three failures within 600 seconds.

Report and trace inputs come from one read transaction. A trace replays its
captured evaluation; separate queries with fully tied native rows can enumerate
those ties differently. This does not establish a new persistent tie order.
The half-open time filter runs before detector ordering and retains untimed
observations, including breakers and middle members.

File-source tool entries distinguish each retained row generation from the latest
committed prefix's own generation. Latest file-prefix metadata does not establish a
row's producing revision: that revision remains `null` / `not_recorded`.
Current source state is `not_checked`. Trace opens neither source files nor
keys and performs no backfill. Missing latest metadata alone does not prevent
replaying surviving stored evidence. Missing essential evidence produces
`insufficient_evidence` and explicit limitations; unsupported episodes are
omitted with their original report indices while report counts remain visible.
Coverage counts sources once using their persisted file-session linkage;
Codex tool-call coverage remains `not read`.

## Sealed replay (#13)

Record the exact complete JSONL prefixes and OpenRig table cutoffs consumed by a run:

```bash
dendro run --db fleet.duckdb --root claude=/data/claude --root openrig=/data/openrig.sqlite --write-manifest cutoff.json
dendro run --db fleet.duckdb --root claude=/data/claude --root openrig=/data/openrig.sqlite --sealed cutoff.json
```

`--manifest-out` is an alias for `--write-manifest`. Capture and replay flags are
mutually exclusive. Manifest paths follow the secret-path policy; capture refuses
analytics files, key/lock sidecars, source files, and paths inside source roots.
The JSON contains content-free source identifiers, byte lengths, high-water keys,
row counts, and SHA-256 digests. It contains no transcript or raw source path.

Replay reads only named files, through their complete-line byte cutoffs, and only
OpenRig events, transitions, and snapshots through the recorded keys. New files,
rows, queue items, and partial tails are excluded. A missing, truncated, or changed
named prefix produces `mismatch`, identifying the source and reason, before the
analytics database is published. Unsealed runs continue ingesting new data.

When capture includes history from earlier ingests, it also preserves the accepted
analytics snapshot as `.dendro-sealed-<digest>.duckdb` beside the manifest. This
companion contains the same sanitized derived evidence as the analytics database;
its SHA-256 digest is recorded in the JSON. Keep it with the manifest.
Missing or changed companions refuse replay. Fresh captures without prior history
need no analytics companion. Replay into another database should retain the original
analytics key sidecar: sealed secret-shaped item/session identifiers use that key,
and historical companions require its key id. Source validation still runs before
any historical analytics snapshot is restored.

Mutable queue-item fields are reconstructed from creation events where supported.
Otherwise report and trace mark affected metrics `not reproducible: mutable source`
with the item and reason. OpenRig facts carry their producing `generation` (starting
at 1) and `producing_revision`. Appended rows get a new producing generation;
unchanged rows retain theirs. Reset/replacement retains superseded history under
new fact keys. Retention-deleted rows retain their totals and trace as `gone` /
`no longer available`. In-place changes issue an `openrig source changed` warning
and replace the affected evidence in current recomputation. Trace also marks a
missing OpenRig file without changing accepted totals.

Source access remains read-only: SQLite is read from a validated private main/WAL
copy, and the original `-shm` is untouched. Live producers may continue changing
source hashes, sizes, and modification times throughout a run.

## v0.6.0 Operator Usability & Smart Window Extensions

In `v0.6.0`, Dendro adds zero-ceremony operator entry points and rig-scoped smart window resolution while preserving all R1/v0.5.0 contracts and flags:

```bash
dendro                                              # defaults to `dendro status`
dendro status [--db <path>] [--rig <rig>] [--format text|json]
dendro dashboard [--db <path>] [--rig <rig>] [--window 1h|24h|7d|30d|all] [--once] [--html [path]] [--auto-ingest]
dendro ui [--db <path>] [--rig <rig>] [--window 1h|24h|7d|30d|all] [--once] [--auto-ingest]
dendro ask [--db <path>] ["<question>"] [--window 1h|24h|7d|30d|all] [--list-intents] [--no-refresh]
dendro simulate topology [--db <path>] [--rig <rig>] [--window 1h|24h|7d|30d|all] [options]
```

- **Status preflight (`dendro` / `dendro status`):** Opens the database read-only and reports `status` (`ready`, `empty`, or `not_initialized`), rig-scoped `min_timestamp` and `max_timestamp`, redacted `rigs` and `seats`, `usage_rows`, `activity_rows`, `queue_rows`, `semantic_fresh`, discovered local harness roots, and copy-pasteable next commands.
- **Rig-scoped smart `--window` (`1h|24h|7d|30d|all`):** Supported across `dashboard`, `ui`, `report`, `trace`, `materialize`, `export`, `ask`, and `simulate topology`. (`report` and `trace` preserve their v0.5.0 `--from`/`--to` defaults when `--window` is omitted.) When `--window` is resolved against a populated database, `resolve_window` scopes timestamp bounds to `--rig` (when supplied), floors `min_timestamp` to whole seconds, sets the half-open `[from, to)` upper bound to `max_timestamp + 1s` so events at `max_timestamp` (including sub-second timestamps) are included, and discloses `"window_resolution"` (`"recorded_bounds"`, `"anchored_to_recorded_data"`, `"explicit"`, or `"trailing_utc"`).
- **Conditional `--auto-ingest`:** When `--auto-ingest` is passed to `dendro dashboard` or `dendro ui` and the target database does not exist or is not `ready`, Dendro runs a read-only local ingest over default harness roots before opening the dashboard.
- **In-app TUI help (`?` / `h`):** Pressing `?` or `h` inside `dendro dashboard` or `dendro ui` opens the interactive keyboard navigation guide.

