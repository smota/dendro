//! Metadata-only observations. Source bytes are never read here.
//! Windows uses the platform producer-metadata handle. That is not a private-output check.

use serde::{Deserialize, Serialize};
use std::{fs::File, path::Path};

const MARKERS: &[&str] = &[
    "sk-",
    "ghp_",
    "gho_",
    "github_pat_",
    "xai-",
    "AKIA",
    "Bearer ",
    "xoxb-",
    "xoxp-",
];

/// The exact, portable file-metadata-v1 representation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FileMetadata {
    pub r#type: String,
    pub device: String,
    pub inode: String,
    pub size: String,
    pub mtime_ns: String,
}

impl FileMetadata {
    /// Read the actual ingest handle, rather than re-opening its pathname.
    /// This is source identity only. It does not apply private-output policy.
    pub fn from_file(file: &File) -> Option<Self> {
        held_source_metadata(file).ok()
    }
}

/// Source-handle metadata. `Err` is a stable reason, never a zero identity.
/// Private-output admission is a different check and is not performed here.
pub fn held_source_metadata(file: &File) -> Result<FileMetadata, &'static str> {
    #[cfg(any(windows, target_os = "linux"))]
    {
        held_via_platform(file)
    }
    #[cfg(all(unix, not(target_os = "linux")))]
    {
        held_via_unix_std(file)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = file;
        Err("unsupported_platform")
    }
}

/// Recover the native identity stored in file-metadata-v1 strings.
/// Unix rows stay decimal. Windows rows use the `w128:` namespace and all 16 id bytes.
pub fn reported_identity(
    metadata: &FileMetadata,
) -> Result<crate::platform_v1::FileIdentity, &'static str> {
    use crate::platform_v1::FileIdentity;
    if metadata.r#type != "regular" {
        return Err("nonregular_file");
    }
    let windows_device = metadata.device.starts_with(WINDOWS_IDENTITY_PREFIX);
    let windows_inode = metadata.inode.starts_with(WINDOWS_IDENTITY_PREFIX);
    if windows_device || windows_inode {
        if !windows_device || !windows_inode {
            return Err("identity_unusable");
        }
        let volume = &metadata.device[WINDOWS_IDENTITY_PREFIX.len()..];
        let id = &metadata.inode[WINDOWS_IDENTITY_PREFIX.len()..];
        if !canonical_u64(volume) || !canonical_file_id_hex(id) {
            return Err("identity_unusable");
        }
        let bytes = hex::decode(id).map_err(|_| "identity_unusable")?;
        let mut file_id = [0_u8; 16];
        file_id.copy_from_slice(&bytes);
        if file_id == [0; 16] {
            return Err("identity_unusable");
        }
        let volume_serial_64 = volume.parse::<u64>().map_err(|_| "identity_unusable")?;
        return Ok(FileIdentity::WindowsFileId128 {
            volume_serial_64,
            file_id,
        });
    }
    if !canonical_u64(&metadata.device) || !canonical_u64(&metadata.inode) {
        return Err("identity_unusable");
    }
    Ok(FileIdentity::Unix {
        device: metadata.device.parse().map_err(|_| "identity_unusable")?,
        inode: metadata.inode.parse().map_err(|_| "identity_unusable")?,
    })
}

const WINDOWS_IDENTITY_PREFIX: &str = "w128:";

fn canonical_u64(value: &str) -> bool {
    value
        .parse::<u64>()
        .is_ok_and(|number| number.to_string() == value)
}

fn canonical_file_id_hex(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        && value != "00000000000000000000000000000000"
}

fn project_source_metadata(
    meta: &crate::platform_v1::ObservedMetadata,
) -> Result<FileMetadata, &'static str> {
    use crate::platform_v1::{FileIdentity, ObjectKind};
    if meta.reparse || meta.kind == ObjectKind::Reparse {
        return Err("reparse_not_followed");
    }
    if meta.kind != ObjectKind::RegularFile {
        return Err("nonregular_file");
    }
    let (device, inode) = match meta.identity {
        FileIdentity::Unix { device, inode } => (device.to_string(), inode.to_string()),
        FileIdentity::WindowsFileId128 {
            volume_serial_64,
            file_id,
        } => {
            if file_id == [0; 16] {
                return Err("identity_unusable");
            }
            (
                format!("{WINDOWS_IDENTITY_PREFIX}{volume_serial_64}"),
                format!("{WINDOWS_IDENTITY_PREFIX}{}", hex::encode(file_id)),
            )
        }
        FileIdentity::WindowsByHandle64 { .. } => {
            return Err("windows_by_handle_64_not_admitted");
        }
    };
    Ok(FileMetadata {
        r#type: "regular".into(),
        device,
        inode,
        size: meta.size.to_string(),
        mtime_ns: meta.modified_unix_ns.to_string(),
    })
}

