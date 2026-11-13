//! I01 model.invocations. Distinct qualified call identities at call grain.
use super::super::contract::{
    finish, refused, Insight, InsightDomain, InsightInput, InsightPopulation, InsightResult,
    InsightStatus,
};
use super::support::count_measure;
use duckdb::params;
use std::collections::BTreeSet;

pub struct Invocations;

pub static INSIGHT: Invocations = Invocations;

const COVERAGE: &str = "qualified call grain only; usage-only and unqualified rows are counted separately and are not included in distinct_calls";

struct Row {
    source_key: String,
    message_id: Option<String>,
    harness: String,
    native_session: String,
    source_id: String,
}

// link-id-v1/vocabulary.json (blob ad0865ef39898bf5a7f8d9e0741f2456049a9b3d).
// Only harness identity is normalized; provider/model names never supply it.
fn canonical_harness(raw: &str) -> Option<&'static str> {
    if !raw.is_ascii() {
        return None;
    }
    [
        ("antigravity", "antigravity"),
        ("agy", "antigravity"),
        ("claude", "claude"),
        ("claude-code", "claude"),
        ("codex", "codex"),
        ("grok", "grok"),
        ("pi", "pi"),
    ]
    .into_iter()
    .find_map(|(alias, canonical)| raw.eq_ignore_ascii_case(alias).then_some(canonical))
}

fn known_qualifier(value: &str) -> bool {
    !value.trim().is_empty() && !value.eq_ignore_ascii_case("unknown")
}

impl Insight for Invocations {
    fn id(&self) -> &'static str {
        "I01"
    }
    fn name(&self) -> &'static str {
        "model.invocations"
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
        let mut calls = BTreeSet::new();
        let mut usage_only = 0i64;
        let mut unqualified = 0i64;
        for row in &rows {
            let Some(id) = row.message_id.as_deref().filter(|id| !id.trim().is_empty()) else {
                usage_only += 1;
                continue;
            };
            match canonical_harness(&row.harness) {
                Some(harness)
                    if known_qualifier(&row.native_session) && known_qualifier(&row.source_id) =>
                {
                    calls.insert((
                        harness,
                        row.native_session.as_str(),
                        row.source_id.as_str(),
                        id,
                    ));
                }
                _ => unqualified += 1,
            }
        }
        let distinct_calls = calls.len() as i64;
        drop(calls);
        let evidence: Vec<String> = rows.into_iter().map(|row| row.source_key).collect();
        let measures = vec![
            count_measure(
                "I01.distinct_calls",
                "distinct_calls",
                distinct_calls,
                "calls",
                "count of distinct (canonical_harness, native_session, source_id, message_id) tuples",
            ),
            count_measure(
                "I01.usage_only",
                "usage_only",
                usage_only,
                "rows",
                "usage rows with no call id; not included in distinct_calls",
            ),
            count_measure(
                "I01.unqualified",
                "unqualified",
                unqualified,
                "rows",
                "usage rows with a call id but missing/unknown harness, session, or source; counted per row, not merged or included in distinct_calls",
            ),
        ];
        let status = if distinct_calls == 0 {
            InsightStatus::Refused {
                reason: "not_observable".to_string(),
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
                n: evidence.len() as u64,
            },
            measures,
            coverage: COVERAGE.to_string(),
            evidence,
        })
    }
}

