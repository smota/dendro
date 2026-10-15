#32 Phase 1 metadata-only file-source contract recorded. Please independently
accept/return criteria, then route only through governed handoff deliver.

Pass: https://github.com/smota/dendro/issues/32#issuecomment-6184029553
Spec: /home/sam/code/dendro/goals/file-source-state/phase-1.md
SHA-256: bee97f69956f11adb93f263aa5c073575019646317b5121fc4d3971bf58d69cd
Receipt: /home/sam/code/dendro/goals/file-source-state/phase-1-receipt.md
Read-back verifies phase1, analyst signer, allowedNext2/0 and body match.

Pins canonical file_source_states with safe shared trace refs; separates current
path availability, recorded metadata comparison and generation supersession.
Present with no baseline is baseline_unavailable, not content/ingest equivalence.
Changed needs comparable metadata; superseded requires actual newer committed
generation of same source_id. Gone is observed pathname absence, not global
content loss. Permission/metadata errors, unsafe symlink traversal and unresolvable/
redacted/out-of-scope paths get explicit unavailable/not_checked, never guesses.

Current schema has no full-size/mtime/inode baseline; committed byte_offset is
not file size (partial lines). Fresh normal ingest may capture honest metadata;
trace never rereads content/key, rehashes, writes/migrates/backfills or invents
history. Detached/missing-key/supported old DB remains usable. Same-size/mtime
content changes and races carry explicit metadata-only limits. File-state changes
cannot alter same-snapshot historical metric inputs/values or #33 replay/tie scope.

Synthetic current-entry probe at a1db495b: five no-ingest mutation/detachment
cases preserve full report/trace, 15 tokens/count3 loop and DB bytes, but all
states remain not_checked. Four Cargo gates pass; no product code/candidate proof.
Architecture owns metadata persistence/compatibility/assurance; Phase3 freezes
literal state vectors, observer/decoded privacy and expected annotation deltas.
No mandatory #13 dependency proven; return any concrete shared-model conflict
before expansion. No OpenRig retention/replay/lineage, detector/UI/metric change,
#12/#13 acceptance or publication. Prior acceptances stay candidate-bound.
