//! Compiled insight registry. Each insight is one Rust type plus one registry line.
pub mod contract;
pub mod reference;

pub use contract::{
    display_status, format_text, Disclosure, Insight, InsightDomain, InsightInput,
    InsightPopulation, InsightResult, InsightStatus, Measure, MeasureValue, Scope,
};
pub use reference::{evaluate_i22_scoped, CATALOG, REFERENCE};

use crate::domain_v1::Identifier;
use contract::{refused, InsightInput as Input};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsightRefusal {
    RegistryNotAdmitted,
}

pub fn check_question_admission(question: &Identifier) -> Result<(), InsightRefusal> {
    let name = question.as_str();
    if CATALOG
        .iter()
        .any(|insight| insight.id() == name || insight.name() == name)
    {
        Ok(())
    } else {
        Err(InsightRefusal::RegistryNotAdmitted)
    }
}

/// Run the six shipped v0.8.0 insights. Results are ordered by id.
pub fn evaluate(store: &crate::AnalyticsStore, scope: &Scope) -> Vec<InsightResult> {
    run(REFERENCE, store, scope)
}

/// Run the A10 catalog I01–I25. Results are ordered by id.
/// A08 is the I01–I21 prefix of [`CATALOG`].
pub fn evaluate_catalog(store: &crate::AnalyticsStore, scope: &Scope) -> Vec<InsightResult> {
    run(CATALOG, store, scope)
}

fn run(
    insights: &[&dyn Insight],
    store: &crate::AnalyticsStore,
    scope: &Scope,
) -> Vec<InsightResult> {
    let input = Input::new(store, scope);
    let mut results: Vec<_> = insights
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
