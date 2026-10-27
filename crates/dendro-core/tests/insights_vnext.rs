//! A08, A10, and A14 proof for the I01–I25 catalog.
use dendro_core::insight_v1::{
    display_status, evaluate, evaluate_catalog, InsightResult, InsightStatus, MeasureValue, Scope,
    CATALOG, REFERENCE,
};
use dendro_core::platform_v1::{self as platform, PrivateRoot, StorageRootChoice};
use dendro_core::AnalyticsStore;

fn reason(status: &InsightStatus) -> Option<&str> {
    match status {
        InsightStatus::Computed => None,
        InsightStatus::Partial { reason } | InsightStatus::Refused { reason } => Some(reason),
    }
}

fn by_id<'a>(results: &'a [InsightResult], id: &str) -> &'a InsightResult {
    results
        .iter()
        .find(|result| result.id == id)
        .unwrap_or_else(|| panic!("missing {id}"))
}

fn int_at(result: &InsightResult, id: &str) -> Option<i64> {
    result.measures.iter().find_map(|measure| {
        if measure.id != id {
            return None;
        }
        match measure.value {
            Some(MeasureValue::Int(value)) => Some(value),
            _ => None,
        }
    })
}

fn float_at(result: &InsightResult, id: &str) -> Option<f64> {
    result.measures.iter().find_map(|measure| {
        if measure.id != id {
            return None;
        }
        match measure.value {
            Some(MeasureValue::Float(value)) => Some(value),
            _ => None,
        }
    })
}

fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
}

fn writable(label: &str) -> (PrivateRoot, AnalyticsStore) {
    let root = platform::create_user_private_root_with_choice(
        &format!("dendro-insights-vnext-{label}-"),
        &StorageRootChoice::UserProfile,
    )
    .unwrap_or_else(|err| panic!("private root: {err}"));
    let path = root.path().join("insights.duckdb");
    {
        let created = AnalyticsStore::open_path(&path, "vnext").expect("create");
        created.checkpoint().expect("checkpoint");
    }
    let store = AnalyticsStore::open_staged(&path).expect("staged");
    (root, store)
}

fn sql(store: &AnalyticsStore, statement: &str) {
    store
        .with_private_connection(|conn| {
            conn.execute_batch(statement)?;
            Ok(())
        })
        .unwrap_or_else(|err| panic!("{err}\n{statement}"));
}

fn catalog(store: &AnalyticsStore) -> Vec<InsightResult> {
    evaluate_catalog(store, &Scope::default())
}

fn one(store: &AnalyticsStore, id: &str) -> InsightResult {
    catalog(store)
        .into_iter()
        .find(|result| result.id == id)
        .unwrap_or_else(|| panic!("missing {id}"))
}

#[test]
fn shipped_evaluate_stays_six_empty_population_answers() {
    let store = AnalyticsStore::open_in_memory("v080").unwrap();
    let results = evaluate(&store, &Scope::default());
    let ids: Vec<_> = results.iter().map(|result| result.id).collect();
    assert_eq!(ids, ["I02", "I04", "I08", "I09", "I14", "I18"]);
    assert_eq!(REFERENCE.len(), 6);
    for result in &results {
        assert_eq!(reason(&result.status), Some("empty population"));
        assert!(result.measures.is_empty());
    }
}

