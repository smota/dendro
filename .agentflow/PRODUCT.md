# Dendro — Registro de Produto

Fonte completa: `docs/PRODUCT_SPECIFICATION.md`. Dono: `pm.manager`. Estado: Go-Ahead do sponsor em 2026-10-10 16:56Z. Fase 0 registrada (`dendro-r1-p0`).

## Job Statement

Quando eu opero uma equipe de agentes (rig) que já trabalhou por dias,
quero ver onde o tempo, os tokens e o retrabalho realmente foram, por assento, handoff e missão,
para que eu possa mudar a topologia, os handoffs ou o orçamento com evidência, e não com intuição.

## Usuário

Primário: operador de um rig OpenRig multi-agente (sponsor). Secundário: contribuidor (G3). Comprador (G4): não validado.

## Resultado observável (G1)

- O1: primeira resposta em até 5 minutos, partindo dos logs locais existentes.
- O2–O4: responde às perguntas de assentos, handoffs e custos para um rig e um período nomeados.
- O5: cada número leva ao registro de origem.
- O6: rodar de novo não duplica os totais.
- O7: nenhum prompt bruto nem segredo armazenado.
- O8: o sponsor toma uma decisão real em `dendro-dev` ou `agentflow-dev` usando ROI do assento, eficiência de handoff, retrabalho QA→Build e impacto de skills.

## Escopo

| Goal | Estado |
| --- | --- |
| G1 Motor local (OpenRig, Claude Code, Codex; depois AGY, Pi, Grok) | Prosseguir |
| G2 Linguagem natural, painéis, alertas | Próximo |
| G3 Extensibilidade | Depois. As três documentações: paralelo, Antigravity |
| G4 Site, marca | Paralelo, Antigravity (fora deste squad) |

## Anti-goals

Não é SaaS nem APM de nuvem. Não controla agentes. Não avalia a qualidade do código. Não adivinha vínculos. O site não mostra números inventados como reais.

## Intenção de release

`v0.1.0`, impacto `minor`, build local, sem publicação.

## Decisão PM

Prosseguir. Fase 0 entregue ao `pm.analyst`.
