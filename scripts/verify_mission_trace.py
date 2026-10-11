#!/usr/bin/env python3
"""Mission attribution proof (S4) from the trace alone.

Builds synthetic sources where each ambiguity reason occurs alone and all three
occur together, plus a clean mission seat and a no-mission seat, ingests them
through the CLI and then, using ONLY the trace (and the report for equality):
- the printed definition is identical in report and trace and states the order;
- each record's reasons are re-derived from its retained item evidence, and the
  primary is the first in the fixed order;
- bands per seat, primary shares and token categories recompute exactly;
- reordering the source rows does not change any record's attribution.

Usage:
  python3 scripts/verify_mission_trace.py --binary target/debug/dendro \\
      --scratch .agent-runs/proof/mission-final --trace-only
"""
import argparse
import json
import shutil
import sqlite3
import subprocess
import sys
from pathlib import Path

ORDER = ["insufficient history", "detectable tag change", "conflicting missions"]
FROM, TO = "2026-10-09T00:00:00Z", "2026-10-09T01:00:00Z"
results = []

# seat -> (items: [(qitem, created?, tags first ingest, tags second ingest)], usage tokens)
SEATS = {
    "s-ins": ([("old", False, ["m1"], ["m1"])], 7),
    "s-tag": ([("drift", True, ["a"], ["b"])], 11),
    "s-con": ([("two", True, ["a", "c"], ["a", "c"])], 13),
    "s-all": ([("all-old", False, ["m1"], ["m1"]), ("all-drift", True, ["a"], ["b"]),
               ("all-two", True, ["a", "c"], ["a", "c"])], 17),
    "s-ok": ([("fine", True, ["m-ok"], ["m-ok"])], 19),
    "s-none": ([], 23),
}


def check(name, ok, detail=""):
    results.append({"check": name, "ok": bool(ok), "detail": str(detail)[:300]})
    print(("PASS" if ok else "FAIL"), name, "" if ok else detail)


def build(root, second, reverse):
    claude = root / "claude"
    claude.mkdir(parents=True, exist_ok=True)
    lines = []
    for seat, (_, tokens) in SEATS.items():
        lines.append(json.dumps({"type": "assistant", "sessionId": f"sess-{seat}", "timestamp": "2026-10-09T00:05:00Z",
                                 "message": {"id": f"m-{seat}", "usage": {"input_tokens": tokens, "output_tokens": 2,
                                             "cache_read_input_tokens": 3, "cache_creation_input_tokens": 5}}}))
    if reverse:
        lines.reverse()
    (claude / "s.jsonl").write_text("\n".join(lines) + "\n")
    db = root / "openrig.sqlite"
    if db.exists():
        db.unlink()
    con = sqlite3.connect(db)
    con.executescript("""CREATE TABLE events (seq INTEGER, type TEXT, payload TEXT, created_at TEXT);
        CREATE TABLE queue_items (qitem_id TEXT, destination_session TEXT, ts_updated TEXT, tags TEXT);""")
    events, items = [], []
    for seat, (owned, _) in SEATS.items():
        events.append(("agent.session_identity", {"sessionId": f"sess-{seat}", "runtime": "claude-code", "sessionName": seat, "rigId": "r"}))
        for qitem, created, first, later in owned:
            if created:
                events.append(("queue.created", {"qitemId": qitem, "sourceSession": "orch", "destinationSession": seat}))
            events.append(("queue.claimed", {"qitemId": qitem, "destinationSession": seat, "claimedAt": "2026-10-09T00:01:00Z"}))
            tags = later if second else first
            items.append((qitem, seat, "2026-10-09T00:02:00Z" if second else "2026-10-09T00:01:30Z",
                          json.dumps([f"mission:{t}" for t in tags])))
    if reverse:
        events.reverse()
        items.reverse()
    con.executemany("INSERT INTO events VALUES (?,?,?,'2026-10-09 00:00:00')",
                    [(i + 1, k, json.dumps(p)) for i, (k, p) in enumerate(events)])
    con.executemany("INSERT INTO queue_items VALUES (?,?,?,?)", items)
    con.commit()
    con.close()
    return [f"claude={claude}", f"openrig={db}"]


def cli(binary, args):
    p = subprocess.run([binary, *args], capture_output=True, text=True, timeout=300)
    if p.returncode != 0:
        sys.exit(f"command failed: {args[0]} {p.stderr[-300:]}")
    return json.loads(p.stdout)


def ingest(binary, root, reverse):
    out = root / "out" / "a.duckdb"
    for second in (False, True):
        roots = build(root / "src", second, reverse)
        args = ["run", "--db", str(out)]
        for r in roots:
            args += ["--root", r]
        cli(binary, args)
    return out


