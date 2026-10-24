**Scope verdict:** accept the publication handoff design. Reject the candidate as it stands until finding 1 is fixed and the receipt is regenerated. I ran no builds, only a read-only `rustfmt --check` and a few `gh api` lookups.

## Blocking

1. **`cargo fmt --all --check` fails on `migration_v4/mod.rs`:21–30, so the first CI gate (`foundation-ci-verify.sh:138`) would fail.**
   - `rustfmt --check` wants `pub use publication` before `pub use revision`. The new `pub use read_validation::…` is also out of order.
   - `publication.rs`, the test file and both fixture `.rs` files pass `rustfmt --check`.
   - `mod.rs` changed from +4 to +6 lines while I was reviewing, so another agent is editing it. Re-run the format check after that work lands.

2. **The compact receipt (`historical-migration-proof.json`) is stale and weakly bound.**
   - `publication_api_files["…/mod.rs"]` is `1e61469f…`. The current file hashes to `f043df3d…` (LF-normalised) or `25399783…` (raw), so it does not match.
   - The receipt records no candidate commit.
   - `complete_receipt_path` and `regression_log` point to `C:\Users\samue\Documents\…`, outside the repo. The hashes cannot be verified from the repo, and the logs aren't committed.
   - `/home/sam/...` paths are embedded in the receipt.
   - Treat it as WSL-only, non-native evidence. It honestly says `execution_environment: WSL` and `native_hosted_linux_acceptance: false`.
   - Regenerate it after the final commit, or drop `publication_api_files` from it and rely on the CI-side blob-hash check.

## Handoff API (`publication.rs`)

3. **`publication.rs:185–212` — the validated stage database is not bound to the held handles.**
   - `reuse_validated` checks the staged DB by path and reads it through DuckDB.
   - The held files are opened afterwards, and the only hash taken is in `capture`.
   - A swap in that window would be captured and held as if it had been validated.
   - Fix: hash `stage.duckdb` (and the seal, identity record and preimage files) before and after validation and compare. Or re-validate through the held handle.
   - Severity is limited, since the root is private and owned by the user.

4. **`publication.rs:101–127` — `verify_unchanged` does not re-check the original source DB or key.**
   - It checks only the WAL files, the held-file identity and size, and a re-hash by name of the five stage files.
   - `SourceGuard` is dropped at return, and `source_database_sha256` is never re-verified.
   - The doc comment at :38–40 implies more than this.
   - Either keep the guard and call `ensure()` inside `verify_unchanged`, or document that source immutability is only checked inside `validate`.

5. **`publication.rs:85–99` — `&File` handles are exposed.**
   - `&File` implements `Write` and `Read`. Confirm `open_existing_private_file` opens read-only on both platforms. The Linux file is excluded from this review, and `windows.rs:1012` has a write branch.
   - The cursor is shared, so a publisher must seek to 0 before copying.
   - Indexing `held_files[0..4]` and the `zip` in `verify_unchanged` depend on array order matching. A small struct or array would be safer.

6. **`publication.rs:59–67` — `key()` and `preimage_key()` return the same file.**
   - The API reads as if there were a separate staged key. Document that the key is shared, or drop `key()`.

## Tests (`migration_publication_v080.rs`)

7. **`migration_publication_v080.rs:118–137` — the same-file-edit test can pass without detecting anything.**
   - If `Connection::open` or the `UPDATE` fails, `changed` is false and the test only asserts `verify_unchanged().is_ok()`.
   - On Linux nothing blocks the competing write, so `changed` should be asserted true with `#[cfg(not(windows))]`.
   - Also assert that the DB bytes actually changed. A no-op `UPDATE` plus checkpoint may leave the file identical.

8. **`migration_publication_v080.rs:143–152` — the "untrusted fields" assertions are trivially true.**
   - `ne("invented.duckdb")` and `ne("untrusted")` hold for any output.
   - There is no negative test that tampers with a real stage file (stage DB, preimage, seal or identity record) and expects `validate` to refuse.
   - There is also no test for swapping a preimage file, or for the WAL refusal.
   - The foreign-key refusal case is good.

9. Fixtures leave private stage roots in the real user data directory on Linux, because only the temp directory is removed. The tests use synthetic data, so this is pollution rather than a leak.

## Historical proof (`historical_migration_proof.py`, `scripts/fixtures`)

10. **`historical_migration_proof.py:92–97` — both checkouts share one `CARGO_TARGET_DIR`.**
    - The historical and current workspaces both contain `dendro-core` and share a target directory, so cargo fingerprint collisions are possible.
    - The claim that `historical_openers` links the immutable historical core rests on that.
    - Use separate target directories per checkout.
    - The probe prints `historical_schema: SCHEMA_VERSION`, but the script never asserts it equals 3. Add that assertion (`historical_openers.rs:74`).

