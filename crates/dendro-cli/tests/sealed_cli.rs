use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        #[cfg(any(windows, target_os = "linux"))]
        {
            let prefix = format!(
                "dendro-cli-test-{}-{}-",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            );
            let root = dendro_core::platform_v1::create_user_private_root_with_choice(
                &prefix,
                &dendro_core::platform_v1::StorageRootChoice::UserProfile,
            )
            .unwrap();
            Self(root.path().to_path_buf())
        }
        #[cfg(not(any(windows, target_os = "linux")))]
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/issue-13-tests")
            .join(format!(
                "cli-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        #[cfg(not(any(windows, target_os = "linux")))]
        {
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
struct RemoveDir(PathBuf);
impl Drop for RemoveDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
// A deliberately non-private directory: a restrictive umask (077 on CI) would otherwise
// make it an admitted private root and defeat the refusal under test.
fn ordinary_dir(path: &Path) {
    fs::create_dir(path).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

fn command() -> Command {
    Command::new(env!("CARGO_BIN_EXE_dendro"))
}
fn error(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn selected_bytes(db: &Path) -> Vec<u8> {
    let request = dendro_core::presentation::ReadRequest::open(db).unwrap();
    fs::read(request.database()).unwrap()
}

#[cfg(any(windows, target_os = "linux"))]
fn selected_snapshot(db: &Path) -> (dendro_core::presentation::ReadRequest, Vec<u8>, Vec<u8>) {
    let request = dendro_core::presentation::ReadRequest::open(db).unwrap();
    assert!(
        request.selected().is_some(),
        "Windows run must publish an immutable selection"
    );
    let bytes = fs::read(request.database()).unwrap();
    let key = fs::read(&request.selected().unwrap().key).unwrap();
    (request, bytes, key)
}

#[test]
fn top_level_help_lists_quickstart_shortcuts_and_report_kinds() {
    let output = command().arg("--help").output().unwrap();
    assert!(output.status.success(), "{}", error(&output));
    let text = String::from_utf8(output.stdout).unwrap();
    for needle in [
        "Quickstart Examples",
        "Interactive TUI Shortcuts",
        "Report Kinds",
        "links, activity, handoffs, tokens, decisions",
        "status",
        "dashboard",
        "simulate",
    ] {
        assert!(text.contains(needle), "missing {needle}: {text}");
    }
    let status_help = command().args(["status", "--help"]).output().unwrap();
    assert!(status_help.status.success(), "{}", error(&status_help));
    let status_text = String::from_utf8(status_help.stdout).unwrap();
    assert!(status_text.contains("Database & Scope"), "{status_text}");
    let report_help = command().args(["report", "--help"]).output().unwrap();
    assert!(report_help.status.success(), "{}", error(&report_help));
    let report_text = String::from_utf8(report_help.stdout).unwrap();
    assert!(report_text.contains("Friction Thresholds"), "{report_text}");
    assert!(
        report_text.contains("Advisory & Spotlights"),
        "{report_text}"
    );
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
    let before = selected_bytes(&db);
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
        assert_eq!(selected_bytes(&db), before);
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

#[test]
fn status_dashboard_and_auto_materializing_ask_work_without_ceremony() {
    let scratch = Scratch::new();
    let db = scratch.0.join("analytics.duckdb");

    let missing_status = command()
        .args(["status", "--db"])
        .arg(&db)
        .output()
        .unwrap();
    assert!(
        missing_status.status.success(),
        "{}",
        error(&missing_status)
    );
    let missing_text = String::from_utf8(missing_status.stdout).unwrap();
    assert!(missing_text.contains("NOT INITIALIZED"), "{missing_text}");

    let source = scratch.0.join("claude");
    fs::create_dir(&source).unwrap();
    fs::write(
        source.join("session.jsonl"),
        "{\"type\":\"assistant\",\"sessionId\":\"cli-session\",\"timestamp\":\"2026-10-09T01:00:00Z\",\"message\":{\"id\":\"m1\",\"usage\":{\"input_tokens\":5,\"output_tokens\":1,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0}}}\n",
    )
    .unwrap();
    let root = format!("claude={}", source.display());
    let run_out = command()
        .args(["run", "--db"])
        .arg(&db)
        .args(["--root", &root])
        .output()
        .unwrap();
    assert!(run_out.status.success(), "{}", error(&run_out));

    let ready_status = command()
        .args(["status", "--db"])
        .arg(&db)
        .output()
        .unwrap();
    assert!(ready_status.status.success(), "{}", error(&ready_status));
    let ready_text = String::from_utf8(ready_status.stdout).unwrap();
    assert!(ready_text.contains("State: READY"), "{ready_text}");

    let dash_once = command()
        .args(["dashboard", "--db"])
        .arg(&db)
        .arg("--once")
        .output()
        .unwrap();
    assert!(dash_once.status.success(), "{}", error(&dash_once));
    let dash_text = String::from_utf8(dash_once.stdout).unwrap();
    assert!(dash_text.contains("Dendro local explorer"), "{dash_text}");

    let html_path = scratch.0.join("dashboard.html");
    let dash_html = command()
        .args(["dashboard", "--db"])
        .arg(&db)
        .arg("--html")
        .arg(&html_path)
        .output()
        .unwrap();
    assert!(dash_html.status.success(), "{}", error(&dash_html));
    assert!(html_path.exists());

    #[cfg(any(windows, target_os = "linux"))]
    let (prior, prior_bytes, prior_key) = selected_snapshot(&db);
    let ask_out = command()
        .args(["ask", "--db"])
        .arg(&db)
        .arg("trace tokens")
        .output()
        .unwrap();
    assert!(ask_out.status.success(), "{}", error(&ask_out));
    let ask_json: serde_json::Value = serde_json::from_slice(&ask_out.stdout).unwrap();
    assert_eq!(ask_json["status"], "ok");
    assert_eq!(ask_json["auto_materialized"], true);
    assert_eq!(ask_json["window_resolution"], "recorded_bounds");
    #[cfg(any(windows, target_os = "linux"))]
    {
        let current = dendro_core::presentation::ReadRequest::open(&db).unwrap();
        assert_ne!(
            current.database(),
            prior.database(),
            "auto materialization must publish a new immutable version"
        );
        assert_eq!(fs::read(prior.database()).unwrap(), prior_bytes);
        assert_eq!(fs::read(&prior.selected().unwrap().key).unwrap(), prior_key);
        assert!(
            dendro_core::semantic::cached_scope_physical(current.database())
                .unwrap()
                .3
        );
    }

    let intents_out = command().args(["ask", "--list-intents"]).output().unwrap();
    assert!(intents_out.status.success(), "{}", error(&intents_out));
    let intents_json: serde_json::Value = serde_json::from_slice(&intents_out.stdout).unwrap();
    assert_eq!(intents_json["intents"].as_array().unwrap().len(), 6);
}

#[cfg(any(windows, target_os = "linux"))]
#[test]
fn unicode_long_logical_marker_supports_all_read_surfaces_and_immutable_materialize() {
    let scratch = Scratch::new();
    let db = scratch.0.join(format!("分析-{}.duckdb", "a".repeat(110)));
    let source = scratch.0.join("synthetic");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("session.jsonl"), "{\"type\":\"assistant\",\"sessionId\":\"synthetic\",\"timestamp\":\"2026-10-09T01:00:00Z\",\"message\":{\"id\":\"m1\",\"usage\":{\"input_tokens\":7,\"output_tokens\":2}}}\n").unwrap();
    let root = format!("claude={}", source.display());
    let run = command()
        .args(["run", "--db"])
        .arg(&db)
        .args(["--root", &root])
        .output()
        .unwrap();
    assert!(run.status.success(), "{}", error(&run));
    assert!(fs::read(&db).unwrap().starts_with(b"DENDRO-LOGICAL-DB"));
    let (before, bytes, key) = selected_snapshot(&db);
    for args in [
        vec!["report", "decisions", "--window", "all"],
        vec!["trace", "--metric-id", "tokens", "--window", "all"],
        vec!["materialize", "--window", "all"],
    ] {
        let output = command().args(args).arg("--db").arg(&db).output().unwrap();
        assert!(output.status.success(), "{}", error(&output));
        let _: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    }
    let current = dendro_core::presentation::ReadRequest::open(&db).unwrap();
    assert_ne!(current.database(), before.database());
    assert_eq!(fs::read(before.database()).unwrap(), bytes);
    assert_eq!(fs::read(&before.selected().unwrap().key).unwrap(), key);
    let export = scratch.0.join("export.html");
    let output = command()
        .args(["export", "--db"])
        .arg(&db)
        .args(["--window", "all", "--output"])
        .arg(&export)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", error(&output));
    assert!(fs::read_to_string(export)
        .unwrap()
        .to_ascii_lowercase()
        .contains("<!doctype html>"));
    let mapping = scratch.0.join("portfolio.json");
    fs::write(
        &mapping,
        serde_json::to_vec(&serde_json::json!([{"db":db,"rigs":["synthetic"]}])).unwrap(),
    )
    .unwrap();
    let output = command()
        .args(["portfolio", "--sources"])
        .arg(&mapping)
        .args([
            "--from",
            "2026-10-09T00:00:00Z",
            "--to",
            "2026-10-10T00:00:00Z",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", error(&output));
}

#[cfg(any(windows, target_os = "linux"))]
fn isolated_default_parent() -> PathBuf {
    let root = dendro_core::platform_v1::create_user_private_root_with_choice(
        "dendro-cli-default-",
        &dendro_core::platform_v1::StorageRootChoice::UserProfile,
    )
    .unwrap();
    root.path().to_path_buf()
}

#[cfg(any(windows, target_os = "linux"))]
fn with_default_parent(mut cmd: Command, parent: &Path) -> Command {
    #[cfg(windows)]
    cmd.env("LOCALAPPDATA", parent);
    #[cfg(target_os = "linux")]
    cmd.env("XDG_DATA_HOME", parent);
    cmd
}

#[cfg(windows)]
fn default_folder_name() -> &'static str {
    "Dendro"
}

#[cfg(target_os = "linux")]
fn default_folder_name() -> &'static str {
    "dendro"
}

#[cfg(any(windows, target_os = "linux"))]
#[test]
fn storage_root_choice_reuses_exact_verified_path() {
    let parent = isolated_default_parent();
    let _parent_cleanup = RemoveDir(parent.clone());
    let setup = with_default_parent(command(), &parent)
        .arg("storage-init")
        .output()
        .unwrap();
    assert!(setup.status.success(), "{}", error(&setup));
    let setup: serde_json::Value = serde_json::from_slice(&setup.stdout).unwrap();
    let scratch = Scratch(PathBuf::from(setup["storage_root"].as_str().unwrap()));
    assert!(
        scratch.0.ends_with(default_folder_name()),
        "{}",
        scratch.0.display()
    );
    assert!(scratch.0.starts_with(&parent), "{}", scratch.0.display());
    for _ in 0..2 {
        let status = command()
            .args(["status", "--storage-root"])
            .arg(&scratch.0)
            .args(["--format", "json"])
            .output()
            .unwrap();
        assert!(status.status.success(), "{}", error(&status));
        let status: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
        assert_eq!(status["status"], "not_initialized");
        assert!(status["db"]
            .as_str()
            .unwrap()
            .contains(scratch.0.to_str().unwrap()));
    }
    let refused = command()
        .args(["status", "--storage-root", "userprofile"])
        .output()
        .unwrap();
    assert!(!refused.status.success());
    assert!(error(&refused).contains("storage-init"));
    let again = with_default_parent(command(), &parent)
        .arg("storage-init")
        .output()
        .unwrap();
    assert!(again.status.success(), "{}", error(&again));
    let again: serde_json::Value = serde_json::from_slice(&again.stdout).unwrap();
    assert_eq!(again["status"], "created");
    assert_eq!(again["storage_root"], setup["storage_root"]);
}

#[cfg(any(windows, target_os = "linux"))]
#[test]
fn default_database_resolution_order() {
    let parent = isolated_default_parent();
    let _parent_cleanup = RemoveDir(parent.clone());
    let folder_name = default_folder_name();
    let empty = parent.join("empty-cwd");
    fs::create_dir(&empty).unwrap();

    let fresh = with_default_parent(command(), &parent)
        .current_dir(&empty)
        .args(["status", "--format", "json"])
        .output()
        .unwrap();
    assert!(fresh.status.success(), "{}", error(&fresh));
    let fresh_err = error(&fresh);
    assert!(!fresh_err.contains("using legacy"), "{fresh_err}");
    assert!(!fresh_err.contains("unprotected"), "{fresh_err}");
    let fresh: serde_json::Value = serde_json::from_slice(&fresh.stdout).unwrap();
    assert_eq!(fresh["status"], "not_initialized");
    let default_db = fresh["db"].as_str().unwrap();
    assert!(default_db.contains(folder_name), "{default_db}");
    assert!(default_db.ends_with("dendro.duckdb"), "{default_db}");
    let folder = PathBuf::from(default_db).parent().unwrap().to_path_buf();
    let default_file = folder.join("dendro.duckdb");
    assert!(!default_file.exists());
    let before = dir_names(&folder);

    let cwd = parent.join("legacy-cwd");
    ordinary_dir(&cwd);
    let legacy = cwd.join("dendro.duckdb");
    fs::write(&legacy, b"legacy-bytes").unwrap();
    let legacy_wal = PathBuf::from(format!("{}.wal", legacy.display()));
    fs::write(&legacy_wal, b"wal-bytes").unwrap();
    let refused = with_default_parent(command(), &parent)
        .current_dir(&cwd)
        .args(["status", "--format", "json"])
        .output()
        .unwrap();
    let refusal = error(&refused);
    let expected = refusal_message(&legacy, &folder, true);
    assert!(!refused.status.success(), "{refusal}");
    assert!(refusal.contains(&expected), "{refusal}");
    assert_eq!(
        refusal.matches("outside a private folder").count(),
        1,
        "{refusal}"
    );
    assert_eq!(fs::read(&legacy).unwrap(), b"legacy-bytes");
    assert_eq!(fs::read(&legacy_wal).unwrap(), b"wal-bytes");
    assert_eq!(dir_names(&folder), before, "refused command must not copy");

    let private_db = parent.join("dendro.duckdb");
    fs::write(&private_db, b"private-cwd-bytes").unwrap();
    let kept = with_default_parent(command(), &parent)
        .current_dir(&parent)
        .args(["status", "--format", "json"])
        .output()
        .unwrap();
    let kept_err = error(&kept);
    assert!(!kept_err.contains("outside a private folder"), "{kept_err}");
    assert!(!kept_err.contains(copy_command_needle()), "{kept_err}");
    assert!(!default_file.exists());
    assert_eq!(fs::read(&private_db).unwrap(), b"private-cwd-bytes");

    let plain = parent.join("plain");
    ordinary_dir(&plain);
    let explicit = plain.join("custom.duckdb");
    fs::write(&explicit, b"explicit-bytes").unwrap();
    let before_explicit = dir_names(&folder);
    let warned = with_default_parent(command(), &parent)
        .args(["status", "--format", "json", "--db"])
        .arg(&explicit)
        .output()
        .unwrap();
    let warning = error(&warned);
    let expected_explicit = refusal_message(&explicit, &folder, false);
    assert!(!warned.status.success(), "{warning}");
    assert!(warning.contains(&expected_explicit), "{warning}");
    assert!(!warning.contains("Also copy the WAL"), "{warning}");
    assert_eq!(
        warning.matches("outside a private folder").count(),
        1,
        "{warning}"
    );
    assert_eq!(fs::read(&explicit).unwrap(), b"explicit-bytes");
    assert_eq!(dir_names(&folder), before_explicit);

    fs::write(&default_file, b"default-bytes").unwrap();
    let preferred = with_default_parent(command(), &parent)
        .current_dir(&cwd)
        .args(["status", "--format", "json"])
        .output()
        .unwrap();
    let preferred_err = error(&preferred);
    assert!(
        !preferred_err.contains("outside a private folder"),
        "{preferred_err}"
    );
    assert_eq!(fs::read(&legacy).unwrap(), b"legacy-bytes");
    assert_eq!(fs::read(&legacy_wal).unwrap(), b"wal-bytes");
    assert_eq!(fs::read(&default_file).unwrap(), b"default-bytes");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&preferred.stdout),
        preferred_err
    );
    assert!(
        combined.contains(folder_name),
        "default folder missing from output: {combined}"
    );
}

#[cfg(any(windows, target_os = "linux"))]
fn dir_names(dir: &Path) -> Vec<String> {
    let mut names = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    names.sort();
    names
}

#[cfg(windows)]
fn copy_command_needle() -> &'static str {
    "Copy-Item -LiteralPath"
}

#[cfg(target_os = "linux")]
fn copy_command_needle() -> &'static str {
    "cp --"
}

#[cfg(windows)]
fn copy_line(source: &Path, destination: &Path) -> String {
    format!(
        "Copy-Item -LiteralPath \"{}\" -Destination \"{}\"",
        source.display(),
        destination.display()
    )
}

#[cfg(target_os = "linux")]
fn copy_line(source: &Path, destination: &Path) -> String {
    format!("cp -- '{}' '{}'", source.display(), destination.display())
}

#[cfg(any(windows, target_os = "linux"))]
fn refusal_message(source: &Path, folder: &Path, wal: bool) -> String {
    let source = std::path::absolute(source).unwrap();
    let mut message = format!(
        "database {} is outside a private folder and cannot be opened. Move it into the private default folder {}: {}",
        source.display(),
        folder.display(),
        copy_line(&source, &folder.join("dendro.duckdb"))
    );
    if wal {
        let wal_path = PathBuf::from(format!("{}.wal", source.display()));
        message.push_str(&format!(
            " Also copy the WAL: {}",
            copy_line(&wal_path, &folder.join("dendro.duckdb.wal"))
        ));
    }
    message
}

#[cfg(any(windows, target_os = "linux"))]
fn legacy_fixture(scratch: &Scratch, leaf: &str) -> PathBuf {
    let db = scratch.0.join(leaf);
    let key_path = scratch.0.join(format!("{leaf}.key"));
    let key = dendro_core::output::load_or_create_key(&key_path, true).unwrap();
    let store = dendro_core::AnalyticsStore::open_path(&db, &key.id).unwrap();
    store
        .insert_activity(&dendro_core::ActivityFact {
            source_key: "synthetic-legacy-activity".into(),
            seat: "synthetic@rig".into(),
            ts: dendro_core::parse_utc_time("2026-10-09T01:00:00Z").unwrap(),
            state: "idle".into(),
            seq: 1,
        })
        .unwrap();
    store.checkpoint().unwrap();
    db
}

#[cfg(any(windows, target_os = "linux"))]
#[test]
fn legacy_materialize_and_autoask_adopt_without_mutating_legacy_anchor() {
    let scratch = Scratch::new();
    for (leaf, args) in [
        ("materialize.duckdb", vec!["materialize", "--window", "all"]),
        ("ask.duckdb", vec!["ask", "highest idle"]),
    ] {
        let db = legacy_fixture(&scratch, leaf);
        let key_path = scratch.0.join(format!("{leaf}.key"));
        let before = fs::read(&db).unwrap();
        let before_key = fs::read(&key_path).unwrap();
        {
            let request = dendro_core::presentation::ReadRequest::open(&db).unwrap();
            assert!(request.is_legacy());
            #[cfg(windows)]
            assert!(
                fs::OpenOptions::new().write(true).open(&db).is_err(),
                "legacy request must retain deny-write custody"
            );
            #[cfg(target_os = "linux")]
            {
                let writer = fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(&db)
                    .unwrap();
                assert!(
                    writer.try_lock().is_err(),
                    "legacy read pin conflicts with cooperative exclusive writer"
                );
            }
        }
        let output = command().args(args).arg("--db").arg(&db).output().unwrap();
        assert!(output.status.success(), "{}", error(&output));
        let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        if leaf == "ask.duckdb" {
            assert_eq!(response["auto_materialized"], true);
            assert!(response["analytics_selection_sha256"]
                .as_str()
                .is_some_and(|s| s.len() == 64));
        }
        let request = dendro_core::presentation::ReadRequest::open(&db).unwrap();
        assert!(request.selected().is_some());
        assert!(
            dendro_core::semantic::cached_scope_physical(request.database())
                .unwrap()
                .3
        );
        assert_eq!(fs::read(&db).unwrap(), before);
        assert_eq!(fs::read(key_path).unwrap(), before_key);
    }
}

#[cfg(any(windows, target_os = "linux"))]
fn selected_fixture(scratch: &Scratch, leaf: &str) -> PathBuf {
    let db = scratch.0.join(leaf);
    let source = scratch.0.join(format!("source-{leaf}"));
    fs::create_dir(&source).unwrap();
    let root = format!("claude={}", source.display());
    let output = command()
        .args(["run", "--db"])
        .arg(&db)
        .args(["--root", &root])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", error(&output));
    db
}

#[cfg(any(windows, target_os = "linux"))]
#[test]
fn nao_workspace_is_private_version_scoped_copy_and_cannot_write_selected_bundle() {
    let scratch = Scratch::new();
    let db = selected_fixture(&scratch, "nao.duckdb");
    let materialize = command()
        .args(["materialize", "--db"])
        .arg(&db)
        .output()
        .unwrap();
    assert!(materialize.status.success(), "{}", error(&materialize));
    let selected = dendro_core::presentation::ReadRequest::open(&db).unwrap();
    let selected_bytes = fs::read(selected.database()).unwrap();
    let exact_only = scratch.0.join("exact-nao-workspace");
    #[cfg(windows)]
    {
        let exact_error = dendro_core::semantic::init_workspace(&exact_only, &db).unwrap_err();
        assert!(exact_error
            .to_string()
            .contains("nao_exact_workspace_path_unproved"));
        assert!(!exact_only.exists());
    }
    #[cfg(target_os = "linux")]
    {
        dendro_core::semantic::init_workspace(&exact_only, &db).unwrap();
        assert!(exact_only.join("semantics/dendro.md").is_file());
    }
    let requested_workspace = scratch.0.join("nao-workspace");
    let output = command()
        .args(["init-nao", "--db"])
        .arg(&db)
        .arg("--output")
        .arg(&requested_workspace)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", error(&output));
    let receipt: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let workspace = PathBuf::from(receipt["workspace"].as_str().unwrap());
    assert!(workspace.is_dir());
    #[cfg(windows)]
    {
        assert_ne!(workspace, requested_workspace);
        assert!(!requested_workspace.exists());
        assert_eq!(receipt["status"], "snapshot_created_integration_pending");
        assert!(workspace.join("semantics-dendro.md").is_file());
        assert!(!workspace.join("semantics").exists());
        assert!(receipt["snapshot"]["semantic_context_status"]
            .as_str()
            .unwrap()
            .contains("not verified"));
    }
    #[cfg(target_os = "linux")]
    {
        assert_eq!(workspace, requested_workspace);
        assert_eq!(receipt["status"], "created");
        assert!(workspace.join("semantics/dendro.md").is_file());
    }
    assert_eq!(receipt["snapshot"]["workspace"], receipt["workspace"]);
    assert_eq!(receipt["snapshot"]["read_only"], true);
    assert_eq!(receipt["snapshot"]["scope"], "immutable_version_snapshot");
    let metadata: serde_json::Value =
        serde_json::from_slice(&fs::read(workspace.join("database.json")).unwrap()).unwrap();
    assert_eq!(metadata, receipt["snapshot"]);
    let snapshot = PathBuf::from(metadata["path"].as_str().unwrap());
    assert_ne!(snapshot, selected.database());
    assert_eq!(fs::read(&snapshot).unwrap(), selected_bytes);
    let config = fs::read_to_string(workspace.join("nao_config.yaml")).unwrap();
    assert!(config.contains("read_only: true"));
    assert!(config.contains("does not follow later publications"));
    assert!(!config.contains(&selected.database().display().to_string()));
    let _private = dendro_core::platform_v1::open_existing_private_root(&workspace).unwrap();
    let store = selected.open_store().unwrap();
    let key_id = store.key_id.clone();
    drop(store);
    // Even a consumer disregarding read_only can only mutate its dedicated copy.
    let external = dendro_core::AnalyticsStore::open_existing(&snapshot, &key_id).unwrap();
    external
        .insert_activity(&dendro_core::ActivityFact {
            source_key: "external-snapshot-probe".into(),
            seat: "external@copy".into(),
            ts: dendro_core::parse_utc_time("2026-10-09T02:00:00Z").unwrap(),
            state: "active".into(),
            seq: 2,
        })
        .unwrap();
    external.checkpoint().unwrap();
    drop(external);
    assert_eq!(fs::read(selected.database()).unwrap(), selected_bytes);
    assert!(dendro_core::presentation::ReadRequest::open(&db).is_ok());
    let refresh = command()
        .args(["materialize", "--db"])
        .arg(&db)
        .output()
        .unwrap();
    assert!(refresh.status.success(), "{}", error(&refresh));
    let refresh: serde_json::Value = serde_json::from_slice(&refresh.stdout).unwrap();
    assert_ne!(
        refresh["analytics_selection_sha256"],
        metadata["analytics_selection_sha256"]
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(
            &fs::read(workspace.join("database.json")).unwrap()
        )
        .unwrap(),
        metadata
    );
}

#[cfg(any(windows, target_os = "linux"))]
#[test]
fn recovery_cli_reuses_recorded_selection_and_refuses_invalid_pointer() {
    let scratch = Scratch::new();
    let db = selected_fixture(&scratch, "recover.duckdb");
    let adopt = command()
        .args(["materialize", "--db"])
        .arg(&db)
        .output()
        .unwrap();
    assert!(adopt.status.success(), "{}", error(&adopt));
    let recover = command()
        .args(["recover-selection", "--db"])
        .arg(&db)
        .output()
        .unwrap();
    assert!(recover.status.success(), "{}", error(&recover));
    let receipt: serde_json::Value = serde_json::from_slice(&recover.stdout).unwrap();
    assert_eq!(receipt["status"], "unchanged");
    assert_eq!(receipt["cutoff"]["status"], "not_requested");
    let absent_cutoff = scratch.0.join("absent-cutoff.json");
    let before_files = fs::read_dir(&scratch.0).unwrap().count();
    let unchanged = command()
        .args(["recover-selection", "--db"])
        .arg(&db)
        .arg("--cutoff")
        .arg(&absent_cutoff)
        .output()
        .unwrap();
    assert!(unchanged.status.success(), "{}", error(&unchanged));
    let unchanged: serde_json::Value = serde_json::from_slice(&unchanged.stdout).unwrap();
    assert_eq!(unchanged["cutoff"]["status"], "unchanged");
    assert_eq!(unchanged["cutoff"]["published_by_recovery"], false);
    assert!(!absent_cutoff.exists());
    assert_eq!(fs::read_dir(&scratch.0).unwrap().count(), before_files);

    let partial = command()
        .args(["recover-selection", "--db"])
        .arg(&db)
        .arg("--cutoff")
        .arg(scratch.0.join("missing-parent/cutoff.json"))
        .output()
        .unwrap();
    assert!(!partial.status.success());
    let partial: serde_json::Value = serde_json::from_slice(&partial.stdout).unwrap();
    assert_eq!(partial["status"], "partial_recovery");
    assert_eq!(
        partial["analytics_selection_sha256"],
        receipt["analytics_selection_sha256"]
    );
    let manifest = fs::read_dir(&scratch.0)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("selection-")
        })
        .unwrap();
    fs::write(&manifest, b"{}").unwrap();
    let invalid = command()
        .args(["recover-selection", "--db"])
        .arg(&db)
        .output()
        .unwrap();
    assert!(
        !invalid.status.success(),
        "invalid selection must never be automatically accepted"
    );
    assert_eq!(fs::read(manifest).unwrap(), b"{}");
}

