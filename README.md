<div align="center">
  <img src="docs/assets/logo.jpg" alt="Dendro growth rings" width="180" />
  <h1>Dendro</h1>
  <p><strong>Decipher the growth layers of your AI agent fleets.</strong></p>
  <p>The Local-First Flight Recorder, Causal X-Ray &amp; Operational Intelligence Studio for Multi-Agent Engineering.</p>
  <p>
    <a href="#visual-tour">Visual Tour</a> ·
    <a href="#why-dendro">Why Dendro</a> ·
    <a href="#architecture">4-Layer + 5-Capability Architecture</a> ·
    <a href="#active-intelligence">Active Intelligence (I22–I25)</a> ·
    <a href="#quickstart">60-Second Quickstart</a> ·
    <a href="docs/README.md">Documentation Hub (Index)</a> ·
    <a href="#documentation">Documentation</a>
  </p>
</div>

**Your fleet has a history. Now you can read it.** Dendro is the local-first flight recorder and Operational Intelligence engine for multi-agent AI fleets across **Claude Code, Codex, AGY, Grok, Pi, and OpenRig**. It turns authorized local JSONL and SQLite harness records into an embedded DuckDB evidence layer: see where tokens went, which capabilities ran, and where coordination slowed down. It observes execution; fleet governance stays with the operator.

<div align="center">
  <img src="docs/assets/screenshots/hub-living-growth-rings.png" alt="Dendro connected studio with Living Growth Rings, a date scrubber, and fleet story highlights" width="1200" />
  <p><strong>Read your entire fleet like an instrument.</strong> Scrub across any time window with the <strong>Time Machine</strong>, spot anomalies at a glance on <strong>The Living Growth Rings</strong>, and jump straight from <strong>Story Highlights</strong> into the underlying layer.</p>
</div>

<a id="why-dendro"></a>

## Why Dendro? From opaque logs to operational clarity.

Modern AI engineering spans heterogeneous harnesses—Claude Code, Codex, AGY, Grok, Pi—and orchestrators such as OpenRig, AgentFlow, and Meshloop. When tokens spike, subagents loop, or handoffs stall, the answer is scattered across incompatible logs. A token total shows consumption. It rarely tells you where to investigate.

Dendro brings those records into a **4-Layer Causal Architecture**, then adds **5-Layer Capability & Execution Intelligence**. Move from a fleet-wide pattern to the harness primitive, skill, MCP call, or guardrail that helps explain it.

<a id="architecture"></a>

### Four architectural layers. Five lenses on execution.

| Architectural layer | What becomes visible | The operator's question |
| --- | --- | --- |
| **Model** | Tokens, cache yield, pricing, counterfactual routing | Where is consumption concentrated, and what might another routing policy change? |
| **Harness** | Tool families, primitives, Trajectory Vortex, Schema Drift | Which execution behaviors repeat, fail, or change between versions? |
| **Coordination** | Sub-agent lineage, handoff friction, session yield | Where does work wait, return, or lose context? |
| **Environment** | CPU/RSS footprint, sandbox posture, authority blast radius | What resources and evidenced dependencies surround this session? |

The **5-Layer Capability & Execution Intelligence** view resolves what happened within that architecture:

| Capability layer | What you inspect |
| --- | --- |
| **Tool Families** | 10 behavioral families: **Shell / Exec**, **Read / Inspect**, **Edit / Patch**, **Search / Grep**, **Subagent Spawn**, **MCP Call**, **Skill Load**, **Web / Fetch**, **Task / Queue**, **Governance / Gate** |
| **Harness Primitives** | Native tools and their observed use, outcomes, and failure patterns |
| **Skills (Declared vs. Invoked)** | Which skills are available and which have evidence of execution |
| **MCP Servers & Tools** | Observed calls grouped by server and tool |
| **Lifecycle Hooks & Guardrails** | Recorded lifecycle stages, interceptions, and governance observations |

**Zero-Content Privacy: zero prompts, zero thoughts, zero secrets stored.** Source logs are read-only. No prompts, agent thoughts, completions, or secrets (`sk-`, `ghp_`, Bearer tokens) enter DuckDB. Dendro retains allowlisted operational facts, redacted diagnostics, and opaque evidence keys. Local-first analysis keeps this evidence on your machine; sharing remains an explicit operator action.

### Evidence you can follow

- **Seat dynamics:** five disjoint activity bands, unknown intervals retained, fleet concurrency and container hold evidence.
- **Handoff flow:** pickup and execution latency, return rates, parent-linked rework loops, and conservative token attribution.
- **Consumption:** uncached input, cache reads/writes and output, with separate reasoning detail and explicit linkage coverage.
- **Traceability:** reports and forensic traces share the same metric definitions, observation window and evidence keys.

Missing observations are not measured zero. Tokens do not establish value or causal productivity. Recorded cost and operator-supplied prices must be distinguished from one another.

<a id="visual-tour"></a>

## Visual tour

Follow a fleet-wide signal into the stack, inspect the capabilities behind it, and choose your next move.

### 4-Layer Stack X-Ray

![4-Layer Stack X-Ray across Model, Harness, Coordination, and Environment](docs/assets/screenshots/hub-stack-xray-3d.png)

**Pull the stack apart (`⌘E`).** Trace execution across **Model → Harness → Coordination → Environment** in flat swimlanes or 3D isometric view, and pivot instantly by harness, layer, or date.

### 5-Layer Capability & Execution Intelligence

