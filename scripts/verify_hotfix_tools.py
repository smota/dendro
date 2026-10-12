#!/usr/bin/env python3
"""Focused proof for hotfix v0.1.1: Claude tool calls (#35), rejected records
(#36), tool outcomes from tool_result (#37) and object-input fingerprints (Q4).

Plan 536f040a, analyst Phase 1 0002 (408fc5b1), addendum 9a089c87. Expected
values are fixture constants from those rules, never Dendro output. The
sealed part compares new and old binaries on private copies of the verified
sealed input against an independent oracle that reads the raw JSONL files.
Exit 0 only if every gated assertion passes.

  python3 scripts/verify_hotfix_tools.py --binary /abs/dendro --scratch /abs/new/dir \\
      [--old-binary /abs/old/dendro] [--skip-sealed]
"""
import argparse
import collections
import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parent))
import verify_qa_batch_fixes as q  # noqa: E402  (shared fixture helpers)

REPO = Path(__file__).resolve().parent.parent
SEALED = REPO / ".agent-runs/proof/sealed-input-40c65db7"
CLAUDE_TYPES = {"agent-name", "assistant", "atis-latch", "attachment", "cost-state", "custom-title",
                "file-history-delta", "file-history-snapshot", "last-prompt", "mode", "permission-mode",
                "pr-link", "queue-operation", "system", "user"}
results = collections.OrderedDict()


def check(case, name, ok, detail=None):
    results.setdefault(case, []).append({"check": name, "ok": bool(ok), "detail": detail})
    print(("PASS " if ok else "FAIL ") + f"{case}: {name}" + ("" if ok or detail is None else f" -- {detail}"))


# ------------------------------------------------------------ raw fixture lines
def tu(tid, inp='{"p":1}', name="Read", is_error=None):
    flag = "" if is_error is None else f',"is_error":{json.dumps(is_error)}'
    return f'{{"type":"tool_use","id":"{tid}","name":"{name}","input":{inp}{flag}}}'


def chunk(msg, ms, blocks, tokens=1, session="s", kind="assistant"):
    ts = "" if ms is None else f',"timestamp":"{q.at(ms)}"'
    return (f'{{"type":"{kind}","sessionId":"{session}"{ts},"message":{{"id":"{msg}","content":[{",".join(blocks)}],'
            f'"usage":{{"input_tokens":{tokens},"output_tokens":0,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}}}}')


def res(ms, outcomes, session="s", content="done"):
    blocks = []
    for tid, flag in outcomes:
        extra = "" if flag is None else f',"is_error":{flag}'
        blocks.append(f'{{"type":"tool_result","tool_use_id":"{tid}","content":"{content}"{extra}}}')
    return f'{{"type":"user","sessionId":"{session}","timestamp":"{q.at(ms)}","message":{{"role":"user","content":[{",".join(blocks)}]}}}}'


def write_lines(path, lines, mode="w"):
    path.parent.mkdir(parents=True, exist_ok=True)
    with open(path, mode) as f:
        for line in lines:
            f.write(line + "\n")


def census(db):
    code, out, err = q.run(["cargo", "run", "-q", "-p", "dendro-connectors", "--example", "tool_census", "--", db])
    if code != 0:
        raise SystemExit(f"tool_census failed: {err[-300:]}")
    return {row["harness"]: row for row in json.loads(out)["tool_facts"]}


def loops_of(binary, db, frm=q.FROM, to=q.TO):
    code, out, _ = q.run([binary, "report", "decisions", "--db", db, "--from", frm, "--to", to])
    return sorted(l["count"] for l in json.loads(out)["payload"]["tool_loops"]) if code == 0 else None


# ------------------------------------------------------------ loop and outcome cases
S = 1000  # ms per second


def loop_cases():
    three = [tu("a"), tu("b"), tu("c")]
    return {
        # name: (lines, expected loop counts)
        "#35 three failures on later chunks of one streamed message": (
            [chunk("m1", 1 * S, [], 3), chunk("m1", 2 * S, [three[0]], 3), chunk("m1", 3 * S, [three[1]], 3),
             chunk("m1", 4 * S, [three[2]], 3), res(5 * S, [("a", "true"), ("b", "true"), ("c", "true")])], [3]),
        "#37 old shape is_error on tool_use, no results": (
            [chunk("m1", 1 * S, [tu("a", is_error=True), tu("b", is_error=True), tu("c", is_error=True)])], []),
        "#35 one tool_use id repeated in three chunks is one call": (
            [chunk("m1", 1 * S, [tu("a")]), chunk("m1", 2 * S, [tu("a")]), chunk("m1", 3 * S, [tu("a")]),
             res(4 * S, [("a", "true")])], []),
        "#36 unknown-type records with tool blocks": (
            [chunk(f"x{n}", n * S, [tu(f"u{n}")], kind="future_kind") for n in (1, 2, 3)]
            + [res(5 * S, [("u1", "true"), ("u2", "true"), ("u3", "true")])], []),
        "#37 middle result missing": ([chunk("m1", 1 * S, three), res(2 * S, [("a", "true"), ("c", "true")])], []),
        "#37 middle is_error absent": ([chunk("m1", 1 * S, three), res(2 * S, [("a", "true"), ("b", None), ("c", "true")])], []),
        "#37 middle is_error not a boolean": ([chunk("m1", 1 * S, three), res(2 * S, [("a", "true"), ("b", '"yes"'), ("c", "true")])], []),
        "#37 middle result false (success)": ([chunk("m1", 1 * S, three), res(2 * S, [("a", "true"), ("b", "false"), ("c", "true")])], []),
        "#37 middle results disagree": ([chunk("m1", 1 * S, three), res(2 * S, [("a", "true"), ("b", "true"), ("b", "false"), ("c", "true")])], []),
        "#37 orphan result creates no call": ([chunk("m1", 1 * S, [tu("a"), tu("c")]), res(2 * S, [("a", "true"), ("orphan", "true"), ("c", "true")])], []),
        "#37 same ids in another session never settle": ([chunk("m1", 1 * S, three), res(2 * S, [("a", "true"), ("b", "true"), ("c", "true")], session="t")], []),
        "#37 delayed results keep the tool_use time": (
            [chunk("m1", 0, [tu("a")]), chunk("m2", 300 * S, [tu("b")]), chunk("m3", 600 * S, [tu("c")]),
             res(1300 * S, [("a", "true"), ("b", "true"), ("c", "true")])], [3]),
        "#37 tool_use times span 700 s, results close together": (
            [chunk("m1", 0, [tu("a")]), chunk("m2", 300 * S, [tu("b")]), chunk("m3", 700 * S, [tu("c")]),
             res(701 * S, [("a", "true"), ("b", "true"), ("c", "true")])], []),
        "#37 untimestamped middle failure stays in the run (K11)": (
            [chunk("m1", 1 * S, [tu("a")]), chunk("m2", None, [tu("b")]), chunk("m3", 3 * S, [tu("c")]),
             res(4 * S, [("a", "true"), ("b", "true"), ("c", "true")])], [3]),
        "Q4 equal object inputs form a run": (
            [chunk("m1", 1 * S, [tu("a", '{"path":"/x","n":1}'), tu("b", '{"path":"/x","n":1}'), tu("c", '  {"path":"/x","n":1}')]),
             res(2 * S, [("a", "true"), ("b", "true"), ("c", "true")])], [3]),
        "Q4 key order differs: not the same input": (
            [chunk("m1", 1 * S, [tu("a", '{"path":"/x","n":1}'), tu("b", '{"n":1,"path":"/x"}'), tu("c", '{"path":"/x","n":1}')]),
             res(2 * S, [("a", "true"), ("b", "true"), ("c", "true")])], []),
        "Q4 interior whitespace differs: not the same input": (
            [chunk("m1", 1 * S, [tu("a", '{"path":"/x"}'), tu("b", '{"path": "/x"}'), tu("c", '{"path":"/x"}')]),
             res(2 * S, [("a", "true"), ("b", "true"), ("c", "true")])], []),
    }


