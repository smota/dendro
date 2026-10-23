//! Regression for the real report path's distinct native Codex observation collection.
use dendro_core::{AnalyticsStore, IdentityFact, ReaderState, ToolCategory, ToolFact, UsageFact};
use serde_json::Value;
fn report(mixed: bool) -> Value {
    #[cfg(windows)]
    let root = dendro_core::platform_v1::create_user_private_root_with_choice(
        "dendro-routing-",
        &dendro_core::platform_v1::StorageRootChoice::UserProfile,
    )
    .unwrap();
    #[cfg(windows)]
    let dir = root.path().to_path_buf();
    #[cfg(windows)]
    drop(root);
    #[cfg(not(windows))]
    let dir = std::env::current_dir()
        .unwrap()
        .join("target/routing-tests")
        .join(format!("{}-{}", std::process::id(), rand::random::<u64>()));
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("analytics.duckdb");
    let store = AnalyticsStore::open_path(&db, "fixture").unwrap();
    let ts = dendro_core::parse_utc_time("2026-01-01T00:00:01Z").unwrap();
    for harness in if mixed {
        vec!["claude", "codex"]
    } else {
        vec!["codex"]
    } {
        let native = format!("session-{harness}");
        let source = format!("source-{harness}");
        let tool_key = format!("{source}:1:1:tool");
        store
            .insert_identity(&IdentityFact {
                source_key: format!("id-{harness}"),
                harness: harness.into(),
                native_session: native.clone(),
                rig: "test".into(),
                seat: "same-seat".into(),
                ts: Some(ts),
            })
            .unwrap();
        let tool = ToolFact {
            source_key: tool_key.clone(),
            harness: harness.into(),
            native_session: native.clone(),
            ts: Some(ts),
            seq: 1,
            name: if harness == "codex" {
                dendro_core::CODEX_COMMAND.into()
            } else {
                "Bash".into()
            },
            fingerprint: Some("a".repeat(64)),
            failed: Some(false),
            structured_write: false,
            generation: 1,
        };
        let usage = if harness == "codex" {
            vec![UsageFact {
                source_key: "usage-codex".into(),
                harness: harness.into(),
                native_session: native.clone(),
                message_id: None,
                ts: Some(ts),
                seq: 1,
                input_uncached: 10,
                output: 2,
                cache_read: 3,
                cache_write: 0,
                reasoning: 2,
                currency_total: None,
                generation: 1,
                predecessor_source_key: None,
                predecessor_reason: None,
                prior_input_tokens: None,
                prior_cached_input_tokens: None,
                prior_cache_write_input_tokens: None,
                prior_output_tokens: None,
                prior_reasoning_output_tokens: None,
            }]
        } else {
            vec![]
        };
        store
            .commit_file_batch_reading(
                &source,
                harness,
                "synthetic.jsonl",
                1,
                10,
                &"a".repeat(64),
                None,
                &usage,
                &[tool],
                &[],
                &ReaderState {
                    complete: true,
                    native_session: Some(native),
                },
            )
            .unwrap();
        store
            .record_tool_categories(&[(
                tool_key,
                if harness == "codex" && mixed {
                    ToolCategory::EngCodeEdit
                } else {
                    ToolCategory::CoordQueue
                },
            )])
            .unwrap();
    }
    store
        .record_usage_models(&[(
            "usage-codex".into(),
            Some("fixture-model".into()),
            Some("xhigh".into()),
        )])
        .unwrap();
    drop(store);
    let report = dendro_core::report_json(
        &db,
        "decisions",
        None,
        "2026-01-01T00:00:00Z",
        "2026-01-02T00:00:00Z",
    )
    .unwrap();
    std::fs::remove_dir_all(dir).unwrap();
    report
}
#[test]
fn codex_engineering_prevents_mixed_harness_routing_only_false_positive() {
    let r = report(true);
    assert_eq!(
        r["payload"]["routing_effort_evidence"],
        serde_json::json!([])
    );
    assert!(!r["payload"]["anomaly_spotlights"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f["metric"] == "xhigh_routing_only_observed_seat_window_tokens"));
}
#[test]
fn codex_only_routing_is_reachable_with_its_real_coverage_and_tool_collection() {
    let r = report(false);
    let records = r["payload"]["routing_effort_evidence"].as_array().unwrap();
    assert_eq!(
        records.len(),
        1,
        "coverage={} categories={} routing={}",
        r["payload"]["outcome_coverage"],
        r["payload"]["tool_categories"],
        r["payload"]["routing_effort_coverage"]
    );
    assert_eq!(records[0]["tokens"], 15);
    assert!(records[0]["source_keys"]
        .as_array()
        .unwrap()
        .iter()
        .any(|k| k == "source-codex:1:1:tool"));
    assert!(r["payload"]["anomaly_spotlights"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .any(
            |f| f["metric"] == "xhigh_routing_only_observed_seat_window_tokens"
                && f["value"].as_f64() == Some(15.0)
        ));
}