#[cfg(any(windows, target_os = "linux"))]
fn refusal_reason(code: &str) -> &'static str {
    match code {
        "volume_not_local_ntfs" => "unsupported_filesystem",
        "file_id_128_unusable" => "identity_unusable",
        "platform_unsupported" | "macos_metadata_not_admitted" => "unsupported_platform",
        _ => "metadata_error",
    }
}

#[cfg(any(windows, target_os = "linux"))]
fn held_via_platform(file: &File) -> Result<FileMetadata, &'static str> {
    match crate::platform_v1::metadata_from_handle(file) {
        Ok(meta) => project_source_metadata(&meta),
        Err(crate::platform_v1::PlatformError::Rejected(verdict)) => {
            Err(refusal_reason(verdict.code))
        }
        Err(crate::platform_v1::PlatformError::Io(_)) => Err("metadata_error"),
    }
}

#[cfg(all(unix, not(target_os = "linux")))]
fn held_via_unix_std(file: &File) -> Result<FileMetadata, &'static str> {
    use std::os::unix::fs::MetadataExt;
    let metadata = file.metadata().map_err(|_| "metadata_error")?;
    if !metadata.is_file() {
        return Err("nonregular_file");
    }
    Ok(FileMetadata {
        r#type: "regular".into(),
        device: metadata.dev().to_string(),
        inode: metadata.ino().to_string(),
        size: metadata.size().to_string(),
        mtime_ns: (i128::from(metadata.mtime()) * 1_000_000_000
            + i128::from(metadata.mtime_nsec()))
        .to_string(),
    })
}

#[derive(Clone, Debug)]
pub struct PathObservation {
    pub path_state: &'static str,
    pub reason: &'static str,
    pub checked_at: Option<String>,
    pub metadata: Option<FileMetadata>,
}

/// Reject unsafe identities without echoing their contents in a diagnostic.
pub fn safe_identity(value: &str) -> bool {
    !value.is_empty()
        && !value.contains('\0')
        && !value.contains('\u{fffd}')
        && !value.to_lowercase().contains("[redacted]")
        && !MARKERS.iter().any(|marker| value.contains(marker))
}

/// Pure lexical policy; callers must apply this before observation cache lookup.
pub fn lexical_reason(
    path: Option<&str>,
    in_scope: bool,
    trusted: bool,
    identity: bool,
) -> Option<&'static str> {
    if !in_scope {
        return Some("out_of_scope");
    }
    if !identity {
        return Some("identity_unavailable");
    }
    let Some(path) = path else {
        return Some("path_unresolvable");
    };
    #[cfg(windows)]
    if let Some(reason) = windows_grammar(path) {
        return Some(reason);
    }
    #[cfg(all(unix, not(windows)))]
    if let Some(reason) = unix_grammar(path) {
        return Some(reason);
    }
    #[cfg(not(any(windows, unix)))]
    {
        return Some("unsupported_platform");
    }
    sensitive_reason(path, trusted)
}

#[cfg(unix)]
fn unix_grammar(path: &str) -> Option<&'static str> {
    if !path.starts_with('/')
        || path.contains(['\0', '\u{fffd}'])
        || path.split('/').any(|part| matches!(part, "." | ".."))
    {
        Some("path_unresolvable")
    } else {
        None
    }
}

