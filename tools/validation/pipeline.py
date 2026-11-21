# /// script
# requires-python = ">=3.12"
# dependencies = ["dagger-io==0.21.12"]
# ///
"""Isolated Linux Rust gates. No Cloud, host home, git history or runtime databases."""
import asyncio
from pathlib import Path
import sys
import subprocess
import shutil
import tempfile
import hashlib
import json

import dagger
from verify import RUST_GATES

ROOT = Path(__file__).resolve().parents[2]


def stage_source(destination):
    candidates = subprocess.check_output(
        ["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"], cwd=ROOT,
    ).decode().split("\0")
    roots = ("crates/", "docs/", "examples/", "scripts/", "site/", "templates/", ".agentflow/oracles/", "tools/validation/")
    files = {"Cargo.toml", "Cargo.lock", "README.md", "LICENSE", "NOTICE", ".gitattributes", "mise.toml", "lefthook.yml", "package.json", "pnpm-lock.yaml"}
    manifest = []
    for name in sorted(set(candidates) - {""}):
        if name not in files and not name.startswith(roots):
            continue
        relative = Path(name)
        if any(part in {".git", "node_modules", "target", "private", "secrets", "__pycache__"}
               or part == ".env" or part.startswith(".env.") for part in relative.parts):
            continue
        if any(token in relative.name for token in (".duckdb", ".sqlite")) or relative.suffix in {".key", ".pem"}:
            continue
        source = ROOT / relative
        if not source.is_file():
            continue
        if not source.resolve().is_relative_to(ROOT) or any(
            path.is_symlink() for path in (source, *source.parents) if path != ROOT
        ):
            raise ValueError(f"refusing symlink or escaped validation input: {name}")
        target = destination / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, target)
        manifest.append((name, hashlib.sha256(target.read_bytes()).hexdigest()))
    head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    dirty = subprocess.check_output(["git", "status", "--porcelain", "-z"], cwd=ROOT)
    print(json.dumps({"head": head, "dirty": bool(dirty), "file_count": len(manifest),
                      "candidate_manifest_sha256": hashlib.sha256(json.dumps(manifest).encode()).hexdigest()}), flush=True)


async def verify(stage):
    async with dagger.Connection(dagger.Config(
        log_output=sys.stderr, timeout=dagger.Timeout(60, connect=30, read=7200),
    )) as client:
        # The only host upload is the freshly sanitized candidate stage.
        source = client.host().directory(str(stage))
        print(f"Dagger candidate source digest: {await source.digest()}", flush=True)
        container = (client.container().from_("rust:1.98.1-bookworm")
            .with_exec(["apt-get", "update"])
            .with_exec(["apt-get", "install", "-y", "--no-install-recommends", "cmake", "clang"])
            .with_exec(["rustup", "component", "add", "rustfmt", "clippy"])
            .with_exec(["useradd", "--create-home", "--uid", "1000", "validator"])
            .with_exec(["chmod", "700", "/home/validator"])
            .with_directory("/src", source, owner="validator")
            .with_workdir("/src")
            .with_user("validator")
            .with_env_variable("HOME", "/home/validator")
            .with_env_variable("CARGO_BUILD_JOBS", "2")
            .with_env_variable("CMAKE_BUILD_PARALLEL_LEVEL", "2")
            .with_env_variable("CARGO_PROFILE_DEV_DEBUG", "0")
            .with_env_variable("CARGO_PROFILE_TEST_DEBUG", "0")
            .with_env_variable("RUST_TEST_THREADS", "2")
            .with_env_variable("CARGO_TERM_COLOR", "never")
            .with_mounted_cache("/usr/local/cargo/registry", client.cache_volume("dendro-rust-1.98.1-registry"), owner="validator")
            .with_mounted_cache("/src/target", client.cache_volume("dendro-linux-rust-1.98.1-target"), owner="validator"))
        for command in RUST_GATES:
            container = container.with_exec(command)
        await container.sync()
        print("Linux container Rust gates passed; native historical/cross-volume certification is separate.")


async def main():
    with tempfile.TemporaryDirectory(prefix="dendro-validation-") as stage:
        stage_source(Path(stage))
        await verify(stage)


if __name__ == "__main__":
    async def bounded():
        async with asyncio.timeout(7200):
            await main()
    asyncio.run(bounded())
