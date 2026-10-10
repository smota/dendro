use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use fd_lock::RwLock;
use rand::RngCore;

use crate::privacy::key_id;

const MAGIC: &[u8] = b"DNDK";

pub fn with_output_lock<T>(lock_path: &Path, body: impl FnOnce() -> Result<T>) -> Result<T> {
    if let Some(parent) = lock_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(lock_path)?;
    let mut lock = RwLock::new(file);
    let _guard = lock
        .try_write()
        .map_err(|_| anyhow!("analytics output lock is busy"))?;
    body()
}

pub struct KeyMaterial {
    pub id: String,
    pub bytes: [u8; 32],
}

pub fn load_or_create_key(path: &Path, allow_create: bool) -> Result<KeyMaterial> {
    if path.exists() {
        return read_key(path);
    }
    if !allow_create {
        return Err(anyhow!("analytics key is missing"));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    let id = key_id(&bytes);
    let mut body = Vec::new();
    body.extend_from_slice(MAGIC);
    body.extend_from_slice(&1u32.to_le_bytes());
    body.extend_from_slice(id.as_bytes());
    body.extend_from_slice(&bytes);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(&body)?;
    file.sync_all()?;
    Ok(KeyMaterial { id, bytes })
}

pub fn read_key(path: &Path) -> Result<KeyMaterial> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.file_type().is_file() {
        return Err(anyhow!("analytics key must be a regular file"));
    }
    if meta.permissions().mode() & 0o077 != 0 {
        return Err(anyhow!("analytics key permissions are too open"));
    }
    let mut file = File::open(path)?;
    let mut body = Vec::new();
    file.read_to_end(&mut body)?;
    if body.len() < 4 + 4 + 32 + 32 || &body[..4] != MAGIC {
        return Err(anyhow!("analytics key file is not valid"));
    }
    let id = String::from_utf8(body[8..40].to_vec())
        .map_err(|_| anyhow!("analytics key id is invalid"))?;
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(&body[40..72]);
    if key_id(&bytes) != id {
        return Err(anyhow!("analytics key id does not match key material"));
    }
    Ok(KeyMaterial { id, bytes })
}

pub fn refuse_aliased_output(dest: &Path, source_roots: &[PathBuf]) -> Result<()> {
    if dest
        .symlink_metadata()
        .is_ok_and(|m| m.file_type().is_symlink())
    {
        return Err(anyhow!("refusing symlink analytics output"));
    }
    let dest_parent = dest.parent().unwrap_or_else(|| Path::new("."));
    let parent_canon = fs::canonicalize(dest_parent).unwrap_or_else(|_| dest_parent.to_path_buf());
    for root in source_roots {
        if let Ok(canon) = fs::canonicalize(root) {
            if parent_canon.starts_with(&canon) {
                return Err(anyhow!("refusing analytics output inside a source root"));
            }
        }
    }
    if dest.exists() {
        let dest_meta = fs::metadata(dest)?;
        if dest_meta.file_type().is_symlink() {
            return Err(anyhow!("refusing symlink analytics output"));
        }
        for root in source_roots {
            if same_inode_under(root, &dest_meta)? {
                return Err(anyhow!(
                    "refusing analytics output that aliases a source file"
                ));
            }
        }
        let wal = wal_sibling(dest);
        if wal.exists() {
            return Err(anyhow!(
                "refusing output that already has a DuckDB wal sibling"
            ));
        }
    }
    Ok(())
}

fn wal_sibling(dest: &Path) -> PathBuf {
    let mut name = dest.as_os_str().to_os_string();
    name.push(".wal");
    dest.with_file_name(name)
}

fn same_inode_under(root: &Path, dest_meta: &fs::Metadata) -> Result<bool> {
    use std::os::unix::fs::MetadataExt;
    let dest_key = (dest_meta.dev(), dest_meta.ino());
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let meta = match fs::symlink_metadata(&path) {
                Ok(meta) => meta,
                Err(_) => continue,
            };
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.file_type().is_dir() {
                stack.push(path);
                continue;
            }
            if (meta.dev(), meta.ino()) == dest_key {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

pub fn sync_dir(path: &Path) -> Result<()> {
    File::open(path)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn key_is_created_once_and_reused() {
        let dir = std::env::temp_dir().join(format!("dendro-key-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("analytics.duckdb.key");
        let created = load_or_create_key(&path, true).unwrap();
        let loaded = load_or_create_key(&path, false).unwrap();
        assert_eq!(created.id, loaded.id);
        assert_eq!(created.bytes, loaded.bytes);
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let err = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn symlink_output_is_refused() {
        let dir = std::env::temp_dir().join(format!("dendro-link-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("src")).unwrap();
        let target = dir.join("src").join("secret.duckdb");
        fs::write(&target, b"source").unwrap();
        let link = dir.join("out.duckdb");
        symlink(&target, &link).unwrap();
        let err = refuse_aliased_output(&link, &[dir.join("src")]).unwrap_err();
        assert!(err.to_string().contains("symlink"));
        assert_eq!(fs::read(&target).unwrap(), b"source");
        let _ = fs::remove_dir_all(&dir);
    }
}
