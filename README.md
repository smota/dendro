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

## Start locally

Install stable Rust and Git, then:

```sh
git clone https://github.com/smota/dendro.git
cd dendro
cargo build --release --locked
./target/release/dendro --help
```

Choose authorized source roots explicitly. Keep the output database outside those roots:

```sh
./target/release/dendro run --db ./dendro.duckdb \
  --root openrig=/absolute/path/to/openrig \
  --root codex=/absolute/path/to/codex
./target/release/dendro report decisions --db ./dendro.duckdb \
  --from 2026-10-14T00:00:00Z --to 2026-10-15T00:00:00Z
./target/release/dendro trace --db ./dendro.duckdb \
  --metric-id handoff_friction \
  --from 2026-10-14T00:00:00Z --to 2026-10-15T00:00:00Z
```

Dates above are examples; choose the actual period recorded by your sources. Windows are half-open `[from, to)`. An empty report may mean absent timestamps, unsupported data, or an unobserved period. A build from source is not a measured five-minute installation guarantee.

## Local investigation

Open [the offline explorer](site/viewer.html) and load a report or trace JSON file. The viewer aliases seat labels and displays supported numeric facts. Its synthetic demo is labelled. Aggregate concurrency counts are a distribution, not a reconstructed time series; handoff counts do not establish token transfer.

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

- [Operator guide](docs/adoption/README.md): source selection, reporting, privacy and reproducible replay.
- [Maintainer guide](docs/maintainers/README.md): workspace architecture and quality gates.
- [Connector extension guide](docs/extensions/README.md): trait contract and runnable example.
- [Product specification](docs/PRODUCT_SPECIFICATION.md) and [R1 acceptance contracts](docs/ACCEPTANCE_R1.md).
- [Historical narrated walkthrough](docs/assets/dendro-demo.mp4) and [preview animation](docs/assets/dendro-preview.gif). These are presentation assets, not evidence for current CLI behavior or performance.

## Privacy and license

Source logs are read-only. The analytics store uses allowlisted facts, redaction, opaque identifiers and explicit uncertainty. Never select credential files as ingestion roots. Exporting or publishing a report is a separate operator action; local storage alone does not make every possible export safe to share.

MIT License. Copyright 2026 Samuel Mota.
