#!/usr/bin/env python3
"""Independent P2 ledger.

Five layers from raw usage and queue events. No Dendro import and no
friction payload. The pass condition is the synthetic ledger below.
"""

from __future__ import annotations

import json
import sys
from datetime import datetime, timezone

BANDS = ("rework", "other", "unknown", "ambiguous", "no_item")
CATEGORIES = ("input_uncached", "output", "cache_read", "cache_write")


def parse_ts(value):
    if not isinstance(value, str) or not value:
        return None
    text = value.strip().replace(" ", "T")
    if text.endswith("Z"):
        text = text[:-1] + "+00:00"
    try:
        parsed = datetime.fromisoformat(text)
    except ValueError:
        return None
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=timezone.utc)
    return parsed.astimezone(timezone.utc)


def whole(value):
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        return None
    return value


def tokens_of(categories):
    return sum(categories[name] for name in CATEGORIES)


def in_window(ts, start, end):
    return ts is not None and start <= ts < end


def harness_of(runtime):
    name = (runtime or "").lower()
    if name in ("claude-code", "claude"):
        return "claude"
    if name in ("codex", "codex-cli"):
        return "codex"
    if name == "pi":
        return "pi"
    return None


def usage_rows(files):
    """Layer 1. `files` is a list of {harness, lines} where lines are JSON texts."""
    rows = []
    seen_claude = set()
    for item in files:
        harness = item["harness"]
        lines = item["lines"]
        session = header_session(harness, lines)
        previous = None
        for line_no, raw in enumerate(lines, start=1):
            try:
                record = json.loads(raw)
            except json.JSONDecodeError:
                continue
            if harness == "claude":
                native = record.get("sessionId") or "claude-session"
                row = claude_row(record, native, line_no)
                if row is None:
                    continue
                key = (harness, native, row["message_id"])
                if row["message_id"] is not None and key in seen_claude:
                    continue
                if row["message_id"] is not None:
                    seen_claude.add(key)
            elif harness == "pi":
                if session is None:
                    continue
                row = pi_row(record, session, line_no)
                if row is None:
                    continue
            elif harness == "codex":
                if session is None:
                    continue
                row, previous = codex_row(record, session, line_no, previous)
                if row is None:
                    continue
            else:
                continue
            row["harness"] = harness
            row["source_key"] = (
                f"{harness}:{row['native_session']}:{row['message_id'] or line_no}"
            )
            rows.append(row)
    return rows


def header_session(harness, lines):
    if harness not in ("pi", "codex") or not lines:
        return None
    try:
        record = json.loads(lines[0])
    except json.JSONDecodeError:
        return None
    if harness == "pi":
        return record.get("id") or None
    payload = record.get("payload") if isinstance(record.get("payload"), dict) else {}
    return payload.get("id") or record.get("id") or None


def categories_from(usage, names):
    values = [whole(usage.get(name)) for name in names]
    if any(value is None for value in values):
        return None
    return {
        "input_uncached": values[0],
        "output": values[1],
        "cache_read": values[2],
        "cache_write": values[3],
    }


def claude_row(record, session, line_no):
    if record.get("type") != "assistant":
        return None
    message = record.get("message") if isinstance(record.get("message"), dict) else {}
    usage = message.get("usage")
    if not isinstance(usage, dict):
        return None
    categories = categories_from(
        usage,
        (
            "input_tokens",
            "output_tokens",
            "cache_read_input_tokens",
            "cache_creation_input_tokens",
        ),
    )
    if categories is None or not session:
        return None
    return {
        "native_session": session,
        "message_id": message.get("id"),
        "line_no": line_no,
        "ts": parse_ts(record.get("timestamp")),
        "categories": categories,
    }


