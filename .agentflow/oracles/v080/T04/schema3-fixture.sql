-- Synthetic schema3 oracle recipe; test-only key. No production data.

            CREATE TABLE dendro_meta (
                schema_version INTEGER NOT NULL,
                key_id VARCHAR NOT NULL
            );
            CREATE TABLE source_files (
                source_id VARCHAR PRIMARY KEY,
                harness VARCHAR NOT NULL,
                display_path VARCHAR NOT NULL,
                generation BIGINT NOT NULL,
                byte_offset BIGINT NOT NULL,
                prefix_sha256 VARCHAR NOT NULL,
                native_session VARCHAR
            );
            CREATE TABLE codex_counters (
                source_id VARCHAR NOT NULL,
                generation BIGINT NOT NULL,
                input_tokens BIGINT NOT NULL,
                cached_input_tokens BIGINT NOT NULL,
                cache_write_input_tokens BIGINT NOT NULL,
                output_tokens BIGINT NOT NULL,
                reasoning_output_tokens BIGINT NOT NULL,
                PRIMARY KEY (source_id, generation)
            );
            CREATE TABLE usage_facts (
                source_key VARCHAR PRIMARY KEY,
                harness VARCHAR NOT NULL,
                native_session VARCHAR NOT NULL,
                message_id VARCHAR,
                ts TIMESTAMP,
                seq BIGINT NOT NULL,
                input_uncached BIGINT NOT NULL,
                output_tokens BIGINT NOT NULL,
                cache_read BIGINT NOT NULL,
                cache_write BIGINT NOT NULL,
                reasoning BIGINT NOT NULL,
                currency_total DOUBLE,
                generation BIGINT NOT NULL,
                source_id VARCHAR NOT NULL,
                predecessor_source_key VARCHAR,
                predecessor_reason VARCHAR,
                prior_input_tokens BIGINT,
                prior_cached_input_tokens BIGINT,
                prior_cache_write_input_tokens BIGINT,
                prior_output_tokens BIGINT,
                prior_reasoning_output_tokens BIGINT,
                model VARCHAR,
                effort VARCHAR
            );
            CREATE TABLE tool_facts (
                source_key VARCHAR PRIMARY KEY,
                harness VARCHAR NOT NULL,
                native_session VARCHAR NOT NULL,
                ts TIMESTAMP,
                seq BIGINT NOT NULL,
                tool_name VARCHAR NOT NULL,
                fingerprint VARCHAR,
                failed BOOLEAN,
                structured_write BOOLEAN NOT NULL,
                generation BIGINT NOT NULL,
                source_id VARCHAR NOT NULL,
                category VARCHAR NOT NULL DEFAULT 'other'
            );
            CREATE TABLE identity_facts (
                source_key VARCHAR PRIMARY KEY,
                harness VARCHAR NOT NULL,
                native_session VARCHAR NOT NULL,
                rig VARCHAR NOT NULL,
                seat VARCHAR NOT NULL,
                ts TIMESTAMP
            );
            CREATE TABLE activity_facts (
                source_key VARCHAR PRIMARY KEY,
                seat VARCHAR NOT NULL,
                ts TIMESTAMP NOT NULL,
                state VARCHAR NOT NULL,
                seq BIGINT NOT NULL
            );
            CREATE TABLE queue_facts (
                source_key VARCHAR PRIMARY KEY,
                transition_id BIGINT NOT NULL,
                qitem_id VARCHAR NOT NULL,
                ts TIMESTAMP NOT NULL,
                state VARCHAR NOT NULL,
                source_seat VARCHAR NOT NULL,
                dest_seat VARCHAR NOT NULL,
                closure_reason VARCHAR,
                closure_target VARCHAR,
                chain_id VARCHAR,
                missions VARCHAR NOT NULL,
                is_recovery BOOLEAN NOT NULL DEFAULT FALSE,
                recovery_reason VARCHAR,
                last_nudge_result VARCHAR,
                issue_tags VARCHAR NOT NULL DEFAULT '[]',
                phase_tags VARCHAR NOT NULL DEFAULT '[]',
                parent_qitem_id VARCHAR
            );
            CREATE TABLE skill_facts (
                source_key VARCHAR PRIMARY KEY,
                seat VARCHAR NOT NULL,
                skill VARCHAR NOT NULL,
                invoked BOOLEAN NOT NULL
            );
            CREATE TABLE seat_bounds (
                seat VARCHAR NOT NULL,
                created_at TIMESTAMP,
                stopped_at TIMESTAMP
            );
            CREATE TABLE seat_roles (
                seat VARCHAR NOT NULL,
                role VARCHAR NOT NULL,
                evidence VARCHAR NOT NULL,
                PRIMARY KEY (seat, role, evidence)
            );
            CREATE TABLE warnings (
                path VARCHAR NOT NULL,
                line_no BIGINT,
                reason VARCHAR NOT NULL
            );
            CREATE TABLE destination_snapshots (
                source_key VARCHAR PRIMARY KEY,
                qitem_id VARCHAR NOT NULL,
                destination VARCHAR NOT NULL,
                snapshot_cutoff VARCHAR NOT NULL,
                reroute_marked BOOLEAN NOT NULL,
                missions VARCHAR NOT NULL
            );
            
