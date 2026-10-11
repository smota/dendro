#!/usr/bin/env python3
"""AC12 private-copy proof (cycle 2, rows W1-W6) against one dendro binary.

Uses QA's ptrace observer UNCHANGED (imported from --observer-dir) and the
builder's synthetic WAL fixture. Synthetic sources only; never the owner's live
store. Every source here is a WAL-mode SQLite file kept open by this script, so
the original main, -wal and -shm exist and the -wal is non-empty.

W1 syscalls: default and --frozen runs open the original main and -wal only
   read-only, never touch the -shm, and make no mutation or network attempt;
   observer errors are zero. Negative control: the 06cd1a7 binary still opens
   the -wal write-capably.
W2 fidelity: the WAL-only committed row appears exactly once in report and
   trace; an independent source oracle matches; a main-only copy omits it.
W3 provenance and replay: original path and accepted revision in report and
   trace, no temporary path; trace-only replay; append, update and replacement
   without duplicate source keys; a concurrent producer with PASSIVE and
   TRUNCATE checkpoints yields a coherent cutoff or a clean bounded failure.
W4 the stability helper's unit tests.
W5 default and --frozen give equal reports on a static source.
W6 WAL-absent source, read-only permissions, and output alias refusal.

Usage:
  python3 scripts/verify_ac12_copy.py --binary /abs/target/debug/dendro \\
      --scratch /abs/new/ac12-copy [--negative-binary /abs/06cd1a7/dendro]
"""
import argparse
import hashlib
import importlib.util
import json
import os
import shutil
import sqlite3
import stat
import subprocess
import sys
import threading
import time
from pathlib import Path

sys.dont_write_bytecode = True
REPO = Path(__file__).resolve().parent.parent
FROM, TO = "2026-10-09T00:00:00Z", "2026-10-09T00:01:00Z"
WRITE_BITS = os.O_WRONLY | os.O_RDWR | os.O_CREAT | os.O_TRUNC | os.O_APPEND
results = []


def check(row, name, ok, detail=""):
    results.append({"row": row, "check": name, "ok": bool(ok), "detail": str(detail)[:400]})
    print(("PASS" if ok else "FAIL"), f"{row}: {name}", "" if ok else detail, flush=True)


def load(path, name):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def manifest(paths):
    out = {}
    for p in paths:
        p = Path(p)
        if p.exists():
            st = p.stat()
            out[p.name] = [hashlib.sha256(p.read_bytes()).hexdigest(), st.st_size, st.st_mtime_ns, st.st_ino]
        else:
            out[p.name] = None
    return out


def cli(binary, args, timeout=600):
    p = subprocess.run([binary, *args], capture_output=True, text=True, timeout=timeout)
    return p.returncode, p.stdout, p.stderr


def trace_activity_keys(binary, db):
    code, out, err = cli(binary, ["trace", "--metric-id", "activity", "--db", str(db), "--from", FROM, "--to", TO])
    if code != 0:
        return None, None, err
    trace = json.loads(out)
    keys = [e["source_key"] for e in trace["inputs"]["activity_events"]]
    return keys, trace, ""


def seqs(keys):
    return sorted(int(k.rsplit(":", 1)[1]) for k in keys if k.startswith("seat.activity_changed:"))


def source_events(observation, source):
    return [e for e in observation["events"] if any(str(p).startswith(str(source)) for p in e.get("paths", []))]


