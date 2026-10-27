# Harness capability negotiation

AgentFlow describes desired execution behavior without embedding Claude, Codex, Grok, Pi, Agy, or
another harness into lifecycle roles or skills. The active provider is inspected at runtime and the
resolver selects the smallest implementation that satisfies the declared intent and controls.

## Bounded taxonomy

| Concept          | Question                          | Examples                                  |
| ---------------- | --------------------------------- | ----------------------------------------- |
| Lifecycle role   | Who owns the SDLC decision?       | `agentflow:architect`, `agentflow:tester` |
| Skill or method  | How is the work approached?       | `agentflow-scanner`, TDD, event storming  |
| Execution intent | What portable behavior is needed? | `delegated-work`, `parallel-fanout`       |
| Control          | What may never be weakened?       | `single-writer`, `review-independence`    |
| Provider facet   | What service surface exists?      | `execution`, `evidence`, `workspace`      |
| Provider binding | How will this run now?            | provider, transport, fidelity, limits     |
| Evidence         | What actually happened?           | intent resolutions and execution receipt  |

Controls are not capabilities. Provider facets are not execution intents. Runtime platform,
execution target, transport, model, and delegation boundary remain distinct provenance fields.

## Execution intents

| Intent                   | Purpose                                             | Typical consumers               |
| ------------------------ | --------------------------------------------------- | ------------------------------- |
| `plan-before-edit`       | Establish an approach before mutation               | architect, planner, developer   |
| `delegated-work`         | Run bounded work in another context                 | collaborator, reviewer, scanner |
| `parallel-fanout`        | Evaluate independent lanes concurrently             | scanner, tester, QA expert      |
| `isolated-workspace`     | Keep experimental writes outside the issue worktree | migrator, developer spike       |
| `background-execution`   | Continue bounded long-running work asynchronously   | orchestrator, tester            |
| `structured-result`      | Return a schema-constrained result                  | auditor, reviewer, PR readiness |
| `bounded-loop`           | Repeat with explicit limits and stop conditions     | developer, migrator             |
| `workflow-orchestration` | Coordinate steps while preserving AgentFlow phases  | orchestrator                    |

`delegated-work` is intentionally mechanism-neutral. A provider may implement it as a subagent,
fresh session, provider call, sequential role lens, or human handoff.

## Provider descriptors

Providers expose structural `facets` and a separate `intentSupport` descriptor:

```json
{
  "facets": ["execution", "evidence"],
  "intentSupport": [
    {
      "id": "delegated-work",
      "implementation": "native",
      "fidelity": "full",
      "evidence": "probed",
      "limits": { "maxDelegates": 4 }
    }
  ]
}
```

Availability is `available`, `configured`, `manual`, `unknown`, or `unavailable`. Implementation is
`native`, `plugin`, `adapter`, `emulated`, or `manual`; fidelity is `full`, `partial`, or `degraded`;
evidence is `probed`, `contract-tested`, or `self-declared`.

`inspect()` is the runtime authority. There is no target-by-capability matrix. A new provider adds a
descriptor and probe without changing role, skill, or method catalogs.

## Resolution and fallback

The resolver:

1. validates the portable execution intent;
2. checks required provider facets;
3. calls the provider's `inspect()`;
4. resolves every intent with fidelity, evidence, limits, and status;
5. applies only explicitly allowed semantic fallbacks;
6. emits a digest-bound plan and execution receipt.

Valid fallbacks include parallel fanout to sequential role lenses and native planning to an
AgentFlow plan artifact. Invalid fallbacks include a shared-worktree second writer, same-context
self-review presented as independent, or a spike without an isolated workspace.

```bash
agentflow-sdlc providers list --json
agentflow-sdlc providers inspect grok-cli --json
agentflow-sdlc collaboration plan --mode council --provider grok-cli --json
```

Provider-specific documentation is illustrative only. The observed descriptor and receipt are the
authority for the current run.

## Evidence

Role passes record `executionIntentsUsed`. `ExecutionReceipt.executionIntentSource` records the
provider's declared support and actual resolutions. Required loops include maximum iterations and
stop conditions; planning includes its pre-edit artifact; delegated work includes permissions,
context, result, and owner synthesis.

