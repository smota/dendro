"""Samuel's performance-only exception for this validation tooling migration."""
import hashlib
from pathlib import Path
import subprocess

ENVIRONMENT_KEY = "DENDRO_VALIDATION_PERF_WAIVER"
TOKEN = "local-validation-163"
ANCHOR = "836335bff01a8287b30ec61b925d9e210be6b7b0"
TEST_FILE = "crates/dendro-cli/tests/advise.rs"
# Bound the allowed Rust test delta to the reviewed performance-only switch.
TEST_SHA256 = "315ff42a23f61377e2362f974f657ad86a4b7257ec7cfd235841cf29dce017b3"
ALLOWED_FILES = {".gitattributes", ".gitignore", "CONTRIBUTING.md", "agent-workflow.config.json",
                 "lefthook.yml", "mise.toml", TEST_FILE}
ALLOWED_PREFIXES = (".github/workflows/", "docs/validation/", "tools/validation/")


def decision(root):
    root = Path(root)
    def git(*args):
        return subprocess.check_output(["git", *args], cwd=root, text=True).strip()
    try:
        if git("branch", "--show-current") != "work/local-validation":
            return {"enabled": False, "reason": "not the approved migration branch"}
        subprocess.run(["git", "merge-base", "--is-ancestor", ANCHOR, "HEAD"], cwd=root, check=True,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        if git("status", "--porcelain"):
            return {"enabled": False, "reason": "candidate is not clean"}
        base = git("merge-base", "origin/development", ANCHOR)
        changed = git("diff", "--name-only", base, "HEAD").splitlines()
        if any(name not in ALLOWED_FILES and not name.startswith(ALLOWED_PREFIXES) for name in changed):
            return {"enabled": False, "reason": "changes extend beyond validation tooling"}
        content = (root / TEST_FILE).read_bytes().replace(b"\r\n", b"\n")
        if hashlib.sha256(content).hexdigest() != TEST_SHA256:
            return {"enabled": False, "reason": "performance-only test delta changed"}
        return {"enabled": True, "id": TOKEN, "reason": "Samuel authorized ignoring performance requirements for this migration",
                "head": git("rev-parse", "HEAD"), "base": base, "changed": changed,
                "functionalAssertionsRequired": True}
    except (OSError, subprocess.SubprocessError):
        return {"enabled": False, "reason": "cannot establish the approved migration scope"}
