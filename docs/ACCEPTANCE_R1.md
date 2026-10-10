# Dendro R1 — Aceitação (Fase 1)

| Campo | Valor |
| --- | --- |
| Dono | `pm.analyst` (`pm-analyst@dendro-pm`) |
| Entrada | Fase 0 `dendro-r1-p0`, `docs/PRODUCT_SPECIFICATION.md` |
| Data | 2026-10-10 |
| Estado | **Congelado** em 2026-10-10. Sponsor aprovou às 17:01Z. Decisão PM: prosseguir (`docs/PRODUCT_SPECIFICATION.md` §12). A1, A2, A5, M1 e M4 aceitos. |
| Próximo dono | `orch-arch@dendro-dev` (Fase 2) |

Este documento define o que o G1 precisa provar. Ele não escolhe arquivos, crates, schemas nem consultas. Isso é do `orch.arch`.

---

## 1. Conclusão

**O G1 é viável com os dados desta máquina. Três das quatro métricas de decisão do O8 são mensuráveis. O impacto de skills não é mensurável na R1.**

Quatro fatos dos dados mudam o escopo:

| # | Fato (medido em 2026-10-10, somente leitura) | Consequência |
| --- | --- | --- |
| F1 | `dendro-dev` foi criado hoje às 15:38Z. Tem 0 itens de fila. `agentflow-dev` tem 152 itens e 2 dias de trabalho. | O rig de referência da aceitação é `agentflow-dev`. O O8 em `dendro-dev` só é observável depois que ele trabalhar. |
| F2 | `dendro-dev` usa só Codex e Pi. Em `agentflow-dev`, `orch.arch` e `rev.review` rodam em Pi. | **O Pi é obrigatório para o O8.** Sem Pi, os assentos mais ativos não têm custo. O sponsor aceitou (A1). |
| F3 | Há 0 handoffs diretos `dev-qa → dev-build`. Em `agentflow-dev`, o trabalho volta ao build por `rev-review → dev-build*` (9). | "Retrabalho QA→Build" precisa de uma definição pela cadeia do item, não pela aresta direta. Ver M3. |
| F4 | Skills aparecem como carregadas no início (452 entradas de projeção). O Claude Code registra 0 invocações de skill. Não há base de comparação "com e sem skill" para o mesmo papel. | O impacto de skills mostra "dados insuficientes" na R1. A R1 entrega só a exposição a skills. |

---

## 2. Mapa de capacidades

Dependências em uma direção, de cima para baixo. A ordem de construção segue os números.

```
 1 source-discovery ──► 2 session-seat-linking ──┬──► 4 seat-activity      (Pergunta 1)
        │                                        ├──► 5 handoff-flow       (Pergunta 2)
        ▼                                        └──► 6 token-burn         (Pergunta 3)
 3 incremental-ingest                                     │
   (O6, envolve 1)                                        ▼
                                            7 decision-metrics (O8: M1–M4)
 transversais, aplicadas a todos:  8 provenance-trace (O5)   9 privacy-guard (O7)
```

| ID | Capacidade | Saídas | Outcome |
| --- | --- | --- | --- |
| `source-discovery` | Encontra e lê as fontes conhecidas. | Fontes encontradas, ausentes, registros lidos, avisos. | O1, G1.1 |
| `incremental-ingest` | Lê só o que mudou. | Totais estáveis entre execuções. | O6, G1.6 |
| `session-seat-linking` | Liga cada sessão de harness a rig, assento e missão. | Vínculos, "sem vínculo", taxa de vínculo. | G1.2 |
| `seat-activity` | Ativo e ocioso por assento. | Pergunta 1. | O2, G1.3 |
| `handoff-flow` | Handoffs, retornos, custo de entrada. | Pergunta 2. | O3, G1.4 |
| `token-burn` | Tokens por missão e assento, loops. | Pergunta 3. | O4, G1.5 |
| `decision-metrics` | M1–M4 para um rig. | Métricas de decisão. | O8, G1.8, G1.9 |
| `provenance-trace` | Leva cada número à origem. | Caminho até a origem. | O5, G1.7 |
| `privacy-guard` | Mantém conteúdo e segredos fora. | Armazenamento só com números e identificadores. | O7 |

---

## 3. Fontes e vínculo

