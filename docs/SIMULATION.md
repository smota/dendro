# Historical topology simulation

`simulation::simulate` is a pure calculation over a canonical decisions report and explicit operator assumptions. It never reruns agents, changes a rig, writes source logs, or predicts code quality. The selected report's rig, half-open window and formula version accompany every result.

Seat removal preserves historical work: supply a replacement seat before claiming projected cost changes. Without one, removed-seat work remains unassigned and projected total savings are unavailable. Tier rules attach explicit uncached/cache-read multipliers to a seat, optionally labelled L1/P0/P1/P2/P3, effort and model family. These ratios are operator assumptions; no cheaper-model productivity or quality benefit is inferred.

Direct routing considers only unique, complete, parent-linked in-ring leaf → coordinator → leaf paths from canonical handoff samples. Cross-rig seams, supervisory-plane edges, recovery traffic, ambiguous lineage, forks and absent timings refuse a bypass. The model assumes a direct transfer removes the intermediate coordinator's recorded hop; median pickup plus execution is a typical-cycle estimate, not measured elapsed time saved. Assigned coordinator usage keys are deduplicated before any input-unit projection. Destination engineering work stays present.

Historical input cost is `uncached * uncached_weight + cache_read * cache_read_weight`. Default weights 1.0 and 0.1 express uncached-equivalent input units, not currency. Output/cache-write components remain visible in the exact conservation ledger and are outside this input-only cost model. Any zero denominator stays unavailable.

Boundary scenarios require explicit turns, stale context, clean context and cold boot values. Warm carryover is `turns * stale_context * cache_read_weight`; boundary rebuild is `cold_boot * uncached_weight + turns * clean_context * cache_read_weight`. A threshold controls scenario eligibility; it cannot infer a clean-context counterfactual. Boundary costs are reported independently from tier/routing estimates, since their source usage can overlap. For the stated 15-turn, 350k/60k context, 35k cold boot example, weights 1.0/0.1 yield exactly 525k warm and 125k boundary units.

Observed mid-item compaction hazards remain historical facts. `projected_eliminated_hazards` stays unavailable: a static historical what-if cannot prove that a boundary reset would have eliminated actual hazards. Model-family diversity is projected only when explicit rule families or canonical chain evidence establish it, and never rewrites historical independence findings.

All projections cite their actual historical usage/handoff keys and enumerate assumptions. Missing baselines, unresolved allocation, unsupported source provenance, or incomplete causal adjacency return `no_data` rather than a fabricated saving. All numeric weights/multipliers must be finite and nonnegative.

`dendro simulate topology` runs `simulation::simulate` over the canonical `decisions` report for `--db <path>`, optional `--rig <rig>`, and `--from <utc>` / `--to <utc>`. It accepts `--scenario <JSON-path>` for explicit assumptions and `--tier-policy <JSON-path>` for an array of tier rules. Supplied topology flags (`--remove`, `--replacement-seat`, `--direct-handoffs`, `--boundary-reset-threshold`, `--enforce-model-family-diversity`) overlay only the specified fields. Unknown JSON fields, duplicate tier rules, removed replacement seats and nonfinite/negative numbers refuse. Example scenario:

```json
{
  "remove": ["dev-build-jr@team"],
  "replacement_seat": "dev-build@team",
  "direct_handoffs": false,
  "tier_policy": [{
    "seat": "dev-build@team",
    "uncached_multiplier": 0.8,
    "cache_read_multiplier": 1.0,
    "work_class": "P1",
    "model_family": "claude",
    "effort": "medium"
  }],
  "boundary_reset_threshold": 300000,
  "weights": {"uncached": 1.0, "cache_read": 0.1},
  "boundary": {
    "turns": 15,
    "stale_context_tokens": 350000,
    "clean_context_tokens": 60000,
    "cold_boot_tokens": 35000
  },
  "enforce_model_family_diversity": false
}
```

Omitted scenario fields default to empty removals/rules, no replacement/boundary/threshold, false direct-handoff/diversity flags, and weights 1.0/0.1. Tier multipliers default to 1.0; work_class, model_family and effort are optional. The example's ratios are assumptions, not recommended routing or validated quality claims. Seat identifiers must match the canonical baseline. Direct routing requires canonical four-eyes builder/reviewer role evidence; seat naming alone does not verify a leaf role. Unlinked usage stays conserved and outside named-seat policy changes.
