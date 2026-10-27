# Dendro Visual Product Tour

Dendro is the local flight recorder for a multi-agent coding fleet. Claude Code, Codex, AGY, Grok, Pi, and OpenRig already write session history on the machine. Dendro reads that history afterward and shows where time, handoffs, rework, and token budget actually went. It does not sit in the prompt path. It does not change routing. It does not keep prompts or secrets.

The breakthrough is the stack, not another chart of "AI usage." One local DuckDB store answers four questions that used to live in four different tools: what the model spent, what the harness did, how work moved between actors, and what the machine and the permission boundary looked like while that happened. The Connected Dashboard Suite draws those answers as one instrument, from growth rings down to a single evidenced edge.

Open it on the loopback interface only:

```bash
dendro dashboard --serve 127.0.0.1:4242
```

Non-loopback binds, including `0.0.0.0`, are refused. The server serves `site/` and exposes `GET /api/query` and `GET /api/events`. Until a live poll returns a non-null object, every console wears the badge **ILLUSTRATIVE SNAPSHOT**. A JSON `null` is not live. A later failed poll returns the badge to the snapshot. **LIVE TELEMETRY** means a real local query landed.

In the **Live Connected Web Studio** (`dendro dashboard --serve`), `/api/query` is the JSON query projection endpoint and `/api/events` is the Server-Sent Events live invalidation stream. The Time Machine Scrubber actively filters live SQL queries (`WHERE timestamp >= ? AND timestamp < ?`) across the Hub and all 12 views. Presets `today`, `24h`, `7d`, `30d`, `this_month`, `last_month`, `all`, and custom `YYYY-MM-DD..YYYY-MM-DD` selections persist across navigation through URL query params and `localStorage`.

In **Offline Materialized Nao Views** (`dendro materialize`), the four v0.8.0 domain views (`v_message_mix`, `v_subagent_lineage`, `v_security_posture`, `v_process_resources`) are all-time aggregates: `window_from` / `window_to` label the snapshot without filtering rows. The four OpenRig-compatibility views (`v_mission_token_burn`, `v_handoff_rework_analysis`, `v_seat_idle_ranking`, `v_executive_governance`) are scoped to the materialized `[from, to)` window.

Live connected workspace screenshots display local operator seat bindings (for example, `build-marcus@mtn` and `dev-build-sr@dendro-dev`) from the authenticated local database. Offline HTML exports (`dendro dashboard --html`) and `site/viewer.html` alias actor labels as `Seat A`, `Seat B`.

Product version on these surfaces: **v0.8.0**.

| Surface | What you use it for |
| --- | --- |
| Intelligence Hub (`site/dashboards/index.html`) | Growth rings, time scrubber, stack X-ray, capability studio, economics, insight strip I01–I25 |
| Active consoles I22–I25 | Trajectory vortex, schema drift, authority blast radius, counterfactual routing |
| Fleet Explorer (`site/viewer.html`) | Session timelines, handoff ribbons, pair friction, L0–L3 inspection |
| Terminal cockpit and `dendro ask` | The same evidence, in the terminal, with six bounded English intents |
| This tour | How to read every picture without inventing a cause |

Numbers printed on a screenshot belong to that recorded window. A different window prints a different total. An empty population is null, not zero.

---

## 1. The Living Growth Rings

The hub opens on a 420px heartwood. Four concentric rings are the four architectural layers, read from the outside in as a year of weather is read in a trunk.

![Living Growth Rings with the tactile time machine scrubber, preset windows, custom date range, rounded calendar labels, and Apple Health story highlights](assets/screenshots/hub-living-growth-rings.png)

**Screenshot scope:** `hub-living-growth-rings.png` is **Last Week · 7d (Oct 20 – Oct 27, 2026)**: the Hub summary reports **0 materialized Schema 5 drift anomalies** and **0 Trajectory Vortex loops**. **AUTONOMOUS FLEET HEALTH · NOMINAL** is a UI status pill summarizing 0 active materialized tripwires, not a synthetic composite health score. The `console-schema-drift.png` capture uses **All Time (`?window=all`)** and shows **76 daily metadata cohort buckets from the Mode B heuristic fallback**. These are different scopes and different measures.

