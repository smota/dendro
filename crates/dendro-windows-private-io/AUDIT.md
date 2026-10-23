This is an isolated Windows native-I/O crate, prepared for independent integration
review. It is not production admission, T24 acceptance or release approval.

The crate denies unsafe code and unsafe operations in unsafe functions. The sole
exception is src/ffi.rs, copied byte-for-byte from the narrowly approved scratch
review-02 boundary. Its SHA256 is
`31b2d35de0f36b88d665ee03c3f19254a70e2f7dbcf5b68b1debe7af30147412`.
The original source, audit, conditional review and five-file hash verification
remain under .agentflow/runs/v080/T24/native-api-evidence/held-mutation/.
No audited scratch file was changed. The original src/tests.rs is also copied
byte-for-byte. The original unsafe boundary and source-mode/membership checks
remain intact; this document does not extend the reviewer's approval to changed
safe code.

The only unsafe implementation remains the reviewed handle-relative NtCreateFile,
NtQueryInformationFile and NtSetInformationFile boundary, including aligned
buffers, checked lengths, owned handles, synchronous source-mode checks and pending
I/O lifetimes. Directory handles are used only as RootDirectory and by the safe
query library, never by ffi::query/complete. Preserve this directory-mode invariant.
Do not format or edit ffi.rs without a new exact-byte review.

The production safe adapter replaces the scratch private-principal evaluator with
dendro-platform-policy::evaluate_private_object. win-custody's typed dacl parser
runs first: missing/NULL DACLs and every unsupported ACE kind, including deny,
callback and object ACEs, refuse before allow-ACE mapping. There is no parallel
private-grant policy. TrustedInstaller is identified by its exact SID, then denied
as a private owner or grant by the shared evaluator. Its separate ancestor trust
continues to be owned by core admission. The safe verify_inner change and added
typed source identity accessor require independent integration review.

StagingSource still has no public capture, File accessor, raw handle, Debug or
Clone. Its only constructor is held-parent exclusive create_source. Publication
consumes the source on success and error; the compile-fail doctest verifies E0382
for post-publication reuse. Full identities use FILE_ID_INFO's 64-bit volume and
128-bit ID; the native NTFS parent membership reference remains a separate 64-bit
file index. These serial/reference domains are never substituted for each other.

Same-principal residual: held handles prevent substitution, but the current user
(including lower integrity), SYSTEM, Administrators and kernel actors can change
ACLs, links, contents or deletion state between checks and the syscall. No
resistance to those principals is claimed. Publication syncs source data before
one namespace replacement; it provides no directory durability or old-target CAS.
Failure can retain a staging entry. Pending I/O is uncancellable and must execute
under spawn_blocking in a future Tokio integration.

The SE_KERNEL_OBJECT ACE inherited-flag limitation remains: these flags are not
proof of ACE inheritance provenance. Handle inheritance is a different property:
the reviewed boundary creates noninheritable synchronous source handles and checks
them before mutation.

The core publication seam is intentionally absent. VerifiedDirectory::from_held
validates a directory object; it does not establish root ancestry, pathname pinning
or publication authority. The later core wrapper must borrow the actual already
admitted PrivateRoot directory pin and retain that root/pin lifetime, compare all
24 identity bytes with persisted canonical staging/destination records, enforce
canonical sentinel and single-writer locks, execute blocking publication correctly,
and define bounded busy-target retry/retention behavior. It must never reopen a
mutable pathname as authorization. Windows core rename remains typed refusal.

The scoped confinement check is:

    python -X utf8 .agentflow/runs/v080/T24b/production-helper/check-unsafe.py

It checks the two new crates and the owned core integration Rust files, rejects
unsafe tokens/lint escapes outside ffi.rs, and pins ffi.rs to its audited hash.
It does not claim to inspect generated code, dependencies or unrelated crates;
release CI adoption and independent review remain pending.
