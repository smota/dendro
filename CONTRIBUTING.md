# Contributing to Dendro

Welcome to **Dendro**! We are building a local-first AI Fleet Intelligence engine for multi-agent coding teams and autonomous developer tools. We are excited to collaborate with engineers, researchers, and open-source contributors.

---

## Getting Started with Development

You can build and run Dendro locally in two ways:

### Path A: Build and Run from Git Checkout (Standard for Development)

```sh
# 1. Clone the repository
git clone https://github.com/smota/dendro.git
cd dendro

# 2. Build the workspace
cargo build --release --locked

# 3. Run the CLI
./target/release/dendro status
```

### Path B: Install from Git via Cargo (`dendro` on `PATH`)

```sh
cargo install --git https://github.com/smota/dendro.git dendro-cli
dendro status
```

*(From a local checkout: `cargo install --path crates/dendro-cli --locked`)*

---

## Where to Contribute

1. **AI Coding Connectors & Export Adapters:**
   - Add native support for emerging AI coding assistants or editor formats.
   - Read the [**Extensibility & Bring-Your-Own-Component Guide**](docs/extensions/README.md).
   - Review the [**Verified Export Compatibility Matrix**](docs/extensions/formats.md) for Cursor, Windsurf, Goose, and Devin (`ATIF-v1.7`).
2. **Analytical Insights & DuckDB Views:**
   - Implement new diagnostic rules (`I01`–`I25`) in SQL/DuckDB.
   - Explore `crates/dendro-core/src/analytics/` and `templates/nao/dashboards/`.
3. **Connected Intelligence Studio (Web UI):**
   - Enhance the Apple-grade interactive visualizations in `site/dashboards/` (SVG Growth Rings, 3D Stack Explode, Canvas attractors).
   - Test UI components with `node --test site/site.test.cjs`.
4. **Maintainer & Release Operations:**
   - Review [**Maintainers Guide**](docs/maintainers/README.md) for release workflows and policy governance.

---

## Core Engineering Standards

Dendro enforces strict performance and safety guarantees:

- **Safe Rust Only:** `#![deny(unsafe_code)]` across all workspace crates. No `unsafe` without an audited, explicit rationale.
- **Zero-Copy & Streaming I/O:** Stream JSONL files line-by-line using buffered readers (`BufReader`). Never load gigabyte session files directly into RAM.
- **SQL Injection Prevention:** Parameterized queries only (`duckdb::params![...]`, `rusqlite::params![...]`). Never interpolate strings into SQL.
- **Strict Privacy Sanitization:** Redact auth headers and API token patterns (`sk-`, `ghp_`, `gho_`, `xai-`, `Bearer`) before writing to DuckDB. Never persist private conversation prompts or credentials.
- **Watermark Conformance:** Maintain incremental byte-offset cursors in `_dendro_watermarks` and verify prefix digests before consuming appended data.

---

## Quality Gates & Verification

Before opening a pull request, all checks must pass cleanly:

```sh
# 1. Workspace compilation (including examples)
cargo check --locked --workspace --examples

# 2. Complete test suite
cargo test --locked --workspace

# 3. Clippy linter (zero warnings policy)
cargo clippy --locked --workspace --all-targets -- -D warnings

# 4. Code formatting
cargo fmt --check

# 5. Dashboard web test suite
node --test site/site.test.cjs
```

---

## Submitting a Pull Request

1. Create a descriptive feature branch: `git checkout -b feature/my-new-connector`.
2. Keep commits atomic with clear, imperative commit messages.
3. Verify that all 5 quality gates above pass with zero warnings.
4. Open a pull request against `main`. All pull requests are reviewed under governed autonomous delivery standards.