#[cfg(any(windows, target_os = "linux"))]
#[test]
fn storage_root_conflicts_are_explicit_even_when_db_equals_default() {
    let scratch = Scratch::new();
    for args in [
        vec!["status", "--db", "dendro.duckdb"],
        vec!["status", "--db=dendro.duckdb"],
        vec![
            "portfolio",
            "--sources",
            "missing.json",
            "--from",
            "2026-10-09T00:00:00Z",
            "--to",
            "2026-10-10T00:00:00Z",
        ],
        vec!["storage-init"],
    ] {
        let output = command()
            .args(args)
            .arg("--storage-root")
            .arg(&scratch.0)
            .output()
            .unwrap();
        assert!(
            !output.status.success(),
            "storage location conflict must refuse"
        );
        assert!(
            error(&output).contains("storage-root"),
            "{}",
            error(&output)
        );
    }
    assert!(!scratch.0.join("dendro.duckdb").exists());
}

#[cfg(any(windows, target_os = "linux"))]
#[test]
fn cutoff_recovery_cli_reports_unchanged_and_reconciled_without_claiming_draft_publication() {
    let scratch = Scratch::new();
    let db = selected_fixture(&scratch, "cutoff-recovery.duckdb");
    let source = scratch.0.join("source-cutoff-recovery.duckdb");
    let root = format!("claude={}", source.display());
    let cutoff = scratch.0.join("cutoff.json");
    let captured = command()
        .args(["run", "--db"])
        .arg(&db)
        .args(["--root", &root, "--write-manifest"])
        .arg(&cutoff)
        .output()
        .unwrap();
    assert!(captured.status.success(), "{}", error(&captured));
    let bytes = fs::read(&cutoff).unwrap();
    let recover = || {
        command()
            .args(["recover-selection", "--db"])
            .arg(&db)
            .arg("--cutoff")
            .arg(&cutoff)
            .output()
            .unwrap()
    };
    let unchanged = recover();
    assert!(unchanged.status.success(), "{}", error(&unchanged));
    let unchanged: serde_json::Value = serde_json::from_slice(&unchanged.stdout).unwrap();
    assert_eq!(unchanged["cutoff"]["status"], "unchanged");
    assert_eq!(unchanged["cutoff"]["published_by_recovery"], false);
    assert_eq!(
        unchanged["cutoff"]["recapture_required_for_retained_draft"],
        true
    );
    assert!(unchanged["cutoff"]["message"]
        .as_str()
        .unwrap()
        .contains("remains unpublished"));
    let state = fs::read_dir(&scratch.0)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|path| {
            let leaf = path.file_name().unwrap().to_string_lossy();
            leaf.starts_with("cutoff-state-") && leaf.ends_with("-000001.json")
        })
        .unwrap();
    // Simulate interruption after native switch and before the durable custody receipt.
    fs::remove_file(&state).unwrap();
    let reconciled = recover();
    assert!(reconciled.status.success(), "{}", error(&reconciled));
    let reconciled: serde_json::Value = serde_json::from_slice(&reconciled.stdout).unwrap();
    assert_eq!(reconciled["cutoff"]["status"], "reconciled");
    assert_eq!(reconciled["cutoff"]["custody_reconciled"], true);
    assert!(state.exists());
    assert_eq!(fs::read(&cutoff).unwrap(), bytes);
    assert_eq!(
        reconciled["analytics_selection_sha256"],
        unchanged["analytics_selection_sha256"]
    );
}

