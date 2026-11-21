# /// script
# requires-python = ">=3.12"
# dependencies = ["dagger-io==0.21.12"]
# ///
"""Isolated Linux Rust gates. No Cloud, host home, git history or runtime databases."""
import asyncio
from pathlib import Path
import sys
import tempfile

import dagger
from verify import RUST_GATES
from source_stage import stage_source

ROOT = Path(__file__).resolve().parents[2]


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
        build_gates = {command[1]: command for command in RUST_GATES if command[:2] != ["cargo", "test"]}
        if len(build_gates) != 3 or set(build_gates) != {"fmt", "check", "clippy"}:
            raise RuntimeError("Classify changed Rust gates before container validation")
        for name in ("fmt", "check", "clippy"):
            container = container.with_exec(build_gates[name])
        await container.sync()
        print("Linux container format/check/Clippy gates passed; full tests require the mandatory native Linux leg.")


async def main():
    with tempfile.TemporaryDirectory(prefix="dendro-validation-") as stage:
        stage_source(Path(stage))
        await verify(stage)


if __name__ == "__main__":
    async def bounded():
        async with asyncio.timeout(7200):
            await main()
    asyncio.run(bounded())