| Ring | Layer | What a sector encodes |
| --- | --- | --- |
| Amber | Model | Token density and cache yield (`I02`, cost on `I04`) |
| Cyan | Harness | Execution rhythm, tool loops, envelope shape (`I08`, `I22`, `I23`) |
| Green | Coordination | Queue handoffs and lineage (`I09`, `I10`, `I12`) |
| Violet | Environment & Security | Placement, resources, grants, reachability (`I14`, `I18`, `I24`) |

Forty-eight radial clock sectors tile the selected window. Scrub or rotate the ring and the heartwood readout names the sector, the layer, and the measure under the hand. Click a legend pill to isolate one layer. **All Layers** restores the trunk. Glowing scars mark anomalies. A scar is a place to inspect, not a verdict.

### Tactile Time Machine Scrubber

The scrubber is the window for every surface below it. The active slice is carried across views.

* **Presets:** Today (24h), Last Week (7d), 30 Days, This Month, Last Month, All Time.
* **Custom Date Range Picker:** From / To dates and **Apply Slice**.
* **Rounded calendar labels** on the range control, so the window reads as a span a person chose, not a raw timestamp pair.
* **Dual handles** on the sparkline drag the start and the end. Anomaly pearls sit on the trace where the window contains a scar.

The recorded-window pill says whether you are looking at an illustrative snapshot or a live DuckDB slice. Half-open windows are `[from, to)`. Live selections filter SQL; the offline domain-view snapshot labels described above do not filter their all-time aggregates.

### Apple Health Story Highlights

Under the rings, **Highlights from Your Stack** tells three stories in plain sentences:

1. **Model.** Cache yield against uncached input. Reasoning stays inside output and is not added again. `cache_read_ratio_of_sums` and `cache_read_mean_of_ratios` are different measures.
2. **Harness.** Message rhythm across Claude Code, Codex, AGY, Grok, Pi, and OpenRig. Message text is never the measure.
3. **Coordination.** Clear, rework, or vortex weather, plus handoff returns. A clear day with zero vortex trips is a count of trips, not proof that nothing else moved.

Each card jumps to the console that holds the proof.

---

## 2. The 4-Layer Exploding Stack X-Ray

A fleet fails between layers. The model was cheap and the tool looped. The tool succeeded and the handoff waited. The handoff closed and the sandbox grant did not match the attempt. The X-ray keeps those layers in one picture.

![Four-layer exploding stack X-ray, Model to Harness to Coordination to Environment, with the 3D isometric explode and the multi-dimensional group-by explorer](assets/screenshots/hub-stack-xray-3d.png)

**Explode 3D Stack** (`⌘E` or `Cmd+E`) lifts the four planes into an isometric stack. Swimlane mode lays the same spans flat, Instruments-style, so a thread from a model call through a tool, a handoff, and a resource sample stays on one horizontal clock. Click a node and the inspector quotes the insight id and the evidenced detail. Threads are correlation, not causation.

| Plane | You are looking at | Canonical focus |
| --- | --- | --- |
| 1. Model | Calls, cache yield, uncached input, pinned-tier simulation | `I01`–`I05`, `I25` |
| 2. Harness | Actions, messages, capability funnel, vortex distance | `I06`–`I08`, `I22`, `I23` |
| 3. Coordination | Spawn, delegate, handoff, queue wait, return | `I09`–`I12`, `I19`–`I21` |
| 4. Environment & Security | Grants, sandbox posture, CPU delta, max RSS | `I13`–`I18`, `I24` |

### Multi-Dimensional Group-By Explorer

The same window pivots without a second query language. The Fleet Telemetry Rollup & Dimension Explorer has three group-bys:

* **By Harness / Tool.** Calls, observed tokens, share.
* **By Architectural Layer.** Fact volume, anomalies, primary insight.
* **By Date / Time Bucket.** Model, harness, coordination, and environment volume side by side.

Seat, rig, and four-eyes columns on the OpenRig-compatibility projections are adapter bindings. They are not the primary actor model.

---

## 3. Capability & Execution Intelligence Studio

