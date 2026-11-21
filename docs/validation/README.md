# Local validation

Run `mise run verify` before requesting review. It runs the four locked Rust gates
on the current host, the existing governance and site checks, then the same Rust
gates inside Dagger Linux. Failures stop the task. `verify:fast` runs formatting
and governance/site checks; it is an iteration aid, not full acceptance.

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
worked during diagnosis. This configuration adds no platform wrapper.
The Dagger tasks use mise's `raw = true` to inherit terminal I/O. Agents should
run those tasks in an Orca terminal, without piping or capturing their process
streams through a shell executor. Redirect diagnostic output only when investigating
the Windows CLI bootstrap issue.
Mise tools must already be available or installed by an authorized native station
process. This change does not install station tools or activate hooks.

`mise run verify:windows` refuses other platforms and runs all four Rust gates on
native Windows. `mise run verify:linux` runs container gates only. They are
separate evidence: a container pass does not prove Windows ACLs, native ext4,
actual cross-volume publication, or historical migration/rollback.

Dagger receives candidate files from an explicit source allowlist and narrowly
selected `.agentflow/oracles` test fixtures. It excludes git history, agent
configuration, environment files, private keys and databases. Edits to tracked
files and permitted untracked files are included. A temporary sanitized copy is
the only host upload; literal paths avoid glob syntax and request-header limits.
Symlinks and escaped paths are refused. Review new candidate files for secrets.
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

The former hosted foundation wrappers remain unchanged in
`workflow-reference/` for review. They retain hosted-runner admission checks and
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

`lefthook.yml` declares `mise run verify` for pre-push. Review the station's existing
`core.hooksPath` before opting in to Lefthook; do not replace managed station
hooks. The configuration is delivered without an installation task or activation.
The station chain remains the active owner through `ciCommands`. Samuel approved
the exact protected patch in Plannotator message `msg_01M7Z0ZHZ0P8A7BK848AAVJX5C`,
and Codex applied it to this source worktree on 2026-11-21. `ciCommands` now
delegates to `mise run verify`. The patch is retained as a review record; do not
apply it again. Installed runtimes and trust still require a verified native
executor, and the existing Linux filesystem acceptance limit remains. A later
Lefthook activation must select one owner and retire the old verify invocation;
do not activate both forwarding chains.

Routine hosted foundation/governance workflows are archived outside
`.github/workflows`. GitHub capacity remains available for the existing release
publication workflow. Branch promotion, release notes, review and human merge
authority remain required; moving validation does not grant publication consent.
