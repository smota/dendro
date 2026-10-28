# Dendro Visual Product Tour

**Your fleet has a history. Now you can read it.** Dendro brings local session history from AI coding clients **Claude Code, OpenAI Codex, Google Antigravity, Grok, and Pi** into one studio, with verified export adapters for **Cursor, Windsurf, Goose, and Devin**. See where time and tokens went, which capabilities ran, and where work lost momentum.

Fleet orchestration runtimes such as **OpenRig, AgentFlow, and Meshloop** coordinate teams and handoffs across those clients. OpenRig is supported out of the box; you can [bring another client, runtime, or capability mapper](extensions/README.md) through Dendro's open extension contracts. Check the [supported export formats](extensions/formats.md) for verified adapter coverage or the [Contributing Guide](../CONTRIBUTING.md) to help extend it.

[Documentation Hub](README.md) · [60-Second Quickstart](../README.md#quickstart) · [Capabilities](capabilities.md)

[![Watch the Connected Intelligence Studio walkthrough](assets/dendro-preview.gif)](assets/dendro-demo.mp4)

[Watch the full video](assets/dendro-demo.mp4) · [Follow the walkthrough](VIDEO_WALKTHROUGH.md)

Open the connected studio after ingesting your local sources:

```sh
dendro run
dendro dashboard --serve 127.0.0.1:4242
```

Visit `http://127.0.0.1:4242` to explore your fleet.

## 1. The Living Growth Rings

![Living Growth Rings with the Time Machine and Story Highlights](assets/screenshots/hub-living-growth-rings.png)

**Read your entire fleet like an instrument.** Scrub across any time window with the **Time Machine**, spot anomalies at a glance on **The Living Growth Rings**, and jump straight from **Story Highlights** into the underlying layer.

## 2. The 4-Layer Exploding Stack X-Ray

![4-Layer Stack X-Ray in an exploded isometric view](assets/screenshots/hub-stack-xray-3d.png)

**Pull the stack apart (`⌘E`).** Trace execution across **Model → Harness → Coordination → Environment** in flat swimlanes or 3D isometric view, and pivot instantly by harness, layer, or date.

## 3. 5-Layer Capability & Execution Intelligence

![Capability Studio showing tool families, harness primitives, skills, MCP, and lifecycle hooks](assets/screenshots/hub-capability-studio.png)

**Know what your agents actually use.** See the balance across **Tool Families, Harness Primitives, Skills (Declared vs. Invoked), MCP Servers & Tools, and Lifecycle Hooks & Guardrails**—so you know which capabilities drive results and which sit dormant.

## 4. Active Intelligence Consoles (I22–I25)

<table>
  <tr>
    <td width="50%" valign="top">
      <img src="assets/screenshots/console-trajectory-vortex.png" alt="I22 · Trajectory Vortex console" width="600" />
      <p><strong>I22 · Trajectory Vortex: Catch loops before runaway token burn.</strong></p>
      <p>Inspect repetitive behavior and stalled reasoning orbits to choose the next session to investigate.</p>
    </td>
    <td width="50%" valign="top">
      <img src="assets/screenshots/console-schema-drift.png" alt="I23 · Schema Drift console" width="600" />
      <p><strong>I23 · Schema Drift: Spot structural shifts after an upgrade.</strong></p>
      <p>Compare telemetry shapes across Claude Code, OpenAI Codex, Google Antigravity, Grok, and Pi to identify the connector or version that needs attention.</p>
    </td>
  </tr>
  <tr>
    <td width="50%" valign="top">
      <img src="assets/screenshots/console-blast-radius.png" alt="I24 · Authority Blast Radius console" width="600" />
      <p><strong>I24 · Authority Blast Radius: Map your dependency exposure.</strong></p>
      <p>Follow downstream session reachability from a permission grant or failure to focus your review.</p>
    </td>
    <td width="50%" valign="top">
      <img src="assets/screenshots/console-counterfactual-routing.png" alt="I25 · Counterfactual Model Routing console" width="600" />
      <p><strong>I25 · Counterfactual Model Routing: Test routing choices against history.</strong></p>
      <p>Explore fast-tier thresholds and context-pressure inflections against recorded sessions without touching live production traffic.</p>
    </td>
  </tr>
</table>

## 5. Operational Economics & Multi-Dimensional Rollups

![Operational Economics with token components and multidimensional rollups](assets/screenshots/console-token-economics.png)

**Every token accounted for, sliced your way.** Separate cache reads, cache writes, fresh input, and reasoning output, then group by **Harness, Capability, Date, Workspace, or Outcome** in one click.

## Choose your first investigation

| Your job | Start here | Decision it supports |
| --- | --- | --- |
| **Applied-AI Engineer** | Isolate a busy interval, follow the Stack X-Ray, then inspect tool loops. | Which session needs intervention? |
| **Fleet Architect** | Compare harness activity, skill usage, and handoff flow. | Where should responsibilities or capabilities change? |
| **FinOps / Token Economist** | Group token components by workspace, harness, and outcome. | Where should you investigate budget concentration? |
| **Security & Governance Lead** | Review authority exposure alongside sandbox and hook observations. | Which dependencies and permission boundaries need review? |

## Evidence you can trust

Every investigation stays connected to its observation window and supporting records. Dendro keeps missing evidence visible, separates observations from simulations, and leaves fleet changes with the operator. Prompts, thoughts, responses, tool payloads, and secrets stay out of the analytics store.

Continue with the [Operator & Adoption Guide](adoption/README.md), inspect the [Architecture Blueprint](ARCHITECTURE.md), or [ask a question with Nao](NAO_SEMANTIC.md).
