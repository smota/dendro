//! I14 environment.resources. Per root process tree and harness resource deltas, max RSS by unit, and attribution.
use super::super::contract::{
    finish, label_id, refused, Insight, InsightDomain, InsightInput, InsightPopulation,
    InsightResult, InsightStatus, Measure, MeasureValue,
};
use duckdb::params;
use std::collections::{BTreeMap, BTreeSet};

pub struct EnvironmentResources;

pub static INSIGHT: EnvironmentResources = EnvironmentResources;

const COVERAGE: &str =
    "per root_key and harness; CPU and disk deltas require readable non-decreasing samples; RSS units are never mixed across platforms";

#[derive(Debug, Clone)]
struct IdentityRow {
    identity_key: String,
    root_key: String,
    harness: Option<String>,
    attribution: Option<String>,
    session_link: Option<String>,
    visibility_limits: String,
}

#[derive(Debug, Clone)]
struct SampleRow {
    identity_key: String,
    cpu_time_us: Option<i64>,
    rss_bytes: Option<i64>,
    rss_unit: String,
    disk_read_bytes: Option<i64>,
    disk_write_bytes: Option<i64>,
    visibility_limits: String,
}

/// One root tree's identities and its samples, keyed by (root_key, harness).
type SelectedTree<'a> = (&'a [&'a IdentityRow], Vec<&'a SampleRow>);

