# Dendro: Storytelling & Product Manifesto

> *"Just as dendrochronology reveals history through tree rings, Dendro reads the session rings of your AI agents to uncover performance, handovers, and costs."*

---

## 1. O Manifesto: Da "Maçaroca" à Inteligência

### O Novo Mundo dos Agentes
Nos últimos meses, a forma de programar mudou radicalmente. Deixamos para trás os chats simples de IA e entramos na era das **equipes de agentes autônomos**.
Hoje, em uma única máquina de desenvolvedor convivem múltiplos agentes e ferramentas:
* **Claude Code** refatorando arquivos em loop.
* **Codex** executando tarefas em segundo plano.
* **AGY** gerindo planos e passos complexos.
* **Grok** e **Pi** atuando como especialistas pontuais.
* E camadas de runtime como o **OpenRig**, orquestrando assentos (*seats*), missões e transições de fila.

### O Paradoxo da Visibilidade (A "Maçaroca de Dados")
Quanto mais agentes você tem trabalhando juntos, mais cego você fica.
* O terminal vira uma enxurrada ininterrupta de chamadas `bash`, `edit_file` e contagens de tokens soltas.
* Quando uma missão demora ou custa caro, você não sabe se o problema foi um prompt ruim, um loop infinito de compilação ou um handover desastroso entre agentes.
* As ferramentas existentes de observabilidade (APMs de nuvem) tratam agentes como se fossem APIs web normais — despejando métricas estéreis de latência em dashboards que ninguém olha.

### A Inspiração: Dendrocronologia
Na natureza, uma árvore não descarta o seu passado. Cada ano de seca, cada estação chuvosa, cada queimada e cada período de crescimento acelerado fica gravado com precisão cirúrgica em seus **anéis concêntricos** (*tree rings*). Cientistas usam a **dendrocronologia** para olhar para o tronco e reconstruir séculos de história climática.

**No ecossistema de software, as sessões dos seus agentes são esses anéis.**
Cada tool call, cada transição de assento, cada watchdog disparado é uma camada de crescimento do seu código. 

O **Dendro** é a broca que extrai e lê esses anéis — transformando logs fragmentados na crônica viva da sua equipe autônoma.