agent-otel-bridge names what an agent can do. Dendro counts what it did. The studio is the join: five layers, one spectrum bar, one Apple bento console dock.

![agent-otel-bridge five-layer capability studio: ten behavioral tool families, harness primitives, skills declared versus invoked, MCP servers, lifecycle hooks, and the bento console dock](assets/screenshots/hub-capability-studio.png)

The `hub-capability-studio.png` screenshot displays the **FUNNEL I06** (`harness.capability_funnel`) badge, the **5-layer Capability Proportion Spectrum Bar**, the **10 Behavioral Tool Families**, and observed **`ToolArchetype` execution tiles** such as `generic_exec`. The spectrum is a composition, not a score. A hub snapshot decomposes observed events roughly as Tool Families, Harness Primitives, Skills, MCP, and Lifecycle Hooks. Your window will differ. A missing slice renders an empty state, not a zero-call fleet.

| Layer | Dock | What it answers |
| --- | --- | --- |
| 1 | 10 behavioral tool families | Which kind of action the fleet actually ran |
| 2 | Harness primitives | Read, edit, write, grep, glob, bash, subagent — the built-in verbs |
| 3 | Skills, declared vs invoked | `I06` `harness.capability_funnel`: invoked ∩ loaded / loaded |
| 4 | MCP servers | `mcp__*` calls by server, without tool arguments |
| 5 | Lifecycle hooks | `HookEvent` counts and guardrail observations |

### Ten behavioral tool families

Layer 1 is the bento of ten families:

1. **Shell/Exec** — sandboxed command execution.
2. **Read/Inspect** — file and diff inspection.
3. **Edit/Patch** — atomic writes and patches.
4. **Search/Grep** — repository search.
5. **Subagent Spawn** — delegated workers.
6. **MCP Call** — model-context-protocol tool calls.
7. **Skill Load** — skill declaration and load, distinct from a later invoke.
8. **Web/Fetch** — egress-bound fetch.
9. **Task/Queue** — queue, task, and handoff verbs.
10. **Governance/Gate** — review, approval, and acceptance gates.

The same layer on the hub also renders the agent-otel-bridge `ToolArchetype` tiles when that classification is present: `inspector_diff`, `search_retrieval`, `state_mutation`, `build_test_verify`, `filter_compressor`, `vcs_lifecycle`, `structured_parser`, `env_pkg_manager`, `network_transfer`, `generic_exec`. Families and archetypes are two groupings of tool facts. Neither one stores the command line or the file body.

Skills show declared, invoked, and dormant. Dormant means no invoke in the slice. It does not mean the skill is missing from disk. The canonical funnel is **I06**, not a placement insight. The screenshot badge is **FUNNEL I06**; `I13` is `environment.placement`.

Hooks and MCP cards report counts, latency bands, and error counts. They do not report payloads.

---

## 4. Active Intelligence Consoles (I22–I25)

Four bespoke SVG consoles sit on the hub as harness, harness, environment, and model. Each one discloses which resolution mode supplies its data.

The connected consoles have **two transparent resolution modes**:

* **Mode A — Materialized Schema 5 Reference Views.** `v_trajectory_vortex` computes exact 32-dim `FLOAT[32]` `array_cosine_distance` (`I22`); `v_schema_drift` measures `VARIANT` envelope L1 structural drift across harness versions (`I23`); `v_authority_blast_radius` provides 8-hop `duckpgq` evidenced-only session reachability (`I24`); `v_counterfactual_routing` performs pinned-tier simulation (`I25`) and refuses when no pricing tier is pinned.
* **Mode B — Live Windowed Fact-Table Fallbacks.** `intelligence_fallback` in `connected_web.rs` derives **labeled heuristic proxies** from `tool_facts`, `usage_facts`, `message_facts`, and `session_edges` for arbitrary live windows (`?window=7d`, `YYYY-MM-DD..YYYY-MM-DD`) or empty/unmaterialized Schema 5 tables. These keep the consoles diagnostically populated without claiming the reference measurements.

