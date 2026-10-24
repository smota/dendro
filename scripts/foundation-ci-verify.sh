#!/usr/bin/env bash
# Scoped native foundation proof. Does not certify T27 or authorize publication.
set -euo pipefail
umask 077

phase="${1:?expected prepare, verify, historical or collect}"
root="${2:?expected isolated foundation directory}"
receipts="$root/receipts"
test -d "$receipts"

# A synchronous pipeline waits for tee before recording the phase exit. A killed
# worker leaves .started without .exit, which the final collector treats as incomplete.
case "$phase" in
  prepare|verify|historical|native-fixtures)
    case "$phase" in
      prepare) log=environment ;;
      verify) log=gates ;;
      historical) log=historical-ci ;;
      native-fixtures) log=native-fixtures-setup ;;
    esac
    date -u +%FT%TZ > "$receipts/$phase.started"
    set +e
    bash "$0" "$phase-inner" "$root" 2>&1 | tee "$receipts/$log.log"
    statuses=("${PIPESTATUS[@]}")
    set -e
    status="${statuses[0]}"
    if [[ "${statuses[1]}" != 0 ]]; then status="${statuses[1]}"; fi
    printf '%s\n' "$status" > "$receipts/$phase.exit"
    exit "$status"
    ;;
  *-inner) phase="${phase%-inner}" ;;
esac

if [[ "$phase" == collect ]]; then
  # Only explicit top-level proof logs/JSON: never databases, keys, archives or binaries.
  if [[ -d "$root/historical" ]]; then
    mkdir -p "$receipts/historical"
    find "$root/historical" -maxdepth 1 -type f \
      \( -name '*.stdout' -o -name '*.stderr' -o -name 'receipt.json' \) \
      -exec cp -- {} "$receipts/historical/" \;
  fi
  python3 - "$receipts" <<'PY'
import json
from pathlib import Path
import sys

receipts = Path(sys.argv[1])
def read_json(path):
    try:
        return json.loads(path.read_text())
    except (FileNotFoundError, json.JSONDecodeError):
        return {}

checks = {}
for name in ("prepare", "toolchain", "verify", "fmt", "check", "test", "clippy", "historical", "native-fixtures"):
    exit_file = receipts / f"{name}.exit"
    try:
        code = int(exit_file.read_text().strip())
    except (FileNotFoundError, ValueError):
        code = None
    checks[name] = {"status": "passed" if code == 0 else "failed" if code is not None else "not_run_or_interrupted", "exit_code": code}
fixture_log = receipts / "native-fixtures.log"
fixture_text = fixture_log.read_text() if fixture_log.exists() else ""
fixture_markers = all(marker in fixture_text for marker in (
    "T24-29 actual distinct-device copy, private destination, unchanged closed source and verified local pointer: passed",
    "1 passed; 0 failed",
))
if (receipts / "native-fixtures-notrun.json").exists():
    checks["native-fixtures"]["status"] = "not_run_prerequisites_absent"
elif checks["native-fixtures"]["exit_code"] == 0 and not fixture_markers:
    checks["native-fixtures"]["status"] = "failed_positive_markers_absent"
validation = receipts / "historical-ci-validation.json"
historical_validated = read_json(validation).get("passed") is True
passed = all(item["status"] == "passed" for item in checks.values()) and historical_validated and fixture_markers
source = receipts / "candidate-source.json"
verdict = {
    "schema": "dendro.foundation-ci-verdict/v1", "passed": passed,
    "status": "passed" if passed else "failed_or_incomplete",
    "scope": "native Linux foundation validation only; not T27 certification or release approval",
    "candidate": read_json(source) or None,
    "checks": checks, "historical_receipt_validated": historical_validated,
    "native_fixture_positive_markers": fixture_markers,
    "historical_harness_acceptance": "harness keeps native_hosted_linux_acceptance false; wrapper verifies scoped hosted evidence only",
}
(receipts / "verdict.json").write_text(json.dumps(verdict, indent=2) + "\n")
print(json.dumps(verdict, indent=2))
PY
  exit 0
fi

if [[ "$phase" == native-fixtures ]]; then
  if [[ ! -f "$receipts/prepare.exit" || "$(cat "$receipts/prepare.exit")" != 0 ||
        ! -f "$receipts/toolchain.exit" || "$(cat "$receipts/toolchain.exit")" != 0 ||
        ! -f "$root/source/scripts/foundation-ci-native-fixtures.sh" ]]; then
    printf '{"status":"not_run_prerequisites_absent","passed":false}\n' > "$receipts/native-fixtures-notrun.json"
    echo 'Native fixture not run: admitted source and pinned toolchain are required.'
    exit 1
  fi
  bash "$root/source/scripts/foundation-ci-native-fixtures.sh" "$root"
  exit 0
fi

