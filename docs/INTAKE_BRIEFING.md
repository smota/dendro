# INTAKE BRIEFING: Dendro — Product Discovery & Solution Design

**Sponsor:** Sam (via Antigravity Pairing)  
**Target Squad:** `dendro-pm` (`pm-manager`, `pm-analyst`) & `dendro-dev` (`orch-arch`, `dev-build*`, `dev-qa`, `rev-review`)  
**Data:** 10 de Outubro de 2026  
**Status:** Fase 0 — Descoberta de Produto, JTBD e Desenho da Solução  

---

## 1. Visão Geral e Contexto de Origem

O **Dendro** nasceu da necessidade real de telemetria, observabilidade e governança analítica para equipes de agentes autônomos (multi-agent fleets). O nome é inspirado na **dendrocronologia** (*a ciência de datar e compreender eventos climáticos e históricos através dos anéis de crescimento das árvores*). No nosso ecossistema, os anéis são as sessões, transições de fila, trocas de assento e chamadas de ferramenta executadas pelos agentes de IA.

Na discussão fundacional (registrada em `docs/research/openrig-discussion-1099.md` do OpenRig), foram identificadas as **três dores centrais** que o produto deve solucionar:

1. **Otimização de Assentos (Seat Optimization):**
   * Identificar ociosidade real vs. trabalho ativo dos assentos.
   * Evitar over-engineering de assentos (ex: pipelines longos onde assentos intermediários apenas repassam texto sem agregar valor).
   * Medir tempo de convergência por tarefa e gargalos no fluxo de entrega.

2. **Eficiência de Handover (Handover Efficiency):**
   * Medir perda de contexto ou inchaço excessivo de tokens durante transferências entre assentos (ex: PM -> Arquiteto -> Builder -> QA -> Reviewer).
   * Monitorar taxa de retrabalho (quantas vezes uma tarefa volta de QA ou Review para o Builder).

3. **Domínio de Recursos e Custos (Resource & Cost Mastery):**
   * Aferir o consumo real de tokens (input, output, cache read/write) agrupados por Missão, Rig e Assento.
   * Detectar loops de retry (ex: comandos bash repetitivos com falha).
   * Alertar anomalias e queima excessiva antes que esgote o orçamento ou cause estagnação.

---

## 2. Dados Reais Já Existentes para Testes e Validação

Você tem dados reais de sessões rodando nesta máquina local para servir de fixtures e validação empírica:
* **OpenRig:** `~/.openrig/openrig.sqlite` e estados em `~/.openrig/state/` (incluindo os rigs reais `dendro-dev`, `dendro-pm`, `agentflow-dev`, `agentflow-pm`).
* **Claude Code:** Sessões em `~/.claude/projects/` com metadados completos de tokens, ferramentas e seats vinculados.
* **Codex:** Sessões em `~/.codex/sessions/` (`rollout-*.jsonl`).
* **Pi / Grok:** Logs em `~/.pi/agent/sessions/` e `~/.grok/sessions/`.
* **AGY:** Transcrições em `~/.gemini/antigravity-cli/brain/**/transcript.jsonl`.

---

## 3. Escopo Geral da Solução Completa

O sponsor (Sam) estabeleceu uma visão de produto completa que abrange:

### A. Engine Local-First de Telemetria (Rust + DuckDB + Nao)
* **Core Rust Workspace:**
  * Ingestão incremental orientada a watermarks (byte offsets sem re-leitura de gigabytes).
  * Conectores modulares para Claude Code, Codex, AGY, Grok, Pi e OpenRig.
  * Resolução de correlação: amarração entre logs de harnesses locais e as entidades do OpenRig (Rigs, Assentos, Missões).
  * Storage colunar em DuckDB (`dendro.duckdb`).
  * CLI executável: `dendro run`, `dendro watch` (daemon debounced), `dendro status` e `dendro init-nao`.
  * Camada semântica Nao (`templates/nao/nao_config.yaml`) com dashboards iniciais e consultas em linguagem natural.

### B. Presença Digital & Website de Produto
* Sugestão de domínio disponível de alta memorabilidade (preferência `.ai`, ex: `dendro.ai`, `dendrorings.ai`, etc.).
* Site completo estático/SPA com navegação fluida:
  * Landing page de alta conversão para líderes técnicos e operadores de agentes (Pitch para CTOs/Head de IA).
  * Demonstração visual interativa da metáfora dos anéis da árvore.
  * Seção de casos de uso reais (resolvendo as 3 perguntas).
  * Documentação interativa integrada (Quickstart, Guias).
* Identidade visual e logotipo (conceito visual limpo, anéis de árvore estilizados como grafos de telemetria).

### C. Documentação Oficial Integrada
* **Documentação de Mantenedores:** Guia de arquitetura, CI/CD, convenções de código Rust e contratos de governança.
* **Documentação de Extensibilidade:** Como plugar novos harnesses (ex: Cursor, Devin, Goose, Windsurf) através das traits canônicas.
* **Documentação de Adoção:** Guia passo-a-passo para times começarem a rodar em 5 minutos com DuckDB local.

---

## 4. O que é Esperado do PM (`pm-manager`) Agora

Neste momento, você deve conduzir a **Fase 0 de Descoberta**:

1. **Entendimento & JTBD (Jobs-to-be-Done):**
   * Mapeie a situação, motivação e resultado esperado: *"Quando [situação], quero [motivação], para que [resultado]"*.
   * Defina a mudança observável para o usuário final.

2. **Breakdown Completo de Escopo & Roadmap:**
   * Decomposição estruturada em **Goals**, **Épicos / Histórias de Usuário** e **Critérios de Aceitação**.
   * Matriz de prioridades (MVP imediato para entrega amanhã vs. melhorias incrementais).

3. **Desenho da Solução (Architecture & Experience Blueprint):**
   * Estrutura do produto, fluxos de uso, schemas analíticos, primeiros dashboards do Nao, arquitetura do site/landing page e especificações de documentação.

4. **Entrega do Documento:**
   * Registre o resultado completo no arquivo local `docs/PRODUCT_SPECIFICATION.md` e em `.agentflow/PRODUCT.md`.
   * Envie uma mensagem de conclusão detalhada quando o documento estiver finalizado para que possamos apresentar ao Sam para revisão e Go-Ahead oficial!
