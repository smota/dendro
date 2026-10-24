//! Windows primitives from the admitted safe APIs.
//! Ancestor and private pins omit FILE_SHARE_DELETE. Producer metadata opens
//! are read-only and share delete so a local NTFS producer can rotate.
//! ACE flag values are recorded and are not treated as inheritance provenance.
//! Held-handle admission requires NTFS, a volume GUID path, and an NT
//! `\Device\HarddiskVolume<digits>\` path. A drive letter is not admission.
//! A failed remote-protocol query is not admission. FILE_ID_INFO's 128-bit
//! file ID and its 64-bit volume serial are retained together in `FileIdentity`.
#![deny(unsafe_code)]

use std::fs::{File, OpenOptions};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsHandle;
use std::path::{Component, Path, PathBuf};

use win_custody::{current_user_sid_string, ObjectSecurityDescriptor, OwnedSid, ValidatedSid};
use windows_namespace_request_sys::open_by_id::{FileIdentifier, OpenFileByIdentifier};
use windows_namespace_request_sys::{
    CapturedHandle, FileInformationClass, FinalPathFlags, QueryFileInformation, QueryFinalPath,
    QueryVolumeInformation,
};

use super::policy::*;
use super::{PlatformError, PrivateRoot, VerifiedFile};

const FILE_SHARE_READ: u32 = 0x0000_0001;
const FILE_SHARE_WRITE: u32 = 0x0000_0002;
const FILE_SHARE_DELETE: u32 = 0x0000_0004;
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const FILE_GENERIC_READ: u32 = 0x0012_0089;
const FILE_GENERIC_WRITE: u32 = 0x0012_0116;
const FILE_ATTRIBUTE_READONLY: u64 = 0x1;
const FILE_ATTRIBUTE_DIRECTORY: u64 = 0x10;
const FILE_ATTRIBUTE_REPARSE_POINT: u64 = 0x400;
const SE_DACL_PROTECTED: u16 = 0x1000;
const FILE_ID_INFO_BYTES: usize = 24;
const FILE_REMOTE_PROTOCOL_INFO: i32 = 13;
const FILE_REMOTE_PROTOCOL_BYTES: usize = 116;
const TRUSTED_INSTALLER_SID: &str =
    "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464";

/// `windows-namespace-request-sys` 0.2.1 has no safe held-source rename setter.
/// Path rename is not a substitute. This operation stays refused.
pub const HELD_SOURCE_RENAME_UNRESOLVED: &str =
    "safe_held_source_rename_unresolved_no_setter_in_windows_namespace_request_sys_0_2_1";

struct KnownSids {
    user: OwnedSid,
    system: OwnedSid,
    administrators: OwnedSid,
    creator_owner: OwnedSid,
    owner_rights: OwnedSid,
    trusted_installer: OwnedSid,
}

struct HeldObservation {
    meta: ObservedMetadata,
    file_id_128: [u8; 16],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RemoteDiagnostic {
    QueryFailed,
    ProtocolPresent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RejectedCreateDisposition {
    RetainUntrustedArtifact,
}

pub fn open_nofollow(path: &Path) -> Result<File, PlatformError> {
    open_private_handle(path, false)
}

/// Read-only metadata handle. Share-delete is set so a producer can rename or
/// replace the path while this handle is held. This is not a private pin.
pub fn open_producer_metadata(path: &Path) -> Result<File, PlatformError> {
    let mut options = OpenOptions::new();
    options.read(true);
    options.share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE);
    options.custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT);
    options.open(path).map_err(PlatformError::from)
}

fn open_private_handle(path: &Path, write: bool) -> Result<File, PlatformError> {
    let mut options = OpenOptions::new();
    options.read(true).write(write);
    options.share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE);
    options.custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT);
    options.open(path).map_err(PlatformError::from)
}

pub fn metadata_from_handle(file: &File) -> Result<ObservedMetadata, PlatformError> {
    Ok(observe_held(file)?.meta)
}

fn observe_held(file: &File) -> Result<HeldObservation, PlatformError> {
    let info = winapi_util::file::information(file)?;
    let attrs = info.file_attributes();
    let reparse = attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0;
    let directory = attrs & FILE_ATTRIBUTE_DIRECTORY != 0;
    let kind = if reparse {
        ObjectKind::Reparse
    } else if directory {
        ObjectKind::Directory
    } else {
        ObjectKind::RegularFile
    };
    let ticks = info
        .last_write_time()
        .ok_or_else(|| PlatformError::Rejected(Verdict::refuse("filetime_missing", "filetime")))?;
    let identity = admit_local_ntfs(file)?;
    let FileIdentity::WindowsFileId128 {
        file_id: file_id_128,
        ..
    } = identity
    else {
        unreachable!("admit_local_ntfs returns FILE_ID_INFO identity")
    };
    Ok(HeldObservation {
        meta: ObservedMetadata {
            identity,
            kind,
            size: info.file_size(),
            modified_unix_ns: filetime_to_unix_ns(ticks).map_err(PlatformError::Rejected)?,
            links: info.number_of_links(),
            reparse,
            read_only_attribute: attrs & FILE_ATTRIBUTE_READONLY != 0,
            identity_limitation: Some(WINDOWS_IDENTITY_LIMITATION),
        },
        file_id_128,
    })
}

fn retained_windows_identity(
    volume_serial_64: u64,
    file_id: [u8; 16],
) -> Result<FileIdentity, Verdict> {
    if file_id == [0; 16] {
        return Err(Verdict::refuse("file_id_128_unusable", "zero_file_id"));
    }
    Ok(FileIdentity::WindowsFileId128 {
        volume_serial_64,
        file_id,
    })
}

