//! Compiled insight registry. Each insight is one Rust type plus one registry line.
pub mod contract;
pub mod reference;

pub use contract::{
    format_text, Insight, InsightDomain, InsightInput, InsightPopulation, InsightResult,
    InsightStatus, Measure, MeasureValue, Scope,
};
pub use reference::REFERENCE;

use crate::domain_v1::Identifier;
use contract::{refused, InsightInput as Input};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsightRefusal {
    RegistryNotAdmitted,
}

pub fn check_question_admission(question: &Identifier) -> Result<(), InsightRefusal> {
    let name = question.as_str();
    if REFERENCE
        .iter()
        .any(|insight| insight.id() == name || insight.name() == name)
    {
        Ok(())
    } else {
        Err(InsightRefusal::RegistryNotAdmitted)
    }
}

/// Run every registered insight. Results are ordered by id.
pub fn evaluate(store: &crate::AnalyticsStore, scope: &Scope) -> Vec<InsightResult> {
    let input = Input::new(store, scope);
    let mut results: Vec<_> = REFERENCE
        .iter()
        .map(|insight| {
            if let Some(reason) = scope.refusal() {
                refused(*insight, reason, "undefined")
            } else {
                insight.compute(&input)
            }
        })
        .collect();
    results.sort_by(|left, right| left.id.cmp(right.id).then(left.name.cmp(right.name)));
    results
}
