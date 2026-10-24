//! Actual native fixtures for the remaining original T24 filesystem/lock cases.
#![cfg(any(windows, target_os = "linux"))]
#![deny(unsafe_code)]

use dendro_core::platform_v1::{self as p, ChildName};
use std::fs;
#[cfg(windows)]
use std::path::PathBuf;

fn name(value: &str) -> ChildName {
    p::parse_child_name(value, p::current_native_platform()).unwrap()
}
fn fresh_root(prefix: &str) -> p::PrivateRoot {
    p::create_user_private_root_with_choice(prefix, &p::StorageRootChoice::UserProfile).unwrap()
}

#[test]
fn unrelated_fd_close_preserves_lock_and_original_canonical_token_refuses_recreated_inode() {
    let root = fresh_root("dendro-canonical-lock-");
    let token = root
        .create_file(&name("writer.lock"))
        .unwrap()
        .canonical_identity()
        .unwrap();
    drop(root.create_file(&name("replacement.lock")).unwrap());
    let removed = p::with_exclusive_lock(&root, &name("writer.lock"), &token, |_| {
        drop(p::open_existing_private_file(&root, &name("writer.lock"))?);
        assert!(p::os_exclusive_lock_conflicts(&root, &name("writer.lock"))?);
        let removed = fs::remove_file(root.path().join("writer.lock")).is_ok();
        if removed {
            // Windows can keep a delete-pending handle; Linux can unlink an
            // open inode. Neither behavior grants replacement lock authority.
            let _ = fs::rename(
                root.path().join("replacement.lock"),
                root.path().join("writer.lock"),
            );
            assert!(
                p::with_exclusive_lock(&root, &name("writer.lock"), &token, |_| Ok(())).is_err()
            );
        }
        Ok(removed)
    })
    .unwrap();
    if removed {
        if !root.path().join("writer.lock").exists() {
            fs::rename(
                root.path().join("replacement.lock"),
                root.path().join("writer.lock"),
            )
            .unwrap();
        }
        assert!(p::with_exclusive_lock(&root, &name("writer.lock"), &token, |_| Ok(())).is_err());
    }
    let path = root.path().to_owned();
    drop(root);
    fs::remove_dir_all(path).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn actual_symlink_escape_and_cycle_refuse_without_outside_mutation() {
    let root = fresh_root("dendro-symlink-");
    let outside = fresh_root("dendro-symlink-outside-");
    fs::write(outside.path().join("canary"), b"outside unchanged").unwrap();
    std::os::unix::fs::symlink(outside.path(), root.path().join("escape")).unwrap();
    std::os::unix::fs::symlink("cycle-b", root.path().join("cycle-a")).unwrap();
    std::os::unix::fs::symlink("cycle-a", root.path().join("cycle-b")).unwrap();
    for leaf in ["escape", "cycle-a", "cycle-b"] {
        assert!(p::open_existing_private_root(&root.path().join(leaf)).is_err());
        assert!(p::open_existing_private_file(&root, &name(leaf)).is_err());
    }
    assert_eq!(
        fs::read(outside.path().join("canary")).unwrap(),
        b"outside unchanged"
    );
    for leaf in ["escape", "cycle-a", "cycle-b"] {
        fs::remove_file(root.path().join(leaf)).unwrap();
    }
    let paths = [root.path().to_owned(), outside.path().to_owned()];
    drop((root, outside));
    for path in paths {
        fs::remove_dir_all(path).unwrap();
    }
}

#[test]
fn replacement_from_another_thread_cannot_redirect_verified_root_writes() {
    let envelope = fresh_root("dendro-open-race-");
    let target = p::create_unique_private_directory(&envelope, "target-").unwrap();
    let original = target.path().to_owned();
    let parked = envelope.path().join("parked");
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let moved = fs::rename(&original, &parked).is_ok();
        if moved {
            fs::create_dir(&original).unwrap();
        }
        ready_tx.send(moved).unwrap();
        done_rx.recv().unwrap();
        if moved {
            assert!(
                !original.join("payload").exists(),
                "outside replacement received a write under old custody"
            );
            fs::remove_dir(&original).unwrap();
            fs::rename(&parked, &original).unwrap();
        }
    });
    let moved = ready_rx.recv().unwrap();
    let result = target.create_file(&name("payload"));
    let allowed = result.is_ok();
    drop(result);
    done_tx.send(()).unwrap();
    worker.join().unwrap();
    if moved {
        assert!(
            !allowed,
            "stale root must fail after deterministic concurrent replacement"
        );
    } else {
        assert!(
            allowed,
            "a pinned root that denied replacement must stay usable"
        );
    }
    drop(target);
    let path = envelope.path().to_owned();
    drop(envelope);
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn explicitly_admitted_link_resolves_once_and_later_alias_replacement_cannot_redirect() {
    let envelope = fresh_root("dendro-admitted-link-");
    let original = p::create_unique_private_directory(&envelope, "original-").unwrap();
    let alternate = p::create_unique_private_directory(&envelope, "alternate-").unwrap();
    let selected_root = p::create_unique_private_directory(&original, "selected-").unwrap();
    let alias = envelope.path().join("intentional-alias");
    fn create_link(target: &std::path::Path, alias: &std::path::Path) {
        #[cfg(target_os = "linux")]
        std::os::unix::fs::symlink(target, alias).unwrap();
        #[cfg(windows)]
        {
            assert!(std::process::Command::new("cmd.exe")
                .args(["/D", "/C", "mklink", "/J"])
                .arg(alias)
                .arg(target)
                .output()
                .unwrap()
                .status
                .success());
        }
    }
    fn remove_link(alias: &std::path::Path) {
        #[cfg(target_os = "linux")]
        fs::remove_file(alias).unwrap();
        #[cfg(windows)]
        fs::remove_dir(alias).unwrap();
    }
    create_link(original.path(), &alias);
    let lexical_root = alias.join(selected_root.path().file_name().unwrap());
    assert!(
        p::open_existing_private_root(&lexical_root).is_err(),
        "ordinary opener has no link admission authority"
    );
    assert!(p::open_existing_private_root_with_link_policy(
        &lexical_root,
        alternate.metadata().identity
    )
    .is_err());
    let admitted = p::open_existing_private_root_with_link_policy(
        &lexical_root,
        selected_root.metadata().identity,
    )
    .unwrap();
    assert_eq!(
        admitted.metadata().identity,
        selected_root.metadata().identity
    );
    remove_link(&alias);
    create_link(alternate.path(), &alias);
    drop(admitted.create_file(&name("payload")).unwrap());
    assert!(selected_root.path().join("payload").is_file());
    assert!(!alternate.path().join("payload").exists());
    remove_link(&alias);
    drop((admitted, selected_root, original, alternate));
    let path = envelope.path().to_owned();
    drop(envelope);
    fs::remove_dir_all(path).unwrap();
}

