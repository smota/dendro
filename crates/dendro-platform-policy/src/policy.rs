//! OS-neutral policy. Inputs are explicit evidence; nothing here touches a disk.
#![deny(unsafe_code)]

use std::fmt;
use std::path::{Component, Path};

use super::NativePlatform;

pub const FILETIME_UNIX_EPOCH_TICKS: i128 = 116_444_736_000_000_000;
pub const WINDOWS_IDENTITY_LIMITATION: &str =
    "file_id_info_64bit_volume_128bit_id_local_ntfs_only_not_universal_filesystem_admission";
pub const ACE_INHERITANCE_LIMITATION: &str = "kernel_ace_flags_are_not_inheritance_provenance";
/// DELETE, WRITE_DAC, WRITE_OWNER, FILE_DELETE_CHILD, GENERIC_ALL, GENERIC_WRITE.
/// GENERIC_* is included because Windows may return the generic bit without the specific rights.
pub const REPLACEMENT_MASK: u32 =
    0x0001_0000 | 0x0004_0000 | 0x0008_0000 | 0x0000_0040 | 0x1000_0000 | 0x4000_0000;
pub const INHERIT_ONLY_ACE: u8 = 0x08;
pub const CREATE_OR_READ_MASK: u32 =
    0x0000_0001 | 0x0000_0002 | 0x0000_0004 | 0x0000_0020 | 0x0000_0080 | 0x0002_0000 | 0x0010_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Allow,
    Deny,
    Refuse,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Verdict {
    pub outcome: Outcome,
    pub code: &'static str,
    pub note: Option<&'static str>,
}

impl Verdict {
    pub const fn allow(code: &'static str) -> Self {
        Self {
            outcome: Outcome::Allow,
            code,
            note: None,
        }
    }

    pub const fn allow_note(code: &'static str, note: &'static str) -> Self {
        Self {
            outcome: Outcome::Allow,
            code,
            note: Some(note),
        }
    }

    pub const fn deny(code: &'static str) -> Self {
        Self {
            outcome: Outcome::Deny,
            code,
            note: None,
        }
    }

    pub const fn refuse(code: &'static str, note: &'static str) -> Self {
        Self {
            outcome: Outcome::Refuse,
            code,
            note: Some(note),
        }
    }

    pub const fn cancelled(code: &'static str) -> Self {
        Self {
            outcome: Outcome::Cancelled,
            code,
            note: None,
        }
    }

    pub const fn is_allow(self) -> bool {
        matches!(self.outcome, Outcome::Allow)
    }
}

#[derive(Debug)]
pub enum PlatformError {
    Rejected(Verdict),
    Io(std::io::Error),
}

impl fmt::Display for PlatformError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rejected(verdict) => write!(f, "{}:{:?}", verdict.code, verdict.outcome),
            Self::Io(err) => write!(f, "io:{err}"),
        }
    }
}

impl std::error::Error for PlatformError {}

