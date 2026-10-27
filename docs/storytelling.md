# Dendro: Storytelling & Product Manifesto

> *"Just as dendrochronology deciphers centuries of climate history through tree growth layers, Dendro reveals the cumulative execution layers of autonomous AI squads to uncover team dynamics, bottlenecks, and real operational costs."*

---

## 1. O Manifesto: Da "Maçaroca" à Inteligência

### O Novo Mundo dos Agentes
Nos últimos meses, a forma de programar mudou radicalmente. Deixamos para trás os chats simples de IA e entramos na era das **equipes de agentes autônomos**.
Hoje, em uma única máquina de desenvolvedor convivem múltiplos agentes e ferramentas:
* **Claude Code** refatorando arquivos em loop.
* **Codex** executando tarefas em segundo plano.
* **AGY** gerindo planos e passos complexos.
* **Grok** e **Pi** atuando como especialistas pontuais.
* E orquestradores modernos como o **OpenRig**, gerenciando assentos (*seats*), missões e transições de fila.

### O Paradoxo da Visibilidade (A "Maçaroca de Dados")
Quanto mais agentes você tem trabalhando juntos, mais cego você fica.
* O terminal vira uma enxurrada ininterrupta de chamadas `bash`, `edit_file` e contagens de tokens soltas.
* Quando uma missão demora ou custa caro, você não sabe se o problema foi um prompt ruim, um loop infinito de compilação ou um handover desastroso entre agentes.
* As ferramentas existentes de observabilidade (APMs de nuvem) tratam agentes como se fossem APIs web normais — despejando métricas estéreis de latência em dashboards que ninguém olha.

### A Inspiração: Dendrocronologia
Na natureza, uma árvore não descarta o seu passado. Cada ano de seca, cada estação chuvosa, cada queimada e cada período de crescimento acelerado fica gravado com precisão cirúrgica em suas **camadas concêntricas de crescimento**. Cientistas usam a **dendrocronologia** para olhar para o tronco e reconstruir séculos de história climática.

**No ecossistema de software, as sessões e transições dos seus agentes são essas camadas.**
Cada chamada de ferramenta, cada transição entre assentos, cada evento de coordenação é uma camada viva de evolução do seu sistema. 

O **Dendro** é a ciência e o instrumento que decifra essas camadas — transformando logs fragmentados na crônica viva e auditável da sua frota autônoma.

---

## 2. The instrument you can actually open

The manifesto above is the reason. The Connected Dashboard Suite is the instrument. On v0.8.0 it is one loopback server:

```bash
dendro dashboard --serve 127.0.0.1:4242
```

`127.0.0.1`, `localhost`, and `[::1]` are allowed. Any other bind is refused. The hub is `site/dashboards/index.html`. The Fleet Explorer is `site/viewer.html`. A picture is an **ILLUSTRATIVE SNAPSHOT** until a live local poll returns a real object and the badge changes to **LIVE TELEMETRY**.

In the **Live Connected Web Studio** (`dendro dashboard --serve`), `/api/query` is the JSON query projection endpoint and `/api/events` is the Server-Sent Events live invalidation stream. The Time Machine Scrubber actively filters live SQL queries (`WHERE timestamp >= ? AND timestamp < ?`) across the Hub and all 12 views. Presets `today`, `24h`, `7d`, `30d`, `this_month`, `last_month`, `all`, and custom `YYYY-MM-DD..YYYY-MM-DD` selections persist across navigation through URL query params and `localStorage`.

In **Offline Materialized Nao Views** (`dendro materialize`), the four v0.8.0 domain views (`v_message_mix`, `v_subagent_lineage`, `v_security_posture`, `v_process_resources`) are all-time aggregates: `window_from` / `window_to` label the snapshot without filtering rows. The four OpenRig-compatibility views (`v_mission_token_burn`, `v_handoff_rework_analysis`, `v_seat_idle_ranking`, `v_executive_governance`) are scoped to the materialized `[from, to)` window.

