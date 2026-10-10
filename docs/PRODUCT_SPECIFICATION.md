# Dendro — Especificação de Produto (Fase 0)

| Campo | Valor |
| --- | --- |
| Dono | `pm.manager` (`pm-manager@dendro-pm`) |
| Sponsor | Sam |
| Data | 2026-10-10 |
| Estado | Go-Ahead do sponsor em 2026-10-10 16:56Z. Fase 0 registrada (`dendro-r1-p0`). |
| Fonte | `docs/INTAKE_BRIEFING.md`, `docs/research/openrig-discussion-1099.md`, dados locais |
| Próximo dono | `pm.analyst` (Fase 1) |

---

## 1. Conclusão

**Recomendação: prosseguir com o motor local (Goal 1). Pausar site, domínio e marca até o Goal 1 provar valor.**

Motivos:

1. **Um job real e urgente existe.** O operador de um rig multi-agente não consegue dizer se a topologia valeu o custo. Os dados existem. A resposta não existe.
2. **O briefing junta três jobs diferentes.** Motor (operar), documentação (adotar/estender) e site (convencer um comprador). Só o primeiro tem usuário presente e dor medida hoje.
3. **"MVP para amanhã" com cinco frentes não cabe.** Um escopo honesto para amanhã é: o operador responde às três perguntas para um rig real, a partir dos dados desta máquina.
4. **O site depende do motor.** Sem números reais, a landing page só promete. Com o Goal 1 pronto, o site mostra prova.

### Decisão do sponsor (2026-10-10 16:56Z)

O sponsor concedeu o Go-Ahead para a Release 1 (`v0.1.0`) e ajustou a recomendação:

1. **Motor:** OpenRig, Claude Code e Codex obrigatórios. AGY, Pi e Grok em seguida, na mesma release.
2. **Site, marca e documentação:** aprovados em paralelo. O Antigravity é o dono, fora deste squad. O squad de produto não define esse escopo.
3. **Teste de valor (O8):** métricas de decisão executiva nos rigs `dendro-dev` e `agentflow-dev`: ROI do assento, eficiência de handoff, retrabalho QA→Build e impacto de skills.

Decisão PM registrada: **prosseguir**.

---

## 2. Job Statement

> **Quando** eu opero uma equipe de agentes (rig) que já trabalhou por dias,
> **quero** ver onde o tempo, os tokens e o retrabalho realmente foram, por assento, handoff e missão,
> **para que** eu possa mudar a topologia, os handoffs ou o orçamento com evidência, e não com intuição.

### Jobs relacionados (fora do Goal 1)

| Job | Usuário | Estado |
| --- | --- | --- |
| Quando eu adoto o Dendro, quero respostas em 5 minutos, para que eu decida se ele fica. | Novo operador | Parte do Goal 1 (primeira execução) |
| Quando eu uso um harness não suportado, quero plugar um conector, para que meus dados entrem. | Contribuidor | Goal 3 |
| Quando eu avalio ferramentas para minha equipe de IA, quero ver prova do valor, para que eu justifique a adoção. | CTO / Head de IA | Goal 4. Não validado. |

---

## 3. Momento de luta

O operador termina uma missão. A missão demorou ou custou caro. Ele não sabe por quê.

Evidência nesta máquina (2026-10-10):

| Fonte | Volume | O que falta |
| --- | --- | --- |
| `~/.openrig/openrig.sqlite` | 6 rigs, 28 nós, 208 itens de fila, 702 transições, 4.169 amostras de uso, 133 eventos de watchdog | Nenhuma visão junta transições com custo |
| `~/.claude/projects` | 44 MB | Tokens por sessão, sem assento nem missão |
| `~/.codex/sessions` | 29 MB | Outro formato, sem vínculo com o rig |
| `~/.gemini/antigravity-cli/brain` | 105 MB | Outro formato, sem vínculo com o rig |
| `~/.pi`, `~/.grok` | < 1 MB | Outro formato |

