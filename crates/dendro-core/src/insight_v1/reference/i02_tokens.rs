//! I02 model.tokens.direct. Disjoint token sums, plus two cache-read ratios.
use super::super::contract::{
    finish, model_key, refused, Insight, InsightDomain, InsightInput, InsightPopulation,
    InsightResult, InsightStatus, Measure, MeasureValue, UsageRow,
};
use std::collections::BTreeMap;

pub struct TokensDirect;

pub static INSIGHT: TokensDirect = TokensDirect;

const COVERAGE: &str = "reasoning is included in output and is not part of the disjoint sum";

impl Insight for TokensDirect {
    fn id(&self) -> &'static str {
        "I02"
    }
    fn name(&self) -> &'static str {
        "model.tokens.direct"
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
        let mut undefined = false;
        let mut measures = Vec::new();
        for (model, group) in &groups {
            let mut input_uncached = 0i128;
            let mut cache_write = 0i128;
            let mut cache_read = 0i128;
            let mut output = 0i128;
            let mut reasoning = 0i128;
            let mut defined = 0u64;
            let mut ratio_sum = 0.0f64;
            for row in group {
                input_uncached += i128::from(row.input_uncached);
                cache_write += i128::from(row.cache_write);
                cache_read += i128::from(row.cache_read);
                output += i128::from(row.output);
                reasoning += i128::from(row.reasoning);
                let negative = [
                    row.input_uncached,
                    row.cache_write,
                    row.cache_read,
                    row.output,
                    row.reasoning,
                ]
                .iter()
                .any(|value| *value < 0);
                let denom = i128::from(row.input_uncached)
                    + i128::from(row.cache_write)
                    + i128::from(row.cache_read);
                if negative || denom <= 0 {
                    undefined = true;
                } else {
                    defined += 1;
                    ratio_sum += row.cache_read as f64 / denom as f64;
                }
            }
            let prefix = format!("I02.{model}");
            push_sum(
                &mut measures,
                &prefix,
                "input_uncached",
                input_uncached,
                "sum",
            );
            push_sum(&mut measures, &prefix, "cache_write", cache_write, "sum");
            push_sum(&mut measures, &prefix, "cache_read", cache_read, "sum");
            push_sum(
                &mut measures,
                &prefix,
                "output",
                output,
                "output_tokens; reasoning is included in this component",
            );
            push_sum(
                &mut measures,
                &prefix,
                "reasoning",
                reasoning,
                "included in output; not added to the disjoint sum or to either cache ratio",
            );
            let denom = input_uncached + cache_write + cache_read;
            let ratio_of_sums = if denom > 0 {
                Some(cache_read as f64 / denom as f64)
            } else {
                undefined = true;
                None
            };
            let mean = if defined > 0 {
                Some(ratio_sum / defined as f64)
            } else {
                None
            };
            measures.push(ratio_measure(
                &prefix,
                "cache_read_ratio_of_sums",
                ratio_of_sums,
                "sum(cache_read)/sum(input_uncached+cache_write+cache_read)",
            ));
            measures.push(ratio_measure(
                &prefix,
                "cache_read_mean_of_ratios",
                mean,
                "mean of per-invocation cache_read/(input_uncached+cache_write+cache_read); undefined invocations omitted",
            ));
        }
        let status = if undefined {
            InsightStatus::Partial {
                reason: "undefined cache ratio".to_string(),
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
                n: rows.len() as u64,
            },
            measures,
            coverage: COVERAGE.to_string(),
            evidence: rows.into_iter().map(|row| row.source_key).collect(),
        })
    }
}

fn push_sum(measures: &mut Vec<Measure>, prefix: &str, name: &str, sum: i128, basis: &str) {
    let value = i64::try_from(sum).ok().map(MeasureValue::Int);
    measures.push(Measure {
        id: format!("{prefix}.{name}"),
        name: name.to_string(),
        value,
        unit: "tokens".to_string(),
        basis: basis.to_string(),
    });
}

