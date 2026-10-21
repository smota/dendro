# Dendro v0.7.0 Release Notes — Fleet Brief, Cockpit, and Honest Ask

**Release Date:** 2026-10-20
**Tag:** `v0.7.0`
**Previous Release:** `v0.6.0`

---

## Executive Summary

**Dendro v0.7.0** makes the operator surface match the evidence that v0.6.0 already stored. Bare `dendro` still opens a read-only status, and that status is now a sectioned fleet brief. `dendro --help` carries a quickstart, the cockpit keys, and the report kinds. The interactive dashboard is a six-screen Ratatui cockpit. Ask routing is two explicit tiers. Pi ingest reads the OpenRig tree and the standalone `~/.pi/agent/sessions` tree, and status says which is which.

Analytical rules are unchanged. Missing observations stay unknown. Reports do not invent ROI. Source logs stay read-only. `#![deny(unsafe_code)]` and parameterized DuckDB queries stay in force. The workspace package version is `0.7.0`.

---

## Highlights

### 1. Fleet status brief

`dendro` with no arguments runs `dendro status`. `dendro --help` and `dendro --version` are unchanged in role. The text report is organized as:

- `=== DENDRO FLEET INTELLIGENCE ===`
- `DATABASE & COVERAGE`
- `DISCOVERED HARNESS SOURCES`
- `FLEET TOPOLOGY (RIGS & AGENT SEATS)`
- `QUICKSTART & COMMANDS`, then `Next commands:`

The coverage line is still one machine-readable sentence:

- Missing database: `State: NOT INITIALIZED`
- Populated database: `State: READY · usage=<n> · activity=<n> · queue=<n>`
- Database with no timestamps: `State: EMPTY · usage=... · activity=... · queue=...`

When Pi rows exist, coverage includes:

`Pi sources: openrig_files=<n> · standalone_files=<n> · linked_usage=<n> · standalone_usage=<n>`

That string is `pi_breakdown` in `dendro status --format json` (and `harnesses.pi`). The brief also prints `Usage: dendro <COMMAND> [OPTIONS]` and a command table for `status`, `dashboard`, `ui`, `ask`, `run`, `report`, `trace`, and `simulate`.

Agent seats are printed per rig, for example `- dendro-dev (8): dev-build, dev-qa, ...`. A long rig wraps onto indented lines instead of one 40-seat line. A short roster also keeps `Agent Seats (N): role@rig, ...`. The compact hygiene line counts total ids, agent seats, queue endpoints, system/gates, and unlinked ids.

### 2. Entity hygiene

The same classifier feeds status and the cockpit.

| Kind | What it matches |
| --- | --- |
| Agent seat | `<role>@<rig>` when the rig is not a raw ULID |
| Queue endpoint | `qitem-…` |
| System or gate | `external:…`, `system:…`, `…@system`, or exactly `system`, `steerer`, `steer-lead` |
| Unlinked id | every other id, including a 26-character Crockford ULID that starts with `01` |

Named rigs are listed. Ephemeral ULID rigs are a count (`+N ephemeral ULID`), not extra operator rigs. In the cockpit, `x` shows or hides queue endpoints and gates. The default view stays on canonical agent seats.

### 3. Six-screen cockpit

`dendro dashboard` and `dendro ui` render a Ratatui cockpit. TachyonFX plays the screen transitions. Modal overlays sit on a Porter-Duff RGBA backdrop (the OpenTUI-style compositor in `dendro-cli`) instead of clearing the screen to a flat color.

| Screen | Key | What it shows |
| --- | --- | --- |
| Pulse | `1` | Fleet overview and entity hygiene |
| Seats & Timeline | `2` | Five-band ribbon or shared-axis timeline |
| Handoffs & Flow | `3` | Pair flow and the heatmap |
| Tokens & Cache | `4` | Token categories and cache |
| Topology Simulator | `5` | Historical what-if over the decisions report |
| Trace & Spotlights | `6` | Metric trace and spotlight evidence |

Keys: `/` filter seats, `x` hygiene toggle, `m` ribbon or timeline, `v` cycle the handoff heatmap (return rate, count, wait median, exec median), `r` cycle named rigs, `w` cycle `1h` / `24h` / `7d` / `30d` / `all`, `:` or `a` ask Nao, `i` preview an issue draft, `?` or `h` help, `q` quit.

`--once`, `--html`, `--auto-ingest`, and `--window` behave as they did in v0.6.0.

### 4. Two-tier Nao routing

`dendro ask` still refuses unknown questions rather than inventing SQL.

1. Exact match on the normalized phrase (case, whitespace, and a trailing `?` or `.`).
2. Token scoring against the supported vocabulary for Rank, Explain, Compare, Verify, Trace, and HonestLimits. Unsupported qualifiers refuse first: mission or calendar words, SQL verbs, forecast or savings language, secrets, and any token containing `@`.

A miss returns ranked suggestions (`suggest_closest_intents`) and the text renderer prints the closest examples. `--format text` prints that answer. `--format json` prints the document. The default is text when stdout is a terminal and json otherwise, so existing pipes keep a single JSON object. `--list-intents` still prints the catalog as JSON and does not need a database.

### 5. Dual-root Pi ingestion

Default roots include both `$OPENRIG_HOME/state/pi` (or `~/.openrig/state/pi`) and `~/.pi/agent/sessions`. Two distinct `--root pi=<path>` values are both kept. The same path twice is deduplicated. A Pi file under `.pi/agent/sessions` counts as a standalone file. Other Pi files count as OpenRig files. Usage whose `native_session` is in Pi `identity_facts` is linked usage. The rest is standalone usage. Session identity is still the first-line header rule from v0.5.0 (K10). Standalone usage is not promoted to a seat.

### 6. CLI help

`dendro --help` orders commands for an operator: `status`, `run`, cockpit and ask, reports, then governance and cache commands. The after-help block is Quickstart Examples, Interactive TUI Shortcuts, Report Kinds, and command groups. Report kinds are `links`, `activity`, `handoffs`, `tokens`, `decisions`, `advisory`, `anomalies`, and `spotlights`. Subcommand flags sit under headings such as Database & Scope, Ingest, Scope, Cockpit, Question, Friction Thresholds, Advisory & Spotlights, and Scenario.

---

## Compatibility

- JSON status, report, trace, and ask documents keep their fields. Text status gained sections around the existing state, Pi, and next-command lines.
- `dendro ask` without `--format` now prints text on a terminal. Scripts that parse ask output should pass `--format json` or pipe stdout.
- v0.6.0 window resolution, `--auto-ingest`, sealed replay, and the R1 report contracts are unchanged.
