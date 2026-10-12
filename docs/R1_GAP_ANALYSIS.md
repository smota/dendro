# R1 Gap Analysis — candidate 63cebb0 vs Amendment 1

| Field | Value |
| --- | --- |
| Owner | `pm.analyst` |
| For | `pm.manager` → sponsor re-scope decision (`qitem-20261010193859-2397176d7b7be3bb`) |
| Contract | `docs/ACCEPTANCE_R1.md`, Amendment 1, sha256 `046c1722…`. Not edited. |
| Candidate | `63cebb0` on `work/dendro-r1` |
| Method | Read-only: frozen wording, candidate source, CLI help, and the QA fixture DB (`qa/63cebb0`). I ran no real-source ingest. "Unknown" means no exact-SHA proof exists, not that it fails. |
| Date | 2026-10-10 |

## Conclusion

**63cebb0 is not releasable as v0.1.0. Three ACs have confirmed gaps against the frozen wording (AC6, AC9, AC10). Two have small gaps (AC3, AC7). Six have no exact-SHA proof yet. Only M4 exposure and the loop-threshold setting can move to v0.2 without breaking the job, and both need a recorded sponsor re-scope because the frozen ACs name them.**

## Table

| AC | Status | Evidence (63cebb0) | Gap | Move to v0.2? |
| --- | --- | --- | --- | --- |
| AC1 First run ≤ 5 min, sources found/missing with counts | unknown | `run` returns a `sources` summary. No exact-SHA timing of ingest **plus first report**. An earlier 77 s ingest was on another SHA and did not time the report. | S (proof only) | **No.** O1 is the adoption promise, and the proof is cheap. |
| AC2 Unknown format warns, totals unchanged | unknown | No exact-SHA proof seen. | S (proof only) | **No.** It guards against wrong numbers. |
| AC3 Linking | gap | Per-session `links` with `status` and `evidence`: met. The link rate per harness is **not emitted**: `link_rate()` exists in the resolver but the pipeline never outputs it. | S | No. G1.2 promises "the link rate appears". Trivial to fix. |
| AC4 Q1 activity bands | unknown (fixture met) | Five bands with `sum` equal to the window on the fixture (`60 = 10 + 50`). Real-data and edge fixtures (carry-in, mid-window seat, destination change) not proven on this SHA. | S (proof) | No. Q1 is the job. |
| AC5 Q2 handoffs | unknown (fixture met) | Edges with `return`, intake `estimate` / reason, pair medians with samples, and `m3_paths`. Not proven on real data. | S (proof) | No. Q2 is the job. |
| AC6 Q3 tokens and loops | **gap** | (a) Mission buckets exist only **rig-wide**, so the per-seat check "missions + no mission + ambiguous = seat total" cannot be run. (b) Tokens from **unlinked** sessions and from records **without a timestamp** are folded into `no_mission`. §4.1 says records without a timestamp stay out of every window and count only in the session total, with a visible count. (c) No per-category or per-session totals in the report (categories appear only in trace rows). (d) Loop thresholds are hard-coded (`tool_loops(&tools, 3, 600)`). AC6's last bullet tests a "threshold changed by the operator", and A8 says they are configurable. | M | (a)–(c) **No.** "Tokens by mission and seat" is O4. (d) **Yes**, with a re-scope: the hard-coded defaults equal the frozen values, so the answers stay correct. Only tuning is lost. |
| AC7 Trace | **gap** | `trace` exists for activity, tokens, intake, medians and m1–m4, with formula, version and input rows. SQLite inputs carry `table:key` (for example `queue_transitions:3`), which meets the wording. File inputs carry an opaque key (`claude:0:1:1`). The path and generation exist in `source_files` but are **not shown** in the trace. Recomputation after append, reset or row change is not proven. | S–M | **No.** Non-negotiable (O5). |
| AC8 Re-run on a frozen copy | unknown | `run --frozen` exists. No exact-SHA proof. | S (proof) | No. O6. |
| AC9 Decision metrics | **gap** | **M1** is one rig-wide number (`m1_yield = total tokens / total non-returned handoffs`). The contract defines it **per seat**, plus tokens per closed item. **M2**: the return rate is rig-wide; per pair there are only medians, no per-pair return rate. **M3**: count and paths are present; the proportion is derivable. **M4**: the impact text is "insufficient data" (met), but **exposure per seat is absent**. | M | M1 per seat and M2 per pair: **No.** They are the O8 decision metrics. M4 exposure: **Yes**, with a re-scope. Impact is already "insufficient data", so exposure is an informational list and no decision depends on it. |
| AC10 Privacy | **gap (open)** | Follow-up returns are both used (0007, 0010). There is no passing QA transition. | L (unknown) | **No.** Non-negotiable. |
| AC11 Frozen copy: hash/mtime, network, output alias | unknown | Alias, symlink and hard-link refusal is implemented and has fixtures. Hash/mtime (including WAL/SHM) and network observation are not proven on this SHA. | S (proof) | **No.** Non-negotiable. |
| AC12 Live sources read-only | unknown | No exact-SHA open-mode instrumentation proof. | S (proof) | **No.** Non-negotiable. |

