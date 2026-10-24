//! Manifest-aware wrapper over the existing transactional publication path.
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{
    refuse_aliased_output_with_openrig, refuse_secret_path, AnalyticsStore, CutoffManifest,
};

#[cfg(any(windows, target_os = "linux"))]
#[path = "sealed_run/cutoff_publication.rs"]
mod cutoff_publication;

#[cfg(any(windows, target_os = "linux"))]
#[derive(Debug, serde::Serialize)]
#[serde(tag = "state", content = "sha256", rename_all = "snake_case")]
pub enum AnalyticsSelectionOutcome {
    New(String),
    Uncertain,
}

#[cfg(any(windows, target_os = "linux"))]
#[derive(Debug, serde::Serialize)]
#[serde(tag = "state", content = "path", rename_all = "snake_case")]
pub enum CutoffOutcome {
    Retained(PathBuf),
    NotPrepared,
}

#[cfg(any(windows, target_os = "linux"))]
#[derive(Debug, serde::Serialize)]
pub struct PartialTransactionError {
    pub analytics_selection: AnalyticsSelectionOutcome,
    pub cutoff: CutoffOutcome,
    pub cause: String,
}

#[cfg(any(windows, target_os = "linux"))]
impl std::fmt::Display for PartialTransactionError {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            out,
            "{}",
            serde_json::to_string(self).map_err(|_| std::fmt::Error)?
        )
    }
}

#[cfg(any(windows, target_os = "linux"))]
impl std::error::Error for PartialTransactionError {}

#[cfg(any(windows, target_os = "linux"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CutoffRecoveryOutcome {
    Reconciled,
    Unchanged,
}

#[cfg(any(windows, target_os = "linux"))]
pub fn recover_cutoff_manifest(path: &Path) -> Result<CutoffRecoveryOutcome> {
    cutoff_publication::recover(path)
}

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
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .join(format!(".dendro-sealed-{digest}.duckdb")))
}

/// The only external-history read is this copy. All integrity/key checks then
/// read the private copy, so a replacement of `history` cannot change the bytes
/// that are restored after validation. `staged` belongs to the held run stage.
fn copy_sealed_history(history: &Path, staged: &Path) -> Result<()> {
    fs::remove_file(staged)?;
    crate::lifecycle::private_copy(history, staged, &crate::RUN_CANCEL)
}