```
 missão lenta/cara
        │
        ▼
 "Foi o prompt? Um loop? Um handoff ruim? Um assento inútil?"
        │
        ├── rig capture / transcript  → um assento por vez, sem custo
        ├── logs do harness           → custo sem assento nem missão
        ├── openrig.sqlite            → filas sem custo, SQL manual
        └── APM de nuvem              → latência de API, sem conceito de assento
        │
        ▼
 decisão por intuição  (ou nenhuma decisão)
```

### Falhas das alternativas atuais

| Alternativa | Onde falha |
| --- | --- |
| Ler transcripts e `rig capture` | Um assento por vez. Sem total. Sem custo. Não escala além de 2–3 assentos. |
| Painel de uso do provedor | Total por conta. Não separa rig, assento ou missão. |
| SQL manual no `openrig.sqlite` | Sem dados de tokens dos harnesses. Exige conhecer o schema. Ninguém repete. |
| APM de nuvem (Langfuse, Datadog etc.) | Exige instrumentar e enviar dados para fora. Modela chamadas de API, não assentos e handoffs. |
| Não fazer nada | O operador mantém assentos que só repassam texto e paga por loops que ninguém vê. |

---

## 4. Forças do progresso

```
        A FAVOR DA MUDANÇA                     CONTRA A MUDANÇA
 ┌──────────────────────────────┐     ┌──────────────────────────────┐
 │ PUSH                         │     │ ANSIEDADE                    │
 │ • custo cresce sem explicação│     │ • números errados levam a    │
 │ • rigs com 6+ assentos       │     │   decisões erradas           │
 │ • retrabalho QA→Build oculto │     │ • ler transcripts expõe      │
 │                              │     │   segredos                   │
 │ PULL                         │     │ • mais uma ferramenta a      │
 │ • local, sem enviar dados    │     │   manter                     │
 │ • respostas em linguagem     │     │                              │
 │   natural                    │     │ INÉRCIA                      │
 │ • decidir a topologia com    │     │ • "eu leio os logs quando    │
 │   evidência                  │     │   preciso"                   │
 └──────────────────────────────┘     └──────────────────────────────┘
```

Implicações para o produto:

1. **Confiança antes de amplitude.** Cada número deve levar ao registro de origem. Um número errado custa mais do que um número ausente.
2. **Local e somente leitura.** O Dendro nunca escreve nas fontes. Nenhum dado sai da máquina.
3. **Segredos ficam fora.** O conteúdo dos prompts não vai para o banco por padrão. Isso preserva `no-raw-secrets-or-transcripts`.
4. **Zero manutenção.** Um comando gera as respostas. Rodar de novo só lê o que mudou.

### Dimensões

| Dimensão | Progresso desejado |
| --- | --- |
| Funcional | Responder às três perguntas para um rig e um período. |
| Emocional | Sair do "acho que o rig está caro" para o "sei onde está o custo". |
| Social | Mostrar ao time ou ao comprador por que a topologia tem essa forma. |

---

## 5. Resultado observável

O Goal 1 está feito quando isto é verdade para o operador, nesta máquina, com dados reais:

| # | Mudança observável |
| --- | --- |
| O1 | Partindo dos logs existentes, o operador obtém a primeira resposta em até 5 minutos após a instalação. |
| O2 | Para um rig nomeado e um período, o operador vê o trabalho ativo e o tempo ocioso de cada assento. (Pergunta 1) |
| O3 | Para o mesmo rig, o operador vê cada handoff, quantas vezes o trabalho voltou e o custo de contexto da transferência. (Pergunta 2) |
| O4 | Para o mesmo rig, o operador vê o consumo de tokens por missão e assento, e os loops de retry detectados. (Pergunta 3) |
| O5 | O operador consegue rastrear qualquer número até o registro de origem (arquivo e linha, ou linha da tabela). |
| O6 | Ao rodar de novo, só os dados novos entram, e os totais não duplicam. |
| O7 | Nenhum conteúdo de prompt e nenhum segredo fica no armazenamento analítico. |
| O8 | Sinal de valor: com base no Dendro, o sponsor toma pelo menos uma decisão real de topologia, handoff ou orçamento em `dendro-dev` ou `agentflow-dev`, usando quatro métricas de decisão: ROI do assento, eficiência de handoff, retrabalho QA→Build e impacto de skills. |

