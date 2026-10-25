//! I18 coverage.source. Reader markers and warning rows, per source.
use super::super::contract::{
    finish, label_id, refused, Insight, InsightDomain, InsightInput, InsightPopulation,
    InsightResult, InsightStatus, Measure, MeasureValue, WarningRow,
};
use crate::SourceCoverage;
use std::collections::{BTreeMap, BTreeSet};

pub struct CoverageSource;

pub static INSIGHT: CoverageSource = CoverageSource;

impl Insight for CoverageSource {
    fn id(&self) -> &'static str {
        "I18"
    }
    fn name(&self) -> &'static str {
        "coverage.source"
    }
    fn version(&self) -> &'static str {
        "1.0.0"
    }
    fn domain(&self) -> InsightDomain {
        InsightDomain::Environment
    }
    fn compute(&self, input: &InsightInput<'_>) -> InsightResult {
        if input.scope().from.is_some() || input.scope().to.is_some() {
            return refused(self, "undefined input", "source_files");
        }
        let sources = match input.store().load_source_coverage() {
            Ok(sources) => sources,
            Err(_) => return refused(self, "store read failed", "source_files"),
        };
        let warnings = match input.warning_rows() {
            Ok(warnings) => warnings,
            Err(_) => return refused(self, "store read failed", "warnings"),
        };
        let selected = select_sources(&sources, input.scope());
        let mut warnings_by_path: BTreeMap<&str, Vec<&WarningRow>> = BTreeMap::new();
        for warning in &warnings {
            warnings_by_path
                .entry(warning.path.as_str())
                .or_default()
                .push(warning);
        }
        let mut keys: BTreeSet<String> = selected
            .iter()
            .map(|source| source.source_id.clone())
            .collect();
        if input.scope().harness.is_none() && input.scope().session.is_none() {
            for warning in &warnings {
                keys.insert(warning.path.clone());
            }
        }
        if keys.is_empty() {
            return refused(self, "empty population", "source_files");
        }
        let by_id: BTreeMap<&str, &SourceCoverage> = selected
            .iter()
            .map(|source| (source.source_id.as_str(), *source))
            .collect();
        let mut partial = false;
        let mut measures = Vec::new();
        for key in &keys {
            let source = by_id.get(key.as_str()).copied();
            let rows = warnings_by_path
                .get(key.as_str())
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            if source.is_some_and(|source| !source.complete)
                || rows.iter().any(|row| is_partial_reason(&row.reason))
            {
                partial = true;
            }
            measures.push(count_measure(
                key,
                "read",
                "sources",
                if source.is_some() {
                    Some(MeasureValue::Int(1))
                } else {
                    None
                },
                if source.is_some() {
                    "kind=source; recorded in source_files"
                } else {
                    "kind=source; not recorded in source_files"
                },
            ));
            let projected =
                source.and_then(|source| source.complete.then_some(MeasureValue::Int(1)));
            if source.is_some_and(|source| !source.complete) {
                partial = true;
            }
            measures.push(count_measure(
                key,
                "projected",
                "sources",
                projected,
                if projected.is_some() {
                    "kind=source; reader complete"
                } else {
                    "kind=source; reader completion unknown"
                },
            ));
            let mut unprojected: BTreeMap<&str, i64> = BTreeMap::new();
            let mut refused_rows = Vec::new();
            for row in rows {
                if is_refused(&row.reason) {
                    refused_rows.push(row.reason.as_str());
                } else if is_status_only(&row.reason) {
                    // Status flag only. The reason counts live on their own rows.
                } else {
                    *unprojected.entry(row.reason.as_str()).or_default() += 1;
                    partial = true;
                }
            }
            for (reason, count) in unprojected {
                measures.push(Measure {
                    id: format!("I18.{}.unprojected.{}", label_id(key), label_id(reason)),
                    name: "unprojected".to_string(),
                    value: Some(MeasureValue::Int(count)),
                    unit: "warnings".to_string(),
                    basis: format!("kind=warning; reason={reason}"),
                });
            }
            if !refused_rows.is_empty() {
                refused_rows.sort_unstable();
                measures.push(Measure {
                    id: format!("I18.{}.refused", label_id(key)),
                    name: "refused".to_string(),
                    value: Some(MeasureValue::Int(refused_rows.len() as i64)),
                    unit: "warnings".to_string(),
                    basis: format!("kind=warning; reason={}", refused_rows.join("; ")),
                });
            }
        }
        let status = if partial {
            InsightStatus::Partial {
                reason: "source coverage is partial".to_string(),
            }
        } else {
            InsightStatus::Computed
        };
        let coverage = if partial {
            "partial; reader markers and warning rows"
        } else {
            "complete; reader markers and warning rows"
        };
        finish(InsightResult {
            id: self.id(),
            name: self.name(),
            version: self.version(),
            domain: self.domain(),
            status,
            population: InsightPopulation {
                what: "source_files".to_string(),
                n: keys.len() as u64,
            },
            measures,
            coverage: coverage.to_string(),
            evidence: keys.into_iter().collect(),
        })
    }
}

fn select_sources<'a>(
    sources: &'a [SourceCoverage],
    scope: &super::super::contract::Scope,
) -> Vec<&'a SourceCoverage> {
    sources
        .iter()
        .filter(|source| {
            scope
                .harness
                .as_ref()
                .is_none_or(|harness| source.harness == *harness)
                && scope.session.as_ref().is_none_or(|session| {
                    source.native_session.as_deref() == Some(session.as_str())
                })
        })
        .collect()
}