if [[ "$phase" == historical ]]; then
  python3 --version
  # The harness overlays candidate files: require them to equal the recorded commit.
  test "$(git -C "$GITHUB_WORKSPACE" rev-parse HEAD)" = "$(cat "$receipts/candidate.commit")"
  test -z "$(git -C "$GITHUB_WORKSPACE" status --porcelain --untracked-files=all -- Cargo.toml Cargo.lock crates scripts/historical_migration_proof.py scripts/fixtures .agentflow/oracles/v080/T04)"
  git -C "$GITHUB_WORKSPACE" cat-file -e '6b755135f95e497071790e8e5e358995a8244a18^{commit}'
  status=0
  python3 "$GITHUB_WORKSPACE/scripts/historical_migration_proof.py" \
    --repo "$GITHUB_WORKSPACE" --scratch "$root/historical" \
    --target-dir "$root/historical-targets" || status=$?
  printf '%s\n' "$status" > "$receipts/historical-harness.exit"
  test "$status" = 0
  python3 - "$GITHUB_WORKSPACE" "$root/historical/receipt.json" "$receipts" <<'PY'
import hashlib
import json
from pathlib import Path
import subprocess
import sys

repo, receipt_path, receipts = map(Path, sys.argv[1:])
proof = json.loads(receipt_path.read_text())
candidate = (receipts / "candidate.commit").read_text().strip()
historical = "6b755135f95e497071790e8e5e358995a8244a18"
assert proof["passed"] is True
assert proof["execution_environment"] == "native-linux"
assert proof["historical_commit"] == historical
assert proof["historical_release_artifact"] is False
assert proof["historical_binary_version"] == "dendro 0.7.0"
assert proof["historical_schema"] == 3
assert proof["historical_report_same_after_rollback"] is True
assert proof["historical_report_same_after_selected_rollback"] is True
labels = ("oracle", "historical-produced", "selected-oracle", "selected-historical-produced")
modes = ("open_existing", "open_staged", "open_read_only", "cli-report")
cases = proof["historical_refusal_cases"]
assert len(cases) == 16
assert {(case["fixture"], case["mode"]) for case in cases} == {(label, mode) for label in labels for mode in modes}
assert all(case["target_kind"] == ("actual_selected_file" if case["fixture"].startswith("selected-") else "stage_private_copy") for case in cases)
assert all(case["refused"] and case["canonical_unchanged"] for case in proof["historical_refusal_cases"])
negative = proof["old_anchor_negative_control"]
assert negative["old_anchor_write_divergence_detected"] is True
assert negative["selected_database_unchanged"] is True
# Same accepted refusals as scripts/fixtures/migration_proof_driver.rs.
assert ("legacy_anchor_diverged_after_selection" in negative["error"]
        or "logical anchor changed or disappeared after selection binding" in negative["error"])

def blob_hash(commit, path):
    data = subprocess.check_output(["git", "-C", str(repo), "show", f"{commit}:{path}"])
    return hashlib.sha256(data).hexdigest()

assert proof["historical_lock_sha256"] == blob_hash(historical, "Cargo.lock")
assert proof["historical_storage_sha256"] == blob_hash(historical, "crates/dendro-core/src/storage.rs")
historical_tree = subprocess.check_output(["git", "-C", str(repo), "rev-parse", f"{historical}^{{tree}}"], text=True).strip()
assert proof["historical_tree"] == historical_tree
with subprocess.Popen(["git", "-C", str(repo), "archive", "--format=tar", historical], stdout=subprocess.PIPE) as archive:
    archive_hash = hashlib.file_digest(archive.stdout, "sha256").hexdigest()
    assert archive.wait() == 0, "historical archive failed"
assert proof["historical_source_archive_sha256"] == archive_hash
target_base = receipt_path.parent.parent / "historical-targets"
assert proof["cargo_target_dirs"] == {"historical": str(target_base / "historical"), "candidate": str(target_base / "candidate")}
assert proof["oracle_sha256"] == blob_hash(candidate, ".agentflow/oracles/v080/T04/frozen.json")
for field in ("candidate_files", "harness_files", "frozen_fixture_files"):
    assert proof[field], f"empty {field}"
    for path, digest in proof[field].items():
        assert digest == blob_hash(candidate, path), f"source mismatch: {path}"
(receipts / "historical-ci-validation.json").write_text(json.dumps({
    "schema": "dendro.foundation-historical-ci/v1", "passed": True,
    "candidate_commit": candidate, "historical_commit": historical,
    "scope": "native Linux foundation historical migration proof; not T27 release certification",
    "receipt_sha256": hashlib.sha256(receipt_path.read_bytes()).hexdigest(),
}, indent=2) + "\n")
print("Historical proof passed; receipt source hashes match committed candidate and historical source.")
PY
  exit 0
fi