```bash
node scripts/validate-execution-intent-evidence.mjs --path evidence.json --json
```

See [Intelligent collaboration](intelligent-collaboration.md) for mode selection and
[Role collaboration](role-collaboration.md) for deterministic handover acceptance and councils.

Execution integrations use `planBoundExecution({ provider, collaborationPlan, request })` from
`lib/collaboration-plan.mjs` to carry the selected binding into the provider's digest-bound plan.
This preserves intent resolutions and degraded status in the resulting `ExecutionReceipt`.
Do not reconstruct a binding from a provider name or discard its fallback evidence.

---

# Dendro fleet capability surfaces

The negotiation model above is how an AgentFlow provider declares intent. Dendro is the local instrument that records what the fleet actually did after the run. It does not replace `inspect()`, and it does not become a target-by-capability matrix. Evidence still wins: a descriptor is a claim, a fact row is an observation.

## Connected Dashboard Suite

```bash
dendro dashboard --serve 127.0.0.1:4242
```

v0.8.0 serves the Intelligence Hub from `site/dashboards/index.html` and the Fleet Explorer from `site/viewer.html`. The bind must be loopback (`127.0.0.1`, `localhost`, or `[::1]`). `0.0.0.0` is refused. Live reads use `GET /api/query` and `GET /api/events`. The badge stays **ILLUSTRATIVE SNAPSHOT** until a poll returns a non-null object, then reads **LIVE TELEMETRY**. JSON `null` is not live. A later failure clears the live badge.

In the **Live Connected Web Studio** (`dendro dashboard --serve`), `/api/query` is the JSON query projection endpoint and `/api/events` is the Server-Sent Events live invalidation stream. The Time Machine Scrubber actively filters live SQL queries (`WHERE timestamp >= ? AND timestamp < ?`) across the Hub and all 12 views. Presets `today`, `24h`, `7d`, `30d`, `this_month`, `last_month`, `all`, and custom `YYYY-MM-DD..YYYY-MM-DD` selections persist across navigation through URL query params and `localStorage`.

In **Offline Materialized Nao Views** (`dendro materialize`), the four v0.8.0 domain views (`v_message_mix`, `v_subagent_lineage`, `v_security_posture`, `v_process_resources`) are all-time aggregates: `window_from` / `window_to` label the snapshot without filtering rows. The four OpenRig-compatibility views (`v_mission_token_burn`, `v_handoff_rework_analysis`, `v_seat_idle_ranking`, `v_executive_governance`) are scoped to the materialized `[from, to)` window.

### Living Growth Rings

Four concentric rings on a 48-sector clock: Model, Harness, Coordination, Environment & Security. Scrub or rotate to read one sector in the heartwood. Legend pills isolate a layer. Scars mark anomalies for inspection. A scar is not a root-cause finding.

### Tactile Time Machine Scrubber

Presets are Today (24h), Last Week (7d), 30 Days, This Month, Last Month, and All Time. The custom date range picker applies an explicit slice. Rounded calendar labels show the span the operator chose. Dual handles trim the sparkline. The active slice is carried across hub views. Intervals are half-open, `[from, to)`.

### Apple Health Story Highlights

Three story cards summarize the same window in sentences: model cache yield (`I02`, with cost left on `I04`), harness message rhythm (`I08` and companions, never message text), and coordination weather including vortex and rework (`I22`, handoff returns). Cards link into the consoles. They do not recompute the insights.

### 4-Layer Exploding Stack X-Ray

Planes run Model, then Harness, then Coordination, then Environment. Swimlane keeps one clock. **Explode 3D Stack** (`Cmd+E`) lifts an isometric stack. The multi-dimensional Group-By explorer pivots the same facts by harness/tool, by architectural layer, and by date/time bucket. Cross-layer threads are correlation. They are not causation. Seat, rig, and four-eyes fields on compatibility projections are OpenRig adapter bindings, not the primary actor model.

### agent-otel-bridge 5-Layer Capability Studio

The studio decomposes observed actions into five layers and an Apple bento console dock:

