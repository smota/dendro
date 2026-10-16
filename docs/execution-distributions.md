# Execution distributions

AgentFlow owns a versioned portable distribution in
`manifests/execution-distributions/subscription-balanced-v1.json`. It is the source for existing
and expansion roles: default model, explicit client/provider route, effort and exactly two ordered
fallbacks. Subscription budget classes are person-provided constraints, not verified entitlements.
The balanced profile favors the existing Gemini subscription for delivery work; Munito experiments
are outside its default and fallback routes. Persistent roles use low/medium effort; high effort
is reserved for a bounded clean-context specialist. Quota control defaults off.

## Select and inspect

The authoritative workflow configuration (`agent-workflow.config.json`, or unified
`agentflow.config.json`) contains `executionDistribution`: profile ID, immutable profile digest
and explicit reasoned overrides. New projects receive a seeded selection. Existing projects use
an explicit migration; an absent selection retains legacy compatibility, not an invented adoption.

```bash
agentflow-sdlc distribution catalog --json
agentflow-sdlc distribution inspect --target /path/to/project --json
agentflow-sdlc distribution check --target /path/to/project --rig /path/to/rig.yaml --json
```

Inspect reports desired tuples for every role, including expansion roles; it launches no agent.
An expansion role still needs its own supported profile and task authority before materialization.
Check compares the supplied installed roster's model/client/effort against the selected defaults;
it does not prove a complete fleet or observe a live model. Missing/unrecognized seats are errors
rather than silently adopted roles. Existing workflow/review/publication authority is unchanged.

## Change policy separately from software

```bash
agentflow-sdlc distribution plan --target /path/to/project --json > policy-plan.json
agentflow-sdlc distribution apply --target /path/to/project --plan policy-plan.json --confirm DIGEST
```

Store plans outside the project or in ignored `.agent-runs/`. Review the plan's selected profile,
all role tuples, previous selection and removal of legacy per-seat overrides. `--selection` accepts
a JSON selection with reasoned per-role `overrides`, each containing a complete three-option chain.
Experimental/new provider routes cannot enter the subscription-balanced profile through an override.
A different distribution requires a separately defined version and supported migration.

Apply requires the exact current plan digest, shares the adoption writer lock, preserves unrelated
configuration and native state, and records a private rollback preimage under `.agent-runs/`.
It clears superseded legacy adapter seat/preset maps, leaving activation/publication holds intact.
It does not write installed rigs, start a daemon, dispatch work or grant native authority. Installed
rollout uses the separately authorized adapter operation with stopped/fenced writers, preview,
readback and rollback evidence. The project selection is not an online mutation grant.

```bash
agentflow-sdlc distribution undo --target /path/to/project --receipt .agent-runs/RECEIPT.json --confirm RECEIPT_DIGEST
```

Undo refuses newer configuration changes and restores exact prior bytes. A prepared receipt after
interruption requires inspection; if the replacement landed, matching readback permits compensation.
A successful policy edit is not an installed or live runtime confirmation.

## Temporary availability is separate

`distribution inspect --availability observations.json` can propose the first eligible option.
Each observation names a subscription, status (`healthy` or `unavailable`), `observedAt` and
`expiresAt` in epoch milliseconds. Evidence is unique per subscription, fresh for at most five
minutes, and must not be future-dated. Missing, duplicate or stale evidence holds selection.
The result is an eligibility proposal with no effects, not authenticated runtime acceptance.

The shared selector supports excluded provider families for independent review. When building
falls back to OpenAI, exclude OpenAI from the reviewer options as required by the task contract.
Changing Gemini models on the same subscription does not create an independent quota pool.
After verified recovery, return to the preferred route at a safe checkpoint. Never rewrite the
saved policy to explain a transient quota event. Automatic monitoring belongs to goal #535.

## Generated projections and compatibility

`node scripts/generate-execution-distribution.mjs --write` projects the default preset and culture
rules into the three OpenRig bundles. `pnpm validate:distribution` detects drift. Do not maintain
independent model tables in AGENTS.md, skills or culture. Existing legacy presets remain explicit
compatibility options for projects without a selected distribution; a selected project cannot
silently replace its policy with a preset, invocation tuple or old per-seat map.

Software upgrades preserve project-owned selections and overrides. Digest mismatch or a missing
profile version holds and asks for a separate migration instead of substituting the newest profile.
Generated template defaults serve new projects; they do not authorize existing project migration.
Desired settings, temporary eligibility and observed execution remain distinct evidence.
