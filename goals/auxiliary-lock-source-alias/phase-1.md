# Phase 1 — #47 auxiliary output-lock source protection

Status: pass, product investigation and safety contract; not candidate QA/review.
pm.analyst / pm-analyst@dendro-pm, Codex / codex-cli / local-cli /
current-session. 2026-10-15 UTC. Key: 47-phase1-auxiliary-lock-safety.
Goal https://github.com/smota/dendro/issues/47.
Phase 0 https://github.com/smota/dendro/issues/47#issuecomment-6181442392.

## Outcome and actual public path

When my analytics destination is distinct but its auxiliary lock aliases an
original source, I want safe refusal before write-capable source access, so
output locking cannot disturb the source even when ingest ultimately fails.

Use existing one-shot ingest with a new synthetic source S and distinct primary
analytics destination D. For example, D is /fixture/output/analytics.duckdb,
S is /fixture/source/openrig.sqlite, and the actual lock L is
/fixture/output/.dendro-output.lock:

```sh
dendro run --db /fixture/output/analytics.duckdb --root openrig=/fixture/source/openrig.sqlite
```

These are illustrative synthetic paths. The lock is the destination parent's
`.dendro-output.lock`, shared by different output filenames in that directory.
It is not a `D.lock` suffix. Literal path spelling, cwd and resolved parent
identity must be pinned in the fixture. Normal/default/frozen public entrypoints
retain existing selection and locking semantics; no new safety flag is required.

The supported static precondition is a preexisting regular L hard-linked to a
selected protected source file while D itself is distinct and outside declared
source directories. The parent directory exists; the fixture can create the
hard link on the same filesystem with ordinary permissions. L and its target
have the same device/inode and link count at least two. The unsafe condition is
not established by matching bytes or a similar name. Before manifests are taken,
record the actual link identity and verify primary D is not a source alias.

Protect selected original regular files, including explicit file roots and
original regular descendants of declared directory roots without widening source
symlink traversal. For OpenRig, include the selected original main, WAL and SHM
under the established source/sidecar association. Parent-directory symlink or
relative/canonical spellings do not turn a shared physical lock into a new lock.
This is preexisting auxiliary lock hard-link protection, not arbitrary
key/stage/recovery identity redesign or universal namespace-race safety.

## Investigation result and exact limits

The conditional Phase 0 reproduction requirement is met on the tested entry
build: the normal public command successfully opens L with O_RDWR|O_CREAT when
L hard-links to the original main, WAL or SHM. Each then acquires LOCK_EX|LOCK_NB
on that source inode and exits 1 with `output lock identity is unsafe`.
The final refusal is too late to establish source read-only protection.

For these recorded cases, source hashes/size/mode/device/inode/link-count/mtime
and the fixture tree are unchanged. No direct source write/truncate or shared
writable mapping is observed. Confirmed defect is write-capable source access
and source-inode lock acquisition before refusal, not demonstrated byte
corruption, owner exposure, a live exploit or an occurrence-frequency claim.
A successful or failed write-capable attempted open violates the safety outcome
even if source bytes remain equal. Retain that distinction in docs and QA.

## Expected safe refusal and state

Given D distinct and L already hard-linked to a protected source, normal ingest
must refuse before any write-capable/creating source open through L or another
alias, and before acquiring the output exclusion lock on that protected inode.
Use the existing sanitized body `output lock identity is unsafe`, exit 1 on
stderr, no ready/success stdout. Ordinary `Error: ` framing and final newline
remain unchanged. Do not rely on missing key, schema, SQLite contention, inability
to create output or a late nlink rejection as the source-safety proof.

Metadata/identity inspection is permitted. Do not open the protected inode as a
writable output-lock file, write/truncate it, create/delete/rename/replace an
original main/WAL/SHM path, or map it shared writable, including protection
upgrades. Failed prohibited attempts count as violations. Preserve the source
and both hard-link names, contents, size, mode, inode/device, nlink and mtime;
absent associated sidecars remain absent. Atime is outside the existing state
promise. Do not unlink or recreate L to make the run succeed. The historical
live SHM exception is not permission to open a source SHM as an output lock.

No analytics database/facts are published, existing D stays unchanged, and the
refused auxiliary-alias path must not proceed to analytics key creation/read,
private stage/recovery or ingest. Preexisting lock and fixture artifacts remain;
no blanket no-filesystem-activity claim is made. No key material is inspected
or printed to demonstrate refusal. Source/lock identities are determined
without reading source payloads or owner credentials.

## Lock compatibility and public acceptance

1. Given the three reproduced static cases L -> main, L -> WAL and L -> SHM,
   when the normal public command runs on independent quiescent synthetic
   fixtures, then it has the safe refusal/state above and zero prohibited
   source-operation attempts. Cover a primary existing D as well as fresh D;
   protect existing analytics from failed-run changes. Repeat a representative
   case with --frozen and safe isolated default-source selection.
