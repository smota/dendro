//! Focused source regressions. Independent native/browser acceptance is separately owned.
use super::*;

fn at(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}
fn request() -> OverviewRequest {
    OverviewRequest {
        rig: Some("r".into()),
        from: at("2026-10-01T00:00:00Z"),
        to: at("2026-10-01T04:00:00Z"),
        observation_clock: Some(at("2026-10-01T04:00:00Z")),
        evidence_as_of: None,
        advisory_options: AdvisoryOptions::default(),
        friction_thresholds: FrictionThresholds::default(),
        expect_evidence_digest: None,
    }
}
fn identity() -> IdentityFact {
    IdentityFact {
        source_key: "id".into(),
        harness: "codex".into(),
        native_session: "s".into(),
        rig: "r".into(),
        seat: "builder".into(),
        ts: Some(at("2026-10-01T00:00:00Z")),
    }
}
fn usage(key: &str, ts: &str) -> OverviewUsageFact {
    OverviewUsageFact {
        source_key: key.into(),
        harness: "codex".into(),
        native_session: "s".into(),
        ts_ms: Some(at(ts).timestamp_millis()),
        total_tokens: 7,
        currency_total: Some(1.0),
        ..Default::default()
    }
}

#[test]
fn digest_rejects_changed_operands_and_preserves_fractional_scope() {
    let ids = [identity()];
    let mut usage = [usage("u", "2026-10-01T01:00:00Z")];
    let mut req = request();
    let baseline = evaluate_direct_overview_facts(
        &RawOverviewFacts {
            identity_facts: &ids,
            usage_facts: &usage,
            ..Default::default()
        },
        &req,
    )
    .unwrap();
    usage[0].total_tokens = 999_999;
    let changed = evaluate_direct_overview_facts(
        &RawOverviewFacts {
            identity_facts: &ids,
            usage_facts: &usage,
            ..Default::default()
        },
        &req,
    )
    .unwrap();
    assert_ne!(baseline.digest, changed.digest);
    req.expect_evidence_digest = Some(baseline.digest);
    assert!(evaluate_direct_overview_facts(
        &RawOverviewFacts {
            identity_facts: &ids,
            usage_facts: &usage,
            ..Default::default()
        },
        &req
    )
    .unwrap_err()
    .to_string()
    .contains("evidence digest mismatch"));
    req.expect_evidence_digest = None;
    req.from = at("2026-10-01T00:00:00.000001Z");
    let fractional = evaluate_direct_overview_facts(
        &RawOverviewFacts {
            identity_facts: &ids,
            usage_facts: &usage,
            ..Default::default()
        },
        &req,
    )
    .unwrap();
    assert_eq!(fractional.scope.from, "2026-10-01T00:00:00.000001Z");
    assert_ne!(fractional.digest, changed.digest);
    req.observation_clock = Some(at("2026-10-01T06:00:00Z"));
    assert_eq!(
        fractional.digest,
        evaluate_direct_overview_facts(
            &RawOverviewFacts {
                identity_facts: &ids,
                usage_facts: &usage,
                ..Default::default()
            },
            &req
        )
        .unwrap()
        .digest
    );
}

#[test]
fn context_requires_supported_time_unique_binding_and_known_outcomes() {
    let ids = [identity()];
    let usage = [usage("event:1", "2026-10-01T01:00:00Z")];
    let skills = [SkillFact {
        source_key: "event:1:skill:0".into(),
        seat: "builder".into(),
        skill: "tool".into(),
        invoked: false,
    }];
    let mut tools = [OverviewToolFact {
        source_key: "historical".into(),
        harness: "codex".into(),
        native_session: "s".into(),
        ts_ms: Some(at("2026-09-30T23:00:00Z").timestamp_millis()),
        tool_name: "tool".into(),
        failed: Some(true),
    }];
    let req = request();
    let eval = |tools: &[OverviewToolFact]| {
        evaluate_direct_overview_facts(
            &RawOverviewFacts {
                identity_facts: &ids,
                usage_facts: &usage,
                skill_facts: &skills,
                tool_facts: tools,
                ..Default::default()
            },
            &req,
        )
        .unwrap()
    };
    assert_eq!(eval(&tools).context_tax.uncalled_tools_count, Some(1));
    tools[0].ts_ms = Some(at("2026-10-01T02:00:00Z").timestamp_millis());
    let called = eval(&tools);
    assert_eq!(called.context_tax.uncalled_tools_count, Some(0));
    assert_eq!(called.context_tax.active_tools[0].error_rate, Some(1.0));
    tools[0].failed = None;
    assert_eq!(
        eval(&tools).context_tax.active_tools[0].unknown_outcome_count,
        1
    );
    assert_eq!(eval(&tools).context_tax.active_tools[0].error_rate, None);
    let unknown = [SkillFact {
        source_key: "undated".into(),
        ..skills[0].clone()
    }];
    let result = evaluate_direct_overview_facts(
        &RawOverviewFacts {
            identity_facts: &ids,
            skill_facts: &unknown,
            ..Default::default()
        },
        &req,
    )
    .unwrap();
    assert_eq!(result.context_tax.uncalled_tools_count, None);
    assert_eq!(result.context_tax.excluded_source_keys, vec!["undated"]);
}

