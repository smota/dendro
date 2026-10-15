# Custom harness connector SDK

A connector translates an explicitly authorized source into Dendro events or analytics facts. Start with a versioned export contract, not an assumed private harness storage layout. The runnable example at [`examples/custom_connector.rs`](../../examples/custom_connector.rs) implements the actual trait and uses the bounded reader in `dendro_connectors::sdk`.

## Run the example

From the workspace root:

```sh
cargo run --locked -p dendro-connectors --example custom_connector -- docs/extensions/fixtures target/custom-example.key
```

The second argument explicitly authorizes a Dendro analytics privacy-key file. The existing key provider creates it with mode 0600 if absent or validates that selected key file if present. It never reads harness credentials or prints key bytes. The included fixture is synthetic. Each `dendro-custom-v1` line declares a session ID, UTC timestamp, nonnegative input/output token **deltas**, and observed cost in USD. The example prints only counts and accepted byte offsets. It does not scan HOME, read authentication files, contact providers, or install services.

## Lifecycle contract

The source of truth is `crates/dendro-core/src/traits.rs`:

```rust
pub trait HarnessConnector: Send + Sync {
    fn name(&self) -> &'static str;
    fn discover_files(&self) -> anyhow::Result<Vec<std::path::PathBuf>>;
    fn extract_incremental(
        &self,
        path: &std::path::Path,
        watermark: Option<&dendro_core::Watermark>,
    ) -> anyhow::Result<(Vec<dendro_core::CanonicalEvent>, dendro_core::Watermark)>;
}
```

The trait currently carries `#[async_trait]`, but these methods are synchronous. There are no trait methods named `discover`, `stream_events`, `extract_usage`, or `extract_tools`; those are lifecycle responsibilities. Discovery maps to `discover_files`. Streaming, usage projection, and tool projection happen inside `extract_incremental` or a connector's richer batch API.

1. Define a stable harness name and require an explicit export directory. Missing optional roots return an empty discovery list.
2. Implement discovery using `sdk::ExportRoot::discover`. It selects direct regular `.jsonl` children deterministically, skips symlinks, and never recurses through configuration directories.
3. Implement extraction using `ExportRoot::read_batch`. It seeks to the accepted offset, returns at most 128 complete lines, bounds each line to 1 MiB, and retains the unfinished final line for the next call. A record may contain an array: bound its processing within your decoder too.
4. Validate the schema/version before projecting fields. Reject unknown shapes with a constant diagnostic; do not include raw JSON or parser errors containing transcript text. A malformed complete line produces a static warning in the reader; choose explicitly whether your contract refuses the batch or accepts a documented gap.
5. Extract usage only when the source actually supplies counters. Determine whether they are cumulative or per-turn, account for cache inclusion, and never add both a total and its components. Missing counters and prices mean unknown, not zero.
6. Extract structured tool names and known outcomes. Do not retain tool arguments, shell text, prompts, file contents, arbitrary labels, or transcript metadata. Use allowlisted names and keyed `dendro_core::opaque_id` for analytics identifiers. The example also uses a keyed opaque ID through the explicitly selected analytics key provider.
7. Commit facts and their watermark together. Advance the caller's watermark only after durable success. Repeat extraction until the accepted offset stops advancing.

## Watermarks and bounded memory

The SDK's `Watermark.file_path` is an opaque path digest. `session_id` holds a versioned prefix digest, not a native session ID. Persist the complete watermark unchanged. Before resuming, the reader hashes the accepted prefix in a fixed-size buffer and refuses truncation or replacement of accepted bytes. Prefix validation costs O(accepted bytes), but only the appended suffix is JSON-decoded. No entire session is loaded into memory. Each bounded batch revalidates the entire accepted prefix, so many-batch ingestion can require quadratic total hashing work; large-file throughput remains unproven. Resetting a changed source is an explicit consumer decision; do not silently append its replacement to the old generation.

The reader borrows buffers while reading, then materializes JSON for one bounded batch. Do not claim zero-copy JSON deserialization: `serde_json::Value` allocates. For higher volume, project a typed borrowed record inside a bounded line reader and release it before reusing the buffer. Keep output batches bounded and reuse database connections and prepared statements.

## Storage and privacy

`AnalyticsStore::commit_file_batch` atomically writes `UsageFact`/`ToolFact` and source state using the existing transaction mechanism. For rich coverage state use `commit_file_batch_reading`. The current implementation stores these offsets in `source_files` (the watermark role called `_dendro_watermarks` in engineering policy); avoid a second independent cursor. `commit_file_batch_observed` records verified source metadata and observation timestamps in the same transaction. Use parameterized SQL exclusively. No database write belongs in a per-record transaction loop.

The legacy `CanonicalSession` has required counters and cost and the legacy `CanonicalToolCall` has a required timestamp. These types cannot accurately represent every export's missing fields. A rich connector should expose a batch of facts with optional timestamps and absent usage rows, and refuse an unrepresentable legacy projection instead of fabricating measurements. The current default `run`/`watch` path uses `ingest_staged`, not an automatic plugin registry. Adding a public Rust connector alone does not register it in those commands; document that boundary and test any separate integration.

Redact sensitive values with `dendro_core::redact` before persisting any permitted free-text diagnostic. Prefer constant reason codes. Never serialize the `JsonlBatch.records` values to reports or storage. Source exports can contain secrets even when extensions and shapes look safe.

## Read-only access and concurrency

The reader uses read-only file handles, never locks the producer, and validates file metadata before returning. Metadata verification currently requires Unix; unsupported platforms refuse the read. Files that change during a batch are refused for retry. Use a dedicated export directory; only its direct regular files are supported. Canonical containment and symlink checks protect ordinary traversal, but portable pathname checks do not provide a sandbox against a hostile process racing directory replacement. Do not use a directory writable by an adversarial local account.

For SQLite, use a coherent read-only snapshot of the database and its WAL through the existing snapshot implementation; never use `immutable=1` on a live writer or modify source journal modes. Read transactions can still contend with writers; do not promise zero lock contention for arbitrary SQLite sources. SDK JSONL exports avoid SQLite locking entirely.

Run blocking extraction and database work in `tokio::task::spawn_blocking` when called from Tokio. Use bounded channels for backpressure and debounce watcher events for 1.5–2 seconds. The synchronous example needs no runtime dependency.

## Conformance checks

Test synthetic fixtures against the pinned public schema: first import, unchanged rerun, append, partial final line, malformed complete line, invalid numeric fields, unknown schema, oversized record, root escape, symlink, and replaced/truncated prefix. Check stored facts and source offsets, not just event counts. Check that prompts, arguments, fake token markers, and source labels never appear in facts or diagnostics.

```sh
cargo check --locked -p dendro-connectors --examples
cargo test --locked -p dendro-connectors
cargo clippy --locked -p dendro-connectors --all-targets -- -D warnings
cargo fmt --check
```

See [verified export compatibility](formats.md) for editor and autonomous harness boundaries.