CREATE TABLE IF NOT EXISTS file_source_baselines (
    source_id VARCHAR NOT NULL,
    generation BIGINT NOT NULL,
    metadata_version INTEGER NOT NULL,
    latest_association VARCHAR NOT NULL,
    latest_attempt_at VARCHAR NOT NULL,
    last_stable_observed_at VARCHAR,
    last_stable_type VARCHAR,
    last_stable_device VARCHAR,
    last_stable_inode VARCHAR,
    last_stable_size VARCHAR,
    last_stable_mtime_ns VARCHAR,
    association_reason VARCHAR NOT NULL,
    PRIMARY KEY (source_id, generation)
);
CREATE TABLE IF NOT EXISTS dendro_semantic_state (id INTEGER PRIMARY KEY, dirty BOOLEAN NOT NULL); INSERT INTO dendro_semantic_state VALUES (1, TRUE) ON CONFLICT DO NOTHING;

CREATE TABLE IF NOT EXISTS openrig_row_records (
 source_id VARCHAR NOT NULL, generation BIGINT NOT NULL, producing_revision VARCHAR NOT NULL,
 table_name VARCHAR NOT NULL, row_id VARCHAR NOT NULL, row_digest VARCHAR NOT NULL,
 row_state VARCHAR NOT NULL, source_key VARCHAR NOT NULL,
 PRIMARY KEY (source_id, generation, table_name, row_id, source_key)
);
CREATE TABLE IF NOT EXISTS mutable_source_disclosures (
 qitem_id VARCHAR NOT NULL, seat VARCHAR NOT NULL, reason VARCHAR NOT NULL,
 PRIMARY KEY (qitem_id, seat, reason)
);