def case_loops(binary, base, tag):
    case = f"loops and outcomes [{tag}]"
    for name, (lines, want) in loop_cases().items():
        b = base / re.sub(r"[^A-Za-z0-9]+", "-", name)
        write_lines(b / "claude" / "s.jsonl", lines)
        db, code, _, err = q.ingest(binary, b, [("claude", b / "claude")])
        got = loops_of(binary, db) if code == 0 else None
        check(case, f"{name}: loops {want}", got == want, {"got": got, "exit": code, "err": err[-200:]})
    # The orphan result creates no call; the repeated id is one call.
    b = base / re.sub(r"[^A-Za-z0-9]+", "-", "#37 orphan result creates no call")
    c = census(b / "out" / "a.duckdb").get("claude", {})
    check(case, "orphan result: 2 calls stored, none for the orphan", c.get("calls") == 2, c)
    b = base / re.sub(r"[^A-Za-z0-9]+", "-", "#35 one tool_use id repeated in three chunks is one call")
    c = census(b / "out" / "a.duckdb").get("claude", {})
    check(case, "repeated id: 1 call, failed", c.get("calls") == 1 and c.get("failed") == 1, c)
    b = base / re.sub(r"[^A-Za-z0-9]+", "-", "#35 three failures on later chunks of one streamed message")
    pay = q.report(binary, b / "out" / "a.duckdb") or {}
    check(case, "streamed loop fixture: usage counted once (3 tokens)", pay.get("tokens") == 3, pay.get("tokens"))
    # Identical rerun adds nothing.
    db, code, _, _ = q.ingest(binary, b, [("claude", b / "claude")])
    c = census(db).get("claude", {})
    check(case, "identical rerun: still 3 calls, loop [3]", c.get("calls") == 3 and loops_of(binary, db) == [3], c)


def case_append(binary, base, tag):
    case = f"split ingest equals fresh [{tag}]"
    part1 = [chunk("m1", 1 * S, [tu("a"), tu("b"), tu("c")], 2)]
    part2 = [res(2 * S, [("a", "true"), ("b", "true"), ("c", "true")])]
    part3 = [res(3 * S, [("b", "false")])]
    for name, parts, want in [("results appended later", [part1, part2], [3]),
                              ("late conflicting result", [part1, part2, part3], []),
                              ("results before their calls", [part2, part1], [3])]:
        fresh, split = base / name / "fresh", base / name / "split"
        write_lines(fresh / "claude" / "s.jsonl", [l for p in parts for l in p])
        dbf, _, _, _ = q.ingest(binary, fresh, [("claude", fresh / "claude")])
        f_loops, f_census = loops_of(binary, dbf), census(dbf).get("claude")
        write_lines(split / "claude" / "s.jsonl", parts[0])
        dbs, _, _, _ = q.ingest(binary, split, [("claude", split / "claude")])
        for p in parts[1:]:
            write_lines(split / "claude" / "s.jsonl", p, mode="a")
            dbs, _, _, _ = q.ingest(binary, split, [("claude", split / "claude")])
        s_loops, s_census = loops_of(binary, dbs), census(dbs).get("claude")
        check(case, f"{name}: fresh loops {want}", f_loops == want, f_loops)
        check(case, f"{name}: split ingest equals fresh (loops and outcomes)", (s_loops, s_census) == (f_loops, f_census),
              {"fresh": [f_loops, f_census], "split": [s_loops, s_census]})


def case_intake(binary, base, tag):
    case = f"#35 streamed intake end [{tag}]"
    # Claim (events only) at 1 s; message m1 streams: 5 tokens at 2 s, then a
    # Write chunk at 3 s (same id, usage counted once); m2 7 tokens at 5 s;
    # the receiver closes the item at 10 s. End = the Write at 3 s: 5 tokens.
    b = base / "streamed-intake"
    q.openrig_db(b / "openrig.sqlite", [
        ("agent.session_identity", {"sessionId": "qa-session", "runtime": "claude-code", "sessionName": "qa-seat", "rigId": "rig-id"}, "2026-10-09 00:00:00"),
        ("queue.created", {"qitemId": "next", "sourceSession": "build-seat", "destinationSession": "qa-seat"}, "2026-10-09 00:00:00"),
        ("queue.claimed", {"qitemId": "next", "destinationSession": "qa-seat", "claimedAt": q.at(1 * S)}, "2026-10-09 00:00:01"),
    ], [(1, "root", q.at(500), "done", "build-seat", "handed_off_to", "qa-seat", None),
        (2, "next", q.at(10 * S), "done", "qa-seat", "no-follow-on", None, None)])
    write_lines(b / "claude" / "s.jsonl", [
        chunk("m1", 2 * S, [], 5, session="qa-session"),
        chunk("m1", 3 * S, [tu("w1", '{"file":"x"}', name="Write")], 5, session="qa-session"),
        chunk("m2", 5 * S, [], 7, session="qa-session"),
    ])
    db, code, _, err = q.ingest(binary, b, [("claude", b / "claude"), ("openrig", b / "openrig.sqlite")])
    rows = ((q.trace(binary, db, "intake") or {}).get("value") or []) if code == 0 else []
    row = next((r for r in rows if r["to"] == "qa-seat"), None)
    got = (row["status"], row["tokens"], row["write"]) if row else None
    want = ("estimate", 5, q.at(3 * S))
    check(case, "estimate 5 ending at the streamed Write (3 s)", got is not None and got[:2] == want[:2] and q.parse(got[2]) == q.parse(want[2]),
          {"got": got, "rows": rows})
    pay = q.report(binary, db) or {}
    check(case, "usage unchanged: 12 tokens, m1 once", pay.get("tokens") == 12, pay.get("tokens"))


