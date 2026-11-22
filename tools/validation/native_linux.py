# /// script
# requires-python = ">=3.12"
# dependencies = []
# ///
"""Run Linux workspace tests on admitted ext4; Windows retains all source Git work."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import uuid
import signal
import time


def disk_limits(cache, workspace, host_source, limits):
    guard = local_module("windows_guard")
    floor, cache_max, workspace_max = limits
    for path in (cache, workspace, host_source):
        guard.safe_path(path)
        if shutil.disk_usage(path).free < floor:
            raise SystemExit(f"Native Linux free-space floor breached: {path} (minimum {floor} bytes)")
    if guard.tree_bytes(cache) > cache_max:
        raise SystemExit(f"Native Linux cache budget exceeded: {cache} (maximum {cache_max} bytes)")
    if guard.tree_bytes(workspace) > workspace_max:
        raise SystemExit(f"Native Linux workspace budget exceeded: {workspace} (maximum {workspace_max} bytes)")
    if guard.tree_bytes(host_source) > workspace_max:
        raise SystemExit(f"Native Linux source-stage budget exceeded: {host_source} (maximum {workspace_max} bytes)")


def bounded_tests(command, source, environment, cache, workspace, host_source, limits):
    """The Linux supervisor retains ownership even if its Windows client exits."""
    disk_limits(cache, workspace, host_source, limits)
    previous_signals = {}
    def interrupted(signum, frame):
        raise SystemExit(f"Native Linux supervisor interrupted by signal {signum}")
    for signum in (signal.SIGTERM, signal.SIGHUP):
        previous_signals[signum] = signal.signal(signum, interrupted)
    process = None
    deadline = time.monotonic() + 7200
    try:
        process = subprocess.Popen(command, cwd=source, env=environment, start_new_session=True)
        while process.poll() is None:
            disk_limits(cache, workspace, host_source, limits)
            if time.monotonic() >= deadline:
                raise SystemExit("Native Linux validation exceeded its two-hour deadline")
            time.sleep(0.2)
        disk_limits(cache, workspace, host_source, limits)
        return process.returncode
    finally:
        # Signal the owned process group even if Cargo exited before its children.
        if process:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait()
        for signum, previous in previous_signals.items():
            signal.signal(signum, previous)

ROOT = Path(__file__).resolve().parents[2]
TOOLCHAIN = "1.98.1"


def local_module(name):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def candidate():
    head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    if subprocess.check_output(["git", "status", "--porcelain", "-z"], cwd=ROOT):
        raise SystemExit("Native Linux acceptance requires a clean committed candidate")
    return head


def receipt_result(head, staged, metadata, child_code, unchanged):
    expected = {"sourceHead": head, "sourceTree": staged["git_tree"],
                "sourceCopyManifestSha256": staged["candidate_manifest_sha256"],
                "sourceFileCount": staged["file_count"], "sourceExecutablePaths": staged["executable_paths"]}
    attested = staged["head"] == head and isinstance(metadata, dict) and metadata.get("filesystem") == "ext4" and all(
        type(metadata.get(key)) is type(value) and metadata[key] == value for key, value in expected.items()
    )
    passed = child_code == 0 and unchanged and attested
    return {"head": head, "childExitCode": child_code,
            "exitCode": child_code if child_code else (0 if passed else 1),
            "candidateUnchanged": unchanged, "sourceAttested": attested, "passed": passed,
            "sourceStage": staged, "native": metadata}


def child_metadata(output):
    records = []
    for line in output.splitlines():
        if line.startswith('{"filesystem":'):
            try:
                records.append(json.loads(line))
            except ValueError:
                return None
    return records[0] if len(records) == 1 else None


def linux_tests(source, cache, limits, expected):
    staging = local_module("source_stage")
    copied = staging.verify_source(source, expected_head=expected["head"],
                                   expected_digest=expected["candidate_manifest_sha256"])
    limits = limits or (10 * 1024 ** 3, 24 * 1024 ** 3, 4 * 1024 ** 3)
    host_source = source
    original_home = Path.home()
    cache = (cache or original_home / ".cache/dendro-validation/linux-rust-1.98.1").resolve()
    if cache in (original_home, Path("/")) or source == cache or source in cache.parents:
        raise SystemExit("Linux cache must be a separate private build directory")
    cache.mkdir(parents=True, mode=0o700, exist_ok=True)
    if cache.stat().st_uid != os.getuid() or cache.stat().st_mode & 0o077:
        raise SystemExit("Linux cache must be owned by this user and mode 0700")
    filesystem = subprocess.check_output(["findmnt", "-n", "-o", "FSTYPE", "-T", str(cache)], text=True).strip()
    if filesystem != "ext4":
        raise SystemExit(f"Native Linux fixture cache needs ext4, found {filesystem}")
    rustup = shutil.which("rustup")
    if not rustup:
        raise SystemExit("Install the pinned Rust toolchain through rustup before validation")
    probe_environment = dict(os.environ, MISE_AUTO_UPDATE="false", MISE_SELF_UPDATE_AUTO="false",
        MISE_AUTO_INSTALL="false", MISE_TASK_RUN_AUTO_INSTALL="false", RUSTUP_AUTO_INSTALL="0")
    cargo = Path(subprocess.check_output([rustup, "which", "--toolchain", TOOLCHAIN, "cargo"],
                                       text=True, env=probe_environment, cwd=original_home).strip())
    if not cargo.is_file():
        raise SystemExit("Pinned Linux cargo executable is unavailable")
    target = (cache / "target").resolve()
    target.mkdir(mode=0o700, exist_ok=True)
    cargo_home = cache / "cargo"
    cargo_home.mkdir(mode=0o700, exist_ok=True)
    commands = [command for command in local_module("verify").RUST_GATES if command[:2] == ["cargo", "test"]]
    if len(commands) != 1 or commands[0][2:] != ["--workspace", "--locked"]:
        raise SystemExit("Classify changed Rust test gates before native acceptance")
    command = [str(cargo), "test", "--manifest-path", str(source / "Cargo.toml"), *commands[0][2:]]
    fixture_base = original_home / ".cache/dendro-tests"
    fixture_base.mkdir(parents=True, mode=0o700, exist_ok=True)
    if fixture_base.stat().st_uid != os.getuid() or fixture_base.stat().st_mode & 0o077:
        raise SystemExit("Native fixture base must be private and owned by this user")
    if subprocess.check_output(["findmnt", "-n", "-o", "FSTYPE", "-T", str(fixture_base)], text=True).strip() != "ext4":
        raise SystemExit("Native fixture base must be on ext4")
    disk_limits(cache, fixture_base, host_source, limits)
    # Keep fixture paths short enough for Unix sockets; retain the long-lived build cache.
    with tempfile.TemporaryDirectory(prefix="r-", dir=fixture_base) as temporary:
        profile = Path(temporary) / "h"
        if len(os.fsencode(str(profile))) > 50:
            raise SystemExit("Native fixture HOME is too long for the Unix socket tests")
        profile.mkdir(mode=0o700)
        private_source = profile / "s"
        shutil.copytree(source, private_source, copy_function=shutil.copyfile, symlinks=True)
        staging.apply_source_modes(private_source)
        staging.verify_source(private_source, expected_head=copied["head"],
                              expected_digest=copied["candidate_manifest_sha256"], enforce_modes=True)
        if subprocess.check_output(["findmnt", "-n", "-o", "FSTYPE", "-T", str(private_source)], text=True).strip() != "ext4":
            raise SystemExit("Native source scratch must also be on ext4")
        source = private_source
        command = [str(cargo), "test", "--manifest-path", str(source / "Cargo.toml"), *commands[0][2:]]
        temporary_files = profile / "t"
        temporary_files.mkdir(mode=0o700)
        for name in (".config", ".local/share", ".cache", ".local/state", ".runtime"):
            (profile / name).mkdir(parents=True, mode=0o700, exist_ok=True)
        environment = {key: value for key, value in os.environ.items() if key in ("LANG", "LC_ALL", "TERM")}
        environment.update(dict(HOME=str(profile), USERPROFILE=str(profile),
            XDG_CONFIG_HOME=str(profile / ".config"), XDG_DATA_HOME=str(profile / ".local/share"),
            XDG_CACHE_HOME=str(profile / ".cache"), XDG_STATE_HOME=str(profile / ".local/state"),
            XDG_RUNTIME_DIR=str(profile / ".runtime"), TMPDIR=str(temporary_files),
            TMP=str(temporary_files), TEMP=str(temporary_files), CARGO_TARGET_DIR=str(target),
            CARGO_HOME=str(cargo_home), CARGO_PROFILE_DEV_DEBUG="0", CARGO_PROFILE_TEST_DEBUG="0", CARGO_INCREMENTAL="0",
            RUSTUP_HOME=os.environ.get("RUSTUP_HOME", str(original_home / ".rustup")),
            RUSTUP_TOOLCHAIN=TOOLCHAIN, RUSTUP_AUTO_INSTALL="0", PATH=str(cargo.parent) + ":/usr/bin:/bin",
            MISE_AUTO_UPDATE="false", MISE_SELF_UPDATE_AUTO="false", MISE_AUTO_INSTALL="false", MISE_TASK_RUN_AUTO_INSTALL="false",
            CARGO_BUILD_JOBS="2", CMAKE_BUILD_PARALLEL_LEVEL="2", RUST_TEST_THREADS="2"))
        rustc_version = subprocess.check_output([str(cargo.parent / "rustc"), "--version"], text=True).strip()
        if not rustc_version.startswith("rustc " + TOOLCHAIN + " "):
            raise SystemExit("Native rustc differs from the pinned toolchain")
        code = bounded_tests(command, source, environment, cache, fixture_base, host_source, limits)
        staging.verify_source(source, expected_head=copied["head"],
                              expected_digest=copied["candidate_manifest_sha256"], enforce_modes=True)
        print(json.dumps({"filesystem": filesystem, "cargo": str(cargo), "target": str(target),
                          "fixtureHome": str(profile), "source": str(source), "rustc": rustc_version,
                          "sourceHead": copied["head"], "sourceTree": copied["git_tree"],
                          "sourceCopyManifestSha256": copied["candidate_manifest_sha256"],
                          "sourceFileCount": copied["file_count"], "sourceExecutablePaths": copied["executable_paths"],
                          "argv": command}), flush=True)
        return code


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cache-dir", type=Path, default=os.environ.get("DENDRO_LINUX_CACHE_DIR"))
    parser.add_argument("--wsl-distro", default=os.environ.get("DENDRO_WSL_DISTRO", "Ubuntu"))
    parser.add_argument("--wsl-child", action="store_true", help=argparse.SUPPRESS)
    parser.add_argument("--source-root", type=Path, help=argparse.SUPPRESS)
    parser.add_argument("--expected-head", help=argparse.SUPPRESS)
    parser.add_argument("--expected-source-sha256", help=argparse.SUPPRESS)
    positive = local_module("windows_guard").positive
    parser.add_argument("--min-free-bytes", type=positive, default=os.environ.get("DENDRO_LINUX_MIN_FREE_BYTES", os.environ.get("DENDRO_WINDOWS_MIN_FREE_BYTES", 10 * 1024 ** 3)))
    parser.add_argument("--cache-max-bytes", type=positive, default=os.environ.get("DENDRO_LINUX_CACHE_MAX_BYTES", 24 * 1024 ** 3))
    parser.add_argument("--workspace-max-bytes", type=positive, default=os.environ.get("DENDRO_LINUX_WORKSPACE_MAX_BYTES", 4 * 1024 ** 3))
    args = parser.parse_args()
    limits = (positive(args.min_free_bytes), positive(args.cache_max_bytes), positive(args.workspace_max_bytes))
    if args.wsl_child:
        if sys.platform != "linux":
            parser.error("WSL child requires Linux")
        if not args.source_root or not args.expected_head or not args.expected_source_sha256:
            parser.error("WSL child requires the sanitized source root and expected HEAD/digest")
        return linux_tests(args.source_root, args.cache_dir, limits,
                           {"head": args.expected_head, "candidate_manifest_sha256": args.expected_source_sha256})
    head = candidate()
    staging = local_module("source_stage")
    if sys.platform == "win32":
        prefix = ["wsl.exe", "--distribution", args.wsl_distro, "--exec"]
        script = subprocess.check_output([*prefix, "wslpath", "-u", str(Path(__file__).resolve())], text=True).strip()
        command = [*prefix, "/usr/bin/python3", "-I", script, "--wsl-child"]
        command += ["--min-free-bytes", str(limits[0]), "--cache-max-bytes", str(limits[1]), "--workspace-max-bytes", str(limits[2])]
        if args.cache_dir:
            if not args.cache_dir.as_posix().startswith("/"):
                parser.error("--cache-dir must be an absolute Linux path")
            command += ["--cache-dir", args.cache_dir.as_posix()]
        logs = ROOT / ".agent-runs/local-validation"
        logs.mkdir(parents=True, exist_ok=True)
        log = logs / ("native-linux-" + uuid.uuid4().hex + ".log")
        print(f"Native Linux candidate {head}; log: {log}", flush=True)
        stages = logs / "stages"
        stages.mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(prefix="native-linux-", dir=stages) as temporary:
            stage = Path(temporary).resolve()
            if not stage.is_relative_to(stages.resolve()):
                raise SystemExit("Source stage escaped the intended workspace")
            staged = staging.stage_source(stage)
            if staged["head"] != head:
                raise SystemExit("Candidate changed before source staging")
            source_linux = subprocess.check_output([*prefix, "wslpath", "-u", str(stage)], text=True).strip()
            command += ["--source-root", source_linux, "--expected-head", staged["head"],
                        "--expected-source-sha256", staged["candidate_manifest_sha256"]]
            with log.open("w", encoding="utf-8") as output:
                code = subprocess.run(command, cwd=ROOT, stdout=output, stderr=subprocess.STDOUT, timeout=7800).returncode
        print(log.read_text(encoding="utf-8", errors="replace")[-16000:], flush=True)
        metadata = child_metadata(log.read_text(encoding="utf-8", errors="replace"))
        unchanged = candidate() == head
        receipt = receipt_result(head, staged, metadata, code, unchanged)
        receipt["command"] = command
        (log.with_suffix(".json")).write_text(json.dumps(receipt, indent=2), encoding="utf-8")
        code = receipt["exitCode"]
    elif sys.platform == "linux":
        stages = ROOT / ".agent-runs/local-validation/stages"
        stages.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(prefix="native-linux-", dir=stages) as temporary:
            stage = Path(temporary).resolve()
            staged = staging.stage_source(stage)
            if staged["head"] != head:
                raise SystemExit("Candidate changed before source staging")
            code = linux_tests(stage, args.cache_dir, limits, staged)
    else:
        raise SystemExit("Native Linux acceptance needs Linux or Windows with WSL")
    if candidate() != head:
        raise SystemExit("Candidate changed during native Linux acceptance")
    return code


if __name__ == "__main__":
    sys.exit(main())