#[cfg(windows)]
#[test]
fn windows_native_root_discovery_child() {
    let Some(expected) = std::env::var_os("FOUNDATION_NATIVE_ROOT_EXPECTED") else {
        return;
    };
    let created = p::create_user_private_root("dendro-home-independent-").unwrap();
    assert_eq!(
        created.path().parent().unwrap().canonicalize().unwrap(),
        PathBuf::from(expected).canonicalize().unwrap()
    );
    let path = created.path().to_owned();
    drop(created);
    fs::remove_dir_all(path).unwrap();
}

#[cfg(windows)]
#[test]
fn actual_native_localappdata_discovery_ignores_absent_and_foreign_home() {
    let root = fresh_root("dendro-localappdata-");
    for foreign_home in [None, Some("/foreign/msys/home")] {
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "windows_native_root_discovery_child",
                "--nocapture",
            ])
            .env("LOCALAPPDATA", root.path())
            .env("FOUNDATION_NATIVE_ROOT_EXPECTED", root.path());
        if let Some(home) = foreign_home {
            command.env("HOME", home);
        } else {
            command.env_remove("HOME");
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "actual native discovery failed: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let path = root.path().to_owned();
    drop(root);
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn actual_binding_never_adopts_recreated_canonical_sentinel() {
    use dendro_core::selection_binding as b;
    let root = fresh_root("dendro-canonical-binding-");
    let logical = root.path().join("analytics.duckdb");
    let choice = p::StorageRootChoice::Explicit(root.path().to_owned());
    let run = || {
        b::run_locked_with_selection(&logical, &choice, &[], &[], |db, _| {
            let store = dendro_core::AnalyticsStore::open_staged(db)?;
            store.checkpoint()?;
            Ok(vec![])
        })
    };
    run().unwrap();
    let selected = match b::resolve_logical_db(&logical, &choice).unwrap() {
        b::SelectedDb::Selected(read) => read,
        _ => panic!("selected fixture required"),
    };
    let before = fs::read(&selected.database).unwrap();
    let binding_path = fs::read_dir(root.path())
        .unwrap()
        .map(Result::unwrap)
        .map(|e| e.path())
        .find(|path| {
            path.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("binding-")
        })
        .unwrap();
    let record: serde_json::Value =
        serde_json::from_slice(&fs::read(binding_path).unwrap()).unwrap();
    let writer = name(record["writer"].as_str().unwrap());
    // Create the replacement before unlinking the old inode, so the fixture
    // cannot accidentally reuse the same native identity on a fast filesystem.
    drop(root.create_file(&name("replacement.lock")).unwrap());
    fs::remove_file(root.path().join(writer.as_str())).unwrap();
    fs::rename(
        root.path().join("replacement.lock"),
        root.path().join(writer.as_str()),
    )
    .unwrap();
    assert!(b::resolve_logical_db(&logical, &choice).is_err());
    assert!(
        run().is_err(),
        "production admission must not mint new authority for the replacement inode"
    );
    assert_eq!(fs::read(&selected.database).unwrap(), before);
    drop(selected);
    let path = root.path().to_owned();
    drop(root);
    fs::remove_dir_all(path).unwrap();
}

#[cfg(windows)]
#[test]
fn junction_escape_refuses_without_touching_outside_bytes() {
    let root = fresh_root("dendro-junction-");
    let outside = fresh_root("dendro-junction-outside-");
    fs::write(outside.path().join("canary"), b"unchanged outside").unwrap();
    let junction = root.path().join("escape");
    let result = std::process::Command::new("cmd.exe")
        .args(["/D", "/C", "mklink", "/J"])
        .arg(&junction)
        .arg(outside.path())
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "actual junction fixture creation failed"
    );
    assert!(p::open_existing_private_root(&junction).is_err());
    assert!(p::open_existing_private_file(&root, &name("escape")).is_err());
    assert_eq!(
        fs::read(outside.path().join("canary")).unwrap(),
        b"unchanged outside"
    );
    fs::remove_dir(&junction).unwrap(); // remove only the junction, never recurse through it
    let paths = [root.path().to_owned(), outside.path().to_owned()];
    drop((root, outside));
    for path in paths {
        fs::remove_dir_all(path).unwrap();
    }
}

