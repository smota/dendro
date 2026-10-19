//! Synthetic analytics outputs and primary-schema-derived handoff holds.
use dendro_core::portfolio::{portfolio_json, portfolio_ndjson, portfolio_table, PortfolioSource};
use dendro_core::{AnalyticsStore, IdentityFact, QueueFact, QueueItemMeta, UsageFact};
use serde_json::json;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
const FROM: &str = "2026-01-01T00:00:00Z";
const TO: &str = "2026-01-01T01:00:00Z";
struct Fixture {
    dir: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/portfolio-fixtures")
            .join(format!(
                "{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        fs::create_dir_all(&dir).unwrap();
        Self {
            dir: dir.canonicalize().unwrap(),
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}
fn time(seconds: i64) -> chrono::DateTime<chrono::Utc> {
    dendro_core::parse_utc_time(FROM).unwrap() + chrono::Duration::seconds(seconds)
}
fn identity(store: &AnalyticsStore, seat: &str, rig: &str) {
    store
        .insert_identity(&IdentityFact {
            source_key: format!("identity:{rig}:{seat}"),
            harness: "codex".into(),
            native_session: seat.into(),
            rig: rig.into(),
            seat: seat.into(),
            ts: Some(time(0)),
        })
        .unwrap();
}
fn edge(store: &AnalyticsStore, id: &str, from: &str, to: &str, seq: i64) {
    store
        .insert_queue(&QueueFact {
            source_key: format!("edge:{id}"),
            transition_id: seq,
            qitem_id: id.into(),
            ts: time(seq),
            state: "done".into(),
            source_seat: from.into(),
            dest_seat: from.into(),
            closure_reason: Some("handed_off_to".into()),
            closure_target: Some(to.into()),
            chain_id: None,
            missions: vec![],
        })
        .unwrap();
}
fn usage(store: &AnalyticsStore, seat: &str, input: i64) {
    store
        .commit_file_batch(
            "source",
            "codex",
            "synthetic",
            1,
            0,
            "hash",
            None,
            &[UsageFact {
                source_key: format!("usage:{seat}"),
                harness: "codex".into(),
                native_session: seat.into(),
                message_id: None,
                ts: Some(time(2)),
                seq: 1,
                input_uncached: input,
                output: 10,
                cache_read: 0,
                cache_write: 0,
                reasoning: 0,
                currency_total: None,
                generation: 1,
                predecessor_source_key: None,
                predecessor_reason: None,
                prior_input_tokens: None,
                prior_cached_input_tokens: None,
                prior_cache_write_input_tokens: None,
                prior_output_tokens: None,
                prior_reasoning_output_tokens: None,
            }],
            &[],
        )
        .unwrap();
}
fn source(db: PathBuf, rigs: &[&str]) -> PortfolioSource {
    PortfolioSource {
        db,
        rigs: rigs.iter().map(|r| r.to_string()).collect(),
        hold_dir: None,
    }
}
#[test]
fn multiple_databases_preserve_exact_canonical_per_rig_metrics_and_matrix_counts() {
    let f = Fixture::new();
    let a = f.dir.join("worker.duckdb");
    let b = f.dir.join("pm.duckdb");
    let store = AnalyticsStore::open_path(&a, "synthetic").unwrap();
    for (seat, rig) in [
        ("supervisor", "kernel"),
        ("builder", "worker"),
        ("reviewer", "worker"),
        ("manager", "pm"),
    ] {
        identity(&store, seat, rig);
    }
    store
        .insert_seat_role(
            "supervisor",
            "other",
            "snapshots:role:1:agentflow-seat:steer.lead",
        )
        .unwrap();
    store
        .insert_seat_role(
            "builder",
            "other",
            "snapshots:role:2:agentflow-seat:dev.build",
        )
        .unwrap();
    edge(&store, "seed", "supervisor", "builder", 10);
    edge(&store, "review", "builder", "reviewer", 20);
    edge(&store, "seam", "builder", "manager", 30);
    usage(&store, "builder", 100);
    drop(store);
    let store = AnalyticsStore::open_path(&b, "synthetic").unwrap();
    identity(&store, "manager", "pm");
    usage(&store, "manager", 25);
    drop(store);
    let mappings = [
        source(a.clone(), &["worker", "kernel"]),
        source(b.clone(), &["pm"]),
    ];
    let report = portfolio_json(&mappings, FROM, TO).unwrap();
    let rows = report["payload"]["rigs"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    for row in rows {
        let rig = row["rig"].as_str().unwrap();
        let db = if rig == "pm" { &b } else { &a };
        let canonical = dendro_core::report_json(db, "decisions", Some(rig), FROM, TO).unwrap();
        assert_eq!(row["canonical"], canonical);
        assert_eq!(row["tokens"], canonical["payload"]["tokens"]);
        assert_eq!(
            row["ceremony_ratio"],
            canonical["payload"]["tool_categories"]["ceremony_ratio"]
        );
    }
    let matrix = &report["payload"]["matrix"];
    assert_eq!(matrix["edges"].as_array().unwrap().len(), 3);
    assert_eq!(matrix["counts"]["in_ring"], 1);
    assert_eq!(matrix["counts"]["cross_rig_seam"], 1);
    assert_eq!(matrix["counts"]["supervisory_plane"], 1);
    assert_eq!(
        report["payload"]["supervisory_dispatch"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let ndjson = portfolio_ndjson(&report).unwrap();
    assert_eq!(ndjson.lines().count(), 4);
    for line in ndjson.lines() {
        serde_json::from_str::<serde_json::Value>(line).unwrap();
    }
    let table = portfolio_table(&report).unwrap();
    assert_eq!(table.lines().count(), 4);
    assert!(table.contains("ceremony_ratio"));
}
#[test]
fn duplicate_database_or_rig_mappings_refuse_double_counting() {
    let f = Fixture::new();
    let a = f.dir.join("a.duckdb");
    let b = f.dir.join("b.duckdb");
    drop(AnalyticsStore::open_path(&a, "test").unwrap());
    drop(AnalyticsStore::open_path(&b, "test").unwrap());
    assert!(portfolio_json(
        &[source(a.clone(), &["a"]), source(a.clone(), &["b"])],
        FROM,
        TO
    )
    .is_err());
    assert!(portfolio_json(&[source(a, &["a"]), source(b, &["a"])], FROM, TO).is_err());
}
#[test]
fn names_and_unknown_memberships_do_not_invent_supervisory_bypasses() {
    let f = Fixture::new();
    let db = f.dir.join("analytics.duckdb");
    let store = AnalyticsStore::open_path(&db, "test").unwrap();
    identity(&store, "operator-agent@kernel", "kernel");
    identity(&store, "dev-build@worker", "worker");
    edge(
        &store,
        "fake-role",
        "operator-agent@kernel",
        "dev-build@worker",
        10,
    );
    edge(&store, "unknown", "outsider", "dev-build@worker", 20);
    drop(store);
    let report = portfolio_json(&[source(db, &["kernel", "worker"])], FROM, TO).unwrap();
    assert_eq!(report["payload"]["matrix"]["counts"]["no_data"], 1);
    assert_eq!(report["payload"]["matrix"]["counts"]["cross_rig_seam"], 1);
    assert!(report["payload"]["supervisory_dispatch"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(report["payload"]["supervisory_dispatch_status"], "no_data");
}
#[test]
fn native_hold_schema_is_bounded_read_only_traced_and_not_claimed_live() {
    let f = Fixture::new();
    let db = f.dir.join("a.duckdb");
    let store = AnalyticsStore::open_path(&db, "test").unwrap();
    identity(&store, "sender", "pm");
    identity(&store, "target", "worker");
    drop(store);
    let holds = f.dir.join("holds");
    fs::create_dir(&holds).unwrap();
    let record = json!({"id":"synthetic-hold","state":"delivered","rig":"worker","target":"target","sender":"sender","createdAt":FROM,"deliveredAt":"2026-01-01T00:01:00Z","signals":[{"id":"B1","detail":"PRIVATE_TRANSCRIPT"},{"id":"B3","detail":"sk-SYNTHETIC_PRIVATE"}],"body":"PRIVATE_TRANSCRIPT","goal":"/private/goal"});
    let file = holds.join("hold.json");
    let bytes = serde_json::to_vec(&record).unwrap();
    fs::write(&file, &bytes).unwrap();
    fs::write(holds.join("malformed.json"), b"bad-json").unwrap();
    let mut mapping = source(db, &["pm", "worker"]);
    mapping.hold_dir = Some(holds.clone());
    let report = portfolio_json(&[mapping], FROM, TO).unwrap();
    let admission = &report["payload"]["admission_holds"];
    assert_eq!(admission["records"].as_array().unwrap().len(), 1);
    assert_eq!(admission["records"][0]["completed_elapsed_secs"], 60);
    assert_eq!(admission["heatmap"].as_array().unwrap().len(), 2);
    assert_eq!(admission["heatmap"][0]["from_rig"], "pm");
    assert_eq!(
        admission["records"][0]["status"],
        "observed_record_not_live_verification"
    );
    assert!(report.to_string().find("PRIVATE_TRANSCRIPT").is_none());
    assert!(report.to_string().find("sk-SYNTHETIC").is_none());
    assert!(report.to_string().find("/private/goal").is_none());
    assert_eq!(fs::read(file).unwrap(), bytes);
}
#[cfg(unix)]
#[test]
fn hold_symlinks_are_not_followed_and_missing_holds_are_no_data() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let db = f.dir.join("a.duckdb");
    drop(AnalyticsStore::open_path(&db, "test").unwrap());
    let holds = f.dir.join("holds");
    fs::create_dir(&holds).unwrap();
    symlink(
        "/not-authorized/DO_NOT_FOLLOW.json",
        holds.join("link.json"),
    )
    .unwrap();
    let mut mapping = source(db, &["worker"]);
    mapping.hold_dir = Some(holds);
    let report = portfolio_json(&[mapping], FROM, TO).unwrap();
    assert_eq!(report["payload"]["admission_holds"]["status"], "no_data");
    assert!(report["payload"]["warnings"]
        .as_array()
        .unwrap()
        .contains(&json!("symlink-hold-skipped")));
}

#[test]
fn foreign_database_roles_do_not_upgrade_a_local_unknown_role() {
    let f = Fixture::new();
    let a = f.dir.join("a.duckdb");
    let b = f.dir.join("b.duckdb");
    let store = AnalyticsStore::open_path(&a, "test").unwrap();
    identity(&store, "same-name", "worker");
    identity(&store, "builder", "worker");
    store
        .insert_seat_role("builder", "other", "agentflow-seat:dev.build")
        .unwrap();
    edge(&store, "local", "same-name", "builder", 10);
    drop(store);
    let store = AnalyticsStore::open_path(&b, "test").unwrap();
    identity(&store, "same-name", "kernel");
    store
        .insert_seat_role("same-name", "other", "agentflow-seat:steer.lead")
        .unwrap();
    drop(store);
    let report =
        portfolio_json(&[source(a, &["worker"]), source(b, &["kernel"])], FROM, TO).unwrap();
    assert!(report["payload"]["supervisory_dispatch"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(report["payload"]["matrix"]["counts"]["in_ring"], 1);
}

#[test]
fn duplicated_recorded_openrig_origin_across_outputs_is_refused() {
    let f = Fixture::new();
    let a = f.dir.join("a.duckdb");
    let b = f.dir.join("b.duckdb");
    let origin = f.dir.join("synthetic-openrig.sqlite");
    for db in [&a, &b] {
        let store = AnalyticsStore::open_path(db, "test").unwrap();
        store
            .commit_file_batch(
                "openrig-origin",
                "openrig",
                origin.to_str().unwrap(),
                1,
                0,
                "hash",
                None,
                &[],
                &[],
            )
            .unwrap();
    }
    let error =
        portfolio_json(&[source(a, &["worker"]), source(b, &["pm"])], FROM, TO).unwrap_err();
    assert!(error
        .to_string()
        .contains("duplicate recorded OpenRig origin"));
}

#[test]
fn holds_outside_window_and_conflicting_snapshots_are_not_counted() {
    let f = Fixture::new();
    let db = f.dir.join("a.duckdb");
    drop(AnalyticsStore::open_path(&db, "test").unwrap());
    let holds = f.dir.join("holds");
    fs::create_dir(&holds).unwrap();
    let record = json!({"id":"old","state":"held","rig":"worker","target":"target","sender":"sender","createdAt":"2025-01-01T00:00:00Z","signals":[{"id":"B1"}]});
    fs::write(holds.join("old.json"), serde_json::to_vec(&record).unwrap()).unwrap();
    let mut mapping = source(db, &["worker"]);
    mapping.hold_dir = Some(holds);
    let report = portfolio_json(&[mapping], FROM, TO).unwrap();
    assert!(report["payload"]["admission_holds"]["records"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[test]
fn proven_failed_receiver_wake_emits_a_traced_supervisory_relay_observation() {
    let f = Fixture::new();
    let db = f.dir.join("a.duckdb");
    let store = AnalyticsStore::open_path(&db, "test").unwrap();
    identity(&store, "builder", "worker");
    identity(&store, "supervisor", "kernel");
    store
        .insert_seat_role("builder", "other", "agentflow-seat:dev.build")
        .unwrap();
    store
        .insert_seat_role("supervisor", "other", "agentflow-seat:steer.lead")
        .unwrap();
    edge(&store, "relay", "builder", "supervisor", 10);
    store
        .insert_queue(&QueueFact {
            source_key: "events:queue.created:11".into(),
            transition_id: 11,
            qitem_id: "receiver".into(),
            ts: time(11),
            state: "pending".into(),
            source_seat: "builder".into(),
            dest_seat: "supervisor".into(),
            closure_reason: None,
            closure_target: None,
            chain_id: Some("relay".into()),
            missions: vec![],
        })
        .unwrap();
    store
        .record_queue_item_meta(&QueueItemMeta {
            qitem_id: "receiver".into(),
            parent_qitem_id: Some("relay".into()),
            last_nudge_result: Some("failed".into()),
            ..QueueItemMeta::default()
        })
        .unwrap();
    drop(store);
    let report = portfolio_json(&[source(db, &["worker", "kernel"])], FROM, TO).unwrap();
    let dispatch = report["payload"]["supervisory_dispatch"]
        .as_array()
        .unwrap();
    assert_eq!(dispatch.len(), 1);
    assert_eq!(dispatch[0]["classification"], "dead_supervisory_relay");
    assert_eq!(dispatch[0]["source_seat"], "builder");
    assert_eq!(dispatch[0]["dest_seat"], "supervisor");
    assert_eq!(dispatch[0]["pending_duration_secs"], 3589);
    assert!(!dispatch[0]["evidence"].as_array().unwrap().is_empty());
}

#[test]
fn duplicate_hold_copies_count_once_and_conflicting_semantics_are_excluded() {
    let f = Fixture::new();
    let db = f.dir.join("a.duckdb");
    drop(AnalyticsStore::open_path(&db, "test").unwrap());
    let holds = f.dir.join("holds");
    fs::create_dir(&holds).unwrap();
    let record = json!({"id":"same","state":"held","rig":"worker","target":"target","sender":"sender","createdAt":FROM,"signals":[{"id":"B1"}]});
    fs::write(holds.join("a.json"), serde_json::to_vec(&record).unwrap()).unwrap();
    fs::write(holds.join("b.json"), serde_json::to_vec(&record).unwrap()).unwrap();
    let mut mapping = source(db, &["worker"]);
    mapping.hold_dir = Some(holds.clone());
    let report = portfolio_json(&[mapping.clone()], FROM, TO).unwrap();
    let records = report["payload"]["admission_holds"]["records"]
        .as_array()
        .unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["source_keys"].as_array().unwrap().len(), 2);
    let trace_keys = &report["payload"]["admission_holds"]["heatmap"][0]["source_keys"];
    assert_eq!(trace_keys.as_array().unwrap().len(), 2);
    assert_eq!(trace_keys, &records[0]["source_keys"]);
    assert_eq!(records[0]["observed_record_age_secs"], 3600);
    let mut conflict = record;
    conflict["state"] = json!("delivered");
    conflict["deliveredAt"] = json!("2026-01-01T00:01:00Z");
    fs::write(holds.join("c.json"), serde_json::to_vec(&conflict).unwrap()).unwrap();
    let report = portfolio_json(&[mapping], FROM, TO).unwrap();
    assert!(report["payload"]["admission_holds"]["records"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(report["payload"]["warnings"]
        .as_array()
        .unwrap()
        .contains(&json!("conflicting-hold-records-excluded")));
}

#[test]
fn selected_rig_names_never_admit_an_unselected_foreign_database_rig() {
    let f = Fixture::new();
    let a = f.dir.join("a.duckdb");
    let b = f.dir.join("b.duckdb");
    let store = AnalyticsStore::open_path(&a, "test").unwrap();
    identity(&store, "a-builder", "worker");
    identity(&store, "a-reviewer", "worker");
    edge(&store, "selected", "a-builder", "a-reviewer", 10);
    drop(store);
    let store = AnalyticsStore::open_path(&b, "test").unwrap();
    identity(&store, "b-supervisor", "worker");
    identity(&store, "b-builder", "worker");
    identity(&store, "b-manager", "pm");
    store
        .insert_seat_role("b-supervisor", "other", "agentflow-seat:steer.lead")
        .unwrap();
    store
        .insert_seat_role("b-builder", "other", "agentflow-seat:dev.build")
        .unwrap();
    edge(&store, "unselected", "b-supervisor", "b-builder", 20);
    drop(store);
    let report = portfolio_json(&[source(a, &["worker"]), source(b, &["pm"])], FROM, TO).unwrap();
    let matrix = &report["payload"]["matrix"];
    assert_eq!(matrix["edges"].as_array().unwrap().len(), 1);
    assert_eq!(matrix["edges"][0]["qitem_id"], "selected");
    assert_eq!(matrix["coverage"]["canonical_edges"], 2);
    assert_eq!(matrix["coverage"]["excluded_unselected"], 1);
    assert_eq!(matrix["counts"]["supervisory_plane"], 0);
    assert!(report["payload"]["supervisory_dispatch"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[test]
fn unknown_both_endpoints_remain_counted_and_prevent_measured_negative_status() {
    let f = Fixture::new();
    let db = f.dir.join("a.duckdb");
    let store = AnalyticsStore::open_path(&db, "test").unwrap();
    for seat in ["builder", "reviewer"] {
        identity(&store, seat, "worker");
    }
    store
        .insert_seat_role("builder", "other", "agentflow-seat:dev.build")
        .unwrap();
    store
        .insert_seat_role("reviewer", "check", "agentflow-seat:dev.qa")
        .unwrap();
    edge(&store, "proven", "builder", "reviewer", 10);
    edge(&store, "unknown", "x", "y", 20);
    drop(store);
    let report = portfolio_json(&[source(db, &["worker"])], FROM, TO).unwrap();
    let matrix = &report["payload"]["matrix"];
    assert_eq!(matrix["edges"].as_array().unwrap().len(), 2);
    assert_eq!(matrix["counts"]["in_ring"], 1);
    assert_eq!(matrix["counts"]["no_data"], 1);
    assert_eq!(matrix["coverage"]["excluded_unselected"], 0);
    assert_eq!(report["payload"]["supervisory_dispatch_status"], "no_data");
}

#[test]
fn missing_canonical_edge_key_is_counted_and_prevents_complete_status() {
    let f = Fixture::new();
    let db = f.dir.join("a.duckdb");
    let store = AnalyticsStore::open_path(&db, "test").unwrap();
    for seat in ["builder", "reviewer"] {
        identity(&store, seat, "worker");
    }
    store
        .insert_seat_role("builder", "other", "agentflow-seat:dev.build")
        .unwrap();
    store
        .insert_seat_role("reviewer", "check", "agentflow-seat:dev.qa")
        .unwrap();
    edge(&store, "proven", "builder", "reviewer", 10);
    store
        .insert_queue(&QueueFact {
            source_key: String::new(),
            transition_id: 20,
            qitem_id: "missing-key".into(),
            ts: time(20),
            state: "done".into(),
            source_seat: "builder".into(),
            dest_seat: "builder".into(),
            closure_reason: Some("handed_off_to".into()),
            closure_target: Some("reviewer".into()),
            chain_id: None,
            missions: vec![],
        })
        .unwrap();
    drop(store);
    let report = portfolio_json(&[source(db, &["worker"])], FROM, TO).unwrap();
    assert_eq!(
        report["payload"]["matrix"]["coverage"]["skipped_missing_keys"],
        1
    );
    assert_eq!(report["payload"]["matrix"]["counts"]["no_data"], 1);
    assert_eq!(report["payload"]["supervisory_dispatch_status"], "no_data");
    assert!(report["payload"]["warnings"]
        .as_array()
        .unwrap()
        .contains(&json!("edge-required-key-missing")));
}

#[test]
fn unknown_explicit_role_observation_does_not_leave_a_false_unique_supervisor() {
    let f = Fixture::new();
    let db = f.dir.join("a.duckdb");
    let store = AnalyticsStore::open_path(&db, "test").unwrap();
    identity(&store, "supervisor", "kernel");
    identity(&store, "builder", "worker");
    store
        .insert_seat_role("supervisor", "other", "agentflow-seat:steer.lead")
        .unwrap();
    store
        .insert_seat_role("supervisor", "other", "agentflow-seat:unrecognized")
        .unwrap();
    store
        .insert_seat_role("builder", "other", "agentflow-seat:dev.build")
        .unwrap();
    edge(&store, "ambiguous-role", "supervisor", "builder", 10);
    drop(store);
    let report = portfolio_json(&[source(db, &["kernel", "worker"])], FROM, TO).unwrap();
    assert!(report["payload"]["supervisory_dispatch"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(
        report["payload"]["matrix"]["edges"][0]["role_status"],
        "no_data"
    );
    assert_eq!(report["payload"]["supervisory_dispatch_status"], "no_data");
}

#[test]
fn secret_shaped_selected_paths_are_refused_before_source_contents_are_opened() {
    let f = Fixture::new();
    let missing_secret = f.dir.join("ghp_SYNTHETIC_DO_NOT_OPEN.duckdb");
    let error = portfolio_json(&[source(missing_secret, &["worker"])], FROM, TO).unwrap_err();
    assert!(error.to_string().contains("secret-shaped"));
    let db = f.dir.join("a.duckdb");
    drop(AnalyticsStore::open_path(&db, "test").unwrap());
    let mut mapping = source(db, &["worker"]);
    mapping.hold_dir = Some(f.dir.join("ghp_SYNTHETIC_DO_NOT_OPEN"));
    let error = portfolio_json(&[mapping], FROM, TO).unwrap_err();
    assert!(error.to_string().contains("secret-shaped"));
}

#[test]
fn future_delivered_hold_endpoint_is_not_treated_as_measured_elapsed_time() {
    let f = Fixture::new();
    let db = f.dir.join("a.duckdb");
    drop(AnalyticsStore::open_path(&db, "test").unwrap());
    let holds = f.dir.join("holds");
    fs::create_dir(&holds).unwrap();
    let record = json!({"id":"future","state":"delivered","rig":"worker","target":"target","sender":"sender","createdAt":FROM,"deliveredAt":"2999-01-01T00:00:00Z","signals":[{"id":"B1"}]});
    fs::write(
        holds.join("future.json"),
        serde_json::to_vec(&record).unwrap(),
    )
    .unwrap();
    let mut mapping = source(db, &["worker"]);
    mapping.hold_dir = Some(holds);
    let report = portfolio_json(&[mapping], FROM, TO).unwrap();
    assert!(report["payload"]["admission_holds"]["records"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(report["payload"]["warnings"]
        .as_array()
        .unwrap()
        .contains(&json!("invalid-hold-interval")));
}

#[test]
fn sealed_sender_labels_remain_unknown_without_guessing_a_redacted_join() {
    let f = Fixture::new();
    let db = f.dir.join("a.duckdb");
    let store = AnalyticsStore::open_path(&db, "test").unwrap();
    identity(&store, "ghp_SYNTHETIC_SENDER", "pm");
    drop(store);
    let holds = f.dir.join("holds");
    fs::create_dir(&holds).unwrap();
    let record = json!({"id":"redacted-sender","state":"delivered","rig":"worker","target":"target","sender":"ghp_SYNTHETIC_SENDER","createdAt":FROM,"deliveredAt":"2026-01-01T00:01:00Z","signals":[{"id":"B1"}]});
    fs::write(
        holds.join("record.json"),
        serde_json::to_vec(&record).unwrap(),
    )
    .unwrap();
    let mut mapping = source(db, &["worker", "pm"]);
    mapping.hold_dir = Some(holds);
    let report = portfolio_json(&[mapping], FROM, TO).unwrap();
    assert!(
        report["payload"]["admission_holds"]["records"][0]["from_rig"].is_null(),
        "a raw export cannot recover a privacy-sealed canonical join without its key"
    );
    assert!(!report.to_string().contains("ghp_SYNTHETIC_SENDER"));
}

#[test]
fn dangling_symlink_hold_dir_is_refused() {
    let f = Fixture::new();
    let db = f.dir.join("a.duckdb");
    drop(AnalyticsStore::open_path(&db, "test").unwrap());
    let dangling = f.dir.join("dangling-holds");
    std::os::unix::fs::symlink(f.dir.join("missing-target"), &dangling).unwrap();
    let mut mapping = source(db, &["worker"]);
    mapping.hold_dir = Some(dangling);
    let err = portfolio_json(&[mapping], FROM, TO).unwrap_err();
    assert!(
        err.to_string().contains("symlink hold directory refused"),
        "unexpected error: {err}"
    );
}
