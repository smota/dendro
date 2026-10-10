# Dendro — Hierarquia de Goals, Fluxos de Dados e Ciclos de Correção

**Autor:** Sponsor (Sam / Antigravity)  
**Destinatários:** `orch-arch@dendro-dev`, `dev-build*`, `dev-qa`, `rev-review`  
**Referência:** Release 1 (v0.1.0) — Épico #1  
**Status:** Diretriz Arquitetural Normativa  

---

## 1. Visão Geral: Por que a Hierarquia é Vital?

O desenvolvimento da Release 1 do Dendro **não pode ser um monólito de código único**. Para garantir previsibilidade, ritmo constante de entrega e **ciclos curtos de correção**, a solução é dividida em **5 Fluxos Lineares e Incrementais**.

Se um fluxo falhar na verificação de QA, o ciclo de correção é imediato e restrito ao seu escopo, sem contaminar os outros componentes.

```
 [Fontes Brutas Locais]
        │
        ▼
 ┌────────────────────────────────────────────────────────┐
 │ FLUXO 1: Ingestão Streaming & Watermarks Incrementais  │ ──► Ciclo 1: Prova de Ingestão e Integridade
 └────────────────────────────────────────────────────────┘
        │
        ▼
 ┌────────────────────────────────────────────────────────┐
 │ FLUXO 2: Resolução & Vínculo Determinístico de Sessão  │ ──► Ciclo 2: Prova de Vínculo (>=95%)
 └────────────────────────────────────────────────────────┘
        │
        ▼
 ┌────────────────────────────────────────────────────────┐
 │ FLUXO 3: Cálculo das Três Camadas Operacionais         │ ──► Ciclo 3: Prova das 3 Perguntas (O2, O3, O4)
 └────────────────────────────────────────────────────────┘
        │
        ▼
 ┌────────────────────────────────────────────────────────┐
 │ FLUXO 4: Índices de Decisão Executiva (M1 a M4)        │ ──► Ciclo 4: Prova de Valor do Operador (O8)
 └────────────────────────────────────────────────────────┘
        │
        ▼
 ┌────────────────────────────────────────────────────────┐
 │ FLUXO 5: Ponto de Entrada CLI (`run`, `status`)        │ ──► Ciclo 5: Prova de Usabilidade (< 5 min)
 └────────────────────────────────────────────────────────┘
```

---

## 2. A Hierarquia de Goals e Slices

