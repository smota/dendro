//! A01: DuckDB 2.0.0 runtime, schema-4 shadow of the six shipped insights,
//! and `library_privacy` locks.
use std::path::Path;

use dendro_core::engine_v200::{
    evaluate_schema4_shadow, probe_private_library, write_schema4_shadow_fixture, ShadowInsight,
};
use dendro_core::platform_v1::{self as platform, StorageRootChoice};

#[test]
fn runtime_is_v2_and_library_privacy_locks() {
    let root = platform::create_user_private_root_with_choice(
        "dendro-engine-v200-",
        &StorageRootChoice::UserProfile,
    )
    .expect("private root");
    let database = root.path().join("library.duckdb");
    let probe = probe_private_library(&database).expect("private open");
    assert!(
        probe.version.starts_with("v2.0"),
        "runtime version {}",
        probe.version
    );
    assert!(
        probe.version.starts_with("v2.0.0") || probe.version.starts_with("v2.0"),
        "runtime version {}",
        probe.version
    );
    assert_eq!(probe.autoinstall_known_extensions, "false");
    assert_eq!(probe.autoload_known_extensions, "false");
    assert_eq!(probe.lock_configuration, "true");
    assert!(
        Path::new(&probe.temp_directory).is_dir(),
        "temp_directory {}",
        probe.temp_directory
    );
    assert!(
        probe.temp_directory.contains("spill-"),
        "temp_directory {}",
        probe.temp_directory
    );
    assert!(
        probe.extension_directory.contains("extensions-"),
        "extension_directory {}",
        probe.extension_directory
    );
    drop(root);
}

#[test]
fn schema4_shadow_insights_are_deterministic() {
    let root = platform::create_user_private_root_with_choice(
        "dendro-shadow-v200-",
        &StorageRootChoice::UserProfile,
    )
    .expect("private root");
    let database = root.path().join("schema4.duckdb");
    write_schema4_shadow_fixture(&database).expect("schema4 shadow fixture");
    let first = shipped(&evaluate_schema4_shadow(&database).expect("first shadow pass"));
    let second = shipped(&evaluate_schema4_shadow(&database).expect("second shadow pass"));
    assert_eq!(
        first, second,
        "shadow statuses, measures, and evidence drifted"
    );
    let ids: Vec<_> = first.iter().map(|insight| insight.id.as_str()).collect();
    assert_eq!(ids, ["I02", "I04", "I08", "I09", "I14", "I18"]);
    assert_insight(
        &first,
        "I02",
        "computed",
        &["u1"],
        "I02.gpt-test.input_uncached",
        "100",
    );
    assert_insight(
        &first,
        "I04",
        "partial:",
        &["u1"],
        "I04.gpt-test.observed_usd",
        "0.020000000000",
    );
    assert_insight(
        &first,
        "I08",
        "computed",
        &["t1"],
        "I08.codex.failure_rate",
        "0.000000000000",
    );
    assert_insight(
        &first,
        "I09",
        "computed",
        &["e1"],
        "I09.relation.spawn",
        "1",
    );
    assert_insight(
        &first,
        "I14",
        "computed",
        &["root-a"],
        "I14.codex.attribution.launched",
        "1",
    );
    assert_insight(
        &first,
        "I18",
        "partial:",
        &["shadow-src"],
        "I18.shadow-src.read",
        "1",
    );
    drop(root);
}

fn shipped(insights: &[ShadowInsight]) -> Vec<ShadowInsight> {
    const IDS: &[&str] = &["I02", "I04", "I08", "I09", "I14", "I18"];
    insights
        .iter()
        .filter(|insight| IDS.contains(&insight.id.as_str()))
        .cloned()
        .collect()
}

fn assert_insight(
    insights: &[ShadowInsight],
    id: &str,
    status_prefix: &str,
    evidence: &[&str],
    measure_id: &str,
    measure_value: &str,
) {
    let insight = insights
        .iter()
        .find(|insight| insight.id == id)
        .unwrap_or_else(|| panic!("{id} missing"));
    assert!(
        insight.status.starts_with(status_prefix),
        "{id} status {}",
        insight.status
    );
    assert_eq!(insight.evidence, evidence, "{id} evidence");
    let measure = insight
        .measures
        .iter()
        .find(|(name, _)| name == measure_id)
        .unwrap_or_else(|| panic!("{measure_id} missing in {insight:?}"));
    assert_eq!(measure.1, measure_value, "{measure_id}");
}
