-- Arrays stay grouping dimensions: never unnest them into a token-multiplying join.
CREATE OR REPLACE TABLE dendro_semantic_missions AS
SELECT seat_id,model,effort,attribution_bucket,missions,issues,phases,
 SUM(input_uncached)::BIGINT input_uncached, SUM(output)::BIGINT output,
 SUM(cache_read)::BIGINT cache_read, SUM(cache_write)::BIGINT cache_write,
 list(source_key ORDER BY source_key) source_keys
FROM dendro_semantic_usage GROUP BY ALL;

CREATE OR REPLACE VIEW v_seat_idle_ranking AS
SELECT t.*, s.* FROM dendro_semantic_seats t CROSS JOIN dendro_semantic_scope s CROSS JOIN dendro_semantic_guard g WHERE g.fresh;
CREATE OR REPLACE VIEW v_handoff_rework_analysis AS
SELECT t.*, s.* FROM dendro_semantic_handoffs t CROSS JOIN dendro_semantic_scope s CROSS JOIN dendro_semantic_guard g WHERE g.fresh;
CREATE OR REPLACE VIEW v_mission_token_burn AS
SELECT t.*, s.* FROM dendro_semantic_missions t CROSS JOIN dendro_semantic_scope s CROSS JOIN dendro_semantic_guard g WHERE g.fresh;
CREATE OR REPLACE VIEW v_executive_governance AS
SELECT t.*, s.* FROM dendro_semantic_executive t CROSS JOIN dendro_semantic_scope s CROSS JOIN dendro_semantic_guard g WHERE g.fresh;
CREATE OR REPLACE VIEW v_message_mix AS
SELECT t.*, s.* FROM dendro_semantic_message_mix t CROSS JOIN dendro_semantic_scope s CROSS JOIN dendro_semantic_guard g WHERE g.fresh;
CREATE OR REPLACE VIEW v_subagent_lineage AS
SELECT t.*, s.* FROM dendro_semantic_subagent_lineage t CROSS JOIN dendro_semantic_scope s CROSS JOIN dendro_semantic_guard g WHERE g.fresh;
CREATE OR REPLACE VIEW v_security_posture AS
SELECT t.*, s.* FROM dendro_semantic_security_posture t CROSS JOIN dendro_semantic_scope s CROSS JOIN dendro_semantic_guard g WHERE g.fresh;
CREATE OR REPLACE VIEW v_process_resources AS
SELECT t.*, s.* FROM dendro_semantic_process_resources t CROSS JOIN dendro_semantic_scope s CROSS JOIN dendro_semantic_guard g WHERE g.fresh;