| Console | Mode B proxy and limits |
| --- | --- |
| `I22` | Turn-length variance and self-transition proxy, clamped to `0.08..0.82`; trips at `>= 25` turns and `>= 0.55` self-transition. **"Not an embedding cosine measurement"**. |
| `I23` | Daily metadata cohort buckets from `message_facts`; L1 proxy clamped at `0.45`, with `keys_retyped = 0`. These are not harness-version envelope comparisons. |
| `I24` | Recursive CTE (`depth < 4`) over recorded `session_edges`, plus `security_facts` authority-check scopes. This is distinct from the reference 8-hop graph walk; reachability does not grant authority. |
| `I25` | Heuristic `observed * 0.75` simulated fast-tier envelope total (`0.75 × observed`, leaving a `25%` counterfactual gap) in normalized token units against an assumed `128k` context threshold. A **counterfactual simulation proxy**, never a measured financial saving. |

The live `I22`–`I25` screenshots showcase **Mode B (`intelligence_fallback`) over the active workspace's live fact tables**. The routing display says **SIMULATED TIER TOTAL (COUNTERFACTUAL)** representing the simulated fast-tier envelope total (`0.75 × observed`, leaving a `25%` counterfactual gap) in normalized token units; it is not a priced Mode A result.

The reference contracts below describe **Mode A**; each pictured console uses the corresponding **Mode B** proxy above.

### 4.1 Trajectory Vortex — I22

![Trajectory Vortex orbital phase-space attractor for I22, Mode B turn-length and self-transition proxy](assets/screenshots/console-trajectory-vortex.png)

**Mode A reference:** `harness.trajectory_vortex` stores a `FLOAT[32]` sketch per turn in `harness_trajectory` and measures exact `array_cosine_distance` to the centroid of completed cohort sessions. An orbit grows when cosine distance or the self-transition rate grows. The console is there to show a loop **before** it becomes runaway token burn.

A trip is a distance signal. **A trip is not a cause.** It does not stop the agent, rewrite the prompt, or prove a bug. The cohort minimum is 5 untripped sessions; below that the status is Refused with reason `insufficient_sample`, even if a row would draw an orbit. Short cohorts stay `insufficient_evidence` on the badge. Canonical view: `v_trajectory_vortex`.

### 4.2 Schema Drift — I23

![Schema Drift DNA fingerprint ribbon for I23 across Claude Code, Codex, AGY, Grok, Pi, and OpenRig](assets/screenshots/console-schema-drift.png)

**Mode A reference:** `harness.schema_drift` compares `envelope_shape` key distributions between harness versions and reports L1 distance plus keys added, removed, and retyped. The DNA ribbon is that comparison drawn as bands, one gene per key-path, across **Claude Code, Codex, AGY, Grok, Pi, and OpenRig**.

Added, removed, and retyped stay separate columns. Drift is not a compatibility verdict. A pair with fewer than 5 envelopes on a side is Refused, reason **`insufficient_sample`**. Real key names appear only when a row carries them. Unknown keys in storage are dropped or HMAC-masked. Canonical view: `v_schema_drift`.

### 4.3 Authority Blast Radius — I24

![Authority Blast Radius radial ripple graph for I24, Mode B recorded session-edge reachability](assets/screenshots/console-blast-radius.png)

**Mode A reference:** `security.authority_blast_radius` walks the session graph with DuckDB `duckpgq` from a revoked or failed grant. The walk is **evidenced-only** (`status = evidenced`). Edges marked absent, truncated, or conflict are not traversable. Direction is fixed: parent to child for spawn, delegate, and handoff; child to parent for return and join; prior attempt to later attempt for retry. The hop ceiling is **8**. A pure Rust walk in `session_lineage.rs` matches the fixtures when `duckpgq` is absent.

Click an origin and the radial ripple draws the downstream cone. **The count is dependency exposure.** A downstream session can be reachable without holding the revoked grant. It is not inherited authority and it is not effective authority. Canonical view: `v_authority_blast_radius`.

### 4.4 Counterfactual Model Routing — I25

![Counterfactual model routing split-beam comparator for I25, cache-bleed inflection without mutating live routing](assets/screenshots/console-counterfactual-routing.png)

