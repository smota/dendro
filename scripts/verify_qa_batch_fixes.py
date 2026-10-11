#!/usr/bin/env python3
"""Focused proof for the QA batch fixes F1, F2, F6 and F4 (brief 6c891eb3,
addendum df29666a, F2 table 4acf5390, K11, F4 disposition 3b8d1551).

Every expected value is a fixture constant derived by hand from the contract
rule, never from Dendro output. Synthetic sources are written into a NEW
private scratch directory; the sealed part reads only a private copy of the
verified sealed input. Exit 0 only if every assertion passes.

  python3 scripts/verify_qa_batch_fixes.py --binary /abs/dendro --scratch /abs/new/dir \\
      [--old-binary /abs/old/dendro] [--skip-sealed]

With --old-binary the same synthetic cases also run against the old candidate
and their results are reported as negative-control evidence (not gated). The
sealed part then reports old and new values side by side (deltas).
"""
import argparse
import collections
import datetime
import hashlib
import json
import os
import shutil
import sqlite3
import subprocess
import sys
from pathlib import Path

sys.dont_write_bytecode = True
REPO = Path(__file__).resolve().parent.parent
SEALED = REPO / ".agent-runs/proof/sealed-input-40c65db7"
T0 = datetime.datetime(2026, 10, 9, 0, 0, 0, tzinfo=datetime.timezone.utc)
FROM, TO = "2026-10-09T00:00:00Z", "2026-10-09T01:00:00Z"

results = collections.OrderedDict()


def check(case, name, ok, detail=None):
    results.setdefault(case, []).append({"check": name, "ok": bool(ok), "detail": detail})
    print(("PASS " if ok else "FAIL ") + f"{case}: {name}" + ("" if ok or detail is None else f" -- {detail}"))


def at(ms):
    """RFC 3339 with milliseconds, ms after T0."""
    t = T0 + datetime.timedelta(milliseconds=ms)
    return t.strftime("%Y-%m-%dT%H:%M:%S.") + f"{t.microsecond // 1000:03d}Z"


def parse(text):
    return datetime.datetime.fromisoformat(text.replace("Z", "+00:00"))


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def run(cmd):
    p = subprocess.run([str(c) for c in cmd], capture_output=True, text=True)
    return p.returncode, p.stdout, p.stderr


def openrig_db(path, events=(), transitions=(), rigs=(("rig-id", "qa-rig"),)):
    path.parent.mkdir(parents=True, exist_ok=True)
    conn = sqlite3.connect(path)
    conn.executescript(
        "CREATE TABLE rigs (id TEXT, name TEXT);"
        "CREATE TABLE events (seq INTEGER, type TEXT, payload TEXT, created_at TEXT);"
        "CREATE TABLE queue_transitions (transition_id INTEGER, qitem_id TEXT, ts TEXT, state TEXT,"
        " actor_session TEXT, closure_reason TEXT, closure_target TEXT, transition_note TEXT);"
        "CREATE TABLE queue_items (qitem_id TEXT, destination_session TEXT, ts_updated TEXT,"
        " chain_of_record TEXT, handed_off_from TEXT);"
    )
    conn.executemany("INSERT INTO rigs VALUES (?,?)", rigs)
    for seq, (kind, payload, created) in enumerate(events, 1):
        conn.execute("INSERT INTO events VALUES (?,?,?,?)", (seq, kind, json.dumps(payload), created))
    conn.executemany("INSERT INTO queue_transitions VALUES (?,?,?,?,?,?,?,?)", transitions)
    conn.execute("INSERT INTO queue_items VALUES ('root','build-seat',?, '[]', NULL)", (at(0),))
    conn.execute("INSERT INTO queue_items VALUES ('next','qa-seat',?, '[\"root\"]', 'root')", (at(0),))
    conn.commit()
    conn.close()


def write_jsonl(path, lines):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(b"".join((line if isinstance(line, bytes) else json.dumps(line).encode()) + b"\n" for line in lines))


