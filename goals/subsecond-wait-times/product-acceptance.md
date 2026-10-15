# #34 product acceptance — local candidate

Decision: accept local delivery against Phase 1; pause integration/publication.
pm.manager / pm-manager@dendro-pm, Codex / codex-cli / local-cli /
current-session, 2026-10-14. No delivery rework requested.

Goal: https://github.com/smota/dendro/issues/34
Contract: https://github.com/smota/dendro/issues/34#issuecomment-6178194115
Contract SHA-256: bc1b32bbcfad446945bacc4054990511752a982e0e11b0d8dc5731627655bc08
Phase 8: https://github.com/smota/dendro/issues/34#issuecomment-6178917779
Latest read goal revision: 8fe8c4fff32b57096f0ae9d143d5351427b3f2b149ccd1e4d476c75d6543ed13
Candidate: 51998a6af30155919632b224af6cb4698a81bfb9
Branch: fix/fractional-handoff-waits
Base: 13423047ba4f4094e49a9267fcf2278be0da43ab
Worktree: /home/sam/code/dendro/.worktrees/fractional-handoff-waits
Release SHA-256: d3d93457de8fd98463b20f79d861573d6f9e35f08e2d7d5b793279188c378bec

## Accepted outcome

The existing report handoffs and trace medians expose fractional wait samples
and medians computed from selected timestamps before presentation rounding.
24.734 remains 24.734; crossed endpoints retain 0.2; odd/even examples retain
0.051/0.099; microsecond samples and half-microsecond even medians survive in
the bounded examples. Numeric JSON, sample order/keys, invalid/null/zero rules,
claim selection, filters and attribution remain accepted. Medians trace
formula/version identifies the changed wait calculation. AC4 integer-prefix
conservation remains unchanged.

Verified live phase8 pass, independent QA5/review6, clean candidate HEAD,
contract/binary/oracle/QA-manifest hashes, and reviewed the QA report, reviewer
verdict/final checks and concrete local PR draft. Latest read still shows the
named Phase 8 transition; its generated next4 edge is a possible return, not an
instruction to rebuild. Receipt revision differs from the later live projection;
no replacement Phase 8 or changed issue outcome was found on the surfaces read.

Independent QA and reviewer evidence covers32 immutable timestamp cases through
64 paired public journeys, legacy report-only behavior, rerun/append, missing
keys, privacy and AC4 preservation. Four Cargo gates pass independently:
136 tests passed, one ignored; the ignored benchmark is not pass evidence.
All26 corpus report/trace surfaces preserve every nonwait field exactly.
Independent Decimal arithmetic verifies265 wait samples and42 medians with
maximum errors1e-15/4e-15 seconds, below the accepted1e-12 tolerance. Original
oracle expectations are preserved with narrow per-sample wait deltas; no oracle
correction or blanket rebaseline occurred.

Oracle manifest: f0a57ac92190f21e464d5034e93274304370fb2fb68121966ab8c3bf2943450b
QA manifest: 6883f35532acbf90f81786c646d9906a1a883a81812c1424c756335a0737da2a
QA: .agent-runs/issues/34/qa-51998a6/QA_REPORT.md
Review: .agent-runs/review/34-51998a6/verdict.md
Draft: .agent-runs/issues/34/pr-draft.md

This product acceptance uses independent delivery evidence, not a new PM test
execution, self-review or a requester door-test witness. Code ownership and
candidate gate authority remain with delivery.

## Limits and supervisory disposition

Retain finer-than-microsecond persistence loss and magnitude-dependent binary64
resolution. Very large durations do not receive an all-magnitude microsecond
guarantee; documentation and range evidence disclose this without dropping
eligible samples. No clock accuracy, timestamp-parser or schema promise added.
Fresh full-corpus ingest performance is unproved; existing-DB regression proof
does not supply it. Token/intake/activity/loop semantics and privacy remain.

v0.2.0 minor, local build only. Integration is outstanding. No push, PR opening,
merge, tag or publication. Existing publication holds remain. #12 F12-1 must
still compare R1/#12 on the same build; no #12/#13 acceptance is granted here.
Supervisory steering owns later integration disposition and requester journey
judgment. Return proof to the Phase 0 requester steer.lead@kernel through the
durable queue; a persisted relay is not proof of recipient pickup.

## Relay receipt

Acceptance recorded as issue comment6178939827. Delivery return
qitem-20261014212926-b19669b4 closed transactionally into supervisory relay
qitem-20261014213100-5038174d. The original requester address steer.lead@kernel
is unroutable: no terminal transport or registered human on this daemon.
Rerouted the same row to registered operator-agent@kernel, the existing fallback
for earlier local acceptance relays, preserving its history and hold. Architect
notified of acceptance/no rework. Supervisory pickup and integration remain
unverified; no seat or account was provisioned to manufacture a recipient.
