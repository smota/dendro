#[cfg(not(dendro_platform_v080_harness))]
use dendro_core::platform_v1 as pv;
#[cfg(dendro_platform_v080_harness)]
use t24a_platform_harness::platform_v1 as pv;

use pv::{
    AceView, AncestorInput, NativePlatform, Outcome, OwnerEvidence, PrivateObjectInput, RootQuery,
    SourceRequest, TrustPolicy, Trustee, UnixAncestorInput, UnixObjectInput,
};

fn win() -> NativePlatform {
    NativePlatform::Windows
}

fn ace(trustee: Trustee, mask: u32, flags: u8) -> AceView {
    AceView {
        trustee,
        mask,
        flags,
    }
}

#[test]
fn hostile_child_names_are_refused_without_rewriting() {
    let good = pv::parse_child_name("файл-A", win()).unwrap();
    assert_eq!(good.as_str(), "файл-A");
    assert!(pv::case_distinct_names("a", "A"));
    assert!(
        pv::parse_child_name("a", win()).unwrap().as_str()
            != pv::parse_child_name("A", win()).unwrap().as_str()
    );
    for raw in [
        "",
        ".",
        "..",
        "a/b",
        "a\\b",
        "a\0b",
        "CON",
        "nul.txt",
        "COM1",
        "name.",
        "name ",
        "a:stream",
        "aux.dat",
        "CONIN$",
        "conout$",
        "CONIN$.txt",
        "COM¹",
        "LPT²",
        "COM³.txt",
    ] {
        assert!(pv::parse_child_name(raw, win()).is_err(), "{raw}");
    }
    assert!(pv::parse_child_name("COM0", win()).is_ok());
    assert!(pv::parse_child_name("commit.txt", win()).is_ok());
    assert!(pv::parse_child_name("LPT0", win()).is_ok());
    let surrogate = std::ffi::OsString::from("ok");
    assert!(pv::parse_child_os(&surrogate, win()).is_ok());
}

#[test]
fn private_ace_policy_rejects_foreign_grants_and_owner_mismatch() {
    let trust = TrustPolicy::FROZEN_HOST;
    let allow = pv::evaluate_private_object(&PrivateObjectInput {
        owner: OwnerEvidence::CurrentUser,
        dacl_present: true,
        dacl_protected: true,
        require_protected: true,
        trust,
        aces: vec![
            ace(Trustee::CurrentUser, 0x1f01ff, 3),
            ace(Trustee::LocalSystem, 0x1f01ff, 3),
        ],
        read_only_attribute: true,
        deny_ace_present: true,
    });
    assert_eq!(allow.outcome, Outcome::Allow);

    let with_admin = pv::evaluate_private_object(&PrivateObjectInput {
        aces: vec![
            ace(Trustee::CurrentUser, 0x1f01ff, 3),
            ace(Trustee::LocalSystem, 0x1f01ff, 3),
            ace(Trustee::BuiltinAdministrators, 0x1f01ff, 3),
        ],
        ..private_base(trust)
    });
    assert_eq!(with_admin.outcome, Outcome::Allow);

    let foreign = pv::evaluate_private_object(&PrivateObjectInput {
        aces: vec![
            ace(Trustee::CurrentUser, 0x1f01ff, 0),
            ace(Trustee::Untrusted, 0x1, 0),
        ],
        deny_ace_present: true,
        read_only_attribute: true,
        require_protected: false,
        dacl_protected: false,
        ..private_base(trust)
    });
    assert_eq!(foreign.code, "untrusted_allow_ace");

    let placeholder = pv::evaluate_private_object(&PrivateObjectInput {
        aces: vec![
            ace(Trustee::CurrentUser, 0x1f01ff, 0),
            ace(Trustee::CreatorOwner, 0x1f01ff, pv::INHERIT_ONLY_ACE),
        ],
        require_protected: false,
        dacl_protected: false,
        ..private_base(trust)
    });
    assert_eq!(placeholder.outcome, Outcome::Allow);
    assert_eq!(
        placeholder.note,
        Some("protected_flag_not_required_on_file")
    );

    assert_eq!(
        pv::evaluate_private_object(&PrivateObjectInput {
            owner: OwnerEvidence::Foreign,
            ..private_base(trust)
        })
        .code,
        "foreign_owner"
    );
    assert_eq!(
        pv::evaluate_private_object(&PrivateObjectInput {
            owner: OwnerEvidence::TrustedService,
            ..private_base(trust)
        })
        .code,
        "service_owner_not_private_object"
    );
    assert_eq!(
        pv::evaluate_private_object(&PrivateObjectInput {
            owner: OwnerEvidence::Unavailable,
            ..private_base(trust)
        })
        .outcome,
        Outcome::Refuse
    );
    assert_eq!(
        pv::evaluate_private_object(&PrivateObjectInput {
            owner: OwnerEvidence::BuiltinAdministrators,
            trust: TrustPolicy::OWNER_ONLY,
            ..private_base(TrustPolicy::OWNER_ONLY)
        })
        .code,
        "admin_owner_not_in_policy"
    );
}