def case_privacy(binary, base, tag):
    case = f"privacy of inputs, ids and results [{tag}]"
    b = base / "privacy"
    write_lines(b / "claude" / "s.jsonl", [
        chunk("m1", 1 * S, [tu("toolu_ZZIDmarker", '{"cmd":"ZZINPUTmarker sk-ant-api03-ZZSECRETinput"}')]),
        res(2 * S, [("toolu_ZZIDmarker", "true")], content="ZZRESULTmarker"),
        chunk("x", 3 * S, [tu("toolu_ZZUNKNOWNid", '"ZZUNKNOWNinput"')], kind="ZZUNKNOWNtype"),
    ])
    db, code, _, _ = q.ingest(binary, b, [("claude", b / "claude")])
    needles = ["ZZIDmarker", "ZZINPUTmarker", "ZZSECRETinput", "ZZRESULTmarker", "ZZUNKNOWNid", "ZZUNKNOWNinput"]
    code2, out, err = q.run(["cargo", "run", "-q", "-p", "dendro-connectors", "--example", "decoded_scan", "--", db, *needles])
    hits = collections.Counter()
    for column in json.loads(out)["columns"] if code2 == 0 else []:
        for needle, n in column["hits"].items():
            hits[needle] += n
    check(case, "decoded DB: no input, result, tool id or rejected-record text", code2 == 0 and sum(hits.values()) == 0, dict(hits))
    raw = b"".join(p.read_bytes() for p in (b / "out").iterdir() if p.is_file() and not p.name.endswith(".key"))
    leaked = [n for n in needles if n.encode() in raw]
    check(case, "DB and WAL bytes: no marker", not leaked, leaked)
    c = census(db).get("claude", {})
    check(case, "the call is stored, fingerprinted and failed", c.get("calls") == 1 and c.get("fingerprinted") == 1 and c.get("failed") == 1, c)


# ------------------------------------------------------------ Pi (Q5) and Codex (Q6)
def pi_head(sid="pi-s"):
    return f'{{"type":"session","version":3,"id":"{sid}","timestamp":"{q.at(0)}"}}'


def pi_calls(msg, ms, calls, tokens=1):
    blocks = ",".join(f'{{"type":"toolCall","id":"{cid}","name":"{name}","arguments":{args}}}' for cid, name, args in calls)
    ts = "" if ms is None else f',"timestamp":"{q.at(ms)}"'
    return (f'{{"type":"message","id":"{msg}"{ts},"message":{{"role":"assistant","content":[{blocks}],'
            f'"usage":{{"input":{tokens},"output":0,"cacheRead":0,"cacheWrite":0}}}}}}')


def pi_res(ms, cid, flag):
    extra = "" if flag is None else f',"isError":{flag}'
    return (f'{{"type":"message","id":"r-{cid}-{ms}","timestamp":"{q.at(ms)}","message":{{"role":"toolResult",'
            f'"toolCallId":"{cid}","toolName":"read","content":[{{"type":"text","text":"PIRESULTcontent"}}]{extra}}}}}')


def pi_loop_cases():
    rd = lambda cid, args='{"path":"/a"}': (cid, "read", args)
    three = [rd("a"), rd("b"), rd("c")]
    res3 = lambda b="true": [pi_res(5 * S, "a", "true"), pi_res(5 * S, "b", b), pi_res(5 * S, "c", "true")]
    return {
        "Pi three failures": ([pi_head(), pi_calls("m1", 1 * S, three)] + res3(), [3]),
        "Pi middle result missing": ([pi_head(), pi_calls("m1", 1 * S, three), pi_res(5 * S, "a", "true"), pi_res(5 * S, "c", "true")], []),
        "Pi middle isError absent": ([pi_head(), pi_calls("m1", 1 * S, three)] + res3(None), []),
        "Pi middle isError not a boolean": ([pi_head(), pi_calls("m1", 1 * S, three)] + res3('"yes"'), []),
        "Pi middle success": ([pi_head(), pi_calls("m1", 1 * S, three)] + res3("false"), []),
        "Pi middle results disagree": ([pi_head(), pi_calls("m1", 1 * S, three)] + res3() + [pi_res(6 * S, "b", "false")], []),
        "Pi repeated id is one call": ([pi_head(), pi_calls("m1", 1 * S, [rd("a")]), pi_calls("m2", 2 * S, [rd("a")]), pi_calls("m3", 3 * S, [rd("a")]), pi_res(4 * S, "a", "true")], []),
        "Pi orphan results create no calls": ([pi_head(), pi_calls("m1", 1 * S, [rd("a"), rd("c")]), pi_res(4 * S, "a", "true"), pi_res(4 * S, "zz", "true"), pi_res(4 * S, "c", "true")], []),
        "Pi delayed results keep the call time": ([pi_head(), pi_calls("m1", 0, [rd("a")]), pi_calls("m2", 300 * S, [rd("b")]), pi_calls("m3", 600 * S, [rd("c")])]
                                                  + [pi_res(1300 * S, x, "true") for x in "abc"], [3]),
        "Pi untimestamped middle call stays in the run": ([pi_head(), pi_calls("m1", 1 * S, [rd("a")]), pi_calls("m2", None, [rd("b")]), pi_calls("m3", 3 * S, [rd("c")])]
                                                          + [pi_res(4 * S, x, "true") for x in "abc"], [3]),
        "Pi key order differs: not the same arguments": ([pi_head(), pi_calls("m1", 1 * S, [rd("a", '{"path":"/a","n":1}'), rd("b", '{"n":1,"path":"/a"}'), rd("c", '{"path":"/a","n":1}')])]
                                                         + [pi_res(4 * S, x, "true") for x in "abc"], []),
        "Pi interior whitespace differs": ([pi_head(), pi_calls("m1", 1 * S, [rd("a", '{"path":"/a"}'), rd("b", '{"path": "/a"}'), rd("c", '{"path":"/a"}')])]
                                           + [pi_res(4 * S, x, "true") for x in "abc"], []),
        "Pi outer whitespace only": ([pi_head(), pi_calls("m1", 1 * S, [rd("a", '{"path":"/a"}'), rd("b", '  {"path":"/a"}  '), rd("c", '{"path":"/a"}')])]
                                     + [pi_res(4 * S, x, "true") for x in "abc"], [3]),
        "Pi unknown-type records yield no calls": ([pi_head()] + [f'{{"type":"future_kind","timestamp":"{q.at(n * S)}","message":{{"role":"assistant","content":[{{"type":"toolCall","id":"u{n}","name":"read","arguments":{{}}}}]}}}}' for n in (1, 2, 3)]
                                                   + [pi_res(4 * S, f"u{n}", "true") for n in (1, 2, 3)], []),
    }


