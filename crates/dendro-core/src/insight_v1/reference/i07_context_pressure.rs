//! I07 harness.context_pressure. Context used over context limit at invocation.
use super::super::contract::{
    finish, refused, Insight, InsightDomain, InsightInput, InsightPopulation, InsightResult,
    InsightStatus,
};
use super::support::{float_measure, table_exists};
use duckdb::params;

pub struct ContextPressure;

pub static INSIGHT: ContextPressure = ContextPressure;

const COVERAGE: &str =
    "prompt tokens do not substitute for context-used; a missing limit refuses the ratio";

impl Insight for ContextPressure {
    fn id(&self) -> &'static str {
        "I07"
    }
    fn name(&self) -> &'static str {
        "harness.context_pressure"
    }
    fn version(&self) -> &'static str {
        "1.0.0"
    }
    fn domain(&self) -> InsightDomain {
        InsightDomain::Harness
    }
    fn compute(&self, input: &InsightInput<'_>) -> InsightResult {
        if !table_exists(input.connection(), "context_facts") {
            let usage = match usage_count(input) {
                Ok(count) => count,
                Err(_) => return refused(self, "store read failed", "usage_facts"),
            };
            if usage == 0 {
                return refused(self, "empty population", "context_facts");
            }
            return missing_limit(self, usage, Vec::new());
        }
        let rows = match load(input) {
            Ok(rows) => rows,
            Err(_) => return refused(self, "store read failed", "context_facts"),
        };
        if rows.is_empty() {
            return refused(self, "empty population", "context_facts");
        }
        let mut ratios = Vec::new();
        let mut excluded = 0i64;
        let mut evidence = Vec::new();
        for (key, used, limit) in &rows {
            evidence.push(key.clone());
            if *used < 0 || limit.is_none_or(|limit| limit <= 0) {
                excluded += 1;
                continue;
            }
            ratios.push(*used as f64 / limit.unwrap() as f64);
        }
        if ratios.is_empty() {
            return missing_limit(self, rows.len() as u64, evidence);
        }
        let peak = ratios.iter().copied().fold(f64::MIN, f64::max);
        let status = if excluded > 0 {
            InsightStatus::Partial {
                reason: "missing context limit".to_string(),
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
                what: "context_facts".to_string(),
                n: rows.len() as u64,
            },
            measures: vec![
                float_measure(
                    "I07.peak_ratio",
                    "peak_ratio",
                    Some(peak),
                    "ratio",
                    "max context_used / context_limit where both share one unit and the limit is positive",
                ),
                super::support::count_measure(
                    "I07.excluded",
                    "excluded",
                    excluded,
                    "invocations",
                    "rows with a missing or non-positive context limit",
                ),
            ],
            coverage: COVERAGE.to_string(),
            evidence,
        })
    }
}

fn missing_limit(insight: &ContextPressure, n: u64, evidence: Vec<String>) -> InsightResult {
    finish(InsightResult {
        id: insight.id(),
        name: insight.name(),
        version: insight.version(),
        domain: insight.domain(),
        status: InsightStatus::Partial {
            reason: "missing context limit".to_string(),
        },
        population: InsightPopulation {
            what: "context_facts".to_string(),
            n,
        },
        measures: vec![float_measure(
            "I07.peak_ratio",
            "peak_ratio",
            None,
            "ratio",
            "unknown; prompt tokens are not used as context-used and no context limit was recorded",
        )],
        coverage: COVERAGE.to_string(),
        evidence,
    })
}

fn usage_count(input: &InsightInput<'_>) -> anyhow::Result<u64> {
    let count: i64 = input.connection().query_row(
        "SELECT COUNT(*) FROM usage_facts
         WHERE (? IS NULL OR harness = ?)
           AND (? IS NULL OR native_session = ?)",
        params![
            input.scope().harness.as_deref(),
            input.scope().harness.as_deref(),
            input.scope().session.as_deref(),
            input.scope().session.as_deref()
        ],
        |row| row.get(0),
    )?;
    Ok(count as u64)
}

fn load(input: &InsightInput<'_>) -> anyhow::Result<Vec<(String, i64, Option<i64>)>> {
    let mut stmt = input.connection().prepare(
        "SELECT source_key, context_used, context_limit
         FROM context_facts
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
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}
