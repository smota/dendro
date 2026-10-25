//! I04 model.cost. Observed USD and estimated USD stay on separate measures.
use super::super::contract::{
    finish, model_key, refused, Insight, InsightDomain, InsightInput, InsightPopulation,
    InsightResult, InsightStatus, Measure, MeasureValue, UsageRow,
};
use crate::accounting_v1::{estimate_usd, rollup_costs, CostRollup, CostStatus};
use std::collections::BTreeMap;

pub struct ModelCost;

pub static INSIGHT: ModelCost = ModelCost;

const COVERAGE: &str = "observed and estimated are never added; unknown is not zero";

impl Insight for ModelCost {
    fn id(&self) -> &'static str {
        "I04"
    }
    fn name(&self) -> &'static str {
        "model.cost"
    }
    fn version(&self) -> &'static str {
        "1.0.0"
    }
    fn domain(&self) -> InsightDomain {
        InsightDomain::Model
    }
    fn compute(&self, input: &InsightInput<'_>) -> InsightResult {
        let rows = match input.usage_rows() {
            Ok(rows) => rows,
            Err(_) => return refused(self, "store read failed", "usage_facts"),
        };
        if rows.is_empty() {
            return refused(self, "empty population", "usage_facts");
        }
        let mut groups: BTreeMap<String, Vec<&UsageRow>> = BTreeMap::new();
        for row in &rows {
            groups.entry(model_key(&row.model)).or_default().push(row);
        }
        let mut measures = Vec::new();
        let mut notes = Vec::new();
        for (model, group) in &groups {
            let observed = rollup_costs(group.iter().map(|row| row.currency_total));
            let estimated = rollup_costs(group.iter().map(|row| {
                estimate_usd(
                    row.model.as_deref().filter(|name| !name.is_empty()),
                    row.input_uncached,
                    row.output,
                    row.cache_read,
                    row.cache_write,
                )
            }));
            note(&mut notes, model, "observed", observed);
            note(&mut notes, model, "estimated", estimated);
            measures.push(cost_measure(model, "observed_usd", observed));
            measures.push(cost_measure(model, "estimated_usd", estimated));
        }
        let status = if notes.is_empty() {
            InsightStatus::Computed
        } else {
            InsightStatus::Partial {
                reason: notes.join("; "),
            }
        };
        finish(InsightResult {
            id: self.id(),
            name: self.name(),
            version: self.version(),
            domain: self.domain(),
            status,
            population: InsightPopulation {
                what: "usage_facts".to_string(),
                n: rows.len() as u64,
            },
            measures,
            coverage: COVERAGE.to_string(),
            evidence: rows.into_iter().map(|row| row.source_key).collect(),
        })
    }
}

fn note(notes: &mut Vec<String>, model: &str, label: &str, rollup: CostRollup) {
    if rollup.status == CostStatus::Complete {
        return;
    }
    notes.push(format!(
        "{model} {label}={} unknown_rows={}",
        status_name(rollup.status),
        rollup.unknown_rows
    ));
}

fn cost_measure(model: &str, name: &str, rollup: CostRollup) -> Measure {
    Measure {
        id: format!("I04.{model}.{name}"),
        name: name.to_string(),
        value: rollup.total_usd().map(MeasureValue::Float),
        unit: "usd".to_string(),
        basis: format!(
            "status={}; known_rows={}; unknown_rows={}",
            status_name(rollup.status),
            rollup.known_rows,
            rollup.unknown_rows
        ),
    }
}

fn status_name(status: CostStatus) -> &'static str {
    match status {
        CostStatus::Complete => "complete",
        CostStatus::Partial => "partial",
        CostStatus::Unknown => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insight_v1::{evaluate, Scope};
    use crate::AnalyticsStore;
    use duckdb::params;

    fn store() -> AnalyticsStore {
        AnalyticsStore::open_in_memory("insight-i04").unwrap()
    }

    fn insert(store: &AnalyticsStore, key: &str, model: &str, usd: Option<f64>) {
        store
            .connection()
            .execute(
                "INSERT INTO usage_facts (source_key, harness, native_session, ts, seq, input_uncached, output_tokens, cache_read, cache_write, reasoning, currency_total, generation, source_id, model)
                 VALUES (?, 'claude', 's', NULL, 1, 10, 1, 0, 0, 0, ?, 1, 'src', ?)",
                params![key, usd, model],
            )
            .unwrap();
    }

    fn only(store: &AnalyticsStore) -> InsightResult {
        evaluate(store, &Scope::default())
            .into_iter()
            .find(|result| result.id == "I04")
            .unwrap()
    }

    fn named<'a>(result: &'a InsightResult, id: &str) -> &'a Measure {
        result
            .measures
            .iter()
            .find(|measure| measure.id == id)
            .unwrap()
    }

    #[test]
    fn unknown_observed_cost_is_partial_not_the_smaller_sum() {
        let store = store();
        insert(&store, "k1", "m", Some(4.0));
        insert(&store, "k2", "m", None);
        let result = only(&store);
        assert!(matches!(result.status, InsightStatus::Partial { .. }));
        let observed = named(&result, "I04.m.observed_usd");
        assert_eq!(observed.value, None);
        assert!(observed.basis.contains("status=partial"));
        assert!(observed.basis.contains("known_rows=1"));
        assert!(observed.basis.contains("unknown_rows=1"));
        assert!(!observed.basis.contains("4"));
        let estimated = named(&result, "I04.m.estimated_usd");
        assert_eq!(estimated.value, None);
        assert!(estimated.basis.contains("status=unknown"));
        assert!(result
            .measures
            .iter()
            .all(|measure| measure.value != Some(MeasureValue::Float(0.0))));
        assert!(result
            .measures
            .iter()
            .all(|measure| measure.value != Some(MeasureValue::Float(4.0))));
        assert_eq!(result.measures.len(), 2);
        assert_eq!(result, only(&store));
    }

    #[test]
    fn complete_observed_stays_apart_from_unknown_estimate() {
        let store = store();
        insert(&store, "k1", "m", Some(1.5));
        insert(&store, "k2", "m", Some(2.5));
        let result = only(&store);
        assert!(matches!(result.status, InsightStatus::Partial { .. }));
        assert_eq!(
            named(&result, "I04.m.observed_usd").value,
            Some(MeasureValue::Float(4.0))
        );
        assert_eq!(named(&result, "I04.m.estimated_usd").value, None);
        assert!(result.coverage.contains("never added"));
    }

    #[test]
    fn empty_population_is_refused() {
        let result = only(&store());
        assert!(matches!(
            result.status,
            InsightStatus::Refused { reason } if reason == "empty population"
        ));
        assert!(result.measures.is_empty());
    }
}
