#![cfg(any(windows, target_os = "linux"))]

use std::fs;
use std::path::PathBuf;

use dendro_core::platform_v1::{self, StorageRootChoice};
use dendro_core::selection_binding::{
    recover_logical_db, resolve_logical_db, run_locked_with_selection,
    run_locked_with_selection_receipt, RecoveryDisposition, SelectedDb,
};
use dendro_core::AnalyticsStore;

#[test]
fn fresh_logical_marker_selects_closed_bundle_and_keeps_key() {
    let root = platform_v1::create_user_private_root_with_choice(
        "dendro-selection-test-",
        &StorageRootChoice::UserProfile,
    )
    .expect("private root");
    let logical = root.path().join("données-分析.duckdb");
    let choice = StorageRootChoice::Explicit(root.path().to_path_buf());
    let run = || {
        run_locked_with_selection(
            &logical,
            &choice,
            &[PathBuf::from("missing-synthetic-source")],
            &[],
            |staged, _| {
                let store = AnalyticsStore::open_staged(staged)?;
                store.checkpoint()?;
                Ok(vec!["synthetic".into()])
            },
        )
    };
    let first = run().expect("first publication");
    assert_eq!(first["status"], "ready");
    assert_eq!(fs::read(&logical).unwrap(), b"DENDRO-LOGICAL-DB v1\n");
    let unicode_case_alias = root.path().join("DONNÉES-分析.duckdb");
    #[cfg(windows)]
    assert!(matches!(
        resolve_logical_db(&unicode_case_alias, &choice).unwrap(),
        SelectedDb::Selected(_)
    ));
    #[cfg(target_os = "linux")]
    assert!(matches!(
        resolve_logical_db(&unicode_case_alias, &choice).unwrap(),
        SelectedDb::Uninitialized
    ));
    let prior = match resolve_logical_db(&logical, &choice).unwrap() {
        SelectedDb::Selected(read) => read,
        _ => panic!("expected selected version"),
    };
    let first_db = prior.database.clone();
    let old_bytes = fs::read(&first_db).unwrap();
    let key = fs::read(&prior.key).unwrap();
    let store = AnalyticsStore::open_read_only_selected(&prior).unwrap();
    assert_eq!(store.key_id, first["key_id"].as_str().unwrap());
    drop(store);
    run().expect("second publication");
    let next = match resolve_logical_db(&logical, &choice).unwrap() {
        SelectedDb::Selected(read) => read,
        _ => panic!("expected newer selected version"),
    };
    assert_ne!(next.database, first_db);
    assert_eq!(fs::read(&first_db).unwrap(), old_bytes);
    assert_eq!(fs::read(&next.key).unwrap(), key);
    assert!(!prior.is_current().unwrap());
    assert!(next.is_current().unwrap());
}

fn synthetic_run(
    logical: &std::path::Path,
    choice: &StorageRootChoice,
) -> anyhow::Result<serde_json::Value> {
    run_locked_with_selection(
        logical,
        choice,
        &[PathBuf::from("missing-synthetic-source")],
        &[],
        |staged, _| {
            let store = AnalyticsStore::open_staged(staged)?;
            store.checkpoint()?;
            Ok(vec!["synthetic".into()])
        },
    )
}

fn leaf(root: &std::path::Path, prefix: &str) -> PathBuf {
    fs::read_dir(root)
        .unwrap()
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            path.file_name()?
                .to_str()?
                .starts_with(prefix)
                .then_some(path)
        })
        .next()
        .expect("selected generated leaf")
}

