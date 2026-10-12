# Dendro G2 — Staged Acceptance for #12 and #13 (Phase 1)

| Field | Value |
| --- | --- |
| Owner | `pm.analyst` (`pm-analyst@dendro-pm`) |
| Input | G2 Phase 0 `dendro-g2-p0` (root `goals/g2`), `docs/PRODUCT_SPECIFICATION_G2.md` |
| Date | 2026-10-10; Revision 2 2026-10-11 22:21Z, rebased on v0.1.1 2026-10-12 06:10Z; Revision 3 2026-10-12 06:45Z; Revision 4 2026-10-12 06:40Z; Revision 5 2026-10-12 06:50Z; Revision 6 2026-10-12 06:55Z; Revision 7 2026-10-12 07:00Z |
| State | **Frozen** 2026-10-10. `pm.manager` decision 19:00Z: proceed, with one refine (rework-token alert basis, §3). **Revision 1** (19:45Z): R1 Emenda 2 knock-on, C4. #12 builds the handoff-loop detector (F12-0); G-A1 updated. **Revision 2**: R1 released (PR #11 merged 2026-10-11 22:16Z, merge commit `f37e6ad`, candidate `2abeb8f`; merge authorized by the sponsor, 2026-10-12 06:05Z, via `pm.manager`). The official release is **v0.1.1** (hotfix). Rebased on v0.1.1: the K1–K11 terms in #12; the K9 OpenRig deferrals and the source-state notices in #13 (F13-9); the measured v0.1.1 AC1 margin in F13-8 and #41 (G-A7); #40 out of scope. Changes are marked "(Rev 2)". **Revision 3** (readings confirmed by `pm.manager` 06:30Z): prerequisite slices S31 (#31) and S41 (#41) in §4A, asked by `pm.manager` 06:27Z; F12-0 corrected to the R1 text (unordered seat pair; analyst error in Rev 1/2); pair rework tokens and the handoff-loop episode and window stated (answers `orch.arch` 06:28Z). Marked "(Rev 3)". **Revision 4** (bound by `pm.manager` 06:33Z, answering `orch.arch` 06:31Z/06:32Z): pair returns taken verbatim from R1 M2; bands follow the period's M3 population; events with candidates in more than one chain are ambiguous. §4A unchanged. Marked "(Rev 4)". **Revision 5** (bound by `pm.manager` 06:43Z, answering `orch.arch` 06:43Z): self-handoffs stay in the matrix, labelled, and never form a loop. §4A unchanged. Marked "(Rev 5)". **Revision 6** (bound by `pm.manager` 06:50Z, answering `orch.arch` 06:50Z): pair membership in a rework chain uses the M3 predicate on every qualifying edge. §4A unchanged. Marked "(Rev 6)". **Revision 7** (`pm.manager` 06:53Z): rework chains with no qualifying pair are reconciled on one line. §4A unchanged. Marked "(Rev 7)". |
| Scope | #12 rework friction, #13 sealed replay, and their prerequisite slices #31 and #41 (§4A, Rev 3). Interface acceptance (#7, #14, #16, #17, #18) is not here (sponsor E1). The other v0.2.0 items (#30, #31, #33, #34, #40, #41) are separate; §6 lists how they touch this scope. |
| Base contract | R1 as released in **v0.1.1** (hotfix branch `fix/r1-hotfix-v0.1.1` at `bb13e39`, product code `45f3e59`, binary `ff0169af`; merged to main and tagged v0.1.1 by the sponsor's 06:05Z decision). Contract: Emenda 2, `docs/ACCEPTANCE_R1.md`, sha256 `cc365c88…`, plus the clarifications register `docs/ACCEPTANCE_R1_CLARIFICATIONS.md` K1–K11, sha256 `44a3032e…`. v0.1.1 changes neither. Nothing here changes either. |
| Next owner | `orch-arch@dendro-dev` |

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
| F6 (Rev 2) | R1 shipped with OpenRig treated as append-only (register K9, a sponsor scope decision). It deferred to #13: per-row source generation for OpenRig facts, reset and reused-id replacement, the "gone" notice for retention-deleted rows, and multi-generation replay. Known limits (a) and (b) shipped. File sources have the same gap for deleted or replaced files (#32, QA F3). | #13 closes the K9 deferrals and gives file and OpenRig sources one source-state model (F13-9). |

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

All R1 terms keep their released meaning (contract `cc365c88` plus register K1–K11): handoff, return, chain, rework chain (M3), handoff loop, token categories, per-event attribution. (Rev 2) In particular:
- Returns are window-bounded: a return counts only inside the same half-open window (K6).
- M3 seat roles come from recorded evidence. A chain whose only checking pass is through a role-unknown seat is an **M3 unknown chain**, never rework or non-rework (K7).
- Mission and item attribution uses the historical destination (K3, K5). "Insufficient history" is shown, never painted backward.
- A seat's tokens are its sessions linked by the single session resolver: one seat means linked; distinct seats mean a collision, which is unlinked. Unlinked tokens belong to no seat and are shown separately, as in R1.

| Term | Definition |
| --- | --- |
| Pair | An ordered pair of seats `(from → to)` with at least one handoff in the period. |
| Pair handoffs and returns (Rev 4) | Taken verbatim from R1 M2 for the period, with no re-derivation. A return counts on the returning handoff's own ordered pair, which is itself a handoff in the period, so a pair with a return always has a handoff. K6 `returned_within` (M1's mark on the original delivery) never adds to M2 and never moves a return onto another pair. Pairs absent from M2 are omitted, never shown as 0 or 0%. (Rev 5) A self-handoff `A → A` stays as its own ordered row, exactly as R1 M2 counts it, labelled "self-handoff". |
| Pair return rate | Returns on the pair ÷ handoffs on the pair. With 0 handoffs: "no data", never 0%. |
| Pair rework chains | Rework chains (M3) in which this pair carries the return into a `dev-build*` seat. |
| Carries the return (Rev 6) | Within a chain the period's M3 counts as rework, pair `(X → Y)` qualifies on **every** edge of M3's edge population where Y is a `dev-build*` seat (K7), X is not, and the chain has already passed a checking seat earlier in that population (the same K7 roles and pass rule as R1 `m3_rework`). It is M3's build-class predicate, not M2's `is_return`. `build → build` (including a build self-handoff) never qualifies. An edge before any checking pass does not qualify. An M3 unknown chain gives no pair membership. A pair counts each chain once, however many qualifying edges it has. (Rev 7) R1 M3 is unchanged and can trigger on a `build → build` or build self-handoff edge. A rework chain whose only qualifying trigger is such an edge has no pair membership. Its tokens stay in the seats' rework bands and in the distinct rework total. |
| Unpaired rework line (Rev 7) | One line in the friction output: "rework chains with no qualifying pair: N chains, T tokens". It counts existing facts and is not a new metric. Reconciliation is by chain sets, never by summing the pair column: the distinct union of all pairs' rework chains plus the unpaired chains equals R1 M3's rework chains, and their tokens, each chain once, equal the distinct rework total. |
| Rework tokens | Tokens attributed (per event, Amendment 1 rule) to items that belong to a rework chain. Shown per category. |
| Pair rework tokens (Rev 3) | All rework tokens of each of the pair's rework chains. A chain whose returns into `dev-build*` run on several pairs counts in full on each of them. The column is not additive across pairs. |
| Distinct rework total (Rev 3) | Rework tokens over distinct rework chains, each chain once. |
| Attribution bands | For each seat: tokens on rework chains + tokens on other chains + tokens on M3 unknown chains (Rev 2, K7) + ambiguous + no item = seat total. |
| Band rule (Rev 4) | Each usage event takes the band of its attributed item's chain, as the period's M3 population classifies it: rework, M3 unknown, or else **other chains (not rework or unknown in this period)**, which also covers a chain with no handoff in the period and a lone item. That label is shown as written. If the event's candidate items lie in more than one distinct chain, it goes to **ambiguous**, even when the chains share a class: never split, duplicated or guessed. Candidates all in one chain give that chain's band once. Known limit: in-period work on a chain whose handoffs all fall outside the period shows as "other chains"; the operator widens the period. |
| Friction matrix | All pairs with their handoffs, returns, return rate, rework chains and rework tokens. Pairs without handoffs are not shown as zero. (Rev 3) It never sums pair rework tokens. The column is labelled visibly "full chain, not additive across pairs" (`pm.manager` 06:30Z), and the matrix shows the single distinct rework total. Handoff loops are shown once per unordered seat pair (F12-0). |
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
- **and** (Rev 4) every pair's handoffs and returns equal R1 M2's for that ordered pair, byte for byte. Fixture: a delivery `A → B` before `from` and `B → A` inside the period gives `(B → A)` 1 handoff with R1's return count, and no `(A → B)` row,
- **and** M3 unknown chains appear as their own count with the reason, as in R1 (K7), and never inside a pair's rework chains (Rev 2),
- **and** no pair count disagrees with the R1 handoff report for the same period and the same build.

