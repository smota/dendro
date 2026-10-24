//! Separate native private regular-file publication for cutoff manifests.
//! The old destination and every failed source/journal are retained.
#![deny(unsafe_code)]

use std::fs;
use std::io::Read;
use std::path::Path;

use anyhow::{anyhow, Context, Result};
#[cfg(windows)]
use dendro_windows_private_io::{ChildName as NativeName, VerifiedDirectory};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::platform_v1::{self as p, ChildName, FileIdentity, PrivateRoot, SelectionIdentity};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pointer {
    identity: SelectionIdentity,
    sha256: [u8; 32],
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Binding {
    schema: String,
    version: u32,
    parent: SelectionIdentity,
    leaf: String,
    lock: String,
    lock_identity: SelectionIdentity,
    baseline: Option<Pointer>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    schema: String,
    version: u32,
    generation: u32,
    previous: Option<Pointer>,
    selected: Pointer,
}

fn publication_busy(error: &anyhow::Error) -> bool {
    let io = error.downcast_ref::<std::io::Error>().or_else(|| {
        match error.downcast_ref::<p::PlatformError>() {
            Some(p::PlatformError::Io(error)) => Some(error),
            _ => None,
        }
    });
    #[cfg(windows)]
    {
        io.is_some_and(|error| matches!(error.raw_os_error(), Some(5 | 32 | 33)))
    }
    #[cfg(target_os = "linux")]
    {
        io.is_some_and(|error| matches!(error.raw_os_error(), Some(11 | 16 | 26)))
    }
}

fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn child(leaf: &str) -> Result<ChildName> {
    p::parse_child_name(leaf, p::current_native_platform())
        .map_err(p::PlatformError::Rejected)
        .map_err(Into::into)
}

fn leaf_hash(parent: SelectionIdentity, leaf: &str) -> String {
    let mut hasher = Sha256::new();
    #[cfg(windows)]
    {
        hasher.update(parent.volume_serial_64.to_le_bytes());
        hasher.update(parent.file_id);
    }
    #[cfg(target_os = "linux")]
    {
        hasher.update(b"linux-device-inode-v1");
        hasher.update(parent.device.to_le_bytes());
        hasher.update(parent.inode.to_le_bytes());
    }
    hasher.update(leaf.as_bytes());
    hex::encode(hasher.finalize())
}

fn read_existing(root: &PrivateRoot, leaf: &str) -> Result<Option<(Pointer, Vec<u8>)>> {
    match fs::symlink_metadata(root.path().join(leaf)) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
        Ok(_) => {
            let mut file = p::open_existing_private_file(root, &child(leaf)?)?;
            if file.metadata().size > 1024 * 1024 {
                return Err(anyhow!("cutoff manifest or record too large"));
            }
            let mut bytes = Vec::new();
            file.file_mut()
                .take(1024 * 1024 + 1)
                .read_to_end(&mut bytes)?;
            Ok(Some((
                Pointer {
                    identity: file.metadata().identity.try_into()?,
                    sha256: digest(&bytes),
                },
                bytes,
            )))
        }
    }
}

fn write_new<T: Serialize>(root: &PrivateRoot, leaf: &str, record: &T) -> Result<()> {
    crate::selection_binding::write_record(root, leaf, record)?;
    Ok(())
}

fn state_name(hash: &str, generation: u32) -> String {
    format!("cutoff-state-{hash}-{generation:06}.json")
}

