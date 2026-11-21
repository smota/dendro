"""Owned, disk-bounded scratch profiles for native Windows validation."""
import contextlib
import ctypes
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import tempfile
import time
import uuid

MARKER = ".dendro-validation-owner.json"
LEASE = ".lease"
GIB = 1024 ** 3
NATIVE_ENVIRONMENT = set("""
PATH PATHEXT SYSTEMROOT SYSTEMDRIVE WINDIR COMSPEC OS NUMBER_OF_PROCESSORS
PROCESSOR_ARCHITECTURE PROCESSOR_IDENTIFIER PROCESSOR_LEVEL PROCESSOR_REVISION
PROGRAMFILES PROGRAMFILES(X86) PROGRAMW6432 COMMONPROGRAMFILES COMMONPROGRAMFILES(X86)
COMMONPROGRAMW6432 PROGRAMDATA ALLUSERSPROFILE USERNAME USERDOMAIN COMPUTERNAME
LANG LC_ALL LC_CTYPE TERM COLORTERM NO_COLOR
INCLUDE LIB LIBPATH VCINSTALLDIR VCTOOLSINSTALLDIR VSINSTALLDIR VISUALSTUDIOVERSION
WINDOWSSDKDIR WINDOWSSDKVERSION WINDOWSSDKBINPATH WINDOWSSDKVERBINPATH
UNIVERSALCRTSDKDIR UCRTVERSION FRAMEWORKDIR FRAMEWORKDIR64 FRAMEWORKVERSION
FRAMEWORKVERSION64 VSCMD_ARG_HOST_ARCH VSCMD_ARG_TGT_ARCH VSCMD_VER
CC CXX AR CFLAGS CXXFLAGS LDFLAGS LIBCLANG_PATH CLANG_PATH LLVM_CONFIG_PATH
BINDGEN_EXTRA_CLANG_ARGS PKG_CONFIG_PATH CMAKE_GENERATOR CMAKE_GENERATOR_PLATFORM
CMAKE_MAKE_PROGRAM CMAKE_TOOLCHAIN_FILE CMAKE_BUILD_PARALLEL_LEVEL
OPENSSL_DIR OPENSSL_LIB_DIR OPENSSL_INCLUDE_DIR OPENSSL_STATIC OPENSSL_CONF
HTTP_PROXY HTTPS_PROXY ALL_PROXY NO_PROXY SSL_CERT_FILE SSL_CERT_DIR
REQUESTS_CA_BUNDLE CURL_CA_BUNDLE CARGO_HTTP_CAINFO CARGO_HTTP_PROXY
RUSTUP_HOME RUSTUP_TOOLCHAIN CARGO_HOME CARGO_BUILD_JOBS RUST_TEST_THREADS
MISE_DATA_DIR MISE_CACHE_DIR MISE_CONFIG_DIR MISE_STATE_DIR MISE_GLOBAL_CONFIG_FILE
UV_CACHE_DIR UV_PYTHON_INSTALL_DIR UV_TOOL_DIR UV_TOOL_BIN_DIR UV_PYTHON_BIN_DIR
PNPM_HOME VOLTA_HOME NVM_HOME NVM_SYMLINK SCOOP SCOOP_GLOBAL
FOUNDATION_CROSS_VOLUME_SOURCE FOUNDATION_CROSS_VOLUME_DEST
""".split())


class GuardError(RuntimeError):
    pass


def positive(value):
    try:
        number = int(value)
    except (TypeError, ValueError) as error:
        raise GuardError("Windows validation budgets must be positive integer bytes") from error
    if number <= 0:
        raise GuardError("Windows validation budgets must be positive integer bytes")
    return number


def safe_path(path):
    """Reject links/junctions in every existing component, without following them."""
    path = Path(os.path.abspath(path))
    for part in (*reversed(path.parents), path):
        if part.exists() or part.is_symlink():
            info = part.lstat()
            if stat.S_ISLNK(info.st_mode) or getattr(info, "st_file_attributes", 0) & 0x400:
                raise GuardError(f"Refusing validation reparse point: {part}")
    return path


