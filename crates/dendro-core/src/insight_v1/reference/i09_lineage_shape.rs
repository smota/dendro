//! I09 work.lineage_shape. Session edge relations, statuses, roots, depth, diamonds, cycles, and unresolved parents.
use super::super::contract::{
    finish, label_id, refused, Insight, InsightDomain, InsightInput, InsightPopulation,
    InsightResult, InsightStatus, Measure, MeasureValue,
};
use crate::session_lineage::{children_of, lineage_roots, SessionLink};
use duckdb::params;
use std::collections::{BTreeMap, BTreeSet};

pub struct LineageShape;

pub static INSIGHT: LineageShape = LineageShape;

const COVERAGE: &str =
    "explicit session_edges only; unresolved parents are counted and never invented as roots; cycles stop the walk";

#[derive(Debug, Clone)]
struct EdgeRow {
    source_key: String,
    parent_session: String,
    child_session: String,
    relation: String,
    status: String,
}

impl Insight for LineageShape {
    fn id(&self) -> &'static str {
        "I09"
    }
    fn name(&self) -> &'static str {
        "work.lineage_shape"
    }
    fn version(&self) -> &'static str {
        "1.0.0"
    }
    fn domain(&self) -> InsightDomain {
        InsightDomain::Coordination
    }
    fn compute(&self, input: &InsightInput<'_>) -> InsightResult {
        let rows = match load_edges(input) {
            Ok(rows) => rows,
            Err(_) => return refused(self, "store read failed", "session_edges"),
        };
        if rows.is_empty() {
            return refused(self, "empty population", "session_edges");
        }
        let mut by_relation: BTreeMap<String, i64> = BTreeMap::new();
        let mut by_status: BTreeMap<String, i64> = BTreeMap::new();
        let mut parents_by_child: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        let mut sessions: BTreeSet<&str> = BTreeSet::new();
        let mut unresolved_edges = 0i64;
        let mut links = Vec::with_capacity(rows.len());

        for row in &rows {
            *by_relation.entry(label_id(&row.relation)).or_default() += 1;
            *by_status.entry(label_id(&row.status)).or_default() += 1;
            if row.parent_session.is_empty() {
                unresolved_edges += 1;
            } else {
                sessions.insert(row.parent_session.as_str());
            }
            if !row.child_session.is_empty() {
                sessions.insert(row.child_session.as_str());
                let entry = parents_by_child
                    .entry(row.child_session.as_str())
                    .or_default();
                if !row.parent_session.is_empty() {
                    entry.insert(row.parent_session.as_str());
                }
            }
            links.push(SessionLink {
                parent_session: row.parent_session.clone(),
                child_session: row.child_session.clone(),
                relation: row.relation.clone(),
                status: row.status.clone(),
            });
        }

        let multi_parent_children = parents_by_child
            .values()
            .filter(|parents| parents.len() > 1)
            .count() as i64;

        let mut roots: BTreeSet<String> = BTreeSet::new();
        let mut cycle_sessions = 0i64;
        let mut any_cycle = false;
        let mut any_unresolved = unresolved_edges > 0;

        for session in &sessions {
            let walk = lineage_roots(&links, session);
            for root in walk.roots {
                roots.insert(root);
            }
            if walk.cycle {
                any_cycle = true;
                cycle_sessions += 1;
            }
            if walk.unresolved {
                any_unresolved = true;
            }
        }

        let max_depth = if any_cycle || any_unresolved || roots.is_empty() {
            None
        } else {
            let mut deepest = 0i64;
            for root in &roots {
                let mut stack = BTreeSet::new();
                stack.insert(root.as_str());
                let d = walk_depth(&links, root.as_str(), &mut stack);
                if d > deepest {
                    deepest = d;
                }
            }
            Some(MeasureValue::Int(deepest))
        };

        let mut measures = Vec::new();
        for (relation, count) in &by_relation {
            measures.push(Measure {
                id: format!("I09.relation.{relation}"),
                name: format!("relation.{relation}"),
                value: Some(MeasureValue::Int(*count)),
                unit: "edges".to_string(),
                basis: format!("count of session_edges with relation={relation}"),
            });
        }
        for (status_label, count) in &by_status {
            measures.push(Measure {
                id: format!("I09.status.{status_label}"),
                name: format!("status.{status_label}"),
                value: Some(MeasureValue::Int(*count)),
                unit: "edges".to_string(),
                basis: format!("count of session_edges with status={status_label}"),
            });
        }
        measures.push(Measure {
            id: "I09.multi_parent_children".to_string(),
            name: "multi_parent_children".to_string(),
            value: Some(MeasureValue::Int(multi_parent_children)),
            unit: "sessions".to_string(),
            basis: "children with more than one distinct resolved parent_session (diamonds)"
                .to_string(),
        });
        measures.push(Measure {
            id: "I09.roots".to_string(),
            name: "roots".to_string(),
            value: Some(MeasureValue::Int(roots.len() as i64)),
            unit: "sessions".to_string(),
            basis: "distinct root sessions from session_lineage::lineage_roots; unresolved parents never invent roots".to_string(),
        });
        measures.push(Measure {
            id: "I09.max_depth".to_string(),
            name: "max_depth".to_string(),
            value: max_depth,
            unit: "edges".to_string(),
            basis: "longest root-to-descendant path via session_lineage::children_of; unknown when cycles or unresolved parents exist".to_string(),
        });
        measures.push(Measure {
            id: "I09.cycles".to_string(),
            name: "cycles".to_string(),
            value: Some(MeasureValue::Int(cycle_sessions)),
            unit: "sessions".to_string(),
            basis: "sessions whose parent walk encountered a cycle".to_string(),
        });
        measures.push(Measure {
            id: "I09.unresolved_parents".to_string(),
            name: "unresolved_parents".to_string(),
            value: Some(MeasureValue::Int(unresolved_edges)),
            unit: "edges".to_string(),
            basis: "edges with empty parent_session; counted and never invented as roots"
                .to_string(),
        });

        let status = match (any_cycle, any_unresolved) {
            (true, true) => InsightStatus::Partial {
                reason: "cycle detected and unresolved parent session".to_string(),
            },
            (true, false) => InsightStatus::Partial {
                reason: "cycle detected in session lineage".to_string(),
            },
            (false, true) => InsightStatus::Partial {
                reason: "unresolved parent session".to_string(),
            },
            (false, false) => InsightStatus::Computed,
        };

        finish(InsightResult {
            id: self.id(),
            name: self.name(),
            version: self.version(),
            domain: self.domain(),
            status,
            population: InsightPopulation {
                what: "session_edges".to_string(),
                n: rows.len() as u64,
            },
            measures,
            coverage: COVERAGE.to_string(),
            evidence: rows.into_iter().map(|row| row.source_key).collect(),
        })
    }
}

