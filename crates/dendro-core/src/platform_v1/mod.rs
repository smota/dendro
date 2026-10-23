//! Scoped native private-I/O primitives. This is not T24 acceptance.
//! Windows identity retains FILE_ID_INFO: 64-bit volume plus 128-bit file ID.
//! Admission is restricted to the proved local NTFS profile.
//! Kernel ACE flag bits are not inheritance provenance.
#![deny(unsafe_code)]

#[cfg(not(dendro_platform_v080_harness))]
pub use crate::domain_v1::{CreationMarker, ProcessIdentity};

#[cfg(target_os = "linux")]
mod linux;
mod policy;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
mod windows_publication;
#[cfg(windows)]
pub use windows_publication::*;

pub use policy::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformRefusal {
    NativeProofNotAdmitted,
}

/// Compiled capability only. This is not runtime filesystem admission,
/// a native acceptance lane, or an oracle pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivateIoScope {
    CapabilityCompiled,
    NotInThisSlice,
}

pub fn check_private_io_admission(platform: NativePlatform) -> PrivateIoScope {
    match platform {
        NativePlatform::Windows | NativePlatform::Linux => PrivateIoScope::CapabilityCompiled,
        NativePlatform::Macos => PrivateIoScope::NotInThisSlice,
    }
}

pub fn current_native_platform() -> NativePlatform {
    if cfg!(windows) {
        NativePlatform::Windows
    } else if cfg!(target_os = "linux") {
        NativePlatform::Linux
    } else {
        NativePlatform::Macos
    }
}

/// Cases this slice does not complete. Policy helpers may still describe them.
pub fn incomplete_oracle_cases() -> &'static [&'static str] {
    &[
        "T24-11", "T24-14", "T24-15", "T24-17", "T24-18", "T24-21", "T24-23", "T24-25", "T24-26",
        "T24-27", "T24-29", "T24-30", "T24-32", "T24-33", "T24-38", "T24-39", "T24-41", "T24-46",
        "T24-47", "T24-48", "T24-49", "T24-52",
    ]
}

#[derive(Debug)]
pub struct PrivateRoot {
    path: std::path::PathBuf,
    meta: ObservedMetadata,
    limitations: Vec<&'static str>,
    #[cfg(windows)]
    _pins: Vec<std::fs::File>,
    #[cfg(target_os = "linux")]
    _pins: Vec<std::os::fd::OwnedFd>,
}

impl PrivateRoot {
    /// Borrow the admitted held directory for the separate cutoff publication
    /// route. The returned handle never comes from a reopened path.
    #[cfg(windows)]
    pub(crate) fn held_directory_file(&self) -> Result<&std::fs::File, PlatformError> {
        self.check_anchor()?;
        self._pins.last().ok_or_else(|| {
            PlatformError::Rejected(Verdict::refuse("ancestor_pin_absent", "T24-45"))
        })
    }
    /// Revalidate both the retained root and its current namespace entry.
    #[cfg(windows)]
    pub fn check_anchor(&self) -> Result<(), PlatformError> {
        windows::check_anchor(self)
    }