fn validate_copied_history(staged: &Path, digest: &str, key: &[u8]) -> Result<()> {
    if file_digest(staged)? != digest {
        return Err(anyhow!("mismatch: sealed analytics history digest differs"));
    }
    let historical = AnalyticsStore::open_read_only(staged)?;
    if historical.key_id != crate::key_id(key) {
        return Err(anyhow!(
            "mismatch: sealed analytics history requires the original analytics key"
        ));
    }
    Ok(())
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
    #[cfg(any(windows, target_os = "linux"))]
    if let Some(path) = write_manifest {
        let absolute = std::path::absolute(path)?;
        let parent = absolute
            .parent()
            .ok_or_else(|| anyhow!("cutoff destination has no parent"))?;
        crate::platform_v1::open_existing_private_root(parent)?;
        let leaf = absolute
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| anyhow!("invalid cutoff leaf"))?;
        crate::platform_v1::parse_child_name(leaf, crate::platform_v1::current_native_platform())
            .map_err(|_| {
            anyhow!("cutoff manifest leaf must be lowercase ASCII of at most 100 bytes")
        })?;
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
            .filter(|p| !p.as_os_str().is_empty())
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
    let sealed_bytes = sealed.map(fs::read).transpose()?;
    #[cfg(any(windows, target_os = "linux"))]
    let destination_expectation = if let Some(bytes) = sealed_bytes.as_deref() {
        let raw: serde_json::Value = serde_json::from_slice(bytes)?;
        // The source selection digest is required metadata and checked for
        // presence/hex format only; it is not authenticated source provenance.
        // History bytes and original key are validated separately below.
        let captured = raw
            .get("analytics_selection_sha256")
            .and_then(|value| value.as_str())
            .ok_or_else(|| anyhow!("sealed manifest lacks selected database identity"))?;
        if captured.len() != 64 || !captured.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(anyhow!("invalid sealed manifest selected database digest"));
        }
        let current = match crate::selection_binding::resolve_logical_db(
            db,
            &crate::platform_v1::StorageRootChoice::LocalAppData,
        )? {
            crate::selection_binding::SelectedDb::Selected(read) => Some(read.selection()),
            crate::selection_binding::SelectedDb::Legacy(_)
            | crate::selection_binding::SelectedDb::Uninitialized => None,
        };
        // The captured selection belongs to the source of the sealed manifest.
        // A destination may be fresh or have advanced since capture. Pin its
        // own observed selection for the writer-lock/CAS race check below.
        Some(current)
    } else {
        None
    };
    let manifest = sealed_bytes
        .as_deref()
        .map(|bytes| -> Result<CutoffManifest> {
            let mut manifest: CutoffManifest =
                serde_json::from_slice(bytes).map_err(|_| anyhow!("invalid sealed manifest"))?;
            manifest.redact_identifiers();
            Ok(manifest)
        })
        .transpose()?;
    let mut pending = None;
    let ingest_run = |staged: &Path, key: &[u8]| {
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
            let fresh = AnalyticsStore::open_staged(staged)?;
            let disclosures = fresh.load_mutable_source_disclosures()?;
            fresh.checkpoint()?;
            drop(fresh);
            copy_sealed_history(&history, staged)?;
            validate_copied_history(staged, digest, key)?;
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
            let parent = path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new("."));
            #[cfg(not(any(windows, target_os = "linux")))]
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
                    // private_copy flushed the write-capable private destination.
                }
                captured.analytics_snapshot_sha256 = Some(digest);
            }
            captured.redact_identifiers();
            let temporary = parent.join(format!(
                ".dendro-manifest-{:016x}.tmp",
                rand::random::<u64>()
            ));
            let mut file = crate::lifecycle::create_private_file(&temporary)?;
            pending = Some(temporary);
            serde_json::to_writer_pretty(&mut file, &captured)?;
            file.write_all(b"\n")?;
            file.sync_all()?;
        }
        Ok(notes)
    };
    #[cfg(any(windows, target_os = "linux"))]
    let result = crate::selection_binding::run_locked_with_destination_expectation(
        db,
        &crate::platform_v1::StorageRootChoice::LocalAppData,
        source_roots,
        openrig_roots,
        destination_expectation,
        ingest_run,
    )
    .map(|(report, selected)| (report, Some(selected)));
    #[cfg(not(any(windows, target_os = "linux")))]
    let result = crate::run_locked_with_openrig(db, source_roots, openrig_roots, ingest_run)
        .map(|report| (report, None::<()>));
    // Prepared manifest is published only after the analytics stage succeeded.
    // It lives on the destination filesystem so cross-device paths also work.
    match (result, pending) {
        (Ok((result, published_selection)), Some(temporary)) => {
            let path = write_manifest.expect("prepared manifest has destination");
            #[cfg(any(windows, target_os = "linux"))]
            {
                let selected = published_selection.expect("native selected publication receipt");
                let publish = (|| -> Result<()> {
                    let mut manifest: serde_json::Value =
                        serde_json::from_reader(fs::File::open(&temporary)?)?;
                    manifest["analytics_selection_sha256"] =
                        serde_json::Value::String(hex::encode(selected.sha256));
                    let body = serde_json::to_vec_pretty(&manifest)?;
                    cutoff_publication::publish(path, &body)
                })();
                if let Err(error) = publish {
                    return Err(PartialTransactionError {
                        analytics_selection: AnalyticsSelectionOutcome::New(hex::encode(
                            selected.sha256,
                        )),
                        cutoff: CutoffOutcome::Retained(temporary),
                        cause: error.to_string(),
                    }
                    .into());
                }
                // The cutoff is already durably published. A failed scratch
                // cleanup must not turn that success into a partial result.
                let _ = fs::remove_file(&temporary);
                let mut result = result;
                result["analytics_selection_sha256"] = Value::String(hex::encode(selected.sha256));
                result["cutoff_manifest"] = Value::String("published".into());
                Ok(result)
            }
            #[cfg(not(any(windows, target_os = "linux")))]
            {
                let _ = published_selection;
                let renamed = fs::rename(&temporary, path);
                if renamed.is_err() {
                    let _ = fs::remove_file(&temporary);
                }
                renamed?;
                crate::sync_dir(path.parent().unwrap_or_else(|| Path::new(".")))?;
                Ok(result)
            }
        }
        (Err(error), Some(temporary)) => {
            #[cfg(any(windows, target_os = "linux"))]
            {
                Err(PartialTransactionError {
                    analytics_selection: AnalyticsSelectionOutcome::Uncertain,
                    cutoff: CutoffOutcome::Retained(temporary),
                    cause: error.to_string(),
                }
                .into())
            }
            #[cfg(not(any(windows, target_os = "linux")))]
            {
                let _ = fs::remove_file(temporary);
                Err(error)
            }
        }
        (result, None) => result.map(|(report, _)| report),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch {
        path: PathBuf,
        #[cfg(any(windows, target_os = "linux"))]
        root: Option<crate::platform_v1::PrivateRoot>,
    }

    impl Scratch {
        fn new() -> Self {
            #[cfg(any(windows, target_os = "linux"))]
            {
                let root = crate::platform_v1::create_user_private_root_with_choice(
                    "dendro-sealed-test",
                    &crate::platform_v1::StorageRootChoice::UserProfile,
                )
                .unwrap();
                Self {
                    path: root.path().to_path_buf(),
                    root: Some(root),
                }
            }
            #[cfg(not(any(windows, target_os = "linux")))]
            {
                let path = std::env::temp_dir()
                    .join(format!("dendro-sealed-test-{:016x}", rand::random::<u64>()));
                crate::lifecycle::create_private_dir(&path).unwrap();
                Self { path }
            }
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            #[cfg(any(windows, target_os = "linux"))]
            self.root.take();
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn history(dir: &Scratch, key: &[u8]) -> (PathBuf, String) {
        let path = dir.path.join("history.duckdb");
        let store = AnalyticsStore::open_path(&path, &crate::key_id(key)).unwrap();
        store.checkpoint().unwrap();
        drop(store);
        let digest = file_digest(&path).unwrap();
        (path, digest)
    }

    #[test]
    fn sealed_replay_validates_private_copy_after_external_history_changes() {
        let dir = Scratch::new();
        let key = [23_u8; 32];
        let (history, digest) = history(&dir, &key);
        let staged = dir.path.join("analytics.duckdb");
        fs::write(&staged, b"old temporary stage").unwrap();
        copy_sealed_history(&history, &staged).unwrap();
        // Deterministic mutation between copy and validation: only private
        // staged bytes may be hashed/opened, never this external path again.
        fs::write(&history, b"changed external history").unwrap();
        validate_copied_history(&staged, &digest, &key).unwrap();
        assert_eq!(file_digest(&staged).unwrap(), digest);
        assert_eq!(fs::read(&history).unwrap(), b"changed external history");
    }

    #[test]
    fn sealed_replay_rejects_history_replacement_before_copy() {
        let dir = Scratch::new();
        let key = [29_u8; 32];
        let (history, digest) = history(&dir, &key);
        fs::write(
            &history,
            b"replacement after old verification would have passed",
        )
        .unwrap();
        let staged = dir.path.join("analytics.duckdb");
        fs::write(&staged, b"old temporary stage").unwrap();
        copy_sealed_history(&history, &staged).unwrap();
        let error = validate_copied_history(&staged, &digest, &key).unwrap_err();
        assert!(
            error.to_string().contains("history digest differs"),
            "{error}"
        );
    }

    #[test]
    fn sealed_replay_checks_key_on_the_exact_digest_validated_copy() {
        let dir = Scratch::new();
        let (history, digest) = history(&dir, &[31_u8; 32]);
        let staged = dir.path.join("analytics.duckdb");
        fs::write(&staged, b"old temporary stage").unwrap();
        copy_sealed_history(&history, &staged).unwrap();
        let error = validate_copied_history(&staged, &digest, &[37_u8; 32]).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("requires the original analytics key"),
            "{error}"
        );
    }

    #[cfg(any(windows, target_os = "linux"))]
    #[test]
    fn unadmitted_manifest_parent_refuses_before_ingest_or_directory_creation() {
        let absent = std::env::temp_dir().join(format!(
            "dendro-sealed-refusal-{:016x}",
            rand::random::<u64>()
        ));
        let error = run_locked_with_manifest(
            &absent.join("analytics.duckdb"),
            &[],
            &[],
            Some(&absent.join("manifest.json")),
            None,
            |_, _, _| panic!("ingest must not run before publication admission"),
        )
        .unwrap_err();
        #[cfg(windows)]
        assert!(
            error.to_string().contains("untrusted_ancestor_replacement"),
            "{error}"
        );
        #[cfg(target_os = "linux")]
        assert!(error.to_string().contains("no_trustworthy_root"), "{error}");
        assert!(
            !absent.exists(),
            "refusal must not create the output parent"
        );
    }

    #[cfg(any(windows, target_os = "linux"))]
    #[test]
    fn second_cutoff_capture_flushes_existing_history_and_publishes() {
        let dir = Scratch::new();
        let db = dir.path.join("analytics.duckdb");
        let cutoff = dir.path.join("capture.json");
        for _ in 0..2 {
            run_locked_with_manifest(&db, &[], &[], Some(&cutoff), None, |staged, _, _| {
                let store = AnalyticsStore::open_staged(staged)?;
                store.connection().execute(
                    "INSERT OR IGNORE INTO source_files (source_id, harness, display_path, generation, byte_offset, prefix_sha256) VALUES (?, ?, ?, ?, ?, ?)",
                    duckdb::params!["synthetic", "codex", "synthetic", 1_i64, 0_i64, "none"],
                )?;
                store.checkpoint()?;
                Ok((vec!["synthetic".into()], CutoffManifest::default()))
            }).unwrap();
        }
        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&cutoff).unwrap()).unwrap();
        let digest = manifest["analytics_snapshot_sha256"]
            .as_str()
            .expect("second capture has history");
        assert!(history_path(&cutoff, digest).unwrap().exists());
        assert_eq!(
            manifest["analytics_selection_sha256"]
                .as_str()
                .unwrap()
                .len(),
            64
        );
    }

    #[cfg(any(windows, target_os = "linux"))]
    #[test]
    fn sealed_manifest_is_read_once_even_if_named_file_changes_during_ingest() {
        let dir = Scratch::new();
        let db = dir.path.join("analytics.duckdb");
        crate::selection_binding::run_locked_with_selection(
            &db,
            &crate::platform_v1::StorageRootChoice::Explicit(dir.path.clone()),
            &[],
            &[],
            |_, _| Ok(vec![]),
        )
        .unwrap();
        let selected = match crate::selection_binding::resolve_logical_db(
            &db,
            &crate::platform_v1::StorageRootChoice::Explicit(dir.path.clone()),
        )
        .unwrap()
        {
            crate::selection_binding::SelectedDb::Selected(read) => read.selection(),
            _ => panic!("selected"),
        };
        let sealed = dir.path.join("replay.json");
        fs::write(
            &sealed,
            format!(
                "{{\"analytics_selection_sha256\":\"{}\",\"files\":[],\"openrig\":[]}}",
                hex::encode(selected.sha256)
            ),
        )
        .unwrap();
        run_locked_with_manifest(&db, &[], &[], None, Some(&sealed), |_, _, manifest| {
            assert!(manifest.is_some());
            fs::write(&sealed, b"invalid replacement").unwrap();
            Ok((vec![], CutoffManifest::default()))
        })
        .unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn cutoff_failure_retains_prepared_manifest_and_exact_new_selection_receipt() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = Scratch::new();
        let db = dir.path.join("analytics.duckdb");
        let cutoff = dir.path.join("capture.json");
        let capture = || {
            run_locked_with_manifest(&db, &[], &[], Some(&cutoff), None, |_, _, _| {
                Ok((vec![], CutoffManifest::default()))
            })
        };
        capture().unwrap();
        let held = fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&cutoff)
            .unwrap();
        let error = capture().unwrap_err();
        let partial = error
            .downcast_ref::<PartialTransactionError>()
            .expect("typed partial transaction");
        let AnalyticsSelectionOutcome::New(receipt_sha) = &partial.analytics_selection else {
            panic!("known new selection")
        };
        let CutoffOutcome::Retained(path) = &partial.cutoff else {
            panic!("prepared manifest retained")
        };
        assert!(path.exists());
        let selected = match crate::selection_binding::resolve_logical_db(
            &db,
            &crate::platform_v1::StorageRootChoice::Explicit(dir.path.clone()),
        )
        .unwrap()
        {
            crate::selection_binding::SelectedDb::Selected(read) => read.selection(),
            _ => panic!("selected"),
        };
        assert_eq!(*receipt_sha, hex::encode(selected.sha256));
        drop(held);
    }
}
