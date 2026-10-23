#![forbid(unsafe_code)]
use super::*;
use std::{
    fs::{self, OpenOptions},
    os::windows::fs::OpenOptionsExt,
    path::Path,
};
fn name(s: &str) -> ChildName {
    ChildName::parse(s).unwrap()
}
fn raw_dir(path: &Path) -> File {
    OpenOptions::new()
        .read(true)
        .share_mode(7)
        .custom_flags(0x02200000)
        .open(path)
        .unwrap()
}
#[test]
fn native_staging_rework() -> Result<(), Box<dyn std::error::Error>> {
    let root = bound_platform::fs::create_private_dir(
        Path::new(&std::env::var_os("USERPROFILE").unwrap()),
        ".dendro-held-rework-",
    )?;
    println!("FIXTURE {}", root.display());
    let raw = raw_dir(&root);
    let parent = VerifiedDirectory::from_held(&raw)?;
    let mut source = parent.create_source(&name("source.txt"))?;
    source.write_all(b"published")?;
    let id = identity(&source.file)?;
    ffi::require_synchronous(&source.file)?;
    ffi::require_noninherited(&source.file)?;
    println!(
        "PASS anchored exclusive create, FileMode synchronous/no-delete-on-close, noninherit flag"
    );
    assert!(parent.create_source(&name("source.txt")).is_err());
    let mut existing = parent.create_source(&name("target.txt"))?;
    existing.write_all(b"keep")?;
    drop(existing);
    let error = source
        .rename_absent(&parent, &name("target.txt"))
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read(root.join("target.txt"))?, b"keep");
    source.publish_replace(&parent, &name("target.txt"))?;
    assert_eq!(identity(&File::open(root.join("target.txt"))?)?, id);
    println!("PASS no-replace collision and consuming replacement publication");
    let mut moved = parent.create_source(&name("moving.txt"))?;
    moved.write_all(b"held")?;
    fs::rename(root.join("moving.txt"), root.join("displaced.txt"))?;
    let replacement = parent.create_source(&name("moving.txt"))?;
    drop(replacement);
    moved.rename_absent(&parent, &name("actual.txt"))?;
    assert_eq!(fs::read(root.join("actual.txt"))?, b"held");
    assert!(root.join("moving.txt").exists());
    println!("PASS same-staging-parent source name substitution moves original only");
    let dest_path = bound_platform::fs::create_private_dir(&root, "dest-")?;
    let dest = VerifiedDirectory::from_held(&raw_dir(&dest_path))?;
    moved.rename_absent(&dest, &name("moved.txt"))?;
    moved.check(false)?;
    println!("PASS same-volume move updates held staging parent membership");
    let mut hijacked = parent.create_source(&name("hijacked.txt"))?;
    fs::rename(root.join("hijacked.txt"), dest_path.join("hijacked.txt"))?;
    assert!(hijacked.rename_absent(&parent, &name("no.txt")).is_err());
    println!("PASS source moved outside verified staging parent refuses mutation");
    let mut pending = parent.create_source(&name("pending.txt"))?;
    pending.set_delete(true)?;
    assert!(pending
        .rename_absent(&parent, &name("bad-pending.txt"))
        .is_err());
    assert!(pending
        .publish_replace(&parent, &name("bad-pending.txt"))
        .is_err());
    assert!(!root.join("bad-pending.txt").exists());
    println!("PASS delete-pending source refuses rename and consuming publish");
    let mut cancel = parent.create_source(&name("cancel.txt"))?;
    cancel.set_delete(true)?;
    cancel.set_delete(false)?;
    cancel.write_all(b"cancelled")?;
    cancel.publish_replace(&parent, &name("cancel-published.txt"))?;
    assert_eq!(fs::read(root.join("cancel-published.txt"))?, b"cancelled");
    println!("PASS verified delete setter/cancel before consuming publish");
    let base = parent.create_source(&name("base.txt"))?;
    drop(base);
    let stream = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .share_mode(7)
        .open(root.join("base.txt:ads"))?;
    let stream_error = verify(&stream, false).unwrap_err();
    assert!(stream_error.to_string().contains("stream"));
    let mut injected = StagingSource {
        file: stream,
        parent: parent.duplicate()?,
    };
    assert!(injected
        .rename_absent(&parent, &name("no-stream.txt"))
        .is_err());
    assert!(injected.set_delete(true).is_err());
    println!("PASS native held ADS rejection, including injected private token (test only)");
    fs::create_dir(root.join("unprotected"))?;
    assert!(VerifiedDirectory::from_held(&raw_dir(&root.join("unprotected"))).is_err());
    assert!(VerifiedDirectory::from_held(&File::open(root.join("base.txt"))?).is_err());
    println!("PASS unverified/unprotected and wrong-kind parents rejected");
    let me = win_custody::current_user_sid_string()?;
    let admin = ffi::fixture_directory(
        &raw,
        &name("admin").0,
        &format!("O:{me}D:P(A;OICI;FA;;;{me})(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)"),
    )?;
    let admin_parent = VerifiedDirectory::from_held(&admin)?;
    let mut admin_source = admin_parent.create_source(&name("admin-source.txt"))?;
    admin_source.write_all(b"trusted host")?;
    admin_source.publish_replace(&admin_parent, &name("admin-published.txt"))?;
    println!("PASS Administrators grant accepted consistently with FROZEN_HOST");
    let conditional = ffi::fixture_directory(
        &raw,
        &name("conditional").0,
        &format!("O:{me}D:P(A;OICI;FA;;;{me})(XA;OICI;FR;;;WD;(Member_of {{SID(S-1-5-32-545)}}))"),
    )?;
    let dacl = win_custody::ObjectSecurityDescriptor::query_kernel_object(&conditional)?.dacl();
    assert!(matches!(
        dacl,
        Err(win_custody::SecurityAclError::Invalid(
            win_custody::AclValidationError::UnsupportedAceType
        ))
    ));
    assert!(VerifiedDirectory::from_held(&conditional).is_err());
    println!("PASS native conditional callback ACE parent rejected by win-custody and verifier");
    let null = ffi::fixture_directory(
        &raw,
        &name("null-dacl").0,
        &format!("O:{me}D:NO_ACCESS_CONTROL"),
    )?;
    assert!(
        win_custody::ObjectSecurityDescriptor::query_kernel_object(&null)?
            .dacl()
            .is_err()
    );
    assert!(VerifiedDirectory::from_held(&null).is_err());
    println!("PASS native null DACL parent rejected");
    for text in [
        "", ".", "..", "x:y", "x/y", "x\\y", "x\0y", "x.", "CON", "con.txt", "com1.txt", "Mixed",
        "é",
    ] {
        assert!(ChildName::parse(text).is_err());
    }
    assert!(ChildName::parse(&"a".repeat(101)).is_err());
    assert!(decode_id(&[0; 23]).is_err());
    assert!(decode_id(&[]).is_err());
    let mut linked = parent.create_source(&name("linked.txt"))?;
    fs::hard_link(root.join("linked.txt"), root.join("second-link.txt"))?;
    assert!(linked
        .rename_absent(&parent, &name("no-linked.txt"))
        .is_err());
    println!("PASS lowercase names, bounded ID decode, multiple-link refusals");
    let anchor_path = bound_platform::fs::create_private_dir(&root, "anchor-")?;
    let anchor = VerifiedDirectory::from_held(&raw_dir(&anchor_path))?;
    fs::rename(&anchor_path, root.join("anchor-moved"))?;
    let decoy = bound_platform::fs::create_private_dir(&root, "decoy-")?;
    fs::rename(decoy, &anchor_path)?;
    let mut anchored = anchor.create_source(&name("anchored.txt"))?;
    anchored.write_all(b"anchored")?;
    assert!(root.join("anchor-moved/anchored.txt").exists());
    assert!(!anchor_path.join("anchored.txt").exists());
    anchored.publish_replace(&parent, &name("anchored-published.txt"))?;
    println!("PASS held-parent anchored create after parent pathname replacement");
    println!("COMPLETE rework round1 native positives/negatives; fixtures retained; not integration acceptance");
    Ok(())
}