def tree_bytes(path):
    safe_path(path)
    total = 0
    if not path.exists():
        return total
    def refuse(error):
        if isinstance(error, (FileNotFoundError, NotADirectoryError)):
            return  # An enumerated compiler directory can disappear before descent.
        raise GuardError(f"Cannot measure validation disk usage at {error.filename or path}: {error}") from error
    for root, dirs, files in os.walk(path, followlinks=False, onerror=refuse):
        for name in dirs + files:
            entry = Path(root) / name
            try:
                info = entry.lstat()
            except (FileNotFoundError, NotADirectoryError):
                continue  # Compilers legitimately rename/remove files during a scan.
            reparse = getattr(info, "st_file_attributes", 0) & 0x400
            tag = getattr(info, "st_reparse_tag", 0)
            if reparse and tag not in (0xA000000C, 0xA0000003):
                raise GuardError(f"Refusing unknown validation reparse point: {entry}")
            if stat.S_ISLNK(info.st_mode) or reparse:
                if name in dirs:
                    dirs.remove(name)
                total += info.st_size  # Account only the entry, never its target.
                continue
            if stat.S_ISREG(info.st_mode):
                total += info.st_size
    return total


def lock(file):
    file.seek(0)
    if os.name == "nt":
        import msvcrt
        msvcrt.locking(file.fileno(), msvcrt.LK_NBLCK, 1)
    else:
        import fcntl
        fcntl.flock(file, fcntl.LOCK_EX | fcntl.LOCK_NB)


class WindowsJob:
    """An assigned helper cannot start its command before the parent releases it."""
    def __init__(self):
        from ctypes import wintypes
        class BasicLimits(ctypes.Structure):
            _fields_ = [("process_time", ctypes.c_int64), ("job_time", ctypes.c_int64),
                        ("flags", wintypes.DWORD), ("minimum", ctypes.c_size_t),
                        ("maximum", ctypes.c_size_t), ("active", wintypes.DWORD),
                        ("affinity", ctypes.c_size_t), ("priority", wintypes.DWORD),
                        ("scheduling", wintypes.DWORD)]
        class ExtendedLimits(ctypes.Structure):
            _fields_ = [("basic", BasicLimits), ("io", ctypes.c_uint64 * 6),
                        ("process_memory", ctypes.c_size_t), ("job_memory", ctypes.c_size_t),
                        ("peak_process", ctypes.c_size_t), ("peak_job", ctypes.c_size_t)]
        self.api = ctypes.WinDLL("kernel32", use_last_error=True)
        self.api.CreateJobObjectW.argtypes = [ctypes.c_void_p, wintypes.LPCWSTR]
        self.api.CreateJobObjectW.restype = wintypes.HANDLE
        self.api.AssignProcessToJobObject.argtypes = [wintypes.HANDLE, wintypes.HANDLE]
        self.api.AssignProcessToJobObject.restype = wintypes.BOOL
        self.api.TerminateJobObject.argtypes = [wintypes.HANDLE, wintypes.UINT]
        self.api.TerminateJobObject.restype = wintypes.BOOL
        self.api.CloseHandle.argtypes = [wintypes.HANDLE]
        self.api.CloseHandle.restype = wintypes.BOOL
        self.api.SetInformationJobObject.argtypes = [wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p, wintypes.DWORD]
        self.api.SetInformationJobObject.restype = wintypes.BOOL
        self.api.QueryInformationJobObject.argtypes = [wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p, wintypes.DWORD, ctypes.c_void_p]
        self.api.QueryInformationJobObject.restype = wintypes.BOOL
        self.handle = self.api.CreateJobObjectW(None, None)
        if not self.handle:
            raise ctypes.WinError(ctypes.get_last_error())
        limits = ExtendedLimits()
        limits.basic.flags = 0x2000  # JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
        if not self.api.SetInformationJobObject(self.handle, 9, ctypes.byref(limits), ctypes.sizeof(limits)):
            self.api.CloseHandle(self.handle)
            self.handle = None
            raise ctypes.WinError(ctypes.get_last_error())

    def assign(self, process):
        if not self.api.AssignProcessToJobObject(self.handle, int(process._handle)):
            raise ctypes.WinError(ctypes.get_last_error())

    def close(self):
        if self.handle:
            if not self.api.TerminateJobObject(self.handle, 1):
                raise ctypes.WinError(ctypes.get_last_error())
            deadline = time.monotonic() + 10
            while True:
                accounting = (ctypes.c_uint64 * 6)()
                if not self.api.QueryInformationJobObject(self.handle, 1, accounting, ctypes.sizeof(accounting), None):
                    raise ctypes.WinError(ctypes.get_last_error())
                if ctypes.cast(accounting, ctypes.POINTER(ctypes.c_uint32))[10] == 0:
                    break
                if time.monotonic() >= deadline:
                    raise GuardError("Owned Windows job did not drain; refusing scratch cleanup")
                time.sleep(0.05)
            self.api.CloseHandle(self.handle)
            self.handle = None