#[test]
fn legacy_bytes_are_preserved_and_missing_pointer_never_falls_back() {
    let root = platform_v1::create_user_private_root_with_choice(
        "dendro-legacy-test-",
        &StorageRootChoice::UserProfile,
    )
    .unwrap();
    let logical = root.path().join("legacy.duckdb");
    let key = dendro_core::output::load_or_create_key(
        &PathBuf::from(format!("{}.key", logical.display())),
        true,
    )
    .unwrap();
    let store = AnalyticsStore::open_path(&logical, &key.id).unwrap();
    store.checkpoint().unwrap();
    drop(store);
    let before = fs::read(&logical).unwrap();
    let choice = StorageRootChoice::Explicit(root.path().to_path_buf());
    match resolve_logical_db(&logical, &choice).unwrap() {
        SelectedDb::Legacy(read) => assert_eq!(
            AnalyticsStore::open_read_only(&read.database)
                .unwrap()
                .key_id,
            key.id
        ),
        _ => panic!("expected admitted legacy read"),
    }
    synthetic_run(&logical, &choice).unwrap();
    assert_eq!(fs::read(&logical).unwrap(), before);
    let pointer = leaf(root.path(), "selection-");
    fs::remove_file(&pointer).unwrap();
    let error = resolve_logical_db(&logical, &choice)
        .err()
        .expect("missing pointer must fail");
    assert!(
        error.to_string().contains("selection") || error.to_string().contains("io:"),
        "{error:#}"
    );
    assert_eq!(fs::read(&logical).unwrap(), before);
}

#[test]
fn prepared_native_journal_recovers_selected_pointer_without_legacy_rollback() {
    let root = platform_v1::create_user_private_root_with_choice(
        "dendro-recover-test-",
        &StorageRootChoice::UserProfile,
    )
    .unwrap();
    let logical = root.path().join("analytics.duckdb");
    let choice = StorageRootChoice::Explicit(root.path().to_path_buf());
    synthetic_run(&logical, &choice).unwrap();
    let state = leaf(root.path(), "selected-");
    fs::remove_file(state).unwrap();
    assert!(resolve_logical_db(&logical, &choice).is_err());
    let recovered = recover_logical_db(&logical, &choice).unwrap();
    assert!(recovered.is_current().unwrap());
    assert!(!AnalyticsStore::open_read_only_selected(&recovered)
        .unwrap()
        .key_id
        .is_empty());
}

#[cfg(windows)]
#[test]
fn busy_pointer_preserves_old_selection_and_candidate() {
    use std::os::windows::fs::OpenOptionsExt;
    let root = platform_v1::create_user_private_root_with_choice(
        "dendro-busy-test-",
        &StorageRootChoice::UserProfile,
    )
    .unwrap();
    let logical = root.path().join("analytics.duckdb");
    let choice = StorageRootChoice::Explicit(root.path().to_path_buf());
    synthetic_run(&logical, &choice).unwrap();
    let pointer = leaf(root.path(), "selection-");
    let old = fs::read(&pointer).unwrap();
    let held = fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&pointer)
        .unwrap();
    let error = synthetic_run(&logical, &choice).unwrap_err();
    assert!(
        error.to_string().contains("candidate retained"),
        "{error:#}"
    );
    assert_eq!(fs::read(&pointer).unwrap(), old);
    drop(held);
    assert!(matches!(
        resolve_logical_db(&logical, &choice).unwrap(),
        SelectedDb::Selected(_)
    ));
    assert!(fs::read_dir(root.path())
        .unwrap()
        .filter_map(Result::ok)
        .any(|entry| entry.file_name().to_string_lossy().starts_with("bundle-")));
}

#[test]
fn changed_bundle_sidecar_refuses_selected_read() {
    let root = platform_v1::create_user_private_root_with_choice(
        "dendro-sidecar-test-",
        &StorageRootChoice::UserProfile,
    )
    .unwrap();
    let logical = root.path().join("analytics.duckdb");
    let choice = StorageRootChoice::Explicit(root.path().to_path_buf());
    synthetic_run(&logical, &choice).unwrap();
    let selected = match resolve_logical_db(&logical, &choice).unwrap() {
        SelectedDb::Selected(read) => read,
        _ => panic!("expected selected version"),
    };
    let provenance = selected.database.parent().unwrap().join("provenance.json");
    drop(selected);
    fs::write(provenance, b"{\"schema\":\"tampered\"}").unwrap();
    let error = resolve_logical_db(&logical, &choice)
        .err()
        .expect("tampered sidecar must refuse");
    assert!(
        error.to_string().contains("selection") || error.to_string().contains("digest"),
        "{error:#}"
    );
}