def case_pi(binary, base, tag):
    case = f"Pi tool calls Q5 [{tag}]"
    for name, (lines, want) in pi_loop_cases().items():
        b = base / re.sub(r"[^A-Za-z0-9]+", "-", name)
        write_lines(b / "pi" / "p.jsonl", lines)
        db, code, _, err = q.ingest(binary, b, [("pi", b / "pi")])
        got = loops_of(binary, db) if code == 0 else None
        check(case, f"{name}: loops {want}", got == want, {"got": got, "exit": code, "err": err[-200:]})
    b = base / re.sub(r"[^A-Za-z0-9]+", "-", "Pi orphan results create no calls")
    c = census(b / "out" / "a.duckdb").get("pi", {})
    check(case, "orphan results: 2 Pi calls stored", c.get("calls") == 2, c)
    b = base / re.sub(r"[^A-Za-z0-9]+", "-", "Pi repeated id is one call")
    c = census(b / "out" / "a.duckdb").get("pi", {})
    check(case, "repeated id: 1 call, failed", c.get("calls") == 1 and c.get("failed") == 1, c)
    # Writes: write and edit only.
    b = base / "pi-writes"
    write_lines(b / "pi" / "p.jsonl", [pi_head(), pi_calls("m1", 1 * S, [("w", "write", "{}"), ("e", "edit", "{}"), ("x", "bash", "{}"), ("r", "read", "{}")])])
    db, code, _, _ = q.ingest(binary, b, [("pi", b / "pi")])
    c = census(db).get("pi", {})
    check(case, "write and edit are structured writes; bash and read are not", c.get("calls") == 4 and c.get("structured_writes") == 2, c)
    # Split ingest and a late conflict equal a fresh ingest.
    rd = lambda cid: (cid, "read", '{"path":"/a"}')
    parts = [[pi_head(), pi_calls("m1", 1 * S, [rd("a"), rd("b"), rd("c")])], [pi_res(2 * S, x, "true") for x in "abc"], [pi_res(3 * S, "b", "false")]]
    for name, use, want in (("appended results", parts[:2], [3]), ("late conflict", parts, [])):
        fresh, split = base / f"pi-{name}" / "fresh", base / f"pi-{name}" / "split"
        write_lines(fresh / "pi" / "p.jsonl", [l for p_ in use for l in p_])
        dbf, _, _, _ = q.ingest(binary, fresh, [("pi", fresh / "pi")])
        write_lines(split / "pi" / "p.jsonl", use[0])
        dbs, _, _, _ = q.ingest(binary, split, [("pi", split / "pi")])
        for p_ in use[1:]:
            write_lines(split / "pi" / "p.jsonl", p_, mode="a")
            dbs, _, _, _ = q.ingest(binary, split, [("pi", split / "pi")])
        f_state, s_state = (loops_of(binary, dbf), census(dbf).get("pi")), (loops_of(binary, dbs), census(dbs).get("pi"))
        check(case, f"{name}: fresh loops {want} and split ingest equal", f_state[0] == want and f_state == s_state, {"fresh": f_state, "split": s_state})


def case_pi_intake(binary, base, tag):
    case = f"Pi write as intake end Q5 [{tag}]"
    cases = [
        # name, Pi calls after the claim at 1 s, expected (status, tokens, end ms)
        ("Pi write at 3 s ends the window", [("w", "write", 3 * S)], ("estimate", 5, 3 * S)),
        ("Pi edit at 3 s ends the window", [("e", "edit", 3 * S)], ("estimate", 5, 3 * S)),
        ("Pi bash and read never end it", [("x", "bash", 3 * S), ("r", "read", 4 * S)], ("estimate", 12, 10 * S)),
        ("Pi write exactly at the claim gives zero", [("w", "write", 1 * S)], ("estimate", 0, 1 * S)),
        ("Pi write before the claim does not count", [("w", "write", 500)], ("estimate", 12, 10 * S)),
    ]
    for name, calls, (status, tokens, end) in cases:
        b = base / re.sub(r"[^A-Za-z0-9]+", "-", name)
        q.openrig_db(b / "openrig.sqlite", [
            ("agent.session_identity", {"sessionId": "pi-intake", "runtime": "pi", "sessionName": "qa-seat", "rigId": "rig-id"}, "2026-10-09 00:00:00"),
            ("queue.created", {"qitemId": "next", "sourceSession": "build-seat", "destinationSession": "qa-seat"}, "2026-10-09 00:00:00"),
            ("queue.claimed", {"qitemId": "next", "destinationSession": "qa-seat", "claimedAt": q.at(1 * S)}, "2026-10-09 00:00:01"),
        ], [(1, "root", q.at(500), "done", "build-seat", "handed_off_to", "qa-seat", None),
            (2, "next", q.at(10 * S), "done", "qa-seat", "no-follow-on", None, None)])
        lines = [pi_head("pi-intake"), pi_calls("u1", 2 * S, [], 5), pi_calls("u2", 5 * S, [], 7)]
        lines += [pi_calls(f"c{n}", ms, [(f"t{n}", nm, '{"path":"/x"}')], 0) for n, (_, nm, ms) in enumerate(calls)]
        write_lines(b / "pi" / "p.jsonl", lines)
        db, code, _, err = q.ingest(binary, b, [("pi", b / "pi"), ("openrig", b / "openrig.sqlite")])
        rows = ((q.trace(binary, db, "intake") or {}).get("value") or []) if code == 0 else []
        row = next((r for r in rows if r["to"] == "qa-seat"), None)
        got = (row["status"], row["tokens"], row["write"]) if row else None
        ok = got is not None and got[:2] == (status, tokens) and got[2] is not None and q.parse(got[2]) == q.parse(q.at(end))
        check(case, f"{name}: {status} {tokens}", ok, {"got": got, "exit": code, "err": err[-200:]})


