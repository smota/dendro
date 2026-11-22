# Local validation

Run `mise run verify` before requesting review. It first runs `verify:guard`, the
stdlib-only Python boundary regression suite, then runs the four locked Rust gates
on the current host and the existing governance and site checks. Its Linux leg
runs formatting, check, and Clippy in Dagger, followed by the complete unfiltered
workspace test suite on native ext4. Failures stop the task. `verify:fast` runs
formatting and governance/site checks; it is an iteration aid, not full acceptance.
Full native verification refuses a dirty candidate before running a Rust command.
The cheap gate runs both Windows resource-guard and committed-input/receipt tests;
neither suite compiles Rust or loads the Dagger SDK.
The local governance leg includes the dependency-automation and CI-scope unit
tests introduced by the current development baseline.
`mise run verify:dependency-automation` runs the upstream Dependabot author/path
validator against `origin/development` without a Rust build. A full local run on
a `dependabot/*` branch also adds this check. That name grants no permission:
the existing validator still checks every commit's author and allowed paths, and
ordinary path-ownership/catalog checks still run. `BASE_REF` comes from the
explicit `--base main|development` argument, never a caller-supplied environment.

The repository pins Rust 1.98.1 and Python Dagger SDK 0.21.12. Dagger requires a
local Linux Docker backend. Mise declares the matching Dagger CLI; tasks use
`dagger run -- uv run tools/validation/pipeline.py` so the SDK joins that CLI's
session instead of bootstrapping another CLI. Engine images download on first use.
No Dagger Cloud account or token is needed. Consult the
[versioned SDK source](https://github.com/dagger/dagger/tree/v0.21.12/sdk/python)
when changing the pipeline; current 1.0 examples describe a different API.
The SDK has a 30-second connection timeout, two-hour read timeout and 60-second
write/pool timeout; the
async pipeline has a two-hour deadline. SDK bootstrap can block in a synchronous
worker thread, so that async deadline cannot guarantee termination of bootstrap.
Use an external task deadline when supervising unattended runs. On this Windows
station a direct CLI pipe stalled before command entry; file-backed execution
worked during diagnosis. The Windows disk guard retains inherited terminal streams.
The Dagger tasks use mise's `raw = true` to inherit terminal I/O. Agents should
run those tasks in an Orca terminal, without piping or capturing their process
streams through a shell executor. Redirect diagnostic output only when investigating
the Windows CLI bootstrap issue.
Mise tools must already be available or installed by an authorized native station
process. This change does not install station tools or activate hooks.

`mise run verify:windows` refuses other platforms and runs all four Rust gates on
native Windows. Every native Windows verification mode uses one private profile
under `.agent-runs/windows-validation/<run-id>/`. Child `USERPROFILE`, `HOME`,
`APPDATA`, `LOCALAPPDATA`, `TEMP`, and `TMP` point inside that run. Tests cannot
leave their synthetic library folders in the owner's profile through these paths.
Windows requires Python 3.13 or newer so `mkdir(mode=0o700)` gives the run root
an explicit private ACL instead of inheriting repository access. Older Python
versions fail before launch; the guard does not install an interpreter.
Before creating children, the guard replaces Python's `OWNER RIGHTS` ACE on that
new empty run root with an explicit creator-SID grant plus SYSTEM and Administrators.
Its protected DACL grants only those three principals inherited full control.
This preserves Dendro's ancestor-admission policy, which refuses replacement rights
granted to `OWNER RIGHTS`. Profile descendants inherit the admitted grants; existing
guard-base, worktree, home, and system ancestor permissions are not changed.
The guard preserves Rustup, Cargo, mise, uv, and pnpm installation/cache locations
before redirecting the profile. It disables Rustup automatic toolchain installation.
Native child environments admit only Windows/build variables, known tool paths,
proxy/CA settings, and fixture paths.
Agent data overrides and provider credentials are excluded. XDG directories also
point inside the private profile; mise auto-install/update flags are disabled.
Validation sets `CARGO_PROFILE_DEV_DEBUG=0`, `CARGO_PROFILE_TEST_DEBUG=0`, and
`CARGO_INCREMENTAL=0`; debug assertions remain enabled. Existing build artifacts
remain reusable in `CARGO_TARGET_DIR` (default: this worktree's `target/`).

The Windows guard accepts these positive integer byte limits:

| Environment variable | Default | Bound |
| --- | --- | --- |
| `DENDRO_WINDOWS_MIN_FREE_BYTES` | 10737418240 (10 GiB) | Free space on source, cache, and run volumes |
| `DENDRO_WINDOWS_TARGET_MAX_BYTES` | 25769803776 (24 GiB) | Reusable Cargo target directory |
| `DENDRO_WINDOWS_RUN_MAX_BYTES` | 4294967296 (4 GiB) | All Windows run workspaces combined |

Preflight refuses an over-budget run. While a command runs, the guard scans usage
between 200 ms waits and stops its Windows Job Object on a breach. These are
sampled limits, so a child can overshoot during a scan or between samples.
Commands start only after their helper joins the owned job. A parent crash closes
the job handle and stops its children. Success, failure, and interruption drain
the job before scratch cleanup. A marker and exclusive lease permit pruning
only abandoned workspaces recorded as idle. An abandoned `running` marker,
missing ownership evidence, an ownership-root reparse point, or an unknown reparse
tag stops validation and leaves the directory for manual inspection. Ordinary
descendant symlinks and Windows junctions are legitimate test fixtures: scans
count their entry metadata and never descend into linked targets. Cleanup removes
only link entries; it preserves outside targets. Read-only ordinary fixture files
are made writable and retried. Other cleanup failures report the exact path and
retain ownership evidence while payload files remain. Cleanup never searches the owner's profile or
removes the reusable build cache. Large caches require deliberate cleanup or an
explicitly adjusted budget before retrying. External cross-volume fixture paths
remain separately authorized disposable inputs and are outside scratch cleanup.
Read-only Git admission metadata runs outside the child-command guard.

For an abandoned run, retain the error path and inspect that exact UUID directory
and its marker. Verify that its recorded root matches this worktree and that its
root, marker, lease, and ancestors are ordinary entries. Identify the corresponding
validation invocation and establish that its owned children have stopped; for WSL,
also check its Linux supervisor and test process group. A missing parent PID alone
does not establish this. Do not change a `running` marker to `idle` to bypass refusal.
After ownership and termination are established, inspect descendant links and
remove only those entries, then remove only the identified disposable run directory.
Never follow link targets, clear the registry by prefix, or delete tool/build caches
as a workaround. If ownership or child termination remains uncertain, leave the
directory intact and report the exact path for operator review. An idle directory
blocked by a temporary file handle can be retried after that handle is released.

Run the cheap guard checks without compiling Rust:
`python -m unittest discover -s tools/validation -p test_windows_guard.py -v`.

`mise run verify:linux` composes the Dagger gates and native
Linux tests. On Windows, the native adapter uses WSL and a private temporary ext4
copy of a sanitized source snapshot. Source editing and Git work stay on Windows.
The adapter verifies the copied content digest, admits only private ext4 roots,
uses a short isolated fixture HOME and TMPDIR, and passes an allowlisted environment.
It verifies that the Windows candidate remained unchanged and records a local
receipt under `.agent-runs/local-validation/`. The pinned Linux Rust toolchain must
already be installed; this task does not install it.

`verify:linux` launches its existing Dagger and native-adapter commands through
`windows_guard.py --external -- ...`. This mode retains the caller's tool and
desktop profiles. On Windows it checks the host free-space floor and contains
the client process tree in an owned Job Object; other platforms pass the command
through unchanged. It does not impose quotas on Docker/WSL virtual disks or kill
their services. Stopping a client is not proof that engine-side work has stopped.
Do not treat this wrapper as a Dagger engine cache limit.

The native Linux supervisor checks cache, aggregate fixture workspace, and staged
source sizes before Cargo starts and while it runs. It checks free space on all
three filesystems, including the Windows `/mnt/c` source-stage volume in WSL.
`DENDRO_LINUX_MIN_FREE_BYTES`, `DENDRO_LINUX_CACHE_MAX_BYTES`, and
`DENDRO_LINUX_WORKSPACE_MAX_BYTES` set positive byte limits; defaults are 10 GiB,
24 GiB, and 4 GiB. The free-space default also accepts the Windows floor override.
The adapter forwards these limits explicitly to its WSL child. Linux Cargo uses
the same no-debug/no-incremental settings as Windows and retains the complete
unfiltered workspace test command. Its owned process group stops on a budget
breach, two-hour deadline, interruption, or supervisor TERM/HUP signal. The Linux
supervisor owns this enforcement independently of its Windows client. These are
sampled checks, not filesystem quotas; SIGKILL or a host crash cannot run cleanup.

Dependency and compiler caches remain on ext4. `DENDRO_LINUX_CACHE_DIR` can select
an existing admitted private cache; it never selects a production data directory.
The native test command has a two-hour deadline. These are separate evidence:
a container pass does not prove Windows ACLs or native ext4, and the routine
native suite does not prove actual cross-volume publication or historical rollback.

Advisor tests match development commit
`65451963296afa312ea8b68fe976d0b2d034e70e` exactly. Upstream enforces the timing
threshold in release builds; debug checks retain semantic results, zero writes,
privacy, and the explicit zero-budget rejection assertion. The validation runners
do not override these upstream requirements.

Dagger and native Linux receive committed HEAD blobs from one streaming Git
`cat-file --batch` process on the source host. Staging requires a clean worktree
before and after the copy and refuses a changed HEAD. Checkout line endings and
untracked files cannot replace committed bytes. No Git operations run in WSL.

Every tracked input must be classified explicitly. Public roots are `adapters`,
`crates`, `docs`, `examples`, `goals`, `lib`, `manifests`, `openrig`, `roles`,
`schemas`, `scripts`, `site`, `skills`, `templates`, `transitions`,
`tools/validation`, and `.agentflow/oracles`. The named root-file list also admits
Cargo/Node/workspace metadata, licenses, public documentation, project governance
configuration, and canonical Rust/tool configuration such as `rustfmt.toml`,
`clippy.toml`, `.cargo/config.toml`, `rust-toolchain.toml`, and `deny.toml` when
tracked. An unclassified tracked path fails staging; extend the classification
through review when adding a new public package or configuration location.

Explicit exclusions cover agent installations/instructions, known `.agentflow`
operational state, hosted-service metadata, VCS/hooks, generated output,
environment files, credentials, keys, and databases. Exclusion counts are recorded
by category without uploading their contents. A public input must be a regular
Git `100644` or `100755` blob; symlinks, gitlinks, reparse points, and escaped paths
are refused. Review new candidate files for secrets.

The generated `.validation-source-manifest.json` binds candidate HEAD/tree to
each admitted path, Git mode, blob object ID, and SHA-256. Receivers verify the
complete file set and recompute both blob IDs and byte digests. Git executable
modes travel as metadata, because Windows stat bits cannot prove those modes.
The Linux private copy applies `0700` to executable files and `0600` to ordinary
files; Dagger explicitly sets the admitted scripts executable before Rust gates.
The current executable inputs are `scripts/verify-architecture-guardrails.mjs`,
`scripts/verify-design-guardrails.mjs`, and `scripts/worktree-slice.sh`.

The Linux child verifies the source before and after tests, then emits candidate
HEAD/tree, manifest SHA-256, file count, and executable paths. The Windows receipt
requires exactly one valid child record matching all staged facts. Missing or
mismatched metadata makes `passed` false and the adapter exit nonzero even when
the child returned zero. `childExitCode` preserves that original status; `exitCode`
records the adapter's attestation decision. A temporary sanitized copy remains
the only host upload; literal paths avoid glob syntax and request-header limits.
Containers mount only project-specific dependency/build caches, never host home,
production stores, Docker sockets, or installed Dendro data.

## Native acceptance remains required

For historical migration/rollback, run the existing harness from a real Linux or
WSL checkout on ext4 with real repository history (including commit
`6b755135f95e497071790e8e5e358995a8244a18`):

```sh
mise run verify:historical -- --scratch /private/new-proof --target-dir /private/proof-targets
```

The harness requires fresh scratch and isolates its synthetic databases. Read
`scripts/historical_migration_proof.py` before selecting paths. Never point it at
owner data or reuse evidence directories.
The wrapper requires a clean committed candidate and verifies the full original
receipt assertions, including historical archive hashes, candidate blob hashes,
16 refusal cases and rollback controls. WSL evidence retains its WSL identity.

For actual cross-volume acceptance, create private empty disposable directories
on two existing distinct volumes. Set `FOUNDATION_CROSS_VOLUME_SOURCE` and
`FOUNDATION_CROSS_VOLUME_DEST` to their absolute paths, then run
`mise run verify:native-fixture`. The exact existing ignored Rust acceptance test
checks real device identity and refuses invalid filesystem/private-root setups.
The task neither creates volumes nor mounts or formats devices. Preserve the
positive test output and candidate revision as acceptance evidence.
The fixture task requires a clean committed candidate, a successful exit and both
original positive output markers; a zero-test invocation fails acceptance.

The former hosted governance, foundation, and nightly workflows are retained in
`workflow-reference/` as exact snapshots from development commit
`65451963296afa312ea8b68fe976d0b2d034e70e`. They retain hosted-runner admission checks and
are not portable local commands. Their full receipt collector and native-host
certification have not been replaced or certified by the Dagger task. Missing
historical or cross-volume evidence is incomplete release acceptance, even when
routine `verify` passes.

Local governance compares against the real `origin/development` ref and refuses
incidental committed version bumps. Fetch the integration ref before review.
For promotion topology, use `uv run tools/validation/verify.py --fast --base main`;
only `rc/v*` or `hotfix/*` branches may target main. Run
`mise run verify:release-candidate` for release-note presence and the workspace
check before a release-candidate handoff.

## Hooks and GitHub

`lefthook.yml` declares `mise run verify` for pre-push. These source files do not
install hooks. Inspect the effective owner with
`git config --show-origin --get core.hooksPath`; use the station's guarded
preview, activation, and rollback procedure for ownership changes. A managed
repository-local owner must preserve station checks and keep the global setting
unchanged. Do not run `lefthook install` over managed station hooks. Samuel approved
the exact protected patch in Plannotator message `msg_01M7Z0ZHZ0P8A7BK848AAVJX5C`,
and Codex applied it to this source worktree on 2026-11-21. `ciCommands` now
delegates to `mise run verify`. The patch is retained as a review record; do not
apply it again. Installed runtimes and trust still require a verified native
executor, and historical/cross-volume acceptance remains separate. A guarded
Lefthook handover must select one owner and retire the old verify invocation;
do not activate both forwarding chains.

Routine hosted foundation/governance workflows are archived outside
`.github/workflows`. GitHub capacity remains available for the existing release
publication workflow. Branch promotion, release notes, review and human merge
authority remain required; moving validation does not grant publication consent.