11. **Identity of the historical source (`:66, :78–79`).**
    - The pin is a full 40-hex SHA via `git archive`, not a tag, which is correct.
    - Only `Cargo.lock` and `storage.rs` hashes are recorded. CI cross-checks them against `git show PIN:path`, which is good.
    - Also record `git rev-parse PIN^{tree}` and a hash of the whole archive.
    - `dendro 0.7.0` is a weak binary identity because it is only a version string.

12. **`:68–77` — the overlay of `crates/` onto the `HEAD` archive cannot delete files.**
    - Stale `HEAD` files survive when they were removed in the working tree.
    - It is also unclear whether non-`src/*.rs` files such as manifests, examples and tests are covered by hashes.
    - CI is safe because it requires a clean tree, but local runs can prove a tree that differs from the working tree.

13. **Parts of the rollback evidence are weaker than they look.**
    - `rollback_to_preserved_schema3` only computes a receipt and never restores. Schema3 is never written, so "historical report same after rollback" holds because the bytes are identical.
    - That makes the check valid as proof that the source is preserved, but not as proof that rollback works.
    - The pristine and loss-rollback flags come from the code under test (`full_continuity_claim`, `unresolved_loss`). The independent parts are the hashes, the hard-coded `excluded_fact_count == 2`, and the old CLI/probe refusals.
    - `byte_identical` is recorded but not asserted. Assert it in `:136`, or state that only canonical content is checked.
    - The old-CLI-produced schema3 and the old CLI/probe refusals are non-circular and good.

## CI (`foundation.yml`, `foundation-ci-verify.sh`)

14. **Candidate scope.** Only committed and pushed content runs. Everything here is currently untracked or modified, and the workflow triggers only on `push` to `codex/foundation-completion`, a PR to `main`, or `workflow_dispatch`. For PRs the "candidate" is the merge commit (`GITHUB_SHA`), not the PR head. Label it that way in the receipts.

15. **Action pins.** I resolved all three SHAs with `gh api`:
    - `actions/checkout` `11d5960a…` is `v4.4.0`.
    - `actions/cache` `0057852b…` is `v4.3.0`.
    - `actions/upload-artifact` `ea165f8d…` is `v4.6.2`.
    - All are real commits. Put the exact tag in the comments instead of `# v4`.

16. **Native ext4, pinned Rust and secrets are handled well.**
    - Prepare refuses WSL, requires `x86_64`, and requires `ext4`, owner and mode 700, and `HEAD == GITHUB_SHA`.
    - `RUSTUP_TOOLCHAIN` is pinned to 1.98.1, with a `rustc -Vv` release check.
    - `permissions: contents: read`, `persist-credentials: false`, and no `pull_request_target`.
    - Collect uploads only top-level `*.stdout`, `*.stderr` and `receipt.json`, plus the logs. No DB, key or binary is uploaded.
    - The synthetic table dumps in the probe `*.stdout` files are the only content-bearing uploads.
    - Minor: `cargo test` runs with the runner's real `HOME`. Gate-phase `HOME` and `XDG_DATA_HOME` are not isolated.

17. **Failures are not claimed as passes.**
    - `pipefail` plus the `|| status=$?` pattern propagates gate exits.
    - The historical step is skipped when gates fail.
    - `historical-ci-validation.json` is only written after assertions on native-linux, 8 refusal cases and source-blob hashes.
    - Gaps: `exec > >(tee …)` followed by `exit` can truncate `*.log`. There is no consolidated verdict file. `*.exit` files contain `0` for passes and could be misread.
    - `native_hosted_linux_acceptance` is hard-coded `False` in the script, which is honest but never flipped. The wrapper `historical-ci-validation.json` carries the native claim, so keep the two consistent.

18. **Cache.** The `actions/cache` step restores `registry` and `git` directories. Extracted sources are not re-verified against `Cargo.lock` checksums on restore. The risk is low with `--locked`. Consider caching only `registry/cache`, or skipping the cache for this evidence run.

19. **Line endings.**
    - The working-tree files are CRLF, `core.autocrlf=true`, and there is no `.gitattributes`.
    - Hashes in the receipt match only after LF normalisation.
    - `git add` on this machine will normalise to LF. A commit with `autocrlf=false` would break `foundation-ci-verify.sh`.
    - Add `.gitattributes` with `*.sh *.py *.rs *.yml text eol=lf`.

## Confirmations

- Frozen `T04/frozen.json` hashes to `b7e80ab7…`, matching `ORACLE`. The T04 fixture files are not modified.
- The user's note says existing frozen evidence is unchanged, but `git status` shows `.agentflow/oracles/v080/T05/frozen.json` as `M`. My `git diff` on it printed nothing, because the diff output was cut off by the earlier failed command. Someone should confirm that is line-ending or mode noise and not an oracle rewrite.
- No claim of release acceptance or of WSL being native is made here.
