-- Typed staging is populated from the canonical Value with appenders/prepared statements.
-- Never expand a large parent report through lateral JSON table functions.
CREATE OR REPLACE TABLE dendro_semantic_scope (
 rig VARCHAR, window_from VARCHAR, window_to VARCHAR, definition_version VARCHAR, refreshed_at TIMESTAMP
);
CREATE OR REPLACE TABLE dendro_semantic_seats (
 seat_id VARCHAR, observed_secs BIGINT, active_secs BIGINT, idle_with_work_secs BIGINT,
 idle_between_work_secs BIGINT, waiting_external_secs BIGINT, unknown_secs BIGINT,
 unparked_container_secs BIGINT, leaf_stall_secs BIGINT, ceremony_ratio DOUBLE,
 tool_categories JSON, row_json JSON, source_keys VARCHAR[], m1 JSON
);
CREATE OR REPLACE TABLE dendro_semantic_handoffs (
 from_seat VARCHAR, to_seat VARCHAR, handoffs BIGINT, rework_returns BIGINT, return_rate DOUBLE,
 wait_median_ms DOUBLE, exec_turnaround_median_ms DOUBLE, ring_boundary VARCHAR,
 source_keys JSON, row_json JSON
);
CREATE OR REPLACE TABLE dendro_semantic_usage (
 source_key VARCHAR PRIMARY KEY, seat_id VARCHAR, model VARCHAR, effort VARCHAR,
 attribution_bucket VARCHAR, missions JSON, issues JSON, phases JSON,
 input_uncached BIGINT, output BIGINT, cache_read BIGINT, cache_write BIGINT, lineage JSON
);
CREATE OR REPLACE TABLE dendro_semantic_executive (
 row_json JSON, source_keys VARCHAR[], m1_yield JSON, m1_by_seat JSON, m2_return_rate JSON,
 m3_rework_chains JSON, token_categories JSON, link_rates JSON, fleet_concurrency JSON,
 watchdog_recovery JSON, four_eyes_independence JSON, context_compactions JSON,
 cross_slice_cache_bleed JSON, task_intake_overhead JSON, issue_lineage JSON,
 vortex_loops JSON, ring_boundaries JSON
);
CREATE OR REPLACE TABLE dendro_semantic_message_mix (
 harness VARCHAR, native_session VARCHAR, role VARCHAR, direction VARCHAR, modality VARCHAR,
 message_count BIGINT, source_keys VARCHAR[]
);
CREATE OR REPLACE TABLE dendro_semantic_subagent_lineage (
 harness VARCHAR, parent_session VARCHAR, child_count BIGINT, edge_count BIGINT,
 started_count BIGINT, finished_count BIGINT, failed_count BIGINT, unknown_count BIGINT,
 child_sessions VARCHAR[], relations VARCHAR[], source_keys VARCHAR[]
);
CREATE OR REPLACE TABLE dendro_semantic_security_posture (
 harness VARCHAR, kind VARCHAR, value VARCHAR, observation_count BIGINT,
 session_count BIGINT, source_keys VARCHAR[]
);
CREATE OR REPLACE TABLE dendro_semantic_process_resources (
 root_key VARCHAR, harness VARCHAR, attribution VARCHAR, rss_unit VARCHAR,
 process_count BIGINT, max_rss_bytes BIGINT, sample_count BIGINT, source_keys VARCHAR[]
);