2. Given an original regular source file under a declared source directory, or
   another supported explicit regular-file root, when L is a hard link to it
   and primary D is disjoint, then the same source-safe refusal applies. Include
   a parent-directory alias spelling with proven physical lock identity. Do not
   infer identity solely from filenames, extensions or equal contents.
3. Given L is a symlink (including dangling), directory, or otherwise rejected
   nonregular path, when ingest runs, then existing refusal is preserved:
   `refusing an output lock that is not a regular file` for normal symlink/
   directory fixtures. Do not follow a lock symlink to a source or replace it.
   Existing rejection of multiply linked or held/named identity-mismatched
   locks remains; auxiliary protection is not relaxed for unrelated targets.
4. Given a normal absent safe L in a safe output directory, when ordinary ingest
   runs, then existing private lock creation/permissions and successful coherent
   ingest remain. Given an ordinary existing single-link lock, reuse it under
   the same exclusion/identity rules; do not introduce a new chmod policy or
   require deleting an ordinary persistent lock file.
5. Given another process holds the actual safe shared directory lock, when runs
   choose D1 or D2 in that same directory, then each retains nonblocking exit 1
   with `analytics output lock is busy`, no ready JSON or new database. Do not
   split lock scope by output filename, bypass an active lock, wait indefinitely,
   rename it away, or recreate it to allow a competing writer. After the holder
   releases it, a new ordinary invocation succeeds. Hold/release only synthetic
   lock descriptors; no agent or live owner process is stopped or controlled.
6. Given genuine permission failure or incomplete lock identity evidence, when
   the existing error route is exercised without privilege changes, then fail
   closed with sanitized diagnostics and no source-write bypass. Preserve
   existing error/exit behavior; chmod under bypass privileges is not a real
   permission control. Unexercised permission conditions remain unknown.
7. Given normal distinct output and safe auxiliary artifacts, when ingest/
   unchanged rerun/report/trace runs on a pinned source set, then facts,
   provenance, metrics, key behavior and privacy are unchanged. Preserve #46
   primary alias/containment/identity-uncertainty protection; second-root and
   multi-root refusals; ordinary missing-source behavior; first/missing/mismatch
   key and schema checks; snapshot copy/integrity/cancellation/cleanup/publish
   behavior. Keep #30's exact manual retry sentence, five-attempt policy and
   retained quick_check/exhaustion classification unchanged. No source-root,
   parser, metric, schema, rate or performance expansion is included.

## Payload-free operation proof and independent oracle

Phase 3 freezes independent source/lock identity fixtures, literal expected
refusals, operation/state answers, commands/cwd/child environment, baseline
and candidate source/binary pins and observer provenance before build. QA owns
the frozen expectations; builder must not generate goldens from candidate
results. Preserve the original unsafe public traces and state alongside the
intended operation delta. Candidate exit/error parity alone is not acceptance.

Observe the public process and descendants with a validated payload-free
instrument: attempted open flags and actual descriptor paths/inodes, link/path
aliases, descriptor inheritance/duplication/reuse, source mutations and memory
mapping protection/sharing/upgrades. Track resumed/unfinished syscalls and normal
descendant exits. The synthetic producer remains quiescent/open from before
state capture through after capture; close/checkpoint afterward. Do not inherit
producer/source descriptors into the child. Record protected inode/path and
hard-link name manifests, including main/WAL/SHM and output trees. Read/write
syscalls must render pointers/lengths rather than payload buffers; no source
content or key material belongs in syscall artifacts. Hashing for manifests
occurs outside the traced CLI; synthetic key sidecars are excluded from reads.

A path in execve argv alone is not a file open. An observer matching only S's
pathname would miss L; prove inode-aware hard-link detection with independent
controls. Retain controls for actual unsafe source access, failed writable
opens, read-only/no-write positive cases, descendant/shared/duplicated FDs,
mapping upgrades and unknown descriptors. Missing operations, unresolved
relevant descriptors/processes/maps, parser failures or incomplete exit coverage
mean unknown, not zero violations. State equality never replaces operation
proof. Do not use a helper-only reproduction as evidence that normal CLI has
this defect, or run a damaging control against an owner source.

Synthetic privacy canaries must remain absent from diagnostics and persistable
analytics under existing AC10 coverage. Positive source-read evidence must
include applicable WAL-only committed data and no source SHM opens on the
existing snapshot route. Source effects and output/lock ownership changes must
be attributed to Dendro separately from fixture setup/holder teardown. Do not
silently generalize finite synthetic outcomes to real-owner behavior.

## Reproduced entry evidence

