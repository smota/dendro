"""Run the production Rust canonical-fact benchmark; no separate prototype schema."""
import pathlib
import subprocess

root = pathlib.Path(__file__).resolve().parents[2]
subprocess.run(
    ["cargo", "test", "-p", "dendro-core", "--test", "benchmark_nao", "--", "--nocapture"],
    cwd=root,
    check=True,
)