def w1_observe(observer, binary, base, Source, frozen, producer_on, label, expect_clean=True):
    case = base / label
    case.mkdir(parents=True)
    src = Source(case / "source.sqlite", 200)
    before = manifest(src.files)
    stop = threading.Event()

    def producer():
        c = sqlite3.connect(src.path)
        i = 1000
        while not stop.wait(0.015):
            c.execute("INSERT INTO events VALUES (?,?,?,?)", (i, "seat.activity_changed",
                      json.dumps({"sessionName": "producer", "state": "idle", "at": "2026-10-09T00:00:01Z"}),
                      "2026-10-09 00:00:01"))
            c.commit()
            i += 1
        c.close()

    thread = threading.Thread(target=producer) if producer_on else None
    if thread:
        thread.start()
    try:
        command = [binary, "run", *(["--frozen"] if frozen else []), "--db", str(case / "out" / "a.duckdb"),
                   "--root", f"openrig={src.path}"]
        r = observer.observe(command, [str(src.path), str(src.path) + "-wal", str(src.path) + "-shm"], producer_on)
    finally:
        stop.set()
        if thread:
            thread.join()
    after = manifest(src.files)
    (case / "observation.json").write_text(json.dumps(r, indent=1))
    (case / "source-before.json").write_text(json.dumps(before, indent=1))
    (case / "source-after.json").write_text(json.dumps(after, indent=1))
    events = source_events(r, src.path)
    shm = [e for e in events if any(str(p).endswith("-shm") for p in e["paths"])]
    writes = [e for e in events if e["syscall"] == "openat" and e.get("flags", 0) & WRITE_BITS]
    mutations = [e for e in events if e.get("mutation")]
    nonempty = src.files[1].stat().st_size > 0
    src.conn.close()
    if not expect_clean:
        return r, writes
    check("W1", f"{label}: root exit 0", r["root_exit"] == 0, r["root_exit"])
    check("W1", f"{label}: observer errors 0", not r["errors"], r["errors"])
    check("W1", f"{label}: source violations 0", r["source_violations"] == 0, r["source_violations"])
    check("W1", f"{label}: network attempts 0", r["network_attempts"] == 0, r["network_attempts"])
    check("W1", f"{label}: no write-capable open of source main or -wal", writes == [], writes[:3])
    check("W1", f"{label}: no mutation event on any source path", mutations == [], mutations[:3])
    check("W1", f"{label}: source -shm never accessed", shm == [], [e["syscall"] for e in shm][:5])
    check("W1", f"{label}: source main and -wal were actually read", any(e["syscall"] == "openat" for e in events))
    check("W1", f"{label}: source -wal non-empty", nonempty)
    if not producer_on:
        check("W1", f"{label}: static source byte+mtime unchanged", before == after)
    return r, writes