Entry source built: d7f180715440d0389101626cfc1eedbb0ce8490b.
Debug binary SHA-256:
0262540a9f54f70ef2218470ec8aba6f41182c33c00190f8844817ca2ac89d9e.
Probe: goals/auxiliary-lock-source-alias/baseline-probe.py.
It creates separate source/output directories, a synthetic WAL-mode SQLite
source held open/quiescent, and the actual .dendro-output.lock hard link. It
uses only explicit synthetic roots and close_fds=True. Source metadata and all
link names are captured before/after; producer closes afterward. No owner data
or credentials were read and no product code was edited.

Results, all with exact recorded normal descendant exit coverage and zero
observer unknowns:

| Entry case | Public result | Observed write-capable original-source opens | Other observed prohibited source mutations | State |
| --- | --- | --- | --- | --- |
| L hard-links main | exit 1, unsafe-lock error | one successful O_RDWR|O_CREAT through L | none | source/tree unchanged |
| L hard-links WAL | exit 1, unsafe-lock error | one successful O_RDWR|O_CREAT through L | none | source/tree unchanged |
| L hard-links SHM | exit 1, unsafe-lock error | one successful O_RDWR|O_CREAT through L | none | source/tree unchanged |
| Safe distinct output, safe new lock | exit 0, ready | zero | none | source unchanged, expected output created |

Manual inspection of main trace lines 526–533 confirms the writable open,
LOCK_EX|LOCK_NB, subsequent metadata/link checks and unlock/close. Analogous
WAL/SHM opens are at lines 508 and 507 in their traces. Raw source paths and L
match by device/inode in each pre-command manifest. Captured CLI canaries are
absent. No decoded-storage privacy pass is claimed by this analyst probe.

All eight pinned project-local strace artifacts verify by SHA-256 before use;
no system install, profile or privilege change. Existing frozen #46 observer
module SHA-256 b5fcb570c54fa1ab89345faa688b1f05938d2cd0d267fb5d9f2b58214aa76c09
is imported without modification or bytecode writes. Its literal parser controls
pass for descendant/FD/hard-link detection, failed writable open, unresolved/
unfinished records, argv non-access and mapping upgrade. No payload-buffer
capture is enabled. See baseline-probe/summary.json, observer-controls.stdout,
and per-case receipt.json, trace.syscalls and operations.json. Evidence manifest
pins 27 analyst artifacts; SHA-256
fd17add26ddf9e9065b1dadeeebe9cc387a230eaf198032fc114d6618c877546
for investigation-evidence-manifest.json. It is not a Phase 3 frozen QA oracle.

Additional public baseline control at locking-controls/summary.json: a parent
holds the actual safe shared directory lock using flock; two distinct output
filenames in that directory each exit 1 with lock-busy and no database. A new
ordinary run succeeds after release. This is a finite synthetic locking control,
not a live-agent concurrency or performance guarantee. Other compatibility
cases above remain candidate/delivery proof requirements, not asserted passes.

## Ownership, known limits and release

Confirmed runtime risk warrants the bounded conditional repair selected by
Phase 0, subject to manager acceptance and delivery phases. Architecture owns
assurance, isolation, solution and static-path/race limits. This specification
prescribes no implementation files or algorithm. Ordinary accepted high-
assurance/autonomous policy applies; do not invent a person-confirmation gate
or downgrade assurance. Record actual tool rejections and unresolved proof
honestly. Required cargo check --workspace, cargo test --workspace, cargo clippy
--workspace -- -D warnings and cargo fmt --check remain gates. Independent
QA5/review6 must bind the exact candidate; analyst runtime investigation is
not candidate acceptance.

Unknowns: candidate repair, exhaustive permission/platform/filesystem behavior,
real-owner exposure/frequency and malicious namespace races. No source corruption
was observed in the tested cases. No arbitrary key/stage/recovery redesign,
universal race guarantee, broader filesystem support or unrelated hardening is
included. Broader findings return to product separately.

Earlier backlog prose is historical: #46 was locally accepted on exact
28cb41a; #30's local source-protection acceptance hold was satisfied on exact
830a9209600e5437ffe29d6079421efe0eebc0e9, with repaired composition and independent
proof. Those acceptances do not establish #47 safety. This new contract does
not reopen their historical hold or retroactively accept unsafe e34b2c1.
Any later composition/verification must name its actual build and scope.

One local v0.2.0 outcome; no main edit, push, PR, merge, tag, integration or
publication authorization and no #12/#13 acceptance. Existing release
boundaries remain. Return this pass, digest, evidence and unknowns to pm.manager
for independent product acceptance. Only accepted Phase 1 through governed
handoff deliver may proceed to delivery when available; no direct delivery
contact or implementation dispatch is authorized by this analyst pass.

References: current Phase 0/steering brief, origin static observation/backlog,
current public output-lock construction/open/link checks, R1 source/privacy
contracts, accepted #46 and exact #30 local product receipts, existing pinned
observer/provenance/parser controls and the new synthetic public evidence.
