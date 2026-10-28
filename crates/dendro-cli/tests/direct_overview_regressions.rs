#![deny(unsafe_code)]
//! Issue101 repair regressions through persisted native tables, public loader/API/CLI.
#[path = "support/issue101_fixture.rs"]
mod fixture;
use fixture::*;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Output};

fn has(v: &Value, key: &str) -> bool {
    v.as_array().unwrap().iter().any(|v| v == key)
}
fn populated() -> Fixture {
    let f = Fixture::empty();
    f.populate();
    f
}
fn trace(f: &Fixture, expected: Option<&str>) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_dendro"));
    cmd.args(["trace", "--db"]).arg(&f.db).args([
        "--metric-id",
        "direct_overview",
        "--rig",
        RIG,
        "--from",
        FROM,
        "--to",
        TO,
    ]);
    if let Some(d) = expected {
        cmd.args(["--expect-evidence-digest", d]);
    }
    cmd.output().unwrap()
}

#[test]
fn b1_native_stall_and_unrelated_completion_are_not_nominal() {
    let f = populated();
    let ov = f.overview();
    assert_eq!(
        ov["critical_focus"]["category"], "Leaf Stall",
        "persisted idle+claimed leaf must reach canonical derivation"
    );
    assert_eq!(ov["critical_focus"]["entity_id"], SEAT);
    assert!(has(
        &ov["critical_focus"]["selected_source_keys"],
        "stall:claim"
    ));
    assert!(has(
        &ov["critical_focus"]["selected_source_keys"],
        "act:idle"
    ));
    let api = f.api(RIG);
    assert_eq!(
        api["critical_focus"]["category"], "Leaf Stall",
        "production API must derive incidents too"
    );
}

#[test]
fn b1_native_parent_with_active_child_derives_unparked_container_hold() {
    let f = Fixture::empty();
    f.base_identity();
    f.identity("id:review", "session-review", RIG, "review@test-rig");
    f.conn()
        .execute(
            "INSERT INTO seat_bounds VALUES (?,?::TIMESTAMP,?::TIMESTAMP)",
            duckdb::params![SEAT, FROM, TO],
        )
        .unwrap();
    f.activity("hold:idle", "2026-10-01T01:00:00Z", "idle", 1);
    f.queue((
        "events:queue.created:1",
        1,
        "parent",
        "2026-10-01T00:20:00Z",
        "pending",
        "[]",
        None,
        SEAT,
    ));
    f.queue((
        "hold:parent-claim",
        2,
        "parent",
        "2026-10-01T00:30:00Z",
        "in-progress",
        "[]",
        None,
        SEAT,
    ));
    f.queue((
        "events:queue.created:3",
        3,
        "child",
        "2026-10-01T01:20:00Z",
        "pending",
        "[]",
        None,
        "review@test-rig",
    ));
    f.queue((
        "hold:child-claim",
        4,
        "child",
        "2026-10-01T01:30:00Z",
        "in-progress",
        "[]",
        None,
        "review@test-rig",
    ));
    f.conn()
        .execute(
            "UPDATE queue_facts SET parent_qitem_id=? WHERE qitem_id=?",
            duckdb::params!["parent", "child"],
        )
        .unwrap();
    let ov = f.overview();
    assert_eq!(ov["critical_focus"]["category"], "Unparked Container Hold");
    assert_eq!(ov["critical_focus"]["entity_id"], SEAT);
    assert!(has(
        &ov["critical_focus"]["selected_source_keys"],
        "hold:child-claim"
    ));
}

#[test]
fn b1_empty_and_stale_evidence_cannot_assert_a_healthy_live_fleet() {
    let f = Fixture::empty();
    let empty = f.overview();
    assert_eq!(empty["critical_focus"]["focus_state"], "unavailable");
    assert_eq!(empty["live_alert"], false);
    let f = populated();
    let stale = f.api(RIG);
    assert_eq!(stale["snapshot"], true);
    assert_eq!(stale["live_alert"], false);
    assert!(stale["evidence_as_of"].is_string());
}

