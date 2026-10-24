//! Native T24-29 proof. The ignored test requires two explicitly provisioned ext4 devices.
#![cfg(any(windows, target_os = "linux"))]
#![deny(unsafe_code)]

use dendro_core::platform_v1::{self as p, ChildName};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Read, Write};
#[cfg(target_os = "linux")]
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::PathBuf;

fn name(value: &str) -> ChildName {
    p::parse_child_name(value, p::current_native_platform()).unwrap()
}

#[test]
fn cleanup_refuses_current_predecessor_and_older_live_reader_without_mutation() {
    use dendro_core::selection_binding as b;
    let root = p::create_user_private_root_with_choice(
        "dendro-cleanup-proof-",
        &p::StorageRootChoice::UserProfile,
    )
    .unwrap();
    let logical = root.path().join("analytics.duckdb");
    let choice = p::StorageRootChoice::Explicit(root.path().to_owned());
    let publish = || {
        b::run_locked_with_selection(&logical, &choice, &[], &[], |db, _| {
            let store = dendro_core::AnalyticsStore::open_staged(db)?;
            store.checkpoint()?;
            Ok(vec![])
        })
        .unwrap();
        match b::resolve_logical_db(&logical, &choice).unwrap() {
            b::SelectedDb::Selected(read) => *read,
            _ => panic!("published database must resolve selected"),
        }
    };
    let first = publish();
    let bytes = fs::read(&first.database).unwrap();
    let key = fs::read(&first.key).unwrap();
    let refusal = b::request_retained_bundle_cleanup(&first).unwrap();
    assert!(refusal.current_manifest_reference && refusal.live_reader_reference);
    assert!(!refusal.disposal_authority_present);
    let second = publish();
    let refusal = b::request_retained_bundle_cleanup(&first).unwrap();
    assert!(!refusal.current_manifest_reference);
    assert!(refusal.immediate_predecessor_reference && refusal.live_reader_reference);
    let third = publish();
    let refusal = b::request_retained_bundle_cleanup(&first).unwrap();
    assert!(!refusal.current_manifest_reference && !refusal.immediate_predecessor_reference);
    assert!(refusal.live_reader_reference && !refusal.disposal_authority_present);
    assert_eq!(fs::read(&first.database).unwrap(), bytes);
    assert_eq!(fs::read(&first.key).unwrap(), key);
    let store = dendro_core::AnalyticsStore::open_read_only_selected(&first).unwrap();
    drop(store);
    assert!(third.is_current().unwrap());
    drop((first, second, third));
    let path = root.path().to_owned();
    drop(root);
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn cleanup_refuses_actual_rollback_selection_and_retains_failed_schema4() {
    use dendro_core::selection_binding as b;
    let root = p::create_user_private_root_with_choice(
        "dendro-cleanup-rollback-",
        &p::StorageRootChoice::UserProfile,
    )
    .unwrap();
    let logical = root.path().join("analytics.duckdb");
    let choice = p::StorageRootChoice::Explicit(root.path().to_owned());
    b::run_locked_with_selection(&logical, &choice, &[], &[], |db, _| {
        let store = dendro_core::AnalyticsStore::open_staged(db)?;
        store.checkpoint()?;
        Ok(vec![])
    })
    .unwrap();
    let migrated = b::migrate_logical_db(&logical, &choice).unwrap();
    let failed_bytes = fs::read(&migrated.database).unwrap();
    let (rollback, loss) = b::rollback_logical_db(&logical, &choice, false).unwrap();
    assert!(loss.pointer_changed && !loss.destructive_downmigration);
    assert!(rollback.custody.file("rollback-loss.json").is_some());
    let bytes = fs::read(&rollback.database).unwrap();
    let refusal = b::request_retained_bundle_cleanup(&rollback).unwrap();
    assert!(refusal.current_manifest_reference && refusal.live_reader_reference);
    let refusal = b::request_retained_bundle_cleanup(&migrated).unwrap();
    assert!(refusal.immediate_predecessor_reference && refusal.live_reader_reference);
    assert_eq!(fs::read(&rollback.database).unwrap(), bytes);
    assert_eq!(fs::read(&migrated.database).unwrap(), failed_bytes);
    assert!(rollback.is_current().unwrap());
    drop((migrated, rollback));
    let path = root.path().to_owned();
    drop(root);
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn actual_duckdb_spill_and_wal_stay_under_pinned_private_parents() {
    use dendro_core::{selection_binding as b, AnalyticsStore};
    let root = p::create_user_private_root_with_choice(
        "dendro-library-privacy-",
        &p::StorageRootChoice::UserProfile,
    )
    .unwrap();
    let logical = root.path().join("analytics.duckdb");
    let choice = p::StorageRootChoice::Explicit(root.path().to_owned());
    b::run_locked_with_selection(&logical, &choice, &[], &[], |database, _| {
        let store = AnalyticsStore::open_staged(database)?;
        store.with_private_connection(|conn| {
            conn.execute_batch("SET memory_limit='16MB'; SET threads=1; SET preserve_insertion_order=false; CREATE TABLE persisted(value INTEGER); INSERT INTO persisted VALUES (47); CREATE TEMP TABLE forced_spill AS SELECT i, repeat(md5(i::VARCHAR), 32) AS payload FROM range(100000) t(i)")?;
            assert!(database.with_extension("duckdb.wal").is_file(), "actual staged writer must create a WAL");
            assert_actual_spill(conn)?;
            assert!(conn.execute_batch("SET temp_directory='/tmp/dendro-outside-spill'").is_err(), "locked production configuration must refuse outside spill");
            assert!(conn.execute_batch("SET extension_directory='/tmp/dendro-outside-extension'").is_err());
            Ok(())
        })?; // native runtime verification checks live WAL and all spill leaves
        store.checkpoint()?;
        Ok(vec![])
    }).unwrap();
    let selected = match b::resolve_logical_db(&logical, &choice).unwrap() {
        b::SelectedDb::Selected(read) => read,
        _ => panic!("selected expected"),
    };
    let store = AnalyticsStore::open_read_only_selected(&selected).unwrap();
    let second = AnalyticsStore::open_read_only_selected(&selected).unwrap();
    second
        .with_private_connection(|conn| {
            assert_eq!(
                conn.query_row("SELECT count(*) FROM persisted", [], |row| row
                    .get::<_, i64>(0))?,
                1
            );
            Ok(())
        })
        .unwrap();
    drop(second);
    store.with_private_connection(|conn| {
        conn.execute_batch("SET memory_limit='16MB'; SET threads=1; SET preserve_insertion_order=false; CREATE TEMP TABLE forced_read_spill AS SELECT i, repeat(md5(i::VARCHAR), 32) AS payload FROM range(100000) t(i)")?;
        assert_actual_spill(conn)?;
        Ok(())
    }).unwrap();
    assert!(
        !selected.database.with_extension("duckdb.wal").exists(),
        "read-only selected reader must not create WAL"
    );
    drop((store, selected));
    let path = root.path().to_owned();
    drop(root);
    fs::remove_dir_all(path).unwrap();
}

fn assert_actual_spill(conn: &duckdb::Connection) -> anyhow::Result<()> {
    let paths = conn
        .prepare("SELECT path FROM duckdb_temporary_files()")?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    assert!(!paths.is_empty(), "must force actual engine spill");
    let configured: String =
        conn.query_row("SELECT current_setting('temp_directory')", [], |row| {
            row.get(0)
        })?;
    let spill_parent = p::open_existing_private_root(&PathBuf::from(&configured))?;
    for path in paths {
        let path = PathBuf::from(path);
        assert_eq!(path.parent().unwrap(), spill_parent.path());
        p::verify_library_leaf(
            &spill_parent,
            &name(path.file_name().unwrap().to_str().unwrap()),
        )?;
    }
    Ok(())
}

#[test]
fn production_staged_opener_refuses_unsafe_wal_and_default_spill_aliases() {
    use dendro_core::AnalyticsStore;
    let root = p::create_user_private_root_with_choice(
        "dendro-library-refusal-",
        &p::StorageRootChoice::UserProfile,
    )
    .unwrap();
    let db = root.path().join("analytics.duckdb");
    let store = AnalyticsStore::open_path(&db, "privacy-test-key").unwrap();
    store.checkpoint().unwrap();
    drop(store);
    let outside = root.create_file(&name("outside")).unwrap();
    let wal = db.with_extension("duckdb.wal");
    fs::hard_link(outside.path(), &wal).unwrap();
    assert!(
        AnalyticsStore::open_staged(&db).is_err(),
        "existing hard-linked WAL must refuse before DuckDB open"
    );
    fs::remove_file(&wal).unwrap();
    #[cfg(target_os = "linux")]
    {
        std::os::unix::fs::symlink(outside.path(), &wal).unwrap();
        assert!(
            AnalyticsStore::open_staged(&db).is_err(),
            "symlink WAL must refuse"
        );
        fs::remove_file(&wal).unwrap();
        root.create_file(&name("analytics.duckdb.wal")).unwrap();
        fs::set_permissions(&wal, fs::Permissions::from_mode(0o666)).unwrap();
        assert!(
            AnalyticsStore::open_staged(&db).is_err(),
            "unsafe existing WAL permissions must refuse without repair"
        );
        assert_eq!(fs::metadata(&wal).unwrap().mode() & 0o777, 0o666);
        fs::remove_file(&wal).unwrap();
        std::os::unix::fs::symlink(root.path(), db.with_extension("duckdb.tmp")).unwrap();
    }
    #[cfg(windows)]
    fs::create_dir(db.with_extension("duckdb.tmp")).unwrap();
    assert!(
        AnalyticsStore::open_staged(&db).is_err(),
        "unadmitted default spill tree must refuse"
    );
    #[cfg(target_os = "linux")]
    fs::remove_file(db.with_extension("duckdb.tmp")).unwrap();
    #[cfg(windows)]
    fs::remove_dir(db.with_extension("duckdb.tmp")).unwrap();
    drop(outside);
    let path = root.path().to_owned();
    drop(root);
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn production_runtime_refuses_injected_live_spill_alias() {
    use dendro_core::AnalyticsStore;
    let root = p::create_user_private_root_with_choice(
        "dendro-spill-refusal-",
        &p::StorageRootChoice::UserProfile,
    )
    .unwrap();
    let db = root.path().join("analytics.duckdb");
    let initial = AnalyticsStore::open_path(&db, "spill-refusal").unwrap();
    initial.checkpoint().unwrap();
    drop(initial);
    let outside = root.create_file(&name("outside")).unwrap();
    let store = AnalyticsStore::open_staged(&db).unwrap();
    let mut injected = None;
    let result = store.with_private_connection(|conn| {
        let temp: String =
            conn.query_row("SELECT current_setting('temp_directory')", [], |row| {
                row.get(0)
            })?;
        let alias = PathBuf::from(temp).join("injected.tmp");
        fs::hard_link(outside.path(), &alias)?;
        injected = Some(alias);
        Ok(())
    });
    assert!(
        result.is_err(),
        "post-operation verification must refuse hard-linked library leaf"
    );
    fs::remove_file(injected.take().unwrap()).unwrap();
    #[cfg(target_os = "linux")]
    {
        let result = store.with_private_connection(|conn| {
            let temp: String =
                conn.query_row("SELECT current_setting('temp_directory')", [], |row| {
                    row.get(0)
                })?;
            let alias = PathBuf::from(temp).join("injected.tmp");
            std::os::unix::fs::symlink(outside.path(), &alias)?;
            injected = Some(alias);
            Ok(())
        });
        assert!(result.is_err(), "live symlink library leaf must refuse");
        fs::remove_file(injected.take().unwrap()).unwrap();
    }
    drop((store, outside));
    let path = root.path().to_owned();
    drop(root);
    fs::remove_dir_all(path).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn native_lock_holder_child() {
    let Some(path) = std::env::var_os("FOUNDATION_LOCK_CHILD") else {
        return;
    };
    let root = p::open_existing_private_root(&PathBuf::from(path)).unwrap();
    let lock = p::open_existing_private_file(&root, &name("writer.lock")).unwrap();
    let identity = p::metadata_from_handle(lock.file()).unwrap().identity;
    let canonical = p::canonical_identity_from_persisted_record(identity);
    p::with_exclusive_lock(
        &root,
        &name("writer.lock"),
        &canonical,
        |_| -> Result<(), p::PlatformError> {
            fs::write(root.path().join("ready"), b"held").unwrap();
            loop {
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
        },
    )
    .unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn native_interprocess_lock_contention_and_crash_release() {
    let root = p::create_user_private_root("dendro-native-lock-").unwrap();
    let canonical = root
        .create_file(&name("writer.lock"))
        .unwrap()
        .canonical_identity()
        .unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "native_lock_holder_child", "--nocapture"])
        .env("FOUNDATION_LOCK_CHILD", root.path())
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while !root.path().join("ready").exists() {
        if child.try_wait().unwrap().is_some() || std::time::Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("lock child failed to become ready");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let busy = p::with_exclusive_lock(&root, &name("writer.lock"), &canonical, |_| Ok(()));
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(
        busy.is_err(),
        "second native process must not acquire held lock"
    );
    p::with_exclusive_lock(&root, &name("writer.lock"), &canonical, |_| Ok(())).unwrap();
    // An exec'd child must not retain the sentinel's flock after its owner exits
    // the closure. Rust's safe opens use close-on-exec; prove the consumer effect.
    let mut descendant = p::with_exclusive_lock(&root, &name("writer.lock"), &canonical, |_| {
        Ok(std::process::Command::new("/bin/sleep")
            .arg("30")
            .spawn()
            .unwrap())
    })
    .unwrap();
    let reacquired = p::with_exclusive_lock(&root, &name("writer.lock"), &canonical, |_| Ok(()));
    descendant.kill().unwrap();
    descendant.wait().unwrap();
    reacquired.unwrap();
    fs::hard_link(root.path().join("writer.lock"), root.path().join("key.bin")).unwrap();
    assert!(
        p::with_exclusive_lock(&root, &name("writer.lock"), &canonical, |_| Ok(())).is_err(),
        "hardlink alias must not admit a lock sentinel"
    );
    let path = root.path().to_owned();
    drop(root);
    fs::remove_dir_all(path).unwrap();
}

#[test]
#[ignore = "T24-29 requires FOUNDATION_CROSS_VOLUME_SOURCE and FOUNDATION_CROSS_VOLUME_DEST; CI explicitly executes this test"]
fn cross_volume_closed_copy_then_local_pointer_publication() {
    let source_path = PathBuf::from(
        std::env::var_os("FOUNDATION_CROSS_VOLUME_SOURCE").expect("required source fixture"),
    );
    let destination_path = PathBuf::from(
        std::env::var_os("FOUNDATION_CROSS_VOLUME_DEST").expect("required destination fixture"),
    );
    let source = p::open_existing_private_root(&source_path).unwrap();
    let destination = p::open_existing_private_root(&destination_path).unwrap();
    fn volume(identity: p::FileIdentity) -> u64 {
        match identity {
            p::FileIdentity::Unix { device, .. } => device,
            p::FileIdentity::WindowsFileId128 {
                volume_serial_64, ..
            } => volume_serial_64,
            p::FileIdentity::WindowsByHandle64 { volume_serial, .. } => volume_serial,
        }
    }
    assert_ne!(
        volume(source.metadata().identity),
        volume(destination.metadata().identity),
        "must execute actual device crossing"
    );
    // Product path: the migration engine itself performs the closed source
    // copy across devices before a destination-local publication. The manual
    // primitive fixture below remains separate evidence for publisher behavior.
    use dendro_core::migration_v4 as m;
    let product_source = p::create_unique_private_directory(&source, "product-source-").unwrap();
    let product_db = product_source.path().join("source.duckdb");
    let product_key = product_source.path().join("source.key");
    let key_bytes = std::array::from_fn(|index| index as u8);
    let store = dendro_core::AnalyticsStore::open_path(
        &product_db,
        &dendro_core::privacy::key_id(&key_bytes),
    )
    .unwrap();
    store.checkpoint().unwrap();
    drop(store);
    let mut key = product_source.create_file(&name("source.key")).unwrap();
    key.file_mut()
        .write_all(&m::encode_sidecar(m::SIDECAR_VERSION, &key_bytes).unwrap())
        .unwrap();
    key.file().sync_all().unwrap();
    drop(key);
    assert!(!product_db.with_extension("duckdb.wal").exists());
    let source_before = Sha256::digest(fs::read(&product_db).unwrap());
    let candidate = m::migrate_closed_snapshot_with_root_choice(
        m::MigrateRequest {
            source_database: &product_db,
            key_sidecar: Some(&product_key),
            previous_private_root: None,
            stop_at: None,
            expected_source_sha256_hex: None,
            damage_staged_usage_row: false,
        },
        &p::StorageRootChoice::Explicit(destination_path.clone()),
    )
    .unwrap();
    assert_eq!(
        volume(candidate.root.metadata().identity),
        volume(destination.metadata().identity)
    );
    let ready = m::validate_publication_candidate(&product_db, &product_key, &candidate).unwrap();
    ready.verify_unchanged().unwrap();
    let mut descriptors = Vec::new();
    for (role, file, handle) in [
        ("database", ready.database(), ready.database_handle()),
        (
            "legacy_database",
            ready.preimage_database(),
            ready.preimage_database_handle(),
        ),
        ("key", ready.key(), ready.key_handle()),
        (
            "migration_identity",
            ready.identity_record(),
            ready.identity_record_handle(),
        ),
        (
            "migration_seal",
            ready.content_seal(),
            ready.content_seal_handle(),
        ),
    ] {
        descriptors.push(p::ClosedBundleFile {
            role: role.into(),
            name: name(file.path().file_name().unwrap().to_str().unwrap()),
            expected_identity: p::metadata_from_handle(handle).unwrap().identity,
            expected_size: file.size(),
            expected_sha256: hex::decode(file.sha256()).unwrap().try_into().unwrap(),
        });
    }
    let product_stage =
        p::create_unique_private_directory(&destination, "product-publish-").unwrap();
    let product_writer = destination
        .create_file(&name("product-writer.lock"))
        .unwrap()
        .canonical_identity()
        .unwrap();
    let product_publisher = destination
        .create_file(&name("product-publisher.lock"))
        .unwrap()
        .canonical_identity()
        .unwrap();
    let product_receipt = p::publish_closed_bundle(&p::PublicationRequest {
        destination: &destination,
        expected_destination: destination.metadata().identity,
        staging: &product_stage,
        expected_staging: product_stage.metadata().identity,
        bundle: &candidate.root,
        expected_bundle: candidate.root.metadata().identity,
        files: &descriptors,
        manifest: name("product-selection.json"),
        expected_current: None,
        writer: p::PublicationSentinel {
            name: name("product-writer.lock"),
            expected: product_writer,
        },
        publisher: p::PublicationSentinel {
            name: name("product-publisher.lock"),
            expected: product_publisher,
        },
    })
    .unwrap();
    let product_selected = p::resolve_selection(
        &destination,
        destination.metadata().identity,
        &name("product-selection.json"),
    )
    .unwrap();
    assert_eq!(
        product_selected.selection.identity,
        product_receipt.manifest_identity
    );
    assert_eq!(
        volume(
            p::metadata_from_handle(product_selected.file("stage.duckdb").unwrap())
                .unwrap()
                .identity
        ),
        volume(destination.metadata().identity)
    );
    let migrated_reader =
        dendro_core::AnalyticsStore::open_read_only(&candidate.database_path).unwrap();
    migrated_reader
        .with_private_connection(|conn| {
            assert_eq!(
                conn.query_row("SELECT schema_version FROM dendro_meta", [], |row| row
                    .get::<_, i32>(0))?,
                4
            );
            Ok(())
        })
        .unwrap();
    drop(migrated_reader);
    assert_eq!(
        Sha256::digest(fs::read(&product_db).unwrap()),
        source_before
    );
    assert!(!product_db.with_extension("duckdb.wal").exists());
    ready.verify_unchanged().unwrap();
    println!("T24-29 product migrate_closed_snapshot copied a closed schema3 source across native volumes; destination-local schema4 pointer resolved and source bytes unchanged");
    let source_db = source_path.join("analytics.duckdb");
    // The database is checkpointed and all writer handles closed before copying.
    let conn = duckdb::Connection::open(&source_db).unwrap();
    conn.execute_batch(
        "CREATE TABLE proof(value INTEGER); INSERT INTO proof VALUES (29); CHECKPOINT",
    )
    .unwrap();
    drop(conn);
    assert!(!source_db.with_extension("duckdb.wal").exists());
    let mut key = source.create_file(&name("key.bin")).unwrap();
    key.file_mut()
        .write_all(b"synthetic-fixture-key-not-a-credential")
        .unwrap();
    key.file().sync_all().unwrap();
    drop(key);
    let bundle = p::create_unique_private_directory(&destination, "bundle-").unwrap();
    let staging = p::create_unique_private_directory(&destination, "staging-").unwrap();
    let mut files = Vec::new();
    let mut before = Vec::new();
    for (role, leaf) in [("database", "analytics.duckdb"), ("key", "key.bin")] {
        let mut input = p::open_existing_private_read_pin(&source, &name(leaf)).unwrap();
        let input_identity = p::metadata_from_handle(&input).unwrap().identity;
        let mut bytes = Vec::new();
        input.read_to_end(&mut bytes).unwrap();
        let digest: [u8; 32] = Sha256::digest(&bytes).into();
        before.push((leaf, digest, input_identity));
        let mut output = bundle.create_file(&name(leaf)).unwrap();
        output.file_mut().write_all(&bytes).unwrap();
        output.file().sync_all().unwrap();
        #[cfg(target_os = "linux")]
        {
            let metadata = output.file().metadata().unwrap();
            assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
            assert_eq!(metadata.nlink(), 1);
            assert_eq!(
                metadata.uid(),
                fs::metadata(&destination_path).unwrap().uid()
            );
        }
        assert_eq!(
            volume(output.metadata().identity),
            volume(destination.metadata().identity)
        );
        files.push(p::ClosedBundleFile {
            role: role.into(),
            name: name(leaf),
            expected_identity: output.metadata().identity,
            expected_size: bytes.len() as u64,
            expected_sha256: digest,
        });
        drop(output);
        assert_eq!(
            <[u8; 32]>::from(Sha256::digest(fs::read(bundle.path().join(leaf)).unwrap())),
            digest
        );
    }
    let writer = destination
        .create_file(&name("writer.lock"))
        .unwrap()
        .canonical_identity()
        .unwrap();
    let publisher = destination
        .create_file(&name("publisher.lock"))
        .unwrap()
        .canonical_identity()
        .unwrap();
    let receipt = p::publish_closed_bundle(&p::PublicationRequest {
        destination: &destination,
        expected_destination: destination.metadata().identity,
        staging: &staging,
        expected_staging: staging.metadata().identity,
        bundle: &bundle,
        expected_bundle: bundle.metadata().identity,
        files: &files,
        manifest: name("selection.json"),
        expected_current: None,
        writer: p::PublicationSentinel {
            name: name("writer.lock"),
            expected: writer,
        },
        publisher: p::PublicationSentinel {
            name: name("publisher.lock"),
            expected: publisher,
        },
    })
    .unwrap();
    let selected = p::resolve_selection(
        &destination,
        destination.metadata().identity,
        &name("selection.json"),
    )
    .unwrap();
    assert_eq!(selected.selection.identity, receipt.manifest_identity);
    assert_eq!(selected.selection.sha256, receipt.manifest_sha256);
    for (leaf, digest, inode) in before {
        let held = p::open_existing_private_read_pin(&source, &name(leaf)).unwrap();
        assert_eq!(p::metadata_from_handle(&held).unwrap().identity, inode);
        assert_eq!(
            <[u8; 32]>::from(Sha256::digest(fs::read(source_path.join(leaf)).unwrap())),
            digest
        );
    }
    let read = duckdb::Connection::open_with_flags(
        bundle.path().join("analytics.duckdb"),
        duckdb::Config::default()
            .access_mode(duckdb::AccessMode::ReadOnly)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        read.query_row("SELECT value FROM proof", [], |row| row.get::<_, i32>(0))
            .unwrap(),
        29
    );
    println!("T24-29 actual distinct-device copy, private destination, unchanged closed source and verified local pointer: passed");
}
