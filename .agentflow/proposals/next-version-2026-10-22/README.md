# Dendro v0.8.0 proposal: review entry point

Status: **full revised implementation authorized by the user on2026-10-23; bootstrap/admission in progress**. This packet proposes v0.8.0 and database schema 4 from v0.7.0/schema 3 at `6b755135f95e497071790e8e5e358995a8244a18`. The packet itself establishes no product delivery, runtime adoption, merge or publication. Actual scoped implementation records now live under `.agentflow/runs/v080/`; each result requires its own evidence. Claude completed an independent critique of the initial architecture skeleton and rejected implementation readiness. The lead accepted, adapted or rejected its findings as recorded in ADVERSARIAL; Claude has not approved this finalized packet. Grok supplied source/domain, insight and mathematical reviews; Codex authored and adjudicated the design. An independent Grok final-packet audit returned six HIGH findings on the identified five-file snapshot. The lead applied bounded repairs, and a subsequent scoped Grok excerpt re-review closed those six contract concerns. Those reviews predate the user correction requiring native Windows AND Linux; they do not approve the expanded dual-native design. User implementation authorization now satisfies design intent; remaining technical/source/oracle admission checks are tracked in the implementation records. ADVERSARIAL records both review stages, hashes, semantic finding mappings and limits.

The release makes Dendro specialized intelligence for agent systems across four independent domains: model interaction, harness execution, coordination/work and environment. Identity, provenance, coverage, outcomes and security cross every domain. It reuses the existing engine and preserves OpenRig semantics through an adapter instead of requiring every system to look like a rig and seat.

The user has instructed implementation of these full revised contracts, scope, oracles and budgets. Recorded design approval authorizes the scoped implementation intent, subject to the existing admission, high-assurance, ownership, custody and publication gates. Bounded read-only admission investigation needs no extra approval ritual. Missing implementation authority, policy, private-source access or live adoption authority must still be resolved at the actual applicable gate; design approval cannot resolve them implicitly.

## Read progressively

| Reader / question | Artifact |
|---|---|
| Decision owner: what is proposed and still blocked? | This page, then [execution gates](EXECUTION.md) |
| Architect: what exactly do facts and answers mean? | [DESIGN.md](DESIGN.md) |
| Dispatcher/worker: which inputs, paths and tests apply? | [tasks.json](tasks.json), then [EXECUTION.md](EXECUTION.md) |
| Reviewer: what could make a plausible answer wrong? | [ADVERSARIAL.md](ADVERSARIAL.md) |
| vNext owner: what does Goal 9 add after v0.8.0? | [ADDENDUM-DUCKDB2-AND-INTELLIGENCE.md](ADDENDUM-DUCKDB2-AND-INTELLIGENCE.md) |

The JSON DAG alone defines task dependencies. Document sections explain contracts, not a competing schedule. Each release requirement maps to task acceptance IDs. Proposed module paths and test targets are reservations to create during authorized delivery, not claims they exist today.

## Decisions with the largest consequences

- Session, participant, execution attempt, invocation and usage observation are separate typed identities. Exact source-backed equivalence permits deduplication; absent invocation IDs do not discard usage.
- Lineage supports multiple callers, long-lived children, retries and continuations. Fleet totals sum unique direct observations; inclusive answers are nonadditive; shared work stays unallocated without explicit weights.
- Environment delivery includes real native Windows and Linux unprivileged collectors, versioned observation import, a pinned Windows event/effect trace reader and Linux syscall trace reader. Grok gets a specimen-backed native reader. AGY gets a Dendro-defined export adapter; AGY native support is not claimed.
- Reference analyzers actually compute four-domain findings. Missing historical facts stay unknown. A release consisting only of unsupported answers fails acceptance.
- Schema 3 is preserved with its privacy key and sidecars. Migration builds a separate schema-4 database. Rollback selects the old snapshot; old binaries do not read the new database.
- Native Windows and native Linux are both mandatory v0.8.0 runtimes, including capture, storage, analysis, CLI/TUI/dashboard/Nao, privacy and lifecycle behavior. WSL is optional and never satisfies Windows-native acceptance. Existing documented macOS support is preserved; platform capability differences remain explicit.

