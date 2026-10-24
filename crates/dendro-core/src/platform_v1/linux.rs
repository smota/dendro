//! Linux private primitives. Modes are set with fchmod on the held fd.
//! This module never changes the process umask and never unlinks by path.
//! Extended POSIX ACLs fail closed; admission is ext4 on the held fd.
#![deny(unsafe_code)]

use std::fs::File;
use std::io::{self, Read};
use std::os::fd::{AsFd, OwnedFd};
use std::path::{Component, Path, PathBuf};

use rand::RngCore;
use rustix::fs::{
    fchmod, fgetxattr, fstat, fstatfs, mkdirat, open, openat, renameat_with, Mode, OFlags,
    RenameFlags,
};
use rustix::io::Errno;

use super::policy::*;
use super::{PlatformError, PrivateRoot, VerifiedFile};

const DIR_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC)
    .union(OFlags::NONBLOCK);
/// `O_PATH` supports `fstat` and does not open the object for I/O.
const META_FLAGS: OFlags = OFlags::PATH.union(OFlags::NOFOLLOW).union(OFlags::CLOEXEC);
const LOCK_FLAGS: OFlags = OFlags::RDWR
    .union(OFlags::NONBLOCK)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC)
    .union(OFlags::NOCTTY);

pub(super) fn map_errno(err: Errno) -> PlatformError {
    if err == Errno::LOOP {
        PlatformError::Rejected(Verdict::refuse("link_policy_or_pin_absent", "T24-16"))
    } else {
        PlatformError::from(io::Error::from(err))
    }
}

fn pin_errno(err: Errno) -> PlatformError {
    if err == Errno::NOENT {
        PlatformError::Rejected(Verdict::refuse("no_trustworthy_root", "T24-40"))
    } else if err == Errno::NOTDIR {
        PlatformError::Rejected(Verdict::refuse("ancestor_not_directory", "T24-22"))
    } else {
        map_errno(err)
    }
}

pub fn open_metadata(path: &Path) -> Result<File, PlatformError> {
    let fd = open(path, META_FLAGS, Mode::empty()).map_err(map_errno)?;
    let meta = stat_only(&fd)?;
    if meta.reparse || meta.kind == ObjectKind::Reparse {
        return Err(PlatformError::Rejected(Verdict::refuse(
            "link_policy_or_pin_absent",
            "T24-16",
        )));
    }
    Ok(File::from(fd))
}

fn open_directory_at(dir: &OwnedFd, name: &std::ffi::OsStr) -> Result<OwnedFd, PlatformError> {
    openat(dir, name, DIR_FLAGS, Mode::empty()).map_err(pin_errno)
}

fn open_directory_root(path: &Path) -> Result<OwnedFd, PlatformError> {
    open(path, DIR_FLAGS, Mode::empty()).map_err(pin_errno)
}

pub fn metadata_from_handle(file: &File) -> Result<ObservedMetadata, PlatformError> {
    stat_only(file)
}

fn stat_only(fd: impl AsFd) -> Result<ObservedMetadata, PlatformError> {
    let stat = fstat(&fd).map_err(map_errno)?;
    metadata_from_stat(&stat)
}

fn unix_owner_ok(uid: u32, euid: u32) -> bool {
    uid == euid || (uid == 0 && euid == 0)
}

fn metadata_from_stat(stat: &rustix::fs::Stat) -> Result<ObservedMetadata, PlatformError> {
    let mode = stat.st_mode;
    let file_type = mode & 0o170000;
    let kind = if file_type == 0o120000 {
        ObjectKind::Reparse
    } else if file_type == 0o040000 {
        ObjectKind::Directory
    } else if file_type == 0o100000 {
        ObjectKind::RegularFile
    } else {
        ObjectKind::Other
    };
    let nsec = i128::from(stat.st_mtime_nsec);
    let modified = i128::from(stat.st_mtime)
        .checked_mul(1_000_000_000)
        .and_then(|sec| sec.checked_add(nsec))
        .ok_or_else(|| PlatformError::Rejected(Verdict::refuse("filetime_overflow", "mtime")))?;
    let size = u64::try_from(stat.st_size)
        .map_err(|_| PlatformError::Rejected(Verdict::refuse("size_unusable", "st_size")))?;
    Ok(ObservedMetadata {
        identity: FileIdentity::Unix {
            device: stat.st_dev,
            inode: stat.st_ino,
        },
        kind,
        size,
        modified_unix_ns: modified,
        links: stat.st_nlink,
        reparse: kind == ObjectKind::Reparse,
        read_only_attribute: false,
        identity_limitation: None,
    })
}

fn admit_local_filesystem(fd: impl AsFd) -> Result<(), PlatformError> {
    let stat = fstatfs(&fd).map_err(map_errno)?;
    let magic = stat.f_type;
    // ext4 native profile only. NFS, CIFS, FUSE, overlay and drvfs stay refused.
    if magic == 0xEF53 {
        super::linux_mount::admit_ext4_mount(&fd)
    } else {
        Err(PlatformError::Rejected(Verdict::refuse(
            "unadmitted_filesystem",
            "held_fd_fstatfs",
        )))
    }
}

