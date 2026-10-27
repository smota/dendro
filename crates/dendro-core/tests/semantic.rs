#[path = "support/semantic_fixture.rs"]
mod fixture;
use dendro_core::{report_json, semantic};
use duckdb::Connection;
use serde_json::Value;

fn fixture(facts: usize) -> fixture::Fixture {
    let original = fixture::Fixture::new(facts);
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        original
    }
    #[cfg(any(windows, target_os = "linux"))]
    {
        let root = dendro_core::platform_v1::create_user_private_root_with_choice(
            "dendro-semantic-integration-",
            &dendro_core::platform_v1::StorageRootChoice::UserProfile,
        )
        .unwrap();
        let db = root.path().join("analytics.duckdb");
        dendro_core::lifecycle::private_copy(&original.db, &db, &dendro_core::RUN_CANCEL).unwrap();
        let key = dendro_core::output::load_or_create_key(
            &root.path().join("analytics.duckdb.key"),
            true,
        )
        .unwrap();
        let conn = Connection::open(&db).unwrap();
        conn.execute("UPDATE dendro_meta SET key_id=?", duckdb::params![key.id])
            .unwrap();
        conn.execute_batch("CHECKPOINT").unwrap();
        drop(conn);
        fixture::Fixture {
            dir: root.path().to_path_buf(),
            db,
        }
    }
}

fn read_fixture(
    db: &std::path::Path,
) -> anyhow::Result<(dendro_core::presentation::ReadRequest, Connection)> {
    let request = dendro_core::presentation::ReadRequest::open(db)?;
    let config = duckdb::Config::default().access_mode(duckdb::AccessMode::ReadOnly)?;
    let connection = Connection::open_with_flags(request.database(), config)?;
    Ok((request, connection))
}

fn mutate_fixture(
    db: &std::path::Path,
    mutate: impl FnOnce(&std::path::Path) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    #[cfg(any(windows, target_os = "linux"))]
    {
        let request = dendro_core::presentation::ReadRequest::open(db)?;
        if let Some(selected) = request.selected() {
            dendro_core::selection_binding::mutate_selected(selected, |staged, _| mutate(staged))?;
            return Ok(());
        }
    }
    mutate(db)
}

