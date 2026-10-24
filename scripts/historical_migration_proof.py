#!/usr/bin/env python3
"""Run historical 0.7.0 CLI + historical core opener probes on synthetic copies.

Run on native Linux or WSL ext4; no installed binaries, owner databases, or source
logs are used. A source-pinned historical build is not a published release artifact.
Requires Rust dependencies already present when --offline is selected.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess

PIN = "6b755135f95e497071790e8e5e358995a8244a18"
ORACLE = "33837c362c332b3021b2319800a8ab94a455cc43c306393915535634ffe8f113"


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def canonical_sha(path):
    return hashlib.sha256(Path(path).read_bytes().replace(b"\r\n", b"\n")).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, required=True)
    parser.add_argument("--scratch", type=Path, required=True)
    parser.add_argument("--target-dir", type=Path, required=True)
    parser.add_argument("--offline", action="store_true")
    args = parser.parse_args()
    repo, scratch, target = (p.resolve() for p in (args.repo, args.scratch, args.target_dir))
    if scratch.exists():
        raise SystemExit("scratch must not exist; evidence is never overwritten")
    if os.name != "posix" or str(scratch).startswith("/mnt/") or str(target).startswith("/mnt/"):
        raise SystemExit("run on native Linux or ext4 WSL scratch and Cargo cache")
    scratch.mkdir(parents=True, mode=0o700)
    os.chmod(scratch, 0o700)
    targets = {"historical": target / "historical", "candidate": target / "candidate"}
    env = dict(os.environ, CARGO_BUILD_JOBS="2")
    # Every subprocess gets isolated roots, including old CLI root discovery.
    home = scratch / "home"
    home.mkdir(mode=0o700)
    (home / ".local/share").mkdir(parents=True, mode=0o700)
    runtime_env = dict(env, HOME=str(home), USERPROFILE=str(home), XDG_CONFIG_HOME=str(home / ".config"), XDG_DATA_HOME=str(home / ".local/share"))
    commands = []

    def run(name, argv, cwd=None, runtime=False, check=True, target_kind=None):
        process_env = dict(runtime_env if runtime else env)
        if target_kind:
            process_env["CARGO_TARGET_DIR"] = str(targets[target_kind])
        stdout_path, stderr_path = scratch / f"{name}.stdout", scratch / f"{name}.stderr"
        with stdout_path.open("w") as stdout, stderr_path.open("w") as stderr:
            p = subprocess.run([str(x) for x in argv], cwd=cwd, env=process_env, stdout=stdout, stderr=stderr, text=True)
        p.stdout, p.stderr = stdout_path.read_text(), stderr_path.read_text()
        commands.append({"name": name, "argv": [str(x) for x in argv], "exit_code": p.returncode})
        if check and p.returncode:
            raise RuntimeError(f"{name} failed ({p.returncode}): {p.stderr[-2000:]}")
        return p

    def archive(commit, destination):
        tar = scratch / f"{destination.name}.tar"
        run(f"archive-{destination.name}", ["git", "-C", repo, "archive", "--format=tar", f"--output={tar}", commit])
        destination.mkdir()
        run(f"extract-{destination.name}", ["tar", "-xf", tar, "-C", destination])
        return sha(tar)

    result = {"schema": "dendro.historical-migration-proof/v1", "passed": False, "historical_commit": PIN, "historical_release_artifact": False, "execution_environment": "WSL" if "microsoft" in os.uname().release.lower() else "native-linux", "native_hosted_linux_acceptance": False}
    try:
        frozen = repo / ".agentflow/oracles/v080/T04/frozen.json"
        assert canonical_sha(frozen) == ORACLE, "frozen oracle canonical committed content changed"
        old, current = scratch / "historical-source", scratch / "current-source"
        result["cargo_target_dirs"] = {key: str(path) for key, path in targets.items()}
        result["historical_tree"] = run("historical-tree", ["git", "-C", repo, "rev-parse", f"{PIN}^{{tree}}"]).stdout.strip()
        result["historical_source_archive_sha256"] = archive(PIN, old)
        archive("HEAD", current)
        # Candidate files may be uncommitted. Overlay only regular Rust source,
        # manifests, lock and frozen oracle files; never owner state or links.
        for directory in ("crates", ".agentflow/oracles/v080/T04"):
            snapshot_directory = current / directory
            assert snapshot_directory.resolve().is_relative_to(scratch)
            shutil.rmtree(snapshot_directory)
            for path in (repo / directory).rglob("*"):
                if path.is_symlink():
                    raise RuntimeError(f"proof snapshot refuses source links: {path}")
                if path.is_file() and not path.is_symlink() and "target" not in path.parts:
                    dst = current / path.relative_to(repo)
                    dst.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copyfile(path, dst)
        shutil.copyfile(repo / "Cargo.lock", current / "Cargo.lock")
        shutil.copyfile(repo / "Cargo.toml", current / "Cargo.toml")
        result["historical_lock_sha256"] = sha(old / "Cargo.lock")
        result["historical_storage_sha256"] = sha(old / "crates/dendro-core/src/storage.rs")
        candidate_files = [p for p in (current / "crates").rglob("*") if p.is_file()]
        candidate_files.extend([current / "Cargo.toml", current / "Cargo.lock"])
        result["candidate_files"] = {str(p.relative_to(current)): sha(p) for p in sorted(candidate_files)}
        result["oracle_sha256"] = canonical_sha(frozen)
        result["oracle_checkout_sha256"] = sha(frozen)
        result["oracle_digest_policy"] = "canonical committed LF bytes; CRLF checkout translation accepted; no other normalization"
        result["harness_files"] = {str(p.relative_to(repo)): sha(p) for p in [Path(__file__).resolve(), repo / "scripts/fixtures/historical_openers.rs", repo / "scripts/fixtures/migration_proof_driver.rs"]}
        harness_snapshot = scratch / "harness-fixtures"
        harness_snapshot.mkdir()
        for source in ("historical_openers.rs", "migration_proof_driver.rs"):
            shutil.copyfile(repo / "scripts/fixtures" / source, harness_snapshot / source)
        result["frozen_fixture_files"] = {str(p.relative_to(repo)): sha(p) for p in (repo / ".agentflow/oracles/v080/T04").glob("schema3-*") if p.is_file()}
        result["toolchain"] = run("rustc", ["rustc", "-Vv"]).stdout
        build_flags = ["--locked"] + (["--offline"] if args.offline else [])
        run("historical-cli-build", ["cargo", "build", *build_flags, "-p", "dendro-cli"], old, target_kind="historical")
        binary = scratch / "dendro-historical"
        shutil.copy2(targets["historical"] / "debug/dendro", binary)
        result["historical_binary_sha256"] = sha(binary)
        result["historical_binary_version"] = run("historical-version", [binary, "--version"], runtime=True).stdout.strip()
        assert result["historical_binary_version"] == "dendro 0.7.0"
        for source, checkout, example in [("historical_openers.rs", old, "historical_openers"), ("migration_proof_driver.rs", current, "migration_proof_driver")]:
            dest = checkout / "crates/dendro-core/examples" / source
            dest.parent.mkdir(exist_ok=True)
            shutil.copyfile(harness_snapshot / source, dest)
            kind = "historical" if checkout == old else "candidate"
            run(f"build-{example}", ["cargo", "build", *build_flags, "-p", "dendro-core", "--example", example], checkout, target_kind=kind)
            shutil.copy2(targets[kind] / "debug/examples" / example, scratch / example)
        probe, driver = scratch / "historical_openers", scratch / "migration_proof_driver"
        result["historical_probe_sha256"] = sha(probe)
        result["candidate_driver_sha256"] = sha(driver)
        sql = current / ".agentflow/oracles/v080/T04/schema3-fixture.sql"
        fixture = json.loads(run("fixture-migration-rollback", [driver, scratch / "oracle-fixture", sql], runtime=True).stdout)
        result["oracle_fixture"] = fixture
        # The actual old CLI creates schema3 using a deterministic synthetic log.
        logs = scratch / "synthetic-logs"
        logs.mkdir()
        (logs / "session.jsonl").write_text(json.dumps({"type": "assistant", "sessionId": "historical-proof", "timestamp": "2026-10-22T00:00:00Z", "message": {"id": "message-1", "model": "proof-model", "content": [], "usage": {"input_tokens": 17, "output_tokens": 3, "cache_read_input_tokens": 0, "cache_creation_input_tokens": 0}}}) + "\n")
        produced = scratch / "historical-produced.duckdb"
        run("historical-ingest", [binary, "run", "--db", produced, "--root", f"claude={logs}"], runtime=True)
        before_report = run("historical-report-before", [binary, "report", "tokens", "--db", produced, "--window", "all"], runtime=True).stdout
        source_before = {"db": sha(produced), "key": sha(str(produced) + ".key")}
        # Remove exactly this harness-created synthetic file, not any user logs.
        (logs / "session.jsonl").unlink()
        logs.rmdir()
        produced_migration = json.loads(run("produced-migration-rollback", [driver, scratch / "produced-fixture", produced, str(produced) + ".key"], runtime=True).stdout)
        result["historical_produced_fixture"] = produced_migration
        after_report = run("historical-report-rollback", [binary, "report", "tokens", "--db", produced_migration["schema3"], "--window", "all"], runtime=True).stdout
        assert json.loads(before_report) == json.loads(after_report), "historical report facts changed after rollback"
        assert source_before == {"db": sha(produced), "key": sha(str(produced) + ".key")}, "historical source mutated"
        result["historical_report_same_after_rollback"] = True
        selected_report = run("historical-report-selected-rollback", [binary, "report", "tokens", "--db", produced_migration["selected_rollback_database"], "--window", "all"], runtime=True).stdout
        assert json.loads(before_report) == json.loads(selected_report), "historical facts changed after actual pointer rollback"
        assert produced_migration["selected_pointer_rollback"], "driver did not publish actual migration/rollback selections"
        result["historical_report_same_after_selected_rollback"] = True
        result["historical_refusal_cases"] = []
        for label, data in [("oracle", fixture), ("historical-produced", produced_migration), ("selected-oracle", dict(fixture, schema4=fixture["selected_migration_database"])), ("selected-historical-produced", dict(produced_migration, schema4=produced_migration["selected_migration_database"]))]:
            for mode in ("open_existing", "open_staged", "open_read_only", "cli-report"):
                actual_selection = label.startswith("selected-")
                copy = Path(data["schema4"]) if actual_selection else scratch / f"{label}-{mode}.duckdb"
                if not actual_selection:
                    shutil.copyfile(data["schema4"], copy)
                before_hash = sha(copy)
                canonical_before = json.loads(run(f"{label}-{mode}-before", [probe, "snapshot", copy], runtime=True).stdout)
                if mode == "cli-report":
                    p = run(f"{label}-{mode}", [binary, "report", "tokens", "--db", copy, "--window", "all"], runtime=True, check=False)
                    assert p.returncode != 0 and "schema version 4" in p.stderr, "historical CLI did not refuse schema4"
                else:
                    p = run(f"{label}-{mode}", [probe, mode, copy], runtime=True)
                    assert json.loads(p.stdout)["refused"]
                    assert json.loads(p.stdout)["historical_schema"] == 3
                    result["historical_schema"] = 3
                canonical_after = json.loads(run(f"{label}-{mode}-after", [probe, "snapshot", copy], runtime=True).stdout)
                assert canonical_before == canonical_after, "canonical content changed"
                assert before_hash == sha(copy), "historical opener changed bytes"
                result["historical_refusal_cases"].append({"fixture": label, "mode": mode, "target_kind": "actual_selected_file" if actual_selection else "stage_private_copy", "refused": True, "canonical_unchanged": True, "sha256_before": before_hash, "sha256_after": sha(copy), "byte_identical": before_hash == sha(copy)})
        # An old executable cannot interpret the new selection pointer. Prove
        # that a real old-CLI write to its schema3 anchor is explicitly detected.
        negative_logs = scratch / "old-anchor-write-logs"
        negative_logs.mkdir()
        (negative_logs / "new.jsonl").write_text(json.dumps({"type": "assistant", "sessionId": "old-anchor-negative", "timestamp": "2026-10-22T00:00:05Z", "message": {"id": "new-old-message", "content": [], "usage": {"input_tokens": 29, "output_tokens": 7, "cache_read_input_tokens": 0, "cache_creation_input_tokens": 0}}}) + "\n")
        anchor = Path(produced_migration["schema3"])
        before_anchor, stable_selected = sha(anchor), sha(produced_migration["selected_rollback_database"])
        run("old-cli-anchor-write", [binary, "run", "--db", anchor, "--root", f"claude={negative_logs}"], runtime=True)
        assert sha(anchor) != before_anchor, "old CLI negative control made no anchor write"
        result["old_anchor_negative_control"] = json.loads(run("new-reader-detects-old-anchor-write", [driver, "--check-anchor-divergence", anchor], runtime=True).stdout)
        assert sha(produced_migration["selected_rollback_database"]) == stable_selected, "old anchor write changed selected immutable data"
        result["old_anchor_negative_control"]["selected_database_unchanged"] = True
        assert canonical_sha(frozen) == ORACLE
        result["passed"] = True
    finally:
        result["commands"] = commands
        (scratch / "receipt.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({"passed": result["passed"], "receipt": str(scratch / "receipt.json")}))


if __name__ == "__main__":
    main()