fn current_euid() -> Result<u32, PlatformError> {
    let mut file = File::open("/proc/self/status")?;
    let mut buf = [0u8; 4096];
    let read = file.read(&mut buf)?;
    if read == buf.len() {
        return Err(PlatformError::Rejected(Verdict::refuse(
            "euid_unknown",
            "proc_status_truncated",
        )));
    }
    let text = std::str::from_utf8(&buf[..read])
        .map_err(|_| PlatformError::Rejected(Verdict::refuse("euid_malformed", "proc_status")))?;
    if let Some(start) = text.find("Uid:") {
        let line = &text[start..];
        if !line.contains('\n') {
            return Err(PlatformError::Rejected(Verdict::refuse(
                "euid_unknown",
                "proc_status_truncated",
            )));
        }
    }
    parse_proc_status_euid(text).map_err(PlatformError::Rejected)
}

fn ancestor_paths(path: &Path) -> Result<Vec<PathBuf>, PlatformError> {
    if path.is_relative() {
        return Err(PlatformError::Rejected(Verdict::refuse(
            "relative_root",
            "T24-40",
        )));
    }
    let mut acc = PathBuf::new();
    let mut out = Vec::new();
    for component in path.components() {
        match component {
            Component::RootDir => {
                acc.push(component);
                out.push(acc.clone());
            }
            Component::Normal(name) => {
                parse_child_os(name, super::NativePlatform::Linux)
                    .map_err(PlatformError::Rejected)?;
                acc.push(name);
                out.push(acc.clone());
            }
            Component::Prefix(_) | Component::CurDir | Component::ParentDir => {
                return Err(PlatformError::Rejected(Verdict::refuse(
                    "child_name_rejected",
                    "dot_component",
                )));
            }
        }
    }
    if out.is_empty() {
        return Err(PlatformError::Rejected(Verdict::refuse(
            "relative_root",
            "T24-40",
        )));
    }
    Ok(out)
}

fn pin_directory(path: &Path) -> Result<Vec<OwnedFd>, PlatformError> {
    let ancestors = ancestor_paths(path)?;
    let euid = current_euid()?;
    let mut pins = Vec::with_capacity(ancestors.len());
    for (index, ancestor) in ancestors.iter().enumerate() {
        let fd = if index == 0 {
            open_directory_root(ancestor)?
        } else {
            let name = ancestor.file_name().ok_or_else(|| {
                PlatformError::Rejected(Verdict::refuse("child_name_rejected", "missing_component"))
            })?;
            let parent = pins.last().ok_or_else(|| {
                PlatformError::Rejected(Verdict::refuse("ancestor_pin_absent", "T24-45"))
            })?;
            open_directory_at(parent, name)?
        };
        let stat = fstat(&fd).map_err(map_errno)?;
        let meta = metadata_from_stat(&stat)?;
        if meta.kind != ObjectKind::Directory {
            return Err(PlatformError::Rejected(Verdict::refuse(
                "ancestor_not_directory",
                "T24-22",
            )));
        }
        let verdict = evaluate_unix_ancestor(UnixAncestorInput {
            owner_uid: stat.st_uid,
            current_euid: euid,
            mode: stat.st_mode,
            pinned: true,
        });
        if !verdict.is_allow() {
            return Err(PlatformError::Rejected(verdict));
        }
        pins.push(fd);
    }
    Ok(pins)
}

/// Reject any extended access/default ACL rather than interpreting only mode bits.
/// ENODATA proves absence. Unsupported/permission/malformed queries refuse.
fn reject_extended_acl(fd: impl AsFd) -> Result<bool, PlatformError> {
    for attribute in ["system.posix_acl_access", "system.posix_acl_default"] {
        let mut bytes = [0u8; 4096];
        match fgetxattr(&fd, attribute, &mut bytes[..]) {
            Err(Errno::NODATA) => {}
            Ok(_) => return Ok(true),
            Err(error) if error == Errno::RANGE => return Ok(true),
            Err(_) => {
                return Err(PlatformError::Rejected(Verdict::refuse(
                    "posix_acl_unverified",
                    "held_fd_fgetxattr",
                )))
            }
        }
    }
    Ok(false)
}

pub(super) fn held_root(root: &PrivateRoot) -> Result<&OwnedFd, PlatformError> {
    root._pins
        .last()
        .ok_or_else(|| PlatformError::Rejected(Verdict::refuse("ancestor_pin_absent", "T24-45")))
}

/// Detect displacement without promoting a reopened directory to authority.
pub(super) fn check_anchor(root: &PrivateRoot) -> Result<(), PlatformError> {
    let paths = ancestor_paths(&root.path)?;
    if paths.len() != root._pins.len() {
        return Err(PlatformError::Rejected(Verdict::refuse(
            "root_identity_changed",
            "anchor_depth",
        )));
    }
    for (index, held) in root._pins.iter().enumerate() {
        let named = if index == 0 {
            open_directory_root(&paths[index])?
        } else {
            open_directory_at(
                &root._pins[index - 1],
                paths[index].file_name().ok_or_else(|| {
                    PlatformError::Rejected(Verdict::refuse("root_identity_changed", "anchor_name"))
                })?,
            )?
        };
        let stat = fstat(held).map_err(map_errno)?;
        let verdict = evaluate_unix_ancestor(UnixAncestorInput {
            owner_uid: stat.st_uid,
            current_euid: current_euid()?,
            mode: stat.st_mode,
            pinned: true,
        });
        if !verdict.is_allow() {
            return Err(PlatformError::Rejected(verdict));
        }
        if stat_only(&named)?.identity != stat_only(held)?.identity {
            return Err(PlatformError::Rejected(Verdict::refuse(
                "root_identity_changed",
                "anchor_displaced",
            )));
        }
    }
    let fd = held_root(root)?;
    admit_local_filesystem(fd)?;
    let stat = fstat(fd).map_err(map_errno)?;
    let meta = metadata_from_stat(&stat)?;
    let verdict = evaluate_unix_private(UnixObjectInput {
        uid: stat.st_uid,
        current_euid: current_euid()?,
        mode: stat.st_mode,
        kind: meta.kind,
        extended_acl_foreign: Some(reject_extended_acl(fd)?),
        admit_library_0644: false,
        parent_effective_private: true,
        privileged_root_trust: current_euid()? == 0,
    });
    if !verdict.is_allow() {
        return Err(PlatformError::Rejected(verdict));
    }
    held_directory_matches(fd, &root.meta.identity)
}

