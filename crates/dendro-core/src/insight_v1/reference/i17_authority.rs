//! I17 security.authority. Comparator counts, or NOT_ASSESSED without a policy.
use super::super::contract::{
    finish, refused, Insight, InsightDomain, InsightInput, InsightPopulation, InsightResult,
    InsightStatus, Measure,
};
use super::support::{count_measure, table_exists, unknown_measure};
use duckdb::params;

pub struct Authority;

pub static INSIGHT: Authority = Authority;

const COVERAGE: &str =
    "NOT_ASSESSED is not a zero violation count; a mismatch requires an explicit rule and the required facts";

impl Insight for Authority {
    fn id(&self) -> &'static str {
        "I17"
    }
    fn name(&self) -> &'static str {
        "security.authority"
    }
    fn version(&self) -> &'static str {
        "1.0.0"
    }
    fn domain(&self) -> InsightDomain {
        InsightDomain::Security
    }
    fn compute(&self, input: &InsightInput<'_>) -> InsightResult {
        let facts = match load_facts(input) {
            Ok(rows) => rows,
            Err(_) => return refused(self, "store read failed", "security_facts"),
        };
        let policy = facts
            .iter()
            .filter(|(_, kind, _)| kind == "permission_mode" || kind == "approval_policy")
            .count();
        if policy == 0 {
            return not_assessed(self);
        }
        if !table_exists(input.connection(), "authority_rules") {
            return insufficient(self, &facts);
        }
        let rules = match load_rules(input) {
            Ok(rows) => rows,
            Err(_) => return refused(self, "store read failed", "authority_rules"),
        };
        if rules.is_empty() {
            return insufficient(self, &facts);
        }
        let mut consistent = 0i64;
        let mut mismatch = 0i64;
        let mut missing = 0i64;
        for (kind, expected) in &rules {
            let values: Vec<&str> = facts
                .iter()
                .filter(|(_, fact_kind, _)| fact_kind == kind)
                .map(|(_, _, value)| value.as_str())
                .collect();
            if values.is_empty() {
                missing += 1;
            } else if values.iter().all(|value| *value == expected) {
                consistent += 1;
            } else {
                mismatch += 1;
            }
        }
        let status = if missing > 0 {
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
                n: facts.len() as u64,
            },
            measures: vec![
                count_measure(
                    "I17.consistent",
                    "consistent",
                    consistent,
                    "rules",
                    "rules whose observed values all match the explicit expected value",
                ),
                count_measure(
                    "I17.mismatch",
                    "mismatch",
                    mismatch,
                    "rules",
                    "rules with an observed value that differs from the explicit rule",
                ),
                count_measure(
                    "I17.insufficient",
                    "insufficient",
                    missing,
                    "rules",
                    "rules with no matching fact",
                ),
            ],
            coverage: COVERAGE.to_string(),
            evidence: facts.into_iter().map(|(key, _, _)| key).collect(),
        })
    }
}

fn not_assessed(insight: &Authority) -> InsightResult {
    finish(InsightResult {
        id: insight.id(),
        name: insight.name(),
        version: insight.version(),
        domain: insight.domain(),
        status: InsightStatus::Refused {
            reason: "NOT_ASSESSED".to_string(),
        },
        population: InsightPopulation {
            what: "security_facts".to_string(),
            n: 0,
        },
        measures: Vec::new(),
        coverage: COVERAGE.to_string(),
        evidence: Vec::new(),
    })
}

fn insufficient(insight: &Authority, facts: &[(String, String, String)]) -> InsightResult {
    let measures: Vec<Measure> = vec![
        unknown_measure(
            "I17.consistent",
            "consistent",
            "rules",
            "unknown without an explicit rule",
        ),
        unknown_measure(
            "I17.mismatch",
            "mismatch",
            "rules",
            "unknown without an explicit rule; not a zero violation count",
        ),
        count_measure(
            "I17.insufficient",
            "insufficient",
            facts.len() as i64,
            "observations",
            "policy observations with no admitted comparator rule",
        ),
    ];
    finish(InsightResult {
        id: insight.id(),
        name: insight.name(),
        version: insight.version(),
        domain: insight.domain(),
        status: InsightStatus::Partial {
            reason: "insufficient".to_string(),
        },
        population: InsightPopulation {
            what: "security_facts".to_string(),
            n: facts.len() as u64,
        },
        measures,
        coverage: COVERAGE.to_string(),
        evidence: facts.iter().map(|(key, _, _)| key.clone()).collect(),
    })
}

fn load_facts(input: &InsightInput<'_>) -> anyhow::Result<Vec<(String, String, String)>> {
    let mut stmt = input.connection().prepare(
        "SELECT source_key, kind, value
         FROM security_facts
         WHERE (? IS NULL OR harness = ?)
           AND (? IS NULL OR native_session = ?)
         ORDER BY source_key",
    )?;
    let scope = input.scope();
    let harness = scope.harness.as_deref();
    let session = scope.session.as_deref();
    let rows = stmt.query_map(params![harness, harness, session, session], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn load_rules(input: &InsightInput<'_>) -> anyhow::Result<Vec<(String, String)>> {
    let mut stmt = input
        .connection()
        .prepare("SELECT kind, expected_value FROM authority_rules ORDER BY rule_id, kind")?;
    let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}