#[test]
fn native_loader_derives_incidents_activity_and_historical_closures() {
    let dir = std::env::temp_dir().join(format!(
        "d101-{}-{}",
        std::process::id(),
        rand::random::<u32>()
    ));
    std::fs::create_dir(&dir).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let db = dir.join("facts.duckdb");
    let store = AnalyticsStore::open_path(&db, "fixture").unwrap();
    let conn = store.connection();
    conn.execute_batch(r#"
        INSERT INTO identity_facts VALUES ('id','codex','s','r','builder','2026-10-01 00:00:00');
        INSERT INTO identity_facts VALUES ('id-other','codex','other','r','other','2026-10-01 00:00:00');
        INSERT INTO activity_facts VALUES ('idle','builder','2026-10-01 00:00:00','idle',1),('run','other','2026-10-01 00:00:00','running',1);
        INSERT INTO seat_bounds VALUES ('other','2026-10-01 00:00:00','2026-10-01 02:00:00');
        INSERT INTO queue_facts (source_key,transition_id,qitem_id,ts,state,source_seat,dest_seat,closure_reason,missions,issue_tags) VALUES
            ('events:queue.created:stalled',0,'stalled','2026-10-01 00:00:00','pending','orch','builder',NULL,'[]','["101"]'),
            ('claim',1,'stalled','2026-10-01 00:00:01','in-progress','orch','builder',NULL,'[]','["101"]'),
            ('own-claim',2,'closed','2026-10-01 00:00:01','in-progress','orch','other',NULL,'[]','["101"]'),
            ('own-done',3,'closed','2026-10-01 01:00:00','done','orch','other','completed','[]','["101"]'),
            ('own-future-reopen',4,'closed','2026-10-01 05:00:00','pending','orch','other',NULL,'[]','["101"]'),
            ('denied',5,'denied','2026-10-01 01:00:00','done','orch','other','denied','[]','["101"]'),
            ('malformed',6,'malformed','2026-10-01 01:00:00','done','orch','other','completed','[]','["garbage"]');
        INSERT INTO process_identities (identity_key,root_key,pid,platform,start_time_unix_s,exe_name,harness,cwd_fingerprint,session_link,environment_kind,isolation_hint,first_seen,last_seen,visibility_limits) VALUES
            ('p','root',1,'linux',0,'proc','codex','fp-project','other','local','none','2026-10-01 00:00:00','2026-10-01 04:00:00','none');
    "#).unwrap();
    let overview = evaluate_overview_snapshot(&store, &request()).unwrap();
    assert_eq!(
        overview.critical_focus.category.as_deref(),
        Some("Leaf Stall")
    );
    assert_eq!(
        overview.critical_focus.entity_id.as_deref(),
        Some("builder")
    );
    assert!(overview
        .critical_focus
        .selected_source_keys
        .contains(&"claim".into()));
    assert!(overview
        .critical_focus
        .selected_source_keys
        .contains(&"idle".into()));
    assert_eq!(overview.throughput_and_cost.closed_issue_qitems, 1);
    assert_eq!(overview.project_groups[0].active_hours, 2.0);
    assert_eq!(overview.project_groups[0].closed_issue_qitems, 1);
    assert_eq!(overview.observed_active_hours, 2.0);
    assert_eq!(overview.unassigned_active_hours, 0.0);
    assert_eq!(overview.source_coverage["admission_holds"], "unavailable");
    drop(store);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn empty_evidence_is_unavailable_and_never_live() {
    let value = evaluate_direct_overview_facts(&RawOverviewFacts::default(), &request()).unwrap();
    assert_eq!(value.coverage, "unavailable");
    assert_eq!(value.critical_focus.focus_state, "unavailable");
    assert!(!value.live_alert);
    assert_eq!(value.context_tax.uncalled_tools_count, None);
}
