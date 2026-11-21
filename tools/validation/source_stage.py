"""Shared sanitized source input for Dagger and native Linux validation."""
from pathlib import Path
import subprocess
import shutil
import hashlib
import json

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
    result = {"head": head, "dirty": bool(dirty), "file_count": len(manifest),
              "candidate_manifest_sha256": hashlib.sha256(json.dumps(manifest).encode()).hexdigest()}
    print(json.dumps(result), flush=True)
    return result