def ingest(binary, base, roots):
    db = base / "out" / "a.duckdb"
    cmd = [binary, "run", "--db", db]
    for name, path in roots:
        cmd += ["--root", f"{name}={path}"]
    code, out, err = run(cmd)
    return db, code, out, err


def report(binary, db, rig=None):
    cmd = [binary, "report", "decisions", "--db", db, "--from", FROM, "--to", TO]
    if rig:
        cmd += ["--rig", rig]
    code, out, _ = run(cmd)
    return json.loads(out)["payload"] if code == 0 else None


def trace(binary, db, metric, frm=FROM, to=TO, rig=None):
    cmd = [binary, "trace", "--metric-id", metric, "--db", db, "--from", frm, "--to", to]
    if rig:
        cmd += ["--rig", rig]
    code, out, _ = run(cmd)
    return json.loads(out) if code == 0 else None


def decoded(db, needles):
    """Rows per needle in warnings.reason, read by the dev-only decoded_scan reader."""
    code, out, err = run(["cargo", "run", "-q", "-p", "dendro-connectors", "--example", "decoded_scan", "--", db, *needles])
    if code != 0:
        raise SystemExit(f"decoded_scan failed: {err[-300:]}")
    for column in json.loads(out)["columns"]:
        if column["column"] == "warnings.reason":
            return column["hits"], column["values"]
    return {n: 0 for n in needles}, 0


def bands_of(row):
    return {k: row[k] for k in ("active_secs", "idle_with_work_secs", "idle_without_work_secs", "waiting_human_secs", "unknown_secs")}