#[test]
fn catalog_is_i01_through_i25_and_empty_store_refuses_without_panic() {
    assert_eq!(CATALOG.len(), 25);
    let a08: Vec<_> = CATALOG
        .iter()
        .take(21)
        .map(|insight| insight.id())
        .collect();
    assert_eq!(a08[0], "I01");
    assert_eq!(a08[20], "I21");
    assert_eq!(CATALOG[24].id(), "I25");
    let store = AnalyticsStore::open_in_memory("catalog").unwrap();
    let results = catalog(&store);
    let ids: Vec<_> = results.iter().map(|result| result.id.to_string()).collect();
    assert_eq!(
        ids,
        (1..=25).map(|id| format!("I{id:02}")).collect::<Vec<_>>()
    );
    let expect = [
        ("I01", "empty population"),
        ("I02", "empty population"),
        ("I03", "empty population"),
        ("I04", "empty population"),
        ("I05", "empty population"),
        ("I06", "empty population"),
        ("I07", "empty population"),
        ("I08", "empty population"),
        ("I09", "empty population"),
        ("I10", "empty population"),
        ("I11", "empty population"),
        ("I12", "empty population"),
        ("I13", "empty population"),
        ("I14", "empty population"),
        ("I15", "not_observable"),
        ("I16", "empty population"),
        ("I17", "NOT_ASSESSED"),
        ("I18", "empty population"),
        ("I19", "not_observable"),
        ("I20", "not_observable"),
        ("I21", "empty population"),
        ("I22", "insufficient_sample"),
        ("I23", "insufficient_sample"),
        ("I24", "NOT_ASSESSED"),
        ("I25", "empty population"),
    ];
    for (id, expected) in expect {
        let result = by_id(&results, id);
        assert_eq!(reason(&result.status), Some(expected), "{id}");
    }
    let i15 = by_id(&results, "I15");
    assert!(matches!(
        i15.status,
        InsightStatus::Refused { ref reason } if reason == "not_observable"
    ));
    assert_eq!(display_status(&i15.status), "insufficient_evidence");
    let i24 = by_id(&results, "I24");
    assert_ne!(reason(&i24.status), Some("not_observable"));
    assert_eq!(display_status(&i24.status), "not_applicable");
    assert!(i24.coverage.contains("dependency exposure"));
    assert!(int_at(i24, "I24.downstream_sessions").is_none());
    assert!(i24.measures.iter().any(|measure| {
        measure.id == "I24.artifacts"
            && measure.value.is_none()
            && measure.basis.contains("not_observable")
    }));
    assert!(i24
        .measures
        .iter()
        .all(|measure| !measure.name.to_ascii_lowercase().contains("pagerank")));
}

#[test]
fn display_status_covers_every_a14_row() {
    let rows = [
        (
            InsightStatus::Refused {
                reason: "not_observable".into(),
            },
            "insufficient_evidence",
        ),
        (
            InsightStatus::Refused {
                reason: "insufficient_sample".into(),
            },
            "insufficient_evidence",
        ),
        (
            InsightStatus::Refused {
                reason: "not_applicable".into(),
            },
            "not_applicable",
        ),
        (
            InsightStatus::Partial {
                reason: "NOT_ASSESSED".into(),
            },
            "not_applicable",
        ),
        (
            InsightStatus::Partial {
                reason: "threshold_exceeded".into(),
            },
            "partial",
        ),
        (
            InsightStatus::Refused {
                reason: "noncomparable".into(),
            },
            "insufficient_evidence",
        ),
        (InsightStatus::Computed, "ok"),
        (
            InsightStatus::Partial {
                reason: "missing context limit".into(),
            },
            "partial",
        ),
        (
            InsightStatus::Refused {
                reason: "unpinned tier profile".into(),
            },
            "refused",
        ),
    ];
    for (status, badge) in rows {
        assert_eq!(display_status(&status), badge, "{status:?}");
    }
}

