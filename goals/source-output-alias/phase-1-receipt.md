# #46 Phase 1 analyst receipt

2026-10-14 UTC, pm.analyst / pm-analyst@dendro-pm, Codex / codex-cli /
local-cli / current-session. Claimed qitem-20261014215916-cebc8d36.

Pass: https://github.com/smota/dendro/issues/46#issuecomment-6179510551
Specification: goals/source-output-alias/phase-1.md
SHA-256: 69e6994ba18d182e416b11cae7465f5917383ef5335327af508040f5ec8d08f8
Key: 46-phase1-source-output-alias.
Read-back: phase 1, allowedNext 2/0, verified analyst signer and body match
except reader-stripped final newline; phase-1-readback.json.

Derived boundary from accepted R1 AC11 direct/symlink/hard-link refusal before
opening, AC12 original-file access rules/SHM exception and #30 frozen source
protection. Inspected current root/output helper and preflight paths at HEAD
1e4909bee5eefe924cff6c1c1c87ce51f32bfd46. Existing-output preflight precedes
alias guard; inode walk only compares regular descendants, not a root file.
Spec pins source-file and directory roots, direct/equivalent path identities,
outside hard links, symlink primary/parent spellings, source-tree containment
including ordinary new nested output paths, and existing/absent OpenRig main/
WAL/SHM protection. Refuse before original content/SQLite or write-capable
access; no alias-side output artifacts in source locations. No universal race
or arbitrary auxiliary-filesystem expansion. Architecture owns assurance,
isolation and solution; no code/edit list prescribed.

Bound final origin QA manifest SHA-256
9e15d5c9f8bd037879f6589c06af3a934461b7f04259f9ebbdad4932db82ed0e.
Streamed hashes/lengths of all 67 manifest-listed evidence artifacts; zero
mismatches. Pinned finding, candidate syscall trace, QA report and frozen #30
oracle hashes in specification. Inspected candidate syscall lines 418-425,
showing WAL/SHM O_RDWR|O_CREAT, SHM truncate/shared writable mapping, and compared
recorded state changes. No new mutation probe or raw source/key access.
Integrity verification is not independent acceptance/rerun of all QA assertions.
Detailed integrity result: evidence-pin-verification.json.

Required current-workspace cargo check --workspace, cargo test --workspace,
cargo clippy --workspace -- -D warnings, cargo fmt --check all exit 0.
Logs/exits: phase-1-checks.json and phase-1-cargo-*.log. Existing workspace checks
are not a #46 repair proof. No product code, commit, QA/review verdict, direct
delivery contact, owner-source/key access, integration or publication.
This tenure writes only goals/source-output-alias; owner changes preserved.

Next owner pm.manager: independently accept or return criteria and route only
accepted Phase 1 through governed handoff deliver. #46 needs independent QA5
and review6 with observed operation plus state evidence; missing observer proof
is unknown. Actual repair candidate/assurance and detailed oracle remain open.
#30 remains pending QA/held with wording scope unchanged. Independent #46
QA/review and composed-candidate reverification of #30's frozen contract are
prerequisites to any unqualified #30 outcome. No synthetic phase return,
#12/#13 pass or integration/publication permission is inferred.

Durable custody: qitem-20261014215916-cebc8d36 closed as handed_off_to
pm-manager@dendro-pm; successor qitem-20261014220559-5ee9e1b8.
