# /// script
# requires-python = ">=3.13,<3.15"
# dependencies = []
# ///
"""Run authoritative native checks without starting Dendro or installing tools."""
import argparse
import os
from pathlib import Path
import subprocess
import sys
import tomllib
import re

ROOT = Path(__file__).resolve().parents[2]
RUST_GATES = [
    ["cargo", "fmt", "--check"],
    ["cargo", "check", "--workspace", "--locked"],
    ["cargo", "clippy", "--workspace", "--locked", "--", "-D", "warnings"],
    ["cargo", "test", "--workspace", "--locked"],
]
GOVERNANCE = [
    "validate-path-ownership.mjs", "validate-role-catalog.mjs",
    "validate-skill-catalog.mjs", "verify-architecture-guardrails.mjs",
    "verify-design-guardrails.mjs",
]
GOVERNANCE_TESTS = [
    "scripts/__tests__/validate-dependency-automation.test.mjs",
    "scripts/__tests__/ci-scope.test.mjs",
]


GUARD = None


def execute(command, **kwargs):
    if GUARD is not None:
        return GUARD.run(command, cwd=ROOT, **kwargs)
    return subprocess.run(command, cwd=ROOT, **kwargs)


def run(command):
    print("+ " + " ".join(command), flush=True)
    execute(command, check=True)


def require_clean_candidate():
    head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    if subprocess.check_output(["git", "status", "--porcelain", "-z"], cwd=ROOT):
        raise SystemExit("Native acceptance requires a clean committed candidate; working-tree edits are not certified")
    print(f"Native acceptance candidate: {head}", flush=True)
    return head


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    group = parser.add_mutually_exclusive_group()
    group.add_argument("--fast", action="store_true")
    group.add_argument("--windows", action="store_true")
    group.add_argument("--native-fixture", action="store_true")
    group.add_argument("--release-candidate", action="store_true")
    group.add_argument("--dependency-automation", action="store_true")
    parser.add_argument("--base", choices=("main", "development"), default="development")
    args = parser.parse_args()
    if args.windows and sys.platform != "win32":
        parser.error("verify:windows must run on native Windows")
    if not args.fast:
        require_clean_candidate()
    os.environ.setdefault("RUSTUP_TOOLCHAIN", "1.98.1")
    os.environ.setdefault("CARGO_BUILD_JOBS", "2")
    os.environ.setdefault("CMAKE_BUILD_PARALLEL_LEVEL", "2")
    os.environ.setdefault("RUST_TEST_THREADS", "2")
    if GUARD is not None:
        for key in ("RUSTUP_TOOLCHAIN", "CARGO_BUILD_JOBS", "CMAKE_BUILD_PARALLEL_LEVEL", "RUST_TEST_THREADS"):
            if key in os.environ:
                GUARD.env[key] = os.environ[key]
    def dependency_automation():
        os.environ["BASE_REF"] = args.base
        if GUARD is not None:
            GUARD.env["BASE_REF"] = args.base
        run(["node", "scripts/validate-dependency-automation.mjs"])
    if args.dependency_automation:
        dependency_automation()
        return
    if args.release_candidate:
        version = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]
        if not (ROOT / "docs" / f"RELEASE_NOTES_v{version}.md").is_file():
            parser.error(f"missing docs/RELEASE_NOTES_v{version}.md")
        run(["cargo", "check", "--workspace", "--locked"])
        return
    if args.native_fixture:
        paths = []
        for key in ("FOUNDATION_CROSS_VOLUME_SOURCE", "FOUNDATION_CROSS_VOLUME_DEST"):
            value = os.environ.get(key)
            if not value or not Path(value).is_absolute() or not Path(value).is_dir():
                parser.error(f"{key} must name an existing private disposable absolute directory")
            path = Path(value).resolve(strict=True)
            if any(path.iterdir()) or path == Path(path.anchor) or path == Path.home():
                parser.error(f"{key} must be an empty disposable directory, never a root or home")
            paths.append(path)
        if paths[0] == paths[1] or paths[0] in paths[1].parents or paths[1] in paths[0].parents:
            parser.error("fixture directories must be distinct and disjoint")
        command = ["cargo", "test", "-p", "dendro-core", "--locked", "--test",
             "native_foundation_acceptance", "cross_volume_closed_copy_then_local_pointer_publication",
             "--", "--ignored", "--exact", "--nocapture"]
        result = execute(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        print(result.stdout, end="", flush=True)
        result.check_returncode()
        if not all(marker in result.stdout for marker in (
            "T24-29 actual distinct-device copy, private destination, unchanged closed source and verified local pointer: passed",
            "1 passed; 0 failed",
        )):
            raise SystemExit("Native cross-volume acceptance failed: required positive markers absent (zero tests is not acceptance)")
        return
    for command in RUST_GATES[:1] if args.fast else RUST_GATES:
        run(command)
    if not args.windows:
        branch = subprocess.check_output(["git", "branch", "--show-current"], cwd=ROOT, text=True).strip()
        if args.base == "main" and not branch.startswith(("rc/v", "hotfix/")):
            parser.error("direct feature promotion to main is forbidden; use rc/v* or hotfix/*")
        if args.base == "development":
            base = subprocess.check_output(["git", "merge-base", "origin/development", "HEAD"], cwd=ROOT, text=True).strip()
            diff = subprocess.check_output(["git", "diff", base, "--", "Cargo.toml"], cwd=ROOT, text=True)
            if re.search(r"^\+version\s*=", diff, re.MULTILINE):
                parser.error("incidental SemVer bump targeting development; use rc/v* or hotfix/*")
        run(["pnpm", "install", "--frozen-lockfile"])
        for script in GOVERNANCE_TESTS:
            run(["node", "--test", script])
        if branch.startswith("dependabot/"):
            dependency_automation()
        for script in GOVERNANCE:
            run(["node", f"scripts/{script}"])
        run(["node", "--test", "site/site.test.cjs"])
    print("Native historical and real cross-volume acceptance remain separate required release evidence.")


if __name__ == "__main__":
    if sys.platform == "win32":
        from windows_guard import GuardError, WindowsGuard
        try:
            with WindowsGuard(ROOT) as guard:
                GUARD = guard
                main()
        except GuardError as error:
            raise SystemExit(str(error)) from error
    else:
        main()
