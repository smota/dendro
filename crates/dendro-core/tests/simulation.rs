#[path = "support/semantic_fixture.rs"]
mod fixture;
use dendro_core::{
    report_json,
    simulation::{self, BoundaryModel, CostWeights, Scenario, TierRule},
};
use serde_json::{json, Value};
fn report() -> Value {
    let sample = |key: &str, item: &str, child: &str, from: &str, to: &str, usage: &str| json!({"source_key":key,"qitem_id":item,"receiver_qitem_id":child,"from":from,"to":to,"root":"root","lineage_status":"complete","recovery":false,"assigned_usage_keys":[usage],"claim_source_key":format!("claim:{item}"),"terminal_source_key":format!("terminal:{item}")});
    let pair = |from: &str, to: &str, wait: f64, exec: f64, sample: Value| json!({"source_seat":from,"dest_seat":to,"ring_boundary":"in_ring","identity_evidence":"proven","coverage":"complete","wait_median_ms":wait,"exec_median_ms":exec,"samples":[sample]});
    json!({"metric":"decisions","definition_version":"acceptance-r1-a1","payload":{
     "rig":"rig","kind":"decisions","from":fixture::FROM,"to":fixture::TO,"usage_rows":3,"tokens":308,"token_categories":{"input_uncached":230,"output":23,"cache_read":45,"cache_write":10},
     "trace_inputs":[{"source_key":"u-build","input_uncached":100,"output":10,"cache_read":20,"cache_write":5},{"source_key":"u-orch","input_uncached":50,"output":5,"cache_read":10,"cache_write":2},{"source_key":"u-review","input_uncached":80,"output":8,"cache_read":15,"cache_write":3}],
     "issue_lineage":{"records":[{"source_key":"u-build","seat":"dev-build@rig"},{"source_key":"u-orch","seat":"orch-arch@rig"},{"source_key":"u-review","seat":"rev-review@rig"}]},
     "four_eyes_independence":{"checks":[{"chain":"root","builder":"dev-build@rig","reviewer":"rev-review@rig","builder_families":["claude"],"reviewer_families":["gpt"],"status":"independent","source_key":"e1","review_handoff":"e2"}]},
     "handoff_friction":{"formula_version":"handoff-friction-v1","pairs":[pair("dev-build@rig","orch-arch@rig",10.0,20.0,sample("e1","i1","i2","dev-build@rig","orch-arch@rig","u-orch")),pair("orch-arch@rig","rev-review@rig",5.0,30.0,sample("e2","i2","i3","orch-arch@rig","rev-review@rig","u-review"))]},
     "context_compactions":{"mid_item_hazard_count":1,"events":[{"pre_source_key":"before","post_source_key":"after","queue_source_keys":["q-active"]}]}
    }})
}
fn rule(seat: &str, uncached: f64, cache: f64) -> TierRule {
    TierRule {
        seat: seat.into(),
        uncached_multiplier: uncached,
        cache_read_multiplier: cache,
        work_class: Some("P1".into()),
        model_family: None,
        effort: Some("medium".into()),
    }
}
#[test]
fn canonical_real_report_has_exact_baseline_and_deterministic_default() -> anyhow::Result<()> {
    let f = fixture::Fixture::new(30);
    let r = report_json(
        &f.db,
        "decisions",
        Some("test-rig"),
        fixture::FROM,
        fixture::TO,
    )?;
    let s = Scenario::default();
    let a = simulation::simulate(&r, &s)?;
    let b = simulation::simulate(&r, &s)?;
    assert_eq!(a, b);
    assert_eq!(a["status"], "projected");
    assert_eq!(a["baseline"]["input_units"], 345.0);
    assert_eq!(a["projection"]["reduction_input_units"], 0.0);
    assert_eq!(a["conservation"]["conserved"], true);
    assert_eq!(a["baseline"]["source_keys"].as_array().unwrap().len(), 30);
    Ok(())
}
#[test]
fn direct_handoffs_use_causal_parent_links_and_conserve_all_four_components() -> anyhow::Result<()>
{
    let s = Scenario {
        direct_handoffs: true,
        ..Scenario::default()
    };
    let r = simulation::simulate(&report(), &s)?;
    assert_eq!(r["status"], "projected");
    assert_eq!(r["routing"]["paths"].as_array().unwrap().len(), 1);
    assert_eq!(
        r["routing"]["median_based_projected_total_cycle_reduction_ms"],
        30.0
    );
    assert_eq!(r["projection"]["reduction_input_units"], 51.0);
    assert_eq!(r["projection"]["input_units"], 183.5);
    assert_eq!(r["conservation"]["conserved"], true);
    assert_eq!(r["conservation"]["recombined"], r["baseline"]["tokens"]);
    assert_eq!(
        r["conservation"]["bypassed_coordination_tokens"],
        json!({"input_uncached":50,"output":5,"cache_read":10,"cache_write":2})
    );
    for key in [
        "u-orch",
        "e1",
        "e2",
        "claim:i1",
        "terminal:i1",
        "before",
        "after",
        "q-active",
    ] {
        assert!(
            r["source_keys"].as_array().unwrap().contains(&json!(key)),
            "{key}"
        );
    }
    assert!(r["compaction_hazards"]["projected_eliminated_hazards"].is_null());
    assert_eq!(r["compaction_hazards"]["observed_mid_item_hazards"], 1);
    Ok(())
}
#[test]
fn no_bypass_for_missing_roles_forks_recovery_cross_ring_or_missing_latencies() -> anyhow::Result<()>
{
    let s = Scenario {
        direct_handoffs: true,
        ..Scenario::default()
    };
    for fault in 0..7 {
        let mut r = report();
        match fault {
            0 => r["payload"]["four_eyes_independence"]["checks"] = json!([]),
            1 => {
                let duplicate = r["payload"]["handoff_friction"]["pairs"][1]["samples"][0].clone();
                r["payload"]["handoff_friction"]["pairs"][1]["samples"]
                    .as_array_mut()
                    .unwrap()
                    .push(duplicate);
            }
            2 => {
                r["payload"]["handoff_friction"]["pairs"][0]["samples"][0]["recovery"] = json!(true)
            }
            3 => {
                r["payload"]["handoff_friction"]["pairs"][1]["ring_boundary"] =
                    json!("cross_rig_seam")
            }
            4 => r["payload"]["handoff_friction"]["pairs"][0]["exec_median_ms"] = Value::Null,
            5 => {
                r["payload"]["handoff_friction"]["pairs"][0]["samples"][0]["assigned_usage_keys"] =
                    json!(["u-build"])
            }
            _ => {
                r["payload"]["handoff_friction"]["pairs"][1]["samples"][0]["lineage_status"] =
                    json!("partial")
            }
        }
        let out = simulation::simulate(&r, &s)?;
        assert_eq!(out["status"], "no_data", "fault {fault}");
        assert!(out["projection"]["input_units"].is_null());
    }
    Ok(())
}
#[test]
fn removing_a_seat_preserves_work_and_requires_explicit_replacement() -> anyhow::Result<()> {
    let mut s = Scenario {
        remove: vec!["dev-build@rig".into()],
        ..Scenario::default()
    };
    let unavailable = simulation::simulate(&report(), &s)?;
    assert_eq!(unavailable["status"], "no_data");
    assert_eq!(
        unavailable["conservation"]["unassigned_work_tokens"]["input_uncached"],
        100
    );
    s.replacement_seat = Some("rev-review@rig".into());
    s.tier_policy = vec![rule("rev-review@rig", 0.5, 1.0)];
    let out = simulation::simulate(&report(), &s)?;
    assert_eq!(out["status"], "projected");
    assert_eq!(out["projection"]["input_units"], 144.5);
    assert_eq!(out["projection"]["reduction_input_units"], 90.0);
    assert_eq!(
        out["conservation"]["allocation_records"]["rev-review@rig"],
        2
    );
    assert_eq!(out["conservation"]["conserved"], true);
    s.enforce_model_family_diversity = true;
    assert_eq!(simulation::simulate(&report(), &s)?["status"], "no_data");
    s.replacement_seat = Some("dev-build@rig".into());
    assert!(simulation::simulate(&report(), &s).is_err());
    Ok(())
}
#[test]
fn october_boundary_example_and_break_even_are_exact() -> anyhow::Result<()> {
    let model = BoundaryModel {
        turns: 15,
        stale_context_tokens: 350000,
        clean_context_tokens: 60000,
        cold_boot_tokens: 35000,
    };
    let r = simulation::boundary_cost(&model, &CostWeights::default())?;
    assert_eq!(r["warm_input_units"], 525000.0);
    assert_eq!(r["boundary_input_units"], 125000.0);
    assert_eq!(r["reduction_input_units"], 400000.0);
    assert!((r["break_even_turns"].as_f64().unwrap() - 35000.0 / 29000.0).abs() < 1e-12);
    assert_eq!(r["first_strictly_cheaper_integer_turn"], 2.0);
    let mut s = Scenario {
        boundary: Some(model),
        boundary_reset_threshold: Some(350000),
        ..Scenario::default()
    };
    let r = simulation::simulate(&report(), &s)?;
    assert_eq!(r["boundary"]["eligible"], true);
    assert_eq!(r["projection"]["reduction_input_units"], 0.0);
    assert_eq!(r["boundary"]["source_keys"], json!([]));
    s.boundary_reset_threshold = Some(400000);
    assert_eq!(
        simulation::simulate(&report(), &s)?["boundary"]["eligible"],
        false
    );
    Ok(())
}
#[test]
fn missing_boundary_and_nonpositive_context_advantage_are_honest() -> anyhow::Result<()> {
    let s = Scenario {
        boundary_reset_threshold: Some(1000),
        ..Scenario::default()
    };
    let r = simulation::simulate(&report(), &s)?;
    assert_eq!(r["status"], "no_data");
    assert_eq!(r["boundary"]["status"], "no_data");
    let b = BoundaryModel {
        turns: 0,
        stale_context_tokens: 60,
        clean_context_tokens: 60,
        cold_boot_tokens: 35,
    };
    let r = simulation::boundary_cost(&b, &CostWeights::default())?;
    assert_eq!(r["warm_input_units"], 0.0);
    assert_eq!(r["boundary_input_units"], 35.0);
    assert!(r["reduction_ratio"].is_null());
    assert!(r["break_even_turns"].is_null());
    Ok(())
}
#[test]
fn invalid_numeric_and_unknown_policy_inputs_refuse_without_nan() -> anyhow::Result<()> {
    for n in [-1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let s = Scenario {
            weights: CostWeights {
                uncached: n,
                cache_read: 0.1,
            },
            ..Scenario::default()
        };
        assert!(simulation::simulate(&report(), &s).is_err());
    }
    let s = Scenario {
        tier_policy: vec![rule("dev-build@rig", f64::NAN, 1.0)],
        ..Scenario::default()
    };
    assert!(simulation::simulate(&report(), &s).is_err());
    let huge = BoundaryModel {
        turns: u64::MAX,
        stale_context_tokens: u64::MAX,
        clean_context_tokens: 1,
        cold_boot_tokens: u64::MAX,
    };
    assert!(simulation::boundary_cost(
        &huge,
        &CostWeights {
            uncached: f64::MAX,
            cache_read: f64::MAX
        }
    )
    .is_err());
    assert!(serde_json::from_value::<Scenario>(json!({"silently_ignore":true})).is_err());
    assert!(serde_json::from_value::<TierRule>(json!({"seat":"a","unknown":1})).is_err());
    let s = Scenario {
        tier_policy: vec![
            rule("dev-build@rig", 1.0, 1.0),
            rule("dev-build@rig", 0.5, 1.0),
        ],
        ..Scenario::default()
    };
    assert!(simulation::simulate(&report(), &s).is_err());
    Ok(())
}
#[test]
fn unavailable_canonical_baselines_do_not_become_zero_savings() -> anyhow::Result<()> {
    assert_eq!(
        simulation::simulate(&Value::Null, &Scenario::default())?["status"],
        "no_data"
    );
    let mut r = report();
    r["payload"]["trace_inputs"] = json!([]);
    assert_eq!(
        simulation::simulate(&r, &Scenario::default())?["status"],
        "no_data"
    );
    let mut r = report();
    r["payload"]["usage_rows"] = json!(100);
    assert_eq!(
        simulation::simulate(&r, &Scenario::default())?["status"],
        "no_data"
    );
    Ok(())
}
#[test]
fn diversity_requires_complete_evidence_or_explicit_families() -> anyhow::Result<()> {
    let mut r = report();
    r["payload"]["four_eyes_independence"]["checks"][0]["status"] = json!("insufficient_evidence");
    let mut s = Scenario {
        enforce_model_family_diversity: true,
        ..Scenario::default()
    };
    assert_eq!(simulation::simulate(&r, &s)?["status"], "no_data");
    let mut a = rule("dev-build@rig", 1.0, 1.0);
    a.model_family = Some("claude".into());
    let mut b = rule("rev-review@rig", 1.0, 1.0);
    b.model_family = Some("gpt".into());
    s.tier_policy = vec![a, b];
    assert_eq!(simulation::simulate(&r, &s)?["status"], "projected");
    s.tier_policy[1].model_family = Some("claude".into());
    assert_eq!(simulation::simulate(&r, &s)?["status"], "no_data");
    Ok(())
}
