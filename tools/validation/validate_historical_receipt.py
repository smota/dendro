"""Preserved foundation receipt assertions; WSL identity remains explicit."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys

repo, receipt_path, receipts, target_base = map(Path, sys.argv[1:])
proof = json.loads(receipt_path.read_text())
candidate = subprocess.check_output(["git", "-C", str(repo), "rev-parse", "HEAD"], text=True).strip()
historical = "6b755135f95e497071790e8e5e358995a8244a18"
assert proof["passed"] is True
assert proof["execution_environment"] in {"native-linux", "WSL"}
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
assert proof["cargo_target_dirs"] == {"historical": str(target_base / "historical"), "candidate": str(target_base / "candidate")}
assert proof["oracle_sha256"] == blob_hash(candidate, ".agentflow/oracles/v080/T04/frozen.json")
for field in ("candidate_files", "harness_files", "frozen_fixture_files"):
    assert proof[field], f"empty {field}"
    for path, digest in proof[field].items():
        assert digest == blob_hash(candidate, path), f"source mismatch: {path}"
(receipts / "historical-ci-validation.json").write_text(json.dumps({
    "schema": "dendro.foundation-historical-ci/v1", "passed": True,
    "candidate_commit": candidate, "historical_commit": historical,
    "scope": f"{proof['execution_environment']} foundation historical migration proof; not T27 release certification",
    "receipt_sha256": hashlib.sha256(receipt_path.read_bytes()).hexdigest(),
}, indent=2) + "\n")
print("Historical proof passed; receipt source hashes match committed candidate and historical source.")
