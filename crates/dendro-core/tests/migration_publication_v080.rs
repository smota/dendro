//! Publication handoff tests use synthetic closed snapshots, never owner state.
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use dendro_core::migration_v4 as mig;
use duckdb::Connection;

struct Fixture {
    root: PathBuf,
    db: PathBuf,
    key: PathBuf,
    candidate: mig::StageCandidate,
}

impl Fixture {
    fn build() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "dendro-publish-proof-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&root).unwrap();
        let db = root.join("source.duckdb");
        let key = root.join("source.key");
        let connection = Connection::open(&db).unwrap();
        connection
            .execute_batch(include_str!(
                "../../../.agentflow/oracles/v080/T04/schema3-fixture.sql"
            ))
            .unwrap();
        connection.execute_batch("CHECKPOINT").unwrap();
        drop(connection);
        let bytes = std::array::from_fn(|index| index as u8);
        fs::write(
            &key,
            mig::encode_sidecar(mig::SIDECAR_VERSION, &bytes).unwrap(),
        )
        .unwrap();
        let request = mig::MigrateRequest {
            source_database: &db,
            key_sidecar: Some(&key),
            previous_private_root: None,
            stop_at: None,
            expected_source_sha256_hex: None,
            damage_staged_usage_row: false,
        };
        #[cfg(windows)]
        let candidate = mig::migrate_closed_snapshot_with_root_choice(
            request,
            &dendro_core::platform_v1::StorageRootChoice::UserProfile,
        )
        .unwrap();
        #[cfg(not(windows))]
        let candidate = mig::migrate_closed_snapshot(request).unwrap();
        Self {
            root,
            db,
            key,
            candidate,
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Windows root handles may still be live. Retaining synthetic private
        // output is safer than treating a failed cleanup as a product failure.
        let _ = fs::remove_dir_all(&self.root);
        let stage = self.candidate.root.path();
        if stage
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with(".dendro-t04-"))
        {
            let _ = fs::remove_dir_all(stage);
        }
    }
}

#[test]
fn publication_handoff_recomputes_evidence_and_retains_preimage_linkage() {
    let fixture = Fixture::build();
    let ready =
        mig::validate_publication_candidate(&fixture.db, &fixture.key, &fixture.candidate).unwrap();
    assert_eq!(
        ready.source_database_sha256(),
        mig::sha256_file(&fixture.db).unwrap()
    );
    assert_eq!(
        ready.preimage_database().sha256(),
        ready.source_database_sha256()
    );
    assert_eq!(
        ready.preimage_key().sha256(),
        mig::sha256_file(&fixture.key).unwrap()
    );
    assert_eq!(ready.key().sha256(), ready.preimage_key().sha256());
    assert_eq!(ready.database().path(), fixture.candidate.database_path);
    assert_eq!(
        ready.database().size(),
        fs::metadata(ready.database().path()).unwrap().len()
    );
    assert_eq!(
        ready.projection_sha256(),
        fixture.candidate.projection_sha256
    );
    ready.verify_unchanged().unwrap();
    assert!(!fixture.candidate.selected);
    assert_eq!(
        fixture.candidate.publication_code,
        "atomic_existing_manifest_replace_unproved"
    );
}

#[test]
fn publication_handoff_detects_same_file_edits_after_validation() {
    let fixture = Fixture::build();
    let ready =
        mig::validate_publication_candidate(&fixture.db, &fixture.key, &fixture.candidate).unwrap();
    let source = fs::read(&fixture.db).unwrap();
    let key = fs::read(&fixture.key).unwrap();
    let changed = match Connection::open(ready.database().path()) {
        Ok(connection) => {
            let result = connection.execute_batch(
                "UPDATE dendro_schema4_field_state SET state = 'known'; CHECKPOINT;",
            );
            drop(connection);
            result.is_ok()
        }
        Err(_) => false,
    };
    #[cfg(target_os = "linux")]
    assert!(
        changed,
        "Linux must actually exercise the same-inode tamper"
    );
    if changed {
        assert!(ready.verify_unchanged().is_err());
        assert!(
            mig::validate_publication_candidate(&fixture.db, &fixture.key, &fixture.candidate)
                .is_err()
        );
    } else {
        // A native held handle may prevent the competing write altogether.
        ready.verify_unchanged().unwrap();
    }
    assert_eq!(source, fs::read(&fixture.db).unwrap());
    assert_eq!(key, fs::read(&fixture.key).unwrap());
}

#[test]
fn publication_handoff_does_not_trust_public_candidate_fields_or_foreign_key() {
    let mut fixture = Fixture::build();
    let actual_database = fixture.candidate.root.path().join("stage.duckdb");
    let actual_projection = fixture.candidate.projection_sha256.clone();
    fixture.candidate.database_path = fixture.root.join("invented.duckdb");
    fixture.candidate.key_id = "untrusted".into();
    fixture.candidate.projection_sha256 = "untrusted".into();
    let ready =
        mig::validate_publication_candidate(&fixture.db, &fixture.key, &fixture.candidate).unwrap();
    assert_eq!(ready.database().path(), actual_database);
    assert_eq!(ready.key_id(), "630dcd2966c4336691125448bbb25b4f");
    assert_eq!(ready.projection_sha256(), actual_projection);
    let foreign_key = fixture.root.join("foreign.key");
    fs::write(
        &foreign_key,
        mig::encode_sidecar(mig::SIDECAR_VERSION, &[99; 32]).unwrap(),
    )
    .unwrap();
    assert!(
        mig::validate_publication_candidate(&fixture.db, &foreign_key, &fixture.candidate).is_err()
    );
    ready.verify_unchanged().unwrap();
}

#[test]
fn publication_handoff_refuses_stage_wal_and_seal_tampering() {
    let fixture = Fixture::build();
    let ready =
        mig::validate_publication_candidate(&fixture.db, &fixture.key, &fixture.candidate).unwrap();
    let wal = ready.database().path().with_file_name("stage.duckdb.wal");
    fs::write(&wal, b"synthetic active writer").unwrap();
    assert!(ready.verify_unchanged().is_err());
    assert!(
        mig::validate_publication_candidate(&fixture.db, &fixture.key, &fixture.candidate).is_err()
    );
    fs::remove_file(wal).unwrap();
    ready.verify_unchanged().unwrap();
    drop(ready);
    fs::write(
        fixture.candidate.root.path().join("stage.content.sha256"),
        b"invented seal",
    )
    .unwrap();
    assert!(
        mig::validate_publication_candidate(&fixture.db, &fixture.key, &fixture.candidate).is_err()
    );
}

#[test]
fn publication_handoff_detects_source_edits_and_preimage_replacement() {
    let fixture = Fixture::build();
    let ready =
        mig::validate_publication_candidate(&fixture.db, &fixture.key, &fixture.candidate).unwrap();
    let original_key = fs::read(&fixture.key).unwrap();
    let mut changed_key = original_key.clone();
    changed_key[71] ^= 1;
    fs::write(&fixture.key, changed_key).unwrap();
    assert!(ready.verify_unchanged().is_err());
    fs::write(&fixture.key, original_key).unwrap();
    ready.verify_unchanged().unwrap();
    let temporary = fixture.candidate.root.path().join("replacement.key");
    fs::copy(ready.preimage_key().path(), &temporary).unwrap();
    let swapped = fs::rename(&temporary, ready.preimage_key().path()).is_ok();
    #[cfg(target_os = "linux")]
    assert!(
        swapped,
        "Linux must exercise the same-content inode replacement"
    );
    if swapped {
        assert!(ready.verify_unchanged().is_err());
    } else {
        ready.verify_unchanged().unwrap();
    }
}
