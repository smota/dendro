//! I08 harness.action_failure. Settled failure rate and unsettled count per harness.
use super::super::contract::{
    finish, label_id, refused, Insight, InsightDomain, InsightInput, InsightPopulation,
    InsightResult, InsightStatus, Measure, MeasureValue,
};
use crate::facts::READER_NAMESPACE;
use duckdb::params;
use std::collections::BTreeMap;

pub struct ActionFailure;

pub static INSIGHT: ActionFailure = ActionFailure;

const COVERAGE: &str =
    "settled terminal actions (failed IS NOT NULL) form the denominator; unsettled (failed IS NULL) are counted separately and never treated as succeeded";

#[derive(Debug, Clone)]
struct ToolOutcomeRow {
    source_key: String,
    harness: String,
    failed: Option<bool>,
}

impl Insight for ActionFailure {
    fn id(&self) -> &'static str {
        "I08"
    }
    fn name(&self) -> &'static str {
        "harness.action_failure"
    }
    fn version(&self) -> &'static str {
        "1.0.0"
    }
    fn domain(&self) -> InsightDomain {
        InsightDomain::Harness
    }
    fn compute(&self, input: &InsightInput<'_>) -> InsightResult {
        let rows = match load_tool_outcomes(input) {
            Ok(rows) => rows,
            Err(_) => return refused(self, "store read failed", "tool_facts"),
        };
        if rows.is_empty() {
            return refused(self, "empty population", "tool_facts");
        }
        let mut groups: BTreeMap<String, Vec<&ToolOutcomeRow>> = BTreeMap::new();
        for row in &rows {
            let key = if row.harness.is_empty() {
                "absent".to_string()
            } else {
                label_id(&row.harness)
            };
            groups.entry(key).or_default().push(row);
        }
        let mut any_unsettled = false;
        let mut any_zero_settled = false;
        let mut measures = Vec::new();
        for (harness, group) in &groups {
            let mut failed = 0i64;
            let mut settled = 0i64;
            let mut unsettled = 0i64;
            for row in group {
                match row.failed {
                    Some(true) => {
                        failed += 1;
                        settled += 1;
                    }
                    Some(false) => {
                        settled += 1;
                    }
                    None => {
                        unsettled += 1;
                    }
                }
            }
            if unsettled > 0 {
                any_unsettled = true;
            }
            if settled == 0 {
                any_zero_settled = true;
            }
            let prefix = format!("I08.{harness}");
            measures.push(count_measure(
                &prefix,
                "failed",
                failed,
                "count(failed = true); settled terminal actions only",
            ));
            measures.push(count_measure(
                &prefix,
                "settled",
                settled,
                "count(failed IS NOT NULL); denominator for failure_rate",
            ));
            measures.push(count_measure(
                &prefix,
                "unsettled",
                unsettled,
                "count(failed IS NULL); excluded from failure_rate denominator",
            ));
            let failure_rate = if settled > 0 {
                Some(MeasureValue::Float(failed as f64 / settled as f64))
            } else {
                None
            };
            measures.push(Measure {
                id: format!("{prefix}.failure_rate"),
                name: "failure_rate".to_string(),
                value: failure_rate,
                unit: "ratio".to_string(),
                basis: "failed/settled where failed IS NOT NULL; unknown when settled is 0"
                    .to_string(),
            });
        }
        let status = if any_unsettled && any_zero_settled {
            InsightStatus::Partial {
                reason: "unsettled actions and undefined failure rate".to_string(),
            }
        } else if any_unsettled {
            InsightStatus::Partial {
                reason: "unsettled actions excluded from failure rate denominator".to_string(),
            }
        } else if any_zero_settled {
            InsightStatus::Partial {
                reason: "undefined failure rate".to_string(),
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
                what: "tool_facts".to_string(),
                n: rows.len() as u64,
            },
            measures,
            coverage: COVERAGE.to_string(),
            evidence: rows.into_iter().map(|row| row.source_key).collect(),
        })
    }
}

