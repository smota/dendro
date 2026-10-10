# Dendro — Guia do Mantenedor (Maintainer Guide)

Bem-vindo à documentação interna de mantenedores do **Dendro**. Este guia documenta a arquitetura de engenharia, convenções de código em Rust, modelo de concorrência, governança de ciclo de vida e padrões de qualidade do projeto.

---

## 1. Visão Arquitetural do Core

O Dendro é um motor analítico *local-first* construído em Rust modular sobre o **DuckDB**:

```
[Fontes Locais de Sessões]
 ├── OpenRig (~/.openrig/openrig.sqlite, state/)
 ├── Claude Code (~/.claude/projects/*/*.jsonl)
 ├── Codex (~/.codex/sessions/**/rollout-*.jsonl)
 ├── AGY (~/.gemini/antigravity-cli/brain/**/transcript.jsonl)
 ├── Grok (~/.grok/sessions/**/summary.json)
 └── Pi (~/.pi/agent/sessions/**/*.jsonl)
       │
       ▼
 [dendro-connectors] ───► Implementa `HarnessConnector` (streaming zero-copy via BufReader)
       │
       ▼
 [dendro-resolver]   ───► Implementa `CorrelationResolver` (amarra sessão a Rig, Assento e Missão)
       │
       ▼
 [dendro-core]       ───► DuckDbStorage (migração idempotente, inserção transacional em lote)
       │
       ▼
 [dendro.duckdb]     ───► Tabelas dimensionais e analíticas colunares
```

---

## 2. Padrões de Engenharia em Rust

### 2.1. Desempenho & Eficiência de Memória (Zero-Copy)
* **Streaming de logs grandes:** Logs de harnesses (Claude Code, Codex, AGY) podem somar centenas de megabytes. **Nunca** leia arquivos completos para a memória com `fs::read_to_string`. Utilize `BufReader::new(file).lines()` para consumo linha a linha.
* **Watermarks Incrementais:** Cada ingestão armazena em `_dendro_watermarks` o byte offset e o timestamp da última leitura. Na execução seguinte, o conector busca diretamente o offset (`seek(SeekFrom::Start(offset))`).
* **DuckDB Transactions:** Operações de escrita em lote devem ser envelopadas em uma única transação (`BEGIN TRANSACTION` ... `COMMIT`). Evite inserts individuais linha a linha.

### 2.2. Segurança e Defesa em Profundidade
* **Safe Rust Estrito:** Proibido o uso de `unsafe` blocks em todos os crates do workspace (`#![deny(unsafe_code)]`).
* **Prevenção de SQL Injection:** Parâmetros de consulta devem obrigatoriamente utilizar `duckdb::params![...]` ou prepared statements. Concatenação de strings em SQL (`format!("SELECT ... WHERE id = '{}'")`) é reprovada nos gates de CI.
* **Sanitização de Segredos:** Prompts e comandos contendo credenciais conhecidas (`sk-`, `ghp_`, `gho_`, `Bearer `, `xai-`) devem ter seus tokens ofuscados antes da persistência analítica.

---

## 3. Estrutura do Workspace Cargo

```
crates/
├── dendro-core/         # Schemas canônicos (CanonicalSession, CanonicalToolCall), traits fundamentais, DuckDbStorage
├── dendro-connectors/   # Adaptadores de parsing especializados por harness
├── dendro-resolver/     # Algoritmos de correlação e amarração (OpenRigResolver, StandaloneResolver)
├── dendro-watcher/      # Monitoramento em tempo real via notify-debouncer-mini
└── dendro-cli/          # Ponto de entrada binário com comandos CLI (run, watch, status, init-nao)
```

---

## 4. Quality Gates & Comandos de Validação

Antes de enviar qualquer pull request ou submeter ao reviewer:

```bash
# 1. Compilação estrita
cargo check --workspace

# 2. Execução da suíte de testes
cargo test --workspace

# 3. Linter com zero warnings tolerados
cargo clippy --workspace -- -D warnings

# 4. Formatação canônica
cargo fmt --check
```

---

## 5. Governança e Operação Agêntica

Este repositório é governado pelo **AgentFlow SDLC** em postura **autônoma**:
* **Branch Protection:** Nunca commite diretamente na branch `main`. Todo trabalho move-se por worktrees dedicadas em `.worktrees/<feature-name>`.
* **Four-Eyes Estrito:** O builder nunca aprova seu próprio pull request (`allowsSelfReview: false`). O revisor independente em `rev.review` audita o commit SHA final antes do merge.
