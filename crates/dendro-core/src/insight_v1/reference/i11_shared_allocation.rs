//! I11 work.shared_allocation. Shared usage, weighted beneficiaries, and residual.
use super::super::contract::{
    finish, label_id, refused, Insight, InsightDomain, InsightInput, InsightPopulation,
    InsightResult, InsightStatus, Measure,
};
use super::support::{count_measure, disjoint_tokens, fit_i64, table_exists};
use duckdb::params;
use std::collections::{BTreeMap, BTreeSet};

pub struct SharedAllocation;

pub static INSIGHT: SharedAllocation = SharedAllocation;

const COVERAGE: &str =
    "absent allocation policy leaves the shared bucket; excessive weights reject allocation and keep direct usage";

struct Usage {
    key: String,
    session: String,
    tokens: i128,
}

struct Edge {
    parent: String,
    child: String,
}

struct Weight {
    worker: String,
    beneficiary: String,
    numerator: i64,
    denominator: i64,
}

impl Insight for SharedAllocation {
    fn id(&self) -> &'static str {
        "I11"
    }
    fn name(&self) -> &'static str {
        "work.shared_allocation"
    }
    fn version(&self) -> &'static str {
        "1.0.0"
    }
    fn domain(&self) -> InsightDomain {
        InsightDomain::Coordination
    }
    fn compute(&self, input: &InsightInput<'_>) -> InsightResult {
        let usage = match load_usage(input) {
            Ok(rows) => rows,
            Err(_) => return refused(self, "store read failed", "usage_facts"),
        };
        if usage.is_empty() {
            return refused(self, "empty population", "usage_facts");
        }
        let edges = match load_edges(input) {
            Ok(rows) => rows,
            Err(_) => return refused(self, "store read failed", "session_edges"),
        };
        let mut parents: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for edge in &edges {
            parents
                .entry(edge.child.clone())
                .or_default()
                .insert(edge.parent.clone());
        }
        let shared_sessions: BTreeSet<&str> = parents
            .iter()
            .filter(|(_, callers)| callers.len() > 1)
            .map(|(worker, _)| worker.as_str())
            .collect();
        let mut shared: i128 = 0;
        for row in &usage {
            if shared_sessions.contains(row.session.as_str()) {
                shared += row.tokens;
            }
        }
        let weights = if table_exists(input.connection(), "allocation_weights") {
            match load_weights(input) {
                Ok(rows) => rows,
                Err(_) => return refused(self, "store read failed", "allocation_weights"),
            }
        } else {
            Vec::new()
        };
        let mut measures = vec![count_measure(
            "I11.shared_tokens",
            "shared_tokens",
            fit_i64(shared).unwrap_or(0),
            "tokens",
            "direct disjoint tokens on workers with more than one evidenced caller",
        )];
        let mut status = InsightStatus::Computed;
        if weights.is_empty() {
            measures.push(count_measure(
                "I11.residual_tokens",
                "residual_tokens",
                fit_i64(shared).unwrap_or(0),
                "tokens",
                "no weight policy; the shared bucket stays unallocated",
            ));
        } else if !weights_ok(&weights) {
            status = InsightStatus::Partial {
                reason: "excessive weights".to_string(),
            };
            measures.push(count_measure(
                "I11.residual_tokens",
                "residual_tokens",
                fit_i64(shared).unwrap_or(0),
                "tokens",
                "weights were rejected; direct shared usage is unchanged",
            ));
        } else {
            let mut allocated = 0i128;
            let by_worker = usage_by_session(&usage, &shared_sessions);
            for weight in &weights {
                let Some(amount) = by_worker.get(&weight.worker) else {
                    continue;
                };
                let share = amount * i128::from(weight.numerator) / i128::from(weight.denominator);
                allocated += share;
                measures.push(Measure {
                    id: format!(
                        "I11.beneficiary.{}.{}",
                        label_id(&weight.worker),
                        label_id(&weight.beneficiary)
                    ),
                    name: "allocated_tokens".to_string(),
                    value: fit_i64(share).map(super::super::contract::MeasureValue::Int),
                    unit: "tokens".to_string(),
                    basis: format!(
                        "shared tokens * {}/{} for {}",
                        weight.numerator, weight.denominator, weight.beneficiary
                    ),
                });
            }
            let residual = shared - allocated;
            measures.push(count_measure(
                "I11.residual_tokens",
                "residual_tokens",
                fit_i64(residual).unwrap_or(0),
                "tokens",
                "shared tokens not assigned to a named beneficiary",
            ));
        }
        finish(InsightResult {
            id: self.id(),
            name: self.name(),
            version: self.version(),
            domain: self.domain(),
            status,
            population: InsightPopulation {
                what: "usage_facts".to_string(),
                n: usage.len() as u64,
            },
            measures,
            coverage: COVERAGE.to_string(),
            evidence: usage.into_iter().map(|row| row.key).collect(),
        })
    }
}

fn weights_ok(weights: &[Weight]) -> bool {
    if weights
        .iter()
        .any(|weight| weight.numerator < 0 || weight.denominator <= 0)
    {
        return false;
    }
    let mut by_worker: BTreeMap<&str, (i128, i128)> = BTreeMap::new();
    for weight in weights {
        let entry = by_worker.entry(weight.worker.as_str()).or_insert((0, 1));
        let numerator = i128::from(weight.numerator);
        let denominator = i128::from(weight.denominator);
        let (sum_n, sum_d) = *entry;
        *entry = (sum_n * denominator + numerator * sum_d, sum_d * denominator);
    }
    by_worker
        .values()
        .all(|(numerator, denominator)| *numerator >= 0 && *numerator <= *denominator)
}

fn usage_by_session(usage: &[Usage], shared: &BTreeSet<&str>) -> BTreeMap<String, i128> {
    let mut totals = BTreeMap::new();
    for row in usage {
        if shared.contains(row.session.as_str()) {
            *totals.entry(row.session.clone()).or_insert(0) += row.tokens;
        }
    }
    totals
}

fn load_usage(input: &InsightInput<'_>) -> anyhow::Result<Vec<Usage>> {
    let mut stmt = input.connection().prepare(
        "SELECT source_key, native_session, input_uncached, cache_write, cache_read, output_tokens
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
            let tokens =
                disjoint_tokens(row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?).unwrap_or(0);
            Ok(Usage {
                key: row.get(0)?,
                session: row.get(1)?,
                tokens,
            })
        },
    )?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn load_edges(input: &InsightInput<'_>) -> anyhow::Result<Vec<Edge>> {
    let mut stmt = input.connection().prepare(
        "SELECT parent_session, child_session
         FROM session_edges
         WHERE status = 'evidenced'
           AND parent_session <> ''
           AND child_session <> ''
           AND (? IS NULL OR harness = ?)
           AND (? IS NULL OR parent_session = ? OR child_session = ?)
         ORDER BY source_key",
    )?;
    let scope = input.scope();
    let harness = scope.harness.as_deref();
    let session = scope.session.as_deref();
    let rows = stmt.query_map(
        params![harness, harness, session, session, session],
        |row| {
            Ok(Edge {
                parent: row.get(0)?,
                child: row.get(1)?,
            })
        },
    )?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn load_weights(input: &InsightInput<'_>) -> anyhow::Result<Vec<Weight>> {
    let mut stmt = input.connection().prepare(
        "SELECT worker_session, beneficiary, numerator, denominator
         FROM allocation_weights
         ORDER BY worker_session, beneficiary",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(Weight {
            worker: row.get(0)?,
            beneficiary: row.get(1)?,
            numerator: row.get(2)?,
            denominator: row.get(3)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}
