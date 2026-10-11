#!/usr/bin/env python3
"""AC1 timing: built binary, new analytics DB, ingest plus the first report.

Runs on a private copy of the verified sealed input in a NEW scratch directory
(refuses one that exists). Times `dendro run` and the first
`dendro report tokens --rig <rig>` with a monotonic clock. Fails, with a
nonzero exit, on any nonzero command exit, unreadable output, a sealed-input
mismatch, a changed binary, or a total above the budget. It never prints a
time as a pass unless both commands exited 0.

Usage:
  python3 scripts/measure_ac1.py --binary /abs/dendro --scratch /abs/new/dir [--rig agentflow-dev]
"""
import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

sys.dont_write_bytecode = True
REPO = Path(__file__).resolve().parent.parent
SEALED = REPO / ".agent-runs/proof/sealed-input-40c65db7"
BUDGET = 300.0


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def sealed_ok(root):
    items = json.loads((SEALED / "manifest.json").read_text())["items"]
    bad = [i["rel"] for i in items if sha(root / i["rel"]) != i["sha256"]]
    return len(items) - len(bad), len(items), bad


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--binary", required=True)
    ap.add_argument("--scratch", required=True)
    ap.add_argument("--rig", default="agentflow-dev")
    args = ap.parse_args()
    binary = Path(args.binary).resolve()
    base = Path(args.scratch).resolve()
    failures = []
    if base.exists():
        print("FAIL scratch exists; AC1 needs a new empty directory:", base)
        print("RESULT AC1 NOT EXECUTED")
        sys.exit(1)
    base.mkdir(parents=True, mode=0o700)
    b1 = sha(binary)
    print("binary sha256 before", b1)
    print("sealed manifest", sha(SEALED / "manifest.json"))
    ok, total, bad = sealed_ok(SEALED / "sources")
    print("sealed input before", ok, "of", total, "mismatches", bad)
    if bad:
        failures.append("sealed input mismatch before")
    src = base / "sources"
    shutil.copytree(SEALED / "sources", src)
    for p in [src, *src.rglob("*")]:
        os.chmod(p, 0o700 if p.is_dir() else 0o600)
    ok, total, bad = sealed_ok(src)
    print("private copy", ok, "of", total, "mismatches", bad)
    if bad:
        failures.append("private copy mismatch")
    db = base / "out" / "a.duckdb"
    print("analytics DB exists before run:", db.exists(), "| out dir exists:", db.parent.exists())
    if db.parent.exists():
        failures.append("output directory not new")
    run = [str(binary), "run", "--db", str(db), "--root", f"claude={src / 'claude'}", "--root", f"codex={src / 'codex'}",
           "--root", f"pi={src / 'pi'}", "--root", f"openrig={src / 'openrig.sqlite'}"]
    first = [str(binary), "report", "tokens", "--db", str(db), "--rig", args.rig]
    t0 = time.monotonic()
    r1 = subprocess.run(run, capture_output=True, text=True)
    t1 = time.monotonic()
    r2 = subprocess.run(first, capture_output=True, text=True) if r1.returncode == 0 else None
    t2 = time.monotonic()
    (base / "run.json").write_text(r1.stdout)
    (base / "run.err").write_text(r1.stderr)
    print("ingest exit", r1.returncode, "seconds", round(t1 - t0, 3))
    if r1.returncode != 0:
        failures.append("ingest exit nonzero")
        print("ingest stderr:", r1.stderr[-400:])
    if r2 is None:
        failures.append("first report not run")
        print("first report: NOT RUN (ingest failed)")
    else:
        (base / "report.json").write_text(r2.stdout)
        (base / "report.err").write_text(r2.stderr)
        print("first report (tokens --rig", args.rig + ") exit", r2.returncode, "seconds", round(t2 - t1, 3))
        if r2.returncode != 0:
            failures.append("first report exit nonzero")
            print("report stderr:", r2.stderr[-400:])
        else:
            try:
                rep = json.loads(r2.stdout)["payload"]
                print("first report tokens", rep["tokens"], "| sessions", len(rep["sessions"]))
            except (ValueError, KeyError) as error:
                failures.append(f"first report unreadable: {error!r}")
    try:
        sources = json.loads(r1.stdout)["sources"]
        for note in sources:
            print("source:", note)
        found = {note.split()[0] for note in sources}
        for name in ("claude", "codex", "pi", "openrig"):
            if name not in found:
                failures.append(f"source {name} not listed")
    except (ValueError, KeyError) as error:
        failures.append(f"run output unreadable: {error!r}")
    elapsed = t2 - t0
    ok, total, bad = sealed_ok(SEALED / "sources")
    print("sealed input after", ok, "of", total, "mismatches", bad)
    if bad:
        failures.append("sealed input mismatch after")
    b2 = sha(binary)
    print("binary sha256 after", b2)
    if b1 != b2:
        failures.append("binary changed")
    print("total seconds (ingest start to first report end, monotonic)", round(elapsed, 3), "budget", BUDGET)
    if not failures and elapsed > BUDGET:
        failures.append(f"over budget: {elapsed:.3f} s")
    (base / "ac1.json").write_text(json.dumps({"ingest_exit": r1.returncode, "report_exit": None if r2 is None else r2.returncode,
                                              "ingest_s": t1 - t0, "report_s": t2 - t1, "total_s": elapsed,
                                              "failures": failures}, indent=1))
    if failures:
        for f in failures:
            print("FAIL", f)
        print("RESULT AC1 FAILED")
        sys.exit(1)
    print(f"RESULT AC1 PASS {elapsed:.3f} s <= {BUDGET:.0f} s")


if __name__ == "__main__":
    main()
