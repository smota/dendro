//! I10 work.duration. Wall union, summed attempt-seconds, and summed/wall.
use super::super::contract::{
    finish, refused, Insight, InsightDomain, InsightInput, InsightPopulation, InsightResult,
    InsightStatus,
};
use super::support::{count_measure, float_measure};
use duckdb::params;

pub struct WorkDuration;

pub static INSIGHT: WorkDuration = WorkDuration;

const COVERAGE: &str =
    "half-open same-clock running segments; a lone start is missing_end and is not extended to the cutoff";

struct Event {
    key: String,
    seat: String,
    ts: f64,
    state: String,
}

impl Insight for WorkDuration {
    fn id(&self) -> &'static str {
        "I10"
    }
    fn name(&self) -> &'static str {
        "work.duration"
    }
    fn version(&self) -> &'static str {
        "1.0.0"
    }
    fn domain(&self) -> InsightDomain {
        InsightDomain::Coordination
    }
    fn compute(&self, input: &InsightInput<'_>) -> InsightResult {
        let events = match load(input) {
            Ok(rows) => rows,
            Err(_) => return refused(self, "store read failed", "activity_facts"),
        };
        if events.is_empty() {
            return refused(self, "empty population", "activity_facts");
        }
        let (from, to) = match bounds(input) {
            Ok(bounds) => bounds,
            Err(_) => return refused(self, "undefined input", "activity_facts"),
        };
        let mut by_seat: std::collections::BTreeMap<String, Vec<&Event>> =
            std::collections::BTreeMap::new();
        for event in &events {
            by_seat.entry(event.seat.clone()).or_default().push(event);
        }
        let mut segments = Vec::new();
        let mut missing_end = 0i64;
        let mut conflicts = 0i64;
        for group in by_seat.values_mut() {
            group
                .sort_by(|left, right| left.ts.total_cmp(&right.ts).then(left.key.cmp(&right.key)));
            for index in 0..group.len() {
                if group[index].state != "running" {
                    continue;
                }
                let Some(next) = group.get(index + 1) else {
                    missing_end += 1;
                    continue;
                };
                if next.ts < group[index].ts {
                    conflicts += 1;
                    continue;
                }
                if let Some(segment) = clip(group[index].ts, next.ts, from, to) {
                    segments.push(segment);
                }
            }
        }
        let summed: f64 = segments.iter().map(|(start, end)| end - start).sum();
        let wall = union_len(&segments);
        let ratio = if wall == 0.0 {
            None
        } else {
            Some(summed / wall)
        };
        let status = if conflicts > 0 {
            InsightStatus::Partial {
                reason: "invalid interval".to_string(),
            }
        } else if missing_end > 0 {
            InsightStatus::Partial {
                reason: "missing_end".to_string(),
            }
        } else if wall == 0.0 {
            InsightStatus::Partial {
                reason: "wall=0".to_string(),
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
                what: "activity_facts".to_string(),
                n: events.len() as u64,
            },
            measures: vec![
                float_measure(
                    "I10.wall_seconds",
                    "wall_seconds",
                    Some(wall),
                    "seconds",
                    "length of the union of half-open running segments",
                ),
                float_measure(
                    "I10.summed_seconds",
                    "summed_seconds",
                    Some(summed),
                    "seconds",
                    "sum of distinct running segment lengths",
                ),
                float_measure(
                    "I10.concurrency",
                    "concurrency",
                    ratio,
                    "ratio",
                    "summed/wall; unknown when wall is 0",
                ),
                count_measure(
                    "I10.missing_end",
                    "missing_end",
                    missing_end,
                    "events",
                    "running events with no later endpoint; not extended to the cutoff",
                ),
                count_measure(
                    "I10.excluded",
                    "excluded",
                    conflicts,
                    "intervals",
                    "end < start intervals excluded as conflict",
                ),
            ],
            coverage: COVERAGE.to_string(),
            evidence: events.into_iter().map(|event| event.key).collect(),
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

fn load(input: &InsightInput<'_>) -> anyhow::Result<Vec<Event>> {
    let mut stmt = input.connection().prepare(
        "SELECT source_key, seat, epoch(ts), state
         FROM activity_facts
         WHERE (? IS NULL OR seat = ?)
           AND (? IS NULL OR ts >= CAST(? AS TIMESTAMP))
           AND (? IS NULL OR ts < CAST(? AS TIMESTAMP))
         ORDER BY source_key",
    )?;
    let scope = input.scope();
    let session = scope.session.as_deref();
    let from = scope.from.as_deref();
    let to = scope.to.as_deref();
    let rows = stmt.query_map(params![session, session, from, from, to, to], |row| {
        Ok(Event {
            key: row.get(0)?,
            seat: row.get(1)?,
            ts: row.get(2)?,
            state: row.get(3)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}
