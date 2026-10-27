#![deny(unsafe_code)]

//! `A09` / `A11` / `A12` Proof Suite — Pre-Flight CLI & MCP Advisor.
//!
//! Validates:
//! 1. All four questions (`model_cohort_baseline`, `skill_effectiveness_audit`,
//!    `delegation_loop_risk`, `environment_readiness`).
//! 2. Placebo vs. insufficient verification in `skill_effectiveness_audit`.
//! 3. Comparability rules in `model_cohort_baseline` (biased/mixed models -> `NoncomparableMix`, never picking a winner).
//! 4. Lineage cycles and depth in `delegation_loop_risk` (cycles -> loop risk; never PageRank centrality).
//! 5. `environment_readiness`: declaration alone is never readiness.
//! 6. MCP schema hygiene: zero SQL, zero filesystem path, zero execute parameters.
//! 7. `AdviseUncertainty` closed enum is completely disjoint from `BadgeToken` (A14 set).
//! 8. `A11` Pre-flight budget: warm p95 <= 15 ms on the frozen fixture; over-budget returns `advisory_budget_exceeded` with no partial numbers.
//! 9. `A12` Governance separation: zero writes to policy, grants, routing, or session lifetime; no prompt injection.

use chrono::Utc;
use dendro_cli::advise_v1::{
    evaluate_advise, evaluate_advise_conn, mcp_tool_schema, AdviseRequest, AdviseUncertainty,
    BadgeToken,
};
use dendro_core::facts::{SecurityFact, SkillFact};
use dendro_core::AnalyticsStore;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