fn private_base(trust: TrustPolicy) -> PrivateObjectInput {
    PrivateObjectInput {
        owner: OwnerEvidence::CurrentUser,
        dacl_present: true,
        dacl_protected: true,
        require_protected: true,
        trust,
        aces: vec![ace(Trustee::CurrentUser, 0x1f01ff, 3)],
        read_only_attribute: false,
        deny_ace_present: false,
    }
}

#[test]
fn ancestor_rights_are_not_private_object_rights() {
    let trust = TrustPolicy::FROZEN_HOST;
    let readable = AncestorInput {
        owner: OwnerEvidence::CurrentUser,
        pinned_without_share_delete: true,
        reparse: false,
        reparse_escapes_root: Some(false),
        link_policy_admitted: false,
        dacl_present: true,
        aces: vec![
            ace(Trustee::Untrusted, pv::CREATE_OR_READ_MASK, 3),
            ace(Trustee::CurrentUser, 0x1f01ff, 0),
        ],
        trust,
    };
    assert_eq!(pv::evaluate_ancestor(&readable).outcome, Outcome::Allow);
    let foreign_owner = AncestorInput {
        owner: OwnerEvidence::Foreign,
        ..readable.clone()
    };
    assert_eq!(
        pv::evaluate_ancestor(&foreign_owner).code,
        "untrusted_ancestor_owner"
    );
    let service = AncestorInput {
        owner: OwnerEvidence::TrustedService,
        ..readable.clone()
    };
    assert_eq!(pv::evaluate_ancestor(&service).outcome, Outcome::Allow);
    let service_closed = AncestorInput {
        owner: OwnerEvidence::TrustedService,
        trust: TrustPolicy::OWNER_ONLY,
        ..readable.clone()
    };
    assert_eq!(
        pv::evaluate_ancestor(&service_closed).code,
        "service_owner_not_in_policy"
    );
    let service_grant = AncestorInput {
        aces: vec![ace(Trustee::TrustedInstaller, pv::REPLACEMENT_MASK, 0)],
        ..readable.clone()
    };
    assert!(pv::evaluate_ancestor(&service_grant).is_allow());
    assert_eq!(
        pv::evaluate_ancestor(&AncestorInput {
            trust: TrustPolicy::OWNER_ONLY,
            ..service_grant
        })
        .code,
        "untrusted_ancestor_replacement"
    );
    assert_eq!(
        pv::evaluate_private_object(&PrivateObjectInput {
            aces: vec![ace(Trustee::TrustedInstaller, 0x1f01ff, 0)],
            ..private_base(TrustPolicy::FROZEN_HOST)
        })
        .code,
        "untrusted_allow_ace"
    );
    let generic = AncestorInput {
        aces: vec![ace(Trustee::Untrusted, 0x1000_0000, 0)],
        ..readable.clone()
    };
    assert_eq!(
        pv::evaluate_ancestor(&generic).code,
        "untrusted_ancestor_replacement"
    );
    let generic_write = AncestorInput {
        aces: vec![ace(Trustee::Untrusted, 0x4000_0000, 0)],
        ..readable.clone()
    };
    assert_eq!(
        pv::evaluate_ancestor(&generic_write).code,
        "untrusted_ancestor_replacement"
    );
    let deletable = AncestorInput {
        aces: vec![ace(Trustee::Untrusted, pv::REPLACEMENT_MASK, 0)],
        ..readable.clone()
    };
    assert_eq!(
        pv::evaluate_ancestor(&deletable).code,
        "untrusted_ancestor_replacement"
    );
    let unpinned = AncestorInput {
        pinned_without_share_delete: false,
        ..readable.clone()
    };
    assert_eq!(pv::evaluate_ancestor(&unpinned).code, "ancestor_pin_absent");
    let escape = AncestorInput {
        reparse: true,
        reparse_escapes_root: Some(true),
        link_policy_admitted: true,
        ..readable.clone()
    };
    assert_eq!(pv::evaluate_ancestor(&escape).code, "reparse_escapes_root");
    let in_root_link = AncestorInput {
        reparse: true,
        reparse_escapes_root: Some(false),
        link_policy_admitted: false,
        pinned_without_share_delete: true,
        ..readable
    };
    assert_eq!(
        pv::evaluate_ancestor(&in_root_link).outcome,
        Outcome::Refuse
    );
}

