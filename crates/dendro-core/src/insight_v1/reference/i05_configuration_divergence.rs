//! I05 model.configuration_divergence. Paired observed model and effort bindings.
use super::super::contract::{
    finish, refused, Insight, InsightDomain, InsightInput, InsightPopulation, InsightResult,
    InsightStatus,
};
use super::support::{count_measure, table_exists};
use duckdb::params;
use std::collections::BTreeMap;

pub struct ConfigurationDivergence;

pub static INSIGHT: ConfigurationDivergence = ConfigurationDivergence;

const COVERAGE: &str =
    "paired observed bindings only; unknown model or effort is excluded and counted; divergence is not a violation";

#[derive(Clone)]
struct Binding {
    harness: String,
    session: String,
    model: Option<String>,
    effort: Option<String>,
}

impl Insight for ConfigurationDivergence {
    fn id(&self) -> &'static str {
        "I05"
    }
    fn name(&self) -> &'static str {
        "model.configuration_divergence"
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
        if table_exists(input.connection(), "desired_bindings") {
            let wanted = match load_desired(input) {
                Ok(rows) => rows,
                Err(_) => return refused(self, "store read failed", "desired_bindings"),
            };
            if !wanted.is_empty() {
                return against_desired(self, &rows, &wanted);
            }
        }
        paired_observed(self, &rows)
    }
}

fn paired_observed(insight: &ConfigurationDivergence, rows: &[Binding]) -> InsightResult {
    let mut grouped: BTreeMap<String, BTreeMap<String, (String, String)>> = BTreeMap::new();
    let mut excluded = 0i64;
    for row in rows {
        match (text(row.model.as_deref()), text(row.effort.as_deref())) {
            (Some(model), Some(effort)) => {
                grouped
                    .entry(row.harness.clone())
                    .or_default()
                    .insert(row.session.clone(), (model, effort));
            }
            _ => excluded += 1,
        }
    }
    if grouped.values().all(|sessions| sessions.len() < 2) {
        let mut result = refused(insight, "not_observable", "usage_facts");
        result.population.n = rows.len() as u64;
        result.coverage = COVERAGE.to_string();
        result.measures = vec![excluded_measure(excluded)];
        result.evidence = rows.iter().map(|row| row.session.clone()).collect();
        return finish(result);
    }
    let mut divergent = 0i64;
    let mut matched = 0i64;
    for sessions in grouped.values() {
        let keys: Vec<&String> = sessions.keys().collect();
        for left in 0..keys.len() {
            for right in (left + 1)..keys.len() {
                if sessions[keys[left]] == sessions[keys[right]] {
                    matched += 1;
                } else {
                    divergent += 1;
                }
            }
        }
    }
    counts(insight, rows, divergent, matched, excluded, COVERAGE)
}

fn against_desired(
    insight: &ConfigurationDivergence,
    observed: &[Binding],
    desired: &[Binding],
) -> InsightResult {
    let index: BTreeMap<(&str, &str), (String, String)> = desired
        .iter()
        .filter_map(|row| {
            Some((
                (row.harness.as_str(), row.session.as_str()),
                (text(row.model.as_deref())?, text(row.effort.as_deref())?),
            ))
        })
        .collect();
    let mut excluded = 0i64;
    let mut divergent = 0i64;
    let mut matched = 0i64;
    for row in observed {
        let Some(model) = text(row.model.as_deref()) else {
            excluded += 1;
            continue;
        };
        let Some(effort) = text(row.effort.as_deref()) else {
            excluded += 1;
            continue;
        };
        match index.get(&(&row.harness, &row.session)) {
            None => excluded += 1,
            Some(wanted) if wanted == &(model, effort) => matched += 1,
            Some(_) => divergent += 1,
        }
    }
    if matched + divergent == 0 {
        return refused(insight, "not_observable", "usage_facts");
    }
    counts(
        insight,
        observed,
        divergent,
        matched,
        excluded,
        "contemporaneous desired bindings; unknown sides are excluded; divergence is not a violation",
    )
}

fn counts(
    insight: &ConfigurationDivergence,
    rows: &[Binding],
    divergent: i64,
    matched: i64,
    excluded: i64,
    coverage: &str,
) -> InsightResult {
    finish(InsightResult {
        id: insight.id(),
        name: insight.name(),
        version: insight.version(),
        domain: insight.domain(),
        status: InsightStatus::Computed,
        population: InsightPopulation {
            what: "usage_facts".to_string(),
            n: rows.len() as u64,
        },
        measures: vec![
            count_measure(
                "I05.divergent_pairs",
                "divergent_pairs",
                divergent,
                "pairs",
                "bindings whose observed model or effort differs from the paired binding",
            ),
            count_measure(
                "I05.matched_pairs",
                "matched_pairs",
                matched,
                "pairs",
                "bindings whose observed model and effort match",
            ),
            excluded_measure(excluded),
        ],
        coverage: coverage.to_string(),
        evidence: rows.iter().map(|row| row.session.clone()).collect(),
    })
}

fn excluded_measure(excluded: i64) -> super::super::contract::Measure {
    count_measure(
        "I05.excluded_unknown",
        "excluded",
        excluded,
        "sessions",
        "sessions missing model, effort, or a paired desired binding",
    )
}

fn text(value: Option<&str>) -> Option<String> {
    value.filter(|text| !text.is_empty()).map(str::to_string)
}

fn load(input: &InsightInput<'_>) -> anyhow::Result<Vec<Binding>> {
    let mut stmt = input.connection().prepare(
        "SELECT harness, native_session, model, effort
         FROM usage_facts
         WHERE (? IS NULL OR harness = ?)
           AND (? IS NULL OR native_session = ?)
           AND (? IS NULL OR ts >= CAST(? AS TIMESTAMP))
           AND (? IS NULL OR ts < CAST(? AS TIMESTAMP))
         ORDER BY source_key",
    )?;
    bind_rows(input, &mut stmt)
}

fn load_desired(input: &InsightInput<'_>) -> anyhow::Result<Vec<Binding>> {
    let mut stmt = input.connection().prepare(
        "SELECT harness, native_session, model, effort
         FROM desired_bindings
         WHERE (? IS NULL OR harness = ?)
           AND (? IS NULL OR native_session = ?)
         ORDER BY harness, native_session",
    )?;
    let scope = input.scope();
    let harness = scope.harness.as_deref();
    let session = scope.session.as_deref();
    let rows = stmt.query_map(params![harness, harness, session, session], read_binding)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn bind_rows(
    input: &InsightInput<'_>,
    stmt: &mut duckdb::Statement<'_>,
) -> anyhow::Result<Vec<Binding>> {
    let scope = input.scope();
    let harness = scope.harness.as_deref();
    let session = scope.session.as_deref();
    let from = scope.from.as_deref();
    let to = scope.to.as_deref();
    let rows = stmt.query_map(
        params![harness, harness, session, session, from, from, to, to],
        read_binding,
    )?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn read_binding(row: &duckdb::Row<'_>) -> duckdb::Result<Binding> {
    Ok(Binding {
        harness: row.get(0)?,
        session: row.get(1)?,
        model: row.get(2)?,
        effort: row.get(3)?,
    })
}
