#!/usr/bin/env python3
"""Snapshot lifecycle proof (S1-S3, S6) against one dendro binary.

Every case runs in its own directory under --scratch with an isolated TMPDIR.
Signals go only to the Popen child this script started, never by name. The
synthetic OpenRig source is WAL-backed and held open by this script, so its
main, -wal and -shm files exist and must stay byte- and mtime-identical.

Usage:
  python3 scripts/verify_snapshot_lifecycle.py --binary target/debug/dendro \\
      --scratch .agent-runs/proof/snapshot-final \\
      --cases permissions,success,error,sigint,sigterm,sigkill-restart,active-owner,unrelated,cleanup-failure,missing-key
"""
import argparse
import hashlib
import json
import os
import shutil
import signal
import sqlite3
import stat
import subprocess
import sys
import time
from pathlib import Path

ALL_CASES = ["permissions", "success", "error", "sigint", "sigterm", "sigkill-restart",
             "active-owner", "unrelated", "cleanup-failure", "missing-key"]
CANARY = "sk-ant-ZZLIFEcanary"
STAGE = ".dendro-stage-"
MARKER = b"dendro-stage v1\n"
results = []


def check(case, name, ok, detail=""):
    results.append({"case": case, "check": name, "ok": bool(ok), "detail": str(detail)[:300]})
    print(("PASS" if ok else "FAIL"), f"{case}: {name}", "" if ok else detail)


def mode(path):
    return stat.S_IMODE(os.lstat(path).st_mode)


def manifest(paths):
    out = {}
    for p in paths:
        st = os.lstat(p)
        out[str(p)] = (hashlib.sha256(Path(p).read_bytes()).hexdigest(), st.st_mtime_ns)
    return out


class Source:
    """A WAL-backed synthetic OpenRig database kept open by this process."""

    def __init__(self, path, rows, pad=0):
        self.path = path
        self.conn = sqlite3.connect(path)
        self.conn.execute("PRAGMA journal_mode=WAL")
        self.conn.execute("PRAGMA wal_autocheckpoint=0")
        self.conn.execute("CREATE TABLE events (seq INTEGER, type TEXT, payload TEXT, created_at TEXT)")
        self.conn.executemany(
            "INSERT INTO events VALUES (?,?,?,?)",
            ((i, "seat.activity_changed",
              json.dumps({"sessionName": f"seat-{i % 40}", "state": "running",
                          "at": "2026-10-09T00:00:00Z", "note": CANARY, "pad": "p" * pad}),
              "2026-10-09 00:00:00") for i in range(rows)))
        self.conn.commit()
        self.conn.execute("PRAGMA wal_checkpoint(PASSIVE)")
        # Uncheckpointed rows keep a nonempty -wal next to the main file.
        self.conn.execute("INSERT INTO events VALUES (?,?,?,?)",
                          (rows, "seat.activity_changed",
                           json.dumps({"sessionName": "seat-wal", "state": "idle",
                                       "at": "2026-10-09T00:00:01Z"}), "2026-10-09 00:00:01"))
        self.conn.commit()
        self.files = [path, Path(f"{path}-wal"), Path(f"{path}-shm")]
        assert all(f.exists() for f in self.files), "WAL-backed source needs main, -wal and -shm"
        assert self.files[1].stat().st_size > 0, "nonempty -wal"

    def manifest(self):
        return manifest(self.files)


class Case:
    def __init__(self, root, name):
        self.name = name
        self.dir = root / name
        shutil.rmtree(self.dir, ignore_errors=True)
        self.out = self.dir / "out"
        self.tmp = self.dir / "tmp"
        for d in (self.out, self.tmp):
            d.mkdir(parents=True)
        self.out.chmod(0o700)

    def env(self):
        return dict(os.environ, TMPDIR=str(self.tmp))

    def stages(self, parent=None):
        parent = parent or self.out
        return sorted(p for p in parent.iterdir() if p.name.startswith(STAGE))


