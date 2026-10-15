use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/issue-13-tests")
            .join(format!(
                "cli-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn command() -> Command {
    Command::new(env!("CARGO_BIN_EXE_dendro"))
}
fn error(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn run_help_lists_manifest_flags_and_visible_alias() {
    let output = command().args(["run", "--help"]).output().unwrap();
    assert!(output.status.success(), "{}", error(&output));
    let text = String::from_utf8(output.stdout).unwrap();
    for flag in ["--write-manifest", "manifest-out", "--sealed"] {
        assert!(text.contains(flag), "missing {flag}: {text}");
    }
}

#[test]
fn manifest_write_and_sealed_are_mutually_exclusive_before_any_output() {
    let scratch = Scratch::new();
    let db = scratch.0.join("analytics.duckdb");
    let output = command()
        .args(["run", "--db"])
        .arg(&db)
        .args(["--write-manifest"])
        .arg(scratch.0.join("new.json"))
        .args(["--sealed"])
        .arg(scratch.0.join("old.json"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    let text = error(&output);
    assert!(
        text.contains("--write-manifest") && text.contains("--sealed"),
        "{text}"
    );
    assert!(!db.exists());
    assert!(!scratch.0.join("new.json").exists());
}

#[test]
fn manifest_alias_writes_replayable_cutoff_and_refuses_output_aliases() {
    let scratch = Scratch::new();
    let source = scratch.0.join("claude");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("session.jsonl"),"{\"type\":\"assistant\",\"sessionId\":\"cli-session\",\"timestamp\":\"2026-10-09T01:00:00Z\",\"message\":{\"id\":\"m1\",\"usage\":{\"input_tokens\":5,\"output_tokens\":1}}}\n").unwrap();
    let root = format!("claude={}", source.display());
    let db = scratch.0.join("analytics.duckdb");
    let manifest = scratch.0.join("cutoff.json");
    let output = command()
        .args(["run", "--db"])
        .arg(&db)
        .args(["--root", &root, "--manifest-out"])
        .arg(&manifest)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", error(&output));
    let cutoff: serde_json::Value = serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    assert_eq!(cutoff["files"].as_array().unwrap().len(), 1);
    let before = fs::read(&db).unwrap();
    for alias in [&db, &source] {
        let output = command()
            .args(["run", "--db"])
            .arg(&db)
            .args(["--root", &root, "--write-manifest"])
            .arg(alias)
            .output()
            .unwrap();
        assert!(
            !output.status.success(),
            "accepted alias {}",
            alias.display()
        );
        assert_eq!(fs::read(&db).unwrap(), before);
    }
    let output = command()
        .args(["run", "--db"])
        .arg(&db)
        .args(["--root", &root, "--sealed"])
        .arg(&manifest)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", error(&output));
}

#[test]
fn manifest_paths_refuse_secret_shaped_values_before_creating_outputs() {
    for flag in ["--write-manifest", "--sealed"] {
        let scratch = Scratch::new();
        let source = scratch.0.join("source");
        fs::create_dir(&source).unwrap();
        let db = scratch.0.join("analytics.duckdb");
        let secret_path = scratch
            .0
            .join("ghp_manifestPathCanary12345678901234567890.json");
        let root = format!("claude={}", source.display());
        let output = command()
            .args(["run", "--db"])
            .arg(&db)
            .args(["--root", &root, flag])
            .arg(&secret_path)
            .output()
            .unwrap();
        assert!(
            !output.status.success(),
            "accepted {flag} secret-shaped path"
        );
        assert!(!db.exists());
        assert!(!secret_path.exists());
    }
}