## QA claims checked against the frozen wording

| QA claim | Verdict |
| --- | --- |
| "Seat M1 absent" | **Correct.** M1 is "Rendimento do assento" (seat yield), defined per seat. A rig-wide number does not meet it. |
| "Skill exposure absent" | **Correct.** AC9: "M4 mostra a exposição a skills por assento" (M4 shows skill exposure per seat). |
| "Loop thresholds hard-coded 3/600" | **Correct.** AC6 tests a threshold changed by the operator; A8 and §4 rule 7 say configurable. |
| "Trace has only opaque key + numbers" | **Partly over-read.** Amended AC7 asks for "a chave do registro de origem e a geração da fonte" (the source record's key and the source generation), not literally file and line. SQLite inputs already meet it. The real gap is narrower: file-sourced inputs do not expose a resolvable path/line or their generation. O5 ("arquivo e linha", file and line) still requires that, so the gap stands, but it is smaller than stated. |

## Findings QA did not raise

1. **Unlinked and untimestamped tokens are counted as `no_mission`** (AC6 (b) above). This blurs "no mission" with "not linked" and breaks §4.1.
2. **Handoff loop (§4) is not implemented.** No AC explicitly tests it (AC6 tests tool loops only), so it is not an AC failure. If the sponsor accepts that, record it; otherwise O4's "loops detected" is half met.
3. **Out-of-scope commands ship in the R1 CLI:** `watch` (continuous mode, G2.4) and `init-nao` (Nao, G2). They are not acceptance failures, but R1 must not claim them. Recommendation: remove them from v0.1.0 or mark them experimental. This is a sponsor decision.

## Non-negotiables

I agree that AC7, AC10, AC11 and AC12 cannot move. I would add AC6 (a)–(c) and AC9 M1/M2 to the must-have list for v0.1.0. Without per-seat tokens and per-seat yield, the O8 decision ("is this seat worth it?") cannot be made, and O8 is the product test.

## Re-scope candidates

| Item | Effect if moved to v0.2 | Required record |
| --- | --- | --- |
| Loop threshold configuration (AC6 last bullet, A8) | Answers stay correct at the frozen defaults. Tuning waits. | Sponsor re-scope; Phase 1 amendment that removes the bullet from R1. |
| M4 exposure per seat (AC9) | O8 keeps M1–M3. Impact is already "insufficient data". | Sponsor re-scope; Phase 1 amendment. |
| `watch`, `init-nao` | Not acceptance items. Removal shrinks the risk surface. | Sponsor decision; delivery change. |

Any re-scope changes frozen acceptance, so it goes through a recorded Phase 1 amendment. That needs a sponsor decision, because the R1 return budget is used up.