impl From<std::io::Error> for PlatformError {
    fn from(err: std::io::Error) -> Self {
        if err.kind() == std::io::ErrorKind::AlreadyExists {
            Self::Rejected(Verdict::refuse("exclusive_create_exists", "T24-02"))
        } else {
            Self::Io(err)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileIdentity {
    /// FILE_ID_INFO serial and ID from the same held-handle query. Never volume32.
    WindowsFileId128 {
        volume_serial_64: u64,
        file_id: [u8; 16],
    },
    Unix {
        device: u64,
        inode: u64,
    },
    /// Held `BY_HANDLE_FILE_INFORMATION` identity only.
    WindowsByHandle64 {
        volume_serial: u64,
        file_index: u64,
    },
}

/// Identity of a regular sentinel that was verified earlier or loaded from a
/// persisted canonical record. `reopen_metadata` does not produce this token.
/// A same-principal caller that unlinks the sentinel and then builds a token
/// for the replacement can lock that new inode while the old inode is still locked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CanonicalIdentity {
    identity: FileIdentity,
}

impl CanonicalIdentity {
    pub const fn identity(self) -> FileIdentity {
        self.identity
    }

    pub(crate) const fn from_verified(identity: FileIdentity) -> Self {
        Self { identity }
    }
}

/// Interprocess locks must pass the identity stored in the canonical record.
/// Do not pass a value just observed from the current path.
pub fn canonical_identity_from_persisted_record(identity: FileIdentity) -> CanonicalIdentity {
    CanonicalIdentity::from_verified(identity)
}

/// Linux `renameat` names a directory entry. It cannot rename an open file descriptor.
/// A same-principal actor can still replace the source name after the check and before `renameat`.
pub const RENAME_RESIDUAL_SAME_PRINCIPAL: &str =
    "linux_renameat_is_by_name_not_fd; residual same-principal race before renameat";

pub const LOCK_RESIDUAL_SAME_PRINCIPAL: &str =
    "same-principal unlink of a sentinel plus a new persisted identity can lock the replacement while the old inode stays locked";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectKind {
    RegularFile,
    Directory,
    Reparse,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedMetadata {
    pub identity: FileIdentity,
    pub kind: ObjectKind,
    pub size: u64,
    pub modified_unix_ns: i128,
    pub links: u64,
    pub reparse: bool,
    pub read_only_attribute: bool,
    pub identity_limitation: Option<&'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtectedRole {
    Source,
    Output,
    Key,
    Lock,
    Pointer,
    Staging,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChildName(String);

impl ChildName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrustPolicy {
    pub admit_local_system: bool,
    pub admit_builtin_administrators: bool,
    /// Explicit service owner, such as Windows TrustedInstaller once that SID is classified.
    /// A foreign SID is never treated as this variant by policy itself.
    pub admit_trusted_service: bool,
}

impl TrustPolicy {
    /// Frozen disclosed host trust. This is not a claim of admin resistance.
    pub const FROZEN_HOST: Self = Self {
        admit_local_system: true,
        admit_builtin_administrators: true,
        admit_trusted_service: true,
    };

    pub const OWNER_ONLY: Self = Self {
        admit_local_system: false,
        admit_builtin_administrators: false,
        admit_trusted_service: false,
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnerEvidence {
    CurrentUser,
    LocalSystem,
    BuiltinAdministrators,
    /// Admitted only when `TrustPolicy::admit_trusted_service` is set and the caller proved the SID.
    TrustedService,
    Foreign,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trustee {
    /// Exact Windows TrustedInstaller SID only; trusted for ancestors, never private grants.
    TrustedInstaller,
    CurrentUser,
    LocalSystem,
    BuiltinAdministrators,
    CreatorOwner,
    OwnerRights,
    Untrusted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AceView {
    pub trustee: Trustee,
    pub mask: u32,
    pub flags: u8,
}

#[derive(Debug, Clone)]
pub struct PrivateObjectInput {
    pub owner: OwnerEvidence,
    pub dacl_present: bool,
    pub dacl_protected: bool,
    pub require_protected: bool,
    pub trust: TrustPolicy,
    pub aces: Vec<AceView>,
    pub read_only_attribute: bool,
    pub deny_ace_present: bool,
}

#[derive(Debug, Clone)]
pub struct AncestorInput {
    pub owner: OwnerEvidence,
    pub pinned_without_share_delete: bool,
    pub reparse: bool,
    pub reparse_escapes_root: Option<bool>,
    pub link_policy_admitted: bool,
    pub dacl_present: bool,
    pub aces: Vec<AceView>,
    pub trust: TrustPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathClass {
    LocalDrive,
    LocalDriveVerbatim,
    Unc,
    Device,
    VolumeGuid,
    AlternateStream,
    Relative,
    Other,
}

#[derive(Debug, Clone)]
pub struct RootQuery {
    pub home: Option<String>,
    pub userprofile: Option<String>,
    pub local_app_data: Option<String>,
    pub xdg_data_home: Option<String>,
    pub explicit_override: Option<String>,
    pub euid_is_root: bool,
    pub platform: NativePlatform,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRoot {
    pub path: String,
    pub source: &'static str,
}

pub fn filetime_to_unix_ns(ticks: u64) -> Result<i128, Verdict> {
    let delta = i128::from(ticks)
        .checked_sub(FILETIME_UNIX_EPOCH_TICKS)
        .ok_or_else(|| Verdict::refuse("filetime_overflow", "filetime"))?;
    delta
        .checked_mul(100)
        .ok_or_else(|| Verdict::refuse("filetime_overflow", "filetime"))
}

pub fn parse_child_name(raw: &str, platform: NativePlatform) -> Result<ChildName, Verdict> {
    if raw.is_empty() || raw == "." || raw == ".." {
        return Err(Verdict::refuse("child_name_rejected", "empty_or_dot"));
    }
    if raw.contains('/') || raw.contains('\\') || raw.contains('\0') {
        return Err(Verdict::refuse("child_name_rejected", "separator_or_nul"));
    }
    if raw.chars().any(|ch| ch.is_control()) {
        return Err(Verdict::refuse("child_name_rejected", "control_char"));
    }
    if platform == NativePlatform::Windows {
        validate_windows_leaf(raw)?;
    }
    Ok(ChildName(raw.to_string()))
}

pub fn parse_child_os(
    raw: &std::ffi::OsStr,
    platform: NativePlatform,
) -> Result<ChildName, Verdict> {
    match raw.to_str() {
        Some(text) => parse_child_name(text, platform),
        None => Err(Verdict::refuse("non_roundtrip_name", "T24-50")),
    }
}

fn validate_windows_leaf(raw: &str) -> Result<(), Verdict> {
    if raw.ends_with(' ') || raw.ends_with('.') {
        return Err(Verdict::refuse("windows_name_alias", "T24-49"));
    }
    if raw.contains(':')
        || raw
            .chars()
            .any(|ch| matches!(ch, '<' | '>' | '"' | '|' | '?' | '*'))
    {
        return Err(Verdict::refuse("windows_name_alias", "T24-49"));
    }
    let base = raw.split('.').next().unwrap_or(raw);
    if windows_reserved_device(base) {
        return Err(Verdict::refuse("windows_reserved_name", "T24-49"));
    }
    Ok(())
}

fn windows_reserved_device(base: &str) -> bool {
    let upper = base.to_ascii_uppercase();
    if matches!(
        upper.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) {
        return true;
    }
    let Some(rest) = upper
        .strip_prefix("COM")
        .or_else(|| upper.strip_prefix("LPT"))
    else {
        return false;
    };
    matches!(rest.chars().next(), Some('1'..='9' | '¹' | '²' | '³')) && rest.chars().count() == 1
}

pub fn classify_windows_path(path: &str) -> PathClass {
    if path.is_empty() {
        return PathClass::Other;
    }
    let upper = path.to_ascii_uppercase();
    if upper.starts_with("\\\\?\\UNC\\") || upper.starts_with("\\\\?\\UNC/") {
        return PathClass::Unc;
    }
    if upper.starts_with("\\\\?\\VOLUME{") {
        return PathClass::VolumeGuid;
    }
    if upper.starts_with("\\\\?\\GLOBALROOT") || upper.starts_with("\\\\.\\") {
        return PathClass::Device;
    }
    if let Some(rest) = path
        .strip_prefix("\\\\?\\")
        .or_else(|| path.strip_prefix("\\\\?/"))
    {
        let rest_upper = rest.to_ascii_uppercase();
        if rest_upper.len() >= 3
            && rest_upper.as_bytes()[0].is_ascii_alphabetic()
            && rest_upper.as_bytes()[1] == b':'
            && (rest_upper.as_bytes()[2] == b'\\' || rest_upper.as_bytes()[2] == b'/')
        {
            return if rest[2..].contains(':') {
                PathClass::AlternateStream
            } else {
                PathClass::LocalDriveVerbatim
            };
        }
        return PathClass::Device;
    }
    if path.starts_with("\\\\") {
        return PathClass::Unc;
    }
    let bytes = path.as_bytes();
    if bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
    {
        return if path[2..].contains(':') {
            PathClass::AlternateStream
        } else {
            PathClass::LocalDrive
        };
    }
    PathClass::Relative
}

pub fn syntax_admission(class: PathClass) -> Verdict {
    match class {
        PathClass::LocalDrive | PathClass::LocalDriveVerbatim => {
            Verdict::allow_note("local_drive_syntax", WINDOWS_IDENTITY_LIMITATION)
        }
        PathClass::Relative => Verdict::refuse("relative_root", "T24-40"),
        PathClass::Unc => Verdict::refuse("unadmitted_unc", "T24-18"),
        PathClass::Device => Verdict::refuse("unadmitted_device", "T24-18"),
        PathClass::VolumeGuid => Verdict::refuse("unadmitted_volume_guid", "T24-18"),
        PathClass::AlternateStream => Verdict::refuse("unadmitted_ads", "T24-18"),
        PathClass::Other => Verdict::refuse("unadmitted_namespace", "T24-18"),
    }
}

pub fn letter_is_not_volume_proof() -> Verdict {
    Verdict::refuse("drive_letter_not_volume_proof", "T24-49")
}

pub fn evaluate_private_object(input: &PrivateObjectInput) -> Verdict {
    let _ = input.read_only_attribute;
    let _ = input.deny_ace_present;
    if input.owner == OwnerEvidence::Unavailable || !input.dacl_present {
        return Verdict::refuse("unverifiable_owner_or_control", "T24-06");
    }
    match input.owner {
        OwnerEvidence::Foreign => return Verdict::deny("foreign_owner"),
        OwnerEvidence::TrustedService => return Verdict::deny("service_owner_not_private_object"),
        OwnerEvidence::LocalSystem if !input.trust.admit_local_system => {
            return Verdict::deny("system_owner_not_in_policy");
        }
        OwnerEvidence::BuiltinAdministrators if !input.trust.admit_builtin_administrators => {
            return Verdict::deny("admin_owner_not_in_policy");
        }
        OwnerEvidence::CurrentUser
        | OwnerEvidence::LocalSystem
        | OwnerEvidence::BuiltinAdministrators => {}
        OwnerEvidence::Unavailable => {
            return Verdict::refuse("unverifiable_owner_or_control", "T24-06")
        }
    }
    if input.require_protected && !input.dacl_protected {
        return Verdict::deny("private_directory_dacl_not_protected");
    }
    for ace in &input.aces {
        match ace.trustee {
            Trustee::CurrentUser => {}
            Trustee::LocalSystem if input.trust.admit_local_system => {}
            Trustee::BuiltinAdministrators if input.trust.admit_builtin_administrators => {}
            Trustee::CreatorOwner | Trustee::OwnerRights if ace.flags & INHERIT_ONLY_ACE != 0 => {}
            Trustee::TrustedInstaller
            | Trustee::LocalSystem
            | Trustee::BuiltinAdministrators
            | Trustee::CreatorOwner
            | Trustee::OwnerRights
            | Trustee::Untrusted => {
                return Verdict::deny("untrusted_allow_ace");
            }
        }
    }
    if input.dacl_protected {
        Verdict::allow("private_object_protected")
    } else {
        Verdict::allow_note(
            "private_object_effective_grants",
            "protected_flag_not_required_on_file",
        )
    }
}

pub fn evaluate_ancestor(input: &AncestorInput) -> Verdict {
    if input.owner == OwnerEvidence::Unavailable || !input.dacl_present {
        return Verdict::refuse("unverifiable_ancestor", "T24-22");
    }
    // The owner can rewrite the DACL. ACE allow-lists do not offset a foreign owner.
    match input.owner {
        OwnerEvidence::CurrentUser => {}
        OwnerEvidence::LocalSystem if input.trust.admit_local_system => {}
        OwnerEvidence::BuiltinAdministrators if input.trust.admit_builtin_administrators => {}
        OwnerEvidence::TrustedService if input.trust.admit_trusted_service => {}
        OwnerEvidence::LocalSystem => {
            return Verdict::refuse("system_owner_not_in_policy", "T24-22")
        }
        OwnerEvidence::BuiltinAdministrators => {
            return Verdict::refuse("admin_owner_not_in_policy", "T24-22");
        }
        OwnerEvidence::TrustedService => {
            return Verdict::refuse("service_owner_not_in_policy", "T24-22")
        }
        OwnerEvidence::Foreign => return Verdict::refuse("untrusted_ancestor_owner", "T24-22"),
        OwnerEvidence::Unavailable => return Verdict::refuse("unverifiable_ancestor", "T24-22"),
    }
    if input.reparse {
        if input.reparse_escapes_root == Some(true) {
            return Verdict::deny("reparse_escapes_root");
        }
        if !input.link_policy_admitted || !input.pinned_without_share_delete {
            return Verdict::refuse("link_policy_or_pin_absent", "T24-16");
        }
        if input.reparse_escapes_root != Some(false) {
            return Verdict::refuse("link_target_unproven", "T24-16");
        }
    }
    let mut saw_untrusted_replacement = false;
    for ace in &input.aces {
        if ace.flags & INHERIT_ONLY_ACE != 0 {
            continue;
        }
        if ace.mask & REPLACEMENT_MASK == 0 {
            continue;
        }
        let trusted = match ace.trustee {
            Trustee::CurrentUser => true,
            Trustee::LocalSystem => input.trust.admit_local_system,
            Trustee::BuiltinAdministrators => input.trust.admit_builtin_administrators,
            Trustee::TrustedInstaller => input.trust.admit_trusted_service,
            Trustee::CreatorOwner | Trustee::OwnerRights | Trustee::Untrusted => false,
        };
        if !trusted {
            saw_untrusted_replacement = true;
        }
    }
    if saw_untrusted_replacement {
        return Verdict::refuse("untrusted_ancestor_replacement", "T24-22");
    }
    if !input.pinned_without_share_delete {
        return Verdict::refuse("ancestor_pin_absent", "T24-22");
    }
    Verdict::allow("ancestor_replacement_rights_clear")
}

pub fn evaluate_ancestors(items: &[AncestorInput]) -> Verdict {
    if items.is_empty() {
        return Verdict::refuse("unverifiable_ancestor", "T24-22");
    }
    for item in items {
        let verdict = evaluate_ancestor(item);
        if !verdict.is_allow() {
            return verdict;
        }
    }
    Verdict::allow("ancestor_chain_pinned")
}

pub fn cross_role_identity(
    id_a: &FileIdentity,
    role_a: ProtectedRole,
    id_b: &FileIdentity,
    role_b: ProtectedRole,
) -> Verdict {
    if role_a != role_b && id_a == id_b {
        Verdict::deny("same_object_cross_role")
    } else {
        Verdict::allow("identities_distinct_or_same_role")
    }
}

pub fn private_regular_links(kind: ObjectKind, links: u64) -> Verdict {
    match kind {
        ObjectKind::Directory => Verdict::allow("directory_link_count_not_this_predicate"),
        ObjectKind::RegularFile if links == 1 => Verdict::allow("single_link_regular"),
        ObjectKind::RegularFile if links > 1 => Verdict::deny("multiple_hardlinks"),
        ObjectKind::Reparse => Verdict::deny("reparse_not_private_regular"),
        ObjectKind::RegularFile | ObjectKind::Other => {
            Verdict::refuse("link_count_unusable", "T24-12")
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnixObjectInput {
    pub uid: u32,
    pub current_euid: u32,
    pub mode: u32,
    pub kind: ObjectKind,
    pub extended_acl_foreign: Option<bool>,
    pub admit_library_0644: bool,
    pub parent_effective_private: bool,
    pub privileged_root_trust: bool,
}

pub fn evaluate_unix_private(input: UnixObjectInput) -> Verdict {
    if input.kind == ObjectKind::Reparse {
        return Verdict::deny("symlink_not_private_leaf");
    }
    let owner_ok = input.uid == input.current_euid
        || (input.uid == 0 && input.current_euid == 0 && input.privileged_root_trust);
    if !owner_ok {
        return Verdict::deny("foreign_unix_owner");
    }
    let bits = input.mode & 0o777;
    let exact = match input.kind {
        ObjectKind::Directory => bits == 0o700,
        ObjectKind::RegularFile => bits == 0o600,
        ObjectKind::Reparse | ObjectKind::Other => false,
    };
    let library_ok = input.admit_library_0644
        && input.parent_effective_private
        && input.kind == ObjectKind::RegularFile
        && bits == 0o644;
    if !exact && !library_ok {
        return Verdict::deny("unix_mode_policy");
    }
    match input.extended_acl_foreign {
        Some(true) => Verdict::deny("unix_foreign_acl"),
        Some(false) => Verdict::allow("unix_private_mode"),
        None if library_ok || exact => {
            Verdict::allow_note("unix_mode_acl_unread", "posix_acl_not_read")
        }
        None => Verdict::refuse("posix_acl_unverified", "T24-08"),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnixAncestorInput {
    pub owner_uid: u32,
    pub current_euid: u32,
    pub mode: u32,
    pub pinned: bool,
}

pub fn evaluate_unix_ancestor(input: UnixAncestorInput) -> Verdict {
    if !input.pinned {
        return Verdict::refuse("ancestor_pin_absent", "T24-45");
    }
    let trusted_owner = input.owner_uid == input.current_euid || input.owner_uid == 0;
    if !trusted_owner {
        return Verdict::refuse("untrusted_unix_ancestor_owner", "T24-22");
    }
    let group_other_write = input.mode & 0o022 != 0;
    let sticky = input.mode & 0o1000 != 0;
    if group_other_write && !sticky {
        return Verdict::refuse("untrusted_unix_ancestor_write", "T24-22");
    }
    Verdict::allow("unix_ancestor_ok")
}

/// Effective uid from a `/proc/self/status` `Uid:` line: real, effective, saved, fs.
/// A missing or non-decimal field refuses. This does not call `geteuid`.
pub fn parse_proc_status_euid(text: &str) -> Result<u32, Verdict> {
    let Some(line) = text.lines().find(|line| line.starts_with("Uid:")) else {
        return Err(Verdict::refuse("euid_unknown", "proc_status"));
    };
    let mut fields = line.split_whitespace();
    if fields.next() != Some("Uid:") {
        return Err(Verdict::refuse("euid_malformed", "proc_status"));
    }
    let _real = fields.next();
    let Some(effective) = fields.next() else {
        return Err(Verdict::refuse("euid_malformed", "proc_status"));
    };
    if effective.is_empty()
        || effective.len() > 10
        || !effective.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(Verdict::refuse("euid_malformed", "proc_status"));
    }
    effective
        .parse::<u32>()
        .map_err(|_| Verdict::refuse("euid_malformed", "proc_status"))
}

pub fn resolve_root(query: &RootQuery) -> Result<ResolvedRoot, Verdict> {
    if let Some(path) = query.explicit_override.as_deref() {
        return admit_absolute_root(path, query.platform, "explicit_override");
    }
    match query.platform {
        NativePlatform::Windows => {
            if let Some(path) = query.local_app_data.as_deref() {
                return admit_absolute_root(path, NativePlatform::Windows, "local_app_data");
            }
            if let Some(path) = query.userprofile.as_deref() {
                return admit_absolute_root(path, NativePlatform::Windows, "userprofile");
            }
            Err(Verdict::refuse("no_trustworthy_root", "T24-40"))
        }
        NativePlatform::Linux => linux_data_home(query),
        NativePlatform::Macos => Err(Verdict::refuse("platform_unsupported", "macos")),
    }
}

/// One Linux data directory.
/// For a non-root euid, an absolute `XDG_DATA_HOME` wins.
/// A relative or empty value is ignored, per the XDG Base Directory spec.
/// Otherwise the single default is `$HOME/.local/share`.
/// euid 0 does not consult `XDG_DATA_HOME`. It admits only `/root/.local/share`
/// when `HOME` is `/root`.
fn linux_data_home(query: &RootQuery) -> Result<ResolvedRoot, Verdict> {
    if query.euid_is_root {
        return match query.home.as_deref() {
            Some("/root") => {
                admit_absolute_root("/root/.local/share", NativePlatform::Linux, "root_own_home")
            }
            Some(_) => Err(Verdict::refuse("privileged_foreign_home", "T24-51")),
            None => Err(Verdict::refuse("no_trustworthy_root", "T24-40")),
        };
    }
    let home = query.home.as_deref();
    if let Some(xdg) = query.xdg_data_home.as_deref() {
        if linux_absolute(xdg) {
            return admit_absolute_root(xdg, NativePlatform::Linux, "xdg_data_home");
        }
    }
    let Some(home) = home else {
        return Err(Verdict::refuse("no_trustworthy_root", "T24-40"));
    };
    if !linux_absolute(home) {
        return Err(Verdict::refuse("relative_root", "T24-40"));
    }
    let mut path = home.trim_end_matches('/').to_string();
    path.push_str("/.local/share");
    admit_absolute_root(&path, NativePlatform::Linux, "xdg_data_home_default")
}

fn linux_absolute(path: &str) -> bool {
    path.starts_with('/') && !path.contains('\0')
}

fn admit_absolute_root(
    path: &str,
    platform: NativePlatform,
    source: &'static str,
) -> Result<ResolvedRoot, Verdict> {
    let relative = match platform {
        NativePlatform::Linux => !linux_absolute(path),
        NativePlatform::Windows | NativePlatform::Macos => {
            path.is_empty() || Path::new(path).is_relative()
        }
    };
    if relative {
        return Err(Verdict::refuse("relative_root", "T24-40"));
    }
    if platform == NativePlatform::Windows {
        let class = classify_windows_path(path);
        let verdict = syntax_admission(class);
        if !verdict.is_allow() {
            return Err(verdict);
        }
    }
    if path.contains('\0') {
        return Err(Verdict::refuse("non_roundtrip_name", "T24-50"));
    }
    Ok(ResolvedRoot {
        path: path.to_string(),
        source,
    })
}

pub fn broad_temp_rejected(parent: &Path) -> bool {
    let lower = lexical_temp_key(parent);
    lower.contains("/windows/temp")
        || lower == "/tmp"
        || lower.ends_with("/temp")
        || lower.ends_with("/tmp")
}

fn lexical_temp_key(parent: &Path) -> String {
    let mut text = parent.to_string_lossy().replace('\\', "/");
    loop {
        let trimmed = text.trim_end_matches('/');
        let stripped = trimmed.strip_suffix("/.").unwrap_or(trimmed);
        if stripped == text {
            break;
        }
        text = stripped.to_string();
    }
    if text.is_empty() {
        text = "/".to_string();
    }
    text.to_ascii_lowercase()
}

pub fn cancellation(owned: bool, publication_started: bool, completion_confirmed: bool) -> Verdict {
    if !owned {
        return Verdict::deny("foreign_process_cancel");
    }
    if publication_started || !completion_confirmed {
        return Verdict::refuse("cancel_outcome_unknown", "T24-36");
    }
    Verdict::cancelled("owned_work_cancelled")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceRequest {
    ReadOnlyProtocol,
    CreateMissingSidecar,
    Checkpoint,
    JournalMode,
    Ddl,
    BlindCopyLiveDuckDb,
}

pub fn source_request(request: SourceRequest) -> Verdict {
    match request {
        SourceRequest::ReadOnlyProtocol => Verdict::allow("sqlite_admitted_protocol"),
        SourceRequest::CreateMissingSidecar => Verdict::deny("create_missing_producer_sidecar"),
        SourceRequest::Checkpoint => Verdict::deny("checkpoint_forbidden"),
        SourceRequest::JournalMode => Verdict::deny("journal_mode_forbidden"),
        SourceRequest::Ddl => Verdict::deny("ddl_forbidden"),
        SourceRequest::BlindCopyLiveDuckDb => Verdict::refuse("duckdb_live_writer", "T24-33"),
    }
}

pub fn snapshot_retry_exhausted() -> Verdict {
    Verdict::refuse("snapshot_retry_exhausted", "T24-34")
}

pub fn crash_before_promotion() -> Verdict {
    Verdict::refuse("next_start_recovery_only", "T24-31")
}

pub fn pointer_kind_regular(regular_file: bool) -> Verdict {
    if regular_file {
        Verdict::allow("regular_manifest_pointer")
    } else {
        Verdict::deny("pointer_not_regular_file")
    }
}

pub fn retain_referenced_snapshot(still_referenced: bool, retention_decision: bool) -> Verdict {
    if still_referenced || !retention_decision {
        Verdict::deny("referenced_snapshot_retained")
    } else {
        Verdict::allow("authorized_snapshot_disposition")
    }
}

pub fn native_lane(required: NativePlatform, windows_native: bool, linux_native: bool) -> Verdict {
    let ok = match required {
        NativePlatform::Windows => windows_native,
        NativePlatform::Linux => linux_native,
        NativePlatform::Macos => false,
    };
    if ok {
        Verdict::allow_note("native_lane_present", "not_full_t24_acceptance")
    } else {
        Verdict::refuse("native_lane_not_satisfied", "T24-41")
    }
}

pub fn scoped_artifact_handoff(windows_receipt: bool, linux_receipt: bool) -> Verdict {
    if windows_receipt && !linux_receipt {
        Verdict::allow_note("scoped_construction_linux_hold", "not_full_t24_acceptance")
    } else if !windows_receipt && !linux_receipt {
        Verdict::refuse("no_native_receipt", "T24-42")
    } else {
        Verdict::allow_note(
            "receipts_do_not_close_product_acceptance",
            "not_full_t24_acceptance",
        )
    }
}

pub fn case_distinct_names(left: &str, right: &str) -> bool {
    left != right
}

pub fn component_stays_lexical(path: &Path) -> Result<(), Verdict> {
    for component in path.components() {
        match component {
            Component::ParentDir | Component::CurDir => {
                return Err(Verdict::refuse("child_name_rejected", "dot_component"));
            }
            Component::Normal(text) => {
                if text.to_str().is_none() {
                    return Err(Verdict::refuse("non_roundtrip_name", "T24-50"));
                }
            }
            Component::Prefix(_) | Component::RootDir => {}
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublicationReceipt {
    pub replaced_existing: bool,
    pub power_loss_guarantee: bool,
}

pub fn plan_cross_volume_then_local_pointer(
    copy_verified: bool,
    same_volume_rename_used: bool,
) -> Verdict {
    if same_volume_rename_used {
        return Verdict::deny("cross_volume_rename_not_atomic");
    }
    if copy_verified {
        Verdict::allow_note("cross_volume_plan_only", "execution_belongs_to_T24b")
    } else {
        Verdict::refuse("cross_volume_copy_unverified", "T24-29")
    }
}
