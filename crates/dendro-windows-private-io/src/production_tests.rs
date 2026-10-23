#![forbid(unsafe_code)]
use super::*;
use dendro_platform_policy::FileIdentity;
use std::{fs::OpenOptions, os::windows::fs::OpenOptionsExt, path::Path};

#[test]
fn typed_staging_identity_refuses_zero_id_and_retains_every_identity_bit() {
    for volume in [0, 0x1866_3be0_663b_bd76, u64::MAX] {
        let error = typed_staging_identity((volume, [0; 16])).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(error.to_string(), "file_id_128_unusable");
    }
    // A nonzero high byte with zero low 64 bits is a usable 128-bit ID.
    for index in 0..16 {
        let mut file_id = [0; 16];
        file_id[index] = 0x80;
        let volume_serial_64 = 0x1866_3be0_663b_bd76;
        let retained = typed_staging_identity((volume_serial_64, file_id)).unwrap();
        assert_eq!(
            retained,
            FileIdentity::WindowsFileId128 {
                volume_serial_64,
                file_id
            }
        );
        assert_ne!(
            retained,
            typed_staging_identity((volume_serial_64 ^ (1 << 63), file_id)).unwrap()
        );
        let mut changed = file_id;
        changed[(index + 1) % 16] = 1;
        assert_ne!(
            retained,
            typed_staging_identity((volume_serial_64, changed)).unwrap()
        );
    }
    eprintln!("evidence_branch=zero128_predicate_complete; zero_ids_refused; every_id_byte_retained; high_volume_bits_retained");
}

#[test]
fn module_created_source_retains_typed_full_identity_through_consuming_publish() -> io::Result<()> {
    let root = bound_platform::fs::create_private_dir(
        Path::new(&std::env::var_os("USERPROFILE").unwrap()),
        ".dendro-production-held-",
    )?;
    let raw = OpenOptions::new()
        .read(true)
        .share_mode(7)
        .custom_flags(0x02200000)
        .open(&root)?;
    let parent = VerifiedDirectory::from_held(&raw)?;
    let name = |value| ChildName::parse(value).unwrap();
    let mut source = parent.create_source(&name("staged.txt"))?;
    let expected = source.identity()?;
    let (volume_serial_64, file_id) = identity(&source.file)?;
    assert_eq!(
        expected,
        FileIdentity::WindowsFileId128 {
            volume_serial_64,
            file_id
        }
    );
    assert_ne!(file_id, [0; 16]);
    source.write_all(b"typed identity")?;
    source.publish_replace(&parent, &name("published.txt"))?;
    let observed = identity(&File::open(root.join("published.txt"))?)?;
    assert_eq!(observed, (volume_serial_64, file_id));
    assert!(!SAME_PRINCIPAL_RESIDUAL.is_empty());
    eprintln!(
        "evidence_branch=production_typed_identity_complete; expected={expected:?}; fixture={}",
        root.display()
    );
    Ok(())
}
