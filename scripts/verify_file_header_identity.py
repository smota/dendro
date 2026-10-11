#!/usr/bin/env python3
"""File-header session identity proof (K10) for Claude, Pi and Codex.

Part A, synthetic fixtures through the real CLI. Expected values are fixed by
the fixture definitions below, never read from Dendro:
- two native sessions per harness with colliding session-local message ids
  (Claude 30, Pi 30, Codex 35 = 10, reset 5, + 20), unique ids, an in-session
  duplicate, a zero-usage header-only file, a missing header with a misleading
  file name and identity event, a malformed header, a missing first header
  followed by a valid later one, a conflicting later header, a conflicting
  Codex payload.session_id, a conflict appended after a linked ingest, an
  unchanged rerun and a replacement with a new header;
- links and link rates, per-seat and unlinked totals, M1 seat tokens, and
  warning counts from the decoded warnings table.

Part B, independent oracle over a private copy of the verified sealed input:
the Claude, Pi and Codex files are parsed here from the K10 and section 4.1
rules (Claude per-session message dedupe, Pi per-message totals, Codex
per-file cumulative counters with reset). Compared with Dendro per (path,
line) token row, per linked seat, unlinked, untimestamped and per-session
link status, using the sealed OpenRig identity events.

Usage:
  python3 scripts/verify_file_header_identity.py --binary /abs/dendro --scratch /abs/new/scratch
"""
import argparse
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
WIDE = ["--from", "1970-01-01T00:00:00Z", "--to", "2100-01-01T00:00:00Z"]
results = []


def check(row, name, ok, detail=""):
    results.append({"row": row, "check": name, "ok": bool(ok), "detail": str(detail)[:400]})
    print(("PASS" if ok else "FAIL"), f"{row}: {name}", "" if ok else detail, flush=True)


def cli(binary, args):
    p = subprocess.run([binary, *args], capture_output=True, text=True, timeout=1200)
    return p.returncode, p.stdout, p.stderr


def report(binary, db, kind):
    code, out, err = cli(binary, ["report", kind, "--db", str(db), *WIDE])
    if code != 0:
        raise SystemExit(f"report {kind} failed: {err[-300:]}")
    return json.loads(out)["payload"]


def warning_count(db, text):
    p = subprocess.run(["cargo", "run", "-q", "-p", "dendro-connectors", "--example", "decoded_scan", "--", str(db), text],
                       cwd=REPO, capture_output=True, text=True, timeout=1200)
    cols = json.loads(p.stdout)["columns"]
    return next(c["hits"][text] for c in cols if c["column"] == "warnings.reason")


# ---------------------------------------------------------------- Part A

def claude_line(session, msg, tokens):
    return {"type": "assistant", "sessionId": session, "timestamp": "2026-10-09T00:00:01Z",
            "message": {"id": msg, "usage": {"input_tokens": tokens, "output_tokens": 0,
                        "cache_read_input_tokens": 0, "cache_creation_input_tokens": 0}}}


def pi_header(sid):
    return {"type": "session", "version": 3, "id": sid, "timestamp": "2026-10-09T00:00:00Z"}


def pi_line(msg, tokens):
    return {"type": "message", "id": msg, "timestamp": "2026-10-09T00:00:01Z",
            "message": {"role": "assistant", "usage": {"input": tokens, "output": 0, "cacheRead": 0, "cacheWrite": 0}}}


def codex_header(sid):
    return {"type": "session_meta", "timestamp": "2026-10-09T00:00:00Z", "payload": {"id": sid}}


def codex_line(total, sid=None):
    payload = {"type": "token_count", "info": {"total_token_usage": {"input_tokens": total, "cached_input_tokens": 0,
               "cache_write_input_tokens": 0, "output_tokens": 0, "reasoning_output_tokens": 0}}}
    if sid:
        payload["session_id"] = sid
    return {"type": "event_msg", "timestamp": "2026-10-09T00:00:01Z", "payload": payload}


def write(path, records, raw_first=None):
    path.parent.mkdir(parents=True, exist_ok=True)
    lines = ([raw_first] if raw_first is not None else []) + [json.dumps(r) for r in records]
    path.write_text("".join(l + "\n" for l in lines))