def start(binary, case, db, source, umask=None):
    args = [binary, "run", "--db", str(db), "--root", f"openrig={source}", "--frozen"]
    pre = (lambda: os.umask(umask)) if umask is not None else None
    return subprocess.Popen(args, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
                            env=case.env(), preexec_fn=pre)


def run(binary, case, args):
    p = subprocess.run([binary, *args], capture_output=True, text=True, env=case.env(), timeout=300)
    return p.returncode, p.stdout, p.stderr


def finish(p, case, name, limit=300):
    """Waits for this script's own child; on timeout, stops it and fails the check."""
    try:
        return p.communicate(timeout=limit)
    except subprocess.TimeoutExpired:
        p.send_signal(signal.SIGTERM)
        out, err = p.communicate(timeout=60)
        check(case, f"{name}: finished within {limit}s", False, "timed out; stopped own child")
        return out, err


def wait_for(predicate, limit=60.0):
    end = time.time() + limit
    while time.time() < end:
        value = predicate()
        if value:
            return value
        time.sleep(0.005)
    return None


def in_copy(case, source_size):
    """Barrier: the raw snapshot exists and is still being copied."""
    for s in case.stages():
        snap = s / "snapshot" / "openrig.sqlite"
        if snap.exists() and 0 < snap.stat().st_size < source_size:
            return s
    return None


def in_ingest(case):
    """Barrier: the snapshot is complete and rows are being written to the staged db WAL."""
    for s in case.stages():
        wal = s / "analytics.duckdb.wal"
        if (s / "snapshot" / "openrig.sqlite-wal").exists() and wal.exists() and wal.stat().st_size > 0:
            return s
    return None


def snapshot_ready(case):
    """Barrier: the staged db copy and the complete raw snapshot both exist."""
    for s in case.stages():
        if (s / "analytics.duckdb").exists() and (s / "snapshot" / "openrig.sqlite-shm").exists():
            return s
    return None


def leftovers(case):
    raw = [str(p) for p in case.tmp.rglob("*")]
    return [p.name for p in case.stages()] + raw


def no_canary(*texts):
    return all(CANARY not in t and "ZZLIFE" not in t for t in texts)


def baseline_db(binary, case, small):
    db = case.out / "analytics.duckdb"
    code, out, err = run(binary, case, ["run", "--db", str(db), "--root", f"openrig={small.path}", "--frozen"])
    assert code == 0, err
    return db, hashlib.sha256(db.read_bytes()).hexdigest()


def case_permissions(binary, root, big, small):
    c = Case(root, "permissions")
    seen = {}
    # First run creates the db (DuckDB-created staged file); second copies it (Dendro-created 0600 copy).
    for attempt in ("new-db", "existing-db"):
        p = start(binary, c, c.out / "analytics.duckdb", big.path, umask=0)
        # New db: wait for rows being written. Existing db: rows are already
        # ingested, so wait for the staged copy plus the complete snapshot.
        barrier = in_ingest if attempt == "new-db" else snapshot_ready
        stage = wait_for(lambda: barrier(c))
        if stage:
            for f in [stage, stage / "snapshot", *stage.rglob("*")]:
                seen[f"{attempt}:{f.relative_to(stage.parent)}"] = oct(mode(f))
            check("permissions", f"{attempt}: stage dir 0700 under umask 000", mode(stage) == 0o700, oct(mode(stage)))
            check("permissions", f"{attempt}: snapshot dir 0700", mode(stage / "snapshot") == 0o700)
            for f in (stage / "snapshot").iterdir():
                check("permissions", f"{attempt}: raw snapshot {f.name} 0600", mode(f) == 0o600, oct(mode(f)))
            marker = stage / ".dendro-stage"
            check("permissions", f"{attempt}: marker 0600 with fixed content",
                  mode(marker) == 0o600 and marker.read_bytes() == MARKER)
            staged = stage / "analytics.duckdb"
            if attempt == "existing-db":
                check("permissions", "staged COPY of the existing db is 0600", mode(staged) == 0o600, oct(mode(staged)))
            for f in stage.iterdir():
                if f.name.startswith("analytics.duckdb"):
                    # Disclosed per the L1 disposition: DuckDB-created files take the umask.
                    st = os.lstat(f)
                    check("permissions", f"{attempt}: {f.name} is a regular file owned by us inside the 0700 stage",
                          stat.S_ISREG(st.st_mode) and st.st_uid == os.getuid() and mode(stage) == 0o700)
        check("permissions", f"{attempt}: in-flight barrier reached", stage is not None)
        out, err = finish(p, "permissions", attempt)
        check("permissions", f"{attempt}: run exit 0", p.returncode == 0, err[-300:])
    check("permissions", "nothing left in TMPDIR or as stages", leftovers(c) == [], leftovers(c))
    return seen