#[test]
fn populated_catalog_computes_i01_through_i25_contracts() {
    let (_root, store) = writable("main");
    sql(
        &store,
        "
        INSERT INTO usage_facts (
            source_key, harness, native_session, message_id, ts, seq,
            input_uncached, output_tokens, cache_read, cache_write, reasoning,
            currency_total, generation, source_id, model, effort
        ) VALUES
        ('u-p','claude','P','m-p',TIMESTAMP '2026-01-01 00:00:00',1,1,0,0,0,0,NULL,1,'src','alpha','high'),
        ('u-a','claude','A','m-a',TIMESTAMP '2026-01-01 00:00:01',2,1,0,0,0,0,NULL,1,'src','beta','high'),
        ('u-b','claude','B','m-b',TIMESTAMP '2026-01-01 00:00:02',3,1,0,0,0,0,NULL,1,'src',NULL,NULL),
        ('u-d','claude','D','m-d',TIMESTAMP '2026-01-01 00:00:03',4,1,0,0,0,0,NULL,1,'src',NULL,NULL),
        ('u-solo','claude','solo',NULL,TIMESTAMP '2026-01-01 00:00:04',5,0,0,0,0,0,NULL,1,'src',NULL,NULL),
        ('u-cost','claude','item-accepted',NULL,TIMESTAMP '2026-01-01 00:00:05',6,0,0,0,0,0,10.0,1,'src',NULL,NULL);
        INSERT INTO session_edges (
            source_key, source_id, harness, parent_session, child_session, relation, status, evidence_kind, generation
        ) VALUES
        ('e1','src','claude','P','A','spawn','evidenced','explicit',1),
        ('e2','src','claude','P','B','spawn','evidenced','explicit',1),
        ('e3','src','claude','A','D','delegate_existing','evidenced','explicit',1),
        ('e4','src','claude','B','D','delegate_existing','evidenced','explicit',1);
        INSERT INTO tool_facts (
            source_key, harness, native_session, ts, seq, tool_name, failed, structured_write, generation, source_id
        ) VALUES
        ('tool-1','claude','P',TIMESTAMP '2026-01-01 00:00:00',1,'bash',false,false,1,'src');
        INSERT INTO skill_facts (source_key, seat, skill, invoked) VALUES
        ('sk-load','builder','plan',false),
        ('sk-run','builder','plan',true);
        CREATE TABLE context_facts (
            source_key VARCHAR, harness VARCHAR, native_session VARCHAR, ts TIMESTAMP,
            context_used BIGINT, context_limit BIGINT
        );
        INSERT INTO context_facts VALUES
        ('c1','claude','P',TIMESTAMP '2026-01-01 00:00:00',50,100);
        INSERT INTO activity_facts (source_key, seat, ts, state, seq) VALUES
        ('a1','seat-a',TIMESTAMP '2026-01-01 00:00:00','running',1),
        ('a2','seat-a',TIMESTAMP '2026-01-01 00:00:10','idle',2),
        ('b1','seat-b',TIMESTAMP '2026-01-01 00:00:00','running',1),
        ('b2','seat-b',TIMESTAMP '2026-01-01 00:00:10','idle',2);
        INSERT INTO queue_facts (
            source_key, transition_id, qitem_id, ts, state, source_seat, dest_seat, closure_reason, missions
        ) VALUES
        ('q1',1,'item-accepted',TIMESTAMP '2026-01-01 00:00:00','pending','s','d',NULL,'[]'),
        ('q2',2,'item-accepted',TIMESTAMP '2026-01-01 00:00:05','in-progress','s','d',NULL,'[]'),
        ('q3',3,'item-accepted',TIMESTAMP '2026-01-01 00:00:06','done','s','d','merged','[]'),
        ('q4',1,'item-attempt',TIMESTAMP '2026-01-01 00:00:00','pending','s','d',NULL,'[]'),
        ('q5',2,'item-attempt',TIMESTAMP '2026-01-01 00:00:05','in-progress','s','d','ran','[]');
        CREATE TABLE acceptance_rules (closure_reason VARCHAR, effect VARCHAR);
        INSERT INTO acceptance_rules VALUES ('merged','accepted'), ('ran','attempt_success');
        INSERT INTO process_identities (
            identity_key, root_key, pid, platform, start_time_unix_s, exe_name, harness,
            environment_kind, isolation_hint, first_seen, last_seen, visibility_limits
        ) VALUES
        ('i1','root',1,'linux',1,'worker','claude','container','none',TIMESTAMP '2026-01-01 00:00:00',TIMESTAMP '2026-01-01 00:00:10','none'),
        ('i2','root',2,'linux',2,'worker','claude','container','none',TIMESTAMP '2026-01-01 00:00:00',TIMESTAMP '2026-01-01 00:00:10','none');
        CREATE TABLE syscall_facts (
            source_key VARCHAR, grammar VARCHAR, operation VARCHAR, resource_class VARCHAR, result VARCHAR
        );
        INSERT INTO syscall_facts VALUES
        ('sy1','syscall_v1','open','file','success'),
        ('sy2','syscall_v1','open','file','nope'),
        ('sy3','other','open','file','success');
        INSERT INTO security_facts (
            source_key, source_id, harness, native_session, kind, value, source_field, generation
        ) VALUES
        ('s1','src','claude','P','permission_mode','default','permission_mode',1),
        ('s2','src','claude','P','sandbox_policy','workspace','sandbox_policy',1),
        ('s3','src','claude','P','isolation_test','passed','isolation_test',1),
        ('s4','src','claude','P','grant','revoked','grant',1);
        CREATE TABLE authority_rules (rule_id VARCHAR, kind VARCHAR, expected_value VARCHAR);
        INSERT INTO authority_rules VALUES ('r1','permission_mode','default');
        INSERT INTO warnings (path, line_no, reason) VALUES ('src-a', 1, 'redacted-label');
        ",
    );
    sql(&store, &trajectory_sql());
    sql(&store, &envelope_sql(5, 5, true));
    let results = catalog(&store);

    let i01 = by_id(&results, "I01");
    assert_eq!(reason(&i01.status), None);
    assert_eq!(int_at(i01, "I01.distinct_calls"), Some(4));
    assert_eq!(int_at(i01, "I01.usage_only"), Some(2));

    let i02 = by_id(&results, "I02");
    assert_eq!(reason(&i02.status), Some("undefined cache ratio"));
    assert_eq!(int_at(i02, "I02.alpha.input_uncached"), Some(1));

    let i03 = by_id(&results, "I03");
    assert_eq!(reason(&i03.status), None);
    assert_eq!(int_at(i03, "I03.node.P"), Some(4));
    assert_eq!(int_at(i03, "I03.node.A"), Some(2));
    assert_eq!(int_at(i03, "I03.node.B"), Some(2));
    assert_eq!(int_at(i03, "I03.node.D"), Some(1));
    assert_eq!(int_at(i03, "I03.fleet_deduped"), Some(4));

    assert!(by_id(&results, "I04").id == "I04");

    let i05 = by_id(&results, "I05");
    assert_eq!(reason(&i05.status), None);
    assert_eq!(int_at(i05, "I05.divergent_pairs"), Some(1));
    assert_eq!(int_at(i05, "I05.matched_pairs"), Some(0));

    let i06 = by_id(&results, "I06");
    assert_eq!(int_at(i06, "I06.loaded"), Some(1));
    assert_eq!(int_at(i06, "I06.invoked"), Some(1));
    assert_eq!(int_at(i06, "I06.intersection"), Some(1));
    near(float_at(i06, "I06.rate").unwrap(), 1.0);
    assert_eq!(int_at(i06, "I06.actions"), Some(1));

    let i07 = by_id(&results, "I07");
    assert_eq!(reason(&i07.status), None);
    near(float_at(i07, "I07.peak_ratio").unwrap(), 0.5);

    let i08 = by_id(&results, "I08");
    assert_eq!(reason(&i08.status), None);
    assert_eq!(int_at(i08, "I08.claude.failed"), Some(0));

    assert_eq!(
        int_at(by_id(&results, "I09"), "I09.relation.spawn"),
        Some(2)
    );

    let i10 = by_id(&results, "I10");
    assert_eq!(reason(&i10.status), None);
    near(float_at(i10, "I10.wall_seconds").unwrap(), 10.0);
    near(float_at(i10, "I10.summed_seconds").unwrap(), 20.0);
    near(float_at(i10, "I10.concurrency").unwrap(), 2.0);

    let i11 = by_id(&results, "I11");
    assert_eq!(reason(&i11.status), None);
    assert_eq!(int_at(i11, "I11.shared_tokens"), Some(1));
    assert_eq!(int_at(i11, "I11.residual_tokens"), Some(1));

    let i12 = by_id(&results, "I12");
    assert_eq!(reason(&i12.status), None);
    near(float_at(i12, "I12.wait_sum_seconds").unwrap(), 10.0);
    near(float_at(i12, "I12.wait_median_seconds").unwrap(), 5.0);
    near(float_at(i12, "I12.wall_seconds").unwrap(), 5.0);

    let i13 = by_id(&results, "I13");
    assert_eq!(reason(&i13.status), None);
    assert_eq!(int_at(i13, "I13.incarnations"), Some(2));
    assert_eq!(int_at(i13, "I13.rebuilds"), Some(1));
    assert_eq!(int_at(i13, "I13.unresolved"), Some(0));

    assert_eq!(
        reason(&by_id(&results, "I14").status),
        Some("empty population")
    );

    let i15 = by_id(&results, "I15");
    assert_eq!(reason(&i15.status), None);
    assert_eq!(int_at(i15, "I15.open.file.success"), Some(1));
    assert_eq!(int_at(i15, "I15.open.file.unknown"), Some(1));
    assert!(int_at(i15, "I15.open.file.failed").is_none());

    let i16 = by_id(&results, "I16");
    assert_eq!(reason(&i16.status), None);
    assert_eq!(int_at(i16, "I16.declared"), Some(1));
    assert_eq!(int_at(i16, "I16.configured"), Some(1));
    assert_eq!(int_at(i16, "I16.tested"), Some(1));

    let i17 = by_id(&results, "I17");
    assert_eq!(reason(&i17.status), None);
    assert_eq!(int_at(i17, "I17.consistent"), Some(1));
    assert_eq!(int_at(i17, "I17.mismatch"), Some(0));

    assert_ne!(
        reason(&by_id(&results, "I18").status),
        Some("empty population")
    );

    let i19 = by_id(&results, "I19");
    assert_eq!(reason(&i19.status), None);
    assert_eq!(int_at(i19, "I19.accepted"), Some(1));
    assert_eq!(int_at(i19, "I19.evaluated"), Some(2));
    assert_eq!(int_at(i19, "I19.attempt_success"), Some(1));
    near(float_at(i19, "I19.acceptance_rate").unwrap(), 0.5);

    let i20 = by_id(&results, "I20");
    assert_eq!(reason(&i20.status), None);
    near(float_at(i20, "I20.cost_per_accepted").unwrap(), 10.0);
    near(float_at(i20, "I20.mean_per_work").unwrap(), 10.0);

    let i21 = by_id(&results, "I21");
    assert_eq!(reason(&i21.status), Some("noncomparable"));
    assert_eq!(display_status(&i21.status), "insufficient_evidence");
    assert!(int_at(i21, "I21.claude.alpha").is_some());
    assert!(int_at(i21, "I21.claude.beta").is_some());

    let i22 = by_id(&results, "I22");
    assert_eq!(reason(&i22.status), None);
    assert_eq!(int_at(i22, "I22.tripped"), Some(1));
    assert_eq!(int_at(i22, "I22.cohort"), Some(5));
    assert_eq!(int_at(i22, "I22.capability_class_other"), Some(1));
    assert!(i22.coverage.contains("assumption:"));
    assert!(i22.coverage.contains("alternative:"));
    let l0 = i22.disclosure_l0();
    assert_eq!(l0.fields[0].0, "status");
    assert_eq!(l0.fields[1].1, "I22 harness.trajectory_vortex");
    assert!(l0.fields[2].1.contains("not a cause"));
    assert_eq!(l0.fields[3].1, "harness");
    let l1 = i22.disclosure_l1();
    assert!(l1.fields[0].1.contains("k=3"));
    assert!(l1.fields[1].1.contains("capability class"));
    assert_eq!(i22.disclosure_l2().fields[1].1, "1.0.0");
    assert_eq!(i22.disclosure_l3().fields[0].0, "formula");

    let i23 = by_id(&results, "I23");
    assert_eq!(reason(&i23.status), None);
    near(float_at(i23, "I23.h.v1.v2.l1").unwrap(), 3.0);
    assert_eq!(int_at(i23, "I23.h.v1.v2.added"), Some(1));
    assert_eq!(int_at(i23, "I23.h.v1.v2.removed"), Some(0));
    assert_eq!(int_at(i23, "I23.h.v1.v2.retyped"), Some(1));
    assert_eq!(int_at(i23, "I23.excluded"), Some(1));

    let i24 = by_id(&results, "I24");
    assert_eq!(reason(&i24.status), None);
    assert_eq!(display_status(&i24.status), "ok");
    assert_eq!(int_at(i24, "I24.downstream_sessions"), Some(3));
    assert!(i24.coverage.contains("dependency exposure"));
    assert!(i24
        .measures
        .iter()
        .any(|measure| measure.id == "I24.artifacts" && measure.value.is_none()));

    let i25 = by_id(&results, "I25");
    assert_eq!(reason(&i25.status), Some("unpinned tier profile"));
    assert_eq!(display_status(&i25.status), "refused");
    near(float_at(i25, "I25.observed_cache_read_ratio").unwrap(), 0.0);
    assert!(float_at(i25, "I25.simulated_cost").is_none());
    assert!(i25.coverage.contains("assumption:"));
    assert!(i25.coverage.contains("alternative:"));
    assert!(i25.coverage.contains("counterfactual"));
    let l0 = i25.disclosure_l0();
    assert_eq!(l0.fields[3].1, "model");
    assert!(i25.disclosure_l1().fields[0].1.contains("pinned"));
    assert_eq!(i25.disclosure_l3().fields[1].0, "population_what");
}