class WindowsGuard:
    def __init__(self, root, environment=None):
        if os.name == "nt" and sys.version_info < (3, 13):
            raise GuardError("Private Windows validation directories require Python 3.13 or newer; no tools were installed")
        self.root = safe_path(root)
        self.original = dict(os.environ if environment is None else environment)
        self.base = safe_path(self.root / ".agent-runs" / "windows-validation")
        target = Path(self.original.get("CARGO_TARGET_DIR", self.root / "target"))
        self.target = safe_path(target if target.is_absolute() else self.root / target)
        self.floor = positive(self.original.get("DENDRO_WINDOWS_MIN_FREE_BYTES", 10 * GIB))
        self.target_max = positive(self.original.get("DENDRO_WINDOWS_TARGET_MAX_BYTES", 24 * GIB))
        self.run_max = positive(self.original.get("DENDRO_WINDOWS_RUN_MAX_BYTES", 4 * GIB))
        if self.target == self.root or self.target in self.root.parents or self.base == self.target or self.base in self.target.parents or self.target in self.base.parents:
            raise GuardError("Build cache must be separate from source and run workspaces")
        self.path = None
        self.lease = None
        self.env = None

    def _remove(self, path):
        safe_path(path)
        tree_bytes(path)
        safe_path(path / MARKER)
        safe_path(path / LEASE)
        # A locked lease is live; a missing lease is uncertain, never abandoned proof.
        try:
            lease_file = (path / LEASE).open("r+b")
        except OSError as error:
            raise GuardError(f"Uncertain validation lease; leaving {path}") from error
        with lease_file as lease:
            try:
                lock(lease)
            except OSError:
                return False
            try:
                data = json.loads((path / MARKER).read_text())
            except (OSError, ValueError) as error:
                raise GuardError(f"Uncertain validation ownership; leaving {path}") from error
            if data != self._marker(path, "idle"):
                raise GuardError(f"Uncertain validation ownership or undrained child job; leaving {path}")
        def retry_readonly(function, entry, error):
            candidate = Path(entry)
            if not candidate.is_relative_to(path):
                raise GuardError(f"Cleanup escaped owned workspace: {candidate}")
            safe_path(candidate)
            info = candidate.lstat()
            readonly = getattr(info, "st_file_attributes", 0) & 1 or not info.st_mode & stat.S_IWUSR
            if not isinstance(error, PermissionError) or not readonly:
                raise error
            os.chmod(candidate, info.st_mode | stat.S_IWUSR, follow_symlinks=False)
            function(candidate)
        try:
            # Python >=3.13 on Windows removes junction entries without traversing
            # their targets. tree_bytes has already rejected unknown reparse tags.
            # Keep ownership evidence until payload removal succeeds, so a held
            # fixture file can be retried after its external handle is released.
            for entry in path.iterdir():
                if entry.name in (MARKER, LEASE):
                    continue
                info = entry.lstat()
                if stat.S_ISLNK(info.st_mode):
                    entry.unlink()
                elif getattr(info, "st_reparse_tag", 0) == 0xA0000003:
                    entry.rmdir()  # Remove the junction entry only.
                elif stat.S_ISDIR(info.st_mode):
                    shutil.rmtree(entry, onexc=retry_readonly)
                else:
                    try:
                        entry.unlink()
                    except OSError as error:
                        retry_readonly(os.unlink, entry, error)
            (path / MARKER).unlink()
            (path / LEASE).unlink()
            path.rmdir()
        except OSError as error:
            raise GuardError(f"Cannot clean owned validation workspace {error.filename or path}: {error}") from error
        return True

    def _marker(self, path, state):
        return {"kind": "dendro-windows-validation-v1", "id": path.name, "root": str(self.root), "state": state}

    def _state(self, state):
        (self.path / MARKER).write_text(json.dumps(self._marker(self.path, state)))

    @contextlib.contextmanager
    def _registry(self):
        safe_path(self.base / ".registry-lock")
        with (self.base / ".registry-lock").open("a+b") as file:
            if file.tell() == 0:
                file.write(b"1")
                file.flush()
            try:
                lock(file)
            except OSError as error:
                raise GuardError("Another Windows validation invocation is updating its run registry") from error
            yield

    def __enter__(self):
        self.base.mkdir(parents=True, exist_ok=True)
        with self._registry():
            for candidate in self.base.iterdir():
                if candidate.name != ".registry-lock":
                    self._remove(candidate)
            self.check()
            self.path = self.base / uuid.uuid4().hex
            self.path.mkdir(mode=0o700)
            self._state("idle")
            (self.path / LEASE).write_bytes(b"1")
            self.lease = (self.path / LEASE).open("r+b")
            lock(self.lease)
        try:
            self.env = self._environment()
            self.check()
            return self
        except BaseException:
            self.__exit__(*sys.exc_info())
            raise

    def _environment(self):
        env = {key: value for key, value in self.original.items() if key.upper() in NATIVE_ENVIRONMENT}
        home = Path(self.original.get("USERPROFILE", Path.home()))
        local = Path(self.original.get("LOCALAPPDATA", home / "AppData" / "Local"))
        roaming = Path(self.original.get("APPDATA", home / "AppData" / "Roaming"))
        # Materialize tool defaults before changing profiles. Never create/install them.
        defaults = {"RUSTUP_HOME": home / ".rustup", "CARGO_HOME": home / ".cargo",
                    "MISE_DATA_DIR": local / "mise", "MISE_CACHE_DIR": local / "mise" / "cache",
                    "MISE_CONFIG_DIR": roaming / "mise", "MISE_STATE_DIR": local / "mise" / "state",
                    "UV_CACHE_DIR": local / "uv" / "cache", "UV_PYTHON_INSTALL_DIR": roaming / "uv" / "python",
                    "UV_TOOL_DIR": roaming / "uv" / "tools", "UV_TOOL_BIN_DIR": home / ".local" / "bin",
                    "PNPM_HOME": local / "pnpm"}
        for key, value in defaults.items():
            env.setdefault(key, str(value))
        for key, suffix in {"USERPROFILE": "home", "HOME": "home", "APPDATA": "home/AppData/Roaming",
                            "LOCALAPPDATA": "home/AppData/Local", "TEMP": "tmp", "TMP": "tmp",
                            "XDG_CONFIG_HOME": "home/.config", "XDG_DATA_HOME": "home/.local/share",
                            "XDG_CACHE_HOME": "home/.cache", "XDG_STATE_HOME": "home/.local/state",
                            "XDG_RUNTIME_DIR": "home/.runtime"}.items():
            directory = self.path / suffix
            directory.mkdir(parents=True, exist_ok=True)
            env[key] = str(directory)
        env.update(CARGO_TARGET_DIR=str(self.target), CARGO_PROFILE_DEV_DEBUG="0",
                   CARGO_PROFILE_TEST_DEBUG="0", CARGO_INCREMENTAL="0")
        if os.name == "nt":
            env["HOMEDRIVE"], env["HOMEPATH"] = os.path.splitdrive(env["HOME"])
        # Rustup must fail if the pinned toolchain is absent rather than download it.
        env["RUSTUP_AUTO_INSTALL"] = "0"
        for key in ("MISE_AUTO_UPDATE", "MISE_SELF_UPDATE_AUTO", "MISE_AUTO_INSTALL", "MISE_TASK_RUN_AUTO_INSTALL"):
            env[key] = "false"
        return env

    def check(self):
        for path in (self.root, self.target, self.base):
            safe_path(path)
            existing = path
            while not existing.exists():
                existing = existing.parent
            if shutil.disk_usage(existing).free < self.floor:
                raise GuardError(f"Windows validation free-space floor breached: {path} (minimum {self.floor} bytes)")
        if tree_bytes(self.target) > self.target_max:
            raise GuardError(f"Windows validation target budget exceeded: {self.target} (maximum {self.target_max} bytes)")
        if tree_bytes(self.base) > self.run_max:
            raise GuardError(f"Windows validation run budget exceeded: {self.base} (aggregate maximum {self.run_max} bytes)")

    def run(self, command, *, cwd=None, check=False, stdout=None, stderr=None, text=False, **kwargs):
        if kwargs:
            raise GuardError(f"Unsupported guarded subprocess options: {sorted(kwargs)}")
        self.check()
        job = WindowsJob() if os.name == "nt" else None
        process = None
        # File-backed capture avoids a full pipe blocking the monitored child.
        with contextlib.ExitStack() as stack:
            captured = stack.enter_context(tempfile.TemporaryFile(dir=self.path)) if stdout == subprocess.PIPE else None
            gate = self.path / ("launch-" + uuid.uuid4().hex)
            released = False
            try:
                self._state("running")
                helper = [sys.executable, str(Path(__file__).resolve()), "--child", json.dumps(list(command)), str(gate)]
                process = subprocess.Popen(helper, cwd=cwd or self.root, env=self.env,
                                           stdout=captured if captured else stdout, stderr=stderr,
                                           start_new_session=os.name != "nt")
                if job:
                    job.assign(process)
                ready = gate.with_suffix(".ready")
                ready.write_text("go")
                ready.replace(gate)
                released = True
                while process.poll() is None:
                    self.check()
                    time.sleep(0.2)
                self.check()
                output = None
                if captured:
                    captured.seek(0)
                    output = captured.read()
                    if text:
                        output = output.decode()
                result = subprocess.CompletedProcess(command, process.returncode, output)
                if check:
                    result.check_returncode()
                return result
            finally:
                # An unassigned helper has never been released to launch work.
                if process and not released and process.poll() is None:
                    process.kill()
                if job:
                    job.close()
                elif process:
                    import signal
                    with contextlib.suppress(ProcessLookupError):
                        os.killpg(process.pid, signal.SIGKILL)
                if process:
                    process.wait()
                gate.unlink(missing_ok=True)
                self._state("idle")

    def check_output(self, command, **kwargs):
        return self.run(command, stdout=subprocess.PIPE, check=True, **kwargs).stdout

    def __exit__(self, *exc):
        if self.path:
            with self._registry():
                if self.lease:
                    self.lease.close()
                    self.lease = None
                if not self._remove(self.path):
                    raise GuardError(f"Cannot acquire owned validation cleanup lease: {self.path}")


