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
    if fs::symlink_metadata(lock_path).is_ok_and(|meta| !meta.file_type().is_file()) {
        return Err(anyhow!(
            "refusing an output lock that is not a regular file"
        ));
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(lock_path)?;
    let mut lock = RwLock::new(file);
    let guard = lock
        .try_write()
        .map_err(|_| anyhow!("analytics output lock is busy"))?;
    // The locked inode must be the one named by the lock path (not reached
    // through a symlink) and have one link, so every creator and recovery
    // uses the same lock.
    {
        use std::os::unix::fs::MetadataExt;
        let held = guard.metadata()?;
        let named = fs::symlink_metadata(lock_path)?;
        if held.ino() != named.ino() || held.dev() != named.dev() || held.nlink() != 1 {
            return Err(anyhow!("output lock identity is unsafe"));
        }
    }
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

/// Refuse output/source collisions using metadata only, before output effects.
pub fn refuse_aliased_output(dest: &Path, source_roots: &[PathBuf]) -> Result<()> {
    refuse_aliased_output_with_openrig(dest, source_roots, &[])?;
    refuse_existing_output_wal(dest)
}

/// Typed OpenRig roots protect their selected main and associated sidecar names.
/// This is a static namespace check, not protection against concurrent replacement.
pub fn refuse_aliased_output_with_openrig(
    dest: &Path,
    source_roots: &[PathBuf],
    openrig_roots: &[PathBuf],
) -> Result<()> {
    // Raw lstat catches even dangling final symlinks. The resolver reapplies
    // this rule if a missing component followed by `..` makes one reachable.
    if metadata_if_present(dest)?.is_some_and(|meta| meta.file_type().is_symlink()) {
        return Err(anyhow!("refusing symlink analytics output"));
    }
    let resolved_dest = resolve_identity_path(dest, true)?;
    let dest_meta = metadata_if_present(&resolved_dest)?;
    for root in source_roots {
        let resolved_root = resolve_identity_path(root, false)?;
        let Some(root_meta) = metadata_if_present(&resolved_root)? else {
            // Ordinary genuinely missing roots keep the existing ingest warning.
            continue;
        };
        if root_meta.is_dir() {
            if resolved_dest.starts_with(&resolved_root) {
                return Err(anyhow!("refusing analytics output inside a source root"));
            }
            if let Some(dest_meta) = &dest_meta {
                if same_inode_under(&resolved_root, dest_meta)? {
                    return Err(alias_error());
                }
            }
        } else if root_meta.is_file()
            && (resolved_dest == resolved_root
                || dest_meta
                    .as_ref()
                    .is_some_and(|meta| same_inode(meta, &root_meta)))
        {
            return Err(alias_error());
        }
    }
    for root in openrig_roots {
        // Match the same selected spelling used for snapshot sidecar lookup;
        // a different hard-link spelling does not inherit this sidecar family.
        for suffix in ["", "-wal", "-shm"] {
            let mut name = root.as_os_str().to_os_string();
            name.push(suffix);
            let protected = resolve_identity_path(Path::new(&name), false)?;
            if resolved_dest == protected {
                return Err(alias_error());
            }
            if let (Some(dest_meta), Some(source_meta)) =
                (&dest_meta, metadata_if_present(&protected)?)
            {
                if same_inode(dest_meta, &source_meta) {
                    return Err(alias_error());
                }
            }
        }
    }
    Ok(())
}

// Keep the ordinary existing-output WAL policy after root/schema preflight.
pub(crate) fn refuse_existing_output_wal(dest: &Path) -> Result<()> {
    if metadata_if_present(dest)?.is_some() && metadata_if_present(&wal_sibling(dest))?.is_some() {
        return Err(anyhow!(
            "refusing output that already has a DuckDB wal sibling"
        ));
    }
    Ok(())
}

fn identity_error() -> anyhow::Error {
    anyhow!("refusing analytics output because source identity could not be verified")
}

fn alias_error() -> anyhow::Error {
    anyhow!("refusing analytics output that aliases a source file")
}

fn metadata_if_present(path: &Path) -> Result<Option<fs::Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(meta) => Ok(Some(meta)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(identity_error()),
    }
}

