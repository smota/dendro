#![forbid(unsafe_code)]
use dendro_platform_policy::*;

fn private(trustee: Trustee, flags: u8) -> PrivateObjectInput {
    PrivateObjectInput {
        owner: OwnerEvidence::CurrentUser,
        dacl_present: true,
        dacl_protected: true,
        require_protected: true,
        trust: TrustPolicy::FROZEN_HOST,
        aces: vec![AceView {
            trustee,
            mask: 0x1f01ff,
            flags,
        }],
        read_only_attribute: false,
        deny_ace_present: false,
    }
}

#[test]
fn private_grants_and_owner_templates_keep_the_original_boundary() {
    for trustee in [
        Trustee::CurrentUser,
        Trustee::LocalSystem,
        Trustee::BuiltinAdministrators,
    ] {
        assert!(evaluate_private_object(&private(trustee, 0)).is_allow());
    }
    for trustee in [Trustee::CreatorOwner, Trustee::OwnerRights] {
        assert!(evaluate_private_object(&private(trustee, INHERIT_ONLY_ACE)).is_allow());
        assert_eq!(
            evaluate_private_object(&private(trustee, 0)).code,
            "untrusted_allow_ace"
        );
    }
    for trustee in [Trustee::TrustedInstaller, Trustee::Untrusted] {
        for flags in [0, INHERIT_ONLY_ACE] {
            assert_eq!(
                evaluate_private_object(&private(trustee, flags)).code,
                "untrusted_allow_ace"
            );
        }
    }
    let mut input = private(Trustee::Untrusted, 0);
    input.deny_ace_present = true;
    input.read_only_attribute = true;
    assert_eq!(evaluate_private_object(&input).code, "untrusted_allow_ace");
    input = private(Trustee::CurrentUser, 0);
    input.owner = OwnerEvidence::TrustedService;
    assert_eq!(
        evaluate_private_object(&input).code,
        "service_owner_not_private_object"
    );
    input.owner = OwnerEvidence::CurrentUser;
    input.dacl_present = false;
    assert_eq!(
        evaluate_private_object(&input).code,
        "unverifiable_owner_or_control"
    );
}

#[test]
fn trusted_service_stays_ancestor_only_and_identity_keeps_all_bits() {
    let mut ancestor = AncestorInput {
        owner: OwnerEvidence::TrustedService,
        pinned_without_share_delete: true,
        reparse: false,
        reparse_escapes_root: Some(false),
        link_policy_admitted: false,
        dacl_present: true,
        trust: TrustPolicy::FROZEN_HOST,
        aces: vec![AceView {
            trustee: Trustee::TrustedInstaller,
            mask: REPLACEMENT_MASK,
            flags: 0,
        }],
    };
    assert!(evaluate_ancestor(&ancestor).is_allow());
    ancestor.trust = TrustPolicy::OWNER_ONLY;
    assert_eq!(
        evaluate_ancestor(&ancestor).code,
        "service_owner_not_in_policy"
    );
    let id = FileIdentity::WindowsFileId128 {
        volume_serial_64: 0x1866_3be0_663b_bd76,
        file_id: [1; 16],
    };
    let canonical = canonical_identity_from_persisted_record(id);
    assert_eq!(canonical.identity(), id);
    let mut different = [1; 16];
    different[15] = 2;
    assert_ne!(
        canonical,
        canonical_identity_from_persisted_record(FileIdentity::WindowsFileId128 {
            volume_serial_64: 0x1866_3be0_663b_bd76,
            file_id: different,
        })
    );
}
