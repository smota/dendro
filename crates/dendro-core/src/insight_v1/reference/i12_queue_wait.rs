//! I12 coord.queue_wait. Ready-to-start elapsed time and its wall union.
use super::super::contract::{
    finish, refused, Insight, InsightDomain, InsightInput, InsightPopulation, InsightResult,
    InsightStatus,
};
use super::support::{count_measure, float_measure};
use duckdb::params;

pub struct QueueWait;

pub static INSIGHT: QueueWait = QueueWait;

const COVERAGE: &str =
    "ready-to-start uses paired pending and in-progress transitions on one clock; silence is not a wait";

struct Row {
    key: String,
    item: String,
    ts: f64,
    transition: i64,
    state: String,
}

impl Insight for QueueWait {
    fn id(&self) -> &'static str {
        "I12"
    }
    fn name(&self) -> &'static str {
        "coord.queue_wait"
    }
    fn version(&self) -> &'static str {
        "1.0.0"
    }
    fn domain(&self) -> InsightDomain {
        InsightDomain::Coordination
    }
    fn compute(&self, input: &InsightInput<'_>) -> InsightResult {
        let rows = match load(input) {
            Ok(rows) => rows,
            Err(_) => return refused(self, "store read failed", "queue_facts"),
        };
        if rows.is_empty() {
            return refused(self, "empty population", "queue_facts");
        }
        let (from, to) = match bounds(input) {
            Ok(bounds) => bounds,
            Err(_) => return refused(self, "undefined input", "queue_facts"),
        };
        let mut by_item: std::collections::BTreeMap<String, Vec<&Row>> =
            std::collections::BTreeMap::new();
        for row in &rows {
            by_item.entry(row.item.clone()).or_default().push(row);
        }
        let mut waits = Vec::new();
        let mut segments = Vec::new();
        let mut missing = 0i64;
        for group in by_item.values_mut() {
            group.sort_by(|left, right| {
                left.ts
                    .total_cmp(&right.ts)
                    .then(left.transition.cmp(&right.transition))
            });
            let mut index = 0;
            while index < group.len() {
                if group[index].state != "pending" {
                    index += 1;
                    continue;
                }
                let start = group
                    .iter()
                    .skip(index + 1)
                    .find(|row| row.state == "in-progress");
                let Some(start) = start else {
                    missing += 1;
                    break;
                };
                if start.ts < group[index].ts {
                    missing += 1;
                } else if let Some(segment) = clip(group[index].ts, start.ts, from, to) {
                    waits.push(segment.1 - segment.0);
                    segments.push(segment);
                }
                index = group
                    .iter()
                    .position(|row| row.key == start.key)
                    .unwrap_or(index)
                    + 1;
            }
        }
        let sum = if waits.is_empty() {
            None
        } else {
            Some(waits.iter().sum())
        };
        let median = median(&waits);
        let wall = union_len(&segments);
        let status = if missing > 0 || waits.is_empty() {
            InsightStatus::Partial {
                reason: "missing start".to_string(),
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
                what: "queue_facts".to_string(),
                n: rows.len() as u64,
            },
            measures: vec![
                float_measure(
                    "I12.wait_sum_seconds",
                    "wait_sum_seconds",
                    sum,
                    "seconds",
                    "sum of ready-to-start intervals; unknown when every interval is refused",
                ),
                float_measure(
                    "I12.wait_median_seconds",
                    "wait_median_seconds",
                    median,
                    "seconds",
                    "median of the same ready-to-start intervals",
                ),
                float_measure(
                    "I12.wall_seconds",
                    "wall_seconds",
                    Some(wall),
                    "seconds",
                    "union of the ready-to-start intervals, separate from the sum",
                ),
                count_measure(
                    "I12.excluded",
                    "excluded",
                    missing,
                    "intervals",
                    "pending rows with no later in-progress, or end < start",
                ),
            ],
            coverage: COVERAGE.to_string(),
            evidence: rows.into_iter().map(|row| row.key).collect(),
        })
    }
}

fn clip(start: f64, end: f64, from: Option<f64>, to: Option<f64>) -> Option<(f64, f64)> {
    let mut left = start;
    let mut right = end;
    if let Some(from) = from {
        if right <= from {
            return None;
        }
        left = left.max(from);
    }
    if let Some(to) = to {
        if left >= to {
            return None;
        }
        right = right.min(to);
    }
    (right > left).then_some((left, right))
}

fn union_len(segments: &[(f64, f64)]) -> f64 {
    let mut ordered = segments.to_vec();
    ordered.sort_by(|left, right| left.0.total_cmp(&right.0));
    let mut total = 0.0;
    let mut current: Option<(f64, f64)> = None;
    for (start, end) in ordered {
        match current {
            None => current = Some((start, end)),
            Some((left, right)) if start <= right => current = Some((left, right.max(end))),
            Some((left, right)) => {
                total += right - left;
                current = Some((start, end));
            }
        }
    }
    if let Some((left, right)) = current {
        total += right - left;
    }
    total
}

fn median(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut ordered = values.to_vec();
    ordered.sort_by(f64::total_cmp);
    let mid = ordered.len() / 2;
    if ordered.len() % 2 == 1 {
        Some(ordered[mid])
    } else {
        Some((ordered[mid - 1] + ordered[mid]) / 2.0)
    }
}

fn bounds(input: &InsightInput<'_>) -> anyhow::Result<(Option<f64>, Option<f64>)> {
    Ok((
        epoch(input, input.scope().from.as_deref())?,
        epoch(input, input.scope().to.as_deref())?,
    ))
}

fn epoch(input: &InsightInput<'_>, text: Option<&str>) -> anyhow::Result<Option<f64>> {
    let Some(text) = text else {
        return Ok(None);
    };
    let value: f64 = input.connection().query_row(
        "SELECT epoch(CAST(? AS TIMESTAMP))",
        params![text],
        |row| row.get(0),
    )?;
    Ok(Some(value))
}

fn load(input: &InsightInput<'_>) -> anyhow::Result<Vec<Row>> {
    let mut stmt = input.connection().prepare(
        "SELECT source_key, qitem_id, epoch(ts), transition_id, state
         FROM queue_facts
         WHERE (? IS NULL OR qitem_id = ? OR source_seat = ? OR dest_seat = ?)
           AND (? IS NULL OR ts >= CAST(? AS TIMESTAMP))
           AND (? IS NULL OR ts < CAST(? AS TIMESTAMP))
         ORDER BY source_key",
    )?;
    let scope = input.scope();
    let session = scope.session.as_deref();
    let from = scope.from.as_deref();
    let to = scope.to.as_deref();
    let rows = stmt.query_map(
        params![session, session, session, session, from, from, to, to],
        |row| {
            Ok(Row {
                key: row.get(0)?,
                item: row.get(1)?,
                ts: row.get(2)?,
                transition: row.get(3)?,
                state: row.get(4)?,
            })
        },
    )?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}