#[test]
fn relative_logical_leaf_in_admitted_current_directory_creates_binding() {
    struct Restore(PathBuf);
    impl Drop for Restore {
        fn drop(&mut self) {
            std::env::set_current_dir(&self.0).unwrap();
        }
    }
    let root = platform_v1::create_user_private_root_with_choice(
        "dendro-relative-test-",
        &StorageRootChoice::UserProfile,
    )
    .unwrap();
    let previous = std::env::current_dir().unwrap();
    let _restore = Restore(previous);
    std::env::set_current_dir(root.path()).unwrap();
    let choice = StorageRootChoice::Explicit(root.path().to_path_buf());
    synthetic_run(std::path::Path::new("relative.duckdb"), &choice).unwrap();
    assert!(matches!(
        resolve_logical_db(std::path::Path::new("relative.duckdb"), &choice).unwrap(),
        SelectedDb::Selected(_)
    ));
}

#[test]
fn replaced_or_deleted_logical_anchor_never_falls_back_to_legacy() {
    let root = platform_v1::create_user_private_root_with_choice(
        "dendro-anchor-test-",
        &StorageRootChoice::UserProfile,
    )
    .unwrap();
    let logical = root.path().join("analytics.duckdb");
    let choice = StorageRootChoice::Explicit(root.path().to_path_buf());
    synthetic_run(&logical, &choice).unwrap();
    // Keep the old inode/file ID alive so the replacement cannot reuse it.
    let _original_anchor = fs::File::open(&logical).unwrap();
    fs::remove_file(&logical).unwrap();
    assert!(resolve_logical_db(&logical, &choice).is_err());
    #[cfg(target_os = "linux")]
    drop(dendro_core::lifecycle::create_private_file(&logical).unwrap());
    fs::write(&logical, b"replacement").unwrap();
    assert!(resolve_logical_db(&logical, &choice).is_err());
    assert!(synthetic_run(&logical, &choice).is_err());
    let error = recover_logical_db(&logical, &choice).err().unwrap();
    assert!(
        error.downcast_ref::<RecoveryDisposition>().is_none(),
        "a displaced anchor is not untouched legacy"
    );
    assert!(error.to_string().contains("anchor changed"), "{error:#}");
}

#[test]
fn distinct_logical_leaves_share_admitted_root_without_cross_binding() {
    let root = platform_v1::create_user_private_root_with_choice(
        "dendro-multi-logical-",
        &StorageRootChoice::UserProfile,
    )
    .unwrap();
    let choice = StorageRootChoice::Explicit(root.path().to_path_buf());
    let first = root.path().join("first.duckdb");
    let second = root.path().join("second.duckdb");
    synthetic_run(&first, &choice).unwrap();
    synthetic_run(&second, &choice).unwrap();
    let first_selected = match resolve_logical_db(&first, &choice).unwrap() {
        SelectedDb::Selected(read) => read.selection(),
        _ => panic!("first selected"),
    };
    let second_selected = match resolve_logical_db(&second, &choice).unwrap() {
        SelectedDb::Selected(read) => read.selection(),
        _ => panic!("second selected"),
    };
    assert_ne!(first_selected.identity, second_selected.identity);
    fs::remove_file(&first).unwrap();
    assert!(resolve_logical_db(&first, &choice).is_err());
    assert!(matches!(
        resolve_logical_db(&second, &choice).unwrap(),
        SelectedDb::Selected(_)
    ));
}