**F12-2 — Token conservation.**
- **Given** the same rig and period,
- **when** the operator asks for rework tokens,
- **then** for every seat the five attribution bands sum exactly to the R1 seat total,
- **and** (Rev 3) the distinct rework total equals the sum of all seats' rework-chain bands, and a fixture chain with returns on two pairs (`qa → build`, `review → build`) gives both pairs that chain's full tokens while the distinct total counts it once,
- **and** (Rev 6) pair membership fixtures, all on one rework chain unless stated: `qa → build-sr` then `review → build-jr` after a check gives both pairs, even if `build-jr` never touched the chain; `orch → build` after a check qualifies; `build-sr → build-jr` and `build → build` self never qualify; `orch → build` before any checking pass does not qualify; a chain whose only checking pass is through a role-unknown seat (M3 unknown) gives no pair; a pair with two qualifying edges on one chain counts that chain once,
- **and** (Rev 7) a rework chain triggered only by a `build → build` edge appears in the seats' rework bands, in the distinct rework total and in the unpaired rework line (1 chain, its tokens), with no pair row; the unpaired line is present with N = 0 when every rework chain has a pair,
- **and** (Rev 3) the text and JSON matrix carry the label "full chain, not additive across pairs" on the pair rework-token column, and no output sums that column,
- **and** each usage event counts in exactly one band,
- **and** (Rev 4) fixtures: an event on a rework chain with no handoff in the period lands in "other chains (not rework or unknown in this period)"; an event whose candidates lie in two rework chains lands in ambiguous and in neither chain's pair tokens; an event with two candidate items in one chain lands in that chain's band once,
- **and** unlinked and collision sessions count in no seat, and their total equals the R1 unlinked total (Rev 2),
- **and** the report shows the "no item" share next to the rework share.