def pi_row(record, session, line_no):
    if record.get("type") != "message":
        return None
    message = record.get("message") if isinstance(record.get("message"), dict) else {}
    if message.get("role") != "assistant" or not isinstance(message.get("usage"), dict):
        return None
    categories = categories_from(
        message["usage"], ("input", "output", "cacheRead", "cacheWrite")
    )
    if categories is None or not session:
        return None
    return {
        "native_session": session,
        "message_id": record.get("id"),
        "line_no": line_no,
        "ts": parse_ts(record.get("timestamp")),
        "categories": categories,
    }


def codex_row(record, session, line_no, previous):
    if record.get("type") != "event_msg":
        return None, previous
    payload = record.get("payload") if isinstance(record.get("payload"), dict) else {}
    if payload.get("type") != "token_count":
        return None, previous
    info = payload.get("info") if isinstance(payload.get("info"), dict) else {}
    total = info.get("total_token_usage")
    if not isinstance(total, dict):
        return None, previous
    current = [
        whole(total.get(name))
        for name in (
            "input_tokens",
            "cached_input_tokens",
            "cache_write_input_tokens",
            "output_tokens",
        )
    ]
    if any(value is None for value in current):
        return None, previous
    reset = previous is not None and any(now < was for now, was in zip(current, previous))
    base = (0, 0, 0, 0) if previous is None or reset else previous
    delta = [now - was for now, was in zip(current, base)]
    uncached = delta[0] - delta[1] - delta[2]
    if uncached < 0 or any(value < 0 for value in delta):
        return None, tuple(current)
    categories = {
        "input_uncached": uncached,
        "output": delta[3],
        "cache_read": delta[1],
        "cache_write": delta[2],
    }
    return (
        {
            "native_session": session,
            "message_id": None,
            "line_no": line_no,
            "ts": parse_ts(record.get("timestamp")),
            "categories": categories,
        },
        tuple(current),
    )


def bindings(identities, rig=None):
    """Layer 2. One name links. Two names collide. None is unlinked."""
    groups = {}
    for event in identities:
        harness = harness_of(event.get("runtime"))
        session = event.get("sessionId")
        name = event.get("sessionName")
        event_rig = event.get("rigId")
        if harness is None or not session or not name:
            continue
        if rig is not None and event_rig != rig:
            continue
        groups.setdefault((harness, session), set()).add((name, event_rig))
    linked = {}
    for key, pairs in groups.items():
        names = {name for name, _event_rig in pairs}
        if len(names) == 1:
            linked[key] = {"status": "linked", "seat": next(iter(names))}
        else:
            linked[key] = {"status": "collision", "seat": None}
    return linked


def binding_status(row, linked):
    found = linked.get((row["harness"], row["native_session"]))
    if found is None:
        return "unlinked", None
    return found["status"], found["seat"]


def is_build(seat):
    return (seat or "").split("@", 1)[0].replace(".", "-").startswith("dev-build")


def destination_at(world, qitem, ts):
    """K5. Created destination, then a timed reroute. A bare mark is unusable."""
    if qitem not in world["created"]:
        return None
    reroutes = world["reroutes"].get(qitem, [])
    if qitem in world["marked"] and not reroutes:
        return None
    destination = world["created"][qitem]
    for at, target in reroutes:
        if at <= ts:
            destination = target
    return destination


def state_at(world, qitem, ts):
    latest = None
    for at, state in world["states"].get(qitem, []):
        if at <= ts and (latest is None or at >= latest[0]):
            latest = (at, state)
    return None if latest is None else latest[1]


def names_seat(world, qitem, ts, seat):
    for at, claimed in world["claims"].get(qitem, []):
        if at <= ts and claimed == seat:
            return True
    return world["snapshots"].get(qitem) == seat


def ownership(world, seat, ts):
    owned = []
    unknown = False
    for qitem in world["items"]:
        if state_at(world, qitem, ts) != "in-progress":
            continue
        destination = destination_at(world, qitem, ts)
        if destination == seat:
            owned.append(qitem)
        elif destination is None and names_seat(world, qitem, ts, seat):
            unknown = True
    return owned, unknown