**Mode A reference:** `model.counterfactual_routing` simulates a fast tier with an operator-pinned table in `_dendro_pricing_tiers`, an `ASOF JOIN`, and a `QUANTILE_CONT` cache-bleed inflection. The split beam shows the observed path on one side and the simulated path on the other.

For the Mode A reference, drag the context-pressure threshold. Sessions at or above the inflection stay in the gate, and recoverable context and cost recompute from loaded rows. If the tier or the cohort pin is missing, the lower beam is withheld and the cost stays **unknown**. That refusal is not a zero. **The picture is not an effect and not a routing change.** Dendro does not write the live router. Canonical view: `v_counterfactual_routing`. Incurred cost itself remains `I04` `model.cost`, and only from recorded currency or a pinned tier.

---

## 5. Operational Economics & Multi-Dimensional Rollups

Token economics answers a narrower question than a cloud invoice: which actor, which task, which model, which cache behavior, inside which window.

![Operational economics and multi-dimensional rollups of task and actor token burn](assets/screenshots/console-token-economics.png)

The economics picture in this tour is one recorded window: **6,848.31M / 6.85B total tokens** and **96.9% cache-read efficiency**, with instant **Group-By** pivots across task and actor. Those two figures are the snapshot on that screen. The hub's own illustrative KPI strip can show a different total for a different slice. Do not add them together. Do not treat either one as the fleet's permanent burn rate.

How the rollup stays honest:

* **Task and actor.** Mission and queue rows group four token components by actor, model, effort, and attribution bucket. Compatibility views may label the actor column as a seat. That binding is OpenRig-specific.
* **Four components, not five additions.** `input_uncached`, `cache_write`, `cache_read`, and `output`. Reasoning stays inside output and is not added again.
* **Cache ratio.** `cache_read / (input_uncached + cache_write + cache_read)` is the ratio of sums. The mean of ratios is a separate measure. 96.9% on this screenshot is a ratio of sums for that window.
* **Group-By pivots.** Harness / tool, architectural layer, and date bucket on the hub explorer; actor, model, effort, and attribution on `v_mission_token_burn`. Do not unnest dimensions and sum again. That multiplies rows.
* **Money.** `I04` uses recorded currency or a pinned tier. Estimated or unverified rates are excluded. An empty USD estimate ships empty. It does not ship as `$0`.
* **Inclusive tokens.** `I03` `model.tokens.inclusive` sums a reachable subgraph. Incomplete lineage is a lower bound. Ambiguous duplicates refuse the total.

The direct component operators see on the token console is `model.tokens.direct`. The registry name for the insight is `I02` `model.tokens`. Both names point at the same accounted tokens.

---

## 6. Persona Workflows

Same suite, four jobs. Each path stays inside local evidence.

### Applied-AI Engineer

You want the session that stalled, not a productivity score.

1. Set the time machine to the sprint window. Custom range if the sprint crossed midnight.
2. Scrub the growth rings until a scar sits under the hand. Isolate Harness or Coordination.
3. Drop into the stack X-ray (`Cmd+E` if you need the planes separated) and follow the thread from the model call to the tool and the handoff.
4. Open Fleet Explorer for the timeline: active work, waiting on a human, handoff wait, stalled queue, parked. Life states do not bleed across unobserved gaps.
5. If a tool family dominates, open the capability studio and see whether the burn is Shell/Exec, Edit/Patch, or a retry loop (`I08`).

You leave with a span, an actor pair, and a tool family. You do not leave with "hours saved."

### Fleet & Harness Architect

You want structural change across harnesses before it becomes a silent breakage.

1. Group the dimension explorer **By Harness / Tool** and confirm Claude Code, Codex, AGY, Grok, Pi, and OpenRig are all in the window you think they are.
2. Open **Schema Drift (I23)** and check the mode. Mode A compares envelope keys across versions; Mode B shows daily metadata cohorts with `keys_retyped = 0`. Ignore reference pairs badged `insufficient_sample`.
3. Open **Trajectory Vortex (I22)**. Mode A measures the 32-dim distance from the completed centroid; Mode B draws a turn-length and self-transition proxy. Treat either as a lead and confirm with tool-loop and handoff evidence.
4. Check the capability funnel (`I06`): skills declared against skills invoked. A dormant skill is an observation about the window.
5. Placement and process shape stay on `I13` and `I14`. The architect's question is whether the envelope and the delegation graph still match the fleet you designed.

