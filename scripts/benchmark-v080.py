#!/usr/bin/env python3
"""Dendro v080 synthetic corpus and bounded benchmark protocol (stdlib only).

See --help and .agentflow/evidence/v080/benchmark-runner/README.md.
This runner produces evidence, never independent product acceptance.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import re
import subprocess
import sys
import tempfile
import threading
import time

U64 = (1 << 64) - 1
SEED = 8675309
CORPUS = "dendro.bench.corpus/v1"
ANSWER = "dendro.bench.answer/v1"
MANIFEST = "dendro.bench.manifest/v1"
REPORT = "dendro.bench.report/v1"
MODES = {"warmup", "warm_answer", "cold_migration", "replay",
         "seven_answer_home", "ui_blocking"}
CAPS = {"warm_answer_p95_ns": 250000000, "seven_answer_home_ns": 1000000000,
        "ui_blocking_ns": 50000000, "small_process_resident_bytes": 536870912,
        "large_reliable_peak_rss_bytes": 1073741824,
        "retained_context_records": 1001, "max_retained_fact_payload_slots": 1,
        "process_tree": 2}
METRICS = set(CAPS) | {"warm_throughput", "cold_migration", "process_tree"}
MANIFEST_FIELDS = set("manifest_version candidate_binary_sha256 candidate_source_sha256 "
    "corpus_sha256 generator_sha256 runner_sha256 os_id compiler_id storage_id argv_sha256 "
    "command_surface shell seed profile corpus_version max_capture_bytes timeout_ns "
    "required_metrics clock".split())
COMPARABLE = "cohort command_surface work_mode unit os_family os_version environment compiler_id storage_id corpus_sha256 corpus_version profile seed metric_method clock".split()


class ProtocolError(ValueError):
    def __init__(self, status="schema_mismatch", code=5):
        super().__init__(status)
        self.status, self.code = status, code


def uint(value):
    if type(value) is not int or not 0 <= value <= U64:
        raise ProtocolError()
    return value


def exact(obj, fields, optional=()):
    if type(obj) is not dict or not set(fields) <= obj.keys() or obj.keys() - set(fields) - set(optional):
        raise ProtocolError()
    return obj


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"),
                      ensure_ascii=False, allow_nan=False).encode("utf-8")


def digest(data):
    return hashlib.sha256(data).hexdigest()


def file_digest(path):
    h = hashlib.sha256()
    with open(path, "rb") as src:
        for block in iter(lambda: src.read(65536), b""):
            h.update(block)
    return h.hexdigest()


def load_json(path, limit=16 * 1024 * 1024):
    # Reject duplicate keys and non-finite numbers, as well as oversized artifacts.
    def pairs(items):
        obj = {}
        for key, value in items:
            if key in obj:
                raise ProtocolError()
            obj[key] = value
        return obj
    with open(path, "rb") as src:
        raw = src.read(limit + 1)
    if len(raw) > limit:
        raise ProtocolError("output_limit_exceeded", 2)
    return parse_json(raw, pairs)


def parse_json(raw, pairs=None):
    if pairs is None:
        def pairs(items):
            obj = {}
            for key, value in items:
                if key in obj:
                    raise ProtocolError()
                obj[key] = value
            return obj
    try:
        return json.loads(raw.decode("utf-8"), object_pairs_hook=pairs,
                          parse_constant=lambda _: (_ for _ in ()).throw(ProtocolError()))
    except (ValueError, UnicodeError) as exc:
        raise ProtocolError() from exc


def atomic_json(path, obj):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, name = tempfile.mkstemp(prefix=".bench-", dir=path.parent)
    try:
        with os.fdopen(fd, "wb") as out:
            out.write(canonical(obj))
            out.flush()
            os.fsync(out.fileno())
        os.replace(name, path)
    finally:
        if os.path.exists(name):
            os.unlink(name)


def identity(profile, seed=SEED):
    if profile not in {"micro", "small", "large"} or type(seed) is not int or seed != SEED:
        raise ProtocolError()
    return {"corpus_version": CORPUS, "profile": profile, "seed": seed}


def corpus_id(profile, seed=SEED):
    identity(profile, seed)
    return f"{CORPUS}/{profile}/{seed}"


def node_id(profile, chain, depth):
    identity(profile)
    chains = 3 if profile == "micro" else 2250 if profile == "small" else 0
    if type(chain) is not int or type(depth) is not int or not 0 <= chain < chains or not 0 <= depth <= 4 or (profile == "micro" and chain == 2 and depth > 1):
        raise ProtocolError()
    return f"node-{profile}-{SEED}-{chain:04d}-{depth}"


def records(profile, seed=SEED):
    """Yield one payload at a time, with no retained fact collection or random input."""
    identity(profile, seed)
    base = {"profile": profile, "seed": seed}
    if profile == "large":
        for cls, count, width in (("fact", 1000000, 7), ("append", 1000, 4)):
            for i in range(count):
                yield dict(base, record_class=cls, i=i, id=f"{cls}-large-{seed}-{i:0{width}d}")
        return
    micro = profile == "micro"
    chains, full = (3, 2) if micro else (2250, 2250)
    def depths(c):
        return range(2 if micro and c == 2 else 5)
    for c in range(chains):
        for d in depths(c):
            yield dict(base, record_class="node", chain=c, depth=d, id=node_id(profile, c, d))
    for c in range(chains):
        for d in list(depths(c))[1:]:
            yield dict(base, record_class="present_nav", chain=c, child_depth=d,
                       id=f"nav-{profile}-{seed}-{c:04d}-{d}",
                       parent=node_id(profile, c, d - 1), target=node_id(profile, c, d))
    for j in range(1 if micro else 1000):
        yield dict(base, record_class="missing_nav", j=j, id=f"nav-missing-{profile}-{seed}-{j:04d}",
                   source=node_id(profile, j, 3), target=None)
    for k in range(19 if micro else 9500):
        yield dict(base, record_class="caller_single", k=k, id=f"caller-{profile}-{seed}-{k:04d}",
                   callers=[f"u-{profile}-{seed}-{k:04d}"], target=node_id(profile, k % chains, 0))
    for m in range(1 if micro else 500):
        yield dict(base, record_class="caller_multi", m=m, id=f"caller-multi-{profile}-{seed}-{m:03d}",
                   callers=[f"u-a-{profile}-{seed}-{m:03d}", f"u-b-{profile}-{seed}-{m:03d}"],
                   target=node_id(profile, m % full, 4))
    for cls, count in (("attempt", 20 if micro else 10000), ("usage", 20 if micro else 50000)):
        for i in range(count):
            rec = dict(base, record_class=cls, i=i, id=f"{cls}-{profile}-{seed}-{i:05d}",
                       target=node_id(profile, i % chains, 0))
            if cls == "usage":
                rec["quantity"] = 1
            yield rec


def corpus_bytes(profile):
    for rec in records(profile):
        yield canonical(rec) + b"\n"


def generate(profile, path):
    """Atomic streaming write; does not buffer a profile in memory."""
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, temp = tempfile.mkstemp(prefix=".corpus-", dir=path.parent)
    h, count, size = hashlib.sha256(), 0, 0
    try:
        with os.fdopen(fd, "wb") as out:
            for line in corpus_bytes(profile):
                out.write(line)
                h.update(line)
                count += 1
                size += len(line)
            out.flush()
            os.fsync(out.fileno())
        os.replace(temp, path)
    finally:
        if os.path.exists(temp):
            os.unlink(temp)
    return dict(identity(profile), corpus_id=corpus_id(profile), corpus_sha256=h.hexdigest(),
                record_count=count, byte_count=size)


def percentile(values, p):
    if p not in (50, 95):
        raise ProtocolError()
    for v in values:
        uint(v)
    if not values or any(v == 0 for v in values):
        return None
    return sorted(values)[(p * len(values) + 99) // 100 - 1]


def elapsed(start, end):
    if start is None or end is None or end <= start:
        return None
    return uint(end - start)


def statistics(values, mode="warm_answer"):
    for v in values:
        uint(v)
    if mode not in MODES:
        raise ProtocolError()
    if mode == "cold_migration" and len(values) > 5:
        raise ProtocolError()
    minimum = 20 if mode == "warm_answer" else 5 if mode == "cold_migration" else 1
    if len(values) < minimum or not values or 0 in values:
        return {"status": "blocked_unmeasured", "runner_exit": 6, "n": len(values),
                "p95_ns": None, "median_ns": None, "worst_ns": None,
                "best_ns": None, "spread_ns": None, "throughput": None}
    total = uint(sum(values))
    return {"status": "measured", "runner_exit": 0, "n": len(values),
            "p95_ns": percentile(values, 95), "median_ns": percentile(values, 50),
            "worst_ns": max(values), "best_ns": min(values),
            "spread_ns": max(values) - min(values),
            "throughput": {"numerator": len(values), "denominator": total}}


def comparable(candidate, baseline):
    return all(k in candidate and k in baseline and candidate[k] == baseline[k] for k in COMPARABLE)


def regression(ca, ce, ba, be, equal=True):
    for v in (ca, ce, ba, be):
        uint(v)
    if not equal or ba == 0 or ce == 0 or be == 0:
        return {"comparison_status": "refused_noncomparable", "regression": None,
                "exceeded": None}
    numerator, denominator = ba * ce - ca * be, ba * ce
    common = math.gcd(numerator, denominator)
    return {"comparison_status": "comparable",
            "regression": {"numerator": numerator // common, "denominator": denominator // common},
            "exceeded": 5 * ca * be < 4 * ba * ce}


def argv_for(candidate, request):
    return [str(candidate), "benchmark", "answer", "--protocol", ANSWER, "--request", str(request)]


def validate_surface(argv, surface="v080", shell=False):
    if shell is not False:
        raise ProtocolError("shell_forbidden")
    if surface != "v080" or type(argv) is not list or len(argv) != 7 or any(type(a) is not str or not a or "\x00" in a for a in argv) or argv[1:6] != ["benchmark", "answer", "--protocol", ANSWER, "--request"]:
        raise ProtocolError("unsupported_surface")


def validate_manifest(m):
    exact(m, MANIFEST_FIELDS)
    if m["manifest_version"] != MANIFEST or m["corpus_version"] != CORPUS or m["clock"] != "monotonic":
        raise ProtocolError()
    identity(m["profile"], m["seed"])
    if m["shell"] is not False:
        raise ProtocolError("shell_forbidden")
    if m["command_surface"] != "v080":
        raise ProtocolError("unsupported_surface")
    for k in ("max_capture_bytes", "timeout_ns"):
        uint(m[k])
        if m[k] == 0:
            raise ProtocolError()
    # Harness safety ceilings, distinct from latency budgets.
    if m["max_capture_bytes"] > 16 * 1024 * 1024 or m["timeout_ns"] > 300000000000:
        raise ProtocolError()
    for k, v in m.items():
        if k.endswith("sha256") and (type(v) is not str or not re.fullmatch("[0-9a-f]{64}", v)):
            raise ProtocolError()
    for k in ("os_id", "compiler_id", "storage_id"):
        if type(m[k]) is not str or not re.fullmatch(r"[A-Za-z0-9_.:+-]{1,128}", m[k]):
            raise ProtocolError()
    metrics = m["required_metrics"]
    if type(metrics) is not list or not metrics or any(type(k) is not str or k not in METRICS for k in metrics) or len(set(metrics)) != len(metrics):
        raise ProtocolError()
    return m


def validate_request(req):
    exact(req, ("protocol", "corpus_id", "question_id", "mode"))
    if req["protocol"] != ANSWER or req["mode"] not in MODES or any(type(req[k]) is not str for k in req):
        raise ProtocolError()
    return req


def parse_answer(raw, request=None):
    a = parse_json(raw)
    exact(a, ("protocol", "corpus_id", "question_id", "mode", "status", "record_count", "answer_count"), ("elapsed_ms",))
    if a["protocol"] != ANSWER or a["mode"] not in MODES or a["status"] != "ok" or any(type(a[k]) is not str for k in ("corpus_id", "question_id")):
        raise ProtocolError()
    uint(a["record_count"])
    uint(a["answer_count"])
    if a["answer_count"] != (7 if a["mode"] == "seven_answer_home" else 1):
        raise ProtocolError()
    if request is not None:
        validate_request(request)
        if any(a[k] != request[k] for k in ("protocol", "corpus_id", "question_id", "mode")):
            raise ProtocolError()
    a.pop("elapsed_ms", None)
    return a


def outcome(process_limit=False, output_limit=False, timeout=False, returncode=0,
            schema_error=False, missing=False):
    if process_limit:
        return "process_limit_exceeded", 7
    if output_limit:
        return "output_limit_exceeded", 2
    if timeout:
        return "timeout", 4
    if returncode is None or returncode != 0:
        return "signal_or_kill" if returncode is None or returncode < 0 else "candidate_nonzero_exit", 3
    if schema_error:
        return "schema_mismatch", 5
    if missing:
        return "blocked_unmeasured", 6
    return "measured", 0


def execute(argv, timeout_ns, max_capture_bytes):
    """Bound combined stdout/stderr, retaining only stdout. No shell or helper process.

    The two reader threads never retain more than max_capture_bytes combined.
    An unproven escaped descendant is NOT reported as absent. Inherited pipe handles
    cannot hold this function open indefinitely. No stdout/stderr is put in reports.
    """
    uint(timeout_ns)
    uint(max_capture_bytes)
    if not timeout_ns or timeout_ns > 300000000000 or not max_capture_bytes or max_capture_bytes > 16777216:
        raise ProtocolError()
    if type(argv) is not list or not argv or any(type(a) is not str or not a or "\x00" in a for a in argv):
        raise ProtocolError()
    start = time.monotonic_ns()
    try:
        p = subprocess.Popen(argv, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                             stderr=subprocess.PIPE, shell=False, close_fds=True)
    except OSError:
        return {"status": "candidate_nonzero_exit", "runner_exit": 3,
                "candidate_exit_code": None, "candidate_exit_method": "launch_failed",
                "termination_reason": "launch_failed", "elapsed_ns": None,
                "stdout": b"", "capture_bytes": 0}
    captured = bytearray()
    lock, exceeded = threading.Lock(), threading.Event()
    count = 0
    def drain(pipe, keep):
        nonlocal count
        try:
            while True:
                block = os.read(pipe.fileno(), 4096)
                if not block:
                    break
                with lock:
                    allowed = max(0, max_capture_bytes - count)
                    if keep:
                        captured.extend(block[:allowed])
                    count = min(max_capture_bytes + 1, count + len(block))
                    if count > max_capture_bytes:
                        exceeded.set()
                        # Wake wait() immediately; output precedence is retained even
                        # when the kill races a natural exit or timeout.
                        try:
                            p.kill()
                        except OSError:
                            pass
                        break
        except (OSError, ValueError):
            pass
        finally:
            pipe.close()
    readers = [threading.Thread(target=drain, args=(p.stdout, True), daemon=True),
               threading.Thread(target=drain, args=(p.stderr, False), daemon=True)]
    for t in readers:
        t.start()
    timed_out = False
    remaining = max(0, timeout_ns - (time.monotonic_ns() - start)) / 1000000000
    try:
        p.wait(timeout=remaining)
    except subprocess.TimeoutExpired:
        timed_out = True
        try:
            p.kill()
        except OSError:
            pass
        p.wait()
    end = time.monotonic_ns()
    # Bound drain time even if an escaped descendant retains a pipe.
    for t in readers:
        t.join(timeout=0.1)
    pipes_open = any(t.is_alive() for t in readers)
    status, code = outcome(output_limit=exceeded.is_set(), timeout=timed_out,
                           returncode=p.returncode, missing=pipes_open)
    with lock:
        raw, size = bytes(captured), count
    return {"status": status, "runner_exit": code, "candidate_exit_code": p.returncode,
            "candidate_exit_method": "subprocess_returncode",
            "termination_reason": "timeout" if timed_out else "output_limit" if exceeded.is_set() else "signal_or_kill" if p.returncode < 0 or (os.name == "nt" and p.returncode >= 0x80000000) else "none",
            "elapsed_ns": elapsed(start, end) if code == 0 else None,
            "stdout": raw if code == 0 else b"", "capture_bytes": size}


def native_info(binary):
    with open(binary, "rb") as src:
        head = src.read(4)
        pe = False
        if head[:2] == b"MZ":
            src.seek(0x3c)
            offset = src.read(4)
            if len(offset) == 4:
                src.seek(int.from_bytes(offset, "little"))
                pe = src.read(4) == b"PE\x00\x00"
    family = platform.system().lower()
    release = platform.release()
    wsl = family == "linux" and ("microsoft" in release.lower() or bool(os.environ.get("WSL_INTEROP")) or bool(os.environ.get("WSL_DISTRO_NAME")))
    fmt = "PE" if pe else "ELF" if head == b"\x7fELF" else "unknown"
    return {"os_family": family, "os_version": release,
            "environment": "wsl" if wsl else "native", "binary_format": fmt}


def native_proof(info, claimed):
    return (claimed in ("windows_native", "linux_native") and
            info["os_family"] == claimed.split("_")[0] and info["environment"] == "native" and
            info["binary_format"] == ("PE" if claimed == "windows_native" else "ELF"))


def memory_metric(metric, observation):
    if observation is None:
        return None
    exact(observation, ("value", "method", "source", "scope"),
          ("collector_verified", "process_handle_held_until_after_exit"))
    uint(observation["value"])
    if observation["scope"] != "candidate_process":
        raise ProtocolError()
    method, source = observation["method"], observation["source"]
    sampled = {"procfs.status.VmRSS", "procfs.status.VmHWM", "win32.GetProcessMemoryInfo.WorkingSetSize"}
    if method == "reliable_peak" and source in sampled:
        raise ProtocolError("method_label_invalid")
    if metric == "small_process_resident_bytes":
        if method not in ("sampled_maximum", "reliable_resident") or source not in sampled or (source == "procfs.status.VmHWM" and method != "sampled_maximum"):
            raise ProtocolError("method_label_invalid")
        return observation["value"]
    if method == "sampled_maximum" and source in sampled:
        return None
    if method != "reliable_peak":
        raise ProtocolError("method_label_invalid")
    if source == "posix.wait_rusage.ru_maxrss":
        return observation["value"] if observation.get("collector_verified") is True else None
    if source == "win32.GetProcessMemoryInfo.PeakWorkingSetSize":
        return observation["value"] if observation.get("process_handle_held_until_after_exit") is True else None
    raise ProtocolError("method_label_invalid")


def ui_metric(trace, binding):
    if trace is None:
        return None
    exact(trace, ("candidate_binding", "method", "clock", "samples_ns"))
    if trace["candidate_binding"] != binding or trace["method"] != "ui_event_loop_trace" or trace["clock"] != "monotonic" or type(trace["samples_ns"]) is not list:
        raise ProtocolError()
    for v in trace["samples_ns"]:
        uint(v)
    return max(trace["samples_ns"]) if trace["samples_ns"] and 0 not in trace["samples_ns"] else None


def process_gate(descendants=None, collector_proven=False):
    if descendants is None or not collector_proven:
        return {"descendant_count": None, "process_tree": None, "process_budget_pass": None,
                "status": "blocked_unmeasured", "runner_exit": 6}
    uint(descendants)
    tree = uint(2 + descendants)
    return {"descendant_count": descendants, "process_tree": tree,
            "process_budget_pass": tree <= 2,
            "status": "measured" if descendants == 0 else "process_limit_exceeded",
            "runner_exit": 0 if descendants == 0 else 7}


PROCESS_COLLECTORS = {
    "windows_job_object": ("windows", "win32.JobObject.accounting.all_descendants"),
    "linux_cgroup_v2": ("linux", "linux.cgroup_v2.accounting.all_descendants"),
}


def process_observation(evidence, binding, os_family):
    """Validate a closed, bound collector claim; schema acceptance is not truth proof."""
    if evidence is None:
        return process_gate()
    exact(evidence, ("candidate_binding", "observation"))
    if evidence["candidate_binding"] != binding:
        raise ProtocolError()
    obs = exact(evidence["observation"],
                ("descendant_count", "method", "source", "scope", "collector_verified"))
    uint(obs["descendant_count"])
    if type(obs["collector_verified"]) is not bool or obs["scope"] != "process_tree":
        raise ProtocolError()
    if type(obs["method"]) is not str or obs["method"] not in PROCESS_COLLECTORS:
        raise ProtocolError("method_label_invalid")
    family, source = PROCESS_COLLECTORS[obs["method"]]
    if family != os_family or obs["source"] != source:
        raise ProtocolError("method_label_invalid")
    return process_gate(obs["descendant_count"], obs["collector_verified"])


def baseline_probe(argv, timeout_ns=1000000000, max_capture_bytes=65536):
    if argv is None:
        return {"comparison_status": "not_measurable_preport", "baseline_metrics": None}
    result = execute(argv, timeout_ns, max_capture_bytes)
    # Successful arbitrary probes are not a parsed, comparable benchmark report.
    return {"comparison_status": "not_measurable_preport" if result["runner_exit"] else "refused_noncomparable",
            "baseline_metrics": None, "probe_exit_code": result["candidate_exit_code"]}


SAMPLE_FIELDS = set("mode phase candidate_exit_code candidate_exit_method termination_reason elapsed_ns record_count answer_count capture_bytes response_schema_valid method source scope clock measurement_label".split())
REPORT_FIELDS = set("report_version manifest native candidate_binding samples instrumentation evaluation independent_acceptance".split())


def binding_for(manifest):
    return digest(canonical(manifest))


def evaluate(report):
    exact(report, REPORT_FIELDS)
    if report["report_version"] != REPORT or report["independent_acceptance"] is not False:
        raise ProtocolError()
    m = validate_manifest(report["manifest"])
    if report["candidate_binding"] != binding_for(m):
        raise ProtocolError()
    native = exact(report["native"], ("os_family", "os_version", "environment", "binary_format"))
    if any(type(v) is not str for v in native.values()):
        raise ProtocolError()
    if m["os_id"] != native["os_family"]:
        raise ProtocolError()
    claimed = native["os_family"] + "_native"
    if not native_proof(native, claimed):
        return {"status": "not_native", "runner_exit": 7, "measurement_valid": False,
                "performance_success": False, "performance_success_false_because": "not_native",
                "metrics": {}, "comparison_status": "not_measurable_preport"}
    if type(report["samples"]) is not list or len(report["samples"]) > 128:
        raise ProtocolError()
    groups = {mode: [] for mode in MODES}
    warmups, failures = 0, []
    for s in report["samples"]:
        exact(s, SAMPLE_FIELDS)
        if s["mode"] not in MODES or s["phase"] not in ("warmup", "measured") or s["candidate_exit_method"] not in ("subprocess_returncode", "launch_failed") or s["termination_reason"] not in ("none", "timeout", "output_limit", "signal_or_kill", "launch_failed"):
            raise ProtocolError()
        if (s["phase"] == "warmup") != (s["mode"] == "warmup"):
            raise ProtocolError()
        if s["method"] != "monotonic_elapsed" or s["source"] != "harness.monotonic_ns" or s["scope"] != "candidate_process" or s["clock"] != "monotonic" or s["measurement_label"] != ("process_warm_os_cache" if s["mode"] in ("warmup", "warm_answer") else "process_elapsed"):
            raise ProtocolError()
        rc = s["candidate_exit_code"]
        if rc is not None and (type(rc) is not int or not -(1 << 31) <= rc <= U64):
            raise ProtocolError()
        uint(s["capture_bytes"])
        if type(s["response_schema_valid"]) is not bool:
            raise ProtocolError()
        if s["elapsed_ns"] is not None:
            uint(s["elapsed_ns"])
        status, code = outcome(output_limit=s["termination_reason"] == "output_limit" or s["capture_bytes"] > m["max_capture_bytes"],
                               timeout=s["termination_reason"] == "timeout", returncode=rc,
                               schema_error=not s["response_schema_valid"],
                               missing=s["elapsed_ns"] is None or s["elapsed_ns"] == 0)
        if code:
            failures.append((status, code))
            continue
        uint(s["record_count"])
        uint(s["answer_count"])
        if s["answer_count"] != (7 if s["mode"] == "seven_answer_home" else 1):
            raise ProtocolError()
        if s["phase"] == "warmup":
            warmups += 1
        else:
            groups[s["mode"]].append(s["elapsed_ns"])
    inst = exact(report["instrumentation"], ("ui_trace", "small_memory", "large_memory", "decode", "descendants"))
    ui_value = ui_metric(inst["ui_trace"], report["candidate_binding"])
    memory_values = {}
    for key, name in (("small_memory", "small_process_resident_bytes"),
                      ("large_memory", "large_reliable_peak_rss_bytes")):
        obs = inst[key]
        memory_values[name] = None
        if obs is not None:
            exact(obs, ("candidate_binding", "observation"))
            if obs["candidate_binding"] != report["candidate_binding"]:
                raise ProtocolError()
            memory_values[name] = memory_metric(name, obs["observation"])
            source = obs["observation"]["source"]
            if (source.startswith("win32.") and native["os_family"] != "windows") or (source.startswith(("posix.", "procfs.")) and native["os_family"] != "linux"):
                raise ProtocolError("method_label_invalid")
    decode = inst["decode"]
    if decode is not None:
        exact(decode, ("candidate_binding", "method", "scope", "source", "retained_context_records", "max_retained_fact_payload_slots", "incremental_decode_records", "prefix_validation_bytes", "payload_io_bytes"))
        if decode["candidate_binding"] != report["candidate_binding"] or decode["method"] != "independent_decode_counter" or decode["scope"] != "candidate_process" or decode["source"] != "independent_decode_trace":
            raise ProtocolError()
        for key in ("retained_context_records", "max_retained_fact_payload_slots", "incremental_decode_records", "prefix_validation_bytes", "payload_io_bytes"):
            uint(decode[key])
    process = process_observation(inst["descendants"], report["candidate_binding"], native["os_family"])
    if process["runner_exit"] == 7:
        failures.append((process["status"], 7))
    metrics = {}
    for name in m["required_metrics"]:
        value, method, source, scope = None, "unavailable", "unavailable", "candidate_process"
        if name in ("warm_answer_p95_ns", "warm_throughput"):
            stat = statistics(groups["warm_answer"])
            if warmups != 2:
                if warmups > 2:
                    raise ProtocolError()
            elif stat["runner_exit"] == 0:
                value = stat["p95_ns"] if name == "warm_answer_p95_ns" else stat["throughput"]
            method, source = "monotonic_elapsed", "harness.monotonic_ns"
        elif name == "cold_migration":
            stat = statistics(groups["cold_migration"], "cold_migration")
            value = stat["throughput"]
            method, source = "monotonic_elapsed", "harness.monotonic_ns"
        elif name == "seven_answer_home_ns":
            value = max(groups["seven_answer_home"], default=None)
            method, source = "monotonic_elapsed", "harness.monotonic_ns"
        elif name == "ui_blocking_ns":
            value = ui_value
            method, source, scope = "ui_event_loop_trace", "independent_ui_trace", "ui_event_loop"
        elif name in ("small_process_resident_bytes", "large_reliable_peak_rss_bytes"):
            obs = inst["small_memory" if name.startswith("small") else "large_memory"]
            if obs is not None:
                value = memory_values[name]
                method, source = obs["observation"]["method"], obs["observation"]["source"]
        elif name in ("retained_context_records", "max_retained_fact_payload_slots"):
            obs = decode
            if obs is not None:
                value, method, source = obs[name], obs["method"], obs["source"]
        elif name == "process_tree":
            value, scope = process["process_tree"], "process_tree"
            if inst["descendants"] is not None:
                obs = inst["descendants"]["observation"]
                method, source = obs["method"], obs["source"]
        cap = CAPS.get(name)
        metrics[name] = {"value": value, "status": "measured" if value is not None else "blocked_unmeasured",
                         "budget_pass": None if value is None or cap is None else value <= cap,
                         "method": method, "source": source, "scope": scope,
                         "unit": "answers_per_ns" if name == "warm_throughput" else "completed_units_per_ns" if name == "cold_migration" else "bytes" if name.endswith("bytes") else "ns" if name.endswith("ns") else "count"}
    missing = any(v["value"] is None for v in metrics.values())
    if failures:
        order = {7: 0, 2: 1, 4: 2, 3: 3, 5: 4, 6: 5}
        status, code = min(failures, key=lambda x: order[x[1]])
    else:
        status, code = outcome(missing=missing)
    if code == 5:
        for metric in metrics.values():
            metric["budget_pass"] = None
    success = code == 0 and all(v["budget_pass"] is not False for v in metrics.values())
    return {"status": status, "runner_exit": code, "measurement_valid": code == 0,
            "performance_success": success,
            "performance_success_false_because": None if success else status if code else "budget_exceeded",
            "metrics": metrics, "comparison_status": "not_measurable_preport",
            "descendant_count": process["descendant_count"],
            "process_budget_pass": None if code == 5 else process["process_budget_pass"],
            "warm_statistics": statistics(groups["warm_answer"]),
            "cold_statistics": statistics(groups["cold_migration"], "cold_migration")}


def empty_report(m, native):
    return {"report_version": REPORT, "manifest": m, "native": native,
            "candidate_binding": binding_for(m), "samples": [],
            "instrumentation": {"ui_trace": None, "small_memory": None,
                                "large_memory": None, "decode": None, "descendants": None},
            "evaluation": None, "independent_acceptance": False}


def run_candidate(m, candidate, corpus, request, output):
    validate_manifest(m)
    paths = [Path(p).resolve() for p in (candidate, corpus, request, output, __file__)]
    if len(set(paths)) != len(paths):
        raise ProtocolError("path_alias")
    argv = argv_for(candidate, request)
    validate_surface(argv, m["command_surface"], m["shell"])
    here = file_digest(__file__)
    if m["candidate_binary_sha256"] != file_digest(candidate) or m["runner_sha256"] != here or m["generator_sha256"] != here or m["argv_sha256"] != digest(canonical(argv)) or m["corpus_sha256"] != file_digest(corpus):
        raise ProtocolError()
    h = hashlib.sha256()
    for line in corpus_bytes(m["profile"]):
        h.update(line)
    if h.hexdigest() != m["corpus_sha256"]:
        raise ProtocolError()
    native = native_info(candidate)
    if m["os_id"] != native["os_family"]:
        raise ProtocolError()
    report = empty_report(m, native)
    if not native_proof(native, native["os_family"] + "_native"):
        report["evaluation"] = evaluate(report)
        atomic_json(output, report)
        return report
    sequence = ["warmup"] * 2 + ["warm_answer"] * 20 + ["cold_migration"] * 5 + ["replay", "seven_answer_home"]
    for mode in sequence:
        req = {"protocol": ANSWER, "corpus_id": corpus_id(m["profile"]),
               "question_id": f"attempt-{m['profile']}-{SEED}-00000", "mode": mode}
        atomic_json(request, req)
        result = execute(argv, m["timeout_ns"], m["max_capture_bytes"])
        counts = {"record_count": None, "answer_count": None}
        response_valid = result["runner_exit"] == 6
        if result["runner_exit"] == 0:
            try:
                answer = parse_answer(result["stdout"], req)
                counts = {k: answer[k] for k in counts}
                response_valid = True
            except ProtocolError:
                pass
        sample = {k: result[k] for k in ("candidate_exit_code", "candidate_exit_method", "termination_reason", "elapsed_ns", "capture_bytes")}
        sample.update(counts, mode=mode, phase="warmup" if mode == "warmup" else "measured",
                      response_schema_valid=response_valid, method="monotonic_elapsed",
                      source="harness.monotonic_ns", scope="candidate_process", clock="monotonic",
                      measurement_label="process_warm_os_cache" if mode in ("warmup", "warm_answer") else "process_elapsed")
        report["samples"].append(sample)
        if result["runner_exit"] or not response_valid:
            break
    report["evaluation"] = evaluate(report)
    atomic_json(output, report)
    return report


def compare_reports(candidate, baseline):
    ce = evaluate(candidate)
    if baseline is None:
        return {"comparison_status": "not_measurable_preport", "regression": None}
    be = evaluate(baseline)
    if not be["measurement_valid"]:
        return {"comparison_status": "not_measurable_preport", "regression": None}
    if not ce["measurement_valid"]:
        return {"comparison_status": "refused_noncomparable", "regression": None}
    def context(r, mode):
        m, n = r["manifest"], r["native"]
        return dict(cohort="v080-synthetic", command_surface=m["command_surface"], work_mode=mode,
                    unit="answers_per_ns" if mode == "warm_answer" else "completed_units_per_ns",
                    **{k: n[k] for k in ("os_family", "os_version", "environment")},
                    **{k: m[k] for k in ("compiler_id", "storage_id", "corpus_sha256", "corpus_version", "profile", "seed", "clock")},
                    metric_method="monotonic_elapsed")
    results = {}
    for metric, mode in (("warm_throughput", "warm_answer"), ("cold_migration", "cold_migration")):
        cv, bv = ce["metrics"].get(metric, {}).get("value"), be["metrics"].get(metric, {}).get("value")
        results[metric] = regression(cv["numerator"], cv["denominator"], bv["numerator"], bv["denominator"], comparable(context(candidate, mode), context(baseline, mode))) if cv and bv else {"comparison_status": "refused_noncomparable", "regression": None}
    return results


def summarize_runs(reports):
    """Revalidate all four profiles; no summary boolean bypasses per-run checks.

    full_acceptance is a computational gate over bound report claims. It does
    not certify collector truth or confer independent governance acceptance.
    """
    required = {(p, n) for p in ("small", "large") for n in ("windows", "linux")}
    present, invalid = set(), False
    candidate_source, corpus_by_profile = None, {}
    for r in reports:
        e = evaluate(r)
        p, n = r["manifest"]["profile"], r["native"]["os_family"]
        key = (p, n)
        if key not in required or key in present:
            raise ProtocolError()
        present.add(key)
        source = r["manifest"]["candidate_source_sha256"]
        if candidate_source is not None and source != candidate_source:
            raise ProtocolError("candidate_source_mismatch")
        candidate_source = source
        corpus = r["manifest"]["corpus_sha256"]
        if p in corpus_by_profile and corpus_by_profile[p] != corpus:
            raise ProtocolError("profile_corpus_mismatch")
        corpus_by_profile[p] = corpus
        metrics = {"cold_migration", "process_tree"}
        metrics |= {"warm_answer_p95_ns", "seven_answer_home_ns", "ui_blocking_ns", "small_process_resident_bytes"} if p == "small" else {"large_reliable_peak_rss_bytes", "retained_context_records", "max_retained_fact_payload_slots"}
        if not metrics <= e["metrics"].keys() or not e["performance_success"] or r["evaluation"] != e:
            invalid = True
    complete = present == required and not invalid
    return {"measurement_complete": complete, "full_acceptance": complete,
            "independent_acceptance": False, "status": "independent_review_required" if complete else "blocked_unmeasured",
            "runner_exit": 0 if complete else 6,
            "missing_runs": sorted(p + "-" + n for p, n in required - present),
            "missing_value": None}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="action", required=True)
    gen = sub.add_parser("generate", help="Stream frozen synthetic corpus, atomically")
    gen.add_argument("--profile", choices=("micro", "small", "large"), required=True)
    gen.add_argument("--output", required=True)
    prep = sub.add_parser("prepare", help="Pin a manifest for explicit candidate and request paths")
    for name in ("candidate", "corpus", "request", "output", "source-sha256", "compiler-id", "storage-id"):
        prep.add_argument("--" + name, required=True)
    prep.add_argument("--profile", choices=("micro", "small", "large"), required=True)
    prep.add_argument("--required-metrics", nargs="+", choices=sorted(METRICS), required=True)
    prep.add_argument("--timeout-ns", type=int, default=10000000000)
    prep.add_argument("--max-capture-bytes", type=int, default=65536)
    run = sub.add_parser("run", help="Run only the frozen v080 argv, without a shell")
    for name in ("manifest", "candidate", "corpus", "request", "output"):
        run.add_argument("--" + name, required=True)
    ev = sub.add_parser("evaluate", help="Strictly validate/recompute a report; never accept product")
    ev.add_argument("--report", required=True)
    ev.add_argument("--baseline")
    ev.add_argument("--instrumentation", help="Independent, candidate-bound instrumentation JSON; requires --output")
    ev.add_argument("--output", help="Write a new atomically evaluated report")
    summary = sub.add_parser("summarize", help="Revalidate four native reports; never self-accept")
    summary.add_argument("--reports", nargs="+", required=True)
    args = parser.parse_args(argv)
    try:
        if args.action == "generate":
            print(canonical(generate(args.profile, args.output)).decode())
            return 0
        if args.action == "prepare":
            if Path(args.output).resolve() in {Path(p).resolve() for p in (args.candidate, args.corpus, args.request, __file__)}:
                raise ProtocolError("path_alias")
            m = {"manifest_version": MANIFEST, "candidate_binary_sha256": file_digest(args.candidate),
                 "candidate_source_sha256": args.source_sha256, "corpus_sha256": file_digest(args.corpus),
                 "generator_sha256": file_digest(__file__), "runner_sha256": file_digest(__file__),
                 "os_id": platform.system().lower(), "compiler_id": args.compiler_id, "storage_id": args.storage_id,
                 "argv_sha256": digest(canonical(argv_for(args.candidate, args.request))),
                 "command_surface": "v080", "shell": False, "seed": SEED, "profile": args.profile,
                 "corpus_version": CORPUS, "max_capture_bytes": args.max_capture_bytes,
                 "timeout_ns": args.timeout_ns, "required_metrics": args.required_metrics, "clock": "monotonic"}
            validate_manifest(m)
            atomic_json(args.output, m)
            return 0
        if args.action == "run":
            r = run_candidate(load_json(args.manifest), args.candidate, args.corpus, args.request, args.output)
            print(canonical(r["evaluation"]).decode())
            return r["evaluation"]["runner_exit"]
        if args.action == "summarize":
            summary = summarize_runs([load_json(p) for p in args.reports])
            print(canonical(summary).decode())
            return summary["runner_exit"]
        r = load_json(args.report)
        if args.instrumentation:
            if not args.output:
                raise ProtocolError()
            r["instrumentation"] = load_json(args.instrumentation)
        e = evaluate(r)
        if not args.output and r["evaluation"] != e:
            raise ProtocolError("evaluation_mismatch")
        if args.output:
            r["evaluation"] = e
            atomic_json(args.output, r)
        if args.baseline:
            try:
                baseline = load_json(args.baseline)
            except FileNotFoundError:
                baseline = None
            comparisons = compare_reports(r, baseline)
            e = dict(e, comparisons=comparisons)
            e["comparison_status"] = comparisons.get("comparison_status", "comparable" if any(type(v) is dict and v.get("comparison_status") == "comparable" for v in comparisons.values()) else "refused_noncomparable")
            if any(type(v) is dict and v.get("exceeded") is True for v in comparisons.values()):
                e["performance_success"] = False
                e["performance_success_false_because"] = "comparable_regression_exceeded"
        print(canonical(e).decode())
        return e["runner_exit"]
    except ProtocolError as exc:
        print(canonical({"status": exc.status, "runner_exit": exc.code,
                         "performance_success": False, "independent_acceptance": False}).decode())
        return exc.code
    except (OSError, ValueError, TypeError, KeyError):
        print(canonical({"status": "schema_mismatch", "runner_exit": 5,
                         "performance_success": False, "independent_acceptance": False}).decode())
        return 5


if __name__ == "__main__":
    sys.exit(main())