The full visual reading is [VISUAL_PRODUCT_TOUR.md](VISUAL_PRODUCT_TOUR.md). The capability boundaries are [capabilities.md](capabilities.md). Nothing in this chapter adds a claim those two documents refuse.

## 3. Living Growth Rings

Six terminals scrolling is the maçaroca. The rings are the trunk.

Four rings, forty-eight sectors, one heartwood readout:

* **Model** holds token density and cache yield (`I02`). Money stays on `I04`.
* **Harness** holds execution rhythm (`I08`, `I22`, `I23`).
* **Coordination** holds handoffs and queue wait (`I09`, `I10`, `I12`).
* **Environment & Security** holds placement, resources, and grants (`I14`, `I18`, `I24`).

You scrub or rotate. The sector under the hand tells you which layer moved. You isolate one ring when the others are noise. A glowing scar is an invitation to look. It is not a verdict.

### Tactile Time Machine Scrubber

The window is a physical control, not a query string you hope every chart honored. Presets: Today (24h), Last Week (7d), 30 Days, This Month, Last Month, All Time. The custom date range picker plus rounded calendar labels make the slice obvious. Two handles trim the sparkline. That slice travels with you into the X-ray, the studio, and the consoles. The interval is half-open: `[from, to)`.

### Apple Health Story Highlights

Under the rings, three cards speak in sentences a person can repeat:

1. How much of the input the cache already knew (`I02`). Reasoning tokens stay inside output. They are not added a second time.
2. How the harnesses took turns — Claude Code, Codex, AGY, Grok, Pi, OpenRig — without quoting a single message body.
3. Whether the fleet's weather was clear, rework, or a vortex (`I22`), and how many handoffs came back.

The sentences are summaries of evidence. They are not a health score for the team.

## 4. Four layers, pulled apart

The Exploding Stack X-Ray is the same trunk with the rings unrolled into planes:

1. **Model.** Calls, cache, uncached input, pinned-tier simulation.
2. **Harness.** Tools, messages, funnel, trajectory.
3. **Coordination.** Spawn, delegate, handoff, return, queue.
4. **Environment.** Sandbox, grant, CPU, RSS.

**Cmd+E** explodes the stack into a 3D isometric view. Swimlane mode keeps one clock, the way a recording engineer keeps one timeline. Click a node to read the insight id on that span. A thread between planes shows that the events sat in one window. It does not show that one caused the next.

The multi-dimensional Group-By explorer pivots the window three ways: by harness and tool, by architectural layer, and by date bucket. Instant pivots are still the same facts. Summing a pivot twice double-counts.

## 5. Five layers of capability

The agent-otel-bridge studio answers a different question than the rings. The rings say when the fleet was loud. The studio says **what kind of loud**.

The Apple bento console dock has five layers:

| Layer | What you see |
| --- | --- |
| Tool families | Shell/Exec, Read/Inspect, Edit/Patch, Search/Grep, Subagent Spawn, MCP Call, Skill Load, Web/Fetch, Task/Queue, Governance/Gate |
| Harness primitives | The built-in verbs under those families |
| Skills | Declared versus invoked (`I06` `harness.capability_funnel`) |
| MCP servers | Call counts for `mcp__*` servers, never the arguments |
| Lifecycle hooks | `HookEvent` counts and guardrails |

`I06` is the funnel. `I13` is where a process sat, not which skill ran. A dormant skill was not invoked in the slice. It was not deleted. Archetype tiles such as `inspector_diff` and `generic_exec` are the bridge's finer labels for the same tool facts. Neither label keeps the command text.

## 6. Four active consoles

I22 through I25 are the bespoke SVG consoles. Each one has a non-claim printed on the glass.

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

The following descriptions are the **Mode A reference contracts**, distinct from the pictured Mode B proxies:

