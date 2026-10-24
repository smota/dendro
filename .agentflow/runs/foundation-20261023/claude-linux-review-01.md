**Verdict: scoped ACCEPT** of the frozen subset: `platform_v1/linux.rs`, `platform_v1/linux_publication.rs` and `platform_v1/mod.rs`. I found no blocker. I did not run builds or tests, and I only read files.

- **Hashes:** `sha256sum` of the three files matches `linux-primitives-candidate-01.json` exactly.
- **Working tree:** `linux_publication.rs` is untracked, and `linux.rs` and `mod.rs` are modified. Commit them before relying on the hashes.
- **Tests:** I found 3 `#[test]` in `linux.rs` and 5 in `linux_publication.rs`, which is 8. I did not locate the "6 policy" tests, and I did not rerun any of them.

## What holds

- **Unsafe and FFI:** There is no `unsafe` or FFI. All three files carry `#![deny(unsafe_code)]`, and the Linux code uses only safe rustix calls.
- **Held-root, ext4, owner/mode:**
  - `check_anchor` (`linux.rs:259-313`) re-opens every ancestor relative to the previous pin and compares identity.
  - It then re-runs the ext4 filesystem check, the owner/mode policy (0700 for directories, 0600 for files) and the ACL check on the held fd.
  - `PrivateRoot::create_file` and `rename_absent_same_directory` now call it (`mod.rs:128`, `:486`).
- **ACL check fails closed (`linux.rs:234-250`):** An ACL attribute that is present, oversized or unreadable (including `EOPNOTSUPP`) refuses. Only `ENODATA` allows. It is applied to roots, files, rename sources and lock sentinels.
- **Linux identity records:**
  - `SelectionIdentity { device, inode }` has `deny_unknown_fields` (`linux_publication.rs:29-51`), and the `TryFrom` accepts only `FileIdentity::Unix`.
  - The test at `:970` confirms a Windows-shaped record is rejected.
  - The Windows struct is likewise strict (`windows_publication.rs:30`).
- **Compare-and-swap (`linux_publication.rs:458-474`, `:577-584`):**
  - `check_current` runs before, and again after, the journal is written. `publish_checked` re-reads the pointer a third time just before the rename.
  - Expected identity and a SHA-256 of the raw pointer bytes are both required.
  - With no expected pointer it uses `NOREPLACE`. With one it uses `EXCHANGE`, which keeps the displaced pointer in the staging directory.
  - A stale or wrong-digest expectation refuses with `NotChanged` (test at `:844`).
- **Mutation reporting:** `published = true` is set immediately after the rename succeeds (`:602`). From then on, every error path in `publish_locked` reports `ReplacedUnconfirmed` (`:716-719`). An unexpected occupant of the manifest name leaves the destination untouched.
- **Locks:** Writer and publisher locks use `with_exclusive_lock` with a canonical identity. Aliasing the two locks, or either lock with the manifest name, is refused (`:504-510`). Both are taken before `publish_locked`.
- **Evidence and durability:**
  - The source file and a journal with `prepared` and `selected` records are fsynced, along with the staging and destination directories.
  - Failed attempts are never unlinked, and the old pointer survives an `EXCHANGE`.
  - `power_loss_guarantee: false` is truthful.
- **Old read pins:** An old reader's pin on the previous pointer stays coherent after a replace (test at `:844`).

## Issues (none blocking)

**P3 (low):**
1. **Windows-parity gap on aliasing:** `check_roots` (`linux_publication.rs:206-230`) rejects a bundle that aliases the destination or staging directory. It does not reject `expected_staging == expected_destination`. Windows has the same gap (`windows_publication.rs:232`). Add the check on both.
2. **Public single-file publisher (`:558-623`):** `LinuxPublicationSource::publish_checked` is public. It does not enforce that the caller holds the writer and publisher locks, and it does not check the destination root's identity or device. Its comment says locks serialize writers, but only convention enforces that. Say so in the cutoff caller's contract, or take a lock proof as a parameter.
3. **Path-based directory listings (`:298`, `:434`):** Both `read_dir` calls go through the path, not the held fd. The `:298` call comes after the last `check_anchor`. A same-uid directory swap there would pass the count check. `resolve_selection` would still catch it later through the bundle identity bound in the manifest. Use `rustix::fs::Dir` on the held fd.
4. **Silent selector aliasing (`mod.rs:279-289`):** `StorageRootChoice::LocalAppData` and `UserProfile` silently map to the XDG/HOME root. Returning a refusal would be more honest, since the comment calls this API compatibility only.
5. **Magic number (`linux.rs:122`):** `0xEF53` also matches ext2 and ext3, and there is no mount-option check. Word it as "ext2/3/4 family" when stating the claim.
6. **Ancestor ACLs and the empty limitations list (`linux.rs:218-226`, `:279`):**
   - Ancestor ACLs are never read. Only the mask-reflecting group bits cover write ACLs, and a sticky ancestor passes regardless.
   - `limitations` is now `Vec::new()`, so it no longer discloses this.
   - Keep a note such as `ancestor_acl_not_read`.
7. **Weaker pre-open probe (`linux.rs:702`, `:759`):** A `RDWR|NONBLOCK` open of an unverified object replaced the earlier `O_PATH` probe. It has no `O_NOCTTY`. Only same-uid FIFOs or sockets can be planted, so this is a small regression.
8. **Exchange mismatch handling (`linux_publication.rs:603-612`):** The post-exchange mismatch error is truthful, but the unexpected object stays live as the pointer and nothing is auto-restored. Say explicitly that recovery must start from the journal's `source` name, where the displaced object now sits.

## Acceptance limits

- **Evidence:** Only the 8 + 6 passes on WSL ext4 are claimed. Record the kernel, filesystem and that the run was WSL. This is not native-Linux, hosted or release evidence. `incomplete_oracle_cases()` still lists T24-11 through T24-52 as not complete, and `mod.rs:7` correctly says it is not T24 acceptance.
- **Missing test coverage:**
  - a torn or missing `selected` journal record (the architecture's crash-window criterion), and a crash between the rename and the journal write
  - a file-level ACL, or group/other mode, on a bundle file or the pointer
  - a hardlink, symlink or undeclared subdirectory inside the bundle
  - a hostile swap that triggers the `exchanged_pointer_changed` path
  - a staging/destination alias
- **Recovery:** No recovery classifier is implemented. Recovery means inspecting the actual pointer and the retained journal, as on Windows. Windows also retries up to 3 times, and Linux makes a single attempt.
- **Trust residual:** Locks and read pins are cooperative `flock`. A same-uid hostile process can mutate content, ACLs, names or the pointer in place. Linux has no inode-checked rename. Root and host administrators stay trusted. Immutability is therefore cooperative, not enforced.
- **Out of scope here:** concurrent consumers, migration code, the Windows module, and any database-level acceptance of the bundle.