O O8 é o teste do produto. O1 a O7 são condições para o O8.

---

## 6. Goals e histórias (rascunho para o analista)

Os critérios de aceitação são do `pm.analyst`. Abaixo estão sinais de aceitação no nível do resultado. O analista os torna testáveis e congela.
A arquitetura é do `orch.arch`. Esta seção não escolhe arquivos, crates nem schemas.

```
 G1 Motor local (AGORA) ──► G2 Respostas guiadas ──► G3 Extensibilidade
        │                                              │
        └──────────── prova real ─────────────► G4 Presença pública
```

### G1 — O operador responde às três perguntas para um rig real (AGORA)

| História | Sinal de aceitação |
| --- | --- |
| G1.1 Como operador, eu rodo um comando e o Dendro lê as fontes locais encontradas. | Informa quais fontes encontrou, quais faltam e quantos registros leu de cada. |
| G1.2 Como operador, vejo as sessões dos harnesses ligadas ao rig, assento e missão certos. | Uma sessão sem vínculo aparece como "sem vínculo", sem palpite. A taxa de vínculo aparece. |
| G1.3 Como operador, vejo o ativo e o ocioso por assento. | Responde à Pergunta 1 para `dendro-dev` nos últimos 7 dias. |
| G1.4 Como operador, vejo handoffs e retrabalho. | Responde à Pergunta 2. Uma volta QA→Build conta como retrabalho. |
| G1.5 Como operador, vejo a queima de tokens e os loops. | Responde à Pergunta 3. Um comando repetido com falha aparece como loop. |
| G1.6 Como operador, rodo de novo e só os dados novos entram. | A segunda execução sem mudanças não altera nenhum total. |
| G1.7 Como operador, confio nos números. | Cada métrica tem uma definição escrita e um caminho até a origem. |
| G1.8 Como operador, vejo o ROI de cada assento. | Para `dendro-dev`, mostra o que cada assento custou e o que entregou adiante. |
| G1.9 Como operador, vejo o impacto de uma skill. | Compara o trabalho com e sem a skill carregada, ou mostra "dados insuficientes". |

Prioridade dos harnesses no G1 (decisão do sponsor): **OpenRig, Claude Code e Codex obrigatórios.** AGY, Pi e Grok entram na mesma release, depois dos três primeiros.

### G2 — O operador pergunta em linguagem natural (PRÓXIMO)

| História | Sinal de aceitação |
| --- | --- |
| G2.1 Como operador, faço as três perguntas em linguagem natural (Nao). | As respostas batem com as do G1 para o mesmo período. |
| G2.2 Como operador, abro um painel inicial por pergunta. | Três painéis, um por pergunta, com dados reais. |
| G2.3 Como operador, recebo um alerta de queima ou loop anômalo. | O alerta aparece antes que o operador perceba sozinho. O limiar é configurável. |
| G2.4 Como operador, mantenho as respostas atualizadas sem rodar comandos. | O modo contínuo reflete novos dados em minutos. |

### G3 — Um contribuidor pluga um novo harness (DEPOIS)

| História | Sinal de aceitação |
| --- | --- |
| G3.1 Como contribuidor, adiciono um conector seguindo um guia. | Um conector novo funciona sem mudar o núcleo. |
| G3.2 Como mantenedor, entendo a arquitetura e as convenções. | Um novo builder entrega a primeira mudança usando só a documentação. |
| G3.3 Como novo operador, sigo o quickstart. | Um operador sem ajuda chega ao O1 em até 5 minutos. |