### FinOps & Token Economist

You want burn you can defend, including the burns you refused to invent.

1. Read the economics console. Note the window, the 6.85B-class total if that is the slice on screen, and the cache-read ratio beside it.
2. Pivot Group-By actor, then by model and effort. Shared allocation (`I11`) weights co-beneficiary missions. Weights sum to at most 1. Unlinked and ambiguous buckets stay visible.
3. Open **Counterfactual Routing (I25)**. Move the cache-bleed inflection. Record simulated cost only for Mode A with a tier pin. Mode B reports normalized token units, not money. If the beam says unknown, write unknown.
4. Keep incurred cost on `I04` and CPU on `I14`. The Nao views do not recompute either one.
5. `dendro ask` will answer "where do tokens go?" inside the six intents. It will refuse forecast, ROI, and savings language. That refusal is the product.

### Security & Governance Lead

You want exposure and gaps, not a false green board.

1. Read **Authority Blast Radius (I24)** from the revoked or failed grant. For Mode A, stay inside 8 hops and evidenced edges; Mode B uses recorded edges with `depth < 4` and authority-check scopes. Write "dependency exposure" in the note, because that is the claim the number supports.
2. Check `I16` isolation and `I17` authority. `NOT_ASSESSED` stays not assessed. A grant observation is not an effective-authority verdict.
3. `I18` counts redactions and coverage refusals. Secret-shaped values are masked. They are not displayed to prove the mask worked.
4. Governance acceptance is `I19`: accepted items over rule-eligible evaluated items. Missing eligibility does not become a 100% pass.
5. Confirm the serving badge and the bind address. The dashboard is loopback-only. Nothing in this suite is a remote telemetry export.

---

## 7. Honest Measurement Philosophy

Dendro is allowed to say "we cannot know." That sentence is a result.

### Null is not zero

A missing lazy fact table yields an empty projection, not a measured zero. A missing context limit refuses the `I07` ratio. An unpinned price refuses Mode A `I25` and leaves cost unknown; Mode B can show only its labeled normalized-unit simulation proxy. A null ratio, a missing row, or "no data" is not `0`. Display status maps to five badges: `ok`, `partial`, `insufficient_evidence`, `refused`, `not_applicable`. Reasons `not_observable` and `insufficient_sample` win as `insufficient_evidence`. `NOT_ASSESSED` is `not_applicable`. `threshold_exceeded` is `partial`.

`dendro status` may print `rows: 0` with `table: not created` for a lazy table that was never built. The text surface says **not created**. Do not read that JSON zero as an empty fleet you measured.

### Observed versus counterfactual

Observed measures come from facts: usage, tools, messages, queue, sessions, processes, security. Counterfactual measures are labeled simulations. `I25` is the counterfactual. `I21` `comparison.descriptive` compares two disjoint observed windows and returns `noncomparable` when the mix does not match. Explain and before/after do not claim causal proof. A vortex trip does not claim a cause. A blast-radius count does not claim inherited authority. Schema drift does not claim incompatibility. Advisory answers from `dendro advise` are pull-only. They are never injected into a live context window.

### Zero prompt and zero secret capture

Prompts, agent thoughts, response texts, tool arguments, tool results, and error bodies do not enter the analytical store. Message facts keep direction, role, turn, and modality. Tool inputs are compared through a keyed fingerprint. Secret-shaped values (`sk-`, `ghp_`, Bearer tokens, and the same class of canaries) are redacted in every field. Mission tags, tool names, and ordinary paths may appear. The dashboard projection drops labels, secrets, source paths, and non-numeric injections. Actor names on the public explorer are aliased.

L0 through L3 is the disclosure contract, in this order:

* **L0.** Status, finding, practical meaning, scope, and missingness.
* **L1.** Assumptions and alternatives.
* **L2.** Safe evidence IDs and source versions, at most five sealed keys.
* **L3.** Formula, population (what, included, excluded), exclusions, and cutoff.

