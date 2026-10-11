# Dendro — Goals Hierarchy, Data Flows and Correction Cycles

**Author:** Sponsor (Sam / Antigravity)  
**Recipients:** `orch-arch@dendro-dev`, `dev-build*`, `dev-qa`, `rev-review`  
**Reference:** Release 1 (v0.1.0) — Epic #1  
**Status:** Normative Architectural Directive (Governed by Amendment 1 in `docs/ACCEPTANCE_R1.md`)  
**Sponsor Decision:** Semantic definitions of M1–M4 and conservation rules follow frozen Amendment 1. The 5-flow hierarchy governs delivery cadence and isolation of verification cycles.

---

## 1. Overview: Why Hierarchy is Vital

The development of Dendro Release 1 **must not be a monolithic code dump**. To guarantee predictability, a steady delivery heartbeat, and **short, isolated correction cycles**, the architecture is decomposed into **5 Linear and Incremental Data Flows**.

If any flow fails QA verification, the correction cycle is immediate and isolated to that flow's scope, without contaminating other components.

```
 [Raw Local Sources] (OpenRig, Claude Code, Codex, Pi, Grok, AGY)
        │
        ▼
 ┌────────────────────────────────────────────────────────┐
 │ FLOW 1: Streaming Ingestion & Incremental Watermarks   │ ──► Cycle 1: Ingestion & Integrity Proof
 └────────────────────────────────────────────────────────┘
        │
        ▼
 ┌────────────────────────────────────────────────────────┐
 │ FLOW 2: Deterministic Session-to-Seat Linking          │ ──► Cycle 2: Linking Proof (>= 95% Host Rate)
 └────────────────────────────────────────────────────────┘
        │
        ▼
 ┌────────────────────────────────────────────────────────┐
 │ FLOW 3: Three Core Operational Answers (Q1, Q2, Q3)    │ ──► Cycle 3: 3 Questions Proof (O2, O3, O4)
 └────────────────────────────────────────────────────────┘
        │
        ▼
 ┌────────────────────────────────────────────────────────┐
 │ FLOW 4: Executive Decision Metrics (M1 to M4)          │ ──► Cycle 4: Operator Value Proof (O8)
 └────────────────────────────────────────────────────────┘
        │
        ▼
 ┌────────────────────────────────────────────────────────┐
 │ FLOW 5: CLI Entrypoint & Surface (`run`, `status`)     │ ──► Cycle 5: Usability Proof (< 5 min)
 └────────────────────────────────────────────────────────┘
```

---

## 2. Goals Hierarchy & Slices

### Level 1: Core Goal (The Epic)
* **Goal 1 (Issue #1):** Local-First Telemetry & Analytics Engine for AI Agent Fleets.
  * *Operator Job:* Obtain empirical visibility into time, tokens, and rework to make informed squad topology and budget decisions using real host data.

---

### Level 2: Implementation Slices & Correction Cycles

#### Slice 1: Safe Ingestion & Watermarking (Issue #2 & #6)
* **Flow Objective:** Read heterogeneous sources without loading gigabytes into memory, persisting incremental read state.
* **Components:**
  * `dendro-core::storage`: DuckDB connection, idempotent schema migrations, `_dendro_watermarks` table.
  * `dendro-connectors`: Line-by-line zero-copy streaming parser (`BufReader`).
* **Invariants:** Safe Rust (`#![deny(unsafe_code)]`), zero unredacted secrets in storage, symlink output refusal.

#### Slice 2: Deterministic Session-to-Seat Resolution (Issue #2)
* **Flow Objective:** Stitch unstructured session files with OpenRig topology events (`agent.session_identity`).
* **Rule:** Zero heuristics (no guessing by PID, cwd, or timestamps). Unlinked sessions must be explicitly preserved with provenance.

#### Slice 3: The Three Core Operational Answers (Issues #3, #4, #5)
* **Question 1 (Seat Dynamics):** 5 disjoint activity bands (`active`, `idle_with_work`, `idle_without_work`, `waiting_human`, `unknown`). Sum of bands must equal 100% of the observation window.
* **Question 2 (Handoffs & Rework):** End-to-end task handoffs along queue item chains. Rework returns quantified across the review-to-build loop.
* **Question 3 (Token Burn & Retry Loops):** Multi-model token normalization and zero-yield command loop detection.

#### Slice 4: Decision Metrics M1 to M4 (Issues #3, #4, #5)
* **M1 (Seat Yield):** Fleet throughput and delivery yield.
* **M2 (Idle Waste Ratio):** Ratio of idle time with work pending over the observation window.
* **M3 (Build Rework Rate):** Proportion of tasks requiring 2 or more build cycles before passing review.
* **M4 (Skill Exposure & Impact):** Exposure to loaded skills with insufficient-data warnings where baseline is absent.

#### Slice 5: CLI Surface & Usability (Issue #1 & #6)
* CLI commands: `dendro run`, `dendro status`, `dendro watch`, `dendro init-nao`.
* Time to first report under 5 minutes from clean checkout.

---

## 3. Governance Standards
All codebase assets, documentation, terminal outputs, error messages, and tests must strictly be written in **English**.
EOF