* **I22 Trajectory Vortex.** A 32-dimensional cosine orbit around the completed cohort. You see a behavioral loop before it becomes runaway token burn. A trip is **not a cause**. A short cohort refuses with `insufficient_sample`. View: `v_trajectory_vortex`.
* **I23 Schema Drift.** A DNA fingerprint ribbon of L1 structural drift across Claude Code, Codex, AGY, Grok, Pi, and OpenRig. Added, removed, and retyped keys stay apart. A short pair is `insufficient_sample`. Drift is not a compatibility verdict. View: `v_schema_drift`.
* **I24 Authority Blast Radius.** A radial ripple over an 8-hop `duckpgq` walk. Only edges with `status = evidenced` count. The number is **dependency exposure**, not inherited authority and not effective authority. View: `v_authority_blast_radius`.
* **I25 Counterfactual Routing.** A split beam. One side is what the model did. The other is a fast-tier simulation at a cache-bleed inflection you scrub. Unpinned tiers stay unknown, and unknown is not zero. The picture is **not an effect** and **not a routing change**. View: `v_counterfactual_routing`.

The economics page sits beside them. Task and actor Group-By pivots read token burn for the selected window. The tour's economics screenshot shows **6,848.31M / 6.85B total tokens** and **96.9% cache-read efficiency** for that recording. Your next slice can differ. Cache-read ratio of sums is not the mean of ratios. Cost stays on `I04`. CPU stays on `I14`.

## 7. Who the story is for

* **Applied-AI Engineer.** Find the stalled span. Separate active work, human wait, handoff wait, and rework. Leave with a tool family, not a vibe.
* **Fleet & Harness Architect.** Compare envelopes (`I23`) and orbits (`I22`) across harnesses. Check that declared skills were the ones invoked (`I06`).
* **FinOps & Token Economist.** Pivot actor and task. Price a Mode A tier simulation only with a pin (`I25`); Mode B shows a labeled normalized-unit proxy. Refuse any sentence about forecasted savings. `dendro ask` already refuses that wording.
* **Security & Governance Lead.** Read the ripple (`I24`) as exposure. Leave `NOT_ASSESSED` as not assessed (`I17`). Count redactions (`I18`) without revealing the secret that was removed.

## 8. Honest measurement, said once

Three rules keep the chronicle from becoming another vanity dashboard.

**Null is not zero.** No row, no denominator, no pinned price, no lazy table: that is missingness. The badges are `ok`, `partial`, `insufficient_evidence`, `refused`, and `not_applicable`. `not_observable` and `insufficient_sample` surface as insufficient evidence. A status line of `not created` is not an empty fleet you measured.

**Observed versus counterfactual.** Observed facts are what the logs supported. `I25` is a simulation with the label still on it. `I21` compares two observed windows and says `noncomparable` when they are not the same kind of thing. A vortex does not explain itself. A reachable session did not inherit a grant.

**Zero prompt and zero secret capture.** The tree keeps the ring, not the conversation. Prompts, thoughts, replies, tool arguments, and tool results never enter the store. Secret-shaped values are masked in every field. The public explorer aliases actors. L0 tells you the finding and the gap. L1 tells you the assumptions. L2 gives at most five sealed evidence ids. L3 shows the formula, who was included, who was excluded, and the cutoff.

The other insights are still in the trunk. I01 through I21 are invocations, tokens, inclusive tokens, cost, configuration divergence, the capability funnel, context pressure, tool loops, delegation chains, duration, shared allocation, queue wait, placement, process resources, effects, isolation, authority, redactions, acceptance, resource efficiency, and descriptive comparison. I22, I23, I24, and I25 are the four consoles above. Presentation names you will also see on the glass — `model.tokens.direct`, `harness.action_failure`, `work.lineage_shape`, `environment.resources`, `coverage.source` — are aliases of I02, I08, I09, I14, and I18. They are not new insights.

Every catalog id, once each: I01, I02, I03, I04, I05, I06, I07, I08, I09, I10, I11, I12, I13, I14, I15, I16, I17, I18, I19, I20, I21, I22, I23, I24, I25.

Dendro will not invent hours saved, will not call a busy terminal a finished job, and will not touch live routing. The operator still governs the fleet. The rings only make the governing visible.