## Known execution prerequisites

The isolated branch and baseline HEAD were verified for this writing task. Existing differences in `scripts/portfolio-fixture.py` and `templates/nao/validate_performance.py` must remain untouched. Live `rig` identity is unavailable here. The supplied baseline reports broken distribution inspection due to a missing sibling `yaml` dependency, absent ownership manifest, backlog pause, incomplete custody and empty local-preview candidate inputs. B00 must verify these against authoritative records without repairing them implicitly.

Source specimens, syscall producer version, applicable macOS support and historical performance contracts require bounded evidence admission. These are visible release blockers where required; they are not permission to reduce scope. User-selected routes are Codex lead, extensive native Grok, independent Claude, no AGY workers, with at most three simultaneous workers.

This packet reports document consistency only. Future Rust tests, capture correctness, performance and installed-runtime behavior remain unmeasured. Completed Claude findings and lead adjudication are summarized in the review ledger. A final reviewer must identify the exact revised packet digest; no earlier critique is represented as acceptance of later files.

Packet inventory: **35 tasks, 28 release requirements, 69 acceptance criteria, 21 reference insight families (I02 includes named cache measures), 29 fixture groups**. `requirements`, per-task `requirement_ids` and `requirement_test_map` provide the bidirectional acceptance/test mapping. All numeric budgets are design targets; product validation remains future work.

Native feasibility is owned work: B02 pins toolchain/dependency/permission evidence and remedies; T24 fixes portability before storage/adapters, T25/T26 deliver Windows native collector/event import, T27 owns proposed native CI and packages. Both native lanes are mandatory. A scoped Grok excerpt review reported13 concerns; lead adjudication closed concrete admission/security gaps and documented omitted-context findings. ADVERSARIAL records the dispositions; no blanket approval or product validation is claimed.

The new v0.8.0 scope uses direct local agents, permitted by the owner clarification on issue526 comment6200530869; rigs are not required. This does not resume unrelated backlog or reset historical G2. Local generated run/worker IDs identify only current implementation records, never authenticated historical seats/fences. B00 direct-core acceptance depends on reviewed bootstrap/path-control evidence; seat-manifest restoration remains a separate governance hold and does not gate authorized direct task leases; process-local distribution inspection evidence is recorded with its dependency-overlay limits.

Core admission is partitioned from release-only source/native holds: reviewed structural bootstrap assertions are frozen and ready for fresh-worker typed construction. Full numeric/source/native acceptance remains incomplete; no scope is removed.

## vNext addendum (not part of the v0.8.0 DAG)

[ADDENDUM-DUCKDB2-AND-INTELLIGENCE.md](ADDENDUM-DUCKDB2-AND-INTELLIGENCE.md) is the normative Goal 9 / vNext contract (authored 2026-10-27). It does not change the packet inventory above and it does not edit `tasks.json`. `DESIGN.md` still defines I01–I21. `SCOPE-TRIMMED.md` still defines what v0.8.0 built.

| Requirement | Acceptance | Registers |
|---|---|---|
| `R29` DuckDB 2.0.0 LTS, storage 69, schema 5, quack single-owner | `A01`–`A03` | Crate pin `~1.20000.0`, backup `dendro.duckdb.pre-v200.bak`, private quack boundary |
| `R30` `VARIANT`, triggers, `duckpgq`, `fts`, trajectory, DuckLake 1.1 | `A04`–`A07` | Body-free envelope, tripwire outbox, extension-free lineage fallback |
| `R31` Insights I01–I25 and `dendro advise` | `A08`–`A12` | Fifteen remaining `DESIGN.md` insights, `I22`–`I25`, advisory separated from governance |
| `R32` Presentation parity | `A13`–`A14` | Dashboards, viewer, TUI, Obsidian visual tokens |
| `R33` Documentation and Nao catalog parity | `A15` | Same-change docs plus `site/site.test.cjs` and `semantic.rs` |

Backlog rows for these IDs live under Goal 9 in `docs/BACKLOG.md`. None of `A01`–`A15` is done by publishing this page.