**F12-3 — Categories.**
- **Given** the same request,
- **then** rework tokens appear per category (non-cache input, output, cache read, cache write), and no single headline number hides the cache share.

**F12-4 — Small samples and empty pairs.**
- **Given** a pair with fewer handoffs than the guard,
- **then** its return-rate alert shows "sample too small", not an alert,
- **and** a pair with 0 handoffs shows "no data", never 0%.

**F12-0 — Handoff-loop detector (prerequisite, built first).**
- R1 Emenda 2 (C4) moved the detector out of R1. #12 builds it to the R1 §4 definition, with no change: the same item or chain passes between the same pair of seats more than 2 times ("mesmo par de assentos"; A8 "mais de 2 voltas no mesmo par").
- (Rev 3) The pair is **unordered**: a pass in either direction counts. Rev 1 and 2 said "ordered seat pair". That was an analyst error, not the R1 text.
- (Rev 5) The pair has two distinct seats ("passa entre o mesmo par de assentos"; a handoff goes "de um assento para outro"). A self-handoff `A → A` never forms a loop, never counts toward another pair's passes and never breaks them. It stays in the matrix (Rev 4 pair rule).
- (Rev 3) Episode: one per (chain, unordered seat pair). An item with no chain is its own chain. Passes of different items in the same chain add up. Count = passes between the pair in either direction. Loop when count > 2.
- (Rev 3) Window: only passes whose handoff event lies inside the report period `[from, to)` count, the same basis as handoffs, returns and M2 (K6, R1 §4.1). A period's loops do not change when the report runs later. Known limit: a loop that straddles a window edge can fall below 3 passes in each window; the operator widens the window.
- **Given** a fixture where one item passes between the same pair 3 times,
- **then** it is one handoff loop, with the pass count and the chain path,
- **and** (Rev 3) `A → B`, `B → A`, `A → B` on one chain is one loop of 3 passes,
- **and** (Rev 3) a chain with 2 passes before `from` and 1 inside the period is not a loop in that period,
- **and** (Rev 5) `A → A` three times on one chain is no loop, and the matrix shows `A → A` ("self-handoff") with 3 handoffs; `A → B`, `B → B`, `B → A`, `A → B` is one loop of 3 passes on `{A, B}`,
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
- (Rev 2) This includes every handoff loop from F12-0. R1's tool-loop trace gap (#33) is a separate item and gives no exemption here.

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
- (Rev 2) The margin is thin. QA measured v0.1.1 (`45f3e59`, binary `ff0169af`) at 287.961 s of the 300 s budget (ingest 286.090 s, report 1.870 s), 12.0 s of headroom; `2abeb8f` measured 247.1 s. #41 restores headroom (G-A7). The receipt states the fresh-run and sealed-run time on the same reference data, and an unsealed fresh run also stays within R1 AC1.

