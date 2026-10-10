# Dendro CLI

Dendro is an AI Fleet Intelligence tool. Reports are Operational Intelligence. Fleet Governance decisions stay with the operator.

The sealed acceptance contract is Amendment 1. This page only describes the commands.

```text
dendro run --db dendro.duckdb
dendro report <links|activity|handoffs|tokens|decisions> --db dendro.duckdb --rig <rig> --from <utc> --to <utc>
dendro trace --db dendro.duckdb --metric-id <id>
```

`report` and `trace` print JSON. A missing M1 denominator is JSON null, which means no data, not zero. M4 impact stays insufficient data. Nao and watch are outside this R1 proof.

A first-ingest timing result does not by itself prove AC1. AC1 also requires the first report.