#[test]
fn identity_links_namespace_and_roots() {
    let id = pv::FileIdentity::WindowsByHandle64 {
        volume_serial: 7,
        file_index: 9,
    };
    assert_eq!(
        pv::cross_role_identity(
            &id,
            pv::ProtectedRole::Source,
            &id,
            pv::ProtectedRole::Output
        )
        .code,
        "same_object_cross_role"
    );
    assert_eq!(
        pv::private_regular_links(pv::ObjectKind::RegularFile, 2).code,
        "multiple_hardlinks"
    );
    assert!(pv::private_regular_links(pv::ObjectKind::Directory, 4).is_allow());
    assert_eq!(
        pv::syntax_admission(pv::classify_windows_path(r"\\?\C:\Users\имя")).outcome,
        Outcome::Allow
    );
    assert_eq!(
        pv::classify_windows_path(r"\\?\UNC\server\share"),
        pv::PathClass::Unc
    );
    assert_eq!(
        pv::classify_windows_path(r"\\.\PIPE\x"),
        pv::PathClass::Device
    );
    assert_eq!(
        pv::classify_windows_path(r"\\?\GLOBALROOT\device"),
        pv::PathClass::Device
    );
    assert_eq!(
        pv::classify_windows_path(r"\\?\Volume{abc}\"),
        pv::PathClass::VolumeGuid
    );
    assert_eq!(
        pv::classify_windows_path(r"C:\data\file:stream"),
        pv::PathClass::AlternateStream
    );
    assert_eq!(pv::letter_is_not_volume_proof().outcome, Outcome::Refuse);
    assert_eq!(
        pv::parse_proc_status_euid("Name:\tx\nUid:\t1\t1000\t1\t1\n").unwrap(),
        1000
    );
    assert_eq!(pv::parse_proc_status_euid("Uid:\t0\t0\t0\t0\n").unwrap(), 0);
    assert_eq!(
        pv::parse_proc_status_euid("Name:\tonly\n")
            .unwrap_err()
            .code,
        "euid_unknown"
    );
    assert_eq!(
        pv::parse_proc_status_euid("Uid:\t1\n").unwrap_err().code,
        "euid_malformed"
    );
    assert_eq!(
        pv::parse_proc_status_euid("Uid:\t1\tabc\t1\t1\n")
            .unwrap_err()
            .code,
        "euid_malformed"
    );
    assert_eq!(pv::filetime_to_unix_ns(116_444_736_000_000_000).unwrap(), 0);
    assert_eq!(
        pv::filetime_to_unix_ns(116_444_736_000_000_001).unwrap(),
        100
    );

    // These root queries use native Windows Path semantics.
    #[cfg(windows)]
    {
        let home_ignored = pv::resolve_root(&RootQuery {
            home: Some("/c/Users/foreign".into()),
            userprofile: None,
            local_app_data: Some(r"C:\Users\me\AppData\Local".into()),
            xdg_data_home: None,
            explicit_override: None,
            euid_is_root: false,
            platform: win(),
        })
        .unwrap();
        assert_eq!(home_ignored.source, "local_app_data");
        assert!(pv::resolve_root(&RootQuery {
            home: Some(".".into()),
            userprofile: Some("relative".into()),
            local_app_data: None,
            xdg_data_home: None,
            explicit_override: None,
            euid_is_root: false,
            platform: win(),
        })
        .is_err());
    }
    assert_eq!(
        pv::resolve_root(&RootQuery {
            home: Some("/home/other".into()),
            userprofile: None,
            local_app_data: None,
            xdg_data_home: None,
            explicit_override: None,
            euid_is_root: true,
            platform: NativePlatform::Linux,
        })
        .unwrap_err()
        .code,
        "privileged_foreign_home"
    );
    let xdg_default = pv::resolve_root(&RootQuery {
        home: Some("/home/me".into()),
        userprofile: None,
        local_app_data: None,
        xdg_data_home: Some("relative/ignored".into()),
        explicit_override: None,
        euid_is_root: false,
        platform: NativePlatform::Linux,
    })
    .unwrap();
    assert_eq!(xdg_default.path, "/home/me/.local/share");
    assert_eq!(xdg_default.source, "xdg_data_home_default");
    let xdg_absolute = pv::resolve_root(&RootQuery {
        home: Some("/home/me".into()),
        userprofile: None,
        local_app_data: None,
        xdg_data_home: Some("/var/lib/dendro-data".into()),
        explicit_override: None,
        euid_is_root: false,
        platform: NativePlatform::Linux,
    })
    .unwrap();
    assert_eq!(xdg_absolute.path, "/var/lib/dendro-data");
    assert_eq!(xdg_absolute.source, "xdg_data_home");
    let root_own = pv::resolve_root(&RootQuery {
        home: Some("/root".into()),
        userprofile: None,
        local_app_data: None,
        xdg_data_home: None,
        explicit_override: None,
        euid_is_root: true,
        platform: NativePlatform::Linux,
    })
    .unwrap();
    assert_eq!(root_own.path, "/root/.local/share");
    assert_eq!(root_own.source, "root_own_home");
    let root_ignores_xdg = pv::resolve_root(&RootQuery {
        home: Some("/root".into()),
        userprofile: None,
        local_app_data: None,
        xdg_data_home: Some("/var/lib/not-used".into()),
        explicit_override: None,
        euid_is_root: true,
        platform: NativePlatform::Linux,
    })
    .unwrap();
    assert_eq!(root_ignores_xdg.path, "/root/.local/share");
    assert!(pv::broad_temp_rejected(std::path::Path::new("/tmp/")));
    assert!(pv::broad_temp_rejected(std::path::Path::new("/tmp/.")));
    assert!(!pv::broad_temp_rejected(std::path::Path::new("/home/sam")));
    let unix_ok = pv::evaluate_unix_private(UnixObjectInput {
        uid: 1000,
        current_euid: 1000,
        mode: 0o600,
        kind: pv::ObjectKind::RegularFile,
        extended_acl_foreign: Some(false),
        admit_library_0644: false,
        parent_effective_private: true,
        privileged_root_trust: false,
    });
    assert!(unix_ok.is_allow());
    assert_eq!(
        pv::evaluate_unix_private(UnixObjectInput {
            mode: 0o644,
            extended_acl_foreign: Some(false),
            ..unix_leaf()
        })
        .code,
        "unix_mode_policy"
    );
    assert_eq!(
        pv::evaluate_unix_ancestor(UnixAncestorInput {
            owner_uid: 0,
            current_euid: 1000,
            mode: 0o755,
            pinned: true
        })
        .outcome,
        Outcome::Allow
    );
    assert_eq!(
        pv::evaluate_unix_ancestor(UnixAncestorInput {
            owner_uid: 0,
            current_euid: 1000,
            mode: 0o777,
            pinned: true
        })
        .outcome,
        Outcome::Refuse
    );
    assert_eq!(
        pv::evaluate_unix_ancestor(UnixAncestorInput {
            owner_uid: 0,
            current_euid: 1000,
            mode: 0o1777,
            pinned: true
        })
        .outcome,
        Outcome::Allow
    );
}

fn unix_leaf() -> UnixObjectInput {
    UnixObjectInput {
        uid: 1000,
        current_euid: 1000,
        mode: 0o600,
        kind: pv::ObjectKind::RegularFile,
        extended_acl_foreign: Some(false),
        admit_library_0644: false,
        parent_effective_private: true,
        privileged_root_trust: false,
    }
}

#[test]
fn publication_lock_and_source_policy_leave_atomic_replace_open() {
    assert_eq!(
        pv::source_request(SourceRequest::ReadOnlyProtocol).outcome,
        Outcome::Allow
    );
    assert_eq!(
        pv::source_request(SourceRequest::CreateMissingSidecar).outcome,
        Outcome::Deny
    );
    assert_eq!(
        pv::source_request(SourceRequest::BlindCopyLiveDuckDb).outcome,
        Outcome::Refuse
    );
    assert_eq!(
        pv::cancellation(true, false, true).outcome,
        Outcome::Cancelled
    );
    assert_eq!(
        pv::cancellation(true, false, false).outcome,
        Outcome::Refuse
    );
    assert_eq!(pv::cancellation(false, false, true).outcome, Outcome::Deny);
    assert_eq!(
        pv::pointer_kind_regular(false).code,
        "pointer_not_regular_file"
    );
    assert_eq!(pv::crash_before_promotion().outcome, Outcome::Refuse);
    assert_eq!(pv::snapshot_retry_exhausted().outcome, Outcome::Refuse);
    assert_eq!(
        pv::retain_referenced_snapshot(true, false).outcome,
        Outcome::Deny
    );
    assert_eq!(
        pv::native_lane(NativePlatform::Windows, false, true).code,
        "native_lane_not_satisfied"
    );
    assert_eq!(
        pv::scoped_artifact_handoff(true, false).note,
        Some("not_full_t24_acceptance")
    );
    assert!(pv::plan_cross_volume_then_local_pointer(true, false).is_allow());
    assert_eq!(
        pv::plan_cross_volume_then_local_pointer(true, true).outcome,
        Outcome::Deny
    );
    let err = pv::publish_selection_manifest().unwrap_err();
    match err {
        pv::PlatformError::Rejected(verdict) => {
            assert_eq!(verdict.code, "atomic_existing_manifest_replace_unproved");
        }
        other => panic!("{other}"),
    }
    for id in [
        "T24-11", "T24-18", "T24-23", "T24-25", "T24-26", "T24-27", "T24-29", "T24-30", "T24-32",
        "T24-38", "T24-39", "T24-41", "T24-46", "T24-48", "T24-49",
    ] {
        assert!(pv::incomplete_oracle_cases().contains(&id), "{id}");
    }
    assert_eq!(
        pv::check_private_io_admission(NativePlatform::Linux),
        pv::PrivateIoScope::CapabilityCompiled
    );
    assert_eq!(
        pv::check_private_io_admission(NativePlatform::Macos),
        pv::PrivateIoScope::NotInThisSlice
    );
}

#[test]
#[cfg(windows)]
fn native_private_root_under_profile_not_temp() {
    let root = pv::create_user_private_root_with_choice(
        ".dendro-t24a-integration-",
        &pv::StorageRootChoice::UserProfile,
    )
    .expect("explicit USERPROFILE synthetic root must exercise positive branch");
    let root_path = root.path().to_path_buf();
    let cleanup = root_path.clone();
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let temp = std::env::temp_dir();
        assert!(!root_path.starts_with(&temp), "{}", root_path.display());
        assert!(!pv::broad_temp_rejected(root_path.parent().unwrap()));
        assert_eq!(root.metadata().kind, pv::ObjectKind::Directory);
        assert!(!root.metadata().reparse);
        assert!(root
            .limitations()
            .contains(&pv::WINDOWS_IDENTITY_LIMITATION));
        assert!(root.limitations().contains(&pv::ACE_INHERITANCE_LIMITATION));
        assert!(matches!(
            root.metadata().identity,
            pv::FileIdentity::WindowsFileId128 { .. }
        ));
        let opened_root =
            pv::open_existing_private_root(&root_path).expect("public open existing root");
        assert_eq!(opened_root.metadata().identity, root.metadata().identity);
        let name = pv::parse_child_name("held-file.txt", win()).unwrap();
        let mut created = root.create_file(&name).expect("private file");
        assert_eq!(created.metadata().links, 1);
        assert_eq!(created.metadata().kind, pv::ObjectKind::RegularFile);
        std::io::Write::write_all(created.file_mut(), b"kept").unwrap();
        created.file_mut().sync_all().unwrap();
        let held = pv::metadata_from_handle(created.file()).unwrap();
        let reopened = pv::reopen_metadata(created.path()).unwrap();
        assert_eq!(held.identity, reopened.identity);
        assert_eq!(held.identity, created.metadata().identity);
        match root.create_file(&name) {
            Err(pv::PlatformError::Rejected(verdict)) => {
                assert_eq!(verdict.code, "exclusive_create_exists")
            }
            other => panic!("duplicate create: {other:?}"),
        }
        assert_eq!(std::fs::read(created.path()).unwrap(), b"kept");
        let token = created.canonical_identity().unwrap();
        drop(created);
        let opened_file =
            pv::open_existing_private_file(&opened_root, &name).expect("public open existing file");
        assert_eq!(opened_file.canonical_identity().unwrap(), token);
        drop(opened_file);
        let sentinel = pv::parse_child_name("lock-sentinel.txt", win()).unwrap();
        let sentinel_file = root.create_file(&sentinel).unwrap();
        let sentinel_token = sentinel_file.canonical_identity().unwrap();
        let sentinel_path = sentinel_file.path().to_path_buf();
        drop(sentinel_file);
        let pv::FileIdentity::WindowsFileId128 {
            volume_serial_64,
            mut file_id,
        } = sentinel_token.identity()
        else {
            panic!("Windows canonical identity must retain 128 bits");
        };
        file_id[15] ^= 0x80;
        let stale =
            pv::canonical_identity_from_persisted_record(pv::FileIdentity::WindowsFileId128 {
                volume_serial_64,
                file_id,
            });
        match pv::with_exclusive_lock::<()>(&root, &sentinel, &stale, |_| {
            panic!("wrong high ID bits entered lock body")
        }) {
            Err(pv::PlatformError::Rejected(verdict)) => {
                assert_eq!(verdict.code, "lock_identity_changed")
            }
            other => panic!("wrong high ID bits must refuse: {other:?}"),
        }
        pv::with_exclusive_lock(&root, &sentinel, &sentinel_token, |_handle| {
            let extra = std::fs::File::open(&sentinel_path).expect("unrelated fd");
            drop(extra);
            assert!(pv::os_exclusive_lock_conflicts(&root, &sentinel).unwrap());
            match pv::with_exclusive_lock(&root, &sentinel, &sentinel_token, |_| Ok(())) {
                Err(pv::PlatformError::Rejected(verdict)) => {
                    assert_eq!(verdict.code, "same_process_lock_held")
                }
                other => panic!("second lock: {other:?}"),
            }
            Ok(())
        })
        .unwrap();
        assert!(!pv::os_exclusive_lock_conflicts(&root, &sentinel).unwrap());
        let renamed = pv::parse_child_name("renamed.txt", win()).unwrap();
        match pv::rename_absent_same_directory(&root, &token, &name, &renamed) {
            Err(pv::PlatformError::Rejected(verdict)) => {
                assert_eq!(verdict.code, "held_source_rename_unproved");
                assert_eq!(verdict.note, Some("safe_held_source_rename_unresolved_no_setter_in_windows_namespace_request_sys_0_2_1"));
            }
            other => panic!("Windows rename must remain typed refusal: {other:?}"),
        }
        assert_eq!(
            std::fs::read(root_path.join(name.as_str())).unwrap(),
            b"kept"
        );
        assert!(!root_path.join(renamed.as_str()).exists());
        eprintln!("evidence_branch=windows_positive_complete; explicit_userprofile; create_root; open_root; create_file; open_file; metadata_identity128; duplicate_create_refused; os_lock_conflict; same_process_lock_refused; unlock; rename_refused_source_unchanged");
    }));
    drop(root);
    let _ = std::fs::remove_dir_all(&cleanup);
    if let Err(err) = outcome {
        std::panic::resume_unwind(err);
    }
}

