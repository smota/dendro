"""Cheap regression checks; no Cargo commands or Rust compilation."""
import json
import os
import shutil
import stat
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest
from types import SimpleNamespace
from unittest.mock import patch

from windows_guard import ExternalWindowsGuard, GuardError, LEASE, MARKER, WindowsGuard, tree_bytes


class GuardTests(unittest.TestCase):
    def setUp(self):
        self.scratch = tempfile.TemporaryDirectory()
        self.addCleanup(self.scratch.cleanup)
        self.root = Path(self.scratch.name)
        self.environment = dict(os.environ, DENDRO_WINDOWS_MIN_FREE_BYTES="1",
                                DENDRO_WINDOWS_TARGET_MAX_BYTES="1000000",
                                DENDRO_WINDOWS_RUN_MAX_BYTES="1000000")

    def guard(self, **env):
        return WindowsGuard(self.root, dict(self.environment, **env))

    def test_real_child_isolated_profile_and_preserved_tools(self):
        original_home = Path.home()
        before = set(original_home.glob("dendro-library-*"))
        secrets = {"OPENAI_API_KEY": "fake-secret", "OPENRIG_HOME": "real-agent-data", "CODEX_HOME": "real-agent-data",
                   "CLAUDE_CONFIG_DIR": "real-agent-data", "DENDRO_DB": "real-database", "DENDRO_DATA_DIR": "real-data",
                   "XDG_DATA_HOME": "real-xdg", "MISE_AUTO_INSTALL": "true"}
        with self.guard(RUSTUP_HOME="existing-rustup", CARGO_HOME="existing-cargo", **secrets) as guard:
            run_path = guard.path
            result = guard.run([sys.executable, "-c", "import os,json,pathlib; "
                                "pathlib.Path.home().joinpath('dendro-library-0123456789abcdef').mkdir(); "
                                "print(json.dumps(dict(os.environ)))"], stdout=subprocess.PIPE, text=True, check=True)
            env = json.loads(result.stdout)
            for key in ("HOME", "USERPROFILE", "APPDATA", "LOCALAPPDATA", "TEMP", "TMP", "XDG_CONFIG_HOME",
                        "XDG_DATA_HOME", "XDG_CACHE_HOME", "XDG_STATE_HOME", "XDG_RUNTIME_DIR"):
                self.assertTrue(Path(env[key]).is_relative_to(run_path))
            for key in secrets.keys() - {"XDG_DATA_HOME", "MISE_AUTO_INSTALL"}:
                self.assertNotIn(key, env)
            self.assertEqual(env["MISE_AUTO_INSTALL"], "false")
            self.assertEqual(env["RUSTUP_HOME"], "existing-rustup")
            self.assertEqual(env["CARGO_HOME"], "existing-cargo")
            self.assertEqual(env["CARGO_INCREMENTAL"], "0")
            self.assertEqual(env["CARGO_PROFILE_DEV_DEBUG"], "0")
            self.assertEqual(env["CARGO_PROFILE_TEST_DEBUG"], "0")
            self.assertNotIn("CARGO_PROFILE_DEV_DEBUG_ASSERTIONS", env)
            self.assertFalse(Path(env["CARGO_TARGET_DIR"]).is_relative_to(run_path))
        self.assertFalse(run_path.exists())
        self.assertEqual(set(original_home.glob("dendro-library-*")), before)

    def test_command_failure_cleans(self):
        with self.assertRaises(subprocess.CalledProcessError):
            with self.guard() as guard:
                path = guard.path
                guard.run([sys.executable, "-c", "raise SystemExit(7)"], check=True)
        self.assertFalse(path.exists())

    def test_external_command_preserves_profiles(self):
        with ExternalWindowsGuard(self.root, self.environment) as guard:
            result = guard.run([sys.executable, "-c", "import os,json; print(json.dumps(dict(os.environ)))"],
                               stdout=subprocess.PIPE, text=True, check=True)
            self.assertEqual(json.loads(result.stdout), self.environment)
            run_path = guard.path
        self.assertFalse(run_path.exists())

    @unittest.skipUnless(os.name == "nt", "Windows private ACL enforcement")
    def test_private_run_acl_and_unsupported_python_refusal(self):
        with patch("windows_guard.sys.version_info", (3, 12)), self.assertRaisesRegex(GuardError, "Python 3.13"):
            self.guard()
        self.assertFalse((self.root / ".agent-runs").exists())
        with self.guard() as guard:
            powershell = shutil.which("pwsh.exe") or shutil.which("powershell.exe")
            self.assertIsNotNone(powershell, "built-in Windows ACL inspection is required")
            script = ("$ErrorActionPreference='Stop'; $acl=Get-Acl -LiteralPath $env:DENDRO_GUARD_ACL_PATH; "
                "@{protected=$acl.AreAccessRulesProtected; owner=$acl.GetOwner([System.Security.Principal.SecurityIdentifier]).Value; current=[System.Security.Principal.WindowsIdentity]::GetCurrent().User.Value; "
                "rules=@($acl.Access | ForEach-Object { @{sid=$_.IdentityReference.Translate([System.Security.Principal.SecurityIdentifier]).Value; "
                "allow=($_.AccessControlType -eq 'Allow')} })} | ConvertTo-Json -Depth 4 -Compress")
            for directory in (guard.path, Path(guard.env["USERPROFILE"]), Path(guard.env["APPDATA"])):
                with self.subTest(directory=directory):
                    result = subprocess.run([powershell, "-NoProfile", "-NonInteractive", "-Command", script],
                                            env=dict(os.environ, DENDRO_GUARD_ACL_PATH=str(directory)),
                                            capture_output=True, text=True, check=True, timeout=10)
                    acl = json.loads(result.stdout)
                    if directory == guard.path:
                        self.assertTrue(acl["protected"], "private root must not inherit repository access rules")
                    self.assertEqual(acl["owner"], acl["current"])
                    allowed = {acl["current"], "S-1-5-32-544", "S-1-5-18"}
                    granted = {rule["sid"] for rule in acl["rules"] if rule["allow"]}
                    self.assertTrue(granted <= allowed, "ancestor admission refuses OWNER RIGHTS and other replacement principals")
                    self.assertIn(acl["current"], granted, "grant the creator SID explicitly")

    def test_external_floor_stops_owned_child(self):
        with ExternalWindowsGuard(self.root, self.environment) as guard:
            usage = shutil.disk_usage(self.root)
            readings = 0
            def free_space(path):
                nonlocal readings
                readings += 1
                return usage if readings <= 2 else type(usage)(usage.total, usage.used, 0)
            with patch("windows_guard.shutil.disk_usage", free_space), self.assertRaisesRegex(GuardError, "free-space floor"):
                guard.run([sys.executable, "-c", "import time; time.sleep(30)"])

    @unittest.skipUnless(os.name == "nt", "native Windows external CLI")
    def test_external_cli_preserves_profile_and_exit_status(self):
        command = [sys.executable, str(Path(__file__).with_name("windows_guard.py")), "--external", "--",
                   sys.executable, "-c", "import os,json,sys; assert sys.stdin.read() == 'terminal-input'; print(json.dumps(dict(os.environ))); raise SystemExit(9)"]
        result = subprocess.run(command, cwd=self.root, env=self.environment, input="terminal-input", capture_output=True, text=True, timeout=20)
        self.assertEqual(result.returncode, 9, result.stderr)
        self.assertEqual(json.loads(result.stdout), self.environment)

    def test_linux_disk_budgets_without_building(self):
        from native_linux import disk_limits
        cache = self.root / "cache"
        workspace = self.root / "workspace"
        cache.mkdir()
        workspace.mkdir()
        disk_limits(cache, workspace, self.root, (1, 100, 100))
        (cache / "large").write_bytes(b"x" * 101)
        with self.assertRaisesRegex(SystemExit, "cache budget"):
            disk_limits(cache, workspace, self.root, (1, 100, 100))
        (cache / "large").unlink()
        (workspace / "large").write_bytes(b"x" * 101)
        with self.assertRaisesRegex(SystemExit, "workspace budget"):
            disk_limits(cache, workspace, self.root, (1, 100, 100))
        with self.assertRaisesRegex(SystemExit, "free-space floor"):
            disk_limits(cache, workspace, self.root, (2 ** 63, 100, 100))

    def test_invalid_budgets_refused(self):
        for value in ("0", "-1", "no", "1.5"):
            with self.subTest(value=value), self.assertRaises(GuardError):
                self.guard(DENDRO_WINDOWS_RUN_MAX_BYTES=value)

    def test_preflight_target_and_free_space_refuse_child(self):
        target = self.root / "target"
        target.mkdir()
        (target / "large").write_bytes(b"xx")
        with self.assertRaisesRegex(GuardError, "target budget"):
            with self.guard(DENDRO_WINDOWS_TARGET_MAX_BYTES="1"):
                self.fail("over-budget preflight admitted")
        with self.assertRaisesRegex(GuardError, "free-space floor"):
            with self.guard(DENDRO_WINDOWS_MIN_FREE_BYTES=str(2 ** 63)):
                self.fail("free-space preflight admitted")

    def test_live_run_budget_stops_child_and_cleans(self):
        with self.assertRaisesRegex(GuardError, "run budget"):
            with self.guard(DENDRO_WINDOWS_RUN_MAX_BYTES="1000") as guard:
                path = guard.path
                guard.run([sys.executable, "-c", "import os,time,pathlib; "
                           "pathlib.Path(os.environ['TEMP'],'growth').write_bytes(b'x'*2000); time.sleep(30)"])
        self.assertFalse(path.exists())

    @unittest.skipUnless(os.name == "nt", "Windows Job Object integration")
    def test_budget_terminates_descendant_before_cleanup(self):
        escaped = self.root / "descendant-survived"
        descendant = f"import time,pathlib; time.sleep(1); pathlib.Path({str(escaped)!r}).write_text('bad')"
        script = ("import subprocess,sys,os,time,pathlib; "
                  f"subprocess.Popen([sys.executable,'-c',{descendant!r}]); "
                  "pathlib.Path(os.environ['TEMP'],'growth').write_bytes(b'x'*2000); time.sleep(30)")
        with self.assertRaisesRegex(GuardError, "run budget"):
            with self.guard(DENDRO_WINDOWS_RUN_MAX_BYTES="1000") as guard:
                guard.run([sys.executable, "-c", script])
        time.sleep(1.2)
        self.assertFalse(escaped.exists(), "owned descendant survived the job termination")

    def test_target_budget_monitored_during_child(self):
        with self.assertRaisesRegex(GuardError, "target budget"):
            with self.guard(DENDRO_WINDOWS_TARGET_MAX_BYTES="1000") as guard:
                script = ("import os,time,pathlib; p=pathlib.Path(os.environ['CARGO_TARGET_DIR']); "
                          "p.mkdir(); (p/'growth').write_bytes(b'x'*2000); time.sleep(30)")
                guard.run([sys.executable, "-c", script])
        self.assertTrue((self.root / "target" / "growth").exists(), "guard must preserve reusable cache")

    def test_interrupt_cleans(self):
        with self.guard() as guard:
            path = guard.path
            real_check = guard.check
            calls = 0
            def interrupt():
                nonlocal calls
                calls += 1
                real_check()
                if calls > 1:
                    raise KeyboardInterrupt()
            with patch.object(guard, "check", interrupt), self.assertRaises(KeyboardInterrupt):
                guard.run([sys.executable, "-c", "import time; time.sleep(30)"])
        self.assertFalse(path.exists())

    def test_only_proven_idle_abandoned_workspace_pruned(self):
        guard = self.guard()
        guard.base.mkdir(parents=True)
        abandoned = guard.base / ("a" * 32)
        abandoned.mkdir()
        (abandoned / MARKER).write_text(json.dumps(guard._marker(abandoned, "idle")))
        (abandoned / LEASE).write_bytes(b"1")
        with self.guard():
            self.assertFalse(abandoned.exists())
        abandoned.mkdir()
        (abandoned / MARKER).write_text(json.dumps(guard._marker(abandoned, "running")))
        (abandoned / LEASE).write_bytes(b"1")
        with self.assertRaisesRegex(GuardError, "undrained"):
            with self.guard():
                self.fail("uncertain running workspace pruned")
        self.assertTrue(abandoned.exists())

    def test_unmarked_workspace_preserved(self):
        guard = self.guard()
        unknown = guard.base / "unowned"
        unknown.mkdir(parents=True)
        (unknown / LEASE).write_bytes(b"1")
        with self.assertRaisesRegex(GuardError, "ownership"):
            with guard:
                self.fail("unowned workspace admitted")
        self.assertTrue(unknown.exists())

    def test_vanished_directory_during_scan_is_tolerated(self):
        for replaced_by_file in (False, True):
            with self.subTest(replaced_by_file=replaced_by_file):
                directory = self.root / "scan"
                directory.mkdir(exist_ok=True)
                volatile = directory / "volatile"
                volatile.mkdir()
                (directory / "kept").write_bytes(b"abc")
                real_scandir = os.scandir
                def vanish(path):
                    if Path(path) == volatile:
                        volatile.rmdir()
                        if replaced_by_file:
                            volatile.write_bytes(b"changed")
                    return real_scandir(path)
                with patch("windows_guard.os.scandir", vanish):
                    self.assertGreaterEqual(tree_bytes(directory), 3)
                if volatile.exists():
                    volatile.unlink()

    def test_readonly_file_is_cleaned_after_child(self):
        with self.guard() as guard:
            path = guard.path
            guard.run([sys.executable, "-c", "import os,pathlib; p=pathlib.Path(os.environ['TEMP'],'readonly'); p.write_bytes(b'fixture'); p.chmod(0o400)"], check=True)
        self.assertFalse(path.exists())

    def test_unknown_reparse_tag_is_refused_with_path(self):
        with self.guard() as guard:
            unknown = guard.path / "unknown-tag"
            unknown.mkdir()
            original_lstat = Path.lstat
            def metadata(path, *args, **kwargs):
                if path == unknown:
                    return SimpleNamespace(st_mode=stat.S_IFDIR, st_size=0,
                                           st_file_attributes=0x400, st_reparse_tag=0x8000001B)
                return original_lstat(path, *args, **kwargs)
            with patch.object(Path, "lstat", metadata), self.assertRaises(GuardError) as error:
                tree_bytes(guard.path)
            self.assertIn(str(unknown), str(error.exception))

    @unittest.skipUnless(os.name == "nt", "Windows file-sharing cleanup failure")
    def test_held_file_cleanup_reports_path_and_recovers_on_retry(self):
        guard = self.guard()
        guard.__enter__()
        blocked = guard.path / "held"
        try:
            with blocked.open("wb"):
                with self.assertRaises(GuardError) as error:
                    guard.__exit__(None, None, None)
                self.assertIn(str(blocked), str(error.exception))
        finally:
            if guard.lease:
                guard.lease.close()
        with self.guard():
            self.assertFalse(guard.path.exists())

    def test_descendant_symlinks_account_only_entries_and_cleanup_preserves_target(self):
        outside = self.root / "outside"
        outside.mkdir()
        sentinel = outside / "sentinel"
        sentinel.write_bytes(b"s" * 2000000)
        with self.guard() as guard:
            try:
                (guard.path / "directory-link").symlink_to(outside, target_is_directory=True)
                (guard.path / "file-link").symlink_to(sentinel)
            except OSError:
                self.skipTest("Creating symlinks requires Windows privilege")
            self.assertLess(tree_bytes(guard.path), 1000000)
            guard.check()
            path = guard.path
            root_alias = self.root / "root-alias"
            root_alias.symlink_to(guard.path, target_is_directory=True)
            with self.assertRaisesRegex(GuardError, "reparse"):
                guard._remove(root_alias)
            root_alias.unlink()
        self.assertFalse(path.exists())
        self.assertEqual(sentinel.read_bytes(), b"s" * 2000000)

    @unittest.skipUnless(os.name == "nt", "Windows junction fixture")
    def test_descendant_junction_scan_and_cleanup_preserve_external_target(self):
        outside = self.root / "junction-target"
        outside.mkdir()
        sentinel = outside / "sentinel"
        sentinel.write_bytes(b"s" * 2000000)
        with self.guard() as guard:
            junction = guard.path / "junction"
            result = subprocess.run(f'cmd.exe /D /C mklink /J "{junction}" "{outside}"', capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertLess(tree_bytes(guard.path), 1000000)
            guard.check()
            path = guard.path
        self.assertFalse(path.exists())
        self.assertEqual(sentinel.read_bytes(), b"s" * 2000000)

    @unittest.skipUnless(os.name == "nt", "native Windows verify entry point")
    def test_verify_windows_modes_with_fake_tools_no_compilation(self):
        tools = self.root / "tools" / "validation"
        tools.mkdir(parents=True)
        for name in ("verify.py", "windows_guard.py", "performance_policy.py"):
            shutil.copy2(Path(__file__).parent / name, tools / name)
        fake = self.root / "fake"
        fake.mkdir()
        log = self.root / "calls.jsonl"
        (fake / "cargo.py").write_text("import os,json,sys,pathlib\n"
            f"with pathlib.Path({str(log)!r}).open('a') as f: f.write(json.dumps({{'args':sys.argv[1:],'env':dict(os.environ)}})+'\\n')\n"
            "if 'native_foundation_acceptance' in sys.argv:\n"
            " print('T24-29 actual distinct-device copy, private destination, unchanged closed source and verified local pointer: passed')\n"
            " print('1 passed; 0 failed')\n")
        (fake / "cargo.cmd").write_text(f'@"{sys.executable}" "{fake / "cargo.py"}" %*\n')
        (fake / "git.py").write_text("import sys\n"
            "if sys.argv[1]=='branch': print('work/fake-test')\n"
            "elif sys.argv[1] in ('rev-parse','merge-base'): print('a'*40)\n")
        (fake / "git.cmd").write_text(f'@"{sys.executable}" "{fake / "git.py"}" %*\n')
        # Admission metadata intentionally remains outside the command guard.
        # Substitute only that read-only Git surface; no test repository/commit.
        launcher = self.root / "launch.py"
        launcher.write_text("import subprocess,sys,runpy\n"
            "original = subprocess.check_output\n"
            "def output(command, **kwargs):\n"
            f" if command[0]=='git': return subprocess.run([sys.executable,{str(fake / 'git.py')!r},*command[1:]],stdout=subprocess.PIPE,check=True,**kwargs).stdout\n"
            " return original(command,**kwargs)\n"
            "subprocess.check_output=output\n"
            f"sys.path.insert(0,{str(tools)!r})\n"
            f"runpy.run_path({str(tools / 'verify.py')!r},run_name='__main__')\n")
        for name in ("node", "pnpm"):
            (fake / f"{name}.cmd").write_text("@exit /b 0\n")
        (self.root / "Cargo.toml").write_text('[workspace.package]\nversion = "0.0.0"\n')
        (self.root / "docs").mkdir()
        (self.root / "docs" / "RELEASE_NOTES_v0.0.0.md").write_text("fixture")
        source, dest = self.root / "source-volume", self.root / "destination-volume"
        source.mkdir()
        dest.mkdir()
        env = dict(self.environment, PATH=str(fake) + os.pathsep + os.environ["PATH"])
        env.update(FOUNDATION_CROSS_VOLUME_SOURCE=str(source), FOUNDATION_CROSS_VOLUME_DEST=str(dest),
                   DENDRO_VALIDATION_PERF_WAIVER="inherited-invalid-token")
        for flags, expected in ((["--windows"], ["fmt", "check", "clippy", "test"]),
                                ([], ["fmt", "check", "clippy", "test"]),
                                (["--fast"], ["fmt"]), (["--release-candidate"], ["check"]),
                                (["--native-fixture"], ["test"])):
            with self.subTest(flags=flags):
                log.unlink(missing_ok=True)
                result = subprocess.run([sys.executable, str(launcher), *flags],
                                        cwd=self.root, env=env, capture_output=True, text=True, timeout=20)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                calls = [json.loads(line) for line in log.read_text().splitlines()]
                self.assertEqual([call["args"][0] for call in calls], expected)
                homes = {call["env"]["USERPROFILE"] for call in calls}
                self.assertEqual(len(homes), 1, "one workspace must serve the entire invocation")
                self.assertFalse(Path(homes.pop()).exists(), "verify entry point must clean the workspace")
                for call in calls:
                    self.assertEqual(call["env"]["CARGO_PROFILE_TEST_DEBUG"], "0")
                    self.assertNotIn("DENDRO_VALIDATION_PERF_WAIVER", call["env"])
        self.assertTrue(source.exists() and dest.exists(), "external fixture directories must survive")


if __name__ == "__main__":
    unittest.main()