#[test]
fn legacy_wal_refuses_adoption_without_changing_legacy_bytes() {
    let root = platform_v1::create_user_private_root_with_choice(
        "dendro-wal-test-",
        &StorageRootChoice::UserProfile,
    )
    .unwrap();
    let logical = root.path().join("legacy.duckdb");
    let key = dendro_core::output::load_or_create_key(
        &PathBuf::from(format!("{}.key", logical.display())),
        true,
    )
    .unwrap();
    let store = AnalyticsStore::open_path(&logical, &key.id).unwrap();
    store.checkpoint().unwrap();
    drop(store);
    let original = fs::read(&logical).unwrap();
    fs::write(format!("{}.wal", logical.display()), b"pending").unwrap();
    let choice = StorageRootChoice::Explicit(root.path().to_path_buf());
    let error = synthetic_run(&logical, &choice).unwrap_err();
    assert!(error.to_string().contains("wal"), "{error:#}");
    assert_eq!(fs::read(&logical).unwrap(), original);
}

#[test]
fn required_source_is_checked_before_writer_copy_or_ingest() {
    let root = platform_v1::create_user_private_root_with_choice(
        "dendro-required-source-",
        &StorageRootChoice::UserProfile,
    )
    .unwrap();
    let logical = root.path().join("analytics.duckdb");
    let choice = StorageRootChoice::Explicit(root.path().to_path_buf());
    synthetic_run(&logical, &choice).unwrap();
    let old = match resolve_logical_db(&logical, &choice).unwrap() {
        SelectedDb::Selected(read) => read.selection(),
        _ => panic!("selected"),
    };
    synthetic_run(&logical, &choice).unwrap();
    let mut called = false;
    let error =
        run_locked_with_selection_receipt(&logical, &choice, &[], &[], Some(old), |_, _| {
            called = true;
            Ok(vec![])
        })
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("sealed replay source selection changed"),
        "{error:#}"
    );
    assert!(!called);
}

#[test]
fn unpublished_binding_and_orphan_sentinels_resume_without_legacy_read() {
    let root = platform_v1::create_user_private_root_with_choice(
        "dendro-started-retry-",
        &StorageRootChoice::UserProfile,
    )
    .unwrap();
    let logical = root.path().join("analytics.duckdb");
    let choice = StorageRootChoice::Explicit(root.path().to_path_buf());
    let failed = run_locked_with_selection(&logical, &choice, &[], &[], |_, _| {
        anyhow::bail!("injected pre-publication failure")
    });
    assert!(failed.is_err());
    assert!(resolve_logical_db(&logical, &choice).is_err());
    let binding = leaf(root.path(), "binding-");
    let writer = leaf(root.path(), "writer-");
    let publisher = leaf(root.path(), "publisher-");
    let writer_id = platform_v1::reopen_metadata(&writer).unwrap().identity;
    let publisher_id = platform_v1::reopen_metadata(&publisher).unwrap().identity;
    fs::remove_file(binding).unwrap();
    assert!(resolve_logical_db(&logical, &choice).is_err());
    synthetic_run(&logical, &choice).unwrap();
    assert_eq!(
        platform_v1::reopen_metadata(&writer).unwrap().identity,
        writer_id
    );
    assert_eq!(
        platform_v1::reopen_metadata(&publisher).unwrap().identity,
        publisher_id
    );
    assert!(matches!(
        resolve_logical_db(&logical, &choice).unwrap(),
        SelectedDb::Selected(_)
    ));
}

#[test]
fn first_publication_retry_keeps_started_binding_without_pointer() {
    let root = platform_v1::create_user_private_root_with_choice(
        "dendro-first-retry-",
        &StorageRootChoice::UserProfile,
    )
    .unwrap();
    let logical = root.path().join("analytics.duckdb");
    let choice = StorageRootChoice::Explicit(root.path().to_path_buf());
    let failed = run_locked_with_selection(&logical, &choice, &[], &[], |_, _| {
        anyhow::bail!("injected pre-publication failure")
    });
    assert!(failed.is_err());
    assert!(resolve_logical_db(&logical, &choice).is_err());
    let recovery_error = recover_logical_db(&logical, &choice).err().unwrap();
    assert_eq!(
        recovery_error.downcast_ref::<RecoveryDisposition>(),
        Some(&RecoveryDisposition::ResumeRequired)
    );
    let binding = leaf(root.path(), "binding-");
    let original = fs::read(&binding).unwrap();
    synthetic_run(&logical, &choice).unwrap();
    assert_eq!(fs::read(&binding).unwrap(), original);
    assert!(matches!(
        resolve_logical_db(&logical, &choice).unwrap(),
        SelectedDb::Selected(_)
    ));
}