def w2_w3_w5(binary, base, Source):
    static = base / "static"
    static.mkdir()
    src = Source(static / "source.sqlite", 200)
    oracle = list(range(0, 201))  # generator: rows 0..199 checkpointed, 200 (seat-wal) only in the WAL
    db = static / "out" / "a.duckdb"
    code, out, err = cli(binary, ["run", "--db", str(db), "--root", f"openrig={src.path}"])
    check("W2", "default static run exit 0", code == 0, err[-300:])
    note = json.loads(out)["sources"][0] if code == 0 else ""
    keys, trace, err = trace_activity_keys(binary, db)
    got = seqs(keys or [])
    check("W2", "trace activity rows equal the independent source oracle", got == oracle, (len(got), got[-3:]))
    check("W2", "WAL-only committed row (seq 200) exactly once", (keys or []).count("seat.activity_changed:200") == 1)
    code, rep, err = cli(binary, ["report", "activity", "--db", str(db), "--from", FROM, "--to", TO])
    report = json.loads(rep)["payload"] if code == 0 else {}
    check("W2", "report has the WAL-only seat", any(row["seat"] == "seat-wal" for row in report.get("activity", [])))
    # Main-only control: without the WAL the row must be absent.
    main_only = base / "main-only"
    main_only.mkdir()
    shutil.copyfile(src.path, main_only / "source.sqlite")
    mdb = main_only / "out" / "a.duckdb"
    code, out2, err = cli(binary, ["run", "--db", str(mdb), "--root", f"openrig={main_only / 'source.sqlite'}"])
    mkeys, _, _ = trace_activity_keys(binary, mdb)
    check("W2", "main-only control omits the WAL-only row (non-vacuous oracle)",
          code == 0 and "seat.activity_changed:200" not in (mkeys or []) and len(mkeys or []) == 200, (code, len(mkeys or [])))
    # W3 provenance.
    sources = trace["inputs"]["openrig_sources"] if trace else []
    revision = note.split("revision=")[1].split()[0] if "revision=" in note else ""
    check("W3", "trace names the original source path", any(s["path"] == str(src.path) for s in sources), sources)
    check("W3", "trace revision matches the run's accepted revision",
          bool(revision) and any(s["revision"].startswith(revision) for s in sources), (revision, sources))
    check("W3", "report and trace carry the same provenance", report.get("openrig_sources") == sources)
    texts = out + json.dumps(trace) + rep
    check("W3", "no temporary stage or snapshot path in any output", ".dendro-stage" not in texts and "/snapshot/" not in texts)
    (static / "trace-1.json").write_text(json.dumps(trace))
    # Trace-only replay: per-seat event counts from the trace alone.
    replay = {}
    for e in trace["inputs"]["activity_events"]:
        replay[e["seat"]] = replay.get(e["seat"], 0) + 1
    check("W3", "trace-only replay reproduces the source rows per seat",
          replay == {"seat-" + str(i % 40): 5 for i in range(40)} | {"seat-wal": 1}, replay)
    # W5: --frozen on the same static source gives the same report.
    fdb = static / "out-frozen" / "a.duckdb"
    code, outf, err = cli(binary, ["run", "--frozen", "--db", str(fdb), "--root", f"openrig={src.path}"])
    revf = outf.split("revision=")[1].split()[0] if code == 0 and "revision=" in outf else None
    check("W5", "--frozen static run exit 0 with the same accepted revision", code == 0 and revf == revision, (revf, revision))
    for kind in ("activity", "decisions", "tokens", "handoffs", "links"):
        a = json.loads(cli(binary, ["report", kind, "--db", str(db), "--from", FROM, "--to", TO])[1])
        b = json.loads(cli(binary, ["report", kind, "--db", str(fdb), "--from", FROM, "--to", TO])[1])
        for side in (a, b):
            for s in side["payload"].get("openrig_sources", []):
                s.pop("source_id", None)  # sealed with each db's own key
        check("W5", f"report {kind} equal for default and --frozen", a == b)
    # W3 append, update, replacement on the same output db.
    for i in range(300, 310):
        src.conn.execute("INSERT INTO events VALUES (?,?,?,?)", (i, "seat.activity_changed",
                         json.dumps({"sessionName": "seat-append", "state": "running", "at": "2026-10-09T00:00:02Z"}),
                         "2026-10-09 00:00:02"))
    src.conn.commit()
    code, out3, err = cli(binary, ["run", "--db", str(db), "--root", f"openrig={src.path}"])
    keys3, trace3, _ = trace_activity_keys(binary, db)
    check("W3", "append: new rows once, old rows once, no duplicate keys",
          code == 0 and seqs(keys3) == oracle + list(range(300, 310)) and len(keys3) == len(set(keys3)), (code, len(keys3 or [])))
    rev3 = out3.split("revision=")[1].split()[0] if code == 0 else ""
    check("W3", "append: accepted revision changed", rev3 and rev3 != revision)
    check("W3", "earlier trace file retained unchanged", json.loads((static / "trace-1.json").read_text()) == trace)
    src.conn.execute("UPDATE events SET payload = ? WHERE seq = 5",
                     (json.dumps({"sessionName": "seat-5", "state": "idle", "at": "2026-10-09T00:00:00Z"}),))
    src.conn.commit()
    code, out4, err = cli(binary, ["run", "--db", str(db), "--root", f"openrig={src.path}"])
    keys4, trace4, _ = trace_activity_keys(binary, db)
    states = [e["state"] for e in trace4["inputs"]["activity_events"] if e["source_key"] == "seat.activity_changed:5"] if trace4 else []
    check("W3", "update: no duplicate keys after re-ingest", code == 0 and len(keys4) == len(set(keys4)), code)
    check("W3", "update: the event keeps its first-ingested facts (source key is the identity)", states == ["running"], states)
    src.conn.close()
    replacement = static / "replacement.sqlite"
    r = sqlite3.connect(replacement)
    r.execute("CREATE TABLE events (seq INTEGER, type TEXT, payload TEXT, created_at TEXT)")
    r.executemany("INSERT INTO events VALUES (?,?,?,?)", [(i, "seat.activity_changed",
                  json.dumps({"sessionName": "seat-new", "state": "running", "at": "2026-10-09T00:00:03Z"}),
                  "2026-10-09 00:00:03") for i in (0, 1, 500)])
    r.commit()
    r.close()
    for suffix in ("-wal", "-shm"):
        Path(str(src.path) + suffix).unlink(missing_ok=True)
    os.replace(replacement, src.path)
    code, out5, err = cli(binary, ["run", "--db", str(db), "--root", f"openrig={src.path}"])
    keys5, _, _ = trace_activity_keys(binary, db)
    check("W3", "replacement: new file ingested, no duplicate keys",
          code == 0 and "seat.activity_changed:500" in keys5 and len(keys5) == len(set(keys5)), (code, err[-200:]))