if [[ "$phase" == prepare ]]; then
  printf 'scope=native Linux foundation only\n'
  printf 'run_id=%s\nrun_attempt=%s\n' "$GITHUB_RUN_ID" "$GITHUB_RUN_ATTEMPT"
  printf 'event=%s\nref=%s\nevent_sha=%s\n' "$GITHUB_EVENT_NAME" "$GITHUB_REF" "$GITHUB_SHA"
  printf 'runner_os=%s\nrunner_arch=%s\nimage_os=%s\nimage_version=%s\n' \
    "$RUNNER_OS" "$RUNNER_ARCH" "${ImageOS:-unknown}" "${ImageVersion:-unknown}"
  uname -a
  cat /etc/os-release
  id
  findmnt -T "$root" -o TARGET,SOURCE,FSTYPE,OPTIONS
  stat -c 'path=%n owner=%u group=%g mode=%a' "$root" "$root/source" "$root/tmp"
  test "$(uname -s)" = Linux
  test "$(uname -m)" = x86_64
  if grep -qiE 'microsoft|wsl' /proc/sys/kernel/osrelease /proc/version; then
    echo 'Refused: WSL is not native Linux evidence.'
    exit 1
  fi
  test "$(findmnt -n -o FSTYPE -T "$root")" = ext4
  test "$(stat -c %u "$root")" = "$(id -u)"
  test "$(stat -c %a "$root")" = 700
  candidate="$(git rev-parse HEAD)"
  # PR runs validate the checked-out merge commit; push runs validate event SHA.
  test "$candidate" = "$GITHUB_SHA"
  printf '%s\n' "$candidate" > "$receipts/candidate.commit"
  python3 - "$receipts/candidate-source.json" "$candidate" <<'PY'
import json
import os
from pathlib import Path
import sys

event = os.environ["GITHUB_EVENT_NAME"]
Path(sys.argv[1]).write_text(json.dumps({
    "commit": sys.argv[2], "event": event, "ref": os.environ["GITHUB_REF"],
    "source_kind": "pull_request_merge" if event == "pull_request" else "pushed_head" if event == "push" else "dispatch_selected_ref",
    "pr_head_commit": os.environ.get("PR_HEAD_SHA") or None,
}, indent=2) + "\n")
PY
  git rev-parse HEAD^{tree} > "$receipts/candidate.tree"
  git ls-tree -r --full-tree HEAD > "$receipts/source-tree.txt"
  git archive --format=tar HEAD > "$root/source.tar"
  sha256sum "$root/source.tar" > "$receipts/source-archive.sha256"
  tar -xf "$root/source.tar" -C "$root/source"
  rm "$root/source.tar"
  sha256sum "$root/source/Cargo.lock" > "$receipts/cargo-lock.sha256"
  printf 'source=%s\n' "$root/source"
  df -h "$root"
  free -h
  exit 0
fi

test "$phase" = verify
cd "$root/source"
# Keep tests and child CLI discovery away from the runner's real user state.
# CARGO_HOME and RUSTUP_HOME remain the explicitly isolated dependency/toolchain homes.
export HOME="$root/test-home"
export USERPROFILE="$HOME"
export XDG_CONFIG_HOME="$HOME/.config"
export XDG_DATA_HOME="$HOME/.local/share"
export XDG_CACHE_HOME="$HOME/.cache"
export XDG_STATE_HOME="$HOME/.local/state"
mkdir -p "$XDG_CONFIG_HOME" "$XDG_DATA_HOME" "$XDG_CACHE_HOME" "$XDG_STATE_HOME"
printf 'test_home=%s\ntest_xdg_data_home=%s\n' "$HOME" "$XDG_DATA_HOME"
rustc -Vv | tee "$receipts/rustc.txt"
cargo -Vv | tee "$receipts/cargo.txt"
cc --version | head -n 1
c++ --version | head -n 1
ld --version | head -n 1
rustc -Vv | grep -Fx 'host: x86_64-unknown-linux-gnu'
rustc -Vv | grep -Fx 'release: 1.98.1'
printf 'cargo_build_jobs=%s\ncmake_build_parallel_level=%s\nrust_test_threads=%s\n' \
  "$CARGO_BUILD_JOBS" "$CMAKE_BUILD_PARALLEL_LEVEL" "$RUST_TEST_THREADS"
gate() {
  local name="$1"
  shift
  printf '\ncommand:'
  printf ' %q' "$@"
  printf '\n'
  local status=0
  "$@" 2>&1 | tee "$receipts/$name.log" || status=$?
  printf '%s\n' "$status" > "$receipts/$name.exit"
  return "$status"
}
gate fmt cargo fmt --all --check
gate check cargo check --workspace --locked
gate test cargo test --workspace --locked
gate clippy cargo clippy --workspace --locked -- -D warnings
printf 'All four native Linux foundation gates passed.\n'
