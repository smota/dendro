# Operator guide

Dendro is a local, one-shot evidence engine. It does not need a running rig, webserver or background monitoring service to report on retained execution facts. Start with `dendro --help`; command examples below use `./target/release/dendro` after `cargo build --release --locked`.

## Choose sources and a period

Use dedicated authorized roots and put the analytics database outside them:

```sh
./target/release/dendro run --db ./fleet.duckdb \
  --root openrig=/absolute/path/to/openrig \
  --root claude=/absolute/path/to/claude \
  --root codex=/absolute/path/to/codex
./target/release/dendro report decisions --db ./fleet.duckdb \
  --from 2026-10-14T00:00:00Z --to 2026-10-15T00:00:00Z
```

Select dates present in your data. Timestamps must include their timezone; the interval includes its start and excludes its end. The default report dates are historical examples, so supply your own. A repeated append-only ingest advances watermarks instead of counting accepted bytes again. One OpenRig root belongs to each analytics database; do not combine independent roots by bypassing that guard.

Supported baseline roots include OpenRig, Claude Code, Codex, Pi and AGY. Check `run --help` and the [extension guide](../extensions/README.md) for additional export adapters and their version limits. The absence of counters or timestamps remains unknown rather than being replaced with guessed consumption or dates.

## Answer and inspect

```sh
./target/release/dendro report activity --db ./fleet.duckdb --from 2026-10-14T00:00:00Z --to 2026-10-15T00:00:00Z
./target/release/dendro report handoffs --db ./fleet.duckdb --friction --from 2026-10-14T00:00:00Z --to 2026-10-15T00:00:00Z
./target/release/dendro trace --db ./fleet.duckdb --metric-id handoff_friction --from 2026-10-14T00:00:00Z --to 2026-10-15T00:00:00Z
```

`links`, `activity`, `handoffs`, `tokens`, and `decisions` use the same canonical report engine. Keep `--rig`, `--from`, `--to` and thresholds identical when comparing a report with its trace. Inspect coverage before interpreting a rate. M1 is tokens per nonreturned delivery, not monetary ROI. M4 needs a valid comparison population and otherwise refuses an impact claim.

Save a report to a local JSON file and open [the standalone viewer](../../site/viewer.html) to explore supported fields offline. Its activity bands and concurrency histogram are aggregates; they do not invent missing temporal intervals. Uploaded files stay inside that browser page.

## Reproduce ingestion

```sh
./target/release/dendro run --db ./first.duckdb --root codex=/absolute/path/to/codex --write-manifest ./cutoffs.json
./target/release/dendro run --db ./replay.duckdb --root codex=/absolute/path/to/codex --sealed ./cutoffs.json
```

A sealed replay consumes recorded prefixes and refuses changed accepted bytes. The manifest can expose source locations, so treat it as private operational evidence. It is not a sanitized public export.

## Privacy boundaries

Do not select authentication files or credential directories. Source logs remain read-only; analytics facts use allowlists and secret redaction. No cloud request is needed for the core reports. A third-party conversational client may have its own network and model configuration; local DuckDB does not imply that such a client is offline.

This guide does not promise a `status`, `watch`, Prometheus exporter, or OTel bridge. Use actual `--help` output for available commands. Build time depends on the machine; no measured five-minute build claim is made.
