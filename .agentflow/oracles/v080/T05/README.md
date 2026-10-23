# T05 draft expected values

Status: draft for review. This directory is not frozen, not reviewed, and not admitted.

The original draft and receipt were written by Grok. This revision is a distinct Codex oracle amendment under the user's temporary Codex execution override. The original three files are preserved byte for byte under `.agentflow/runs/v080/T05/oracle-amendment/original-grok-draft/`. The original `author-receipt.json` remains unchanged; the amendment has its own hash receipt. Independent review is still required before any freeze.

Task: capture contribution registry and bounded SDK. Acceptance ids: `T05-A1` and `T05-A2`. The vectors live in `draft-expected.json`. Builders can implement tests against that file. They do not change it.

## What is pinned

The hostile ceiling is 8,388,608 encoded bytes per logical JSONL record, nesting depth 64, and 1,024 decoded records per batch. A descriptor may equal those numbers or be tighter. A looser descriptor is refused with `descriptor_looser_than_global_ceiling`.

One MiB is 1,048,576 bytes because `MAX_LINE_BYTES` in `crates/dendro-connectors/src/sdk.rs` is `1024 * 1024`. The same basis fixes 8 MiB and 1.5 MiB (1,572,864). Encoded length includes the complete terminating LF or CRLF. Other formats must declare their framing and record boundaries. Depth counts containers: a scalar is depth 0, an empty array or object is depth 1, and each containing array or object adds one to its deepest child value. Object keys add no depth.

Cursor, Windsurf, Goose, and Devin keep the existing export bounds: 1,048,576 bytes and 128 line attempts. AGY keeps 1,048,576 bytes and 128 decoded records. Those limits are tighter than the global ceiling. A 1.5 MiB record is over those adapter limits. The same size is still not a valid Claude, Codex, Pi, OpenRig, or Grok record, because no pinned descriptor admits it. A synthetic conformance descriptor that explicitly sets 2,097,152 bytes may accept that size. That descriptor is not a producer fact.

`recognized = projected + recognized_unprojected`. Decoded means successfully decoded JSON, including recognized unprojected and unsupported-major records. On the first unsupported major, stop further reading and projection for that source. Count that record once as decoded and unsupported, leave later records UNOBSERVED, and hold the durable watermark at the last safely projected record so a future adapter can replay. A raw read position is separate from the committed watermark. Quarantines, malformed lines, failed resumes, blocked sources, and a missing root are not observed zero. A complete empty admitted read can establish zero captured or projected records in that scope; it does not establish zero model usage, activity, cost, or security events.

The third-party example is compile-time `dendro.capture.contribution/v1`, source `example.capture.synthetic`, limits 1,048,576 / depth 8 / 16 records. Contract majors other than v1, a dynamic loader, a missing fixture digest, and limits above the global ceiling are refused. The descriptor does not sandbox the adapter and does not change core fact types. No runtime plugin is claimed.

Diagnostics are the closed strings in `draft-expected.json`. The canary `sk-SYNTHETIC-T05-CANARY` must not appear in persisted output.

## Fixture bytes

Recipes are generative. SHA-256 values in `draft-expected.json` were computed for those exact UTF-8 bytes, including the final LF where the recipe includes it. Session ids of the form `sdk-v1:<64 hex>` are SHA-256 of the consumed prefix only. The watermark `file_path` hash depends on the canonical path and is not a fixed expected value.

Proven export cases kept as-is:

- `{}` plus LF, then `not-json` plus LF, then an unterminated `{"tail":` leaves `byte_offset` 12 and warning `malformed-json-record`.
- 128 copies of `{}` plus LF stop at offset 384.
- 1,048,577 bytes of `x` with no LF returns `export line exceeds 1 MiB; watermark unchanged`.
- A shorter file than the watermark returns `export truncated; reset source explicitly`.
- A changed prefix returns `export prefix changed; reset source explicitly`.
- A watermark from another file returns `watermark belongs to another export`.

## Not claimed

`cargo test -p dendro-connectors --test registry_v080 --locked` was not run. The B01-G source contract is admitted only for the two exact sanitized fixture digests in its profile; native Grok parser T08 remains pending. Linux strace and Windows ETW remain blocked pending real profiles and controlled specimens. AGY and environment JSON key names are not specified here. Collector bounds of 64 processes, 600 samples, and a 1 second default period are preserved as limits only.

Open items are listed in `open_ambiguities` in `draft-expected.json`.
