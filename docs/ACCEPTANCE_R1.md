# Dendro R1 — Aceitação (Fase 1)

| Campo | Valor |
| --- | --- |
| Dono | `pm.analyst` (`pm-analyst@dendro-pm`) |
| Entrada | Fase 0 `dendro-r1-p0`, `docs/PRODUCT_SPECIFICATION.md` |
| Data | 2026-10-10 |
| Estado | **Congelado** em 2026-10-10. Sponsor aprovou às 17:01Z. Decisão PM: prosseguir (`docs/PRODUCT_SPECIFICATION.md` §12). A1, A2, A5, M1 e M4 aceitos. |
| Emenda 1 | 2026-10-10. Retorno do `orch.arch` (`transitions/0003.json`), ciclo 1 de 2. Atribuição de missão com sobreposição (§4, AC6). Somente leitura em fontes vivas (AC8, AC10, AC11, AC12). Aprovada pelo `pm.manager` às 17:04Z. Inclui as lacunas semânticas do conselho (QA `qitem-20261010170440-5d776632`, revisão `qitem-20261010170435-20c8a356`): §4, §4.1, AC1, AC4–AC7, AC10, AC11. O job não muda. Ver §9. |
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

Regras de intervalo:

1. **Janela de observação do assento** = período `[de, até)` em UTC, recortado pela criação do nó e pela parada do rig ou do nó.
2. Um estado vale do seu evento até o próximo evento do mesmo assento, a parada do rig ou do nó, ou o fim da janela.
3. **Estado no início da janela** = último estado antes de `de`. Sem estado anterior: "desconhecido" até o primeiro evento.
4. Um assento sem nenhum evento na janela fica 100% "desconhecido".
5. Um intervalo `idle` se divide sempre que a demanda de fila muda (item chega, fecha ou muda de destino). A demanda vem do histórico de estados da fila no instante, não do estado atual.
6. As cinco faixas são disjuntas e somam exatamente a janela de observação.

### Pergunta 2 — Handoffs (U3)

| Métrica | Definição |
| --- | --- |
| Handoff | Um item de fila fechado com `handed_off_to`, de um assento para outro. |
| Retorno | Um handoff que leva o trabalho de volta a um assento que já tocou o mesmo item ou a mesma cadeia de registro. |
| Custo de entrada | Entrada sem cache + leitura de cache + escrita de cache (§4.1) que o assento de destino consome na janela de entrada. Rótulo obrigatório: "estimativa". |
| Janela de entrada | Começa no instante da reivindicação, inclusivo. A reivindicação não é a ação final. Termina na primeira ação de escrita seguinte, exclusivo. Ação de escrita = ferramenta estruturada de edição ou escrita de arquivo, ou transição de fila do mesmo item (atualização, bloqueio, handoff, fechamento). O Dendro não classifica texto livre de comando (ex.: `bash`) como escrita. |
| Entrada sem dados | Sem fim classificável, sem sessão ligada ou sem marca de tempo: "sem dados" e o motivo. Nunca zero. |
| Entrada ambígua | Se o assento tem outro item `in-progress` durante a janela, o custo de entrada fica "ambígua". Um registro de uso nunca conta em dois handoffs. |
| Tempo de espera | Do handoff até a reivindicação pelo destino. |

### Pergunta 3 — Tokens e loops (U4)

| Métrica | Definição |
| --- | --- |
| Tokens | As quatro categorias da §4.1. Somadas por sessão, assento, missão e período. |
| Missão | Valor da tag `mission:<id>` do item de fila. |
| Atribuição de missão | Cada registro de uso recebe a missão do item que o assento tem `in-progress` no instante do registro. Sem item `in-progress`, ou item sem tag: "sem missão". Dois ou mais itens `in-progress` com missões diferentes, ou um item com duas tags `mission:` diferentes: "ambígua". Vários itens da mesma missão: essa missão. O Dendro nunca divide nem estima um registro ambíguo. Cada registro conta em exatamente uma faixa. |
| Loop de ferramenta | O mesmo assento executa a mesma ferramenta com a mesma entrada normalizada, com falha, 3 ou mais vezes seguidas, em até 10 minutos. |
| Loop de handoff | O mesmo item ou cadeia passa entre o mesmo par de assentos mais de 2 vezes. |
| Custo em moeda | Exibido só quando a fonte registra o custo (Pi) ou quando o operador fornece uma tabela de preços. O Dendro nunca inventa um preço. |

Regras de loop de ferramenta:

1. Escopo: uma sessão. Sessões diferentes do mesmo assento não se juntam.
2. Entrada igual = mesmo nome de ferramenta e mesma entrada depois de remover espaços nas pontas. Nenhuma outra normalização.
3. Falha = a fonte marca erro (erro da ferramenta ou código de saída diferente de zero). Resultado sem marca = desconhecido. Desconhecido quebra a sequência.
4. Qualquer outra chamada de ferramenta na mesma sessão quebra a sequência.
5. Janela: da 1ª à 3ª falha em até 10 minutos, inclusivo (exatamente 10:00 conta).
6. 4 ou mais falhas seguidas formam **um** episódio, com a contagem total. Não formam vários loops sobrepostos.
7. Os limiares (3 falhas, 10 minutos, mais de 2 voltas) são configuráveis pelo operador.