#[cfg(windows)]
#[test]
fn actual_shortname_alias_cannot_cross_source_output_roles() {
    let root = fresh_root("dendro-shortname-");
    let long = root.path().join("long-source-name-for-alias.bin");
    drop(
        root.create_file(&name("long-source-name-for-alias.bin"))
            .unwrap(),
    );
    let output = std::process::Command::new("powershell.exe").env_remove("PSModulePath")
        .args(["-NoProfile", "-NonInteractive", "-Command", "$ErrorActionPreference='Stop'; (New-Object -ComObject Scripting.FileSystemObject).GetFile($env:FOUNDATION_LONG_FILE).ShortPath"])
        .env("FOUNDATION_LONG_FILE", &long).output().unwrap();
    assert!(output.status.success(), "native shortname discovery failed");
    let short = PathBuf::from(String::from_utf8(output.stdout).unwrap().trim());
    if short == long {
        // No volume setting is changed. No actual alias proof is claimed here.
        assert!(p::open_existing_private_file(&root, &name("unobserved~1.bin")).is_err());
        println!("T24-17 unsupported fixture: 8.3 alias absent; unobserved alias refused; actual alias-positive proof remains unavailable");
    } else {
        let first = fs::File::open(&long).unwrap();
        let second = fs::File::open(&short).unwrap();
        let first = p::metadata_from_handle(&first).unwrap().identity;
        let second = p::metadata_from_handle(&second).unwrap().identity;
        assert_eq!(first, second);
        assert!(!p::cross_role_identity(
            &first,
            p::ProtectedRole::Source,
            &second,
            p::ProtectedRole::Output
        )
        .is_allow());
        println!("T24-17 actual native 8.3 alias identities equal and cross-role admission denied");
    }
    let path = root.path().to_owned();
    drop(root);
    fs::remove_dir_all(path).unwrap();
}