#[test]
fn b2_same_key_changed_tokens_refuses_old_digest_in_loader_and_cli() {
    let f = populated();
    let before = f.overview();
    f.conn()
        .execute(
            "UPDATE usage_facts SET input_uncached=? WHERE source_key=?",
            duckdb::params![999999, "usage:main"],
        )
        .unwrap();
    let after = f.overview();
    assert_ne!(
        before["digest"], after["digest"],
        "same source key with changed token content must change digest"
    );
    let old = before["digest"].as_str().unwrap();
    let mut req = request();
    req.expect_evidence_digest = Some(old.into());
    assert!(dendro_core::overview::evaluate_overview_snapshot_path(&f.db, &req).is_err());
    let out = trace(&f, Some(old));
    assert!(!out.status.success(), "stale copied replay must refuse");
    assert!(String::from_utf8_lossy(&out.stderr).contains("evidence digest mismatch"));
}

#[test]
fn b2_digest_covers_project_partition_and_money_distribution() {
    let f = populated();
    let before = f.overview();
    f.conn()
        .execute(
            "UPDATE process_identities SET cwd_fingerprint=? WHERE identity_key=?",
            duckdb::params!["fp-other", "process:build"],
        )
        .unwrap();
    assert_ne!(
        before["digest"],
        f.overview()["digest"],
        "fingerprint attribution is evidence content"
    );
    let f = Fixture::empty();
    f.base_identity();
    f.usage("money:a", "session-build", Some(FROM), 1, Some(10.0));
    f.usage("money:b", "session-build", Some(FROM), 1, Some(20.0));
    let before = f.overview();
    f.conn()
        .execute(
            "UPDATE usage_facts SET currency_total=? WHERE source_key=?",
            duckdb::params![20.0, "money:a"],
        )
        .unwrap();
    f.conn()
        .execute(
            "UPDATE usage_facts SET currency_total=? WHERE source_key=?",
            duckdb::params![10.0, "money:b"],
        )
        .unwrap();
    assert_ne!(
        before["digest"],
        f.overview()["digest"],
        "same total with redistributed per-source money must change digest"
    );
}

#[test]
fn b2_digest_covers_fractional_scope_effective_options_and_outcome() {
    let f = populated();
    let before = f.overview();
    let mut req = request();
    req.from = dt("2026-10-01T00:00:00.000001Z");
    assert_ne!(
        before["digest"],
        f.overview_with(&req)["digest"],
        "microsecond scope must not truncate to seconds"
    );
    let mut req = request();
    req.advisory_options.leaf_stall_secs = 2000;
    assert_ne!(
        before["digest"],
        f.overview_with(&req)["digest"],
        "effective options belong to evidence identity"
    );
    f.conn()
        .execute(
            "UPDATE tool_facts SET failed=? WHERE source_key=?",
            duckdb::params![true, "tool:read:ok"],
        )
        .unwrap();
    assert_ne!(
        before["digest"],
        f.overview()["digest"],
        "supported outcome facts belong to digest"
    );
}

#[test]
fn b3_historical_completion_survives_post_cutoff_reopen() {
    let f = Fixture::empty();
    f.base_identity();
    f.queue((
        "history:done",
        1,
        "history-item",
        "2026-10-01T12:00:00Z",
        "done",
        "[\"101\"]",
        Some("no-follow-on"),
        SEAT,
    ));
    f.queue((
        "history:future",
        2,
        "history-item",
        "2026-10-02T00:00:01Z",
        "pending",
        "[\"101\"]",
        None,
        SEAT,
    ));
    assert_eq!(
        f.overview()["throughput_and_cost"]["closed_issue_qitems"],
        1
    );
    assert_eq!(f.api(RIG)["throughput_and_cost"]["closed_issue_qitems"], 1);
    let out = trace(&f, None);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&out.stdout).unwrap()["value"]["throughput_and_cost"]
            ["closed_issue_qitems"],
        1
    );
}

