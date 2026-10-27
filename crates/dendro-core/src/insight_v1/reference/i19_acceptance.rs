//! I19 outcome.acceptance. Accepted work stays separate from attempt success.
use super::super::contract::{
    finish, refused, Insight, InsightDomain, InsightInput, InsightPopulation, InsightResult,
    InsightStatus,
};
use super::support::{count_measure, float_measure, table_exists};
use duckdb::params;
use std::collections::BTreeMap;

pub struct Acceptance;

pub static INSIGHT: Acceptance = Acceptance;

const COVERAGE: &str =
    "attempt success is not acceptance; revocations and supersession use the latest mapped closure";

#[derive(Clone)]
pub(super) struct Transition {
    pub key: String,
    pub item: String,
    pub ts: String,
    pub transition: i64,
    pub closure_reason: Option<String>,
}

#[derive(Clone, Debug)]
pub(super) struct AcceptanceCount {
    pub accepted: i64,
    pub evaluated: i64,
    pub attempt_success: i64,
    pub unevaluated: i64,
    pub evidence: Vec<String>,
    pub accepted_items: Vec<String>,
}

impl Insight for Acceptance {
    fn id(&self) -> &'static str {
        "I19"
    }
    fn name(&self) -> &'static str {
        "outcome.acceptance"
    }
    fn version(&self) -> &'static str {
        "1.0.0"
    }
    fn domain(&self) -> InsightDomain {
        InsightDomain::Coordination
    }
    fn compute(&self, input: &InsightInput<'_>) -> InsightResult {
        match load_acceptance(input) {
            Ok(None) => refused(self, "not_observable", "queue_facts"),
            Ok(Some(count)) => finish(answer(self, count)),
            Err(_) => refused(self, "store read failed", "queue_facts"),
        }
    }
}

pub(super) fn load_acceptance(input: &InsightInput<'_>) -> anyhow::Result<Option<AcceptanceCount>> {
    if !table_exists(input.connection(), "acceptance_rules") {
        return Ok(None);
    }
    let rules = load_rules(input)?;
    if rules.is_empty() {
        return Ok(None);
    }
    let rows = load_queue(input)?;
    Ok(Some(count_acceptance(&rows, &rules)))
}

pub(super) fn count_acceptance(
    rows: &[Transition],
    rules: &BTreeMap<String, String>,
) -> AcceptanceCount {
    let mut by_item: BTreeMap<&str, Vec<&Transition>> = BTreeMap::new();
    for row in rows {
        by_item.entry(row.item.as_str()).or_default().push(row);
    }
    let mut count = AcceptanceCount {
        accepted: 0,
        evaluated: 0,
        attempt_success: 0,
        unevaluated: 0,
        evidence: rows.iter().map(|row| row.key.clone()).collect(),
        accepted_items: Vec::new(),
    };
    for (item, mut group) in by_item {
        group.sort_by(|left, right| {
            left.ts
                .cmp(&right.ts)
                .then(left.transition.cmp(&right.transition))
        });
        let latest = group.iter().rev().find_map(|row| {
            row.closure_reason
                .as_deref()
                .and_then(|reason| rules.get(reason))
        });
        match latest.map(String::as_str) {
            Some("accepted") => {
                count.accepted += 1;
                count.evaluated += 1;
                count.accepted_items.push(item.to_string());
            }
            Some("revoked" | "superseded") => count.evaluated += 1,
            Some("attempt_success") => {
                count.attempt_success += 1;
                count.evaluated += 1;
            }
            _ => count.unevaluated += 1,
        }
    }
    count
}

fn answer(insight: &Acceptance, count: AcceptanceCount) -> InsightResult {
    let rate = if count.evaluated > 0 {
        Some(count.accepted as f64 / count.evaluated as f64)
    } else {
        None
    };
    let status = if count.evaluated == 0 {
        InsightStatus::Partial {
            reason: "missing denominator".to_string(),
        }
    } else {
        InsightStatus::Computed
    };
    InsightResult {
        id: insight.id(),
        name: insight.name(),
        version: insight.version(),
        domain: insight.domain(),
        status,
        population: InsightPopulation {
            what: "queue_facts".to_string(),
            n: count.evidence.len() as u64,
        },
        measures: vec![
            count_measure(
                "I19.accepted",
                "accepted",
                count.accepted,
                "work",
                "latest mapped effect is accepted and has not been revoked or superseded",
            ),
            count_measure(
                "I19.evaluated",
                "evaluated",
                count.evaluated,
                "work",
                "work whose latest closure maps through the acceptance rule",
            ),
            count_measure(
                "I19.attempt_success",
                "attempt_success",
                count.attempt_success,
                "work",
                "attempt success kept separate from acceptance",
            ),
            count_measure(
                "I19.unevaluated",
                "unevaluated",
                count.unevaluated,
                "work",
                "work with no rule-mapped closure",
            ),
            float_measure(
                "I19.acceptance_rate",
                "acceptance_rate",
                rate,
                "ratio",
                "accepted / evaluated; unknown when no work was evaluated",
            ),
        ],
        coverage: COVERAGE.to_string(),
        evidence: count.evidence,
    }
}

fn load_rules(input: &InsightInput<'_>) -> anyhow::Result<BTreeMap<String, String>> {
    let mut stmt = input
        .connection()
        .prepare("SELECT closure_reason, effect FROM acceptance_rules ORDER BY closure_reason")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut rules = BTreeMap::new();
    for row in rows {
        let (reason, effect) = row?;
        if matches!(
            effect.as_str(),
            "accepted" | "revoked" | "superseded" | "attempt_success"
        ) {
            rules.insert(reason, effect);
        }
    }
    Ok(rules)
}

fn load_queue(input: &InsightInput<'_>) -> anyhow::Result<Vec<Transition>> {
    let mut stmt = input.connection().prepare(
        "SELECT source_key, qitem_id, CAST(ts AS VARCHAR), transition_id, closure_reason
         FROM queue_facts
         WHERE (? IS NULL OR qitem_id = ?)
           AND (? IS NULL OR ts >= CAST(? AS TIMESTAMP))
           AND (? IS NULL OR ts < CAST(? AS TIMESTAMP))
         ORDER BY source_key",
    )?;
    let scope = input.scope();
    let session = scope.session.as_deref();
    let from = scope.from.as_deref();
    let to = scope.to.as_deref();
    let rows = stmt.query_map(params![session, session, from, from, to, to], |row| {
        Ok(Transition {
            key: row.get(0)?,
            item: row.get(1)?,
            ts: row.get(2)?,
            transition: row.get(3)?,
            closure_reason: row.get(4)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}
