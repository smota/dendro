"""Stage classified committed Git blobs, never checkout or agent-home bytes."""
from collections import Counter
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import stat
import subprocess

ROOT = Path(__file__).resolve().parents[2]
MANIFEST = ".validation-source-manifest.json"
PUBLIC_ROOTS = ("adapters/", "crates/", "docs/", "examples/", "goals/", "lib/",
                "manifests/", "openrig/", "roles/", "schemas/", "scripts/", "site/",
                "skills/", "templates/", "transitions/", "tools/validation/", ".agentflow/oracles/")
PUBLIC_FILES = {"Cargo.toml", "Cargo.lock", "README.md", "CONTRIBUTING.md", "CODE_OF_CONDUCT.md",
                "SECURITY.md", "LICENSE", "NOTICE", ".gitattributes", ".gitignore", "mise.toml",
                "lefthook.yml", "package.json", "pnpm-lock.yaml", "pnpm-workspace.yaml",
                "agent-framework-lock.json", "agent-workflow.config.json", "sdlc.config.json",
                "goal.json", "gates.json", "rustfmt.toml", ".rustfmt.toml", "clippy.toml",
                ".clippy.toml", "rust-toolchain", "rust-toolchain.toml", "deny.toml",
                ".cargo/config", ".cargo/config.toml"}
AGENT_ROOTS = {".agy", ".claude", ".codex", ".gemini", ".grok", ".pi", ".agents", ".openrig"}
AGENT_FILES = {"AGENTS.md", "CLAUDE.md", "CODEX.md", "GEMINI.md", "sibling.md",
               "SITE_BUILDER_HANDOVER.md", ".agentflow-adapters.lock.json"}
AGENTFLOW_STATE_ROOTS = {"community", "evidence", "proposals", "runs", "world", "roles"}
AGENTFLOW_STATE_FILES = {"PRODUCT.md", "capabilities.catalog.yaml", "invariants.catalog.yaml",
                        "journeys.catalog.yaml", "execution-policy.json", "harness-parameters.json",
                        "method-catalog.json", "model-catalog.json", "orchestration-model.json",
                        "role-catalog.json"}


def _relative(name):
    path = PurePosixPath(name)
    if not name or path.is_absolute() or "\\" in name or ":" in name or any(
        part in ("", ".", "..") for part in name.split("/")
    ):
        raise ValueError(f"Unsafe validation input path: {name!r}")
    return path


def input_category(name):
    path = _relative(name)
    if ".git" in path.parts or path.parts[0] == ".githooks":
        return "excluded:vcs-hooks"
    if path.parts[0] in AGENT_ROOTS or name in AGENT_FILES:
        return "excluded:agent-installation-instructions"
    if path.parts[0] == ".github":
        return "excluded:hosted-service-metadata"
    if path.parts[0] == ".agentflow" and len(path.parts) > 1 and (
        path.parts[1] in AGENTFLOW_STATE_ROOTS or name[11:] in AGENTFLOW_STATE_FILES
    ):
        return "excluded:agent-operational-state"
    if any(part in {"private", "secrets", ".ssh", ".aws", ".azure"} for part in path.parts) or any(
        part == ".env" or part.startswith(".env.") for part in path.parts
    ) or path.suffix.lower() in {".key", ".pem", ".p12", ".pfx"} or name in {
        ".cargo/credentials", ".cargo/credentials.toml"
    } or any(token in path.name.lower() for token in (".duckdb", ".sqlite")):
        return "excluded:private-credentials-databases"
    if any(part in {"node_modules", "target", "__pycache__", ".venv", ".agent-runs"} for part in path.parts):
        return "excluded:generated-runtime-output"
    if name in PUBLIC_FILES or name.startswith(PUBLIC_ROOTS):
        return "public"
    raise ValueError(f"Unclassified tracked validation input: {name}")


def _git(root, *args):
    return subprocess.check_output(["git", "--no-optional-locks", *args], cwd=root)


