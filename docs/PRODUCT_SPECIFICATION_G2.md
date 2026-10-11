# Dendro G2 — Product Discovery (Phase 0 draft)

| Field | Value |
| --- | --- |
| Owner | `pm.manager` (`pm-manager@dendro-pm`) |
| Sponsor | Sam |
| Date | 2026-10-10 |
| State | Sponsor approved E1–E3 at 2026-10-10 18:57Z. Phase 0 recorded (`dendro-g2-p0`, root `goals/g2`). |
| Milestone | `v0.2.0` (GitHub `smota/dendro`) |
| Issues in scope | #7 (epic), #12, #13, #14, #16, #17, #18 |
| Depends on | R1 (`v0.1.0`, PR #11) merged and used for at least one real review (O8) |

---

## 1. Conclusion

**Decision: refine (sponsor approved 18:57Z). G2 acceptance freezes only after the sponsor's first real R1 review supplies evidence. #12 and #13 proceed to Phase 1 now.**

1. **The G2 job is clear.** After R1 shows that a number moved, the operator needs to follow up with the next question and reach a cause.
2. **The interface is still a bet.** The backlog holds two interfaces for the same job: conversational (Nao: #7, #16, #17, #18) and visual (TUI: #14). Building both in `v0.2.0` splits effort before we know which one the operator uses.
3. **One cheap step settles it.** Run the first R1 review session, then log every follow-up question the operator asks that the R1 CLI could not answer. Those questions choose the interface and set the acceptance.
4. **Two issues are independent of the interface.** #12 (rework friction) and #13 (sealed replay) deepen and harden the R1 answers. They can be specified first.

---

## 2. Job Statement

> **When** an R1 report shows me that something moved (rework is up, a seat is idle with work, a mission burned more),
> **I want to** ask the next question and the one after it without learning commands, flags or SQL,
> **so I can** reach the cause and a decision in one sitting, not abandon the investigation halfway.

### Relation to R1

| | R1 (`v0.1.0`) | G2 (`v0.2.0`) |
| --- | --- | --- |
| Question shape | Fixed: the three questions | Open: the follow-up the operator thinks of next |
| Journey step | Scan, Locate | Explain, Decide, Verify |
| Trust | Every number traces to source (AC7) | Every answer keeps that trace, and NL answers equal CLI numbers |

---

## 3. Struggling moment (hypothesis)

R1 is not yet in the operator's hands. The struggling moment below is a hypothesis built from R1 analysis and issue #12's exploratory finding. It is not observed operator behaviour yet.

```
 R1 report: "M3 build rework 22% (+9 pts)"
        │
        ▼
 "Which pair? Which mission? Since when? Was it after I changed the reviewer?"
        │
        ├── re-run CLI with new flags   → must know the flags; slow loop
        ├── open DuckDB and write SQL   → must know the schema; nobody repeats it
        └── read transcripts            → back to one seat at a time
        │
        ▼
 investigation stops at "rework is up"  →  no decision
```

Evidence that the follow-up matters: #12 reports that review-to-build rework chains were the largest token sink in an exploratory pass over local logs. The R1 metric M3 shows that rework exists. It does not answer "which pair, which files, since when".

### Forces

| Push | Pull | Anxiety | Inertia |
| --- | --- | --- | --- |
| Investigations stop at the first number | Ask in plain English; see the answer with its trace | An NL answer that is fluent but wrong | "The CLI report is enough" |

**Main design implication:** anxiety dominates. An NL interface that invents an answer once destroys trust in all answers. Honest limits ("insufficient data") are a core feature, not an edge case.

---

## 4. Observable outcome

| # | Observable change |
| --- | --- |
| P1 | In a real review session, the operator goes from an R1 number to a named cause (pair, mission, period or change) without writing SQL or reading the schema. |
| P2 | Every G2 answer shows its metric definition and a path to the source records, as R1 AC7 does. |
| P3 | For the same rig and period, G2 numbers equal R1 CLI numbers. |
| P4 | When data cannot support an answer, the operator sees "insufficient data" and the reason. No answer is invented. |
| P5 | Value signal: the operator closes at least one review with a decision or an explicit "no change", and later verifies it with a before/after comparison (the Verify step). |

---

## 5. Scope triage of the v0.2.0 issues

| Issue | Serves the job? | Recommendation |
| --- | --- | --- |
| #12 Rework friction | Yes. Deepens Explain for M3. | Specify first. Overlaps R1 M3 and the R1 handoff-loop rule. Keep it as a pair-level breakdown of M3, not a new metric. Thresholds are advisory alerts. |
| #13 Sealed replay | Yes. Protects trust (anxiety). | Specify first, reframed as sealed replay (see the issue comment). |
| #17 Interrogation patterns | Yes, if NL is the interface. | Wait for review evidence. The patterns are the acceptance skeleton. |
| #16 Semantic views | Mechanism for #17. | Architecture input, not a product requirement. Must sit on the canonical R1 schema. |
| #18 Latency benchmark | Quality bar, not a job. | Keep as a non-functional constraint inside #17. Re-measure on real data, not synthetic. |
| #14 TUI ribbons | Yes, if visual is the interface. | Wait for review evidence. Do not build in parallel with NL in `v0.2.0`. |
| #7 Epic | Container. | Rewrite its body to cite this document when Phase 0 is recorded. |

---

## 6. Constraints and anti-goals

### Constraints

1. Every R1 constraint applies: local-first, read-only on sources, no content or secrets, English, no "telemetry" wording.
2. G2 reads the canonical R1 schema. It does not define a second one.
3. NL answers never exceed what the data supports.

### Anti-goals

1. Dendro observes and diagnoses. It does not mutate agent execution (sponsor, 2026-10-10 18:56Z).
2. No engagement or "habit" metric as a success measure. Success is decisions per review.
3. No second interface in `v0.2.0` before evidence picks the first.
4. No prediction or simulation (what-if belongs to Goal 5 discovery).

---

## 7. Release intent

| Field | Value |
| --- | --- |
| Release | `v0.2.0` |
| Impact | `minor` |
| Audience | Sponsor, reference rigs `agentflow-dev` and `dendro-dev` |
| Channel | Local build. No publication. |
| Entry condition | R1 merged; one real review session logged |
| Exit criterion | P1–P5 observed |

---

## 8. Unknowns

| # | Unknown | Who resolves |
| --- | --- | --- |
| V1 | Which follow-up questions does the operator actually ask? | Sponsor, in the first R1 review. PM logs them. |
| V2 | NL (Nao) or visual (TUI) first? | Decided by V1 evidence. Sponsor confirms. |
| V3 | Can Nao guarantee "insufficient data" instead of a fluent guess? | `orch.arch` spike before acceptance freezes |
| V4 | Does #12's rework finding hold on R1's canonical M3 definition? | `pm.analyst` against R1 output |

---

## 9. Decisions needed from the sponsor

| # | Question | PM recommendation |
| --- | --- | --- |
| E1 | Accept "refine": freeze G2 acceptance only after one real R1 review session? | Yes. One session costs an hour and removes the biggest bet. |
| E2 | Allow #12 and #13 to start Phase 1 now, independent of the interface? | Yes. |
| E3 | One interface in `v0.2.0`, chosen by evidence? | Yes. The other moves to a later milestone. |

**Resolved 2026-10-10 18:57Z:** E1 yes, E2 yes, E3 yes.

---

## 10. Decision log

### Staged acceptance #12/#13 (2026-10-10 19:02Z)

**Decision: proceed, with one refine.** `docs/ACCEPTANCE_G2_STAGED.md` (sha256 `7babeb54`).

- Refine: the rework-token alert threshold applies by default to non-cache-read tokens (input + output + cache write). 99% of rework tokens are cache reads, so an all-token threshold alerts on almost every rework pair.
- V4 finding: rework is the largest *attributed* token sink (6 of 44 chains hold 46% of chain-attributed tokens). But 60% of linked tokens sit outside any in-progress item. That is larger than all rework.
- Delivery handoff for #12/#13 waits for R1 to merge.

### R1 review session agenda (draft)

1. **G-U3:** why are 60% of linked tokens outside any queue item? Real work outside the queue (orientation, operator chat, startup) or an attribution gap?
2. Walk M1–M4 on `agentflow-dev`. Log every follow-up question the CLI cannot answer (V1).
3. Record one decision or an explicit "no change" (O8).
4. **K4 limit:** OpenRig emits no per-seat stop or removal events (only `node.launched` and rig-wide `rig.stopped`). A seat removed without a rig stop looks present until the next event or the window end, so AC4 idle time can be overstated. Is that acceptable, or does it need an upstream OpenRig event?
5. **K6 limit:** M1 counts only returns inside the report window, so handoffs near the window end look slightly better than they are (less time to come back). Is that acceptable, or should the report show a "recent handoffs not yet settled" count?
6. **Busy-source limit (cycle 2):** to keep live sources read-only, Dendro copies them first; if a source is written too fast to copy consistently after 5 attempts, ingest refuses cleanly and asks for a retry. How often does that happen on a real rig, and is it acceptable?
