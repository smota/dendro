//! I21 comparison.descriptive. Same-version strata, or noncomparable.
use super::super::contract::{
    finish, label_id, refused, Insight, InsightDomain, InsightInput, InsightPopulation,
    InsightResult, InsightStatus, Measure, MeasureValue,
};
use super::support::{disjoint_tokens, fit_i64};
use duckdb::params;
use std::collections::BTreeMap;

pub struct Comparison;

pub static INSIGHT: Comparison = Comparison;

const COVERAGE: &str =
    "raw stratum totals stay available; a mixed model or harness without one cohort is noncomparable and is not a ranking";

struct Row {
    key: String,
    harness: String,
    model: String,
    tokens: i128,
}

impl Insight for Comparison {
    fn id(&self) -> &'static str {
        "I21"
    }
    fn name(&self) -> &'static str {
        "comparison.descriptive"
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
        let mut groups: BTreeMap<(String, String), i128> = BTreeMap::new();
        for row in &rows {
            *groups
                .entry((row.harness.clone(), row.model.clone()))
                .or_default() += row.tokens;
        }
        let measures = groups
            .iter()
            .map(|((harness, model), tokens)| Measure {
                id: format!("I21.{}.{}", label_id(harness), label_id(model)),
                name: "stratum_tokens".to_string(),
                value: fit_i64(*tokens).map(MeasureValue::Int),
                unit: "tokens".to_string(),
                basis: "disjoint token sum for this harness and model; not a rank".to_string(),
            })
            .collect();
        let mixed = groups.len() > 1;
        let status = if mixed {
            InsightStatus::Refused {
                reason: "noncomparable".to_string(),
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
                n: rows.len() as u64,
            },
            measures,
            coverage: COVERAGE.to_string(),
            evidence: rows.into_iter().map(|row| row.key).collect(),
        })
    }
}

fn load(input: &InsightInput<'_>) -> anyhow::Result<Vec<Row>> {
    let mut stmt = input.connection().prepare(
        "SELECT source_key, harness, COALESCE(model, ''), input_uncached, cache_write, cache_read, output_tokens
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
            let model: String = row.get(2)?;
            let model = if model.is_empty() {
                "absent".to_string()
            } else {
                model
            };
            Ok(Row {
                key: row.get(0)?,
                harness: row.get(1)?,
                model,
                tokens: disjoint_tokens(row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?)
                    .unwrap_or(0),
            })
        },
    )?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}
