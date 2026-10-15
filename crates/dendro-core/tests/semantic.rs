#[path = "support/semantic_fixture.rs"]
mod fixture;
use dendro_core::{report_json, semantic};
use duckdb::Connection;
use serde_json::Value;

#[test]
fn exact_canonical_parity_refusals_and_source_updates() -> anyhow::Result<()> {
    let f = fixture::Fixture::new(20);
    let report = report_json(
        &f.db,
        "decisions",
        Some("test-rig"),
        fixture::FROM,
        fixture::TO,
    )?;
    semantic::refresh(&f.db, Some("test-rig"), fixture::FROM, fixture::TO)?;
    let c = Connection::open(&f.db)?;
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
    let c = Connection::open(&f.db)?;
    // In-place edits with unchanged row count and unchanged source keys must invalidate the cache.
    c.execute_batch("UPDATE usage_facts SET output_tokens=99 WHERE source_key='file:0:1'")?;
    // Direct SQL is outside supported writer invalidation; local preflight below catches it.
    drop(c);
    assert_eq!(
        semantic::answer(&f.db, "Trace idle", None)?["status"],
        "insufficient_data"
    );
    semantic::refresh(&f.db, Some("test-rig"), fixture::FROM, fixture::TO)?;
    assert_eq!(
        semantic::answer(&f.db, "Trace idle", None)?["report"]["payload"]["tokens"],
        20 * 26 + 92
    );
    let store = dendro_core::AnalyticsStore::open_existing(&f.db, "fixture")?;
    store.record_usage_models(&[(
        "file:0:1".into(),
        Some("different-family".into()),
        Some("low".into()),
    )])?;
    drop(store);
    let c = Connection::open(&f.db)?;
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
    let f = fixture::Fixture::new(0);
    let path = f.dir.join("nao");
    semantic::init_workspace(&path, &f.db)?;
    assert!(std::fs::read_to_string(path.join("nao_config.yaml"))?.contains("v_mission_token_burn"));
    let config = std::fs::read_to_string(path.join("nao_config.yaml"))?;
    assert!(config.contains("project_name: dendro"));
    assert!(config.contains("allow_listed_only: true"));
    assert!(config.contains(&f.db.canonicalize()?.display().to_string()));
    assert!(path.join("RULES.md").exists());
    assert!(path.join("semantics/dendro.md").exists());
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
    let f = fixture::Fixture::new(0);
    semantic::refresh(&f.db, Some("missing-rig"), fixture::FROM, fixture::TO)?;
    let c = Connection::open(&f.db)?;
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