| Harness | Fonte de sessões | Tokens | Custo em moeda | Vínculo com o assento | Status na R1 |
| --- | --- | --- | --- | --- | --- |
| OpenRig | `~/.openrig/openrig.sqlite` | Amostras de contexto | Não | É a fonte do vínculo | Obrigatório |
| Claude Code | `~/.claude/projects` | Sim | Não | 35/35 sessões encontradas | Obrigatório |
| Codex | `~/.codex/sessions` | Sim | Não | 23/23 sessões encontradas | Obrigatório |
| Pi | `~/.openrig/state/pi/<sessão>/sessions` (não `~/.pi`) | Sim | Sim | 27/29 sessões encontradas | **Obrigatório (F2)** — aceito pelo sponsor |
| AGY | `~/.gemini/antigravity-cli/brain` | A verificar | A verificar | Nenhum nó OpenRig usa AGY hoje | Em seguida |
| Grok | `~/.grok` | A verificar | A verificar | Nenhum nó OpenRig usa Grok hoje | Em seguida |

O vínculo é determinístico: o OpenRig registra o evento `agent.session_identity` com o id nativo da sessão. Taxa medida hoje: 85/87 (98%). Nenhuma heurística (cwd, pid, horário) é necessária para a R1.

---

## 4. Definições das métricas (resolve U1–U8)

Cada métrica é exibida com o nome, a definição abaixo e o caminho até a origem.

### Pergunta 1 — Assentos (U2)

| Métrica | Definição |
| --- | --- |
| Ativo | Tempo em que o estado de atividade do assento é `running`. |
| Ocioso com trabalho | Estado `idle` enquanto o assento tem um item de fila `pending` ou `in-progress` como destino. Isto é desperdício. |
| Ocioso sem trabalho | Estado `idle` sem item de fila aberto. Isto é falta de demanda. |
| Esperando humano | Estado `needs_input`. |
| Desconhecido | Estado `unknown` ou sem evidência. Nunca entra em ativo nem em ocioso. |

Um intervalo dura do evento de estado até o próximo evento do mesmo assento, cortado pelos limites do período.

### Pergunta 2 — Handoffs (U3)

| Métrica | Definição |
| --- | --- |
| Handoff | Um item de fila fechado com `handed_off_to`, de um assento para outro. |
| Retorno | Um handoff que leva o trabalho de volta a um assento que já tocou o mesmo item ou a mesma cadeia de registro. |
| Custo de entrada | Tokens de entrada que o assento de destino consome entre reivindicar o item e a primeira ação de escrita (edição, escrita de arquivo, atualização de fila ou handoff). Rótulo obrigatório: "estimativa". |
| Tempo de espera | Do handoff até a reivindicação pelo destino. |

### Pergunta 3 — Tokens e loops (U4)

| Métrica | Definição |
| --- | --- |
| Tokens | Entrada, saída, leitura de cache e escrita de cache, como a fonte registra. Somados por sessão, assento, missão e período. |
| Missão | Valor da tag `mission:<id>` do item de fila. Trabalho sem tag aparece como "sem missão". |
| Loop de ferramenta | O mesmo assento executa a mesma ferramenta com a mesma entrada normalizada, com falha, 3 ou mais vezes seguidas, em até 10 minutos. |
| Loop de handoff | O mesmo item ou cadeia passa entre o mesmo par de assentos mais de 2 vezes. |
| Custo em moeda | Exibido só quando a fonte registra o custo (Pi) ou quando o operador fornece uma tabela de preços. O Dendro nunca inventa um preço. |

### Métricas de decisão do O8 (U7, U8)

| ID | Métrica | Definição | Mensurável na R1? |
| --- | --- | --- | --- |
| M1 | Rendimento do assento (substitui "ROI do assento") | Tokens do assento divididos pelo número de handoffs que ele entregou e que não voltaram para ele. Mostra também tokens por item fechado. | Sim |
| M2 | Eficiência de handoff | Retornos ÷ handoffs, tempo de espera mediano e custo de entrada mediano, por par de assentos. | Sim |
| M3 | Retrabalho de build | Número e proporção de cadeias em que o trabalho volta a um assento `dev-build*` depois de passar por `dev-qa` ou por um assento de revisão. Mostra o caminho real (ex.: `dev-qa → rev-review → dev-build`). | Sim |
| M4 | Exposição a skills | Quais skills cada assento tinha carregadas e quais invocou, quando a fonte registra. O impacto (com vs. sem) mostra "dados insuficientes" até existir uma comparação válida (ver A7). | Só exposição |