def case_success(binary, root, big, small):
    c = Case(root, "success")
    before = small.manifest()
    code, out, err = run(binary, c, ["run", "--db", str(c.out / "a.duckdb"), "--root", f"openrig={small.path}", "--frozen"])
    check("success", "exit 0", code == 0, err[-300:])
    check("success", "no retained stage or TMPDIR file", leftovers(c) == [], leftovers(c))
    check("success", "source main/WAL/SHM hash and mtime unchanged", small.manifest() == before)
    check("success", "no canary in output", no_canary(out, err))


def case_error(binary, root, big, small):
    c = Case(root, "error")
    bad = c.dir / "not-sqlite.sqlite"
    bad.write_bytes((CANARY + " not a database ").encode() * 4096)
    before = manifest([bad])
    code, out, err = run(binary, c, ["run", "--db", str(c.out / "a.duckdb"), "--root", f"openrig={bad}", "--frozen"])
    check("error", "handled failure exits nonzero", code != 0, code)
    check("error", "no retained stage or TMPDIR file", leftovers(c) == [], leftovers(c))
    check("error", "source unchanged", manifest([bad]) == before)
    check("error", "no canary in output", no_canary(out, err))
    check("error", "never reports ready", '"status":"ready"' not in out)


def copy_progress(case, source_size):
    """Evidence for the copy barrier: (snapshot bytes, source bytes) while the
    raw snapshot is still being copied, else None."""
    for s in case.stages():
        snap = s / "snapshot" / "openrig.sqlite"
        try:
            size = snap.stat().st_size if snap.exists() else 0
        except OSError:
            continue
        if 0 < size < source_size:
            return (size, source_size)
    return None