# Identity events: harness runtime, native id, seat. Misleading ids name files
# whose headers are missing; they must never link.
IDENTITIES = [
    ("claude-code", "native-claude-1", "claude-seat-1"), ("claude-code", "native-claude-2", "claude-seat-2"),
    ("pi", "native-pi-1", "pi-seat-1"), ("pi", "native-pi-2", "pi-seat-2"), ("pi", "native-pi-u", "pi-seat-u"),
    ("pi", "native-pi-zero", "pi-seat-zero"), ("pi", "native-pi-c", "pi-seat-c"), ("pi", "native-pi-x", "pi-seat-x"),
    ("pi", "native-pi-miss", "pi-seat-miss"), ("pi", "native-pi-late", "pi-seat-late"),
    ("codex", "native-codex-1", "codex-seat-1"), ("codex", "native-codex-2", "codex-seat-2"),
    ("codex", "native-codex-c", "codex-seat-c"), ("codex", "native-codex-other", "codex-seat-other"),
    ("codex", "native-codex-miss", "codex-seat-miss"), ("codex", "native-codex-zero", "codex-seat-zero"),
    ("codex", "native-codex-3", "codex-seat-3"),
]


def part_a(binary, base):
    src = base / "fixture"
    claude, pi, codex = src / "claude", src / "pi", src / "codex"
    write(claude / "c1.jsonl", [claude_line("native-claude-1", "m1", 10)])
    write(claude / "c2.jsonl", [claude_line("native-claude-2", "m1", 20)])
    write(pi / "a.jsonl", [pi_header("native-pi-1"), pi_line("same", 10), pi_line("same", 10)])
    write(pi / "b.jsonl", [pi_header("native-pi-2"), pi_line("same", 20)])
    write(pi / "u.jsonl", [pi_header("native-pi-u"), pi_line("u1", 3), pi_line("u2", 4)])
    write(pi / "zero.jsonl", [pi_header("native-pi-zero")])
    write(pi / "native-pi-miss.jsonl", [pi_line("x", 5)])                       # misleading name, no header
    write(pi / "malformed.jsonl", [pi_line("y", 2)], raw_first="{not json")
    write(pi / "late.jsonl", [pi_line("l1", 6), pi_header("native-pi-late"), pi_line("l2", 7)])
    write(pi / "conflict.jsonl", [pi_header("native-pi-c"), pi_line("c1", 2), pi_header("native-pi-x"), pi_line("c2", 3)])
    write(codex / "a.jsonl", [codex_header("native-codex-1"), codex_line(10), codex_line(5)])
    write(codex / "b.jsonl", [codex_header("native-codex-2"), codex_line(20)])
    write(codex / "conflict.jsonl", [codex_header("native-codex-c"), codex_line(4, "native-codex-other"), codex_line(9, "native-codex-c")])
    write(codex / "native-codex-miss.jsonl", [codex_line(8, "native-codex-miss")])  # misleading name and line id
    write(codex / "zero.jsonl", [codex_header("native-codex-zero")])
    db_src = src / "openrig.sqlite"
    con = sqlite3.connect(db_src)
    con.execute("CREATE TABLE events (seq INTEGER, type TEXT, payload TEXT, created_at TEXT)")
    con.executemany("INSERT INTO events VALUES (?,?,?,'2026-10-09 00:00:00')", [
        (i + 1, "agent.session_identity", json.dumps({"sessionId": sid, "runtime": rt, "sessionName": seat, "rigId": "r"}))
        for i, (rt, sid, seat) in enumerate(IDENTITIES)])
    con.commit()
    con.close()
    db = base / "out" / "a.duckdb"
    roots = ["--root", f"claude={claude}", "--root", f"pi={pi}", "--root", f"codex={codex}", "--root", f"openrig={db_src}"]
    code, out, err = cli(binary, ["run", "--db", str(db), *roots])
    check("A", "first ingest exit 0", code == 0, err[-300:])

    links = {(r["harness"], r["native_session"]): r for r in report(binary, db, "links")["links"]}
    linked = lambda h, s, seat: links.get((h, s), {}).get("status") == "linked" and links[(h, s)].get("seat") == seat
    for h, s, seat in [("claude", "native-claude-1", "claude-seat-1"), ("claude", "native-claude-2", "claude-seat-2"),
                       ("pi", "native-pi-1", "pi-seat-1"), ("pi", "native-pi-2", "pi-seat-2"), ("pi", "native-pi-u", "pi-seat-u"),
                       ("pi", "native-pi-c", "pi-seat-c"), ("codex", "native-codex-1", "codex-seat-1"),
                       ("codex", "native-codex-2", "codex-seat-2"), ("codex", "native-codex-c", "codex-seat-c")]:
        check("A", f"{h} {s} links to {seat}", linked(h, s, seat), links.get((h, s)))
    for h, s, seat in [("pi", "native-pi-zero", "pi-seat-zero"), ("codex", "native-codex-zero", "codex-seat-zero")]:
        check("A", f"zero-usage {h} file discovered under its header id and linked", linked(h, s, seat), links.get((h, s)))
    fallbacks = {k: v for k, v in links.items() if k[1].startswith(("pi-file:", "codex-file:"))}
    check("A", "missing/malformed/late-header files use per-file fallbacks (4), all unlinked",
          len(fallbacks) == 4 and all(v["status"] == "unlinked" for v in fallbacks.values()), list(fallbacks))
    # Misleading ids appear in links only because identity events name them;
    # what matters is that no tokens are attributed to them.
    # Expected discovery from the fixture definition: usage sessions, identity
    # sessions and zero-usage header files; linked = exactly one binding.
    usage_sessions = {"pi": {"native-pi-1", "native-pi-2", "native-pi-u", "native-pi-c", "fb1", "fb2", "fb3"},
                      "codex": {"native-codex-1", "native-codex-2", "native-codex-c", "fb4"},
                      "claude": {"native-claude-1", "native-claude-2"}}
    file_only = {"pi": {"native-pi-zero"}, "codex": {"native-codex-zero"}, "claude": set()}
    bound = {}
    for rt, sid, seat in IDENTITIES:
        bound.setdefault({"claude-code": "claude"}.get(rt, rt), set()).add(sid)
    rates = {r["harness"]: r for r in report(binary, db, "links")["link_rates"]}
    for h in ("claude", "pi", "codex"):
        discovered = usage_sessions[h] | file_only[h] | bound[h]
        linked_n = len(discovered & bound[h])
        got = (rates.get(h, {}).get("linked"), rates.get(h, {}).get("discovered"))
        check("A", f"{h} link rate {linked_n}/{len(discovered)} from the fixture definition", got == (linked_n, len(discovered)), got)
    expected_seats = {"claude-seat-1": 10, "claude-seat-2": 20, "pi-seat-1": 10, "pi-seat-2": 20, "pi-seat-u": 7,
                      "pi-seat-c": 5, "codex-seat-1": 15, "codex-seat-2": 20, "codex-seat-c": 9}
    unlinked = 5 + 2 + 13 + 8  # pi missing, pi malformed, pi late-header file, codex missing
    p = report(binary, db, "decisions")
    seats = {r["seat"]: r["total"] for r in p["mission_by_seat"]}
    check("A", "per-seat totals equal the fixture definition (per-session attribution)", seats == expected_seats, seats)
    check("A", "unlinked total equals the fallback files' tokens", p["mission_tokens"].get("unlinked") == unlinked, p["mission_tokens"])
    check("A", "Claude 30, Pi 30 (+7 unique +5 conflict), Codex 35 (+9 conflict) per harness",
          sum(v for k, v in seats.items() if k.startswith("claude")) == 30
          and seats["pi-seat-1"] + seats["pi-seat-2"] == 30 and seats["codex-seat-1"] + seats["codex-seat-2"] == 35)
    check("A", "seat conservation flags all true", all(r["conserved"] for r in p["mission_by_seat"]))
    for seat in ("pi-seat-miss", "pi-seat-late", "pi-seat-x", "pi-seat-zero", "codex-seat-miss", "codex-seat-other",
                 "codex-seat-zero", "codex-seat-3"):
        check("A", f"{seat} receives no tokens (no rescue, no guess)", seat not in seats, seats.get(seat))
    m1 = {r["seat"]: r["tokens"] for r in p["m1_by_seat"]}
    check("A", "M1 seat tokens use the same attribution", all(m1.get(s) == t for s, t in expected_seats.items()), m1)
    check("A", "conflicting header warnings: pi 1, codex 1", warning_count(db, "conflicting session header") == 2)
    check("A", "missing header warnings: 2 pi + 1 pi late + 1 codex", warning_count(db, "missing session header") == 4)

    # Unchanged rerun.
    cli(binary, ["run", "--db", str(db), *roots])
    p2 = report(binary, db, "decisions")
    check("A", "unchanged rerun: no total moves", p2["mission_tokens"] == p["mission_tokens"])
    # Conflict appended after a linked ingest, plus replacement with a new header.
    with open(pi / "a.jsonl", "a") as f:
        f.write(json.dumps(pi_header("native-pi-other")) + "\n" + json.dumps(pi_line("new", 8)) + "\n")
    write(codex / "b.jsonl", [codex_header("native-codex-3"), codex_line(9)])
    code, out, err = cli(binary, ["run", "--db", str(db), *roots])
    p3 = report(binary, db, "decisions")
    seats3 = {r["seat"]: r["total"] for r in p3["mission_by_seat"]}
    check("A", "appended conflict keeps the first link and adds exactly 8", code == 0 and seats3.get("pi-seat-1") == 18, seats3)
    check("A", "replacement with a new header counts under the new session (codex-seat-3 9), old rows kept",
          seats3.get("codex-seat-3") == 9 and seats3.get("codex-seat-2") == 20, seats3)
    others = {k: v for k, v in seats3.items() if k not in ("pi-seat-1", "codex-seat-3")}
    check("A", "every other seat unchanged", all(others[k] == expected_seats.get(k) for k in others), others)
    links3 = {(r["harness"], r["native_session"]): r for r in report(binary, db, "links")["links"]}
    check("A", "the appended conflicting id receives no tokens and no link row", ("pi", "native-pi-other") not in links3
          and not any(k.startswith("pi-seat-other") for k in seats3))
    check("A", "conflicting header warnings now 3", warning_count(db, "conflicting session header") == 3)
    trace = json.loads(cli(binary, ["trace", "--metric-id", "tokens", "--db", str(db), *WIDE])[1])
    rows = [r for r in trace["inputs"]["rows"] if r["path"] and r["path"].endswith("codex/b.jsonl")]
    gens = sorted({r["generation"] for r in rows})
    check("A", "replacement is a new generation with a baseline first reading",
          len(gens) == 2 and any(r["generation"] == gens[1] and r["predecessor_reason"] in ("baseline", "reset") for r in rows),
          [(r["generation"], r["predecessor_reason"]) for r in rows])