pub(super) fn sync_directory(root: &PrivateRoot) -> Result<(), PlatformError> {
    check_anchor(root)?;
    rustix::fs::fsync(held_root(root)?).map_err(map_errno)
}

pub(super) fn create_private_directory(
    parent: &PrivateRoot,
    name: &ChildName,
) -> Result<PrivateRoot, PlatformError> {
    check_anchor(parent)?;
    mkdirat(
        held_root(parent)?,
        name.as_str(),
        Mode::from_bits_truncate(0o700),
    )
    .map_err(map_errno)?;
    let fd = open_directory_at(held_root(parent)?, std::ffi::OsStr::new(name.as_str()))?;
    let meta = require_private_directory(&fd, current_euid()?)?;
    let mut pins = parent
        ._pins
        .iter()
        .map(|pin| rustix::io::dup(pin).map_err(map_errno))
        .collect::<Result<Vec<_>, _>>()?;
    pins.push(fd);
    let root = PrivateRoot {
        path: parent.path.join(name.as_str()),
        meta,
        limitations: vec!["ancestor_acl_not_read"],
        _pins: pins,
    };
    sync_directory(parent)?;
    Ok(root)
}

pub(super) fn create_unique_private_directory(
    parent: &PrivateRoot,
    prefix: &str,
) -> Result<PrivateRoot, PlatformError> {
    for _ in 0..16 {
        let name = parse_child_name(
            &format!("{prefix}{:032x}", rand::random::<u128>()),
            super::NativePlatform::Linux,
        )
        .map_err(PlatformError::Rejected)?;
        match create_private_directory(parent, &name) {
            Err(PlatformError::Io(error)) if error.kind() == io::ErrorKind::AlreadyExists => {
                continue
            }
            result => return result,
        }
    }
    Err(PlatformError::Rejected(Verdict::refuse(
        "exclusive_create_exists",
        "T24-02",
    )))
}

pub(super) fn open_private_read_pin(
    root: &PrivateRoot,
    name: &ChildName,
) -> Result<File, PlatformError> {
    check_anchor(root)?;
    let fd = openat(
        held_root(root)?,
        name.as_str(),
        OFlags::RDONLY
            .union(OFlags::NONBLOCK)
            .union(OFlags::NOFOLLOW)
            .union(OFlags::CLOEXEC),
        Mode::empty(),
    )
    .map_err(map_errno)?;
    let file = finish_private_file(root.path.join(name.as_str()), File::from(fd))?.into_file();
    // Cooperative readers/writers use flock; hostile same-principal mutation is outside custody.
    file.try_lock_shared().map_err(|error| match error {
        std::fs::TryLockError::WouldBlock => {
            PlatformError::Rejected(Verdict::refuse("bundle_file_busy", "flock_shared"))
        }
        std::fs::TryLockError::Error(error) => PlatformError::Io(error),
    })?;
    Ok(file)
}

pub(super) fn directory_names(
    root: &PrivateRoot,
    limit: usize,
) -> Result<Vec<String>, PlatformError> {
    check_anchor(root)?;
    let directory = rustix::fs::Dir::read_from(held_root(root)?).map_err(map_errno)?;
    let mut names = Vec::new();
    for entry in directory {
        let entry = entry.map_err(map_errno)?;
        let bytes = entry.file_name().to_bytes();
        if bytes == b"." || bytes == b".." {
            continue;
        }
        if names.len() >= limit {
            return Err(PlatformError::Rejected(Verdict::refuse(
                "bundle_file_count_outside_bound",
                "held_fd_directory",
            )));
        }
        let name = std::str::from_utf8(bytes).map_err(|_| {
            PlatformError::Rejected(Verdict::refuse("non_roundtrip_name", "held_fd_directory"))
        })?;
        parse_child_name(name, super::NativePlatform::Linux).map_err(PlatformError::Rejected)?;
        names.push(name.to_owned());
    }
    Ok(names)
}

pub fn resolve_native_root() -> Result<ResolvedRoot, PlatformError> {
    let query = RootQuery {
        home: std::env::var("HOME").ok(),
        userprofile: None,
        local_app_data: None,
        xdg_data_home: std::env::var("XDG_DATA_HOME").ok(),
        explicit_override: None,
        euid_is_root: current_euid()? == 0,
        platform: super::NativePlatform::Linux,
    };
    let resolved = resolve_root(&query).map_err(PlatformError::Rejected)?;
    if broad_temp_rejected(Path::new(&resolved.path)) {
        return Err(PlatformError::Rejected(Verdict::refuse(
            "broad_temp_rejected",
            "T24-22",
        )));
    }
    Ok(resolved)
}