fn load_tool_outcomes(input: &InsightInput<'_>) -> anyhow::Result<Vec<ToolOutcomeRow>> {
    let mut stmt = input.connection().prepare(
        "SELECT source_key, harness, failed
         FROM tool_facts
         WHERE NOT starts_with(source_key, ?)
           AND (? IS NULL OR harness = ?)
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
        params![
            READER_NAMESPACE,
            harness,
            harness,
            session,
            session,
            from,
            from,
            to,
            to
        ],
        |row| {
            Ok(ToolOutcomeRow {
                source_key: row.get(0)?,
                harness: row.get(1)?,
                failed: row.get(2)?,
            })
        },
    )?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn count_measure(prefix: &str, name: &str, count: i64, basis: &str) -> Measure {
    Measure {
        id: format!("{prefix}.{name}"),
        name: name.to_string(),
        value: Some(MeasureValue::Int(count)),
        unit: "actions".to_string(),
        basis: basis.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insight_v1::contract::{InsightInput, MeasureValue};
    use crate::insight_v1::{evaluate, Scope};
    use crate::AnalyticsStore;
    use duckdb::params;

    fn store() -> AnalyticsStore {
        AnalyticsStore::open_in_memory("insight-i08").unwrap()
    }

    fn insert_tool(
        store: &AnalyticsStore,
        key: &str,
        harness: &str,
        session: &str,
        ts: Option<&str>,
        failed: Option<bool>,
    ) {
        store
            .connection()
            .execute(
                "INSERT INTO tool_facts (source_key, harness, native_session, ts, seq, tool_name, fingerprint, failed, structured_write, generation, source_id, category)
                 VALUES (?, ?, ?, CAST(? AS TIMESTAMP), 1, 'Bash', NULL, ?, FALSE, 1, 'src', 'eng_build_test')",
                params![key, harness, session, ts, failed],
            )
            .unwrap();
    }

    fn only(store: &AnalyticsStore, scope: &Scope) -> InsightResult {
        evaluate(store, scope)
            .into_iter()
            .find(|result| result.id == "I08")
            .unwrap_or_else(|| {
                if let Some(reason) = scope.refusal() {
                    refused(&INSIGHT, reason, "undefined")
                } else {
                    INSIGHT.compute(&InsightInput::new(store, scope))
                }
            })
    }

    fn int_at(result: &InsightResult, id: &str) -> Option<i64> {
        let measure = result
            .measures
            .iter()
            .find(|measure| measure.id == id)
            .unwrap();
        match measure.value {
            Some(MeasureValue::Int(value)) => Some(value),
            None => None,
            Some(other) => panic!("{id} was {other}"),
        }
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
    fn settled_actions_compute_exact_failure_rate_per_harness() {
        let store = store();
        insert_tool(&store, "k-1", "claude", "s1", None, Some(true));
        insert_tool(&store, "k-2", "claude", "s1", None, Some(false));
        insert_tool(&store, "k-3", "claude", "s1", None, Some(false));
        insert_tool(&store, "k-4", "claude", "s1", None, Some(false));
        insert_tool(&store, "k-5", "codex", "s2", None, Some(true));
        insert_tool(&store, "k-6", "codex", "s2", None, Some(true));
        let result = only(&store, &Scope::default());
        assert!(matches!(result.status, InsightStatus::Computed));
        assert_eq!(result.population.n, 6);
        assert_eq!(int_at(&result, "I08.claude.failed"), Some(1));
        assert_eq!(int_at(&result, "I08.claude.settled"), Some(4));
        assert_eq!(int_at(&result, "I08.claude.unsettled"), Some(0));
        let claude_rate = float_at(&result, "I08.claude.failure_rate").unwrap();
        assert!((claude_rate - 0.25).abs() < 1e-12);

        assert_eq!(int_at(&result, "I08.codex.failed"), Some(2));
        assert_eq!(int_at(&result, "I08.codex.settled"), Some(2));
        assert_eq!(int_at(&result, "I08.codex.unsettled"), Some(0));
        let codex_rate = float_at(&result, "I08.codex.failure_rate").unwrap();
        assert!((codex_rate - 1.0).abs() < 1e-12);
        assert_eq!(
            result.evidence,
            vec!["k-1", "k-2", "k-3", "k-4", "k-5"]
                .into_iter()
                .map(str::to_string)
                .collect::<Vec<_>>()
        );
        assert_eq!(result, only(&store, &Scope::default()));
    }

    #[test]
    fn empty_population_is_refused() {
        let store = store();
        insert_tool(
            &store,
            "dendro-internal:reader:v1:src:1",
            "claude",
            "s1",
            None,
            None,
        );
        let result = only(&store, &Scope::default());
        assert!(matches!(
            result.status,
            InsightStatus::Refused { reason } if reason == "empty population"
        ));
        assert!(result.measures.is_empty());
    }

    #[test]
    fn unsettled_rows_stay_out_of_denominator_and_zero_settled_has_no_zero_rate() {
        let store = store();
        insert_tool(&store, "k-1", "claude", "s1", None, Some(true));
        insert_tool(&store, "k-2", "claude", "s1", None, Some(false));
        insert_tool(&store, "k-3", "claude", "s1", None, None);
        insert_tool(&store, "k-4", "pi", "s2", None, None);
        let result = only(&store, &Scope::default());
        assert!(matches!(result.status, InsightStatus::Partial { .. }));
        assert_eq!(int_at(&result, "I08.claude.failed"), Some(1));
        assert_eq!(int_at(&result, "I08.claude.settled"), Some(2));
        assert_eq!(int_at(&result, "I08.claude.unsettled"), Some(1));
        let claude_rate = float_at(&result, "I08.claude.failure_rate").unwrap();
        assert!((claude_rate - 0.5).abs() < 1e-12);

        assert_eq!(int_at(&result, "I08.pi.failed"), Some(0));
        assert_eq!(int_at(&result, "I08.pi.settled"), Some(0));
        assert_eq!(int_at(&result, "I08.pi.unsettled"), Some(1));
        assert_eq!(float_at(&result, "I08.pi.failure_rate"), None);
    }
}