![5-Layer Capability Studio showing tool families, primitives, skills, MCP, and hooks](docs/assets/screenshots/hub-capability-studio.png)

**Know what your agents actually use.** See the balance across **Tool Families, Harness Primitives, Skills (Declared vs. Invoked), MCP Servers & Tools, and Lifecycle Hooks & Guardrails**—so you know which capabilities drive results and which sit dormant.

<a id="active-intelligence"></a>

### Active Intelligence (`I22`–`I25`)

<table>
  <tr>
    <td width="50%" valign="top">
      <img src="docs/assets/screenshots/console-trajectory-vortex.png" alt="I22 · Trajectory Vortex console" width="600" />
      <p><strong>I22 · Trajectory Vortex: Catch loops before runaway token burn.</strong></p>
      <p>Inspect repetitive behavior and stalled reasoning orbits to choose the next session to investigate.</p>
    </td>
    <td width="50%" valign="top">
      <img src="docs/assets/screenshots/console-schema-drift.png" alt="I23 · Schema Drift console" width="600" />
      <p><strong>I23 · Schema Drift: Spot structural shifts after an upgrade.</strong></p>
      <p>Compare telemetry shapes across Claude Code, Codex, AGY, Grok, and Pi to identify the connector or version that needs attention.</p>
    </td>
  </tr>
  <tr>
    <td width="50%" valign="top">
      <img src="docs/assets/screenshots/console-blast-radius.png" alt="I24 · Authority Blast Radius console" width="600" />
      <p><strong>I24 · Authority Blast Radius: Map your dependency exposure.</strong></p>
      <p>Follow downstream session reachability from a permission grant or failure to focus your review.</p>
    </td>
    <td width="50%" valign="top">
      <img src="docs/assets/screenshots/console-counterfactual-routing.png" alt="I25 · Counterfactual Model Routing console" width="600" />
      <p><strong>I25 · Counterfactual Model Routing: Test routing choices against history.</strong></p>
      <p>Explore fast-tier thresholds and context-pressure inflections against recorded sessions without touching live production traffic.</p>
    </td>
  </tr>
</table>

### Operational Economics

![Operational Economics console with token components and group-by rollups](docs/assets/screenshots/console-token-economics.png)

**Every token accounted for, sliced your way.** Separate cache reads, cache writes, fresh input, and reasoning output, then group by **Harness, Capability, Date, Workspace, or Outcome** in one click.

[Explore the Visual Product Tour →](docs/VISUAL_PRODUCT_TOUR.md)

## Who is Dendro for?

| Operator | Daily playbook |
| --- | --- |
| **Applied-AI Engineers** | Scrub a difficult run, inspect tools and trajectory loops, then trace the evidence behind retries. |
| **Fleet Architects** | Follow sub-agent lineage and handoff friction; compare topology scenarios against recorded work. |
| **FinOps / Token Economists** | Split cache and uncached consumption, group by harness or team, and compare pinned pricing scenarios. |
| **Security & Governance Leads** | Inspect sandbox and hook observations, review evidenced downstream exposure, and retain operator authority over changes. |

<a id="quickstart"></a>

## Start locally (60-Second Quickstart)

From an available binary, choose the surface that fits the job. Build time depends on your machine; “60 seconds” describes the command path, not a timed installation claim.

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

# 2. Ingest authorized local sources, then open the live connected web studio
./target/release/dendro run
./target/release/dendro dashboard --serve 127.0.0.1:4242 --site-dir ./site
# Open http://127.0.0.1:4242 in your browser; Ctrl-C stops the web server.

# 3. Launch the interactive TUI dashboard (or --once snapshot), auto-ingesting if needed
./target/release/dendro dashboard --auto-ingest

# 4. Export the self-contained, secret-redacted, seat-aliased offline HTML dashboard
./target/release/dendro dashboard --html ./dendro-dashboard.html

# 5. Ask natural questions (auto-materializes semantic views over the active window)
./target/release/dendro ask "highest idle"
./target/release/dendro ask --format text "where do tokens go?"
./target/release/dendro ask --list-intents
```

Run these as separate sessions or alternatives: the web server and TUI hold the foreground until you exit. With the binary on `PATH`, the same entry points are `dendro`, `dendro dashboard --serve 127.0.0.1:4242`, `dendro dashboard`, and `dendro ask`.

For the trusted single-owner daemon and pre-flight advisor:

```sh
# Start the trusted owner of the analytics database (foreground)
./target/release/dendro serve

# In a separate terminal, request operator-facing pre-flight advice
./target/release/dendro advise --question environment_readiness
```

`dendro serve` exposes typed local IPC. The connected web studio exposes a loopback HTTP presentation surface, including `/api/query` (JSON query projection endpoint) and `/api/events` (Server-Sent Events live invalidation stream); it is not a raw SQL endpoint. `dendro advise` targets a **warm p95 <=15ms budget**, with explicit budget-exceeded results rather than partial advice. This budget is not a cold-start or end-to-end performance guarantee. `dendro ask` uses Nao semantic projections and a supported intent vocabulary; inspect `--list-intents` to see the questions it can answer.

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

- [Documentation Hub (Index)](docs/README.md): choose your path through Dendro.
- [Visual Product Tour](docs/VISUAL_PRODUCT_TOUR.md): the connected studio and focused consoles.
- [Architecture Blueprint](docs/ARCHITECTURE.md): four architectural layers, five capability layers, Schema 5, and insight contracts.
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
