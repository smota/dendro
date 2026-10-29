# Issue 101 independent reference data and browser preflight

Author provenance: bounded delegated Codex test-engineering leaf
`/root/issue101_oracle_author`, independent of future builder, QA and reviewer.
Base: `6ee2f95506948d63519d0a43a74d293a31f653d7`, branch
`work/issue-101-direct-overview`. This is reference authoring and browser
availability evidence, not candidate verification or a phase acceptance verdict.

`cases.json` contains literal selected-output assertions and synthetic input
projections. It is intentionally data, with no evaluator or production API adapter.
Expected numbers/source memberships were authored independently, without executing
candidate formulas. The case metadata distinguishes Amendment 3 requirements from
architecture v2 decisions awaiting the parent's Phase 3 freeze. Architecture v2
supersedes the inspected draft v1. Test consumers must explicitly adapt projections
to actual storage/API types; assertion labels do not invent a response schema.

Observed source vocabulary:

| Source | Symbol / fact |
| --- | --- |
| `crates/dendro-core/src/storage.rs:509` | `usage_facts`: timestamp `ts`, `input_uncached`, `output_tokens`, cache counters, nullable `currency_total` |
| `crates/dendro-core/src/storage.rs:563` | `queue_facts`: `transition_id`, `qitem_id`, `state`, `issue_tags`, `is_recovery` |
| `crates/dendro-core/src/storage.rs:582` | `skill_facts`: `source_key`, `seat`, `skill`, `invoked`; no timestamp/schema operand |
| `crates/dendro-core/src/storage.rs:193` | `process_identities`: `identity_key`, `harness`, `session_link`, `cwd_fingerprint`, lifetime timestamps |
| `crates/dendro-core/tests/support/semantic_fixture.rs:31` | Canonical queue tags use extracted ID strings such as `"16"` |
| `crates/dendro-core/src/advisory.rs:208` | `emit`: `true_leaf_stall` / `unparked_container_hold`, strict `seconds > threshold`, duration and `source_keys` |
| `crates/dendro-core/src/advisory.rs:33` | Defaults: container hold 900s, leaf stall 1800s |
| `crates/dendro-core/src/vortex.rs:14` | `FrictionThresholds`: return rate, chain returns, rework tokens, loop duration |

The reference covers ranking and ties, strict thresholds, resolved/stale/absent
evidence, temporal labels, zero-call declaration reconciliation, unavailable schema
cost, priced/zero/unpriced/partial spend, zero denominator, DONE-only completion,
closure deduplication, multiple issue tags, recovery and reopen exclusions, unique
versus ambiguous concurrent CWD linkage, token/cost conservation and half-open
window/rig filtering. It does not substitute for later API/CLI replay, async
generation, activity-band, read-only or performance verification.

## Browser runner

Run from the isolated worktree root:

```sh
CI=true pnpm --dir .agentflow/oracles/issue101/browser install --frozen-lockfile --ignore-scripts --ignore-workspace --store-dir "$PWD/.agentflow/oracles/issue101/browser/.cache/pnpm-store"
PLAYWRIGHT_BROWSERS_PATH="$PWD/.agentflow/oracles/issue101/browser/.cache/ms-playwright" pnpm --dir .agentflow/oracles/issue101/browser exec playwright install chromium
pnpm --dir .agentflow/oracles/issue101/browser run probe
```

The probe sets its own local browser cache path and launches real headless Chromium,
sets viewport 1920x1080/DPR1, renders controlled HTML and asserts actual CSS geometry.
It loads no candidate source and makes no candidate UI claim. It has a 30-second
browser launch timeout and closes Chromium even on an assertion failure.
The exact pin comes from observed public npm registry metadata, recorded in
`browser/.runs/registry.json`; the lock contains registry integrity values.
The pnpm 12 lockfile includes a package-manager document and dependency document.
Always pass the local `--store-dir` explicitly; the default store is shared.

Observed environment: Node 22.23.3, pnpm 12.11.2, Playwright 1.64.1,
Chromium 156.0.8078.4 (Playwright revision 1248), Linux ext4. The successful
probe measured innerWidth=1920, innerHeight=1080, devicePixelRatio=1,
scrollY=0, root scrollHeight=1080, rect=(40,30,600,480), bottom=510.
Only installed libraries were used; no system packages or privileged dependency
installation occurred.

Network and cache effects: npm metadata/package downloads and Playwright CDN
downloads occurred. Chromium install also downloaded its headless shell and FFmpeg
into the ignored local browser cache. The first pnpm install populated its default
shared content-addressed store at `/home/sam/.local/share/pnpm/store/v11`; this
was observed and reported, not removed. A subsequent install with explicit local
store rebuilt package links under the ignored test directory; current
`node_modules/.modules.yaml` confirms the local store. No global package, root
package/lock/config, production source or runtime/service changes were made.

Future candidate browser verification must measure four decision cards plus
`direct-overview-fold` and lenses within 900px at 1920x1080, initial window scroll,
mode/filter/URL preservation and a 480–640px nonmodal drawer with initial focus,
reachable close, Escape and trigger focus restoration. No Tab trap is required by
architecture v2. Include loading/error/empty/long-value states. This preflight
deliberately does not freeze candidate DOM selectors beyond the named fold contract.

## Integrity and handoff

`manifest.json` separates frozen reference-data hashes from tooling hashes,
upstream input hashes and ignored environment/run evidence. Verify hashes from
this directory; hash values never depend on a production evaluator.
The manifest does not include its own digest. Parent must independently verify
it and the runnable probe, then freeze the Phase 3 plan before production starts.
All leaf writes stop at handoff; no commit, staging, push, PR, Cargo, rig action or
candidate acceptance was performed by this author.
