//! I13 environment.placement. Incarnations, loci, and same-label rebuilds.
use super::super::contract::{
    finish, label_id, refused, Insight, InsightDomain, InsightInput, InsightPopulation,
    InsightResult, InsightStatus, Measure, MeasureValue,
};
use super::support::count_measure;
use duckdb::params;
use std::collections::BTreeMap;

pub struct Placement;

pub static INSIGHT: Placement = Placement;

const COVERAGE: &str =
    "identity evidence required; an unknown environment or empty executable is unresolved and is not a host count";

struct Identity {
    key: String,
    platform: String,
    exe_name: String,
    environment_kind: String,
    first_seen: String,
    last_seen: String,
}

impl Insight for Placement {
    fn id(&self) -> &'static str {
        "I13"
    }
    fn name(&self) -> &'static str {
        "environment.placement"
    }
    fn version(&self) -> &'static str {
        "1.0.0"
    }
    fn domain(&self) -> InsightDomain {
        InsightDomain::Environment
    }
    fn compute(&self, input: &InsightInput<'_>) -> InsightResult {
        let rows = match load(input) {
            Ok(rows) => rows,
            Err(_) => return refused(self, "store read failed", "process_identities"),
        };
        if rows.is_empty() {
            return refused(self, "empty population", "process_identities");
        }
        let mut unresolved = 0i64;
        let mut intervals = 0i64;
        let mut by_locus: BTreeMap<String, i64> = BTreeMap::new();
        let mut by_label: BTreeMap<String, i64> = BTreeMap::new();
        for row in &rows {
            if row.environment_kind == "unknown" || row.exe_name.is_empty() {
                unresolved += 1;
            }
            if row.first_seen < row.last_seen {
                intervals += 1;
            }
            *by_locus.entry(label_id(&row.platform)).or_default() += 1;
            if !row.exe_name.is_empty() {
                *by_label.entry(row.exe_name.clone()).or_default() += 1;
            }
        }
        let rebuilds = by_label
            .values()
            .map(|count| (*count - 1).max(0))
            .sum::<i64>();
        let mut measures = vec![
            count_measure(
                "I13.incarnations",
                "incarnations",
                rows.len() as i64,
                "processes",
                "distinct process_identities rows",
            ),
            count_measure(
                "I13.unresolved",
                "unresolved",
                unresolved,
                "processes",
                "environment_kind unknown or empty exe_name",
            ),
            count_measure(
                "I13.rebuilds",
                "rebuilds",
                rebuilds,
                "processes",
                "extra incarnations that share an executable label",
            ),
            count_measure(
                "I13.placement_intervals",
                "placement_intervals",
                intervals,
                "intervals",
                "identities whose first_seen is before last_seen",
            ),
        ];
        for (locus, count) in by_locus {
            measures.push(Measure {
                id: format!("I13.locus.{locus}"),
                name: format!("locus.{locus}"),
                value: Some(MeasureValue::Int(count)),
                unit: "processes".to_string(),
                basis: "distinct incarnations on this platform locus".to_string(),
            });
        }
        let status = if unresolved > 0 {
            InsightStatus::Partial {
                reason: "unresolved incarnation".to_string(),
            }
        } else {
            InsightStatus::Computed
        };
        finish(InsightResult {
            id: self.id(),
            name: self.name(),
            version: self.version(),
            domain: self.domain(),
            status,
            population: InsightPopulation {
                what: "process_identities".to_string(),
                n: rows.len() as u64,
            },
            measures,
            coverage: COVERAGE.to_string(),
            evidence: rows.into_iter().map(|row| row.key).collect(),
        })
    }
}

fn load(input: &InsightInput<'_>) -> anyhow::Result<Vec<Identity>> {
    let mut stmt = input.connection().prepare(
        "SELECT identity_key, platform, exe_name, environment_kind,
                CAST(first_seen AS VARCHAR), CAST(last_seen AS VARCHAR)
         FROM process_identities
         WHERE (? IS NULL OR harness = ?)
           AND (? IS NULL OR session_link = ?)
           AND (? IS NULL OR first_seen >= CAST(? AS TIMESTAMP))
           AND (? IS NULL OR first_seen < CAST(? AS TIMESTAMP))
         ORDER BY identity_key",
    )?;
    let scope = input.scope();
    let harness = scope.harness.as_deref();
    let session = scope.session.as_deref();
    let from = scope.from.as_deref();
    let to = scope.to.as_deref();
    let rows = stmt.query_map(
        params![harness, harness, session, session, from, from, to, to],
        |row| {
            Ok(Identity {
                key: row.get(0)?,
                platform: row.get(1)?,
                exe_name: row.get(2)?,
                environment_kind: row.get(3)?,
                first_seen: row.get(4)?,
                last_seen: row.get(5)?,
            })
        },
    )?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}
