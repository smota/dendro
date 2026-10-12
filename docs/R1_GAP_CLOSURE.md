# R1 Gap Closure — `docs/R1_GAP_ANALYSIS.md` items vs candidate 1623cbe

| Field | Value |
| --- | --- |
| Owner | `pm.analyst` |
| For | `pm.manager` (request 06:57Z), `orch.arch` |
| Baseline | Contract `cc365c88` + register K1–K10 `1c510aeb` |
| Candidate | `1623cbe` on `work/dendro-r1` (clean). The next candidate gets a new column. |
| Method | Read-only: candidate source (`pipeline.rs` report payload, CLI), builder receipt `receipt-1623cbe…txt`, `ac1-1623cbe-fresh/`. No Dendro run. |
| Date | 2026-10-11 |

Status key:
- **Closed (code)**: the observable exists in the candidate's output. Independent QA must still confirm it.
- **Builder evidence**: a named builder test or receipt line exists, but it is not independent proof.
- **Deferred**: removed from v0.1.0 by a recorded sponsor decision.
- **Open**: missing, or no evidence on this SHA.

## Conclusion

**Two items are open and one is unproven. Everything else is closed in code or deferred by record.** (Updated 07:20Z: the AC1 timing is now measured. AC1 stays open on source counts.)
- Open: AC6 (c) per-category and per-session token totals in the report.
- Open: AC1, records read per source, and a missing-source test.
- Unproven: AC8 re-run equality has no exact-SHA evidence.
- The proof-only items (AC2, AC4, AC5, AC10, AC11, AC12) depend on independent QA, as they always did.

## Table

