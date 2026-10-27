//! I22 harness.trajectory_vortex. Cohort centroid and k-turn window in Rust.
//! The SQL trigger only marks a session dirty. Changing these constants bumps
//! the analyzer version.
use super::super::contract::{
    finish, refused, Insight, InsightDomain, InsightInput, InsightPopulation, InsightResult,
    InsightStatus,
};
use super::support::{count_measure, float_measure, table_exists};
use duckdb::Connection;
use std::collections::BTreeMap;

pub struct TrajectoryVortex;

pub static INSIGHT: TrajectoryVortex = TrajectoryVortex;

const MIN_COHORT: usize = 5;
const K_TURNS: usize = 3;
const SELF_RATE_MIN: f64 = 0.5;
const DISTANCE_QUANTILE: f64 = 0.95;
const IDENTICAL: f64 = 1e-9;

const COVERAGE: &str = "\
a vortex trip is a cohort distance signal and not a cause \
| assumption: the FLOAT[32] projection, k=3 window, self-transition rate 0.5, and quantile 0.95 are frozen with analyzer 1.0.0 \
| alternative: repeated embeddings can be the same capability class rather than a stuck trajectory \
| formula: cosine distance to the untripped cohort centroid above quantile_cont 0.95, with self-transition rate above 0.5 for 3 consecutive turns";

struct Turn {
    id: String,
    subject: String,
    turn: i64,
    vector: Vec<f64>,
}

impl Insight for TrajectoryVortex {
    fn id(&self) -> &'static str {
        "I22"
    }
    fn name(&self) -> &'static str {
        "harness.trajectory_vortex"
    }
    fn version(&self) -> &'static str {
        "1.0.0"
    }
    fn domain(&self) -> InsightDomain {
        InsightDomain::Harness
    }
    fn compute(&self, input: &InsightInput<'_>) -> InsightResult {
        evaluate_connection(input.connection(), input.scope().session.as_deref())
    }
}

/// Tripwire entry. `"unscoped"` evaluates every self-window subject.
/// Any other subject is the only vortex candidate. The reference cohort
/// is always the full table.
pub(super) fn evaluate_scoped(conn: &Connection, subject_key: &str) -> InsightResult {
    let session = if subject_key == "unscoped" {
        None
    } else {
        Some(subject_key)
    };
    evaluate_connection(conn, session)
}

fn evaluate_connection(conn: &Connection, session: Option<&str>) -> InsightResult {
    let session = match session {
        Some("unscoped") => None,
        other => other,
    };
    let insight = &TrajectoryVortex;
    if !table_exists(conn, "harness_trajectory") {
        return refused(insight, "insufficient_sample", "harness_trajectory");
    }
    let turns = match load(conn) {
        Ok(rows) => rows,
        Err(_) => return refused(insight, "store read failed", "harness_trajectory"),
    };
    if turns.is_empty() {
        return refused(insight, "insufficient_sample", "harness_trajectory");
    }
    let mut by_subject: BTreeMap<String, Vec<Turn>> = BTreeMap::new();
    let mut evidence = Vec::new();
    for turn in turns {
        evidence.push(turn.id.clone());
        by_subject
            .entry(turn.subject.clone())
            .or_default()
            .push(turn);
    }
    let mut other = 0i64;
    let mut reference = Vec::new();
    let mut candidates = Vec::new();
    for (subject, mut group) in by_subject {
        group.sort_by(|left, right| left.turn.cmp(&right.turn).then(left.id.cmp(&right.id)));
        let Some(vector) = session_vector(&group) else {
            other += 1;
            continue;
        };
        if has_self_window(&group) {
            // Outside the scoped target, a self-window is neither reference nor a candidate.
            if session.is_none() || session == Some(subject.as_str()) {
                candidates.push(vector);
            }
        } else {
            reference.push(vector);
        }
    }
    if reference.len() < MIN_COHORT {
        return refused(insight, "insufficient_sample", "harness_trajectory");
    }
    let centroid = mean_vectors(&reference);
    let distances: Vec<f64> = reference
        .iter()
        .map(|vector| cosine_distance(vector, &centroid))
        .filter(|distance| distance.is_finite())
        .collect();
    let threshold = match quantile_cont(conn, &distances, DISTANCE_QUANTILE) {
        Ok(value) => value,
        Err(_) => return refused(insight, "store read failed", "harness_trajectory"),
    };
    let tripped = candidates
        .iter()
        .filter(|vector| {
            let distance = cosine_distance(vector, &centroid);
            distance.is_finite() && distance > threshold
        })
        .count() as i64;
    finish(InsightResult {
        id: insight.id(),
        name: insight.name(),
        version: insight.version(),
        domain: insight.domain(),
        status: InsightStatus::Computed,
        population: InsightPopulation {
            what: "harness_trajectory".to_string(),
            n: evidence.len() as u64,
        },
        measures: vec![
            count_measure(
                "I22.tripped",
                "tripped",
                tripped,
                "sessions",
                "candidates whose cosine distance to the untripped centroid exceeds the cohort quantile",
            ),
            count_measure(
                "I22.cohort",
                "cohort",
                reference.len() as i64,
                "sessions",
                "completed sessions that do not themselves have the k-turn self-transition window",
            ),
            count_measure(
                "I22.capability_class_other",
                "capability_class_other",
                other,
                "sessions",
                "sessions whose embeddings are zero and are not trip fuel",
            ),
            float_measure(
                "I22.distance_threshold",
                "distance_threshold",
                Some(threshold),
                "cosine_distance",
                "quantile_cont of untripped session distances at 0.95",
            ),
        ],
        coverage: COVERAGE.to_string(),
        evidence,
    })
}

