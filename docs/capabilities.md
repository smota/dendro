# Capabilities & Intelligence Layers

Dendro turns local agent records into an operational view of your fleet. Follow token consumption, tool behavior, handoffs, and resource use across Claude Code, Codex, AGY, Grok, Pi, and OpenRig—then decide where to intervene.

[Documentation Hub](README.md) · [Visual Product Tour](VISUAL_PRODUCT_TOUR.md) · [Operator Guide](adoption/README.md)

## Four layers, one investigation

| Layer | What you can see | What you can decide |
| --- | --- | --- |
| **Model** | Token components, cache use, model configuration, and routing scenarios. | Where to investigate consumption or compare model choices. |
| **Harness** | Tool usage, skill invocation, repeated actions, and structural changes in execution records. | Which capabilities need tuning and which sessions need attention. |
| **Coordination** | Delegation, queue wait, handoff returns, duration, and accepted work. | Where to simplify the topology or improve a handoff. |
| **Environment & Security** | Process placement, CPU and memory observations, sandbox posture, and permission exposure. | Which resource constraints and boundaries deserve review. |

The **Living Growth Rings** reveal changes across these layers. The **Time Machine** narrows the period, **Story Highlights** point to findings, and the **Stack X-Ray** connects the evidence in flat swimlanes or an exploded 3D view (`⌘E`).

## 5-Layer Capability & Execution Intelligence

Discover which capabilities your fleet uses and where available capabilities sit dormant. The Capability Studio groups execution into five complementary layers:

| Capability layer | What you inspect | Question it answers |
| --- | --- | --- |
| **Tool Families** | Shell / Exec, Read / Inspect, Edit / Patch, Search / Grep, Subagent Spawn, MCP Call, Skill Load, Web / Fetch, Task / Queue, Governance / Gate. | What kinds of action dominate the work? |
| **Harness Primitives** | Native tools, their outcomes, and failure patterns. | Which built-in operations carry the workload? |
| **Skills** | Declared and loaded skills compared with recorded invocations. | Which skills reach execution, and which remain dormant? |
| **MCP Servers & Tools** | Calls grouped by server and tool. | Which integrations are used and where do failures cluster? |
| **Lifecycle Hooks & Guardrails** | Recorded hook stages and governance observations. | Where are checks and interceptions occurring? |

Use the proportion spectrum to find the balance, then open a focused view from the console dock. Skill availability and skill invocation remain distinct so you can evaluate adoption against actual use.

## Active Intelligence

| Console | What it helps you do |
| --- | --- |
| **I22 · Trajectory Vortex** | Catch repetitive behavioral loops and stalled reasoning orbits before runaway token burn. |
| **I23 · Schema Drift** | Spot structural shifts across harness versions and focus connector checks after an upgrade. |
| **I24 · Authority Blast Radius** | Map downstream session reachability around a permission grant or failure to review dependency exposure. |
| **I25 · Counterfactual Model Routing** | Test fast-tier thresholds and context-pressure assumptions against recorded sessions while live routing stays under your control. |

The consoles guide an investigation. Results carry evidence coverage and assumptions; routing scenarios are simulations. For analytical definitions and calculation details, see the [Architecture Blueprint](ARCHITECTURE.md#4-advanced-analytical-subsystems).

## Operational economics

Separate cache reads, cache writes, fresh input, and output, with reasoning shown within output. Group available records by harness, capability, date, workspace, or outcome to find where consumption concentrates. Cost analysis uses recorded currency or operator-pinned pricing.

Move from the rollup to its supporting records before changing a budget. Handoff and shared-allocation views help you understand consumption across collaborating actors.

## Choose your surface

| Surface | Best for | Entry point |
| --- | --- | --- |
| **Connected Web Studio** | Visual investigation with live local updates. | `dendro dashboard --serve 127.0.0.1:4242` |
| **Terminal Cockpit** | Explore timelines, handoffs, tokens, and topology from the terminal. | `dendro dashboard` |
| **Conversational Queries** | Ask supported questions about idle time, token burn, rework, and comparisons. | `dendro ask --list-intents` |
| **Reports & Traces** | Repeatable analysis and evidence inspection. | `dendro report` and `dendro trace` |
| **Offline Export** | Review a self-contained local snapshot. | `dendro dashboard --html ./fleet-dashboard.html` |

## The full insight catalog

Dendro's stable insight identifiers connect the studio, reports, and reference documentation.

| Layer | Insights |
| --- | --- |
| **Model** | I01 Invocations · I02 Tokens · I03 Inclusive tokens · I04 Cost · I05 Configuration divergence · I25 Counterfactual routing |
| **Harness** | I06 Capability funnel · I07 Context pressure · I08 Tool loops · I09 Delegation chains · I22 Trajectory Vortex · I23 Schema Drift |
| **Coordination** | I10 Duration · I11 Shared allocation · I12 Queue wait · I19 Acceptance · I20 Resource efficiency · I21 Descriptive comparison |
| **Environment & Security** | I13 Placement · I14 Process resources · I15 Effects · I16 Isolation · I17 Authority · I18 Redactions · I24 Authority Blast Radius |

## Built for local evidence

Dendro reads authorized source logs without changing them. The analytics store keeps allowlisted operational facts and redacted diagnostics; prompts, thoughts, replies, tool payloads, and secrets stay out. Missing evidence stays visible, and the operator owns changes to the fleet.

Explore the [Data Dictionary](DATA_DICTIONARY.md) for fields and metric definitions, or the [Connector SDK](extensions/README.md) to add another harness.
