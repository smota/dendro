# OpenRig Discussion #1099: Learning and Improving

**Referência Original:** [mvschwarz/openrig Discussion #1099](https://github.com/mvschwarz/openrig/discussions/1099)
**Autor:** `@smota`

## As Três Perguntas Centrais

1. **"Estou superdimensionando os assentos (seats)?"**
   * *Dúvida:* Am I over-engineering the seats? (too much, too little, redundancy).
   * *Métrica Dendro:* Volume de trabalho por assento, tempo em idle, tarefas repetidas entre assentos.

2. **"O handover entre os agentes está ótimo?"**
   * *Dúvida:* Is the handover optimal for agents? (context, cost, friction).
   * *Métrica Dendro:* Overhead de tokens na transição de contexto, taxa de bloqueio nas filas (`held`), retrabalho após handoff.

3. **"Estou usando os recursos computacionais adequadamente?"**
   * *Dúvida:* Am I using computing resources adequately? (too much idle, infinite loops, token burn).
   * *Métrica Dendro:* Top queima de tokens por assento/missão, detecção de loops de ferramentas e alertas de watchdog.
