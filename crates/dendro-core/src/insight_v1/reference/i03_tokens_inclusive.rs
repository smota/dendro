//! I03 model.tokens.inclusive. Deduped union of reachable usage observations.
use super::super::contract::{
    finish, label_id, refused, Insight, InsightDomain, InsightInput, InsightPopulation,
    InsightResult, InsightStatus, Measure, MeasureValue,
};
use super::support::{disjoint_tokens, fit_i64};
use duckdb::params;
use std::collections::{BTreeMap, BTreeSet};

pub struct TokensInclusive;

pub static INSIGHT: TokensInclusive = TokensInclusive;

const COVERAGE: &str =
    "inclusive totals are the deduped observation union and are nonadditive across nodes; fleet_deduped is the only fleet total";

struct Usage {
    source_key: String,
    session: String,
    message_id: Option<String>,
    tokens: Option<i128>,
}

struct Edge {
    parent: String,
    child: String,
    relation: String,
    status: String,
}

impl Insight for TokensInclusive {
    fn id(&self) -> &'static str {
        "I03"
    }
    fn name(&self) -> &'static str {
        "model.tokens.inclusive"
    }
    fn version(&self) -> &'static str {
        "1.0.0"
    }
    fn domain(&self) -> InsightDomain {
        InsightDomain::Model
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
        if ambiguous_calls(&usage) {
            return refused(self, "ambiguous duplicates", "usage_facts");
        }
        let mut by_session: BTreeMap<String, Vec<&Usage>> = BTreeMap::new();
        for row in &usage {
            by_session.entry(row.session.clone()).or_default().push(row);
        }
        let incomplete = edges.iter().any(|edge| {
            edge.status != "evidenced" || edge.parent.is_empty() || edge.child.is_empty()
        });
        let mut children: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut cycle = false;
        for edge in &edges {
            if edge.status != "evidenced" || edge.parent.is_empty() || edge.child.is_empty() {
                continue;
            }
            if !descendant_relation(&edge.relation) {
                continue;
            }
            children
                .entry(edge.parent.clone())
                .or_default()
                .insert(edge.child.clone());
        }
        let mut nodes: BTreeSet<String> = by_session.keys().cloned().collect();
        for edge in &edges {
            if edge.status == "evidenced" {
                if !edge.parent.is_empty() {
                    nodes.insert(edge.parent.clone());
                }
                if !edge.child.is_empty() {
                    nodes.insert(edge.child.clone());
                }
            }
        }
        let mut measures = Vec::new();
        let mut fleet: BTreeMap<String, i128> = BTreeMap::new();
        let mut negative = false;
        for row in &usage {
            match row.tokens {
                Some(tokens) => {
                    fleet.insert(row.source_key.clone(), tokens);
                }
                None => negative = true,
            }
        }
        for node in &nodes {
            let mut observations: BTreeMap<String, i128> = BTreeMap::new();
            let mut visiting = BTreeSet::new();
            let mut visited = BTreeSet::new();
            visit(
                node,
                &children,
                &by_session,
                &mut visiting,
                &mut visited,
                &mut observations,
                &mut cycle,
            );
            let sum: i128 = observations.values().sum();
            measures.push(token_measure(
                &format!("I03.node.{}", label_id(node)),
                "inclusive_tokens",
                fit_i64(sum).map(MeasureValue::Int),
                "deduped sum of disjoint token components on this node and its evidenced descendants",
            ));
        }
        let fleet_sum: i128 = fleet.values().sum();
        measures.push(token_measure(
            "I03.fleet_deduped",
            "fleet_deduped",
            fit_i64(fleet_sum).map(MeasureValue::Int),
            "each usage source_key once; summing node inclusives is not a fleet total",
        ));
        let status = if negative {
            InsightStatus::Partial {
                reason: "negative token component excluded".to_string(),
            }
        } else if cycle {
            InsightStatus::Partial {
                reason: "lineage cycle".to_string(),
            }
        } else if incomplete {
            InsightStatus::Partial {
                reason: "observed lower bound".to_string(),
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
                n: usage.len() as u64,
            },
            measures,
            coverage: COVERAGE.to_string(),
            evidence: usage.into_iter().map(|row| row.source_key).collect(),
        })
    }
}

fn visit(
    node: &str,
    children: &BTreeMap<String, BTreeSet<String>>,
    by_session: &BTreeMap<String, Vec<&Usage>>,
    visiting: &mut BTreeSet<String>,
    visited: &mut BTreeSet<String>,
    observations: &mut BTreeMap<String, i128>,
    cycle: &mut bool,
) {
    if visiting.contains(node) {
        *cycle = true;
        return;
    }
    if !visited.insert(node.to_string()) {
        return;
    }
    visiting.insert(node.to_string());
    if let Some(rows) = by_session.get(node) {
        for row in rows {
            if let Some(tokens) = row.tokens {
                observations.insert(row.source_key.clone(), tokens);
            }
        }
    }
    if let Some(next) = children.get(node) {
        for child in next {
            visit(
                child,
                children,
                by_session,
                visiting,
                visited,
                observations,
                cycle,
            );
        }
    }
    visiting.remove(node);
}

fn descendant_relation(relation: &str) -> bool {
    matches!(
        relation,
        "spawn" | "delegate" | "delegate_existing" | "handoff" | "resume" | "dependency" | "retry"
    )
}

fn ambiguous_calls(usage: &[Usage]) -> bool {
    let mut seen: BTreeMap<&str, i128> = BTreeMap::new();
    for row in usage {
        let Some(id) = row.message_id.as_deref().filter(|id| !id.is_empty()) else {
            continue;
        };
        let Some(tokens) = row.tokens else {
            continue;
        };
        if let Some(prior) = seen.get(id) {
            if *prior != tokens {
                return true;
            }
        } else {
            seen.insert(id, tokens);
        }
    }
    false
}

fn token_measure(id: &str, name: &str, value: Option<MeasureValue>, basis: &str) -> Measure {
    Measure {
        id: id.to_string(),
        name: name.to_string(),
        value,
        unit: "tokens".to_string(),
        basis: basis.to_string(),
    }
}

fn load_usage(input: &InsightInput<'_>) -> anyhow::Result<Vec<Usage>> {
    let mut stmt = input.connection().prepare(
        "SELECT source_key, native_session, message_id, input_uncached, cache_write, cache_read, output_tokens
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
            let input_uncached: i64 = row.get(3)?;
            let cache_write: i64 = row.get(4)?;
            let cache_read: i64 = row.get(5)?;
            let output: i64 = row.get(6)?;
            Ok(Usage {
                source_key: row.get(0)?,
                session: row.get(1)?,
                message_id: row.get(2)?,
                tokens: disjoint_tokens(input_uncached, cache_write, cache_read, output),
            })
        },
    )?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn load_edges(input: &InsightInput<'_>) -> anyhow::Result<Vec<Edge>> {
    let mut stmt = input.connection().prepare(
        "SELECT parent_session, child_session, relation, status
         FROM session_edges
         WHERE (? IS NULL OR harness = ?)
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
                relation: row.get(2)?,
                status: row.get(3)?,
            })
        },
    )?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}
