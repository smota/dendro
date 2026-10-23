//! Sole unsafe boundary. See AUDIT.md; first rework, not admission.
#![allow(unsafe_code)]
use std::{
    fs::File,
    io,
    mem::{align_of, offset_of, size_of},
    os::windows::io::{AsRawHandle, FromRawHandle},
    ptr,
};
use windows_sys::{
    Wdk::{
        Foundation::OBJECT_ATTRIBUTES,
        Storage::FileSystem::{
            FileDispositionInformation, FileRenameInformation, NtCreateFile,
            NtQueryInformationFile, NtSetInformationFile, FILE_RENAME_INFORMATION,
        },
    },
    Win32::{
        Foundation::{
            GetHandleInformation, RtlNtStatusToDosError, SetHandleInformation, HANDLE_FLAG_INHERIT,
            INVALID_HANDLE_VALUE, UNICODE_STRING, WAIT_OBJECT_0,
        },
        System::{
            Threading::{WaitForSingleObject, INFINITE},
            IO::IO_STATUS_BLOCK,
        },
    },
};
fn error(status: i32) -> io::Error {
    // SAFETY: scalar NTSTATUS translation, no pointers or ownership changes.
    io::Error::from_raw_os_error(unsafe { RtlNtStatusToDosError(status) } as i32)
}
pub(super) fn clear_inherit(file: &File) -> io::Result<()> {
    // SAFETY: live borrowed File; changes only its handle-table inherit flag.
    if unsafe { SetHandleInformation(file.as_raw_handle(), HANDLE_FLAG_INHERIT, 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    require_noninherited(file)
}
pub(super) fn require_noninherited(file: &File) -> io::Result<()> {
    let mut flags = 0;
    // SAFETY: genuine live handle and initialized writable DWORD output.
    if unsafe { GetHandleInformation(file.as_raw_handle(), &mut flags) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if flags & HANDLE_FLAG_INHERIT != 0 {
        return Err(io::Error::other("inheritable staging handle"));
    }
    Ok(())
}
fn allocate(bytes: usize) -> io::Result<Vec<usize>> {
    let words = bytes
        .checked_add(size_of::<usize>() - 1)
        .ok_or_else(|| io::Error::other("buffer overflow"))?
        / size_of::<usize>();
    Ok(vec![0; words])
}
fn complete(file: &File, mut status: i32, iosb: *const IO_STATUS_BLOCK) -> io::Result<()> {
    if status == 0x103 {
        // SAFETY: module-created synchronous File with SYNCHRONIZE. Private
        // capability serializes calls; caller retains status/output allocations.
        if unsafe { WaitForSingleObject(file.as_raw_handle(), INFINITE) } != WAIT_OBJECT_0 {
            std::process::abort();
        }
        // SAFETY: wait completed the only request; NT writes Status union member.
        status = unsafe { (*iosb).Anonymous.Status };
    }
    if status < 0 {
        return Err(error(status));
    }
    Ok(())
}
pub(super) fn create(parent: &File, name: &[u16]) -> io::Result<File> {
    create_inner(parent, name, ptr::null(), false)
}
fn create_inner(
    parent: &File,
    name: &[u16],
    sd: *const windows_sys::Win32::Security::SECURITY_DESCRIPTOR,
    directory: bool,
) -> io::Result<File> {
    let mut utf16 = name.to_vec();
    let bytes = u16::try_from(
        utf16
            .len()
            .checked_mul(2)
            .ok_or_else(|| io::Error::other("UTF16 overflow"))?,
    )
    .map_err(io::Error::other)?;
    let text = UNICODE_STRING {
        Length: bytes,
        MaximumLength: bytes,
        Buffer: utf16.as_mut_ptr(),
    };
    let attrs = OBJECT_ATTRIBUTES {
        Length: u32::try_from(size_of::<OBJECT_ATTRIBUTES>()).map_err(io::Error::other)?,
        RootDirectory: parent.as_raw_handle(),
        ObjectName: &text,
        Attributes: 0x40,
        SecurityDescriptor: sd,
        SecurityQualityOfService: ptr::null(),
    };
    let mut iosb = Box::<IO_STATUS_BLOCK>::default();
    let status_ptr = ptr::from_mut(&mut *iosb);
    let mut raw = ptr::null_mut();
    let kind = if directory { 1 } else { 0x40 };
    // SAFETY: borrowed parent remains live; local Unicode/attributes and owned
    // string remain valid throughout synchronous creation. Single validated leaf,
    // FILE_CREATE refuses existing names; no delete-on-close, OBJ_INHERIT or
    // backup privilege bypass. FILE_SYNCHRONOUS_IO_NONALERT with SYNCHRONIZE.
    // SD is null normally; test-only fixture caller owns a live converted SD.
    let status = unsafe {
        NtCreateFile(
            &mut raw,
            0xc0110000,
            &attrs,
            status_ptr,
            ptr::null(),
            0,
            7,
            2,
            0x00200020 | kind,
            ptr::null(),
            0,
        )
    };
    // Synchronous create completes internally. Fail-stop on unexpected pending
    // before stack inputs/output handle/status can leave scope.
    if status == 0x103 {
        std::process::abort();
    }
    if status < 0 {
        return Err(error(status));
    }
    if raw.is_null() || raw == INVALID_HANDLE_VALUE {
        return Err(io::Error::other("NT returned invalid created handle"));
    }
    // SAFETY: successful exclusive create returns one new owned OS handle.
    let file = unsafe { File::from_raw_handle(raw) };
    clear_inherit(&file)?;
    require_synchronous(&file)?;
    Ok(file)
}
/// Bounded native query for module-created synchronous handles only. Classes and
/// capacities are private constants selected by this crate, no arbitrary callers.
pub(super) fn query(file: &File, class: i32, capacity: usize) -> io::Result<Vec<u8>> {
    let mut storage = allocate(capacity)?;
    let mut iosb = Box::<IO_STATUS_BLOCK>::default();
    let status_ptr = ptr::from_mut(&mut *iosb);
    let length = u32::try_from(capacity).map_err(io::Error::other)?;
    // SAFETY: aligned zeroed owned output covers length; boxed IOSB stays live
    // through completion. Private synchronous source and query class require no
    // input pointers or enumeration cursor. Output buffer cannot escape pending.
    let status = unsafe {
        NtQueryInformationFile(
            file.as_raw_handle(),
            status_ptr,
            storage.as_mut_ptr().cast(),
            length,
            class,
        )
    };
    complete(file, status, status_ptr)
        .map_err(|e| io::Error::new(e.kind(), format!("query class {class}: {e}")))?;
    let written = iosb.Information;
    if written > capacity {
        return Err(io::Error::other("native query exceeded buffer"));
    }
    // SAFETY: initialized allocation covers written bytes and call is complete.
    Ok(unsafe { std::slice::from_raw_parts(storage.as_ptr().cast::<u8>(), written) }.to_vec())
}
pub(super) fn require_synchronous(file: &File) -> io::Result<()> {
    let bytes = query(file, 16, 4)?;
    let mode = u32::from_le_bytes(
        bytes
            .get(..4)
            .ok_or_else(|| io::Error::other("short FileModeInformation"))?
            .try_into()
            .map_err(io::Error::other)?,
    );
    if mode & 0x30 == 0 || mode & 0x1000 != 0 {
        return Err(io::Error::other("not synchronous or delete-on-close set"));
    }
    Ok(())
}
fn nt_set(file: &File, storage: Vec<usize>, length: u32, class: i32) -> io::Result<()> {
    let mut iosb = Box::<IO_STATUS_BLOCK>::default();
    let status_ptr = ptr::from_mut(&mut *iosb);
    // SAFETY: private synchronous handle; initialized aligned input covers length.
    // NT captures set-input during call; boxed IOSB may be written later and is
    // retained through completion. Retaining input too is conservative.
    let status = unsafe {
        NtSetInformationFile(
            file.as_raw_handle(),
            status_ptr,
            storage.as_ptr().cast(),
            length,
            class,
        )
    };
    complete(file, status, status_ptr)
}
pub(super) fn rename(source: &File, parent: &File, name: &[u16], replace: bool) -> io::Result<()> {
    let bytes = name
        .len()
        .checked_mul(size_of::<u16>())
        .ok_or_else(|| io::Error::other("UTF16 byte overflow"))?;
    let offset = offset_of!(FILE_RENAME_INFORMATION, FileName);
    let length = size_of::<FILE_RENAME_INFORMATION>()
        .checked_add(bytes)
        .and_then(|n| n.checked_add(2))
        .ok_or_else(|| io::Error::other("rename size overflow"))?;
    let length32 = u32::try_from(length).map_err(io::Error::other)?;
    let bytes32 = u32::try_from(bytes).map_err(io::Error::other)?;
    assert!(align_of::<usize>() >= align_of::<FILE_RENAME_INFORMATION>());
    let mut storage = allocate(length)?;
    let header = storage.as_mut_ptr().cast::<FILE_RENAME_INFORMATION>();
    // SAFETY: zeroed aligned allocation covers fixed header plus checked UTF16
    // tail and spare NUL. No reference spans nominal one-element flexible field.
    // ChildName owns the only admitted leaf encoding; input and output disjoint.
    // Both raw handles are borrowed and kept live through syscall completion.
    unsafe {
        (*header).Anonymous.ReplaceIfExists = replace;
        (*header).RootDirectory = parent.as_raw_handle();
        (*header).FileNameLength = bytes32;
        ptr::copy_nonoverlapping(
            name.as_ptr(),
            storage.as_mut_ptr().cast::<u8>().add(offset).cast::<u16>(),
            name.len(),
        );
    }
    nt_set(source, storage, length32, FileRenameInformation)
}
pub(super) fn disposition(source: &File, delete: bool) -> io::Result<()> {
    // One BOOLEAN, canonical 0/1 byte; supported Windows targets are little endian.
    nt_set(
        source,
        vec![usize::from(delete)],
        1,
        FileDispositionInformation,
    )
}
#[cfg(test)]
pub(super) fn fixture_directory(parent: &File, name: &[u16], sddl: &str) -> io::Result<File> {
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW,
    };
    let text = sddl.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let mut raw = ptr::null_mut();
    // SAFETY: terminated input, live out pointer; successful SD owned until free.
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            text.as_ptr(),
            1,
            &mut raw,
            ptr::null_mut(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let result = create_inner(parent, name, raw.cast(), true);
    // SAFETY: converted LocalAlloc SD released once, after synchronous create.
    unsafe { LocalFree(raw) };
    result
}
