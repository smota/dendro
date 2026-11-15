//! Connector field-level fidelity inspection and grounding helpers (`dendro#134`).
//!
//! Separate from `stream.rs` so connector fidelity declarations and store-level grounding checks
//! do not require modifying `stream.rs`.

pub use dendro_core::insight_v1::{
    canonical_connector_id, canonical_population_what, connector_declaration, denominator_matrix,
    evaluate_catalog_for_connectors, evaluate_denominator, evaluate_denominator_by_id,
    evaluate_for_connectors, evaluate_insight_for_connectors, format_denominator_disclosure,
    format_not_computable_reason, populated_fields_in_store, render_provider_fidelity_markdown,
    ConnectorDeclaration, ConnectorExclusion, DenominatorEvaluation, DenominatorMatrixRow,
    DenominatorScope, Field, FieldDeclaration, FieldFidelity, IncludedConnector,
    PartialFidelityReason, CONNECTOR_DECLARATIONS,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture_registry_v1::Registry;
    use crate::ingest::ingest_staged;
    use crate::SourceRoot;
    use chrono::{TimeZone, Utc};
    use dendro_core::domain_v1::{CaptureMethod, EnvironmentKind, Unit};
    use dendro_core::insight_v1::{InsightStatus, MeasureValue, Scope, CATALOG};
    use dendro_core::{AnalyticsStore, ProcessIdentityFact, ProcessSampleFact};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    struct TempDir {
        #[cfg(any(windows, target_os = "linux"))]
        _guard: Option<dendro_core::platform_v1::PrivateRoot>,
        path: PathBuf,
    }

    impl TempDir {
        fn new(label: &str) -> Self {
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let prefix = format!("dendro-d5-fidelity-{label}-{n}-");
            #[cfg(any(windows, target_os = "linux"))]
            {
                let guard = dendro_core::platform_v1::create_user_private_root_with_choice(
                    &prefix,
                    &dendro_core::platform_v1::StorageRootChoice::UserProfile,
                )
                .expect("explicit UserProfile staged ingest fixture");
                let path = guard.path().to_path_buf();
                Self {
                    _guard: Some(guard),
                    path,
                }
            }
            #[cfg(not(any(windows, target_os = "linux")))]
            {
                let path = std::env::temp_dir().join(format!("{prefix}{}", std::process::id()));
                let _ = std::fs::remove_dir_all(&path);
                std::fs::create_dir_all(&path).unwrap();
                Self { path }
            }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            #[cfg(any(windows, target_os = "linux"))]
            {
                self._guard.take();
            }
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn ingest_roots_into_store(
        stage_dir: &Path,
        roots: &[SourceRoot],
        key: &[u8; 32],
    ) -> AnalyticsStore {
        let db_path = stage_dir.join("staged.duckdb");
        drop(AnalyticsStore::open_path(&db_path, &dendro_core::key_id(key)).unwrap());
        let _warnings = ingest_staged(&db_path, roots, false, key).unwrap();
        AnalyticsStore::open_path(&db_path, &dendro_core::key_id(key)).unwrap()
    }

    fn assert_grounding(connector_name: &str, store: &AnalyticsStore) {
        let decl = connector_declaration(connector_name)
            .unwrap_or_else(|| panic!("missing declaration for {connector_name}"));
        let populated = populated_fields_in_store(store).unwrap();

        for field in decl.observed_fields() {
            assert!(
                populated.contains(&field),
                "connector `{connector_name}` declares `{field}` Observed ({}) but fixture ingestion did not populate it; populated = {populated:?}",
                decl.declaration(field).unwrap().code_location
            );
        }
        for field in decl.derived_fields() {
            assert!(
                populated.contains(&field),
                "connector `{connector_name}` declares `{field}` Derived ({}) but fixture ingestion did not populate it; populated = {populated:?}",
                decl.declaration(field).unwrap().code_location
            );
        }
        for field in decl.absent_fields() {
            assert!(
                !populated.contains(&field),
                "connector `{connector_name}` declares `{field}` Absent ({}) but fixture ingestion populated it!",
                decl.declaration(field).unwrap().code_location
            );
        }
    }

    fn write_claude_fixture(dir: &Path) {
        let file = dir.join("session-claude.jsonl");
        let lines = [
            r#"{"type":"user","sessionId":"claude-s1","permissionMode":"default","timestamp":"2026-10-10T10:00:00Z","message":{"role":"user","content":"run /commit"}}"#,
            r#"{"type":"assistant","sessionId":"claude-s1","isSidechain":true,"agentId":"agent-child-1","timestamp":"2026-10-10T10:00:01Z","effort":"high","message":{"id":"msg_claude_1","model":"claude-sonnet-4-5","effort":"high","role":"assistant","usage":{"input_tokens":120,"output_tokens":45,"cache_read_input_tokens":30,"cache_creation_input_tokens":15},"content":[{"type":"tool_use","id":"toolu_1","name":"Bash","input":{"command":"rm -rf /tmp/test","dangerouslyDisableSandbox":true}},{"type":"tool_use","id":"toolu_2","name":"Skill","input":{"skill":"commit"}}]}}"#,
            r#"{"type":"user","sessionId":"claude-s1","timestamp":"2026-10-10T10:00:02Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_1","is_error":false,"content":"ok"},{"type":"tool_result","tool_use_id":"toolu_2","is_error":false,"content":"ok"}]}}"#,
        ];
        std::fs::write(file, format!("{}\n", lines.join("\n"))).unwrap();
    }

    fn write_codex_fixture(dir: &Path) {
        let file = dir.join("rollout-codex.jsonl");
        let lines = [
            r#"{"timestamp":"2026-10-10T10:00:00Z","type":"session_meta","payload":{"id":"codex-s1","source":{"subagent":{"thread_spawn":{"parent_thread_id":"codex-parent-0","depth":1}}}}}"#,
            r#"{"timestamp":"2026-10-10T10:00:01Z","type":"turn_context","payload":{"model":"gpt-5-codex","approval_policy":"on-request","sandbox_policy":{"type":"workspace-write"},"collaboration_mode":{"mode":"default","settings":{"model":"gpt-5-codex","reasoning_effort":"high"}}}}"#,
            r#"{"timestamp":"2026-10-10T10:00:02Z","type":"event_msg","payload":{"type":"user_message","message":"Use $review-skill now"}}"#,
            r#"{"timestamp":"2026-10-10T10:00:03Z","type":"response_item","payload":{"type":"function_call","name":"exec_command","call_id":"call_codex_1","arguments":"{\"cmd\":\"rm -rf /tmp/test && cat /skills/review-skill/SKILL.md\",\"dangerouslyDisableSandbox\":true}"}}"#,
            r#"{"timestamp":"2026-10-10T10:00:04Z","type":"response_item","payload":{"type":"function_call_output","call_id":"call_codex_1","output":"Process exited with code 0\nOutput:\nok"}}"#,
            r#"{"timestamp":"2026-10-10T10:00:05Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":200,"cached_input_tokens":50,"cache_write_input_tokens":20,"output_tokens":80,"reasoning_output_tokens":25,"total_tokens":280},"last_token_usage":{"input_tokens":200,"cached_input_tokens":50,"cache_write_input_tokens":20,"output_tokens":80,"reasoning_output_tokens":25,"total_tokens":280}}}}"#,
        ];
        std::fs::write(file, format!("{}\n", lines.join("\n"))).unwrap();
    }

    fn write_pi_fixture(dir: &Path) {
        let file = dir.join("session-pi.jsonl");
        let lines = [
            r#"{"type":"session","id":"pi-s1","timestamp":"2026-10-10T10:00:00Z","cwd":"/tmp"}"#,
            r#"{"type":"thinking_level_change","id":"tl1","timestamp":"2026-10-10T10:00:01Z","thinkingLevel":"high"}"#,
            r#"{"type":"message","id":"pi-msg-1","timestamp":"2026-10-10T10:00:02Z","message":{"id":"pi-msg-1","role":"assistant","model":"claude-sonnet-4-5","thinkingLevel":"high","usage":{"input":100,"output":40,"cacheRead":25,"cacheWrite":10,"reasoning":12,"cost":{"total":0.015}},"content":[{"type":"toolCall","id":"pi_tc_1","name":"bash","arguments":{"command":"rm -rf /tmp/test","dangerouslyDisableSandbox":true}},{"type":"toolCall","id":"pi_tc_2","name":"read","arguments":{"path":"/skills/test-skill/SKILL.md"}}]}}"#,
            r#"{"type":"message","id":"pi-msg-2","timestamp":"2026-10-10T10:00:03Z","message":{"role":"toolResult","toolCallId":"pi_tc_1","toolName":"bash","isError":false,"content":[{"type":"text","text":"ok"}]}}"#,
        ];
        std::fs::write(file, format!("{}\n", lines.join("\n"))).unwrap();
    }

    fn write_grok_fixture(dir: &Path) {
        let summary = serde_json::json!({
            "current_model_id": "grok-4.7-build-fast",
            "reasoning_effort": "high",
            "chat_format_version": 1
        });
        std::fs::write(
            dir.join("summary.json"),
            serde_json::to_string_pretty(&summary).unwrap(),
        )
        .unwrap();

        let lines = [
            r#"{"method":"session/update","timestamp":1760090400,"params":{"sessionId":"grok-s1","update":{"sessionUpdate":"subagent_spawned","parent_session_id":"grok-s1","child_session_id":"grok-child-1","subagent_id":"sub-1","attempt_id":"att-1"}}}"#,
            r#"{"method":"session/update","timestamp":1760090401,"params":{"sessionId":"grok-s1","update":{"sessionUpdate":"tool_call","toolCallId":"tc-1","rawInput":{"command":"cargo check"},"_meta":{"x.ai/tool":{"name":"Bash","kind":"execute"}}}}}"#,
            r#"{"method":"session/update","timestamp":1760090402,"params":{"sessionId":"grok-s1","update":{"sessionUpdate":"tool_call_update","toolCallId":"tc-1","status":"completed"}}}"#,
            r#"{"method":"session/update","timestamp":1760090403,"params":{"sessionId":"grok-s1","update":{"sessionUpdate":"turn_completed","usage":{"inputTokens":180,"outputTokens":60,"cachedReadTokens":30,"cacheCreationTokens":10,"reasoningTokens":15,"costUsdTicks":42000000}}}}"#,
        ];
        std::fs::write(dir.join("updates.jsonl"), format!("{}\n", lines.join("\n"))).unwrap();
    }

    #[test]
    fn matrix_test_every_insight_x_every_connector_is_two_state_and_never_panics() {
        let registry = Registry::builtin();
        for id in [
            "claude", "codex", "pi", "grok", "cursor", "windsurf", "goose", "devin", "agy",
        ] {
            assert!(
                registry.descriptor(id).is_some(),
                "builtin capture registry must include `{id}`"
            );
            assert!(
                connector_declaration(id).is_some(),
                "builtin capture registry provider `{id}` must have a field-level ConnectorDeclaration"
            );
        }

        assert_eq!(CATALOG.len(), 25);
        assert_eq!(CONNECTOR_DECLARATIONS.len(), 11);

        for insight in CATALOG {
            for conn in CONNECTOR_DECLARATIONS {
                let eval = evaluate_denominator(*insight, &[conn.connector]);
                match eval {
                    DenominatorEvaluation::Computable { scope } => {
                        assert_eq!(scope.insight_id, insight.id());
                        assert_eq!(scope.insight_name, insight.name());
                        assert_eq!(scope.included.len(), 1);
                        assert_eq!(scope.included[0].connector, conn.connector);
                        assert!(scope.excluded.is_empty());
                    }
                    DenominatorEvaluation::NotComputable {
                        insight_id,
                        insight_name,
                        missing,
                        by_connector,
                    } => {
                        assert_eq!(insight_id, insight.id());
                        assert_eq!(insight_name, insight.name());
                        assert!(
                            !missing.is_empty(),
                            "NotComputable for ({}, {}) must have non-empty missing fields",
                            insight.id(),
                            conn.connector
                        );
                        assert_eq!(by_connector.len(), 1);
                        assert_eq!(by_connector[0].connector, conn.connector);
                        assert_eq!(by_connector[0].missing, missing);
                    }
                }
            }
        }
    }

    #[test]
    fn declaration_grounding_claude_codex_pi_and_grok_fixtures() {
        let key = b"0123456789abcdef0123456789abcdef";

        // 1. Claude
        {
            let tmp = TempDir::new("claude");
            let data_dir = tmp.path().join("data");
            std::fs::create_dir_all(&data_dir).unwrap();
            write_claude_fixture(&data_dir);
            let root = SourceRoot {
                name: "claude".into(),
                path: data_dir,
            };
            let store = ingest_roots_into_store(tmp.path(), &[root], key);
            assert_grounding("claude", &store);
        }

        // 2. Codex
        {
            let tmp = TempDir::new("codex");
            let data_dir = tmp.path().join("data");
            std::fs::create_dir_all(&data_dir).unwrap();
            write_codex_fixture(&data_dir);
            let root = SourceRoot {
                name: "codex".into(),
                path: data_dir,
            };
            let store = ingest_roots_into_store(tmp.path(), &[root], key);
            assert_grounding("codex", &store);
            // Also verify Partial(ExplicitSignalOnly) ToolOutcome was populated by explicit exit code
            let populated = populated_fields_in_store(&store).unwrap();
            assert!(populated.contains(&Field::ToolOutcome));
        }

        // 3. Pi
        {
            let tmp = TempDir::new("pi");
            let data_dir = tmp.path().join("data");
            std::fs::create_dir_all(&data_dir).unwrap();
            write_pi_fixture(&data_dir);
            let root = SourceRoot {
                name: "pi".into(),
                path: data_dir,
            };
            let store = ingest_roots_into_store(tmp.path(), &[root], key);
            assert_grounding("pi", &store);
        }

        // 4. Grok
        {
            let tmp = TempDir::new("grok");
            let data_dir = tmp.path().join("data");
            std::fs::create_dir_all(&data_dir).unwrap();
            write_grok_fixture(&data_dir);
            let root = SourceRoot {
                name: "grok".into(),
                path: data_dir,
            };
            let store = ingest_roots_into_store(tmp.path(), &[root], key);
            assert_grounding("grok", &store);
            let populated = populated_fields_in_store(&store).unwrap();
            assert!(populated.contains(&Field::ObservedCost));
        }
    }

    #[test]
    fn declaration_grounding_export_connectors_cursor_windsurf_goose_devin() {
        let key = b"0123456789abcdef0123456789abcdef";
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("exports");

        let cases: &[(&str, &str)] = &[
            ("cursor", "cursor.jsonl"),
            ("windsurf", "windsurf.jsonl"),
            ("goose", "goose.jsonl"),
            ("devin", "devin.jsonl"),
        ];

        for (name, file_name) in cases {
            let tmp = TempDir::new(name);
            let data_dir = tmp.path().join("data");
            std::fs::create_dir_all(&data_dir).unwrap();
            let dest = data_dir.join(file_name);
            std::fs::copy(fixtures.join(file_name), &dest).unwrap();
            if *name == "cursor" {
                let mut content = std::fs::read_to_string(&dest).unwrap();
                content.push_str(
                    "{\"type\":\"tool_call\",\"subtype\":\"completed\",\"session_id\":\"synthetic-cursor\",\"call_id\":\"call-2\",\"timestamp_ms\":1767225601000,\"tool_call\":{\"readToolCall\":{\"args\":{\"path\":\"a.txt\"},\"result\":{\"success\":{}}}}}\n",
                );
                std::fs::write(&dest, content).unwrap();
            }
            let root = SourceRoot {
                name: (*name).to_string(),
                path: data_dir,
            };
            let store = ingest_roots_into_store(tmp.path(), &[root], key);
            assert_grounding(name, &store);
        }
    }

    #[test]
    fn declaration_grounding_openrig_agy_and_environment() {
        let key = b"0123456789abcdef0123456789abcdef";

        // 1. OpenRig
        {
            let tmp = TempDir::new("openrig");
            let sqlite_path = tmp.path().join("openrig.sqlite");
            let conn = rusqlite::Connection::open(&sqlite_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE events (seq INTEGER, type TEXT, payload TEXT, created_at TEXT);
                 CREATE TABLE queue_transitions (
                     transition_id INTEGER, qitem_id TEXT, ts TEXT, state TEXT,
                     transition_note TEXT, actor_session TEXT, closure_reason TEXT, closure_target TEXT
                 );
                 CREATE TABLE snapshots (
                     id TEXT, rig_id TEXT, data TEXT, created_at TEXT
                 );",
            )
            .unwrap();
            conn.execute(
                "INSERT INTO events VALUES (1, 'node.launched', '{\"rigId\":\"r1\",\"nodeId\":\"n1\",\"sessionName\":\"dev.build\"}', '2026-10-10T10:00:00Z')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO events VALUES (2, 'agent.session_identity', '{\"sessionId\":\"sess-1\",\"runtime\":\"claude\",\"sessionName\":\"dev.build\",\"rigId\":\"r1\"}', '2026-10-10T10:00:01Z')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO events VALUES (3, 'seat.activity_changed', '{\"sessionName\":\"dev.build\",\"state\":\"running\",\"at\":\"2026-10-10T10:00:02Z\"}', '2026-10-10T10:00:02Z')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO queue_transitions VALUES (1, 'q-1', '2026-10-10T10:00:03Z', 'pending', NULL, 'dev.build', NULL, NULL)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO snapshots VALUES ('snap-1', 'r1', '{\"nodes\":[{\"id\":\"n1\",\"packageRefs\":[\"skill-alpha\"]}],\"sessions\":[{\"sessionName\":\"dev.build\",\"nodeId\":\"n1\"}]}', '2026-10-10T10:00:04Z')",
                [],
            )
            .unwrap();
            drop(conn);

            let root = SourceRoot {
                name: "openrig".into(),
                path: sqlite_path,
            };
            let store = ingest_roots_into_store(tmp.path(), &[root], key);
            assert_grounding("openrig", &store);
        }

        // 2. AGY (metadata-only / refuses protobuf ingest -> zero fields populated, all Absent)
        {
            let tmp = TempDir::new("agy");
            let data_dir = tmp.path().join("data");
            std::fs::create_dir_all(&data_dir).unwrap();
            std::fs::write(data_dir.join("conv.pb"), b"\x0a\x04test").unwrap();
            let root = SourceRoot {
                name: "agy".into(),
                path: data_dir,
            };
            let store = ingest_roots_into_store(tmp.path(), &[root], key);
            assert_grounding("agy", &store);
        }

        // 3. Environment (`environment_v1`)
        {
            let store = AnalyticsStore::open_in_memory("ground-environment").unwrap();
            let now = Utc.with_ymd_and_hms(2026, 10, 10, 10, 0, 0).unwrap();
            let identity = ProcessIdentityFact {
                identity_key: "proc-1".into(),
                root_key: "proc-1".into(),
                pid: 1234,
                parent_pid: Some(1),
                platform: "windows".into(),
                start_time_unix_s: 1_760_000_000,
                exe_name: "claude.exe".into(),
                harness: Some("claude".into()),
                cwd_fingerprint: None,
                session_link: None,
                environment_kind: EnvironmentKind::Host,
                isolation_hint: "none".into(),
                first_seen: now,
                last_seen: now,
                ended_at: None,
                visibility_limits: Default::default(),
                attribution: Some("launched".into()),
            };
            let sample = ProcessSampleFact {
                watch_id: "watch-1".into(),
                identity_key: "proc-1".into(),
                seq: 0,
                ts: now,
                cpu_time_us: Some(50_000),
                rss_bytes: Some(64 * 1024 * 1024),
                rss_unit: Unit::WindowsWorkingSetBytes,
                disk_read_bytes: Some(1024),
                disk_write_bytes: Some(512),
                capture_method: CaptureMethod::WindowsProcessApi,
                visibility_limits: Default::default(),
            };
            store
                .commit_process_samples(
                    std::slice::from_ref(&identity),
                    std::slice::from_ref(&sample),
                )
                .unwrap();
            assert_grounding("environment", &store);
        }
    }

    #[test]
    fn mixed_source_denominator_test_excludes_connector_missing_required_field_and_reports_it() {
        let key = b"0123456789abcdef0123456789abcdef";
        let tmp = TempDir::new("mixed");
        let claude_dir = tmp.path().join("claude");
        let codex_dir = tmp.path().join("codex");
        let devin_dir = tmp.path().join("devin");
        std::fs::create_dir_all(&claude_dir).unwrap();
        std::fs::create_dir_all(&codex_dir).unwrap();
        std::fs::create_dir_all(&devin_dir).unwrap();

        write_claude_fixture(&claude_dir);
        write_codex_fixture(&codex_dir);
        let devin_fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("exports")
            .join("devin.jsonl");
        std::fs::copy(&devin_fixture, devin_dir.join("devin.jsonl")).unwrap();

        let roots = vec![
            SourceRoot {
                name: "claude".into(),
                path: claude_dir,
            },
            SourceRoot {
                name: "codex".into(),
                path: codex_dir,
            },
            SourceRoot {
                name: "devin".into(),
                path: devin_dir,
            },
        ];
        let store = ingest_roots_into_store(tmp.path(), &roots, key);

        // 1. Evaluate I01 (model.invocations) over {claude, codex}.
        // Codex lacks MessageId -> excluded from denominator; only claude's 1 call is in the denominator,
        // and codex's usage-only row is NOT pooled into I01.usage_only!
        let results =
            evaluate_catalog_for_connectors(&store, &Scope::default(), &["claude", "codex"]);
        let i01 = results.iter().find(|r| r.id == "I01").unwrap();
        assert_eq!(i01.status, InsightStatus::Computed);
        assert_eq!(
            i01.population.n, 1,
            "codex row must not be pooled into I01 denominator"
        );
        assert!(
            i01.coverage.contains("denominator_included: claude"),
            "coverage must report claude in denominator: {}",
            i01.coverage
        );
        assert!(
            i01.coverage
                .contains("denominator_excluded: codex (missing message_id)"),
            "coverage must report codex excluded with missing message_id: {}",
            i01.coverage
        );
        let distinct_calls = i01
            .measures
            .iter()
            .find(|m| m.id == "I01.distinct_calls")
            .unwrap();
        assert_eq!(distinct_calls.value, Some(MeasureValue::Int(1)));
        let usage_only = i01
            .measures
            .iter()
            .find(|m| m.id == "I01.usage_only")
            .unwrap();
        assert_eq!(usage_only.value, Some(MeasureValue::Int(0)));

        // 2. Evaluate I04 (model.cost) over {claude, codex}.
        // Neither supplies ObservedCost -> NotComputable refusal!
        let i04 = results.iter().find(|r| r.id == "I04").unwrap();
        assert!(
            matches!(
                &i04.status,
                InsightStatus::Refused { reason }
                    if reason.starts_with("not_computable: missing observed_cost")
                        && reason.contains("claude:observed_cost")
                        && reason.contains("codex:observed_cost")
            ),
            "expected not_computable refusal for I04 over {{claude, codex}}, got {:?}",
            i04.status
        );

        // 3. Evaluate I02 (model.tokens.direct) over {claude, codex, devin}.
        // Devin lacks CacheReadTokens and CacheWriteTokens -> excluded from denominator so its -1
        // sentinels are never pooled with claude and codex!
        let results_3 = evaluate_catalog_for_connectors(
            &store,
            &Scope::default(),
            &["claude", "codex", "devin"],
        );
        let i02 = results_3.iter().find(|r| r.id == "I02").unwrap();
        assert_eq!(i02.status, InsightStatus::Computed);
        assert_eq!(
            i02.population.n, 2,
            "only claude and codex rows belong in I02 denominator; devin must be excluded"
        );
        assert!(
            i02.coverage.contains("denominator_included: claude, codex"),
            "coverage must list claude, codex as included: {}",
            i02.coverage
        );
        assert!(
            i02.coverage.contains(
                "denominator_excluded: devin (missing cache_read_tokens+cache_write_tokens)"
            ),
            "coverage must report devin exclusion reason: {}",
            i02.coverage
        );
    }

    #[test]
    fn drift_test_provider_matrix_md_matches_compiled_declarations() {
        let matrix_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("docs")
            .join("providers")
            .join("provider-matrix.md");
        let content = std::fs::read_to_string(&matrix_path)
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", matrix_path.display()))
            .replace("\r\n", "\n");
        let expected = render_provider_fidelity_markdown();
        let begin = "<!-- BEGIN D5_FIELD_FIDELITY -->";
        let end = "<!-- END D5_FIELD_FIDELITY -->";
        let start_idx = content.find(begin).unwrap_or_else(|| {
            panic!("docs/providers/provider-matrix.md is missing `{begin}` marker")
        });
        let end_idx = content.find(end).unwrap_or_else(|| {
            panic!("docs/providers/provider-matrix.md is missing `{end}` marker")
        }) + end.len();
        let actual = &content[start_idx..end_idx];
        assert_eq!(
            actual, expected,
            "docs/providers/provider-matrix.md D5_FIELD_FIDELITY block drifted from compiled declarations"
        );
    }
}
