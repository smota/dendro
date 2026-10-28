# Dendro v0.6.0 — Real Operation & Usage Video Script

> [!NOTE]
> **Modern UI Walkthrough Available**: This document records the historical v0.6.0 terminal explorer walkthrough. For the modern, production **Connected Intelligence Studio** walkthrough (Apple-grade web UI, 4-Layer Exploding Stack X-Ray, 5-Layer Capability Studio, and 5 Active Intelligence Consoles), see [**`docs/VIDEO_WALKTHROUGH.md`**](VIDEO_WALKTHROUGH.md).

**Video Asset:** [`docs/assets/dendro-demo.mp4`](assets/dendro-demo.mp4) (historical v0.6.0 archive)  
**Animated Preview:** [`docs/assets/dendro-preview.gif`](assets/dendro-preview.gif) (historical v0.6.0 archive)  
**Recorded Binary:** `dendro 0.6.0` (`./target/release/dendro`)  
**Resolution / Duration:** `1920x1080` @ `12 fps` (`41.0s` H.264 MP4 + `960x540` optimized palette GIF)

---

## Purpose & Analytical Integrity

This walkthrough is recorded end-to-end by executing the real `./target/release/dendro` (`v0.6.0`) binary against a local multi-seat OpenRig + Claude dataset (`test-rig`: `orch-arch@test-rig`, `dev-build@test-rig`, `rev-review@test-rig`) and capturing the exact terminal and JSON outputs produced by the binary.

In accordance with Dendro's analytical governance:
- Every terminal screen in the video displays **unedited output** captured from `./target/release/dendro`.
- Time windows are disclosed explicitly (`window=recorded_bounds` / `anchored_to_recorded_data`).
- Missing denominators remain `Unavailable` (`null`), never fabricated zero.
- Before/after comparisons and `simulate topology` outputs are explicitly labeled as observational what-if deltas, not causal predictions.

---

## Scene-by-Scene Script & Captured Real Outputs

### Scene 1 — Zero-Config Status Preflight (`00:00 – 00:06.5`)
* **Visual Goal:** Show how a first-time or daily operator runs bare `dendro` (or `dendro status`) to inspect local harness discovery and database readiness without memorizing flags or RFC3339 timestamps.
* **Command Executed:**
  ```sh
  $ dendro status --db ./fleet.duckdb
  ```
* **Captured Real Output:**
  ```text
  Dendro Status · db=./fleet.duckdb
  State: READY · usage=23 · activity=5 · queue=9
  Recorded window: 2026-10-01T00:00:00Z to 2026-10-01T18:00:00Z
  Rigs (1): test-rig
  Seats (4): dev-build@test-rig, orch-arch@test-rig, rev-review@test-rig, unknown
  Semantic (Nao) cache: STALE / NOT MATERIALIZED (auto-refreshed by 'dendro ask')

  Next commands:
    $ dendro dashboard --db ./fleet.duckdb
    $ dendro ask --db ./fleet.duckdb "Which seat had the highest idle time?"
    $ dendro dashboard --db ./fleet.duckdb --html dendro-dashboard.html
  ```

---

### Scene 2 — One-Command Terminal Dashboard & Activity Ribbons (`00:06.5 – 00:13.5`)
* **Visual Goal:** Launch the terminal explorer in one command (`dendro dashboard`) without supplying `--from`/`--to`, demonstrating automatic window anchoring (`window=recorded_bounds`), the 5 disjoint activity bands, and the `?`/`h` keyboard help overlay.
* **Command Executed:**
  ```sh
  $ dendro dashboard --db ./fleet.duckdb --once
  ```
* **Captured Real Output:**
  ```text
  Dendro local explorer · 2026-10-01T00:00:00Z to 2026-10-01T18:00:01Z · window=recorded_bounds
  Tab/←/→ panels · ↑/↓ seats · Enter detail · 1/2/3 trace · ?/h help · q quit

  Activity bands (aggregate duration, not inferred tool execution)
  > dev-build@test-rig ████████████████████████▓▓▓▓▓▓▓▓▓▓▓░░░░
    rev-review@test-rig ███████████████████████████████░░░░░░░░░
  █ RUNNING · ▓ IDLE-WITH-WORK · ░ IDLE-WITHOUT-WORK · ▒ NEEDS-INPUT · ? UNKNOWN
  RUNNING: 39601 s
  IDLE-WITH-WORK: 18000 s
  IDLE-WITHOUT-WORK: 7200 s
  NEEDS-INPUT: 0 s
  UNKNOWN: 0 s
  ```

---

### Scene 3 — Zero-Ceremony Conversational Analytics (`00:13.5 – 00:20.5`)
* **Visual Goal:** Ask natural plain-English questions directly via positional `dendro ask "<question>"` without running `dendro materialize` first. Show automatic semantic view materialization (`"auto_materialized": true`) and natural phrase support.
* **Command Executed:**
  ```sh
  $ dendro ask --db ./fleet.duckdb "highest idle"
  ```
* **Captured Real Output (summary projection):**
  ```json
  {
    "status": "ok",
    "auto_materialized": true,
    "window_resolution": "recorded_bounds",
    "ordering": "idle_with_work_secs descending",
    "scope": {
      "definition_version": "acceptance-r1-a1",
      "from": "2026-10-01T00:00:00Z",
      "rig": null,
      "to": "2026-10-01T18:00:01Z"
    },
    "top_seat": {
      "seat": "dev-build@test-rig",
      "active_secs": 39601,
      "idle_with_work_secs": 18000,
      "idle_without_work_secs": 7200
    }
  }
  ```

---

### Scene 4 — Executive Decisions Table & Smart `--window` (`00:20.5 – 00:27.5`)
* **Visual Goal:** Inspect human-readable decision tables (`M1` yield, `M2` return rate, token categories, concurrency, and per-seat attribution) using `--window 7d` anchored automatically to `test-rig`'s recorded bounds.
* **Command Executed:**
  ```sh
  $ dendro report decisions --db ./fleet.duckdb --rig test-rig --window 7d --format table
  ```

---

### Scene 5 — Observational Topology What-If Simulator (`00:27.5 – 00:34.0`)
* **Visual Goal:** Model a topology change (`--remove rev-review@test-rig --direct-handoffs`) over historical facts using `--window 7d`, showing token conservation (`"conserved": true`) and explicit non-causal limits.
* **Command Executed:**
  ```sh
  $ dendro simulate topology --db ./fleet.duckdb --rig test-rig --window 7d --remove rev-review@test-rig --direct-handoffs
  ```

---

### Scene 6 — One-Command Offline HTML Dashboard Export (`00:34.0 – 00:41.0`)
* **Visual Goal:** Export the self-contained, secret-redacted, seat-aliased offline HTML viewer (`29,368 bytes`) in a single command (`dendro dashboard --html`) and preview the interactive 3-layer visualization (`Seat Activity Timeline`, `Handoff Flux Sankey`, and `Pair Friction Heatmap`).
* **Command Executed:**
  ```sh
  $ dendro dashboard --db ./fleet.duckdb --rig test-rig --html ./fleet-dashboard.html
  ```
* **Captured Real Output:**
  ```json
  {
    "format": "html",
    "from": "2026-10-01T00:00:00Z",
    "output": "./fleet-dashboard.html",
    "seat_labels": "aliases",
    "status": "exported",
    "to": "2026-10-01T18:00:01Z",
    "window_resolution": "recorded_bounds"
  }
  ```