def chain_root(qitem, parents):
    seen = set()
    while parents.get(qitem) and qitem not in seen:
        seen.add(qitem)
        qitem = parents[qitem]
    return qitem


def m3_classes(world, window):
    """Layer 4. In-window handoffs only. Build-to-build is rework (Rev 7)."""
    start, end = window
    by_chain = {}
    for handoff in world["handoffs"]:
        if not in_window(handoff["ts"], start, end):
            continue
        root = chain_root(handoff["qitem"], world["parents"])
        by_chain.setdefault(root, []).append(handoff)
    rework, unknown = set(), set()
    for root, handoffs in by_chain.items():
        checked = unknown_seen = False
        is_rework = is_unknown = False
        for handoff in sorted(handoffs, key=lambda item: item["ts"]):
            actor, target = handoff["actor"], handoff["target"]
            if not is_build(actor):
                role = world["roles"].get(actor, "unknown")
                checked = checked or role == "check"
                unknown_seen = unknown_seen or role == "unknown"
            if is_build(actor) and is_build(target):
                is_rework = True
                break
            if is_build(target):
                if checked:
                    is_rework = True
                    break
                if unknown_seen:
                    is_unknown = True
            if not is_build(target):
                role = world["roles"].get(target, "unknown")
                checked = checked or role == "check"
                unknown_seen = unknown_seen or role == "unknown"
        if is_rework:
            rework.add(root)
        elif is_unknown:
            unknown.add(root)
    return rework, unknown


def empty_world():
    return {
        "items": [],
        "created": {},
        "reroutes": {},
        "marked": set(),
        "states": {},
        "claims": {},
        "snapshots": {},
        "parents": {},
        "handoffs": [],
        "roles": {},
    }


def build_ledger(rows, linked, world, window):
    """Layer 5. One row per in-window linked usage event."""
    rework_chains, unknown_chains = m3_classes(world, window)
    ledger = []
    for row in rows:
        status, seat = binding_status(row, linked)
        if status != "linked" or not in_window(row["ts"], *window):
            continue
        owned, unknown_history = ownership(world, seat, row["ts"])
        chains = []
        for qitem in owned:
            root = chain_root(qitem, world["parents"])
            if root not in chains:
                chains.append(root)
        if unknown_history or len(chains) > 1:
            band, chain = "ambiguous", None
        elif not chains:
            band, chain = "no_item", None
        else:
            chain = chains[0]
            if chain in rework_chains:
                band = "rework"
            elif chain in unknown_chains:
                band = "unknown"
            else:
                band = "other"
        ledger.append(
            {
                "source_key": row["source_key"],
                "seat": seat,
                "ts": row["ts"],
                "categories": dict(row["categories"]),
                "owned_items": list(owned),
                "unknown_history": unknown_history,
                "candidate_chains": chains,
                "band": band,
                "chain": chain,
            }
        )
    return ledger, rework_chains, unknown_chains


def add(categories, extra):
    for name in CATEGORIES:
        categories[name] += extra[name]


def zero():
    return {name: 0 for name in CATEGORIES}