#[test]
fn shared_allocation_applies_rational_weights_and_rejects_excess() {
    let (_root, store) = writable("weights");
    sql(
        &store,
        "
        INSERT INTO usage_facts (
            source_key, harness, native_session, ts, seq,
            input_uncached, output_tokens, cache_read, cache_write, reasoning,
            generation, source_id
        ) VALUES
        ('u','claude','W',TIMESTAMP '2026-01-01 00:00:00',1,1000,0,0,0,0,1,'src');
        INSERT INTO session_edges (
            source_key, source_id, harness, parent_session, child_session, relation, status, evidence_kind, generation
        ) VALUES
        ('e1','src','claude','p1','W','spawn','evidenced','explicit',1),
        ('e2','src','claude','p2','W','spawn','evidenced','explicit',1);
        CREATE TABLE allocation_weights (
            worker_session VARCHAR, beneficiary VARCHAR, numerator BIGINT, denominator BIGINT
        );
        INSERT INTO allocation_weights VALUES ('W','alpha',3,5), ('W','beta',3,10);
        ",
    );
    let result = one(&store, "I11");
    assert_eq!(reason(&result.status), None);
    assert_eq!(int_at(&result, "I11.shared_tokens"), Some(1000));
    assert_eq!(int_at(&result, "I11.beneficiary.W.alpha"), Some(600));
    assert_eq!(int_at(&result, "I11.beneficiary.W.beta"), Some(300));
    assert_eq!(int_at(&result, "I11.residual_tokens"), Some(100));

    let (_root, excess) = writable("weights-excess");
    sql(
        &excess,
        "
        INSERT INTO usage_facts (
            source_key, harness, native_session, ts, seq,
            input_uncached, output_tokens, cache_read, cache_write, reasoning,
            generation, source_id
        ) VALUES
        ('u','claude','W',TIMESTAMP '2026-01-01 00:00:00',1,1000,0,0,0,0,1,'src');
        INSERT INTO session_edges (
            source_key, source_id, harness, parent_session, child_session, relation, status, evidence_kind, generation
        ) VALUES
        ('e1','src','claude','p1','W','spawn','evidenced','explicit',1),
        ('e2','src','claude','p2','W','spawn','evidenced','explicit',1);
        CREATE TABLE allocation_weights (
            worker_session VARCHAR, beneficiary VARCHAR, numerator BIGINT, denominator BIGINT
        );
        INSERT INTO allocation_weights VALUES ('W','alpha',7,10), ('W','beta',2,5);
        ",
    );
    let result = one(&excess, "I11");
    assert_eq!(reason(&result.status), Some("excessive weights"));
    assert_eq!(int_at(&result, "I11.shared_tokens"), Some(1000));
    assert_eq!(int_at(&result, "I11.residual_tokens"), Some(1000));
}

