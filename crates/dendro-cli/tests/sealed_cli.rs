use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        #[cfg(windows)]
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
        #[cfg(not(windows))]
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/issue-13-tests")
            .join(format!(
                "cli-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        #[cfg(not(windows))]
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

#[cfg(windows)]
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

    #[cfg(windows)]
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
    #[cfg(windows)]
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

#[cfg(windows)]
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

#[cfg(windows)]
#[test]
fn storage_root_choice_reuses_exact_verified_path() {
    let setup = command().arg("storage-init").output().unwrap();
    assert!(setup.status.success(), "{}", error(&setup));
    let setup: serde_json::Value = serde_json::from_slice(&setup.stdout).unwrap();
    let scratch = Scratch(PathBuf::from(setup["storage_root"].as_str().unwrap()));
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
}

#[cfg(windows)]
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

#[cfg(windows)]
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
            assert!(
                fs::OpenOptions::new().write(true).open(&db).is_err(),
                "legacy request must retain deny-write custody"
            );
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

#[cfg(windows)]
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

#[cfg(windows)]
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
    let exact_error = dendro_core::semantic::init_workspace(&exact_only, &db).unwrap_err();
    assert!(exact_error
        .to_string()
        .contains("nao_exact_workspace_path_unproved"));
    assert!(!exact_only.exists());
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
    assert_ne!(workspace, requested_workspace);
    assert!(!requested_workspace.exists());
    assert_eq!(receipt["snapshot"]["workspace"], receipt["workspace"]);
    assert_eq!(receipt["status"], "snapshot_created_integration_pending");
    assert!(workspace.join("semantics-dendro.md").is_file());
    assert!(!workspace.join("semantics").exists());
    assert!(receipt["snapshot"]["semantic_context_status"]
        .as_str()
        .unwrap()
        .contains("not verified"));
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

#[cfg(windows)]
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
    assert_eq!(receipt["status"], "recovered");
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

#[cfg(windows)]
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

#[cfg(windows)]
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