### Nível 1: Goal Estruturante (O Épico)
* **Goal 1 (Issue #1):** Motor Local-First de Telemetria e Inteligência Analítica para Frotas de Agentes.
  * *Job do Operador:* Obter visibilidade empírica sobre tempo, tokens e retrabalho para tomar decisões de topologia e orçamento com dados reais.

---

### Nível 2: Slices de Implementação & Ciclos de Correção

#### Slice 1: Ingestão Segura & Watermarking (Issue #2 & #6)
* **Objetivo do Fluxo:** Ler as fontes heterogêneas sem carregar gigabytes na memória e salvar o estado de leitura.
* **Componentes:**
  * `dendro-core::storage`: Conexão DuckDB, migração idempotente, tabela `_dendro_watermarks`.
  * `dendro-core::privacy`: Opaque HMAC join keys para identificadores, sanitização de credenciais.
  * `dendro-connectors`: Leitura streaming via `BufReader` para OpenRig (`openrig.sqlite`), Claude Code (`.jsonl`), Codex (`rollout-*.jsonl`) e Pi (`sessions/*.jsonl`).
* **Ciclo de Correção & Critérios de Aceitação:**
  * `AC1`: Leitura executada em $\le 300$ segundos.
  * `AC9`: Segunda execução idempotente sem alteração de totais.
  * `AC11`: Zero prompts brutos e zero segredos no DuckDB.
* **Se quebrar aqui:** O erro está no parser de algum arquivo ou no lock de staging do DuckDB. O cálculo analítico ainda nem foi tocado.

---

#### Slice 2: Motor de Vínculo Determinístico (Issue #2)
* **Objetivo do Fluxo:** Cruzar as sessões dos harnesses com os assentos, rigs e missões sem heurísticas adivinhadas.
* **Componentes:**
  * `dendro-resolver`: Leitor do evento `agent.session_identity` do OpenRig.
  * Mapeamento 1:1 de `session_id` local para `(rig_id, seat_id, mission_id)`.
  * Sessões sem evento correspondente são rotuladas explicitamente como `unlinked`.
* **Ciclo de Correção & Critérios de Aceitação:**
  * `AC2`: Taxa de vínculo determinístico $\ge 95\%$ nas sessões locais reais (85 de 87 comprovadas).
* **Se quebrar aqui:** O erro está na extração da chave de identidade ou no filtro de normalização de timestamps.

---

#### Slice 3: As Três Camadas Operacionais (Issues #3, #4, #5)
* **Objetivo do Fluxo:** Transformar os fatos brutos vinculados nas 3 respostas centrais do operador.
* **Componentes:**
  * **Camada 1 — Assentos (`fct_seat_activity`):**
    * Segmentação em: Ativo (`running`), Ocioso com trabalho (`idle` com fila aberta — *desperdício*), Ocioso sem trabalho (*capacidade ociosa*), Espera humana (`needs_input`), Desconhecido.
  * **Camada 2 — Handoffs & Retrabalho (`fct_handoffs`):**
    * Grafo de transferências `handed_off_to`.
    * Detecção de voltas ao builder na cadeia do item (`rev-review -> dev-build*` ou `qa -> dev-build`).
    * Inchaço de contexto na entrada (*intake token cost*).
  * **Camada 3 — Custos & Loops (`fct_token_burn`, `fct_tool_loops`):**
    * Atribuição de tokens (input, output, cache read/write) e custos por missão/assento.
    * Detector de retry loops (comandos consecutivos repetidos com código de saída != 0).
* **Ciclo de Correção & Critérios de Aceitação:**
  * `AC3`, `AC4`, `AC5`, `AC6`, `AC7`, `AC8`.
  * `AC10`: Todo número aponta para o arquivo e linha de origem (*provenance trace*).
* **Se quebrar aqui:** O erro é na agregação analítica ou na regra de intervalo de tempo.

---

#### Slice 4: Métricas de Decisão do Operador (Issue #3)
* **Objetivo do Fluxo:** Calcular os 4 índices de governança para o operador tomar decisões de equipe:
  * **$M_1$ — Rendimento do Assento (*Seat Yield*):** Relação entre tempo ativo do assento e itens de fila entregues à frente.
  * **$M_2$ — Eficiência de Transferência (*Handoff Efficiency*):** Percentual de handoffs que completam sem rollback.
  * **$M_3$ — Índice de Retrabalho por Cadeia (*Rework Rate*):** Frequência em que itens em revisão/QA voltam para o assento builder.
  * **$M_4$ — Exposição de Skills (*Skill Exposure*):** Frequência de assentos operando com skills especializadas.
* **Ciclo de Correção & Critérios de Aceitação:**
  * Validação contra o rig de referência `agentflow-dev` (com seus 152 itens de fila históricos) e teste de valor $O_8$.

---

#### Slice 5: Interface de Entrega CLI (Issue #1 & #6)
* **Objetivo do Fluxo:** Expor o motor via linha de comando limpa e confiável.
* **Componentes:**
  * `dendro run --db dendro.duckdb`: Executa a pipeline completa (Fluxos 1 $\to$ 4).
  * `dendro status --db dendro.duckdb`: Renderiza a tabela executiva no terminal.
  * `dendro report --format json`: Export auditável para integrações e dashboards.
* **Ciclo de Correção & Critérios de Aceitação:**
  * Execução fluida sem crash, código de saída 0, logs claros informando fontes lidas e registros processados.

---

## 3. Diretriz para o Ritmo de Desenvolvimento

1. **Testes Unitários por Fluxo:** Cada fluxo deve ter seus testes unitários em seu respectivo crate antes de avançar para o próximo.
2. **Commit por Fluxo:** Commits isolados com referência à issue (`git commit -m "feat(core): implement flow 1 streaming ingestion (Refs #2, #6)"`).
3. **QA Independente:** O assento `dev-qa@dendro-dev` valida fluxo por fluxo com o tracer do sistema antes de submeter ao `rev-review`.