#[test]
fn trajectory_and_schema_short_samples_refuse() {
    let (_root, vortex) = writable("vortex-short");
    sql(&vortex, &trajectory_reference_only(4));
    let result = one(&vortex, "I22");
    assert_eq!(reason(&result.status), Some("insufficient_sample"));
    assert_eq!(display_status(&result.status), "insufficient_evidence");

    let (_root, drift) = writable("drift-short");
    sql(&drift, &envelope_sql(4, 5, false));
    let result = one(&drift, "I23");
    assert_eq!(reason(&result.status), Some("insufficient_sample"));
    assert_eq!(display_status(&result.status), "insufficient_evidence");
}

#[test]
fn blast_radius_skips_non_evidenced_edges_and_stops_at_the_hop_cap() {
    let (_root, store) = writable("blast");
    sql(
        &store,
        "
        INSERT INTO session_edges (
            source_key, source_id, harness, parent_session, child_session, relation, status, evidence_kind, generation
        ) VALUES
        ('e1','src','claude','root','kept','spawn','evidenced','explicit',1),
        ('e2','src','claude','kept','grand','delegate','evidenced','explicit',1),
        ('e3','src','claude','root','missing','spawn','absent','explicit',1),
        ('e4','src','claude','missing','hidden','spawn','evidenced','explicit',1);
        INSERT INTO security_facts (
            source_key, source_id, harness, native_session, kind, value, source_field, generation
        ) VALUES
        ('s1','src','claude','root','grant','revoked','grant',1);
        ",
    );
    let result = one(&store, "I24");
    assert_eq!(reason(&result.status), None);
    assert_eq!(int_at(&result, "I24.downstream_sessions"), Some(2));

    let (_root, capped) = writable("blast-cap");
    let mut edges = String::new();
    for index in 0..10 {
        edges.push_str(&format!(
            "('e{index}','src','claude','s{index}','s{}','spawn','evidenced','explicit',1),",
            index + 1
        ));
    }
    edges.pop();
    sql(
        &capped,
        &format!(
            "
            INSERT INTO session_edges (
                source_key, source_id, harness, parent_session, child_session, relation, status, evidence_kind, generation
            ) VALUES {edges};
            INSERT INTO security_facts (
                source_key, source_id, harness, native_session, kind, value, source_field, generation
            ) VALUES
            ('s','src','claude','s0','sandbox','unverified','sandbox',1);
            "
        ),
    );
    let result = one(&capped, "I24");
    assert_eq!(reason(&result.status), Some("hop limit exceeded"));
    assert_eq!(display_status(&result.status), "partial");
    assert_eq!(int_at(&result, "I24.downstream_sessions"), Some(8));
    assert_eq!(int_at(&result, "I24.truncated_frontier"), Some(1));

    let (_root, joined) = writable("blast-join");
    sql(
        &joined,
        "
        INSERT INTO session_edges (
            source_key, source_id, harness, parent_session, child_session, relation, status, evidence_kind, generation
        ) VALUES
        ('e1','src','claude','waiter','child','join','evidenced','explicit',1),
        ('e2','src','claude','child','grandchild','spawn','evidenced','explicit',1),
        ('e3','src','claude','waiter','other','spawn','evidenced','explicit',1);
        INSERT INTO security_facts (
            source_key, source_id, harness, native_session, kind, value, source_field, generation
        ) VALUES
        ('s','src','claude','child','grant','failed','grant',1);
        ",
    );
    let result = one(&joined, "I24");
    assert_eq!(int_at(&result, "I24.downstream_sessions"), Some(3));
}

