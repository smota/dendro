//! I24 security.authority_blast_radius. Evidenced reachability, not authority.
use super::super::contract::{
    finish, refused, Insight, InsightDomain, InsightInput, InsightPopulation, InsightResult,
    InsightStatus,
};
use super::support::{count_measure, unknown_measure};
use crate::lineage_graph_v1::{
    i24_blast_radius, AnswerStatus, HopCap, SecurityObservation, DEPENDENCY_EXPOSURE_NON_CLAIM,
};
use crate::session_lineage::SessionLink;
use duckdb::params;

pub struct BlastRadiusInsight;

pub static INSIGHT: BlastRadiusInsight = BlastRadiusInsight;

const COVERAGE: &str = "\
downstream session count is dependency exposure and not security authority \
| assumption: only status=evidenced edges are walked and the hop cap is 8 \
| alternative: a downstream session can be reachable without holding the revoked grant \
| formula: bounded evidenced walk from a revoked or failed grant or an unverified sandbox \
| dependency_exposure_only: structural reachability does not imply inherited or effective security authority";

impl Insight for BlastRadiusInsight {
    fn id(&self) -> &'static str {
        "I24"
    }
    fn name(&self) -> &'static str {
        "security.authority_blast_radius"
    }
    fn version(&self) -> &'static str {
        "1.0.0"
    }
    fn domain(&self) -> InsightDomain {
        InsightDomain::Security
    }
    fn compute(&self, input: &InsightInput<'_>) -> InsightResult {
        let (edges, edge_keys) = match load_edges(input) {
            Ok(rows) => rows,
            Err(_) => return refused(self, "store read failed", "session_edges"),
        };
        let (facts, fact_keys) = match load_facts(input) {
            Ok(rows) => rows,
            Err(_) => return refused(self, "store read failed", "security_facts"),
        };
        let answer = match i24_blast_radius(&edges, &facts, HopCap::DEFAULT) {
            Ok(answer) => answer,
            Err(_) => return refused(self, "store read failed", "session_edges"),
        };
        debug_assert!(!answer.rank_statistic_used);
        debug_assert_eq!(answer.non_claim, DEPENDENCY_EXPOSURE_NON_CLAIM);
        let (status, downstream) = match answer.status {
            AnswerStatus::NotAssessed => (
                InsightStatus::Refused {
                    reason: "NOT_ASSESSED".to_string(),
                },
                None,
            ),
            AnswerStatus::Partial => (
                InsightStatus::Partial {
                    reason: "hop limit exceeded".to_string(),
                },
                Some(i64::try_from(answer.downstream_count).unwrap_or(i64::MAX)),
            ),
            AnswerStatus::Computed => (
                InsightStatus::Computed,
                Some(i64::try_from(answer.downstream_count).unwrap_or(i64::MAX)),
            ),
            AnswerStatus::Refused => (
                InsightStatus::Refused {
                    reason: "refused".to_string(),
                },
                None,
            ),
        };
        let mut evidence = edge_keys;
        evidence.extend(fact_keys);
        let population = edges.len() as u64;
        let downstream_measure = match downstream {
            Some(count) => count_measure(
                "I24.downstream_sessions",
                "downstream_sessions",
                count,
                "sessions",
                "sessions reached from a revoked grant, failed grant, or unverified sandbox; seeds are excluded",
            ),
            None => unknown_measure(
                "I24.downstream_sessions",
                "downstream_sessions",
                "sessions",
                "unknown because no applicable decision exists; this is not a zero violation count",
            ),
        };
        finish(InsightResult {
            id: self.id(),
            name: self.name(),
            version: self.version(),
            domain: self.domain(),
            status,
            population: InsightPopulation {
                what: "session_edges".to_string(),
                n: population,
            },
            measures: vec![
                downstream_measure,
                count_measure(
                    "I24.truncated_frontier",
                    "truncated_frontier",
                    i64::try_from(answer.truncated_frontier_count).unwrap_or(i64::MAX),
                    "sessions",
                    "sessions past hop cap 8; the walk stops and counts the frontier",
                ),
                count_measure(
                    "I24.cycles",
                    "cycles",
                    answer.cycles.len() as i64,
                    "cycles",
                    "cycles touched by the evidenced walk; edges are reported and not deleted",
                ),
                unknown_measure(
                    "I24.artifacts",
                    "artifacts",
                    "artifacts",
                    "not_observable; no typed artifact identity is stored on this graph",
                ),
            ],
            coverage: COVERAGE.to_string(),
            evidence,
        })
    }
}

fn load_edges(input: &InsightInput<'_>) -> anyhow::Result<(Vec<SessionLink>, Vec<String>)> {
    let mut stmt = input.connection().prepare(
        "SELECT source_key, parent_session, child_session, relation, status
         FROM session_edges
         WHERE (? IS NULL OR harness = ?)
         ORDER BY source_key",
    )?;
    let harness = input.scope().harness.as_deref();
    let rows = stmt.query_map(params![harness, harness], |row| {
        Ok((
            row.get::<_, String>(0)?,
            SessionLink {
                parent_session: row.get(1)?,
                child_session: row.get(2)?,
                relation: row.get(3)?,
                status: row.get(4)?,
            },
        ))
    })?;
    let mut edges = Vec::new();
    let mut keys = Vec::new();
    for row in rows {
        let (key, edge) = row?;
        keys.push(key);
        edges.push(edge);
    }
    Ok((edges, keys))
}

fn load_facts(input: &InsightInput<'_>) -> anyhow::Result<(Vec<SecurityObservation>, Vec<String>)> {
    let mut stmt = input.connection().prepare(
        "SELECT source_key, native_session, kind, value
         FROM security_facts
         WHERE (? IS NULL OR harness = ?)
           AND (? IS NULL OR native_session = ?)
         ORDER BY source_key",
    )?;
    let harness = input.scope().harness.as_deref();
    let session = input.scope().session.as_deref();
    let rows = stmt.query_map(params![harness, harness, session, session], |row| {
        Ok((
            row.get::<_, String>(0)?,
            SecurityObservation {
                native_session: row.get(1)?,
                kind: row.get(2)?,
                value: row.get(3)?,
            },
        ))
    })?;
    let mut facts = Vec::new();
    let mut keys = Vec::new();
    for row in rows {
        let (key, fact) = row?;
        keys.push(key);
        facts.push(fact);
    }
    Ok((facts, keys))
}
