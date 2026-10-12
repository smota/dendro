# R1 Amendment 2 — DRAFT (not frozen, not in force)

| Field | Value |
| --- | --- |
| Owner | `pm.analyst` |
| State | **Superseded.** Approved by the sponsor at 19:44Z and frozen into `docs/ACCEPTANCE_R1.md` as Emenda 2 (sha256 `cc365c88…`). This draft is kept for history only. |
| Base | `docs/ACCEPTANCE_R1.md`, Amendment 1 |
| Trigger | Gap analysis on `63cebb0`; `pm.manager` recommendation 19:41Z |

When accepted, these changes are applied to `docs/ACCEPTANCE_R1.md` as "Emenda 2", and Phase 1 is appended again under sponsor authority, because the R1 return budget is used up.

## Changes

| # | Clause in Amendment 1 | Change | Why the job still holds |
| --- | --- | --- | --- |
| C1 | A8 and §4 tool-loop rule 7 ("os limiares são configuráveis pelo operador", the thresholds are configurable by the operator); AC6 last bullet ("limiar alterado pelo operador", threshold changed by the operator) | R1 uses the fixed defaults: 3 failures, 10 minutes, more than 2 passes. Operator configuration moves to v0.2. AC6 keeps every boundary case (exactly 10:00, intervening success, different tool or input, other session, 4 failures = one episode) and drops only the "threshold changed" case. | The answers are correct at the frozen defaults. Only tuning waits. |
| C2 | AC9 bullet "M4 mostra a exposição a skills por assento" (M4 shows skill exposure per seat); §4 M4 row | R1 shows M4 as "insufficient data" with the reason (unchanged). Exposure per seat moves to v0.2. | Impact was already unmeasurable. No O8 decision depends on the exposure list. |
| C3 | (new scope boundary, §7) | The R1 CLI exposes only ingest, report and trace. `watch` and `init-nao` are not part of v0.1.0. | They are G2 surfaces (G2.4, Nao). An untested surface is also AC10 exposure. |
| C4 | §4 "Loop de handoff" (handoff loop); O4 | The handoff-loop detector is **not** required in v0.1.0. It becomes a v0.2 **prerequisite** of #12, which reuses it (`docs/ACCEPTANCE_G2_STAGED.md`). R1 reports show no handoff-loop field rather than an empty list that reads as "no loops". | R1 O4 is met by tool loops plus M3 rework chains. See the note on placement. |

## Unchanged and confirmed as must-have for v0.1.0

AC1–AC5, AC7, AC8, AC10, AC11 and AC12 as written. AC6 (a)–(c): per-seat mission bands with per-seat conservation; untimestamped records excluded from windows and counted visibly; unlinked tokens separate from "no mission"; per-category and per-session totals. AC9: M1 per seat (plus tokens per closed item), M2 per pair (return rate, median wait, median intake cost), and M3.

## Note on C4 placement

My recommendation is v0.2, as a prerequisite owned by #12.

- **For v0.1.0:** the detector is small (S), and §4 already defines it, so it is not new scope.
- **For v0.2:** R1 has no return budget left and has open gaps in AC6, AC9 and AC10. Adding an implementation item to a failing candidate raises the risk of another failed QA round. #12 needs the detector and will test it under F12-5.

If the sponsor prefers v0.1.0, C4 becomes: "AC6 adds: a test case where the same item passes between the same pair 3 times is one handoff loop, and 2 passes is not."

## Knock-on for G2

`docs/ACCEPTANCE_G2_STAGED.md` assumes the R1 detector exists (G-A1, §3 alert table). With C4 in v0.2, G2 Phase 1 needs one amendment: #12 builds the handoff-loop detector to the Amendment 1 §4 definition before F12-5. Under G-A1 this goes back through G2 Phase 1, which on the G2 record means a `pm.manager` phase 0 append first (see the record route). It adds no new metric.

## Record route (checked 2026-10-10 against `agentflow-sdlc` `lib/core/phase-graph.mjs`, read-only)

**Phase 1 cannot be appended again on the parent R1 root. The carrier is a new legacy follow-up record under sponsor authority.**

| Root | State | Can Phase 1 be appended? |
| --- | --- | --- |
| Parent R1 (`/home/sam/code/dendro`) | Latest is phase 5 skipped with no return phrase, so `allowedNext` = [6]. Phase 6 can only return to 4, and the 6→4 cycles are at 2, so the validator throws "Return cycle limit reached". | **No.** Phase 1 is unreachable. |
| AC10 follow-up (`.agent-runs/follow-ups/ac10-legacy-spec`) | Latest is 4 pass, so `allowedNext` = [5, 3, 1]. 4→1 cycles = 0. | **Mechanically yes, but wrong record.** It is sealed to the AC10 capability ("Excludes changed acceptance formulas"). Amendment 2 is R1-wide (AC6, AC9, CLI scope). |
| **New legacy follow-up** (proposed: `.agent-runs/follow-ups/r1-amendment2`) | Fresh record. | **Yes.** `pm.manager` phase 0 (reused R1 intent, skipped with reason, as in the 19:18Z AC10 precedent), then `pm.analyst` phase 1 = Amendment 2, with a fresh 2-return budget. It links to the parent R1 goal, the AC10 follow-up and the gap analysis. Parent history stays unchanged. |
| G2 (`goals/g2`) | Latest is 1 pass, so `allowedNext` = [2, 0]. | Only after a `pm.manager` phase 0 append. Needed for the C4 knock-on. |

Needs from the sponsor: approval of the new follow-up record (as at 19:18Z), and whether the AC10 follow-up continues separately or is absorbed by the new one.
Confirmation from `orch.arch` (19:43Z, read-only, using the node CLI): routes 1–3 agreed, with these conditions:
1. The new record needs explicit sponsor authorization for an R1-wide carrier with a fresh budget, and for the actual Amendment 2 criteria. Earlier gates (`cdbb68a40b99`, `799dd427c955`) are linked as history only and are never presented as approval of changed criteria.
2. Links: parent `transitions/0014.json`, AC10 record `transitions/0011.json`, the failed QA evidence, the prior gates and the new sponsor disposition.
3. No `--kind` (avoids the known altitude/subject gate mismatch). Consent is never inferred from the legacy route.
4. Sequence: PM 0 (reused intent) → analyst 1 (freeze the full new contract digest) → architect 2/3 with council and full proof → builder 4 → QA 5 on all amended ACs → review 6. Parent phase 6 final acceptance must cite the new normative amendment and the preserved failures.
5. Amendment 1 stays in force until the new freeze.
G2 route re-read live at 19:44Z: `goals/g2` is at phase 1, `allowedNext` = [2, 0], as stated above.

Tooling note: since 19:38Z, `~/.local/bin/agentflow-sdlc` fails with "Permission denied". The upstream checkout `230426e` left `bin/cli.mjs` at mode 644. Workaround: `node /home/sam/code/agentflow-sdlc/bin/cli.mjs`. Nothing was changed by this seat.