#[cfg(any(windows, target_os = "linux"))]
#[test]
fn recovery_cli_returns_json_for_legacy_noop_and_first_publication_resume() {
    let scratch = Scratch::new();
    let legacy = scratch.0.join("legacy-recovery.duckdb");
    let key = dendro_core::output::load_or_create_key(
        &PathBuf::from(format!("{}.key", legacy.display())),
        true,
    )
    .unwrap();
    let store = dendro_core::AnalyticsStore::open_path(&legacy, &key.id).unwrap();
    store.checkpoint().unwrap();
    drop(store);
    let bytes = fs::read(&legacy).unwrap();
    let cutoff = scratch.0.join("absent.json");
    let output = command()
        .args(["recover-selection", "--db"])
        .arg(&legacy)
        .arg("--cutoff")
        .arg(&cutoff)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", error(&output));
    let receipt: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(receipt["status"], "nothing_to_recover");
    assert_eq!(receipt["cutoff"]["status"], "unchanged");
    assert_eq!(receipt["published_by_recovery"], false);
    assert_eq!(fs::read(&legacy).unwrap(), bytes);
    let started = scratch.0.join("resume-recovery.duckdb");
    let choice = dendro_core::platform_v1::StorageRootChoice::Explicit(scratch.0.clone());
    assert!(dendro_core::selection_binding::run_locked_with_selection(
        &started,
        &choice,
        &[],
        &[],
        |_, _| { anyhow::bail!("injected pre-publication interruption") }
    )
    .is_err());
    let output = command()
        .args(["recover-selection", "--db"])
        .arg(&started)
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "resume_required must signal incomplete storage"
    );
    let receipt: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(receipt["status"], "resume_required");
    assert_eq!(receipt["published_by_recovery"], false);
    assert!(receipt["message"].as_str().unwrap().contains("dendro run"));
}

