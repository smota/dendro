#![deny(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]
#![cfg(windows)]
mod ffi;
#[cfg(test)]
mod production_tests;
#[cfg(test)]
mod tests;
use std::{
    fs::File,
    io::{self, Write},
    mem::{offset_of, size_of},
    os::windows::io::AsHandle,
};
use windows_namespace_request_sys::{
    CapturedHandle, FileInformationClass, FinalPathFlags, QueryFileInformation, QueryFinalPath,
    QueryVolumeInformation,
};
use windows_sys::Wdk::Storage::FileSystem::{FILE_LINKS_INFORMATION, FILE_LINK_ENTRY_INFORMATION};

/// Lowercase ASCII leaf only: no paths, streams, devices or case-only aliases.
#[derive(Debug)]
pub struct ChildName(Vec<u16>);
impl ChildName {
    pub fn parse(name: &str) -> io::Result<Self> {
        let stem = name.split('.').next().unwrap_or("");
        let device = matches!(stem, "con" | "prn" | "aux" | "nul")
            || (stem.len() == 4
                && (stem.starts_with("com") || stem.starts_with("lpt"))
                && stem.as_bytes()[3].is_ascii_digit());
        if name.is_empty()
            || name.len() > 100
            || name.starts_with('.')
            || name.ends_with('.')
            || device
            || !name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid lowercase private child name",
            ));
        }
        Ok(Self(name.encode_utf16().collect()))
    }
}
fn bad(s: &str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, s)
}
fn other(e: impl std::fmt::Display) -> io::Error {
    io::Error::other(e.to_string())
}
fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "short/malformed native query")
}
fn bytes_at<const N: usize>(b: &[u8], offset: usize) -> io::Result<[u8; N]> {
    b.get(offset..offset.checked_add(N).ok_or_else(invalid)?)
        .ok_or_else(invalid)?
        .try_into()
        .map_err(other)
}
fn capture(file: &File) -> io::Result<CapturedHandle> {
    CapturedHandle::capture(file.as_handle()).map_err(other)
}
fn decode_id(b: &[u8]) -> io::Result<(u64, [u8; 16])> {
    Ok((u64::from_le_bytes(bytes_at(b, 0)?), bytes_at(b, 8)?))
}
fn typed_staging_identity(
    (volume_serial_64, file_id): (u64, [u8; 16]),
) -> io::Result<dendro_platform_policy::FileIdentity> {
    if file_id == [0; 16] {
        return Err(bad("file_id_128_unusable"));
    }
    Ok(dendro_platform_policy::FileIdentity::WindowsFileId128 {
        volume_serial_64,
        file_id,
    })
}
/// Held 128-bit identity, with bounds checked before decoding.
pub fn identity(file: &File) -> io::Result<(u64, [u8; 16])> {
    let b = QueryFileInformation::new(capture(file)?, FileInformationClass::ID)
        .with_capacity(24)
        .perform()
        .map_err(other)?;
    decode_id(b.as_slice())
}
fn verify(file: &File, directory: bool) -> io::Result<u64> {
    verify_inner(file, directory, false)
}
fn verify_inner(file: &File, directory: bool, pending: bool) -> io::Result<u64> {
    let m = file.metadata()?;
    let i = winapi_util::file::information(file)?;
    if m.is_dir() != directory
        || (!directory && !m.is_file())
        || i.file_attributes() & 0x400 != 0
        || (!directory && i.number_of_links() != 1 && !(pending && i.number_of_links() == 0))
    {
        return Err(bad("wrong kind, reparse or links"));
    }
    let v = QueryVolumeInformation::new(capture(file)?)
        .perform()
        .map_err(other)?;
    let nt = QueryFinalPath::new(capture(file)?)
        .with_flags(FinalPathFlags::VOLUME_NAME_NT)
        .perform()
        .map_err(other)?
        .to_string_lossy();
    let guid = QueryFinalPath::new(capture(file)?)
        .with_flags(FinalPathFlags::VOLUME_NAME_GUID)
        .perform()
        .map_err(other)?
        .to_string_lossy();
    let local = nt
        .strip_prefix(r"\Device\HarddiskVolume")
        .and_then(|r| r.split_once('\\'))
        .is_some_and(|(n, _)| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
    // NT volume path has no drive colon: any colon means a named/typed stream.
    if nt.contains(':') {
        return Err(bad("held stream refused"));
    }
    if v.filesystem_name().to_string_lossy() != "NTFS"
        || !local
        || !guid.starts_with(r"\\?\Volume{")
    {
        return Err(bad("local NTFS profile required"));
    }
    verify_private_security(file, directory)?;
    Ok(identity(file)?.0)
}
// Unsupported, deny, callback, object, missing and NULL DACLs are rejected
// by win-custody's typed parser before any allow ACE is passed to policy.
fn verify_private_security(file: &File, directory: bool) -> io::Result<()> {
    use dendro_platform_policy::{
        evaluate_private_object, AceView, OwnerEvidence, PrivateObjectInput, TrustPolicy, Trustee,
    };
    let sd = win_custody::ObjectSecurityDescriptor::query_kernel_object(file)?;
    let dacl = sd.dacl().map_err(other)?;
    let me = win_custody::OwnedSid::from_string(&win_custody::current_user_sid_string()?)?;
    let system = win_custody::OwnedSid::from_string("S-1-5-18")?;
    let admin = win_custody::OwnedSid::from_string("S-1-5-32-544")?;
    let creator = win_custody::OwnedSid::from_string("S-1-3-0")?;
    let owner_rights = win_custody::OwnedSid::from_string("S-1-3-4")?;
    let installer = win_custody::OwnedSid::from_string(
        "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464",
    )?;
    let owner = sd.owner().map_err(other)?;
    let owner = if owner.matches(&me) {
        OwnerEvidence::CurrentUser
    } else if owner.matches(&system) {
        OwnerEvidence::LocalSystem
    } else if owner.matches(&admin) {
        OwnerEvidence::BuiltinAdministrators
    } else if owner.matches(&installer) {
        OwnerEvidence::TrustedService
    } else {
        OwnerEvidence::Foreign
    };
    let aces = dacl
        .allowed_aces()
        .map(|ace| {
            let trustee = if ace.trustee_matches(&me) {
                Trustee::CurrentUser
            } else if ace.trustee_matches(&system) {
                Trustee::LocalSystem
            } else if ace.trustee_matches(&admin) {
                Trustee::BuiltinAdministrators
            } else if ace.trustee_matches(&creator) {
                Trustee::CreatorOwner
            } else if ace.trustee_matches(&owner_rights) {
                Trustee::OwnerRights
            } else if ace.trustee_matches(&installer) {
                Trustee::TrustedInstaller
            } else {
                Trustee::Untrusted
            };
            AceView {
                trustee,
                mask: ace.mask(),
                flags: ace.flags(),
            }
        })
        .collect();
    let verdict = evaluate_private_object(&PrivateObjectInput {
        owner,
        dacl_present: true,
        dacl_protected: sd.control()? & 0x1000 != 0,
        require_protected: directory,
        trust: TrustPolicy::FROZEN_HOST,
        aces,
        read_only_attribute: false,
        deny_ace_present: false,
    });
    if !verdict.is_allow() {
        return Err(bad(verdict.code));
    }
    Ok(())
}

/// Point-in-time checks cannot freeze ACLs, links, contents or deletion state
/// changed by the same user, SYSTEM, Administrators or kernel actors.
pub const SAME_PRINCIPAL_RESIDUAL: &str =
    "held handles prevent substitution; trusted principals may change state between check and syscall";

/// Verified held private directory. No pathname or raw handle is exposed.
pub struct VerifiedDirectory {
    file: File,
}
impl VerifiedDirectory {
    pub fn from_held(file: &File) -> io::Result<Self> {
        verify(file, true)?;
        let file = file.try_clone()?;
        ffi::clear_inherit(&file)?;
        verify(&file, true)?;
        Ok(Self { file })
    }
    fn duplicate(&self) -> io::Result<Self> {
        Self::from_held(&self.file)
    }
    /// Creates exclusively under this held parent, never by reconstructed path.
    pub fn create_source(&self, name: &ChildName) -> io::Result<StagingSource> {
        verify(&self.file, true)?;
        let file = ffi::create(&self.file, &name.0)
            .map_err(|e| io::Error::new(e.kind(), format!("anchored create: {e}")))?;
        let source = StagingSource {
            file,
            parent: self.duplicate()?,
        };
        source
            .check(false)
            .map_err(|e| io::Error::new(e.kind(), format!("new source check: {e}")))?;
        Ok(source)
    }
}
/// Module-created, non-delete-on-close, synchronous staging source. No capture,
/// clone, Debug, File accessor or raw-handle capability is publicly available.
pub struct StagingSource {
    file: File,
    parent: VerifiedDirectory,
}
impl StagingSource {
    fn check(&self, allow_pending: bool) -> io::Result<()> {
        ffi::require_synchronous(&self.file)?;
        ffi::require_noninherited(&self.file)?;
        let standard =
            QueryFileInformation::new(capture(&self.file)?, FileInformationClass::from_raw(1))
                .with_capacity(24)
                .perform()
                .map_err(other)?;
        let b = standard.as_slice();
        let pending = bytes_at::<1>(b, 20)?[0] != 0;
        let standard_links = u32::from_le_bytes(bytes_at(b, 16)?);
        if (pending && !allow_pending)
            || (standard_links != 1 && !(pending && allow_pending && standard_links == 0))
        {
            return Err(bad("source links/delete-pending"));
        }
        if verify_inner(&self.file, false, pending && allow_pending)?
            != verify(&self.parent.file, true)?
        {
            return Err(bad("source parent volume mismatch"));
        }
        let links = ffi::query(&self.file, 46, 4096)?;
        let b = &links;
        let entry = offset_of!(FILE_LINKS_INFORMATION, Entry);
        let end = entry
            .checked_add(size_of::<FILE_LINK_ENTRY_INFORMATION>())
            .ok_or_else(invalid)?;
        if b.len() < end
            || u32::from_le_bytes(bytes_at(b, 0)?) as usize > b.len()
            || u32::from_le_bytes(bytes_at(b, 4)?) != 1
            || u32::from_le_bytes(bytes_at(b, entry)?) != 0
        {
            return Err(bad("single parent link unproven"));
        }
        let name_units = u32::from_le_bytes(bytes_at(
            b,
            entry + offset_of!(FILE_LINK_ENTRY_INFORMATION, FileNameLength),
        )?) as usize;
        let name_bytes = name_units.checked_mul(2).ok_or_else(invalid)?;
        let name_start = entry
            .checked_add(offset_of!(FILE_LINK_ENTRY_INFORMATION, FileName))
            .ok_or_else(invalid)?;
        if b.get(name_start..name_start.checked_add(name_bytes).ok_or_else(invalid)?)
            .is_none()
        {
            return Err(invalid());
        }
        let parent_id = u64::from_le_bytes(bytes_at(
            b,
            entry + offset_of!(FILE_LINK_ENTRY_INFORMATION, ParentFileId),
        )?);
        let expected = winapi_util::file::information(&self.parent.file)?.file_index();
        if parent_id == 0 || parent_id != expected {
            return Err(bad("source outside held staging parent"));
        }
        Ok(())
    }
    /// Full identity of this module-created source after its held checks.
    /// Persist all 24 bytes. A later path observation is not authorization.
    pub fn identity(&mut self) -> io::Result<dendro_platform_policy::FileIdentity> {
        self.check(false)?;
        typed_staging_identity(identity(&self.file)?)
    }
    pub fn write_all(&mut self, data: &[u8]) -> io::Result<()> {
        self.check(false)?;
        self.file.write_all(data)
    }
    /// Flush the verified held source before an absent-name custody rename.
    pub fn sync_all(&self) -> io::Result<()> {
        self.check(false)?;
        self.file.sync_all()?;
        self.check(false)
    }
    pub fn rename_absent(
        &mut self,
        destination: &VerifiedDirectory,
        name: &ChildName,
    ) -> io::Result<()> {
        self.check(false)?;
        if identity(&self.file)?.0 != verify(&destination.file, true)? {
            return Err(bad("cross-volume rename refused"));
        }
        let next = destination.duplicate()?;
        self.check(false)?;
        ffi::rename(&self.file, &destination.file, &name.0, false)?;
        self.parent = next;
        self.check(false)
    }
    /// Consumes on success AND error. No post-publication deletion setter exists.
    /// Data sync precedes one NT namespace replacement; directory durability is separate.
    /// ```compile_fail
    /// use dendro_windows_private_io::{StagingSource,VerifiedDirectory,ChildName};
    /// fn no_reuse(mut s:StagingSource,p:&VerifiedDirectory,n:&ChildName){
    ///     s.publish_replace(p,n).unwrap(); s.set_delete(true).unwrap();
    /// }
    /// ```
    pub fn publish_replace(
        self,
        destination: &VerifiedDirectory,
        name: &ChildName,
    ) -> io::Result<()> {
        self.check(false)?;
        if identity(&self.file)?.0 != verify(&destination.file, true)? {
            return Err(bad("cross-volume publication refused"));
        }
        self.file.sync_all()?;
        // Check immediately before mutation; same-user/privileged races remain out of scope.
        self.check(false)?;
        ffi::rename(&self.file, &destination.file, &name.0, true)
    }
    pub fn set_delete(&mut self, delete: bool) -> io::Result<()> {
        self.check(!delete)?;
        ffi::disposition(&self.file, delete)
    }
}
