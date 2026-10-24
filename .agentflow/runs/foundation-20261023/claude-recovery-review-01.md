**Verdict: scoped ACCEPT for native Windows. I found no blockers.** This covers the six owned files and nothing else. It does not grant T24, Linux or release acceptance. I did not review `platform_v1` Linux or module wiring. I did not rerun the tests.

**Candidate bytes:** all six source and test hashes match `recovery-candidate-01.json`. The `ffi.rs` sha256 is `31b2d35d…7412`, equal to the HEAD blob, and `git diff --stat` shows no content change. The `M` status on that file is only the CRLF checkout.

## Verified
- **F1 direct lookup:**
  - `publish_candidate` creates the stage and writes the locator before the pointer switch (`selection_binding.rs:1394-1401`). The cutoff path does the same after its journal is durable (`cutoff_publication.rs:298-316`).
  - `write_record` flushes the file, renames it without replacing, and syncs the directory (`:378-433`).
  - Recovery opens one stage (`:1226-1233`; cutoff `:510-517`). The locator only chooses the stage. Each journal must still match source and manifest identity and digest, parent, stage identity, both locks, predecessor and bundle identity. The `== 1` unique-match rule is kept.
- **Old records without a locator:** the fallback scans every stage with no lifetime count (`:1237-1250`; cutoff `:521-535`). It checks `RUN_CANCEL` per entry and holds only one stage at a time. The 4096 limit is gone. The only cap left is 16 journals per stage.
- **Fail-closed on bad locators:** a malformed locator fails the read. A wrong schema or prefix, a changed stage identity, or a missing stage also fails (`:238-247`). None of these falls back to the scan, so a locator can't be used to downgrade the check.
- **Torn or missing binding:** the binding is rebuilt only from the canonical locks, the anchor and a unique matching journal (`:1095-1161`, `:1356-1369`). A crash between quarantine and rewrite is recoverable (`:1271-1286`, test at `windows_selection.rs:484-502`).
- **Displaced database:** `refuse_displaced_binding` runs before `NothingToRecover` (`:1272`). Tests cover a replaced anchor and an unbound database next to a torn binding.
- **Orphan cutoff lock:** an empty lock with no destination, binding or history returns `Unchanged` under the lock (`cutoff_publication.rs:389-409`).
- **Quarantine:** it moves the record into a fresh private `torn-*` directory and checks the retained identity, so nothing is overwritten (`:1073-1093`).
- **CLI:** `NothingToRecover`, `ResumeRequired`, `Reconciled` and `Unchanged` are typed JSON. `published_by_recovery:false` is always set. Cutoff recovery still runs after those outcomes (`main.rs:586-636`).
- **Provenance:** `sealed_run.rs:189-195` states "presence/hex format only; not authenticated source provenance", which is correct.
- **Regression authority:** the absent-destination replay pin and its test are intact. The `ffi.rs` hash is unchanged.

## Findings (all non-blocking, most severe first)
1. **Locator failure paths are untested.** `recovery_stage` (`selection_binding.rs:227-249`) has four refusal branches: malformed JSON, wrong prefix, changed identity, deleted stage.
   - Only the happy path and "locator removed" are exercised (`:1813-1876`).
   - The "valid locator can't replace a forged journal" test exists only for cutoff (`cutoff_publication.rs:741`).
   - **Fix:** add a small table of tests in `windows_selection.rs` that corrupt the `recovery-selection-*.json` file one way at a time and assert a refusal with no state change. Add the forged-journal case for the selection path.
2. **Cancellation is claimed but untested.** The receipt says the pre-locator scan is cancellable. **Fix:** add one test that sets the cancel flag and asserts the fallback aborts without writing.
3. **Pre-existing cutoff file with no custody gives a raw error.** `cutoff_publication.rs:375-376` only handles an absent destination. If the leaf exists but there is no binding, lock or state, `load_recovery_binding` fails on opening the lock (`:217`). If only a lock exists, it fails with "no baseline state proof" (`:220`). Both fail closed, but neither is the typed `Unchanged` the CLI contract implies.
   - **Fix:** when there is no binding and no `cutoff-state-<hash>-*`, return `Unchanged`, after the same lock check if a lock exists.
4. **CLI exit code and wording (`main.rs:~633`).**
   - `resume_required` exits 0, so a script that checks only the exit code reads an incomplete first publication as healthy. **Fix:** exit non-zero, or document that `status` is the contract.
   - `RecoveryDisposition::NothingToRecover` says the legacy database "remains readable" (`selection_binding.rs:256`), but readability is never checked. **Fix:** "no selected-storage custody exists; nothing to recover".
5. **Receipt wording on cost.** "Direct 1 stage open" is true, but recovery is not constant overall.
   - `read_states_mode` (`:547`) and `refuse_displaced_binding` (`:667`) read the whole root directory with no cancel check.
   - The state-chain walk is linear in the number of publications, and it runs on every resolve.
   - The limitations entry covers this. **Fix:** say outright that only stage lookup is independent of retained history.
6. **Missing stage gives a bare OS error (`:244`).** The architecture review asked for a distinct message. **Fix:** add context such as "locator names a missing recovery stage".
7. **Locator write failure has no hint (`:1395-1401`).** The bundle is already built and the error is returned bare. **Fix:** wrap it with the "candidate retained at …" text used by the other failure paths.
8. **Duplicate scan (`:1351-1362`).** `matching_prepared_journals` runs twice when the binding is torn and the pointer unexpected. **Fix:** compute it once.
9. **Accepted limits, worth stating in the receipt.**
   - One foreign stage with more than 16 journals aborts the old-record fallback (`:1186`, cutoff `:469`). That is fail-closed.
   - If a deleted bundle's NTFS id were reused, `rename_absent` would refuse that publication. This is unlikely, because old bundles are never cleaned up.

Items 1 and 3 are the best fixes before any wider acceptance.

I saved no plan file and made no edits.