| Layer | Surface | Canonical join |
| --- | --- | --- |
| 1 | 10 behavioral tool families | Tool facts, grouped |
| 2 | Harness primitives | Built-in verbs (read, edit, write, grep, glob, bash, subagent) |
| 3 | Skills, declared vs invoked | `I06` `harness.capability_funnel` |
| 4 | MCP servers | `mcp__*` call counts, no payloads |
| 5 | Lifecycle hooks | `HookEvent` counts and guardrails |

The ten families are Shell/Exec, Read/Inspect, Edit/Patch, Search/Grep, Subagent Spawn, MCP Call, Skill Load, Web/Fetch, Task/Queue, and Governance/Gate. When the hub has agent-otel-bridge archetype rows, the same layer also renders `inspector_diff`, `search_retrieval`, `state_mutation`, `build_test_verify`, `filter_compressor`, `vcs_lifecycle`, `structured_parser`, `env_pkg_manager`, `network_transfer`, and `generic_exec`. Skill Load is not an invoke. The funnel ratio is invoked ∩ loaded / loaded. `I13` is placement, not the funnel.

No command text, prompt, or tool argument is stored to draw these tiles.

### Active intelligence consoles I22–I25

The table below describes **Mode A materialized Schema 5 reference views**. Connected consoles also disclose **Mode B live windowed fact-table heuristic fallbacks** (`intelligence_fallback` in `connected_web.rs`) for arbitrary live windows or empty/unmaterialized Schema 5 tables; see the [two-mode console contract](VISUAL_PRODUCT_TOUR.md#4-active-intelligence-consoles-i22i25). The live console screenshots use Mode B, including **SIMULATED TIER TOTAL (COUNTERFACTUAL)** representing the simulated fast-tier envelope total (`0.75 × observed`, leaving a `25%` counterfactual gap) in normalized token units. CPU stays on `I14`. Cost stays on `I04`. A proxy is not a reference measurement; a missing fact table is not a measured zero.

| Console | ID | Identifier | Non-claim |
| --- | --- | --- | --- |
| Trajectory Vortex, orbital phase-space attractor | I22 | `harness.trajectory_vortex` | 32-dim cosine distance to the completed centroid. A trip is not a cause. Cohort under 5 untripped sessions refuses with `insufficient_sample`. View: `v_trajectory_vortex`. |
| Schema Drift, DNA fingerprint ribbon | I23 | `harness.schema_drift` | L1 distance of `envelope_shape` across Claude Code, Codex, AGY, Grok, Pi, and OpenRig. Added, removed, and retyped stay separate. A short pair is `insufficient_sample`. Drift is not a compatibility verdict. View: `v_schema_drift`. |
| Authority Blast Radius, radial ripple | I24 | `security.authority_blast_radius` | `duckpgq` reachability, evidenced edges only, 8-hop ceiling. The count is dependency exposure, not inherited or effective authority. View: `v_authority_blast_radius`. |
| Counterfactual Routing, split-beam comparator | I25 | `model.counterfactual_routing` | Pinned fast-tier simulation and cache-bleed inflection. Unknown when the tier is unpinned. Not an effect. Not a routing change. View: `v_counterfactual_routing`. |

### Operational economics

Task and actor token burn roll up through `v_mission_token_burn` and the hub Group-By pivots. One tour snapshot shows 6.85B tokens and 96.9% cache-read efficiency for its own window. Another slice, including the hub's illustrative KPI strip, can show a different total. Do not add snapshots together.

Token components are `input_uncached`, `cache_write`, `cache_read`, and `output`. Reasoning stays inside output and is not added again. The ratio of sums and the mean of ratios are different measures. `I04` `model.cost` accepts recorded currency or a pinned tier and excludes unverified rates. An empty money figure stays empty. It is not zero. `I03` refuses an ambiguous inclusive total. `I11` shares burn with weights that sum to at most 1.

## Insight registry I01–I25

Identifiers are stable. Presentation aliases used on the v0.8.0 viewer and Nao pages are the same insight, not a second catalog.

| ID | Identifier | Domain | Boundary |
| --- | --- | --- | --- |
| I01 | `model.invocations` | Model | Empty population is `not_observable`. |
| I02 | `model.tokens` | Model | Also shown as `model.tokens.direct`. |
| I03 | `model.tokens.inclusive` | Model | Incomplete lineage is a lower bound. Ambiguous duplicates refuse. |
| I04 | `model.cost` | Model | No estimated rate. |
| I05 | `model.configuration_divergence` | Model | Missing pairs are `not_observable`. |
| I06 | `harness.capability_funnel` | Harness | Declared or loaded versus invoked. |
| I07 | `harness.context_pressure` | Harness | Missing limit refuses the ratio. |
| I08 | `harness.tool_loops` | Harness | Presentation alias `harness.action_failure`. |
| I09 | `harness.delegation_chains` | Harness | Presentation alias `work.lineage_shape`. |
| I10 | `work.duration` | Coordination | Half-open intervals. |
| I11 | `work.shared_allocation` | Coordination | Weights sum at most 1. |
| I12 | `coord.queue_wait` | Coordination | Ready to first start. |
| I13 | `environment.placement` | Environment | Host and platform locus. |
| I14 | `environment.process_resources` | Environment | Presentation alias `environment.resources`. CPU deltas and max sampled RSS. |
| I15 | `environment.effects` | Environment | `not_observable` without a privileged specimen. |
| I16 | `security.isolation` | Security | Declared versus observed sandbox. |
| I17 | `security.authority` | Security | Unassessed policies stay `NOT_ASSESSED`. |
| I18 | `security.redactions` | Security | Presentation alias `coverage.source`. |
| I19 | `outcome.acceptance` | Coordination | Accepted over rule-eligible evaluated. |
| I20 | `outcome.resource_efficiency` | Coordination | Matched population only. |
| I21 | `comparison.descriptive` | Coordination | `noncomparable` when the mix differs. |
| I22 | `harness.trajectory_vortex` | Harness | Not a cause. |
| I23 | `harness.schema_drift` | Harness | `insufficient_sample` on a short pair. |
| I24 | `security.authority_blast_radius` | Security | Dependency exposure only. |
| I25 | `model.counterfactual_routing` | Model | Not an effect. Not a routing change. |

Progressive disclosure is fixed. L0 is status, finding, practical meaning, scope, and missingness. L1 is assumptions and alternatives. L2 is safe evidence IDs and source versions. L3 is formula, population (what, included, excluded), exclusions, and cutoff. Badges are `ok`, `partial`, `insufficient_evidence`, `refused`, and `not_applicable`.

## Non-claim boundaries

* **Null is not zero.** A missing row, a missing denominator, an unpinned tier, or a table that was never created is not a measured zero. `rows: 0` with `table: not created` means the lazy table is absent.
* **Observed versus counterfactual.** `I25` is a labeled simulation. `I21`, Explain, and before/after comparison do not claim causal proof. Advisory results are pull-only and are not injected into agent context.
* **Zero prompt and zero secret capture.** Prompts, thoughts, responses, tool arguments, tool results, and error bodies are not stored. Secret-shaped values are redacted in every field. Message facts keep role, direction, and modality only.
* **Reachability is not authority.** Mode A `I24` traverses `status = evidenced` only, at most 8 hops; Mode B uses recorded edges with a recursive CTE (`depth < 4`) and authority-check scopes. Downstream reachability does not transfer a grant.
* **Drift is not a compatibility verdict.** Mode A `I23` reports structural L1 change; Mode B reports a daily metadata cohort proxy.
* **A vortex is not a stop.** `I22` does not kill a session or edit a prompt.
* **Routing stays with the operator.** `I25` does not mutate live routing. Dendro does not autonomously alter prompts.
* **No vanity productivity.** The product does not claim developer-hours saved, subjective quality scores, forecast savings, or an SLA on the historical 20ms semantic target.
* **Nao is local and optional.** `dendro init-nao` does not claim a deployed Nao UI or a live provider round-trip. `dendro ask` has six bounded intents and refuses forecast, ROI, savings, secret, and free SQL phrasing.
* **Provider negotiation stays upstream.** Controls are not capabilities. Facets are not execution intents. A Dendro count of `delegated-work` or `parallel-fanout` does not weaken `single-writer` or `review-independence`.

The visual walkthrough of these surfaces is [VISUAL_PRODUCT_TOUR.md](VISUAL_PRODUCT_TOUR.md). The manifesto voice is [storytelling.md](storytelling.md).