#[cfg(any(windows, target_os = "linux"))]
#[test]
fn explicit_cli_migration_reads_schema4_and_rollback_selects_preserved_schema3() {
    let scratch = Scratch::new();
    let logical = scratch.0.join("legacy.duckdb");
    let key_path = PathBuf::from(format!("{}.key", logical.display()));
    let key = dendro_core::output::load_or_create_key(&key_path, true).unwrap();
    let legacy = dendro_core::AnalyticsStore::open_path(&logical, &key.id).unwrap();
    legacy.checkpoint().unwrap();
    drop(legacy);
    let original = fs::read(&logical).unwrap();
    let original_key = fs::read(&key_path).unwrap();
    let output = command()
        .args(["migrate", "--db"])
        .arg(&logical)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", error(&output));
    let migrated: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(migrated["status"], "schema4_selected");
    let choice = dendro_core::platform_v1::StorageRootChoice::Explicit(scratch.0.clone());
    let selected =
        match dendro_core::selection_binding::resolve_logical_db(&logical, &choice).unwrap() {
            dendro_core::selection_binding::SelectedDb::Selected(selected) => selected,
            _ => panic!("schema4 selected"),
        };
    assert_eq!(
        dendro_core::AnalyticsStore::open_read_only_selected(&selected)
            .unwrap()
            .key_id,
        key.id
    );
    let after_fact = dendro_core::selection_binding::mutate_selected(&selected, |staged, _| {
        dendro_core::migration_v4::record_v4_only_fact(
            staged,
            &dendro_core::migration_v4::V4OnlyFact {
                id: "cli-v4-only".into(),
                ts: "2026-10-23T00:00:00Z".into(),
                source_id: "synthetic".into(),
                generation: 1,
                byte_range_half_open: [10, 20],
            },
        )?;
        Ok(())
    })
    .unwrap();
    drop(after_fact);
    drop(selected);
    let status = command()
        .args(["status", "--db"])
        .arg(&logical)
        .args(["--format", "json"])
        .output()
        .unwrap();
    assert!(status.status.success(), "{}", error(&status));
    let output = command()
        .args(["rollback", "--db"])
        .arg(&logical)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", error(&output));
    let rolled_back: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(rolled_back["status"], "schema3_selected");
    assert_eq!(rolled_back["loss_receipt"]["pointer_changed"], true);
    assert_eq!(rolled_back["loss_receipt"]["excluded_fact_count"], 1);
    assert_eq!(
        rolled_back["loss_receipt"]["failed_schema4_retained_private"],
        true
    );
    assert_ne!(
        rolled_back["analytics_selection_sha256"],
        migrated["analytics_selection_sha256"]
    );
    let restored = dendro_core::presentation::ReadRequest::open(&logical).unwrap();
    assert_eq!(fs::read(restored.database()).unwrap(), original);
    assert_eq!(fs::read(&logical).unwrap(), original);
    assert_eq!(fs::read(&key_path).unwrap(), original_key);
    let status = command()
        .args(["status", "--db"])
        .arg(&logical)
        .args(["--format", "json"])
        .output()
        .unwrap();
    assert!(status.status.success(), "{}", error(&status));
}