impl Insight for EnvironmentResources {
    fn id(&self) -> &'static str {
        "I14"
    }
    fn name(&self) -> &'static str {
        "environment.resources"
    }
    fn version(&self) -> &'static str {
        "1.0.0"
    }
    fn domain(&self) -> InsightDomain {
        InsightDomain::Environment
    }
    fn compute(&self, input: &InsightInput<'_>) -> InsightResult {
        let identities = match load_identities(input) {
            Ok(rows) => rows,
            Err(_) => return refused(self, "store read failed", "process_identities"),
        };
        let samples = match load_samples(input) {
            Ok(rows) => rows,
            Err(_) => return refused(self, "store read failed", "process_samples"),
        };
        if identities.is_empty() || samples.is_empty() {
            return refused(self, "empty population", "process_samples");
        }

        let mut trees: BTreeMap<String, Vec<&IdentityRow>> = BTreeMap::new();
        let mut identity_to_root: BTreeMap<&str, &str> = BTreeMap::new();
        for row in &identities {
            trees.entry(row.root_key.clone()).or_default().push(row);
            identity_to_root.insert(row.identity_key.as_str(), row.root_key.as_str());
        }

        let mut samples_by_root: BTreeMap<&str, Vec<&SampleRow>> = BTreeMap::new();
        for sample in &samples {
            if let Some(&root_key) = identity_to_root.get(sample.identity_key.as_str()) {
                samples_by_root.entry(root_key).or_default().push(sample);
            }
        }

        let scope = input.scope();
        let mut selected_trees: BTreeMap<(String, String), SelectedTree<'_>> = BTreeMap::new();

        for (root_key, id_rows) in &trees {
            let Some(tree_samples) = samples_by_root.get(root_key.as_str()) else {
                continue;
            };
            if tree_samples.is_empty() {
                continue;
            }
            let raw_harness = id_rows
                .iter()
                .find(|r| r.identity_key == *root_key)
                .and_then(|r| r.harness.as_deref())
                .or_else(|| id_rows.iter().find_map(|r| r.harness.as_deref()))
                .filter(|h| !h.is_empty());
            if let Some(target_harness) = scope.harness.as_deref() {
                if raw_harness != Some(target_harness) {
                    continue;
                }
            }
            if let Some(target_session) = scope.session.as_deref() {
                let matches_session = id_rows
                    .iter()
                    .any(|r| r.session_link.as_deref() == Some(target_session));
                if !matches_session {
                    continue;
                }
            }
            let harness_key = raw_harness.map(label_id).unwrap_or_else(|| "absent".into());
            selected_trees.insert(
                (harness_key, root_key.clone()),
                (id_rows.as_slice(), tree_samples.clone()),
            );
        }

        if selected_trees.is_empty() {
            return refused(self, "empty population", "process_samples");
        }

        let mut measures = Vec::new();
        let mut has_visibility_limits = false;
        let mut has_unreadable_metric = false;
        let mut evidence = BTreeSet::new();

        let mut by_harness: BTreeMap<String, (i64, i64, i64)> = BTreeMap::new();

        for ((harness_key, root_key), (id_rows, tree_samples)) in &selected_trees {
            evidence.insert(root_key.clone());
            let counts = by_harness.entry(harness_key.clone()).or_default();

            let root_attribution = id_rows
                .iter()
                .find(|r| r.identity_key == *root_key)
                .and_then(|r| r.attribution.as_deref())
                .or_else(|| id_rows.iter().find_map(|r| r.attribution.as_deref()));
            match root_attribution {
                Some("launched") => counts.0 += 1,
                Some("discovered") => counts.1 += 1,
                _ => {}
            }

            if id_rows
                .iter()
                .any(|r| !r.visibility_limits.trim().is_empty())
                || tree_samples
                    .iter()
                    .any(|s| !s.visibility_limits.trim().is_empty())
            {
                has_visibility_limits = true;
            }

            let prefix = format!("I14.{harness_key}.{}", label_id(root_key));

            let mut samples_by_identity: BTreeMap<&str, Vec<&SampleRow>> = BTreeMap::new();
            for sample in tree_samples {
                samples_by_identity
                    .entry(sample.identity_key.as_str())
                    .or_default()
                    .push(*sample);
            }

            let cpu_delta = tree_counter_delta(id_rows, &samples_by_identity, |s| s.cpu_time_us);
            if cpu_delta.is_none() {
                counts.2 += 1;
                has_unreadable_metric = true;
            }
            measures.push(Measure {
                id: format!("{prefix}.cpu_time_delta_us"),
                name: "cpu_time_delta_us".to_string(),
                value: cpu_delta.map(MeasureValue::Int),
                unit: "microseconds".to_string(),
                basis:
                    "sum of per-identity (last - first cpu_time_us) across tree when all identities are readable and non-decreasing"
                        .to_string(),
            });

            let mut rss_by_unit: BTreeMap<&str, Vec<Option<i64>>> = BTreeMap::new();
            for sample in tree_samples {
                rss_by_unit
                    .entry(sample.rss_unit.as_str())
                    .or_default()
                    .push(sample.rss_bytes);
            }
            for (unit, values) in rss_by_unit {
                let max_rss = if values.iter().all(Option::is_some) {
                    values.into_iter().flatten().max()
                } else {
                    None
                };
                if max_rss.is_none() {
                    has_unreadable_metric = true;
                }
                measures.push(Measure {
                    id: format!("{prefix}.max_rss.{}", label_id(unit)),
                    name: "max_rss".to_string(),
                    value: max_rss.map(MeasureValue::Int),
                    unit: unit.to_string(),
                    basis: format!("max(rss_bytes) for rss_unit={unit}; units are never mixed"),
                });
            }

            let disk_read_delta =
                tree_counter_delta(id_rows, &samples_by_identity, |s| s.disk_read_bytes);
            if disk_read_delta.is_none() {
                has_unreadable_metric = true;
            }
            measures.push(Measure {
                id: format!("{prefix}.disk_read_delta_bytes"),
                name: "disk_read_delta_bytes".to_string(),
                value: disk_read_delta.map(MeasureValue::Int),
                unit: "bytes".to_string(),
                basis: "sum of per-identity (last - first disk_read_bytes) across tree when all identities are readable and non-decreasing".to_string(),
            });

            let disk_write_delta =
                tree_counter_delta(id_rows, &samples_by_identity, |s| s.disk_write_bytes);
            if disk_write_delta.is_none() {
                has_unreadable_metric = true;
            }
            measures.push(Measure {
                id: format!("{prefix}.disk_write_delta_bytes"),
                name: "disk_write_delta_bytes".to_string(),
                value: disk_write_delta.map(MeasureValue::Int),
                unit: "bytes".to_string(),
                basis: "sum of per-identity (last - first disk_write_bytes) across tree when all identities are readable and non-decreasing".to_string(),
            });
        }

        for (harness_key, (launched, discovered, cpu_unreadable_trees)) in &by_harness {
            let prefix = format!("I14.{harness_key}");
            measures.push(Measure {
                id: format!("{prefix}.attribution.launched"),
                name: "attribution.launched".to_string(),
                value: Some(MeasureValue::Int(*launched)),
                unit: "trees".to_string(),
                basis: "root process trees with attribution=launched".to_string(),
            });
            measures.push(Measure {
                id: format!("{prefix}.attribution.discovered"),
                name: "attribution.discovered".to_string(),
                value: Some(MeasureValue::Int(*discovered)),
                unit: "trees".to_string(),
                basis: "root process trees with attribution=discovered".to_string(),
            });
            measures.push(Measure {
                id: format!("{prefix}.cpu_unreadable_trees"),
                name: "cpu_unreadable_trees".to_string(),
                value: Some(MeasureValue::Int(*cpu_unreadable_trees)),
                unit: "trees".to_string(),
                basis: "root process trees where cpu_time_us delta is unreadable or decreasing"
                    .to_string(),
            });
        }

        let status = match (has_visibility_limits, has_unreadable_metric) {
            (true, _) => InsightStatus::Partial {
                reason: "visibility limits present on process observations".to_string(),
            },
            (false, true) => InsightStatus::Partial {
                reason: "unreadable or non-monotonic resource counter".to_string(),
            },
            (false, false) => InsightStatus::Computed,
        };

        finish(InsightResult {
            id: self.id(),
            name: self.name(),
            version: self.version(),
            domain: self.domain(),
            status,
            population: InsightPopulation {
                what: "process_samples".to_string(),
                n: selected_trees.len() as u64,
            },
            measures,
            coverage: COVERAGE.to_string(),
            evidence: evidence.into_iter().collect(),
        })
    }
}

