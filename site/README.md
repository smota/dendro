# Static site and offline fleet explorer

Open `index.html` in a browser, or open `viewer.html` directly. No build, server,
package install, account, telemetry, remote fonts or network connection is required.
The existing video and logo remain local assets. Downloading the explorer produces
one self-contained HTML file; it accepts a sanitized analytical JSON file locally.

## Inputs

Supported inputs are the exact `dendro report <kind>` JSON envelope
(`metric`, `definition_version`, `payload`), a report payload whose `product` is
`AI Fleet Intelligence`, or a `dendro trace --metric-id <id>` envelope with `inputs`.
Use the CLI's real `--db`, `--from`, `--to`, `--rig` and threshold flags as needed.
The viewer does not query DuckDB or create missing measurements.

Only these analytical projections are consumed:

| Field | Display |
| --- | --- |
| `tokens`, `m3_rework_chains`, `fleet_concurrency.effective_parallelism` | Macro pulse |
| `m1_by_seat` | Tokens, closed items, nonreturned deliveries, tokens per closed item |
| `activity` | Five activity durations, container subset, tool-call totals, observed life spans |
| `activity_events`, `from/to` or `window.from/to` | Recorded state timeline, clipped to observation spans |
| `context_compactions.events` | Timestamped context-drop markers when seat and time are available |
| `fleet_concurrency.histogram` | Duration at k active seats (distribution, not time series) |
| `handoff_members` or `handoff_edges` | Count-weighted sender/receiver flow and explicit returns |
| `token_categories` | Fresh input, cache reads, cache writes and output |
| `pair_medians`, `m2_pairs`, `ring_boundaries.edges` | Pair matrix: counts, return rates, wait/execution medians and boundary classifications |
| `task_intake_overhead` | Actual assigned, ambiguous, unlinked and intake token buckets |

Null, missing and nonnumeric metrics remain unavailable. Input is limited to
10 MiB and 50,000 rows per supported array. At most 100 timeline lanes and 100
pair ribbons render; the view discloses these bounds. Exact seat names, item IDs,
source paths, source keys, commands, transcripts and arbitrary metadata are never
rendered; seat identities become consistent `Seat N` aliases in each load. The
original file is not rewritten or sanitized. Do not redistribute raw analytical
files merely because their rendered labels are aliased.

A running-state interval is not proof of leaf execution. Aggregate container hold
seconds cannot be positioned on a timeline. Tool-category totals cannot color
individual execution intervals without interval evidence. Token categories cannot
be attributed to a handoff or productive value from these fields alone. The view
states those limitations instead of synthesizing evidence. Context reset markers
appear only when the export includes actual timestamped compaction events.

## Embedded CLI export contract

The literal marker is:

```html
<script id="dendro-data" type="application/json">null</script>
```

An exporter may replace `null` with an allowlisted analytical report/trace JSON
object. It must scrub the exported bytes before sharing: the browser's projection
only protects the display. Escape `<`, `>`, `&`, U+2028 and U+2029 as JSON Unicode
escapes before insertion. In particular, a raw `</script>` must never enter the
HTML script element. The document has no runtime dependency outside itself.
An invalid embedded object displays an error and does not fall back to demo data.

## Calculator model

Every slider value, including defaults, is an illustrative operator assumption.
Fleet missions are the total across all seats; increasing seats divides workload
without silently multiplying token consumption.

- Expected returned missions = missions × rework proportion.
- Tokens in rework = expected returns × average tokens per mission.
- Monthly modeled rework spend = tokens in rework ÷ 1,000,000 × blended rate.
- Potentially avoidable developer hours = expected returns × hours per return × assumed reduction.
- Released token capacity (%) = rework proportion (%) × assumed reduction (%)/100.
- Missions per seat = fleet missions ÷ seats.

The model assumes a return consumes one average mission's tokens. Fractional
expected returns are retained rather than rounded before downstream calculations.
No empirical return rate, human effort estimate or velocity improvement is built
in. Blended cost is an operator estimate, not a current provider price quote.
Neither released capacity nor consumption measures business value.

## Verification

Run `node --test site/site.test.cjs` from the repository root. These dependency-free
checks execute the real scripts against a minimal DOM to verify the calculation,
report/trace adapters, missing-data behavior, privacy projection, SVG interactions,
keyboard tabs and failed imports. They are not a replacement for browser layout QA.