/// Resolve existing physical components, retaining absent ordinary components.
/// Canceling an absent component resumes physical resolution, rather than
/// treating the rest of the path as a blindly normalized lexical suffix.
fn resolve_identity_path(path: &Path, refuse_final_symlink: bool) -> Result<PathBuf> {
    use std::path::Component;
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|_| identity_error())?
            .join(path)
    };
    let mut components = absolute.components().peekable();
    let mut resolved = PathBuf::new();
    let mut missing = 0usize;
    while let Some(component) = components.next() {
        match component {
            Component::RootDir => resolved.push(component.as_os_str()),
            Component::CurDir => {}
            Component::Prefix(_) => return Err(identity_error()),
            Component::ParentDir => {
                if missing == 0
                    && !fs::metadata(&resolved)
                        .map_err(|_| identity_error())?
                        .is_dir()
                {
                    return Err(identity_error());
                }
                resolved.pop();
                if resolved.as_os_str().is_empty() {
                    resolved.push("/");
                }
                missing = missing.saturating_sub(1);
            }
            Component::Normal(name) => {
                if missing == 0
                    && !fs::metadata(&resolved)
                        .map_err(|_| identity_error())?
                        .is_dir()
                {
                    return Err(identity_error());
                }
                resolved.push(name);
                if missing > 0 {
                    missing += 1;
                    continue;
                }
                match metadata_if_present(&resolved)? {
                    None => missing = 1,
                    Some(meta) if meta.file_type().is_symlink() => {
                        if refuse_final_symlink && components.peek().is_none() {
                            return Err(anyhow!("refusing symlink analytics output"));
                        }
                        // canonicalize bounds symlink expansion and rejects
                        // dangling/looping intermediate paths without opening content.
                        resolved = fs::canonicalize(&resolved).map_err(|_| identity_error())?;
                    }
                    Some(_) => {}
                }
            }
        }
    }
    Ok(resolved)
}

fn wal_sibling(dest: &Path) -> PathBuf {
    let mut name = dest.as_os_str().to_os_string();
    name.push(".wal");
    PathBuf::from(name)
}

fn same_inode(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    left.dev() == right.dev() && left.ino() == right.ino()
}