#[test]
fn exact_canonical_parity_refusals_and_source_updates() -> anyhow::Result<()> {
    let f = fixture(20);
    let report = report_json(
        &f.db,
        "decisions",
        Some("test-rig"),
        fixture::FROM,
        fixture::TO,
    )?;
    semantic::refresh(&f.db, Some("test-rig"), fixture::FROM, fixture::TO)?;
    let (_read, c) = read_fixture(&f.db)?;
    let payload: String = c.query_row(
        "SELECT row_json::VARCHAR FROM v_executive_governance",
        [],
        |r| r.get(0),
    )?;
    let executive: Value = serde_json::from_str(&payload)?;
    for field in [
        "m1_yield",
        "m1_by_seat",
        "m2_return_rate",
        "m3_rework_chains",
        "token_categories",
        "link_rates",
        "fleet_concurrency",
        "watchdog_recovery",
        "four_eyes_independence",
        "context_compactions",
        "cross_slice_cache_bleed",
        "task_intake_overhead",
        "ring_boundaries",
    ] {
        assert_eq!(executive[field], report["payload"][field], "{field}");
    }
    assert_eq!(executive, report["payload"]);
    let mut pairs = c.prepare("SELECT row_json::VARCHAR FROM v_handoff_rework_analysis")?;
    for row in pairs.query_map([], |r| r.get::<_, String>(0))? {
        let value: Value = serde_json::from_str(&row?)?;
        assert!(report["payload"]["handoff_friction"]["pairs"]
            .as_array()
            .unwrap()
            .contains(&value));
    }
    drop(pairs);
    let mut stmt =
        c.prepare("SELECT row_json::VARCHAR FROM v_seat_idle_ranking ORDER BY seat_id")?;
    for row in stmt.query_map([], |r| r.get::<_, String>(0))? {
        let value: Value = serde_json::from_str(&row?)?;
        assert!(report["payload"]["activity"]
            .as_array()
            .unwrap()
            .contains(&value));
    }
    drop(stmt);
    drop(c);
    assert_eq!(
        semantic::answer(&f.db, "Compare handoff efficiency", None)?["message"],
        "Insufficient data: no comparison baseline"
    );
    assert_eq!(
        semantic::answer(&f.db, "drop table usage_facts", None)?["status"],
        "insufficient_data"
    );
    assert_eq!(
        semantic::answer(&f.db, "Rank secrets", None)?["status"],
        "insufficient_data"
    );
    assert_eq!(
        semantic::answer(&f.db, "Which seat has highest idle?", None)?["status"],
        "ok"
    );
    let yield_answer = semantic::answer(&f.db, "Which seat had worst yield?", None)?;
    if yield_answer["status"] == "ok" {
        assert!(yield_answer["ordering"]
            .as_str()
            .unwrap()
            .contains("worst observed burn"));
    }
    for q in [
        "Which seat has highest idle on unrelated mission last year?",
        "Compare handoff efficiency this week vs last week",
        "Explain rework in mission other",
        "What changed since I removed dev-build-jr?",
    ] {
        let refusal = semantic::answer(&f.db, q, None)?;
        assert_eq!(refusal["status"], "insufficient_data");
        assert_eq!(refusal["scope"]["rig"], "test-rig");
    }
    let trace = semantic::answer(&f.db, "Trace idle", None)?;
    assert_eq!(trace["report"], report);
    assert!(trace["source_keys"]
        .as_array()
        .unwrap()
        .iter()
        .any(|k| k == "file:0:1"));
    assert_eq!(
        semantic::answer(&f.db, "What changed?", Some(&report))?["status"],
        "ok"
    );
    assert_eq!(
        semantic::answer(&f.db, "Honest limits", None)?["status"],
        "insufficient_data"
    );
    assert_eq!(
        semantic::answer(&f.db, "Compare launch speed", None)?["status"],
        "insufficient_data"
    );
    assert_eq!(
        semantic::answer(&f.db, "Compare handoff efficiency", Some(&report))?["current"],
        report["payload"]
    );
    assert!(semantic::answer(&f.db, "Explain rework", None)?["limit"]
        .as_str()
        .unwrap()
        .contains("not proof"));
    mutate_fixture(&f.db, |path| {
        let c = Connection::open(path)?;
        // Equal row count/source keys must still invalidate the semantic signature.
        c.execute_batch(
            "UPDATE usage_facts SET output_tokens=99 WHERE source_key='file:0:1'; CHECKPOINT",
        )?;
        Ok(())
    })?;
    assert_eq!(
        semantic::answer(&f.db, "Trace idle", None)?["status"],
        "insufficient_data"
    );
    semantic::refresh(&f.db, Some("test-rig"), fixture::FROM, fixture::TO)?;
    assert_eq!(
        semantic::answer(&f.db, "Trace idle", None)?["report"]["payload"]["tokens"],
        20 * 26 + 92
    );
    mutate_fixture(&f.db, |path| {
        let key_id = dendro_core::AnalyticsStore::open_read_only(path)?.key_id;
        let store = dendro_core::AnalyticsStore::open_existing(path, &key_id)?;
        store.record_usage_models(&[(
            "file:0:1".into(),
            Some("different-family".into()),
            Some("low".into()),
        )])?;
        store.checkpoint()?;
        Ok(())
    })?;
    let (_read, c) = read_fixture(&f.db)?;
    assert!(c
        .query_row(
            "SELECT row_json::VARCHAR FROM v_executive_governance",
            [],
            |r| r.get::<_, String>(0)
        )
        .is_err());
    drop(c);
    Ok(())
}
#[test]
fn workspace_is_new_only_and_contains_real_configuration() -> anyhow::Result<()> {
    let f = fixture(0);
    let requested = f.dir.join("nao");
    #[cfg(windows)]
    let receipt = {
        assert!(semantic::init_workspace(&requested, &f.db)
            .unwrap_err()
            .to_string()
            .contains("nao_exact_workspace_path_unproved"));
        semantic::init_workspace_with_receipt(&requested, &f.db)?
    };
    #[cfg(windows)]
    let path = std::path::PathBuf::from(receipt["workspace"].as_str().unwrap());
    #[cfg(not(windows))]
    let path = {
        semantic::init_workspace(&requested, &f.db)?;
        requested
    };
    assert!(std::fs::read_to_string(path.join("nao_config.yaml"))?.contains("v_mission_token_burn"));
    let config = std::fs::read_to_string(path.join("nao_config.yaml"))?;
    assert!(config.contains("project_name: dendro"));
    assert!(config.contains("allow_listed_only: true"));
    #[cfg(not(any(windows, target_os = "linux")))]
    assert!(config.contains(&f.db.canonicalize()?.display().to_string()));
    #[cfg(target_os = "linux")]
    {
        assert!(config.contains(&serde_json::to_string(&path.join("analytics.duckdb"))?));
        assert!(config.contains("read_only: true"));
    }
    #[cfg(windows)]
    {
        assert!(config.contains(&serde_json::to_string(&receipt["path"].as_str().unwrap())?));
        assert_eq!(receipt["read_only"], true);
        assert_eq!(
            receipt["integration_status"],
            "snapshot_created_runtime_integration_pending"
        );
        assert!(receipt["analytics_selection_sha256"]
            .as_str()
            .is_some_and(|v| v.len() == 64));
        assert!(config.contains("read_only: true"));
        assert_ne!(path, requested);
        assert!(!requested.exists());
        assert!(path.join("semantics-dendro.md").is_file());
        assert!(!path.join("semantics").exists());
        assert!(receipt["semantic_context_status"]
            .as_str()
            .unwrap()
            .contains("not verified"));
    }
    assert!(path.join("RULES.md").exists());
    #[cfg(not(windows))]
    assert!(path.join("semantics/dendro.md").exists());
    #[cfg(windows)]
    assert!(semantic::init_workspace_with_receipt(&path, &f.db).is_err());
    assert!(semantic::init_workspace(&path, &f.db).is_err());
    assert!(semantic::init_workspace(&f.dir.join("../bad"), &f.db).is_err());
    #[cfg(unix)]
    {
        let linked = f.dir.join("link");
        std::os::unix::fs::symlink(&path, &linked)?;
        assert!(semantic::init_workspace(&linked, &f.db).is_err());
    }
    Ok(())
}
#[test]
fn no_data_does_not_become_measured_zero() -> anyhow::Result<()> {
    let f = fixture(0);
    semantic::refresh(&f.db, Some("missing-rig"), fixture::FROM, fixture::TO)?;
    let (_read, c) = read_fixture(&f.db)?;
    let n: i64 = c.query_row("SELECT count(*) FROM v_mission_token_burn", [], |r| {
        r.get(0)
    })?;
    assert_eq!(n, 0);
    drop(c);
    assert_eq!(
        semantic::answer(&f.db, "Explain rework", None)?["status"],
        "insufficient_data"
    );
    assert_eq!(
        semantic::answer(&f.db, "Which seat has highest idle", None)?["status"],
        "insufficient_data"
    );
    Ok(())
}
#[test]
fn natural_aliases_supported_intents_and_window_bounds_work() -> anyhow::Result<()> {
    let f = fixture(10);
    let intents = semantic::supported_intents();
    assert_eq!(intents["intents"].as_array().unwrap().len(), 6);
    let names: Vec<&str> = intents["intents"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["intent"].as_str())
        .collect();
    assert_eq!(
        names,
        vec![
            "Rank (Idle)",
            "Rank (Yield / Token Burn)",
            "Explain (Rework & Handoffs)",
            "Compare (Before/After Baseline)",
            "Verify (Topology Change)",
            "Trace & HonestLimits",
        ]
    );
    let views: Vec<&str> = intents["semantic_views"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["view"].as_str())
        .collect();
    assert_eq!(
        views,
        vec![
            "v_message_mix",
            "v_subagent_lineage",
            "v_security_posture",
            "v_process_resources",
        ]
    );

    let (from, to, mode) = dendro_core::presentation::resolve_window(
        &f.db,
        Some("test-rig"),
        Some("all"),
        None,
        None,
    )?;
    assert_eq!(mode, "recorded_bounds");
    assert_eq!(from, "2026-10-01T00:00:00Z");
    assert_eq!(to, "2026-10-02T00:00:01Z");

    let (_, _, mode_7d) =
        dendro_core::presentation::resolve_window(&f.db, Some("test-rig"), Some("7d"), None, None)?;
    assert_eq!(mode_7d, "anchored_to_recorded_data");

    // Rig-scoped anchoring and single-timestamp ceiling: insert a second rig at a single timestamp.
    let conn = Connection::open(&f.db)?;
    conn.execute_batch(
        "INSERT INTO identity_facts VALUES ('id:single','claude','sess-single','rig-ghp_secret12345678901234567890','seat-sk-secret12345678901234567890@rig-ghp_secret12345678901234567890','2026-09-15 05:00:00');
         INSERT INTO usage_facts (source_key,harness,native_session,ts,seq,input_uncached,output_tokens,cache_read,cache_write,reasoning,generation,source_id)
         VALUES ('single:0:1','claude','sess-single','2026-09-15 05:00:00.500',1,10,5,0,0,0,0,'file');",
    )?;
    drop(conn);

    let (single_from, single_to, single_mode) = dendro_core::presentation::resolve_window(
        &f.db,
        Some("rig-ghp_secret12345678901234567890"),
        Some("all"),
        None,
        None,
    )?;
    assert_eq!(single_mode, "recorded_bounds");
    assert_eq!(single_from, "2026-09-15T05:00:00Z");
    assert_eq!(single_to, "2026-09-15T05:00:01Z");

    // Verify test-rig bounds remain unaffected by the older rig.
    let (test_rig_from, test_rig_to, _) = dendro_core::presentation::resolve_window(
        &f.db,
        Some("test-rig"),
        Some("all"),
        None,
        None,
    )?;
    assert_eq!(test_rig_from, "2026-10-01T00:00:00Z");
    assert_eq!(test_rig_to, "2026-10-02T00:00:01Z");

    semantic::refresh(&f.db, Some("test-rig"), fixture::FROM, fixture::TO)?;
    let status = dendro_core::presentation::database_status(&f.db, Some("test-rig"))?;
    assert_eq!(status["status"], "ready");
    assert_eq!(status["semantic_fresh"], true);
    assert!(status["usage_rows"].as_i64().unwrap() > 0);
    let status_str = status.to_string();
    assert!(!status_str.contains("ghp_secret"));
    assert!(!status_str.contains("sk-secret"));

    assert_eq!(
        semantic::answer(&f.db, "highest idle", None)?["status"],
        "ok"
    );
    assert_eq!(
        semantic::answer(&f.db, "where do tokens go?", None)?["ordering"],
        "tokens_per_closed_item descending: worst observed burn per completed item"
    );
    assert_eq!(semantic::answer(&f.db, "rework", None)?["status"], "ok");
    assert_eq!(
        semantic::answer(&f.db, "verify", None)?["message"],
        "Insufficient data: no comparison baseline"
    );
    assert_eq!(
        semantic::answer(&f.db, "trace tokens", None)?["status"],
        "ok"
    );
    assert_eq!(
        semantic::answer(&f.db, "trace handoffs", None)?["status"],
        "ok"
    );
    assert_eq!(
        semantic::answer(&f.db, "known limits", None)?["status"],
        "insufficient_data"
    );
    let unknown = semantic::answer(&f.db, "invent unsupported forecast", None)?;
    assert_eq!(unknown["status"], "insufficient_data");
    assert!(unknown["supported_examples"].is_array());
    Ok(())
}