def clean_head(root):
    head = _git(root, "rev-parse", "--verify", "HEAD^{commit}").decode().strip()
    if _git(root, "status", "--porcelain", "-z", "--untracked-files=all"):
        raise ValueError("Committed source staging requires a clean candidate")
    return head


def tracked_inputs(root, head):
    admitted, excluded = [], Counter()
    for record in _git(root, "ls-tree", "-r", "-z", head).split(b"\0"):
        if not record:
            continue
        fields, raw_name = record.split(b"\t", 1)
        mode, kind, oid = fields.decode("ascii").split()
        name = raw_name.decode("utf-8")
        category = input_category(name)
        if category != "public":
            excluded[category] += 1
            continue
        if kind != "blob" or mode not in ("100644", "100755"):
            raise ValueError(f"Refusing non-regular Git input ({mode} {kind}): {name}")
        admitted.append((name, mode, oid))
    return sorted(admitted), dict(sorted(excluded.items()))


def _ordinary(path):
    for component in (*reversed(path.parents), path):
        info = component.lstat()
        if stat.S_ISLNK(info.st_mode) or getattr(info, "st_file_attributes", 0) & 0x400:
            raise ValueError(f"Refusing source-stage link/reparse point: {component}")


def _digest(payload):
    return hashlib.sha256(json.dumps(payload, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def _summary(payload):
    return {"head": payload["head"], "git_tree": payload["tree"], "dirty": False,
            "file_count": len(payload["entries"]), "candidate_manifest_sha256": _digest(payload),
            "executable_paths": [entry[0] for entry in payload["entries"] if entry[1] == "100755"]}


def stage_source(destination, root=None):
    root = ROOT if root is None else Path(root)
    head = clean_head(root)
    entries, excluded = tracked_inputs(root, head)
    destination = Path(destination).absolute()
    _ordinary(destination)
    if not destination.is_dir() or any(destination.iterdir()):
        raise ValueError("Source stage must be an existing empty ordinary directory")
    payload = {"version": 1, "head": head,
               "tree": _git(root, "rev-parse", head + "^{tree}").decode().strip(), "entries": []}
    # One Git process; drain each response before submitting the next request.
    with subprocess.Popen(["git", "--no-optional-locks", "cat-file", "--batch"], cwd=root,
                          stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE) as batch:
        try:
            for name, mode, oid in entries:
                batch.stdin.write((oid + "\n").encode("ascii"))
                batch.stdin.flush()
                header = batch.stdout.readline().decode("ascii").split()
                if len(header) != 3 or header[:2] != [oid, "blob"]:
                    raise ValueError(f"Git blob response does not match committed input: {name}")
                remaining = int(header[2])
                target = destination / name
                target.parent.mkdir(parents=True, exist_ok=True)
                checksum = hashlib.sha256()
                with target.open("xb") as output:
                    while remaining:
                        chunk = batch.stdout.read(min(remaining, 1024 * 1024))
                        if not chunk:
                            raise ValueError(f"Truncated committed source blob: {name}")
                        output.write(chunk)
                        checksum.update(chunk)
                        remaining -= len(chunk)
                if batch.stdout.read(1) != b"\n":
                    raise ValueError(f"Invalid Git blob terminator: {name}")
                target.chmod(0o700 if mode == "100755" else 0o600)
                payload["entries"].append([name, mode, oid, checksum.hexdigest()])
            batch.stdin.close()
            error = batch.stderr.read().decode(errors="replace")
            if batch.wait():
                raise ValueError(f"Cannot read committed validation blobs: {error}")
        finally:
            if batch.poll() is None:
                batch.kill()
                batch.wait()
    (destination / MANIFEST).write_text(json.dumps(payload, sort_keys=True), encoding="utf-8")
    if clean_head(root) != head:
        raise ValueError("Candidate HEAD changed during source staging")
    result = verify_source(destination, expected_head=head, expected_digest=_digest(payload))
    result["excluded_categories"] = excluded
    print(json.dumps(result), flush=True)
    return result


def verify_source(directory, *, expected_head=None, expected_digest=None, enforce_modes=False):
    """No Git operations: verify complete transported source and frozen modes."""
    directory = Path(directory).absolute()
    _ordinary(directory / MANIFEST)
    payload = json.loads((directory / MANIFEST).read_text(encoding="utf-8"))
    if not isinstance(payload, dict) or set(payload) != {"version", "head", "tree", "entries"} or payload["version"] != 1:
        raise ValueError("Invalid committed-source manifest schema")
    for key in ("head", "tree"):
        if not isinstance(payload[key], str) or not re.fullmatch(r"[0-9a-f]{40}|[0-9a-f]{64}", payload[key]):
            raise ValueError(f"Invalid committed-source {key}")
    entries = payload["entries"]
    if not isinstance(entries, list) or not all(isinstance(entry, list) and len(entry) == 4 and
                                             all(isinstance(value, str) for value in entry) for entry in entries):
        raise ValueError("Invalid committed-source entries")
    names = [entry[0] for entry in entries]
    if names != sorted(set(names)):
        raise ValueError("Committed-source paths must be unique and sorted")
    def scan_error(error):
        raise error
    actual_names = set()
    for root, directories, files in os.walk(directory, followlinks=False, onerror=scan_error):
        for name in directories + files:
            path = Path(root) / name
            info = path.lstat()
            if stat.S_ISLNK(info.st_mode) or getattr(info, "st_file_attributes", 0) & 0x400:
                raise ValueError(f"Refusing transported source link/reparse point: {path}")
            if stat.S_ISREG(info.st_mode):
                actual_names.add(path.relative_to(directory).as_posix())
            elif not stat.S_ISDIR(info.st_mode):
                raise ValueError(f"Refusing non-regular transported input: {path}")
    if actual_names != set(names) | {MANIFEST}:
        raise ValueError("Transported source paths differ from the committed manifest")
    for name, mode, oid, sha256 in entries:
        if input_category(name) != "public" or mode not in ("100644", "100755") or not re.fullmatch(
            r"[0-9a-f]{40}|[0-9a-f]{64}", oid
        ) or not re.fullmatch(r"[0-9a-f]{64}", sha256):
            raise ValueError(f"Invalid or private committed-source input: {name}")
        path = directory / name
        blob = hashlib.new("sha1" if len(oid) == 40 else "sha256")
        blob.update(f"blob {path.stat().st_size}\0".encode("ascii"))
        checksum = hashlib.sha256()
        with path.open("rb") as source:
            while chunk := source.read(1024 * 1024):
                blob.update(chunk)
                checksum.update(chunk)
        if blob.hexdigest() != oid or checksum.hexdigest() != sha256:
            raise ValueError(f"Transported bytes differ from the committed Git blob: {name}")
        if enforce_modes and stat.S_IMODE(path.stat().st_mode) != (0o700 if mode == "100755" else 0o600):
            raise ValueError(f"Transported executable mode differs from Git: {name}")
    summary = _summary(payload)
    if expected_head is not None and summary["head"] != expected_head:
        raise ValueError("Transported source HEAD does not match the candidate")
    if expected_digest is not None and summary["candidate_manifest_sha256"] != expected_digest:
        raise ValueError("Transported source manifest does not match the candidate")
    return summary


def apply_source_modes(directory):
    """Apply transported Git modes on the Linux private copy."""
    summary = verify_source(directory)
    directory = Path(directory)
    executables = set(summary["executable_paths"])
    for root, directories, files in os.walk(directory):
        Path(root).chmod(0o700)
        for name in files:
            path = Path(root) / name
            path.chmod(0o700 if path.relative_to(directory).as_posix() in executables else 0o600)
    return summary
