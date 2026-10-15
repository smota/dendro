# Dendro — Maintainer Guide

Welcome to the internal maintainer guide for **Dendro**. This guide details the engineering architecture, Rust code conventions, concurrency model, lifecycle governance, and quality standards for the project.

---

## 1. Core Architectural Overview

Dendro is a local-first AI fleet intelligence and decision engine built in modular Rust on embedded **DuckDB**:

```
[Local Session Sources]
 ├── OpenRig (~/.openrig/openrig.sqlite, state/)
 ├── Claude Code (~/.claude/projects/*/*.jsonl)
 ├── Codex (~/.codex/sessions/**/rollout-*.jsonl)
 ├── AGY (~/.gemini/antigravity-cli/brain/**/transcript.jsonl)
 ├── Grok (~/.grok/sessions/**/summary.json)
 └── Pi (~/.pi/agent/sessions/**/*.jsonl)
       │
       ▼
 [dendro-connectors] ───► Implements `HarnessConnector` (zero-copy streaming via BufReader)
       │
       ▼
 [dendro-resolver]   ───► Implements `CorrelationResolver` (binds sessions to Rig, Seat, and Mission)
       │
       ▼
 [dendro-core]       ───► DuckDbStorage (idempotent migrations, transactional batch insertion)
       │
       ▼
 [dendro.duckdb]     ───► Dimensional and columnar analytical tables
```

---

## 2. Rust Engineering Standards

### 2.1 Performance & Memory Efficiency (Zero-Copy)
* **Streaming Large Log Files:** Harness logs (Claude Code, Codex, AGY) can reach hundreds of megabytes. **Never** load entire files into memory with `fs::read_to_string`. Consume streams line-by-line using `BufReader::new(file).lines()`.
* **Incremental Watermarks:** Every ingestion writes byte offsets and last-read timestamps to `_dendro_watermarks`. On subsequent runs, connectors seek directly to the offset (`seek(SeekFrom::Start(offset))`).
* **DuckDB Transactions:** Batch write operations must be wrapped within a single transaction (`BEGIN TRANSACTION` ... `COMMIT`). Avoid individual single-row inserts.

### 2.2 Security & Defense in Depth
* **Strict Safe Rust:** The use of `unsafe` blocks is strictly forbidden across all workspace crates (`#![deny(unsafe_code)]`).
* **SQL Injection Prevention:** Query parameters must use `duckdb::params![...]` or prepared statements. SQL string concatenation (`format!("SELECT ... WHERE id = '{}'")`) fails CI quality gates.
* **Secret Redaction:** Prompts and commands matching known credential patterns across 10 families (`sk-`, `ghp_`, `gho_`, `Bearer `, `xai-`) must be redacted before analytical persistence.

---

## 3. Cargo Workspace Structure

```
crates/
├── dendro-core/         # Canonical schemas (CanonicalSession, CanonicalToolCall), core traits, DuckDbStorage
├── dendro-connectors/   # Specialized parsing adapters per agent harness
├── dendro-resolver/     # Correlation and binding algorithms (OpenRigResolver, StandaloneResolver)
├── dendro-watcher/      # Real-time monitoring via notify-debouncer-mini
└── dendro-cli/          # Binary entry point and CLI commands (run, watch, status, init-nao)
```

---

## 4. Quality Gates & Validation Commands

Before submitting any candidate receipt or handing off to the reviewer:

```bash
# 1. Strict compilation check
cargo check --workspace

# 2. Test suite execution
cargo test --workspace

# 3. Linter with zero warnings tolerated
cargo clippy --workspace -- -D warnings

# 4. Canonical formatting
cargo fmt --check
```

---

## 5. Autonomous Governance & SDLC Protocols

This repository is governed under the **AgentFlow SDLC** in **autonomous** posture:
* **Branch Protection:** Never commit directly to `main`. All implementation occurs in dedicated worktrees under `.worktrees/<feature-name>`.
* **Strict Four-Eyes Gate:** Builders never approve their own pull requests (`allowsSelfReview: false`). Independent review seats in `rev.review` audit the final candidate commit SHA before merge.