def w3_concurrent(binary, base, Source):
    case = base / "concurrent"
    case.mkdir()
    src = Source(case / "source.sqlite", 200)
    stop = threading.Event()
    committed = []

    def producer():
        c = sqlite3.connect(src.path)
        c.execute("PRAGMA wal_autocheckpoint=0")
        i = 1000
        while not stop.wait(0.005):
            c.execute("INSERT INTO events VALUES (?,?,?,?)", (i, "seat.activity_changed",
                      json.dumps({"sessionName": "producer", "state": "idle", "at": "2026-10-09T00:00:01Z"}),
                      "2026-10-09 00:00:01"))
            c.commit()
            committed.append(i)
            if i % 20 == 0:
                c.execute("PRAGMA wal_checkpoint(PASSIVE)").fetchall()
            if i % 50 == 0:
                c.execute("PRAGMA wal_checkpoint(TRUNCATE)").fetchall()
            i += 1
        c.close()

    thread = threading.Thread(target=producer)
    thread.start()
    time.sleep(0.2)
    first = committed[-1] if committed else 999
    code, out, err = cli(binary, ["run", "--db", str(case / "out" / "a.duckdb"), "--root", f"openrig={src.path}"])
    last = committed[-1] if committed else 999
    stop.set()
    thread.join()
    stages = list((case / "out").glob(".dendro-stage-*"))
    if code == 0:
        keys, _, _ = trace_activity_keys(binary, case / "out" / "a.duckdb")
        s = seqs(keys)
        produced = [x for x in s if x >= 1000]
        coherent = (set(range(0, 201)) <= set(s) and produced == list(range(1000, 1000 + len(produced)))
                    and len(s) == len(set(s)) and first <= (produced[-1] if produced else 999) <= last)
        check("W3", "concurrent producer: coherent complete cutoff (mandatory rows, contiguous producer prefix, no duplicates)",
              coherent, (len(s), produced[-1] if produced else None, first, last, out[-200:]))
        attempts = int(out.split("attempts=")[1].split()[0]) if "attempts=" in out else None
        check("W3", f"concurrent producer: accepted after {attempts} attempt(s), bounded", attempts and attempts <= 5)
    else:
        check("W3", "concurrent producer: clean bounded-retry failure, never partial",
              "changed during each of 5 snapshot attempts" in err and '"status":"ready"' not in out, err[-300:])
    check("W3", "concurrent producer: no stage left", stages == [], stages)
    src.conn.close()
    return code


def w4_unit():
    p = subprocess.run(["cargo", "test", "-q", "-p", "dendro-connectors", "--lib", "snapshot::"], cwd=REPO,
                       capture_output=True, text=True, timeout=1200)
    line = [l for l in p.stdout.splitlines() if l.startswith("test result")]
    passed = int(line[-1].split(" passed")[0].split()[-1]) if line else 0
    check("W4", f"stability helper unit tests pass ({passed})", p.returncode == 0 and passed >= 6, p.stdout[-300:])