#[test]
fn b3_completion_requires_positive_canonical_tags_and_successful_reason() {
    let f = Fixture::empty();
    f.base_identity();
    f.queue((
        "valid:done",
        1,
        "valid",
        "2026-10-01T01:00:00Z",
        "done",
        "[\"101\",\"91\"]",
        Some("no-follow-on"),
        SEAT,
    ));
    for (i, tags, reason) in [
        (2, "[\"101\"]", Some("denied")),
        (3, "[\"101\"]", Some("canceled")),
        (4, "[\"101\"]", Some("escalation")),
        (5, "[\"101\"]", Some("blocked_on")),
        (6, "[\"101\"]", None),
        (7, "[\"abc\"]", Some("no-follow-on")),
        (8, "[\"0\"]", Some("no-follow-on")),
        (9, "[\"-1\"]", Some("no-follow-on")),
        (10, "{invalid-json", Some("no-follow-on")),
    ] {
        f.queue((
            &format!("invalid:{i}"),
            i,
            &format!("invalid-item-{i}"),
            "2026-10-01T02:00:00Z",
            "done",
            tags,
            reason,
            SEAT,
        ));
    }
    let tc = f.overview()["throughput_and_cost"].clone();
    assert_eq!(
        tc["closed_issue_qitems"], 1,
        "invalid completion evidence cannot inflate the denominator"
    );
    assert_eq!(tc["closed_issue_qitem_ids"], json!(["valid"]));
    assert!(has(&tc["closed_issue_source_keys"], "valid:done"));
}

#[test]
fn b3_completion_dedup_recovery_handoff_and_half_open_scope() {
    let f = Fixture::empty();
    f.base_identity();
    f.identity("id:foreign", "foreign", "other-rig", "dev@other-rig");
    for (key, seq, item, ts, state, seat) in [
        ("scope:lower", 1, "valid", FROM, "done", SEAT),
        (
            "scope:duplicate",
            2,
            "valid",
            "2026-10-01T01:00:00Z",
            "done",
            SEAT,
        ),
        ("scope:upper", 3, "upper", TO, "done", SEAT),
        (
            "scope:foreign",
            4,
            "foreign",
            "2026-10-01T01:00:00Z",
            "done",
            "dev@other-rig",
        ),
        (
            "scope:handoff",
            5,
            "handoff",
            "2026-10-01T01:00:00Z",
            "handed-off",
            SEAT,
        ),
        (
            "scope:recovery",
            6,
            "qitem-recovery-synthetic",
            "2026-10-01T01:00:00Z",
            "done",
            SEAT,
        ),
    ] {
        f.queue((
            key,
            seq,
            item,
            ts,
            state,
            "[\"101\"]",
            Some("no-follow-on"),
            seat,
        ));
    }
    assert_eq!(
        f.overview()["throughput_and_cost"]["closed_issue_qitems"],
        1
    );
    assert_eq!(f.api(RIG)["throughput_and_cost"]["closed_issue_qitems"], 1);
}

#[test]
fn b3_context_out_of_window_invocation_cannot_reconcile_current_declaration() {
    let f = Fixture::empty();
    f.base_identity();
    f.usage(
        "context:now",
        "session-build",
        Some("2026-10-01T01:00:00Z"),
        1,
        Some(0.0),
    );
    f.skill("context:now:skill:0", "tool-alpha", false, SEAT);
    f.tool(
        "context:old",
        "session-build",
        Some("2026-09-30T23:59:59Z"),
        "tool-alpha",
        Some(false),
    );
    let ct = f.overview()["context_tax"].clone();
    assert_eq!(ct["uncalled_tools_count"], 1);
    assert!(!has(&ct["invocation_support_source_keys"], "context:old"));
    assert_eq!(f.api(RIG)["context_tax"]["uncalled_tools_count"], 1);
}

