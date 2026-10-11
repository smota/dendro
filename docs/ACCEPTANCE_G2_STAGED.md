# Dendro G2 — Staged Acceptance for #12 and #13 (Phase 1)

| Field | Value |
| --- | --- |
| Owner | `pm.analyst` (`pm-analyst@dendro-pm`) |
| Input | G2 Phase 0 `dendro-g2-p0` (root `goals/g2`), `docs/PRODUCT_SPECIFICATION_G2.md` |
| Date | 2026-10-10 |
| State | **Frozen** 2026-10-10. `pm.manager` decision 19:00Z: proceed, with one refine (rework-token alert basis, §3). **Revision 1** (19:45Z): R1 Emenda 2 knock-on, C4. #12 builds the handoff-loop detector (F12-0); G-A1 updated. Delivery handoff held until R1 merges. |
| Scope | #12 rework friction, #13 sealed replay. Interface acceptance (#7, #14, #16, #17, #18) is not here (sponsor E1). |
| Base contract | R1 Emenda 2, `docs/ACCEPTANCE_R1.md`, sha256 `cc365c88…`. Nothing here changes it. |
| Next owner | `orch-arch@dendro-dev`, after R1 merges |

This document says what #12 and #13 must prove. It does not choose files, crates, tables or queries.

---

## 1. Conclusion

**Both issues are specifiable now. Both are extensions of R1, not new metrics. #12 must drop its exploratory numbers. #13 must drop the live-hash promise and the 130-second benchmark.**

| # | Finding | Consequence |
| --- | --- | --- |
| F1 | V4: on canonical M3, rework chains are the largest **attributed** token sink in `agentflow-dev`. 6 of 44 chains (14%) hold 46% of chain-attributed tokens, 5.3× the tokens of a non-rework chain on average. | The direction of #12 holds. #12 is worth building. |
| F2 | V4: #12's figures (9 chains, 18.4M tokens, 122 chains, 216M tokens) do not reproduce under Amendment 1. | The acceptance cites no exploratory number. Dendro computes the numbers. |
| F3 | V4: 60% of linked tokens (333M of 553M) fall outside any in-progress queue item. That is larger than all rework. | "Largest token sink" is false without the word "attributed". #12 must show the unattributed share next to rework. |
| F4 | V4: 99% of rework tokens are cache tokens, read and write (only 1.05M of 100M are non-cache). Excluding only cache read, rework totals 1.64M, at 0.1–0.5M per chain. | Rework cost is mostly context re-reading. #12 shows token categories separately (§4.1 of Amendment 1), or the cost looks 100× larger than the fresh work. |
| F5 | `openrig.sqlite` holds mutable current rows (queue items) next to append-only rows (events, transitions). | #13 cannot pin a mutable row to the past. Replay must reproduce or say "not reproducible". It must never differ silently. |

