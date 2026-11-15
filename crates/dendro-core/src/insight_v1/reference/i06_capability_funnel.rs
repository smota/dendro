//! I06 harness.capability_funnel. Loaded and invoked capability IDs.
use super::super::contract::{
    finish, refused, Insight, InsightDomain, InsightInput, InsightPopulation, InsightResult,
    InsightStatus,
};
use super::support::{count_measure, float_measure};
use crate::facts::READER_NAMESPACE;
use duckdb::params;
use std::collections::BTreeSet;

pub struct CapabilityFunnel;

pub static INSIGHT: CapabilityFunnel = CapabilityFunnel;

const COVERAGE: &str =
    "rate is invoked intersect loaded over loaded; absent loaded evidence refuses the rate and still counts actions";

impl Insight for CapabilityFunnel {
    fn id(&self) -> &'static str {
        "I06"
    }
    fn name(&self) -> &'static str {
        "harness.capability_funnel"
    }
    fn version(&self) -> &'static str {
        "1.0.0"
    }
    fn domain(&self) -> InsightDomain {
        InsightDomain::Harness
    }
    fn required_fields(&self) -> &'static [crate::insight_v1::Field] {
        use crate::insight_v1::Field;
        &[
            Field::SkillDeclarations,
            Field::SkillInvocations,
            Field::ToolCallName,
        ]
    }
    fn optional_fields(&self) -> &'static [crate::insight_v1::Field] {
        use crate::insight_v1::Field;
        &[Field::NativeSession, Field::TurnTimestamp]
    }
    fn coverage(&self) -> &'static str {
        COVERAGE
    }
    fn compute(&self, input: &InsightInput<'_>) -> InsightResult {
        let (skills, unattributed_excluded) = match load_skills(input) {
            Ok(rows) => rows,
            Err(_) => return refused(self, "store read failed", "skill_facts"),
        };
        let actions = match load_actions(input) {
            Ok(count) => count,
            Err(_) => return refused(self, "store read failed", "tool_facts"),
        };
        let mut coverage = COVERAGE.to_string();
        if unattributed_excluded > 0 {
            coverage.push_str(&format!(
                "; excluded: {unattributed_excluded} unattributed rows"
            ));
        }
        if skills.is_empty() && actions.1 == 0 {
            let mut res = refused(self, "empty population", "skill_facts");
            res.coverage = coverage;
            return res;
        }
        let mut loaded = BTreeSet::new();
        let mut invoked = BTreeSet::new();
        for (skill, was_invoked) in &skills {
            if *was_invoked {
                invoked.insert(skill.clone());
            } else {
                loaded.insert(skill.clone());
            }
        }
        let intersection = loaded.intersection(&invoked).count() as i64;
        let rate = if loaded.is_empty() {
            None
        } else {
            Some(intersection as f64 / loaded.len() as f64)
        };
        let status = if loaded.is_empty() {
            InsightStatus::Partial {
                reason: "absent loaded evidence".to_string(),
            }
        } else {
            InsightStatus::Computed
        };
        let evidence = skills
            .iter()
            .map(|(skill, _)| skill.clone())
            .chain(actions.0)
            .collect();
        finish(InsightResult {
            id: self.id(),
            name: self.name(),
            version: self.version(),
            domain: self.domain(),
            status,
            population: InsightPopulation {
                what: "skill_facts".to_string(),
                n: (skills.len() as u64) + actions.1 as u64,
            },
            measures: vec![
                count_measure(
                    "I06.loaded",
                    "loaded",
                    loaded.len() as i64,
                    "capabilities",
                    "distinct skills with invoked = false",
                ),
                count_measure(
                    "I06.invoked",
                    "invoked",
                    invoked.len() as i64,
                    "capabilities",
                    "distinct skills with invoked = true",
                ),
                count_measure(
                    "I06.intersection",
                    "intersection",
                    intersection,
                    "capabilities",
                    "skills that are both loaded and invoked",
                ),
                float_measure(
                    "I06.rate",
                    "rate",
                    rate,
                    "ratio",
                    "intersection / loaded; unknown when loaded evidence is absent",
                ),
                count_measure(
                    "I06.actions",
                    "actions",
                    actions.1,
                    "actions",
                    "tool_facts rows in scope; counted even when the rate is refused",
                ),
            ],
            coverage,
            evidence,
        })
    }
}

fn load_skills(input: &InsightInput<'_>) -> anyhow::Result<(Vec<(String, bool)>, usize)> {
    use crate::insight_v1::fidelity::{infer_skill_row_connector, load_seat_harnesses};

    let check_allow = input.has_allowed_harnesses();
    let seat_harnesses = if check_allow {
        load_seat_harnesses(input.connection())
    } else {
        std::collections::BTreeMap::new()
    };

    let mut stmt = input
        .connection()
        .prepare("SELECT source_key, seat, skill, invoked FROM skill_facts ORDER BY source_key")?;
    let scope = input.scope();
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, bool>(3)?,
        ))
    })?;
    let mut out = Vec::new();
    let mut unattributed_excluded = 0usize;
    for row in rows {
        let (key, seat, skill, invoked) = row?;
        if scope.harness.as_ref().is_some_and(|harness| {
            skill != *harness
                && seat != *harness
                && !seat.starts_with(&format!("{harness}:"))
                && !key.starts_with(harness)
                && !skill.starts_with(harness)
        }) {
            continue;
        }
        if check_allow {
            match infer_skill_row_connector(&seat_harnesses, &key, &seat, &skill) {
                Some(conn) => {
                    if !input.allows_harness(&conn) {
                        continue;
                    }
                }
                None => {
                    unattributed_excluded += 1;
                    continue;
                }
            }
        }
        out.push((skill, invoked));
    }
    Ok((out, unattributed_excluded))
}

fn load_actions(input: &InsightInput<'_>) -> anyhow::Result<(Vec<String>, i64)> {
    let mut stmt = input.connection().prepare(
        "SELECT source_key, harness
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
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
    )?;
    let mut keys = Vec::new();
    for row in rows {
        let (key, row_harness) = row?;
        if input.allows_harness(&row_harness) {
            keys.push(key);
        }
    }
    let count = keys.len() as i64;
    Ok((keys, count))
}