BEGIN TRANSACTION;
INSERT INTO dendro_meta (schema_version, key_id) VALUES (3, '630dcd2966c4336691125448bbb25b4f');
INSERT INTO source_files (source_id, harness, display_path, generation, byte_offset, prefix_sha256, native_session) VALUES ('src-A', 'codex', 'synthetic-source-A', 2, 512, '1111111111111111111111111111111111111111111111111111111111111111', 'native-A');
INSERT INTO codex_counters (source_id, generation, input_tokens, cached_input_tokens, cache_write_input_tokens, output_tokens, reasoning_output_tokens) VALUES ('src-A', 2, 100, 10, 0, 4, 0);
INSERT INTO usage_facts (source_key, harness, native_session, message_id, ts, seq, input_uncached, output_tokens, cache_read, cache_write, reasoning, currency_total, generation, source_id, predecessor_source_key, predecessor_reason, prior_input_tokens, prior_cached_input_tokens, prior_cache_write_input_tokens, prior_output_tokens, prior_reasoning_output_tokens, model, effort) VALUES ('usage-0', 'codex', 'native-A', NULL, NULL, 0, 0, 0, 0, 0, 0, NULL, 2, 'src-A', NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL);
INSERT INTO usage_facts (source_key, harness, native_session, message_id, ts, seq, input_uncached, output_tokens, cache_read, cache_write, reasoning, currency_total, generation, source_id, predecessor_source_key, predecessor_reason, prior_input_tokens, prior_cached_input_tokens, prior_cache_write_input_tokens, prior_output_tokens, prior_reasoning_output_tokens, model, effort) VALUES ('usage-1', 'codex', 'native-A', 'm1', '2026-10-22 00:00:00', 1, 90, 4, 10, 0, 0, 0.0, 2, 'src-A', 'usage-0', 'counter_delta', 0, 0, 0, 0, 0, NULL, NULL);
INSERT INTO usage_facts (source_key, harness, native_session, message_id, ts, seq, input_uncached, output_tokens, cache_read, cache_write, reasoning, currency_total, generation, source_id, predecessor_source_key, predecessor_reason, prior_input_tokens, prior_cached_input_tokens, prior_cache_write_input_tokens, prior_output_tokens, prior_reasoning_output_tokens, model, effort) VALUES ('usage-2', 'codex', 'native-A', 'm2', '2026-10-22 00:00:00', 2, 0, 0, 0, 0, 0, 0.1, 2, 'src-A', NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL);
INSERT INTO usage_facts (source_key, harness, native_session, message_id, ts, seq, input_uncached, output_tokens, cache_read, cache_write, reasoning, currency_total, generation, source_id, predecessor_source_key, predecessor_reason, prior_input_tokens, prior_cached_input_tokens, prior_cache_write_input_tokens, prior_output_tokens, prior_reasoning_output_tokens, model, effort) VALUES ('usage-3', 'codex', 'native-A', 'm3', '2026-10-22 00:00:00', 3, 0, 0, 0, 0, 0, 1e-320, 2, 'src-A', NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL);
INSERT INTO tool_facts (source_key, harness, native_session, ts, seq, tool_name, fingerprint, failed, structured_write, generation, source_id, category) VALUES ('tool-A', 'codex', 'native-A', '2026-10-22 00:00:00', 5, 'CommandExecution', 'synthetic-fingerprint', NULL, FALSE, 2, 'src-A', 'other');
INSERT INTO identity_facts (source_key, harness, native_session, rig, seat, ts) VALUES ('identity-A', 'codex', 'native-A', 'rig-A', 'seat-A', NULL);
INSERT INTO activity_facts (source_key, seat, ts, state, seq) VALUES ('activity-A', 'seat-A', '2026-10-22 00:00:00', 'working', 1);
INSERT INTO queue_facts (source_key, transition_id, qitem_id, ts, state, source_seat, dest_seat, closure_reason, closure_target, chain_id, missions, is_recovery, recovery_reason, last_nudge_result, issue_tags, phase_tags, parent_qitem_id) VALUES ('queue-A', 17, 'q17', '2026-10-22 00:00:01', 'closed', 'seat-A', 'seat-B', 'completed', NULL, 'chain-A', '["mission-A"]', FALSE, NULL, NULL, '[]', '["build"]', NULL);
INSERT INTO skill_facts (source_key, seat, skill, invoked) VALUES ('skill-A', 'seat-A', 'synthetic-skill', FALSE);
INSERT INTO seat_bounds (seat, created_at, stopped_at) VALUES ('seat-A', '2026-10-22 00:00:00', NULL);
INSERT INTO seat_roles (seat, role, evidence) VALUES ('seat-A', 'builder', 'synthetic-configured-role');
INSERT INTO warnings (path, line_no, reason) VALUES ('synthetic-source-A', NULL, 'synthetic-warning');
INSERT INTO destination_snapshots (source_key, qitem_id, destination, snapshot_cutoff, reroute_marked, missions) VALUES ('destination-A', 'q17', 'seat-B', '2026-10-22 00:00:01', FALSE, '["mission-A"]');
INSERT INTO file_source_baselines (source_id, generation, metadata_version, latest_association, latest_attempt_at, last_stable_observed_at, last_stable_type, last_stable_device, last_stable_inode, last_stable_size, last_stable_mtime_ns, association_reason) VALUES ('src-A', 2, 1, 'stable', '2026-10-22 00:00:01', '2026-10-22 00:00:00', 'regular', '7', '19', '512', '1792627200000000000', 'synthetic-stable');
INSERT INTO openrig_row_records (source_id, generation, producing_revision, table_name, row_id, row_digest, row_state, source_key) VALUES ('rig-source', 2, 'r2', 'queue_items', '17', '2222222222222222222222222222222222222222222222222222222222222222', 'present', 'queue-A');
INSERT INTO mutable_source_disclosures (qitem_id, seat, reason) VALUES ('q17', 'seat-B', 'synthetic-destination-change');
COMMIT;