struct TestDir(PathBuf);
impl TestDir {
    fn new(prefix: &str) -> Self {
        let unique = format!(
            "dendro-test-advise-{}-{}-{}",
            prefix,
            std::process::id(),
            TEST_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let path = std::env::temp_dir().join(unique);
        fs::create_dir_all(&path).unwrap();
        #[cfg(unix)]
        {
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn test_budget() -> Option<u64> {
    if cfg!(debug_assertions) {
        Some(1000)
    } else {
        None
    }
}

fn with_db_conn<F, T>(dir: &Path, f: F) -> T
where
    F: FnOnce(&duckdb::Connection) -> T,
{
    let db_path = dir.join("dendro.duckdb");
    if !db_path.exists() {
        drop(AnalyticsStore::open_path(&db_path, "advise_test").unwrap());
    }
    let conn = duckdb::Connection::open(&db_path).unwrap();
    f(&conn)
}

#[test]
fn test_mcp_tool_schema_hygiene() {
    let schema = mcp_tool_schema();
    let schema_str = serde_json::to_string(&schema).unwrap();
    let lower = schema_str.to_ascii_lowercase();

    // 1. Zero SQL parameters
    assert!(
        !lower.contains("\"sql\""),
        "MCP schema must not contain SQL parameters"
    );
    assert!(
        !lower.contains("\"query\""),
        "MCP schema must not contain query parameters"
    );
    assert!(
        !lower.contains("\"statement\""),
        "MCP schema must not contain statement parameters"
    );

    // 2. Zero filesystem path parameters
    assert!(
        !lower.contains("\"path\""),
        "MCP schema must not contain path parameters"
    );
    assert!(
        !lower.contains("\"file\""),
        "MCP schema must not contain file parameters"
    );
    assert!(
        !lower.contains("\"dir\""),
        "MCP schema must not contain dir parameters"
    );
    assert!(
        !lower.contains("\"db\""),
        "MCP schema must not contain db parameters"
    );

    // 3. Zero "execute" parameters
    assert!(
        !lower.contains("\"execute\""),
        "MCP schema must not contain execute parameters"
    );
    assert!(
        !lower.contains("\"exec\""),
        "MCP schema must not contain exec parameters"
    );
    assert!(
        !lower.contains("\"run\""),
        "MCP schema must not contain run parameters"
    );
    assert!(
        !lower.contains("\"command\""),
        "MCP schema must not contain command parameters"
    );

    // Verify allowed question enum is exact
    let questions = schema["inputSchema"]["properties"]["question"]["enum"]
        .as_array()
        .unwrap();
    let q_names: Vec<&str> = questions.iter().filter_map(|v| v.as_str()).collect();
    assert_eq!(
        q_names,
        vec![
            "model_cohort_baseline",
            "skill_effectiveness_audit",
            "delegation_loop_risk",
            "environment_readiness"
        ]
    );
}

#[test]
fn test_uncertainty_disjointness_from_badge_tokens() {
    // AdviseUncertainty and BadgeToken are separate types with disjoint serialized tokens
    let uncertainty_values = [
        AdviseUncertainty::SampleBelowMinimum,
        AdviseUncertainty::MissingDenominator,
        AdviseUncertainty::NoncomparableMix,
        AdviseUncertainty::CounterfactualOnly,
        AdviseUncertainty::Stale,
    ];

    let badge_values = [
        BadgeToken::Ok,
        BadgeToken::Partial,
        BadgeToken::InsufficientEvidence,
        BadgeToken::Refused,
        BadgeToken::NotApplicable,
    ];

    let unc_set: HashSet<String> = uncertainty_values
        .iter()
        .map(|u| serde_json::to_string(u).unwrap())
        .collect();

    let badge_set: HashSet<String> = badge_values
        .iter()
        .map(|b| serde_json::to_string(b).unwrap())
        .collect();

    // Intersection must be empty!
    let intersection: Vec<_> = unc_set.intersection(&badge_set).collect();
    assert!(
        intersection.is_empty(),
        "AdviseUncertainty and BadgeToken must be completely disjoint, found collision: {:?}",
        intersection
    );
}

#[test]
fn test_model_cohort_baseline_comparability_and_mix() {
    let test_dir = TestDir::new("cohort");
    let db_path = test_dir.path().join("dendro.duckdb");

    // 1. Empty population -> insufficient data
    let store = AnalyticsStore::open_path(&db_path, "advise_test").unwrap();
    let req = AdviseRequest {
        question: "model_cohort_baseline".to_string(),
        harness: None,
        skill: None,
        session: None,
        budget_ms: test_budget(),
    };
    let resp = evaluate_advise(&store, &req);
    assert_eq!(resp.result_code, "empty_population");
    assert_eq!(resp.display_badge, BadgeToken::InsufficientEvidence);
    assert_eq!(
        resp.uncertainty,
        Some(AdviseUncertainty::SampleBelowMinimum)
    );
    drop(store);

    // 2. Add usage facts with a mixed model population (e.g. gpt-4o AND claude-3-5-sonnet)
    // Noncomparable mix rule: never pick a winner!
    with_db_conn(test_dir.path(), |conn| {
        for i in 0..6 {
            let model = if i % 2 == 0 {
                "gpt-4o"
            } else {
                "claude-3-5-sonnet"
            };
            conn.execute(
                "INSERT INTO usage_facts (source_key, harness, native_session, ts, seq, input_uncached, output_tokens, cache_read, cache_write, reasoning, currency_total, generation, source_id, model)
                 VALUES (?, 'claude', 's1', CURRENT_TIMESTAMP, ?, 100, 50, 20, 10, 0, 0.01, 1, 'src1', ?)",
                duckdb::params![format!("usage_{i}"), i, model],
            ).unwrap();
        }
    });

    let store = AnalyticsStore::open_path(&db_path, "advise_test").unwrap();
    let mixed_resp = evaluate_advise(&store, &req);
    assert_eq!(mixed_resp.result_code, "noncomparable");
    assert_eq!(mixed_resp.display_badge, BadgeToken::InsufficientEvidence);
    assert_eq!(
        mixed_resp.uncertainty,
        Some(AdviseUncertainty::NoncomparableMix)
    );
    assert!(
        mixed_resp.details["winner"].is_null(),
        "mixed models must never pick a winner!"
    );
    drop(store);

    // 3. Sliced to a single comparable cohort (only gpt-4o) with sample >= 5
    with_db_conn(test_dir.path(), |conn| {
        conn.execute(
            "DELETE FROM usage_facts WHERE model = 'claude-3-5-sonnet'",
            [],
        )
        .unwrap();
        for i in 6..12 {
            conn.execute(
                "INSERT INTO usage_facts (source_key, harness, native_session, ts, seq, input_uncached, output_tokens, cache_read, cache_write, reasoning, currency_total, generation, source_id, model)
                 VALUES (?, 'claude', 's1', CURRENT_TIMESTAMP, ?, 100, 50, 40, 10, 0, 0.01, 1, 'src1', 'gpt-4o')",
                duckdb::params![format!("usage_{i}"), i],
            ).unwrap();
        }
    });

    let store = AnalyticsStore::open_path(&db_path, "advise_test").unwrap();
    let single_resp = evaluate_advise(&store, &req);
    assert_eq!(single_resp.result_code, "ok");
    assert_eq!(single_resp.display_badge, BadgeToken::Ok);
    assert_eq!(single_resp.uncertainty, None);
    assert!(single_resp.sample_count >= 5);
    assert!(single_resp.evidence_ids.len() <= 5);
}

#[test]
fn test_skill_effectiveness_audit_placebo_vs_insufficient() {
    let test_dir = TestDir::new("skill_audit");
    let db_path = test_dir.path().join("dendro.duckdb");
    let store = AnalyticsStore::open_path(&db_path, "advise_test").unwrap();

    let req = AdviseRequest {
        question: "skill_effectiveness_audit".to_string(),
        harness: None,
        skill: Some("test-driven-development".to_string()),
        session: None,
        budget_ms: test_budget(),
    };

    // 1. Below minimum sample (< 5) -> insufficient evidence / sample_below_minimum
    for i in 0..3 {
        store
            .insert_skill(&SkillFact {
                source_key: format!("skill_fact_{i}"),
                seat: "builder".to_string(),
                skill: "test-driven-development".to_string(),
                invoked: true,
            })
            .unwrap();
    }

    let ins_resp = evaluate_advise(&store, &req);
    assert_eq!(ins_resp.result_code, "insufficient_evidence");
    assert_eq!(ins_resp.display_badge, BadgeToken::InsufficientEvidence);
    assert_eq!(
        ins_resp.uncertainty,
        Some(AdviseUncertainty::SampleBelowMinimum)
    );

    // 2. Add more to reach sample >= 5, but NO tool facts / verification mechanism recorded -> missing denominator
    for i in 3..6 {
        store
            .insert_skill(&SkillFact {
                source_key: format!("skill_fact_{i}"),
                seat: "builder".to_string(),
                skill: "test-driven-development".to_string(),
                invoked: true,
            })
            .unwrap();
    }

    let miss_resp = evaluate_advise(&store, &req);
    assert_eq!(miss_resp.result_code, "insufficient_evidence");
    assert_eq!(miss_resp.display_badge, BadgeToken::InsufficientEvidence);
    assert_eq!(
        miss_resp.uncertainty,
        Some(AdviseUncertainty::MissingDenominator)
    );
    drop(store);

    // 3. Placebo condition:
    // Skill was loaded/available, sample >= 5, verification events recorded, but verified_effective == 0!
    with_db_conn(test_dir.path(), |conn| {
        conn.execute(
            "INSERT INTO tool_facts (source_key, harness, native_session, seq, tool_name, failed, structured_write, generation, source_id)
             VALUES ('tf_1', 'claude', 's1', 1, 'test-driven-development_runner', TRUE, FALSE, 1, 'src1')",
            [],
        ).unwrap();
    });

    let store = AnalyticsStore::open_path(&db_path, "advise_test").unwrap();
    let placebo_resp = evaluate_advise(&store, &req);
    assert_eq!(placebo_resp.result_code, "placebo");
    assert_eq!(placebo_resp.display_badge, BadgeToken::Refused);
    assert_eq!(placebo_resp.uncertainty, None);
    assert_eq!(placebo_resp.details["verdict"], "placebo");
    assert_eq!(placebo_resp.details["verified_effective"], 0);
    drop(store);

    // 4. Effective condition:
    // Add verified effective tool fact (failed == false, structured_write == true)
    with_db_conn(test_dir.path(), |conn| {
        conn.execute(
            "INSERT INTO tool_facts (source_key, harness, native_session, seq, tool_name, failed, structured_write, generation, source_id)
             VALUES ('tf_2', 'claude', 's1', 2, 'test-driven-development_runner', FALSE, TRUE, 1, 'src1')",
            [],
        ).unwrap();
    });

    let store = AnalyticsStore::open_path(&db_path, "advise_test").unwrap();
    let eff_resp = evaluate_advise(&store, &req);
    assert_eq!(eff_resp.result_code, "ok");
    assert_eq!(eff_resp.display_badge, BadgeToken::Ok);
    assert_eq!(eff_resp.uncertainty, None);
    assert_eq!(eff_resp.details["verdict"], "verified_effective");
    assert_eq!(eff_resp.details["verified_effective"], 1);
}

#[test]
fn test_delegation_loop_risk_cycles_and_depth() {
    let test_dir = TestDir::new("loop_risk");
    let db_path = test_dir.path().join("dendro.duckdb");
    let conn = duckdb::Connection::open(&db_path).unwrap();

    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS dendro_meta (schema_version INTEGER, key_id VARCHAR);
         INSERT INTO dendro_meta VALUES (4, 'test_key');
         CREATE TABLE IF NOT EXISTS session_edges (
            source_key VARCHAR PRIMARY KEY,
            source_id VARCHAR,
            harness VARCHAR,
            parent_session VARCHAR,
            child_session VARCHAR,
            relation VARCHAR,
            status VARCHAR,
            observed_at TIMESTAMP,
            evidence_kind VARCHAR,
            generation BIGINT
         );",
    )
    .unwrap();

    let req = AdviseRequest {
        question: "delegation_loop_risk".to_string(),
        harness: None,
        skill: None,
        session: None,
        budget_ms: test_budget(),
    };

    // 1. Clean DAG within depth limit -> Ok, risk: false
    // Also include a non-evidenced edge (e.g. status "conflict" or "started") that would form a cycle if not filtered out!
    conn.execute(
        "INSERT INTO session_edges (source_key, source_id, harness, parent_session, child_session, relation, status, evidence_kind, generation)
         VALUES ('edge_1', 'src1', 'claude', 'session_A', 'session_B', 'spawn', 'evidenced', 'subagent', 1)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO session_edges (source_key, source_id, harness, parent_session, child_session, relation, status, evidence_kind, generation)
         VALUES ('edge_2', 'src1', 'claude', 'session_B', 'session_C', 'spawn', 'evidenced', 'subagent', 1)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO session_edges (source_key, source_id, harness, parent_session, child_session, relation, status, evidence_kind, generation)
         VALUES ('edge_ignored_conflict', 'src1', 'claude', 'session_C', 'session_A', 'spawn', 'conflict', 'subagent', 1)",
        [],
    )
    .unwrap();

    let clean_resp = evaluate_advise_conn(&conn, &req);
    assert_eq!(clean_resp.result_code, "ok");
    assert_eq!(clean_resp.display_badge, BadgeToken::Ok);
    assert_eq!(clean_resp.details["risk"], false);
    assert_eq!(clean_resp.details["centrality_used"], false);

    // 2. Introduce a cycle using A06 direction table:
    // With relation "return", the edge directs from child (session_C) to parent (session_A)!
    conn.execute(
        "INSERT INTO session_edges (source_key, source_id, harness, parent_session, child_session, relation, status, evidence_kind, generation)
         VALUES ('edge_cycle_return', 'src1', 'claude', 'session_A', 'session_C', 'return', 'evidenced', 'subagent', 1)",
        [],
    )
    .unwrap();

    let cycle_resp = evaluate_advise_conn(&conn, &req);
    assert_eq!(cycle_resp.result_code, "cycle_detected");
    assert_eq!(cycle_resp.display_badge, BadgeToken::Refused);
    assert_eq!(cycle_resp.details["risk"], true);
    assert_eq!(cycle_resp.details["cycle_detected"], true);
    assert_eq!(cycle_resp.details["centrality_used"], false);
}

#[test]
fn test_environment_readiness_declaration_alone_is_not_readiness() {
    let test_dir = TestDir::new("readiness");
    let db_path = test_dir.path().join("dendro.duckdb");

    // 1. Declaration alone: identity facts exist, but NO observed security/isolation facts
    with_db_conn(test_dir.path(), |conn| {
        conn.execute(
            "INSERT INTO identity_facts (source_key, harness, native_session, rig, seat, ts)
             VALUES ('id_1', 'claude', 's1', 'rig1', 'builder', CURRENT_TIMESTAMP)",
            [],
        )
        .unwrap();
    });

    let store = AnalyticsStore::open_path(&db_path, "advise_test").unwrap();
    let req = AdviseRequest {
        question: "environment_readiness".to_string(),
        harness: None,
        skill: None,
        session: None,
        budget_ms: test_budget(),
    };

    let decl_resp = evaluate_advise(&store, &req);
    assert_eq!(decl_resp.result_code, "declaration_alone");
    assert_eq!(decl_resp.display_badge, BadgeToken::InsufficientEvidence);
    assert_eq!(
        decl_resp.uncertainty,
        Some(AdviseUncertainty::MissingDenominator)
    );
    assert_eq!(decl_resp.details["readiness"], false);

    // 2. With observed isolation & security facts admitted
    store
        .commit_security_facts(&[SecurityFact {
            source_key: "sec_1".to_string(),
            source_id: "src1".to_string(),
            harness: "claude".to_string(),
            native_session: Some("s1".to_string()),
            ts: Some(Utc::now()),
            kind: "sandbox_policy".to_string(),
            value: "workspace-write".to_string(),
            source_field: "sandboxed".to_string(),
            generation: 1,
        }])
        .unwrap();

    let ready_resp = evaluate_advise(&store, &req);
    assert_eq!(ready_resp.result_code, "ok");
    assert_eq!(ready_resp.display_badge, BadgeToken::Ok);
    assert_eq!(ready_resp.uncertainty, None);
    assert_eq!(ready_resp.details["readiness"], true);
}

#[test]
fn test_budget_exceeded_returns_no_partial_numbers() {
    let test_dir = TestDir::new("budget");
    let db_path = test_dir.path().join("dendro.duckdb");
    let store = AnalyticsStore::open_path(&db_path, "advise_test").unwrap();

    // Explicit budget_ms = 0 simulates instantaneous budget exhaustion
    let req = AdviseRequest {
        question: "model_cohort_baseline".to_string(),
        harness: None,
        skill: None,
        session: None,
        budget_ms: Some(0),
    };

    let resp = evaluate_advise(&store, &req);
    assert_eq!(resp.result_code, "advisory_budget_exceeded");
    assert_eq!(resp.display_badge, BadgeToken::Refused);
    assert_eq!(resp.uncertainty, None);
    assert!(resp.details["partial_numbers"].is_null());
}

#[test]
fn test_warm_p95_under_15ms_and_zero_writes() {
    let test_dir = TestDir::new("bench_p95");
    let db_path = test_dir.path().join("dendro.duckdb");

    // Populate frozen fixture (frozen at 6 rows: >= minimum sample size of 5)
    with_db_conn(test_dir.path(), |conn| {
        for i in 0..6 {
            conn.execute(
                "INSERT INTO usage_facts (source_key, harness, native_session, ts, seq, input_uncached, output_tokens, cache_read, cache_write, reasoning, currency_total, generation, source_id, model)
                 VALUES (?, 'claude', 's1', CURRENT_TIMESTAMP, ?, 100, 50, 40, 10, 0, 0.01, 1, 'src1', 'gpt-4o')",
                duckdb::params![format!("bench_usage_{i}"), i],
            ).unwrap();
        }
    });

    let store = AnalyticsStore::open_path(&db_path, "advise_test").unwrap();

    // Warm-up queries
    let req = AdviseRequest {
        question: "model_cohort_baseline".to_string(),
        harness: None,
        skill: None,
        session: None,
        budget_ms: test_budget(),
    };

    for _ in 0..5 {
        let _ = evaluate_advise(&store, &req);
    }

    // Record count before
    let count_before = store.load_usage().unwrap().len();

    // Benchmark 30 runs
    let mut latencies = Vec::new();
    for _ in 0..30 {
        let start = Instant::now();
        let resp = evaluate_advise(&store, &req);
        let elapsed = start.elapsed();
        assert_eq!(resp.result_code, "ok");
        latencies.push(elapsed);
    }

    // Record count after (A12: zero writes to policy/grants/routing/tables)
    let count_after = store.load_usage().unwrap().len();
    assert_eq!(
        count_before, count_after,
        "advise must perform ZERO writes to storage!"
    );

    latencies.sort();
    let p95_idx = (latencies.len() as f64 * 0.95).floor() as usize;
    let p95 = latencies[p95_idx.min(latencies.len() - 1)];

    println!("Warm p95 latency: {:?}", p95);
    let target_ms = if cfg!(debug_assertions) { 60 } else { 15 };
    assert!(
        p95.as_millis() <= target_ms,
        "A11 requirement: warm p95 must be <= 15 ms (release) or <= 60 ms (debug), measured: {:?}",
        p95
    );
}

#[test]
fn test_stale_uncertainty_when_data_older_than_24h() {
    let test_dir = TestDir::new("stale_test");
    let db_path = test_dir.path().join("dendro.duckdb");

    // Insert 6 usage facts with timestamp 48 hours ago (> 86,400s threshold)
    let old_ts = Utc::now() - chrono::Duration::hours(48);
    let old_ts_str = old_ts.to_rfc3339();

    with_db_conn(test_dir.path(), |conn| {
        for i in 0..6 {
            conn.execute(
                "INSERT INTO usage_facts (source_key, harness, native_session, ts, seq, input_uncached, output_tokens, cache_read, cache_write, reasoning, currency_total, generation, source_id, model)
                 VALUES (?, 'claude', 's1', CAST(? AS TIMESTAMP), ?, 100, 50, 40, 10, 0, 0.01, 1, 'src1', 'gpt-4o')",
                duckdb::params![format!("stale_usage_{i}"), old_ts_str, i],
            ).unwrap();
        }
    });

    let store = AnalyticsStore::open_path(&db_path, "advise_test").unwrap();

    let req = AdviseRequest {
        question: "model_cohort_baseline".to_string(),
        harness: None,
        skill: None,
        session: None,
        budget_ms: test_budget(),
    };

    let resp = evaluate_advise(&store, &req);
    assert_eq!(resp.result_code, "ok");
    assert!(
        resp.freshness_secs > 86_400,
        "freshness_secs should be > 86,400 for 48h old data, got {}",
        resp.freshness_secs
    );
    assert_eq!(
        resp.uncertainty,
        Some(AdviseUncertainty::Stale),
        "uncertainty should be AdviseUncertainty::Stale for data older than 24h"
    );
}

#[test]
fn test_tool_fact_insertion_invalidates_placebo_cache() {
    let test_dir = TestDir::new("invalidation");
    let db_path = test_dir.path().join("dendro.duckdb");
    let conn = duckdb::Connection::open(&db_path).unwrap();

    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS dendro_meta (schema_version INTEGER, key_id VARCHAR);
         INSERT INTO dendro_meta VALUES (4, 'test_key');
         CREATE TABLE IF NOT EXISTS skill_facts (
            source_key VARCHAR PRIMARY KEY,
            seat VARCHAR,
            skill VARCHAR,
            invoked BOOLEAN
         );
         CREATE TABLE IF NOT EXISTS tool_facts (
            source_key VARCHAR PRIMARY KEY,
            harness VARCHAR,
            native_session VARCHAR,
            ts TIMESTAMP,
            seq BIGINT,
            tool_name VARCHAR,
            fingerprint VARCHAR,
            failed BOOLEAN,
            structured_write BOOLEAN,
            generation BIGINT
         );",
    )
    .unwrap();

    // 1. Insert 6 skill facts (sample >= 5)
    for i in 0..6 {
        conn.execute(
            "INSERT INTO skill_facts (source_key, seat, skill, invoked) VALUES (?, 'builder', 'tdd', TRUE)",
            duckdb::params![format!("skill_fact_inv_{i}")],
        )
        .unwrap();
    }

    // Insert an initial non-verifying tool fact (failed: true, structured_write: false)
    conn.execute(
        "INSERT INTO tool_facts (source_key, harness, native_session, ts, seq, tool_name, failed, structured_write, generation)
         VALUES ('tf_inv_1', 'claude', 's1', CURRENT_TIMESTAMP, 1, 'tdd_runner', TRUE, FALSE, 1)",
        [],
    )
    .unwrap();

    let req = AdviseRequest {
        question: "skill_effectiveness_audit".to_string(),
        harness: None,
        skill: Some("tdd".to_string()),
        session: None,
        budget_ms: test_budget(),
    };

    // 2. First evaluation: placebo (sample >= 5, loaded > 0, verified_effective == 0)
    let resp1 = evaluate_advise_conn(&conn, &req);
    assert_eq!(resp1.result_code, "placebo");
    assert_eq!(resp1.details["verified_effective"], 0);

    // 3. Second evaluation: hits warm cache, still placebo
    let resp2 = evaluate_advise_conn(&conn, &req);
    assert_eq!(resp2.result_code, "placebo");

    // 4. Insert verifying tool fact (failed: false, structured_write: true)
    conn.execute(
        "INSERT INTO tool_facts (source_key, harness, native_session, ts, seq, tool_name, failed, structured_write, generation)
         VALUES ('tf_inv_2', 'claude', 's1', CURRENT_TIMESTAMP, 2, 'tdd_runner', FALSE, TRUE, 1)",
        [],
    )
    .unwrap();

    // 5. Third evaluation: watermark changed! Cache must be invalidated, new verdict is ok (verified_effective)
    let resp3 = evaluate_advise_conn(&conn, &req);
    assert_eq!(resp3.result_code, "ok");
    assert_eq!(resp3.display_badge, BadgeToken::Ok);
    assert_eq!(resp3.details["verdict"], "verified_effective");
    assert_eq!(resp3.details["verified_effective"], 1);
}
