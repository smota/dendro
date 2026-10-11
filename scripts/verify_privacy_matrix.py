#!/usr/bin/env python3
"""Privacy matrix proof (S5) through the real CLI.

For each supported credential family, one frozen ingest of synthetic Claude,
Codex, Pi and OpenRig sources whose every read text field carries a tagged
canary, plus content canaries in fields Dendro must never store. Checks:
- dynamic schema: every VARCHAR column listed by the database itself, decoded
  by the dev-only decoded_scan example; the list equals TEXT_COLUMNS and every
  column has a lineage class; source-injected columns are non-vacuous;
- the staged DuckDB WAL captured non-empty BEFORE checkpoint;
- the raw snapshot holds canaries only while the run is active and is gone after;
- all surfaces: every report kind and trace id (unscoped and linked rig), CLI
  error paths, TMPDIR, output directory bytes (db, WAL, key, lock);
- ordinary identifiers stay visible and distinct secret identifiers stay apart.

Usage:
  python3 scripts/verify_privacy_matrix.py --binary target/debug/dendro \\
      --scratch .agent-runs/proof/privacy-final --dynamic-schema --all-surfaces
"""
import argparse
import json
import os
import re
import shutil
import sqlite3
import subprocess
import sys
import time
from pathlib import Path

FAMILIES = ["sk-", "sk-ant-", "ghp_", "gho_", "github_pat_", "xai-", "AKIA", "Bearer ", "xoxb-", "xoxp-"]
REPORTS = ["links", "activity", "handoffs", "tokens", "decisions"]
TRACES = ["m1", "m2", "m3", "m4", "tokens", "intake", "medians", "activity"]
FROM, TO = "2026-10-09T00:00:00Z", "2026-10-10T00:00:00Z"
CONTENT = "CONTENTCANARY"
results = []

# Lineage class of every text column (see the ledger column map).
LINEAGE = {
    "source": ["source_files.display_path", "source_files.native_session", "usage_facts.native_session", "usage_facts.message_id",
               "tool_facts.native_session", "tool_facts.tool_name", "identity_facts.harness",
               "identity_facts.native_session", "identity_facts.rig", "identity_facts.seat",
               "activity_facts.seat", "queue_facts.qitem_id", "queue_facts.source_seat",
               "queue_facts.dest_seat", "queue_facts.closure_reason", "queue_facts.closure_target",
               "queue_facts.chain_id", "seat_bounds.seat", "destination_snapshots.qitem_id",
               "destination_snapshots.destination", "destination_snapshots.snapshot_cutoff",
               "destination_snapshots.missions", "seat_roles.seat", "seat_roles.evidence"],
    "generated": ["dendro_meta.key_id", "source_files.source_id", "source_files.prefix_sha256",
                  "codex_counters.source_id", "usage_facts.source_key", "usage_facts.source_id",
                  "usage_facts.predecessor_source_key", "tool_facts.source_key", "tool_facts.fingerprint",
                  "tool_facts.source_id", "identity_facts.source_key", "activity_facts.source_key",
                  "queue_facts.source_key", "destination_snapshots.source_key"],
    "vocabulary": ["source_files.harness", "usage_facts.harness", "usage_facts.predecessor_reason",
                   "tool_facts.harness", "activity_facts.state", "queue_facts.state", "warnings.reason",
                   "seat_roles.role"],
    "invariant": ["queue_facts.missions", "skill_facts.source_key", "skill_facts.seat", "skill_facts.skill",
                  "warnings.path"],
}


def check(name, ok, detail=""):
    results.append({"check": name, "ok": bool(ok), "detail": str(detail)[:300]})
    print(("PASS" if ok else "FAIL"), name, "" if ok else detail)


def text_columns(repo):
    source = (repo / "crates/dendro-core/src/privacy.rs").read_text()
    block = source[source.index("pub const TEXT_COLUMNS"):]
    block = block[:block.index("];")]
    return sorted(re.findall(r'"([a-z_]+\.[a-z_0-9]+)"', block))


