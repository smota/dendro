# Dendro v0.6.0 Release Notes — Operator Usability, Zero-Ceremony Dashboards & Smart Defaults

**Release Date:** 2026-10-19  
**Tag:** `v0.6.0`  
**Previous Release:** `v0.5.0`

---

## Executive Summary

**Dendro v0.6.0** focuses on **operator usability and zero-ceremony workflows**, shaped by a 5-member multi-model **Usability Council** (Solo Operator Persona, Fleet Supervisor Persona, UX Specialist, Information Architect, and Product Owner across AGY, Codex, and Claude).

Previously, inspecting fleet health or asking a semantic question required knowing internal plumbing: running `dendro run`, discovering exact RFC3339 `--from` and `--to` timestamps in the database, manually running `dendro materialize` before `dendro ask`, and matching 6 exact canned strings. Running `dendro ui --window 7d` against a missing database failed with an opaque `Error: analytics database could not be opened`, and running it against recorded historical corpora anchored to `Utc::now()` instead of the recorded data window, rendering `100% UNKNOWN`.

In **v0.6.0**, operators can go from zero to insights in a single command while preserving Dendro's strict analytical truthfulness, read-only source guarantees, `#![deny(unsafe_code)]`, and parameterized DuckDB queries.

---

## Highlights & New Capabilities

### 1. Zero-Config Entry (`dendro` & `dendro status`)
- Running bare `dendro` (or `dendro status [--db <path>] [--rig <rig>] [--format text|json]`) inspects the analytics database and prints an immediate human-readable readiness summary:
  - Database status (`ready`, `empty`, or `missing_database`)
  - Discovered rigs, `usage_facts` count, `seat_events` count, and `handoffs` count
  - Exact recorded timestamp bounds (`[min_ts, max_ts]`)
  - Semantic snapshot status (`fresh`, `stale`, or `unmaterialized`)
  - Copy-pasteable next commands tailored to the current state
- Missing database errors now include actionable guidance:
  `analytics database could not be opened (not found; run 'dendro run' or 'dendro dashboard --auto-ingest' first)`

### 2. One-Command Dashboard (`dendro dashboard`) & `--auto-ingest`
- **`dendro dashboard`**: Launches the interactive terminal explorer (`--once` for a non-interactive snapshot) over the database's active time window without requiring `--from`/`--to`.
- **`dendro dashboard --html [path]`**: Exports the self-contained, secret-redacted, seat-aliased offline HTML dashboard (`dendro-dashboard.html` by default) in one step.
- **`--auto-ingest`**: Supported on both `dendro dashboard` and `dendro ui`. When the target DuckDB file does not exist or has zero recorded facts, Dendro automatically runs a local read-only ingestion pass before opening the dashboard.

### 3. Universal Smart `--window` Anchored to Recorded Data
- `--window 1h|24h|7d|30d|all` is now supported across `dashboard`, `ui`, `report`, `trace`, `materialize`, `export`, `ask`, and `simulate topology`.
- When `--from`/`--to` are omitted on a populated database, `resolve_window` anchors relative windows (`1h`, `24h`, `7d`, `30d`) to the database's recorded `max_ts` (clamped to `min_ts`) or uses the full `[min_ts, max_ts]` range (`all`), disclosing `"window_resolution": "anchored_to_recorded_data"` or `"recorded_bounds"` so historical and fixture databases immediately display real activity ribbons instead of `100% UNKNOWN`.

### 4. Zero-Ceremony `dendro ask` (Auto-Materialization & Natural Aliases)
- **Positional Question:** `dendro ask "highest idle"` works directly alongside `--question`.
- **Auto-Materialization:** When the semantic snapshot (`_dendro_semantic_meta`) is missing or stale, `dendro ask` automatically refreshes the 4 canonical views (`v_seat_idle_ranking`, `v_handoff_rework_analysis`, `v_mission_token_burn`, `v_executive_governance`) over the active window unless `--no-refresh` is set, recording `"auto_materialized": true` in the JSON response.
- **Natural Intent Discovery (`--list-intents`):** `dendro ask --list-intents` (or bare `dendro ask` with no question) prints all 6 canonical intents and example phrases without requiring a database.
- **Expanded Natural Phrases:** Recognizes natural questions for idle time (`"idle"`, `"highest idle"`, `"where is time being wasted?"`), yield/token burn (`"worst yield"`, `"token burn"`, `"where do tokens go?"`), rework (`"rework"`, `"handoff friction"`, `"why is there rework?"`), verification (`"verify"`, `"provenance"`), trace (`"trace idle"`, `"trace tokens"`, `"trace handoffs"`), and honest limits (`"limits"`, `"known limits"`), while still refusing out-of-scope mission/date qualifiers truthfully.

### 5. In-App TUI Keyboard Help (`?` / `h`)
- Pressing `?` or `h` in `dendro ui` / `dendro dashboard` opens an interactive help view documenting panel cycling (`Tab`, `Left`/`Right`), seat selection (`Up`/`Down`, `Enter`), forensic trace inspection (`1`/`2`/`3`), and exit (`q`, `Ctrl-C`).
