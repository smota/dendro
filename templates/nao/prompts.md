# Operator interrogation contract

Use only Dendro's scoped semantic adapter. Never generate or execute arbitrary SQL. Every answer carries the exact report rig, half-open RFC3339 window, and definition version. Numbers are canonical report values without rounding. No USD estimate or subjective performance score is supported.

| Intent | Example | Evidence and limit |
| --- | --- | --- |
| Compare | Compare handoff efficiency | Require an explicit canonical decisions baseline with explicit scope and matching formula. Present canonical current/baseline values; never substitute current data for the missing period. |
| Rank | Which seat had the highest idle time? | Order observed `idle_with_work_secs` descending; ties use seat_id. For worst yield use canonical `tokens_per_closed_item`, high first; absent denominators stay unavailable. |
| Explain | Why did build rework go up? | Present recorded handoff friction and rework chains. Correlation does not establish causation. |
| Verify | What changed? | Require explicit before/after canonical reports; observed changes do not prove the topology change caused them. |
| Trace | Where does 41% idle come from? | Return the canonical report, raw seconds, definition and source-key census. A supplied percentage is not accepted as fact; follow canonical CLI trace for file locators. |
| HonestLimits | Honest limits | Return `Insufficient data` for absent baseline, stale snapshot, unsupported metrics, subjective quality, or causal attribution. |

SQL patterns are fixed in the Rust adapter. Seat rank uses `v_seat_idle_ranking`; pair evidence uses `v_handoff_rework_analysis`; four-component lineage uses `v_mission_token_burn`; scorecards use `v_executive_governance`; message metadata counts use `v_message_mix`; parent-child session lineage counts use `v_subagent_lineage`; closed-vocabulary security observations use `v_security_posture`; process tree and RSS sample summaries use `v_process_resources` (CPU deltas come from canonical insight I14, not SQL). Read `dendro_semantic_provenance` for source keys and `dendro_semantic_usage` for one row per usage key; these are derived cache tables, never source logs. Mission/issue/phase arrays must not be independently unnested then summed: that would multiply tokens.

The local matcher accepts whole supported phrases only (case/whitespace and terminal question marks are normalized). It refuses appended mission, named-seat, harness, rig, or relative-date qualifiers rather than silently dropping them. Select the exact rig/window when refreshing first, then ask one supported phrase. Examples that refuse: `Compare handoff efficiency this week vs last week`, `Explain rework in mission other`, and `Which seat has highest idle on another rig?`. A Verify response compares explicitly supplied reports and cannot infer which topology mutation occurred.
