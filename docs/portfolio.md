# Read-only fleet portfolio

The portfolio API observes independently ingested analytics databases. It never combines multiple OpenRig roots in one output, acquires an ingest lock across projects, modifies schedulers, or releases holds. Keep the existing ingest safety guard: build each analytics output from its authorized root, then map the existing outputs to rigs explicitly.

`dendro_core::portfolio` exposes:

```rust,ignore
PortfolioSource { db: PathBuf, rigs: Vec<String>, hold_dir: Option<PathBuf> }
portfolio_json(&[PortfolioSource], from: &str, to: &str) -> anyhow::Result<Value>
portfolio_ndjson(&Value) -> anyhow::Result<String>
portfolio_table(&Value) -> anyhow::Result<String>
```

Inputs are existing analytics outputs and optional explicit direct-child hold directories. `PortfolioSource` supports JSON deserialization and rejects unknown fields. A single source can map several rigs when its database already contains them. Multiple sources can represent independent projects. Rig names cannot be assigned twice; canonical database aliases and duplicate recorded OpenRig origins across separate outputs are refused, preventing duplicate snapshots from inflating the matrix. No source or credential directory is scanned implicitly. Secret-shaped selected database and hold paths, including canonical resolved paths, are refused before their contents are opened.

The JSON schema is `dendro-portfolio-v1`. Each `payload.rigs` row embeds the exact canonical `report_json(..., "decisions", Some(rig), from, to)` result. Summary columns copy its token count, ceremony ratio, effective parallelism, return rate, rework rate, per-seat throughput, and model/effort attribution. Missing values remain missing. A rig with no scoped usage is marked `no_data`, even if it has other observed coordination evidence.

`payload.source_census` supplies opaque output identity, selected rigs, core schema version, and read start/end times. These are independent read snapshots per database and canonical report, **not an atomic global snapshot**. Reproducible comparisons should use independently frozen analytics copies from the same capture period. The portfolio reader does not reopen original OpenRig source contents.

## Coordination matrix and supervisory evidence

The matrix uses each database's unscoped canonical `ring_boundaries.edges` exactly once. Per-rig report duplication cannot duplicate seams. Every edge retains its canonical source key and opaque database source identity. Edges from genuinely independent origins remain distinct even if local sequence keys happen to coincide.

Membership and role evidence stay in the originating database namespace. A similarly named seat in another database cannot supply missing membership or upgrade an unknown role. Selection is also source-specific: selecting `worker` in one database never admits an unselected `worker` in another. An edge is excluded as unrelated only when both endpoints have proven unselected memberships; unresolved endpoints remain counted `no_data` coverage. Missing edge keys are counted explicitly in `matrix.coverage.skipped_missing_keys` and `counts.no_data`, and prevent complete supervisory coverage status. Classification requires recorded identities: matching rig membership is `in_ring`; different proven memberships are `cross_rig_seam`; an endpoint with explicit supervisory role metadata is `supervisory_plane`. Missing or conflicting memberships remain `no_data`. The canonical boundary label is also retained, so the portfolio refinement can be audited.

Supervisor, builder, and orchestrator roles come from recorded snapshot role evidence (`agentflow-seat:steer.lead`, `dev.build*`, `orch.arch`, or explicitly mapped resolved agent specs). A seat name containing `kernel`, `operator`, or `build` does not prove a role. Unknown roles do not produce a bypass finding. Every unrecognized explicit role observation remains an unknown observation, so a known-plus-unknown conflict cannot become a falsely unique role.

`payload.supervisory_dispatch` is an array for advisory consumers:

```json
{
  "source_key": "canonical source key",
  "source_seat": "recorded sender",
  "dest_seat": "recorded receiver",
  "source_rig": "recorded source rig",
  "dest_rig": "recorded destination rig",
  "classification": "proven_bypass",
  "evidence": ["canonical edge key", "recorded role evidence keys"]
}
```