fn load_state_mode(
    root: &PrivateRoot,
    hash: &str,
    baseline: Option<Pointer>,
    allow_torn_top: bool,
) -> Result<(u32, Option<Pointer>, Option<String>)> {
    let mut generation = 0u32;
    let mut expected = baseline;
    let mut torn = None;
    loop {
        let next = generation
            .checked_add(1)
            .ok_or_else(|| anyhow!("cutoff state overflow"))?;
        let leaf = state_name(hash, next);
        let Some((_, bytes)) = read_existing(root, &leaf)? else {
            break;
        };
        let state: State = match serde_json::from_slice(&bytes) {
            Ok(state) => state,
            Err(_) if allow_torn_top => {
                torn = Some(leaf);
                break;
            }
            Err(error) => return Err(error).context("invalid cutoff state"),
        };
        if state.schema != "dendro.cutoff-state"
            || state.version != 1
            || state.generation != next
            || state.previous != expected
        {
            return Err(anyhow!("contradictory cutoff state"));
        }
        expected = Some(state.selected);
        generation = next;
    }
    let prefix = format!("cutoff-state-{hash}-");
    for entry in fs::read_dir(root.path())? {
        let entry = entry?;
        if let Some(leaf) = entry.file_name().to_str() {
            if leaf.starts_with(&prefix) && leaf.ends_with(".json") && torn.as_deref() != Some(leaf)
            {
                let ordinal = leaf
                    .strip_prefix(&prefix)
                    .and_then(|name| name.strip_suffix(".json"));
                if ordinal
                    .is_some_and(|name| name.parse::<u32>().is_ok_and(|number| number > generation))
                {
                    return Err(anyhow!("cutoff custody state has a gap"));
                }
            }
        }
    }
    Ok((generation, expected, torn))
}

fn load_state(
    root: &PrivateRoot,
    hash: &str,
    baseline: Option<Pointer>,
) -> Result<(u32, Option<Pointer>)> {
    let (generation, expected, _) = load_state_mode(root, hash, baseline, false)?;
    Ok((generation, expected))
}

fn load_or_create_binding(root: &PrivateRoot, leaf: &str) -> Result<(String, Binding)> {
    let parent = SelectionIdentity::try_from(root.metadata().identity)?;
    let hash = leaf_hash(parent, leaf);
    let record_leaf = format!("cutoff-bind-{hash}.json");
    if let Some((_, bytes)) = read_existing(root, &record_leaf)? {
        let record: Binding = serde_json::from_slice(&bytes).context("invalid cutoff binding")?;
        if record.schema != "dendro.cutoff-binding"
            || record.version != 1
            || record.parent != parent
            || record.leaf != leaf
            || record.lock != format!("cutoff-lock-{hash}.lock")
        {
            return Err(anyhow!("cutoff binding identity changed"));
        }
        let lock = p::open_existing_private_file(root, &child(&record.lock)?)?;
        if SelectionIdentity::try_from(lock.metadata().identity)? != record.lock_identity {
            return Err(anyhow!("cutoff canonical lock identity changed"));
        }
        return Ok((hash, record));
    }
    let baseline = read_existing(root, leaf)?.map(|(pointer, _)| pointer);
    let lock_name = format!("cutoff-lock-{hash}.lock");
    let lock = if read_existing(root, &lock_name)?.is_some() {
        p::open_existing_private_file(root, &child(&lock_name)?)?
    } else {
        root.create_file(&child(&lock_name)?)?
    };
    let lock_identity = SelectionIdentity::try_from(lock.metadata().identity)?;
    drop(lock);
    let record = Binding {
        schema: "dendro.cutoff-binding".into(),
        version: 1,
        parent,
        leaf: leaf.into(),
        lock: lock_name,
        lock_identity,
        baseline,
    };
    write_new(root, &record_leaf, &record)?;
    Ok((hash, record))
}

fn load_recovery_binding(root: &PrivateRoot, leaf: &str) -> Result<(String, Binding, bool)> {
    let parent = SelectionIdentity::try_from(root.metadata().identity)?;
    let hash = leaf_hash(parent, leaf);
    let record_leaf = format!("cutoff-bind-{hash}.json");
    let existing = read_existing(root, &record_leaf)?;
    if existing
        .as_ref()
        .is_some_and(|(_, bytes)| serde_json::from_slice::<Binding>(bytes).is_ok())
    {
        let (loaded_hash, binding) = load_or_create_binding(root, leaf)?;
        return Ok((loaded_hash, binding, false));
    }
    let lock_name = format!("cutoff-lock-{hash}.lock");
    let lock = p::open_existing_private_file(root, &child(&lock_name)?)?;
    let lock_identity = SelectionIdentity::try_from(lock.metadata().identity)?;
    let first_state = read_existing(root, &state_name(&hash, 1))?
        .ok_or_else(|| anyhow!("torn cutoff binding has no baseline state proof"))?;
    let first: State = serde_json::from_slice(&first_state.1)
        .context("torn cutoff binding baseline state invalid")?;
    if first.schema != "dendro.cutoff-state" || first.version != 1 || first.generation != 1 {
        return Err(anyhow!("torn cutoff binding baseline state invalid"));
    }
    Ok((
        hash,
        Binding {
            schema: "dendro.cutoff-binding".into(),
            version: 1,
            parent,
            leaf: leaf.into(),
            lock: lock_name,
            lock_identity,
            baseline: first.previous,
        },
        true,
    ))
}