## Handoff wait precision (local v0.2.0 candidate, #34)

The existing `report handoffs` and `trace --metric-id medians` JSON journeys
compute each wait as the selected receiver claim timestamp minus the handoff
timestamp, in seconds. Timestamp subtraction precedes conversion to binary64;
neither endpoint nor the elapsed sample is deliberately truncated or rounded.
`wait_samples` retains its source order, duplicates and aligned source keys.
The median uses the sorted central sample for odd counts and the arithmetic
mean of the two central samples for even counts. Intake samples and medians
remain in tokens. Empty valid sample sets have a null median; equal timestamps
produce zero. Claim eligibility, attribution and window selection retain their
existing rules, including eligible claims after the handoff window ends.

JSON exposes finite computational numbers with normal round-trip representation
and no fixed-decimal presentation rounding. For the specified small vectors,
persisted microsecond endpoints and half-microsecond even medians survive within
1e-12 seconds. This improves numerical precision, not source clock accuracy.
Fractions beyond six decimal places remain outside the exact-preservation
guarantee: precision lost during persistence cannot be recovered by reporting.
Binary64 resolution depends on magnitude: an even half-microsecond increment
can disappear around 2^32 seconds, and a one-microsecond increment can disappear
around 2^34 seconds. Exact microsecond resolution is therefore not promised for
every large duration. These limits do not introduce a new date eligibility cutoff.

Only the medians trace uses formula version
`acceptance-r1-a1-wait-fractional-v1`; other trace versions retain
`acceptance-r1-a1`. Shared pair-median payloads expose the same corrected waits.
A report-only read of a supported existing database computes fractions from
retained timestamps without reingest or database writes. AC4 activity allocation
continues to use observation-span whole-second prefixes for conservation;
fractional wait conversion does not replace that quantizer.

## Codex command outcomes (local v0.2.0 candidate, #40)

The `report decisions` JSON payload keeps `tool_loops` and adds its sibling
`outcome_coverage`. Episodes retain session, tool and count, and add harness,
unit and seat. Claude/Pi units are tool calls. Codex native `CommandExecution`
items are command executions; only their own integer exit codes determine
success or failure. Codex seat-call outcomes remain `not read`, and observed
seat calls interrupt execution failure runs. Outputs never settle an outcome.
Commands never become structured writes or change intake endpoints.

Coverage describes the persisted complete prefix of each current source
generation. Zero sources, unread legacy prefixes, unsupported records, and
observed unknown fields remain distinct. Report-only opens never read sources
or backfill a database. Normal ingest backfills complete Codex prefixes without
recounting usage. Reader metadata, facts, watermark and session identity commit
atomically; metadata is excluded from observation loaders and the unit-aware
census, while privacy scans inspect every physical row.

Command arrays are compared using their original lexical JSON slices and a
keyed HMAC with fixed unit and array tags. String commands are currently unknown.
Raw input and output are never persisted. Ordering uses deterministic source
order, retains untimed anchors, and sorts timed segments between anchors. The
report window is applied first; qualifying endpoint spans must be nonnegative
and at most 600 seconds. A trusted Codex episode requires unique seat linkage.

The new reader can report a legacy schema-3 database with truthful unread
coverage, then backfill it on ingest. Older binaries interpreting a database
with the new observation units or reader metadata are semantically unsupported,
even though the physical schema version is unchanged. This describes a local
candidate; it does not announce a published release or a migration.

### Refusing analytics destinations that alias sources

One-shot ingest checks source identity using filesystem metadata before creating
output directories, acquiring the output lock, opening a database, or looking up
an analytics key. Explicit source files and regular files beneath declared
source directories are protected by path and device/inode identity, including
hard links outside the source tree. Directory containment includes new nested
outputs whose parents do not yet exist. Descendant symlink trees are skipped.

