//! Compiled insight registry. Each insight is one Rust type plus one registry line.
pub mod contract;
pub mod fidelity;
pub mod reference;

pub use contract::{
    display_status, format_text, Disclosure, Insight, InsightDomain, InsightInput,
    InsightPopulation, InsightResult, InsightStatus, Measure, MeasureValue, Scope,
};
pub use fidelity::{
    canonical_connector_id, canonical_population_what, connector_declaration, denominator_matrix,
    evaluate_denominator, evaluate_denominator_by_id, format_denominator_disclosure,
    format_not_computable_reason, populated_fields_in_store, render_provider_fidelity_markdown,
    ConnectorDeclaration, ConnectorExclusion, DenominatorEvaluation, DenominatorMatrixRow,
    DenominatorScope, Field, FieldDeclaration, FieldFidelity, IncludedConnector,
    PartialFidelityReason, CONNECTOR_DECLARATIONS,
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

/// Run the six shipped v0.8.0 insights over an explicit connector set, enforcing
/// denominator scope and refusing insights whose required fields are not computable.
pub fn evaluate_for_connectors<S: AsRef<str>>(
    store: &crate::AnalyticsStore,
    scope: &Scope,
    connectors: &[S],
) -> Vec<InsightResult> {
    run_for_connectors(REFERENCE, store, scope, connectors)
}

/// Run the A10 catalog I01–I25 over an explicit connector set, enforcing
/// denominator scope and refusing insights whose required fields are not computable.
pub fn evaluate_catalog_for_connectors<S: AsRef<str>>(
    store: &crate::AnalyticsStore,
    scope: &Scope,
    connectors: &[S],
) -> Vec<InsightResult> {
    run_for_connectors(CATALOG, store, scope, connectors)
}

/// Evaluate a single insight over an explicit connector set, enforcing denominator scope
/// so connectors lacking required fields are never silently pooled into the denominator.
pub fn evaluate_insight_for_connectors<S: AsRef<str>>(
    insight: &dyn Insight,
    store: &crate::AnalyticsStore,
    scope: &Scope,
    connectors: &[S],
) -> InsightResult {
    if let Some(reason) = scope.refusal() {
        return refused(insight, reason, "undefined");
    }
    match evaluate_denominator(insight, connectors) {
        DenominatorEvaluation::NotComputable {
            missing,
            by_connector,
            ..
        } => {
            let reason = format_not_computable_reason(&missing, &by_connector);
            refused(insight, &reason, canonical_population_what(insight.id()))
        }
        DenominatorEvaluation::Computable { scope: denom_scope } => {
            let excluded_harnesses: Vec<String> = denom_scope
                .excluded
                .iter()
                .map(|exc| exc.connector.clone())
                .collect();
            let mut effective_scope = scope.clone();
            if effective_scope.harness.is_none()
                && denom_scope.included.len() == 1
                && !denom_scope.excluded.is_empty()
            {
                effective_scope.harness = Some(denom_scope.included[0].connector.to_string());
            }
            let input = Input::with_excluded_harnesses(store, &effective_scope, excluded_harnesses);
            let mut result = insight.compute(&input);
            if let Some(disclosure) = format_denominator_disclosure(&denom_scope) {
                result.coverage = format!("{} | {disclosure}", result.coverage);
            }
            result
        }
    }
}

fn run_for_connectors<S: AsRef<str>>(
    insights: &[&dyn Insight],
    store: &crate::AnalyticsStore,
    scope: &Scope,
    connectors: &[S],
) -> Vec<InsightResult> {
    let mut results: Vec<_> = insights
        .iter()
        .map(|insight| evaluate_insight_for_connectors(*insight, store, scope, connectors))
        .collect();
    results.sort_by(|left, right| left.id.cmp(right.id).then(left.name.cmp(right.name)));
    results
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
                let mut result = insight.compute(&input);
                if let Some(harness) = scope.harness.as_deref() {
                    if connector_declaration(harness).is_some() {
                        if let DenominatorEvaluation::NotComputable {
                            missing,
                            by_connector,
                            ..
                        } = evaluate_denominator(*insight, &[harness])
                        {
                            if matches!(
                                &result.status,
                                InsightStatus::Refused { reason }
                                    if reason == "empty population" || reason == "not_observable"
                            ) {
                                let reason = format_not_computable_reason(&missing, &by_connector);
                                result.status = InsightStatus::Refused {
                                    reason: reason.clone(),
                                };
                                result.coverage = reason;
                            }
                        }
                    }
                }
                result
            }
        })
        .collect();
    results.sort_by(|left, right| left.id.cmp(right.id).then(left.name.cmp(right.name)));
    results
}