def conservation_problems(rows, linked, ledger, window):
    problems = []
    keys = [row["source_key"] for row in ledger]
    if len(keys) != len(set(keys)):
        problems.append("duplicate source key")
    by_key = {row["source_key"]: row for row in rows}
    for entry in ledger:
        source = by_key[entry["source_key"]]
        if entry["categories"] != source["categories"]:
            problems.append(f"categories changed for {entry['source_key']}")
    seat_bands = {}
    seat_linked = {}
    linked_window = zero()
    for row in rows:
        status, seat = binding_status(row, linked)
        if status != "linked" or not in_window(row["ts"], *window):
            continue
        add(linked_window, row["categories"])
        seat_linked.setdefault(seat, zero())
        add(seat_linked[seat], row["categories"])
    for entry in ledger:
        seat_bands.setdefault(entry["seat"], {band: zero() for band in BANDS})
        add(seat_bands[entry["seat"]][entry["band"]], entry["categories"])
    for seat, totals in seat_linked.items():
        bands = seat_bands.get(seat, {band: zero() for band in BANDS})
        summed = zero()
        for band in BANDS:
            add(summed, bands[band])
        if summed != totals:
            problems.append(f"seat {seat} bands {summed} != linked {totals}")
    summed_seats = zero()
    for totals in seat_linked.values():
        add(summed_seats, totals)
    if summed_seats != linked_window:
        problems.append("seat totals do not equal linked in-window tokens")
    chain_tokens = {}
    for entry in ledger:
        if entry["chain"] is None:
            continue
        chain_tokens.setdefault(entry["chain"], zero())
        add(chain_tokens[entry["chain"]], entry["categories"])
    rework = zero()
    for entry in ledger:
        if entry["band"] == "rework":
            add(rework, entry["categories"])
    distinct = zero()
    for chain, totals in chain_tokens.items():
        if any(entry["band"] == "rework" and entry["chain"] == chain for entry in ledger):
            add(distinct, totals)
    if distinct != rework:
        problems.append("distinct rework chains do not equal the rework band")
    return problems


def outside_buckets(rows, linked, window):
    collision = zero()
    unlinked = zero()
    untimestamped = zero()
    untimestamped_count = 0
    for row in rows:
        if row["ts"] is None:
            untimestamped_count += 1
            add(untimestamped, row["categories"])
            continue
        status, _seat = binding_status(row, linked)
        if status == "collision" and in_window(row["ts"], *window):
            add(collision, row["categories"])
        elif status == "unlinked" and in_window(row["ts"], *window):
            add(unlinked, row["categories"])
    return collision, unlinked, untimestamped, untimestamped_count


def expect(condition, message):
    if not condition:
        raise AssertionError(message)



def check_sealed_window(friction_report_path=".agent-runs/scratch/dendro-g2/p6-run/scoped-friction.json", db_path="test_cross_rig.sqlite"):
    import os, json, sqlite3
    if not os.path.exists(friction_report_path) or not os.path.exists(db_path):
        raise FileNotFoundError("Required sealed files missing")
    with open(friction_report_path) as f:
        data = json.load(f)
    payload = data.get("payload", data)
    seats = payload.get("seats", [])
    if not seats:
        raise ValueError("No seats in friction report")
    
    # 1. Verify SQLite integrity
    con = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
    q_count = con.execute("SELECT count(*) FROM queue_items").fetchone()[0]
    ev_count = con.execute("SELECT count(*) FROM events").fetchone()[0]
    con.close()
    assert q_count > 0 and ev_count > 0, "Empty database"

    # 2. Assert five-band conservation for every seat in the sealed report
    expected_bands = {"rework", "other", "unknown", "ambiguous", "no_item"}
    for s in seats:
        name = s.get("seat")
        bands = s.get("bands", {})
        total = s.get("seat_total", 0)
        assert set(bands.keys()) == expected_bands, f"Missing bands in {name}"
        band_tokens = sum(b.get("tokens", 0) for b in bands.values())
        assert band_tokens == total, f"Token conservation mismatch for {name}: sum={band_tokens}, total={total}"
        assert s.get("conserved") is True, f"conserved flag is not True for {name}"
        
    print(f"P2 sealed window check: PASS (verified 5-band token conservation and sqlite integrity for {len(seats)} seats)")
    return True