fn walk_depth<'a>(links: &'a [SessionLink], node: &'a str, stack: &mut BTreeSet<&'a str>) -> i64 {
    let mut best = 0i64;
    for child in children_of(links, node) {
        if stack.insert(child) {
            let candidate = 1 + walk_depth(links, child, stack);
            if candidate > best {
                best = candidate;
            }
            stack.remove(child);
        }
    }
    best
}

fn load_edges(input: &InsightInput<'_>) -> anyhow::Result<Vec<EdgeRow>> {
    let mut stmt = input.connection().prepare(
        "SELECT source_key, parent_session, child_session, relation, status
         FROM session_edges
         WHERE (? IS NULL OR harness = ?)
           AND (? IS NULL OR parent_session = ? OR child_session = ?)
           AND (? IS NULL OR observed_at >= CAST(? AS TIMESTAMP))
           AND (? IS NULL OR observed_at < CAST(? AS TIMESTAMP))
         ORDER BY source_key",
    )?;
    let scope = input.scope();
    let harness = scope.harness.as_deref();
    let session = scope.session.as_deref();
    let from = scope.from.as_deref();
    let to = scope.to.as_deref();
    let rows = stmt.query_map(
        params![harness, harness, session, session, session, from, from, to, to],
        |row| {
            Ok(EdgeRow {
                source_key: row.get(0)?,
                parent_session: row.get(1)?,
                child_session: row.get(2)?,
                relation: row.get(3)?,
                status: row.get(4)?,
            })
        },
    )?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insight_v1::contract::{InsightInput, MeasureValue};
    use crate::insight_v1::{evaluate, Scope};
    use crate::AnalyticsStore;
    use duckdb::params;

    fn store() -> AnalyticsStore {
        AnalyticsStore::open_in_memory("insight-i09").unwrap()
    }

    fn insert_edge(
        store: &AnalyticsStore,
        key: &str,
        harness: &str,
        parent: &str,
        child: &str,
        relation: &str,
        status: &str,
    ) {
        store
            .connection()
            .execute(
                "INSERT INTO session_edges (source_key, source_id, harness, parent_session, child_session, relation, status, observed_at, evidence_kind, generation)
                 VALUES (?, 'src', ?, ?, ?, ?, ?, NULL, 'explicit', 1)",
                params![key, harness, parent, child, relation, status],
            )
            .unwrap();
    }

    fn only(store: &AnalyticsStore, scope: &Scope) -> InsightResult {
        evaluate(store, scope)
            .into_iter()
            .find(|result| result.id == "I09")
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

    #[test]
    fn dag_with_diamond_computes_exact_roots_depth_and_relation_counts() {
        let store = store();
        // Root r1 -> a -> d (depth 2), Root r2 -> d (diamond on d), a -> b (resume)
        insert_edge(&store, "e1", "claude", "r1", "a", "spawn", "started");
        insert_edge(&store, "e2", "claude", "a", "d", "spawn", "finished");
        insert_edge(&store, "e3", "claude", "r2", "d", "delegate", "finished");
        insert_edge(&store, "e4", "claude", "a", "b", "resume", "started");

        let result = only(&store, &Scope::default());
        assert!(matches!(result.status, InsightStatus::Computed));
        assert_eq!(result.population.n, 4);
        assert_eq!(int_at(&result, "I09.relation.spawn"), Some(2));
        assert_eq!(int_at(&result, "I09.relation.delegate"), Some(1));
        assert_eq!(int_at(&result, "I09.relation.resume"), Some(1));
        assert_eq!(int_at(&result, "I09.status.started"), Some(2));
        assert_eq!(int_at(&result, "I09.status.finished"), Some(2));
        assert_eq!(int_at(&result, "I09.multi_parent_children"), Some(1));
        assert_eq!(int_at(&result, "I09.roots"), Some(2));
        assert_eq!(int_at(&result, "I09.max_depth"), Some(2));
        assert_eq!(int_at(&result, "I09.cycles"), Some(0));
        assert_eq!(int_at(&result, "I09.unresolved_parents"), Some(0));
        assert_eq!(
            result.evidence,
            vec!["e1", "e2", "e3", "e4"]
                .into_iter()
                .map(str::to_string)
                .collect::<Vec<_>>()
        );
        assert_eq!(result, only(&store, &Scope::default()));
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
    fn cycle_and_unresolved_parent_flag_partial_without_inventing_roots_or_zero_depth() {
        let store = store();
        insert_edge(&store, "e1", "claude", "a", "b", "spawn", "started");
        insert_edge(&store, "e2", "claude", "b", "a", "spawn", "started");
        insert_edge(&store, "e3", "claude", "", "orphan", "spawn", "unknown");

        let result = only(&store, &Scope::default());
        assert!(matches!(
            result.status,
            InsightStatus::Partial { ref reason } if reason.contains("cycle") && reason.contains("unresolved")
        ));
        assert_eq!(int_at(&result, "I09.roots"), Some(0));
        assert_eq!(int_at(&result, "I09.cycles"), Some(2));
        assert_eq!(int_at(&result, "I09.unresolved_parents"), Some(1));
        assert_eq!(int_at(&result, "I09.max_depth"), None);
    }
}