fn require_private_directory(fd: &OwnedFd, euid: u32) -> Result<ObservedMetadata, PlatformError> {
    let stat = fstat(fd).map_err(map_errno)?;
    if !unix_owner_ok(stat.st_uid, euid) {
        return Err(PlatformError::Rejected(Verdict::deny("foreign_unix_owner")));
    }
    fchmod(fd, Mode::from_bits_truncate(0o700)).map_err(map_errno)?;
    let stat = fstat(fd).map_err(map_errno)?;
    let meta = {
        admit_local_filesystem(fd)?;
        metadata_from_stat(&stat)?
    };
    let verdict = evaluate_unix_private(UnixObjectInput {
        uid: stat.st_uid,
        current_euid: euid,
        mode: stat.st_mode,
        kind: meta.kind,
        extended_acl_foreign: Some(reject_extended_acl(fd)?),
        admit_library_0644: false,
        parent_effective_private: true,
        privileged_root_trust: euid == 0,
    });
    if !verdict.is_allow() {
        return Err(PlatformError::Rejected(verdict));
    }
    Ok(meta)
}

pub(super) fn create_user_private_root_in_home(prefix: &str) -> Result<PrivateRoot, PlatformError> {
    let home = std::env::var("HOME").map_err(|_| {
        PlatformError::Rejected(Verdict::refuse("no_trustworthy_root", "HOME_missing"))
    })?;
    if current_euid()? == 0 && home != "/root" {
        return Err(PlatformError::Rejected(Verdict::refuse(
            "privileged_foreign_home",
            "T24-51",
        )));
    }
    create_user_private_root_at(prefix, PathBuf::from(home))
}

pub fn create_user_private_root(prefix: &str) -> Result<PrivateRoot, PlatformError> {
    let resolved = resolve_native_root()?;
    create_user_private_root_at(prefix, PathBuf::from(resolved.path))
}

fn create_user_private_root_at(
    prefix: &str,
    parent: PathBuf,
) -> Result<PrivateRoot, PlatformError> {
    let stem = prefix.trim_end_matches(|ch: char| ch == '-' || ch.is_ascii_hexdigit());
    parse_child_name(stem, super::NativePlatform::Linux).map_err(PlatformError::Rejected)?;
    if broad_temp_rejected(&parent) {
        return Err(PlatformError::Rejected(Verdict::refuse(
            "broad_temp_rejected",
            "T24-22",
        )));
    }
    let mut pins = pin_directory(&parent)?;
    let parent_fd = pins
        .last()
        .ok_or_else(|| PlatformError::Rejected(Verdict::refuse("ancestor_pin_absent", "T24-45")))?;
    let name = unique_dir_name(parent_fd, prefix)?;
    let parent_fd = pins
        .last()
        .ok_or_else(|| PlatformError::Rejected(Verdict::refuse("ancestor_pin_absent", "T24-45")))?;
    let fd = open_directory_at(parent_fd, std::ffi::OsStr::new(&name))?;
    let euid = current_euid()?;
    let meta = match require_private_directory(&fd, euid) {
        Ok(meta) => meta,
        Err(err) => {
            drop(fd);
            return Err(err);
        }
    };
    let created = parent.join(&name);
    pins.push(fd);
    Ok(PrivateRoot {
        path: created,
        meta,
        limitations: vec!["ancestor_acl_not_read"],
        _pins: pins,
    })
}

fn unique_dir_name(parent: &OwnedFd, prefix: &str) -> Result<String, PlatformError> {
    for _ in 0..16u32 {
        let mut nonce = [0u8; 16];
        rand::rngs::OsRng
            .try_fill_bytes(&mut nonce)
            .map_err(|error| PlatformError::from(io::Error::other(error.to_string())))?;
        // The random name avoids namespace exhaustion; identity still comes
        // from the held directory descriptor, never from this suffix.
        let suffix: String = nonce.iter().map(|byte| format!("{byte:02x}")).collect();
        let name = format!("{prefix}{suffix}");
        parse_child_name(&name, super::NativePlatform::Linux).map_err(PlatformError::Rejected)?;
        match mkdirat(parent, &name, Mode::from_bits_truncate(0o700)) {
            Ok(()) => return Ok(name),
            Err(err) => {
                let io_err = io::Error::from(err);
                if io_err.kind() == io::ErrorKind::AlreadyExists {
                    continue;
                }
                return Err(if err == Errno::NOENT {
                    PlatformError::Rejected(Verdict::refuse("no_trustworthy_root", "T24-40"))
                } else {
                    map_errno(err)
                });
            }
        }
    }
    Err(PlatformError::Rejected(Verdict::refuse(
        "exclusive_create_exists",
        "T24-02",
    )))
}

pub fn open_existing_private_root(path: &Path) -> Result<PrivateRoot, PlatformError> {
    if broad_temp_rejected(path) {
        return Err(PlatformError::Rejected(Verdict::refuse(
            "broad_temp_rejected",
            "T24-22",
        )));
    }
    let pins = pin_directory(path)?;
    let fd = pins
        .last()
        .ok_or_else(|| PlatformError::Rejected(Verdict::refuse("ancestor_pin_absent", "T24-45")))?;
    let euid = current_euid()?;
    let stat = fstat(fd).map_err(map_errno)?;
    admit_local_filesystem(fd)?;
    let meta = metadata_from_stat(&stat)?;
    let verdict = evaluate_unix_private(UnixObjectInput {
        uid: stat.st_uid,
        current_euid: euid,
        mode: stat.st_mode,
        kind: meta.kind,
        extended_acl_foreign: Some(reject_extended_acl(fd)?),
        admit_library_0644: false,
        parent_effective_private: true,
        privileged_root_trust: euid == 0,
    });
    if !verdict.is_allow() {
        return Err(PlatformError::Rejected(verdict));
    }
    Ok(PrivateRoot {
        path: path.to_path_buf(),
        meta,
        limitations: vec!["ancestor_acl_not_read"],
        _pins: pins,
    })
}