class ExternalWindowsGuard(WindowsGuard):
    """Contain host clients without redirecting their authenticated tool profiles."""
    def _environment(self):
        return self.original.copy()

    def check(self):
        for path in (self.root, self.base):
            existing = safe_path(path)
            while not existing.exists():
                existing = existing.parent
            if shutil.disk_usage(existing).free < self.floor:
                raise GuardError(f"Windows external-command free-space floor breached: {path} (minimum {self.floor} bytes)")
        if tree_bytes(self.base) > self.run_max:
            raise GuardError("Windows external-command scratch budget exceeded")


def main():
    if len(sys.argv) == 4 and sys.argv[1] == "--child":
        gate = Path(sys.argv[3])
        deadline = time.monotonic() + 10
        while not gate.exists() and time.monotonic() < deadline:
            time.sleep(0.01)
        if not gate.exists() or gate.read_text() != "go":
            raise SystemExit("Guard child requires an owned launch barrier")
        command = json.loads(sys.argv[2])
        command[0] = shutil.which(command[0]) or command[0]
        return subprocess.run(command).returncode
    if len(sys.argv) < 4 or sys.argv[1:3] != ["--external", "--"]:
        raise SystemExit("Usage: windows_guard.py --external -- command [arguments]")
    command = sys.argv[3:]
    if sys.platform != "win32":
        return subprocess.run(command).returncode
    try:
        with ExternalWindowsGuard(Path.cwd()) as guard:
            return guard.run(command).returncode
    except GuardError as error:
        raise SystemExit(str(error)) from error


if __name__ == "__main__":
    raise SystemExit(main())
