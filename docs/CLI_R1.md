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

## Private staging and snapshots

`dendro run` holds a lock file, `.dendro-output.lock`, next to the analytics database for the whole run. A second run in the same directory is refused while the lock is held. The run works in one private stage directory in that same directory, `.dendro-stage-<16 hex>`. The stage has mode 0700 and a fixed marker file, `.dendro-stage`. With `--frozen`, Dendro copies the OpenRig database and its `-wal` and `-shm` files into the stage's `snapshot/` directory before reading them. Dendro creates those copies, the marker and the staged copy of an existing analytics database with mode 0600. Files DuckDB creates in the stage follow the process umask; the 0700 stage keeps other users out. Raw snapshots never go to `TMPDIR`.

- A run removes its stage when it ends normally or with an error. If removal fails, the run exits nonzero and names the stage.
- On SIGINT or SIGTERM the run stops at the next line, row or copy chunk, closes its databases, removes its stage and exits with 130 or 143. If it cannot stop within 30 seconds, or a second signal arrives, it exits without deleting files that may still be written and says that cleanup failed.
- After SIGKILL or a host crash, the next run in that directory removes the old stage while it holds the lock. It removes only real directories owned by the user with a valid marker and expected contents. It keeps and lists everything else, including look-alike names, symlinks and directories without a marker. The `staging` field of the run's JSON output lists what was removed or kept.

## Missing key

An existing analytics database without its `.key` sidecar refuses ingest and stays unchanged. `report` and `trace` still read it. They create no key, compute no new fingerprint and print no key material. A first run with no database creates the key, as before (K8).