#[test]
fn b3_context_foreign_undated_or_ambiguous_is_unknown_not_measured_zero() {
    let f = Fixture::empty();
    f.base_identity();
    f.identity("id:foreign", "foreign", "other-rig", "dev@other-rig");
    f.skill("foreign:skill:0", "tool-alpha", false, "dev@other-rig");
    let ct = f.overview()["context_tax"].clone();
    assert!(ct["uncalled_tools_count"].is_null());
    assert_eq!(ct["declaration_status"], "unavailable");
    let f = Fixture::empty();
    f.base_identity();
    f.skill("undated:skill:0", "tool-alpha", false, SEAT);
    assert!(
        f.overview()["context_tax"]["uncalled_tools_count"].is_null(),
        "declaration has no dated supporting observation"
    );
    let f = Fixture::empty();
    f.base_identity();
    f.identity(
        "id:collision",
        "session-build",
        "other-rig",
        "dev@other-rig",
    );
    f.usage("ambiguous:decl", "session-build", Some(FROM), 1, Some(0.0));
    f.skill("ambiguous:decl:skill:0", "tool-alpha", false, SEAT);
    assert!(
        f.overview()["context_tax"]["uncalled_tools_count"].is_null(),
        "colliding session-rig binding must not imply supported declarations"
    );
}

#[test]
fn b3_context_later_supported_invocation_reconciles_and_known_zero_is_measured() {
    let f = Fixture::empty();
    f.base_identity();
    f.usage(
        "context:decl",
        "session-build",
        Some("2026-10-01T01:00:00Z"),
        1,
        Some(0.0),
    );
    f.skill("context:decl:skill:0", "tool-alpha", false, SEAT);
    f.tool(
        "context:later",
        "session-build",
        Some("2026-10-01T02:00:00Z"),
        "tool-alpha",
        Some(false),
    );
    let ct = f.overview()["context_tax"].clone();
    assert_eq!(ct["uncalled_tools_count"], 0);
    assert_eq!(ct["declaration_status"], "measured");
    assert!(has(&ct["invocation_support_source_keys"], "context:later"));
    assert!(ct["wasted_schema_tokens"].is_null());
    assert!(ct["wasted_cost_usd"].is_null());
}

#[test]
fn b4_native_activity_and_completion_are_partitioned_without_multiplication() {
    let f = populated();
    let ov = f.overview();
    let p = &ov["project_groups"][0];
    assert_eq!(p["cwd_fingerprint"], "fp-a81f");
    assert_eq!(p["total_tokens"], 160);
    assert_eq!(
        p["active_hours"], 1.0,
        "one observed running hour; idle/session age is not active time"
    );
    assert_eq!(p["closed_issue_qitems"], 3);
    f.process("process:same", "fp-a81f");
    let dup = f.overview();
    assert_eq!(dup["project_groups"][0]["active_hours"], 1.0);
    assert_eq!(dup["total_tokens"], 160);
    f.process("process:ambiguous", "fp-other");
    let ambiguous = f.overview();
    assert_eq!(ambiguous["unassigned_total_tokens"], 160);
    assert_eq!(ambiguous["all_projects_total_tokens"], 160);
    assert_eq!(ambiguous["unassigned_active_hours"], 1.0);
    assert_eq!(ambiguous["unassigned_closed_issue_qitems"], 3);
}

#[test]
fn b4_active_tool_yield_keeps_unknown_outcomes_distinct_from_success() {
    let f = populated();
    let ov = f.overview();
    let tools = ov["context_tax"]["active_tools"]
        .as_array()
        .expect("required active tool yield is absent");
    let read = tools.iter().find(|v| v["tool_name"] == "read").unwrap();
    assert_eq!(read["invocation_count"], 3);
    assert_eq!(read["error_count"], 1);
    assert_eq!(read["unknown_outcome_count"], 1);
    assert_eq!(
        read["error_rate"], 0.5,
        "one error of two known outcomes; unknown is not success"
    );
}