M1 não mede a qualidade do código. Ele mede o que o assento consumiu e o que passou adiante sem voltar. O nome "ROI" promete valor, e o Dendro não mede valor. Por isso o nome é "rendimento".

---

## 5. Critérios de aceitação

Todos rodam nesta máquina, com dados reais, sobre `agentflow-dev` de 2026-10-08 a 2026-10-10, salvo indicação. Os números de referência vêm da leitura de 2026-10-10 e podem crescer. O teste compara com a fonte no momento da execução.

### AC1 — Primeira execução (O1, G1.1)

- **Dado** uma instalação nova e as fontes desta máquina,
- **quando** o operador roda o comando de ingestão uma vez,
- **então** a execução termina em até 5 minutos de relógio,
- **e** lista cada fonte da tabela §3 como encontrada ou ausente, com o número de registros lidos,
- **e** uma fonte ausente gera um aviso e não interrompe a execução.

### AC2 — Formato desconhecido (restrição 5)

- **Dado** um arquivo de sessão com uma linha que não segue o formato conhecido,
- **quando** a ingestão roda,
- **então** a linha gera um aviso com arquivo e número de linha,
- **e** nenhum total muda por causa dessa linha,
- **e** as outras linhas do arquivo entram normalmente.

### AC3 — Vínculo (G1.2, U1)

- **Dado** os eventos de identidade de sessão do OpenRig,
- **quando** o operador pede o vínculo de `agentflow-dev`,
- **então** cada sessão ligada mostra rig, assento e a evidência do vínculo,
- **e** cada sessão sem evento de identidade aparece como "sem vínculo",
- **e** a taxa de vínculo aparece por harness,
- **e** nenhuma sessão recebe um vínculo sem evento de identidade.

### AC4 — Pergunta 1 (O2, G1.3)

- **Dado** `agentflow-dev` e o período,
- **quando** o operador pede atividade por assento,
- **então** cada um dos 6 assentos mostra ativo, ocioso com trabalho, ocioso sem trabalho, esperando humano e desconhecido,
- **e** a soma das cinco faixas é igual à duração observada do assento no período.

### AC5 — Pergunta 2 (O3, G1.4)

- **Dado** o mesmo rig e período,
- **quando** o operador pede handoffs,
- **então** a contagem por par de assentos é igual à contagem de handoffs na fonte,
- **e** cada retorno aparece com o caminho completo da cadeia,
- **e** o custo de entrada aparece por handoff com o rótulo "estimativa", ou "sem dados" quando o assento de destino não tem sessão ligada.

### AC6 — Pergunta 3 (O4, G1.5)

- **Dado** o mesmo rig e período,
- **quando** o operador pede tokens,
- **então** o total por sessão é igual à soma dos registros de uso dessa sessão na fonte,
- **e** os totais aparecem por missão e por assento, com "sem missão" para o trabalho sem tag,
- **e** uma sequência de teste com 3 falhas iguais seguidas em 10 minutos aparece como um loop,
- **e** uma sequência com 2 falhas iguais não aparece como loop.

### AC7 — Rastreabilidade (O5, G1.7)

- **Dado** qualquer número das AC4–AC6 e AC9,
- **quando** o operador pede a origem,
- **então** o Dendro mostra a definição da métrica e os registros de origem (arquivo e linha, ou tabela e chave),
- **e** recalcular o número a partir desses registros dá o mesmo valor.

### AC8 — Reexecução (O6, G1.6)

- **Dado** uma ingestão completa,
- **quando** o operador roda a ingestão de novo sem mudanças nas fontes,
- **então** nenhum total muda,
- **e quando** uma sessão ganha novas linhas e a ingestão roda de novo,
- **então** só as novas linhas entram, e os totais crescem exatamente pelo valor delas.

### AC9 — Métricas de decisão (O8, G1.8, G1.9)

- **Dado** `agentflow-dev` e o período,
- **quando** o operador pede as métricas de decisão,
- **então** M1, M2 e M3 aparecem com valores e origem, segundo as definições da §4,
- **e** M4 mostra a exposição a skills por assento,
- **e** o impacto de skills mostra "dados insuficientes" e o motivo.

### AC10 — Privacidade (O7)

- **Dado** um arquivo de sessão de teste com um texto marcador único no prompt e um token de API falso,
- **quando** a ingestão roda,
- **então** uma busca por esses dois textos no armazenamento analítico não encontra nada,
- **e** as fontes ficam inalteradas (mesmo hash antes e depois).