fn is_refused(reason: &str) -> bool {
    reason.starts_with("coverage=refused")
}

fn is_partial_reason(reason: &str) -> bool {
    reason == "coverage-partial"
        || reason.starts_with("coverage-partial ")
        || reason.starts_with("coverage=partial")
}

fn is_status_only(reason: &str) -> bool {
    is_partial_reason(reason) || reason.starts_with("coverage=complete")
}

fn count_measure(
    key: &str,
    name: &str,
    unit: &str,
    value: Option<MeasureValue>,
    basis: &str,
) -> Measure {
    Measure {
        id: format!("I18.{}.{}", label_id(key), name),
        name: name.to_string(),
        value,
        unit: unit.to_string(),
        basis: basis.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facts::{READER_COMPLETE, READER_NAMESPACE, READER_VERSION};
    use crate::insight_v1::{evaluate, InsightStatus, Scope};
    use crate::AnalyticsStore;
    use duckdb::params;

    const PREFIX: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn store() -> AnalyticsStore {
        AnalyticsStore::open_in_memory("insight-i18").unwrap()
    }

    fn source(store: &AnalyticsStore, id: &str, harness: &str, session: &str, complete: bool) {
        store
            .connection()
            .execute(
                "INSERT INTO source_files VALUES (?, ?, 'sealed-path', 1, 0, ?, ?)",
                params![id, harness, PREFIX, session],
            )
            .unwrap();
        if complete {
            let marker = format!("{READER_NAMESPACE}{READER_VERSION}:{id}:1");
            store
                .connection()
                .execute(
                    "INSERT INTO tool_facts (source_key, harness, native_session, ts, seq, tool_name, fingerprint, failed, structured_write, generation, source_id, category)
                     VALUES (?, ?, ?, NULL, 0, ?, ?, NULL, FALSE, 1, ?, 'other')",
                    params![marker, harness, session, READER_COMPLETE, PREFIX, id],
                )
                .unwrap();
        }
    }

    fn warn(store: &AnalyticsStore, path: &str, reason: &str) {
        store
            .connection()
            .execute(
                "INSERT INTO warnings (path, line_no, reason) VALUES (?, NULL, ?)",
                params![path, reason],
            )
            .unwrap();
    }

    fn only(store: &AnalyticsStore, scope: &Scope) -> InsightResult {
        evaluate(store, scope)
            .into_iter()
            .find(|result| result.id == "I18")
            .unwrap()
    }

    fn value(result: &InsightResult, id: &str) -> Option<MeasureValue> {
        result
            .measures
            .iter()
            .find(|measure| measure.id == id)
            .unwrap()
            .value
    }

    #[test]
    fn partial_source_counts_unprojected_and_does_not_invent_projected_zero() {
        let store = store();
        source(&store, "src-a", "claude", "s", true);
        source(&store, "src-b", "claude", "s", false);
        warn(&store, "src-b", "unsupported_schema");
        warn(&store, "src-b", "coverage-partial");
        let result = only(&store, &Scope::default());
        assert!(matches!(
            result.status,
            InsightStatus::Partial { ref reason } if reason == "source coverage is partial"
        ));
        assert_eq!(result.population.n, 2);
        assert_eq!(value(&result, "I18.src-a.read"), Some(MeasureValue::Int(1)));
        assert_eq!(
            value(&result, "I18.src-a.projected"),
            Some(MeasureValue::Int(1))
        );
        assert_eq!(value(&result, "I18.src-b.read"), Some(MeasureValue::Int(1)));
        assert_eq!(value(&result, "I18.src-b.projected"), None);
        assert_eq!(
            value(&result, "I18.src-b.unprojected.unsupported_schema"),
            Some(MeasureValue::Int(1))
        );
        assert!(result
            .measures
            .iter()
            .all(|measure| measure.value != Some(MeasureValue::Int(0))));
        assert_eq!(
            result.evidence,
            vec!["src-a".to_string(), "src-b".to_string()]
        );
        assert!(result.coverage.starts_with("partial"));
        assert_eq!(result, only(&store, &Scope::default()));
    }

    #[test]
    fn refused_warning_is_counted_and_read_is_unknown() {
        let store = store();
        warn(
            &store,
            "src-c",
            "coverage=refused reason=native_file_unreadable",
        );
        let result = only(&store, &Scope::default());
        assert!(matches!(result.status, InsightStatus::Computed));
        assert_eq!(value(&result, "I18.src-c.read"), None);
        assert_eq!(
            value(&result, "I18.src-c.refused"),
            Some(MeasureValue::Int(1))
        );
        assert!(result
            .measures
            .iter()
            .all(|measure| measure.value != Some(MeasureValue::Int(0))));
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
    fn time_window_is_undefined_for_untimed_coverage() {
        let store = store();
        source(&store, "src-a", "claude", "s", true);
        let result = only(
            &store,
            &Scope {
                from: Some("2026-10-15 00:00:00".into()),
                ..Scope::default()
            },
        );
        assert!(matches!(
            result.status,
            InsightStatus::Refused { reason } if reason == "undefined input"
        ));
    }

    #[test]
    fn harness_scope_drops_other_sources() {
        let store = store();
        source(&store, "src-a", "claude", "s", true);
        source(&store, "src-b", "pi", "s", true);
        let result = only(
            &store,
            &Scope {
                harness: Some("claude".into()),
                ..Scope::default()
            },
        );
        assert!(matches!(result.status, InsightStatus::Computed));
        assert_eq!(result.population.n, 1);
        assert_eq!(result.evidence, vec!["src-a".to_string()]);
    }
}
