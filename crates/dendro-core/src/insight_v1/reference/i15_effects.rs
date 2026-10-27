//! I15 environment.effects. Admitted syscall or ETW counts, or not_observable.
use super::super::contract::{
    finish, label_id, refused, Insight, InsightDomain, InsightInput, InsightPopulation,
    InsightResult, InsightStatus, Measure, MeasureValue,
};
use super::support::table_exists;
use std::collections::BTreeMap;

pub struct EnvironmentEffects;

pub static INSIGHT: EnvironmentEffects = EnvironmentEffects;

const COVERAGE: &str =
    "only admitted syscall_v1 or windows_events_v1 rows; an unmatched result is unknown, not success";

impl Insight for EnvironmentEffects {
    fn id(&self) -> &'static str {
        "I15"
    }
    fn name(&self) -> &'static str {
        "environment.effects"
    }
    fn version(&self) -> &'static str {
        "1.0.0"
    }
    fn domain(&self) -> InsightDomain {
        InsightDomain::Environment
    }
    fn compute(&self, input: &InsightInput<'_>) -> InsightResult {
        if !table_exists(input.connection(), "syscall_facts") {
            return refused(self, "not_observable", "syscall_facts");
        }
        let rows = match load(input) {
            Ok(rows) => rows,
            Err(_) => return refused(self, "store read failed", "syscall_facts"),
        };
        if rows.is_empty() {
            return refused(self, "not_observable", "syscall_facts");
        }
        let mut counts: BTreeMap<(String, String, String), i64> = BTreeMap::new();
        let mut evidence = Vec::new();
        for (key, operation, class, result) in &rows {
            evidence.push(key.clone());
            let outcome = match result.as_str() {
                "success" => "success",
                "failed" => "failed",
                _ => "unknown",
            };
            *counts
                .entry((label_id(operation), label_id(class), outcome.to_string()))
                .or_default() += 1;
        }
        let measures = counts
            .into_iter()
            .map(|((operation, class, outcome), count)| Measure {
                id: format!("I15.{operation}.{class}.{outcome}"),
                name: outcome,
                value: Some(MeasureValue::Int(count)),
                unit: "syscalls".to_string(),
                basis: format!(
                    "admitted trace rows for {operation}/{class}; unknown is not success"
                ),
            })
            .collect();
        finish(InsightResult {
            id: self.id(),
            name: self.name(),
            version: self.version(),
            domain: self.domain(),
            status: InsightStatus::Computed,
            population: InsightPopulation {
                what: "syscall_facts".to_string(),
                n: rows.len() as u64,
            },
            measures,
            coverage: COVERAGE.to_string(),
            evidence,
        })
    }
}

fn load(input: &InsightInput<'_>) -> anyhow::Result<Vec<(String, String, String, String)>> {
    let mut stmt = input.connection().prepare(
        "SELECT source_key, operation, resource_class, result
         FROM syscall_facts
         WHERE grammar IN ('syscall_v1', 'windows_events_v1')
         ORDER BY source_key",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}