| # | Gap item (from R1_GAP_ANALYSIS) | Status on 1623cbe | Evidence or reference |
| --- | --- | --- | --- |
| 1 | AC1: ingest plus first report ≤ 5 min, exact-SHA timing | Timing: builder evidence. Source counts: **Open** | `ac1-1623cbe-fresh/ac1-receipt.txt`: fresh analytics DB, sealed input (97/97 unchanged before and after), run exit 0, first report exit 0. Ingest 242.2 s, report 3.9 s, **total 246.2 s** (limit 300 s, margin 54 s). Same binary `b19e2b71` and commit before and after. New finding, missed in the original gap analysis: AC1 also requires each source "com o número de registros lidos" (with the number of records read). The run output gives file counts for Claude, Codex and Pi (`claude files=44`); only OpenRig gives records (`events=19202`). No test has a source root missing (warning, run continues). |
| 2 | AC2: unknown format warns, totals unchanged | Builder evidence | Test `partial_tail_and_unknown_line_do_not_move_totals`. QA must check that the warning is visible. |
| 3 | AC3: link rate per harness not emitted | **Closed (code)** | The report has `link_rates`, with linked and discovered counts per harness and "no data" at 0. Pi and Codex sessions now link by their line-1 id (K10, `5d6527c`). |
| 4 | AC4: real-data and edge fixtures | Builder evidence | K4 lifecycle tests (`a_stop_without_a_launch…`, `detach_status_and_unknown_events_are_not_stops`, `no_event_in_window_is_entirely_unknown`). Independent QA pending. |
| 5 | AC5: real-data proof | Builder evidence | The report has `handoff_edges`, `intakes` and `pair_medians` with samples. Independent QA pending. |
| 6 | AC6 (a): mission bands per seat with conservation | **Closed (code)** | `mission_by_seat` gives per-seat bands and a `conserved` check. |
| 7 | AC6 (b): unlinked and untimestamped tokens folded into `no_mission` | **Closed (code)** | Unlinked usage goes to a separate `unlinked` bucket, not `no_mission`. The window excludes untimestamped records; they appear as `untimestamped_count` and `untimestamped_tokens_all_time`. |
| 8 | AC6 (c): per-category and per-session totals in the report | **Open** | The report's `tokens` is one combined number. Category totals exist only in `trace tokens` and in per-record rows; per-session totals exist nowhere. AC6 bullet 1, §4 Tokens, §4.1 and §10 require them in the token report. The required observable was sent to `orch.arch` at 06:55Z. |
| 9 | AC6 (d): loop thresholds configurable | **Deferred** | Emenda 2 C1. Code uses the fixed 3 failures / 600 s, matching the contract. |
| 10 | AC7: file inputs lack path and generation in the trace | **Closed (code)** | `trace_inputs` carries `source_key`, `generation`, `line` and `path`. |
| 11 | AC7: recomputation after append, reset and changed SQLite row | File sources: builder evidence. OpenRig: **Deferred** | Builder tests `append_adds_only_new_lines_and_replacement_drops_old_generation` and `codex_append_keeps_same_generation_prior_and_replacement_resets`. For OpenRig, changed rows, resets and deleted rows are deferred by K9 (sponsor 05:50Z and 05:55Z). |
| 12 | AC8: re-run on a frozen copy | **Unproven** | No exact-SHA "re-run, no total changes" line found in the receipt. The append behaviour has builder tests (row 11). QA must run it. |
| 13 | AC9: M1 per seat plus tokens per closed item | **Closed (code)** | `m1_by_seat` with `tokens_per_closed_item`. The K2 text is printed (`k2_closed_item`), and the K5 insufficient-history count is shown. |
| 14 | AC9: M2 return rate per pair | **Closed (code)** | `m2_pairs[].return_rate`. |
| 15 | AC9: M3 proportion | **Closed (code)** | `m3_rework_proportion`, plus K7 unknown chains shown separately (`m3_unknown_chains`). |
| 16 | AC9: M4 exposure per seat | **Deferred** | Emenda 2 C2. `m4_impact` = "insufficient data" is present. |
| 17 | AC10: privacy | Builder evidence | Privacy sweep: 10 families × 35 fields, 420 assertions, 51 decoded text columns (S5). Independent QA pending. |
| 18 | AC11: frozen copy hash/mtime, network, output alias | Builder evidence (partial) | Tests `frozen_sqlite_copy_preserves_source_hashes_and_reads_wal` and `symlink_output_is_refused`. The network observer is QA-owned. |
| 19 | AC12: live sources read-only | Builder evidence (partial) | Private coherent copy (`8276f7d`). The receipt shows "data dir bytes, names and mtimes UNCHANGED". Open-mode instrumentation is QA-owned. |
| 20 | Not raised by QA: handoff loop not implemented | **Deferred** | Emenda 2 C4 (to G2 #12). The report shows only `tool_loops`, with no handoff-loop field, as C4 requires. |
| 21 | Not raised by QA: `watch` and `init-nao` in the R1 CLI | **Closed (code)** | The CLI has only `run`, `report` and `trace` (Emenda 2 C3). |
| 22 | Not raised by QA: unlinked and untimestamped counted as `no_mission` | **Closed (code)** | Same as row 7. |

## Items that are not from the gap analysis but are tracked elsewhere

These are listed so they are not lost. They are tracked in their own records.
- **Snapshot follow-up S1–S6** (`.agent-runs/follow-ups/r1-snapshot-lifecycle`, 0002).
- **K8 to K10** (register `1c510aeb`).
- **Old analytics databases:** a v1 database is refused with "unsupported analytics schema version 1" and the data directory stays unchanged (receipt). This matches §8 (no migration). The fresh-ingest need after the K10 fix should be stated in the release notes.

## Mandatory checklist for the next bounded package

These read existing text only (cc365c88 + K1–K10 1c510aeb). Nothing is a new criterion. Everything is proven on one exact SHA, with a fresh analytics DB unless stated.

### AC6 token report (row 8)
Output: the tokens report (not only the trace) must have:
1. **Per-session rows, one per session in scope.** Each row gives harness, session id (K1 sealed/display), link status (rig and seat, or "sem vínculo"), and the four category totals for the period (non-cache input, output, cache read, cache write). Reasoning is shown separately when the source records it. Each row also gives the session total, and its untimestamped records (count and tokens), outside the period.
2. **Per-category totals for the period**, rig-wide and per seat.
Proof:
1. An independent per-session aggregate (no Dendro ingestion code, §4.1 rules, K10 identities) equals every report session row, field by field, on synthetic files covering a Codex counter reset, a duplicate Claude message and cache included in input. It must also match on the sealed input.
2. Linked session totals sum exactly to each seat total; unlinked sessions sum to the `unlinked` bucket.
3. Category sums equal the combined `tokens`.
4. Report and trace agree on every session row (AC7).

### AC8 frozen re-run (row 12)
1. Ingest the sealed input into a fresh DB. Capture every total: per session, per seat, per mission band, categories, M1–M3 and link rates.
2. Re-run ingest on the same copy and capture again. Pass only if all totals are byte-identical and nothing is added.
3. Append N known lines to one Claude, one Codex and one Pi session copy, and add new OpenRig events, then re-run. Each changed total grows by exactly the independently computed value of the new lines, and nothing else changes.
4. The source manifest is unchanged before and after each step.
5. Under K9, OpenRig changed or deleted rows and reused-id replacement are not tested.

### AC1 first run (row 1)
1. The fresh-DB timing on the exact final SHA stays ≤ 300 s from ingest start to first report ready. 246.2 s is on 1623cbe; re-measure on the new SHA.
2. **The output lists each R1 source (OpenRig, Claude Code, Codex, Pi; §3 rows marked Obrigatório) as found or missing, with the number of records read**, not only files.
3. With one source root absent, the run warns, continues, and lists that source as missing.

### Carry-over (unchanged, still required in the same package)
1. The snapshot follow-up S1–S6 proof rows.
2. The S5 decoded scan, which must also cover the new per-session report fields.
3. Full independent AC1–AC12 QA on the exact SHA.
