**Review result: read-only, no edits, no agents spawned.** I read the source and both evidence files. I ran no tests and computed no hashes, so there is no rehash of the candidate bytes or of `ffi.rs`.

The source is unchanged since findings-03. `selection_binding.rs` has only the one commit, `c3c9b54`, and the F1 limits and all five smaller defects are still present.

## 1. Recovery scan lifetime exhaustion (F1)

**Where the limit lives**
- `selection_binding.rs:1075-1106` counts every `publication-*` directory and every journal against one 4096 total.
- `selection_binding.rs:1273` creates a `publication-<random>` directory per publication. Nothing retires it.
- `cutoff_publication.rs:416-446` has the same cap.
- `cutoff_publication.rs:268` creates `cutoff-stage-<random>` on every attempt, before it takes the lock, so even refused attempts leave one.

**Why the cap is easy to reach**
- The cutoff scan covers the whole parent directory, but its budget is shared across every cutoff leaf in that directory (`:416`). Cutoff exhaustion therefore arrives sooner than the selection one.
- `ATTEMPTS` retries are retained by design, so one busy publication consumes several budget slots.

**Direct-lookup repair**
- Name the stage from authenticated data, so recovery opens one stage instead of scanning.
- Selection:
  - `bundle-<random>` is created at `:899` and is already named in the signed manifest at `windows_publication.rs:77` (`bundle_name`).
  - Create the stage with `p::create_private_directory(parent, name)` (`mod.rs:179`, exact name) as `publication-<bundle suffix>`.
  - Recovery derives the stage name from `resolved.manifest.payload.bundle_name`.
  - The journal's `manifest_sha256` and `bundle_identity` check (`:1115, :1130`) then binds the stage to the pointer.
- Cutoff:
  - Use `cutoff-stage-<hash>-g<generation+1>-<rand>`.
  - The scan filters by prefix, so other leaves and other generations no longer consume this leaf's budget.
- Keep the `== 1` unique-match rule inside the single stage.

**Acceptance criteria**
1. Recovery succeeds after more than 4096 unrelated, succeeded or failed `publication-*` or `cutoff-stage-*` directories exist. The test seeds them as empty or malformed directories and a real crash window in the middle.
2. Recovery cost does not grow with the number of old stages. Assert on the number of stages opened, not on timing.
3. Stores written by the current candidate have randomly named stages. Define the behaviour for them:
   - Either keep a clearly named legacy scan with the 4096 cap, and state that cap as a publication-count limit in the receipt.
   - Or add a one-time, journal-verified migration.
   - Do not silently drop recovery for those stores.
4. If the derived stage is absent, refuse with a message distinct from the scan-limit error.
5. Option B from findings-03 (retire the journal once the state record is durable) is the less safe route. It deletes the only authority that recovery adopts, and `windows_publication.rs:509-510` says retained evidence is never unlinked. If cleanup is wanted, remove a stage only when all of these hold:
   - The mutation was `NotChanged`.
   - The pointer still equals `expected_current` under both locks.
   - The state record is durable.

**Hazards**
- Pointer file identities come from NTFS file IDs, which can be reused after deletion. Do not rely on `source_identity` alone to match a journal. Keep the stage-name derivation as a second binding.
- Exact-name stage creation can collide on a retry of the same bundle. Treat that collision as a refusal, not an overwrite.

## 2. Smaller findings still open (`selection_binding.rs`, unchanged)

- **Defect 1: recovery claims the database is readable when it is not.**
  - `recover_logical_db` returns `NothingToRecover` at `:1155-1161` without calling `refuse_displaced_binding`.
  - Add `refuse_displaced_binding(&root, &logical, Some(anchor_id))` before that branch.
  - The criterion is a test with a bound but replaced database file, which must produce the displaced-binding refusal.
- **Defect 2: repairing a torn binding is not crash-safe.**
  - `:1245-1249` and `cutoff_publication.rs:487-490` quarantine the torn record, then write the new one.
  - A crash between the two leaves a pointer with no binding.
  - Fix by writing the new record to a temporary file first, then renaming the old one aside and the new one into place with no-replace semantics. Alternatively, make a missing binding with a unique matching journal recoverable.
- **Defect 3: a cutoff lock with no binding gets "manual recovery required".** `:366-374` should return `Unchanged` when no destination exists and the lock is empty. `load_or_create_binding` writes the lock at `:187` before the binding at `:200`, so a crash in between hits this.
- **Defect 4: receipt wording.**
  - `analytics_selection_sha256` is only checked as 64 hex characters (`sealed_run.rs:189-195`).
  - The receipt must say it is a presence-and-format check, not validated provenance.
- **Minor items.**
  - `quarantine_torn_record` (`:985-989`) uses `fs::rename`, which replaces an existing file.
  - `recover-selection` prints `"recovered"` unconditionally (`main.rs:610`).
  - `NothingToRecover` and `ResumeRequired` reach the CLI as plain error text, not a JSON status.

## 3. Linux immutable selection parity