def derive(items):
    """Reasons from a record's retained evidence only."""
    owned = [i for i in items if i["relation"] == "owned"]
    insufficient = any(i["relation"] != "owned" for i in items) or any(not i["tag_readings"] for i in owned)
    tag_change = any(any(a != b for a, b in zip(i["tag_readings"], i["tag_readings"][1:])) for i in owned)
    latest = [i["tag_readings"][-1] for i in owned if i["tag_readings"]]
    conflicting = any(len(t) > 1 for t in latest) or len({m for t in latest for m in t}) > 1
    reasons = [r for r, applies in zip(ORDER, (insufficient, tag_change, conflicting)) if applies]
    missions = sorted({m for t in latest for m in t})
    bucket = "ambiguous" if reasons else (f"mission:{missions[0]}" if len(missions) == 1 else "no_mission")
    return reasons, bucket


def attribution(records):
    """Order-independent attribution. Evidence source keys carry the source
    sequence number, which legitimately follows row order, so only the
    evidence kind (e.g. events:queue.created) is compared."""
    def kind(evidence):
        head, _, tail = evidence.rpartition(":")
        return head if tail.isdigit() else evidence
    def items(record):
        return sorted((i["qitem"], i["relation"], kind(i["destination_evidence"]),
                       json.dumps(i["tag_readings"])) for i in record["items"])
    return sorted((r["seat"], r["ts"], r["bucket"], r["primary_reason"], tuple(r["secondary_reasons"]),
                   tuple(items(r))) for r in records)


def verify(binary, root, label, reverse):
    db = ingest(binary, root / label, reverse)
    trace = cli(binary, ["trace", "--metric-id", "tokens", "--db", str(db), "--from", FROM, "--to", TO])
    report = cli(binary, ["report", "tokens", "--db", str(db), "--from", FROM, "--to", TO])["payload"]
    inputs = trace["inputs"]
    definition = inputs["mission_definition"]
    check(f"{label}: definition identical in report and trace", definition == report["mission_definition"])
    check(f"{label}: definition states the fixed order", " > ".join(ORDER) in definition)
    records = inputs["mission_records"]
    check(f"{label}: one record per linked usage record", len(records) == len(SEATS), len(records))
    bands, shares = {}, {}
    for r in records:
        reasons, bucket = derive(r["items"])
        recorded = ([r["primary_reason"]] if r["primary_reason"] else []) + r["secondary_reasons"]
        check(f"{label}: {r['seat']} reasons re-derived from evidence", reasons == recorded, f"{reasons} vs {recorded}")
        check(f"{label}: {r['seat']} bucket re-derived", bucket == r["bucket"], f"{bucket} vs {r['bucket']}")
        parts = r["input_uncached"] + r["output"] + r["cache_read"] + r["cache_write"]
        check(f"{label}: {r['seat']} categories sum to the record", parts == r["tokens"])
        bands.setdefault(r["seat"], {})
        bands[r["seat"]][r["bucket"]] = bands[r["seat"]].get(r["bucket"], 0) + r["tokens"]
        if r["primary_reason"]:
            shares[r["primary_reason"]] = shares.get(r["primary_reason"], 0) + r["tokens"]
    expected = {row["seat"]: row["bands"] for row in inputs["mission_by_seat"]}
    check(f"{label}: per-seat bands recompute exactly", bands == expected, f"{bands} vs {expected}")
    check(f"{label}: per-seat conservation flag", all(row["conserved"] for row in inputs["mission_by_seat"]))
    check(f"{label}: primary shares recompute exactly", shares == inputs["mission_ambiguous_by_reason"])
    check(f"{label}: primary shares sum to the ambiguous band",
          sum(shares.values()) == inputs["mission_tokens"].get("ambiguous", 0))
    by_seat = {r["seat"]: r for r in records}
    expect = {"s-ins": ORDER[:1], "s-tag": ORDER[1:2], "s-con": ORDER[2:], "s-all": ORDER, "s-ok": [], "s-none": []}
    for seat, reasons in expect.items():
        r = by_seat.get(seat, {})
        got = ([r.get("primary_reason")] if r.get("primary_reason") else []) + r.get("secondary_reasons", [])
        check(f"{label}: {seat} has exactly {reasons or 'no reasons'}", got == reasons, got)
    check(f"{label}: clean seat gets its mission", by_seat.get("s-ok", {}).get("bucket") == "mission:m-ok")
    check(f"{label}: no-item seat is no mission", by_seat.get("s-none", {}).get("bucket") == "no_mission")
    return attribution(records)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", required=True)
    parser.add_argument("--scratch", required=True)
    parser.add_argument("--trace-only", action="store_true", required=True)
    args = parser.parse_args()
    binary = str(Path(args.binary).resolve())
    root = Path(args.scratch).resolve()
    shutil.rmtree(root, ignore_errors=True)
    root.mkdir(parents=True)
    forward = verify(binary, root, "forward", False)
    reverse = verify(binary, root, "reversed", True)
    check("reordered source rows give identical attribution", forward == reverse)
    failed = [r for r in results if not r["ok"]]
    (root / "results.json").write_text(json.dumps({"checks": len(results), "failed": len(failed), "results": results}, indent=1))
    print(f"RESULT {len(results) - len(failed)}/{len(results)} checks passed")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