V4 method: read-only script over the live sources on 2026-10-10, using Amendment 1 rules (chain root from `chain_of_record`, per-event attribution to the seat's in-progress item, Codex cumulative deltas, Claude message-id dedupe, disjoint categories). It is an analyst check, not R1 output. R1 is not yet verified (Phase 5 returned). Delivery recomputes with the R1 engine.

---

## 2. Capability map

```
 R1 (Amendment 1)
   handoff-flow ─┬─► token-burn ─► decision-metrics (M2, M3)
                 │                       │
                 └───────────────────────┴──► rework-friction   (#12)

   source-discovery ─► incremental-ingest ──► sealed-replay      (#13)
```

| ID | Capability | Depends on | Independent of |
| --- | --- | --- | --- |
| `rework-friction` | Handoff-loop detector (R1 §4 definition, moved here by R1 Emenda 2 C4), then the pair-level breakdown of M2/M3, with tokens and advisory alerts. | R1 `handoff-flow`, `token-burn`, `decision-metrics` | `sealed-replay` |
| `sealed-replay` | Re-run against a recorded cutoff and get identical totals. | R1 `source-discovery`, `incremental-ingest` | `rework-friction` |

Both read the canonical R1 schema. Neither defines a second one.

---

## 3. #12 — Rework friction

### Definitions

All R1 terms keep their Amendment 1 meaning: handoff, return, chain, rework chain (M3), handoff loop, token categories, per-event attribution.

| Term | Definition |
| --- | --- |
| Pair | An ordered pair of seats `(from → to)` with at least one handoff in the period. |
| Pair return rate | Returns on the pair ÷ handoffs on the pair. With 0 handoffs: "no data", never 0%. |
| Pair rework chains | Rework chains (M3) in which this pair carries the return into a `dev-build*` seat. |
| Rework tokens | Tokens attributed (per event, Amendment 1 rule) to items that belong to a rework chain. Shown per category. |
| Attribution bands | For each seat: tokens on rework chains + tokens on other chains + ambiguous + no item = seat total. |
| Friction matrix | All pairs with their handoffs, returns, return rate, rework chains and rework tokens. Pairs without handoffs are not shown as zero. |
| Advisory alert | A line in the report when a pair crosses a threshold. It is text only. It sends nothing and changes nothing. |

Default thresholds, all configurable by the operator:

| Alert | Default | Guard |
| --- | --- | --- |
| Return rate | > 10% | Only when the pair has ≥ 5 handoffs in the period. Below that: "sample too small". |
| Rework tokens | > 1M **non-cache-read** tokens (non-cache input + output + cache write) per pair per period | Cache read is excluded from the threshold by default and shown separately in the alert. The operator can change the basis. |
| Handoff loop | R1 §4 rule (more than 2 passes on the same pair) | #12 builds this detector first (F12-0). One detector, one rule. |

The issue's "> 10 minutes" condition is dropped. In R1, 10 minutes belongs to the tool-loop rule, not to handoffs.

### Acceptance criteria

**F12-1 — Consistency with R1.**
- **Given** `agentflow-dev` and a period,
- **when** the operator asks for the friction matrix,
- **then** the sum of pair handoffs equals R1 total handoffs, the sum of pair returns equals R1 returns (M2), and the count of distinct rework chains equals R1 M3,
- **and** no pair count disagrees with the R1 handoff report for the same period.

**F12-2 — Token conservation.**
- **Given** the same rig and period,
- **when** the operator asks for rework tokens,
- **then** for every seat the four attribution bands sum exactly to the R1 seat total,
- **and** each usage event counts in exactly one band,
- **and** the report shows the "no item" share next to the rework share.

**F12-3 — Categories.**
- **Given** the same request,
- **then** rework tokens appear per category (non-cache input, output, cache read, cache write), and no single headline number hides the cache share.

**F12-4 — Small samples and empty pairs.**
- **Given** a pair with fewer handoffs than the guard,
- **then** its return-rate alert shows "sample too small", not an alert,
- **and** a pair with 0 handoffs shows "no data", never 0%.

**F12-0 — Handoff-loop detector (prerequisite, built first).**
- R1 Emenda 2 (C4) moved the detector out of R1. #12 builds it to the R1 §4 definition, with no change: the same item or chain passes between the same ordered seat pair more than 2 times.
- **Given** a fixture where one item passes between the same pair 3 times,
- **then** it is one handoff loop, with the pass count and the chain path,
- **and** a fixture with exactly 2 passes is not a loop,
- **and** passes between different pairs on the same chain do not add up into one loop,
- **and** every loop traces to its queue transitions (R1 AC7).
- F12-5 starts only after F12-0 passes.

**F12-5 — Advisory alerts.**
- **Given** fixtures where one pair is just above and one just below each threshold,
- **and** a pair with > 1M cache-read tokens but < 1M non-cache-read tokens,
- **when** the report runs,
- **then** only the pair above the threshold gets an alert, and the cache-read-heavy pair gets no rework-token alert by default,
- **and** every rework-token alert shows cache read as a separate number,
- **and** changing a threshold in configuration changes the result,
- **and** the run sends no message, writes no queue item and touches no agent.

**F12-6 — Trace.**
- **Given** any number in the friction matrix,
- **then** R1 AC7 applies unchanged: definition, version, inputs, numerator and denominator, recomputable from the trace output.

**F12-7 — Surface.**
- The matrix and alerts are available in the R1 CLI report output, including its JSON form.
- Nao, TUI and any other interface are out of scope here (sponsor E1, E3).

---

## 4. #13 — Sealed replay

### Definitions

| Term | Definition |
| --- | --- |
| Cutoff manifest | A record, written at the start of a run, of exactly what that run read. For each file: a source identifier, the byte length read, and a digest of the bytes read. For each SQLite source: the high-water key of each append-only table read. |
| Sealed run | A run that reads only what a given manifest names: no byte past each file's cutoff and no row past each table's high-water key. |
| Mismatch | A named byte range is no longer identical (file truncated, replaced or rewritten below the cutoff), or a named row is gone. |
| Not reproducible | A number that depends on a mutable current row (for example, a queue item's current state) that the manifest cannot pin. |

### Acceptance criteria

**F13-1 — Identical totals.**
- **Given** a run that wrote manifest M, and sources that kept growing afterwards,
- **when** the operator runs a sealed run against M,
- **then** every total equals the total from the original run, bit for bit.

**F13-2 — Live growth excluded.**
- **Given** a source file that gains new complete lines and one partial line after M was written,
- **when** a sealed run against M executes,
- **then** none of the new bytes count, including the partial line.

**F13-3 — Unsealed runs still grow.**
- **Given** the same sources,
- **when** the operator runs without a manifest,
- **then** R1 AC8 holds: the new lines enter, and totals grow by exactly their value.

**F13-4 — Mismatch is refused.**
- **Given** a manifest M and a source whose bytes below the cutoff changed, or a named row that disappeared,
- **when** a sealed run against M executes,
- **then** it stops with a "mismatch" result that names the source and the reason,
- **and** it reports no totals for the affected metrics. It never reports silently different numbers.

**F13-5 — Mutable rows.**
- **Given** a metric that depends on a mutable current row,
- **when** a sealed run executes,
- **then** the metric is either reproduced from append-only records or shown as "not reproducible: mutable source", with the reason.

**F13-6 — Read-only and honest.**
- **Given** live sources with agents writing to them,
- **then** R1 AC12 applies unchanged: every source opens read-only, with the `-shm` exception,
- **and** neither the product nor its documentation claims that live source hashes or mtimes stay unchanged.

**F13-7 — Privacy.**
- **Given** the R1 AC10 fixtures,
- **then** the manifest holds no content: only identifiers, lengths, keys and digests,
- **and** path identifiers follow the AC10 secret-redaction rule.

**F13-8 — Cost.**
- **Given** the R1 reference data,
- **then** a sealed run finishes within the R1 AC1 budget (5 minutes to the first report).
- The issue's "< 130 s for 100+ files / 20k events" benchmark is dropped. It had no stated user need beyond the R1 budget.

---

## 5. Assumptions (ASSUMPTIONS I'M MAKING)

| # | Assumption | If wrong |
| --- | --- | --- |
| G-A1 | R1 merges under Emenda 2 (`cc365c88`): Amendment 1 semantics, minus C1–C4. In particular R1 ships no handoff-loop detector (C4), so #12 builds it (F12-0). Loop thresholds become configurable in v0.2 (C1), and #12's advisory thresholds may share that mechanism. | If R1 changes a definition again, this document returns to Phase 1. |
| G-A2 | The V4 numbers come from an analyst script and will move slightly under the R1 engine. The conclusions (direction, unattributed share, cache share) are robust; the exact numbers are not acceptance values. | If the R1 engine shows rework is not the largest attributed sink, #12 still stands as a breakdown of M3, but its motivation weakens. |
| G-A3 | The 5-handoff guard and the 1M-token default are reasonable starting points. They are configurable. | Noisy or silent alerts. The operator tunes them. |
| G-A4 | Append-only OpenRig tables have a stable increasing key. | Sealing SQLite needs another mechanism. `orch.arch` confirms. |
| G-A5 | "Sealed replay" is the sponsor's intent for #13, per the PM reframe. The live-producer hash claim is withdrawn. | Return to Phase 0. |

---

## 6. Anti-goals and out of scope

### Anti-goals

1. Alerts do not act. They do not message agents, create queue items or change execution (sponsor anti-goal, 18:56Z).
2. No new metric competes with M2 or M3. #12 breaks them down.
3. No claim that live sources keep their hash or mtime.
4. No exploratory number appears as an acceptance value.
5. No second schema.

### Out of scope

- Interfaces: Nao (#7, #16, #17, #18) and TUI (#14). They wait for the first R1 review (E1).
- Alert delivery (notifications, continuous mode).
- Predictions and what-if (Goal 5).
- Explaining **why** the 60% "no item" tokens exist. It is a candidate question for the first R1 review, not part of #12.

---

## 7. Unknowns

| # | Unknown | Owner |
| --- | --- | --- |
| V4 | Does #12's finding hold on canonical M3? | Resolved (§1): the direction holds; the exploratory numbers do not. |
| G-U1 | Can each OpenRig append-only table be sealed by key? | `orch.arch` |
| G-U2 | Which R1 metrics depend on mutable rows, and can they be rebuilt from transitions? | `orch.arch`. F13-5 bounds the damage. |
| G-U3 | Is the 60% unattributed share real work outside the queue, or an R1 attribution gap? | First R1 review session (sponsor, PM). Candidate input for V1. |