#[cfg(windows)]
fn windows_grammar(path: &str) -> Option<&'static str> {
    use crate::platform_v1::{classify_windows_path, parse_child_name, NativePlatform, PathClass};
    if path.contains(['\0', '\u{fffd}']) {
        return Some("path_unresolvable");
    }
    match classify_windows_path(path) {
        PathClass::LocalDrive | PathClass::LocalDriveVerbatim => {}
        PathClass::Relative | PathClass::Other => return Some("path_unresolvable"),
        PathClass::Unc | PathClass::Device | PathClass::VolumeGuid | PathClass::AlternateStream => {
            return Some("unadmitted_namespace");
        }
    }
    let tail = path
        .strip_prefix(r"\\?\")
        .or_else(|| path.strip_prefix(r"\\?/"))
        .unwrap_or(path);
    let mut parts = tail.split(['\\', '/']);
    let _drive = parts.next();
    for part in parts {
        if part.is_empty() {
            continue;
        }
        if part == "." || part == ".." {
            return Some("path_unresolvable");
        }
        if let Err(verdict) = parse_child_name(part, NativePlatform::Windows) {
            return Some(match verdict.code {
                "windows_reserved_name" => "windows_reserved_name",
                "windows_name_alias" => "windows_name_alias",
                _ => "path_unresolvable",
            });
        }
    }
    None
}

fn sensitive_reason(path: &str, trusted: bool) -> Option<&'static str> {
    let lower = path.to_lowercase();
    if lower.contains("[redacted]") || MARKERS.iter().any(|marker| path.contains(marker)) {
        return Some("redacted_path");
    }
    if !trusted {
        return Some("containment_unverified");
    }
    const FILES: &[&str] = &[
        "auth.json",
        "credentials.json",
        "settings.json",
        "settings.local.json",
        "secrets.json",
        "config.json.key",
        "id_rsa",
        "id_dsa",
        "id_ecdsa",
        "id_ed25519",
        ".netrc",
        ".npmrc",
        ".pypirc",
    ];
    const ANCESTORS: &[&str] = &[
        "credentials",
        "credential",
        "secrets",
        "secret",
        "auth",
        "tokens",
        ".ssh",
        ".aws",
        ".gnupg",
        ".azure",
        "gcloud",
    ];
    let credential = |part: &str| {
        FILES.contains(&part)
            || ANCESTORS.contains(&part)
            || part == ".env"
            || part.starts_with(".env.")
            || part.starts_with(".env-")
            || [".key", ".pem", ".p12", ".pfx"]
                .iter()
                .any(|suffix| part.ends_with(suffix))
    };
    let matched = if cfg!(windows) {
        lower.split(['/', '\\']).any(credential)
    } else {
        lower.split('/').any(credential)
    };
    if matched {
        return Some("credential_path");
    }
    None
}

/// Observe an absolute, lexically authorized source path.
/// Unix keeps the anchored no-follow directory walk. Windows asks the platform
/// producer-metadata seam for held identity and does not apply private-output policy.
/// This module never reads source bytes.
pub fn observe_path(path: &Path) -> PathObservation {
    if let Some(reason) = lexical_reason(path.to_str(), true, true, true) {
        return PathObservation {
            path_state: "not_checked",
            reason,
            checked_at: None,
            metadata: None,
        };
    }
    let checked_at = Some(chrono::Utc::now().to_rfc3339());
    #[cfg(unix)]
    let result = anchored::observe(path.to_str().expect("lexical path checked"));
    #[cfg(windows)]
    let result = observe_windows_source(path);
    #[cfg(not(any(unix, windows)))]
    let result: Result<FileMetadata, (&str, &str)> = Err(("unavailable", "unsupported_platform"));
    match result {
        Ok(metadata) => PathObservation {
            path_state: "present",
            reason: "regular_file",
            checked_at,
            metadata: Some(metadata),
        },
        Err((path_state, reason)) => PathObservation {
            path_state,
            reason,
            checked_at,
            metadata: None,
        },
    }
}

#[cfg(windows)]
fn observe_windows_source(path: &Path) -> Result<FileMetadata, (&'static str, &'static str)> {
    match crate::platform_v1::reopen_metadata(path) {
        Ok(meta) => project_source_metadata(&meta).map_err(|reason| ("unavailable", reason)),
        Err(crate::platform_v1::PlatformError::Io(err)) => {
            if err.kind() == std::io::ErrorKind::NotFound {
                Err(("gone", "path_not_found"))
            } else {
                Err(("unavailable", "metadata_error"))
            }
        }
        Err(crate::platform_v1::PlatformError::Rejected(verdict)) => {
            Err(("unavailable", refusal_reason(verdict.code)))
        }
    }
}