#[cfg(any(windows, target_os = "linux"))]
#[test]
fn cli_ingest_after_schema4_migration_preserves_preimage_and_selected_read_custody() {
    let scratch = Scratch::new();
    let logical = scratch.0.join("post-migration.duckdb");
    let key_path = PathBuf::from(format!("{}.key", logical.display()));
    let key = dendro_core::output::load_or_create_key(&key_path, true).unwrap();
    let legacy = dendro_core::AnalyticsStore::open_path(&logical, &key.id).unwrap();
    legacy.checkpoint().unwrap();
    drop(legacy);
    let original = fs::read(&logical).unwrap();
    let migrated = command()
        .args(["migrate", "--db"])
        .arg(&logical)
        .output()
        .unwrap();
    assert!(migrated.status.success(), "{}", error(&migrated));
    let source = scratch.0.join("claude-after-migration");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("synthetic.jsonl"), b"{\"type\":\"assistant\",\"sessionId\":\"synthetic-session\",\"timestamp\":\"2026-10-23T00:00:00Z\",\"message\":{\"id\":\"synthetic-message\",\"usage\":{\"input_tokens\":5,\"output_tokens\":2},\"content\":[]}}\n").unwrap();
    let output = command()
        .args(["run", "--db"])
        .arg(&logical)
        .args(["--root", &format!("claude={}", source.display())])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", error(&output));
    let selected = dendro_core::presentation::ReadRequest::open(&logical).unwrap();
    assert_eq!(
        selected.open_store().unwrap().load_sources().unwrap().len(),
        1
    );
    let custody = &selected.selected().unwrap().custody;
    assert_eq!(
        fs::read(custody.bundle().path().join("schema3-preimage.duckdb")).unwrap(),
        original
    );
    assert_eq!(fs::read(&logical).unwrap(), original);
    drop(selected);
    let output = command()
        .args(["rollback", "--db"])
        .arg(&logical)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", error(&output));
    let receipt: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(receipt["status"], "schema3_selected");
    assert!(receipt["loss_receipt"]["unresolved_loss"]
        .as_bool()
        .unwrap());
    assert_eq!(
        fs::read(
            dendro_core::presentation::ReadRequest::open(&logical)
                .unwrap()
                .database()
        )
        .unwrap(),
        original
    );
}

