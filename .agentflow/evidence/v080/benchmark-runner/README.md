# T02-runner builder evidence

The executable runner is `scripts/benchmark-v080.py`. Python standard library only;
no dependency installation, Rust changes, real session logs, or product measurements.
Run from the repository root. Every command has `--help`.

## Commands

Generate the exact frozen micro fixture (82 records / 12,541 bytes):

```text
python scripts/benchmark-v080.py generate --profile micro --output .agentflow/evidence/v080/benchmark-runner/micro.jsonl
```

`--profile small` streams 91,250 records; `--profile large` streams 1,000,000
facts followed by 1,000 append records. Seed 8675309 and corpus version
`dendro.bench.corpus/v1` are fixed. No entropy, alternate seeds, private data, text
payloads, or whole-profile materialization. JSONL uses compact sorted UTF-8 JSON
and LF, including the final line. Identity includes profile, version, and seed.

The following examples are future operator commands, **not measurements performed**.
Substitute actual paths, a source revision SHA-256, and attributable compiler/storage
identifiers. IDs accept letters, digits, `_.:+-` only. Hashes are lowercase hex.

```text
python scripts/benchmark-v080.py prepare --candidate /absolute/native/dendro --corpus /absolute/small.jsonl --request /absolute/request.json --output /absolute/manifest.json --profile small --source-sha256 SOURCE_SHA256 --compiler-id COMPILER_ID --storage-id STORAGE_ID --required-metrics warm_answer_p95_ns warm_throughput cold_migration seven_answer_home_ns ui_blocking_ns small_process_resident_bytes process_tree
python scripts/benchmark-v080.py run --manifest /absolute/manifest.json --candidate /absolute/native/dendro --corpus /absolute/small.jsonl --request /absolute/request.json --output /absolute/report.json
python scripts/benchmark-v080.py evaluate --report /absolute/report.json
```

Native Windows uses `python` and Windows paths, e.g. `C:/bench/dendro.exe`.
Native Linux may use `python3` and ELF candidate paths. These are distinct runs;
WSL is labeled `environment=wsl` and fails any native proof. PE signature/ELF magic
and the live OS are observed by `run`; the compiler and storage IDs are operator
attributions, not autodetected facts. Cross-compilation does not supply another
OS proof. Imported reports are claims to validate, not independent proof of execution.

For large profile, require `cold_migration large_reliable_peak_rss_bytes
retained_context_records max_retained_fact_payload_slots process_tree`. A full
four-report completeness check also requires all small-profile budgets:

```text
python scripts/benchmark-v080.py summarize --reports small-windows.json small-linux.json large-windows.json large-linux.json
```

The runner always emits `independent_acceptance=false`. `summarize` revalidates
each run and the required metric set. `full_acceptance=true` computes that all
four bound reports satisfy the required budgets; the resulting status is
`independent_review_required`. This computational gate does not certify actual
native execution, collector truth, or governance acceptance. All four runs must
share one `candidate_source_sha256`; corpus hashes must match across OSs within
each profile. Native Windows/Linux binaries may have different hashes.

## Protocol and measurement rules

Candidate argv is exactly the seven-element list:

```text
[candidate, benchmark, answer, --protocol, dendro.bench.answer/v1, --request, request_path]
```

No shell, command-string splitting, environment expansion, or invented v070 CLI.
Manifest hashes pin the binary, claimed source digest, runner/generator, canonical
argv, and actual corpus bytes. Run verifies the corpus against a streamed
reconstruction. Candidate, corpus, request, output, and runner paths must be
distinct. Paths and raw stdout/stderr/argv are absent from reports.

Requests have exactly `protocol, corpus_id, question_id, mode`. Responses have
exactly those four binding fields plus `status, record_count, answer_count`.
`elapsed_ms` alone is tolerated and discarded. Counts are uint64 integers;
booleans, floats, duplicates, non-finite JSON, unknown keys, wrong bindings,
wrong answer counts, and status other than `ok` fail schema validation.
The harness assigns phase and monotonic elapsed time; stdout cannot assign either.

Each run schedules 2 warmups, 20 measured warm-answer calls, 5 cold-migration
calls, one replay, and one seven-answer-home call, stopping at failure. All select
the frozen `attempt-{profile}-8675309-00000` question. Cold completed units are
successful answer calls (one per repetition); there is no invented cold latency
cap. Warm label is `process_warm_os_cache`, including process startup and DB open.
These calls do not claim an in-process cache or physically cold storage.

Warmups enter no measured population. Positive monotonic samples only; nearest
rank p50/p95 is `(p*n+99)//100`, not interpolation. Minimum warm population is 20;
cold population is exactly 5 (six is schema failure). Max/min/spread and rational
throughput are computed from the same measured population. Absolute thresholds
use inclusive integer comparisons; display rounding never changes a decision.
Regression uses unbounded Python products and the exact 20/100 inequality.

Subprocess output is bounded **jointly** across stdout/stderr. Default 65,536
bytes, hard ceiling 16 MiB; default timeout 10 seconds, hard ceiling 300 seconds
per call. Two in-process reader threads drain output; no sampler child is spawned.
Threads are daemonized and drain joins are bounded even with inherited handles.
The harness uses a bounded `wait(timeout=remaining)` for process exit; output
readers kill immediately when the combined limit is exceeded. There is no explicit
one-millisecond poll/sleep quantization in the harness.
The candidate is killed on timeout/output overrun. Absent a whole-tree collector,
escaped descendants cannot be counted or guaranteed terminated. This limitation
is explicitly blocking for the process gate; no zero count is invented.