Typed OpenRig roots additionally protect the selected main file and its appended
`-wal` and `-shm` names, even when those sidecars are absent. Final output symlinks
remain forbidden. Physical parent resolution resumes after cancellation of a
missing component with `..`; unresolved identity or incomplete enumeration
refuses with a fixed message that contains no source path or OS error detail.
Genuinely missing roots retain their ordinary missing-root handling for distinct
outputs. This is a static identity check with existing publication checks; it
does not guarantee safety against concurrent malicious namespace replacement or
redesign auxiliary lock/key identity policy.

## Busy OpenRig source (local v0.2.0 candidate, #30)

After five unsuccessful snapshot attempts, the terminal diagnostic says:

> the OpenRig source changed during each of 5 snapshot attempts; nothing was ingested. Retry in a moment, or retry when the rig is quieter.

Run the same command again in a moment, or when the rig is quieter. This is a
manual new invocation; it does not guarantee that the next snapshot succeeds.
The five-attempt policy is unchanged. A non-OK private SQLite `quick_check` can
also exhaust this existing path; the diagnostic does not distinguish every
corruption case from source changes.

### Auxiliary output-lock source protection

Every analytics filename in one output directory shares `.dendro-output.lock`.
A preexisting regular lock with multiple hard-link names is refused before a
writable open or exclusion-lock attempt, including links to original main, WAL,
SHM or other regular source files. Nonregular and symlink locks remain refused.
Only a genuine missing-lock result permits creation; other metadata errors stop
the run with the existing path-free IO error class.

Opened, named and previously observed lock identities are checked before locking,
then held/named identity is checked again under exclusion before ingest. New safe
locks retain mode0600, existing single-link locks retain their permissions and
bytes, and other output names in the directory retain nonblocking busy behavior.
No lock is unlinked or replaced to bypass a holder. Static metadata checks do not
eliminate malicious namespace races; this protection covers the accepted
preexisting two-name regular hard-link condition. Prior primary-output safeguards
and the manual snapshot-retry policy remain in force.

## Handoff friction and rework vortex evidence (#12)

`report handoffs` and `report decisions` include `payload.handoff_friction` with
formula `handoff-friction-v1`. `report handoffs --friction` requests the same
field explicitly. Replay the metric with:

```bash
dendro report handoffs --db dendro.duckdb --friction --friction-return-rate 0.10 --friction-chain-returns 2 --friction-rework-tokens 1000000 --friction-loop-duration 600s
dendro trace --db dendro.duckdb --metric-id handoff_friction --friction-loop-duration 10m
```

These four threshold options apply to handoffs, decisions and trace. They use
CLI values only. Durations require an unsigned integer followed by `s` or `m`;
`600s` and `10m` normalize to 600 seconds. Rates must be finite in [0,1], and
count/token limits must be nonnegative integers. All comparisons are strictly
greater than the limit. Existing `--handoff-loop-returns` controls the separate
legacy metric. Friction options on unrelated report kinds are rejected before
opening the analytics database; `trace` has no `--friction` switch.

Pairs separate productive and recovery counts. Their new pickup median measures
receiver creation to first valid claim in milliseconds; execution measures that
claim to its uninterrupted successful terminal. Missing/multiple explicit
children and interrupted first claims carry invalid reasons. Legacy
`wait_median` values retain their existing definition. Sparse absent pairs imply
zero counts, null ratios/medians and no observed token data.

Returns and 2–4 edge closed witnesses use explicit parent links and matching
seat endpoints. Shared chain IDs or issue tags provide labels, never causal
links. Prewindow ancestors can establish returns; all edges of a loop must be
in the selected window. Forks, recovery gaps and unresolved conflicting lineage
break loop adjacency. Witnesses can overlap: A-B-A-B-A has three ping-pongs and
one quad cycle. Do not sum loop totals into pair or fleet totals.

Rework tokens are the four disjoint usage components assigned uniquely to
productive return receiver intervals `[claim, terminal)`, intersected with the
report window. Reasoning is already contained in output. Concurrent competing
runs exclude uncertain usage; no proportional allocation is made. Coverage is
`no_data` without relevant usage or activity, `ambiguous` for proven concurrent
competitors/conflicting evidence, `partial` for missing boundaries, otherwise
`complete`. An incomplete token lower bound over a limit proves a breach;
otherwise its status is null. Each threshold reason is reported separately.

