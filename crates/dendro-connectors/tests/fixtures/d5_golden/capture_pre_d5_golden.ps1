# Reproducible pre-D5 golden capture script for `dendro#134` (MTN-23 / D5).
# Checks out `origin/development` (e7c39e872834bbf30127624dcfa74f32e3001190) in a temporary
# worktree with its own isolated CARGO_TARGET_DIR, captures both default-scope and
# time-window-scope (`2026-10-10T00:00:00Z` .. `2026-10-11T00:00:00Z`) golden outputs over the
# ingested `{claude, codex, pi}` fixture store, and removes the temporary worktree and target
# directory on exit.

$ErrorActionPreference = "Stop"

$GoldenDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$RepoRoot = (Resolve-Path (Join-Path $GoldenDir "..\..\..\..\..")).Path
$TmpWt = Join-Path $RepoRoot ".tmp-pre-d5-wt"
$TmpTarget = Join-Path $RepoRoot ".tmp-pre-d5-target"
$BaseCommit = "e7c39e872834bbf30127624dcfa74f32e3001190"

try {
    if (Test-Path $TmpWt) {
        git -C $RepoRoot worktree remove --force $TmpWt | Out-Null
    }
    if (Test-Path $TmpTarget) {
        Remove-Item -Recurse -Force $TmpTarget
    }

    git -C $RepoRoot worktree add --detach $TmpWt $BaseCommit | Out-Null

    $WtSources = Join-Path $TmpWt "crates\dendro-connectors\tests\fixtures\d5_golden\sources"
    New-Item -ItemType Directory -Force -Path $WtSources | Out-Null
    Copy-Item -Recurse -Force (Join-Path $GoldenDir "sources\*") $WtSources

    $LibRs = Join-Path $TmpWt "crates\dendro-connectors\src\lib.rs"
    $OutDirEscaped = $GoldenDir.Replace('\', '\\')
    $CaptureTest = @"

#[cfg(test)]
mod pre_d5_golden_capture {
    use crate::ingest::ingest_staged;
    use crate::SourceRoot;
    use dendro_core::insight_v1::{evaluate, evaluate_catalog, Scope};
    use dendro_core::AnalyticsStore;
    use std::path::{Path, PathBuf};

    #[test]
    fn capture_pre_d5_golden_outputs() {
        let key = b"0123456789abcdef0123456789abcdef";
        let guard = dendro_core::platform_v1::create_user_private_root_with_choice(
            "dendro-pre-d5-cap-",
            &dendro_core::platform_v1::StorageRootChoice::UserProfile,
        )
        .unwrap();
        let db_path = guard.path().join("staged.duckdb");
        drop(AnalyticsStore::open_path(&db_path, &dendro_core::key_id(key)).unwrap());
        let roots = vec![
            SourceRoot {
                name: "claude".into(),
                path: PathBuf::from("tests/fixtures/d5_golden/sources/claude"),
            },
            SourceRoot {
                name: "codex".into(),
                path: PathBuf::from("tests/fixtures/d5_golden/sources/codex"),
            },
            SourceRoot {
                name: "pi".into(),
                path: PathBuf::from("tests/fixtures/d5_golden/sources/pi"),
            },
        ];
        let _warnings = ingest_staged(&db_path, &roots, false, key).unwrap();
        let store = AnalyticsStore::open_path(&db_path, &dendro_core::key_id(key)).unwrap();
        let out_dir = Path::new("$OutDirEscaped");

        let default_scope = Scope::default();
        std::fs::write(
            out_dir.join("pre_d5_evaluate.json"),
            serde_json::to_string_pretty(&evaluate(&store, &default_scope)).unwrap() + "\n",
        )
        .unwrap();
        std::fs::write(
            out_dir.join("pre_d5_evaluate_catalog.json"),
            serde_json::to_string_pretty(&evaluate_catalog(&store, &default_scope)).unwrap() + "\n",
        )
        .unwrap();

        let windowed_scope = Scope {
            from: Some("2026-10-10T00:00:00Z".into()),
            to: Some("2026-10-11T00:00:00Z".into()),
            ..Scope::default()
        };
        std::fs::write(
            out_dir.join("pre_d5_evaluate_windowed.json"),
            serde_json::to_string_pretty(&evaluate(&store, &windowed_scope)).unwrap() + "\n",
        )
        .unwrap();
        std::fs::write(
            out_dir.join("pre_d5_evaluate_catalog_windowed.json"),
            serde_json::to_string_pretty(&evaluate_catalog(&store, &windowed_scope)).unwrap() + "\n",
        )
        .unwrap();

        drop(store);
        drop(guard);
    }
}
"@
    Add-Content -Path $LibRs -Value $CaptureTest

    $env:CARGO_TARGET_DIR = $TmpTarget
    $env:CARGO_INCREMENTAL = "0"
    $env:CARGO_PROFILE_DEV_DEBUG = "0"
    $env:CARGO_PROFILE_TEST_DEBUG = "0"

    Push-Location $TmpWt
    try {
        cargo +1.98.1 test -p dendro-connectors --lib --locked -j 4 capture_pre_d5_golden_outputs -- --nocapture
        if ($LASTEXITCODE -ne 0) {
            throw "cargo test capture_pre_d5_golden_outputs failed with exit code $LASTEXITCODE"
        }
    } finally {
        Pop-Location
        Remove-Item Env:\CARGO_TARGET_DIR -ErrorAction SilentlyContinue
    }
} finally {
    if (Test-Path $TmpWt) {
        git -C $RepoRoot worktree remove --force $TmpWt | Out-Null
    }
    if (Test-Path $TmpTarget) {
        Remove-Item -Recurse -Force $TmpTarget
    }
}