def sources(src, c, rows):
    s1, s2 = c("seat1"), c("seat2")
    d = src / "claude" / c("dir")
    d.mkdir(parents=True)
    line = lambda session, msg: json.dumps({
        "type": "assistant", "sessionId": session, "timestamp": "2026-10-09T01:00:00Z",
        "message": {"id": msg, "usage": {"input_tokens": 5, "output_tokens": 1, "cache_read_input_tokens": 0,
                    "cache_creation_input_tokens": 0},
                    "content": [{"type": "text", "text": CONTENT + "-message"},
                                {"type": "tool_use", "name": c("tname"), "input": c("tinput") + CONTENT, "is_error": True}]}})
    (d / "s.jsonl").write_text("\n".join([line(c("csess"), c("cmsg")), line(c("csess2"), "m-plain"),
                                          line("plain-session", "m-plain-2")]) + "\n")
    d = src / "codex" / c("xdir")
    d.mkdir(parents=True)
    (d / "s.jsonl").write_text(json.dumps({"type": "session_meta", "timestamp": "2026-10-09T00:00:00Z", "payload": {"id": c("xhdr")}}) + "\n" + json.dumps({"type": "event_msg", "timestamp": "2026-10-09T01:00:00Z", "payload": {
        "type": "token_count", "session_id": c("xsess"), "info": {"total_token_usage": {
            "input_tokens": 9, "cached_input_tokens": 1, "cache_write_input_tokens": 0, "output_tokens": 2,
            "reasoning_output_tokens": 0}}}}) + "\n")
    d = src / "pi" / c("pdir")
    d.mkdir(parents=True)
    (d / "s.jsonl").write_text(json.dumps({"type": "session", "version": 3, "id": c("phdr"), "timestamp": "2026-10-09T00:00:00Z"}) + "\n" + json.dumps({"type": "message", "id": c("pmsg"), "timestamp": "2026-10-09T01:00:00Z",
                                           "message": {"role": "assistant", "content": CONTENT,
                                                       "usage": {"input": 3, "output": 1, "cacheRead": 0, "cacheWrite": 0}}}) + "\n")
    db = src / "openrig.sqlite"
    con = sqlite3.connect(db)
    con.execute("PRAGMA journal_mode=WAL")
    con.executescript("""CREATE TABLE events (seq INTEGER, type TEXT, payload TEXT, created_at TEXT);
        CREATE TABLE rigs (id TEXT, name TEXT);
        CREATE TABLE queue_transitions (transition_id INTEGER, qitem_id TEXT, ts TEXT, state TEXT, actor_session TEXT,
          closure_reason TEXT, closure_target TEXT, transition_note TEXT);
        CREATE TABLE queue_items (qitem_id TEXT, destination_session TEXT, ts_updated TEXT, chain_of_record TEXT,
          handed_off_from TEXT, tags TEXT, body TEXT);
        CREATE TABLE snapshots (data TEXT);""")
    con.executemany("INSERT INTO rigs VALUES (?,?)", [("rid", "rig-plain"), (c("rigid"), c("rigname"))])
    q1, q2 = c("q1"), c("q2")
    events = [
        ("agent.session_identity", {"sessionId": c("csess"), "runtime": "claude-code", "sessionName": s1, "rigId": "rid"}),
        ("agent.session_identity", {"sessionId": c("csess2"), "runtime": "claude-code", "sessionName": s2, "rigId": "rid"}),
        ("agent.session_identity", {"sessionId": c("isess"), "runtime": c("runtime"), "sessionName": c("iseat"), "rigId": c("rigid")}),
        ("seat.activity_changed", {"sessionName": s1, "state": "running", "at": "2026-10-09T00:40:00Z"}),
        ("seat.activity_changed", {"sessionName": s2, "state": c("state"), "at": "2026-10-09T00:41:00Z"}),
        ("node.launched", {"rigId": "rid", "nodeId": c("node"), "sessionName": s1}),
        ("queue.created", {"qitemId": q1, "sourceSession": s2, "destinationSession": s1, "summary": CONTENT}),
        ("queue.claimed", {"qitemId": q1, "destinationSession": s1, "claimedAt": "2026-10-09T00:50:00Z"}),
        ("queue.created", {"qitemId": q2, "sourceSession": s1, "destinationSession": s2}),
        ("qitem.fallback_routed", {"qitemId": c("q3"), "originalDestination": s1, "rerouteDestination": c("reroute"), "reason": CONTENT}),
    ]
    # Padding rows make the run long enough to observe the staged WAL before checkpoint.
    events += [("seat.activity_changed", {"sessionName": s1, "state": "idle", "at": "2026-10-09T00:42:00Z", "pad": i})
               for i in range(rows)]
    con.executemany("INSERT INTO events VALUES (?,?,?,'2026-10-09 00:30:00')",
                    [(i + 1, k, json.dumps(p)) for i, (k, p) in enumerate(events)])
    con.executemany("INSERT INTO queue_transitions VALUES (?,?,?,?,?,?,?,?)", [
        (1, q1, "2026-10-09T01:10:00.000Z", "handed-off", s1, "handed_off_to", s2, c("note") + CONTENT),
        (2, q2, "2026-10-09T01:11:00.000Z", "done", s2, c("reason"), c("target"), CONTENT),
    ])
    con.executemany("INSERT INTO queue_items VALUES (?,?,?,?,?,?,?)", [
        (q1, s1, c("cutoff"), "[]", None, json.dumps(["mission:" + c("mission")]), CONTENT),
        (q2, s2, "2026-10-09T01:11:00.000Z", json.dumps([q1]), q1, json.dumps(["mission:plain-mission"]), CONTENT),
    ])
    con.execute("INSERT INTO snapshots VALUES (?)", (json.dumps({
        "nodes": [{"id": "n1", "logicalId": "plain.logical", "agentRef": c("agentref"), "resolvedSpecName": c("spec")},
                  {"id": "n2", "logicalId": c("logical"), "agentRef": "local:agents/agentflow", "resolvedSpecName": "agentflow-seat"}],
        "sessions": [{"nodeId": "n1", "sessionName": s1}, {"nodeId": "n2", "sessionName": s2}]}),))
    con.commit()
    con.close()
    return [f"claude={src / 'claude'}", f"codex={src / 'codex'}", f"pi={src / 'pi'}", f"openrig={db}"]