### G4 — Um comprador entende o valor (paralelo, dono: Antigravity)

O sponsor aprovou o G4 e a documentação do G3 em paralelo, com o Antigravity como dono. As histórias abaixo ficam como referência. Elas não entram na Fase 1 deste squad.

| História | Sinal de aceitação |
| --- | --- |
| G4.1 Como CTO, entendo em 30 segundos o que o Dendro responde. | Validar com 3–5 pessoas do público-alvo antes de construir. |
| G4.2 Como visitante, vejo a metáfora dos anéis com dados reais. | Usa números reais do G1, não dados inventados. |
| G4.3 Domínio, logotipo e identidade visual. | Decisão do sponsor. Ver §9. |

### Matriz de prioridade

| | Valor alto | Valor incerto |
| --- | --- | --- |
| **Viável agora** | G1 (OpenRig + Claude Code), G1.7 | G2.1 Nao |
| **Depende do G1** | G2.2–G2.4, G3.3 | G4 site, marca, domínio |

---

## 7. Restrições e anti-goals

### Restrições

1. Local-first. Nenhum dado sai da máquina.
2. Somente leitura sobre as fontes (`~/.openrig`, `~/.claude`, `~/.codex`, `~/.gemini`, `~/.pi`, `~/.grok`).
3. Sem conteúdo bruto de prompts nem segredos no armazenamento.
4. Stack já decidida pelo sponsor: Rust, DuckDB, Nao. A decisão é do sponsor. Não reabrir.
5. As fontes mudam de formato. Um formato desconhecido gera aviso, não falha nem dado errado.

### Anti-goals

1. Não é um APM de nuvem nem um SaaS.
2. Não controla nem reinicia agentes. Observa. Não age.
3. Não avalia a qualidade do código produzido pelos agentes.
4. O site não mostra números inventados como se fossem reais.
5. Não adivinha vínculos. "Sem vínculo" é melhor que um vínculo errado.

---

## 8. Intenção de release

| Campo | Valor |
| --- | --- |
| Release | `v0.1.0` — prévia local do G1 |
| Impacto | `minor` (primeiro incremento funcional) |
| Público | Sponsor. Rigs de teste: `dendro-dev`, `agentflow-dev`. |
| Canal | Build local a partir do repositório. Sem publicação, sem crates.io, sem site. |
| Critério de saída | O1–O7 verificados com dados reais. O8 observado pelo sponsor. |
| Paralelo | Site, marca e as três documentações: Antigravity. A compra de domínio exige aprovação explícita do sponsor. |
| Depois | `v0.2` = G2. |

---

## 9. Decisões do sponsor (resolvidas em 2026-10-10 16:56Z; ver §1)

| # | Pergunta | Recomendação do PM |
| --- | --- | --- |
| D1 | Aceita reduzir "amanhã" ao G1 (OpenRig + Claude Code)? | Sim. Cinco frentes em um dia produzem cinco frentes rasas. |
| D2 | Pausa G4 (site, domínio, marca) até o O8? | Sim. Se quiser reservar um domínio agora, é uma ação externa e paga. Exige sua aprovação explícita. Nenhum agente compra domínio. |
| D3 | O público inicial é só você, ou há outros operadores para validar? | Nomear 1–2 operadores externos aumenta a confiança no O1 e no G3.3. |
| D4 | Qual decisão real você quer tomar com o Dendro primeiro? | Ela vira o teste do O8. Exemplo: "o assento `dev-build-jr` vale a pena?" |

---

## 10. Desenho da solução: limites de propriedade

O briefing pede também o desenho da solução: schemas, fluxos, painéis, arquitetura do site. Esse desenho não é da Fase 0.

| Artefato | Dono | Fase |
| --- | --- | --- |
| Job, resultado, release | `pm.manager` | 0 (este documento) |
| Critérios de aceitação testáveis, escopo, suposições | `pm.analyst` | 1 |
| Arquitetura, schemas analíticos, fluxos, plano | `orch.arch` (`dendro-dev`) | 2+ |
| Painéis Nao, site | Delivery, após o G1 | G2 / G4 |