The compiled insight strip is the catalog index. It does not recompute measures. At most seven persona questions sit above that strip. The strip is not the headline list.

### What this product will not say

* It will not invent developer-hours saved or a code-quality score.
* It will not call a busy terminal a productive one.
* It will not treat OpenRig seat bindings as the universal actor model.
* It will not claim a deployed Nao UI or a live provider round-trip. `dendro init-nao` writes a local workspace. `dendro ask` routes six intents in-process and does not generate SQL from the question.
* It will not claim the 20ms semantic target as a release gate. Measured timings are evidence. They are not an SLA.
* It will not modify prompts, kill sessions, or change live model routing. Fleet governance stays with the operator.

---

## Appendix. Insight registry I01–I25

Registry names below match the architecture and data dictionary. Where a dashboard or Nao page prints a presentation alias, both names are the same insight.

| ID | Identifier | Domain | One-line contract |
| --- | --- | --- | --- |
| I01 | `model.invocations` | Model | Distinct evidenced invocations. Empty population is `not_observable`. |
| I02 | `model.tokens` | Model | Token account. Presentation component: `model.tokens.direct`. |
| I03 | `model.tokens.inclusive` | Model | Reachable subgraph sum. Incomplete lineage is a lower bound. |
| I04 | `model.cost` | Model | Recorded currency or pinned tier only. |
| I05 | `model.configuration_divergence` | Model | Observed versus desired model and effort. |
| I06 | `harness.capability_funnel` | Harness | Invoked ∩ loaded / loaded. |
| I07 | `harness.context_pressure` | Harness | Context used / context limit. Missing limit refuses. |
| I08 | `harness.tool_loops` | Harness | Failure loops and retries. Presentation: `harness.action_failure`. |
| I09 | `harness.delegation_chains` | Harness | Multi-hop handoffs. Presentation: `work.lineage_shape`. |
| I10 | `work.duration` | Coordination | Wall-clock union, active seconds, concurrency. |
| I11 | `work.shared_allocation` | Coordination | Shared burn. Weights sum at most 1. |
| I12 | `coord.queue_wait` | Coordination | Ready to first start. |
| I13 | `environment.placement` | Environment | Host, platform, rebuilds. |
| I14 | `environment.process_resources` | Environment | CPU deltas and max sampled RSS. Presentation: `environment.resources`. |
| I15 | `environment.effects` | Environment | `not_observable` without a privileged trace specimen. |
| I16 | `security.isolation` | Security | Declared versus observed isolation. |
| I17 | `security.authority` | Security | Grant consistency. Unassessed stays `NOT_ASSESSED`. |
| I18 | `security.redactions` | Security | Scrub and coverage counts. Presentation: `coverage.source`. |
| I19 | `outcome.acceptance` | Coordination | Accepted / rule-eligible evaluated. |
| I20 | `outcome.resource_efficiency` | Coordination | Burn per accepted outcome on a matched population. |
| I21 | `comparison.descriptive` | Coordination | Two disjoint windows. `noncomparable` when the mix differs. |
| I22 | `harness.trajectory_vortex` | Harness | 32-dim cosine distance. Not a cause. |
| I23 | `harness.schema_drift` | Harness | L1 envelope drift. `insufficient_sample` below the pair minimum. |
| I24 | `security.authority_blast_radius` | Security | 8-hop evidenced reachability. Dependency exposure only. |
| I25 | `model.counterfactual_routing` | Model | Pinned fast-tier simulation. Not an effect. Not a routing change. |

Offline Nao reference projections and economics views (separate from the connected Mode B proxies): `v_trajectory_vortex`, `v_schema_drift`, `v_authority_blast_radius`, `v_counterfactual_routing`, `v_mission_token_burn`, `v_handoff_rework_analysis`, `v_seat_idle_ranking`, `v_executive_governance`, `v_message_mix`, `v_subagent_lineage`, `v_security_posture`, `v_process_resources`. Only the four v0.8.0 domain views listed above use all-time aggregates with snapshot labels; the four OpenRig-compatibility views filter the materialized `[from, to)` window. Live connected queries actively filter the selected window. A missing table is an empty projection, not a measured zero.
