# Operator guide

Dendro is a local, one-shot evidence engine. It does not need a running rig, webserver or background monitoring service to report on retained execution facts. Start with bare `./target/release/dendro` (which runs `dendro status`) or `./target/release/dendro --help` after `cargo build --release --locked`.

## Zero-ceremony quickstart (`v0.7.0`)

Inspect database readiness, discovered local harness roots, and recorded timestamp bounds, or open the six-screen cockpit in one command. Bare `dendro` prints the same sectioned status brief as `dendro status`: coverage, harness roots, seats grouped by rig, and a command reference.

```sh
# Inspect status, discovered local harness roots, and recorded [min_timestamp, max_timestamp]
./target/release/dendro status --db ./fleet.duckdb

# Open the interactive TUI explorer (auto-ingests local harness roots if fleet.duckdb is missing/empty)
./target/release/dendro dashboard --db ./fleet.duckdb --auto-ingest

# Print a non-interactive terminal snapshot or export a self-contained offline HTML dashboard
./target/release/dendro dashboard --db ./fleet.duckdb --once
./target/release/dendro dashboard --db ./fleet.duckdb --html ./fleet-dashboard.html

# Ask natural questions (automatically refreshes semantic views when missing or stale)
./target/release/dendro ask --format text --db ./fleet.duckdb "highest idle"
./target/release/dendro ask --db ./fleet.duckdb "where do tokens go?"
./target/release/dendro ask --list-intents
```

`dendro ask` tries an exact normalized phrase, then scores tokens against the supported intent vocabulary. A question with an unsupported qualifier (a mission, a relative date, a seat `@`, or a SQL verb) is refused and the closest intents are suggested. `--format text` prints the terminal answer. `--format json` prints the document. A terminal defaults to text; a pipe defaults to json.

Inside the cockpit (`dendro dashboard` or `dendro ui`), the screens are `1` Pulse, `2` Seats & Timeline, `3` Handoffs & Flow, `4` Tokens & Cache, `5` Topology Simulator, and `6` Trace & Spotlights. Press `?` for help. `/` filters seats, `x` reveals queue endpoints and gates, `m` switches ribbon and timeline, `v` cycles the handoff heatmap, `r` cycles named rigs, `w` cycles the window, `:` asks Nao, `i` previews an issue draft, and `q` quits.

## What the status brief calls a seat

An agent seat is `<role>@<named-rig>`, for example `dev-build@dendro-dev`. Status groups those seats under the rig instead of printing every `seat@rig` on one line. `qitem-*` ids are queue endpoints. `external:`, `system:`, `@system`, `system`, `steerer`, and `steer-lead` are gates. Other ids, including ephemeral 26-character `01…` ULIDs, are unlinked. Named rigs are listed; ULID rigs are counted as ephemeral and are not operator rig names. Press `x` in the cockpit when you need the hidden endpoints on screen.

## Pi sessions from two roots

A default `dendro run` reads Pi from OpenRig (`$OPENRIG_HOME/state/pi`, otherwise `~/.openrig/state/pi`) and from standalone `~/.pi/agent/sessions`. Status reports both:

`Pi sources: openrig_files=<n> · standalone_files=<n> · linked_usage=<n> · standalone_usage=<n>`

Standalone usage is Pi usage whose session is not in Pi identity facts. It is recorded, and it is not a linked seat. Pass `--root pi=<path>` twice when the files live somewhere else; two different Pi paths are both ingested.

## Choose sources and a period

Use dedicated authorized roots and put the analytics database outside them:

```sh
./target/release/dendro run --db ./fleet.duckdb \
  --root openrig=/absolute/path/to/openrig \
  --root claude=/absolute/path/to/claude \
  --root codex=/absolute/path/to/codex
./target/release/dendro report decisions --db ./fleet.duckdb --window 7d
./target/release/dendro report decisions --db ./fleet.duckdb \
  --from 2026-10-14T00:00:00Z --to 2026-10-15T00:00:00Z
```

You can supply `--window 1h|24h|7d|30d|all` (which anchors relative windows to the database's rig-scoped `[min_timestamp, max_timestamp + 1s)` bounds when data exists and discloses `"window_resolution"`) or explicit RFC3339 `--from` and `--to` timestamps. Timestamps must include their timezone; the interval includes its start and excludes its end (`[from, to)`). A repeated append-only ingest advances watermarks instead of counting accepted bytes again. One OpenRig root belongs to each analytics database; do not combine independent roots by bypassing that guard.

Supported baseline roots include OpenRig, Claude Code, Codex, Pi, AGY, and Grok. Check `run --help` and the [extension guide](../extensions/README.md) for additional export adapters and their version limits. The absence of counters or timestamps remains unknown rather than being replaced with guessed consumption or dates.

## Answer and inspect

```sh
./target/release/dendro report activity --db ./fleet.duckdb --window 7d
./target/release/dendro report handoffs --db ./fleet.duckdb --friction --window 7d
./target/release/dendro trace --db ./fleet.duckdb --metric-id handoff_friction --window 7d
```

`links`, `activity`, `handoffs`, `tokens`, and `decisions` use the same canonical report engine. Keep `--rig`, `--window` (or `--from`/`--to`) and thresholds identical when comparing a report with its trace. Inspect coverage before interpreting a rate. M1 is tokens per nonreturned delivery, not monetary ROI. M4 needs a valid comparison population and otherwise refuses an impact claim.

Export a self-contained HTML bundle via `dendro dashboard --html ./fleet-dashboard.html`, or save a report to a local JSON file and open [the standalone viewer](../../site/viewer.html) to explore supported fields offline. Its activity bands and concurrency histogram are aggregates; they do not invent missing temporal intervals. Uploaded files stay inside that browser page.

## Reproduce ingestion

```sh
./target/release/dendro run --db ./first.duckdb --root codex=/absolute/path/to/codex --write-manifest ./cutoffs.json
./target/release/dendro run --db ./replay.duckdb --root codex=/absolute/path/to/codex --sealed ./cutoffs.json
```

A sealed replay consumes recorded prefixes and refuses changed accepted bytes. The manifest can expose source locations, so treat it as private operational evidence. It is not a sanitized public export.

## Privacy boundaries

Do not select authentication files or credential directories. Source logs remain read-only; analytics facts use allowlists and secret redaction. No cloud request is needed for the core reports. A third-party conversational client may have its own network and model configuration; local DuckDB does not imply that such a client is offline.

`dendro status` inspects local database readiness and discovered harness roots read-only. This guide does not promise a background `watch` daemon, Prometheus exporter, or OTel bridge. Use actual `--help` output for available commands. Build time depends on the machine; no measured five-minute build claim is made.