#[test]
fn recover_untouched_legacy_is_typed_noop_and_preserves_readability() {
    let root = platform_v1::create_user_private_root_with_choice(
        "dendro-legacy-noop-",
        &StorageRootChoice::UserProfile,
    )
    .unwrap();
    let logical = root.path().join("legacy.duckdb");
    let key = dendro_core::output::load_or_create_key(
        &PathBuf::from(format!("{}.key", logical.display())),
        true,
    )
    .unwrap();
    let store = AnalyticsStore::open_path(&logical, &key.id).unwrap();
    store.checkpoint().unwrap();
    drop(store);
    let before = fs::read_dir(root.path()).unwrap().count();
    let bytes = fs::read(&logical).unwrap();
    let choice = StorageRootChoice::Explicit(root.path().to_path_buf());
    let error = recover_logical_db(&logical, &choice).err().unwrap();
    assert_eq!(
        error.downcast_ref::<RecoveryDisposition>(),
        Some(&RecoveryDisposition::NothingToRecover)
    );
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), before);
    assert_eq!(fs::read(&logical).unwrap(), bytes);
    assert!(matches!(
        resolve_logical_db(&logical, &choice).unwrap(),
        SelectedDb::Legacy(_)
    ));
}

#[test]
fn torn_binding_recovers_from_own_journal_without_bricking_other_logical_db() {
    let root = platform_v1::create_user_private_root_with_choice(
        "dendro-torn-binding-",
        &StorageRootChoice::UserProfile,
    )
    .unwrap();
    let choice = StorageRootChoice::Explicit(root.path().to_path_buf());
    let first = root.path().join("first.duckdb");
    let second = root.path().join("second.duckdb");
    synthetic_run(&first, &choice).unwrap();
    let binding = leaf(root.path(), "binding-");
    synthetic_run(&second, &choice).unwrap();
    fs::write(&binding, b"").unwrap();
    assert!(resolve_logical_db(&first, &choice).is_err());
    let unbound = root.path().join("unbound.duckdb");
    #[cfg(target_os = "linux")]
    drop(dendro_core::lifecycle::create_private_file(&unbound).unwrap());
    fs::write(&unbound, b"unbound").unwrap();
    let ambiguous = recover_logical_db(&unbound, &choice).err().unwrap();
    assert!(ambiguous.downcast_ref::<RecoveryDisposition>().is_none());
    assert!(
        ambiguous
            .to_string()
            .contains("ambiguous with a torn binding"),
        "{ambiguous:#}"
    );
    assert!(matches!(
        resolve_logical_db(&second, &choice).unwrap(),
        SelectedDb::Selected(_)
    ));
    assert!(recover_logical_db(&first, &choice)
        .unwrap()
        .is_current()
        .unwrap());
    assert!(fs::read_dir(root.path())
        .unwrap()
        .filter_map(Result::ok)
        .any(|entry| entry.file_name().to_string_lossy().starts_with("torn-")));
    // Simulate a second repair interrupted after its quarantine rename and
    // before the canonical binding rewrite. Locks and own journal remain.
    fs::rename(&binding, root.path().join("interrupted-binding.json")).unwrap();
    assert!(recover_logical_db(&first, &choice)
        .unwrap()
        .is_current()
        .unwrap());
    // Missing binding never permits an unrelated pointer to establish custody.
    let record: serde_json::Value = serde_json::from_slice(&fs::read(&binding).unwrap()).unwrap();
    let pointer = root.path().join(record["manifest"].as_str().unwrap());
    fs::remove_file(&binding).unwrap();
    let original_pointer = fs::read(&pointer).unwrap();
    fs::write(&pointer, b"{}").unwrap();
    assert!(recover_logical_db(&first, &choice).is_err());
    assert!(!binding.exists());
    fs::write(&pointer, original_pointer).unwrap();
    // Rewriting the pointer changes bytes only, not its identity; the journal
    // proves the exact original pointer again.
    assert!(recover_logical_db(&first, &choice).is_ok());
}