def w6(binary, base, Source):
    case = base / "nowal"
    case.mkdir()
    path = case / "source.sqlite"
    c = sqlite3.connect(path)
    c.execute("CREATE TABLE events (seq INTEGER, type TEXT, payload TEXT, created_at TEXT)")
    c.execute("INSERT INTO events VALUES (1,'seat.activity_changed',?,'2026-10-09 00:00:00')",
              (json.dumps({"sessionName": "s", "state": "running", "at": "2026-10-09T00:00:00Z"}),))
    c.commit()
    c.close()
    before = manifest([path])
    code, out, err = cli(binary, ["run", "--db", str(case / "out" / "a.duckdb"), "--root", f"openrig={path}"])
    check("W6", "WAL-absent source: run exit 0", code == 0, err[-200:])
    check("W6", "WAL-absent source: no -wal or -shm created next to the source",
          not Path(f"{path}-wal").exists() and not Path(f"{path}-shm").exists())
    check("W6", "WAL-absent source unchanged", manifest([path]) == before)
    ro = base / "readonly"
    ro.mkdir()
    src = Source(ro / "source.sqlite", 50)
    src.conn.close()
    for f in src.files:
        if f.exists():
            f.chmod(0o444)
    ro.chmod(0o555)
    before = manifest(src.files)
    out_dir = base / "readonly-out"
    code, out, err = cli(binary, ["run", "--db", str(out_dir / "a.duckdb"), "--root", f"openrig={src.path}"])
    check("W6", "read-only source files and directory: run exit 0", code == 0, err[-200:])
    check("W6", "read-only source unchanged", manifest(src.files) == before)
    ro.chmod(0o755)
    alias = base / "alias"
    alias.mkdir()
    asrc = Source(alias / "source.sqlite", 20)
    os.link(asrc.path, alias / "hardlink.duckdb")
    before = manifest(asrc.files)
    code, out, err = cli(binary, ["run", "--db", str(alias / "hardlink.duckdb"), "--root", f"openrig={asrc.path}"])
    # The refusal reason is recorded, not asserted: the existing alias check
    # covers files under a root directory; for a file root the key or the
    # database preflight refuses first. AC12 requires refusal and no write.
    check("W6", "output hard-linked to the source is refused", code != 0, f"refusal: {err.strip()[-160:]}")
    keyed = base / "keyed"
    code_k, out_k, err_k = cli(binary, ["run", "--db", str(keyed / "a.duckdb"), "--root", f"openrig={asrc.path}"])
    shutil.copyfile(keyed / "a.duckdb.key", alias / "hardlink.duckdb.key")
    code, out, err = cli(binary, ["run", "--db", str(alias / "hardlink.duckdb"), "--root", f"openrig={asrc.path}"])
    check("W6", "hard-linked output with a key sidecar present is still refused",
          code_k == 0 and code != 0, f"refusal: {err.strip()[-160:]}")
    code, out, err = cli(binary, ["run", "--db", str(asrc.path), "--root", f"openrig={asrc.path}"])
    check("W6", "output equal to the source path is refused", code != 0, err[-200:])
    check("W6", "aliased source unchanged", manifest(asrc.files) == before)
    asrc.conn.close()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--binary", required=True)
    ap.add_argument("--scratch", required=True)
    ap.add_argument("--negative-binary")
    ap.add_argument("--observer-dir", default=str(REPO.parent.parent.parent / ".agent-runs/qa/06cd1a7"))
    args = ap.parse_args()
    binary = str(Path(args.binary).resolve())
    base = Path(args.scratch).resolve()
    if base.exists():
        sys.exit("refusing to overwrite existing evidence scratch")
    base.mkdir(parents=True, mode=0o700)
    sys.path.insert(0, args.observer_dir)
    observer = load(Path(args.observer_dir) / "observer.py", "observer")
    Source = load(REPO / "scripts/verify_snapshot_lifecycle.py", "lifecycle").Source
    hashes = {name: hashlib.sha256(Path(p).read_bytes()).hexdigest() for name, p in
              (("binary", binary), ("observer.py", Path(args.observer_dir) / "observer.py"), ("script", __file__))}
    w1_observe(observer, binary, base / "w1", Source, False, True, "default-live-producer")
    w1_observe(observer, binary, base / "w1", Source, False, False, "default-static")
    w1_observe(observer, binary, base / "w1", Source, True, False, "frozen-static")
    if args.negative_binary:
        r, writes = w1_observe(observer, str(Path(args.negative_binary).resolve()), base / "w1", Source, False, False,
                               "negative-06cd1a7", expect_clean=False)
        check("W1", "negative control: 06cd1a7 binary still opens the source -wal write-capably",
              r["source_violations"] > 0 and any(str(p).endswith("-wal") for e in writes for p in e["paths"]),
              r["source_violations"])
    else:
        check("W1", "negative control binary supplied", False, "--negative-binary missing")
    w2_w3_w5(binary, base, Source)
    w3_concurrent(binary, base, Source)
    w4_unit()
    w6(binary, base, Source)
    failed = [r for r in results if not r["ok"]]
    (base / "results.json").write_text(json.dumps({"hashes": hashes, "checks": len(results), "failed": len(failed),
                                                   "results": results}, indent=1))
    print(f"RESULT {len(results) - len(failed)}/{len(results)} checks passed")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