fn tree_counter_delta(
    id_rows: &[&IdentityRow],
    samples_by_identity: &BTreeMap<&str, Vec<&SampleRow>>,
    extract: impl Fn(&SampleRow) -> Option<i64>,
) -> Option<i64> {
    if id_rows.is_empty() {
        return None;
    }
    let mut total = 0i64;
    for id_row in id_rows {
        let id_samples = samples_by_identity.get(id_row.identity_key.as_str())?;
        let delta = monotonic_delta(id_samples.iter().map(|s| extract(s)))?;
        total = total.checked_add(delta)?;
    }
    Some(total)
}

fn monotonic_delta(values: impl Iterator<Item = Option<i64>>) -> Option<i64> {
    let mut first = None;
    let mut prev = None;
    for value in values {
        let v = value?;
        if v < 0 {
            return None;
        }
        if let Some(p) = prev {
            if v < p {
                return None;
            }
        } else {
            first = Some(v);
        }
        prev = Some(v);
    }
    match (first, prev) {
        (Some(start), Some(end)) => end.checked_sub(start),
        _ => None,
    }
}

fn load_identities(input: &InsightInput<'_>) -> anyhow::Result<Vec<IdentityRow>> {
    let mut stmt = input.connection().prepare(
        "SELECT identity_key, root_key, harness, attribution, session_link, visibility_limits
         FROM process_identities
         ORDER BY root_key, identity_key",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(IdentityRow {
            identity_key: row.get(0)?,
            root_key: row.get(1)?,
            harness: row.get(2)?,
            attribution: row.get(3)?,
            session_link: row.get(4)?,
            visibility_limits: row.get(5)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn load_samples(input: &InsightInput<'_>) -> anyhow::Result<Vec<SampleRow>> {
    let mut stmt = input.connection().prepare(
        "SELECT identity_key, cpu_time_us, rss_bytes, rss_unit, disk_read_bytes, disk_write_bytes, visibility_limits
         FROM process_samples
         WHERE (? IS NULL OR ts >= CAST(? AS TIMESTAMP))
           AND (? IS NULL OR ts < CAST(? AS TIMESTAMP))
         ORDER BY ts, watch_id, seq, identity_key",
    )?;
    let scope = input.scope();
    let from = scope.from.as_deref();
    let to = scope.to.as_deref();
    let rows = stmt.query_map(params![from, from, to, to], |row| {
        Ok(SampleRow {
            identity_key: row.get(0)?,
            cpu_time_us: row.get(1)?,
            rss_bytes: row.get(2)?,
            rss_unit: row.get(3)?,
            disk_read_bytes: row.get(4)?,
            disk_write_bytes: row.get(5)?,
            visibility_limits: row.get(6)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insight_v1::contract::{InsightInput, MeasureValue};
    use crate::insight_v1::{evaluate, Scope};
    use crate::AnalyticsStore;
    use duckdb::params;

    fn store() -> AnalyticsStore {
        AnalyticsStore::open_in_memory("insight-i14").unwrap()
    }

    fn insert_identity(
        store: &AnalyticsStore,
        identity_key: &str,
        root_key: &str,
        harness: Option<&str>,
        attribution: Option<&str>,
        limits: &str,
    ) {
        store
            .connection()
            .execute(
                "INSERT INTO process_identities (
                    identity_key, root_key, pid, parent_pid, platform, start_time_unix_s,
                    exe_name, harness, cwd_fingerprint, session_link, environment_kind,
                    isolation_hint, first_seen, last_seen, ended_at, visibility_limits, attribution
                 ) VALUES (?, ?, 100, NULL, 'linux', 1700000000, 'claude', ?, NULL, 's1', 'host', 'none',
                           CAST('2026-10-15 00:00:00' AS TIMESTAMP), CAST('2026-10-15 00:01:00' AS TIMESTAMP),
                           NULL, ?, ?)",
                params![identity_key, root_key, harness, limits, attribution],
            )
            .unwrap();
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_sample(
        store: &AnalyticsStore,
        identity_key: &str,
        seq: i64,
        ts: &str,
        cpu_us: Option<i64>,
        rss_bytes: Option<i64>,
        rss_unit: &str,
        disk_read: Option<i64>,
        disk_write: Option<i64>,
        limits: &str,
    ) {
        store
            .connection()
            .execute(
                "INSERT INTO process_samples (
                    watch_id, identity_key, seq, ts, cpu_time_us, rss_bytes, rss_unit,
                    disk_read_bytes, disk_write_bytes, capture_method, visibility_limits
                 ) VALUES ('w1', ?, ?, CAST(? AS TIMESTAMP), ?, ?, ?, ?, ?, 'procfs', ?)",
                params![
                    identity_key,
                    seq,
                    ts,
                    cpu_us,
                    rss_bytes,
                    rss_unit,
                    disk_read,
                    disk_write,
                    limits
                ],
            )
            .unwrap();
    }

    fn only(store: &AnalyticsStore, scope: &Scope) -> InsightResult {
        evaluate(store, scope)
            .into_iter()
            .find(|result| result.id == "I14")
            .unwrap_or_else(|| {
                if let Some(reason) = scope.refusal() {
                    refused(&INSIGHT, reason, "undefined")
                } else {
                    INSIGHT.compute(&InsightInput::new(store, scope))
                }
            })
    }

    fn int_at(result: &InsightResult, id: &str) -> Option<i64> {
        let measure = result
            .measures
            .iter()
            .find(|measure| measure.id == id)
            .unwrap();
        match measure.value {
            Some(MeasureValue::Int(value)) => Some(value),
            None => None,
            Some(other) => panic!("{id} was {other}"),
        }
    }

    #[test]
    fn computes_deltas_and_keeps_rss_units_separate_with_attribution_breakdown() {
        let store = store();
        insert_identity(
            &store,
            "root-a",
            "root-a",
            Some("claude"),
            Some("launched"),
            "",
        );
        insert_identity(
            &store,
            "root-b",
            "root-b",
            Some("claude"),
            Some("discovered"),
            "",
        );

        insert_sample(
            &store,
            "root-a",
            1,
            "2026-10-15 00:00:01",
            Some(100),
            Some(4096),
            "rss_bytes",
            Some(1000),
            Some(500),
            "",
        );
        insert_sample(
            &store,
            "root-a",
            2,
            "2026-10-15 00:00:02",
            Some(450),
            Some(8192),
            "rss_bytes",
            Some(1600),
            Some(900),
            "",
        );

        insert_sample(
            &store,
            "root-b",
            1,
            "2026-10-15 00:00:01",
            Some(50),
            Some(16384),
            "working_set_bytes",
            Some(200),
            Some(100),
            "",
        );
        insert_sample(
            &store,
            "root-b",
            2,
            "2026-10-15 00:00:02",
            Some(125),
            Some(12288),
            "working_set_bytes",
            Some(500),
            Some(300),
            "",
        );

        let result = only(&store, &Scope::default());
        assert!(matches!(result.status, InsightStatus::Computed));
        assert_eq!(result.population.n, 2);

        assert_eq!(
            int_at(&result, "I14.claude.root-a.cpu_time_delta_us"),
            Some(350)
        );
        assert_eq!(
            int_at(&result, "I14.claude.root-a.max_rss.rss_bytes"),
            Some(8192)
        );
        assert_eq!(
            int_at(&result, "I14.claude.root-a.disk_read_delta_bytes"),
            Some(600)
        );
        assert_eq!(
            int_at(&result, "I14.claude.root-a.disk_write_delta_bytes"),
            Some(400)
        );

        assert_eq!(
            int_at(&result, "I14.claude.root-b.cpu_time_delta_us"),
            Some(75)
        );
        assert_eq!(
            int_at(&result, "I14.claude.root-b.max_rss.working_set_bytes"),
            Some(16384)
        );
        assert_eq!(
            int_at(&result, "I14.claude.root-b.disk_read_delta_bytes"),
            Some(300)
        );
        assert_eq!(
            int_at(&result, "I14.claude.root-b.disk_write_delta_bytes"),
            Some(200)
        );

        assert_eq!(int_at(&result, "I14.claude.attribution.launched"), Some(1));
        assert_eq!(
            int_at(&result, "I14.claude.attribution.discovered"),
            Some(1)
        );
        assert_eq!(int_at(&result, "I14.claude.cpu_unreadable_trees"), Some(0));
        assert_eq!(
            result.evidence,
            vec!["root-a".to_string(), "root-b".to_string()]
        );
        assert_eq!(result, only(&store, &Scope::default()));
    }

    #[test]
    fn empty_population_is_refused() {
        let result = only(&store(), &Scope::default());
        assert!(matches!(
            result.status,
            InsightStatus::Refused { reason } if reason == "empty population"
        ));
        assert!(result.measures.is_empty());
    }

    #[test]
    fn visibility_limits_and_unreadable_or_decreasing_cpu_yield_partial_and_none_without_zero() {
        let store = store();
        insert_identity(
            &store,
            "root-u",
            "root-u",
            Some("claude"),
            Some("discovered"),
            "permission_denied",
        );
        insert_sample(
            &store,
            "root-u",
            1,
            "2026-10-15 00:00:01",
            Some(500),
            None,
            "rss_bytes",
            None,
            None,
            "permission_denied",
        );
        insert_sample(
            &store,
            "root-u",
            2,
            "2026-10-15 00:00:02",
            Some(200),
            None,
            "rss_bytes",
            None,
            None,
            "permission_denied",
        );

        let result = only(&store, &Scope::default());
        assert!(matches!(
            result.status,
            InsightStatus::Partial { ref reason } if reason.contains("visibility limits")
        ));
        assert_eq!(int_at(&result, "I14.claude.root-u.cpu_time_delta_us"), None);
        assert_eq!(int_at(&result, "I14.claude.root-u.max_rss.rss_bytes"), None);
        assert_eq!(
            int_at(&result, "I14.claude.root-u.disk_read_delta_bytes"),
            None
        );
        assert_eq!(
            int_at(&result, "I14.claude.root-u.disk_write_delta_bytes"),
            None
        );
        assert_eq!(int_at(&result, "I14.claude.cpu_unreadable_trees"), Some(1));
    }

    #[test]
    fn interleaved_parent_and_child_samples_sum_per_identity_deltas() {
        let store = store();
        insert_identity(
            &store,
            "root-p",
            "root-p",
            Some("claude"),
            Some("launched"),
            "",
        );
        insert_identity(&store, "child-c", "root-p", None, None, "");

        insert_sample(
            &store,
            "root-p",
            1,
            "2026-10-15 00:00:01",
            Some(1000),
            Some(4096),
            "rss_bytes",
            Some(2000),
            Some(1000),
            "",
        );
        insert_sample(
            &store,
            "child-c",
            2,
            "2026-10-15 00:00:02",
            Some(50),
            Some(2048),
            "rss_bytes",
            Some(100),
            Some(40),
            "",
        );
        insert_sample(
            &store,
            "root-p",
            3,
            "2026-10-15 00:00:03",
            Some(1010),
            Some(6144),
            "rss_bytes",
            Some(2300),
            Some(1100),
            "",
        );
        insert_sample(
            &store,
            "child-c",
            4,
            "2026-10-15 00:00:04",
            Some(60),
            Some(3072),
            "rss_bytes",
            Some(150),
            Some(90),
            "",
        );

        let result = only(&store, &Scope::default());
        assert!(matches!(result.status, InsightStatus::Computed));
        assert_eq!(result.population.n, 1);
        assert_eq!(
            int_at(&result, "I14.claude.root-p.cpu_time_delta_us"),
            Some(20)
        );
        assert_eq!(
            int_at(&result, "I14.claude.root-p.disk_read_delta_bytes"),
            Some(350)
        );
        assert_eq!(
            int_at(&result, "I14.claude.root-p.disk_write_delta_bytes"),
            Some(150)
        );
        assert_eq!(
            int_at(&result, "I14.claude.root-p.max_rss.rss_bytes"),
            Some(6144)
        );
        assert_eq!(int_at(&result, "I14.claude.cpu_unreadable_trees"), Some(0));
    }

    #[test]
    fn non_monotonic_child_makes_tree_cpu_none_and_counted() {
        let store = store();
        insert_identity(
            &store,
            "root-p",
            "root-p",
            Some("claude"),
            Some("launched"),
            "",
        );
        insert_identity(&store, "child-c", "root-p", None, None, "");

        insert_sample(
            &store,
            "root-p",
            1,
            "2026-10-15 00:00:01",
            Some(1000),
            Some(4096),
            "rss_bytes",
            Some(100),
            Some(100),
            "",
        );
        insert_sample(
            &store,
            "child-c",
            2,
            "2026-10-15 00:00:02",
            Some(60),
            Some(2048),
            "rss_bytes",
            Some(50),
            Some(50),
            "",
        );
        insert_sample(
            &store,
            "root-p",
            3,
            "2026-10-15 00:00:03",
            Some(1010),
            Some(4096),
            "rss_bytes",
            Some(200),
            Some(200),
            "",
        );
        insert_sample(
            &store,
            "child-c",
            4,
            "2026-10-15 00:00:04",
            Some(50),
            Some(2048),
            "rss_bytes",
            Some(80),
            Some(80),
            "",
        );

        let result = only(&store, &Scope::default());
        assert!(matches!(result.status, InsightStatus::Partial { .. }));
        assert_eq!(int_at(&result, "I14.claude.root-p.cpu_time_delta_us"), None);
        assert_eq!(int_at(&result, "I14.claude.cpu_unreadable_trees"), Some(1));
        assert_eq!(
            int_at(&result, "I14.claude.root-p.disk_read_delta_bytes"),
            Some(130)
        );
        assert_eq!(
            int_at(&result, "I14.claude.root-p.disk_write_delta_bytes"),
            Some(130)
        );
    }
}
