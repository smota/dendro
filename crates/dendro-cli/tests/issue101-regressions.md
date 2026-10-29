# Issue101 independent regression fixtures

These tests exercise persisted synthetic native table rows through the public
overview loader, live-query API and matching CLI. They never inject derived
overview incidents or derive expected numbers from production functions. Frozen
`.agentflow/oracles/issue101/**` files remain unchanged.

Author: delegated Codex test-engineering leaf `/root/issue101_oracle_author`,
independent of dev.build/dev.qa/rev.review. This is not a persistent seat identity
or a Phase5/6 verdict. Requested route was Codex gpt-6.1-sol/high; the collaboration
runtime is observed as Codex. Actual serving model/provider identity is not
independently attested by an inference receipt in this leaf.

## Native fixture and assertions

```sh
CARGO_TARGET_DIR=/home/sam/code/dendro/target cargo test --offline -p dendro-cli --test direct_overview_regressions -- --test-threads=1 --nocapture
```

The fixture owns fresh private directories under this worktree's `.agent-runs/`,
creates a new synthetic test key, initializes `AnalyticsStore`, and inserts safe
synthetic facts using parameterized SQL. Nothing opens an owner corpus.
Native Linux admission requires every ancestor to be non-group/world-writable;
the fixture removes those write bits only from its authorized scratch ancestors.
The worktree root must already meet admission requirements. Do not weaken the
native verifier to admit a test directory.

Primary literal population: one observed running hour, project fingerprint
`fp-a81f`, 160 tokens/$30 observed spend, three DONE issue tasks, seven internal
tasks, two supported uncalled declarations, three read invocations (one success,
one error, one unknown outcome), and a separate foreign rig. A claimed idle leaf
tests production incident derivation. Additional isolated persisted cases cover
container lineage, token/fingerprint/money/status/options/fractional-scope digest
changes, historical cutoff/reopen behavior, rejected completion evidence,
recovery/dedup/handoff, scoped invocation reconciliation and unknown versus zero.

Additive output interface confirmed by architect:

- `project_groups[].active_hours`, `project_groups[].closed_issue_qitems`.
- `unassigned_active_hours`, `unassigned_closed_issue_qitems` conserve unattributed facts.
- `context_tax.active_tools[]`: `tool_name`, `invocation_count`, `error_count`,
  `error_rate`, `unknown_outcome_count`.
- Error rate uses known outcomes: one error / two known outcomes = 0.5, with one
  additional unknown outcome explicitly disclosed. No known outcomes means null,
  not zero. The UI must label the known-outcome denominator and partial coverage.

No test in this suite manufactures an admission-hold persistence interface. Real
cross-rig hold/vortex integration and the full 5500-fact performance fixture remain
production/QA responsibilities under the frozen architecture; the native stall
and container cases specifically prevent the disconnected-loader defect.

## Populated browser acceptance

Export with the Cargo-built executable, using a fresh, unused absolute path:

```sh
DENDRO_ISSUE101_FIXTURE_DIR="$PWD/.agent-runs/issues/101/browser-native" CARGO_TARGET_DIR=/home/sam/code/dendro/target cargo test --offline -p dendro-cli --test direct_overview_regressions export_issue101_browser_fixture -- --ignored --exact --nocapture
DENDRO_ISSUE101_FIXTURE="$PWD/.agent-runs/issues/101/browser-native/fixture.json" node --test site/direct-overview.browser.test.cjs
```

The exporter records the exact `CARGO_BIN_EXE_dendro` path/SHA256, source HEAD,
fixture DB, site directory and literal expected metadata. The browser suite
requires this receipt; it never searches shared targets for an ambient binary.
Re-export after candidate HEAD or executable changes. Production source must match
HEAD. Test-only uncommitted changes do not affect the recorded production identity.

Use the installed isolated Playwright package/cache without modifying protected
oracle files. When the package is available in another worktree, supply its exact
`DENDRO_PLAYWRIGHT_MODULE` directory and `PLAYWRIGHT_BROWSERS_PATH` explicitly.

Eight browser cases exercise populated card values and source keys, viewport
geometry, active filter/mode retention, exact scope and copied digest replay,
mismatch refusal, lens requests, delayed obsolete responses, visible fetch errors,
loading/no-data disabling and a second rig. Request delays/aborts test transport
behavior; numerical payloads come from the real native backend. Copy commands are
parsed without shell execution and replayed against the receipt's executable and
explicit synthetic DB. Browser failures are acceptance failures, never a reason
to edit frozen expectations.

## Separate Linux socket fixture fix

`quack_owner.rs` uses a short `/tmp/dq-<pid>-<counter>/dendro.sock` path on Unix,
with mode0700 and RAII cleanup. Non-Unix platforms retain the native temp directory.
This prevents long CI TMPDIR/test labels from exceeding `sockaddr_un.sun_path`.
All numeric serialization/RPC assertions are retained; no test is skipped.

```sh
CARGO_TARGET_DIR=/home/sam/code/dendro/target cargo test --offline -p dendro-cli --test quack_owner test_query_view_numeric_column_serialization -- --exact --nocapture
CARGO_TARGET_DIR=/home/sam/code/dendro/target cargo test --offline -p dendro-cli --test quack_owner test_socket_fixture_path_fits_portable_unix_limit -- --exact --nocapture
```

Run receipts and red baseline output are scratch evidence in
`.agent-runs/issues/101/regression-round1/`. Native/browser test failures on rejected
3531146 are expected and do not certify the repaired candidate. Workspace Rust4,
full benchmark, fresh QA/review and Linux/Windows CI remain separate required gates.
