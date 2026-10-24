//! Synthetic no-log migration and rollback setup; copied only into a private source snapshot.
#![deny(unsafe_code)]
use dendro_core::migration_v4 as mig;
use duckdb::Connection;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::path::Path;

fn hash(path: &Path) -> String {
    hex::encode(Sha256::digest(std::fs::read(path).unwrap()))
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args
        .get(1)
        .is_some_and(|mode| mode == "--check-anchor-divergence")
    {
        let database = Path::new(&args[2]);
        let choice = dendro_core::platform_v1::StorageRootChoice::Explicit(
            database.parent().unwrap().to_path_buf(),
        );
        let error = dendro_core::selection_binding::resolve_logical_db(database, &choice)
            .err()
            .ok_or_else(|| anyhow::anyhow!("old-binary anchor write silently admitted"))?
            .to_string();
        anyhow::ensure!(
            error.contains("legacy_anchor_diverged_after_selection")
                || error.contains("logical anchor changed or disappeared after selection binding"),
            "unexpected anchor refusal: {error}"
        );
        println!(
            "{}",
            json!({"old_anchor_write_divergence_detected": true, "error": error})
        );
        return Ok(());
    }
    let output = Path::new(&args[1]);
    std::fs::create_dir(output)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(output, std::fs::Permissions::from_mode(0o700))?;
    }
    let database = output.join("schema3.duckdb");
    let sidecar = output.join("schema3.duckdb.key");
    if args.len() == 4 {
        std::fs::copy(&args[2], &database)?;
        std::fs::copy(&args[3], &sidecar)?;
    } else {
        let conn = Connection::open(&database)?;
        conn.execute_batch(&std::fs::read_to_string(&args[2])?)?;
        conn.execute_batch("CHECKPOINT")?;
        drop(conn);
        let mut key = [0u8; 32];
        for (index, byte) in key.iter_mut().enumerate() {
            *byte = index as u8;
        }
        std::fs::write(
            &sidecar,
            mig::encode_sidecar(mig::SIDECAR_VERSION, &key)
                .map_err(|error| anyhow::anyhow!("{error:?}"))?,
        )?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for path in [&database, &sidecar] {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }
    }
    let before = hash(&database);
    let key_before = hash(&sidecar);
    let bad_sidecar = output.join("tampered-synthetic.key");
    let mut bad_key = std::fs::read(&sidecar)?;
    bad_key[71] ^= 1;
    std::fs::write(&bad_sidecar, bad_key)?;
    let bad_request = mig::MigrateRequest {
        source_database: &database,
        key_sidecar: Some(&bad_sidecar),
        previous_private_root: None,
        stop_at: None,
        expected_source_sha256_hex: Some(&before),
        damage_staged_usage_row: false,
    };
    let bad_result = mig::migrate_closed_snapshot(bad_request);
    anyhow::ensure!(
        matches!(bad_result, Err(mig::MigrationError::Refused(ref refusal)) if refusal.code == "key_mismatch" && !refusal.source_mutated && !refusal.pointer_changed),
        "tampered key was not refused before publication"
    );
    let request = mig::MigrateRequest {
        source_database: &database,
        key_sidecar: Some(&sidecar),
        previous_private_root: None,
        stop_at: None,
        expected_source_sha256_hex: Some(&before),
        damage_staged_usage_row: false,
    };
    #[cfg(windows)]
    let candidate = mig::migrate_closed_snapshot_with_root_choice(
        request,
        &dendro_core::platform_v1::StorageRootChoice::UserProfile,
    )?;
    #[cfg(not(windows))]
    let candidate = mig::migrate_closed_snapshot(request)?;
    anyhow::ensure!(
        before == hash(&database) && key_before == hash(&sidecar),
        "source bundle mutated"
    );
    anyhow::ensure!(
        before == hash(&candidate.preimage_database) && key_before == hash(&candidate.preimage_key),
        "preimage mismatch"
    );
    let schema4_before = hash(&candidate.database_path);
    let pristine = mig::rollback_to_preserved_schema3(&database, &candidate.database_path, false)?;
    anyhow::ensure!(
        !pristine.replay_available
            && pristine.excluded_fact_count == 0
            && pristine.full_continuity_claim
            && !pristine.pointer_changed
            && !pristine.destructive_downmigration,
        "pristine rollback claim mismatch"
    );
    anyhow::ensure!(
        schema4_before == hash(&candidate.database_path),
        "rollback mutated schema4"
    );
    let schema4 = output.join("schema4.duckdb");
    std::fs::copy(&candidate.database_path, &schema4)?;
    for (id, ts, range) in [
        ("v4-A", "2026-10-22 00:00:02", [512, 544]),
        ("v4-B", "2026-10-22 00:00:03", [544, 576]),
    ] {
        mig::record_v4_only_fact(
            &candidate.database_path,
            &mig::V4OnlyFact {
                id: id.into(),
                ts: ts.into(),
                source_id: "src-A".into(),
                generation: 2,
                byte_range_half_open: range,
            },
        )?;
    }
    let dirty_before = hash(&candidate.database_path);
    let loss = mig::rollback_to_preserved_schema3(&database, &candidate.database_path, false)?;
    anyhow::ensure!(
        loss.excluded_fact_count == 2
            && loss.unresolved_loss
            && !loss.full_continuity_claim
            && !loss.replay_available
            && !loss.pointer_changed
            && !loss.destructive_downmigration,
        "loss rollback claim mismatch"
    );
    anyhow::ensure!(
        before == hash(&database)
            && key_before == hash(&sidecar)
            && dirty_before == hash(&candidate.database_path),
        "rollback mutated bundle"
    );
    // Exercise actual consumer selection, not just the staging/receipt helpers.
    let choice = dendro_core::platform_v1::StorageRootChoice::Explicit(output.to_path_buf());
    let selected = dendro_core::selection_binding::migrate_logical_db(&database, &choice)?;
    let selected_migration_database = selected.database.clone();
    let migration_pointer = selected.selection();
    drop(selected);
    let reopened = match dendro_core::selection_binding::resolve_logical_db(&database, &choice)? {
        dendro_core::selection_binding::SelectedDb::Selected(read) => *read,
        _ => anyhow::bail!("migrated selection missing after reopen"),
    };
    anyhow::ensure!(
        reopened.selection().sha256 == migration_pointer.sha256,
        "migration pointer changed across reopen"
    );
    let updated = dendro_core::selection_binding::mutate_selected(&reopened, |staged, _| {
        for (id, ts, range) in [
            ("selected-v4-A", "2026-10-22 00:00:02", [512, 544]),
            ("selected-v4-B", "2026-10-22 00:00:03", [544, 576]),
        ] {
            mig::record_v4_only_fact(
                staged,
                &mig::V4OnlyFact {
                    id: id.into(),
                    ts: ts.into(),
                    source_id: "src-A".into(),
                    generation: 2,
                    byte_range_half_open: range,
                },
            )?;
        }
        Ok(())
    })?;
    drop(updated);
    drop(reopened);
    let (rolled_back, selected_loss) =
        dendro_core::selection_binding::rollback_logical_db(&database, &choice, false)?;
    anyhow::ensure!(
        selected_loss.pointer_changed
            && selected_loss.excluded_fact_count == 2
            && !selected_loss.full_continuity_claim
            && selected_loss.unresolved_loss
            && !selected_loss.replay_available,
        "actual selected rollback loss mismatch"
    );
    let selected_rollback_database = rolled_back.database.clone();
    anyhow::ensure!(
        before == hash(&selected_rollback_database) && key_before == hash(&rolled_back.key),
        "selected rollback did not restore exact pair"
    );
    drop(rolled_back);
    let restart = dendro_core::selection_binding::resolve_logical_db(&database, &choice)?;
    anyhow::ensure!(
        matches!(
            restart,
            dendro_core::selection_binding::SelectedDb::Selected(_)
        ),
        "rollback selection missing after restart"
    );
    anyhow::ensure!(
        before == hash(&database) && key_before == hash(&sidecar),
        "actual publication mutated legacy source"
    );
    println!(
        "{}",
        json!({"schema3": database, "schema4": schema4, "schema3_sha256": before, "sidecar_sha256": key_before, "schema4_sha256": schema4_before, "candidate_selected": candidate.selected, "publication_code": candidate.publication_code, "pristine_rollback": format!("{pristine:?}"), "loss_rollback": format!("{loss:?}"), "private_retained_stage": candidate.database_path, "source_bundle_unchanged": true, "tampered_key_refused": true, "old_source_logs_supplied": false, "selected_migration_database": selected_migration_database, "selected_rollback_database": selected_rollback_database, "selected_pointer_rollback": true, "selected_loss_receipt": selected_loss})
    );
    Ok(())
}