fn load(input: &InsightInput<'_>) -> anyhow::Result<Vec<Row>> {
    let mut stmt = input.connection().prepare(
        "SELECT source_key, message_id, harness, native_session, source_id
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
            Ok(Row {
                source_key: row.get(0)?,
                message_id: row.get(1)?,
                harness: row.get(2)?,
                native_session: row.get(3)?,
                source_id: row.get(4)?,
            })
        },
    )?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insight_v1::contract::MeasureValue;
    use crate::insight_v1::Scope;
    use crate::AnalyticsStore;

    fn insert(
        store: &AnalyticsStore,
        source_key: &str,
        harness: &str,
        native_session: &str,
        message_id: Option<&str>,
        seq: i64,
        source_id: &str,
    ) {
        store
            .connection()
            .execute(
                "INSERT INTO usage_facts (
                    source_key, harness, native_session, message_id, seq,
                    input_uncached, output_tokens, cache_read, cache_write, reasoning,
                    generation, source_id
                 ) VALUES (?, ?, ?, ?, ?, 10, 5, 0, 0, 0, 1, ?)",
                params![
                    source_key,
                    harness,
                    native_session,
                    message_id,
                    seq,
                    source_id
                ],
            )
            .unwrap();
    }

    fn measure_int(result: &InsightResult, id: &str) -> Option<i64> {
        result.measures.iter().find_map(|measure| {
            if measure.id != id {
                return None;
            }
            match measure.value {
                Some(MeasureValue::Int(value)) => Some(value),
                _ => None,
            }
        })
    }

    #[test]
    fn harness_vocabulary_is_closed_ascii_case_insensitive_and_never_trimmed() {
        let store = AnalyticsStore::open_in_memory("insight-i01-vocabulary").unwrap();
        for (i, harness) in [
            "antigravity",
            "AGY",
            "claude",
            "CLAUDE-CODE",
            "codex",
            "GROK",
            "pi",
            " claude",
            "claude ",
            "cl\u{e1}ude",
            "gemini",
            "anthropic",
            "openai",
            "xai",
            "ai-agent",
        ]
        .iter()
        .enumerate()
        {
            insert(
                &store,
                &format!("u-{i}"),
                harness,
                "s1",
                Some("m1"),
                i as i64,
                "src",
            );
        }
        let result = INSIGHT.compute(&InsightInput::new(&store, &Scope::default()));
        assert_eq!(measure_int(&result, "I01.distinct_calls"), Some(5));
        assert_eq!(measure_int(&result, "I01.unqualified"), Some(8));
    }

    #[test]
    fn source_namespaces_are_distinct_but_replays_dedupe() {
        let store = AnalyticsStore::open_in_memory("insight-i01-sources").unwrap();
        insert(&store, "u-1", "claude", "s1", Some("m1"), 1, "src-a");
        insert(&store, "u-2", "claude", "s1", Some("m1"), 2, "src-b");
        insert(&store, "u-3", "claude", "s1", Some("m1"), 3, "src-a");
        let result = INSIGHT.compute(&InsightInput::new(&store, &Scope::default()));
        assert_eq!(measure_int(&result, "I01.distinct_calls"), Some(2));
        assert_eq!(measure_int(&result, "I01.unqualified"), Some(0));
    }

    #[test]
    fn incomplete_qualifiers_are_counted_per_row_and_refuse_without_calls() {
        let store = AnalyticsStore::open_in_memory("insight-i01-missing").unwrap();
        for (i, (harness, session, source)) in [
            ("", "s1", "src"),
            ("unknown", "s1", "src"),
            ("google", "s1", "src"),
            ("claude", "\u{3000}", "src"),
            ("claude", "s1", ""),
            ("claude", "s1", "unknown"),
            ("claude", "s1", "  "),
        ]
        .iter()
        .enumerate()
        {
            insert(
                &store,
                &format!("u-{i}"),
                harness,
                session,
                Some("m1"),
                i as i64,
                source,
            );
        }
        let result = INSIGHT.compute(&InsightInput::new(&store, &Scope::default()));
        assert_eq!(measure_int(&result, "I01.distinct_calls"), Some(0));
        assert_eq!(measure_int(&result, "I01.unqualified"), Some(7));
        assert_eq!(measure_int(&result, "I01.usage_only"), Some(0));
        assert!(
            matches!(result.status, InsightStatus::Refused { ref reason } if reason == "not_observable")
        );
    }

    #[test]
    fn absent_message_ids_remain_usage_only_even_with_missing_qualifiers() {
        let store = AnalyticsStore::open_in_memory("insight-i01-usage-only").unwrap();
        for (i, id) in [None, Some(""), Some(" \u{3000}")].into_iter().enumerate() {
            insert(&store, &format!("u-{i}"), "", "", id, i as i64, "");
        }
        let result = INSIGHT.compute(&InsightInput::new(&store, &Scope::default()));
        assert_eq!(measure_int(&result, "I01.distinct_calls"), Some(0));
        assert_eq!(measure_int(&result, "I01.unqualified"), Some(0));
        assert_eq!(measure_int(&result, "I01.usage_only"), Some(3));
    }

    #[test]
    fn opaque_identity_fields_do_not_collapse_at_delimiters_or_whitespace() {
        let store = AnalyticsStore::open_in_memory("insight-i01-opaque").unwrap();
        for (i, (session, message)) in [("a:b", "c"), ("a", "b:c"), ("a", " b:c ")]
            .iter()
            .enumerate()
        {
            insert(
                &store,
                &format!("u-{i}"),
                "claude",
                session,
                Some(message),
                i as i64,
                "src",
            );
        }
        let result = INSIGHT.compute(&InsightInput::new(&store, &Scope::default()));
        assert_eq!(measure_int(&result, "I01.distinct_calls"), Some(3));
    }

    #[test]
    fn fixture_a_same_message_id_in_two_different_sessions_counts_two() {
        let store = AnalyticsStore::open_in_memory("insight-i01-a").unwrap();
        insert(&store, "u-1", "claude", "s1", Some("m-shared"), 1, "src");
        insert(&store, "u-2", "claude", "s2", Some("m-shared"), 2, "src");
        let scope = Scope::default();
        let input = InsightInput::new(&store, &scope);
        let result = INSIGHT.compute(&input);
        assert!(matches!(result.status, InsightStatus::Computed));
        assert_eq!(measure_int(&result, "I01.distinct_calls"), Some(2));
        assert_eq!(measure_int(&result, "I01.usage_only"), Some(0));
        assert_eq!(measure_int(&result, "I01.unqualified"), Some(0));
    }

    #[test]
    fn fixture_b_same_message_id_across_claude_and_codex_counts_two() {
        let store = AnalyticsStore::open_in_memory("insight-i01-b").unwrap();
        insert(&store, "u-1", "claude", "s1", Some("m-shared"), 1, "src");
        insert(&store, "u-2", "codex", "s1", Some("m-shared"), 2, "src");
        let scope = Scope::default();
        let input = InsightInput::new(&store, &scope);
        let result = INSIGHT.compute(&input);
        assert!(matches!(result.status, InsightStatus::Computed));
        assert_eq!(measure_int(&result, "I01.distinct_calls"), Some(2));
        assert_eq!(measure_int(&result, "I01.usage_only"), Some(0));
        assert_eq!(measure_int(&result, "I01.unqualified"), Some(0));
    }

    #[test]
    fn fixture_c_true_duplicate_same_session_harness_and_message_id_counts_one() {
        let store = AnalyticsStore::open_in_memory("insight-i01-c").unwrap();
        insert(&store, "u-1", "claude", "s1", Some("m-dup"), 1, "src");
        insert(&store, "u-2", "claude-code", "s1", Some("m-dup"), 2, "src");
        insert(&store, "u-3", "claude", "s1", Some("m-dup"), 3, "src");
        let scope = Scope::default();
        let input = InsightInput::new(&store, &scope);
        let result = INSIGHT.compute(&input);
        assert!(matches!(result.status, InsightStatus::Computed));
        assert_eq!(measure_int(&result, "I01.distinct_calls"), Some(1));
        assert_eq!(measure_int(&result, "I01.usage_only"), Some(0));
        assert_eq!(measure_int(&result, "I01.unqualified"), Some(0));
    }

    #[test]
    fn fixture_d_missing_session_id_counted_as_unqualified_not_merged() {
        let store = AnalyticsStore::open_in_memory("insight-i01-d").unwrap();
        insert(&store, "u-1", "claude", "", Some("m-shared"), 1, "src");
        insert(&store, "u-2", "claude", "", Some("m-shared"), 2, "src");
        insert(
            &store,
            "u-3",
            "claude",
            "unknown",
            Some("m-shared"),
            3,
            "src",
        );
        insert(&store, "u-4", "claude", "s1", Some("m-shared"), 4, "src");
        let scope = Scope::default();
        let input = InsightInput::new(&store, &scope);
        let result = INSIGHT.compute(&input);
        assert!(matches!(result.status, InsightStatus::Computed));
        assert_eq!(measure_int(&result, "I01.distinct_calls"), Some(1));
        assert_eq!(measure_int(&result, "I01.usage_only"), Some(0));
        assert_eq!(measure_int(&result, "I01.unqualified"), Some(3));
    }

    #[test]
    fn fixture_e_g0a_s2_two_session_claude_capture_keeps_distinct_sessions_distinct() {
        // Extracted assistant identities from G0a claude-2.1.322/S2/
        // session-log-sanitized.jsonl, blob 327054ed9b1d590b011b0e41c75602c044f1807a.
        // Lines 25, 26, 42, 71, 72, 88, 89: two sessions, four calls,
        // three repeated assistant rows. Cost-state rows are not invocations.
        let store = AnalyticsStore::open_in_memory("insight-i01-e").unwrap();
        insert(
            &store,
            "s2:25",
            "claude",
            "SESSION_82d7a11553f766a4",
            Some("MSG_f5e821758b6583fb"),
            1,
            "g0a-s2",
        );
        insert(
            &store,
            "s2:26",
            "claude",
            "SESSION_82d7a11553f766a4",
            Some("MSG_f5e821758b6583fb"),
            2,
            "g0a-s2",
        );
        insert(
            &store,
            "s2:42",
            "claude",
            "SESSION_82d7a11553f766a4",
            Some("MSG_93ea8b84a756ce1f"),
            3,
            "g0a-s2",
        );
        insert(
            &store,
            "s2:71",
            "claude",
            "SESSION_fa2814579786c60e",
            Some("MSG_28861758043ece6d"),
            5,
            "g0a-s2",
        );
        insert(
            &store,
            "s2:72",
            "claude",
            "SESSION_fa2814579786c60e",
            Some("MSG_28861758043ece6d"),
            6,
            "g0a-s2",
        );
        insert(
            &store,
            "s2:88",
            "claude",
            "SESSION_fa2814579786c60e",
            Some("MSG_9d1d7224b0db2e4a"),
            7,
            "g0a-s2",
        );
        insert(
            &store,
            "s2:89",
            "claude",
            "SESSION_fa2814579786c60e",
            Some("MSG_9d1d7224b0db2e4a"),
            8,
            "g0a-s2",
        );
        let all = INSIGHT.compute(&InsightInput::new(&store, &Scope::default()));
        assert!(matches!(all.status, InsightStatus::Computed));
        assert_eq!(measure_int(&all, "I01.distinct_calls"), Some(4));
        assert_eq!(measure_int(&all, "I01.usage_only"), Some(0));
        assert_eq!(measure_int(&all, "I01.unqualified"), Some(0));

        let session_b = INSIGHT.compute(&InsightInput::new(
            &store,
            &Scope {
                session: Some("SESSION_82d7a11553f766a4".to_string()),
                ..Scope::default()
            },
        ));
        assert_eq!(measure_int(&session_b, "I01.distinct_calls"), Some(2));
        assert_eq!(measure_int(&session_b, "I01.usage_only"), Some(0));

        let session_a = INSIGHT.compute(&InsightInput::new(
            &store,
            &Scope {
                session: Some("SESSION_fa2814579786c60e".to_string()),
                ..Scope::default()
            },
        ));
        assert_eq!(measure_int(&session_a, "I01.distinct_calls"), Some(2));
        assert_eq!(measure_int(&session_a, "I01.usage_only"), Some(0));
    }
}
