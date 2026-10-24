//! Ext4 admission by a held descriptor's kernel mount ID, never path prefixes.
#![deny(unsafe_code)]

use std::io::{BufRead, BufReader, Read};
use std::os::fd::AsFd;

use rustix::fs::{statx, AtFlags, StatxFlags};

use super::{PlatformError, Verdict};

const MAX_MOUNTINFO_BYTES: usize = 8 * 1024 * 1024;
const MAX_MOUNTINFO_LINE: usize = 64 * 1024;

fn refuse(code: &'static str) -> PlatformError {
    PlatformError::Rejected(Verdict::refuse(code, "held_fd_statx_mountinfo"))
}

pub(super) fn admit_ext4_mount(fd: impl AsFd) -> Result<(), PlatformError> {
    let observed = statx(&fd, "", AtFlags::EMPTY_PATH, StatxFlags::MNT_ID)
        .map_err(|_| refuse("mount_identity_unverified"))?;
    if observed.stx_mask & StatxFlags::MNT_ID.bits() == 0 || observed.stx_mnt_id == 0 {
        return Err(refuse("mount_identity_unverified"));
    }
    let file = std::fs::File::open("/proc/self/mountinfo")
        .map_err(|_| refuse("mount_identity_unverified"))?;
    verify_mountinfo(
        BufReader::new(file),
        observed.stx_mnt_id,
        observed.stx_dev_major,
        observed.stx_dev_minor,
    )
}

fn decimal(value: &[u8]) -> Result<u64, PlatformError> {
    if value.is_empty() || !value.iter().all(u8::is_ascii_digit) {
        return Err(refuse("mount_identity_unverified"));
    }
    std::str::from_utf8(value)
        .ok()
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| refuse("mount_identity_unverified"))
}

fn verify_mountinfo(
    mut reader: impl BufRead,
    mount_id: u64,
    major: u32,
    minor: u32,
) -> Result<(), PlatformError> {
    let mut total = 0usize;
    let mut found = false;
    let mut line = Vec::new();
    loop {
        line.clear();
        let read = reader
            .by_ref()
            .take((MAX_MOUNTINFO_LINE + 1) as u64)
            .read_until(b'\n', &mut line)
            .map_err(|_| refuse("mount_identity_unverified"))?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(read)
            .ok_or_else(|| refuse("mountinfo_outside_bound"))?;
        if read > MAX_MOUNTINFO_LINE || total > MAX_MOUNTINFO_BYTES {
            return Err(refuse("mountinfo_outside_bound"));
        }
        if line.last() != Some(&b'\n') {
            return Err(refuse("mount_identity_unverified"));
        }
        let fields: Vec<_> = line
            .split(|byte| byte.is_ascii_whitespace())
            .filter(|field| !field.is_empty())
            .collect();
        if fields.len() < 10 {
            return Err(refuse("mount_identity_unverified"));
        }
        let id = decimal(fields[0])?;
        if id != mount_id {
            continue;
        }
        if found {
            return Err(refuse("mount_identity_ambiguous"));
        }
        let separator = fields
            .iter()
            .position(|field| *field == b"-")
            .filter(|index| *index >= 6 && fields.len() == index + 4)
            .ok_or_else(|| refuse("mount_identity_unverified"))?;
        let mut device = fields[2].split(|byte| *byte == b':');
        if decimal(device.next().unwrap_or_default())? != u64::from(major)
            || decimal(device.next().unwrap_or_default())? != u64::from(minor)
            || device.next().is_some()
        {
            return Err(refuse("mount_identity_unverified"));
        }
        if fields[separator + 1] != b"ext4" {
            return Err(refuse("unadmitted_filesystem"));
        }
        found = true;
    }
    if !found {
        return Err(refuse("mount_identity_unverified"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mount_id_and_device_bind_exact_ext4_type() {
        let ext4 = b"41 1 8:2 / /home rw,relatime - ext4 /dev/test rw\n";
        assert!(verify_mountinfo(&ext4[..], 41, 8, 2).is_ok());
        for other in [
            b"ext2".as_slice(),
            b"ext3",
            b"xfs",
            b"btrfs",
            b"overlay",
            b"9p",
        ] {
            let text = format!(
                "41 1 8:2 / /home rw - {} /dev/test rw\n",
                std::str::from_utf8(other).unwrap()
            );
            assert!(verify_mountinfo(text.as_bytes(), 41, 8, 2).is_err());
        }
        assert!(verify_mountinfo(&ext4[..], 42, 8, 2).is_err());
        assert!(verify_mountinfo(&ext4[..], 41, 8, 3).is_err());
        let duplicate = [ext4.as_slice(), ext4.as_slice()].concat();
        assert!(verify_mountinfo(&duplicate[..], 41, 8, 2).is_err());
    }

    #[test]
    fn missing_truncated_oversize_and_malformed_mountinfo_refuse() {
        assert!(verify_mountinfo(&b""[..], 41, 8, 2).is_err());
        assert!(
            verify_mountinfo(&b"41 1 8:2 / /home rw - ext4 /dev/test rw"[..], 41, 8, 2).is_err()
        );
        assert!(
            verify_mountinfo(&b"41 1 bad / /home rw - ext4 /dev/test rw\n"[..], 41, 8, 2).is_err()
        );
        assert!(verify_mountinfo(&b"41 1 8:2 / /home rw - ext4\n"[..], 41, 8, 2).is_err());
        let oversized = vec![b'x'; MAX_MOUNTINFO_LINE + 1];
        assert!(verify_mountinfo(&oversized[..], 41, 8, 2).is_err());
    }
}