pub fn create_private_file(
    root: &OwnedFd,
    root_path: &Path,
    name: &ChildName,
) -> Result<VerifiedFile, PlatformError> {
    let fd = openat(
        root,
        name.as_str(),
        OFlags::RDWR
            .union(OFlags::CREATE)
            .union(OFlags::EXCL)
            .union(OFlags::NOFOLLOW)
            .union(OFlags::CLOEXEC)
            .union(OFlags::NONBLOCK),
        Mode::from_bits_truncate(0o600),
    )
    .map_err(map_errno)?;
    let file = File::from(fd);
    let euid = current_euid()?;
    let stat = match fstat(&file) {
        Ok(stat) => stat,
        Err(err) => {
            drop(file);
            return Err(map_errno(err));
        }
    };
    if !unix_owner_ok(stat.st_uid, euid) {
        drop(file);
        return Err(PlatformError::Rejected(Verdict::deny("foreign_unix_owner")));
    }
    if let Err(err) = fchmod(&file, Mode::from_bits_truncate(0o600)) {
        drop(file);
        return Err(map_errno(err));
    }
    finish_private_file(root_path.join(name.as_str()), file)
}

pub fn open_existing_private_file(
    root: &OwnedFd,
    root_path: &Path,
    name: &ChildName,
) -> Result<VerifiedFile, PlatformError> {
    let probe = openat(root, name.as_str(), META_FLAGS, Mode::empty()).map_err(map_errno)?;
    let probed = stat_only(&probe)?;
    drop(probe);
    if probed.kind != ObjectKind::RegularFile || probed.reparse {
        return Err(PlatformError::Rejected(Verdict::refuse(
            "open_existing_not_regular",
            "T24-12",
        )));
    }
    let fd = openat(root, name.as_str(), LOCK_FLAGS, Mode::empty()).map_err(map_errno)?;
    finish_private_file(root_path.join(name.as_str()), File::from(fd))
}

pub(super) fn verify_library_leaf(
    root: &PrivateRoot,
    name: &ChildName,
) -> Result<ObservedMetadata, PlatformError> {
    check_anchor(root)?;
    let parent = held_root(root)?;
    let probe = openat(parent, name.as_str(), META_FLAGS, Mode::empty()).map_err(map_errno)?;
    let observed = stat_only(&probe)?;
    if observed.kind != ObjectKind::RegularFile || observed.reparse || observed.links != 1 {
        return Err(PlatformError::Rejected(Verdict::refuse(
            "library_leaf_not_single_regular",
            "library_leaf",
        )));
    }
    let fd = openat(
        parent,
        name.as_str(),
        OFlags::RDONLY
            .union(OFlags::NONBLOCK)
            .union(OFlags::NOFOLLOW)
            .union(OFlags::CLOEXEC)
            .union(OFlags::NOCTTY),
        Mode::empty(),
    )
    .map_err(map_errno)?;
    let stat = fstat(&fd).map_err(map_errno)?;
    let meta = metadata_from_stat(&stat)?;
    if meta.identity != observed.identity {
        return Err(PlatformError::Rejected(Verdict::refuse(
            "library_leaf_identity_changed",
            "library_leaf",
        )));
    }
    let verdict = evaluate_unix_private(UnixObjectInput {
        uid: stat.st_uid,
        current_euid: current_euid()?,
        mode: stat.st_mode,
        kind: meta.kind,
        extended_acl_foreign: Some(reject_extended_acl(&fd)?),
        admit_library_0644: true,
        parent_effective_private: true,
        privileged_root_trust: current_euid()? == 0,
    });
    if !verdict.is_allow() {
        return Err(PlatformError::Rejected(verdict));
    }
    check_anchor(root)?;
    Ok(meta)
}

fn finish_private_file(path: PathBuf, file: File) -> Result<VerifiedFile, PlatformError> {
    let stat = fstat(&file).map_err(map_errno)?;
    let meta = {
        admit_local_filesystem(&file)?;
        metadata_from_stat(&stat)?
    };
    let euid = current_euid()?;
    let verdict = evaluate_unix_private(UnixObjectInput {
        uid: stat.st_uid,
        current_euid: euid,
        mode: stat.st_mode,
        kind: meta.kind,
        extended_acl_foreign: Some(reject_extended_acl(&file)?),
        admit_library_0644: false,
        parent_effective_private: true,
        privileged_root_trust: euid == 0,
    });
    if !verdict.is_allow() {
        return Err(PlatformError::Rejected(verdict));
    }
    let links = private_regular_links(meta.kind, meta.links);
    if !links.is_allow() {
        return Err(PlatformError::Rejected(links));
    }
    Ok(VerifiedFile {
        path,
        file,
        meta,
        limitations: vec!["ancestor_acl_not_read"],
    })
}

