# Verified editor and autonomous harness exports

These are explicit **export** imports. Select an authorized, dedicated directory with `dendro run --root cursor=/absolute/export-directory` (or `windsurf`, `goose`, `devin`). Only direct regular `.jsonl` children are read. Missing optional roots remain nonfatal. The default roots are unchanged; none of these adapters scans owner credentials, private IDE databases, or remote accounts.

All four adapters implement `HarnessConnector`. Their richer `read_batch` API returns analytics facts with optional timestamps and no usage rows when counters are absent. `ingest_file` commits bounded fact batches and source watermarks atomically. The normal staged ingestion router uses the same decoder and supports sealed prefix capture/replay. Appends beyond a sealed cutoff are excluded; modified accepted bytes are refused.

| Root | Accepted format | Usage coverage | Tool coverage | Explicit limits |
| --- | --- | --- | --- | --- |
| `cursor` | Cursor CLI `--print --output-format stream-json` NDJSON | Unknown: no counters in the documented export | Completed `tool_call` records; started events excluded | Private Cursor SQLite/projects storage unsupported; timestamps absent unless a supplied `timestamp_ms` exists |
| `windsurf` | Cascade transcript steps or recorded post-hook input, one JSON object per line | Unknown: documented formats expose no counters | `code_action` with `done`; `post_read_code`, `post_write_code`, `post_run_command`, `post_mcp_tool_use` | Private trajectory/protobuf storage unsupported; other step types warn; transcript paths never followed |
| `goose` | Legacy Goose JSONL: metadata header then messages | Header `accumulated_input_tokens`/`accumulated_output_tokens`, falling back to current header counters when accumulated fields are absent | Structured `toolRequest` blocks | Modern `sessions.db`, context-window reconstruction, and tool outcome correlation unsupported; header-only usage is a snapshot |
| `devin` | One compact ATIF-v1.7 trajectory per `.jsonl` file, selected from Devin CLI `--export` output | Agent step `metrics.prompt_tokens`, `completion_tokens`, optional `cost`; final totals ignored | Step `tool_calls` | Other ATIF versions and Devin Cloud API/webhooks unsupported; compatibility requires the exported document actually declare `ATIF-v1.7` |

A refused, corrupt, oversized, unreadable, or changed export fails the entire staged run, including other selected harness roots. This is deliberate fail-closed behavior: the staged pipeline does not publish a partially accepted output. Fix or isolate the refused source and retry.

Unsupported schemas refuse ingestion or emit constant `unsupported-*` warnings for unknown record kinds; they never silently infer a private format. Warning notes distinguish unknown usage from measured zero. An empty usage table is **no data**, not proof of zero consumption. Do not infer zero spend from the absence of a cost field. Input token cache components are not independently attributed in these adapters; ATIF records with nonzero `cached_tokens` are refused because prompt/cache inclusion semantics are unproven; absent or explicitly zero cached counts are accepted. Goose accumulated input/output fields are used exactly as provided, with no cache accounting claims.

Tool names are projected to a small fixed vocabulary (`read_file`, `write_file`, `edit_file`, `shell`, `mcp_tool`, `other`). Unknown provider labels become `other`; they are never retained as arbitrary transcript text. Arguments, commands, prompts, responses, paths, descriptions, model labels, and native session names are omitted. Identifiers use keyed opaque hashes. Post-hook inputs without outcome fields retain unknown outcomes. Transcript `code_action` with explicit `status: done` is recorded as completed. Other unknown outcomes stay unknown. Timestamp-free events remain in the fact store with a null timestamp and cannot be attributed to dated report windows.

The older `CanonicalSession` requires cost and time, and `CanonicalToolCall` requires time. Consequently `extract_incremental` refuses a batch with facts that cannot be represented accurately. Use `read_batch`, `ingest_file`, or normal `dendro run --root` ingestion for these exports. It does not fill missing values with invented timestamps or prices.

## Preparing exports

For Cursor, capture the documented CLI stdout as `.jsonl` in a dedicated export directory. Do not combine separate runs by concatenating unrelated sessions into one mutable file. Cursor tool start/completion correlation is represented by completed records only; repeated completed records in the source are distinct observations.