def case_codex(binary, base, tag):
    case = f"Codex not read Q6 [{tag}]"
    b = base / "codex"
    for name, sid in (("x.jsonl", "cx"), ("y.jsonl", "cy")):
        write_lines(b / "codex" / name, [
            f'{{"type":"session_meta","timestamp":"{q.at(0)}","payload":{{"id":"{sid}"}}}}',
            f'{{"type":"response_item","timestamp":"{q.at(1 * S)}","payload":{{"type":"custom_tool_call","id":"i","call_id":"c","name":"exec","input":"ls","status":"completed"}}}}',
            f'{{"type":"event_msg","timestamp":"{q.at(2 * S)}","payload":{{"type":"token_count","info":{{"total_token_usage":{{"input_tokens":4,"cached_input_tokens":0,"cache_write_input_tokens":0,"output_tokens":0,"reasoning_output_tokens":0}}}}}}}}',
        ])
    db, code, out, err = q.ingest(binary, b, [("codex", b / "codex")])
    notes = json.loads(out).get("sources", []) if code == 0 else []
    check(case, "run source note says tool_calls=not read", any(n.startswith("codex files=2 ") and n.endswith(" tool_calls=not read") for n in notes), notes)
    write_lines(b / "codex" / "x.jsonl", [f'{{"type":"event_msg","timestamp":"{q.at(3 * S)}","payload":{{"type":"token_count","info":{{"total_token_usage":{{"input_tokens":6,"cached_input_tokens":0,"cache_write_input_tokens":0,"output_tokens":0,"reasoning_output_tokens":0}}}}}}}}'], mode="a")
    q.ingest(binary, b, [("codex", b / "codex")])
    q.ingest(binary, b, [("codex", b / "codex")])
    hits, _ = q.decoded(db, ["tool calls not read for codex"])
    check(case, "one stored warning per Codex file across reruns (2)", hits["tool calls not read for codex"] == 2, hits)
    check(case, "no Codex tool facts and no Codex loop row", "codex" not in census(db) and loops_of(binary, db) == [], census(db))


def case_wal_privacy(binary, base, tag):
    """Non-empty pre-checkpoint WAL scan for Claude and Pi object inputs,
    result text and call ids, with a stored-name positive control."""
    case = f"WAL privacy Claude+Pi objects [{tag}]"
    import sqlite3 as _sq
    import time as _t
    src, out, tmp = base / "src", base / "out", base / "tmp"
    for d in (src / "claude" / "proj", src / "pi" / "proj", out, tmp):
        d.mkdir(parents=True, exist_ok=True)
    out.chmod(0o700)
    n_calls = 10000
    cl, pl = [], [pi_head("pi-wal")]
    for n in range(n_calls):
        cl.append(chunk(f"m{n}", n, [tu(f"toolu_ZZCLID{n}", '{"command":"ZZCLINPUT sk-ant-api03-ZZCLSECRET %d"}' % n, name="ZZVISIBLEclaude")]))
        cl.append(res(n, [(f"toolu_ZZCLID{n}", "true")], content="ZZCLRESULT"))
        pl.append(pi_calls(f"p{n}", n, [(f"ZZPIID{n}", "ZZVISIBLEpi", '{"command":"ZZPIINPUT sk-ant-api03-ZZPISECRET %d"}' % n)]))
        pl.append(pi_res(n, f"ZZPIID{n}", "true").replace("PIRESULTcontent", "ZZPIRESULT"))
    write_lines(src / "claude" / "proj" / "s.jsonl", cl)
    write_lines(src / "pi" / "proj" / "p.jsonl", pl)
    needles = ["ZZCLID", "ZZCLINPUT", "ZZCLSECRET", "ZZCLRESULT", "ZZPIID", "ZZPIINPUT", "ZZPISECRET", "ZZPIRESULT"]
    raw_src = (src / "claude" / "proj" / "s.jsonl").read_bytes() + (src / "pi" / "proj" / "p.jsonl").read_bytes()
    check(case, "non-vacuous: sources hold every canary", all(n.encode() in raw_src for n in needles))
    db = src / "openrig.sqlite"
    con = _sq.connect(db)
    con.execute("PRAGMA journal_mode=WAL")
    con.executescript("CREATE TABLE events (seq INTEGER, type TEXT, payload TEXT, created_at TEXT); CREATE TABLE rigs (id TEXT, name TEXT);")
    con.executemany("INSERT INTO events VALUES (?,?,?,'2026-10-09 00:30:00')",
                    [(i + 1, "seat.activity_changed", json.dumps({"sessionName": "s1", "state": "idle", "at": "2026-10-09T00:42:00Z", "pad": i})) for i in range(400)])
    con.commit()
    con.close()
    analytics = out / "analytics.duckdb"
    cmd = [str(binary), "run", "--db", str(analytics), "--frozen", "--root", f"claude={src / 'claude'}", "--root", f"pi={src / 'pi'}", "--root", f"openrig={db}"]
    p = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, env=dict(os.environ, TMPDIR=str(tmp)))
    wal = b""
    while p.poll() is None:
        for stage in out.glob(".dendro-stage-*"):
            try:
                w = stage / "analytics.duckdb.wal"
                if w.exists() and w.stat().st_size > len(wal):
                    wal = w.read_bytes()
            except OSError:
                pass
        _t.sleep(0.005)
    stdout, stderr = p.communicate()
    check(case, "run exit 0", p.returncode == 0, stderr[-300:])
    check(case, "staged WAL captured non-empty before checkpoint", len(wal) > 0, len(wal))
    print(f"INFO {case}: pre-checkpoint WAL bytes {len(wal)}")
    check(case, "positive control: stored Claude and Pi tool names visible in that WAL", b"ZZVISIBLEclaude" in wal and b"ZZVISIBLEpi" in wal)
    leaked = [n for n in needles if n.encode() in wal]
    check(case, "pre-checkpoint WAL holds no input, secret, result or id canary", not leaked, leaked)
    final = b"".join(f.read_bytes() for f in out.rglob("*") if f.is_file() and not f.name.endswith(".key"))
    leaked = [n for n in needles if n.encode() in final]
    check(case, "final DB/WAL bytes hold no canary", not leaked, leaked)
    hits, _ = q.decoded(analytics, needles)
    r = q.run(["cargo", "run", "-q", "-p", "dendro-connectors", "--example", "decoded_scan", "--", analytics, *needles])
    total = sum(n for col in json.loads(r[1])["columns"] for n in col["hits"].values()) if r[0] == 0 else None
    check(case, "decoded VARCHAR columns hold no canary", total == 0, total)
    c = census(analytics)
    check(case, f"non-vacuous: {n_calls} Claude and {n_calls} Pi calls stored, fingerprinted, failed",
          all(c.get(h, {}).get(k) == n_calls for h in ("claude", "pi") for k in ("calls", "fingerprinted", "failed")), c)
    surf = stdout + stderr
    for kind in ("tokens", "decisions"):
        rr = subprocess.run([str(binary), "report", kind, "--db", str(analytics), "--from", "2026-10-08T00:00:00Z", "--to", "2026-10-11T00:00:00Z"], capture_output=True, text=True)
        surf += rr.stdout + rr.stderr
    leaked = [n for n in needles if n in surf]
    check(case, "run and report output hold no canary", not leaked, leaked)


