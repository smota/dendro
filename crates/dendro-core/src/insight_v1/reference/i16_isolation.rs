//! I16 security.isolation. Declared, configured, and tested dimensions.
use super::super::contract::{
    finish, refused, Insight, InsightDomain, InsightInput, InsightPopulation, InsightResult,
    InsightStatus,
};
use super::support::count_measure;
use duckdb::params;

pub struct Isolation;

pub static INSIGHT: Isolation = Isolation;

const COVERAGE: &str =
    "a lone declaration is insufficient; namespace equality is configuration, not a universal verification";

impl Insight for Isolation {
    fn id(&self) -> &'static str {
        "I16"
    }
    fn name(&self) -> &'static str {
        "security.isolation"
    }
    fn version(&self) -> &'static str {
        "1.0.0"
    }
    fn domain(&self) -> InsightDomain {
        InsightDomain::Security
    }
    fn compute(&self, input: &InsightInput<'_>) -> InsightResult {
        let rows = match load(input) {
            Ok(rows) => rows,
            Err(_) => return refused(self, "store read failed", "security_facts"),
        };
        if rows.is_empty() {
            return refused(self, "empty population", "security_facts");
        }
        let mut declared = 0i64;
        let mut configured = 0i64;
        let mut tested = 0i64;
        let mut evidence = Vec::new();
        for (key, kind) in &rows {
            evidence.push(key.clone());
            match kind.as_str() {
                "permission_mode" | "approval_policy" | "permission_profile" => declared += 1,
                "sandbox_policy" => configured += 1,
                "isolation_test" => tested += 1,
                _ => {}
            }
        }
        let status = if tested == 0 {
            InsightStatus::Partial {
                reason: "insufficient".to_string(),
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
                what: "security_facts".to_string(),
                n: rows.len() as u64,
            },
            measures: vec![
                count_measure(
                    "I16.declared",
                    "declared",
                    declared,
                    "observations",
                    "permission_mode, approval_policy, and permission_profile rows",
                ),
                count_measure(
                    "I16.configured",
                    "observed_configuration",
                    configured,
                    "observations",
                    "sandbox_policy rows; equality with a declaration is not verification",
                ),
                count_measure(
                    "I16.tested",
                    "verified_effective",
                    tested,
                    "observations",
                    "isolation_test rows only; namespace equality does not increment this count",
                ),
            ],
            coverage: COVERAGE.to_string(),
            evidence,
        })
    }
}

fn load(input: &InsightInput<'_>) -> anyhow::Result<Vec<(String, String)>> {
    let mut stmt = input.connection().prepare(
        "SELECT source_key, kind
         FROM security_facts
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
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}
