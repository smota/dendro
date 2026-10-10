# Dendro: Technical Architecture & System Blueprint

## 1. Visão Geral

Dendro is a local AI Fleet Intelligence engine, written in Rust. Operational Intelligence is the evidence about seats, handoffs, and tokens. Fleet Governance stays with the operator. 

Ele resolve a lacuna de visibilidade em ambientes multi-agentes e multi-runtimes unificando:
1. **Camada de Orquestração / Topologia (OpenRig)**: Rigs, Assentos (*seats*), Missões, Filas e Watchdogs.
2. **Camada de Execução de Agentes (Harnesses)**: Claude Code, Codex, AGY, Grok, Pi.
3. **Engine de Correlação Extensível**: Amarra a execução física ao contexto de negócio através de Resolvers desacoplados.
4. **Armazenamento e Semântica (DuckDB + Nao)**: Banco analítico colunar local consultado via linguagem natural pelo framework Nao (`getnao/nao`).

---

## 2. Diagrama de Arquitetura

```
┌────────────────────────────────────────────────────────────────────────┐
│                        FONTES DE TELEMETRIA LOCAL                      │
│                                                                        │
│   [Runtime Layer]                   [Agent Harness Layer]              │
│   ~/.openrig/                       ~/.claude/    ~/.codex/            │
│   (openrig.sqlite, context-usage)   ~/.gemini/    ~/.grok/   ~/.pi/    │
└───────────────────────────────────┬────────────────────────────────────┘
                                    │
                                    ▼
┌────────────────────────────────────────────────────────────────────────┐
│                         DENDRO CORE (RUST)                             │
│                                                                        │
│  ┌───────────────────────┐             ┌────────────────────────────┐  │
│  │   File Watcher Mode   │             │       Batch CLI Mode       │  │
│  │    `dendro watch`     │             │        `dendro run`        │  │
│  └───────────┬───────────┘             └─────────────┬──────────────┘  │
│              └───────────────────┬───────────────────┘                 │
│                                  ▼                                     │
│                     Watermark & Offset State                           │
│                      (Leitura Incremental)                             │
│                                  │                                     │
│         ┌────────────────────────┴────────────────────────┐            │
│         ▼                                                 ▼            │
│   [Runtime Connectors]                          [Harness Connectors]   │
│   • OpenRigConnector                            • ClaudeCodeConnector  │
│   • StandaloneConnector                         • CodexConnector       │
│                                                 • AgyConnector         │
│                                                 • GrokConnector        │
│                                                 • PiConnector          │
│         │                                                 │            │
│         └────────────────────────┬────────────────────────┘            │
│                                  ▼                                     │
│                    Correlation Engine (Resolvers)                      │
│                • OpenRigResolver (bindings/tmux/pid)                   │
│                • StandaloneResolver (execução direta)                  │
└──────────────────────────────────┬─────────────────────────────────────┘
                                   │
                                   ▼
┌────────────────────────────────────────────────────────────────────────┐
│                  STORAGE LAYER (DuckDB: `dendro.duckdb`)               │
│                                                                        │
│  Tabelas Canônicas:                                                    │
│  • `fct_sessions`           • `fct_tool_calls`                         │
│  • `_dendro_watermarks`                                                │
└──────────────────────────────────┬─────────────────────────────────────┘
                                   │
                                   ▼
┌────────────────────────────────────────────────────────────────────────┐
│                 SEMANTIC & CONVERSATIONAL LAYER (NAO)                  │
│                                                                        │
│  • Configuração gerada por `dendro init-nao` (`nao_config.yaml`)       │
│  • Chat conversacional & gráficos nativos em linguagem natural        │
└────────────────────────────────────────────────────────────────────────┘
```
