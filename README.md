<div align="center">
  <img src="docs/assets/logo.jpg" alt="Dendro growth rings" width="180" />
  <h1>Dendro</h1>
  <p><strong>Decipher the growth layers of your AI agent fleets.</strong></p>
  <p>Local-first AI Fleet Intelligence. Operational evidence for the operator.</p>
</div>

Dendro reads authorized local harness logs and OpenRig records into embedded DuckDB. It answers questions about seat activity, handoff friction, token consumption, rework, and context carryover. It observes execution; fleet governance stays with the operator.

- **Seat dynamics:** five disjoint activity bands, unknown intervals retained, fleet concurrency and container hold evidence.
- **Handoff flow:** pickup and execution latency, return rates, parent-linked rework loops, and conservative token attribution.
- **Consumption:** uncached input, cache reads/writes and output, with separate reasoning detail and explicit linkage coverage.
- **Traceability:** reports and forensic traces share the same metric definitions, observation window and evidence keys.

Missing observations are not measured zero. Tokens do not establish value or causal productivity. Recorded cost and operator-supplied prices must be distinguished from one another.

## Walkthrough & Real Operation (`v0.6.0`)

<div align="center">
  <a href="docs/assets/dendro-demo.mp4">
    <img src="docs/assets/dendro-preview.gif" alt="Dendro v0.6.0 real operation walkthrough" width="900" />
  </a>
  <p>
    <a href="docs/assets/dendro-demo.mp4"><strong>▶ Watch the full HD video (`docs/assets/dendro-demo.mp4`)</strong></a>
    &nbsp;·&nbsp;
    <a href="docs/VIDEO_WALKTHROUGH_v0.6.md"><strong>Read the scene-by-scene script & captured outputs (`docs/VIDEO_WALKTHROUGH_v0.6.md`)</strong></a>
  </p>
</div>

The walkthrough above is recorded from real `./target/release/dendro` (`v0.6.0`) execution over a local OpenRig + Claude multi-seat dataset (`dev-build@test-rig`, `rev-review@test-rig`, `orch-arch@test-rig`), showing zero-config status preflight, the terminal activity ribbon explorer, auto-materializing conversational queries, decision tables, observational topology simulation, and one-command offline HTML dashboard export.

## Wave v0.7

`v0.7.0` is the operator surface on top of that evidence engine. Bare `dendro` still runs `dendro status`. The text report is a sectioned fleet brief: database coverage, discovered harness roots, seats grouped by rig, and a command reference. `dendro --help` adds quickstart examples, cockpit shortcuts, and report kinds.

- **Cockpit:** `dendro dashboard` and `dendro ui` open six Ratatui screens (Pulse, Seats & Timeline, Handoffs & Flow, Tokens & Cache, Topology Simulator, Trace & Spotlights) with TachyonFX transitions and an OpenTUI-style RGBA backdrop under the modals. Keys: `1`–`6`, `/`, `x`, `m`, `v`, `r`, `w`, `:`, `i`, `?`, `q`.
- **Entity hygiene:** canonical agent seats are `<role>@<named-rig>`. `qitem-*` ids are queue endpoints. `external:`, `system:`, `@system`, `system`, `steerer`, and `steer-lead` are gates. Everything else, including 26-character `01…` ULIDs, stays unlinked. Named rigs and ephemeral ULID rigs are counted separately.
- **Ask:** exact phrase match, then token scoring against supported intent vocabulary. Refusals suggest the closest intents. `--format text|json` selects the rendering; a terminal defaults to text and a pipe defaults to json.
- **Pi:** default ingest reads both OpenRig `state/pi` and standalone `~/.pi/agent/sessions`. Status reports `pi_breakdown` as openrig files, standalone files, linked usage, and standalone usage.

Details are in the [v0.7.0 release notes](docs/RELEASE_NOTES_v0.7.0.md).

## Start locally

Install stable Rust and Git, then:

```sh
git clone https://github.com/smota/dendro.git
cd dendro
cargo build --release --locked
./target/release/dendro
```

### Zero-ceremony quickstart

Running bare `dendro` (or `dendro status`) prints the fleet brief: database readiness, discovered local harness roots, rig-scoped recorded bounds, seats grouped by rig, Pi source coverage, semantic cache freshness, and the next commands.

With no `--db` and no `--storage-root`, the database is `dendro.duckdb` in the private default folder: `%LOCALAPPDATA%\Dendro` on Windows, or `$XDG_DATA_HOME/dendro` (default `~/.local/share/dendro`) on Linux. `dendro storage-init` creates or verifies that folder and prints its path. If `./dendro.duckdb` is in the current directory and that folder has no database yet, Dendro refuses to open it and prints the command to copy it there (PowerShell `Copy-Item -LiteralPath` on Windows, `cp --` on Linux), including `dendro.duckdb.wal` when that file is beside it. Dendro does not copy or move the file. `--db` outside a private folder is refused the same way. When the current directory is already a private folder, `./dendro.duckdb` is opened there with no extra message.