### 4.1 Semântica comum

| Regra | Definição |
| --- | --- |
| Período | `[de, até)`, UTC, meio aberto. |
| Ordem e empate | Marca de tempo da fonte, depois a ordem na fonte (linha ou chave). Um evento exatamente em `de` entra. Um evento exatamente em `até` não entra. |
| Categorias de token | Quatro categorias disjuntas: entrada sem cache, saída, leitura de cache, escrita de cache. Se a fonte inclui o cache na entrada, o cache conta só na categoria de cache. Tokens de raciocínio contam na saída e aparecem também em separado quando a fonte registra. Nenhum token conta duas vezes. |
| Contadores acumulados | Uma fonte com contador acumulado (Codex) contribui com a diferença entre leituras. Um contador que volta para baixo inicia uma nova geração. A diferença dessa leitura é o próprio valor. |
| Duplicatas | Um registro com o mesmo id nativo de mensagem conta uma vez (Claude Code). |
| Uso fora da janela | Um registro de uso conta no instante da sua marca de tempo. Registro sem marca de tempo: fica fora de qualquer janela e entra só no total da sessão, com contagem visível. |
| Mediana | Com contagem par, é a média dos dois valores centrais. |
| Divisão por zero | "sem dados", nunca zero nem infinito. |

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

- **Dado** um binário já compilado, um banco analítico novo e as fontes desta máquina,
- **quando** o operador roda a ingestão e pede o primeiro relatório,
- **então** o relatório fica pronto em até 5 minutos de relógio, contados do início da ingestão (o build não conta),
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
- **e** a soma das cinco faixas é igual à janela de observação do assento (§4),
- **e** casos de teste cobrem: item que chega e fecha durante um `idle`; estado anterior a `de`; assento sem eventos; assento criado no meio da janela; item que muda de destino.

### AC5 — Pergunta 2 (O3, G1.4)

- **Dado** o mesmo rig e período,
- **quando** o operador pede handoffs,
- **então** a contagem por par de assentos é igual à contagem de handoffs na fonte,
- **e** cada retorno aparece com o caminho completo da cadeia,
- **e** o custo de entrada aparece por handoff com o rótulo "estimativa", ou como "sem dados" ou "ambígua" com o motivo (§4),
- **e** casos de teste cobrem: empate de marca de tempo na reivindicação e no fim, marca ausente, duas reivindicações simultâneas, e uma sessão só com `bash` (resultado: "sem dados").

### AC6 — Pergunta 3 (O4, G1.5)

- **Dado** o mesmo rig e período,
- **quando** o operador pede tokens,
- **então** o total por sessão é igual a um agregado independente da fonte, calculado sem o código de ingestão do Dendro, com as regras da §4.1,
- **e** casos de teste cobrem: reinício de contador acumulado, mensagem duplicada e cache incluído na entrada,
- **e** os totais aparecem por missão e por assento, com as faixas "sem missão" e "ambígua",
- **e**, para cada assento, a soma das missões mais "sem missão" mais "ambígua" é igual ao total do assento,
- **e** um caso de teste com dois itens `in-progress` de missões diferentes no mesmo instante conta o uso desse instante como "ambígua", e não em nenhuma das duas missões,
- **e** uma sequência de teste com 3 falhas iguais seguidas em 10 minutos aparece como um loop,
- **e** uma sequência com 2 falhas iguais não aparece como loop,
- **e** casos de teste cobrem: exatamente 10 minutos, sucesso no meio, ferramenta ou entrada diferente no meio, mesma entrada em outra sessão do mesmo assento, 4 falhas (um episódio), e limiar alterado pelo operador.

### AC7 — Rastreabilidade (O5, G1.7)

- **Dado** qualquer número das AC4–AC6 e AC9,
- **quando** o operador pede a origem,
- **então** o Dendro mostra a definição e a versão da métrica, os limites aplicados, os filtros e os valores de entrada (números e identificadores, com a chave do registro de origem e a geração da fonte),
- **e** para razões mostra numerador e denominador; para medianas, as amostras; para durações, o estado na borda e as mudanças de demanda,
- **e** recalcular o número só a partir da saída do rastreio dá o mesmo valor,
- **e** o rastreio descreve a ingestão que produziu o número. Se a fonte mudou desde então, o rastreio diz isso, em vez de apontar para bytes que não existem mais,
- **e** a recomputação funciona depois de um acréscimo, de um reinício de fonte e de uma linha alterada no SQLite.

### AC8 — Reexecução (O6, G1.6)

