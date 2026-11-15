//! Connector field-level fidelity inspection and grounding helpers (`dendro#134`).
//!
//! Separate from `stream.rs` so connector fidelity declarations and store-level grounding checks
//! do not require modifying `stream.rs`.

pub use dendro_core::insight_v1::{
    canonical_connector_id, canonical_population_what, connector_declaration, denominator_matrix,
    evaluate_catalog_for_connectors, evaluate_denominator, evaluate_denominator_by_id,
    evaluate_denominator_in_store, evaluate_for_connectors, evaluate_insight_for_connectors,
    format_denominator_disclosure, format_exclusions_list, format_not_computable_reason,
    populated_fields_in_store, present_connectors_for_insight, render_provider_fidelity_markdown,
    token_field_in_store, ConnectorDeclaration, ConnectorExclusion, DenominatorEvaluation,
    DenominatorMatrixRow, DenominatorScope, Field, FieldDeclaration, FieldFidelity,
    IncludedConnector, PartialFidelityReason, CONNECTOR_DECLARATIONS,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture_registry_v1::Registry;
    use crate::environment_v1::{
        detect_isolation, Collector, Config as EnvConfig, ProcessSource, RawMetrics, RawProcess,
    };
    use crate::ingest::ingest_staged;
    use crate::SourceRoot;
    use chrono::{TimeZone, Utc};
    use dendro_core::insight_v1::{
        evaluate, evaluate_catalog, InsightStatus, MeasureValue, Scope, CATALOG,
    };
    use dendro_core::presentation::{attach_dashboard_surface, public_projection};
    use dendro_core::AnalyticsStore;
    use std::collections::BTreeMap;
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

    const TOKEN_FIELDS: [Field; 5] = [
        Field::InputTokens,
        Field::OutputTokens,
        Field::CacheReadTokens,
        Field::CacheWriteTokens,
        Field::ReasoningTokens,
    ];

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
        for (field, reason) in decl.partial_fields() {
            assert!(
                populated.contains(&field),
                "connector `{connector_name}` declares `{field}` Partial({reason:?}) ({}) but positive fixture ingestion did not populate it; populated = {populated:?}",
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

        // Explicitly verify token presence via `token_field_in_store` (`Some(val)` including 0 vs `None`)
        for token_field in TOKEN_FIELDS {
            let stored = token_field_in_store(store, token_field).unwrap();
            match decl.fidelity(token_field) {
                FieldFidelity::Observed | FieldFidelity::Derived | FieldFidelity::Partial(_) => {
                    assert!(
                        stored.is_some(),
                        "connector `{connector_name}` declares `{token_field}` computable, expected Some(tokens) in store, got None"
                    );
                }
                FieldFidelity::Absent => {
                    assert_eq!(
                        stored, None,
                        "connector `{connector_name}` declares `{token_field}` Absent, expected None in store, got {stored:?}"
                    );
                }
            }
        }
    }

    fn write_claude_fixture(dir: &Path) {
        let file = dir.join("session-claude.jsonl");
        // Includes `"reasoning_tokens":99,"cost_usd":0.05` in raw usage so `ReasoningTokens` and
        // `ObservedCost` being `Absent` is proven against a fixture that contains the source fields.
        let lines = [
            r#"{"type":"user","sessionId":"claude-s1","permissionMode":"default","timestamp":"2026-10-10T10:00:00Z","message":{"role":"user","content":"run /commit"}}"#,
            r#"{"type":"assistant","sessionId":"claude-s1","isSidechain":true,"agentId":"agent-child-1","timestamp":"2026-10-10T10:00:01Z","effort":"high","message":{"id":"msg_claude_1","model":"claude-sonnet-4-5","effort":"high","role":"assistant","usage":{"input_tokens":120,"output_tokens":45,"cache_read_input_tokens":30,"cache_creation_input_tokens":15,"reasoning_tokens":99,"cost_usd":0.05},"content":[{"type":"tool_use","id":"toolu_1","name":"Bash","input":{"command":"rm -rf /tmp/test","dangerouslyDisableSandbox":true}},{"type":"tool_use","id":"toolu_2","name":"Skill","input":{"skill":"commit"}}]}}"#,
            r#"{"type":"user","sessionId":"claude-s1","timestamp":"2026-10-10T10:00:02Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_1","is_error":false,"content":"ok"},{"type":"tool_result","tool_use_id":"toolu_2","is_error":false,"content":"ok"}]}}"#,
        ];
        std::fs::write(file, format!("{}\n", lines.join("\n"))).unwrap();
    }

    fn write_codex_fixture(dir: &Path) {
        let file = dir.join("rollout-codex.jsonl");
        // Includes `"cost_usd":0.12` and `"message_id":"codex-msg-1"` in raw payload so `ObservedCost`
        // and `MessageId` being `Absent` are proven against a fixture that contains those source keys.
        let lines = [
            r#"{"timestamp":"2026-10-10T10:00:00Z","type":"session_meta","payload":{"id":"codex-s1","source":{"subagent":{"thread_spawn":{"parent_thread_id":"codex-parent-0","depth":1}}}}}"#,
            r#"{"timestamp":"2026-10-10T10:00:01Z","type":"turn_context","payload":{"model":"gpt-5-codex","approval_policy":"on-request","sandbox_policy":{"type":"workspace-write"},"collaboration_mode":{"mode":"default","settings":{"model":"gpt-5-codex","reasoning_effort":"high"}}}}"#,
            r#"{"timestamp":"2026-10-10T10:00:02Z","type":"event_msg","payload":{"type":"user_message","message":"Use $review-skill now"}}"#,
            r#"{"timestamp":"2026-10-10T10:00:03Z","type":"response_item","payload":{"type":"function_call","name":"exec_command","call_id":"call_codex_1","arguments":"{\"cmd\":\"rm -rf /tmp/test && cat /skills/review-skill/SKILL.md\",\"dangerouslyDisableSandbox\":true}"}}"#,
            r#"{"timestamp":"2026-10-10T10:00:04Z","type":"response_item","payload":{"type":"function_call_output","call_id":"call_codex_1","output":"Process exited with code 0\nOutput:\nok"}}"#,
            r#"{"timestamp":"2026-10-10T10:00:05Z","type":"event_msg","payload":{"type":"token_count","message_id":"codex-msg-1","cost_usd":0.12,"info":{"total_token_usage":{"input_tokens":200,"cached_input_tokens":50,"cache_write_input_tokens":20,"output_tokens":80,"reasoning_output_tokens":25,"total_tokens":280,"cost_usd":0.12},"last_token_usage":{"input_tokens":200,"cached_input_tokens":50,"cache_write_input_tokens":20,"output_tokens":80,"reasoning_output_tokens":25,"total_tokens":280,"cost_usd":0.12}}}}"#,
        ];
        std::fs::write(file, format!("{}\n", lines.join("\n"))).unwrap();
    }

    fn write_pi_fixture(dir: &Path) {
        let file = dir.join("session-pi.jsonl");
        // Includes `"reasoning":12`, `"parentSessionId":"pi-parent-0"`, `"permissionMode":"default"`,
        // and `"dangerouslyDisableSandbox":true` so Absent fields (`ReasoningTokens`, `SessionEdges`,
        // `SecurityPolicy`, `SecurityCommandCheck`) are proven against present source fields.
        let lines = [
            r#"{"type":"session","id":"pi-s1","parentSessionId":"pi-parent-0","permissionMode":"default","timestamp":"2026-10-10T10:00:00Z","cwd":"/tmp"}"#,
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
            r#"{"method":"session/update","timestamp":1760090403,"params":{"sessionId":"grok-s1","update":{"sessionUpdate":"turn_completed","messageId":"grok-msg-ignored","usage":{"inputTokens":180,"outputTokens":60,"cachedReadTokens":30,"cacheCreationTokens":10,"reasoningTokens":15,"costUsdTicks":42000000}}}}"#,
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
        }
    }

    #[test]
    fn token_presence_distinguishes_written_zero_from_none() {
        let key = b"0123456789abcdef0123456789abcdef";
        let tmp = TempDir::new("claude-zero-tokens");
        let data_dir = tmp.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        // Claude writes real 0s for input, output, cache_read, and cache_write when the JSON has 0,
        // while reasoning_tokens is Absent (stored as -1 sentinel -> None).
        let line = r#"{"type":"assistant","sessionId":"claude-zero","timestamp":"2026-10-10T10:00:01Z","message":{"id":"msg_zero","model":"claude-sonnet-4-5","role":"assistant","usage":{"input_tokens":0,"output_tokens":0,"cache_read_input_tokens":0,"cache_creation_input_tokens":0,"reasoning_tokens":0},"content":[{"type":"text","text":"zero"}]}}"#;
        std::fs::write(data_dir.join("session-zero.jsonl"), format!("{line}\n")).unwrap();
        let root = SourceRoot {
            name: "claude".into(),
            path: data_dir,
        };
        let store = ingest_roots_into_store(tmp.path(), &[root], key);

        assert_eq!(
            token_field_in_store(&store, Field::InputTokens).unwrap(),
            Some(0)
        );
        assert_eq!(
            token_field_in_store(&store, Field::OutputTokens).unwrap(),
            Some(0)
        );
        assert_eq!(
            token_field_in_store(&store, Field::CacheReadTokens).unwrap(),
            Some(0)
        );
        assert_eq!(
            token_field_in_store(&store, Field::CacheWriteTokens).unwrap(),
            Some(0)
        );
        assert_eq!(
            token_field_in_store(&store, Field::ReasoningTokens).unwrap(),
            None
        );

        let populated = populated_fields_in_store(&store).unwrap();
        assert!(populated.contains(&Field::InputTokens));
        assert!(populated.contains(&Field::OutputTokens));
        assert!(populated.contains(&Field::CacheReadTokens));
        assert!(populated.contains(&Field::CacheWriteTokens));
        assert!(!populated.contains(&Field::ReasoningTokens));
    }

    #[test]
    fn partial_fields_negative_when_condition_not_met() {
        let key = b"0123456789abcdef0123456789abcdef";

        // 1. Claude without effort field -> Effort (Partial) not populated
        {
            let tmp = TempDir::new("claude-no-effort");
            let data_dir = tmp.path().join("data");
            std::fs::create_dir_all(&data_dir).unwrap();
            let line = r#"{"type":"assistant","sessionId":"claude-ne","timestamp":"2026-10-10T10:00:01Z","message":{"id":"msg_ne","model":"claude-sonnet-4-5","role":"assistant","usage":{"input_tokens":10,"output_tokens":5,"cache_read_input_tokens":0,"cache_creation_input_tokens":0},"content":[{"type":"text","text":"hi"}]}}"#;
            std::fs::write(data_dir.join("session-ne.jsonl"), format!("{line}\n")).unwrap();
            let root = SourceRoot {
                name: "claude".into(),
                path: data_dir,
            };
            let store = ingest_roots_into_store(tmp.path(), &[root], key);
            let populated = populated_fields_in_store(&store).unwrap();
            assert!(!populated.contains(&Field::Effort));
        }

        // 2. Codex without exit_code in function_call_output -> ToolOutcome (Partial) not populated
        {
            let tmp = TempDir::new("codex-no-exit");
            let data_dir = tmp.path().join("data");
            std::fs::create_dir_all(&data_dir).unwrap();
            let lines = [
                r#"{"timestamp":"2026-10-10T10:00:00Z","type":"session_meta","payload":{"id":"codex-ne"}}"#,
                r#"{"timestamp":"2026-10-10T10:00:01Z","type":"response_item","payload":{"type":"function_call","name":"exec_command","call_id":"call_1","arguments":"{\"cmd\":\"echo ok\"}"}}"#,
                r#"{"timestamp":"2026-10-10T10:00:02Z","type":"response_item","payload":{"type":"function_call_output","call_id":"call_1","output":"plain text without exit marker"}}"#,
            ];
            std::fs::write(
                data_dir.join("rollout-ne.jsonl"),
                format!("{}\n", lines.join("\n")),
            )
            .unwrap();
            let root = SourceRoot {
                name: "codex".into(),
                path: data_dir,
            };
            let store = ingest_roots_into_store(tmp.path(), &[root], key);
            let populated = populated_fields_in_store(&store).unwrap();
            assert!(!populated.contains(&Field::ToolOutcome));
        }

        // 3. Grok without costUsdTicks -> ObservedCost (Partial) not populated
        {
            let tmp = TempDir::new("grok-no-cost");
            let data_dir = tmp.path().join("data");
            std::fs::create_dir_all(&data_dir).unwrap();
            let summary = serde_json::json!({
                "current_model_id": "grok-4.7-build-fast",
                "reasoning_effort": "high",
                "chat_format_version": 1
            });
            std::fs::write(
                data_dir.join("summary.json"),
                serde_json::to_string_pretty(&summary).unwrap(),
            )
            .unwrap();
            let line = r#"{"method":"session/update","timestamp":1760090403,"params":{"sessionId":"grok-nc","update":{"sessionUpdate":"turn_completed","usage":{"inputTokens":180,"outputTokens":60,"cachedReadTokens":30,"cacheCreationTokens":10,"reasoningTokens":15}}}}"#;
            std::fs::write(data_dir.join("updates.jsonl"), format!("{line}\n")).unwrap();
            let root = SourceRoot {
                name: "grok".into(),
                path: data_dir,
            };
            let store = ingest_roots_into_store(tmp.path(), &[root], key);
            let populated = populated_fields_in_store(&store).unwrap();
            assert!(!populated.contains(&Field::ObservedCost));
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
            let mut content = std::fs::read_to_string(&dest).unwrap();
            match *name {
                "cursor" => {
                    // Add completed tool call for Partial(ExplicitSignalOnly) ToolOutcome AND extra
                    // usage/cost/effort/message_id fields to prove Absent fields are not ingested.
                    content.push_str(
                        "{\"type\":\"tool_call\",\"subtype\":\"completed\",\"session_id\":\"synthetic-cursor\",\"call_id\":\"call-2\",\"timestamp_ms\":1767225601000,\"message_id\":\"cursor-m1\",\"effort\":\"high\",\"cost_usd\":0.02,\"usage\":{\"input_tokens\":100,\"output_tokens\":50,\"cache_read_tokens\":10,\"cache_write_tokens\":5,\"reasoning_tokens\":5},\"tool_call\":{\"readToolCall\":{\"args\":{\"path\":\"a.txt\"},\"result\":{\"success\":{}}}}}\n",
                    );
                }
                "windsurf" => {
                    // Include usage/cost/effort/message_id in raw Windsurf JSONL to prove Absent fields are ignored.
                    content.push_str(
                        "{\"type\":\"tool_call\",\"status\":\"done\",\"session_id\":\"synthetic-windsurf\",\"timestamp\":\"2026-01-01T00:00:05Z\",\"model\":\"claude-3-7-sonnet\",\"message_id\":\"ws-m1\",\"effort\":\"high\",\"cost_usd\":0.03,\"usage\":{\"input_tokens\":90,\"output_tokens\":40,\"cache_read_tokens\":10,\"cache_write_tokens\":5,\"reasoning_tokens\":5},\"tool\":{\"name\":\"read_file\",\"arguments\":{\"path\":\"b.txt\"}}}\n",
                    );
                }
                "goose" => {
                    // Include cache/reasoning/cost/model/effort/message_id in raw Goose JSONL so Absent is proven.
                    content.push_str(
                        "{\"session_id\":\"synthetic-goose\",\"ts\":\"2026-01-01T00:00:05Z\",\"message_id\":\"goose-m1\",\"model\":\"gpt-4o\",\"effort\":\"high\",\"cost_usd\":0.04,\"input_tokens\":50,\"output_tokens\":25,\"cache_read_tokens\":20,\"cache_write_tokens\":10,\"reasoning_tokens\":5,\"tool_calls\":[{\"name\":\"shell\",\"status\":\"success\",\"arguments\":{\"cmd\":\"ls\"}}]}\n",
                    );
                }
                "devin" => {
                    // Include cached_tokens, cache_read_tokens, cache_write_tokens, reasoning_tokens, effort, message_id
                    // in raw Devin JSONL so Absent is proven against present source fields.
                    content.push_str(
                        "{\"session_id\":\"synthetic-devin\",\"timestamp\":\"2026-01-01T00:00:05Z\",\"message_id\":\"devin-m1\",\"model\":\"devin-default\",\"effort\":\"high\",\"input_tokens\":80,\"output_tokens\":30,\"cached_tokens\":40,\"cache_read_tokens\":40,\"cache_write_tokens\":15,\"reasoning_tokens\":10,\"cost_usd\":0.05,\"tool_calls\":[{\"name\":\"bash\",\"status\":\"completed\",\"args\":{\"cmd\":\"pwd\"}}]}\n",
                    );
                }
                _ => {}
            }
            std::fs::write(&dest, content).unwrap();
            let root = SourceRoot {
                name: (*name).to_string(),
                path: data_dir,
            };
            let store = ingest_roots_into_store(tmp.path(), &[root], key);
            assert_grounding(name, &store);
        }
    }

    struct FixtureProcessSource;

    impl ProcessSource for FixtureProcessSource {
        fn scan(&mut self) -> Vec<RawProcess> {
            vec![RawProcess {
                pid: 1234,
                parent: Some(1),
                start_s: 1_760_000_000,
                name: "claude.exe".into(),
                own: true,
            }]
        }

        fn metrics(&mut self, pids: &[u32]) -> BTreeMap<u32, RawMetrics> {
            pids.iter()
                .map(|&pid| {
                    (
                        pid,
                        RawMetrics {
                            cpu_time_us: Some(50_000),
                            rss_bytes: Some(64 * 1024 * 1024),
                            disk_read_bytes: Some(1024),
                            disk_write_bytes: Some(512),
                            limits: Default::default(),
                        },
                    )
                })
                .collect()
        }

        fn cwd(&mut self, _pid: u32) -> Option<PathBuf> {
            Some(PathBuf::from("/tmp/work"))
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

        // 3. Environment (`environment_v1::Collector` + `Batch::seal` + `commit_process_samples`)
        {
            let store = AnalyticsStore::open_in_memory("ground-environment").unwrap();
            let now = Utc.with_ymd_and_hms(2026, 10, 10, 10, 0, 0).unwrap();
            let mut collector = Collector::new(
                FixtureProcessSource,
                EnvConfig::default(),
                detect_isolation(),
            );
            collector.tick(now);
            let sealed = collector.drain().seal(key, &[]);
            assert_eq!(sealed.identities.len(), 1);
            assert_eq!(sealed.samples.len(), 1);
            store
                .commit_process_samples(&sealed.identities, &sealed.samples)
                .unwrap();
            assert_grounding("environment", &store);
        }
    }

    #[test]
    fn denominator_policy_user_facing_evaluate_scoped_and_unscoped_with_site_report_json() {
        let key = b"0123456789abcdef0123456789abcdef";
        let tmp = TempDir::new("policy-claude-goose");
        let claude_dir = tmp.path().join("claude");
        let goose_dir = tmp.path().join("goose");
        std::fs::create_dir_all(&claude_dir).unwrap();
        std::fs::create_dir_all(&goose_dir).unwrap();

        write_claude_fixture(&claude_dir);
        let goose_fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("exports")
            .join("goose.jsonl");
        std::fs::copy(&goose_fixture, goose_dir.join("goose.jsonl")).unwrap();

        let roots = vec![
            SourceRoot {
                name: "claude".into(),
                path: claude_dir,
            },
            SourceRoot {
                name: "goose".into(),
                path: goose_dir,
            },
        ];
        let store = ingest_roots_into_store(tmp.path(), &roots, key);

        // 1. `--harness goose` on I02 -> NotComputable (cache fields Absent), preserving original coverage
        // and appending the refusal and exclusion info.
        let goose_scope = Scope {
            harness: Some("goose".into()),
            ..Scope::default()
        };
        let scoped_results = evaluate(&store, &goose_scope);
        let i02_goose = scoped_results.iter().find(|r| r.id == "I02").unwrap();
        assert!(
            matches!(
                &i02_goose.status,
                InsightStatus::Refused { reason }
                    if reason == "not_computable: missing cache_read_tokens, cache_write_tokens (goose:cache_read_tokens+cache_write_tokens)"
            ),
            "expected not_computable refusal for `--harness goose` on I02, got {:?}",
            i02_goose.status
        );
        assert!(
            i02_goose.measures.is_empty(),
            "NotComputable result must never emit a number"
        );
        assert!(
            i02_goose
                .coverage
                .starts_with("reasoning is included in output and is not part of the disjoint sum"),
            "original coverage text must be preserved: {}",
            i02_goose.coverage
        );
        assert!(
            i02_goose
                .coverage
                .contains("excluded: goose — missing cache_read_tokens, cache_write_tokens"),
            "coverage must append exclusion info: {}",
            i02_goose.coverage
        );

        // 2. Unscoped claude+goose store -> I02 computed over claude only, with goose listed as excluded.
        let unscoped_results = evaluate(&store, &Scope::default());
        let i02_unscoped = unscoped_results.iter().find(|r| r.id == "I02").unwrap();
        assert_eq!(i02_unscoped.status, InsightStatus::Computed);
        assert_eq!(
            i02_unscoped.population.n, 1,
            "I02 must be computed over claude only; goose rows must be excluded"
        );
        assert!(
            i02_unscoped
                .coverage
                .starts_with("reasoning is included in output and is not part of the disjoint sum"),
            "original coverage text must be kept: {}",
            i02_unscoped.coverage
        );
        assert!(
            i02_unscoped
                .coverage
                .contains("excluded: goose — missing cache_read_tokens, cache_write_tokens"),
            "unscoped coverage must list goose as excluded: {}",
            i02_unscoped.coverage
        );
        let input_tokens = i02_unscoped
            .measures
            .iter()
            .find(|m| m.id == "I02.claude-sonnet-4-5.input_uncached")
            .unwrap();
        assert_eq!(input_tokens.value, Some(MeasureValue::Int(120)));

        // 3. Site/report JSON carries the exclusion (`attach_dashboard_surface` + `public_projection`).
        let mut report = serde_json::json!({ "status": "ok", "payload": {} });
        attach_dashboard_surface(&mut report, &store, "", "").unwrap();
        let public = public_projection(&report);
        let insights_json = public
            .get("payload")
            .and_then(|p| p.get("insights"))
            .and_then(|v| v.as_array())
            .expect("public report payload must contain insights array");
        let i02_json = insights_json
            .iter()
            .find(|item| item.get("id").and_then(|v| v.as_str()) == Some("I02"))
            .expect("I02 must be in public report insights");
        let cov_str = i02_json
            .get("coverage")
            .and_then(|v| v.as_str())
            .expect("public report I02 must carry coverage");
        assert!(
            cov_str.contains("excluded: goose — missing cache_read_tokens, cache_write_tokens"),
            "public report I02 coverage must carry exclusion: {cov_str}"
        );
        let l3 = i02_unscoped.disclosure_l3();
        let l3_exclusions = l3
            .fields
            .iter()
            .find(|(k, _)| *k == "exclusions")
            .map(|(_, v)| v.as_str())
            .expect("I02 disclosure_l3.exclusions must be present");
        assert!(
            l3_exclusions
                .contains("excluded: goose — missing cache_read_tokens, cache_write_tokens"),
            "I02 disclosure_l3.exclusions must carry exclusion: {l3_exclusions}"
        );
    }

    #[test]
    fn regression_guard_byte_identical_when_every_present_connector_is_computable() {
        let key = b"0123456789abcdef0123456789abcdef";
        let tmp = TempDir::new("byte-identical");
        let claude_dir = tmp.path().join("claude");
        let codex_dir = tmp.path().join("codex");
        std::fs::create_dir_all(&claude_dir).unwrap();
        std::fs::create_dir_all(&codex_dir).unwrap();

        write_claude_fixture(&claude_dir);
        write_codex_fixture(&codex_dir);

        let roots = vec![
            SourceRoot {
                name: "claude".into(),
                path: claude_dir,
            },
            SourceRoot {
                name: "codex".into(),
                path: codex_dir,
            },
        ];
        let store = ingest_roots_into_store(tmp.path(), &roots, key);
        let scope = Scope::default();

        // For every insight where both claude and codex are Computable (I01, I02, I03, I05, I08, I09, I11, I16, I17, I18, I21, I24, I25),
        // the output of `evaluate_catalog` must be byte-identical to `evaluate_catalog_for_connectors(&store, &scope, &["claude", "codex"])`
        // with zero exclusion annotations appended to coverage.
        let catalog_results = evaluate_catalog(&store, &scope);
        let explicit_results =
            evaluate_catalog_for_connectors(&store, &scope, &["claude", "codex"]);
        for insight_id in [
            "I01", "I02", "I03", "I05", "I08", "I09", "I11", "I16", "I17", "I18", "I21", "I24",
            "I25",
        ] {
            let insight = CATALOG
                .iter()
                .copied()
                .find(|i| i.id() == insight_id)
                .unwrap();
            let actual = catalog_results.iter().find(|r| r.id == insight_id).unwrap();
            let explicit = explicit_results
                .iter()
                .find(|r| r.id == insight_id)
                .unwrap();
            assert_eq!(
                serde_json::to_string(actual).unwrap(),
                serde_json::to_string(explicit).unwrap(),
                "byte-identical regression guard failed for {insight_id}"
            );
            assert_eq!(
                actual.coverage,
                insight.coverage(),
                "coverage for {insight_id} must be byte-identical to original coverage when all present connectors are computable"
            );
        }

        // Also verify REFERENCE insights I02, I08, I09, I18 via `evaluate` are byte-identical to `evaluate_for_connectors`.
        let ref_results = evaluate(&store, &scope);
        let ref_explicit = evaluate_for_connectors(&store, &scope, &["claude", "codex"]);
        for insight_id in ["I02", "I08", "I09", "I18"] {
            let actual = ref_results.iter().find(|r| r.id == insight_id).unwrap();
            let explicit = ref_explicit.iter().find(|r| r.id == insight_id).unwrap();
            assert_eq!(
                serde_json::to_string(actual).unwrap(),
                serde_json::to_string(explicit).unwrap(),
                "REFERENCE byte-identical regression guard failed for {insight_id}"
            );
        }
    }

    #[test]
    fn allow_list_excludes_unlisted_connector_rows_from_computation_values() {
        let key = b"0123456789abcdef0123456789abcdef";
        let tmp = TempDir::new("allow-list-mixed");
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

        // Store has claude + codex + devin, but we evaluate over `{claude, codex}` only.
        // Because `allows_harness` is an allow-list built from the included connectors `{claude, codex}`,
        // devin rows must NEVER enter the computation of I01 (even though devin is Computable for I01!)
        // or I02.
        let results =
            evaluate_catalog_for_connectors(&store, &Scope::default(), &["claude", "codex"]);

        // Check I01 values: claude (1 distinct call) + codex (1 usage_only row) = population 2,
        // whereas devin's usage_only row is excluded by the `{claude, codex}` allow-list!
        let i01 = results.iter().find(|r| r.id == "I01").unwrap();
        assert_eq!(i01.status, InsightStatus::Computed);
        assert_eq!(
            i01.population.n, 2,
            "devin row must not enter I01 when evaluated over {{claude, codex}}"
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
        assert_eq!(
            usage_only.value,
            Some(MeasureValue::Int(1)),
            "only codex's 1 usage_only row may be counted; devin's row must be excluded"
        );

        // Check I02 values: claude input_uncached (120) + codex input_uncached (130 = 200 - 50 - 20).
        // Devin's `devin-default` model rows must not be pooled into I02!
        let i02 = results.iter().find(|r| r.id == "I02").unwrap();
        assert_eq!(i02.status, InsightStatus::Computed);
        assert_eq!(i02.population.n, 2);
        let claude_input = i02
            .measures
            .iter()
            .find(|m| m.id == "I02.claude-sonnet-4-5.input_uncached")
            .unwrap();
        assert_eq!(claude_input.value, Some(MeasureValue::Int(120)));
        let codex_input = i02
            .measures
            .iter()
            .find(|m| m.id == "I02.gpt-5-codex.input_uncached")
            .unwrap();
        assert_eq!(codex_input.value, Some(MeasureValue::Int(130)));
        assert!(
            !i02.measures.iter().any(|m| m.id.contains("devin")),
            "devin measures must not appear in I02 when excluded by allow-list"
        );

        // Unscoped `evaluate_catalog(&store, &Scope::default())` on `{claude, codex, devin}`:
        // - For I01: all three are Computable (MessageId is optional), so population.n == 3 and usage_only == 2.
        // - For I02: devin is excluded (missing cache_read_tokens, cache_write_tokens), so population.n == 2 and input_tokens == 250.
        let unscoped = evaluate_catalog(&store, &Scope::default());
        let i01_all = unscoped.iter().find(|r| r.id == "I01").unwrap();
        assert_eq!(i01_all.population.n, 3);
        let usage_only_all = i01_all
            .measures
            .iter()
            .find(|m| m.id == "I01.usage_only")
            .unwrap();
        assert_eq!(usage_only_all.value, Some(MeasureValue::Int(2)));

        let i02_all = unscoped.iter().find(|r| r.id == "I02").unwrap();
        assert_eq!(i02_all.population.n, 2);
        assert!(
            i02_all
                .coverage
                .contains("excluded: devin — missing cache_read_tokens, cache_write_tokens"),
            "I02 coverage must report devin excluded: {}",
            i02_all.coverage
        );
    }

    #[test]
    fn ingested_claude_vs_claude_plus_goose_excludes_goose_across_insights() {
        let key = b"0123456789abcdef0123456789abcdef";
        let tmp_only = TempDir::new("table-family-claude-only");
        let claude_only_dir = tmp_only.path().join("claude");
        std::fs::create_dir_all(&claude_only_dir).unwrap();
        write_claude_fixture(&claude_only_dir);
        let store_only = ingest_roots_into_store(
            tmp_only.path(),
            &[SourceRoot {
                name: "claude".into(),
                path: claude_only_dir,
            }],
            key,
        );

        let tmp_mixed = TempDir::new("table-family-claude-goose");
        let claude_dir = tmp_mixed.path().join("claude");
        let goose_dir = tmp_mixed.path().join("goose");
        std::fs::create_dir_all(&claude_dir).unwrap();
        std::fs::create_dir_all(&goose_dir).unwrap();
        write_claude_fixture(&claude_dir);
        let goose_fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("exports")
            .join("goose.jsonl");
        std::fs::copy(&goose_fixture, goose_dir.join("goose.jsonl")).unwrap();
        let store_mixed = ingest_roots_into_store(
            tmp_mixed.path(),
            &[
                SourceRoot {
                    name: "claude".into(),
                    path: claude_dir,
                },
                SourceRoot {
                    name: "goose".into(),
                    path: goose_dir,
                },
            ],
            key,
        );

        let only_results = evaluate_catalog(&store_only, &Scope::default());
        let mixed_results = evaluate_catalog(&store_mixed, &Scope::default());

        // For every insight where claude is Computable, goose is NotComputable, AND goose has
        // fact rows in scope (`usage_facts` or `tool_facts`: `I02`, `I03`, `I05`, `I08`, `I11`, `I21`, `I25`),
        // unscoped `evaluate_catalog` on `claude + goose` must exclude `goose`, disclose the exclusion,
        // and produce the exact same status, population, and measure values as `claude` alone.
        for id in ["I02", "I03", "I05", "I08", "I11", "I21", "I25"] {
            let only = only_results.iter().find(|r| r.id == id).unwrap();
            let mixed = mixed_results.iter().find(|r| r.id == id).unwrap();
            assert_eq!(
                only.status, mixed.status,
                "goose rows must not affect {id} status"
            );
            assert_eq!(
                only.population.n, mixed.population.n,
                "goose rows must be excluded from {id} population"
            );
            assert_eq!(
                only.measures, mixed.measures,
                "goose rows must not affect {id} measures"
            );
            assert!(
                mixed.coverage.contains("excluded: goose — missing"),
                "{id} coverage must disclose goose exclusion: {}",
                mixed.coverage
            );
        }

        // Decision E: for insights whose fact tables (`session_edges`, `security_facts`) have no
        // goose rows (`I09`, `I16`, `I17`, `I24`), `source_files` alone does not count for presence,
        // so `mixed` has the exact same status, population, measures, and coverage (no goose exclusion).
        for id in ["I09", "I16", "I17", "I24"] {
            let only = only_results.iter().find(|r| r.id == id).unwrap();
            let mixed = mixed_results.iter().find(|r| r.id == id).unwrap();
            assert_eq!(only.status, mixed.status, "{id} status");
            assert_eq!(only.population, mixed.population, "{id} population");
            assert_eq!(only.measures, mixed.measures, "{id} measures");
            assert_eq!(
                only.coverage, mixed.coverage,
                "goose has no fact rows in {id}'s tables; source_files alone must not mark goose present in coverage"
            );
        }
    }

    #[test]
    fn ingested_claude_only_i04_is_not_computable_with_reason() {
        let key = b"0123456789abcdef0123456789abcdef";
        let tmp = TempDir::new("claude-only-i04");
        let claude_dir = tmp.path().join("claude");
        std::fs::create_dir_all(&claude_dir).unwrap();
        write_claude_fixture(&claude_dir);
        let store = ingest_roots_into_store(
            tmp.path(),
            &[SourceRoot {
                name: "claude".into(),
                path: claude_dir,
            }],
            key,
        );

        let ref_i04 = evaluate(&store, &Scope::default())
            .into_iter()
            .find(|r| r.id == "I04")
            .unwrap();
        assert_eq!(
            ref_i04.status,
            InsightStatus::Refused {
                reason: "not_computable: missing observed_cost (claude:observed_cost)".to_string(),
            }
        );
        assert!(ref_i04.measures.is_empty());
        assert!(
            ref_i04
                .coverage
                .contains("not_computable: missing observed_cost (claude:observed_cost)")
                && ref_i04
                    .coverage
                    .contains("excluded: claude — missing observed_cost"),
            "I04 coverage on ingested claude-only store must show reason and exclusion: {}",
            ref_i04.coverage
        );

        let cat_i04 = evaluate_catalog(&store, &Scope::default())
            .into_iter()
            .find(|r| r.id == "I04")
            .unwrap();
        assert_eq!(ref_i04, cat_i04);
    }

    #[test]
    fn pre_d5_golden_regression_guard_ingested_claude_codex_pi() {
        let key = b"0123456789abcdef0123456789abcdef";
        let tmp = TempDir::new("pre-d5-golden");
        let roots = vec![
            SourceRoot {
                name: "claude".into(),
                path: PathBuf::from("tests/fixtures/d5_golden/sources/claude"),
            },
            SourceRoot {
                name: "codex".into(),
                path: PathBuf::from("tests/fixtures/d5_golden/sources/codex"),
            },
            SourceRoot {
                name: "pi".into(),
                path: PathBuf::from("tests/fixtures/d5_golden/sources/pi"),
            },
        ];
        let store = ingest_roots_into_store(tmp.path(), &roots, key);
        let scope = Scope::default();

        let golden_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("d5_golden");
        let golden_eval_raw =
            std::fs::read_to_string(golden_dir.join("pre_d5_evaluate.json")).unwrap();
        let golden_cat_raw =
            std::fs::read_to_string(golden_dir.join("pre_d5_evaluate_catalog.json")).unwrap();
        let golden_eval: Vec<serde_json::Value> = serde_json::from_str(&golden_eval_raw).unwrap();
        let golden_cat: Vec<serde_json::Value> = serde_json::from_str(&golden_cat_raw).unwrap();

        let actual_eval = evaluate(&store, &scope);
        let actual_cat = evaluate_catalog(&store, &scope);

        // Explicitly declared allow-list of insight IDs whose output legitimately changes under D5
        // on the `{claude, codex, pi}` ingested fixture store, with the exact policy reason:
        let expected_policy_diffs: BTreeMap<&str, &str> = BTreeMap::from([
            (
                "I03",
                "pi is present in usage_facts but lacks session_edges; excluded from I03 denominator (population 3 -> 2) and disclosed in coverage",
            ),
            (
                "I04",
                "claude and codex are present in usage_facts but lack observed_cost; excluded from I04 denominator (population 3 -> 1, pi only) and disclosed in coverage",
            ),
            (
                "I06",
                "claude, codex, and pi are present in tool_facts/skill_facts but none supplies skill_declarations; I06 refuses as NotComputable instead ofPartial",
            ),
            (
                "I11",
                "pi is present in usage_facts but lacks session_edges; excluded from I11 denominator (population 3 -> 2) and disclosed in coverage",
            ),
            (
                "I16",
                "pi is present in security_facts (via SecurityCommandCheck) but lacks SecurityPolicy; excluded from I16 denominator and disclosed in coverage",
            ),
            (
                "I17",
                "pi is present in security_facts (via SecurityCommandCheck) but lacks SecurityPolicy; excluded from I17 denominator and disclosed in coverage",
            ),
            (
                "I20",
                "pi is present in usage_facts with currency_total but lacks queue_transitions; I20 refuses as NotComputable instead of not_observable",
            ),
            (
                "I24",
                "pi is present in security_facts (via SecurityCommandCheck) but lacks SessionEdges and SecurityPolicy; excluded from I24 denominator and disclosed in coverage",
            ),
        ]);

        // 1. Check `evaluate_catalog` (all 25 insights I01..I25) against `pre_d5_evaluate_catalog.json`
        assert_eq!(actual_cat.len(), golden_cat.len());
        for (actual, golden) in actual_cat.iter().zip(golden_cat.iter()) {
            let id = actual.id;
            assert_eq!(golden["id"].as_str(), Some(id));
            let actual_val = serde_json::to_value(actual).unwrap();

            let insight = CATALOG.iter().copied().find(|i| i.id() == id).unwrap();
            let present = present_connectors_for_insight(insight, &store);
            let all_present_computable = present.is_empty()
                || matches!(
                    evaluate_denominator(insight, &present),
                    DenominatorEvaluation::Computable { ref scope } if scope.excluded.is_empty()
                );

            if all_present_computable {
                assert!(
                    !expected_policy_diffs.contains_key(id),
                    "{id} has all present connectors computable ({present:?}), so it must not be in expected_policy_diffs"
                );
                assert_eq!(
                    serde_json::to_string_pretty(&actual_val).unwrap(),
                    serde_json::to_string_pretty(golden).unwrap(),
                    "byte-identical PRE-D5 golden regression guard failed for {id} (present={present:?})"
                );
            } else {
                let reason = expected_policy_diffs.get(id).unwrap_or_else(|| {
                    panic!("{id} changed from pre-D5 golden output without an explicit entry in expected_policy_diffs")
                });
                assert!(
                    !reason.is_empty(),
                    "expected_policy_diffs reason for {id} must not be empty"
                );
                assert_ne!(
                    serde_json::to_string_pretty(&actual_val).unwrap(),
                    serde_json::to_string_pretty(golden).unwrap(),
                    "{id} is listed in expected_policy_diffs ({reason}) but did not differ from pre-D5 golden output"
                );
                match id {
                    "I03" | "I11" => {
                        assert_eq!(actual.population.n, 2);
                        assert!(actual
                            .coverage
                            .contains("excluded: pi — missing session_edges"));
                    }
                    "I04" => {
                        assert_eq!(actual.population.n, 1);
                        assert!(actual
                            .coverage
                            .contains("excluded: claude — missing observed_cost; excluded: codex — missing observed_cost"));
                    }
                    "I06" => {
                        assert!(
                            matches!(&actual.status, InsightStatus::Refused { reason } if reason.starts_with("not_computable: missing skill_declarations"))
                        );
                    }
                    "I16" | "I17" => {
                        assert!(actual
                            .coverage
                            .contains("excluded: pi — missing security_policy"));
                    }
                    "I20" => {
                        assert!(
                            matches!(&actual.status, InsightStatus::Refused { reason } if reason == "not_computable: missing queue_transitions (pi:queue_transitions)")
                        );
                    }
                    "I24" => {
                        assert!(actual
                            .coverage
                            .contains("excluded: pi — missing session_edges, security_policy"));
                    }
                    other => panic!("unexpected id in expected_policy_diffs: {other}"),
                }
            }
        }

        // 2. Check `evaluate` (REFERENCE: I02, I04, I08, I09, I14, I18) against `pre_d5_evaluate.json`
        assert_eq!(actual_eval.len(), golden_eval.len());
        for (actual, golden) in actual_eval.iter().zip(golden_eval.iter()) {
            let id = actual.id;
            let actual_val = serde_json::to_value(actual).unwrap();
            if id == "I04" {
                assert_eq!(actual.population.n, 1);
                assert!(actual
                    .coverage
                    .contains("excluded: claude — missing observed_cost; excluded: codex — missing observed_cost"));
            } else {
                assert_eq!(
                    serde_json::to_string_pretty(&actual_val).unwrap(),
                    serde_json::to_string_pretty(golden).unwrap(),
                    "REFERENCE byte-identical PRE-D5 golden regression guard failed for {id}"
                );
            }
        }
    }

    #[test]
    fn pre_d5_golden_regression_guard_ingested_claude_codex_pi_windowed() {
        let key = b"0123456789abcdef0123456789abcdef";
        let tmp = TempDir::new("pre-d5-golden-windowed");
        let roots = vec![
            SourceRoot {
                name: "claude".into(),
                path: PathBuf::from("tests/fixtures/d5_golden/sources/claude"),
            },
            SourceRoot {
                name: "codex".into(),
                path: PathBuf::from("tests/fixtures/d5_golden/sources/codex"),
            },
            SourceRoot {
                name: "pi".into(),
                path: PathBuf::from("tests/fixtures/d5_golden/sources/pi"),
            },
        ];
        let store = ingest_roots_into_store(tmp.path(), &roots, key);
        let scope = Scope {
            from: Some("2026-10-10T00:00:00Z".into()),
            to: Some("2026-10-11T00:00:00Z".into()),
            ..Scope::default()
        };

        let golden_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("d5_golden");
        let golden_eval_raw =
            std::fs::read_to_string(golden_dir.join("pre_d5_evaluate_windowed.json")).unwrap();
        let golden_cat_raw =
            std::fs::read_to_string(golden_dir.join("pre_d5_evaluate_catalog_windowed.json"))
                .unwrap();
        let golden_eval: Vec<serde_json::Value> = serde_json::from_str(&golden_eval_raw).unwrap();
        let golden_cat: Vec<serde_json::Value> = serde_json::from_str(&golden_cat_raw).unwrap();

        let actual_eval = evaluate(&store, &scope);
        let actual_cat = evaluate_catalog(&store, &scope);

        let expected_policy_diffs: BTreeMap<&str, &str> = BTreeMap::from([
            (
                "I03",
                "pi is present in usage_facts in window but lacks session_edges; excluded from I03 denominator and disclosed in coverage",
            ),
            (
                "I04",
                "claude and codex are present in usage_facts in window but lack observed_cost; excluded from I04 denominator and disclosed in coverage",
            ),
            (
                "I06",
                "claude, codex, and pi are present in tool_facts/skill_facts in window but none supplies skill_declarations; I06 refuses as NotComputable",
            ),
            (
                "I11",
                "pi is present in usage_facts in window but lacks session_edges; excluded from I11 denominator and disclosed in coverage",
            ),
            (
                "I16",
                "pi is present in security_facts in window (via SecurityCommandCheck) but lacks SecurityPolicy; excluded from I16 denominator and disclosed in coverage",
            ),
            (
                "I17",
                "pi is present in security_facts (session-only filter) but lacks SecurityPolicy; excluded from I17 denominator and disclosed in coverage",
            ),
            (
                "I20",
                "pi is present in usage_facts with currency_total but lacks queue_transitions; I20 refuses as NotComputable instead of not_observable",
            ),
            (
                "I24",
                "pi is present in security_facts (session-only filter) but lacks SessionEdges and SecurityPolicy; excluded from I24 denominator and disclosed in coverage",
            ),
        ]);

        let denom_cache =
            dendro_core::insight_v1::fidelity::StoreDenominatorCache::new(&store, &scope);
        assert_eq!(actual_cat.len(), golden_cat.len());
        for (actual, golden) in actual_cat.iter().zip(golden_cat.iter()) {
            let id = actual.id;
            assert_eq!(golden["id"].as_str(), Some(id));
            let actual_val = serde_json::to_value(actual).unwrap();

            let insight = CATALOG.iter().copied().find(|i| i.id() == id).unwrap();
            let present = denom_cache.present_connectors(insight);
            let all_present_computable = present.is_empty()
                || matches!(
                    evaluate_denominator(insight, &present),
                    DenominatorEvaluation::Computable { ref scope } if scope.excluded.is_empty()
                );

            if all_present_computable {
                assert!(
                    !expected_policy_diffs.contains_key(id),
                    "{id} has all present connectors computable ({present:?}), so it must not be in expected_policy_diffs"
                );
                assert_eq!(
                    serde_json::to_string_pretty(&actual_val).unwrap(),
                    serde_json::to_string_pretty(golden).unwrap(),
                    "windowed byte-identical PRE-D5 golden regression guard failed for {id} (present={present:?})"
                );
            } else {
                let reason = expected_policy_diffs.get(id).unwrap_or_else(|| {
                    panic!("{id} changed from windowed pre-D5 golden output without an explicit entry in expected_policy_diffs")
                });
                assert!(!reason.is_empty());
                assert_ne!(
                    serde_json::to_string_pretty(&actual_val).unwrap(),
                    serde_json::to_string_pretty(golden).unwrap(),
                    "{id} is listed in expected_policy_diffs ({reason}) but did not differ from windowed pre-D5 golden output"
                );
            }
        }

        assert_eq!(actual_eval.len(), golden_eval.len());
        for (actual, golden) in actual_eval.iter().zip(golden_eval.iter()) {
            let id = actual.id;
            let actual_val = serde_json::to_value(actual).unwrap();
            if id == "I04" {
                assert_eq!(actual.population.n, 1);
                assert!(actual
                    .coverage
                    .contains("excluded: claude — missing observed_cost; excluded: codex — missing observed_cost"));
            } else {
                assert_eq!(
                    serde_json::to_string_pretty(&actual_val).unwrap(),
                    serde_json::to_string_pretty(golden).unwrap(),
                    "windowed REFERENCE byte-identical PRE-D5 golden regression guard failed for {id}"
                );
            }
        }
    }

    #[test]
    fn time_window_scope_multi_connector_with_unregistered_harness_never_pools_or_computes_not_computable(
    ) {
        let key = b"0123456789abcdef0123456789abcdef";
        let tmp = TempDir::new("time-window-multi-unreg");
        let roots = vec![
            SourceRoot {
                name: "claude".into(),
                path: PathBuf::from("tests/fixtures/d5_golden/sources/claude"),
            },
            SourceRoot {
                name: "codex".into(),
                path: PathBuf::from("tests/fixtures/d5_golden/sources/codex"),
            },
            SourceRoot {
                name: "pi".into(),
                path: PathBuf::from("tests/fixtures/d5_golden/sources/pi"),
            },
        ];
        let db_path = tmp.path().join("staged.duckdb");
        drop(AnalyticsStore::open_path(&db_path, &dendro_core::key_id(key)).unwrap());
        let _warnings = ingest_staged(&db_path, &roots, false, key).unwrap();

        // Populate non-computable (`agy`) and unregistered harness rows (`unreg_alpha`, `unreg_beta`),
        // including rows with NULL or out-of-window timestamps in `security_facts`, `source_files`,
        // `harness_trajectory`, and `harness_envelope`.
        let vec_one = format!(
            "[{}]::FLOAT[32]",
            std::iter::once("1.0")
                .chain(std::iter::repeat("0.0").take(31))
                .collect::<Vec<_>>()
                .join(", ")
        );
        let vec_alt = format!(
            "[{}]::FLOAT[32]",
            std::iter::once("0.0")
                .chain(std::iter::once("1.0"))
                .chain(std::iter::repeat("0.0").take(30))
                .collect::<Vec<_>>()
                .join(", ")
        );
        {
            let conn = duckdb::Connection::open(&db_path).unwrap();
            conn.execute_batch(&format!(
                "INSERT INTO source_files (source_id, harness, display_path, generation, byte_offset, prefix_sha256, native_session) VALUES
                    ('src-agy', 'agy', 'agy.pb', 1, 100, 'aa', 'agy-s1'),
                    ('src-unreg-a', 'unreg_alpha', 'ua.jsonl', 1, 100, 'bb', 'ua-s1'),
                    ('src-unreg-b', 'unreg_beta', 'ub.jsonl', 1, 100, 'cc', 'ub-s1');
                INSERT INTO security_facts (source_key, source_id, harness, native_session, ts, kind, value, source_field, generation) VALUES
                    ('sec-agy', 'src-agy', 'agy', 'agy-s1', NULL, 'permission_mode', 'default', 'permission_mode', 1),
                    ('sec-ua-1', 'src-unreg-a', 'unreg_alpha', 'ua-s1', TIMESTAMP '2026-01-01 00:00:00', 'permission_mode', 'bypass', 'permission_mode', 1),
                    ('sec-ua-2', 'src-unreg-a', 'unreg_alpha', 'ua-s1', NULL, 'sandbox_policy', 'none', 'sandbox_policy', 1),
                    ('sec-ub-1', 'src-unreg-b', 'unreg_beta', 'ub-s1', TIMESTAMP '2025-05-05 00:00:00', 'permission_mode', 'bypass', 'permission_mode', 1);
                CREATE TABLE IF NOT EXISTS harness_trajectory (
                    observation_id VARCHAR,
                    session_key VARCHAR,
                    subject_key VARCHAR,
                    harness VARCHAR,
                    turn_k BIGINT,
                    embedding FLOAT[32],
                    vec FLOAT[32]
                );
                INSERT INTO harness_trajectory (observation_id, session_key, subject_key, harness, turn_k, embedding) VALUES
                    ('claude:r0', 'claude:s0', 'claude:ref-0', 'claude', 0, {vec_one}),
                    ('claude:r1', 'claude:s1', 'claude:ref-1', 'claude', 0, {vec_one}),
                    ('claude:r2', 'claude:s2', 'claude:ref-2', 'claude', 0, {vec_one}),
                    ('claude:r3', 'claude:s3', 'claude:ref-3', 'claude', 0, {vec_one}),
                    ('claude:r4', 'claude:s4', 'claude:ref-4', 'claude', 0, {vec_one}),
                    ('unreg_alpha:c0', 'ua:s', 'unreg_alpha:cand', 'unreg_alpha', 0, {vec_alt}),
                    ('unreg_alpha:c1', 'ua:s', 'unreg_alpha:cand', 'unreg_alpha', 1, {vec_alt}),
                    ('unreg_beta:c2', 'ub:s', 'unreg_beta:cand', 'unreg_beta', 2, {vec_alt});
                CREATE TABLE IF NOT EXISTS harness_envelope (
                    observation_id VARCHAR,
                    harness VARCHAR,
                    harness_version VARCHAR,
                    captured_at TIMESTAMP,
                    envelope_shape VARIANT
                );
                INSERT INTO harness_envelope (observation_id, harness, harness_version, captured_at, envelope_shape) VALUES
                    ('env-c1', 'claude', '1.0', TIMESTAMP '2025-01-01 00:00:00', CAST('{{\"a\":1}}' AS JSON)),
                    ('env-c2', 'claude', '1.0', TIMESTAMP '2025-01-01 00:00:01', CAST('{{\"a\":1}}' AS JSON)),
                    ('env-c3', 'claude', '1.0', TIMESTAMP '2025-01-01 00:00:02', CAST('{{\"a\":1}}' AS JSON)),
                    ('env-c4', 'claude', '1.0', TIMESTAMP '2025-01-01 00:00:03', CAST('{{\"a\":1}}' AS JSON)),
                    ('env-c5', 'claude', '1.0', TIMESTAMP '2025-01-01 00:00:04', CAST('{{\"a\":1}}' AS JSON)),
                    ('env-c6', 'claude', '2.0', TIMESTAMP '2025-01-01 00:00:05', CAST('{{\"a\":1,\"b\":2}}' AS JSON)),
                    ('env-c7', 'claude', '2.0', TIMESTAMP '2025-01-01 00:00:06', CAST('{{\"a\":1,\"b\":2}}' AS JSON)),
                    ('env-c8', 'claude', '2.0', TIMESTAMP '2025-01-01 00:00:07', CAST('{{\"a\":1,\"b\":2}}' AS JSON)),
                    ('env-c9', 'claude', '2.0', TIMESTAMP '2025-01-01 00:00:08', CAST('{{\"a\":1,\"b\":2}}' AS JSON)),
                    ('env-c10', 'claude', '2.0', TIMESTAMP '2025-01-01 00:00:09', CAST('{{\"a\":1,\"b\":2}}' AS JSON)),
                    ('env-ua1', 'unreg_alpha', '1.0', TIMESTAMP '2025-01-01 00:00:10', CAST('{{\"x\":1}}' AS JSON)),
                    ('env-ub1', 'unreg_beta', '1.0', TIMESTAMP '2025-01-01 00:00:11', CAST('{{\"y\":1}}' AS JSON));"
            ))
            .unwrap();
        }
        let store = AnalyticsStore::open_path(&db_path, &dendro_core::key_id(key)).unwrap();

        // Site always sets `from` / `to` (connected_web.rs ~416).
        let window_scope = Scope {
            from: Some("2026-10-10T00:00:00Z".into()),
            to: Some("2026-10-11T00:00:00Z".into()),
            ..Scope::default()
        };
        let results = evaluate_catalog(&store, &window_scope);

        // I17: presence uses `security_session_only()` (matching the I17 query), so `pi`, `agy`, and
        // unregistered harnesses (`unreg_alpha`, `unreg_beta`) with NULL or out-of-window `ts` are
        // detected as present, excluded from computation (only `claude` + `codex` are pooled), and
        // disclosed in `coverage`.
        let i17 = results.iter().find(|r| r.id == "I17").unwrap();
        assert_eq!(
            i17.population.n, 7,
            "I17 must only include claude (3) and codex (4) security_facts; pi, agy, and unregistered rows must be excluded"
        );
        assert!(
            i17.coverage
                .contains("excluded: agy — missing security_policy")
                && i17
                    .coverage
                    .contains("excluded: pi — missing security_policy")
                && i17
                    .coverage
                    .contains("excluded: unregistered harness (2 values, 3 rows)"),
            "I17 coverage must list agy, pi, and unregistered harness exclusions: {}",
            i17.coverage
        );
        assert!(
            !i17.coverage.contains("unreg_alpha") && !i17.coverage.contains("unreg_beta"),
            "I17 coverage must not print raw unregistered harness names: {}",
            i17.coverage
        );

        // I18: `sources()` does not return empty when `from`/`to` is set, so `agy` and the
        // unregistered harnesses in `source_files` are detected and disclosed in `coverage`.
        let i18 = results.iter().find(|r| r.id == "I18").unwrap();
        assert!(
            i18.coverage.contains("excluded: agy — missing source_coverage")
                && i18.coverage.contains("excluded: unregistered harness (2 values, 2 rows)"),
            "I18 coverage under time-window scope must list agy and unregistered harness exclusions: {}",
            i18.coverage
        );

        // I22: matrix says NotComputable for all connectors; under a time-window scope (and even
        // when `session` is also set), I22 never computes a number, returns NotComputable, and lists
        // the exclusions in coverage.
        let i22 = results.iter().find(|r| r.id == "I22").unwrap();
        assert!(
            matches!(&i22.status, InsightStatus::Refused { reason } if reason.starts_with("not_computable: missing trajectory_vector")),
            "I22 must return NotComputable under time-window scope, got {:?}",
            i22.status
        );
        assert!(
            i22.measures.is_empty(),
            "I22 must never show a number where the matrix says NotComputable"
        );
        assert!(
            i22.coverage
                .contains("excluded: claude — missing trajectory_vector")
                && i22
                    .coverage
                    .contains("excluded: unregistered harness (2 values, 3 rows)"),
            "I22 coverage must list claude and unregistered exclusions: {}",
            i22.coverage
        );

        // I23: `harness_envelope` rows have `captured_at` in 2025 (outside the 2026-10-10 window),
        // and the I23 query has no `captured_at` filter. Because I23 presence matches the I23 query,
        // I23 never falls through to unfiltered compute, returns NotComputable with no measures,
        // and lists the exclusions in coverage.
        let i23 = results.iter().find(|r| r.id == "I23").unwrap();
        assert!(
            matches!(&i23.status, InsightStatus::Refused { reason } if reason.starts_with("not_computable: missing harness_envelope_shape")),
            "I23 must return NotComputable under time-window scope even when captured_at is outside [from, to), got {:?}",
            i23.status
        );
        assert!(
            i23.measures.is_empty(),
            "I23 must never show a number where the matrix says NotComputable"
        );
        assert!(
            i23.coverage
                .contains("excluded: claude — missing harness_envelope_shape")
                && i23
                    .coverage
                    .contains("excluded: unregistered harness (2 values, 2 rows)"),
            "I23 coverage must list claude and unregistered exclusions: {}",
            i23.coverage
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