# ------------------------------------------------------------ sealed comparison
def oracle(src):
    """Independent per-file Claude tool oracle from raw JSONL (Phase 1 rules)."""
    calls, failed, succeeded, unknown, with_input, writes = 0, 0, 0, 0, 0, 0
    loops = []
    sessions = collections.defaultdict(list)
    for path in sorted((src / "claude").rglob("*.jsonl")):
        first, outcomes = {}, collections.defaultdict(list)
        for line_no, raw in enumerate(path.read_bytes().split(b"\n"), 1):
            if not raw.strip():
                continue
            try:
                v = json.loads(raw)
            except ValueError:
                continue
            if not isinstance(v, dict) or v.get("type") not in CLAUDE_TYPES:
                continue
            m = v.get("message") if isinstance(v.get("message"), dict) else {}
            content = m.get("content") if isinstance(m.get("content"), list) else []
            session = v.get("sessionId") or "claude-session"
            for index, block in enumerate(content):
                if not isinstance(block, dict):
                    continue
                if block.get("type") == "tool_use" and isinstance(block.get("name"), str):
                    key = (session, block.get("id"))
                    if block.get("id") is not None and key in first:
                        continue
                    first[key if block.get("id") is not None else (session, f"pos{line_no}:{index}")] = (line_no, index, v.get("timestamp"), block)
                elif block.get("type") == "tool_result" and isinstance(block.get("tool_use_id"), str):
                    flag = block.get("is_error")
                    outcomes[(session, block["tool_use_id"])].append(flag if isinstance(flag, bool) else None)
        for key, (line_no, index, ts, block) in first.items():
            got = outcomes.get(key, [])
            status = got[0] if got and got[0] is not None and all(g == got[0] for g in got) else None
            calls += 1
            failed += status is True
            succeeded += status is False
            unknown += status is None
            inp = block.get("input")
            with_input += inp is not None and not (isinstance(inp, str) and not inp.strip())
            writes += block["name"] in ("Write", "Edit", "MultiEdit", "NotebookEdit")
            # Exact input equality approximated by order-preserving compact JSON
            # (real logs are compact); reported, not used to tune the product.
            sig = (block["name"], inp.strip() if isinstance(inp, str) else json.dumps(inp, separators=(",", ":"), ensure_ascii=False))
            sessions[key[0]].append((line_no, index, ts, status, sig))
    frm, to = q.parse("2026-10-08T00:00:00Z"), q.parse("2026-10-11T00:00:00Z")
    for session, rows in sessions.items():
        rows = [r for r in sorted(rows) if r[2] is None or frm <= q.parse(r[2]) < to]
        run = []

        def flush():
            stamps = [None if r[2] is None else q.parse(r[2]) for r in run]
            if any(stamps[i] is not None and stamps[i + 2] is not None and (stamps[i + 2] - stamps[i]).total_seconds() <= 600
                   for i in range(len(run) - 2)):
                loops.append(len(run))
            run.clear()

        for r in rows:
            if r[3] is True and run and run[-1][4] == r[4]:
                run.append(r)
            else:
                flush()
                if r[3] is True:
                    run.append(r)
        flush()
    return {"calls": calls, "failed": failed, "succeeded": succeeded, "unknown": unknown,
            "with_input": with_input, "structured_writes": writes, "loops": sorted(loops)}


def private_copy(base):
    src = base / "sources"
    shutil.copytree(SEALED / "sources", src)
    for p in [src, *src.rglob("*")]:
        os.chmod(p, 0o700 if p.is_dir() else 0o600)
    return src


def pi_oracle(src, openrig_db):
    """Independent Pi tool oracle (Q5) from raw JSONL, plus each Pi-linked
    seat's write/edit call times for the intake end check."""
    import sqlite3 as _sq
    import tempfile as _tf
    # Read a separate copy: even a read-only sqlite open of a WAL database
    # writes its -shm, and the ingested private copy must stay unchanged.
    own = Path(_tf.mkdtemp(prefix="pi-oracle-", dir=str(src.parent)))
    for suffix in ("", "-wal", "-shm"):
        if Path(f"{openrig_db}{suffix}").exists():
            shutil.copy2(f"{openrig_db}{suffix}", own / f"openrig.sqlite{suffix}")
    conn = _sq.connect(own / "openrig.sqlite")
    rigs = dict(conn.execute("SELECT id, name FROM rigs").fetchall())
    seats = collections.defaultdict(set)
    for (pl,) in conn.execute("SELECT payload FROM events WHERE type = 'agent.session_identity'"):
        p = json.loads(pl)
        if p.get("runtime") == "pi" and p.get("sessionId") and p.get("sessionName"):
            seats[p["sessionId"]].add((p["sessionName"], rigs.get(p.get("rigId"), p.get("rigId"))))
    conn.close()
    calls = failed = succeeded = unknown = writes = 0
    loops, seat_writes = [], collections.defaultdict(list)
    frm, to = q.parse("2026-10-08T00:00:00Z"), q.parse("2026-10-11T00:00:00Z")
    for path in sorted((src / "pi").rglob("*.jsonl")):
        lines = [json.loads(x) for x in path.read_bytes().split(b"\n") if x.strip()]
        sid = lines[0].get("id") if lines and lines[0].get("type") == "session" else None
        first, order, outcomes = {}, [], collections.defaultdict(list)
        for v in lines:
            if v.get("type") != "message":
                continue
            m = v.get("message") or {}
            if m.get("role") == "toolResult" and isinstance(m.get("toolCallId"), str):
                outcomes[m["toolCallId"]].append(m["isError"] if isinstance(m.get("isError"), bool) else None)
            elif isinstance(m.get("content"), list):
                for b in m["content"]:
                    if isinstance(b, dict) and b.get("type") == "toolCall" and isinstance(b.get("name"), str) and b.get("id") not in first:
                        first[b["id"]] = (v.get("timestamp"), b)
                        order.append(b["id"])
        rows = []
        for cid in order:
            ts, b = first[cid]
            got = outcomes.get(cid, [])
            status = got[0] if got and got[0] is not None and all(g == got[0] for g in got) else None
            calls += 1
            failed += status is True
            succeeded += status is False
            unknown += status is None
            if b["name"] in ("write", "edit"):
                writes += 1
                for seat in seats.get(sid, ()):
                    seat_writes[seat].append(q.parse(ts) if ts else None)
            if ts is None or frm <= q.parse(ts) < to:
                rows.append((q.parse(ts) if ts else None, status, (b["name"], json.dumps(b.get("arguments"), separators=(",", ":"), ensure_ascii=False))))
        run = []

        def flush():
            st = [r[0] for r in run]
            if any(st[i] and st[i + 2] and (st[i + 2] - st[i]).total_seconds() <= 600 for i in range(len(run) - 2)):
                loops.append(len(run))
            run.clear()

        for r in rows:
            if r[1] is True and run and run[-1][2] == r[2]:
                run.append(r)
            else:
                flush()
                if r[1] is True:
                    run.append(r)
        flush()
    return {"calls": calls, "failed": failed, "succeeded": succeeded, "unknown": unknown, "writes": writes,
            "loops": sorted(loops)}, seat_writes


