//! I23 harness.schema_drift. L1 distance of envelope key-path and type distributions.
use super::super::contract::{
    finish, label_id, refused, Insight, InsightDomain, InsightInput, InsightPopulation,
    InsightResult, InsightStatus,
};
use super::support::{count_measure, float_measure, table_exists};
use duckdb::params;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub struct SchemaDrift;

pub static INSIGHT: SchemaDrift = SchemaDrift;

const MIN_SAMPLE: usize = 5;

const COVERAGE: &str = "\
key-path and type drift is not a compatibility verdict \
| assumption: only immediate object keys of non-refused envelopes enter the distribution \
| alternative: a new key can be an additive field rather than a breaking change \
| formula: L1 distance of aligned key and type probabilities plus added, removed, and retyped key counts";

struct Envelope {
    id: String,
    harness: String,
    version: String,
    shape: Option<String>,
}

impl Insight for SchemaDrift {
    fn id(&self) -> &'static str {
        "I23"
    }
    fn name(&self) -> &'static str {
        "harness.schema_drift"
    }
    fn version(&self) -> &'static str {
        "1.0.0"
    }
    fn domain(&self) -> InsightDomain {
        InsightDomain::Harness
    }
    fn compute(&self, input: &InsightInput<'_>) -> InsightResult {
        if !table_exists(input.connection(), "harness_envelope") {
            return refused(self, "insufficient_sample", "harness_envelope");
        }
        let rows = match load(input) {
            Ok(rows) => rows,
            Err(_) => return refused(self, "store read failed", "harness_envelope"),
        };
        if rows.is_empty() {
            return refused(self, "insufficient_sample", "harness_envelope");
        }
        let mut excluded = 0i64;
        let mut by_version: BTreeMap<(String, String), Vec<BTreeMap<String, String>>> =
            BTreeMap::new();
        let mut evidence = Vec::new();
        for row in &rows {
            evidence.push(row.id.clone());
            let Some(shape) = row.shape.as_deref() else {
                excluded += 1;
                continue;
            };
            let Some(keys) = key_types(shape) else {
                excluded += 1;
                continue;
            };
            by_version
                .entry((row.harness.clone(), row.version.clone()))
                .or_default()
                .push(keys);
        }
        type HarnessEnvelopes = BTreeMap<String, Vec<(String, Vec<BTreeMap<String, String>>)>>;
        let mut by_harness: HarnessEnvelopes = BTreeMap::new();
        for ((harness, version), shapes) in by_version {
            by_harness
                .entry(harness)
                .or_default()
                .push((version, shapes));
        }
        let mut measures = vec![count_measure(
            "I23.excluded",
            "excluded",
            excluded,
            "envelopes",
            "refused or unparsed envelopes excluded from both sides",
        )];
        let mut valid = 0i64;
        let mut short = 0i64;
        for (harness, mut versions) in by_harness {
            versions.sort_by(|left, right| left.0.cmp(&right.0));
            for pair in versions.windows(2) {
                let (left_version, left) = &pair[0];
                let (right_version, right) = &pair[1];
                if left.len() < MIN_SAMPLE || right.len() < MIN_SAMPLE {
                    short += 1;
                    continue;
                }
                valid += 1;
                let drift = l1(left, right);
                let prefix = format!(
                    "I23.{}.{}.{}",
                    label_id(&harness),
                    label_id(left_version),
                    label_id(right_version)
                );
                measures.push(float_measure(
                    &format!("{prefix}.l1"),
                    "l1",
                    Some(drift.l1),
                    "probability",
                    "sum of absolute probability differences over aligned key and type pairs",
                ));
                measures.push(count_measure(
                    &format!("{prefix}.added"),
                    "added",
                    drift.added,
                    "keys",
                    "keys present on the later version and absent on the earlier version",
                ));
                measures.push(count_measure(
                    &format!("{prefix}.removed"),
                    "removed",
                    drift.removed,
                    "keys",
                    "keys present on the earlier version and absent on the later version",
                ));
                measures.push(count_measure(
                    &format!("{prefix}.retyped"),
                    "retyped",
                    drift.retyped,
                    "keys",
                    "keys whose observed type set changed; presence change is counted separately",
                ));
            }
        }
        let status = if valid == 0 {
            InsightStatus::Refused {
                reason: "insufficient_sample".to_string(),
            }
        } else if short > 0 {
            InsightStatus::Partial {
                reason: "short cohort excluded".to_string(),
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
                what: "harness_envelope".to_string(),
                n: rows.len() as u64,
            },
            measures,
            coverage: COVERAGE.to_string(),
            evidence,
        })
    }
}

struct Drift {
    l1: f64,
    added: i64,
    removed: i64,
    retyped: i64,
}

fn l1(left: &[BTreeMap<String, String>], right: &[BTreeMap<String, String>]) -> Drift {
    let left_dist = distribution(left);
    let right_dist = distribution(right);
    let mut keys = BTreeSet::new();
    keys.extend(left_dist.keys().cloned());
    keys.extend(right_dist.keys().cloned());
    let mut distance = 0.0;
    for key in keys {
        let left_p = left_dist.get(&key).copied().unwrap_or(0.0);
        let right_p = right_dist.get(&key).copied().unwrap_or(0.0);
        distance += (left_p - right_p).abs();
    }
    let left_keys = key_type_sets(left);
    let right_keys = key_type_sets(right);
    let added = right_keys
        .keys()
        .filter(|key| !left_keys.contains_key(*key))
        .count() as i64;
    let removed = left_keys
        .keys()
        .filter(|key| !right_keys.contains_key(*key))
        .count() as i64;
    let retyped = left_keys
        .iter()
        .filter(|(key, types)| right_keys.get(*key).is_some_and(|other| other != *types))
        .count() as i64;
    Drift {
        l1: distance,
        added,
        removed,
        retyped,
    }
}

fn distribution(rows: &[BTreeMap<String, String>]) -> BTreeMap<(String, String), f64> {
    let mut counts: BTreeMap<(String, String), f64> = BTreeMap::new();
    for row in rows {
        for (key, kind) in row {
            *counts.entry((key.clone(), kind.clone())).or_default() += 1.0;
        }
    }
    let n = rows.len() as f64;
    if n == 0.0 {
        return counts;
    }
    for value in counts.values_mut() {
        *value /= n;
    }
    counts
}

fn key_type_sets(rows: &[BTreeMap<String, String>]) -> BTreeMap<String, BTreeSet<String>> {
    let mut sets: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for row in rows {
        for (key, kind) in row {
            sets.entry(key.clone()).or_default().insert(kind.clone());
        }
    }
    sets
}

fn key_types(shape: &str) -> Option<BTreeMap<String, String>> {
    let value: Value = serde_json::from_str(shape).ok()?;
    let object = value.as_object()?;
    let mut keys = BTreeMap::new();
    for (key, child) in object {
        keys.insert(key.clone(), json_type(child).to_string());
    }
    Some(keys)
}

fn json_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn load(input: &InsightInput<'_>) -> anyhow::Result<Vec<Envelope>> {
    let mut stmt = input.connection().prepare(
        "SELECT observation_id, harness, harness_version,
                CAST(CAST(envelope_shape AS JSON) AS VARCHAR)
         FROM harness_envelope
         WHERE (? IS NULL OR harness = ?)
         ORDER BY harness, harness_version, observation_id",
    )?;
    let harness = input.scope().harness.as_deref();
    let rows = stmt.query_map(params![harness, harness], |row| {
        Ok(Envelope {
            id: row.get(0)?,
            harness: row.get(1)?,
            version: row.get(2)?,
            shape: row.get(3)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}
