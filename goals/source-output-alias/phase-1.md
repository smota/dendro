# Phase 1 — #46 source-output alias refusal

Status: pass, product acceptance specification, not candidate QA/review.
Signer pm.analyst / pm-analyst@dendro-pm; Codex / codex-cli / local-cli /
current-session. Date 2026-10-14 UTC. Key: 46-phase1-source-output-alias.
Goal https://github.com/smota/dendro/issues/46.
Phase 0 https://github.com/smota/dendro/issues/46#issuecomment-6179407454.
#30 hold https://github.com/smota/dendro/issues/30#issuecomment-6179407996.

## Outcome and public journey

When I accidentally select an original source as analytics output, I want
Dendro to refuse safely, so even a failed ingest cannot disturb the fleet's
source data or its SQLite sidecars.

Use the existing public one-shot ingest. In a synthetic fixture, S denotes an
existing selected OpenRig SQLite source and D the analytics destination:

```sh
dendro run --db S --root openrig=S
```

S and D stand for operator paths, not literal filenames. The same protection
applies through existing default source selection and `--frozen`; the operator
must not need an extra safety flag. This changes a refusal's safety, not the
snapshot retry policy or a report/metric. A distinct safe output continues to
work through `dendro run --db D --root openrig=S`.

## Existing contract and bounded identity rule

R1 AC11 explicitly requires source-output refusal for direct paths, symlinks
and hard links before opening, with source state unchanged. AC12 requires
read-only originals; its historical SHM exception never permits WAL writes.
The accepted #30 contract separately preserves read-only main/WAL/SHM and
containment behavior. Baseline presence does not waive any of these promises.

The in-scope primary output collision is --db naming an existing protected
source file by identical filesystem identity, or selecting a destination inside
a declared source directory. This is filesystem identity/containment, not string
equality, filename extension or equality of source bytes. For a selected
OpenRig source, protect its original main file and its associated `-wal` and
`-shm` names, whether each sidecar currently exists or is absent. An output
must not create a formerly absent source sidecar. Protect declared source-file
roots themselves, not only files found beneath directory roots.

Equivalent existing-file identities include absolute/relative spellings,
`.`/`..` through existing directories, a symlink output, symlinked existing
parent-directory spellings, and hard links sharing device/inode. A hard link
outside the source directory is still an alias. Independently copied bytes in
a different file are not a hard-link identity. Existing symlink-output refusal
remains in force even if its target is not a source; this issue does not make
symlink outputs newly legal.

For declared directory roots, preserve the existing prohibition on analytics
output inside the root or its descendants after resolving existing directory
aliases. It applies to existing and new outputs, including an ordinary absent
nested output path whose parents would otherwise be created within the source
root. It also covers outside hard links to original regular files under the
root, including existing OpenRig sidecars there. The source identity check is
not narrowed to a report's harness, a .jsonl extension or discovery's first file.
Directory traversal must retain existing containment/symlink-cycle safeguards;
this contract does not authorize following source symlink trees outside roots.

This is a bounded static-path/identity contract at invocation. It does not
promise universal race freedom against concurrent malicious path replacement,
new filesystem/network/mount support, privileged access, arbitrary special-file
handling or a redesign of auxiliary-file identity policy. Such findings must
be routed to their owner rather than silently waived or repaired in this slice.
Architecture owns race/isolation/assurance assessment and technical design.

## Required refusal and source effect

For an in-scope alias or contained destination, normal ingest exits nonzero
(existing CLI exit 1), emits a sanitized source-alias/symlink/containment refusal
on stderr, and emits no successful ready JSON. Use the existing applicable
message meaning, with these expected bodies for the normal fixtures:

- Direct/equivalent file or hard-link alias: `refusing analytics output that aliases a source file`.
- Symlink primary output: `refusing symlink analytics output`.
- Destination within a declared source directory: `refusing analytics output inside a source root`.

Where multiple protections apply, either applicable safe refusal is acceptable.
Ordinary `Error: ` framing/newline is not part of these bodies. An incidental
schema-version, SQLite lock, permission or missing-key failure does not prove
alias protection. Do not depend on the source being an invalid analytics DB.
An aliasing file that happens to resemble a supported analytics schema is still
a source, never a permissible output.

Refuse before content/SQLite opens on the original protected files and before
any operation capable of changing them. Metadata-only identification, such as
stat/lstat and directory/path resolution, is allowed. The alias cases must
have no original main/WAL/SHM write-capable or creating opens, truncation,
writes, shared writable file mappings, unlink/rename/replacement, or output
publication onto a protected inode/path. Check operations and attempted opens,
not only successful mutations. The same before-opening guarantee applies to
other original source files covered by AC11 when aliased as output.