def sealed_side(binary, base, src):
    """Ingests the one private copy `src` (both binaries read the same paths,
    so path-named sessions compare exactly) into a new database under `base`."""
    bad, total = q.verify_sealed_copy(src)
    db, code, run_out, err = q.ingest(binary, base, [("claude", src / "claude"), ("codex", src / "codex"), ("pi", src / "pi"), ("openrig", src / "openrig.sqlite")])
    notes = json.loads(run_out).get("sources", []) if code == 0 else []
    frm, to = "2026-10-08T00:00:00Z", "2026-10-11T00:00:00Z"
    unscoped = json.loads(q.run([binary, "report", "decisions", "--db", db, "--from", frm, "--to", to])[1])["payload"]
    scoped = json.loads(q.run([binary, "report", "decisions", "--db", db, "--rig", "agentflow-dev", "--from", frm, "--to", to])[1])["payload"]
    intake = (q.trace(binary, db, "intake", frm, to) or {}).get("value") or []
    hits, _ = q.decoded(db, ["tool calls not read for codex"])
    return {"src": src, "bad": bad, "exit": code, "err": err[-300:], "unscoped": unscoped, "scoped": scoped, "census": census(db),
            "intake": intake, "codex_warnings": hits["tool calls not read for codex"], "db": db, "notes": notes}


def usage_view(pay):
    return {k: pay.get(k) for k in ("tokens", "token_categories", "unlinked_tokens", "unlinked_token_categories",
                                    "seat_token_categories", "sessions", "untimestamped_count", "untimestamped_tokens_all_time")}