Trace inputs retain sanitized accepted facts, per-source parent readings,
receiver endpoints, bindings and assigned/excluded usage records. Reports and
trace expose the same metric value. Coverage describes retained accepted
history: source conflicts discarded before storage, or IDs collapsed by
redaction, cannot be reconstructed. Handoffs/decisions and their trace refuse
malformed or overflowing persisted token totals with a fixed arithmetic error
before legacy arithmetic runs. The pure evaluator reports excluded invalid
components and names aggregate overflow with a null token total.
This capability is unreleased.

## Idle advisories, anomaly spotlights and CI verdicts

```bash
dendro report advisory --anomaly idle-claimed --advisory-container-hold-secs 900 --advisory-leaf-stall-secs 1800 --advisory-detached-target-secs 300
dendro report anomalies --format markdown --spotlight-ceremony-ratio 0.40
dendro report spotlights --format ndjson --spotlight-builder-seats dev-build-jr,dev-build-sr
dendro trace --metric-id advisory_idle_claimed
dendro trace --metric-id anomaly_spotlights
dendro issue-draft --metric m3 --usd-per-million 3
dendro assert --max-rework-rate 0.15 --max-idle-rate 0.20
```

Advisory intervals require an observed idle seat holding an in-progress claim.
Explicit recorded parent links distinguish unparked containers with active
children from leaf stalls; shared tags alone do not establish a child. Local
usage/tool evidence ends a continuous idle interval; a new idle observation is
required before counting another. Untimed or unbound local activity is explicit
partial coverage and suppresses unsupported claimed-idle findings. Lifecycle stops bound
observation. Detached-target findings require failed delivery metadata and
recorded stopped/unreachable target evidence. Nudge metadata is the latest
persisted snapshot, not a timed delivery-attempt history. Threshold equality
never triggers. Defaults are 900/1800/300 seconds for container/leaf/detached.

Spotlights A1–A6 consume canonical ceremony, concurrency, container/recovery,
compaction/cache, model-family and transport projections. High thresholds use
strict `>`; low parallelism and duty-cycle thresholds use strict `<`. Ratios and
parallelism overrides are finite; ratios are in [0,1]. Builder starvation needs
an explicit builder roster and observed peer saturation. Missing roster,
authoritative portfolio dispatch evidence is `no_data`, never an invented zero.
Routing effort accepts only the source-complete routing-only observed seat-window
producer, labelled explicitly as a window measurement rather than causal turn
attribution. Incomplete producer records remain unavailable. Canonical compaction events carry only producer-detected drops of at least
50% and 50,000 tokens; smaller ratio overrides are rejected before DB open.
Known evaluated negative results remain observed, distinct from missing data. A6's optional `supervisory_dispatch`
extension accepts source-backed portfolio classifications; it does not claim
hold-file ingestion. Drilldown argv preserves scope and all normalized operator
thresholds. These diagnostics suggest investigations and never operate agents.

JSON is the default. `--format markdown` provides the plain report section;
`--format ndjson` emits coverage and finding records. `issue-draft` writes local
GitHub-compatible Markdown to stdout. Financial annotations require explicit
pricing and the canonical global usage-key union, never a sum of pair amounts.
Published drafts use bounded source fingerprints instead of raw local locators;
annotations describe only observed-window attributed token lower bounds; no
monthly extrapolation or causal waste estimate is invented. Publication is an
explicit opt-in: `dendro issue-draft --metric m3 --publish --repo OWNER/REPO`.
This invokes installed `gh` with an argument vector and never changes agents.

`assert` emits JSON to stderr and exits 0 for a proven pass, 2 for an observed
threshold breach, or 3 when evidence is insufficient. Defaults are 0.15 rework
and 0.20 idle-with-work share (including recorded pending demand); these are operator policy limits from the
issue contract, not universal fleet health claims. Unknown role/activity
coverage cannot establish a pass. CI decides what to block; Dendro only emits
the verdict and its evidence.