#[cfg(unix)]
mod anchored {
    use super::FileMetadata;
    use rustix::{
        fd::OwnedFd,
        fs::{fstat, open, openat, statat, AtFlags, FileType, Mode, OFlags, Stat},
        io::Errno,
    };

    type Failure = (&'static str, &'static str);
    const ERROR: Failure = ("unavailable", "metadata_error");
    const UNCERTAIN: Failure = ("unavailable", "observation_uncertain");
    const SYMLINK: Failure = ("unavailable", "symlink_not_followed");
    const FLAGS: OFlags = OFlags::RDONLY
        .union(OFlags::DIRECTORY)
        .union(OFlags::NOFOLLOW)
        .union(OFlags::CLOEXEC);

    struct Directory<'a> {
        handle: OwnedFd,
        name: &'a str,
        stat: Stat,
    }

    fn same_identity(a: &Stat, b: &Stat) -> bool {
        a.st_dev == b.st_dev && a.st_ino == b.st_ino && a.st_mode == b.st_mode
    }

    /// Every retained directory must still have the same name under its retained parent.
    fn recheck(directories: &[Directory<'_>]) -> Result<(), Failure> {
        for pair in directories.windows(2) {
            let named = statat(&pair[0].handle, pair[1].name, AtFlags::SYMLINK_NOFOLLOW).map_err(
                |error| {
                    if error == Errno::NOENT {
                        UNCERTAIN
                    } else {
                        ERROR
                    }
                },
            )?;
            let held = fstat(&pair[1].handle).map_err(|_| ERROR)?;
            if !same_identity(&named, &pair[1].stat) || !same_identity(&held, &pair[1].stat) {
                return Err(UNCERTAIN);
            }
        }
        Ok(())
    }

    fn named(
        directories: &[Directory<'_>],
        name: &str,
        named_stat: &mut impl FnMut(&OwnedFd, &str) -> rustix::io::Result<Stat>,
    ) -> Result<Stat, Failure> {
        let parent = &directories.last().ok_or(ERROR)?.handle;
        match named_stat(parent, name) {
            Ok(stat) => Ok(stat),
            Err(Errno::NOENT) => {
                recheck(directories)?;
                match named_stat(parent, name) {
                    Err(Errno::NOENT) => {
                        recheck(directories)?;
                        Err(("gone", "path_not_found"))
                    }
                    Ok(_) => Err(UNCERTAIN),
                    Err(_) => Err(ERROR),
                }
            }
            Err(_) => {
                recheck(directories)?;
                Err(ERROR)
            }
        }
    }

    pub(super) fn observe(path: &str) -> Result<FileMetadata, Failure> {
        walk(
            path,
            |_| {},
            |parent, name| statat(parent, name, AtFlags::SYMLINK_NOFOLLOW),
        )
    }

    // Private callbacks keep deterministic tests at the real traversal boundaries.
    // Production passes only the no-op and the actual no-follow syscall above.
    fn walk(
        path: &str,
        mut opened_directory: impl FnMut(&str),
        mut named_stat: impl FnMut(&OwnedFd, &str) -> rustix::io::Result<Stat>,
    ) -> Result<FileMetadata, Failure> {
        let parts: Vec<_> = path.split('/').filter(|part| !part.is_empty()).collect();
        let Some((leaf, parents)) = parts.split_last() else {
            // Root is the nonregular leaf itself; observe it without opening it.
            statat(rustix::fs::CWD, "/", AtFlags::SYMLINK_NOFOLLOW).map_err(|_| ERROR)?;
            return Err(("unavailable", "nonregular_file"));
        };
        let root = open("/", FLAGS, Mode::empty()).map_err(|_| ERROR)?;
        let stat = fstat(&root).map_err(|_| ERROR)?;
        let mut directories = vec![Directory {
            handle: root,
            name: "/",
            stat,
        }];
        opened_directory("/");
        for name in parents {
            let before = named(&directories, name, &mut named_stat)?;
            match FileType::from_raw_mode(before.st_mode) {
                FileType::Symlink => {
                    recheck(&directories)?;
                    return Err(SYMLINK);
                }
                FileType::Directory => {}
                _ => {
                    recheck(&directories)?;
                    return Err(ERROR);
                }
            }
            let parent = &directories.last().ok_or(ERROR)?.handle;
            let handle = match openat(parent, *name, FLAGS, Mode::empty()) {
                Ok(handle) => handle,
                Err(_) => {
                    recheck(&directories)?;
                    let after =
                        statat(parent, *name, AtFlags::SYMLINK_NOFOLLOW).map_err(|error| {
                            if error == Errno::NOENT {
                                UNCERTAIN
                            } else {
                                ERROR
                            }
                        })?;
                    return Err(if same_identity(&before, &after) {
                        ERROR
                    } else {
                        UNCERTAIN
                    });
                }
            };
            let held = fstat(&handle).map_err(|_| ERROR)?;
            if !same_identity(&before, &held) {
                return Err(UNCERTAIN);
            }
            directories.push(Directory {
                handle,
                name,
                stat: held,
            });
            opened_directory(name);
        }
        let stat = named(&directories, leaf, &mut named_stat)?;
        recheck(&directories)?;
        match FileType::from_raw_mode(stat.st_mode) {
            FileType::Symlink => Err(SYMLINK),
            FileType::RegularFile => {
                if path.ends_with('/') {
                    return Err(ERROR);
                }
                let metadata = from_stat(&stat)?;
                let parent = &directories.last().ok_or(ERROR)?.handle;
                let after = named_stat(parent, leaf).map_err(|error| {
                    if error == Errno::NOENT {
                        UNCERTAIN
                    } else {
                        ERROR
                    }
                })?;
                if metadata != from_stat(&after)? {
                    return Err(UNCERTAIN);
                }
                recheck(&directories)?;
                Ok(metadata)
            }
            _ => Err(("unavailable", "nonregular_file")),
        }
    }

    fn from_stat(stat: &Stat) -> Result<FileMetadata, Failure> {
        if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile {
            return Err(UNCERTAIN);
        }
        Ok(FileMetadata {
            r#type: "regular".into(),
            device: stat.st_dev.to_string(),
            inode: stat.st_ino.to_string(),
            size: u64::try_from(stat.st_size).map_err(|_| ERROR)?.to_string(),
            mtime_ns: (i128::from(stat.st_mtime) * 1_000_000_000 + i128::from(stat.st_mtime_nsec))
                .to_string(),
        })
    }

    #[cfg(test)]
    mod boundary_tests {
        use super::*;
        use std::{
            fs,
            path::PathBuf,
            sync::atomic::{AtomicU64, Ordering},
        };

        struct Scratch(PathBuf);

        impl Scratch {
            fn new() -> Self {
                static NEXT: AtomicU64 = AtomicU64::new(0);
                let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../target/file-metadata-boundary-tests")
                    .join(format!(
                        "{}-{}",
                        std::process::id(),
                        NEXT.fetch_add(1, Ordering::Relaxed)
                    ));
                fs::create_dir_all(&path).unwrap();
                Self(fs::canonicalize(path).unwrap())
            }
        }

        impl Drop for Scratch {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }

        #[test]
        fn held_ancestor_substitution_is_uncertain_even_when_old_leaf_is_missing() {
            for old_leaf_present in [true, false] {
                let scratch = Scratch::new();
                let parent = scratch.0.join("held-directory");
                fs::create_dir(&parent).unwrap();
                if old_leaf_present {
                    fs::write(parent.join("source.jsonl"), b"old\n").unwrap();
                }
                let mut swapped = false;
                let result = walk(
                    parent.join("source.jsonl").to_str().unwrap(),
                    |name| {
                        if name == "held-directory" {
                            assert!(!swapped);
                            fs::rename(&parent, scratch.0.join("retired-directory")).unwrap();
                            fs::create_dir(&parent).unwrap();
                            fs::write(parent.join("source.jsonl"), b"replacement\n").unwrap();
                            swapped = true;
                        }
                    },
                    |parent, name| statat(parent, name, AtFlags::SYMLINK_NOFOLLOW),
                );
                assert!(swapped, "the held-directory boundary must actually execute");
                assert_eq!(result, Err(UNCERTAIN));
                assert_eq!(
                    observe(parent.join("source.jsonl").to_str().unwrap())
                        .unwrap()
                        .size,
                    "12"
                );
            }
        }

        #[test]
        fn named_stat_io_and_descriptor_failures_are_not_absence() {
            let scratch = Scratch::new();
            let missing = scratch.0.join("missing.jsonl");
            assert_eq!(
                observe(missing.to_str().unwrap()),
                Err(("gone", "path_not_found"))
            );
            for error in [Errno::IO, Errno::MFILE, Errno::NOTDIR, Errno::NAMETOOLONG] {
                for fail_on in [1, 2] {
                    let mut leaf_attempts = 0;
                    let result = walk(
                        missing.to_str().unwrap(),
                        |_| {},
                        |parent, name| {
                            if name == "missing.jsonl" {
                                leaf_attempts += 1;
                                if leaf_attempts == fail_on {
                                    return Err(error);
                                }
                            }
                            statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)
                        },
                    );
                    assert_eq!(
                        leaf_attempts, fail_on,
                        "the named stat boundary must actually execute"
                    );
                    assert_eq!(result, Err(ERROR));
                }
            }
        }

        #[test]
        fn ancestor_substitution_during_second_absence_check_is_uncertain() {
            let scratch = Scratch::new();
            let parent = scratch.0.join("held-directory");
            fs::create_dir(&parent).unwrap();
            let missing = parent.join("missing.jsonl");
            let mut leaf_attempts = 0;
            let result = walk(
                missing.to_str().unwrap(),
                |_| {},
                |held_parent, name| {
                    if name == "missing.jsonl" {
                        leaf_attempts += 1;
                        if leaf_attempts == 2 {
                            fs::rename(&parent, scratch.0.join("retired-directory")).unwrap();
                            fs::create_dir(&parent).unwrap();
                            fs::write(parent.join(name), b"replacement\n").unwrap();
                        }
                    }
                    // Query the actual retained descriptor, so both absence errors
                    // come from real syscalls against the now-retired directory.
                    statat(held_parent, name, AtFlags::SYMLINK_NOFOLLOW)
                },
            );
            assert_eq!(leaf_attempts, 2);
            assert_eq!(result, Err(UNCERTAIN));
            assert!(parent.join("missing.jsonl").is_file());
        }

        #[test]
        fn root_only_observation_never_opens_a_directory() {
            for path in ["/", "////"] {
                let result = walk(
                    path,
                    |_| panic!("a nonregular root leaf must not be opened"),
                    |_, _| panic!("root-only observation must not traverse components"),
                );
                assert_eq!(result, Err(("unavailable", "nonregular_file")));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexical_policy_matches_precedence_and_negative_vectors() {
        let reason = |path| lexical_reason(path, true, true, true);
        assert_eq!(reason(None), Some("path_unresolvable"));
        #[cfg(unix)]
        {
            for path in [
                Some(""),
                Some("relative"),
                Some("/x/../y"),
                Some("/./x"),
                Some("/x\0"),
                Some("/x\u{fffd}"),
            ] {
                assert_eq!(reason(path), Some("path_unresolvable"));
            }
            for path in ["/x/[REDACTED]", "/x/ghp_", "/x/Bearer token", "/x/AKIA"] {
                assert_eq!(reason(Some(path)), Some("redacted_path"));
            }
            for path in [
                "/AUTH.JSON",
                "/x/.env-local",
                "/x/.env.local",
                "/x/.env",
                "/x/SECRET/y",
                "/x/a.PEM",
                "/x/.aws/y",
                "/x/settings.local.json",
            ] {
                assert_eq!(reason(Some(path)), Some("credential_path"));
            }
            for path in [
                "/x/environment.jsonl",
                "/x/env",
                "/x/.environment",
                "/x/authentication/y",
                "/x/auth.jsonl",
                "/x/notes.key.jsonl",
                "/x//good.jsonl",
            ] {
                assert_eq!(reason(Some(path)), None);
            }
            assert_eq!(
                lexical_reason(Some("/secret/a"), true, false, true),
                Some("containment_unverified")
            );
            assert_eq!(
                lexical_reason(Some("/sk-/a"), true, false, true),
                Some("redacted_path")
            );
        }
        #[cfg(windows)]
        {
            for path in [
                Some(""),
                Some("relative"),
                Some(r"C:rel\x"),
                Some(r"C:\x\..\y"),
                Some(r"C:\.\x"),
                Some("C:\\x\0"),
                Some("C:\\x\u{fffd}"),
            ] {
                assert_eq!(reason(path), Some("path_unresolvable"));
            }
            for path in [
                r"\\server\share\a",
                r"\\?\Volume{01234567-89ab-cdef-0123-456789abcdef}\a",
                r"\\.\C:\a",
                r"\\?\GLOBALROOT\Device\a",
                r"C:\file:stream",
            ] {
                assert_eq!(reason(Some(path)), Some("unadmitted_namespace"));
                let observed = observe_path(Path::new(path));
                assert_eq!(observed.path_state, "not_checked");
                assert!(observed.metadata.is_none());
                assert!(observed.checked_at.is_none());
            }
            for path in [
                r"C:\x\[REDACTED]",
                r"C:\x\ghp_",
                r"C:\x\Bearer token",
                r"C:\x\AKIA",
            ] {
                assert_eq!(reason(Some(path)), Some("redacted_path"));
            }
            for path in [
                r"C:\AUTH.JSON",
                r"C:\x\.env-local",
                r"C:\x\.env.local",
                r"C:\x\.env",
                r"C:\x\SECRET\y",
                r"C:\x\a.PEM",
                r"C:\x\.aws\y",
                r"C:\x\settings.local.json",
            ] {
                assert_eq!(reason(Some(path)), Some("credential_path"));
            }
            assert_eq!(reason(Some(r"C:\x\CON")), Some("windows_reserved_name"));
            assert_eq!(reason(Some(r"C:\x\file.")), Some("windows_name_alias"));
            for path in [
                r"C:\x\environment.jsonl",
                r"C:\x\env",
                r"C:\x\.environment",
                r"C:\x\authentication\y",
                r"C:\x\auth.jsonl",
                r"C:\x\notes.key.jsonl",
                r"C:\x\\good.jsonl",
                r"\\?\C:\x\good.jsonl",
            ] {
                assert_eq!(reason(Some(path)), None);
            }
            assert_eq!(
                lexical_reason(Some(r"C:\secret\a"), true, false, true),
                Some("containment_unverified")
            );
            assert_eq!(
                lexical_reason(Some(r"C:\sk-\a"), true, false, true),
                Some("redacted_path")
            );
        }
        assert_eq!(
            lexical_reason(None, false, false, false),
            Some("out_of_scope")
        );
        assert_eq!(
            lexical_reason(None, true, false, false),
            Some("identity_unavailable")
        );
    }

    #[test]
    fn source_projection_keeps_full_identity_and_refuses_narrow_or_zero_ids() {
        use crate::platform_v1::{FileIdentity, ObjectKind, ObservedMetadata};
        let sample = ObservedMetadata {
            identity: FileIdentity::WindowsFileId128 {
                volume_serial_64: 99,
                file_id: [
                    0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d,
                    0x0e, 0x0f, 0x10,
                ],
            },
            kind: ObjectKind::RegularFile,
            size: 9,
            modified_unix_ns: -1_500_000_000,
            links: 2,
            reparse: false,
            read_only_attribute: false,
            identity_limitation: Some(crate::platform_v1::WINDOWS_IDENTITY_LIMITATION),
        };
        let projected = project_source_metadata(&sample).unwrap();
        assert_eq!(projected.device, "w128:99");
        assert_eq!(projected.inode, "w128:0102030405060708090a0b0c0d0e0f10");
        assert_eq!(projected.size, "9");
        assert_eq!(projected.mtime_ns, "-1500000000");
        assert_eq!(
            reported_identity(&projected).unwrap(),
            sample.identity,
            "all 16 file-id bytes round-trip"
        );
        let mut zero = sample.clone();
        zero.identity = FileIdentity::WindowsFileId128 {
            volume_serial_64: 0,
            file_id: [0; 16],
        };
        assert_eq!(project_source_metadata(&zero), Err("identity_unusable"));
        let mut narrow = sample.clone();
        narrow.identity = FileIdentity::WindowsByHandle64 {
            volume_serial: 0,
            file_index: 0,
        };
        assert_eq!(
            project_source_metadata(&narrow),
            Err("windows_by_handle_64_not_admitted")
        );
        let mut link = sample.clone();
        link.identity = FileIdentity::Unix {
            device: 7,
            inode: 11,
        };
        link.links = 4;
        let unix = project_source_metadata(&link).unwrap();
        assert_eq!(unix.device, "7");
        assert_eq!(unix.inode, "11");
        assert_eq!(
            reported_identity(&unix).unwrap(),
            FileIdentity::Unix {
                device: 7,
                inode: 11
            }
        );
        let mut reparse = sample.clone();
        reparse.reparse = true;
        reparse.kind = ObjectKind::Reparse;
        assert_eq!(
            project_source_metadata(&reparse),
            Err("reparse_not_followed")
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_source_observation_uses_platform_identity_or_explicit_refusal() {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "dendro-metadata-port-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&directory).unwrap();
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(directory.clone());
        let source = directory.join("source.jsonl");
        std::fs::write(&source, b"x\npartial").unwrap();
        let file = File::open(&source).unwrap();
        let observed = observe_path(&source);
        if observed.reason == "unsupported_filesystem" {
            assert_eq!(observed.path_state, "unavailable");
            assert!(observed.metadata.is_none());
            assert_eq!(FileMetadata::from_file(&file), None);
            assert_eq!(held_source_metadata(&file), Err("unsupported_filesystem"));
            return;
        }
        assert_eq!(observed.path_state, "present");
        assert_eq!(observed.reason, "regular_file");
        let metadata = observed.metadata.unwrap();
        assert_eq!(metadata, FileMetadata::from_file(&file).unwrap());
        assert_eq!(metadata.size, "9");
        let identity = reported_identity(&metadata).unwrap();
        let crate::platform_v1::FileIdentity::WindowsFileId128 { file_id, .. } = identity else {
            panic!("source observation must retain WindowsFileId128");
        };
        assert_ne!(file_id, [0; 16]);
        assert_eq!(metadata.inode.len(), "w128:".len() + 32);
        let missing = observe_path(&directory.join("missing.jsonl"));
        assert_eq!(
            (missing.path_state, missing.reason),
            ("gone", "path_not_found")
        );
        assert!(missing.metadata.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn metadata_walk_handles_missing_links_nonregular_and_actual_full_size() {
        use std::{
            fs,
            os::unix::fs::symlink,
            sync::atomic::{AtomicU64, Ordering},
        };
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/file-metadata-tests")
            .join(format!(
                "{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        // Keep test paths absolute without `..`, as required by the public lexical policy.
        fs::create_dir_all(&directory).unwrap();
        let directory = fs::canonicalize(directory).unwrap();
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(directory.clone());
        let source = directory.join("source.jsonl");
        fs::write(&source, b"x\npartial").unwrap();
        let file = File::open(&source).unwrap();
        file.set_times(
            fs::FileTimes::new()
                .set_modified(std::time::UNIX_EPOCH - std::time::Duration::from_millis(1500)),
        )
        .unwrap();
        let observed = observe_path(&source);
        assert_eq!(observed.path_state, "present");
        assert!(observed.checked_at.is_some());
        let metadata = observed.metadata.unwrap();
        assert_eq!(metadata, FileMetadata::from_file(&file).unwrap());
        assert_eq!(metadata.size, "9");
        assert_eq!(metadata.mtime_ns, "-1500000000");
        let missing = observe_path(&directory.join("missing/leaf"));
        assert_eq!(
            (missing.path_state, missing.reason),
            ("gone", "path_not_found")
        );
        symlink("missing-target", directory.join("link")).unwrap();
        for name in ["link", "link/child"] {
            let result = observe_path(&directory.join(name));
            assert_eq!(result.reason, "symlink_not_followed");
            assert!(result.metadata.is_none());
        }
        rustix::fs::mkfifoat(
            rustix::fs::CWD,
            directory.join("fifo"),
            rustix::fs::Mode::RUSR,
        )
        .unwrap();
        for path in [directory.clone(), directory.join("fifo")] {
            assert_eq!(observe_path(&path).reason, "nonregular_file");
        }
        assert_eq!(observe_path(&source.join("child")).reason, "metadata_error");
        assert_eq!(observe_path(&directory.join("auth.json")).checked_at, None);
    }
}