#[test]
fn counterfactual_simulation_uses_the_pinned_fast_tier() {
    let (_root, store) = writable("route");
    sql(
        &store,
        "
        INSERT INTO usage_facts (
            source_key, harness, native_session, ts, seq,
            input_uncached, output_tokens, cache_read, cache_write, reasoning,
            generation, source_id, model, effort
        ) VALUES
        ('a','claude','s',TIMESTAMP '2026-01-01 00:00:00',1,100,10,0,0,0,1,'src','m','low'),
        ('b','claude','s',TIMESTAMP '2026-01-01 00:00:01',2,0,10,900,0,0,1,'src','m','low');
        CREATE TABLE _dendro_pricing_tiers (
            tier_id VARCHAR PRIMARY KEY,
            profile_pin VARCHAR NOT NULL,
            cohort_pin VARCHAR NOT NULL,
            currency VARCHAR NOT NULL,
            input_rate DOUBLE,
            output_rate DOUBLE
        );
        INSERT INTO _dendro_pricing_tiers VALUES ('fast','pin-1','claude','USD',1.0,2.0);
        ALTER TABLE _dendro_pricing_tiers ADD COLUMN context_limit BIGINT;
        UPDATE _dendro_pricing_tiers SET context_limit = 2000 WHERE tier_id = 'fast';
        ",
    );
    let result = one(&store, "I25");
    assert_eq!(reason(&result.status), None, "{:?}", result.status);
    near(
        float_at(&result, "I25.observed_cache_read_ratio").unwrap(),
        0.9,
    );
    near(
        float_at(&result, "I25.cache_bleed_inflection").unwrap(),
        0.9,
    );
    near(float_at(&result, "I25.simulated_cost").unwrap(), 0.00104);
    near(
        float_at(&result, "I25.simulated_context_ratio").unwrap(),
        0.45,
    );
    assert!(result.disclosure_l1().fields[1].1.contains("routing"));

    let (_root, mixed) = writable("route-mixed");
    sql(
        &mixed,
        "
        INSERT INTO usage_facts (
            source_key, harness, native_session, ts, seq,
            input_uncached, output_tokens, cache_read, cache_write, reasoning,
            generation, source_id
        ) VALUES
        ('a','claude','s',TIMESTAMP '2026-01-01 00:00:00',1,10,1,0,0,0,1,'src'),
        ('b','codex','s',TIMESTAMP '2026-01-01 00:00:01',2,10,1,0,0,0,1,'src');
        CREATE TABLE _dendro_pricing_tiers (
            tier_id VARCHAR PRIMARY KEY,
            profile_pin VARCHAR NOT NULL,
            cohort_pin VARCHAR NOT NULL,
            currency VARCHAR NOT NULL,
            input_rate DOUBLE,
            output_rate DOUBLE
        );
        INSERT INTO _dendro_pricing_tiers VALUES ('fast','pin-1','claude','USD',1.0,2.0);
        ",
    );
    let result = one(&mixed, "I25");
    assert_eq!(reason(&result.status), Some("noncomparable"));
    assert_eq!(display_status(&result.status), "insufficient_evidence");
    assert!(float_at(&result, "I25.observed_cache_read_ratio").is_some());
    assert!(float_at(&result, "I25.simulated_cost").is_none());
}

