# /// script
# requires-python = ">=3.13,<3.15"
# dependencies = []
# ///
"""Cheap committed-input and attestation checks; no compiler, SDK, or WSL."""
import contextlib
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import native_linux
import source_stage
import verify


class InputsTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="dendro-input-proof-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.repo = self.root / "repo"
        self.repo.mkdir()
        self.stage = self.root / "stage"
        self.stage.mkdir()
        self.git("init", "--quiet")
        self.write("README.md", b"public fixture\n")
        self.commit()

    def git(self, *args, input=None):
        return subprocess.check_output(["git", "-c", "user.name=Validation Fixture",
            "-c", "user.email=fixture@example.invalid", *args], cwd=self.repo,
            input=input, stderr=subprocess.PIPE)

    def write(self, name, content):
        path = self.repo / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(content)
        return path

    def commit(self):
        self.git("add", "--all")
        self.git("commit", "--quiet", "-m", "test: committed validation fixture")

    def stage_source(self):
        with contextlib.redirect_stdout(io.StringIO()):
            return source_stage.stage_source(self.stage, self.repo)

    def test_stage_uses_canonical_head_bytes_despite_checkout_eol(self):
        self.write(".gitattributes", b"*.txt text eol=crlf\n")
        path = self.write("scripts/input.txt", b"first\nsecond\n")
        self.commit()
        path.unlink()  # Only this test's disposable repository checkout file.
        self.git("checkout-index", "--force", "--index", "--", "scripts/input.txt")
        self.assertEqual(path.read_bytes(), b"first\r\nsecond\r\n")
        self.assertEqual(self.git("status", "--porcelain"), b"")
        result = self.stage_source()
        self.assertEqual((self.stage / "scripts/input.txt").read_bytes(), b"first\nsecond\n")
        self.assertEqual(result["head"], self.git("rev-parse", "HEAD").decode().strip())
        payload = json.loads((self.stage / source_stage.MANIFEST).read_text())
        entry = next(entry for entry in payload["entries"] if entry[0] == "scripts/input.txt")
        self.assertEqual(entry[2], self.git("rev-parse", "HEAD:scripts/input.txt").decode().strip())

    def test_omitted_public_roots_and_build_configs_are_included(self):
        names = [f"{root}/fixture.txt" for root in (
            "adapters", "lib", "manifests", "openrig", "roles", "schemas", "skills", "goals", "transitions")]
        names += ["rustfmt.toml", "clippy.toml", "rust-toolchain.toml", "deny.toml", ".cargo/config.toml"]
        for name in names:
            self.write(name, b"public input\n")
        self.commit()
        self.stage_source()
        for name in names:
            self.assertEqual((self.stage / name).read_bytes(), b"public input\n")

    def test_unknown_tracked_input_fails_before_copying(self):
        self.write("new-public-package/lib.rs", b"new code\n")
        self.commit()
        with self.assertRaisesRegex(ValueError, "Unclassified.*new-public-package"):
            self.stage_source()
        self.assertEqual(list(self.stage.iterdir()), [])

    def test_private_agent_vcs_and_generated_inputs_are_explicitly_excluded(self):
        names = [".claude/config.json", ".codex/auth.json", ".agentflow/runs/fixture.json",
                 ".githooks/pre-commit", ".github/workflows/fixture.yml", "crates/private/owner.txt",
                 "crates/fixture/.env", "crates/fixture/owner.pem", "crates/fixture/data.duckdb",
                 "crates/fixture/target/generated.txt"]
        for name in names:
            self.write(name, b"private fixture must not upload\n")
        self.commit()
        result = self.stage_source()
        for name in names:
            self.assertFalse((self.stage / name).exists())
        self.assertEqual(sum(result["excluded_categories"].values()), len(names))

    def test_dirty_before_or_after_snapshot_is_refused(self):
        self.write("README.md", b"dirty\n")
        with self.assertRaisesRegex(ValueError, "clean candidate"):
            self.stage_source()
        self.commit()
        original = source_stage.clean_head
        calls = 0
        def dirty_after(root):
            nonlocal calls
            calls += 1
            if calls == 2:
                self.write("README.md", b"changed during copy\n")
            return original(root)
        with patch("source_stage.clean_head", dirty_after), self.assertRaisesRegex(ValueError, "clean candidate"):
            self.stage_source()

    def test_head_change_during_snapshot_is_refused(self):
        original = source_stage.clean_head
        calls = 0
        def changed_head(root):
            nonlocal calls
            calls += 1
            if calls == 2:
                self.write("README.md", b"new commit during copy\n")
                self.commit()
            return original(root)
        with patch("source_stage.clean_head", changed_head), self.assertRaisesRegex(ValueError, "HEAD changed"):
            self.stage_source()

    def test_git_symlink_and_gitlink_are_refused_without_following_checkout(self):
        for mode, kind, name in (("120000", "blob", "scripts/linked-input"),
                                 ("160000", "commit", "crates/submodule")):
            with self.subTest(mode=mode):
                oid = (self.git("hash-object", "-w", "--stdin", input=b"outside-owner-data") if kind == "blob"
                       else self.git("rev-parse", "HEAD")).decode().strip()
                self.git("update-index", "--add", "--cacheinfo", f"{mode},{oid},{name}")
                self.git("commit", "--quiet", "-m", "test: nonregular Git input")
                head = self.git("rev-parse", "HEAD").decode().strip()
                with self.assertRaisesRegex(ValueError, "non-regular Git input"):
                    source_stage.tracked_inputs(self.repo, head)
                self.git("update-index", "--force-remove", name)
                self.git("commit", "--quiet", "-m", "test: remove nonregular fixture")

    def test_git_executable_modes_are_frozen_and_applied_from_metadata(self):
        executable = self.write("scripts/worktree-slice.sh", b"#!/bin/sh\nexit 0\n")
        executable.chmod(0o755)
        self.git("add", "--all")
        self.git("update-index", "--chmod=+x", "scripts/worktree-slice.sh")
        self.git("commit", "--quiet", "-m", "test: executable Git fixture")
        staged = self.stage_source()
        self.assertEqual(staged["executable_paths"], ["scripts/worktree-slice.sh"])
        calls = []
        original = Path.chmod
        def chmod(path, mode, *args, **kwargs):
            calls.append((path.relative_to(self.stage).as_posix(), mode))
            return original(path, mode, *args, **kwargs)
        with patch.object(Path, "chmod", chmod):
            source_stage.apply_source_modes(self.stage)
        self.assertIn(("scripts/worktree-slice.sh", 0o700), calls)
        self.assertIn(("README.md", 0o600), calls)
        checked = source_stage.verify_source(self.stage, expected_digest=staged["candidate_manifest_sha256"],
                                             enforce_modes=os.name == "posix")
        self.assertEqual(checked["executable_paths"], staged["executable_paths"])

    def test_transport_tamper_missing_extra_and_private_manifest_fail(self):
        staged = self.stage_source()
        path = self.stage / "README.md"
        path.write_bytes(b"tampered\n")
        with self.assertRaisesRegex(ValueError, "committed Git blob"):
            source_stage.verify_source(self.stage)
        path.unlink()
        with self.assertRaisesRegex(ValueError, "source paths differ"):
            source_stage.verify_source(self.stage)
        path.write_bytes(b"public fixture\n")
        extra = self.stage / "extra.txt"
        extra.write_bytes(b"extra")
        with self.assertRaisesRegex(ValueError, "source paths differ"):
            source_stage.verify_source(self.stage)
        extra.unlink()
        with self.assertRaisesRegex(ValueError, "manifest does not match"):
            source_stage.verify_source(self.stage, expected_digest="0" * 64)
        with self.assertRaisesRegex(ValueError, "HEAD does not match"):
            source_stage.verify_source(self.stage, expected_head="0" * 40)
        manifest = self.stage / source_stage.MANIFEST
        payload = json.loads(manifest.read_text())
        payload["entries"][0][0] = "private/owner.txt"
        (self.stage / "private").mkdir()
        path.rename(self.stage / "private/owner.txt")
        manifest.write_text(json.dumps(payload))
        with self.assertRaisesRegex(ValueError, "private committed-source"):
            source_stage.verify_source(self.stage)

    def test_transported_link_cannot_read_or_modify_outside_sentinel(self):
        self.stage_source()
        outside = self.root / "outside"
        outside.write_bytes(b"outside sentinel")
        (self.stage / "README.md").unlink()
        try:
            (self.stage / "README.md").symlink_to(outside)
        except OSError:
            self.skipTest("Native symlink creation unavailable")
        with self.assertRaisesRegex(ValueError, "source link/reparse"):
            source_stage.verify_source(self.stage)
        self.assertEqual(outside.read_bytes(), b"outside sentinel")