#[test]
fn truncated_top_state_recovers_with_matching_journal_and_skips_malformed_foreign_journal() {
    let root = platform_v1::create_user_private_root_with_choice(
        "dendro-torn-state-",
        &StorageRootChoice::UserProfile,
    )
    .unwrap();
    let logical = root.path().join("analytics.duckdb");
    let choice = StorageRootChoice::Explicit(root.path().to_path_buf());
    synthetic_run(&logical, &choice).unwrap();
    let unrelated = platform_v1::create_unique_private_directory(&root, "publication-").unwrap();
    fs::write(unrelated.path().join("selection-journal-bad.jsonl"), b"{").unwrap();
    let state = leaf(root.path(), "selected-");
    fs::write(&state, b"{").unwrap();
    assert!(resolve_logical_db(&logical, &choice).is_err());
    let recovered = recover_logical_db(&logical, &choice).unwrap();
    assert!(recovered.is_current().unwrap());
    assert!(fs::read_dir(root.path())
        .unwrap()
        .filter_map(Result::ok)
        .any(|entry| entry.file_name().to_string_lossy().starts_with("torn-")));
}

#[test]
fn selection_locator_refusals_never_downgrade_to_history_scan() {
    for case in [
        "malformed",
        "wrong_prefix",
        "wrong_identity",
        "missing_stage",
        "forged_journal",
    ] {
        let root = platform_v1::create_user_private_root_with_choice(
            "dendro-locator-refusal-",
            &StorageRootChoice::UserProfile,
        )
        .unwrap();
        let logical = root.path().join("analytics.duckdb");
        let choice = StorageRootChoice::Explicit(root.path().to_path_buf());
        synthetic_run(&logical, &choice).unwrap();
        let pointer = leaf(root.path(), "selection-");
        let pointer_before = fs::read(&pointer).unwrap();
        let locator = leaf(root.path(), "recovery-selection-");
        let mut record: serde_json::Value =
            serde_json::from_slice(&fs::read(&locator).unwrap()).unwrap();
        let stage = root.path().join(record["stage"].as_str().unwrap());
        fs::remove_file(leaf(root.path(), "selected-")).unwrap();
        match case {
            "malformed" => fs::write(&locator, b"{").unwrap(),
            "wrong_prefix" => {
                record["stage"] = serde_json::json!("foreign-stage");
                fs::write(&locator, serde_json::to_vec(&record).unwrap()).unwrap();
            }
            "wrong_identity" => {
                #[cfg(windows)]
                {
                    record["identity"]["file_id"][0] =
                        serde_json::json!(record["identity"]["file_id"][0].as_u64().unwrap() ^ 1);
                }
                #[cfg(target_os = "linux")]
                {
                    record["identity"]["inode"] =
                        serde_json::json!(record["identity"]["inode"].as_u64().unwrap() + 1);
                }
                fs::write(&locator, serde_json::to_vec(&record).unwrap()).unwrap();
            }
            "missing_stage" => fs::rename(
                &stage,
                stage.with_file_name(format!("retained-{:032x}", rand::random::<u128>())),
            )
            .unwrap(),
            "forged_journal" => {
                let journal = leaf(&stage, "selection-journal-");
                let mut prepared: serde_json::Value = serde_json::from_slice(
                    fs::read(&journal)
                        .unwrap()
                        .split(|byte| *byte == b'\n')
                        .next()
                        .unwrap(),
                )
                .unwrap();
                prepared["manifest_sha256"][0] =
                    serde_json::json!(prepared["manifest_sha256"][0].as_u64().unwrap() ^ 1);
                fs::write(journal, serde_json::to_vec(&prepared).unwrap()).unwrap();
            }
            _ => unreachable!(),
        }
        let error = recover_logical_db(&logical, &choice).err().expect(case);
        if case == "missing_stage" {
            assert!(format!("{error:#}").contains("missing or unadmitted recovery stage"));
        }
        assert_eq!(fs::read(&pointer).unwrap(), pointer_before, "{case}");
        assert!(
            !fs::read_dir(root.path()).unwrap().any(|entry| entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("selected-")),
            "{case}"
        );
        let path = root.path().to_path_buf();
        drop(root);
        fs::remove_dir_all(path).unwrap();
    }
}