```sh
# 1. Check status, discovered local harness sources, and recorded bounds
./target/release/dendro status

# 2. Launch the interactive TUI dashboard (or --once snapshot), auto-ingesting if needed
./target/release/dendro dashboard --auto-ingest

# 3. Export the self-contained, secret-redacted, seat-aliased offline HTML dashboard
./target/release/dendro dashboard --html ./dendro-dashboard.html

# 4. Ask natural questions (auto-materializes semantic views over the active window)
./target/release/dendro ask "highest idle"
./target/release/dendro ask --format text "where do tokens go?"
./target/release/dendro ask --list-intents
```

Inside the cockpit, press `?` for the full key list. `1` through `6` select the screens. `/` filters seats, `x` reveals queue endpoints and gates, `m` switches the seat ribbon and timeline, `v` cycles the handoff heatmap, `r` and `w` cycle rig and window, `:` asks Nao, and `i` previews an issue draft.

### Explicit roots & fixed windows

Choose authorized source roots explicitly and keep the output database outside those roots. You can pass `--window 1h|24h|7d|30d|all` (anchored to the database's rig-scoped recorded bounds when data exists) or explicit half-open `[from, to)` RFC3339 timestamps:

```sh
./target/release/dendro run \
  --root openrig=/absolute/path/to/openrig.sqlite \
  --root codex=/absolute/path/to/codex
./target/release/dendro report decisions --window 7d
./target/release/dendro report decisions \
  --from 2026-10-14T00:00:00Z --to 2026-10-15T00:00:00Z
./target/release/dendro trace \
  --metric-id handoff_friction --window 7d
```

Dates above are examples; choose the actual period recorded by your sources or use `--window`. Windows are half-open `[from, to)`. Every relative window resolution discloses `"window_resolution"` (`"recorded_bounds"`, `"anchored_to_recorded_data"`, `"explicit"`, or `"trailing_utc"`). An empty report may mean absent timestamps, unsupported data, or an unobserved period. A build from source is not a measured five-minute installation guarantee.

## Local investigation

Export a self-contained web bundle with `./target/release/dendro dashboard --html ./dendro-dashboard.html`, or open [the offline explorer](site/viewer.html) and load a report or trace JSON file. The viewer aliases seat labels and displays supported numeric facts. Its synthetic demo is labelled. Aggregate concurrency counts are a distribution, not a reconstructed time series; handoff counts do not establish token transfer.

The [landing page](site/index.html) includes a scenario calculator. Its inputs are assumptions, and its formulas describe potential released capacity, not predicted savings or ROI.

## Verification

```sh
cargo check --workspace --locked
cargo test --workspace --locked
cargo clippy --workspace --locked -- -D warnings
cargo fmt --check
node --test site/site.test.cjs
```

The real-local-log performance test is explicitly ignored by default and requires a separately authorized corpus. Synthetic correctness or benchmark results do not establish production-scale latency. See the benchmark's actual output and corpus before quoting a performance claim; milliseconds must not be called sub-millisecond.

## Documentation

- [Operator guide](docs/adoption/README.md): status, cockpit, source selection, reporting, privacy and reproducible replay.
- [CLI reference](docs/CLI_R1.md) and [Nao & local semantic queries](docs/NAO_SEMANTIC.md).
- [v0.7.0 release notes](docs/RELEASE_NOTES_v0.7.0.md), [v0.6.0 release notes](docs/RELEASE_NOTES_v0.6.0.md), and [real operation video script](docs/VIDEO_WALKTHROUGH_v0.6.md).
- [Maintainer guide](docs/maintainers/README.md): workspace architecture and quality gates.
- [Connector extension guide](docs/extensions/README.md): trait contract and runnable example.
- [Product specification](docs/PRODUCT_SPECIFICATION.md) and [R1 acceptance contracts](docs/ACCEPTANCE_R1.md).
- [Real operation walkthrough video](docs/assets/dendro-demo.mp4) and [preview animation](docs/assets/dendro-preview.gif).

## Privacy and license

Source logs are read-only. The analytics store uses allowlisted facts, redaction, opaque identifiers and explicit uncertainty. Never select credential files as ingestion roots. Exporting or publishing a report is a separate operator action; local storage alone does not make every possible export safe to share.

MIT License. Copyright 2026 Samuel Mota.