fn observed_matches(root: &PrivateRoot, leaf: &str, expected: Option<Pointer>) -> Result<()> {
    let observed = read_existing(root, leaf)?.map(|(pointer, _)| pointer);
    if observed != expected {
        return Err(anyhow!(
            "cutoff manifest pointer changed; recovery required"
        ));
    }
    Ok(())
}

/// Publish a cutoff JSON document through the reviewed held-directory helper.
/// A post-rename failure is reported as uncertain; recovery checks actual state.
pub(super) fn publish(path: &Path, bytes: &[u8]) -> Result<()> {
    if bytes.len() > 1024 * 1024 {
        return Err(anyhow!("cutoff manifest too large"));
    }
    let path = std::path::absolute(path)?;
    let parent_path = path
        .parent()
        .ok_or_else(|| anyhow!("cutoff destination has no parent"))?;
    let leaf = path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow!("cutoff leaf is not Unicode"))?;
    #[cfg(windows)]
    NativeName::parse(leaf)
        .context("cutoff manifest leaf must be lowercase ASCII of at most 100 bytes")?;
    let root = p::open_existing_private_root(parent_path)?;
    let (hash, binding) = load_or_create_binding(&root, leaf)?;
    let (generation, expected) = load_state(&root, &hash, binding.baseline)?;
    let staging = p::create_unique_private_directory(&root, "cutoff-stage-")?;
    let lock_name = child(&binding.lock)?;
    let lock_id =
        p::canonical_identity_from_persisted_record(FileIdentity::from(binding.lock_identity));
    p::with_exclusive_lock(&root, &lock_name, &lock_id, |_| {
        Ok((|| -> Result<()> {
            observed_matches(&root, leaf, expected)?;
            root.check_anchor()?;
            staging.check_anchor()?;
            #[cfg(windows)]
            let native_stage = VerifiedDirectory::from_held(staging.held_directory_file()?)?;
            #[cfg(windows)]
            let native_dest = VerifiedDirectory::from_held(root.held_directory_file()?)?;
            #[cfg(windows)]
            let native_leaf = NativeName::parse(leaf)?;
            let manifest_digest = digest(bytes);
            for attempt in 1..=3u32 {
                observed_matches(&root, leaf, expected)?;
                let source_name = format!(
                    "cutoff-source-{:08x}-{:02x}.json",
                    rand::random::<u32>(),
                    attempt
                );
                let journal_name = format!(
                    "cutoff-journal-{:08x}-{:02x}.json",
                    rand::random::<u32>(),
                    attempt
                );
                #[cfg(windows)]
                let mut source = native_stage.create_source(&NativeName::parse(&source_name)?)?;
                #[cfg(target_os = "linux")]
                let mut source =
                    p::LinuxPublicationSource::create(&staging, &child(&source_name)?)?;
                let source_identity = SelectionIdentity::try_from(source.identity()?)?;
                source.write_all(bytes)?;
                write_new(
                    &staging,
                    &journal_name,
                    &serde_json::json!({
                        "schema":"dendro.cutoff-journal", "state":"prepared", "source":source_name,
                        "source_identity":source_identity, "destination":leaf, "parent":binding.parent,
                        "expected":expected, "new_sha256":manifest_digest,
                        "generation":generation + 1, "lock_identity":binding.lock_identity,
                        "stage_identity":SelectionIdentity::try_from(staging.metadata().identity)?,
                    }),
                )?;
                staging.sync_dir()?;
                crate::selection_binding::record_recovery_stage(
                    &root,
                    "recovery-cutoff",
                    source_identity.into(),
                    Some(manifest_digest),
                    &staging,
                )?;
                observed_matches(&root, leaf, expected)?;
                #[cfg(windows)]
                let publish = source
                    .publish_replace(&native_dest, &native_leaf)
                    .map_err(anyhow::Error::from);
                #[cfg(target_os = "linux")]
                let publish = source
                    .publish_checked(
                        &root,
                        binding.parent.into(),
                        &child(leaf)?,
                        expected.map(|pointer| p::ExpectedSelection {
                            identity: pointer.identity.into(),
                            sha256: pointer.sha256,
                        }),
                    )
                    .map_err(anyhow::Error::from);
                match publish {
                    Ok(()) => {
                        let selected = Pointer {
                            identity: source_identity,
                            sha256: manifest_digest,
                        };
                        observed_matches(&root, leaf, Some(selected))
                            .context("cutoff replacement uncertain after native rename")?;
                        root.sync_dir()
                            .context("cutoff replacement uncertain after directory flush")?;
                        let state = State {
                            schema: "dendro.cutoff-state".into(),
                            version: 1,
                            generation: generation + 1,
                            previous: expected,
                            selected,
                        };
                        write_new(&root, &state_name(&hash, generation + 1), &state)
                            .context("cutoff replacement selected but state receipt incomplete; recovery required")?;
                        return Ok(());
                    }
                    Err(error) if publication_busy(&error) && attempt < 3 => {
                        std::thread::sleep(std::time::Duration::from_millis(
                            25 * u64::from(attempt),
                        ));
                    }
                    Err(error) => {
                        return Err(anyhow!(
                            "cutoff unchanged; prepared source and journal retained at {}: {error}",
                            staging.path().display()
                        ))
                    }
                }
            }
            Err(anyhow!("cutoff unchanged; retry budget exhausted"))
        })())
    })??;
    Ok(())
}