**Current state: no parity exists, only a fallback.**
- `lib.rs:31` compiles `selection_binding` only for Windows.
- Its identity types are Windows-only. `selection_binding.rs:30-48` rejects anything but `WindowsFileId128`, and `:11` imports the Windows crate unconditionally.
- On other platforms `sealed_run.rs:309` runs `run_locked_with_openrig`, which mutates the database in place.
- `sealed_run.rs:346-356` publishes the cutoff with plain `fs::rename`, which replaces the destination.
- `platform_v1/mod.rs:15-18` compiles `publish_closed_bundle` and `resolve_selection` only for Windows, so neither has a Linux equivalent.
- `linux.rs` has only `rename_absent_in_root` (`:527`, `RenameFlags::NOREPLACE`). That cannot atomically replace an existing pointer.

**Hazard:** do not make the Windows module generic by stubbing its identity types. A `Pointer` that holds a Windows file ID and a volume serial would serialize Linux data into Windows-shaped records. Records written by one platform must not be readable as the other's.

**Bounded work boundary**
1. Define a platform-neutral pointer-and-journal contract in `platform_v1`:
   - a `FileIdentity` variant carrying device and inode on Linux;
   - a `publish_closed_bundle` equivalent built on `renameat2` with `RENAME_EXCHANGE`, or on rename-over a pointer inside a held root fd, plus an fsync of the directory.
2. Make the F1 direct-lookup naming scheme part of that contract, so Linux does not inherit the scan.
3. Move the platform-neutral logic out of `selection_binding.rs` (journal matching, state records, recovery rules) behind that contract.
4. Leave the Windows behaviour and the frozen evidence untouched.

**Acceptance criteria**
- Linux publication replaces an existing pointer only against the expected current identity and digest. A stale expectation refuses and leaves the pointer unchanged.
- The same crash-window tests as Windows pass: a crash after the switch and before the state record, and a torn state record.
- Cutoff publication on Linux is atomic and refuses to replace an unexpected destination.
- **Claim limit:** Linux evidence from WSL is recorded as WSL evidence only, with the kernel and filesystem named. Native Linux certification needs a separate run on a native Linux host. Per `implementation-milestone-20261023.md:16` it stays "not accepted". Do not word a WSL pass as a Linux release.
- ext4 only. The existing `linux.rs:13` fstatfs admission should refuse `/mnt/c` and other 9p or drvfs mounts.

## 4. Historical binary migration and rollback proof

**What exists**
- `migration_v080.rs:2` states "Historical v0.7.0 is not executed here". The 23 passing tests use synthetic schema-3 fixtures only.
- `rollback_to_preserved_schema3` (`rollback.rs:117`) is tested against those fixtures.
- Publication is still the `atomic_existing_manifest_replace_unproved` refusal (`migration_v4/mod.rs:36`).
- The T04 scoped acceptance explicitly lists "historical binary proof" as remaining.

**Identity problem to settle first**
- `git tag` shows only v0.1.1, v0.5.0 and v0.6.0. There is no v0.7.0 tag.
- The workspace version is 0.7.0, and the release commit is `6b75513`.
- The proof must pin the binary by commit hash and by the SHA-256 of the built executable, not by a version string.
- If `6b75513` is the "historical" build, state that explicitly.

**Acceptance criteria**
1. Build the historical binary from `6b75513` in a separate worktree, with a recorded toolchain and lockfile hash. Do not touch the working tree.
2. The historical binary writes a real schema-3 store through its own CLI, using synthetic data only. It does not read anyone's real sessions.
3. The current build stages the migration from a byte copy of that store. Record before and after SHA-256 of the preserved schema-3 database and its key sidecar.
4. Roll back with `rollback_to_preserved_schema3`, then run the historical binary against the restored store. It must open and report the same facts as before the migration.
5. Record the loss receipt for the v4-only facts, and the exact claims the receipt does and does not make.
6. Keep the negative cases:
   - the historical binary refusing the schema-4 store;
   - a rollback with a changed preserved file (`ensure_file_unchanged`, `rollback.rs:329`).
7. Scope: Windows native only unless a Linux historical binary is built natively. Do not generalize.

**Hazard:** a `DuckDB` file opened by a different binary version may checkpoint or rewrite it. Hash the files after every open, and use only copies.

## 5. Proposed work boundaries (safe order)

| Slice | Content | Touches |
|---|---|---|
| A | F1 direct lookup for selection, with legacy-store behaviour, plus defects 1, 3 and 4 and the minor items | `selection_binding.rs`, tests, receipt text |
| B | F1 and defect 2 for cutoff | `cutoff_publication.rs`, shared `quarantine_torn_record` |
| C | Historical binary proof harness on Windows (items 1-7 of section 4) | Test support and evidence only, no product code unless it reveals a bug |
| D | Linux immutable selection contract and publish primitive (section 3) | `platform_v1/linux.rs`, `mod.rs`, then core refactor |

- A and B share the stage-naming contract, so agree it first. C is independent and read-only against product code. D is the largest and must come last, because it reuses the A and B naming scheme.
- Each slice needs its own post-change hash of the exact candidate files.
- **Before editing**, check this tree's modified files:
  - `.agentflow/oracles/v080/T05/frozen.json`
  - `scripts/portfolio-fixture.py`
  - `templates/nao/validate_performance.py`
  - the generated skill directories
  
  The `frozen.json` change needs an explanation. Do not reconcile it silently.
- **Preserve:** the exact FFI surface (no new `unsafe`, `#![deny(unsafe_code)]` stays), the frozen oracles, the failed-benchmark record, and the execution defaults. No further approval is needed for the scope the user has already granted.
