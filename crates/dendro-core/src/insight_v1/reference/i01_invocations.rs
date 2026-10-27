//! I01 model.invocations. Distinct evidenced call IDs at call grain.
use super::super::contract::{
    finish, refused, Insight, InsightDomain, InsightInput, InsightPopulation, InsightResult,
    InsightStatus,
};
use super::support::count_measure;
use duckdb::params;
use std::collections::BTreeSet;

pub struct Invocations;

pub static INSIGHT: Invocations = Invocations;

const COVERAGE: &str = "call grain only; usage-only rows are counted separately and are not calls";

struct Row {
    source_key: String,
    message_id: Option<String>,
}

impl Insight for Invocations {
    fn id(&self) -> &'static str {
        "I01"
    }
    fn name(&self) -> &'static str {
        "model.invocations"
    }
    fn version(&self) -> &'static str {
        "1.0.0"
    }
    fn domain(&self) -> InsightDomain {
        InsightDomain::Model
    }
    fn compute(&self, input: &InsightInput<'_>) -> InsightResult {
        let rows = match load(input) {
            Ok(rows) => rows,
            Err(_) => return refused(self, "store read failed", "usage_facts"),
        };
        if rows.is_empty() {
            return refused(self, "empty population", "usage_facts");
        }
        let mut calls = BTreeSet::new();
        let mut usage_only = 0i64;
        for row in &rows {
            match row.message_id.as_deref().filter(|id| !id.is_empty()) {
                Some(id) => {
                    calls.insert(id.to_string());
                }
                None => usage_only += 1,
            }
        }
        let evidence: Vec<String> = rows.into_iter().map(|row| row.source_key).collect();
        let measures = vec![
            count_measure(
                "I01.distinct_calls",
                "distinct_calls",
                calls.len() as i64,
                "calls",
                "count of distinct non-empty usage_facts.message_id values",
            ),
            count_measure(
                "I01.usage_only",
                "usage_only",
                usage_only,
                "rows",
                "usage rows with no call id; not included in distinct_calls",
            ),
        ];
        let status = if calls.is_empty() {
            InsightStatus::Refused {
                reason: "not_observable".to_string(),
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
                what: "usage_facts".to_string(),
                n: evidence.len() as u64,
            },
            measures,
            coverage: COVERAGE.to_string(),
            evidence,
        })
    }
}

fn load(input: &InsightInput<'_>) -> anyhow::Result<Vec<Row>> {
    let mut stmt = input.connection().prepare(
        "SELECT source_key, message_id
         FROM usage_facts
         WHERE (? IS NULL OR harness = ?)
           AND (? IS NULL OR native_session = ?)
           AND (? IS NULL OR ts >= CAST(? AS TIMESTAMP))
           AND (? IS NULL OR ts < CAST(? AS TIMESTAMP))
         ORDER BY source_key",
    )?;
    let scope = input.scope();
    let harness = scope.harness.as_deref();
    let session = scope.session.as_deref();
    let from = scope.from.as_deref();
    let to = scope.to.as_deref();
    let rows = stmt.query_map(
        params![harness, harness, session, session, from, from, to, to],
        |row| {
            Ok(Row {
                source_key: row.get(0)?,
                message_id: row.get(1)?,
            })
        },
    )?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}