/// Reconcile only a uniquely matching durable prepared cutoff journal with
/// the observed destination identity and digest. A foreign pointer refuses.
pub(super) fn recover(path: &Path) -> Result<super::CutoffRecoveryOutcome> {
    let absolute = std::path::absolute(path)?;
    let parent = absolute
        .parent()
        .ok_or_else(|| anyhow!("cutoff destination has no parent"))?;
    let leaf = absolute
        .file_name()
        .and_then(|v| v.to_str())
        .ok_or_else(|| anyhow!("invalid cutoff leaf"))?;
    child(leaf)?;
    let root = p::open_existing_private_root(parent)?;
    let parent_identity = SelectionIdentity::try_from(root.metadata().identity)?;
    let initial_hash = leaf_hash(parent_identity, leaf);
    if read_existing(&root, &format!("cutoff-bind-{initial_hash}.json"))?.is_none() {
        let prefix = format!("cutoff-state-{initial_hash}-");
        let mut history = false;
        for entry in fs::read_dir(root.path())? {
            crate::RUN_CANCEL.check()?;
            if entry?.file_name().to_string_lossy().starts_with(&prefix) {
                history = true;
                break;
            }
        }
        if !history {
            let observed = read_existing(&root, leaf)?.map(|(pointer, _)| pointer);
            if let Some((lock_pointer, bytes)) =
                read_existing(&root, &format!("cutoff-lock-{initial_hash}.lock"))?
            {
                if !bytes.is_empty() {
                    return Err(anyhow!("orphan cutoff lock is not empty"));
                }
                let lock = child(&format!("cutoff-lock-{initial_hash}.lock"))?;
                let lock_id =
                    p::canonical_identity_from_persisted_record(lock_pointer.identity.into());
                return p::with_exclusive_lock(&root, &lock, &lock_id, |_| {
                    Ok((|| -> Result<super::CutoffRecoveryOutcome> {
                        if read_existing(&root, leaf)?.map(|(pointer, _)| pointer) == observed
                            && read_existing(&root, &format!("cutoff-bind-{initial_hash}.json"))?
                                .is_none()
                        {
                            Ok(super::CutoffRecoveryOutcome::Unchanged)
                        } else {
                            Err(anyhow!("cutoff admission advanced before recovery lock"))
                        }
                    })())
                })?;
            }
            return Ok(super::CutoffRecoveryOutcome::Unchanged);
        }
        if read_existing(&root, leaf)?.is_none() {
            return Err(anyhow!(
                "cutoff destination absent with custody history; recovery refuses"
            ));
        }
        // Missing binding with state history is reconstructed only from canonical
        // lock, valid baseline state and matching journal below.
    }
    let (hash, binding, torn_binding) = load_recovery_binding(&root, leaf)?;
    let lock_name = child(&binding.lock)?;
    let lock_id =
        p::canonical_identity_from_persisted_record(FileIdentity::from(binding.lock_identity));
    p::with_exclusive_lock(&root, &lock_name, &lock_id, |_| {
        Ok((|| -> Result<super::CutoffRecoveryOutcome> {
            let (_, current_binding, current_torn_binding) = load_recovery_binding(&root, leaf)?;
            if current_binding.parent != binding.parent
                || current_binding.lock_identity != binding.lock_identity
                || current_binding.baseline != binding.baseline
                || current_torn_binding != torn_binding
            {
                return Err(anyhow!("cutoff binding changed before recovery lock"));
            }
            let (generation, expected, torn_state) =
                load_state_mode(&root, &hash, current_binding.baseline, true)?;
            let actual = read_existing(&root, leaf)?.map(|(pointer, _)| pointer);
            if actual == expected && torn_state.is_none() && !current_torn_binding {
                return Ok(super::CutoffRecoveryOutcome::Unchanged);
            }
            if actual == expected && torn_state.is_some() {
                return Err(anyhow!(
                    "torn cutoff top state with unchanged pointer requires manual recovery"
                ));
            }
            let selected = actual
                .ok_or_else(|| anyhow!("cutoff destination disappeared; recovery refuses"))?;
            let (journal_generation, journal_previous) = if actual == expected {
                let (_, bytes) = read_existing(&root, &state_name(&hash, generation))?
                    .ok_or_else(|| anyhow!("cutoff last state missing"))?;
                let last: State = serde_json::from_slice(&bytes)?;
                if last.selected != selected {
                    return Err(anyhow!("cutoff last state differs from actual pointer"));
                }
                (generation, last.previous)
            } else {
                (generation + 1, expected)
            };
            let mut matches = 0usize;
            let mut examine_stage = |stage: &PrivateRoot| -> Result<()> {
                let mut examined = 0usize;
                for item in fs::read_dir(stage.path())? {
                    crate::RUN_CANCEL.check()?;
                    let item = item?;
                    let Some(journal_leaf) = item.file_name().to_str().map(str::to_owned) else {
                        continue;
                    };
                    if !journal_leaf.starts_with("cutoff-journal-")
                        || !journal_leaf.ends_with(".json")
                    {
                        continue;
                    }
                    examined += 1;
                    if examined > 16 {
                        return Err(anyhow!("cutoff recovery stage has too many journals"));
                    }
                    let Ok(Some((_, bytes))) = read_existing(stage, &journal_leaf) else {
                        continue;
                    };
                    let Ok(journal) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                        continue;
                    };
                    let matching = journal.get("schema").and_then(|v| v.as_str())
                        == Some("dendro.cutoff-journal")
                        && journal.get("state").and_then(|v| v.as_str()) == Some("prepared")
                        && journal.get("destination").and_then(|v| v.as_str()) == Some(leaf)
                        && journal.get("generation")
                            == Some(&serde_json::to_value(journal_generation)?)
                        && journal.get("lock_identity")
                            == Some(&serde_json::to_value(current_binding.lock_identity)?)
                        && journal.get("stage_identity")
                            == Some(&serde_json::to_value(SelectionIdentity::try_from(
                                stage.metadata().identity,
                            )?)?)
                        && journal.get("parent")
                            == Some(&serde_json::to_value(current_binding.parent)?)
                        && journal.get("expected")
                            == Some(&serde_json::to_value(journal_previous)?)
                        && journal.get("source_identity")
                            == Some(&serde_json::to_value(selected.identity)?)
                        && journal.get("new_sha256")
                            == Some(&serde_json::to_value(selected.sha256)?)
                        && journal
                            .get("source")
                            .and_then(|v| v.as_str())
                            .is_some_and(|name| {
                                child(name).is_ok() && !stage.path().join(name).exists()
                            });
                    if matching {
                        matches += 1;
                    }
                }
                Ok(())
            };
            if let Some(stage) = crate::selection_binding::recovery_stage(
                &root,
                "recovery-cutoff",
                selected.identity.into(),
                Some(selected.sha256),
                "cutoff-stage-",
            )? {
                examine_stage(&stage)?;
            } else {
                // Compatibility with stores written before direct locators.
                // Constant memory and no lifetime publication limit.
                for entry in fs::read_dir(root.path())? {
                    crate::RUN_CANCEL.check()?;
                    let entry = entry?;
                    if !entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with("cutoff-stage-")
                    {
                        continue;
                    }
                    if let Ok(stage) = crate::selection_binding::open_recovery_stage(&entry.path())
                    {
                        examine_stage(&stage)?;
                    }
                }
            }
            if matches != 1 {
                return Err(anyhow!(
                    "cutoff pointer has no unique matching prepared journal"
                ));
            }
            if current_torn_binding {
                let record_leaf = format!("cutoff-bind-{hash}.json");
                if read_existing(&root, &record_leaf)?.is_some() {
                    crate::selection_binding::quarantine_torn_record(&root, &record_leaf)?;
                }
                write_new(&root, &record_leaf, &current_binding)?;
            }
            if let Some(torn) = &torn_state {
                crate::selection_binding::quarantine_torn_record(&root, torn)?;
            }
            if actual == expected {
                return Ok(super::CutoffRecoveryOutcome::Reconciled);
            }
            write_new(
                &root,
                &state_name(&hash, generation + 1),
                &State {
                    schema: "dendro.cutoff-state".into(),
                    version: 1,
                    generation: generation + 1,
                    previous: expected,
                    selected,
                },
            )?;
            Ok(super::CutoffRecoveryOutcome::Reconciled)
        })())
    })?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn preexisting_cutoff_without_custody_is_typed_unchanged_with_or_without_empty_lock() {
        let root = p::create_user_private_root_with_choice(
            "dendro-cutoff-uncustodied-",
            &p::StorageRootChoice::UserProfile,
        )
        .unwrap();
        let path = root.path().join("capture.json");
        let mut file = root.create_file(&child("capture.json").unwrap()).unwrap();
        file.file_mut()
            .write_all(br#"{"preexisting":true}"#)
            .unwrap();
        file.file().sync_all().unwrap();
        let identity = file.metadata().identity;
        drop(file);
        assert_eq!(
            recover(&path).unwrap(),
            super::super::CutoffRecoveryOutcome::Unchanged
        );
        let hash = leaf_hash(
            SelectionIdentity::try_from(root.metadata().identity).unwrap(),
            "capture.json",
        );
        drop(
            root.create_file(&child(&format!("cutoff-lock-{hash}.lock")).unwrap())
                .unwrap(),
        );
        assert_eq!(
            recover(&path).unwrap(),
            super::super::CutoffRecoveryOutcome::Unchanged
        );
        assert_eq!(
            p::open_existing_private_file(&root, &child("capture.json").unwrap())
                .unwrap()
                .metadata()
                .identity,
            identity
        );
        assert_eq!(fs::read(&path).unwrap(), br#"{"preexisting":true}"#);
        let cleanup = root.path().to_path_buf();
        drop(root);
        fs::remove_dir_all(cleanup).unwrap();
    }

    #[test]
    fn untouched_cutoff_recovery_reports_unchanged_without_creating_binding() {
        let root = p::create_user_private_root_with_choice(
            "dendro-cutoff-noop-",
            &p::StorageRootChoice::UserProfile,
        )
        .unwrap();
        let path = root.path().join("capture.json");
        let before = fs::read_dir(root.path()).unwrap().count();
        assert_eq!(
            recover(&path).unwrap(),
            super::super::CutoffRecoveryOutcome::Unchanged
        );
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), before);
    }

    #[test]
    fn prepared_cutoff_journal_recovers_switched_pointer() {
        let root = p::create_user_private_root_with_choice(
            "dendro-cutoff-recover-",
            &p::StorageRootChoice::UserProfile,
        )
        .unwrap();
        let path = root.path().join("capture.json");
        publish(&path, br#"{"first":true}"#).unwrap();
        let parent = SelectionIdentity::try_from(root.metadata().identity).unwrap();
        let state = root
            .path()
            .join(state_name(&leaf_hash(parent, "capture.json"), 1));
        fs::remove_file(state).unwrap();
        assert!(publish(&path, br#"{"second":true}"#).is_err());
        recover(&path).unwrap();
        publish(&path, br#"{"second":true}"#).unwrap();
        assert_eq!(fs::read(&path).unwrap(), br#"{"second":true}"#);
    }

    #[test]
    fn torn_cutoff_binding_and_state_recover_only_with_matching_prepared_journal() {
        let root = p::create_user_private_root_with_choice(
            "dendro-cutoff-torn-",
            &p::StorageRootChoice::UserProfile,
        )
        .unwrap();
        let path = root.path().join("capture.json");
        publish(&path, br#"{"first":true}"#).unwrap();
        let parent = SelectionIdentity::try_from(root.metadata().identity).unwrap();
        let hash = leaf_hash(parent, "capture.json");
        let binding = root.path().join(format!("cutoff-bind-{hash}.json"));
        fs::write(&binding, b"").unwrap();
        assert!(publish(&path, br#"{"second":true}"#).is_err());
        recover(&path).unwrap();
        let unrelated = p::create_unique_private_directory(&root, "cutoff-stage-").unwrap();
        fs::write(unrelated.path().join("cutoff-journal-bad.json"), b"{").unwrap();
        let state = root.path().join(state_name(&hash, 1));
        fs::write(&state, b"{").unwrap();
        assert!(publish(&path, br#"{"second":true}"#).is_err());
        recover(&path).unwrap();
        publish(&path, br#"{"second":true}"#).unwrap();
        assert_eq!(fs::read(&path).unwrap(), br#"{"second":true}"#);
    }

    #[test]
    fn cutoff_history_direct_lookup_and_pre_locator_compatibility() {
        use crate::selection_binding::RECOVERY_STAGE_OPENS;
        let root = p::create_user_private_root_with_choice(
            "dendro-cutoff-history-",
            &p::StorageRootChoice::UserProfile,
        )
        .unwrap();
        let path = root.path().join("capture.json");
        publish(&path, br#"{"first":true}"#).unwrap();
        let hash = leaf_hash(root.metadata().identity.try_into().unwrap(), "capture.json");
        let state = root.path().join(state_name(&hash, 1));
        let selected = read_existing(&root, "capture.json").unwrap().unwrap().0;
        for ordinal in 0..4100 {
            let stage = root
                .path()
                .join(format!("cutoff-stage-history-{ordinal:04}"));
            fs::create_dir(&stage).unwrap();
            fs::write(
                stage.join("cutoff-journal-old.json"),
                match ordinal % 3 {
                    0 => b"{\"state\":\"prepared\"}".as_slice(),
                    1 => b"{\"state\":\"published\"}",
                    _ => b"{",
                },
            )
            .unwrap();
        }
        fs::remove_file(&state).unwrap();
        RECOVERY_STAGE_OPENS.with(|n| n.set(0));
        assert_eq!(
            recover(&path).unwrap(),
            super::super::CutoffRecoveryOutcome::Reconciled
        );
        assert_eq!(RECOVERY_STAGE_OPENS.with(|n| n.get()), 1);
        let locator = fs::read_dir(root.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("recovery-cutoff-")
            })
            .unwrap();
        fs::remove_file(locator).unwrap();
        fs::remove_file(&state).unwrap();
        RECOVERY_STAGE_OPENS.with(|n| n.set(0));
        recover(&path).unwrap();
        assert_eq!(RECOVERY_STAGE_OPENS.with(|n| n.get()), 4101);
        assert_eq!(
            read_existing(&root, "capture.json").unwrap().unwrap().0,
            selected
        );
        assert_eq!(
            fs::read_dir(root.path())
                .unwrap()
                .filter_map(Result::ok)
                .filter(|e| e
                    .file_name()
                    .to_string_lossy()
                    .starts_with("cutoff-stage-history-"))
                .count(),
            4100
        );
    }

    #[test]
    fn cutoff_orphan_lock_is_unchanged_and_binding_repair_resumes_after_quarantine() {
        let root = p::create_user_private_root_with_choice(
            "dendro-cutoff-repair-gap-",
            &p::StorageRootChoice::UserProfile,
        )
        .unwrap();
        let path = root.path().join("capture.json");
        let hash = leaf_hash(root.metadata().identity.try_into().unwrap(), "capture.json");
        let lock = format!("cutoff-lock-{hash}.lock");
        drop(root.create_file(&child(&lock).unwrap()).unwrap());
        let before = fs::read_dir(root.path()).unwrap().count();
        assert_eq!(
            recover(&path).unwrap(),
            super::super::CutoffRecoveryOutcome::Unchanged
        );
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), before);
        publish(&path, br#"{"first":true}"#).unwrap();
        let binding = format!("cutoff-bind-{hash}.json");
        fs::write(root.path().join(&binding), b"{").unwrap();
        crate::selection_binding::quarantine_torn_record(&root, &binding).unwrap();
        // Interruption at the former destructive gap, before writing a repair.
        assert_eq!(
            recover(&path).unwrap(),
            super::super::CutoffRecoveryOutcome::Reconciled
        );
        assert!(root.path().join(&binding).exists());
        assert_eq!(
            recover(&path).unwrap(),
            super::super::CutoffRecoveryOutcome::Unchanged
        );
        // Missing pointer with prior custody cannot masquerade as empty admission.
        fs::remove_file(root.path().join(&binding)).unwrap();
        fs::remove_file(&path).unwrap();
        assert!(recover(&path).is_err());
    }

    #[test]
    fn cutoff_direct_locator_cannot_replace_a_journal_proof_or_overwrite_a_collision() {
        let root = p::create_user_private_root_with_choice(
            "dendro-cutoff-locator-",
            &p::StorageRootChoice::UserProfile,
        )
        .unwrap();
        let path = root.path().join("capture.json");
        publish(&path, br#"{"first":true}"#).unwrap();
        let selected = read_existing(&root, "capture.json").unwrap().unwrap().0;
        let stage = crate::selection_binding::recovery_stage(
            &root,
            "recovery-cutoff",
            selected.identity.into(),
            Some(selected.sha256),
            "cutoff-stage-",
        )
        .unwrap()
        .unwrap();
        let error = crate::selection_binding::record_recovery_stage(
            &root,
            "recovery-cutoff",
            selected.identity.into(),
            Some(selected.sha256),
            &stage,
        )
        .unwrap_err();
        assert!(error.to_string().contains("rename absent"), "{error:#}");
        let journal_path = fs::read_dir(stage.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("cutoff-journal-")
            })
            .unwrap();
        let mut journal: serde_json::Value =
            serde_json::from_slice(&fs::read(&journal_path).unwrap()).unwrap();
        journal["expected"] =
            serde_json::json!({"identity": selected.identity, "sha256": selected.sha256});
        fs::write(&journal_path, serde_json::to_vec(&journal).unwrap()).unwrap();
        let hash = leaf_hash(root.metadata().identity.try_into().unwrap(), "capture.json");
        fs::remove_file(root.path().join(state_name(&hash, 1))).unwrap();
        assert!(recover(&path)
            .unwrap_err()
            .to_string()
            .contains("no unique matching"));
        assert_eq!(
            read_existing(&root, "capture.json").unwrap().unwrap().0,
            selected
        );
    }
}