    /// Flush the held directory; unsupported native flushes remain errors.
    #[cfg(windows)]
    pub fn sync_dir(&self) -> Result<(), PlatformError> {
        windows::sync_directory(self)
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    pub fn metadata(&self) -> &ObservedMetadata {
        &self.meta
    }

    pub fn limitations(&self) -> &[&'static str] {
        &self.limitations
    }

    pub fn create_file(&self, name: &ChildName) -> Result<VerifiedFile, PlatformError> {
        #[cfg(windows)]
        {
            self.check_anchor()?;
            windows::create_private_file(&self.path, name)
        }
        #[cfg(target_os = "linux")]
        {
            let root_fd = self._pins.last().ok_or_else(|| {
                PlatformError::Rejected(Verdict::refuse("ancestor_pin_absent", "T24-45"))
            })?;
            linux::create_private_file(root_fd, &self.path, name)
        }
        #[cfg(not(any(windows, target_os = "linux")))]
        {
            let _ = name;
            Err(PlatformError::Rejected(Verdict::refuse(
                "platform_unsupported",
                "macos_private_create_not_admitted",
            )))
        }
    }
}

#[derive(Debug)]
pub struct VerifiedFile {
    path: std::path::PathBuf,
    file: std::fs::File,
    meta: ObservedMetadata,
    limitations: Vec<&'static str>,
}

impl VerifiedFile {
    /// Transfer the verified handle. The caller must retain its parent root.
    pub fn into_file(self) -> std::fs::File {
        self.file
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    pub fn file(&self) -> &std::fs::File {
        &self.file
    }

    pub fn file_mut(&mut self) -> &mut std::fs::File {
        &mut self.file
    }

    pub fn metadata(&self) -> &ObservedMetadata {
        &self.meta
    }

    pub fn limitations(&self) -> &[&'static str] {
        &self.limitations
    }

    /// Token for a regular single-link file this handle already verified.
    /// A later `reopen_metadata` of the same path is not a substitute.
    pub fn canonical_identity(&self) -> Result<CanonicalIdentity, PlatformError> {
        if self.meta.kind != ObjectKind::RegularFile || self.meta.links != 1 {
            return Err(PlatformError::Rejected(Verdict::refuse(
                "lock_not_regular_sentinel",
                "T24-48",
            )));
        }
        Ok(canonical_identity_from_persisted_record(self.meta.identity))
    }
}

/// Create one exact, absent child in a retained private Windows parent.
#[cfg(windows)]
pub fn create_private_directory(
    parent: &PrivateRoot,
    name: &ChildName,
) -> Result<PrivateRoot, PlatformError> {
    windows::create_private_directory(parent, name)
}

/// Atomically create a fresh protected private directory under the held parent.
#[cfg(windows)]
pub fn create_unique_private_directory(
    parent: &PrivateRoot,
    prefix: &str,
) -> Result<PrivateRoot, PlatformError> {
    windows::create_unique_private_directory(parent, prefix)
}

#[cfg(windows)]
pub fn sync_directory(path: &std::path::Path) -> Result<(), PlatformError> {
    open_existing_private_root(path)?.sync_dir()
}

/// Metadata-only scan. Every traversed ancestor remains pinned without delete sharing.
#[cfg(windows)]
pub fn source_tree_contains_identity(
    root: &std::path::Path,
    target: &FileIdentity,
) -> Result<bool, PlatformError> {
    windows::source_tree_contains_identity(root, target)
}

pub fn ancestor_replacement_facts() -> Result<Vec<String>, PlatformError> {
    #[cfg(windows)]
    {
        windows::ancestor_replacement_facts()
    }
    #[cfg(not(windows))]
    {
        Ok(Vec::new())
    }
}

/// Explicit Windows storage parent selection. Every choice is validated and pinned
/// before creation; a refusal never tries another candidate.
#[cfg(windows)]
#[derive(Debug, Clone)]
pub enum StorageRootChoice {
    LocalAppData,
    UserProfile,
    Explicit(std::path::PathBuf),
}

#[cfg(windows)]
pub fn create_user_private_root_with_choice(
    prefix: &str,
    choice: &StorageRootChoice,
) -> Result<PrivateRoot, PlatformError> {
    windows::create_user_private_root_with_choice(prefix, choice)
}

pub fn create_user_private_root(prefix: &str) -> Result<PrivateRoot, PlatformError> {
    #[cfg(windows)]
    {
        windows::create_user_private_root(prefix)
    }
    #[cfg(target_os = "linux")]
    {
        linux::create_user_private_root(prefix)
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = prefix;
        Err(PlatformError::Rejected(Verdict::refuse(
            "platform_unsupported",
            "macos_private_create_not_admitted",
        )))
    }
}

pub fn open_existing_private_root(path: &std::path::Path) -> Result<PrivateRoot, PlatformError> {
    #[cfg(target_os = "linux")]
    {
        linux::open_existing_private_root(path)
    }
    #[cfg(windows)]
    {
        windows::open_existing_private_root(path)
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = path;
        Err(PlatformError::Rejected(Verdict::refuse(
            "platform_unsupported",
            "macos_private_create_not_admitted",
        )))
    }
}

pub fn open_existing_private_file(
    root: &PrivateRoot,
    name: &ChildName,
) -> Result<VerifiedFile, PlatformError> {
    #[cfg(target_os = "linux")]
    {
        let root_fd = root._pins.last().ok_or_else(|| {
            PlatformError::Rejected(Verdict::refuse("ancestor_pin_absent", "T24-45"))
        })?;
        linux::open_existing_private_file(root_fd, &root.path, name)
    }
    #[cfg(windows)]
    {
        root.check_anchor()?;
        windows::open_existing_private_file(root.path(), name)
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = (root, name);
        Err(PlatformError::Rejected(Verdict::refuse(
            "platform_unsupported",
            "macos_private_create_not_admitted",
        )))
    }
}

/// Read-only, deny-write/delete custody for a verified legacy file. A writable
/// verified opener cannot be held while DuckDB opens the same file read-only.
#[cfg(windows)]
pub fn open_existing_private_read_pin(
    root: &PrivateRoot,
    name: &ChildName,
) -> Result<std::fs::File, PlatformError> {
    root.check_anchor()?;
    windows::open_private_read_pin(root, name, true)
}

pub fn reopen_metadata(path: &std::path::Path) -> Result<ObservedMetadata, PlatformError> {
    #[cfg(windows)]
    {
        let file = windows::open_producer_metadata(path)?;
        windows::metadata_from_handle(&file)
    }
    #[cfg(target_os = "linux")]
    {
        let file = linux::open_metadata(path)?;
        linux::metadata_from_handle(&file)
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = path;
        Err(PlatformError::Rejected(Verdict::refuse(
            "platform_unsupported",
            "macos_metadata_not_admitted",
        )))
    }
}

pub fn metadata_from_handle(file: &std::fs::File) -> Result<ObservedMetadata, PlatformError> {
    #[cfg(windows)]
    {
        windows::metadata_from_handle(file)
    }
    #[cfg(target_os = "linux")]
    {
        linux::metadata_from_handle(file)
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = file;
        Err(PlatformError::Rejected(Verdict::refuse(
            "platform_unsupported",
            "macos_metadata_not_admitted",
        )))
    }
}

/// Exclusive lock of one regular sentinel inside `root`.
/// `expected` must be that sentinel's canonical token, not a fresh stat of the path.
/// The body is not re-checked. A same-principal unlink plus a new persisted
/// identity can lock the replacement while this call still holds the old inode.
/// That boundary is `LOCK_RESIDUAL_SAME_PRINCIPAL`. T24-48 stays incomplete.
pub fn with_exclusive_lock<R>(
    root: &PrivateRoot,
    sentinel: &ChildName,
    expected: &CanonicalIdentity,
    body: impl FnOnce(&std::fs::File) -> Result<R, PlatformError>,
) -> Result<R, PlatformError> {
    #[cfg(any(windows, target_os = "linux"))]
    {
        lock::with_exclusive_lock(root, sentinel, expected, body)
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = (root, sentinel, expected, body);
        Err(PlatformError::Rejected(Verdict::refuse(
            "platform_unsupported",
            "macos_lock_not_admitted",
        )))
    }
}

/// True while the sentinel name still has a conflicting byte-range or flock lock.
/// Does not take the process-local registry.
pub fn os_exclusive_lock_conflicts(
    root: &PrivateRoot,
    sentinel: &ChildName,
) -> Result<bool, PlatformError> {
    #[cfg(any(windows, target_os = "linux"))]
    {
        lock::os_exclusive_lock_conflicts(root, sentinel)
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = (root, sentinel);
        Err(PlatformError::Rejected(Verdict::refuse(
            "platform_unsupported",
            "macos_lock_not_admitted",
        )))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenameOutcome {
    /// Linux cannot rename a file descriptor. This string stays on success.
    pub residual_same_principal_race: &'static str,
}

/// Rename `from` to an absent `to` inside the held root.
/// The source must be the verified regular object named by `expected`.
/// On a post-rename mismatch the error note is `moved_unverified_object_at_destination`
/// and the destination is left for recovery. A pre-rename mismatch uses `not_renamed`.
pub fn rename_absent_same_directory(
    root: &PrivateRoot,
    expected: &CanonicalIdentity,
    from: &ChildName,
    to: &ChildName,
) -> Result<RenameOutcome, PlatformError> {
    #[cfg(windows)]
    {
        let _ = expected;
        let from_path = root.path.join(from.as_str());
        let to_path = root.path.join(to.as_str());
        windows::rename_absent_same_directory(&from_path, &to_path)
    }
    #[cfg(target_os = "linux")]
    {
        let root_fd = root._pins.last().ok_or_else(|| {
            PlatformError::Rejected(Verdict::refuse("ancestor_pin_absent", "T24-45"))
        })?;
        let note = linux::rename_absent_in_root(root_fd, &root.meta.identity, expected, from, to)?;
        Ok(RenameOutcome {
            residual_same_principal_race: note,
        })
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = (root, expected, from, to);
        Err(PlatformError::Rejected(Verdict::refuse(
            "platform_unsupported",
            "macos_rename_not_admitted",
        )))
    }
}

pub fn publish_selection_manifest() -> Result<PublicationReceipt, PlatformError> {
    Err(PlatformError::Rejected(Verdict::refuse(
        "atomic_existing_manifest_replace_unproved",
        "T24-26,T24-27,T24-29,T24-32",
    )))
}

#[cfg(any(windows, target_os = "linux"))]
mod lock {
    use super::*;
    use std::collections::HashSet;
    use std::fs::File;
    use std::sync::Mutex;

    struct LockRelease(CanonicalIdentity);

    impl Drop for LockRelease {
        fn drop(&mut self) {
            if let Ok(mut guard) = registry().lock() {
                guard.remove(&self.0);
            }
        }
    }

    fn registry() -> &'static Mutex<HashSet<CanonicalIdentity>> {
        static HELD: std::sync::OnceLock<Mutex<HashSet<CanonicalIdentity>>> =
            std::sync::OnceLock::new();
        HELD.get_or_init(|| Mutex::new(HashSet::new()))
    }

    fn open_lock_file(
        root: &PrivateRoot,
        sentinel: &ChildName,
        expected: &CanonicalIdentity,
    ) -> Result<File, PlatformError> {
        #[cfg(windows)]
        {
            root.check_anchor()?;
            let verified = open_existing_private_file(root, sentinel)?;
            let file = verified.into_file();
            let meta = metadata_from_handle(&file)?;
            if meta.kind != ObjectKind::RegularFile || meta.links != 1 {
                return Err(PlatformError::Rejected(Verdict::refuse(
                    "lock_not_regular_sentinel",
                    "T24-48",
                )));
            }
            if meta.identity != expected.identity() {
                return Err(PlatformError::Rejected(Verdict::refuse(
                    "lock_identity_changed",
                    "T24-48",
                )));
            }
            Ok(file)
        }
        #[cfg(target_os = "linux")]
        {
            let root_fd = root._pins.last().ok_or_else(|| {
                PlatformError::Rejected(Verdict::refuse("ancestor_pin_absent", "T24-45"))
            })?;
            linux::open_lock_sentinel(root_fd, &root.meta.identity, sentinel, expected)
        }
    }

    fn recheck_lock(
        root: &PrivateRoot,
        sentinel: &ChildName,
        expected: &CanonicalIdentity,
        held: &File,
    ) -> Result<(), PlatformError> {
        #[cfg(windows)]
        {
            let path = root.path.join(sentinel.as_str());
            let file = windows::open_nofollow(&path)?;
            let meta = metadata_from_handle(&file)?;
            let held_meta = metadata_from_handle(held)?;
            if meta.identity != expected.identity() || held_meta.identity != expected.identity() {
                return Err(PlatformError::Rejected(Verdict::refuse(
                    "lock_identity_changed",
                    "T24-48",
                )));
            }
            Ok(())
        }
        #[cfg(target_os = "linux")]
        {
            let root_fd = root._pins.last().ok_or_else(|| {
                PlatformError::Rejected(Verdict::refuse("ancestor_pin_absent", "T24-45"))
            })?;
            linux::recheck_lock_name(root_fd, sentinel, expected, held)
        }
    }

    pub fn with_exclusive_lock<R>(
        root: &PrivateRoot,
        sentinel: &ChildName,
        expected: &CanonicalIdentity,
        body: impl FnOnce(&File) -> Result<R, PlatformError>,
    ) -> Result<R, PlatformError> {
        let mut guard = registry().lock().map_err(|_| {
            PlatformError::Rejected(Verdict::refuse("lock_registry_poisoned", "lock_registry"))
        })?;
        if guard.contains(expected) {
            return Err(PlatformError::Rejected(Verdict::refuse(
                "same_process_lock_held",
                "T24-48",
            )));
        }
        let file = open_lock_file(root, sentinel, expected)?;
        let mut rw = fd_lock::RwLock::new(file);
        let write = rw.try_write().map_err(|err| {
            if err.kind() == std::io::ErrorKind::WouldBlock {
                PlatformError::Rejected(Verdict::refuse("lock_contended", "T24-23"))
            } else {
                PlatformError::Io(err)
            }
        })?;
        if let Err(err) = recheck_lock(root, sentinel, expected, &write) {
            drop(write);
            drop(rw);
            return Err(err);
        }
        guard.insert(*expected);
        drop(guard);
        let _release = LockRelease(*expected);
        let result = body(&write);
        drop(write);
        drop(rw);
        drop(_release);
        result
    }

    pub fn os_exclusive_lock_conflicts(
        root: &PrivateRoot,
        sentinel: &ChildName,
    ) -> Result<bool, PlatformError> {
        #[cfg(windows)]
        let file = { windows::open_for_lock(&root.path.join(sentinel.as_str()))? };
        #[cfg(target_os = "linux")]
        let file = {
            let root_fd = root._pins.last().ok_or_else(|| {
                PlatformError::Rejected(Verdict::refuse("ancestor_pin_absent", "T24-45"))
            })?;
            File::from(
                rustix::fs::openat(
                    root_fd,
                    sentinel.as_str(),
                    rustix::fs::OFlags::RDWR
                        .union(rustix::fs::OFlags::NONBLOCK)
                        .union(rustix::fs::OFlags::NOFOLLOW)
                        .union(rustix::fs::OFlags::CLOEXEC),
                    rustix::fs::Mode::empty(),
                )
                .map_err(linux::map_errno)?,
            )
        };
        let mut rw = fd_lock::RwLock::new(file);
        let held = {
            let attempted = rw.try_write();
            match attempted {
                Ok(guard) => {
                    drop(guard);
                    false
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => true,
                Err(err) => return Err(PlatformError::Io(err)),
            }
        };
        drop(rw);
        Ok(held)
    }
}