class AttestationTests(unittest.TestCase):
    def setUp(self):
        self.staged = {"head": "a" * 40, "git_tree": "b" * 40, "file_count": 2,
                       "candidate_manifest_sha256": "c" * 64, "executable_paths": ["scripts/tool.sh"]}
        self.metadata = {"filesystem": "ext4", "sourceHead": "a" * 40, "sourceTree": "b" * 40,
                         "sourceCopyManifestSha256": "c" * 64, "sourceFileCount": 2,
                         "sourceExecutablePaths": ["scripts/tool.sh"]}

    def result(self, metadata, code=0, unchanged=True):
        return native_linux.receipt_result(self.staged["head"], self.staged, metadata, code, unchanged)

    def test_receipt_requires_every_source_fact_and_child_success(self):
        self.assertTrue(self.result(self.metadata)["passed"])
        for field in self.metadata:
            with self.subTest(field=field):
                missing = dict(self.metadata)
                missing.pop(field)
                result = self.result(missing)
                self.assertFalse(result["passed"])
                self.assertNotEqual(result["exitCode"], 0)
        for metadata in (None, {}, {**self.metadata, "sourceCopyManifestSha256": "d" * 64},
                         {**self.metadata, "sourceHead": "e" * 40},
                         {**self.metadata, "sourceExecutablePaths": []}):
            result = self.result(metadata)
            self.assertFalse(result["passed"])
            self.assertEqual(result["childExitCode"], 0)
            self.assertNotEqual(result["exitCode"], 0)
        self.assertEqual(self.result(self.metadata, code=7)["exitCode"], 7)
        self.assertFalse(self.result(self.metadata, unchanged=False)["passed"])

    def test_missing_malformed_or_ambiguous_log_metadata_is_refused(self):
        text = json.dumps(self.metadata)
        self.assertEqual(native_linux.child_metadata("cargo output\n" + text), self.metadata)
        for output in ("cargo succeeded", '{"filesystem":invalid}', text + "\n" + text):
            self.assertIsNone(native_linux.child_metadata(output))

    def test_full_preflight_refuses_dirty_before_any_command(self):
        with patch.object(sys, "argv", ["verify.py"]), patch.object(verify, "GUARD", None), patch.object(
            verify, "require_clean_candidate", side_effect=SystemExit("dirty candidate")
        ), patch.object(verify, "run") as command, self.assertRaisesRegex(SystemExit, "dirty candidate"):
            verify.main()
        command.assert_not_called()

    def test_fast_mode_remains_development_only_and_skips_clean_requirement(self):
        with patch.object(sys, "argv", ["verify.py", "--fast"]), patch.object(verify, "GUARD", None), patch.object(
            verify, "require_clean_candidate"
        ) as clean, patch.object(verify, "run"), patch.object(verify.subprocess, "check_output", return_value=""):
            verify.main()
        clean.assert_not_called()

    def test_explicit_dependency_task_runs_existing_restrictive_validator(self):
        with patch.object(sys, "argv", ["verify.py", "--dependency-automation", "--base", "development"]), patch.object(
            verify, "GUARD", None
        ), patch.object(verify, "require_clean_candidate"), patch.object(verify, "run") as command, patch.dict(os.environ):
            verify.main()
            self.assertEqual(os.environ["BASE_REF"], "development")
        command.assert_called_once_with(["node", "scripts/validate-dependency-automation.mjs"])


if __name__ == "__main__":
    unittest.main()
