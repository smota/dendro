# #34 Phase 1 analyst receipt

2026-10-14, pm.analyst / pm-analyst@dendro-pm, Codex / codex-cli /
local-cli / current-session. Claimed qitem-20261014204636-ad6202d8.
Read Phase 0 and its steering brief, R1 duration/median/trace contract, AC4
conservation disposition, clarification register and G2 same-build dependency.

Phase 1 pass: https://github.com/smota/dendro/issues/34#issuecomment-6178194115
Key: 34-phase1-fractional-wait-contract.
Specification: goals/subsecond-wait-times/phase-1.md
SHA-256: bc1b32bbcfad446945bacc4054990511752a982e0e11b0d8dc5731627655bc08
Live read-back: currentPhase 1, allowedNext 2/0, verified pm.analyst signer;
body matches after the phase reader removes the final newline.
Read-back artifact: phase-1-readback.json.

Current-behavior reproduction, source HEAD 13423047ba4f4094e49a9267fcf2278be0da43ab:
built dendro-cli and generated a synthetic OpenRig source, ran normal ingest,
report handoffs and trace medians with identical rig/window inputs. 24.734
seconds becomes 24; fractional odd/even examples become zero. Missing,
unparseable and pre-handoff claims remain invalid with no samples. Zero is a
valid zero. Microsecond timestamp survives in public trace, 999 ns does not.
Report and trace pair-median arrays match.
Binary SHA-256: b3ba5cd2ea6778a2ba154d8b6f26f6ed717f6b863567324d31bd0124c70fe124
Synthetic source SHA-256 before and after:
8c59b787561e8ac7ef51b4bf3b3da540ad820615510b339a28dcc22b922e800b
Evidence/probe commands: baseline-probe.py and baseline-probe/ outputs/summary.
Expected decimal calculations: analyst-decimal-examples.json, independent of
candidate parsing/metrics; not the Phase 3 frozen QA oracle.

Contract: existing numeric JSON fields in seconds, no fixed-decimal display
rounding, exact microsecond endpoints and half-microsecond even medians;
odd/even/duplicates/zero/null and invalid cases; source-time and key ties;
unchanged handoff-window and claim-history selection; report-only supported
legacy DB behavior. AC4 integer-prefix conservation stays unchanged; common
intent is elapsed duration before lossy presentation, not one shared quantizer.
Manager must independently accept that reconciliation and the explicit retained
finer-than-microsecond limit. No database migration or new renderer requested.

Four required current-workspace checks passed: cargo check --workspace,
cargo test --workspace, cargo clippy --workspace -- -D warnings, cargo fmt --check.
Exits/logs: phase-1-checks.json and phase-1-cargo-*.log.
These verify entry workspace only, not a #34 candidate. No code edits, QA/review
verdict, owner credentials, source mutation, commit, direct delivery message,
push, PR, merge, tag or publication. All tenure-authored artifacts are under
this goal directory; existing owner changes were preserved.

Next owner pm.manager: accept or return product criteria, then use governed
handoff deliver only when delivery is free. Phase 3 owns independent oracle
freeze with original evidence and narrowly recorded M2 deltas; candidate
representation/performance and full-corpus deltas remain unverified. #12 F12-1
must use the same R1 build; no #12/#13 acceptance here. Existing integration
and publication holds remain.

Durable return: qitem-20261014204636-ad6202d8 closed as handed_off_to
pm-manager@dendro-pm; new acceptance item qitem-20261014205227-13685408.
