#!/usr/bin/env python3
"""Create synthetic native source fixtures and explicit portfolio mappings.

No installed harness or owner state is read. Existing fixture outputs are refused.
Run the printed ingest commands with the candidate dendro binary before portfolio QA.
"""
import argparse
import json
from pathlib import Path
import shlex
import sqlite3

parser = argparse.ArgumentParser()
parser.add_argument("output_directory", type=Path)
args = parser.parse_args()
root = args.output_directory.resolve()
root.mkdir(parents=True, exist_ok=False)
(root / "outputs").mkdir()
seats = [("supervisor", "kernel", "steer.lead"), ("builder", "worker", "dev.build"),
         ("reviewer", "worker", "dev.qa"), ("manager", "pm", "pm.manager")]
commands = []
mappings = []
for name, selected, usage_seat, amount in [("worker", ["worker", "kernel"], "builder", 100),
                                          ("product", ["pm"], "manager", 25)]:
    folder = root / "inputs" / name
    folder.mkdir(parents=True)
    db = folder / "openrig.sqlite"
    conn = sqlite3.connect(db)
    conn.executescript("""
      CREATE TABLE events(seq INTEGER PRIMARY KEY,type TEXT,payload TEXT,created_at TEXT);
      CREATE TABLE snapshots(data TEXT);
      CREATE TABLE queue_transitions(transition_id INTEGER,qitem_id TEXT,ts TEXT,state TEXT,
        actor_session TEXT,closure_reason TEXT,closure_target TEXT,transition_note TEXT);
      CREATE TABLE queue_items(qitem_id TEXT,destination_session TEXT,ts_updated TEXT,tags TEXT,
        chain_of_record TEXT,handed_off_from TEXT,last_nudge_result TEXT);
    """)
    seq = 0
    def event(kind, payload, seconds=0):
        global seq
        seq += 1
        conn.execute("INSERT INTO events VALUES (?,?,?,?)",
                     (seq, kind, json.dumps(payload), f"2026-01-01T00:00:{seconds:02d}Z"))
    for seat, rig, logical in seats:
        event("agent.session_identity", {"sessionId": seat, "runtime": "codex", "sessionName": seat, "rigId": rig})
    conn.execute("INSERT INTO snapshots VALUES (?)", (json.dumps({
        "nodes": [{"id": seat, "logicalId": logical, "resolvedSpecName": "agentflow-seat"}
                  for seat, rig, logical in seats],
        "sessions": [{"nodeId": seat, "sessionName": seat} for seat, rig, logical in seats]}),))
    if name == "worker":
        for i, (sender, receiver) in enumerate([("supervisor", "builder"), ("builder", "reviewer"), ("builder", "manager")], 1):
            item = f"synthetic-{i}"
            event("queue.created", {"qitemId": item, "sourceSession": sender, "destinationSession": sender}, i)
            conn.execute("INSERT INTO queue_items VALUES (?,?,?,?,?,?,?)",
                         (item, sender, f"2026-01-01T00:00:{i+10:02d}Z", "[]", "[]", None, None))
            conn.execute("INSERT INTO queue_transitions VALUES (?,?,?,?,?,?,?,?)",
                         (i, item, f"2026-01-01T00:00:{i+10:02d}Z", "done", sender, "handed_off_to", receiver, None))
    conn.commit()
    conn.close()
    logs = folder / "codex"
    logs.mkdir()
    records = [{"type": "session_meta", "payload": {"id": usage_seat}},
               {"type": "event_msg", "timestamp": "2026-01-01T00:00:02Z", "payload": {
                   "type": "token_count", "info": {"total_token_usage": {
                       "input_tokens": amount, "output_tokens": 10, "cached_input_tokens": 0,
                       "reasoning_output_tokens": 0}}}}]
    (logs / "session.jsonl").write_text("".join(json.dumps(row) + "\n" for row in records))
    analytics = root / "outputs" / f"{name}.duckdb"
    commands.append("dendro run --db " + shlex.quote(str(analytics)) + " --root " +
                    shlex.quote("openrig=" + str(db)) + " --root " + shlex.quote("codex=" + str(logs)))
    mapping = {"db": str(analytics), "rigs": selected}
    if name == "worker":
        holds = folder / "holds"
        holds.mkdir()
        (holds / "synthetic.json").write_text(json.dumps({
            "id": "synthetic-hold", "state": "delivered", "rig": "worker", "sender": "manager",
            "target": "builder", "createdAt": "2026-01-01T00:00:00Z", "deliveredAt": "2026-01-01T00:01:00Z",
            "signals": [{"id": "B1", "detail": "synthetic private detail omitted by reader"}],
            "body": "SYNTHETIC PRIVATE BODY MUST NOT REACH REPORT"}) + "\n")
        mapping["hold_dir"] = str(holds)
    mappings.append(mapping)
(root / "portfolio-sources.json").write_text(json.dumps(mappings, indent=2) + "\n")
(root / "ingest.sh").write_text("\n".join(commands) + "\n")
print(json.dumps({"synthetic": True, "mapping": str(root / "portfolio-sources.json"), "ingest_commands": commands}))