# ---------------------------------------------------------------- Part B

def complete_lines(path):
    data = path.read_bytes()
    lines = data.split(b"\n")
    for no, raw in enumerate(lines[:-1], start=1):  # the last piece has no newline: incomplete
        yield no, raw


def parse_json(raw):
    try:
        return json.loads(raw.decode("utf-8"))
    except (UnicodeDecodeError, ValueError):
        return None


def ints(obj, keys):
    out = []
    for k in keys:
        v = obj.get(k) if isinstance(obj, dict) else None
        if not isinstance(v, int) or isinstance(v, bool):
            return None
        out.append(v)
    return out


def oracle_lines(root):
    """{(relpath, line): (harness, session, tokens, timestamped)} from the rules alone."""
    out = {}
    for path in sorted((root / "claude").rglob("*.jsonl")):
        seen = set()
        for no, raw in complete_lines(path):
            v = parse_json(raw)
            if not isinstance(v, dict) or v.get("type") != "assistant":
                continue
            usage = (v.get("message") or {}).get("usage") if isinstance(v.get("message"), dict) else None
            vals = ints(usage, ["input_tokens", "output_tokens", "cache_read_input_tokens", "cache_creation_input_tokens"])
            if vals is None:
                continue
            session = v.get("sessionId") if isinstance(v.get("sessionId"), str) else "claude-session"
            msg = v["message"].get("id")
            if isinstance(msg, str):
                if (session, msg) in seen:
                    continue
                seen.add((session, msg))
            ts = v.get("timestamp") or v["message"].get("timestamp")
            out[(str(path.relative_to(root)), no)] = ("claude", session, sum(vals), isinstance(ts, str))
    for harness in ("pi", "codex"):
        for path in sorted((root / harness).rglob("*.jsonl")):
            first = next(iter(complete_lines(path)), (None, b""))[1]
            header = parse_json(first)
            sid = None
            if isinstance(header, dict):
                if harness == "pi" and header.get("type") == "session":
                    sid = header.get("id")
                if harness == "codex" and header.get("type") == "session_meta":
                    sid = (header.get("payload") or {}).get("id") if isinstance(header.get("payload"), dict) else None
            session = sid if isinstance(sid, str) and sid.strip() else None  # None: file fallback
            seen, prev = set(), None
            for no, raw in complete_lines(path):
                v = parse_json(raw)
                if not isinstance(v, dict):
                    continue
                ts = isinstance(v.get("timestamp"), str) or isinstance((v.get("message") or {}).get("timestamp") if isinstance(v.get("message"), dict) else None, str)
                if harness == "pi":
                    m = v.get("message")
                    if v.get("type") != "message" or not isinstance(m, dict) or m.get("role") != "assistant":
                        continue
                    vals = ints(m.get("usage"), ["input", "output", "cacheRead", "cacheWrite"])
                    if vals is None:
                        continue
                    msg = v.get("id")
                    if isinstance(msg, str):
                        if msg in seen:
                            continue
                        seen.add(msg)
                    out[(str(path.relative_to(root)), no)] = ("pi", session, sum(vals), ts)
                else:
                    p = v.get("payload")
                    if v.get("type") != "event_msg" or not isinstance(p, dict) or p.get("type") != "token_count":
                        continue
                    total = (p.get("info") or {}).get("total_token_usage") if isinstance(p.get("info"), dict) else None
                    cur = ints(total, ["input_tokens", "cached_input_tokens", "cache_write_input_tokens",
                                       "output_tokens", "reasoning_output_tokens"])
                    if cur is None:
                        continue
                    reset = prev is not None and any(c < q for c, q in zip(cur, prev))
                    delta = cur if prev is None or reset else [c - q for c, q in zip(cur, prev)]
                    prev = cur
                    uncached = delta[0] - delta[1] - delta[2]
                    if uncached < 0:
                        continue
                    out[(str(path.relative_to(root)), no)] = ("codex", session, uncached + delta[3] + delta[1] + delta[2], ts)
    return out