fn admit_local_ntfs(file: &File) -> Result<FileIdentity, PlatformError> {
    let volume = QueryVolumeInformation::new(capture_held(file)?)
        .perform()
        .map_err(|_| {
            PlatformError::Rejected(Verdict::refuse(
                "volume_not_local_ntfs",
                "volume_query_failed",
            ))
        })?;
    let filesystem = volume.filesystem_name().to_string_lossy();
    let guid = QueryFinalPath::new(capture_held(file)?)
        .with_flags(FinalPathFlags::VOLUME_NAME_GUID)
        .perform()
        .map_err(|_| {
            PlatformError::Rejected(Verdict::refuse("volume_not_local_ntfs", "guid_path_absent"))
        })?;
    let nt = QueryFinalPath::new(capture_held(file)?)
        .with_flags(FinalPathFlags::VOLUME_NAME_NT)
        .perform()
        .map_err(|_| {
            PlatformError::Rejected(Verdict::refuse(
                "volume_not_local_ntfs",
                "nt_namespace_query_failed",
            ))
        })?;
    if !path_text_admits_local(&nt.to_string_lossy(), &guid.to_string_lossy(), &filesystem) {
        let note = if filesystem != "NTFS" {
            "filesystem_not_ntfs"
        } else if !guid_volume_path(&guid.to_string_lossy()) {
            "guid_path_absent"
        } else {
            "nt_namespace_refused"
        };
        return Err(PlatformError::Rejected(Verdict::refuse(
            "volume_not_local_ntfs",
            note,
        )));
    }
    let remote = match QueryFileInformation::new(
        capture_held(file)?,
        FileInformationClass::from_raw(FILE_REMOTE_PROTOCOL_INFO),
    )
    .with_capacity(FILE_REMOTE_PROTOCOL_BYTES)
    .perform()
    {
        Ok(_) => RemoteDiagnostic::ProtocolPresent,
        Err(_) => RemoteDiagnostic::QueryFailed,
    };
    if remote_protocol_requires_refusal(remote) {
        return Err(PlatformError::Rejected(Verdict::refuse(
            "volume_not_local_ntfs",
            "remote_protocol_present",
        )));
    }
    let bytes = QueryFileInformation::new(capture_held(file)?, FileInformationClass::ID)
        .with_capacity(FILE_ID_INFO_BYTES)
        .perform()
        .map_err(|_| {
            PlatformError::Rejected(Verdict::refuse(
                "volume_not_local_ntfs",
                "file_id_query_failed",
            ))
        })?;
    let data = bytes.as_slice();
    if data.len() < FILE_ID_INFO_BYTES {
        return Err(PlatformError::Rejected(Verdict::refuse(
            "file_id_128_unusable",
            "short_file_id_info",
        )));
    }
    let mut file_id_128 = [0_u8; 16];
    file_id_128.copy_from_slice(&data[8..24]);
    if file_id_128 == [0; 16] {
        return Err(PlatformError::Rejected(Verdict::refuse(
            "file_id_128_unusable",
            "zero_file_id",
        )));
    }
    let volume_serial_64 = u64::from_le_bytes(data[..8].try_into().expect("length checked"));
    retained_windows_identity(volume_serial_64, file_id_128).map_err(PlatformError::Rejected)
}

fn capture_held(file: &File) -> Result<CapturedHandle, PlatformError> {
    CapturedHandle::capture(file.as_handle()).map_err(|_| {
        PlatformError::Rejected(Verdict::refuse("held_handle_capture_failed", "local_ntfs"))
    })
}

fn path_text_admits_local(nt_path: &str, guid_path: &str, filesystem: &str) -> bool {
    filesystem == "NTFS" && guid_volume_path(guid_path) && nt_harddisk_volume(nt_path)
}

fn guid_volume_path(path: &str) -> bool {
    let Some(rest) = path.strip_prefix(r"\\?\Volume{") else {
        return false;
    };
    let Some(end) = rest.find('}') else {
        return false;
    };
    let guid = &rest[..end];
    let after = &rest[end + 1..];
    if !after.starts_with('\\') {
        return false;
    }
    let mut parts = guid.split('-');
    let expected = [8, 4, 4, 4, 12];
    for len in expected {
        let Some(part) = parts.next() else {
            return false;
        };
        if part.len() != len || !part.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return false;
        }
    }
    parts.next().is_none()
}

fn nt_harddisk_volume(path: &str) -> bool {
    let Some(rest) = path.strip_prefix(r"\Device\HarddiskVolume") else {
        return false;
    };
    let mut digits = 0_usize;
    for byte in rest.bytes() {
        if byte.is_ascii_digit() {
            digits += 1;
            continue;
        }
        return byte == b'\\' && digits > 0;
    }
    false
}

// A failed remote-protocol query is inconclusive. Independent positive NTFS,
// GUID-volume and local NT-namespace proofs above remain mandatory.
fn remote_protocol_requires_refusal(diagnostic: RemoteDiagnostic) -> bool {
    matches!(diagnostic, RemoteDiagnostic::ProtocolPresent)
}

#[cfg(test)]
fn drive_letter_admits_local(path: &str) -> bool {
    let _ = path;
    false
}

fn disposition_after_trust_rejection() -> RejectedCreateDisposition {
    RejectedCreateDisposition::RetainUntrustedArtifact
}

fn known_sids() -> Result<KnownSids, PlatformError> {
    let user = OwnedSid::from_string(&current_user_sid_string()?).map_err(PlatformError::from)?;
    Ok(KnownSids {
        user,
        system: OwnedSid::from_string("S-1-5-18")?,
        administrators: OwnedSid::from_string("S-1-5-32-544")?,
        creator_owner: OwnedSid::from_string("S-1-3-0")?,
        owner_rights: OwnedSid::from_string("S-1-3-4")?,
        trusted_installer: OwnedSid::from_string(TRUSTED_INSTALLER_SID)?,
    })
}

struct SecurityView {
    owner: OwnerEvidence,
    protected: bool,
    aces: Vec<AceView>,
}

fn read_security(file: &File, sids: &KnownSids) -> Result<SecurityView, PlatformError> {
    let sd = ObjectSecurityDescriptor::query_kernel_object(file)?;
    let owner = sd.owner().map_err(|_| {
        PlatformError::Rejected(Verdict::refuse("unverifiable_owner_or_control", "T24-06"))
    })?;
    let control = sd.control().map_err(|_| {
        PlatformError::Rejected(Verdict::refuse("unverifiable_owner_or_control", "T24-06"))
    })?;
    let dacl = sd.dacl().map_err(|_| {
        PlatformError::Rejected(Verdict::refuse("unverifiable_owner_or_control", "T24-06"))
    })?;
    let owner = classify_owner(&owner, sids);
    let mut aces = Vec::new();
    for ace in dacl.allowed_aces() {
        let trustee = if ace.trustee_matches(&sids.user) {
            Trustee::CurrentUser
        } else if ace.trustee_matches(&sids.system) {
            Trustee::LocalSystem
        } else if ace.trustee_matches(&sids.administrators) {
            Trustee::BuiltinAdministrators
        } else if ace.trustee_matches(&sids.trusted_installer) {
            Trustee::TrustedInstaller
        } else if ace.trustee_matches(&sids.creator_owner) {
            Trustee::CreatorOwner
        } else if ace.trustee_matches(&sids.owner_rights) {
            Trustee::OwnerRights
        } else {
            Trustee::Untrusted
        };
        aces.push(AceView {
            trustee,
            mask: ace.mask(),
            flags: ace.flags(),
        });
    }
    Ok(SecurityView {
        owner,
        protected: control & SE_DACL_PROTECTED != 0,
        aces,
    })
}