### AC11 — Somente leitura

- **Dado** as fontes da §3,
- **quando** a ingestão roda,
- **então** nenhum arquivo nem banco de origem muda (hash e data de modificação iguais),
- **e** nada sai da máquina pela rede.

### O8 — Sinal de valor (não é um teste automático)

O sponsor revisa M1–M4 de `agentflow-dev` e registra uma decisão real de topologia, handoff ou orçamento. O registro cita as métricas usadas. `dendro-dev` entra quando tiver trabalho real (F1).

---

## 6. Suposições (ASSUMPTIONS I'M MAKING)

| # | Suposição | Se estiver errada |
| --- | --- | --- |
| A1 | O Pi é obrigatório na R1, junto com OpenRig, Claude Code e Codex (F2). | Sem Pi, M1–M3 ficam sem custo para `orch.arch` e `rev.review`. O O8 fica fraco. **Aceito pelo sponsor.** |
| A2 | O rig de referência é `agentflow-dev`. `dendro-dev` não tem dados (F1). | Se o sponsor exigir `dendro-dev`, a aceitação espera ele trabalhar. |
| A3 | Os estados de atividade do OpenRig (`running`, `idle`, `needs_input`, `unknown`) são a fonte de verdade para ativo e ocioso. | Ocioso fica impreciso para runtimes sem hook. |
| A4 | O id de sessão do evento de identidade é o mesmo id usado pelo harness no nome do arquivo de sessão. | Medido: 85/87 batem. As 2 restantes ficam "sem vínculo". |
| A5 | O "retrabalho QA→Build" do sponsor significa "trabalho que volta ao build depois de QA ou revisão" (F3). | Se o sponsor quiser só a aresta direta, M3 mostra 0. |
| A6 | O custo em moeda aparece só onde a fonte registra ou o operador fornece preços. | Totais em moeda ficam incompletos para Claude Code e Codex. Tokens ficam completos. |
| A7 | Uma comparação válida de impacto de skill exige o mesmo papel, com e sem a skill, com pelo menos 5 itens fechados de cada lado. Os dados atuais não têm isso. | Se o sponsor aceitar uma comparação mais fraca, o resultado vira opinião com número. |
| A8 | Os limiares de loop (3 falhas iguais em 10 minutos; mais de 2 voltas no mesmo par) são o padrão. São configuráveis. | Falsos alarmes ou loops perdidos. |

---

## 7. Anti-goals e fora de escopo

### Anti-goals

1. Não é SaaS nem APM de nuvem. Nada sai da máquina.
2. Não controla, reinicia nem envia mensagens a agentes.
3. Não avalia a qualidade do código. M1 não é um julgamento de valor.
4. Não adivinha vínculos.
5. Não inventa preços nem impacto de skills.
6. Não guarda o conteúdo de prompts, respostas, saídas de ferramentas nem segredos.

### Fora de escopo da R1

- Linguagem natural, painéis Nao e alertas (G2).
- Modo contínuo (G2.4).
- Guia de conectores e extensibilidade (G3).
- Site, marca, domínio e as três documentações (Antigravity).
- AGY e Grok como bloqueadores. Eles entram na mesma release depois dos obrigatórios. Hoje nenhum nó OpenRig usa esses harnesses, então eles não alteram o O8.
- Sessões fora de rigs OpenRig. Elas aparecem nos totais de fonte como "sem vínculo".

---

## 8. Desconhecidos restantes (para o `orch.arch`)

| # | Desconhecido | Estado |
| --- | --- | --- |
| U1 | Vínculo sessão → assento | Resolvido: evento de identidade do OpenRig (§3). |
| U2 | Ocioso | Resolvido (§4). Sponsor confirma. |
| U3 | Custo de contexto do handoff | Resolvido como estimativa (§4). Sponsor confirma. |
| U4 | Loop | Resolvido (§4, A8). |
| U5 | Estabilidade dos formatos Codex, AGY, Pi e Grok | `orch.arch`. A AC2 limita o dano. |
| U6 | Nao sobre DuckDB local | Fora da R1 (G2). `orch.arch`. |
| U7 | Impacto de skills | Não mensurável na R1. Só exposição (M4, A7). |
| U8 | ROI do assento | Redefinido como rendimento (M1). Sponsor confirma. |
| U9 | Formato de token e custo de AGY e Grok | `orch.arch`, antes de construir esses conectores. |