fn held_directory_matches(
    root: &OwnedFd,
    root_identity: &FileIdentity,
) -> Result<(), PlatformError> {
    let meta = stat_only(root)?;
    if meta.identity != *root_identity || meta.kind != ObjectKind::Directory {
        return Err(PlatformError::Rejected(Verdict::refuse(
            "root_identity_changed",
            "held_directory_fd",
        )));
    }
    Ok(())
}

fn private_source_ok(
    fd: impl AsFd,
    expected: &CanonicalIdentity,
    kind_code: &'static str,
    mismatch_code: &'static str,
    mismatch_note: &'static str,
) -> Result<ObservedMetadata, PlatformError> {
    let stat = fstat(&fd).map_err(map_errno)?;
    let meta = metadata_from_stat(&stat)?;
    if meta.reparse || meta.kind != ObjectKind::RegularFile {
        return Err(PlatformError::Rejected(Verdict::refuse(
            kind_code,
            mismatch_note,
        )));
    }
    let links = private_regular_links(meta.kind, meta.links);
    if !links.is_allow() {
        return Err(PlatformError::Rejected(links));
    }
    if meta.identity != expected.identity() {
        return Err(PlatformError::Rejected(Verdict::refuse(
            mismatch_code,
            mismatch_note,
        )));
    }
    let euid = current_euid()?;
    let verdict = evaluate_unix_private(UnixObjectInput {
        uid: stat.st_uid,
        current_euid: euid,
        mode: stat.st_mode,
        kind: meta.kind,
        extended_acl_foreign: Some(reject_extended_acl(&fd)?),
        admit_library_0644: false,
        parent_effective_private: true,
        privileged_root_trust: euid == 0,
    });
    if !verdict.is_allow() {
        return Err(PlatformError::Rejected(verdict));
    }
    Ok(meta)
}

pub fn rename_absent_in_root(
    root: &OwnedFd,
    root_identity: &FileIdentity,
    expected: &CanonicalIdentity,
    from: &ChildName,
    to: &ChildName,
) -> Result<&'static str, PlatformError> {
    held_directory_matches(root, root_identity)?;
    let probe = openat(root, from.as_str(), META_FLAGS, Mode::empty()).map_err(map_errno)?;
    let meta = stat_only(&probe)?;
    if meta.kind != ObjectKind::RegularFile || meta.reparse {
        return Err(PlatformError::Rejected(Verdict::refuse(
            "rename_source_not_regular",
            "not_renamed",
        )));
    }
    if meta.identity != expected.identity() {
        return Err(PlatformError::Rejected(Verdict::refuse(
            "rename_source_identity_mismatch",
            "not_renamed",
        )));
    }
    let source = openat(
        root,
        from.as_str(),
        OFlags::RDONLY
            .union(OFlags::NONBLOCK)
            .union(OFlags::NOFOLLOW)
            .union(OFlags::CLOEXEC)
            .union(OFlags::NOCTTY),
        Mode::empty(),
    )
    .map_err(map_errno)?;
    let source_meta = private_source_ok(
        &source,
        expected,
        "rename_source_not_regular",
        "rename_source_identity_mismatch",
        "not_renamed",
    )?;
    drop(source);
    renameat_with(
        root,
        from.as_str(),
        root,
        to.as_str(),
        RenameFlags::NOREPLACE,
    )
    .map_err(map_errno)?;
    let dest = match openat(root, to.as_str(), META_FLAGS, Mode::empty()) {
        Ok(fd) => fd,
        Err(_) => {
            return Err(PlatformError::Rejected(Verdict::refuse(
                "rename_identity_changed",
                "moved_unverified_object_at_destination",
            )));
        }
    };
    let dest_meta = match stat_only(&dest) {
        Ok(meta) => meta,
        Err(_) => {
            return Err(PlatformError::Rejected(Verdict::refuse(
                "rename_identity_changed",
                "moved_unverified_object_at_destination",
            )));
        }
    };
    if dest_meta.identity != source_meta.identity || dest_meta.identity != expected.identity() {
        return Err(PlatformError::Rejected(Verdict::refuse(
            "rename_identity_changed",
            "moved_unverified_object_at_destination",
        )));
    }
    Ok(RENAME_RESIDUAL_SAME_PRINCIPAL)
}

pub fn open_lock_sentinel(
    root: &OwnedFd,
    root_identity: &FileIdentity,
    name: &ChildName,
    expected: &CanonicalIdentity,
) -> Result<File, PlatformError> {
    held_directory_matches(root, root_identity)?;
    let probe = openat(root, name.as_str(), META_FLAGS, Mode::empty()).map_err(map_errno)?;
    let probed = stat_only(&probe)?;
    if probed.kind != ObjectKind::RegularFile || probed.reparse || probed.links != 1 {
        return Err(PlatformError::Rejected(Verdict::refuse(
            "lock_not_regular_sentinel",
            "T24-48",
        )));
    }
    if probed.identity != expected.identity() {
        return Err(PlatformError::Rejected(Verdict::refuse(
            "lock_identity_changed",
            "T24-48",
        )));
    }
    drop(probe);
    admit_local_filesystem_at(root, name)?;
    let file =
        File::from(openat(root, name.as_str(), LOCK_FLAGS, Mode::empty()).map_err(map_errno)?);
    let held = private_source_ok(
        &file,
        expected,
        "lock_not_regular_sentinel",
        "lock_identity_changed",
        "T24-48",
    )?;
    if held.identity != probed.identity || held.kind != ObjectKind::RegularFile || held.links != 1 {
        return Err(PlatformError::Rejected(Verdict::refuse(
            "lock_identity_changed",
            "T24-48",
        )));
    }
    Ok(file)
}