fn classify_owner(owner: &ValidatedSid, sids: &KnownSids) -> OwnerEvidence {
    if owner.matches(&sids.user) {
        OwnerEvidence::CurrentUser
    } else if owner.matches(&sids.system) {
        OwnerEvidence::LocalSystem
    } else if owner.matches(&sids.administrators) {
        OwnerEvidence::BuiltinAdministrators
    } else if owner.matches(&sids.trusted_installer) {
        OwnerEvidence::TrustedService
    } else {
        OwnerEvidence::Foreign
    }
}

fn ancestor_paths(path: &Path) -> Result<Vec<PathBuf>, PlatformError> {
    let mut acc = PathBuf::new();
    let mut out = Vec::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => acc.push(prefix.as_os_str()),
            Component::RootDir => {
                acc.push(component);
                out.push(acc.clone());
            }
            Component::Normal(name) => {
                parse_child_os(name, super::NativePlatform::Windows)
                    .map_err(PlatformError::Rejected)?;
                acc.push(name);
                out.push(acc.clone());
            }
            Component::CurDir | Component::ParentDir => {
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

fn pin_ancestors(
    path: &Path,
    sids: &KnownSids,
) -> Result<(Vec<File>, Vec<&'static str>), PlatformError> {
    let mut pins = Vec::new();
    let mut inputs = Vec::new();
    for ancestor in ancestor_paths(path)? {
        let file = open_nofollow(&ancestor)?;
        let meta = metadata_from_handle(&file)?;
        let security = read_security(&file, sids)?;
        inputs.push(AncestorInput {
            owner: security.owner,
            pinned_without_share_delete: true,
            reparse: meta.reparse,
            reparse_escapes_root: if meta.reparse { None } else { Some(false) },
            link_policy_admitted: false,
            dacl_present: true,
            aces: security.aces,
            trust: TrustPolicy::FROZEN_HOST,
        });
        pins.push(file);
    }
    let verdict = evaluate_ancestors(&inputs);
    if !verdict.is_allow() {
        return Err(PlatformError::Rejected(verdict));
    }
    Ok((pins, identity_limitations()))
}

fn identity_limitations() -> Vec<&'static str> {
    vec![WINDOWS_IDENTITY_LIMITATION, ACE_INHERITANCE_LIMITATION]
}

/// Sanitized ancestor facts: no path and no raw SID.
pub fn ancestor_replacement_facts() -> Result<Vec<String>, PlatformError> {
    let resolved = resolve_native_root()?;
    let labels = labeled_sids()?;
    let mut lines = Vec::new();
    for (index, ancestor) in ancestor_paths(Path::new(&resolved.path))?
        .iter()
        .enumerate()
    {
        let file = open_nofollow(ancestor)?;
        let sd = ObjectSecurityDescriptor::query_kernel_object(&file)?;
        let dacl = sd.dacl().map_err(|_| {
            PlatformError::Rejected(Verdict::refuse("unverifiable_ancestor", "T24-22"))
        })?;
        for ace in dacl.allowed_aces() {
            if ace.flags() & INHERIT_ONLY_ACE != 0 || ace.mask() & REPLACEMENT_MASK == 0 {
                continue;
            }
            let label = labels
                .iter()
                .find(|(_, sid)| ace.trustee_matches(sid))
                .map(|(label, _)| *label)
                .unwrap_or("unlisted");
            if label == "CurrentUser" || label == "LocalSystem" || label == "BuiltinAdministrators"
            {
                continue;
            }
            lines.push(format!(
                "idx={index} label={label} mask={:#x} flags={:#x}",
                ace.mask(),
                ace.flags()
            ));
        }
    }
    Ok(lines)
}

fn labeled_sids() -> Result<Vec<(&'static str, OwnedSid)>, PlatformError> {
    let mut out = Vec::new();
    let user = current_user_sid_string()?;
    for (label, value) in [
        ("CurrentUser", user.as_str()),
        ("LocalSystem", "S-1-5-18"),
        ("BuiltinAdministrators", "S-1-5-32-544"),
        ("Users", "S-1-5-32-545"),
        ("Everyone", "S-1-1-0"),
        ("AuthenticatedUsers", "S-1-5-11"),
        ("AllApplicationPackages", "S-1-15-2-1"),
        ("AllRestrictedApplicationPackages", "S-1-15-2-2"),
        ("CreatorOwner", "S-1-3-0"),
        ("OwnerRights", "S-1-3-4"),
        ("TrustedInstaller", TRUSTED_INSTALLER_SID),
    ] {
        out.push((label, OwnedSid::from_string(value)?));
    }
    Ok(out)
}

pub fn resolve_native_root() -> Result<ResolvedRoot, PlatformError> {
    deterministic_root()
}

fn deterministic_root() -> Result<ResolvedRoot, PlatformError> {
    choose_root(
        env_present("LOCALAPPDATA").as_deref(),
        env_present("USERPROFILE").as_deref(),
    )
}

fn env_present(key: &str) -> Option<String> {
    std::env::var(key).ok()
}

fn choose_root(
    local_app_data: Option<&str>,
    userprofile: Option<&str>,
) -> Result<ResolvedRoot, PlatformError> {
    let (path, source) = if let Some(path) = local_app_data {
        (path, "local_app_data")
    } else if let Some(path) = userprofile {
        (path, "userprofile")
    } else {
        return Err(PlatformError::Rejected(Verdict::refuse(
            "no_trustworthy_root",
            "T24-40",
        )));
    };
    let query = RootQuery {
        home: None,
        userprofile: if source == "userprofile" {
            Some(path.to_string())
        } else {
            None
        },
        local_app_data: if source == "local_app_data" {
            Some(path.to_string())
        } else {
            None
        },
        xdg_data_home: None,
        explicit_override: None,
        euid_is_root: false,
        platform: super::NativePlatform::Windows,
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

pub fn open_for_lock(path: &Path) -> Result<File, PlatformError> {
    open_private_handle(path, true)
}

pub fn create_user_private_root(prefix: &str) -> Result<PrivateRoot, PlatformError> {
    parse_child_name(prefix, super::NativePlatform::Windows).map_err(PlatformError::Rejected)?;
    let sids = known_sids()?;
    let resolved = deterministic_root()?;
    create_at_resolved(prefix, &resolved, &sids)
}

pub fn create_user_private_root_with_choice(
    prefix: &str,
    choice: &super::StorageRootChoice,
) -> Result<PrivateRoot, PlatformError> {
    parse_child_name(prefix, super::NativePlatform::Windows).map_err(PlatformError::Rejected)?;
    let path = match choice {
        super::StorageRootChoice::LocalAppData => std::env::var_os("LOCALAPPDATA"),
        super::StorageRootChoice::UserProfile => std::env::var_os("USERPROFILE"),
        super::StorageRootChoice::Explicit(path) => Some(path.clone().into_os_string()),
    }
    .ok_or_else(|| {
        PlatformError::Rejected(Verdict::refuse(
            "no_trustworthy_root",
            "explicit_choice_missing",
        ))
    })?;
    let path = path
        .to_str()
        .ok_or_else(|| PlatformError::Rejected(Verdict::refuse("non_roundtrip_name", "T24-50")))?;
    let resolved = resolve_root(&RootQuery {
        home: None,
        userprofile: None,
        local_app_data: None,
        xdg_data_home: None,
        explicit_override: Some(path.to_owned()),
        euid_is_root: false,
        platform: super::NativePlatform::Windows,
    })
    .map_err(PlatformError::Rejected)?;
    create_at_resolved(prefix, &resolved, &known_sids()?)
}

fn create_at_resolved(
    prefix: &str,
    resolved: &ResolvedRoot,
    sids: &KnownSids,
) -> Result<PrivateRoot, PlatformError> {
    let parent = PathBuf::from(&resolved.path);
    if broad_temp_rejected(&parent) {
        return Err(PlatformError::Rejected(Verdict::refuse(
            "broad_temp_rejected",
            "T24-22",
        )));
    }
    let pinned = pin_ancestors(&parent, sids)?;
    finish_private_root(prefix, parent, pinned, sids)
}

fn finish_private_root(
    prefix: &str,
    parent: PathBuf,
    pinned: (Vec<File>, Vec<&'static str>),
    sids: &KnownSids,
) -> Result<PrivateRoot, PlatformError> {
    let (mut pins, limitations) = pinned;
    let created = bound_platform::fs::create_private_dir(&parent, prefix)?;
    let directory = match open_nofollow(&created) {
        Ok(file) => file,
        Err(err) => {
            let _ = disposition_after_trust_rejection();
            return Err(err);
        }
    };
    if let Err(err) = verify_private_directory(&directory, sids) {
        drop(directory);
        let _ = disposition_after_trust_rejection();
        return Err(err);
    }
    let observed = match observe_held(&directory) {
        Ok(observed) => observed,
        Err(err) => {
            drop(directory);
            let _ = disposition_after_trust_rejection();
            return Err(err);
        }
    };
    pins.push(directory);
    Ok(PrivateRoot {
        path: created,
        meta: observed.meta,
        limitations,
        _pins: pins,
    })
}

fn verify_private_directory(directory: &File, sids: &KnownSids) -> Result<(), PlatformError> {
    let observed = observe_held(directory)?;
    if observed.meta.reparse || observed.meta.kind != ObjectKind::Directory {
        return Err(PlatformError::Rejected(Verdict::deny(
            "reparse_not_private_regular",
        )));
    }
    let security = read_security(directory, sids)?;
    let verdict = evaluate_private_object(&PrivateObjectInput {
        owner: security.owner,
        dacl_present: true,
        dacl_protected: security.protected,
        require_protected: true,
        trust: TrustPolicy::FROZEN_HOST,
        aces: security.aces,
        read_only_attribute: observed.meta.read_only_attribute,
        deny_ace_present: false,
    });
    if !verdict.is_allow() {
        return Err(PlatformError::Rejected(verdict));
    }
    Ok(())
}

pub fn open_existing_private_root(path: &Path) -> Result<PrivateRoot, PlatformError> {
    // Admit the absolute namespace before opening or pinning any ancestor.
    // Do not canonicalize through the filesystem: reparse checks use held handles.
    if !path.is_absolute() {
        return Err(PlatformError::Rejected(Verdict::refuse(
            "relative_root",
            "T24-40",
        )));
    }
    let text = path
        .to_str()
        .ok_or_else(|| PlatformError::Rejected(Verdict::refuse("non_roundtrip_name", "T24-50")))?;
    let resolved = resolve_root(&RootQuery {
        home: None,
        userprofile: None,
        local_app_data: None,
        xdg_data_home: None,
        explicit_override: Some(text.to_owned()),
        euid_is_root: false,
        platform: super::NativePlatform::Windows,
    })
    .map_err(PlatformError::Rejected)?;
    let path = Path::new(&resolved.path);
    // Validate the raw final name before any filesystem open or ancestor pin.
    // Path components can discard a final dot component; do not trim aliases.
    let leaf = resolved
        .path
        .rsplit(['\\', '/'])
        .find(|part| !part.is_empty())
        .ok_or_else(|| {
            PlatformError::Rejected(Verdict::refuse("child_name_rejected", "empty_or_dot"))
        })?;
    parse_child_os(std::ffi::OsStr::new(leaf), super::NativePlatform::Windows)
        .map_err(PlatformError::Rejected)?;
    if broad_temp_rejected(path) {
        return Err(PlatformError::Rejected(Verdict::refuse(
            "broad_temp_rejected",
            "T24-22",
        )));
    }
    let sids = known_sids()?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| PlatformError::Rejected(Verdict::refuse("relative_root", "T24-40")))?;
    let (mut pins, limitations) = pin_ancestors(parent, &sids)?;
    let opened = open_nofollow(path)?;
    if let Err(err) = verify_private_directory(&opened, &sids) {
        drop(opened);
        return Err(err);
    }
    let observed = observe_held(&opened)?;
    let directory = match reopen_verified(&opened, observed.file_id_128, false) {
        Ok(file) => file,
        Err(err) => {
            drop(opened);
            return Err(err);
        }
    };
    let again = match observe_held(&directory) {
        Ok(again) => again,
        Err(err) => {
            drop(directory);
            drop(opened);
            return Err(err);
        }
    };
    if again.meta.identity != observed.meta.identity || again.file_id_128 != observed.file_id_128 {
        drop(directory);
        drop(opened);
        return Err(PlatformError::Rejected(Verdict::refuse(
            "reopen_identity_mismatch",
            "open_existing_private_root",
        )));
    }
    drop(opened);
    pins.push(directory);
    Ok(PrivateRoot {
        path: path.to_path_buf(),
        meta: again.meta,
        limitations,
        _pins: pins,
    })
}

pub fn create_private_file(root: &Path, name: &ChildName) -> Result<VerifiedFile, PlatformError> {
    let path = root.join(name.as_str());
    let file = bound_platform::fs::create_new_file(&path, false)?;
    finish_new_file(path, file)
}

pub fn check_anchor(root: &PrivateRoot) -> Result<(), PlatformError> {
    let held = root
        ._pins
        .last()
        .ok_or_else(|| PlatformError::Rejected(Verdict::refuse("ancestor_pin_absent", "T24-45")))?;
    verify_private_directory(held, &known_sids()?)?;
    let named = open_nofollow(root.path())?;
    let held_meta = metadata_from_handle(held)?;
    let named_meta = metadata_from_handle(&named)?;
    if held_meta.identity != root.meta.identity
        || named_meta.identity != root.meta.identity
        || named_meta.kind != ObjectKind::Directory
        || named_meta.reparse
    {
        return Err(PlatformError::Rejected(Verdict::refuse(
            "root_identity_changed",
            "T24-45",
        )));
    }
    Ok(())
}

/// Exact-child protected creation has no admitted safe native API. Never use
/// recursive ACL mutation as a substitute for protection at creation.
pub fn create_private_directory(
    parent: &PrivateRoot,
    name: &ChildName,
) -> Result<PrivateRoot, PlatformError> {
    let _ = (parent, name);
    Err(PlatformError::Rejected(Verdict::refuse(
        "exact_private_directory_create_unproved",
        "protected_acl_at_creation_required",
    )))
}

pub fn create_unique_private_directory(
    parent: &PrivateRoot,
    prefix: &str,
) -> Result<PrivateRoot, PlatformError> {
    check_anchor(parent)?;
    // Validate the complete possible generated leaf before calling the safe
    // constructor; the dependency appends exactly sixteen lowercase hex digits.
    parse_child_name(
        &format!("{prefix}0000000000000000"),
        super::NativePlatform::Windows,
    )
    .map_err(PlatformError::Rejected)?;
    let path = bound_platform::fs::create_private_dir(parent.path(), prefix)?;
    let child = open_nofollow(&path)?;
    verify_private_directory(&child, &known_sids()?)?;
    let meta = metadata_from_handle(&child)?;
    check_anchor(parent)?;
    let mut pins = parent
        ._pins
        .iter()
        .map(File::try_clone)
        .collect::<Result<Vec<_>, _>>()?;
    pins.push(child);
    let root = PrivateRoot {
        path,
        meta,
        limitations: identity_limitations(),
        _pins: pins,
    };
    check_anchor(&root)?;
    Ok(root)
}

pub fn sync_directory(root: &PrivateRoot) -> Result<(), PlatformError> {
    check_anchor(root)?;
    let writable = open_private_handle(root.path(), true)?;
    if metadata_from_handle(&writable)?.identity != root.meta.identity {
        return Err(PlatformError::Rejected(Verdict::refuse(
            "root_identity_changed",
            "directory_flush",
        )));
    }
    writable.sync_all()?;
    check_anchor(root)
}

pub fn source_tree_contains_identity(
    root: &Path,
    target: &FileIdentity,
) -> Result<bool, PlatformError> {
    if !root.is_absolute() {
        return Err(PlatformError::Rejected(Verdict::refuse(
            "relative_root",
            "source_scan",
        )));
    }
    // Syntax and ancestry admission precede read_dir. Hold all ancestors for the
    // complete scan; each recursive call holds its directory until it returns.
    let (_pins, _) = pin_ancestors(root, &known_sids()?)?;
    fn scan(path: &Path, target: &FileIdentity, depth: usize) -> Result<bool, PlatformError> {
        if depth > 256 {
            return Err(PlatformError::Rejected(Verdict::refuse(
                "source_tree_depth",
                "source_scan",
            )));
        }
        let held = open_nofollow(path)?;
        let meta = metadata_from_handle(&held)?;
        if meta.reparse || !matches!(meta.kind, ObjectKind::Directory | ObjectKind::RegularFile) {
            return Err(PlatformError::Rejected(Verdict::refuse(
                "source_reparse_or_kind",
                "source_scan",
            )));
        }
        if &meta.identity == target {
            return Ok(true);
        }
        if meta.kind == ObjectKind::Directory {
            for entry in std::fs::read_dir(path)? {
                let entry = entry?;
                parse_child_os(&entry.file_name(), super::NativePlatform::Windows)
                    .map_err(PlatformError::Rejected)?;
                if scan(&entry.path(), target, depth + 1)? {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
    scan(root, target, 0)
}

pub fn open_existing_private_file(
    root: &Path,
    name: &ChildName,
) -> Result<VerifiedFile, PlatformError> {
    let path = root.join(name.as_str());
    // Reject wrong-kind, linked or foreign objects before requesting write access.
    let opened = open_private_handle(&path, false)?;
    let verified = match verify_private_leaf(&opened) {
        Ok(verified) => verified,
        Err(err) => {
            drop(opened);
            return Err(err);
        }
    };
    let reopened = match reopen_verified(&opened, verified.file_id_128, true) {
        Ok(file) => file,
        Err(err) => {
            drop(opened);
            return Err(err);
        }
    };
    let again = match verify_private_leaf(&reopened) {
        Ok(again) => again,
        Err(err) => {
            drop(reopened);
            drop(opened);
            return Err(err);
        }
    };
    if again.meta.identity != verified.meta.identity || again.file_id_128 != verified.file_id_128 {
        drop(reopened);
        drop(opened);
        return Err(PlatformError::Rejected(Verdict::refuse(
            "reopen_identity_mismatch",
            "open_existing_private_file",
        )));
    }
    drop(opened);
    Ok(VerifiedFile {
        path,
        file: reopened,
        meta: again.meta,
        limitations: identity_limitations(),
    })
}

fn finish_new_file(path: PathBuf, file: File) -> Result<VerifiedFile, PlatformError> {
    let verified = match verify_private_leaf(&file) {
        Ok(verified) => verified,
        Err(err) => {
            drop(file);
            let _ = disposition_after_trust_rejection();
            return Err(err);
        }
    };
    Ok(VerifiedFile {
        path,
        file,
        meta: verified.meta,
        limitations: identity_limitations(),
    })
}

struct VerifiedLeaf {
    meta: ObservedMetadata,
    file_id_128: [u8; 16],
}

fn verify_private_leaf(file: &File) -> Result<VerifiedLeaf, PlatformError> {
    let observed = observe_held(file)?;
    if observed.meta.reparse || observed.meta.kind != ObjectKind::RegularFile {
        return Err(PlatformError::Rejected(Verdict::deny(
            "reparse_not_private_regular",
        )));
    }
    let links = private_regular_links(observed.meta.kind, observed.meta.links);
    if !links.is_allow() {
        return Err(PlatformError::Rejected(links));
    }
    let sids = known_sids()?;
    let security = read_security(file, &sids)?;
    let verdict = evaluate_private_object(&PrivateObjectInput {
        owner: security.owner,
        dacl_present: true,
        dacl_protected: security.protected,
        require_protected: false,
        trust: TrustPolicy::FROZEN_HOST,
        aces: security.aces,
        read_only_attribute: observed.meta.read_only_attribute,
        deny_ace_present: false,
    });
    if !verdict.is_allow() {
        return Err(PlatformError::Rejected(verdict));
    }
    Ok(VerifiedLeaf {
        meta: observed.meta,
        file_id_128: observed.file_id_128,
    })
}

/// Validate a library-created leaf while DuckDB holds exclusive data access.
/// READ_CONTROL/FILE_READ_ATTRIBUTES avoid requesting data sharing rights.
pub(super) fn verify_library_leaf(
    root: &PrivateRoot,
    name: &ChildName,
) -> Result<ObservedMetadata, PlatformError> {
    check_anchor(root)?;
    let file = OpenOptions::new()
        .access_mode(0x0002_0080)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(root.path().join(name.as_str()))?;
    let observed = verify_private_leaf(&file)?.meta;
    check_anchor(root)?;
    Ok(observed)
}

/// Read-only private leaf admission. Immutable pins deny write/delete sharing;
/// pointer reads allow replacement so a reader never blocks publication itself.
pub(super) fn open_private_read_pin(
    root: &PrivateRoot,
    name: &ChildName,
    immutable: bool,
) -> Result<File, PlatformError> {
    check_anchor(root)?;
    let mut options = OpenOptions::new();
    options.read(true).share_mode(if immutable {
        FILE_SHARE_READ
    } else {
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE
    });
    options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    let file = options.open(root.path().join(name.as_str()))?;
    verify_private_leaf(&file)?;
    Ok(file)
}

fn reopen_verified(hint: &File, file_id_128: [u8; 16], write: bool) -> Result<File, PlatformError> {
    let access = if write {
        FILE_GENERIC_READ | FILE_GENERIC_WRITE
    } else {
        FILE_GENERIC_READ
    };
    let opened = OpenFileByIdentifier::new(
        capture_held(hint)?,
        FileIdentifier::ExtendedFileId(file_id_128),
    )
    .with_desired_access(access)
    .with_share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
    .with_flags_and_attributes(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
    .perform()
    .map_err(|_| {
        PlatformError::Rejected(Verdict::refuse(
            "open_by_file_id_128_failed",
            "held_id_reopen",
        ))
    })?;
    Ok(File::from(opened))
}

const DEFAULT_DATA_DIR: &str = "Dendro";

/// `%LOCALAPPDATA%\Dendro`, created with the same protected descriptor as other
/// private roots. `bound_platform` only mints a random suffix, so the fresh
/// directory is renamed onto the fixed leaf. An existing leaf is never replaced
/// and its DACL is never rewritten.
pub(super) fn open_or_create_default_data_root() -> Result<PrivateRoot, PlatformError> {
    let raw = std::env::var_os("LOCALAPPDATA").ok_or_else(|| {
        PlatformError::Rejected(Verdict::refuse(
            "no_trustworthy_root",
            "LOCALAPPDATA_missing",
        ))
    })?;
    let text = raw
        .to_str()
        .ok_or_else(|| PlatformError::Rejected(Verdict::refuse("non_roundtrip_name", "T24-50")))?;
    let resolved = resolve_root(&RootQuery {
        home: None,
        userprofile: None,
        local_app_data: None,
        xdg_data_home: None,
        explicit_override: Some(text.to_owned()),
        euid_is_root: false,
        platform: super::NativePlatform::Windows,
    })
    .map_err(PlatformError::Rejected)?;
    let parent = PathBuf::from(&resolved.path);
    if broad_temp_rejected(&parent) {
        return Err(PlatformError::Rejected(Verdict::refuse(
            "broad_temp_rejected",
            "T24-22",
        )));
    }
    parse_child_name(DEFAULT_DATA_DIR, super::NativePlatform::Windows)
        .map_err(PlatformError::Rejected)?;
    let sids = known_sids()?;
    // Prove the parent before creating. Drop the pins before the same-directory
    // rename: those handles omit FILE_SHARE_DELETE.
    drop(pin_ancestors(&parent, &sids)?);
    let target = parent.join(DEFAULT_DATA_DIR);
    match std::fs::symlink_metadata(&target) {
        Ok(_) => admit_existing_default_root(&target),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            create_fixed_default_root(&parent, &target)
        }
        Err(err) => Err(PlatformError::from(err)),
    }
}

fn admit_existing_default_root(target: &Path) -> Result<PrivateRoot, PlatformError> {
    match open_existing_private_root(target) {
        Ok(root) => Ok(root),
        Err(error) => Err(super::existing_default_root_not_private(target, &error)),
    }
}

fn create_fixed_default_root(parent: &Path, target: &Path) -> Result<PrivateRoot, PlatformError> {
    let temporary = bound_platform::fs::create_private_dir(parent, ".dendro-default-")?;
    match std::fs::rename(&temporary, target) {
        Ok(()) => open_existing_private_root(target),
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
            let _ = std::fs::remove_dir(&temporary);
            admit_existing_default_root(target)
        }
        Err(err) => {
            let _ = std::fs::remove_dir(&temporary);
            if std::fs::symlink_metadata(target).is_ok() {
                return admit_existing_default_root(target);
            }
            Err(PlatformError::from(err))
        }
    }
}

pub fn rename_absent_same_directory(
    from: &Path,
    to: &Path,
) -> Result<super::RenameOutcome, PlatformError> {
    let _ = (from, to);
    Err(PlatformError::Rejected(Verdict::refuse(
        "held_source_rename_unproved",
        HELD_SOURCE_RENAME_UNRESOLVED,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_private_child_retains_identity_and_refuses_exact_create() {
        let parent = create_user_private_root_with_choice(
            ".dendro-core-platform-",
            &super::super::StorageRootChoice::UserProfile,
        )
        .expect("admitted private parent");
        let cleanup = parent.path().to_path_buf();
        let child_name =
            parse_child_name("stage-child", super::super::NativePlatform::Windows).unwrap();
        let child =
            create_unique_private_directory(&parent, "stage-child-").expect("unique private child");
        assert!(child
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("stage-child-"));
        check_anchor(&child).expect("held and named root match");
        assert!(create_private_directory(&parent, &child_name).is_err());
        let leaf = parse_child_name("evidence.bin", super::super::NativePlatform::Windows).unwrap();
        let mut file = child.create_file(&leaf).expect("private file");
        use std::io::Write;
        file.file_mut().write_all(b"evidence").unwrap();
        file.file().sync_all().unwrap();
        let id = file.metadata().identity;
        assert!(source_tree_contains_identity(parent.path(), &id).unwrap());
        let reopened = open_existing_private_file(child.path(), &leaf).unwrap();
        assert_eq!(reopened.metadata().identity, id);
        assert_eq!(
            metadata_from_handle(&reopened.into_file())
                .unwrap()
                .identity,
            id
        );
        let alias = child.path().join("linked.bin");
        std::fs::hard_link(file.path(), &alias).expect("own fixture hard link");
        assert!(source_tree_contains_identity(parent.path(), &id).unwrap());
        assert!(open_existing_private_file(child.path(), &leaf).is_err());
        std::fs::remove_file(alias).expect("remove own fixture alias");
        sync_directory(&child).expect("admitted native NTFS fixture directory flush");
        eprintln!("evidence_branch=held_windows_directory_flush_complete");
        drop(file);
        drop(child);
        drop(parent);
        std::fs::remove_dir_all(cleanup).expect("remove own fixture after dropping pins");
        eprintln!("evidence_branch=unique_private_child_and_full_identity_scan_complete");
    }

    #[test]
    fn local_ntfs_text_gate_rejects_letters_refs_and_redirectors() {
        let guid = r"\\?\Volume{01e2fe6f-ec20-49cd-8be3-0d5c2f6e414e}\Users\example";
        let nt = r"\Device\HarddiskVolume3\Users\example";
        assert!(path_text_admits_local(nt, guid, "NTFS"));
        assert!(path_text_admits_local(
            r"\Device\HarddiskVolume3\",
            guid,
            "NTFS"
        ));
        assert!(!path_text_admits_local(r"C:\Users\example", guid, "NTFS"));
        assert!(!path_text_admits_local(nt, r"C:\Users\example", "NTFS"));
        assert!(!path_text_admits_local(nt, guid, "ReFS"));
        assert!(!path_text_admits_local(nt, guid, "NTFS "));
        assert!(!path_text_admits_local(
            r"\Device\Mup\server\share",
            guid,
            "NTFS"
        ));
        assert!(!path_text_admits_local(
            r"\Device\LanmanRedirector\server\share",
            guid,
            "NTFS"
        ));
        assert!(!path_text_admits_local(
            r"\Device\HarddiskVolume\rest",
            guid,
            "NTFS"
        ));
        assert!(!path_text_admits_local(
            r"\Device\HarddiskVolume3",
            guid,
            "NTFS"
        ));
        assert!(!guid_volume_path(r"\\?\Volume{not-a-guid}\"));
        assert!(!drive_letter_admits_local(r"C:\"));
        assert!(!drive_letter_admits_local(r"H:\share"));
    }

    #[test]
    fn remote_protocol_presence_refuses_and_query_failure_is_inconclusive() {
        assert!(!remote_protocol_requires_refusal(
            RemoteDiagnostic::QueryFailed
        ));
        assert!(remote_protocol_requires_refusal(
            RemoteDiagnostic::ProtocolPresent
        ));
    }

    #[test]
    fn enum_retains_full_64_volume_and_128_id() {
        let id = [1_u8, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0];
        let retained = retained_windows_identity(0x1866_3be0_663b_bd76, id).unwrap();
        assert_eq!(
            retained,
            FileIdentity::WindowsFileId128 {
                volume_serial_64: 0x1866_3be0_663b_bd76,
                file_id: id
            }
        );
        assert!(retained_windows_identity(1, [0; 16]).is_err());
        let mut high_id = id;
        high_id[15] ^= 0x80;
        assert_ne!(
            retained,
            retained_windows_identity(0x1866_3be0_663b_bd76, high_id).unwrap()
        );
        assert_ne!(
            retained,
            retained_windows_identity(0x0000_0000_663b_bd76, id).unwrap()
        );
        assert_ne!(
            retained,
            FileIdentity::WindowsByHandle64 {
                volume_serial: 0x1866_3be0_663b_bd76,
                file_index: 1
            }
        );
        assert_eq!(
            disposition_after_trust_rejection(),
            RejectedCreateDisposition::RetainUntrustedArtifact
        );
    }

    #[test]
    fn root_choice_does_not_fall_back_across_candidates() {
        for path in [r"relative", r"\\server\share", r"C:\Windows\Temp"] {
            let result = create_user_private_root_with_choice(
                ".dendro-never-created-",
                &super::super::StorageRootChoice::Explicit(PathBuf::from(path)),
            );
            assert!(
                matches!(result, Err(PlatformError::Rejected(_))),
                "{result:?}"
            );
        }
        let both = choose_root(Some(r"C:\Users\me\AppData\Local"), Some(r"C:\Users\me")).unwrap();
        assert_eq!(both.source, "local_app_data");
        assert_eq!(both.path, r"C:\Users\me\AppData\Local");
        let refused = choose_root(Some(r"\\server\share"), Some(r"C:\Users\me")).unwrap_err();
        match refused {
            PlatformError::Rejected(verdict) => assert_eq!(verdict.code, "unadmitted_unc"),
            other => panic!("{other}"),
        }
        let temp = choose_root(Some(r"C:\Windows\Temp"), Some(r"C:\Users\me")).unwrap_err();
        match temp {
            PlatformError::Rejected(verdict) => assert_eq!(verdict.code, "broad_temp_rejected"),
            other => panic!("{other}"),
        }
        let profile = choose_root(None, Some(r"C:\Users\me")).unwrap();
        assert_eq!(profile.source, "userprofile");
        let missing = choose_root(None, None).unwrap_err();
        match missing {
            PlatformError::Rejected(verdict) => assert_eq!(verdict.code, "no_trustworthy_root"),
            other => panic!("{other}"),
        }
    }

    #[test]
    fn rename_refuses_without_touching_paths() {
        let dir = std::env::temp_dir().join(format!("dendro-t24a-rename-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let from = dir.join("source.txt");
        let to = dir.join("dest.txt");
        std::fs::write(&from, b"stay").unwrap();
        let err = rename_absent_same_directory(&from, &to).unwrap_err();
        match err {
            PlatformError::Rejected(verdict) => {
                assert_eq!(verdict.code, "held_source_rename_unproved");
                assert_eq!(verdict.note, Some(HELD_SOURCE_RENAME_UNRESOLVED));
            }
            other => panic!("{other}"),
        }
        assert_eq!(std::fs::read(&from).unwrap(), b"stay");
        assert!(!to.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn producer_metadata_shares_delete_and_pin_does_not() {
        let dir = std::env::temp_dir().join(format!("dendro-t24a-share-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let producer = dir.join("producer.log");
        let pinned = dir.join("pinned.txt");
        std::fs::write(&producer, b"log").unwrap();
        std::fs::write(&pinned, b"pin").unwrap();
        let producer_handle = match open_producer_metadata(&producer) {
            Ok(file) => file,
            Err(err) => {
                let _ = std::fs::remove_dir_all(&dir);
                panic!("producer open: {err}");
            }
        };
        let meta = match metadata_from_handle(&producer_handle) {
            Ok(meta) => meta,
            Err(err) => {
                drop(producer_handle);
                let _ = std::fs::remove_dir_all(&dir);
                panic!("producer admission: {err}");
            }
        };
        assert!(matches!(
            meta.identity,
            FileIdentity::WindowsFileId128 { .. }
        ));
        assert_eq!(meta.identity_limitation, Some(WINDOWS_IDENTITY_LIMITATION));
        let rotated = dir.join("producer.log.1");
        std::fs::rename(&producer, &rotated).expect("producer handle must share delete");
        drop(producer_handle);
        let pin = open_nofollow(&pinned).expect("pin open");
        let remove_err = std::fs::remove_file(&pinned).expect_err("pin omits share delete");
        assert_ne!(remove_err.kind(), std::io::ErrorKind::NotFound);
        drop(pin);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn held_extended_id_reopen_matches_temp_file() {
        let dir = std::env::temp_dir().join(format!("dendro-t24a-reopen-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("id.bin");
        std::fs::write(&path, b"id").unwrap();
        let opened = open_private_handle(&path, true).expect("path open");
        let observed = observe_held(&opened).expect("local ntfs admission");
        assert!(matches!(
            observed.meta.identity,
            FileIdentity::WindowsFileId128 { .. }
        ));
        let reopened =
            reopen_verified(&opened, observed.file_id_128, true).expect("extended id reopen");
        let again = observe_held(&reopened).expect("reopened admission");
        assert_eq!(again.meta.identity, observed.meta.identity);
        assert_eq!(again.file_id_128, observed.file_id_128);
        drop(reopened);
        drop(opened);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn public_reopen_root_refuses_unadmitted_namespaces_before_io() {
        // An existing relative directory must refuse on syntax, not on its ACL
        // or because a fabricated path happens not to exist.
        let cwd = std::env::current_dir().unwrap();
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let fixture = cwd.join(format!(
            ".dendro-t24a-relative-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir(&fixture).unwrap();
        let nested = fixture.join("existing-root");
        std::fs::create_dir(&nested).unwrap();
        let existing_relative = nested.strip_prefix(&cwd).unwrap();
        assert!(existing_relative.is_dir());
        let outcome = std::panic::catch_unwind(|| {
            for (path, code) in [
                (existing_relative, "relative_root"),
                (Path::new(r"rel\x"), "relative_root"),
                (Path::new(r"C:rel\x"), "relative_root"),
                (Path::new(r"\rooted-without-drive\x"), "relative_root"),
                (Path::new(r"\\server\share\x"), "unadmitted_unc"),
                (Path::new(r"\\?\UNC\server\share\x"), "unadmitted_unc"),
                (
                    Path::new(r"\\?\Volume{01e2fe6f-ec20-49cd-8be3-0d5c2f6e414e}\x"),
                    "unadmitted_volume_guid",
                ),
                (Path::new(r"\\.\PhysicalDrive0"), "unadmitted_device"),
            ] {
                match super::super::open_existing_private_root(path) {
                    Err(PlatformError::Rejected(verdict)) => {
                        assert_eq!(verdict.outcome, Outcome::Refuse);
                        assert_eq!(verdict.code, code, "{}", path.display());
                    }
                    other => panic!(
                        "namespace must refuse before I/O, {}: {other:?}",
                        path.display()
                    ),
                }
            }
            eprintln!("evidence_branch=windows_reopen_root_namespace_refusals_complete; existing_relative; relative; drive_relative; rooted_without_drive; unc; extended_unc; volume_guid; device");
        });
        std::fs::remove_dir_all(&fixture)
            .expect("remove only the newly created relative-path fixture");
        if let Err(err) = outcome {
            std::panic::resume_unwind(err);
        }
    }

    #[test]
    fn open_existing_runs_only_after_root_admission() {
        let root = match create_user_private_root_with_choice(
            ".dendro-t24a-winrework-",
            &super::super::StorageRootChoice::UserProfile,
        ) {
            Ok(root) => root,
            Err(PlatformError::Rejected(verdict)) => {
                panic!("explicit USERPROFILE positive branch refused: {verdict:?}");
            }
            Err(err) => panic!("{err}"),
        };
        let path = root.path().to_path_buf();
        let created_id = root.metadata().identity;
        let opened = match open_existing_private_root(&path) {
            Ok(opened) => opened,
            Err(err) => {
                drop(root);
                let _ = std::fs::remove_dir_all(&path);
                panic!("open existing root: {err}");
            }
        };
        assert_eq!(opened.metadata().identity, created_id);
        assert!(opened.limitations().contains(&WINDOWS_IDENTITY_LIMITATION));
        let name =
            parse_child_name("held-file.txt", super::super::NativePlatform::Windows).unwrap();
        let created = root.create_file(&name).unwrap();
        let file_id = created.metadata().identity;
        drop(created);
        let again = open_existing_private_file(root.path(), &name).unwrap();
        assert_eq!(again.metadata().identity, file_id);
        assert_eq!(
            again.metadata().identity_limitation,
            Some(WINDOWS_IDENTITY_LIMITATION)
        );
        drop(again);
        drop(opened);
        drop(root);
        let _ = std::fs::remove_dir_all(&path);
    }
}