- **Dado** uma ingestão completa sobre uma cópia congelada das fontes (sem outro processo escrevendo),
- **quando** o operador roda a ingestão de novo sobre a mesma cópia,
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

Dois tipos de campo:

- **Conteúdo:** prompt, resposta, argumentos de ferramenta, resultado de ferramenta e mensagem de erro. Nunca aparece.
- **Identificador:** tag de missão, nome de ferramenta e caminho de arquivo. É saída do produto. O operador precisa dele nos totais, nos loops e no rastreio. Aparece, salvo quando tem forma de segredo.

Forma de segredo = um valor que segue um padrão conhecido de credencial (ex.: chaves de API de provedores, tokens do GitHub, chaves da AWS). O teste usa um token falso com um desses padrões.

- **Dado** fontes de teste com um texto marcador único em cada campo de conteúdo e em cada campo de identificador, e um token de API falso em cada campo de conteúdo e de identificador,
- **quando** a ingestão, os relatórios, o rastreio e as exportações rodam,
- **então** o marcador dos campos de conteúdo não aparece no armazenamento analítico, nas saídas, nas exportações nem nos diagnósticos,
- **e** o token falso não aparece em nenhum lugar, em nenhum campo. Num identificador, ele é substituído por uma marca de redação visível,
- **e** o marcador comum de um identificador pode aparecer, porque é um identificador que o operador precisa ver,
- **e** a impressão usada para comparar entradas de ferramenta (loops) não permite recuperar a entrada: calcular a impressão de uma entrada conhecida, sem a chave local da máquina, não reproduz o valor armazenado,
- **e** o arquivo de teste fica inalterado (mesmo hash antes e depois).

### AC11 — Somente leitura: cópia congelada (determinismo)

- **Dado** uma cópia congelada das fontes da §3, sem outro processo escrevendo,
- **quando** a ingestão roda,
- **então** nenhum arquivo nem banco da cópia muda (hash e data de modificação iguais), incluindo o banco SQLite, o WAL e o SHM,
- **e** nada sai da máquina pela rede,
- **e** apontar o banco de saída para uma fonte (caminho direto, link simbólico ou hard link) é recusado antes de abrir, e a fonte fica inalterada,
- **e** a ingestão não lê os próprios arquivos de saída como fonte.

### AC12 — Somente leitura: fontes vivas (segurança)

- **Dado** as fontes vivas da §3, com agentes escrevendo nelas,
- **quando** a ingestão roda,
- **então** o Dendro abre cada arquivo e banco de origem só para leitura,
- **e** o teste falha se o Dendro abrir qualquer fonte para escrita, criar ou apagar um arquivo nas pastas de origem.
- **Exceção única:** o índice de memória compartilhada SQLite (`-shm`) de um banco vivo. Leitores SQLite atualizam esse índice por projeto. O banco principal e o WAL nunca abrem para escrita.
- Uma mudança de hash numa fonte viva não prova uma escrita do Dendro. Só a instrumentação de abertura prova.

A AC11 prova determinismo. A AC12 prova que as fontes vivas ficam seguras. Uma não substitui a outra.

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

---

## 9. Emenda 1 — divisão de responsabilidades

O analista decide o que a métrica significa e o que o teste observa. O `orch.arch` decide como.

| Item do conselho | Dono | Disposição |
| --- | --- | --- |
| Missão com sobreposição | Analista | §4 Atribuição de missão; AC6. |
| Janela de observação do assento | Analista | §4 Regras de intervalo; AC4. |
| Janela de entrada do handoff | Analista | §4 Janela de entrada; AC5. |
| Categorias de token, contadores, duplicatas | Analista | §4.1; AC6. O mapeamento por harness é do `orch.arch`. |
| Regras de loop e limiar configurável | Analista | §4 Regras de loop; AC6. O mecanismo de configuração é do `orch.arch`. |
| Conteúdo do rastreio | Analista | AC7. O formato e o armazenamento são do `orch.arch`. |
| Lugares dos marcadores de privacidade | Analista | AC10: conteúdo nunca aparece; identificador aparece, salvo forma de segredo (redigido). Refinado pelo `pm.manager` às 17:06Z. |
| Impressão de entrada sem recuperação | Analista (o quê), `orch.arch` (como) | AC10 exige que a impressão não seja reproduzível sem a chave local. A escolha do algoritmo é do `orch.arch`. |
| Saída que aponta para uma fonte | Analista | AC11. |
| WAL/SHM em fonte viva | Analista | AC12, exceção única do `-shm`. |
| Bancos antigos com `command_preview` ou `error_message` | `orch.arch` | Não há instalação anterior da R1. Não há migração. O caminho de persistência sai. |
| Substituições no scaffold (custo não nulo, resolver por substring, `OPENRIG_HOME`, SQLite gravável) | `orch.arch` | Obrigação de arquitetura, não muda o escopo. |
| Matriz de oráculos AC1–AC12 e base do candidato | `orch.arch` + `dev.qa` | Plano da Fase 3. |