def case_sealed(binary, old, base):
    case = "sealed (private copy)"
    bad, total = q.verify_sealed_copy(SEALED / "sources")
    check(case, f"sealed input verified before ({total} files)", not bad, bad)
    src = private_copy(base)
    new = sealed_side(binary, base / "new", src)
    check(case, "new: private copy matches manifest, ingest exit 0", not new["bad"] and new["exit"] == 0, new["err"])
    orc = oracle(new["src"])
    c = new["census"].get("claude", {})
    print("INFO independent Claude oracle:", json.dumps(orc))
    print("INFO new tool census per harness:", json.dumps(new["census"]))
    check(case, "Claude calls equal independent distinct tool_use ids", c.get("calls") == orc["calls"], {"product": c.get("calls"), "oracle": orc["calls"]})
    check(case, "Claude failed / succeeded / unknown equal the oracle",
          (c.get("failed"), c.get("succeeded"), c.get("unknown")) == (orc["failed"], orc["succeeded"], orc["unknown"]), {"product": c, "oracle": orc})
    check(case, "Claude fingerprinted calls equal calls with an input", c.get("fingerprinted") == orc["with_input"], {"product": c.get("fingerprinted"), "oracle": orc["with_input"]})
    check(case, "Claude structured writes equal the oracle", c.get("structured_writes") == orc["structured_writes"], {"product": c.get("structured_writes"), "oracle": orc["structured_writes"]})
    # Q5 Pi, independently from raw files.
    pio, seat_writes = pi_oracle(new["src"], new["src"] / "openrig.sqlite")
    pc = new["census"].get("pi", {})
    print("INFO independent Pi oracle:", json.dumps(pio))
    check(case, "Pi calls equal independent distinct toolCall ids", pc.get("calls") == pio["calls"], {"product": pc.get("calls"), "oracle": pio["calls"]})
    check(case, "Pi failed / succeeded / unknown equal the oracle", (pc.get("failed"), pc.get("succeeded"), pc.get("unknown")) == (pio["failed"], pio["succeeded"], pio["unknown"]), {"product": pc, "oracle": pio})
    check(case, "Pi fingerprinted calls equal calls", pc.get("fingerprinted") == pio["calls"], {"product": pc.get("fingerprinted")})
    check(case, "Pi structured writes (write/edit) equal the oracle", pc.get("structured_writes") == pio["writes"], {"product": pc.get("structured_writes"), "oracle": pio["writes"]})
    # Q6 Codex: not read, never a measured zero.
    codex_files = len(list((new["src"] / "codex").rglob("*.jsonl")))
    print("INFO census per harness: claude", c.get("calls"), "| pi", pc.get("calls"), "| codex NOT READ (", new["codex_warnings"], "file warnings )")
    check(case, f"Codex: one 'tool calls not read for codex' warning per file ({codex_files})", new["codex_warnings"] == codex_files, new["codex_warnings"])
    check(case, "Codex: no tool facts stored", "codex" not in new["census"], new["census"])
    check(case, "Codex run note says tool_calls=not read", any(n.startswith("codex ") and n.endswith(" tool_calls=not read") for n in new["notes"]), new["notes"])
    new_loops = sorted(l["count"] for l in new["unscoped"]["tool_loops"])
    want_loops = sorted(orc["loops"] + pio["loops"])
    print("INFO real loops (unscoped, 3 days): product", new_loops, "independent oracle Claude", orc["loops"], "Pi", pio["loops"], "(oracle input equality is an approximation)")
    check(case, "real loops equal the independent oracle (Claude + Pi)", new_loops == want_loops, {"product": new_loops, "oracle": want_loops})
    # QA's four real intake cases (qa/2abeb8f/intake-impact.json): independent end and tokens.
    impact_path = REPO / ".agent-runs/inputs/r1-hotfix-v0.1.1/qa-intake-impact-2abeb8f.json"
    check(case, "QA intake-impact input pinned", impact_path.exists(), str(impact_path))
    impact = json.loads(impact_path.read_text()) if impact_path.exists() else []
    print("INFO QA intake-impact sha256", q.sha(impact_path) if impact_path.exists() else None)
    for case_row in [r for r in impact if not r.get("ok")]:
        hit = [r for r in new["unscoped"]["intakes"] if r["qitem_id"] == case_row["qitem_id"]]
        got = (hit[0]["tokens"], hit[0]["write"]) if len(hit) == 1 else None
        ok = got is not None and got[0] == case_row["expected_tokens"] and q.parse(got[1]) == q.parse(case_row["expected_end"])
        check(case, f"QA real intake {case_row['qitem_id']}: {case_row['expected_tokens']} tokens, end {case_row['expected_end']}", ok, {"got": got})
    deltas = {"new": {"census": new["census"], "oracle": orc, "loops": new_loops,
                      "intake_scoped": q.intake_summary(new["scoped"]), "intake_unscoped": q.intake_summary(new["unscoped"])}}
    if old:
        o = sealed_side(old, base / "old", src)
        same = usage_view(o["unscoped"]) == usage_view(new["unscoped"]) and usage_view(o["scoped"]) == usage_view(new["scoped"])
        check(case, "usage and token totals identical to the old candidate (unscoped and agentflow-dev)", same,
              {"old_tokens": o["unscoped"].get("tokens"), "new_tokens": new["unscoped"].get("tokens")})
        deltas["old"] = {"census": o["census"], "loops": sorted(l["count"] for l in o["unscoped"]["tool_loops"]),
                         "intake_scoped": q.intake_summary(o["scoped"]), "intake_unscoped": q.intake_summary(o["unscoped"])}
        print("INFO old tool census per harness:", json.dumps(o["census"]))
        # Q5 intake: for every receiver with a Pi-linked session, the new end is
        # the earlier of the old end and its first Pi write/edit at or after
        # the claim (independent times from raw files); no_data rows with no
        # classifiable end become estimates when such a write exists.
        writes = collections.defaultdict(list)
        for (seat, rig), times in seat_writes.items():
            writes[seat if "@" in seat else f"{seat}@{rig}"].extend(t for t in times if t)
        old_rows = {(r["qitem_id"], r["to"]): r for r in o["intake"]}
        moved = converted = agree = considered = 0
        mismatch = []
        for r in new["intake"]:
            key = (r["qitem_id"], r["to"])
            prev = old_rows.get(key)
            if prev is None or r["to"] not in writes or not prev.get("claim"):
                continue
            start = q.parse(prev["claim"])
            first_write = min((t for t in writes[r["to"]] if t >= start), default=None)
            if prev["status"] == "estimate" and prev.get("write"):
                considered += 1
                old_end = q.parse(prev["write"])
                expect = min(old_end, first_write) if first_write else old_end
                moved += expect < old_end
            elif prev["status"] == "no_data" and prev.get("reason") == "no classifiable end" and first_write:
                considered += 1
                converted += 1
                expect = first_write
            else:
                continue
            got = q.parse(r["write"]) if r.get("write") else None
            if got == expect:
                agree += 1
            else:
                mismatch.append({"qitem": r["qitem_id"], "to": r["to"], "expect": str(expect), "got": r.get("write"), "status": r["status"]})
        print(f"INFO Pi intake: considered {considered}, ends moved earlier {moved}, no-data converted {converted}, independent end agrees {agree}")
        check(case, "Pi-receiver intake ends equal the independent first write/edit rule", not mismatch, mismatch[:5])
        deltas["pi_intake"] = {"considered": considered, "moved": moved, "converted": converted, "agree": agree}
        print("INFO old loops:", deltas["old"]["loops"])
        print("INFO intake old scoped:", json.dumps(deltas["old"]["intake_scoped"]["statuses"]), "tokens", deltas["old"]["intake_scoped"]["estimate_tokens"])
        print("INFO intake old unscoped:", json.dumps(deltas["old"]["intake_unscoped"]["statuses"]), "tokens", deltas["old"]["intake_unscoped"]["estimate_tokens"])
        print("INFO medians old scoped:", json.dumps(deltas["old"]["intake_scoped"]["intake_medians"]))
    print("INFO intake new scoped:", json.dumps(deltas["new"]["intake_scoped"]["statuses"]), "tokens", deltas["new"]["intake_scoped"]["estimate_tokens"])
    print("INFO intake new unscoped:", json.dumps(deltas["new"]["intake_unscoped"]["statuses"]), "tokens", deltas["new"]["intake_unscoped"]["estimate_tokens"])
    print("INFO medians new scoped:", json.dumps(deltas["new"]["intake_scoped"]["intake_medians"]))
    bad, _ = q.verify_sealed_copy(src)
    check(case, "private copy unchanged after every ingest", not bad, bad)
    bad, _ = q.verify_sealed_copy(SEALED / "sources")
    check(case, "sealed input verified after", not bad, bad)
    (base / "sealed-deltas.json").write_text(json.dumps(deltas, indent=1, sort_keys=True))


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
    print("binary sha256", q.sha(binary), "| old binary sha256", q.sha(old) if old else None)
    fns = (case_loops, case_append, case_intake, case_privacy, case_pi, case_pi_intake, case_codex)
    for fn in fns:
        fn(binary, base / "new" / fn.__name__, "new")
    case_wal_privacy(binary, base / "new" / "wal-privacy", "new")
    gated = dict(results)
    negative = {}
    if old:
        start = len(results)
        for fn in fns:
            fn(old, base / "old" / fn.__name__, "old")
        negative = dict(list(results.items())[start:])
    if not args.skip_sealed:
        case_sealed(binary, old, base / "sealed")
        gated["sealed (private copy)"] = results["sealed (private copy)"]
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
