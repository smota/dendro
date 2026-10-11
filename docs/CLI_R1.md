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

The `sources` field of `dendro run` lists each configured root. A Claude, Codex or Pi root reads `<harness> files=<n> records_read=<r> usage_records=<u>`. A record is one complete, non-blank JSONL line read by this run. Lines below the stored watermark were read by an earlier run and are not counted again, and a trailing line without its newline is not a record yet. `usage_records` counts the usage facts stored by this run after duplicate messages are skipped, so it can be lower than `records_read`. The OpenRig root reads `openrig events=<n> ...`, where `events` counts the rows of the `events` table read from the private copy. The queue tables are read too, for chain and destination evidence, and are not counted. A root that does not exist is listed as `<harness> missing`, records the warning `source <harness> missing`, and the other roots still run.

## Token totals per session and seat (AC6)

`report tokens` and `trace --metric-id tokens` carry the same token fields. Every figure uses the half-open window `[from, to)`.

- `token_categories`: the four categories (`input_uncached`, `output`, `cache_read`, `cache_write`) and their sum `tokens`, for all usage in scope.
- `seat_token_categories`: the same per linked seat. A seat's `tokens` equals its `m1_by_seat` tokens and its `mission_by_seat` total.
- `unlinked_tokens`: usage from sessions with no seat. `unlinked_token_categories` splits it into `unlinked` (no identity event) and `collision` (identity events name different seats).
- `sessions`: one row per discovered session, with `harness`, the sealed `session` id, `status` (`linked`, `unlinked` or `collision`), `rig` and `seat` when linked, the four categories, `tokens`, `period_records`, `discovered_by` (`usage`, `zero-usage file`, `identity`) and `untimestamped`.
- `untimestamped`: the session's records with no time. They are counted per session only and never enter a period figure. `untimestamped_count` and `untimestamped_tokens_all_time` give the in-scope totals.

A session is linked only when every identity event for its exact harness and native id names the same rig and seat. Links, token totals, missions, intake and `--rig` all use this one rule. With `--rig`, the report keeps only sessions linked to that rig, with their untimestamped records, so a collision never enters through one of its candidate seats.

Reasoning is part of output. It is shown as `reasoning` and never added to the four categories again. Claude does not record it, so Claude sessions show `not recorded`.