def signal_case(binary, root, big, small, name, sig, code_expected, huge):
    # The copy phase uses a much larger source, so the copy itself is observed
    # in progress (QA d5ebb07: with the 20 MB source the copy finished before
    # the barrier was seen and the signal reached a run that had already
    # exited, 101/105). No sleep stands in for observed work.
    for phase, source, barrier in (("copy", huge, lambda c: in_copy(c, huge.path.stat().st_size)),
                                   ("ingest", big, in_ingest)):
        c = Case(root, f"{name}-{phase}")
        db, old = baseline_db(binary, c, small)
        before = source.manifest()
        p = start(binary, c, db, source.path)
        stage = wait_for(lambda: barrier(c) if p.poll() is None else "exited", 120)
        evidence = copy_progress(c, huge.path.stat().st_size) if phase == "copy" else None
        alive = p.poll() is None
        reached = stage not in (None, "exited") and alive
        check(name, f"{phase}: barrier reached with real work active, child {p.pid} still running",
              reached, f"stage={stage} alive={alive}")
        if phase == "copy":
            print(f"INFO {name}: copy barrier evidence pid={p.pid} snapshot_bytes/source_bytes={evidence}")
        if not reached:
            # A child that already exited, or a missed barrier, never passes.
            out, err = finish(p, name, phase, 300)
            check(name, f"{phase}: signal not sent (no active work observed)", False, f"exit {p.returncode}")
            continue
        p.send_signal(sig)
        out, err = finish(p, name, phase, 120)
        check(name, f"{phase}: exit {code_expected}", p.returncode == code_expected, f"{p.returncode} {err[-200:]}")
        check(name, f"{phase}: cleanup confirmed, not timed out", "private staging removed" in err and "FAILED" not in err, err[-200:])
        time.sleep(1.0)
        check(name, f"{phase}: no stage or TMPDIR file, none recreated", leftovers(c) == [], leftovers(c))
        new = hashlib.sha256(db.read_bytes()).hexdigest()
        check(name, f"{phase}: db is the old complete db", new == old)
        check(name, f"{phase}: source unchanged", source.manifest() == before)
        check(name, f"{phase}: no canary in output", no_canary(out, err))
    # Early signal: before any stage exists.
    c = Case(root, f"{name}-early")
    p = start(binary, c, c.out / "a.duckdb", big.path)
    p.send_signal(sig)
    out, err = finish(p, name, "early", 120)
    check(name, "early signal: no stage or TMPDIR file", leftovers(c) == [], f"{p.returncode} {leftovers(c)}")
    # Simultaneous completion: a tiny source finishes around the signal.
    c = Case(root, f"{name}-late")
    p = start(binary, c, c.out / "a.duckdb", small.path)
    wait_for(lambda: c.stages() or p.poll() is not None, 10)
    p.send_signal(sig)
    out, err = finish(p, name, "late", 120)
    check(name, "simultaneous completion: exit 0 or interrupted", p.returncode in (0, code_expected), p.returncode)
    check(name, "simultaneous completion: no stage or TMPDIR file", leftovers(c) == [], leftovers(c))


def case_sigkill(binary, root, big, small):
    c = Case(root, "sigkill-restart")
    db = c.out / "a.duckdb"
    p = start(binary, c, db, big.path)
    stage = wait_for(lambda: in_ingest(c))
    check("sigkill-restart", "barrier reached", stage is not None)
    p.send_signal(signal.SIGKILL)
    finish(p, "sigkill-restart", "killed run", 60)
    stale = c.stages()
    check("sigkill-restart", "SIGKILL leaves one private owned marked stage",
          len(stale) == 1 and mode(stale[0]) == 0o700 and os.lstat(stale[0]).st_uid == os.getuid()
          and (stale[0] / ".dendro-stage").read_bytes() == MARKER, [s.name for s in stale])
    check("sigkill-restart", "nothing in TMPDIR", list(c.tmp.rglob("*")) == [])
    # Host crash is simulated by the same rule: the next run recovers.
    code, out, err = run(binary, c, ["run", "--db", str(db), "--root", f"openrig={small.path}", "--frozen"])
    check("sigkill-restart", "next run exit 0", code == 0, err[-300:])
    staging = json.loads(out).get("staging", []) if code == 0 else []
    check("sigkill-restart", "next run reports the removal",
          any(n.startswith("stale staging removed: " + stale[0].name) for n in staging) if stale else False, staging)
    check("sigkill-restart", "stale stage removed, nothing left", leftovers(c) == [], leftovers(c))


def case_active(binary, root, big, small):
    c = Case(root, "active-owner")
    a = start(binary, c, c.out / "first.duckdb", big.path)
    stage = wait_for(lambda: in_ingest(c))
    check("active-owner", "run A active", stage is not None)
    code, out, err = run(binary, c, ["run", "--db", str(c.out / "second.duckdb"), "--root", f"openrig={small.path}", "--frozen"])
    check("active-owner", "run B on another file in the same parent is refused", code != 0 and "lock is busy" in err, err[-200:])
    check("active-owner", "A's stage survives B", stage is not None and stage.exists())
    out, err = finish(a, "active-owner", "run A")
    check("active-owner", "A completes", a.returncode == 0, err[-200:])
    check("active-owner", "nothing left", leftovers(c) == [], leftovers(c))


