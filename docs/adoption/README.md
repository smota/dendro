# Dendro — Guia de Adoção Rápida (5-Minute Quickstart)

> *"Assim como a dendrocronologia decifra a história climática através dos anéis de crescimento das árvores, o Dendro lê os anéis de sessões dos seus agentes de IA para revelar a saúde, os custos e os gargalos da sua equipe autônoma."*

---

## 1. O que é o Dendro?

O **Dendro** é um motor analítico e de telemetria *local-first* para frotas de agentes autônomos. Ele se conecta automaticamente aos logs das suas ferramentas de desenvolvimento (Claude Code, Codex, OpenRig, AGY, Grok, Pi), normaliza tudo no **DuckDB** e permite responder às três perguntas essenciais de qualquer operador de IA:

1. **Otimização de Assentos:** "Meus agentes estão passando tempo ocioso esperando o outro terminar? Tenho assentos a mais na equipe?"
2. **Eficiência de Handover:** "Quanto contexto e custo de tokens são desperdiçados quando um agente passa o trabalho para o próximo? Há retrabalho entre QA e Dev?"
3. **Controle de Recursos & Custos:** "Qual missão consumiu mais orçamento? Algum agente entrou em loop infinito de comandos falhos?"

---

## 2. Instalação e Execução em 5 Minutos

### Pré-requisitos
* Rust (versão estável 1.80+)
* Git

### Passo 1: Clone e compile o binário
```bash
git clone https://github.com/smota/dendro.git
cd dendro
cargo build --release
```

### Passo 2: Execute a primeira ingestão local
O comando `run` descobre automaticamente os logs de agentes presentes na sua máquina (`~/.openrig/`, `~/.claude/`, `~/.codex/`, `~/.gemini/`) e popula o banco local `dendro.duckdb`:

```bash
./target/release/dendro run --db dendro.duckdb
```

### Passo 3: Verifique o status da sua frota
```bash
./target/release/dendro status --db dendro.duckdb
```

Você verá um resumo executivo com:
* Total de sessões catalogadas por harness.
* Rigs do OpenRig identificados e assentos mapeados.
* Contagem total de chamadas de ferramenta e volume de tokens.
* Estimativa de investimento/custo acumulado.

---

## 3. Modo Contínuo (Background Watcher)

Para manter o seu banco de telemetria atualizado em tempo real enquanto os agentes trabalham:

```bash
./target/release/dendro watch --db dendro.duckdb --debounce-secs 2
```

O daemon monitora as alterações no sistema de arquivos com debounce de 2 segundos para evitar sobrecarga durante o streaming rápido de tokens de LLM.

---

## 4. Consultas em Linguagem Natural com o Nao

O Dendro já vem com templates semânticos pré-configurados para a ferramenta [Nao](https://github.com/getnao/nao):

```bash
./target/release/dendro init-nao --target . --db dendro.duckdb
nao "Qual assento teve o maior tempo ocioso esta semana?"
nao "Compare a taxa de falha de ferramentas entre o Claude Code e o Codex"
nao "Qual missão gerou mais loops de repetição de comandos bash?"
```

---

## 5. Garantia de Privacidade

* **100% Local:** Nenhum byte de telemetria sai da sua máquina. Não há telemetria para a nuvem.
* **Sem vazamento de segredos:** Tokens de API (`sk-...`, `ghp_...`, `Bearer ...`) e credenciais são automaticamente ofuscados antes de entrar no DuckDB.