fn vector_literal(first: f64, second: f64) -> String {
    let mut values = vec![0.0; 32];
    values[0] = first;
    values[1] = second;
    let list = values
        .iter()
        .map(|value| format!("{value}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{list}]::FLOAT[32]")
}

fn trajectory_sql() -> String {
    let mut sql = String::from(
        "CREATE TABLE harness_trajectory (
            observation_id VARCHAR PRIMARY KEY,
            subject_key VARCHAR NOT NULL,
            turn_k BIGINT NOT NULL,
            embedding FLOAT[32] NOT NULL,
            session_key VARCHAR,
            harness VARCHAR,
            vec FLOAT[32]
        );",
    );
    let reference = vector_literal(1.0, 0.0);
    for index in 0..5 {
        sql.push_str(&format!(
            "INSERT INTO harness_trajectory (observation_id, subject_key, turn_k, embedding) VALUES ('r{index}','ref-{index}',0,{reference});"
        ));
    }
    let candidate = vector_literal(0.0, 1.0);
    for turn in 0..3 {
        sql.push_str(&format!(
            "INSERT INTO harness_trajectory (observation_id, subject_key, turn_k, embedding) VALUES ('c{turn}','cand',{turn},{candidate});"
        ));
    }
    let zero = vector_literal(0.0, 0.0);
    sql.push_str(&format!(
        "INSERT INTO harness_trajectory (observation_id, subject_key, turn_k, embedding) VALUES ('z0','other',0,{zero});"
    ));
    sql
}