def case_unrelated(binary, root, big, small):
    c = Case(root, "unrelated")
    o = c.out
    def private(path):
        path.mkdir(mode=0o700)
        return path
    controls = {}
    controls["not-hex"] = private(o / ".dendro-stage-NOTHEX")
    (o / ".dendro-stage-notes").write_text("x")
    controls["notes-file"] = o / ".dendro-stage-notes"
    controls["unmarked-empty-init"] = private(o / ".dendro-stage-00000000000000c1.init")
    unknown = private(o / ".dendro-stage-00000000000000c2")
    (unknown / ".dendro-stage").write_bytes(MARKER)
    (unknown / ".dendro-stage").chmod(0o600)
    (unknown / "nested").mkdir()
    (unknown / "nested" / "unknown.txt").write_text("x")
    controls["nested-unknown"] = unknown
    wide = private(o / ".dendro-stage-00000000000000c3")
    (wide / ".dendro-stage").write_bytes(MARKER)
    (wide / ".dendro-stage").chmod(0o600)
    wide.chmod(0o755)
    controls["wrong-mode"] = wide
    linked = private(o / ".dendro-stage-00000000000000c4")
    (linked / ".dendro-stage").write_bytes(MARKER)
    (linked / ".dendro-stage").chmod(0o600)
    (linked / "analytics.duckdb").write_text("x")
    os.link(linked / "analytics.duckdb", c.dir / "outside-hardlink")
    controls["hardlink"] = linked
    bad = private(o / ".dendro-stage-00000000000000c5")
    (bad / ".dendro-stage").write_text("not the marker")
    controls["bad-marker"] = bad
    target = c.dir / "symlink-target"
    target.mkdir()
    (target / "keep.txt").write_text(CANARY)
    (o / ".dendro-stage-00000000000000c6").symlink_to(target)
    controls["symlink"] = o / ".dendro-stage-00000000000000c6"
    code, out, err = run(binary, c, ["run", "--db", str(o / "a.duckdb"), "--root", f"openrig={small.path}", "--frozen"])
    check("unrelated", "run exit 0", code == 0, err[-300:])
    for name, path in controls.items():
        check("unrelated", f"{name} control preserved", os.path.lexists(path))
    check("unrelated", "symlink target untouched", (target / "keep.txt").read_text() == CANARY)
    staging = json.loads(out).get("staging", []) if code == 0 else []
    kept = [n for n in staging if n.startswith("stale staging kept")]
    check("unrelated", "every exact-format control is reported as kept", len(kept) == 6, staging)
    check("unrelated", "no canary in output", no_canary(out, err))


def case_cleanup_failure(binary, root, big, small):
    c = Case(root, "cleanup-failure")
    db = c.out / "a.duckdb"
    p = start(binary, c, db, big.path)
    stage = wait_for(lambda: in_ingest(c))
    check("cleanup-failure", "barrier reached", stage is not None)
    # Safe synthetic refusal on our own file: the snapshot dir loses write permission.
    if stage:
        (stage / "snapshot").chmod(0o500)
    out, err = finish(p, "cleanup-failure", "run")
    check("cleanup-failure", "run fails nonzero, never ready", p.returncode != 0 and '"status":"ready"' not in out, p.returncode)
    check("cleanup-failure", "visible sanitized cleanup diagnostic",
          "cleanup failed" in err and no_canary(out, err), err[-300:])
    # Marker-last cleanup: the failed stage must stay recoverable.
    check("cleanup-failure", "stage still exists after the failed cleanup", bool(stage) and stage.exists())
    if stage and stage.exists():
        marker = stage / ".dendro-stage"
        check("cleanup-failure", "stage stays 0700", mode(stage) == 0o700, oct(mode(stage)))
        check("cleanup-failure", "marker intact: fixed content and 0600",
              marker.exists() and marker.read_bytes() == MARKER and mode(marker) == 0o600)
        # Restore only the synthetic permission this case removed.
        (stage / "snapshot").chmod(0o700)
    code, out2, err2 = run(binary, c, ["run", "--db", str(db), "--root", f"openrig={small.path}", "--frozen"])
    staging = json.loads(out2).get("staging", []) if code == 0 else []
    check("cleanup-failure", "next run removes the leftover stage",
          code == 0 and any(n.startswith("stale staging removed") for n in staging), staging)
    check("cleanup-failure", "nothing left", leftovers(c) == [], leftovers(c))


