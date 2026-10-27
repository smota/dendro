# Operator & Adoption Guide

Bring your fleet's history into focus. Dendro reads authorized local execution records and lets you investigate time, tokens, handoffs, and capability use from a browser, terminal, or repeatable report.

[Documentation Hub](../README.md) · [Visual Product Tour](../VISUAL_PRODUCT_TOUR.md) · [CLI Reference](../CLI_R1.md)

## Start with your local fleet

Build from source with stable Rust and Git, or use an available binary:

```sh
cargo build --release --locked
./target/release/dendro status
./target/release/dendro run
```

Bare `dendro` runs the same status brief: database readiness, discovered harness roots, seats grouped by rig, and recorded bounds. Default storage lives in Dendro's private application-data folder. Run `dendro storage-init` to create or verify it. Use `--db` only with a private analytics folder outside your source roots.

## Choose your workspace

```sh
# Open the connected browser studio; visit http://127.0.0.1:4242
./target/release/dendro dashboard --serve 127.0.0.1:4242 --site-dir ./site

# Or explore the terminal cockpit, ingesting if needed
./target/release/dendro dashboard --auto-ingest

# Or print a terminal snapshot or export a self-contained HTML dashboard
./target/release/dendro dashboard --once
./target/release/dendro dashboard --html ./fleet-dashboard.html
```

The browser server and cockpit each run in the foreground; use them as alternatives or in separate terminals. The cockpit screens are **Pulse**, **Seats & Timeline**, **Handoffs & Flow**, **Tokens & Cache**, **Topology Simulator**, and **Trace & Spotlights**. Press `1`–`6` to switch, `?` for shortcuts, and `q` to quit.

## Make your first decision

1. **Choose the period.** Use the Time Machine in the studio or `--window 7d` in a report.
2. **Find the concentration.** Read the growth rings, isolate a layer, or group consumption by harness or workspace.
3. **Follow the evidence.** Open the Stack X-Ray, capability view, or handoff timeline around that interval.
4. **Choose a next step.** Inspect a repeated failure, review a handoff, or compare a routing scenario.

The [Visual Product Tour](../VISUAL_PRODUCT_TOUR.md) walks through these surfaces.

## Ask a question

```sh
./target/release/dendro ask --format text "highest idle"
./target/release/dendro ask "where do tokens go?"
./target/release/dendro ask --list-intents
```

Dendro refreshes missing or stale semantic views automatically. Supported intents cover idle time, token burn, rework, baseline comparisons, topology verification, and traces. Use `--list-intents` to discover supported phrasing. Answers show available evidence and suggest nearby intents when a question falls outside that scope. See [Conversational Queries & Nao](../NAO_SEMANTIC.md).

## Choose sources and a period

Default discovery covers OpenRig, Claude Code, Codex, Pi, AGY, and Grok. Pi discovery includes both OpenRig-managed and standalone session roots. For custom locations, supply authorized roots explicitly:

```sh
./target/release/dendro run \
  --root openrig=/absolute/path/to/openrig \
  --root claude=/absolute/path/to/claude \
  --root codex=/absolute/path/to/codex
./target/release/dendro report decisions --window 7d
./target/release/dendro report decisions \
  --from 2026-10-14T00:00:00Z --to 2026-10-15T00:00:00Z
```

Relative windows (`1h`, `24h`, `7d`, `30d`, `all`) anchor to recorded bounds when data exists. Explicit timestamps include a timezone and select an interval that includes its start and excludes its end. Reports disclose the resolved window. Repeated append-only ingestion advances watermarks without counting accepted bytes again. Each analytics database accepts one OpenRig root.

For additional export adapters and version limits, see the [Connector SDK](../extensions/README.md).

## Inspect a report and its trace

```sh
./target/release/dendro report activity --window 7d
./target/release/dendro report handoffs --friction --window 7d
./target/release/dendro trace --metric-id handoff_friction --window 7d
```

Keep the rig, period, and thresholds aligned when comparing a report with its trace. Coverage shows which records support the result; missing observations remain visible. Reports and traces share the same canonical metric definitions.

Seats use `<role>@<named-rig>` labels. Queue endpoints and gates are separate entities; press `x` in the cockpit to reveal them. Status also distinguishes linked seats from standalone or unlinked usage.

## Reproduce an ingestion

```sh
./target/release/dendro run --root codex=/absolute/path/to/codex --write-manifest ./cutoffs.json
# Replay into a separate private analytics database
./target/release/dendro run --db /absolute/private/replay/dendro.duckdb \
  --root codex=/absolute/path/to/codex --sealed ./cutoffs.json
```

Sealed replay consumes the recorded source prefixes and checks that accepted bytes have not changed. Keep the manifest private because it can contain source locations. See [Run Operations](../run-operations.md) for ingestion and recovery procedures.

## Privacy and sharing

Source logs remain read-only. Dendro stores allowlisted operational facts and redacted diagnostics while keeping prompts, thoughts, responses, tool payloads, and secrets out of the analytics store. Select session roots, keeping authentication files and credential directories outside your input selection.

Core reports run locally. Offline dashboard exports alias seat labels; inspect any artifact before sharing it. An optional third-party conversational client has its own model and network configuration.