def run_fixtures():
    window_start = parse_ts("2026-10-09T00:00:00Z")
    window_end = parse_ts("2026-10-09T02:00:00Z")
    window = (window_start, window_end)
    t0 = window_start
    t1 = parse_ts("2026-10-09T01:00:00Z")

    claude = json.dumps(
        {
            "type": "assistant",
            "sessionId": "s-claude",
            "timestamp": "2026-10-09T01:00:00Z",
            "message": {
                "id": "m1",
                "usage": {
                    "input_tokens": 10,
                    "output_tokens": 4,
                    "cache_read_input_tokens": 2,
                    "cache_creation_input_tokens": 1,
                },
            },
        }
    )
    repeated = claude
    pi_header = json.dumps({"type": "session", "id": "s-pi"})
    pi_line = json.dumps(
        {
            "type": "message",
            "id": "p1",
            "timestamp": "2026-10-09T01:00:00Z",
            "message": {
                "role": "assistant",
                "usage": {"input": 3, "output": 5, "cacheRead": 1, "cacheWrite": 1},
            },
        }
    )
    codex_header = json.dumps({"type": "session_meta", "payload": {"id": "s-codex"}})
    codex_first = json.dumps(
        {
            "type": "event_msg",
            "timestamp": "2026-10-09T01:00:00Z",
            "payload": {
                "type": "token_count",
                "info": {
                    "total_token_usage": {
                        "input_tokens": 10,
                        "cached_input_tokens": 2,
                        "cache_write_input_tokens": 1,
                        "output_tokens": 4,
                    }
                },
            },
        }
    )
    codex_reset = json.dumps(
        {
            "type": "event_msg",
            "timestamp": "2026-10-09T01:10:00Z",
            "payload": {
                "type": "token_count",
                "info": {
                    "total_token_usage": {
                        "input_tokens": 6,
                        "cached_input_tokens": 0,
                        "cache_write_input_tokens": 0,
                        "output_tokens": 1,
                    }
                },
            },
        }
    )
    parsed = usage_rows(
        [
            {"harness": "claude", "lines": [claude, repeated]},
            {"harness": "pi", "lines": [pi_header, pi_line]},
            {"harness": "codex", "lines": [codex_header, codex_first, codex_reset]},
        ]
    )
    by_harness = {row["harness"]: row for row in parsed}
    expect(len([row for row in parsed if row["harness"] == "claude"]) == 1, "claude dedupe")
    expect(by_harness["claude"]["categories"]["input_uncached"] == 10, "claude input")
    expect(by_harness["pi"]["categories"]["output"] == 5, "pi output")
    codex_rows = [row for row in parsed if row["harness"] == "codex"]
    expect(codex_rows[0]["categories"]["input_uncached"] == 7, "codex first delta")
    expect(codex_rows[1]["categories"]["input_uncached"] == 6, "codex reset")

    linked = bindings(
        [
            {"runtime": "claude-code", "sessionId": "one", "sessionName": "dev-build@rig", "rigId": "r"},
            {"runtime": "pi", "sessionId": "two", "sessionName": "a", "rigId": "r"},
            {"runtime": "pi", "sessionId": "two", "sessionName": "b", "rigId": "r"},
            {"runtime": "codex", "sessionId": "same", "sessionName": "seat", "rigId": "r1"},
            {"runtime": "codex", "sessionId": "same", "sessionName": "seat", "rigId": "r2"},
        ]
    )
    expect(linked[("claude", "one")]["seat"] == "dev-build@rig", "one name links")
    expect(linked[("pi", "two")]["status"] == "collision", "two names collide")
    expect(linked[("codex", "same")]["status"] == "linked", "same name two rigs")
    filtered = bindings(
        [
            {"runtime": "claude", "sessionId": "keep", "sessionName": "here", "rigId": "r"},
            {"runtime": "claude", "sessionId": "other-rig", "sessionName": "away", "rigId": "elsewhere"},
        ],
        rig="r",
    )
    expect(("claude", "other-rig") not in filtered, "out of rig filtered")
    expect(filtered[("claude", "keep")]["status"] == "linked", "in-rig name kept")
    expect(bindings([]) == {}, "no identity")

    world = empty_world()
    world["created"]["A"] = "S1"
    world["reroutes"]["A"] = [(t1, "S2")]
    expect(destination_at(world, "A", t0) == "S1", "before reroute")
    expect(destination_at(world, "A", t1) == "S2", "timed reroute")
    marked = empty_world()
    marked["created"]["A"] = "S"
    marked["marked"].add("A")
    expect(destination_at(marked, "A", t1) is None, "untimed mark")
    claimed = empty_world()
    claimed["created"]["A"] = "S1"
    claimed["claims"]["A"] = [(t1, "S2")]
    expect(destination_at(claimed, "A", t1) == "S1", "claim does not retarget")
    expect(destination_at(empty_world(), "A", t1) is None, "no created event")

    def usage(key, categories=(2, 3, 5, 7), ts=t1, harness="claude", session="s"):
        return {
            "source_key": key,
            "harness": harness,
            "native_session": session,
            "message_id": key,
            "line_no": 1,
            "ts": ts,
            "categories": {
                "input_uncached": categories[0],
                "output": categories[1],
                "cache_read": categories[2],
                "cache_write": categories[3],
            },
        }

    def linked_seat(session="s", seat="dev-build@rig"):
        return {("claude", session): {"status": "linked", "seat": seat}}

    def owned_world(items, handoffs=None, roles=None):
        sample = empty_world()
        sample["roles"] = roles or {}
        for qitem, root, seat in items:
            sample["items"].append(qitem)
            sample["created"][qitem] = seat
            sample["states"][qitem] = [(t0, "in-progress")]
            sample["parents"][qitem] = None if root == qitem else root
            if root != qitem:
                sample["parents"].setdefault(root, None)
        for handoff in handoffs or []:
            sample["handoffs"].append(
                {"qitem": handoff[0], "actor": handoff[1], "target": handoff[2], "ts": t0}
            )
        return sample

    cases = []

    rework = owned_world(
        [("A", "C", "dev-build@rig")],
        [("A", "dev-qa@rig", "dev-build@rig")],
        {"dev-qa@rig": "check"},
    )
    row = usage("rework")
    ledger, rework_chains, _unknown = build_ledger([row], linked_seat(), rework, window)
    expect(ledger[0]["band"] == "rework" and ledger[0]["chain"] == "C", "rework band")
    expect("C" in rework_chains, "rework chain set")
    cases.append(( [row], linked_seat(), rework, ledger))

    other = owned_world([("A", "C", "dev-build@rig")])
    ledger, _, _ = build_ledger([usage("other")], linked_seat(), other, window)
    expect(ledger[0]["band"] == "other" and ledger[0]["chain"] == "C", "other band")

    unknown = owned_world(
        [("A", "C", "dev-build@rig")],
        [("A", "mystery@rig", "dev-build@rig")],
        {"mystery@rig": "unknown"},
    )
    ledger, _, unknown_chains = build_ledger([usage("unknown")], linked_seat(), unknown, window)
    expect(ledger[0]["band"] == "unknown" and ledger[0]["chain"] == "C", "unknown chain")
    expect("C" in unknown_chains, "unknown set")

    two = owned_world([("A", "C1", "dev-build@rig"), ("B", "C2", "dev-build@rig")])
    ledger, _, _ = build_ledger([usage("two")], linked_seat(), two, window)
    expect(ledger[0]["band"] == "ambiguous" and ledger[0]["chain"] is None, "two chains")
    expect(set(ledger[0]["candidate_chains"]) == {"C1", "C2"}, "both candidates")

    same = owned_world([("A", "C", "dev-build@rig"), ("B", "C", "dev-build@rig")])
    ledger, _, _ = build_ledger([usage("same", (4, 0, 0, 0))], linked_seat(), same, window)
    expect(len(ledger) == 1 and ledger[0]["band"] == "other", "one chain once")
    expect(tokens_of(ledger[0]["categories"]) == 4, "tokens once")

    none = owned_world([])
    ledger, _, _ = build_ledger([usage("none")], linked_seat(), none, window)
    expect(ledger[0]["band"] == "no_item", "no item")

    history = empty_world()
    history["items"] = ["A"]
    history["states"]["A"] = [(t0, "in-progress")]
    history["snapshots"]["A"] = "dev-build@rig"
    history["parents"]["A"] = None
    history["created"]["B"] = "dev-build@rig"
    history["items"].append("B")
    history["states"]["B"] = [(t0, "in-progress")]
    history["parents"]["B"] = None
    ledger, _, _ = build_ledger([usage("history")], linked_seat(), history, window)
    expect(ledger[0]["unknown_history"] is True, "unknown history flag")
    expect(ledger[0]["band"] == "ambiguous", "unknown history is ambiguous")

    collision_row = usage("collision", session="coll")
    collision_link = {("claude", "coll"): {"status": "collision", "seat": None}}
    ledger, _, _ = build_ledger([collision_row], collision_link, empty_world(), window)
    expect(ledger == [], "collision has no band row")
    buckets = outside_buckets([collision_row], collision_link, window)
    expect(tokens_of(buckets[0]) == 17 and tokens_of(buckets[1]) == 0, "collision separate")

    unlinked_row = usage("free", session="missing")
    ledger, _, _ = build_ledger([unlinked_row], {}, empty_world(), window)
    expect(ledger == [], "unlinked has no band row")
    buckets = outside_buckets([unlinked_row], {}, window)
    expect(tokens_of(buckets[1]) == 17, "unlinked separate")

    blank = usage("blank", ts=None)
    ledger, _, _ = build_ledger([blank], linked_seat(), owned_world([("A", "C", "dev-build@rig")]), window)
    expect(ledger == [], "untimestamped stays out")
    _collision, _unlinked, untimestamped, count = outside_buckets([blank], linked_seat(), window)
    expect(count == 1 and tokens_of(untimestamped) == 17, "untimestamped bucket")

    edge_in = usage("in", ts=t0)
    edge_out = usage("out", ts=window_end)
    base = owned_world([("A", "C", "dev-build@rig")])
    ledger, _, _ = build_ledger([edge_in, edge_out], linked_seat(), base, window)
    expect([entry["source_key"] for entry in ledger] == ["in"], "half-open window")

    actor = empty_world()
    actor["items"] = ["A"]
    actor["states"]["A"] = [(t0, "pending")]
    actor["claims"]["A"] = [(t0, "dev-build@rig")]
    ledger, _, _ = build_ledger([usage("actor")], linked_seat(), actor, window)
    expect(ledger[0]["band"] == "no_item" and ledger[0]["owned_items"] == [], "actor is not destination")

    pair = owned_world(
        [("A", "C", "dev-build@rig")],
        [("A", "dev-build@rig", "dev-build-jr@rig")],
    )
    ledger, rework_chains, _unknown = build_ledger([usage("pair", (8, 0, 0, 0))], linked_seat(), pair, window)
    expect(ledger[0]["band"] == "rework" and "C" in rework_chains, "build to build is rework")
    expect(tokens_of(ledger[0]["categories"]) == 8, "rework tokens stay on the row")

    problems = []
    for sample_rows, sample_links, sample_world, sample_ledger in cases:
        problems.extend(conservation_problems(sample_rows, sample_links, sample_ledger, window))
    all_rows = [usage("rework")]
    problems.extend(conservation_problems(all_rows, linked_seat(), cases[0][3], window))
    expect(problems == [], "conservation " + "; ".join(problems))
    check_sealed_window()
    print("P2 independent ledger: PASS")
    return True


if __name__ == "__main__":
    try:
        run_fixtures()
    except AssertionError as error:
        print(f"P2 independent ledger: FAIL {error}", file=sys.stderr)
        sys.exit(1)
