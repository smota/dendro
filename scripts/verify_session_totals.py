#!/usr/bin/env python3
"""AC6 per-session token totals, AC1 record counts and AC8 rerun/append proof.

Everything runs on a private copy of the verified sealed input in a new scratch
directory. Expected values come from an independent oracle in this file that
parses the source files and the OpenRig identity events from the contract rules
(K10, section 4.1). Nothing is read back from Dendro to build an expectation.

S  Sealed oracle: every session row of `report tokens` (four categories,
   tokens, reasoning, link status and seat, period records, untimestamped
   records and tokens), the discovered-session populations with explicit sets
   (usage, zero-usage file, identity only), the conservation invariants
   unscoped and under --rig agentflow-dev, report and trace equality, and the
   per-harness records_read counts.
R  AC8 identical rerun: a second run on the same analytics DB and the same
   private copy reads 0 new records and leaves every report and trace payload
   unchanged; the copy still matches the sealed manifest.
P  AC8 append: independently valued lines appended to one Claude, one Codex
   and one Pi file, and one new uniquely keyed OpenRig identity event that
   binds a previously unlinked session. Exact deltas are asserted, every other
   session row is unchanged, the oracle still matches every row, and the
   post-append manifest differs from the sealed one only in the appended files.

Usage:
  python3 scripts/verify_session_totals.py --binary /abs/dendro --scratch /abs/new/scratch
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
RIG = "agentflow-dev"
CATS = ["input_uncached", "output", "cache_read", "cache_write"]
results = []


def check(row, name, ok, detail=""):
    results.append({"row": row, "check": name, "ok": bool(ok), "detail": str(detail)[:600]})
    print(("PASS" if ok else "FAIL"), f"{row}: {name}", "" if ok else detail, flush=True)


def cli(binary, args):
    p = subprocess.run([binary, *args], capture_output=True, text=True, timeout=1200)
    return p.returncode, p.stdout, p.stderr


def payload(binary, db, kind, rig=None):
    extra = ["--rig", rig] if rig else []
    code, out, err = cli(binary, ["report", kind, "--db", str(db), *extra, *WIDE])
    if code != 0:
        raise SystemExit(f"report {kind} failed: {err[-300:]}")
    return json.loads(out)["payload"]


def trace(binary, db, metric, rig=None):
    extra = ["--rig", rig] if rig else []
    code, out, err = cli(binary, ["trace", "--metric-id", metric, "--db", str(db), *extra, *WIDE])
    if code != 0:
        raise SystemExit(f"trace {metric} failed: {err[-300:]}")
    return json.loads(out)


def manifest_of(root):
    return {str(p.relative_to(root)): hashlib.sha256(p.read_bytes()).hexdigest()
            for p in sorted(root.rglob("*")) if p.is_file()}


# ---------------------------------------------------------------- oracle

def complete_lines(path):
    lines = path.read_bytes().split(b"\n")
    for no, raw in enumerate(lines[:-1], start=1):  # the last piece has no newline: not a record yet
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


def has_ts(v):
    m = v.get("message")
    return isinstance(v.get("timestamp"), str) or (isinstance(m, dict) and isinstance(m.get("timestamp"), str))


def header_session(harness, path):
    first = next(iter(complete_lines(path)), (None, b""))[1]
    header = parse_json(first)
    sid = None
    if isinstance(header, dict):
        if harness == "pi" and header.get("type") == "session":
            sid = header.get("id")
        if harness == "codex" and header.get("type") == "session_meta" and isinstance(header.get("payload"), dict):
            sid = header["payload"].get("id")
    return sid if isinstance(sid, str) and sid.strip() else None


def oracle(root):
    """Per usage line: (harness, session, [4 categories], reasoning or None, timestamped).
    Also records read per harness and the files with no usage."""
    lines, records, zero_files, codex_last = {}, {}, [], {}
    for harness in ("claude", "pi", "codex"):
        records[harness] = 0
        for path in sorted((root / harness).rglob("*.jsonl")):
            rel = str(path.relative_to(root))
            records[harness] += sum(1 for _, raw in complete_lines(path) if raw.strip())
            before = len(lines)
            session = header_session(harness, path) if harness != "claude" else None
            seen, prev = set(), None
            for no, raw in complete_lines(path):
                v = parse_json(raw)
                if not isinstance(v, dict):
                    continue
                if harness == "claude":
                    m = v.get("message")
                    if v.get("type") != "assistant" or not isinstance(m, dict):
                        continue
                    vals = ints(m.get("usage"), ["input_tokens", "output_tokens", "cache_read_input_tokens",
                                                 "cache_creation_input_tokens"])
                    if vals is None:
                        continue
                    sid = v.get("sessionId") if isinstance(v.get("sessionId"), str) else "claude-session"
                    msg = m.get("id")
                    if isinstance(msg, str):
                        if (sid, msg) in seen:
                            continue
                        seen.add((sid, msg))
                    lines[(rel, no)] = ("claude", sid, vals, None, has_ts(v))
                elif harness == "pi":
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
                    r = m["usage"].get("reasoning")
                    lines[(rel, no)] = ("pi", session, vals, r if isinstance(r, int) else 0, has_ts(v))
                else:
                    p = v.get("payload")
                    if v.get("type") != "event_msg" or not isinstance(p, dict) or p.get("type") != "token_count":
                        continue
                    info = p.get("info")
                    total = info.get("total_token_usage") if isinstance(info, dict) else None
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
                    lines[(rel, no)] = ("codex", session, [uncached, delta[3], delta[1], delta[2]], delta[4], has_ts(v))
            if harness == "codex":
                codex_last[rel] = prev
            if len(lines) == before:
                zero_files.append((harness, rel, session))
    return lines, records, zero_files, codex_last


def identities(sqlite_triple_dir, scratch):
    """{(harness, raw session): {(rig, seat)}} from a separate copy, WAL included."""
    tmp = scratch / "oracle-sqlite"
    if tmp.exists():
        shutil.rmtree(tmp)
    tmp.mkdir(mode=0o700)
    for name in ("openrig.sqlite", "openrig.sqlite-wal", "openrig.sqlite-shm"):
        if (sqlite_triple_dir / name).exists():
            shutil.copy2(sqlite_triple_dir / name, tmp / name)
    con = sqlite3.connect(str(tmp / "openrig.sqlite"))
    rig_names = dict(con.execute("SELECT id, name FROM rigs").fetchall()) if con.execute(
        "SELECT 1 FROM sqlite_master WHERE name='rigs'").fetchone() else {}
    binds = {}
    events = con.execute("SELECT COUNT(*) FROM events").fetchone()[0]
    for (payload_text,) in con.execute("SELECT payload FROM events WHERE type='agent.session_identity'"):
        try:
            p = json.loads(payload_text)
        except ValueError:
            continue
        if not all(isinstance(p.get(k), str) for k in ("sessionId", "runtime", "sessionName")):
            continue
        rt = {"claude-code": "claude"}.get(p["runtime"], p["runtime"])
        rig_id = p.get("rigId") if isinstance(p.get("rigId"), str) else ""
        binds.setdefault((rt, p["sessionId"]), set()).add((rig_names.get(rig_id, rig_id), p["sessionName"]))
    con.close()
    shutil.rmtree(tmp)
    return binds, events, rig_names


def expected_rows(lines, binds, zero_files, rig=None):
    """Expected session rows keyed by (harness, session); zero-usage Claude files are counted, not named."""
    rows = {}

    def row(key):
        if key not in rows:
            bound = binds.get(key, set())
            status = "linked" if len(bound) == 1 else ("collision" if bound else "unlinked")
            rig_seat = next(iter(bound)) if status == "linked" else (None, None)
            rows[key] = {"status": status, "rig": rig_seat[0], "seat": rig_seat[1], "cats": [0, 0, 0, 0],
                         "reasoning": None if key[0] == "claude" else 0, "period_records": 0,
                         "untimed_records": 0, "untimed_tokens": 0, "found": set()}
        return rows[key]

    for (h, s, cats, reasoning, timed) in lines.values():
        r = row((h, s))
        r["found"].add("usage")
        if timed:
            r["cats"] = [a + b for a, b in zip(r["cats"], cats)]
            r["period_records"] += 1
            if reasoning is not None:
                r["reasoning"] += reasoning
        else:
            r["untimed_records"] += 1
            r["untimed_tokens"] += sum(cats)
    for key in binds:
        row(key)["found"].add("identity")
    for (h, rel, session) in zero_files:
        if session is not None:
            row((h, session))["found"].add("zero-usage file")
    if rig is not None:
        rows = {k: v for k, v in rows.items() if v["status"] == "linked" and v["rig"] == rig}
    return rows


def compare_sessions(tag, got_rows, want, zero_unnamed):
    got = {(r["harness"], r["session"]): r for r in got_rows}
    bad = []
    for key, w in want.items():
        g = got.get(key)
        if g is None:
            bad.append((key, "missing"))
            continue
        cats = [g[c] for c in CATS]
        want_reasoning = "not recorded" if w["reasoning"] is None else w["reasoning"]
        mism = {
            "status": (g["status"], w["status"]), "seat": (g["seat"], w["seat"]), "rig": (g["rig"], w["rig"]),
            "cats": (cats, w["cats"]), "tokens": (g["tokens"], sum(w["cats"])),
            "reasoning": (g["reasoning"], want_reasoning), "period_records": (g["period_records"], w["period_records"]),
            "untimed": ((g["untimestamped"]["records"], g["untimestamped"]["tokens"]),
                        (w["untimed_records"], w["untimed_tokens"])),
            "found": (sorted(g["discovered_by"]), sorted(w["found"])),
        }
        diff = {k: v for k, v in mism.items() if v[0] != v[1]}
        if diff:
            bad.append((key, diff))
    extra = [k for k in got if k not in want]
    unnamed = [k for k in extra if got[k]["discovered_by"] == ["zero-usage file"]
               and (k[1].startswith("file:") or k[1].startswith(f"{k[0]}-file:"))]
    surplus = [k for k in extra if k not in unnamed]
    check(tag, f"every session row equals the oracle ({len(want)} named sessions)", bad == [], bad[:3])
    check(tag, "no session row outside the oracle except unnamed zero-usage files",
          surplus == [] and len(unnamed) == zero_unnamed, (surplus[:3], len(unnamed), zero_unnamed))
    return got


def invariants(tag, p):
    rows = p["sessions"]
    total = sum(r["tokens"] for r in rows)
    check(tag, "sum of session rows equals tokens", total == p["tokens"], (total, p["tokens"]))
    for c in CATS:
        s = sum(r[c] for r in rows)
        check(tag, f"sum of session {c} equals token_categories", s == p["token_categories"][c],
              (s, p["token_categories"][c]))
    unl = sum(r["tokens"] for r in rows if r["status"] != "linked")
    check(tag, "non-linked session rows equal unlinked_tokens", unl == p["unlinked_tokens"], (unl, p["unlinked_tokens"]))
    seats = {}
    for r in rows:
        if r["status"] == "linked":
            seats[r["seat"]] = seats.get(r["seat"], 0) + r["tokens"]
    nonzero = lambda d: {k: v for k, v in d.items() if v}  # noqa: E731
    seats = nonzero(seats)
    # One category row per (seat, rig): a seat name can appear under a rig name and an old rig id.
    cat_seats = {}
    for r in p["seat_token_categories"]:
        cat_seats[r["seat"]] = cat_seats.get(r["seat"], 0) + r["tokens"]
    cat_seats = nonzero(cat_seats)
    rig_pairs = [(r["seat"], r["rig"]) for r in p["seat_token_categories"]]
    check(tag, "seat category rows are unique per (seat, rig)", len(rig_pairs) == len(set(rig_pairs)))
    m1 = nonzero({r["seat"]: r["tokens"] for r in p["m1_by_seat"]})
    mission = nonzero({r["seat"]: r["total"] for r in p["mission_by_seat"]})
    check(tag, f"linked session sums equal seat category totals ({len(seats)} seats)", seats == cat_seats,
          sorted(set(seats.items()) ^ set(cat_seats.items()))[:3])
    check(tag, "linked session sums equal M1 seat tokens", seats == m1, sorted(set(seats.items()) ^ set(m1.items()))[:3])
    check(tag, "linked session sums equal mission seat totals", seats == mission,
          sorted(set(seats.items()) ^ set(mission.items()))[:3])
    untimed = sum(r["untimestamped"]["records"] for r in rows)
    check(tag, "untimestamped session records equal untimestamped_count", untimed == p["untimestamped_count"],
          (untimed, p["untimestamped_count"]))
    check(tag, "untimestamped session tokens equal untimestamped_tokens_all_time",
          sum(r["untimestamped"]["tokens"] for r in rows) == p["untimestamped_tokens_all_time"])


def note_counts(run_json):
    out = {}
    for note in json.loads(run_json)["sources"]:
        parts = note.split()
        fields = dict(x.split("=", 1) for x in parts[1:] if "=" in x)
        out[parts[0]] = fields
    return out


def snapshot_all(binary, db):
    """Every report and trace payload, unscoped and scoped."""
    snap = {}
    for rig in (None, RIG):
        for kind in ("tokens", "links", "activity", "handoffs", "decisions"):
            snap[f"report {kind} {rig}"] = payload(binary, db, kind, rig)
        for metric in ("m1", "m2", "m3", "m4", "tokens", "intake", "medians", "activity"):
            snap[f"trace {metric} {rig}"] = trace(binary, db, metric, rig)
    return snap


def diff_paths(a, b, path=""):
    if type(a) is not type(b):
        return [path]
    if isinstance(a, dict):
        out = []
        for k in sorted(set(a) | set(b)):
            out += diff_paths(a.get(k), b.get(k), f"{path}.{k}") if k in a and k in b else [f"{path}.{k}"]
        return out
    if isinstance(a, list):
        if len(a) != len(b):
            return [f"{path}[len]"]
        out = []
        for i, (x, y) in enumerate(zip(a, b)):
            out += diff_paths(x, y, f"{path}[{i}]")
        return out
    return [] if a == b else [path]


# ---------------------------------------------------------------- run

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
    sealed_manifest = json.loads((SEALED / "manifest.json").read_text())
    sealed = {i["rel"]: i["sha256"] for i in sealed_manifest["items"]}
    work = base / "sources"
    shutil.copytree(SEALED / "sources", work)
    for p in [work, *work.rglob("*")]:
        os.chmod(p, 0o700 if p.is_dir() else 0o600)
    check("S", f"private copy matches the sealed manifest ({len(sealed)} files)", manifest_of(work) == sealed)
    db = base / "out" / "a.duckdb"
    roots = ["--root", f"claude={work / 'claude'}", "--root", f"codex={work / 'codex'}", "--root", f"pi={work / 'pi'}",
             "--root", f"openrig={work / 'openrig.sqlite'}"]

    # S: independent oracle against a first ingest into a new analytics DB.
    lines, records, zero_files, codex_last = oracle(work)
    binds, events, rig_names = identities(work, base)
    code, run1, err = cli(binary, ["run", "--db", str(db), *roots])
    check("S", "first ingest exit 0", code == 0, err[-300:])
    notes = note_counts(run1)
    for h in ("claude", "codex", "pi"):
        check("S", f"AC1 {h} records_read equals the oracle's complete non-blank lines ({records[h]})",
              notes[h].get("records_read") == str(records[h]), notes[h])
    check("S", f"AC1 openrig events equal the rows of the events table ({events})",
          notes["openrig"].get("events") == str(events), notes["openrig"])
    unnamed = sum(1 for (h, rel, s) in zero_files if s is None)
    fallback = sum(1 for v in lines.values() if v[1] is None)
    check("S", "no usage line falls back to a file id (sealed input has line-1 headers)", fallback == 0, fallback)
    p = payload(binary, db, "tokens")
    want = expected_rows(lines, binds, zero_files)
    got = compare_sessions("S", p["sessions"], want, unnamed)
    invariants("S", p)
    # Explicit reconciliation of the populations (92 usage-bearing vs 98 discovered).
    groups = {}
    for key, r in got.items():
        groups.setdefault("+".join(r["discovered_by"]), []).append(key)
    no_usage = lambda k: "usage" not in k.split("+")  # noqa: E731
    usage_bearing = {k for k in want if "usage" in want[k]["found"]}
    print("INFO S: discovered sessions", len(got), "| usage-bearing", len(usage_bearing),
          "| by source:", {k: len(v) for k, v in sorted(groups.items())})
    for k, v in sorted(groups.items()):
        if no_usage(k):
            print(f"INFO S: {k}: {sorted((h, s if not s.startswith('file:') else 'file:<source>') for h, s in v)}")
    check("S", "discovered = usage-bearing + identity-only + zero-usage files, each set explicit",
          len(got) == len(usage_bearing) + sum(len(v) for k, v in groups.items() if no_usage(k)),
          (len(got), len(usage_bearing), {k: len(v) for k, v in groups.items()}))
    # Rig scope: only sessions singly linked to the rig, with their untimestamped rows.
    q = payload(binary, db, "tokens", RIG)
    compare_sessions(f"S --rig {RIG}", q["sessions"], expected_rows(lines, binds, zero_files, RIG), 0)
    invariants(f"S --rig {RIG}", q)
    check(f"S --rig {RIG}", "no collision or unlinked session in scope",
          all(r["status"] == "linked" and r["rig"] == RIG for r in q["sessions"]))
    for rig in (None, RIG):
        t = trace(binary, db, "tokens", rig)["inputs"]
        src = p if rig is None else q
        same = [f for f in ("sessions", "token_categories", "seat_token_categories", "unlinked_tokens",
                            "unlinked_token_categories", "untimestamped_count", "untimestamped_tokens_all_time")
                if t[f] != src[f]]
        check("S", f"trace tokens carries the report's session and category fields (rig {rig})", same == [], same)

    # R: identical rerun on the same DB and copy.
    before = snapshot_all(binary, db)
    code, run2, err = cli(binary, ["run", "--db", str(db), *roots])
    check("R", "identical rerun exit 0", code == 0, err[-300:])
    notes2 = note_counts(run2)
    check("R", "identical rerun reads 0 new records per harness",
          all(notes2[h].get("records_read") == "0" and notes2[h].get("usage_records") == "0" for h in ("claude", "codex", "pi")),
          notes2)
    after = snapshot_all(binary, db)
    changed = {k: diff_paths(before[k], after[k])[:5] for k in before if before[k] != after[k]}
    check("R", f"every report and trace payload unchanged ({len(before)} payloads, unscoped and --rig {RIG})",
          changed == {}, changed)
    check("R", "private copy still matches the sealed manifest", manifest_of(work) == sealed)

    # P: append independently valued records and one new OpenRig event.
    def pick(harness, cond):
        for rel in sorted({k[0] for k, v in lines.items() if v[0] == harness}):
            if (work / rel).read_bytes().endswith(b"\n") and cond(rel):
                return rel
        raise SystemExit(f"no {harness} file to append to")

    claude_rel = pick("claude", lambda rel: True)
    claude_session = next(v[1] for k, v in lines.items() if k[0] == claude_rel)
    pi_rel = pick("pi", lambda rel: header_session("pi", work / rel) is not None)
    codex_rel = pick("codex", lambda rel: header_session("codex", work / rel) is not None and codex_last.get(rel))
    sessions_touched = {("claude", claude_session), ("pi", header_session("pi", work / pi_rel)),
                        ("codex", header_session("codex", work / codex_rel))}
    unbound = sorted(k for k, w in want.items() if w["status"] == "unlinked" and "usage" in w["found"]
                     and k not in sessions_touched and k[1] is not None)
    if not unbound:
        raise SystemExit("no unlinked usage-bearing session to bind")
    bind_key = unbound[0]
    ts = "2026-10-10T12:00:00Z"
    adds = {
        claude_rel: ({"type": "assistant", "sessionId": claude_session, "timestamp": ts,
                      "message": {"id": "ac8-append-claude", "usage": {"input_tokens": 101, "output_tokens": 202,
                                  "cache_read_input_tokens": 303, "cache_creation_input_tokens": 404}}},
                     [101, 202, 303, 404], None),
        pi_rel: ({"type": "message", "id": "ac8-append-pi", "timestamp": ts,
                  "message": {"role": "assistant", "usage": {"input": 11, "output": 22, "cacheRead": 33,
                                                             "cacheWrite": 44, "reasoning": 5}}},
                 [11, 22, 33, 44], 5),
    }
    last = codex_last[codex_rel]
    step = [1000, 300, 100, 50, 7]  # cumulative input incl. cached and cache-write, cached, cache-write, output, reasoning
    cur = [a + b for a, b in zip(last, step)]
    adds[codex_rel] = ({"type": "event_msg", "timestamp": ts, "payload": {"type": "token_count", "info": {
        "total_token_usage": {"input_tokens": cur[0], "cached_input_tokens": cur[1], "cache_write_input_tokens": cur[2],
                              "output_tokens": cur[3], "reasoning_output_tokens": cur[4]}}}},
        [600, 50, 300, 100], 7)
    for rel, (record, _, _) in adds.items():
        with open(work / rel, "ab") as f:
            f.write(json.dumps(record).encode() + b"\n")
    rig_id = next((i for i, n in rig_names.items() if n == RIG), None)
    con = sqlite3.connect(str(work / "openrig.sqlite"))
    seq = con.execute("SELECT MAX(seq) FROM events").fetchone()[0] + 1
    runtime = {"claude": "claude-code"}.get(bind_key[0], bind_key[0])
    probe_seat = "ac8-probe@agentflow-dev"
    con.execute("INSERT INTO events (seq, rig_id, node_id, type, payload, created_at) VALUES (?, ?, ?, ?, ?, ?)",
                (seq, rig_id, "ac8-probe-node", "agent.session_identity",
                 json.dumps({"type": "agent.session_identity", "rigId": rig_id, "nodeId": "ac8-probe-node",
                             "sessionName": probe_seat, "runtime": runtime, "sessionId": bind_key[1]}),
                 "2026-10-10 12:00:00"))
    con.commit()
    con.close()
    post = manifest_of(work)
    touched = sorted(k for k in set(post) | set(sealed) if post.get(k) != sealed.get(k))
    allowed = set(adds) | {"openrig.sqlite", "openrig.sqlite-wal", "openrig.sqlite-shm"}
    check("P", "post-append copy differs from the sealed manifest only in the appended files",
          set(touched) <= allowed and set(adds) <= set(touched), touched)
    (base / "post-append-manifest.json").write_text(json.dumps(post, indent=1))
    lines2, records2, zero2, _ = oracle(work)
    binds2, events2, _ = identities(work, base)
    code, run3, err = cli(binary, ["run", "--db", str(db), *roots])
    check("P", "append run exit 0", code == 0, err[-300:])
    notes3 = note_counts(run3)
    check("P", "exactly one new record read per appended harness",
          all(notes3[h].get("records_read") == "1" and notes3[h].get("usage_records") == "1" for h in ("claude", "codex", "pi")),
          notes3)
    check("P", "openrig events read include the one new event", notes3["openrig"].get("events") == str(events + 1),
          (notes3["openrig"], events))
    p2 = payload(binary, db, "tokens")
    added = sum(sum(c) for (_, c, _) in adds.values())
    check("P", f"overall tokens grow by exactly the appended values ({added})", p2["tokens"] - p["tokens"] == added,
          (p["tokens"], p2["tokens"]))
    old = {(r["harness"], r["session"]): r for r in p["sessions"]}
    new = {(r["harness"], r["session"]): r for r in p2["sessions"]}
    expected_delta = {("claude", claude_session): adds[claude_rel],
                      ("pi", header_session("pi", work / pi_rel)): adds[pi_rel],
                      ("codex", header_session("codex", work / codex_rel)): adds[codex_rel]}
    bad = []
    for key, (_, cats, reasoning) in expected_delta.items():
        d = [new[key][c] - old[key][c] for c in CATS]
        if d != cats:
            bad.append((key[0], d, cats))
        if reasoning is not None and new[key]["reasoning"] - old[key]["reasoning"] != reasoning:
            bad.append((key[0], "reasoning"))
    check("P", "appended sessions grow by exactly their appended categories and reasoning", bad == [], bad)
    b_old, b_new = old[bind_key], new[bind_key]
    check("P", "the newly bound session links to the probe seat with unchanged tokens",
          b_old["status"] == "unlinked" and b_new["status"] == "linked" and b_new["seat"] == probe_seat
          and b_new["tokens"] == b_old["tokens"], (b_old["status"], b_new["status"], b_new["seat"]))
    moved = sum(sum(expected_delta[k][1]) for k in expected_delta if new[k]["status"] != "linked")
    check("P", "unlinked total drops by exactly the bound session (plus appended unlinked tokens)",
          p2["unlinked_tokens"] == p["unlinked_tokens"] - b_old["tokens"] + moved,
          (p["unlinked_tokens"], p2["unlinked_tokens"], b_old["tokens"], moved))
    others = [k for k in old if k not in expected_delta and k != bind_key and old[k] != new.get(k)]
    check("P", f"no other session row moved ({len(old) - len(expected_delta) - 1} rows)", others == [] and set(new) == set(old), others[:3])
    want2 = expected_rows(lines2, binds2, zero2)
    compare_sessions("P", p2["sessions"], want2, sum(1 for (h, rel, s) in zero2 if s is None))
    invariants("P", p2)
    q2 = payload(binary, db, "tokens", RIG)
    compare_sessions(f"P --rig {RIG}", q2["sessions"], expected_rows(lines2, binds2, zero2, RIG), 0)
    invariants(f"P --rig {RIG}", q2)
    sealed_now = {i["rel"]: hashlib.sha256((SEALED / "sources" / i["rel"]).read_bytes()).hexdigest()
                  for i in sealed_manifest["items"]}
    check("P", "the sealed input itself is unchanged", sealed_now == sealed)

    failed = [r for r in results if not r["ok"]]
    (base / "results.json").write_text(json.dumps({"checks": len(results), "failed": len(failed), "results": results},
                                                  indent=1))
    print(f"RESULT {len(results) - len(failed)}/{len(results)} checks passed")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