#[cfg(any(windows, target_os = "linux"))]
#[test]
fn first_openrig_run_aligns_owned_stage_before_logical_pointer_exists() {
    let scratch = Scratch::new();
    let logical = scratch.0.join("first-openrig.duckdb");
    let source = scratch.0.join("missing-synthetic-openrig.sqlite");
    let output = command()
        .args(["run", "--db"])
        .arg(&logical)
        .args(["--root", &format!("openrig={}", source.display())])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", error(&output));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["status"], "ready");
    assert!(dendro_core::presentation::ReadRequest::open(&logical)
        .unwrap()
        .selected()
        .is_some());
}

#[cfg(any(windows, target_os = "linux"))]
#[test]
fn watch_once_samples_a_named_process_into_a_migrated_schema4_database() {
    struct Sleeper(std::process::Child);
    impl Drop for Sleeper {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let scratch = Scratch::new();
    let logical = scratch.0.join("watch.duckdb");
    let key_path = PathBuf::from(format!("{}.key", logical.display()));
    let key = dendro_core::output::load_or_create_key(&key_path, true).unwrap();
    let legacy = dendro_core::AnalyticsStore::open_path(&logical, &key.id).unwrap();
    legacy.checkpoint().unwrap();
    drop(legacy);
    let migrated = command()
        .args(["migrate", "--db"])
        .arg(&logical)
        .output()
        .unwrap();
    assert!(migrated.status.success(), "{}", error(&migrated));

    #[cfg(windows)]
    let (name, mut sleeper) = ("ping", Command::new("ping"));
    #[cfg(windows)]
    sleeper.args(["-n", "30", "127.0.0.1"]);
    #[cfg(not(windows))]
    let (name, mut sleeper) = ("sleep", Command::new("sleep"));
    #[cfg(not(windows))]
    sleeper.arg("30");
    let sleeper = Sleeper(
        sleeper
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap(),
    );

    let output = command()
        .args(["watch", "--once", "--harness", name, "--db"])
        .arg(&logical)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", error(&output));
    let summary: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(summary["status"], "watched");
    assert_eq!(summary["ticks"], 1);
    assert!(summary["samples"].as_i64().unwrap() >= 1, "{summary}");

    let selected = dendro_core::presentation::ReadRequest::open(&logical).unwrap();
    let store = selected.open_store().unwrap();
    let count = |table: &str, filter: &str| -> i64 {
        store
            .with_private_connection(|conn| {
                Ok(conn.query_row(
                    &format!("SELECT COUNT(*) FROM {table} WHERE {filter}"),
                    [],
                    |row| row.get::<_, i64>(0),
                )?)
            })
            .unwrap()
    };
    assert!(count("process_samples", "rss_bytes > 0") >= 1);
    assert!(
        count(
            "process_identities",
            &format!("pid = {} AND ended_at IS NULL", sleeper.0.id())
        ) == 1
    );
    // Nothing from the watched process's command line or directory is stored.
    assert_eq!(
        count("process_identities", "exe_name LIKE '%127.0.0.1%'"),
        0
    );
}

#[cfg(any(windows, target_os = "linux"))]
fn migrated_db(scratch: &Scratch, name: &str) -> PathBuf {
    let logical = scratch.0.join(name);
    let key_path = PathBuf::from(format!("{}.key", logical.display()));
    let key = dendro_core::output::load_or_create_key(&key_path, true).unwrap();
    let legacy = dendro_core::AnalyticsStore::open_path(&logical, &key.id).unwrap();
    legacy.checkpoint().unwrap();
    drop(legacy);
    let migrated = command()
        .args(["migrate", "--db"])
        .arg(&logical)
        .output()
        .unwrap();
    assert!(migrated.status.success(), "{}", error(&migrated));
    logical
}

#[cfg(any(windows, target_os = "linux"))]
fn open_store(db: &Path) -> dendro_core::AnalyticsStore {
    dendro_core::presentation::ReadRequest::open(db)
        .unwrap()
        .open_store()
        .unwrap()
}

#[cfg(any(windows, target_os = "linux"))]
fn run_agent(db: &Path, interval: &str, args: &[&str]) -> Output {
    let mut command = command();
    command
        .arg("run-agent")
        .args(["--interval", interval, "--flush-every", "60s"])
        .arg("--db")
        .arg(db)
        .arg("--")
        .args(args);
    command.output().unwrap()
}

#[cfg(any(windows, target_os = "linux"))]
#[test]
fn run_agent_propagates_exit_code_and_stores_the_root_as_launched() {
    let scratch = Scratch::new();
    let db = migrated_db(&scratch, "exit.duckdb");
    #[cfg(windows)]
    let args = ["cmd", "/c", "exit", "3"];
    #[cfg(not(windows))]
    let args = ["sh", "-c", "exit 3"];
    let output = run_agent(&db, "50ms", &args);
    assert_eq!(output.status.code(), Some(3), "{}", error(&output));
    let (exe, started): (String, i64) = open_store(&db)
        .with_private_connection(|conn| {
            let leaked: i64 = conn.query_row(
                "SELECT COUNT(*) FROM process_identities
                 WHERE exe_name LIKE ? OR COALESCE(harness, '') LIKE ?
                    OR COALESCE(attribution, '') LIKE ?",
                ["%exit%", "%exit%", "%exit%"],
                |row| row.get(0),
            )?;
            assert_eq!(leaked, 0, "command arguments must not be stored");
            Ok(conn.query_row(
                "SELECT exe_name, start_time_unix_s FROM process_identities WHERE attribution = ?",
                ["launched"],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?)
        })
        .unwrap();
    let exe_lower = exe.to_ascii_lowercase();
    #[cfg(windows)]
    assert!(exe_lower == "cmd.exe" || exe_lower == "cmd", "{exe}");
    #[cfg(not(windows))]
    assert_eq!(exe_lower, "sh");
    assert!(started > 0, "pinned root keeps the OS start time");
}

#[cfg(any(windows, target_os = "linux"))]
#[test]
fn run_agent_tracks_a_grandchild_under_the_pinned_root() {
    let scratch = Scratch::new();
    let db = migrated_db(&scratch, "tree.duckdb");
    #[cfg(windows)]
    let args = ["cmd", "/c", "ping", "-n", "6", "127.0.0.1"];
    #[cfg(not(windows))]
    let args = ["sh", "-c", "sleep 2 & wait"];
    let output = run_agent(&db, "50ms", &args);
    assert!(output.status.success(), "{}", error(&output));
    let descendants: i64 = open_store(&db)
        .with_private_connection(|conn| {
            let leaked: i64 = conn.query_row(
                "SELECT COUNT(*) FROM process_identities
                 WHERE exe_name LIKE ? OR COALESCE(harness, '') LIKE ?",
                ["%127.0.0.1%", "%127.0.0.1%"],
                |row| row.get(0),
            )?;
            assert_eq!(leaked, 0);
            Ok(conn.query_row(
                "SELECT COUNT(*) FROM process_identities AS child
                 WHERE child.attribution IS NULL
                   AND child.root_key = (
                       SELECT identity_key FROM process_identities
                       WHERE attribution = ? LIMIT 1
                   )",
                ["launched"],
                |row| row.get(0),
            )?)
        })
        .unwrap();
    assert!(
        descendants >= 1,
        "grandchild must sit under the launched root"
    );
}

#[cfg(any(windows, target_os = "linux"))]
#[test]
fn run_agent_spawn_failure_saves_nothing() {
    let scratch = Scratch::new();
    let db = migrated_db(&scratch, "missing.duckdb");
    let output = run_agent(&db, "1s", &["dendro-missing-run-agent-binary"]);
    assert_ne!(output.status.code().unwrap_or(0), 0);
    let stderr = error(&output);
    assert!(stderr.contains("could not launch"), "stderr was: {stderr}");
    let rows: i64 = open_store(&db)
        .with_private_connection(|conn| {
            let exists: i64 = conn.query_row(
                "SELECT COUNT(*) FROM information_schema.tables WHERE table_name = ?",
                ["process_identities"],
                |row| row.get(0),
            )?;
            if exists == 0 {
                return Ok(0);
            }
            Ok(
                conn.query_row("SELECT COUNT(*) FROM process_identities", [], |row| {
                    row.get(0)
                })?,
            )
        })
        .unwrap();
    assert_eq!(rows, 0);
}