fn same_inode_under(root: &Path, dest_meta: &fs::Metadata) -> Result<bool> {
    use std::collections::HashSet;
    use std::os::unix::fs::MetadataExt;
    let mut seen = HashSet::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let meta = fs::symlink_metadata(&dir).map_err(|_| identity_error())?;
        if !meta.is_dir() {
            return Err(identity_error());
        }
        if !seen.insert((meta.dev(), meta.ino())) {
            continue;
        }
        for entry in fs::read_dir(&dir).map_err(|_| identity_error())? {
            let path = entry.map_err(|_| identity_error())?.path();
            let meta = fs::symlink_metadata(&path).map_err(|_| identity_error())?;
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.is_dir() {
                stack.push(path);
            } else if meta.is_file() && same_inode(&meta, dest_meta) {
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
    fn fixture(label: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("dendro-identity-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn explicit_files_and_typed_existing_and_absent_sidecars_are_protected() {
        let dir = fixture("files");
        let source = dir.join("source.sqlite");
        fs::write(&source, b"synthetic source").unwrap();
        let link = dir.join("hardlink");
        fs::hard_link(&source, &link).unwrap();
        for dest in [&source, &link] {
            assert_eq!(
                refuse_aliased_output(dest, std::slice::from_ref(&source))
                    .unwrap_err()
                    .to_string(),
                alias_error().to_string()
            );
        }
        for suffix in ["-wal", "-shm"] {
            let sidecar = PathBuf::from(format!("{}{suffix}", source.display()));
            assert_eq!(
                refuse_aliased_output_with_openrig(
                    &sidecar,
                    std::slice::from_ref(&source),
                    std::slice::from_ref(&source)
                )
                .unwrap_err()
                .to_string(),
                alias_error().to_string()
            );
            assert!(!sidecar.exists());
            // Untyped file roots do not invent an OpenRig sidecar namespace.
            refuse_aliased_output(&sidecar, std::slice::from_ref(&source)).unwrap();
            fs::write(&sidecar, b"synthetic sidecar").unwrap();
            let alias = dir.join(format!("alias{suffix}"));
            fs::hard_link(&sidecar, &alias).unwrap();
            assert!(refuse_aliased_output_with_openrig(
                &alias,
                std::slice::from_ref(&source),
                std::slice::from_ref(&source)
            )
            .is_err());
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn absent_component_cancellation_resumes_physical_parent_resolution() {
        let dir = fixture("resume");
        let source = dir.join("source");
        fs::create_dir_all(source.join("nested")).unwrap();
        let outside = dir.join("outside");
        fs::create_dir(&outside).unwrap();
        symlink(source.join("nested"), outside.join("parent")).unwrap();
        let dest = outside.join("missing/../parent/../new.duckdb");
        assert_eq!(
            resolve_identity_path(&dest, true).unwrap(),
            source.join("new.duckdb")
        );
        assert!(refuse_aliased_output(&dest, std::slice::from_ref(&source))
            .unwrap_err()
            .to_string()
            .contains("inside a source root"));
        assert!(!outside.join("missing").exists());
        assert!(!source.join("new.duckdb").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn effective_final_symlinks_refuse_before_following_any_target() {
        let dir = fixture("final");
        let source = dir.join("source");
        fs::write(&source, b"synthetic source").unwrap();
        let unrelated = dir.join("unrelated");
        fs::write(&unrelated, b"other").unwrap();
        for (name, target) in [
            ("source-link", source.clone()),
            ("other-link", unrelated),
            ("dangling", dir.join("absent")),
        ] {
            symlink(target, dir.join(name)).unwrap();
            let dest = dir.join("missing/..").join(name);
            assert_eq!(
                refuse_aliased_output(&dest, std::slice::from_ref(&source))
                    .unwrap_err()
                    .to_string(),
                "refusing symlink analytics output"
            );
        }
        assert!(!dir.join("missing").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn directory_walk_checks_arbitrary_files_and_skips_symlink_trees() {
        let dir = fixture("walk");
        let source = dir.join("source");
        fs::create_dir_all(source.join("nested")).unwrap();
        let original = source.join("nested/original.bin");
        fs::write(&original, b"synthetic original").unwrap();
        let alias = dir.join("outside-hardlink");
        fs::hard_link(&original, &alias).unwrap();
        symlink(&source, source.join("cycle")).unwrap();
        symlink(&dir, source.join("outside-tree")).unwrap();
        assert_eq!(
            refuse_aliased_output(&alias, std::slice::from_ref(&source))
                .unwrap_err()
                .to_string(),
            alias_error().to_string()
        );
        let copy = dir.join("copy");
        fs::copy(&original, &copy).unwrap();
        refuse_aliased_output(&copy, std::slice::from_ref(&source)).unwrap();
        refuse_aliased_output(
            &dir.join("source-neighbor/new"),
            std::slice::from_ref(&source),
        )
        .unwrap();
        for dest in [&source, &source.join("absent/deep/new")] {
            assert!(refuse_aliased_output(dest, std::slice::from_ref(&source))
                .unwrap_err()
                .to_string()
                .contains("inside a source root"));
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unresolved_intermediate_paths_refuse_with_fixed_sanitized_error() {
        let dir = fixture("uncertain");
        fs::write(dir.join("plain-file"), b"synthetic").unwrap();
        symlink(dir.join("not-present"), dir.join("dangling")).unwrap();
        symlink(dir.join("loop"), dir.join("loop")).unwrap();
        for dest in [
            dir.join("plain-file/out"),
            dir.join("plain-file/../out"),
            dir.join("dangling/out"),
            dir.join("loop/out"),
        ] {
            assert_eq!(
                refuse_aliased_output(&dest, &[dir.join("missing-source")])
                    .unwrap_err()
                    .to_string(),
                identity_error().to_string()
            );
        }
        refuse_aliased_output(&dir.join("safe-out"), &[dir.join("missing-source")]).unwrap();
        fs::remove_dir_all(dir).unwrap();
    }
}
