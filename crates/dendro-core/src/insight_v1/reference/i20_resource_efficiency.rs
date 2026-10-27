//! I20 outcome.resource_efficiency. Cost per accepted work, and a named mean.
use super::super::contract::{
    finish, refused, Insight, InsightDomain, InsightInput, InsightPopulation, InsightResult,
    InsightStatus,
};
use super::i19_acceptance::load_acceptance;
use super::support::{count_measure, float_measure};
use duckdb::params;
use std::collections::BTreeMap;

pub struct ResourceEfficiency;

pub static INSIGHT: ResourceEfficiency = ResourceEfficiency;

const COVERAGE: &str =
    "zero accepted work yields a null ratio, not infinity; the named mean omits undefined terms and reports the count";

impl Insight for ResourceEfficiency {
    fn id(&self) -> &'static str {
        "I20"
    }
    fn name(&self) -> &'static str {
        "outcome.resource_efficiency"
    }
    fn version(&self) -> &'static str {
        "1.0.0"
    }
    fn domain(&self) -> InsightDomain {
        InsightDomain::Coordination
    }
    fn compute(&self, input: &InsightInput<'_>) -> InsightResult {
        let acceptance = match load_acceptance(input) {
            Ok(Some(count)) => count,
            Ok(None) => return refused(self, "not_observable", "queue_facts"),
            Err(_) => return refused(self, "store read failed", "queue_facts"),
        };
        if acceptance.accepted == 0 {
            return finish(InsightResult {
                id: self.id(),
                name: self.name(),
                version: self.version(),
                domain: self.domain(),
                status: InsightStatus::Partial {
                    reason: "zero accepted".to_string(),
                },
                population: InsightPopulation {
                    what: "queue_facts".to_string(),
                    n: acceptance.evaluated as u64,
                },
                measures: vec![
                    float_measure(
                        "I20.cost_per_accepted",
                        "cost_per_accepted",
                        None,
                        "usd_per_work",
                        "null because accepted work is zero; not infinity",
                    ),
                    float_measure(
                        "I20.mean_per_work",
                        "mean_per_work",
                        None,
                        "usd_per_work",
                        "null because accepted work is zero",
                    ),
                    count_measure(
                        "I20.accepted",
                        "accepted",
                        0,
                        "work",
                        "accepted work in the matched population",
                    ),
                ],
                coverage: COVERAGE.to_string(),
                evidence: acceptance.evidence,
            });
        }
        let costs = match load_costs(input) {
            Ok(rows) => rows,
            Err(_) => return refused(self, "store read failed", "usage_facts"),
        };
        let mut defined = Vec::new();
        let mut undefined = 0i64;
        for item in &acceptance.accepted_items {
            match costs.get(item) {
                Some(amount) => defined.push(*amount),
                None => undefined += 1,
            }
        }
        let ratio = if undefined == 0 && !defined.is_empty() {
            Some(defined.iter().sum::<f64>() / acceptance.accepted as f64)
        } else {
            None
        };
        let mean = if defined.is_empty() {
            None
        } else {
            Some(defined.iter().sum::<f64>() / defined.len() as f64)
        };
        let status = if undefined > 0 || ratio.is_none() {
            InsightStatus::Partial {
                reason: "unmatched population".to_string(),
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
                what: "queue_facts".to_string(),
                n: acceptance.accepted as u64,
            },
            measures: vec![
                float_measure(
                    "I20.cost_per_accepted",
                    "cost_per_accepted",
                    ratio,
                    "usd_per_work",
                    "sum of matched observed currency / accepted work; unknown when any accepted work lacks currency",
                ),
                float_measure(
                    "I20.mean_per_work",
                    "mean_per_work",
                    mean,
                    "usd_per_work",
                    "mean of per-work observed currency; undefined terms are excluded",
                ),
                count_measure(
                    "I20.undefined",
                    "undefined",
                    undefined,
                    "work",
                    "accepted work with no matched observed currency",
                ),
                count_measure(
                    "I20.accepted",
                    "accepted",
                    acceptance.accepted,
                    "work",
                    "accepted work",
                ),
            ],
            coverage: COVERAGE.to_string(),
            evidence: acceptance.evidence,
        })
    }
}

fn load_costs(input: &InsightInput<'_>) -> anyhow::Result<BTreeMap<String, f64>> {
    let mut stmt = input.connection().prepare(
        "SELECT native_session, currency_total
         FROM usage_facts
         WHERE currency_total IS NOT NULL
           AND (? IS NULL OR harness = ?)
           AND (? IS NULL OR native_session = ?)
         ORDER BY source_key",
    )?;
    let scope = input.scope();
    let harness = scope.harness.as_deref();
    let session = scope.session.as_deref();
    let rows = stmt.query_map(params![harness, harness, session, session], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
    })?;
    let mut totals = BTreeMap::new();
    for row in rows {
        let (session, amount) = row?;
        *totals.entry(session).or_insert(0.0) += amount;
    }
    Ok(totals)
}
