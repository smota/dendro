# #32 Phase 1 analyst receipt

2026-10-15 UTC, pm.analyst / pm-analyst@dendro-pm, Codex / codex-cli /
local-cli / current-session. Claimed qitem-20261015034748-5ef4cbb5.

Pass: https://github.com/smota/dendro/issues/32#issuecomment-6184029553
Spec: goals/file-source-state/phase-1.md
SHA-256: bee97f69956f11adb93f263aa5c073575019646317b5121fc4d3971bf58d69cd
Key: 32-phase1-metadata-file-source-state.
Read-back confirms currentPhase1, allowedNext2/0, verified analyst signer and
body match except reader-stripped final newline; phase-1-readback.json.

Read current Phase0/steering brief, source-state schema/commit/read and #33
trace model. Actual entry a1db495b5d131112c6f179e465b8e53411e65480, debug binary
SHA-256 53dfa2d21c405a9d234ef960149352d94c3cf157b7100c1db63ba2b8233f5d17.
Current source_files has path/generation/committed offset/prefix, not full-size/
mtime/device/inode stat baseline. Offset is not full file size when a partial
trailing line exists. Do not infer missing old metadata from that offset/hash.

Public synthetic mixed Claude/Codex/Pi probe: 15 tokens and one count3 Codex
loop remain exactly unchanged, as do full report/trace and analytics bytes,
after append, truncate, same-size/mtime new-inode replacement, deletion and
finally source-tree detachment plus removal of only its synthetic key, without
ingest. Exported source states all remain not_checked. Current gap and historical
invariant reproduced, not candidate annotation proof. Source mutations are
explicit fixture setup between commands, not attributed to Dendro. No owner
source/key content accessed. Construction/argv/results: baseline-probe.py and
baseline-probe/summary.json with public output siblings.

Contract: canonical inputs.file_source_states and safe shared refs. Separate
path availability, metadata-v1 comparison and persisted generation relation;
summary precedence explicit. Superseded requires newer committed generation
of same actual source_id, not filename/session/metadata lineage guesses.
Changed requires comparable recorded baseline; old supported DB can report
present/baseline_unavailable or established gone without migration/key/source
content read. Gone means pathname absence only. Permission/errors/symlinks and
redacted/unresolvable/out-of-scope paths never become guessed absence/presence.
Metadata equality cannot prove content equality; dev/inode distinguishes some
same-size/mtime replacements. No stat-derived generation or historical revision.

Fresh normal ingest may capture honest generation-associated metadata; trace
never captures it by hidden ingest or writes. Architecture owns persistence/
compatibility/assurance and lossless representation. All metric inputs/values,
#33 captured replay/tie limit and source/key safety preserved; file observations
are timed annotations, not metric filters. Independent QA freezes literal state
vectors and payload-free source-operation/privacy proof before build.

No concrete mandatory #13 dependency established: file-only annotations and
recorded generations suffice at contract level. Any actual shared-interface
conflict returns to product before OpenRig retention/replay/lineage expansion.
Candidate design, baseline persistence, full platform/permission/race behavior
and performance remain unknown. No new detector or source-content classifier.

All four required current-workspace Cargo gates pass: cargo check --workspace,
cargo test --workspace, cargo clippy --workspace -- -D warnings, cargo fmt --check.
Logs/exits: phase-1-checks.json and phase-1-cargo-*.log. These are entry workspace
checks, not candidate QA/review. Product code/crates unchanged; all tenure-
authored files are within this goal. No commit, direct delivery message,
owner credential access, integration or publication.

Next owner pm.manager independently accepts/returns criteria, then only governed
handoff deliver moves accepted Phase1 into available delivery. Architecture owns
solution/assurance; QA5/review6 candidate gates remain. Localv0.2.0 only, no
#12/#13 acceptance, main/push/PR/merge/tag/integration/publication authority.
Prior accepted candidates, including #33 b975e1c, are not this entry or #32 proof.

Durable custody: qitem-20261015034748-5ef4cbb5 closed as handed_off_to
pm-manager@dendro-pm; successor qitem-20261015035610-c0e37540.