#[test]
#[cfg(target_os = "linux")]
fn linux_public_canonical_contracts_development_check() {
    let root = pv::create_user_private_root(".dendro-t24a-public-").expect("create synthetic root");
    let path = root.path().to_path_buf();
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let reopened = pv::open_existing_private_root(&path).expect("public open root");
        assert_eq!(reopened.metadata().identity, root.metadata().identity);
        let child = |name: &str| pv::parse_child_name(name, NativePlatform::Linux).unwrap();
        let source = child("source");
        let file = root.create_file(&source).expect("create source");
        let token = file.canonical_identity().unwrap();
        drop(file);
        let opened = pv::open_existing_private_file(&reopened, &source).expect("public open file");
        assert_eq!(opened.canonical_identity().unwrap(), token);
        drop(opened);
        pv::with_exclusive_lock(&root, &source, &token, |_| {
            assert!(pv::os_exclusive_lock_conflicts(&root, &source)?);
            Ok(())
        })
        .expect("public canonical lock");
        let destination = child("destination");
        let renamed = pv::rename_absent_same_directory(&root, &token, &source, &destination)
            .expect("public canonical rename");
        assert_eq!(
            renamed.residual_same_principal_race,
            pv::RENAME_RESIDUAL_SAME_PRINCIPAL
        );
        assert_eq!(
            pv::reopen_metadata(&path.join(destination.as_str()))
                .unwrap()
                .identity,
            token.identity()
        );
        eprintln!("evidence_branch=linux_public_positive_complete; create; public_open_root; public_open_file; canonical_lock; canonical_rename_with_residual");
    }));
    drop(root);
    std::fs::remove_dir_all(&path).expect("synthetic fixture cleanup");
    if let Err(err) = outcome {
        std::panic::resume_unwind(err);
    }
}
