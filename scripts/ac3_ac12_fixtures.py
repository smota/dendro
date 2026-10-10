#!/usr/bin/env python3
"""AC3-AC12 synthetic CLI fixtures. No live logs."""
import hashlib, json, os, sqlite3, subprocess
from pathlib import Path

BIN = "/home/sam/code/dendro/.worktrees/work/dendro-r1/target/debug/dendro"
ROOT = Path("/tmp/dendro-ac-pack")
FROM = "2026-10-09T00:00:00Z"
TO = "2026-10-09T00:01:00Z"
T = {
    0: "2026-10-09T00:00:00Z",
    5: "2026-10-09T00:00:05Z",
    6: "2026-10-09T00:00:06Z",
    10: "2026-10-09T00:00:10Z",
    20: "2026-10-09T00:00:20Z",
    30: "2026-10-09T00:00:30Z",
    40: "2026-10-09T00:00:40Z",
}
rows = []

def check(name, ok, expected, actual):
    rows.append({"name": name, "ok": bool(ok), "expected": expected, "actual": actual})
    print(("PASS" if ok else "FAIL"), name)

def run(args):
    proc = subprocess.run([BIN, *args], text=True, capture_output=True)
    return proc.returncode, proc.stdout, proc.stderr

def report(db, kind, **extra):
    args = ["report", kind, "--db", str(db), "--from", FROM, "--to", TO]
    for key, value in extra.items():
        args.extend(["--" + key, value])
    return run(args)