#[test]
fn pre_locator_recovery_cancellation_preserves_pointer_and_state() {
    const CHILD: &str = "DENDRO_SELECTION_CANCEL_TEST_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "pre_locator_recovery_cancellation_preserves_pointer_and_state",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let root = platform_v1::create_user_private_root_with_choice(
        "dendro-cancel-recovery-",
        &StorageRootChoice::UserProfile,
    )
    .unwrap();
    let logical = root.path().join("analytics.duckdb");
    let choice = StorageRootChoice::Explicit(root.path().to_path_buf());
    synthetic_run(&logical, &choice).unwrap();
    let pointer = leaf(root.path(), "selection-");
    let before = fs::read(&pointer).unwrap();
    fs::remove_file(leaf(root.path(), "recovery-selection-")).unwrap();
    fs::remove_file(leaf(root.path(), "selected-")).unwrap();
    dendro_core::RUN_CANCEL.request();
    let error = recover_logical_db(&logical, &choice).err().unwrap();
    assert!(format!("{error:#}").contains("interrupted"));
    assert_eq!(fs::read(pointer).unwrap(), before);
    assert!(!fs::read_dir(root.path()).unwrap().any(|entry| entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with("selected-")));
    let path = root.path().to_path_buf();
    drop(root);
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn recovery_uses_prepared_journal_when_selected_marker_is_missing_or_torn() {
    for torn in [false, true] {
        let root = platform_v1::create_user_private_root_with_choice(
            "dendro-journal-tail-",
            &StorageRootChoice::UserProfile,
        )
        .unwrap();
        let logical = root.path().join("analytics.duckdb");
        let choice = StorageRootChoice::Explicit(root.path().to_path_buf());
        synthetic_run(&logical, &choice).unwrap();
        let locator: serde_json::Value =
            serde_json::from_slice(&fs::read(leaf(root.path(), "recovery-selection-")).unwrap())
                .unwrap();
        let stage = root.path().join(locator["stage"].as_str().unwrap());
        let journal = leaf(&stage, "selection-journal-");
        let bytes = fs::read(&journal).unwrap();
        let mut prepared = bytes.split(|byte| *byte == b'\n').next().unwrap().to_vec();
        prepared.push(b'\n');
        if torn {
            prepared.extend_from_slice(b"{\"state\":");
        }
        fs::write(&journal, prepared).unwrap();
        fs::remove_file(leaf(root.path(), "selected-")).unwrap();
        let pointer_before = fs::read(leaf(root.path(), "selection-")).unwrap();
        let (recovered, outcome) =
            dendro_core::selection_binding::recover_logical_db_with_outcome(&logical, &choice)
                .unwrap();
        assert_eq!(
            outcome,
            dendro_core::selection_binding::SelectionRecoveryOutcome::Reconciled
        );
        assert!(recovered.is_current().unwrap());
        assert_eq!(
            fs::read(leaf(root.path(), "selection-")).unwrap(),
            pointer_before
        );
        drop(recovered);
        let path = root.path().to_path_buf();
        drop(root);
        fs::remove_dir_all(path).unwrap();
    }
}