fn ratio_measure(prefix: &str, name: &str, value: Option<f64>, basis: &str) -> Measure {
    Measure {
        id: format!("{prefix}.{name}"),
        name: name.to_string(),
        value: value.map(MeasureValue::Float),
        unit: "ratio".to_string(),
        basis: basis.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insight_v1::contract::MeasureValue;
    use crate::insight_v1::{evaluate, Scope};
    use crate::AnalyticsStore;
    use duckdb::params;

    fn store() -> AnalyticsStore {
        AnalyticsStore::open_in_memory("insight-i02").unwrap()
    }

    fn insert(store: &AnalyticsStore, key: &str, harness: &str, ts: Option<&str>, parts: [i64; 5]) {
        let [input_uncached, cache_write, cache_read, output, reasoning] = parts;
        store
            .connection()
            .execute(
                "INSERT INTO usage_facts (source_key, harness, native_session, ts, seq, input_uncached, output_tokens, cache_read, cache_write, reasoning, generation, source_id, model)
                 VALUES (?, ?, 's', CAST(? AS TIMESTAMP), 1, ?, ?, ?, ?, ?, 1, 'src', 'm')",
                params![
                    key,
                    harness,
                    ts,
                    input_uncached,
                    output,
                    cache_read,
                    cache_write,
                    reasoning
                ],
            )
            .unwrap();
    }

    fn only(store: &AnalyticsStore, scope: &Scope) -> InsightResult {
        evaluate(store, scope)
            .into_iter()
            .find(|result| result.id == "I02")
            .unwrap()
    }

    fn float_at(result: &InsightResult, id: &str) -> Option<f64> {
        let measure = result
            .measures
            .iter()
            .find(|measure| measure.id == id)
            .unwrap();
        match measure.value {
            Some(MeasureValue::Float(value)) => Some(value),
            None => None,
            Some(other) => panic!("{id} was {other}"),
        }
    }

    #[test]
    fn ratio_of_sums_and_mean_of_ratios_stay_separate() {
        let store = store();
        insert(&store, "k-b", "claude", None, [9, 0, 1, 7, 2]);
        insert(&store, "k-a", "claude", None, [10, 0, 90, 5, 1]);
        let result = only(&store, &Scope::default());
        assert!(matches!(result.status, InsightStatus::Computed));
        assert_eq!(result.population.n, 2);
        let ratio = float_at(&result, "I02.m.cache_read_ratio_of_sums").unwrap();
        let mean = float_at(&result, "I02.m.cache_read_mean_of_ratios").unwrap();
        assert!((ratio - (91.0 / 110.0)).abs() < 1e-12);
        assert!((mean - ((90.0 / 100.0 + 1.0 / 10.0) / 2.0)).abs() < 1e-12);
        assert!((mean - 0.5).abs() < 1e-9);
        assert_ne!(ratio, mean);
        let output = result
            .measures
            .iter()
            .find(|measure| measure.id == "I02.m.output")
            .unwrap();
        let reasoning = result
            .measures
            .iter()
            .find(|measure| measure.id == "I02.m.reasoning")
            .unwrap();
        assert_eq!(output.value, Some(MeasureValue::Int(12)));
        assert_eq!(reasoning.value, Some(MeasureValue::Int(3)));
        assert!(reasoning.basis.contains("included in output"));
        assert_eq!(result.evidence, vec!["k-a".to_string(), "k-b".to_string()]);
        let again = only(&store, &Scope::default());
        assert_eq!(result, again);
    }

    #[test]
    fn empty_population_is_refused() {
        let result = only(&store(), &Scope::default());
        assert!(matches!(
            result.status,
            InsightStatus::Refused { reason } if reason == "empty population"
        ));
        assert!(result.measures.is_empty());
    }

    #[test]
    fn undefined_denominator_is_not_zero() {
        let store = store();
        insert(&store, "k", "claude", None, [0, 0, 0, 4, 0]);
        let result = only(&store, &Scope::default());
        assert!(matches!(result.status, InsightStatus::Partial { .. }));
        assert_eq!(float_at(&result, "I02.m.cache_read_ratio_of_sums"), None);
        assert_eq!(float_at(&result, "I02.m.cache_read_mean_of_ratios"), None);
    }

    #[test]
    fn inverted_window_is_undefined() {
        let store = store();
        insert(&store, "k", "claude", None, [10, 0, 90, 1, 0]);
        let result = only(
            &store,
            &Scope {
                from: Some("2026-10-17 00:00:00".into()),
                to: Some("2026-10-16 00:00:00".into()),
                ..Scope::default()
            },
        );
        assert!(matches!(
            result.status,
            InsightStatus::Refused { reason } if reason == "undefined input"
        ));
    }

    #[test]
    fn harness_and_window_keep_one_invocation() {
        let store = store();
        insert(
            &store,
            "in",
            "claude",
            Some("2026-10-15 00:00:00"),
            [10, 0, 90, 1, 0],
        );
        insert(
            &store,
            "out",
            "claude",
            Some("2026-10-16 00:00:00"),
            [9, 0, 1, 1, 0],
        );
        insert(
            &store,
            "other",
            "pi",
            Some("2026-10-15 00:00:00"),
            [1, 0, 1, 1, 0],
        );
        let result = only(
            &store,
            &Scope {
                harness: Some("claude".into()),
                from: Some("2026-10-15 00:00:00".into()),
                to: Some("2026-10-16 00:00:00".into()),
                ..Scope::default()
            },
        );
        assert_eq!(result.population.n, 1);
        let ratio = float_at(&result, "I02.m.cache_read_ratio_of_sums").unwrap();
        assert!((ratio - 0.9).abs() < 1e-12);
        assert_eq!(result.evidence, vec!["in".to_string()]);
    }

    #[test]
    fn evidence_is_five_sorted_source_keys() {
        let store = store();
        for key in ["k6", "k2", "k4", "k1", "k5", "k3"] {
            insert(&store, key, "claude", None, [1, 0, 1, 1, 0]);
        }
        let result = only(&store, &Scope::default());
        assert_eq!(
            result.evidence,
            vec!["k1", "k2", "k3", "k4", "k5"]
                .into_iter()
                .map(str::to_string)
                .collect::<Vec<_>>()
        );
    }
}