def scan_bytes(path, needles):
    hits = []
    for f in Path(path).rglob("*"):
        if f.is_file():
            data = f.read_bytes()
            hits += [f"{f.name}:{n}" for n in needles if n.encode() in data]
    return hits


def decoded(repo, db, needles):
    p = subprocess.run(["cargo", "run", "-q", "-p", "dendro-connectors", "--example", "decoded_scan", "--", str(db), *needles],
                       cwd=repo, capture_output=True, text=True, timeout=600)
    if p.returncode != 0:
        sys.exit(f"decoded_scan failed: {p.stderr[-300:]}")
    return json.loads(p.stdout)["columns"]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", required=True)
    parser.add_argument("--scratch", required=True)
    parser.add_argument("--dynamic-schema", action="store_true", required=True)
    parser.add_argument("--all-surfaces", action="store_true", required=True)
    args = parser.parse_args()
    repo = Path(__file__).resolve().parent.parent
    binary = str(Path(args.binary).resolve())
    root = Path(args.scratch).resolve()
    shutil.rmtree(root, ignore_errors=True)
    root.mkdir(parents=True)
    expected = text_columns(repo)
    classified = sorted(c for cols in LINEAGE.values() for c in cols)
    check("lineage classifies every TEXT_COLUMNS entry exactly once", classified == expected,
          sorted(set(expected) ^ set(classified)))
    populated = set()
    for index, marker in enumerate(FAMILIES):
        tag = f"ZZ{index}x"
        c = lambda field, m=marker, t=tag: f"{m}{t}{field}"
        base = root / f"family{index}"
        src, out, tmp = base / "src", base / "out", base / "tmp"
        for d in (src, out, tmp):
            d.mkdir(parents=True)
        out.chmod(0o700)
        env = dict(os.environ, TMPDIR=str(tmp))
        roots = sources(src, c, rows=400)
        db = out / "analytics.duckdb"
        cmd = [binary, "run", "--db", str(db), "--frozen"]
        for r in roots:
            cmd += ["--root", r]
        p = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, env=env)
        wal, raw_seen = b"", False
        while p.poll() is None:
            for stage in out.glob(".dendro-stage-*"):
                snap = stage / "snapshot" / "openrig.sqlite"
                try:
                    if snap.exists() and tag.encode() in snap.read_bytes():
                        raw_seen = True
                    w = stage / "analytics.duckdb.wal"
                    if w.exists() and w.stat().st_size > len(wal):
                        wal = w.read_bytes()
                except OSError:
                    pass
            time.sleep(0.01)
        stdout, stderr = p.communicate()
        check(f"{marker!r}: run exit 0", p.returncode == 0, stderr[-200:])
        check(f"{marker!r}: staged WAL captured non-empty before checkpoint", len(wal) > 0, len(wal))
        check(f"{marker!r}: pre-checkpoint WAL free of canaries", tag.encode() not in wal and CONTENT.encode() not in wal)
        check(f"{marker!r}: raw snapshot held the canary while active (non-vacuous)", raw_seen)
        check(f"{marker!r}: no stage left after the run", not list(out.glob(".dendro-stage-*")))
        surfaces = [stdout, stderr]
        for rig in (None, "rig-plain"):
            sel = ["--rig", rig] if rig else []
            for kind in REPORTS:
                r = subprocess.run([binary, "report", kind, "--db", str(db), "--from", FROM, "--to", TO, *sel],
                                   capture_output=True, text=True, env=env)
                check(f"{marker!r}: report {kind} {rig} exit 0", r.returncode == 0, r.stderr[-200:])
                surfaces += [r.stdout, r.stderr]
            for metric in TRACES:
                r = subprocess.run([binary, "trace", "--metric-id", metric, "--db", str(db), "--from", FROM, "--to", TO, *sel],
                                   capture_output=True, text=True, env=env)
                check(f"{marker!r}: trace {metric} {rig} exit 0", r.returncode == 0, r.stderr[-200:])
                surfaces += [r.stdout, r.stderr]
        linked = json.loads(subprocess.run([binary, "report", "decisions", "--db", str(db), "--from", FROM, "--to", TO,
                                            "--rig", "rig-plain"], capture_output=True, text=True, env=env).stdout)["payload"]
        check(f"{marker!r}: linked-rig outputs non-empty",
              linked["m1_by_seat"] and linked["handoff_members"] and linked["activity"] and linked["tokens"] > 0)
        for bad in (["report", c("kind"), "--db", str(db)], ["trace", "--metric-id", c("metric"), "--db", str(db)],
                    ["report", "tokens", "--db", str(db), "--from", c("from")], ["run", "--db", str(out / c("dbname")), "--root", f"openrig={c('rootpath')}"],
                    ["run", "--root", f"{c('rootname')}=/nonexistent", "--db", str(out / "x.duckdb")], [f"--{c('flag')}"]):
            r = subprocess.run([binary, *bad], capture_output=True, text=True, env=env)
            surfaces += [r.stdout, r.stderr]
        joined = "\n".join(surfaces)
        check(f"{marker!r}: no canary in any report, trace or CLI error surface",
              tag not in joined and CONTENT not in joined, re.findall(tag + r"\w+", joined)[:5])
        check(f"{marker!r}: ordinary values stay visible", "rig-plain" in joined and "plain-mission" in joined)
        links = json.loads(subprocess.run([binary, "report", "links", "--db", str(db)], capture_output=True, text=True,
                                          env=env).stdout)["payload"]["links"]
        secret_sessions = {row.get("native_session") for row in links if "[redacted]" in str(row.get("native_session"))}
        check(f"{marker!r}: distinct secret sessions stay distinct", len(secret_sessions) >= 3, len(secret_sessions))
        check(f"{marker!r}: TMPDIR, output dir and stage bytes free of canaries",
              scan_bytes(tmp, [tag, CONTENT]) == [] and scan_bytes(out, [tag, CONTENT]) == [],
              scan_bytes(tmp, [tag, CONTENT]) + scan_bytes(out, [tag, CONTENT]))
        columns = decoded(repo, db, [tag, CONTENT, "[redacted]"])
        names = sorted(col["column"] for col in columns)
        check(f"{marker!r}: schema VARCHAR columns = TEXT_COLUMNS ({len(names)})", names == expected,
              sorted(set(names) ^ set(expected)))
        check(f"{marker!r}: decoded values free of family canary", sum(col["hits"][tag] for col in columns) == 0)
        check(f"{marker!r}: decoded values free of content canary", sum(col["hits"][CONTENT] for col in columns) == 0)
        check(f"{marker!r}: decoded redacted control non-empty", sum(col["hits"]["[redacted]"] for col in columns) > 0)
        populated |= {col["column"] for col in columns if col["values"] > 0}
    empty_source = sorted(set(LINEAGE["source"]) - populated)
    check("every source-injected column held values in the sweep (non-vacuous)", empty_source == [], empty_source)
    failed = [r for r in results if not r["ok"]]
    (root / "results.json").write_text(json.dumps({"checks": len(results), "failed": len(failed),
                                                    "columns": expected, "lineage": LINEAGE, "results": results}, indent=1))
    print(f"RESULT {len(results) - len(failed)}/{len(results)} checks passed over {len(FAMILIES)} families and {len(expected)} columns")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
