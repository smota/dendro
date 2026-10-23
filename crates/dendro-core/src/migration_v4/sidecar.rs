//! Synthetic schema3 key sidecar. This is not a production key store.
//! Layout: ASCII `DNDK` + little-endian u32 version + 32 ASCII key id + 32 key bytes.

use sha2::{Digest, Sha256};

use crate::privacy;

pub const SIDECAR_LEN: usize = 72;
pub const SIDECAR_VERSION: u32 = 1;
const MAGIC: &[u8; 4] = b"DNDK";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedSidecar {
    pub version: u32,
    pub key_id: String,
    pub key_bytes: [u8; 32],
    pub key_sha256_hex: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidecarError {
    Length,
    Magic,
    Version,
    KeyIdEncoding,
    KeyIdMismatch,
}

pub fn encode_sidecar(
    version: u32,
    key_bytes: &[u8; 32],
) -> Result<[u8; SIDECAR_LEN], SidecarError> {
    let key_id = privacy::key_id(key_bytes);
    if key_id.len() != 32 || !key_id.is_ascii() {
        return Err(SidecarError::KeyIdEncoding);
    }
    let mut out = [0u8; SIDECAR_LEN];
    out[0..4].copy_from_slice(MAGIC);
    out[4..8].copy_from_slice(&version.to_le_bytes());
    out[8..40].copy_from_slice(key_id.as_bytes());
    out[40..72].copy_from_slice(key_bytes);
    Ok(out)
}

pub fn parse_sidecar(bytes: &[u8]) -> Result<ParsedSidecar, SidecarError> {
    if bytes.len() != SIDECAR_LEN {
        return Err(SidecarError::Length);
    }
    if bytes[0..4] != MAGIC[..] {
        return Err(SidecarError::Magic);
    }
    let version = u32::from_le_bytes(bytes[4..8].try_into().expect("4 bytes"));
    if version != SIDECAR_VERSION {
        return Err(SidecarError::Version);
    }
    let key_id = std::str::from_utf8(&bytes[8..40]).map_err(|_| SidecarError::KeyIdEncoding)?;
    if key_id.len() != 32 || !key_id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(SidecarError::KeyIdEncoding);
    }
    let mut key_bytes = [0u8; 32];
    key_bytes.copy_from_slice(&bytes[40..72]);
    let computed = privacy::key_id(&key_bytes);
    if computed != key_id {
        return Err(SidecarError::KeyIdMismatch);
    }
    let key_sha256_hex = hex::encode(Sha256::digest(key_bytes));
    Ok(ParsedSidecar {
        version,
        key_id: key_id.to_string(),
        key_bytes,
        key_sha256_hex,
    })
}