def case_missing_key(binary, root, big, small):
    c = Case(root, "missing-key")
    db = c.out / "a.duckdb"
    code, out, err = run(binary, c, ["run", "--db", str(db), "--root", f"openrig={small.path}", "--frozen"])
    key = Path(f"{db}.key")
    check("missing-key", "first run with no db creates its key 0600", code == 0 and key.exists() and mode(key) == 0o600)
    raw = key.read_bytes()
    db_before = hashlib.sha256(db.read_bytes()).hexdigest()
    key.unlink()
    code, out, err = run(binary, c, ["run", "--db", str(db), "--root", f"openrig={small.path}", "--frozen"])
    check("missing-key", "ingest refused", code != 0, code)
    check("missing-key", "db unchanged, no key recreated",
          hashlib.sha256(db.read_bytes()).hexdigest() == db_before and not key.exists())
    for args in (["report", "tokens", "--db", str(db)], ["trace", "--metric-id", "tokens", "--db", str(db)]):
        code, out, err = run(binary, c, args)
        check("missing-key", f"{args[0]} runs without the key", code == 0, err[-200:])
        check("missing-key", f"{args[0]} creates no key", not key.exists())
        check("missing-key", f"{args[0]} prints no key material",
              raw.hex() not in out + err and raw[4:].hex() not in out + err)
    check("missing-key", "nothing left", leftovers(c) == [], leftovers(c))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", required=True)
    parser.add_argument("--scratch", required=True)
    parser.add_argument("--cases", default=",".join(ALL_CASES))
    args = parser.parse_args()
    binary = str(Path(args.binary).resolve())
    root = Path(args.scratch).resolve()
    shutil.rmtree(root, ignore_errors=True)
    root.mkdir(parents=True)
    root.chmod(0o700)
    # Large in bytes (the copy phase is observable), small in rows (runs finish).
    big = Source(root / "big.sqlite", 2_000, pad=10_000)
    small = Source(root / "small.sqlite", 50)
    # About 100 MB, only for the signal cases' copy phase.
    huge = Source(root / "huge.sqlite", 2_000, pad=50_000) if {"sigint", "sigterm"} & set(args.cases.split(",")) else None
    cases = args.cases.split(",")
    unknown = [c for c in cases if c not in ALL_CASES]
    if unknown:
        sys.exit(f"unknown cases: {unknown}")
    seen = {}
    for case in cases:
        if case == "permissions":
            seen = case_permissions(binary, root, big, small)
        elif case == "sigint":
            signal_case(binary, root, big, small, "sigint", signal.SIGINT, 130, huge)
        elif case == "sigterm":
            signal_case(binary, root, big, small, "sigterm", signal.SIGTERM, 143, huge)
        else:
            {"success": case_success, "error": case_error, "sigkill-restart": case_sigkill,
             "active-owner": case_active, "unrelated": case_unrelated,
             "cleanup-failure": case_cleanup_failure, "missing-key": case_missing_key}[case](binary, root, big, small)
    failed = [r for r in results if not r["ok"]]
    summary = {"binary": binary, "binary_sha256": hashlib.sha256(Path(binary).read_bytes()).hexdigest(),
               "cases": cases, "checks": len(results), "failed": len(failed),
               "observed_modes": seen, "results": results}
    (root / "results.json").write_text(json.dumps(summary, indent=1))
    print(f"RESULT {len(results) - len(failed)}/{len(results)} checks passed over {len(cases)} cases")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