fn trajectory_reference_only(count: i64) -> String {
    let mut sql = String::from(
        "CREATE TABLE harness_trajectory (
            observation_id VARCHAR PRIMARY KEY,
            subject_key VARCHAR NOT NULL,
            turn_k BIGINT NOT NULL,
            embedding FLOAT[32] NOT NULL,
            session_key VARCHAR,
            harness VARCHAR,
            vec FLOAT[32]
        );",
    );
    let reference = vector_literal(1.0, 0.0);
    for index in 0..count {
        sql.push_str(&format!(
            "INSERT INTO harness_trajectory (observation_id, subject_key, turn_k, embedding) VALUES ('r{index}','ref-{index}',0,{reference});"
        ));
    }
    let candidate = vector_literal(0.0, 1.0);
    for turn in 0..3 {
        sql.push_str(&format!(
            "INSERT INTO harness_trajectory (observation_id, subject_key, turn_k, embedding) VALUES ('c{turn}','cand',{turn},{candidate});"
        ));
    }
    sql
}

fn envelope_sql(left: i64, right: i64, null_row: bool) -> String {
    let mut sql = String::from(
        "CREATE TABLE harness_envelope (
            observation_id VARCHAR PRIMARY KEY,
            harness VARCHAR NOT NULL,
            harness_version VARCHAR NOT NULL,
            captured_at TIMESTAMP NOT NULL,
            envelope_shape VARIANT
        );",
    );
    for index in 0..left {
        sql.push_str(&format!(
            "INSERT INTO harness_envelope VALUES ('a{index}','h','v1',TIMESTAMP '2026-01-01 00:00:00', CAST(CAST('{{\"a\":1}}' AS JSON) AS VARIANT));"
        ));
    }
    for index in 0..right {
        sql.push_str(&format!(
            "INSERT INTO harness_envelope VALUES ('b{index}','h','v2',TIMESTAMP '2026-01-01 00:00:01', CAST(CAST('{{\"a\":\"x\",\"b\":true}}' AS JSON) AS VARIANT));"
        ));
    }
    if null_row {
        sql.push_str(
            "INSERT INTO harness_envelope VALUES ('n0','h','v1',TIMESTAMP '2026-01-01 00:00:02', NULL);",
        );
    }
    sql
}