#[test]
fn b6_populated_api_cli_and_loader_agree_on_literal_values_and_provenance() {
    let f = populated();
    let before = fs::read(&f.db).unwrap();
    let ov = f.overview();
    let api = f.api(RIG);
    let out = trace(&f, None);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let cli: Value = serde_json::from_slice(&out.stdout).unwrap();
    let cli = &cli["value"];
    for actual in [&ov, &api, cli] {
        assert_eq!(actual["total_tokens"], 160);
        assert_eq!(actual["throughput_and_cost"]["closed_issue_qitems"], 3);
        assert_eq!(actual["throughput_and_cost"]["total_fleet_spend_usd"], 30.0);
        assert_eq!(
            actual["throughput_and_cost"]["avg_cost_per_closed_issue_qitem"],
            10.0
        );
        assert_eq!(actual["context_tax"]["uncalled_tools_count"], 2);
        assert!(actual["context_tax"]["wasted_schema_tokens"].is_null());
        assert_eq!(actual["usage_source_keys"], json!(["usage:main"]));
        assert!(!has(&actual["source_keys"], "usage:foreign"));
        assert_eq!(actual["digest"], ov["digest"]);
    }
    let replay = trace(&f, ov["digest"].as_str());
    assert!(
        replay.status.success(),
        "{}",
        String::from_utf8_lossy(&replay.stderr)
    );
    assert_eq!(
        fs::read(&f.db).unwrap(),
        before,
        "overview and trace must not alter persisted source bytes"
    );
}

#[test]
fn b6_pricing_unknown_and_known_zero_survive_native_api() {
    let f = Fixture::empty();
    f.base_identity();
    f.usage("zero", "session-build", Some(FROM), 0, Some(0.0));
    assert_eq!(
        f.api(RIG)["throughput_and_cost"]["total_fleet_spend_usd"],
        0.0
    );
    f.usage("unknown", "session-build", Some(FROM), 5, None);
    let ov = f.api(RIG);
    assert!(ov["throughput_and_cost"]["total_fleet_spend_usd"].is_null());
    assert!(ov["throughput_and_cost"]["avg_cost_per_closed_issue_qitem"].is_null());
    assert_eq!(ov["throughput_and_cost"]["money_status"], "partial");
}

fn file_sha256(path: &Path) -> String {
    let mut f = fs::File::open(path).unwrap();
    let mut h = Sha256::new();
    let mut b = [0u8; 65536];
    loop {
        let n = f.read(&mut b).unwrap();
        if n == 0 {
            break;
        }
        h.update(&b[..n]);
    }
    format!("{:x}", h.finalize())
}

#[test]
#[ignore = "explicit exporter for populated browser acceptance; not an acceptance pass"]
fn export_issue101_browser_fixture() {
    let path = std::env::var_os("DENDRO_ISSUE101_FIXTURE_DIR")
        .expect("set an unused absolute fixture path under this worktree .agent-runs/");
    let path = std::path::PathBuf::from(path);
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    assert!(
        path.is_absolute() && path.starts_with(repo.join(".agent-runs")),
        "fixture export stays inside this worktree scratch authority"
    );
    let mut f = Fixture::at(path);
    f.populate();
    let head = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&repo)
        .output()
        .unwrap();
    assert!(head.status.success());
    let clean = Command::new("git")
        .args([
            "diff",
            "--quiet",
            "HEAD",
            "--",
            "crates/*/src",
            "site/dashboards",
            "Cargo.toml",
            "Cargo.lock",
        ])
        .current_dir(&repo)
        .status()
        .unwrap();
    assert!(
        clean.success(),
        "receipt requires production inputs matching HEAD"
    );
    let binary = Path::new(env!("CARGO_BIN_EXE_dendro"))
        .canonicalize()
        .unwrap();
    let receipt = json!({"kind":"issue101-native-populated-fixture","source_sha":String::from_utf8(head.stdout).unwrap().trim(),"binary":binary,"binary_sha256":file_sha256(&binary),"database":f.db.canonicalize().unwrap(),"site_dir":repo.join("site"),"rig":RIG,"from":FROM,"to":TO,"literal_expected":{"tokens":160,"cost_usd":30,"closed_issue_qitems":3,"avg_cost_usd":10,"uncalled_tools_count":2,"active_hours":1,"fingerprint":"fp-a81f"}});
    fs::write(
        f.root.join("fixture.json"),
        serde_json::to_vec_pretty(&receipt).unwrap(),
    )
    .unwrap();
    f.keep = true;
    println!(
        "ISSUE101_FIXTURE_RECEIPT={}",
        f.root.join("fixture.json").display()
    );
}