For Cascade, copy the transcript named by the documented `post_cascade_response_with_transcript` hook into a dedicated import directory, or record the post-hook JSON input as one object per line. The adapter never executes a hook or follows a `transcript_path`. A transcript may be rewritten by its producer; import an immutable snapshot under a distinct filename instead of changing an accepted prefix. Transcript identity is derived from the canonical export pathname, so preserve that name across reruns. It does not correlate distinct hook and transcript copies; importing both can count the same underlying operation twice.

For Goose, explicitly select legacy session JSONL copies. The public source currently migrates these to SQLite. This adapter does not claim compatibility with current SQLite storage. The first record must contain `working_dir` and `message_count`. Header token totals are accepted once, not added again for every later message. If the producer rewrites its metadata header, the SDK refuses the changed prefix; use an immutable snapshot and a fresh analytics output to replace a prior snapshot rather than adding both as independent sessions. Legacy tool requests are counted with unknown outcome; responses are not counted as additional tool calls.

For Devin, the official CLI documents `--export [PATH]` as ATIF output after each turn. It does **not** pin an ATIF schema version; this adapter pins **v1.7**, and refuses all other versions. Inspect the exported document's `schema_version` before selecting it. Do not relabel an older document to bypass validation. A pretty-printed JSON export can be compacted without changing its schema:

```sh
python3 - path/to/authorized-devin-export.json export-directory/session.jsonl <<'PY'
import json, sys
with open(sys.argv[1], encoding="utf-8") as source:
    value = json.load(source)
if value.get("schema_version") != "ATIF-v1.7":
    raise SystemExit("unsupported ATIF version")
with open(sys.argv[2], "x", encoding="utf-8") as destination:
    destination.write(json.dumps(value, separators=(",", ":")) + "\n")
PY
```

The transformation is operator preparation, not a claim that Devin natively emits JSONL. Each accepted trajectory is at most 1 MiB and 4096 steps, with at most 4096 tool calls per step. Larger exports are explicitly refused. Only agent-step metrics are counted; copied context (`is_copied_context`) is currently unsupported and refused to avoid double counting. No remote authenticated API is called.

## Primary sources and test provenance

Format verification date: 2026-10-15. Documentation pages can evolve; the tests pin only the shapes below. All committed fixtures under `crates/dendro-connectors/tests/fixtures/exports/` are **synthetic schema-derived fixtures**, not captures from an installed harness or evidence of a particular installed version.

- Cursor: [official CLI output format](https://cursor.com/docs/cli/reference/output-format), unversioned public documentation. `cursor.jsonl` tests system initialization, started/completed tools, result, and omitted usage/timestamps. A supplied `timestamp_ms` is optional and never synthesized.
- Windsurf/Cascade: [official Cascade hooks](https://docs.windsurf.com/windsurf/cascade/hooks), currently redirects to [Devin Desktop Cascade hooks](https://docs.devin.ai/desktop/cascade/hooks). Public transcript example shows `type`, `status`, and `code_action`; post-hooks document `agent_action_name` and `tool_info`. `windsurf.jsonl` covers these shapes. No broader Cascade token-buffer schema is asserted.
- Goose: [legacy session reader](https://github.com/aaif-goose/goose/blob/0df818175bc854229746c81f01ea154b1c6c5e9e/crates/goose/src/session/legacy.rs), [message schema](https://github.com/aaif-goose/goose/blob/0df818175bc854229746c81f01ea154b1c6c5e9e/crates/goose-provider-types/src/conversation/message.rs), and [tool result serialization](https://github.com/aaif-goose/goose/blob/0df818175bc854229746c81f01ea154b1c6c5e9e/crates/goose-provider-types/src/conversation/tool_result_serde.rs). `goose.jsonl` tests the retained legacy import format, accumulated counters, and `toolCall.status/value` shape.
- Devin: [official CLI commands and flags](https://docs.devin.ai/cli/reference/commands.md) (`--export` ATIF contract; the legacy [reference URL](https://docs.devin.ai/cli/reference.md) also serves it), plus [ATIF-v1.7 schema](https://github.com/neulab/agent-data-protocol/blob/040a279b46b2388ae42b43449f8645b9781c7bf7/schema/atif.py). `devin.jsonl` tests the explicitly selected schema subset, not every version of Devin's exporter or the separate Cloud API.

The conformance suite checks exact byte offsets, partial and malformed records, bounded batches/lines, source mutation/truncation, symlink containment, repeated imports, append-only tools, privacy projection, missing roots, schema refusals, staged routing, and sealed replay. Run `cargo test --locked -p dendro-connectors --test export_connectors` from the workspace root.