O rascunho existente em `docs/architecture.md` é referência para o `orch.arch`. Não é requisito.

```
 Sam ──Go-Ahead──► pm.manager ──Fase 0──► pm.analyst ──Fase 1──► orch-arch@dendro-dev ──► build/qa/review
                       ▲                      │                         │
                       └──── retorno ─────────┘◄──────── retorno ───────┘
```

---

## 11. Desconhecidos

| # | Desconhecido | Por que importa | Quem resolve |
| --- | --- | --- | --- |
| U1 | Como ligar uma sessão do harness ao assento com confiança (bindings, tmux, pid, cwd)? | Sem vínculo, as perguntas 1–3 não fecham. | `orch.arch`, com o analista |
| U2 | O que conta como "ocioso" (sem turno, ou à espera de fila)? | Muda a resposta à Pergunta 1. | `pm.analyst` define, sponsor confirma |
| U3 | Como medir o "custo de contexto" de um handoff? | Muda a resposta à Pergunta 2. | `pm.analyst` |
| U4 | O que é um "loop" (N repetições? com falha? em quanto tempo)? | Evita falsos alarmes. | `pm.analyst` |
| U5 | Os formatos de Codex, AGY, Pi e Grok são estáveis? | Risco de manutenção dos conectores. | `orch.arch` |
| U6 | Como o Nao consulta o DuckDB local nesta versão? | Viabilidade do G2. | `orch.arch` |
| U7 | Como medir o "impacto de skills" (as fontes registram a skill carregada? Há base de comparação)? | Métrica de decisão nova do sponsor. Pode não ter dados. | `pm.analyst` + `orch.arch` |
| U8 | Como definir o "ROI do assento" sem medir a qualidade do código (anti-goal 3)? | Evita uma métrica que promete mais do que mede. | `pm.analyst`, sponsor confirma |

---

## 12. Decisão PM antes do congelamento (2026-10-10 17:05Z)

**Decisão: prosseguir.** O sponsor aprovou `docs/ACCEPTANCE_R1.md` às 17:01Z. O job não muda. Os ajustes abaixo refinam o escopo da R1 e não exigem retorno à Fase 0.

| Ponto da aceitação | Decisão | Motivo |
| --- | --- | --- |
| A1 Pi obrigatório | Aceito | Em `agentflow-dev`, `orch.arch` e `rev.review` rodam em Pi. Em `dendro-dev`, todos os builders rodam em Pi. Sem Pi, o O8 fica sem custo nos assentos centrais. O Pi já estava na R1. A mudança é só de ordem. |
| A2 Rig de referência `agentflow-dev` | Aceito | `dendro-dev` tem 0 itens de fila (verificado). O O8 em `dendro-dev` espera trabalho real. |
| A5 Retrabalho pela cadeia | Aceito | Há 0 handoffs diretos QA→Build. A intenção do sponsor é "trabalho que volta ao build". |
| M1 "rendimento" no lugar de "ROI" | Aceito | O Dendro não mede valor. O nome não deve prometer o que a métrica não mede. |
| M4 só exposição a skills | Aceito | Não há base de comparação. Mostrar "dados insuficientes" é melhor que mostrar opinião com número. |

### Emenda 1 da aceitação (2026-10-10 17:08Z)

**Decisão: refinar um ponto, depois prosseguir.** Retorno da delivery, ciclo 1 de 2. O job não muda.

- Tudo da Emenda 1 está aceito, inclusive a exceção do `-shm` no SQLite.
- AC10: as tags de missão, os nomes de ferramenta e os caminhos de origem são saída do produto. Eles podem aparecer. Um texto livre (prompt, resposta, argumentos, resultados, erros) nunca aparece. Um valor com formato de segredo nunca aparece em nenhum campo e é mascarado.