fn load(conn: &Connection) -> anyhow::Result<Vec<Turn>> {
    let mut stmt = conn.prepare(
        "SELECT COALESCE(observation_id, session_key, 'obs'),
                COALESCE(subject_key, session_key, 'unscoped'),
                COALESCE(turn_k, 0),
                COALESCE(CAST(embedding AS VARCHAR), CAST(vec AS VARCHAR))
         FROM harness_trajectory
         ORDER BY 2, 3, 1",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, String>(3)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (id, subject, turn, text) = row?;
        out.push(Turn {
            id,
            subject,
            turn,
            vector: parse_vector(&text)?,
        });
    }
    Ok(out)
}

fn session_vector(turns: &[Turn]) -> Option<Vec<f64>> {
    let usable: Vec<Vec<f64>> = turns
        .iter()
        .filter(|turn| !is_zero(&turn.vector))
        .map(|turn| turn.vector.clone())
        .collect();
    if usable.is_empty() {
        None
    } else {
        Some(mean_vectors(&usable))
    }
}

fn has_self_window(turns: &[Turn]) -> bool {
    if turns.len() < K_TURNS {
        return false;
    }
    turns.windows(K_TURNS).any(|window| {
        if window.iter().any(|turn| is_zero(&turn.vector)) {
            return false;
        }
        let mut identical = 0.0;
        let mut pairs = 0.0;
        for pair in window.windows(2) {
            pairs += 1.0;
            if cosine_distance(&pair[0].vector, &pair[1].vector) <= IDENTICAL {
                identical += 1.0;
            }
        }
        pairs > 0.0 && identical / pairs > SELF_RATE_MIN
    })
}

fn is_zero(vector: &[f64]) -> bool {
    vector.iter().all(|value| *value == 0.0)
}

fn mean_vectors(vectors: &[Vec<f64>]) -> Vec<f64> {
    let width = vectors.iter().map(Vec::len).max().unwrap_or(0);
    let mut sum = vec![0.0; width];
    for vector in vectors {
        for (index, value) in vector.iter().enumerate() {
            sum[index] += *value;
        }
    }
    let n = vectors.len() as f64;
    if n == 0.0 {
        return sum;
    }
    for value in &mut sum {
        *value /= n;
    }
    sum
}

fn cosine_distance(left: &[f64], right: &[f64]) -> f64 {
    let n = left.len().min(right.len());
    let mut dot = 0.0;
    let mut left_norm = 0.0;
    let mut right_norm = 0.0;
    for index in 0..n {
        dot += left[index] * right[index];
        left_norm += left[index] * left[index];
        right_norm += right[index] * right[index];
    }
    if left_norm == 0.0 || right_norm == 0.0 {
        return 1.0;
    }
    1.0 - (dot / (left_norm.sqrt() * right_norm.sqrt()))
}

fn parse_vector(text: &str) -> anyhow::Result<Vec<f64>> {
    let trimmed = text.trim().trim_matches(['[', ']']);
    if trimmed.is_empty() {
        anyhow::bail!("empty embedding");
    }
    let mut values = Vec::new();
    for part in trimmed.split(',') {
        let value = part
            .trim()
            .parse::<f64>()
            .map_err(|_| anyhow::anyhow!("embedding component is not a float"))?;
        if !value.is_finite() {
            anyhow::bail!("embedding component is not finite");
        }
        values.push(value);
    }
    Ok(values)
}

fn quantile_cont(conn: &Connection, values: &[f64], quantile: f64) -> anyhow::Result<f64> {
    if values.is_empty() {
        anyhow::bail!("empty quantile sample");
    }
    let list = values
        .iter()
        .map(|value| format!("({value})"))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!("SELECT quantile_cont(x, {quantile}) FROM (VALUES {list}) t(x)");
    let value: f64 = conn.query_row(&sql, [], |row| row.get(0))?;
    if !value.is_finite() {
        anyhow::bail!("quantile is not finite");
    }
    Ok(value)
}