def part_b(binary, base):
    if not SEALED.exists():
        check("B", "sealed input present", False, str(SEALED))
        return
    work = base / "sealed"
    shutil.copytree(SEALED / "sources", work / "sources")
    for p in [work, *work.rglob("*")]:
        os.chmod(p, 0o700 if p.is_dir() else 0o600)
    manifest = json.loads((SEALED / "manifest.json").read_text())
    bad = [i["rel"] for i in manifest["items"] if hashlib.sha256((work / "sources" / i["rel"]).read_bytes()).hexdigest() != i["sha256"]]
    check("B", "private sealed copy matches its manifest", bad == [], bad[:3])
    root = work / "sources"
    expected = oracle_lines(root)
    db = work / "out" / "a.duckdb"
    roots = ["--root", f"claude={root / 'claude'}", "--root", f"codex={root / 'codex'}", "--root", f"pi={root / 'pi'}",
             "--root", f"openrig={root / 'openrig.sqlite'}"]
    code, out, err = cli(binary, ["run", "--db", str(db), *roots])
    check("B", "sealed ingest exit 0", code == 0, err[-300:])
    rows = json.loads(cli(binary, ["trace", "--metric-id", "tokens", "--db", str(db), *WIDE])[1])["inputs"]["rows"]
    got = {}
    for r in rows:
        rel = str(Path(r["path"]).relative_to(root))
        got[(rel, r["line"])] = r["input_uncached"] + r["output"] + r["cache_read"] + r["cache_write"]
    timed = {k: v for k, v in expected.items() if v[3]}
    exp_tokens = {k: v[2] for k, v in timed.items()}
    for harness in ("claude", "pi", "codex"):
        e = {k: t for k, t in exp_tokens.items() if expected[k][0] == harness}
        g = {k: t for k, t in got.items() if k[0].startswith(harness + "/")}
        diff = sorted(set(e.items()) ^ set(g.items()))
        check("B", f"{harness}: per-line token rows equal the independent oracle ({len(e)} lines, {sum(e.values())} tokens)",
              diff == [], diff[:4])
    # Identity bindings from the sealed OpenRig events (read from the private copy).
    con = sqlite3.connect(f"file:{root / 'openrig.sqlite'}?mode=ro&immutable=1", uri=True)
    rig_names = dict(con.execute("SELECT id, name FROM rigs").fetchall()) if con.execute(
        "SELECT 1 FROM sqlite_master WHERE name='rigs'").fetchone() else {}
    binds = {}
    for (payload,) in con.execute("SELECT payload FROM events WHERE type='agent.session_identity'"):
        p = json.loads(payload)
        rt = {"claude-code": "claude"}.get(p.get("runtime"), p.get("runtime"))
        if isinstance(p.get("sessionId"), str) and isinstance(p.get("sessionName"), str):
            binds.setdefault((rt, p["sessionId"]), set()).add(p["sessionName"])
    con.close()
    seat_tokens, unlinked = {}, 0
    for (h, s, t, ts) in timed.values():
        seats = binds.get((h, s)) if s is not None else None
        if seats and len(seats) == 1:
            seat = next(iter(seats))
            seat_tokens[seat] = seat_tokens.get(seat, 0) + t
        else:
            unlinked += t
    p = report(binary, db, "decisions")
    got_seats = {r["seat"]: r["total"] for r in p["mission_by_seat"]}
    check("B", f"per-seat totals equal the oracle's linked attribution ({len(seat_tokens)} seats)",
          got_seats == seat_tokens, sorted(set(got_seats.items()) ^ set(seat_tokens.items()))[:4])
    check("B", "unlinked total equals the oracle's unlinked total", p["mission_tokens"].get("unlinked", 0) == unlinked,
          (p["mission_tokens"].get("unlinked"), unlinked))
    untimed = sum(v[2] for v in expected.values() if not v[3])
    check("B", "untimestamped all-time tokens equal the oracle", p["untimestamped_tokens_all_time"] == untimed,
          (p["untimestamped_tokens_all_time"], untimed))
    links = {(r["harness"], r["native_session"]): r["status"] for r in report(binary, db, "links")["links"]}
    sessions = {(h, s) for (h, s, t, ts) in expected.values() if s is not None}
    mismatch = []
    for key in sessions:
        seats = binds.get(key, set())
        want = "linked" if len(seats) == 1 else ("collision" if len(seats) > 1 else "unlinked")
        if links.get(key) != want:
            mismatch.append((key, links.get(key), want))
    check("B", f"per-session link status matches explicit bindings ({len(sessions)} sessions)", mismatch == [], mismatch[:4])
    missing = sorted(k for k in sessions if not binds.get(k))
    print(f"INFO B: sessions without an explicit binding (genuinely unbound, reported separately): {len(missing)}")
    fallback = sum(1 for v in expected.values() if v[1] is None)
    print(f"INFO B: usage lines in files without a valid line-1 header: {fallback}")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--binary", required=True)
    ap.add_argument("--scratch", required=True)
    args = ap.parse_args()
    binary = str(Path(args.binary).resolve())
    base = Path(args.scratch).resolve()
    if base.exists():
        sys.exit("refusing to overwrite existing evidence scratch")
    base.mkdir(parents=True, mode=0o700)
    part_a(binary, base)
    part_b(binary, base)
    failed = [r for r in results if not r["ok"]]
    (base / "results.json").write_text(json.dumps({"checks": len(results), "failed": len(failed), "results": results}, indent=1))
    print(f"RESULT {len(results) - len(failed)}/{len(results)} checks passed")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