fn admit_local_filesystem_at(root: &OwnedFd, name: &ChildName) -> Result<(), PlatformError> {
    let fd = openat(root, name.as_str(), META_FLAGS, Mode::empty()).map_err(map_errno)?;
    admit_local_filesystem(&fd)
}

pub fn recheck_lock_name(
    root: &OwnedFd,
    name: &ChildName,
    expected: &CanonicalIdentity,
    held: &File,
) -> Result<(), PlatformError> {
    let held_meta = stat_only(held)?;
    if held_meta.identity != expected.identity() || held_meta.kind != ObjectKind::RegularFile {
        return Err(PlatformError::Rejected(Verdict::refuse(
            "lock_identity_changed",
            "T24-48",
        )));
    }
    let again = openat(root, name.as_str(), META_FLAGS, Mode::empty())
        .map_err(|_| PlatformError::Rejected(Verdict::refuse("lock_identity_changed", "T24-48")))?;
    let named = stat_only(&again)?;
    if named.identity != expected.identity() {
        return Err(PlatformError::Rejected(Verdict::refuse(
            "lock_identity_changed",
            "T24-48",
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn retained_and_concurrent_private_roots_keep_distinct_names() {
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }

        let root = super::create_user_private_root(".dendro-name-regression-").unwrap();
        let _cleanup = Cleanup(root.path().to_path_buf());
        let parent = root._pins.last().unwrap();
        let mut names = std::collections::HashSet::new();
        for _ in 0..32 {
            assert!(names.insert(super::unique_dir_name(parent, "retained-").unwrap()));
        }
        let concurrent = std::thread::scope(|scope| {
            let workers: Vec<_> = (0..8)
                .map(|_| {
                    scope.spawn(|| {
                        (0..8)
                            .map(|_| super::unique_dir_name(parent, "retained-").unwrap())
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            workers
                .into_iter()
                .flat_map(|worker| worker.join().unwrap())
                .collect::<Vec<_>>()
        });
        for name in concurrent {
            assert!(names.insert(name));
        }
        assert_eq!(names.len(), 96);
        for name in &names {
            let fd = super::open_directory_at(parent, std::ffi::OsStr::new(name)).unwrap();
            let metadata = rustix::fs::fstat(&fd).unwrap();
            assert_eq!(metadata.st_mode & 0o777, 0o700);
            assert_eq!(metadata.st_uid, super::current_euid().unwrap());
        }
    }

    #[test]
    fn proc_status_euid_matches_bounded_read() {
        let uid = super::current_euid().expect("euid");
        let text = std::fs::read_to_string("/proc/self/status").expect("status");
        assert_eq!(uid, super::parse_proc_status_euid(&text).expect("parse"));
        assert!(super::parse_proc_status_euid("Uid:\n").is_err());
    }

    #[test]
    fn held_fd_positive_operations_and_hostile_fixtures() {
        eprintln!("evidence_branch=require_positive_operations");
        let root =
            super::create_user_private_root(".dendro-t24a-").expect("private root must be created");
        let root_path = root.path().to_path_buf();
        let cleanup = root_path.clone();
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let platform = super::super::NativePlatform::Linux;
            let child = |name: &str| super::parse_child_name(name, platform).unwrap();
            assert!(!super::broad_temp_rejected(root_path.parent().unwrap()));
            let reopened = super::open_existing_private_root(&root_path).expect("reopen root");
            assert_eq!(reopened.metadata().identity, root.metadata().identity);
            drop(reopened);
            let source_name = child("held-file.txt");
            let created = root.create_file(&source_name).expect("create");
            let source_token = created.canonical_identity().expect("canonical");
            assert_eq!(created.metadata().links, 1);
            let again = super::open_existing_private_file(
                root._pins.last().expect("pin"),
                root.path(),
                &source_name,
            )
            .expect("reopen file");
            assert_eq!(again.metadata().identity, source_token.identity());
            drop(again);
            drop(created);
            let proc_meta =
                super::super::reopen_metadata(std::path::Path::new("/proc/self/status"))
                    .expect("source metadata must not require the private filesystem allowlist");
            assert_eq!(proc_meta.kind, super::ObjectKind::RegularFile);
            let sentinel_name = child("lock-sentinel");
            let sentinel = root.create_file(&sentinel_name).expect("sentinel");
            let sentinel_token = sentinel.canonical_identity().expect("sentinel token");
            drop(sentinel);
            super::super::with_exclusive_lock(&root, &sentinel_name, &sentinel_token, |_handle| {
                assert!(super::super::os_exclusive_lock_conflicts(&root, &sentinel_name).unwrap());
                match super::super::with_exclusive_lock(
                    &root,
                    &sentinel_name,
                    &sentinel_token,
                    |_| Ok(()),
                ) {
                    Err(super::PlatformError::Rejected(verdict)) => {
                        assert_eq!(verdict.code, "same_process_lock_held");
                    }
                    other => panic!("second lock: {other:?}"),
                }
                Ok(())
            })
            .unwrap();
            let renamed = child("renamed.txt");
            let outcome = super::super::rename_absent_same_directory(
                &root,
                &source_token,
                &source_name,
                &renamed,
            )
            .expect("rename");
            assert_eq!(
                outcome.residual_same_principal_race,
                super::RENAME_RESIDUAL_SAME_PRINCIPAL
            );
            assert!(root_path.join("renamed.txt").is_file());
            assert!(!root_path.join("held-file.txt").exists());
            let occupant_name = child("occupant.txt");
            let occupant = root.create_file(&occupant_name).unwrap();
            let occupant_token = occupant.canonical_identity().unwrap();
            drop(occupant);
            match super::super::rename_absent_same_directory(
                &root,
                &occupant_token,
                &occupant_name,
                &renamed,
            ) {
                Err(super::PlatformError::Rejected(verdict)) => {
                    assert_eq!(verdict.code, "exclusive_create_exists");
                }
                other => panic!("destination exists: {other:?}"),
            }
            assert!(root_path.join("occupant.txt").is_file());
            assert!(root_path.join("renamed.txt").is_file());
            let link_name = child("held-link");
            std::os::unix::fs::symlink(root_path.join("renamed.txt"), root_path.join("held-link"))
                .unwrap();
            match super::open_metadata(&root_path.join("held-link")) {
                Err(super::PlatformError::Rejected(verdict)) => {
                    assert_eq!(verdict.note, Some("T24-16"));
                }
                other => panic!("symlink metadata: {other:?}"),
            }
            match super::super::rename_absent_same_directory(
                &root,
                &source_token,
                &link_name,
                &child("link-dest"),
            ) {
                Err(super::PlatformError::Rejected(verdict)) => {
                    assert_eq!(verdict.note, Some("not_renamed"));
                }
                other => panic!("symlink rename: {other:?}"),
            }
            assert!(!root_path.join("link-dest").exists());
            rustix::fs::mkfifoat(
                root._pins.last().expect("pin"),
                "held-fifo",
                rustix::fs::Mode::from_bits_truncate(0o600),
            )
            .expect("fifo");
            match super::super::rename_absent_same_directory(
                &root,
                &source_token,
                &child("held-fifo"),
                &child("fifo-dest"),
            ) {
                Err(super::PlatformError::Rejected(verdict)) => {
                    assert_eq!(verdict.code, "rename_source_not_regular");
                    assert_eq!(verdict.note, Some("not_renamed"));
                }
                other => panic!("fifo rename: {other:?}"),
            }
            assert!(!root_path.join("fifo-dest").exists());
            let hard_name = child("hardlink.txt");
            std::fs::hard_link(
                root_path.join("renamed.txt"),
                root_path.join("hardlink.txt"),
            )
            .unwrap();
            match super::super::rename_absent_same_directory(
                &root,
                &source_token,
                &renamed,
                &child("hard-dest"),
            ) {
                Err(super::PlatformError::Rejected(verdict)) => {
                    assert_eq!(verdict.code, "multiple_hardlinks")
                }
                other => panic!("hardlink rename: {other:?}"),
            }
            assert!(!root_path.join("hard-dest").exists());
            assert!(root_path.join("hardlink.txt").is_file());
            let swapped_name = child("swap-source.txt");
            let swapped = root.create_file(&swapped_name).unwrap();
            let old_token = swapped.canonical_identity().unwrap();
            drop(swapped);
            let replacement_name = child("swap-replacement.txt");
            let replacement = root.create_file(&replacement_name).unwrap();
            drop(replacement);
            std::fs::remove_file(root_path.join("swap-source.txt")).unwrap();
            std::fs::rename(
                root_path.join("swap-replacement.txt"),
                root_path.join("swap-source.txt"),
            )
            .unwrap();
            match super::super::rename_absent_same_directory(
                &root,
                &old_token,
                &swapped_name,
                &child("swap-dest"),
            ) {
                Err(super::PlatformError::Rejected(verdict)) => {
                    assert_eq!(verdict.code, "rename_source_identity_mismatch");
                    assert_eq!(verdict.note, Some("not_renamed"));
                }
                other => panic!("swapped rename: {other:?}"),
            }
            assert!(!root_path.join("swap-dest").exists());
            assert!(root_path.join("swap-source.txt").is_file());
            super::super::with_exclusive_lock(&root, &sentinel_name, &sentinel_token, |_old| {
                std::fs::remove_file(root_path.join(sentinel_name.as_str())).unwrap();
                let replaced = root
                    .create_file(&sentinel_name)
                    .expect("replacement sentinel");
                let replacement_token = replaced.canonical_identity().unwrap();
                drop(replaced);
                super::super::with_exclusive_lock(
                    &root,
                    &sentinel_name,
                    &replacement_token,
                    |_new| Ok(()),
                )
                .expect(
                    "replacement token locks a different inode; residual same-principal boundary",
                );
                Ok(())
            })
            .expect("original sentinel fd stays locked across the unlink");
            match super::super::with_exclusive_lock(&root, &sentinel_name, &sentinel_token, |_| {
                Ok(())
            }) {
                Err(super::PlatformError::Rejected(verdict)) => {
                    assert_eq!(verdict.code, "lock_identity_changed");
                }
                other => panic!("stale sentinel token: {other:?}"),
            }
            let _ = (hard_name, occupant_name);
            eprintln!("evidence_branch=positive_complete");
        }));
        drop(root);
        let _ = std::fs::remove_dir_all(&cleanup);
        if let Err(err) = outcome {
            std::panic::resume_unwind(err);
        }
    }
}
