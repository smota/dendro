//! Manifest-aware wrapper over the existing transactional publication path.
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{
    refuse_aliased_output_with_openrig, refuse_secret_path, AnalyticsStore, CutoffManifest,
};

fn file_digest(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        crate::RUN_CANCEL.check()?;
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(hex::encode(digest.finalize()))
}

fn history_path(manifest_path: &Path, digest: &str) -> Result<PathBuf> {
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(anyhow!("invalid sealed history digest"));
    }
    Ok(manifest_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!(".dendro-sealed-{digest}.duckdb")))
}

/// Capture or replay source cutoffs without changing the legacy ingest API.
/// Existing accepted history is kept in a content-addressed analytics companion;
/// no source content or path is embedded in the JSON manifest.
pub fn run_locked_with_manifest<F>(
    db: &Path,
    source_roots: &[PathBuf],
    openrig_roots: &[PathBuf],
    write_manifest: Option<&Path>,
    sealed: Option<&Path>,
    ingest: F,
) -> Result<Value>
where
    F: FnOnce(&Path, &[u8], Option<&CutoffManifest>) -> Result<(Vec<String>, CutoffManifest)>,
{
    if write_manifest.is_some() && sealed.is_some() {
        return Err(anyhow!(
            "--write-manifest and --sealed cannot be used together"
        ));
    }
    for path in [write_manifest, sealed].into_iter().flatten() {
        refuse_secret_path(path)?;
    }
    let mut protected = source_roots.to_vec();
    protected.push(db.to_path_buf());
    protected.push(PathBuf::from(format!("{}.key", db.display())));
    protected.push(PathBuf::from(format!("{}.wal", db.display())));
    protected.push(
        db.parent()
            .unwrap_or_else(|| Path::new("."))
            .join(".dendro-output.lock"),
    );
    let check_destination = |path: &Path| -> Result<()> {
        refuse_aliased_output_with_openrig(path, &protected, openrig_roots)?;
        let destination = crate::canonical_root(path);
        if protected
            .iter()
            .any(|root| destination == crate::canonical_root(root))
        {
            return Err(anyhow!(
                "manifest output aliases analytics output or a source"
            ));
        }
        if source_roots
            .iter()
            .any(|root| destination.starts_with(crate::canonical_root(root)))
        {
            return Err(anyhow!("manifest output aliases a source root"));
        }
        Ok(())
    };
    if let Some(path) = write_manifest {
        check_destination(path)?;
    }
    let manifest = sealed
        .map(|path| -> Result<CutoffManifest> {
            let mut manifest: CutoffManifest = serde_json::from_reader(
                fs::File::open(path).map_err(|_| anyhow!("could not open sealed manifest"))?,
            )
            .map_err(|_| anyhow!("invalid sealed manifest"))?;
            manifest.redact_identifiers();
            Ok(manifest)
        })
        .transpose()?;
    let mut pending = None;
    let result = crate::run_locked_with_openrig(db, source_roots, openrig_roots, |staged, key| {
        let has_history = if write_manifest.is_some() {
            !AnalyticsStore::open_staged(staged)?
                .load_sources()?
                .is_empty()
        } else {
            false
        };
        let (notes, mut captured) = ingest(staged, key, manifest.as_ref())?;
        // Source validation/reading always precedes restoring accepted historical
        // facts. A missing or changed named source still refuses publication.
        if let (Some(manifest_path), Some(digest)) = (
            sealed,
            manifest
                .as_ref()
                .and_then(|m| m.analytics_snapshot_sha256.as_deref()),
        ) {
            let history = history_path(manifest_path, digest)?;
            if !fs::symlink_metadata(&history).is_ok_and(|metadata| metadata.is_file()) {
                return Err(anyhow!(
                    "mismatch: sealed analytics history missing or not a regular file"
                ));
            }
            if file_digest(&history)
                .map_err(|_| anyhow!("mismatch: sealed analytics history missing"))?
                != digest
            {
                return Err(anyhow!("mismatch: sealed analytics history digest differs"));
            }
            let fresh = AnalyticsStore::open_staged(staged)?;
            let disclosures = fresh.load_mutable_source_disclosures()?;
            fresh.checkpoint()?;
            drop(fresh);
            let historical = AnalyticsStore::open_read_only(&history)?;
            if historical.key_id != crate::key_id(key) {
                return Err(anyhow!(
                    "mismatch: sealed analytics history requires the original analytics key"
                ));
            }
            drop(historical);
            fs::remove_file(staged)?;
            crate::lifecycle::private_copy(&history, staged, &crate::RUN_CANCEL)?;
            let restored = AnalyticsStore::open_staged(staged)?;
            for disclosure in disclosures {
                restored.record_mutable_source_disclosure(
                    disclosure["qitem_id"].as_str().unwrap_or("unknown"),
                    disclosure["seat"].as_str().unwrap_or("unknown"),
                    disclosure["reason"]
                        .as_str()
                        .unwrap_or("not reproducible: mutable source"),
                )?;
            }
            restored.checkpoint()?;
        }
        if let Some(path) = write_manifest {
            let parent = path.parent().unwrap_or_else(|| Path::new("."));
            fs::create_dir_all(parent)?;
            if has_history {
                let store = AnalyticsStore::open_staged(staged)?;
                store.checkpoint()?;
                drop(store);
                let digest = file_digest(staged)?;
                let history = history_path(path, &digest)?;
                check_destination(&history)?;
                if history.exists() {
                    if file_digest(&history)? != digest {
                        return Err(anyhow!("sealed history artifact mismatch"));
                    }
                } else {
                    crate::lifecycle::private_copy(staged, &history, &crate::RUN_CANCEL)?;
                    fs::File::open(&history)?.sync_all()?;
                }
                captured.analytics_snapshot_sha256 = Some(digest);
            }
            captured.redact_identifiers();
            let temporary = parent.join(format!(
                ".dendro-manifest-{:016x}.tmp",
                rand::random::<u64>()
            ));
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temporary)?;
            pending = Some(temporary);
            serde_json::to_writer_pretty(&mut file, &captured)?;
            file.write_all(b"\n")?;
            file.sync_all()?;
        }
        Ok(notes)
    });
    // Prepared manifest is published only after the analytics stage succeeded.
    // It lives on the destination filesystem so cross-device paths also work.
    match (result, pending) {
        (Ok(result), Some(temporary)) => {
            let path = write_manifest.expect("prepared manifest has destination");
            let renamed = fs::rename(&temporary, path);
            if renamed.is_err() {
                let _ = fs::remove_file(&temporary);
            }
            renamed?;
            crate::sync_dir(path.parent().unwrap_or_else(|| Path::new(".")))?;
            Ok(result)
        }
        (Err(error), Some(temporary)) => {
            let _ = fs::remove_file(temporary);
            Err(error)
        }
        (result, None) => result,
    }
}