def main():
    import shutil
    if ROOT.exists():
        shutil.rmtree(ROOT)
    src = ROOT / "src"
    (src / "claude").mkdir(parents=True)
    (src / "codex").mkdir()
    (src / "pi").mkdir()
    secret_a, secret_b = "sk-ant-SECRETA", "sk-ant-SECRETB"
    canary = "CANARY-CONTENT-111"
    (src / "claude" / "s.jsonl").write_text("\n".join([
        json.dumps({"type": "assistant", "timestamp": T[0], "sessionId": "s-linked", "message": {"id": "m1", "usage": {"input_tokens": 2, "output_tokens": 3, "cache_read_input_tokens": 4, "cache_creation_input_tokens": 5, "cache_creation": {"ephemeral_5m_input_tokens": 5}}}}),
        json.dumps({"type": "assistant", "timestamp": T[5], "sessionId": "s-linked", "message": {"id": "m1", "usage": {"input_tokens": 9, "output_tokens": 9, "cache_read_input_tokens": 9, "cache_creation_input_tokens": 9}}}),
        json.dumps({"type": "assistant", "timestamp": T[6], "sessionId": "s-unlinked", "message": {"id": "u1", "content": canary, "usage": {"input_tokens": 1, "output_tokens": 0, "cache_read_input_tokens": 0, "cache_creation_input_tokens": 0}}}),
        json.dumps({"type": "assistant", "timestamp": T[10], "sessionId": f"sess-{secret_a}", "message": {"id": "a", "content": canary, "usage": {"input_tokens": 1, "output_tokens": 0, "cache_read_input_tokens": 0, "cache_creation_input_tokens": 0}, "content": [{"type": "tool_use", "name": "Read", "input": " same ", "is_error": True}]}}),
        json.dumps({"type": "assistant", "timestamp": "2026-10-09T00:00:11Z", "sessionId": f"sess-{secret_a}", "message": {"id": "b", "content": [{"type": "tool_use", "name": "Read", "input": " same ", "is_error": True}]}}),
        json.dumps({"type": "assistant", "timestamp": "2026-10-09T00:00:12Z", "sessionId": f"sess-{secret_a}", "message": {"id": "c", "content": [{"type": "tool_use", "name": "Read", "input": " same ", "is_error": True}]}}),
        json.dumps({"type": "assistant", "timestamp": T[20], "sessionId": f"sess-{secret_b}", "message": {"id": "d", "usage": {"input_tokens": 4, "output_tokens": 0, "cache_read_input_tokens": 0, "cache_creation_input_tokens": 0}}}),
        json.dumps({"type": "assistant", "timestamp": T[10], "sessionId": "s-b", "message": {"id": "early", "usage": {"input_tokens": 1, "output_tokens": 0, "cache_read_input_tokens": 0, "cache_creation_input_tokens": 0}}}),
        json.dumps({"type": "assistant", "timestamp": "2026-10-09T00:00:15Z", "sessionId": "s-b", "message": {"id": "early-write", "content": [{"type": "tool_use", "name": "Write", "input": "early", "is_error": False}]}}),
        json.dumps({"type": "assistant", "timestamp": T[30], "sessionId": "s-b", "message": {"id": "w0", "usage": {"input_tokens": 2, "output_tokens": 0, "cache_read_input_tokens": 0, "cache_creation_input_tokens": 0}}}),
        json.dumps({"type": "assistant", "timestamp": "2026-10-09T00:00:35Z", "sessionId": "s-b", "message": {"id": "w5", "usage": {"input_tokens": 3, "output_tokens": 0, "cache_read_input_tokens": 0, "cache_creation_input_tokens": 0}}}),
        json.dumps({"type": "assistant", "timestamp": T[40], "sessionId": "s-b", "message": {"id": "w10", "content": [{"type": "tool_use", "name": "Write", "input": "file", "is_error": False}], "usage": {"input_tokens": 9, "output_tokens": 0, "cache_read_input_tokens": 0, "cache_creation_input_tokens": 0}}}),
        json.dumps({"type": "assistant", "timestamp": T[10], "sessionId": "s-b", "message": {"id": "bash", "content": [{"type": "tool_use", "name": "Bash", "input": "echo", "is_error": False}]}}),
        json.dumps({"type": "assistant", "timestamp": T[10], "sessionId": "s-b", "message": {"id": "secret-tool", "content": [{"type": "tool_use", "name": "sk-ant-TOOLNAME", "input": "echo", "is_error": False}]}}),
        "not-json",
    ]) + "\npartial")
    (src / "codex" / "c.jsonl").write_text("\n".join([
        json.dumps({"type": "token_usage_record", "timestamp": T[0], "payload": {"usage": {"input_tokens": 999}}}),
        json.dumps({"type": "event_msg", "timestamp": T[0], "payload": {"type": "token_count", "session_id": "c1", "info": {"total_token_usage": {"input_tokens": 10, "cached_input_tokens": 0, "cache_write_input_tokens": 0, "output_tokens": 0, "reasoning_output_tokens": 0, "total_tokens": 10}}}}),
        json.dumps({"type": "event_msg", "timestamp": T[10], "payload": {"type": "token_count", "session_id": "c1", "info": {"total_token_usage": {"input_tokens": 30, "cached_input_tokens": 0, "cache_write_input_tokens": 0, "output_tokens": 0, "reasoning_output_tokens": 0, "total_tokens": 30}}}}),
        json.dumps({"type": "event_msg", "timestamp": T[20], "payload": {"type": "token_count", "session_id": "c1", "info": {"total_token_usage": {"input_tokens": 5, "cached_input_tokens": 0, "cache_write_input_tokens": 0, "output_tokens": 0, "reasoning_output_tokens": 0, "total_tokens": 5}}}}),
    ]) + "\n")
    (src / "pi" / "p.jsonl").write_text(json.dumps({"type": "message", "timestamp": T[0], "id": "p1", "message": {"role": "assistant", "usage": {"input": 1, "output": 2, "cacheRead": 3, "cacheWrite": 4, "reasoning": 9, "totalTokens": 99, "cost": {"total": 1.5}}}}) + "\n")
    db = src / "openrig.sqlite"
    conn = sqlite3.connect(db)
    conn.execute("CREATE TABLE rigs (id TEXT, name TEXT)")
    conn.execute("INSERT INTO rigs VALUES ('r1','rig-one')")
    conn.execute("CREATE TABLE events (seq INTEGER, type TEXT, payload TEXT, created_at TEXT)")
    conn.execute("INSERT INTO events VALUES (1,'agent.session_identity',? ,?)", (json.dumps({"sessionId": "s-linked", "runtime": "claude-code", "sessionName": "seat-a", "rigId": "r1"}), FROM))
    conn.execute("INSERT INTO events VALUES (2,'agent.session_identity',? ,?)", (json.dumps({"sessionId": f"sess-{secret_a}", "runtime": "claude-code", "sessionName": f"seat-{secret_a}", "rigId": "r1"}), FROM))
    conn.execute("INSERT INTO events VALUES (3,'agent.session_identity',? ,?)", (json.dumps({"sessionId": f"sess-{secret_b}", "runtime": "claude-code", "sessionName": f"seat-{secret_b}", "rigId": "r1"}), FROM))
    conn.execute("INSERT INTO events VALUES (6,'agent.session_identity',? ,?)", (json.dumps({"sessionId": "s-b", "runtime": "claude-code", "sessionName": "seat-b", "rigId": "r1"}), FROM))
    conn.execute("INSERT INTO events VALUES (12,'agent.session_identity',? ,?)", (json.dumps({"sessionId": "s-collide", "runtime": "claude-code", "sessionName": "seat-left", "rigId": "r1"}), FROM))
    conn.execute("INSERT INTO events VALUES (13,'agent.session_identity',? ,?)", (json.dumps({"sessionId": "s-collide", "runtime": "claude-code", "sessionName": "seat-right", "rigId": "r1"}), FROM))
    conn.execute("INSERT INTO events VALUES (4,'seat.activity_changed',? ,?)", (json.dumps({"sessionName": "seat-a", "state": "idle", "at": T[0]}), T[0]))
    conn.execute("INSERT INTO events VALUES (5,'seat.activity_changed',? ,?)", (json.dumps({"sessionName": "seat-old", "state": "running", "at": "2026-10-08T00:00:00Z"}), "2026-10-08T00:00:00Z"))
    conn.execute("INSERT INTO events VALUES (7,'seat.activity_changed',? ,?)", (json.dumps({"sessionName": "seat-carry", "state": "idle", "at": T[0]}), T[0]))
    conn.execute("INSERT INTO events VALUES (8,'seat.activity_changed',? ,?)", (json.dumps({"sessionName": "seat-cross", "state": "idle", "at": T[0]}), T[0]))
    conn.execute("INSERT INTO events VALUES (9,'seat.activity_changed',? ,?)", (json.dumps({"sessionName": "seat-gap", "state": "running", "at": T[0]}), T[0]))
    conn.execute("INSERT INTO events VALUES (10,'seat.activity_changed',? ,?)", (json.dumps({"sessionName": "seat-gap", "at": T[10]}), T[10]))
    conn.execute("INSERT INTO events VALUES (11,'seat.activity_changed',? ,?)", (json.dumps({"sessionName": "seat-secret", "state": "sk-ant-FAKESTATE", "at": T[0]}), T[0]))
    conn.execute("INSERT INTO events VALUES (14,'node.launched',? ,?)", (json.dumps({"sessionName": "seat-quiet"}), T[0]))
    conn.execute("INSERT INTO events VALUES (15,'queue.created',? ,?)", (json.dumps({"qitemId": "hist-item", "sourceSession": "seat-src", "destinationSession": "seat-hist"}), T[0]))
    conn.execute("INSERT INTO events VALUES (16,'queue.claimed',? ,?)", (json.dumps({"qitemId": "hist-item", "destinationSession": "seat-hist", "claimedAt": T[5]}), T[5]))
    conn.execute("INSERT INTO events VALUES (17,'queue.created',? ,?)", (json.dumps({"qitemId": "hist-change", "sourceSession": "seat-src", "destinationSession": "seat-early"}), T[0]))
    conn.execute("INSERT INTO events VALUES (18,'queue.claimed',? ,?)", (json.dumps({"qitemId": "hist-change", "destinationSession": "seat-late", "claimedAt": T[20]}), T[20]))
    conn.execute("""CREATE TABLE queue_transitions (transition_id INTEGER, qitem_id TEXT, ts TEXT, state TEXT, actor_session TEXT, closure_reason TEXT, closure_target TEXT, transition_note TEXT)""")
    conn.executemany("INSERT INTO queue_transitions VALUES (?,?,?,?,?,?,?,?)", [
        (1, "demand", T[10], "pending", "seat-a", None, "seat-a", "[]"),
        (13, "carry", "2026-10-08T23:59:00Z", "pending", "seat-carry", None, "seat-carry", "[]"),
        (14, "cross", "2026-10-08T23:59:00Z", "pending", "seat-cross", None, "seat-cross", "[]"),
        (15, "cross", T[10], "done", "seat-cross", None, "seat-cross", "[]"),
        (2, "demand", T[20], "done", "seat-a", None, "seat-a", "[]"),
        (3, "alpha", T[0], "in-progress", "seat-a", None, "seat-a", '["alpha"]'),
        (8, "alpha", T[0], "done", "seat-a", None, "seat-a", "[]"),
        (4, "chain", T[30], "handed-off", "seat-a", "handed_off_to", "seat-b", "[]"),
        (5, "chain", T[40], "handed-off", "seat-b", "handed_off_to", "seat-a", "[]"),
        (9, "chain", T[30], "in-progress", "seat-b", None, "seat-b", "[]"),
        (10, "wait2", T[0], "handed-off", "seat-a", "handed_off_to", "seat-b", "[]"),
        (11, "wait2", T[10], "in-progress", "seat-b", None, "seat-b", "[]"),
        (12, "wait2", "2026-10-09T00:00:15Z", "done", "seat-b", None, "seat-b", "[]"),
        (6, "secret-a", T[10], "in-progress", f"seat-{secret_a}", None, f"seat-{secret_a}", json.dumps([f"mission-{secret_a}"])),
        (7, "secret-b", T[20], "in-progress", f"seat-{secret_b}", None, f"seat-{secret_b}", json.dumps([f"mission-{secret_b}"])),
        (16, "actor-only", T[10], "pending", "seat-actor", None, None, "[]"),
    ])
    conn.execute("CREATE TABLE queue_items (qitem_id TEXT, destination_session TEXT, ts_updated TEXT)")
    conn.execute("INSERT INTO queue_items VALUES ('actor-only', 'seat-current', '2026-10-09T00:00:10Z')")
    conn.commit(); conn.close()
    out = ROOT / "out"; out.mkdir()
    analytics = out / "analytics.duckdb"
    code, stdout, stderr = run(["run", "--db", str(analytics), "--root", f"claude={src/'claude'}", "--root", f"codex={src/'codex'}", "--root", f"pi={src/'pi'}", "--root", f"openrig={db}", "--frozen"])
    check("ac2_run", code == 0, 0, code)
    code, text, err = report(analytics, "tokens", rig="rig-one")
    payload = json.loads(text)["payload"] if code == 0 else {}
    check("ac3_linked_present", any(row.get("native_session") == "s-linked" and row.get("status") == "linked" for row in payload.get("links", [])), True, True)
    lcode, links_text, _ = report(analytics, "links")
    link_rows = json.loads(links_text)["payload"]["links"] if lcode == 0 else []
    check("ac3_unlinked", any(row.get("native_session") == "s-unlinked" and row.get("status") == "unlinked" for row in link_rows), True, any(row.get("native_session") == "s-unlinked" for row in link_rows))
    sessions = [row.get("native_session") for row in payload.get("links", []) if "sess-" in str(row.get("native_session"))]
    check("ac10_secret_sessions_distinct", len(set(sessions)) == 2, 2, len(set(sessions)))
    blob = analytics.read_bytes().decode("latin1") + text + stdout + stderr
    check("ac10_no_secret", secret_a not in blob and secret_b not in blob and canary not in blob and "TOOLNAME" not in blob, True, "TOOLNAME" not in blob)
    check("ac10_redacted_visible", "[redacted]" in blob, True, "[redacted]" in blob)
    code, activity, _ = report(analytics, "activity")
    act = json.loads(activity)["payload"]["activity"] if code == 0 else []
    seat = next((row for row in act if row["seat"] == "seat-a"), {})
    check("ac4_idle_with", seat.get("idle_with_work_secs") == 10, 10, seat.get("idle_with_work_secs"))
    check("ac4_idle_without", seat.get("idle_without_work_secs") == 50, 50, seat.get("idle_without_work_secs"))
    check("ac4_sum", seat.get("sum") == 60, 60, seat.get("sum"))
    old = next((row for row in act if row["seat"] == "seat-old"), {})
    check("ac4_unknown_window", old.get("unknown_secs") == 60 and old.get("active_secs") == 0, 60, old.get("unknown_secs"))
    carry = next((row for row in act if row["seat"] == "seat-carry"), {})
    check("qa_carry_in_demand", carry.get("idle_with_work_secs") == 60 and carry.get("idle_without_work_secs") == 0, 60, carry.get("idle_with_work_secs"))
    cross = next((row for row in act if row["seat"] == "seat-cross"), {})
    check("qa_cross_window_terminal", cross.get("idle_with_work_secs") == 10 and cross.get("idle_without_work_secs") == 50, 10, cross.get("idle_with_work_secs"))
    gap = next((row for row in act if row["seat"] == "seat-gap"), {})
    check("qa_missing_state_is_unknown", gap.get("active_secs") == 10 and gap.get("unknown_secs") == 50, 50, gap.get("unknown_secs"))
    secret = next((row for row in act if row["seat"] == "seat-secret"), {})
    db_bytes = analytics.read_bytes()
    check("qa_activity_state_canary", secret.get("unknown_secs") == 60 and b"FAKESTATE" not in db_bytes and "FAKESTATE" not in activity, 60, secret.get("unknown_secs"))
    code, handoffs, _ = report(analytics, "handoffs")
    edges = json.loads(handoffs)["payload"]["handoff_edges"] if code == 0 else []
    check("ac5_one_return", sum(1 for edge in edges if edge.get("return")) == 1, 1, sum(1 for edge in edges if edge.get("return")))
    intakes = json.loads(handoffs)["payload"].get("intakes")
    check("ac5_intakes_present", intakes is not None, True, intakes is not None)
    estimate = next((row for row in intakes or [] if row.get("tokens") == 5), {})
    check("ac5_intake_tokens", estimate.get("tokens") == 5, 5, estimate.get("tokens"))
    ambiguous = [row for row in intakes or [] if row.get("status") == "ambiguous"]
    nodata = [row for row in intakes or [] if row.get("status") == "no_data"]
    check("ac5_no_data", len(nodata) >= 1, ">=1", len(nodata))
    medians = json.loads(handoffs)["payload"].get("pair_medians")
    check("ac5_medians_present", medians is not None, True, medians is not None)
    pair = next((row for row in medians or [] if row.get("from") == "seat-a" and row.get("to") == "seat-b"), {})
    check("ac5_even_median", pair.get("wait_median") == 5, 5, pair.get("wait_median"))
    loops = json.loads(handoffs)["payload"].get("tool_loops", [])
    check("ac6_loop", len(loops) == 1 and loops[0].get("count") == 3, 3, loops[0].get("count") if loops else 0)
    code, traced, _ = run(["trace", "--db", str(analytics), "--metric-id", "tokens", "--from", FROM, "--to", TO])
    traced = json.loads(traced) if code == 0 else {}
    replay = sum(r["input_uncached"] + r["output"] + r["cache_read"] + r["cache_write"] for r in traced.get("inputs", {}).get("rows", []))
    check("ac7_trace_replay", replay == traced.get("value"), traced.get("value"), replay)
    rows_in = traced.get("inputs", {}).get("rows", [])
    check("qa_trace_provenance", rows_in and all("generation" in row and "line" in row and "path" in row and "predecessor_reason" in row for row in rows_in), True, bool(rows_in))
    codex_rows = [row for row in rows_in if str(row.get("source_key", "")).startswith("codex:")]
    reasons = [row.get("predecessor_reason") for row in codex_rows]
    check("qa_codex_sample_predecessors", reasons[:3] == ["baseline", "prior", "reset"] and codex_rows[1].get("prior_input_tokens") == 10 and codex_rows[2].get("prior_input_tokens") is None, ["baseline", "prior", "reset"], reasons[:3])
    demand = traced.get("inputs", {}).get("demand_events", [])
    actor_only = [row for row in demand if row.get("qitem_id") == "actor-only"]
    check("qa_actor_is_not_destination", actor_only and all(row.get("dest") == "unknown" for row in actor_only), "unknown", actor_only[0].get("dest") if actor_only else None)
    snapshots = traced.get("inputs", {}).get("destination_snapshots", [])
    current = next((row for row in snapshots if row.get("qitem_id") == "actor-only"), {})
    check("qa_destination_snapshot", current.get("destination") == "seat-current" and current.get("historical") is False, "seat-current", current.get("destination"))
    hist = [row for row in demand if row.get("qitem_id") == "hist-item"]
    check("qa_created_claimed_dest", hist and all(row.get("dest") == "seat-hist" for row in hist), "seat-hist", [row.get("dest") for row in hist])
    changed = [row for row in demand if row.get("qitem_id") == "hist-change"]
    changed.sort(key=lambda row: row.get("ts") or "")
    check("qa_dest_changes_inside_window", [row.get("dest") for row in changed] == ["seat-early", "seat-late"], ["seat-early", "seat-late"], [row.get("dest") for row in changed])
    code, activity_text, _ = report(analytics, "activity")
    quiet = next((row for row in json.loads(activity_text)["payload"]["activity"] if row.get("seat") == "seat-quiet"), {}) if code == 0 else {}
    check("qa_roster_no_event", quiet.get("unknown_secs") == 60 and quiet.get("active_secs") == 0, 60, quiet.get("unknown_secs"))
    _, before_text, _ = report(analytics, "tokens")
    before_rows = json.loads(before_text)["payload"].get("usage_rows")
    code, _, _ = run(["run", "--db", str(analytics), "--root", f"claude={src/'claude'}", "--root", f"codex={src/'codex'}", "--root", f"pi={src/'pi'}", "--root", f"openrig={db}", "--frozen"])
    _, text2, _ = report(analytics, "tokens")
    second = json.loads(text2)["payload"]
    check("ac8_rerun_rows", code == 0 and second.get("usage_rows") == before_rows, before_rows, second.get("usage_rows"))
    code, decisions, _ = report(analytics, "decisions", rig="missing-rig")
    m1 = json.loads(decisions)["payload"].get("m1_yield") if code == 0 else "err"
    check("ac9_m1_no_data", m1 is None, None, m1)
    alias = ROOT / "alias.duckdb"
    alias.symlink_to(src / "claude" / "s.jsonl")
    before = hashlib.sha256((src / "claude" / "s.jsonl").read_bytes()).hexdigest()
    code, _, _ = run(["run", "--db", str(alias), "--root", f"claude={src/'claude'}"])
    after = hashlib.sha256((src / "claude" / "s.jsonl").read_bytes()).hexdigest()
    check("ac11_symlink", code != 0 and before == after, "refused", code)
    link = ROOT / "hardlink.duckdb"
    os.link(src / "claude" / "s.jsonl", link)
    code, _, _ = run(["run", "--db", str(link), "--root", f"claude={src/'claude'}"])
    after_link = hashlib.sha256((src / "claude" / "s.jsonl").read_bytes()).hexdigest()
    check("ac11_hardlink", code != 0 and before == after_link, "refused", code)
    code, isolated, _ = report(analytics, "activity", rig="nonexistent-rig")
    isolated_payload = json.loads(isolated)["payload"] if code == 0 else {}
    check("qa_nonexistent_rig", isolated_payload.get("activity") == [] and isolated_payload.get("tool_loops") == [], [], isolated_payload.get("activity"))
    tcode, ttext, _ = run(["trace", "--db", str(analytics), "--metric-id", "activity", "--rig", "nonexistent-rig", "--from", FROM, "--to", TO])
    traced_activity = json.loads(ttext) if tcode == 0 else {}
    check("qa_trace_activity", tcode == 0 and traced_activity.get("metric_id") == "activity" and "activity_events" in traced_activity.get("inputs", {}), 0, tcode)
    mcode, mtext, _ = run(["trace", "--db", str(analytics), "--metric-id", "m1", "--from", FROM, "--to", TO])
    minputs = json.loads(mtext).get("inputs", {}) if mcode == 0 else {}
    check("qa_trace_m1_denominator", mcode == 0 and isinstance(minputs.get("nonreturn_handoffs"), list), True, isinstance(minputs.get("nonreturn_handoffs"), list))
    rcode, rtext, _ = run(["trace", "--db", str(analytics), "--metric-id", "activity", "--from", FROM, "--to", TO])
    replay_inputs = json.loads(rtext).get("inputs", {}) if rcode == 0 else {}
    demand = replay_inputs.get("demand_events", [])
    carry_open = [row for row in demand if row.get("dest") == "seat-carry" and row.get("state") == "pending"]
    carry_closed = [row for row in demand if row.get("dest") == "seat-carry" and row.get("state") in {"done", "canceled"}]
    check("qa_trace_replay_carry", rcode == 0 and len(carry_open) == 1 and len(carry_closed) == 0, 60, 60 if len(carry_open) == 1 and not carry_closed else 0)
    missing = ROOT / "no-such-root"
    rcode, rstdout, rstderr = run(["run", "--db", str(ROOT / "root-name.duckdb"), "--root", f"sk-ant-QA_SYNTHETIC_ROOTNAME_20261010={missing}"])
    check("qa_root_name_stdout", rcode == 0 and "QA_SYNTHETIC_ROOTNAME" not in rstdout + rstderr and "[redacted]" in rstdout, 0, rcode)
    rig_canary = "sk-ant-QA_SYNTHETIC_RIG"
    code, text, err = report(analytics, "activity", rig=rig_canary)
    check("qa_rig_report_stdout", code == 0 and rig_canary not in text + err, 0, code)
    code, text, err = run(["trace", "--db", str(analytics), "--metric-id", "activity", "--rig", rig_canary, "--from", FROM, "--to", TO])
    check("qa_rig_trace_stdout", code == 0 and rig_canary not in text + err, 0, code)
    code, text, err = run(["report", rig_canary, "--db", str(analytics), "--from", FROM, "--to", TO])
    check("qa_kind_stderr", code != 0 and rig_canary not in text + err, 1, code)
    code, text, err = run(["trace", "--db", str(analytics), "--metric-id", rig_canary, "--from", FROM, "--to", TO])
    check("qa_metric_stderr", code != 0 and rig_canary not in text + err, 1, code)
    families = ["sk-ant-QA", "ghp_QA", "gho_QA", "github_pat_QA", "xai-QA", "AKIAQA", "Bearer QA", "xoxb-QA", "xoxp-QA", "seat-sk-ant-QA"]
    leaked = []
    for token in families:
        for args in (
            ["report", "activity", "--db", str(analytics), "--rig", token, "--from", FROM, "--to", TO],
            ["trace", "--db", str(analytics), "--metric-id", "activity", "--rig", token, "--from", FROM, "--to", TO],
            ["report", token, "--db", str(analytics), "--from", FROM, "--to", TO],
            ["trace", "--db", str(analytics), "--metric-id", token, "--from", FROM, "--to", TO],
        ):
            code, text, err = run(args)
            if token in text + err:
                leaked.append(token)
    check("qa_cli_marker_families", leaked == [], [], leaked)
    surface_leaks = []
    for token in families:
        for kind in ("links", "activity", "handoffs", "tokens", "decisions"):
            code, text, err = report(analytics, kind, rig=token)
            if code != 0 or token in text + err:
                surface_leaks.append(f"report:{kind}")
        for metric in ("m1", "m2", "m3", "m4", "tokens", "intake", "medians", "activity"):
            code, text, err = run(["trace", "--db", str(analytics), "--metric-id", metric, "--rig", token, "--from", FROM, "--to", TO])
            if code != 0 or token in text + err:
                surface_leaks.append(f"trace:{metric}")
    check("qa_all_kinds_and_traces_hide_families", surface_leaks == [], [], surface_leaks[:8])
    payload_leaks = []
    nonempty = []
    for kind in ("links", "activity", "handoffs", "tokens", "decisions"):
        code, text, err = report(analytics, kind, rig="rig-one")
        body = json.loads(text)["payload"] if code == 0 else {}
        present = {
            "links": bool(body.get("links")),
            "activity": bool(body.get("activity")),
            "handoffs": bool(body.get("handoff_edges")),
            "tokens": (body.get("usage_rows") or 0) > 0,
            "decisions": body.get("m1_by_seat") is not None,
        }[kind]
        if code != 0 or not present or secret_a in text + err:
            payload_leaks.append(kind)
        else:
            nonempty.append(kind)
    for metric in ("m1", "m2", "m3", "m4", "tokens", "intake", "medians", "activity"):
        code, text, err = run(["trace", "--db", str(analytics), "--metric-id", metric, "--rig", "rig-one", "--from", FROM, "--to", TO])
        data = json.loads(text) if code == 0 else {}
        inputs = data.get("inputs") or {}
        present = {
            "tokens": bool(inputs.get("rows")),
            "activity": bool(inputs.get("activity")),
            "m1": bool(inputs.get("nonreturn_handoffs")) and bool(inputs.get("k2_closed_item")),
            "m2": bool(inputs.get("m2_pairs")),
            "m3": inputs.get("m3_chains") not in (None, 0),
            "m4": isinstance(data.get("value"), str) and "insufficient data" in data.get("value"),
            "intake": bool(inputs.get("intakes")),
            "medians": bool(inputs.get("pair_medians")),
        }[metric]
        if code != 0 or not present or secret_a in text + err:
            payload_leaks.append(metric)
        else:
            nonempty.append(metric)
    check("qa_linked_rig_payloads_hide_secret", payload_leaks == [] and len(nonempty) == 13, nonempty, payload_leaks)
    code, text, err = run(["--not-a-flag", "sk-ant-QA_SYNTHETIC_FLAG"])
    check("qa_clap_unknown_flag", code != 0 and "QA_SYNTHETIC_FLAG" not in text + err, 1, code)
    code, text, err = run(["--help"])
    check("qa_ordinary_help", code == 0 and "report" in text and "QA_SYNTHETIC_FLAG" not in text, 0, code)
    code, text, err = run(["report", "activity", "--db", str(ROOT / "sk-ant-QA_SYNTHETIC_DB"), "--from", FROM, "--to", TO])
    check("qa_secret_db_path", code != 0 and "QA_SYNTHETIC_DB" not in text + err, 1, code)
    code, text, err = run(["init-nao", "--target", str(ROOT / "sk-ant-QA_SYNTHETIC_NAO"), "--db", str(analytics)])
    check("qa_secret_nao_path", code != 0 and "QA_SYNTHETIC_NAO" not in text + err and not (ROOT / "sk-ant-QA_SYNTHETIC_NAO").exists(), 1, code)
    leaked_surfaces = []
    for kind in ("links", "activity", "handoffs", "tokens", "decisions"):
        code, text, err = report(analytics, kind)
        if secret_a in text + err:
            leaked_surfaces.append(kind)
    for metric in ("m1", "m2", "m3", "m4", "tokens", "intake", "medians", "activity"):
        code, text, err = run(["trace", "--db", str(analytics), "--metric-id", metric, "--from", FROM, "--to", TO])
        if code != 0 or secret_a in text + err:
            leaked_surfaces.append(metric)
    check("qa_all_surfaces_hide_secret", leaked_surfaces == [], [], leaked_surfaces)
    code, links_all, _ = report(analytics, "links")
    collide = next((row for row in json.loads(links_all)["payload"]["links"] if row.get("native_session") == "s-collide"), {}) if code == 0 else {}
    check("qa_identity_collision", collide.get("status") == "collision" and collide.get("seat") is None, "collision", collide.get("status"))
    key_path = Path(str(analytics) + ".key")
    original_key = key_path.read_bytes()
    flipped = original_key[:-1] + bytes([original_key[-1] ^ 1])
    key_path.write_bytes(flipped)
    code, text, err = run(["run", "--db", str(analytics), "--root", f"claude={src/'claude'}", "--root", f"codex={src/'codex'}", "--root", f"pi={src/'pi'}", "--root", f"openrig={db}", "--frozen"])
    key_path.write_bytes(original_key)
    check("qa_wrong_key", code != 0 and secret_a not in text + err, 1, code)
    failed = [row for row in rows if not row["ok"]]
    Path("/tmp/ac3-ac12-results.json").write_text(json.dumps({"commit": subprocess.check_output(["git", "-C", "/home/sam/code/dendro/.worktrees/work/dendro-r1", "rev-parse", "HEAD"], text=True).strip(), "rows": rows}, indent=2))
    print("FAILED", len(failed), "OF", len(rows))
    for row in failed:
        print(row)
    return 1 if failed else 0

if __name__ == "__main__":
    raise SystemExit(main())