#[cfg(windows)]
#[test]
fn inherited_lock_descendant() {
    if std::env::var_os("FOUNDATION_LOCK_DESCENDANT").is_some() {
        std::thread::sleep(std::time::Duration::from_secs(30));
    }
}

#[cfg(windows)]
#[test]
fn windows_lock_holder_child() {
    let Some(path) = std::env::var_os("FOUNDATION_WINDOWS_LOCK_CHILD") else {
        return;
    };
    let root = p::open_existing_private_root(&PathBuf::from(path)).unwrap();
    let held = p::open_existing_private_file(&root, &name("writer.lock")).unwrap();
    let token = held.canonical_identity().unwrap();
    drop(held);
    p::with_exclusive_lock(
        &root,
        &name("writer.lock"),
        &token,
        |_| -> Result<(), p::PlatformError> {
            let mut descendant = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "inherited_lock_descendant"])
                .env("FOUNDATION_LOCK_DESCENDANT", "yes")
                .env_remove("FOUNDATION_WINDOWS_LOCK_CHILD")
                .stdout(std::process::Stdio::null())
                .spawn()
                .unwrap();
            fs::write(
                root.path().join("descendant.pid"),
                descendant.id().to_string(),
            )
            .unwrap();
            fs::write(root.path().join("ready"), b"locked").unwrap();
            loop {
                let _ = descendant.try_wait().unwrap();
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
        },
    )
    .unwrap();
}

#[cfg(windows)]
#[test]
fn windows_subprocess_crash_releases_lock_and_descendant_cannot_inherit_it() {
    let root = fresh_root("dendro-win-lock-child-");
    let token = root
        .create_file(&name("writer.lock"))
        .unwrap()
        .canonical_identity()
        .unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "windows_lock_holder_child"])
        .env("FOUNDATION_WINDOWS_LOCK_CHILD", root.path())
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while !root.path().join("ready").exists() {
        if child.try_wait().unwrap().is_some() || std::time::Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("native lock child did not become ready");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let busy = p::with_exclusive_lock(&root, &name("writer.lock"), &token, |_| Ok(()));
    child.kill().unwrap();
    child.wait().unwrap();
    let released = p::with_exclusive_lock(&root, &name("writer.lock"), &token, |_| Ok(()));
    let descendant: u32 = fs::read_to_string(root.path().join("descendant.pid"))
        .unwrap()
        .parse()
        .unwrap();
    let _ = std::process::Command::new("taskkill.exe")
        .args(["/F", "/PID", &descendant.to_string()])
        .output()
        .unwrap();
    assert!(busy.is_err());
    released.unwrap();
    let path = root.path().to_owned();
    drop(root);
    fs::remove_dir_all(path).unwrap();
}