In the alias/contained-output refusal cases, do not create analytics database,
key, lock, private-stage or recovery artifacts within the declared source
directory or alongside the aliased source as a consequence of treating it as
output. Do not remove, rewrite or recover an existing original main/WAL/SHM.
Existing source sidecars retain bytes, size, mode, device/inode and mtime;
absent sidecars remain absent. Atime changes caused by permitted metadata/
observation are outside the existing byte/mtime promise. Where --db is a hard
link, preserve both pathnames and their shared identity, link count and state.
No new requirement to delete unrelated preexisting files is introduced.

The historical live SHM exception is not permission to run an output database
preflight on the source. For this safely refused alias journey, none of the
three original files needs a SQLite connection or shared writable mapping.
No global amendment to AC12 or unrelated-reader SHM policy is made here.

## Observable acceptance matrix

Use new, contained synthetic fixtures with a quiescent producer connection
held open where necessary to retain a nonempty WAL/SHM. No owner store/key or
live fleet may be used to demonstrate a damaging negative control.

| Given static input and destination | When existing public ingest runs | Expected |
| --- | --- | --- |
| --db equals an explicit source-file root S | normal run, then separate --frozen fixture | Safe alias refusal before original content opens; original main/WAL/SHM unchanged |
| Same S through relative, absolute or dot-component spellings | normal run | Same safe refusal; spelling cannot bypass identity |
| Output is an existing symlink to S, or a symlink-parent spelling of S | normal run | Applicable safe symlink/alias refusal, no target/sidecar content opens or changes |
| Output is a hard link outside the source root to S | normal run | Alias refusal; same inode and link count retained |
| Output names S-wal or S-shm, directly or by existing hard link/symlink | normal run | Safe refusal before access as output; no main/sidecar mutation |
| Associated WAL/SHM is absent and output names that protected sidecar | normal run | Refusal without creating the sidecar |
| Existing original file below a declared directory root, including nested file | output is that file or an outside hard link | Safe alias/containment refusal without source opening or mutation |
| New/existing output within declared source directory, including absent nested parents | normal run | Containment refusal without creating output artifacts in that source tree |
| D is distinct and outside declared source directories, with separate safe output artifacts | normal and --frozen runs | Coherent normal ingest succeeds, same baseline facts/counts/metrics after declared normalization |
| Valid existing D is bound to another distinct OpenRig root | normal run | Existing second-root refusal preserved with output/source unchanged |

For equivalence tests, ensure the alternative path actually resolves to the
same device/inode; for distinct-output tests ensure it does not. A dangling
output symlink remains subject to existing symlink refusal, not a successful
fresh output. Source root file versus directory forms both need explicit
coverage; testing only the existing helper on a directory is insufficient.

Preserve existing missing-root handling for ordinary non-alias outputs, key
first-run/missing/mismatch behavior on distinct outputs, invalid schema refusal,
source containment and secret-shaped path sanitization. An alias refusal must
not require reading/printing key material or creating a key beside the source.
Do not force a missing-key error to run first on a source alias. A distinct
safe output must retain unchanged retry count five, copy order, integrity checks,
cancellation, cleanup and transactional publication, including #30's exact
hint on its own exhaustion path if #30 is present in the composed candidate.
No new rate/performance threshold, source parser or database schema is requested.

## Instrumentation and independent proof

Phase 3 freezes literal path/identity fixtures, independent expected refusals,
source-operation/state expectations, applicable baseline/candidate commits and
binaries, command argv/cwd, source manifests and observer/tool provenance.
Preserve the original failing #30 alias oracle/trace and state; do not replace
it with a candidate-derived golden or treat baseline parity as a safety pass.
QA/reviewer own independent evidence and any explicit oracle correction.

Observe the public process and all descendants with an existing validated
file-operation instrument. Capture metadata/open flags, descriptor/inode
resolution and memory-map protection/sharing information; omit read/write
payload buffers. Account for resumed/unfinished syscalls and descendants;
unexplained source operations, unresolved descriptors or missing observer
coverage mean unknown, not zero violations. A path merely present in execve
argv is not a source-file operation. An O_RDWR/O_CREAT source open violates
the criterion even if bytes happen to remain unchanged or the call fails.
State hashes alone cannot prove the absence of write-capable opens.

Include a retained failing synthetic control from the affected pinned baseline
(or equivalent validated observer control) to show the observer can detect the
WAL write-capable open, SHM truncation and shared writable mapping. Instrument
source main/WAL/SHM even when reached through output alias or alternate links;
matching only the literal --root path is insufficient. Use state manifests
before/after while the synthetic producer is quiescent, before connection
teardown/checkpoint can confound SHM evidence. Include absent-sidecar controls
and no-WAL source. No destructive experiment on real sources is authorized.