**F13-9 — Source state and the K9 deferrals (Rev 2).**
Closes what register K9 deferred from R1, and #32, with one source-state model for file and OpenRig sources.
- **Given** OpenRig facts from more than one ingest,
- **then** each fact carries the source generation that produced it (R1 AC7 bullet "geração da fonte"), not only the revision of the latest ingest (K9 item 2).
- **Given** an existing analytics database and an `openrig.sqlite` that was reset or replaced, so that new rows reuse earlier keys,
- **when** the operator runs ingest,
- **then** Dendro detects the replacement and either ingests the new rows as a new generation or refuses with a named reason. It never silently drops them (K9 item 3, known limit (a)),
- **and** R1 AC7 recomputation after a reset and R1 AC8 after a replacement with reused keys pass for OpenRig, as they already do for file sources.
- **Given** OpenRig rows deleted by retention after ingest (transitions, snapshots), or a file source that was deleted or replaced (#32),
- **then** totals do not change (history is kept),
- **and** the trace marks each such source or row as no longer available or superseded, with its generation (K9 item 4, known limit (b), #32).
- **Given** a past OpenRig row that changed in place (W3; it does not occur in OpenRig 0.6.6),
- **then** a sealed run reports a mismatch (F13-4), and an unsealed run says the source changed (K9 item 1). The delivery squad re-checks OpenRig's write paths for the installed version.
- Privacy: identifiers, keys, generations and digests only; paths follow the AC10 rule (F13-7).

## 4A. Prerequisite slices for #13 (Rev 3)

Two small slices that land before or with #13 (G-A6, G-A7). Each is its own delivery and can ship on its own. Neither changes a definition, a number on correct input, the schema, a command or an output format, except the new refusal message in S31. Baseline: v0.1.1 (`a0671a91`, tag `v0.1.1`).

### S31 — One OpenRig root per analytics database (#31)

Turns register K9 known limit (d) from a user rule into a refusal. It adds no multi-root support (#27, v0.3.0).

| Term | Definition |
| --- | --- |
| OpenRig root | The `openrig.sqlite` file named by `--root openrig=<path>`, identified by its path after resolution (relative parts and symlinks resolved). Two spellings that resolve to the same file are one root. |
| Bound root | The OpenRig root whose facts the analytics database already holds. A database with no OpenRig facts has no bound root. The first successful run with an OpenRig root binds it. |
| Refusal | The run stops before it reads any source and writes nothing: exit status non-zero, one message, no fact, no source state, no run record, and no analytics database file is created if none existed. |

**S31-1 — Two OpenRig roots in one run.**
- **Given** any analytics database, new or existing, and a run with two `--root openrig=` values that resolve to different files, in any order and in any position among the other roots,
- **then** the run is refused,
- **and** the message says that one analytics database holds exactly one OpenRig root, names both roots, and suggests a separate analytics database per OpenRig root.

**S31-2 — A different OpenRig root on a bound database.**
- **Given** an analytics database bound to OpenRig root A,
- **when** the operator runs with OpenRig root B (a different file),
- **then** the run is refused, the message names A as the bound root and B as the one refused, and suggests a fresh analytics database for B,
- **and** a later run with A succeeds and gives totals identical to a run that never saw B.

**S31-3 — Same root, different spelling.**
- **Given** a database bound to A, and a run that names A by another spelling (relative path, `./` or `..` parts, a symlink),
- **then** the run is not refused,
- **and** totals, AC1 record counts and AC7 source keys equal those of a run with the original spelling (R1 AC8).

**S31-4 — What is not refused.**
- A first OpenRig root on a database that has none binds it and runs normally.
- A run with no OpenRig root on a bound database runs normally.
- More than one Claude, Codex or Pi root keeps its R1 behaviour.

**S31-5 — A database that already holds two OpenRig roots.**
- **Given** a database built by v0.1.x with facts from two OpenRig roots,
- **when** the operator runs ingest with any OpenRig root,
- **then** the run is refused with a message that the database already mixes two OpenRig roots and its OpenRig numbers are not reliable, and suggests a fresh database,
- **and** report commands on that database keep their R1 behaviour (reading is not refused).

**S31-6 — Scope boundary and privacy.**
- An `openrig.sqlite` reset or replaced **at the same path** (K9 limit (a)) is the same root for S31. It is closed by #13 F13-9, not here. S31 claims nothing about it, and its documentation says so.
- Paths in the message follow R1 AC10: identifiers appear, and secret-shaped parts are redacted. The message holds no content.
- Release notes replace known limit (d) with the refusal. Limit (a) stays until #13.

### S41 — First-run headroom and bounded tool pass (#41)

Restores the AC1 margin lost in v0.1.1, and pays the review debt recorded at hotfix review (PM 04:30Z, `rev.review` 04:24Z; builder sizing `7abfc21b`): the tool pass copies each Claude or Pi file's current generation, parses it again, and has no cancellation check.

| Term | Definition |
| --- | --- |
| Reference corpus | The sealed agentflow-dev input, manifest `40c65db7`, 97 sources. |
| Reference machine | The 18-CPU machine used for every R1 and v0.1.1 AC1 run. |
| AC1 run | One isolated QA run, the same protocol as v0.1.1: fresh analytics database, `dendro run` over the four roots, then the first `dendro report tokens --rig agentflow-dev`. No retry, nothing heavy running on the host. |
| v0.1.1 oracle set | The outputs of binary `ff0169af` on the reference corpus: every R1 report in text and JSON, the AC7 trace output, and the per-harness tool census (calls, failed calls, loops; Codex "not read"). |

**S41-1 — Headroom.**
- **Given** the reference corpus and machine,
- **then** one AC1 run finishes in 225 s or less (25% under the 300 s budget), with ingest and report times stated,
- **and** the receipt also states the v0.1.1 time (287.961 s) and the change.
- 225 s is a release target on this machine and corpus, not a performance guarantee. The release notes say so.

**S41-2 — No number changes.**
- **Given** the reference corpus,
- **then** every output in the v0.1.1 oracle set is byte-identical to v0.1.1's, apart from fields that identify the run itself (run id, run time), which the receipt lists by name,
- **and** R1 AC8 still holds: a fresh run, an identical re-run, and a run split into appended parts (including a late disagreeing tool result) give identical totals, statuses and loops.

**S41-3 — One bounded pass.**
- Usage and tool extraction share one streaming pass over a file's bytes. No full copy of a file's current generation is held for the tool pass, and each line is parsed once.
- **Given** two synthetic Claude files of 256 MiB and 1 GiB (lines up to 1 MiB each), ingested first fresh and then with a 64 MiB append,
- **then** peak resident memory for the 1 GiB file exceeds that for the 256 MiB file by no more than 64 MiB,
- **and** the receipt states peak resident memory for the reference AC1 run next to v0.1.1's, with no increase.
- The same holds for Pi files.

**S41-4 — Cancellation inside the tool pass.**
- **Given** an ingest of the largest Claude or Pi file in the corpus (or a larger synthetic one),
- **when** SIGINT or SIGTERM arrives while that file's tool extraction is running,
- **then** the process exits non-zero within 2 s of the signal,
- **and** nothing from that file is committed (earlier files keep their R1 per-file commits),
- **and** the next run gives totals, statuses and loops identical to an uninterrupted fresh run.
- The proof is non-vacuous: the receipt shows the signal landed during the tool extraction of that file, not before or after it. A signal that lands elsewhere does not count as a pass.

**S41-5 — Boundaries.**
- No schema, command or output-format change. No threshold, definition or oracle is tuned to reach S41-1.
- QA re-runs the full R1 proof set and the privacy group on the final binary, then one isolated AC1 run.

---

## 5. Assumptions (ASSUMPTIONS I'M MAKING)

| # | Assumption | If wrong |
| --- | --- | --- |
| G-A1 | (Rev 2: confirmed) R1 merged under Emenda 2 (`cc365c88`) plus register K1–K11 (`44a3032e`): Amendment 1 semantics, minus C1–C4, with K9 deferring OpenRig items to #13 (F13-9). No register entry changes a definition #12 or #13 uses; K6, K7, K3/K5 and the session resolver are carried into §3. R1 shipped no handoff-loop detector (C4), so #12 builds it (F12-0). Loop thresholds become configurable in v0.2 (C1), and #12's advisory thresholds may share that mechanism. (Rev 2, v0.1.1) The hotfix changes values, not definitions: Claude and Pi tool calls are read under R1 §4 (a call's status comes only from its own results; usage dedupe never removes calls), Pi `write` and `edit` end intake windows, and Codex tool calls are not read (#40). Usage and token totals are unchanged from `2abeb8f`; M2 intake values and tool-loop counts changed. #12 uses handoffs, returns, M3 and token attribution, not intake windows or tool loops. | If a later R1 change alters a definition, this document returns to Phase 1. |
| G-A6 (Rev 2) | One OpenRig root per analytics database (K9 (d)). #13 seals and replays one root. #31 makes ingest refuse a second root; multi-root is #27 (v0.3.0). | If #31 slips, a sealed run over a database that has silently dropped a second root's facts reproduces an undercount. F13-1 still holds, but the numbers are wrong. Land #31 before or with #13. (Rev 3) Acceptance: S31 (§4A). |
| G-A7 (Rev 2) | #41 restores first-run headroom (target ≥ 25% on the reference corpus) with no number changing. #13 is measured after #41. | If #41 slips, a sealed or unsealed run on the reference data may exceed 300 s, and F13-8 fails. Land #41 before or with #13. (Rev 3) Acceptance: S41 (§4A). |
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
- (Rev 2) Other v0.2.0 items, each with its own acceptance: #30 (busy-source retry hint), #33 (tool-loop trace, register K11), #34 (sub-second waits), #40 (Codex command outcomes for tool loops; taking the CommandExecution item as the call unit is a definition change and needs a new register entry, analyst reading 2026-10-12 00:33Z), #38/#39 (deferred C1/C2; F12-5 may share C1). #31 and #41 have their acceptance here (§4A, Rev 3). #34 changes M2 medians, so F12-1 compares #12 with R1 on the same build. #32 is absorbed into F13-9 (`pm.manager` agreed, 2026-10-11).
- Multi-root fleet (#27, v0.3.0).

---

## 7. Unknowns

| # | Unknown | Owner |
| --- | --- | --- |
| V4 | Does #12's finding hold on canonical M3? | Resolved (§1): the direction holds; the exploratory numbers do not. |
| G-U1 | Can each OpenRig append-only table be sealed by key? | `orch.arch` |
| G-U2 | Which R1 metrics depend on mutable rows, and can they be rebuilt from transitions? | `orch.arch`. F13-5 bounds the damage. |
| G-U3 | Is the 60% unattributed share real work outside the queue, or an R1 attribution gap? | First R1 review session (sponsor, PM). Candidate input for V1. |