A `proven_bypass` requires explicit supervisory-to-builder roles and proven memberships. A `dead_supervisory_relay` requires a proven supervisory destination, the canonical edge's observed failed receiver wake, and a linked receiver with a latest accepted pending queue state at the report window end. A failed wake alone is insufficient; it remains separately inspectable in matrix evidence. This is a retained delivery observation, not a claim about present terminal attachment or liveness. Other failed cross-rig wakes remain separately inspectable. `supervisory_dispatch_status` is `observed` only for a nonempty fully evaluated edge set with proven roles and memberships and no skipped required keys, even if the finding array is empty. Findings can remain inspectable when other edges leave coverage incomplete; the overall status then stays `no_data`. A relay observation always carries its proven `pending_duration_secs`. That age is clipped to the window and is not a failure duration.

## Admission hold snapshots

The supported primary schema is the AgentFlow handoff record written by [`handoff.mjs`](https://github.com/smota/agentflow-sdlc/blob/f641c567eb562dc7d906fc82d71fdfc262eb1126/adapters/openrig/agentflow-product/handoff.mjs): `id`, `state` (`held` or `delivered`), `rig`, `target`, `sender`, `createdAt`, `signals` objects with an `id`, and optional `deliveredAt`. The source's [hold tests](https://github.com/smota/agentflow-sdlc/blob/f641c567eb562dc7d906fc82d71fdfc262eb1126/scripts/__tests__/handoff-hold.test.mjs) establish B1 (in-progress delivery item), B2 (pending delivery item), B3 (unfinished goal), and B4 (working/running delivery seat).

Only B1–B4 signal identifiers are retained. Body, goal URI, transition URI, signal detail, source filenames, and arbitrary metadata are omitted. Canonical identifier redaction applies to the permitted sender, target, and rig labels; hold IDs and file evidence keys are opaque hashes. Source keys let a caller trace a selected snapshot without exposing its contents in the report.

The reader opens at most 4096 direct regular JSON children, each bounded to 1 MiB, with projected identifier fields limited to 256 bytes. Symlinks are skipped or refused, canonical containment is checked, and opened file identity is compared before and after reading. Malformed or unsupported records produce constant warnings. Oversize files and changing sources refuse the operation. Missing optional hold directories produce `no_data` with a warning. The reader does not follow arbitrary references in a hold record or mutate a source. These pathname checks do not create a sandbox against an adversarial local process racing directory replacement; select a trusted read-only snapshot directory.

Delivered records intersecting the half-open report window carry clipped `completed_elapsed_secs`. Held records are selected only when created in the report window, with `observed_record_age_secs` clipped at the window end. Held records created before the window are deliberately excluded, so this is not an exhaustive census of outstanding backlog. Future creation or delivery endpoints beyond the reader observation time are rejected. Neither value proves continuous waiting under any particular gate. The heatmap attributes **record snapshots and endpoint elapsed time** to their observed codes; a record with two codes appears in both code cells, so do not sum those cells as distinct holds or distinct waiting seconds. Current signals are not a historical signal-residence trace. A file claiming `held` is labelled `observed_record_not_live_verification` and never treated as verified current scheduler state.

Duplicate copied hold IDs within an origin are not counted twice; all coalesced evidence keys remain in each affected heatmap cell. Conflicting semantic snapshots, including a held copy beside a delivered copy, are excluded with a warning rather than choosing a state. Only target rigs explicitly mapped for the selected database are included. Sender-rig attribution resolves against that database's recorded membership before export-label redaction, or remains unknown. A raw hold sender cannot be joined to an already privacy-sealed canonical seat without the original key; the read-only portfolio reader never reads a key or guesses a join from a redacted display.

## Output and verification

The table has one row per rig. NDJSON emits one complete rig object per line plus one global evidence object containing the matrix, supervisory observations, holds, source census, and warnings. JSON keeps the same information in a single object. No observation authorizes a mutation or a policy decision.

Synthetic tests exercise exact canonical per-rig parity, multi-database and multi-rig mapping, duplicate output/origin refusal, namespace isolation, missing roles and memberships, proven supervisory edges and failed relay observations, native hold shape, privacy projection, window filtering, symlink refusal, and JSON/NDJSON/table output:

```sh
cargo test --locked -p dendro-core --test portfolio
```

These fixtures are synthetic examples derived from the source schema, not owner runtime captures or evidence of current live holds.