Distinct-output success verifies existing faithful WAL-only committed data,
main/WAL read-only access and existing private-copy behavior, no source SHM
opens on that snapshot route, ordinary facts/provenance, unchanged rerun and
existing output/root/key/privacy controls. Verify source-tree/analytics state,
not just error strings. Fake secret/content canaries stay absent from captured
CLI diagnostics and persistable analytics under existing AC10 proof; no raw
payloads, owner keys or key sidecars enter the proof package. Existing hashes
and metadata may be recorded; this does not permit credential reads.

Run required cargo check --workspace, cargo test --workspace, cargo clippy
--workspace -- -D warnings, and cargo fmt --check. These do not replace observed
source-operation/state evidence or independent QA5/review6. Architecture chooses
assurance, isolation and solution; no file-edit list or guard implementation
is prescribed by this product contract.

## Pinned origin evidence and present knowledge

Source inspected at HEAD 1e4909bee5eefe924cff6c1c1c87ce51f32bfd46. The current
existing-output preflight precedes its alias refusal. The identity helper walks
regular descendants of directory roots, skips symlinks, and does not compare a
root that is itself a file. These observations explain why merely accepting a
current helper test or moving a message is insufficient evidence of the product
boundary. They are technical evidence, not a prescribed repair.

Independent #30 QA candidate e34b2c1ee7cd99a8dd6f41988ed9edcedc80d52c, release
binary SHA-256 b5539076d4781d266b11d500b50544d6271c17a00527e5f93cf66f454060fd33,
compared with its pinned baseline and reported original WAL/SHM O_RDWR|O_CREAT,
SHM ftruncate and shared writable mmap, plus SHM hash/mtime changes. Both
processes exited 1 on a schema refusal. Independent QA held phase 5 despite
98 frozen wording checks passing. The analyst inspected the candidate trace's
lines 418–425 and the comparative finding; no new source mutation probe was run.

Final origin evidence bindings:

| Artifact | SHA-256 |
| --- | --- |
| .agent-runs/issues/30/qa-e34b2c1/evidence-manifest.json | 9e15d5c9f8bd037879f6589c06af3a934461b7f04259f9ebbdad4932db82ed0e |
| .agent-runs/issues/30/qa-e34b2c1/alias-custody-finding.json | 27d2e682c175eb8f15104949ecb0ea9cbd6323346330ff021def8625e2d021e9 |
| .agent-runs/issues/30/qa-e34b2c1/observer/root-output-alias.syscalls | 7a284b27bfc5fa66f85e0f19919d9f88ea139c09e8382320749e802a68aba0d7 |
| .agent-runs/issues/30/qa-e34b2c1/QA_REPORT.md | 59c88526a46bbd38ecc1a6be9ebf3ae7d0f19ea3c909ab275b3c3da9f5b16e16 |
| .agent-runs/issues/30/oracles/v1/manifest.json | 3ee3f1eaca9b8285a1d54b36f5069399be2b85d846216c8247eb7a8c5fd5a6ae |

The analyst streamed hashes/lengths for all 67 files listed by the final QA
manifest and found no mismatch; result at `evidence-pin-verification.json`.
This checks evidence integrity and the inspected operations, not a rerun or
independent acceptance of all QA assertions. No raw private database/key was
read or copied. Candidate repair, assurance, detailed oracle and race/isolation
limits remain unverified and owned by delivery.

## Dependency, release boundary and return

#46 is a separate source-protection defect. #30's wording contract and accepted
quick_check/exhaustion classification limit stay unchanged. Its pending QA is
held; no fabricated phase-5-to-phase-1 return or candidate acceptance is made.
After independent #46 QA and review, delivery must pin the actual composed
candidate containing both changes and reverify #30's frozen diagnostic/custody
criteria on that build. Separate-branch passes alone do not establish the
composition. Product acceptance must cite this proof before giving #30 an
unqualified QA/readiness/product outcome; publication holds still apply.

One local v0.2.0 defect correction; no generic filesystem hardening, universal
race guarantee, retry/metric/schema expansion, #12/#13 acceptance, main edit,
push, PR opening, merge, tag, integration or publication authorization. Existing
owner changes and integration/publication holds remain. No implementation is
authorized by this analyst specification alone.

Return this Phase 1 pass, digest, pinned evidence and unknowns to pm.manager for
independent product acceptance. Only accepted Phase 1 through governed
`agentflow-sdlc handoff deliver` may route #46 into delivery when available.
No direct delivery contact or bypass is authorized here.

References read: Phase 0/issue and architect disposition request; #30 frozen
contract and final QA evidence; R1 AC10/AC11/AC12 and historical SHM exception;
current root identity, alias/containment, key/output-preflight and discovery
source; existing synthetic observer provenance, operation report and tests.