Exit precedence: prelaunch schema/surface/shell/pin failure 5; proven process/native
failure 7; output 2; timeout 4; nonzero/signal 3; missing measurements 6; valid
measurement report 0. A failed absolute budget still has exit 0 and
`performance_success=false`. Failure status and schema errors do not manufacture
zero-valued samples. Atomic reports use a same-directory temporary, fsync, replace.
Aggregate schema failures (exit 5) null every metric's `budget_pass`, including
metrics whose earlier eligible samples individually passed.

## Independent instruments and baseline

UI blocking cannot come from subprocess elapsed time. Memory, decode, UI, and
descendant collectors are not supplied by the candidate protocol. `run` leaves
these instruments null; that blocks each required affected metric only.

`evaluate --instrumentation INPUT.json --output NEW_REPORT.json --report REPORT.json`
imports a separate artifact and recomputes evaluation. The instrumentation object
has exactly `ui_trace, small_memory, large_memory, decode, descendants`. Null means
unavailable. Every nonnull instrument must match the manifest-derived
`candidate_binding`. See the semantic fixtures for executable object examples.
This verifies schema/binding/provenance **labels**, not collector truthfulness;
independent review of the instrument implementation and custody remains required.

UI requires `ui_event_loop_trace`, monotonic clock, and independent event-loop
samples. Memory scope must be `candidate_process`. Small resident allows
`sampled_maximum` or `reliable_resident`; Linux VmRSS/VmHWM polling is sampled only.
Large peak requires waited-child rusage in bytes or a Windows held-handle peak
collector. VmHWM polling cannot satisfy it. Source must match the reported OS.
No polling interval is claimed: this implementation runs no memory sampler.
Decode counters retain separate units for payload bytes, prefix-validation bytes,
decode records, and fact-payload slots. No conversion between these instruments.

The descendant importer accepts this closed shape, with no extra fields:

```json
{"candidate_binding":"MANIFEST_DIGEST","observation":{"descendant_count":0,"method":"windows_job_object","source":"win32.JobObject.accounting.all_descendants","scope":"process_tree","collector_verified":true}}
```

The OS-compatible method/source pairs are `windows_job_object` /
`win32.JobObject.accounting.all_descendants` on Windows and `linux_cgroup_v2` /
`linux.cgroup_v2.accounting.all_descendants` on Linux. A process-group label alone
is rejected. Counts are uint64 integers, including the computed `2 + descendants`
(harness plus candidate plus every descendant). Zero descendants yields tree 2,
budget pass; one yields tree 3, process-limit exit 7. A null instrument or a
collector not declared verified yields null count/gate and blocks a required metric.
As with memory, schema acceptance validates a bounded provenance claim and does
not establish that the collector really accounts for escaped/reparented descendants.
This slice supplies the evaluator, not a native collector implementation.

`evaluate --baseline BASELINE.json` strictly evaluates both reports, compares all
frozen environment/cohort fields, and emits exact warm/cold rational comparisons.
Absent/failed baseline probes are `not_measurable_preport`, with null metrics.
The importable `baseline_probe(argv)` executes an explicit caller-supplied pinned
command; an arbitrary successful probe alone cannot become a comparable report.
There is no v070 CLI assumption or synthetic baseline. Candidate absolute budgets
are independent of a refused comparison.

## Validation and remaining work

```text
python .agentflow/evidence/v080/benchmark-runner/verify_build.py
```

Tests execute real functions and dedicated synthetic subprocesses. The protocol
integration test uses a clearly named synthetic transport around real Python
helper processes; it does not claim to execute a native Dendro binary. Full
streaming corpus enumeration is a correctness check, not a performance run.

`case-coverage.json` accounts for all 89 frozen cases. Coverage means the test
exercises the corresponding semantic behavior, not that every prose/diagnostic
field in an oracle case is implemented verbatim. All 89 cases have semantic tests;
All 89 now have no pending expected-field amendment. The original author corrected
`failure.large_peak_not_satisfied_by_sample.expected.budget_pass` and
`failure.decode_counter_missing.expected.bound_pass`, both false to null, in frozen
SHA-256 `63252603ac39789bfab0a6c93a4372215284c3220fdef5cd47d3a0a456f7908e`.
Their amended values and null/blocked evaluator behavior are asserted in tests.
The legacy-named `oracle-amendments-pending.json` now records resolved status,
the original-author response and freeze provenance, and an empty pending list.
No builder edit was made to any oracle. The ledger maps process,
four-run, invalid-one-run, and refused-OS comparison cases to evaluator/CLI tests.
`synthetic-four-run-fixture.json` is clearly labeled synthetic; its positive
computational gate is not evidence of actual native performance.

`history/pre-review-01-rework/` retains the first candidate source, review,
receipt, coverage, test log, and other pre-rework evidence. The receipt links their
hashes and the review addressed by this rework.
`history/pre-final-oracle-rebind/` additionally preserves the rework receipt and
evidence before rebinding; the current receipt links the narrow Claude review and
original-author two-field correction. Product runner semantics are unchanged.
The verification command executes
the tests once, then compiles all current owned Python sources with explicit
temporary bytecode destinations in this evidence directory.

`build-receipt.json` records exact commands/exits, hashes, live client/session
identity evidence, and limitations. Independent review, a proving descendant
collector, instrument custody review, and actual small/large native Windows AND
Linux evidence remain outstanding. No commits, pushes, installs, oracle edits,
protected-script edits, or final acceptance are authorized by this receipt.