def replay_activity(tr):
    """Bands recomputed from trace inputs alone: state at each boundary, demand
    changes, whole seconds counted from each observed span's start (F1 rule)."""
    i = tr["inputs"]
    ev = i.get("activity_events") or []
    dem = sorted(i.get("demand_events") or [], key=lambda d: (parse(d["ts"]), d["source_key"]))
    out = {}
    for seat in tr["value"]:
        result = collections.Counter()
        mine = sorted([e for e in ev if e["seat"] == seat["seat"]], key=lambda e: (parse(e["ts"]), e["source_key"]))
        spans = seat["observed_spans"]
        if not any(parse(s["from"]) <= parse(e["ts"]) < parse(s["to"]) for s in spans for e in mine):
            result["unknown_secs"] = seat["observed_secs"]
        for span in spans if result["unknown_secs"] == 0 else []:
            start, end = parse(span["from"]), parse(span["to"])
            launches = [parse(l["launched_at"]) for l in i["seat_lives"] if l["seat"] == seat["seat"] and l["launched_at"] and parse(l["launched_at"]) <= start]
            launch = max(launches) if launches else None
            life = [e for e in mine if launch is None or parse(e["ts"]) >= launch]
            marks = sorted({start, end} | {parse(e["ts"]) for e in life if start < parse(e["ts"]) < end}
                           | {parse(d["ts"]) for d in dem if d.get("dest") == seat["seat"] and start < parse(d["ts"]) < end})
            for lo, hi in zip(marks, marks[1:]):
                latest = [e for e in life if parse(e["ts"]) <= lo]
                state = latest[-1]["state"] if latest else "unknown"
                items = {}
                for d in dem:
                    if parse(d["ts"]) <= lo:
                        items[d["qitem_id"]] = d
                demand = any(d.get("dest") == seat["seat"] and d["state"] in ("pending", "in-progress") for d in items.values())
                label = {"running": "active_secs", "needs_input": "waiting_human_secs",
                         "idle": "idle_with_work_secs" if demand else "idle_without_work_secs"}.get(state, "unknown_secs")
                result[label] += int((hi - start).total_seconds() // 1) - int((lo - start).total_seconds() // 1)
        out[seat["seat"]] = {k: result.get(k, 0) for k in ("active_secs", "idle_with_work_secs", "idle_without_work_secs", "waiting_human_secs", "unknown_secs")}
    return out


# ---------------------------------------------------------------- F1 activity
def case_f1(binary, base, tag):
    case = f"F1 activity conservation [{tag}]"
    events = [
        ("node.launched", {"rigId": "rig-id", "nodeId": "n1", "sessionName": "qa-seat"}, "2026-10-09 00:00:00"),
        ("seat.activity_changed", {"sessionName": "qa-seat", "state": "running", "at": at(700)}, "2026-10-09 00:00:00"),
        ("seat.activity_changed", {"sessionName": "qa-seat", "state": "idle", "at": at(5_600)}, "2026-10-09 00:00:05"),
        ("seat.activity_changed", {"sessionName": "qa-seat", "state": "needs_input", "at": at(12_300)}, "2026-10-09 00:00:12"),
        ("seat.activity_changed", {"sessionName": "qa-seat", "state": "idle", "at": at(40_900)}, "2026-10-09 00:00:40"),
        ("session.stopped", {"rigId": "rig-id", "nodeId": "n1", "sessionName": "qa-seat"}, "2026-10-09 00:01:00"),
    ]
    openrig_db(base / "openrig.sqlite", events)
    db, code, _, err = ingest(binary, base, [("openrig", base / "openrig.sqlite")])
    check(case, "ingest exit 0", code == 0, err[-200:])
    rows = (report(binary, db) or {}).get("activity", [])
    seat = next((r for r in rows if r["observed_secs"] == 60), None)
    check(case, "one seat observed for exactly 60 s", seat is not None, rows)
    if seat is None:
        return
    # Offsets from the span start 0: marks 0, 0.7, 5.6, 12.3, 40.9, 60 have
    # whole-second offsets 0, 0, 5, 12, 40, 60. Per-segment truncation gives
    # 4 + 6 + 28 + 19 = 57 instead.
    expected = {"active_secs": 5, "idle_with_work_secs": 0, "idle_without_work_secs": 7 + 20,
                "waiting_human_secs": 28, "unknown_secs": 0}
    check(case, "bands sum to observed seconds", seat["sum"] == seat["observed_secs"] == sum(bands_of(seat).values()), bands_of(seat))
    check(case, "exact fractional band values", bands_of(seat) == expected, {"got": bands_of(seat), "want": expected})
    tr = trace(binary, db, "activity")
    replay = replay_activity(tr) if tr else {}
    check(case, "trace-only replay equals report", replay.get(seat["seat"]) == bands_of(seat), {"replay": replay.get(seat["seat"])})


# ---------------------------------------------------------------- F6 loops
def tool_line(ms, failed=True, name="Read", inp="same input", session="loop-session"):
    line = {"type": "assistant", "sessionId": session,
            "message": {"id": f"m{ms}-{name}", "content": [{"type": "tool_use", "name": name, "input": inp, "is_error": failed}],
                        "usage": {"input_tokens": 1, "output_tokens": 0, "cache_read_input_tokens": 0, "cache_creation_input_tokens": 0}}}
    if ms is not None:
        line["timestamp"] = at(ms)
    return line


def case_f6(binary, base, tag):
    case = f"F6 tool loops K11 [{tag}]"
    s = lambda secs: secs * 1000
    runs = {
        # name: (failure offsets in ms (None = untimestamped), expected loop counts)
        "fractional-excess": ([s(0), s(300), 600_001], []),
        "exact-600": ([s(0), s(300), s(600)], [3]),
        "first-three-of-four": ([s(0), s(300), s(600), s(1200)], [4]),
        "sliding-0-400-700-800": ([s(0), s(400), s(700), s(800)], [4]),
        "sliding-0-400-700-1101": ([s(0), s(400), s(700), s(1101)], []),
        "missing-first": ([None, s(1), s(2)], []),
        "missing-third": ([s(0), s(1), None], []),
        "missing-middle-counts": ([s(0), None, s(2)], [3]),
        "two-failures": ([s(0), s(1)], []),
    }
    for name, (offsets, want) in runs.items():
        b = base / name
        lines = [tool_line(ms + 1_000 if ms is not None else None, session=f"loop-{name}") for ms in offsets]
        write_jsonl(b / "claude" / "s.jsonl", lines)
        db, code, _, err = ingest(binary, b, [("claude", b / "claude")])
        got = sorted(l["count"] for l in (report(binary, db) or {}).get("tool_loops", [])) if code == 0 else None
        check(case, f"{name}: loop counts {want}", got == want, {"got": got, "exit": code, "err": err[-200:]})
    # A break (success, other tool) splits a run; duplicates of the same record count once.
    b = base / "breaks"
    lines = [tool_line(1_000), tool_line(2_000), tool_line(3_000, failed=False), tool_line(4_000), tool_line(5_000, name="Grep"), tool_line(6_000)]
    write_jsonl(b / "claude" / "s.jsonl", lines)
    db, code, _, _ = ingest(binary, b, [("claude", b / "claude")])
    got = (report(binary, db) or {}).get("tool_loops") if code == 0 else None
    check(case, "success and another tool break the run: no loop", got == [], got)


# ---------------------------------------------------------------- F2 intake
S_MS = 100_000  # "S" = T0 + 100 s


def f2_sources(b, claim_event_ms, claim_row, transitions, tools, usage, linked=True):
    events = []
    if linked:
        events.append(("agent.session_identity", {"sessionId": "qa-session", "runtime": "claude-code", "sessionName": "qa-seat", "rigId": "rig-id"}, "2026-10-09 00:00:00"))
    events += [
        ("queue.created", {"qitemId": "root", "sourceSession": "pm-seat", "destinationSession": "build-seat"}, "2026-10-09 00:00:00"),
        ("queue.created", {"qitemId": "next", "sourceSession": "build-seat", "destinationSession": "qa-seat"}, "2026-10-09 00:00:01"),
        ("queue.claimed", {"qitemId": "next", "destinationSession": "qa-seat", "claimedAt": at(S_MS + claim_event_ms)}, at(S_MS + claim_event_ms)[:19].replace("T", " ")),
    ]
    rows = [(1, "root", at(1_000), "done", "build-seat", "handed_off_to", "qa-seat", None)]
    if claim_row:
        ms, tid = claim_row
        rows.append((tid, "next", at(S_MS + ms), "in-progress", "qa-seat", None, None, None))
    for ms, tid, actor in transitions:
        rows.append((tid, "next", at(S_MS + ms), "blocked", actor, None, None, None))
    openrig_db(b / "openrig.sqlite", events, rows)
    lines = []
    for n, (ms, inp) in enumerate(usage):
        lines.append({"type": "assistant", "sessionId": "qa-session", "timestamp": at(S_MS + ms),
                      "message": {"id": f"u{n}", "usage": {"input_tokens": inp, "output_tokens": 0, "cache_read_input_tokens": 0, "cache_creation_input_tokens": 0}}})
    for n, (ms, name) in enumerate(tools):
        line = {"type": "assistant", "sessionId": "qa-session", "message": {"id": f"t{n}", "content": [{"type": "tool_use", "name": name, "input": "x", "is_error": False}],
                "usage": {"input_tokens": 0, "output_tokens": 0, "cache_read_input_tokens": 0, "cache_creation_input_tokens": 0}}}
        if ms is not None:
            line["timestamp"] = at(S_MS + ms)
        lines.append(line)
    write_jsonl(b / "claude" / "s.jsonl", lines)


def case_f2(binary, base, tag):
    case = f"F2 intake end point (table 4acf5390) [{tag}]"
    est = lambda n: ("estimate", None, n)
    nd = lambda reason: ("no_data", reason, None)
    cases = [
        # name, claim_event_ms, claim_row (ms, id), transitions [(ms, id, actor)], tools [(ms, name)], usage [(ms, tokens)], linked, want, want_claim_ms, want_end_ms
        ("A row1 usage in window", 200, (200, 2), [(500, 3, "qa-seat")], [], [(300, 7), (500, 100)], True, est(7), 200, 500),
        ("B row2 no usage, non-empty window", 200, (200, 2), [(500, 3, "qa-seat")], [], [], True, est(0), 200, 500),
        ("C row3 unlinked receiver", 200, (200, 2), [(500, 3, "qa-seat")], [], [(300, 7)], False, nd("no linked session"), 200, None),
        ("D row7 receiver row before claim row", 200, (200, 2), [(100, 3, "qa-seat")], [], [(300, 7)], True, nd("no classifiable end"), 200, None),
        ("E row4 sender update and bash only", 200, (200, 2), [(600, 3, "build-seat")], [(300, "Bash")], [(300, 7)], True, nd("no classifiable end"), 200, None),
        ("F row6 same ts later key", 200, (200, 2), [(200, 3, "qa-seat")], [], [(200, 7)], True, est(0), 200, 200),
        ("row7 same ts earlier key", 200, (200, 5), [(200, 4, "qa-seat")], [], [(300, 7)], True, nd("no classifiable end"), 200, None),
        ("G row8 Write at start", 200, (200, 2), [], [(200, "Write")], [(200, 7)], True, est(0), 200, 200),
        ("H row9 events-only S to S.500", 0, None, [(500, 3, "qa-seat")], [], [(300, 7)], True, est(7), 0, 500),
        ("H row9 events-only, no usage", 0, None, [(500, 3, "qa-seat")], [], [], True, est(0), 0, 500),
        ("I row10 events-only end at S", 0, None, [(0, 3, "qa-seat")], [], [(0, 7)], True, est(0), 0, 0),
        ("QA fixture: close at 10 s", -99_000, None, [(-90_000, 2, "qa-seat")], [], [(-98_000, 5), (-90_000, 9)], True, est(5), -99_000, -90_000),
        ("QA fixture + Write at 12 s", -99_000, None, [(-90_000, 2, "qa-seat")], [(-88_000, "Write")], [(-98_000, 5), (-90_000, 9)], True, est(5), -99_000, -90_000),
    ]
    for name, cms, crow, trans, tools, usage, linked, want, want_claim, want_end in cases:
        b = base / name.replace(" ", "-").replace(",", "")
        f2_sources(b, cms, crow, trans, tools, usage, linked)
        db, code, _, err = ingest(binary, b, [("claude", b / "claude"), ("openrig", b / "openrig.sqlite")])
        tr = trace(binary, db, "intake") if code == 0 else None
        rows = (tr or {}).get("value") or []
        row = rows[0] if len(rows) == 1 else None
        got = (row["status"], row["reason"], row["tokens"]) if row else None
        check(case, f"{name}: {want}", got == want, {"got": got, "rows": len(rows), "exit": code, "err": err[-200:]})
        if row and want[0] == "estimate":
            ok = (row["claim"] is not None and row["write"] is not None
                  and parse(row["claim"]) == parse(at(S_MS + want_claim)) and parse(row["write"]) == parse(at(S_MS + want_end)))
            check(case, f"{name}: window provenance [{want_claim}, {want_end}) ms after S", ok, {"claim": row["claim"], "write": row["write"]})


# ---------------------------------------------------------------- F4 unknown records
def case_f4(binary, base, tag):
    case = f"F4 unknown record types [{tag}]"
    claude = lambda msg, n, ts: {"type": "assistant", "sessionId": "s", **({"timestamp": ts} if ts else {}),
                                 "message": {"id": msg, "usage": {"input_tokens": n, "output_tokens": 0, "cache_read_input_tokens": 0, "cache_creation_input_tokens": 0}}}
    # QA six-line fixture.
    b = base / "qa-six"
    write_jsonl(b / "claude" / "c.jsonl", [claude("m1", 10, at(1_000)), b"{not json", bytes([0xFF, 0xFE, 0x7B, 0x7D]),
                                           {"type": "qa_unknown_record_version_999", "timestamp": at(2_000)},
                                           claude("m2", 20, at(3_000)), claude("m3", 5, None)])
    db, code, _, err = ingest(binary, b, [("claude", b / "claude")])
    hits, total = decoded(db, ["malformed-json", "invalid-utf8", "unknown-record-type: qa_unknown_record_version_999"])
    check(case, "QA six lines: exactly 3 warnings (malformed, UTF-8, unknown type)",
          total == 3 and all(v == 1 for v in hits.values()), {"hits": hits, "total": total, "exit": code})
    pay = report(binary, db) or {}
    check(case, "QA six lines: window 30 and untimestamped 5", pay.get("tokens") == 30 and pay.get("untimestamped_tokens_all_time") == 5,
          {"tokens": pay.get("tokens"), "untimestamped": pay.get("untimestamped_tokens_all_time")})
    # One synthetic unknown type per harness, repeated, beside ordinary records.
    b = base / "parity"
    unknown = {"type": "future_kind", "timestamp": at(2_000)}
    write_jsonl(b / "claude" / "c.jsonl", [claude("m1", 1, at(1_000)), unknown, unknown, {"timestamp": at(3_000)}])
    write_jsonl(b / "codex" / "x.jsonl", [{"type": "session_meta", "timestamp": at(0), "payload": {"id": "cx"}}, unknown, unknown,
                                          {"type": "event_msg", "timestamp": at(1_000), "payload": {"type": "token_count", "info": {"total_token_usage": {"input_tokens": 4, "cached_input_tokens": 0, "cache_write_input_tokens": 0, "output_tokens": 0, "reasoning_output_tokens": 0}}}}])
    write_jsonl(b / "pi" / "p.jsonl", [{"type": "session", "version": 3, "id": "pi", "timestamp": at(0)}, unknown, unknown,
                                       {"type": "message", "id": "a", "timestamp": at(1_000), "message": {"role": "assistant", "usage": {"input": 2, "output": 0, "cacheRead": 0, "cacheWrite": 0}}}])
    db, code, _, err = ingest(binary, b, [("claude", b / "claude"), ("codex", b / "codex"), ("pi", b / "pi")])
    hits, total = decoded(db, ["unknown-record-type: future_kind", "unknown-record-type"])
    check(case, "one warning per (file, type) per harness: 3 named + 1 missing-type", hits["unknown-record-type: future_kind"] == 3 and hits["unknown-record-type"] == 4,
          {"hits": hits, "exit": code, "err": err[-200:]})
    pay = report(binary, db) or {}
    check(case, "ordinary records still counted beside unknown ones (1 + 4 + 2)", pay.get("tokens") == 7, pay.get("tokens"))
    # Appending the same type adds nothing; a new type adds one.
    with open(b / "claude" / "c.jsonl", "ab") as f:
        f.write(json.dumps(unknown).encode() + b"\n" + json.dumps({"type": "other_kind"}).encode() + b"\n")
    db, code, _, _ = ingest(binary, b, [("claude", b / "claude"), ("codex", b / "codex"), ("pi", b / "pi")])
    hits, _ = decoded(db, ["unknown-record-type: future_kind", "unknown-record-type: other_kind"])
    check(case, "re-ingest after append: repeat adds 0, new type adds 1", hits == {"unknown-record-type: future_kind": 3, "unknown-record-type: other_kind": 1}, hits)
    # Secret-shaped and odd type names.
    b = base / "secret"
    write_jsonl(b / "claude" / "c.jsonl", [{"type": "sk-ant-api03-ZZTYPEsecretvalue"}, {"type": "has space"}, {"type": "a" * 65}, {"type": 7}])
    db, code, _, _ = ingest(binary, b, [("claude", b / "claude")])
    hits, total = decoded(db, ["ZZTYPEsecret", "unknown-record-type: [redacted]", "has space", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"])
    check(case, "secret-shaped name redacted, odd names generic, no text leaks", hits["ZZTYPEsecret"] == 0 and hits["unknown-record-type: [redacted]"] == 1
          and hits["has space"] == 0 and hits["a" * 65] == 0 and total == 2, {"hits": hits, "total": total})


# ---------------------------------------------------------------- sealed comparison
def verify_sealed_copy(root):
    items = json.loads((SEALED / "manifest.json").read_text())["items"]
    return [i["rel"] for i in items if sha(root / i["rel"]) != i["sha256"]], len(items)


def sealed_run(binary, base):
    src = base / "sources"
    shutil.copytree(SEALED / "sources", src)
    for p in [src, *src.rglob("*")]:
        os.chmod(p, 0o700 if p.is_dir() else 0o600)
    bad, total = verify_sealed_copy(src)
    db, code, out, err = ingest(binary, base, [("claude", src / "claude"), ("codex", src / "codex"), ("pi", src / "pi"), ("openrig", src / "openrig.sqlite")])
    frm, to = "2026-10-08T00:00:00Z", "2026-10-11T00:00:00Z"
    rig = "agentflow-dev"
    pay = json.loads(run([binary, "report", "decisions", "--db", db, "--rig", rig, "--from", frm, "--to", to])[1])["payload"]
    tr_act = trace(binary, db, "activity", frm, to, rig)
    tr_int = trace(binary, db, "intake", frm, to, rig)
    hits, _ = decoded(db, ["unknown-record-type"])
    return {"copy_bad": bad, "copy_total": total, "exit": code, "err": err[-300:], "payload": pay,
            "activity_trace": tr_act, "intake_trace": tr_int, "unknown_warnings": hits["unknown-record-type"]}


def intake_summary(pay):
    statuses = collections.Counter((r["status"], r["reason"]) for r in pay["intakes"])
    tokens = sum(r["tokens"] or 0 for r in pay["intakes"] if r["status"] == "estimate")
    medians = {f'{m["from"]}->{m["to"]}': m["intake_median"] for m in pay["pair_medians"]}
    return {"statuses": {f"{s}/{r}": n for (s, r), n in sorted(statuses.items(), key=str)}, "estimate_tokens": tokens, "intake_medians": medians}


def case_sealed(binary, old, base):
    case = "sealed agentflow-dev (private copy)"
    before_bad, total = verify_sealed_copy(SEALED / "sources")
    check(case, f"sealed input verified before ({total} files)", not before_bad, before_bad)
    new = sealed_run(binary, base / "new")
    check(case, "private copy matches manifest", not new["copy_bad"], new["copy_bad"])
    check(case, "ingest exit 0", new["exit"] == 0, new["err"])
    act = new["payload"]["activity"]
    gaps = {r["seat"]: r["observed_secs"] - r["sum"] for r in act}
    check(case, f"exact activity conservation for every seat ({len(act)} seats)", len(act) > 0 and all(g == 0 for g in gaps.values()), gaps)
    check(case, "zero unknown-type warnings (compatibility/regression guard only: allowlist is derived from this corpus)", new["unknown_warnings"] == 0, new["unknown_warnings"])
    rep = replay_activity(new["activity_trace"])
    mism = {r["seat"]: {"replay": rep.get(r["seat"]), "report": bands_of(r)} for r in act if rep.get(r["seat"]) != bands_of(r)}
    # Reported, never gated (addendum df29666a: residual trace completeness is a known limit).
    estimates = [r for r in new["intake_trace"]["value"] if r["status"] == "estimate"]
    prov_ok = all(parse(r["claim"]) <= parse(r["write"]) for r in estimates)
    check(case, f"intake provenance: every estimate has claim <= end ({len(estimates)} estimates)", prov_ok)
    deltas = {"new": {"activity": {r["seat"]: bands_of(r) | {"observed": r["observed_secs"]} for r in act},
                      "activity_replay_mismatched_seats": len(mism), "activity_replay_mismatches": mism,
                      "intake": intake_summary(new["payload"]), "tool_loops": new["payload"]["tool_loops"],
                      "unknown_warnings": new["unknown_warnings"]}}
    if old:
        o = sealed_run(old, base / "old")
        oact = o["payload"]["activity"]
        orep = replay_activity(o["activity_trace"])
        deltas["old"] = {"activity": {r["seat"]: bands_of(r) | {"observed": r["observed_secs"]} for r in oact},
                         "activity_gaps": {r["seat"]: r["observed_secs"] - r["sum"] for r in oact},
                         "activity_replay_mismatched_seats": sum(1 for r in oact if orep.get(r["seat"]) != bands_of(r)),
                         "intake": intake_summary(o["payload"]), "tool_loops": o["payload"]["tool_loops"],
                         "unknown_warnings": o["unknown_warnings"], "exit": o["exit"]}
    after_bad, _ = verify_sealed_copy(SEALED / "sources")
    check(case, "sealed input verified after", not after_bad, after_bad)
    (base / "sealed-deltas.json").write_text(json.dumps(deltas, indent=1, sort_keys=True))
    print("INFO sealed deltas written:", base / "sealed-deltas.json")
    print("INFO new activity replay mismatched seats (reported, not gated):", len(mism))
    if old:
        print("INFO old activity gaps per seat:", deltas["old"]["activity_gaps"])
        print("INFO intake old:", json.dumps(deltas["old"]["intake"]["statuses"]), "tokens", deltas["old"]["intake"]["estimate_tokens"])
        print("INFO tool loops old:", len(deltas["old"]["tool_loops"]), "unknown warnings old:", deltas["old"]["unknown_warnings"])
    print("INFO intake new:", json.dumps(deltas["new"]["intake"]["statuses"]), "tokens", deltas["new"]["intake"]["estimate_tokens"])
    print("INFO tool loops new:", len(deltas["new"]["tool_loops"]))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--binary", required=True)
    ap.add_argument("--scratch", required=True)
    ap.add_argument("--old-binary")
    ap.add_argument("--skip-sealed", action="store_true")
    args = ap.parse_args()
    binary = Path(args.binary).resolve()
    old = Path(args.old_binary).resolve() if args.old_binary else None
    base = Path(args.scratch).resolve()
    if base.exists():
        print("FAIL scratch exists; this proof needs a new empty directory:", base)
        sys.exit(1)
    base.mkdir(parents=True, mode=0o700)
    print("binary sha256", sha(binary), "| old binary sha256", sha(old) if old else None)
    for fn in (case_f1, case_f6, case_f2, case_f4):
        fn(binary, base / "new" / fn.__name__, "new")
    gated = {k: v for k, v in results.items()}
    negative = {}
    if old:
        start = len(results)
        for fn in (case_f1, case_f6, case_f2, case_f4):
            fn(old, base / "old" / fn.__name__, "old")
        negative = {k: v for k, v in list(results.items())[start:]}
        for k in negative:
            gated.pop(k, None)
    if not args.skip_sealed:
        case_sealed(binary, old, base / "sealed")
        gated["sealed agentflow-dev (private copy)"] = results["sealed agentflow-dev (private copy)"]
    print("== per-case assertion counts (gated)")
    failed = 0
    for case, rows in gated.items():
        bad = sum(not r["ok"] for r in rows)
        failed += bad
        print(f"{case}: {len(rows) - bad}/{len(rows)} passed")
    if negative:
        print("== old-candidate negative results (reported, not gated)")
        for case, rows in negative.items():
            print(f"{case}: {sum(r['ok'] for r in rows)}/{len(rows)} passed, {sum(not r['ok'] for r in rows)} failed")
    (base / "results.json").write_text(json.dumps(results, indent=1))
    total = sum(len(v) for v in gated.values())
    if failed:
        print(f"RESULT {total - failed}/{total} checks passed; FAILED")
        sys.exit(1)
    print(f"RESULT {total}/{total} checks passed")


if __name__ == "__main__":
    main()
