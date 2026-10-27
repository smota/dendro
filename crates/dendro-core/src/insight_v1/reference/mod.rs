//! Compiled reference insights. Add one module and one `CATALOG` line.
//! `REFERENCE` stays the six v0.8.0 analyzers that `evaluate` runs.
//! `CATALOG` is the A10 set I01–I25. A08 is the I01–I21 prefix of that set.
mod i01_invocations;
mod i02_tokens;
mod i03_tokens_inclusive;
mod i04_cost;
mod i05_configuration_divergence;
mod i06_capability_funnel;
mod i07_context_pressure;
mod i08_action_failure;
mod i09_lineage_shape;
mod i10_duration;
mod i11_shared_allocation;
mod i12_queue_wait;
mod i13_placement;
mod i14_resources;
mod i15_effects;
mod i16_isolation;
mod i17_authority;
mod i18_coverage;
mod i19_acceptance;
mod i20_resource_efficiency;
mod i21_comparison;
mod i22_trajectory_vortex;
mod i23_schema_drift;
mod i24_blast_radius;
mod i25_counterfactual;
mod support;

use super::contract::Insight;
use super::InsightRefusal;
use crate::domain_v1::Identifier;

/// Shipped v0.8.0 analyzers. `evaluate` runs this set and no other.
pub static REFERENCE: &[&dyn Insight] = &[
    &i02_tokens::INSIGHT as &dyn Insight,
    &i04_cost::INSIGHT as &dyn Insight,
    &i08_action_failure::INSIGHT as &dyn Insight,
    &i09_lineage_shape::INSIGHT as &dyn Insight,
    &i14_resources::INSIGHT as &dyn Insight,
    &i18_coverage::INSIGHT as &dyn Insight,
];

/// A10 registry. A08 is `CATALOG[..21]` (I01–I21).
pub static CATALOG: &[&dyn Insight] = &[
    &i01_invocations::INSIGHT as &dyn Insight,
    &i02_tokens::INSIGHT as &dyn Insight,
    &i03_tokens_inclusive::INSIGHT as &dyn Insight,
    &i04_cost::INSIGHT as &dyn Insight,
    &i05_configuration_divergence::INSIGHT as &dyn Insight,
    &i06_capability_funnel::INSIGHT as &dyn Insight,
    &i07_context_pressure::INSIGHT as &dyn Insight,
    &i08_action_failure::INSIGHT as &dyn Insight,
    &i09_lineage_shape::INSIGHT as &dyn Insight,
    &i10_duration::INSIGHT as &dyn Insight,
    &i11_shared_allocation::INSIGHT as &dyn Insight,
    &i12_queue_wait::INSIGHT as &dyn Insight,
    &i13_placement::INSIGHT as &dyn Insight,
    &i14_resources::INSIGHT as &dyn Insight,
    &i15_effects::INSIGHT as &dyn Insight,
    &i16_isolation::INSIGHT as &dyn Insight,
    &i17_authority::INSIGHT as &dyn Insight,
    &i18_coverage::INSIGHT as &dyn Insight,
    &i19_acceptance::INSIGHT as &dyn Insight,
    &i20_resource_efficiency::INSIGHT as &dyn Insight,
    &i21_comparison::INSIGHT as &dyn Insight,
    &i22_trajectory_vortex::INSIGHT as &dyn Insight,
    &i23_schema_drift::INSIGHT as &dyn Insight,
    &i24_blast_radius::INSIGHT as &dyn Insight,
    &i25_counterfactual::INSIGHT as &dyn Insight,
];

/// Presentation catalog `(id, canonical name, domain)`.
/// [`REFERENCE`] is the analyzers that compute. This index is the I01–I25 set.
pub static VNEXT_REFERENCE: &[(&str, &str, &str)] = &[
    // @vnext-insight-catalog
    ("I01", "model.invocations", "model"),
    ("I02", "model.tokens.direct", "model"),
    ("I03", "model.tokens.inclusive", "model"),
    ("I04", "model.cost", "model"),
    ("I05", "model.configuration_divergence", "model"),
    ("I06", "harness.capability_funnel", "harness"),
    ("I07", "harness.context_pressure", "harness"),
    ("I08", "harness.action_failure", "harness"),
    ("I09", "work.lineage_shape", "coordination"),
    ("I10", "work.duration", "coordination"),
    ("I11", "work.shared_allocation", "coordination"),
    ("I12", "coord.queue_wait", "coordination"),
    ("I13", "environment.placement", "environment"),
    ("I14", "environment.resources", "environment"),
    ("I15", "environment.effects", "environment"),
    ("I16", "security.isolation", "security"),
    ("I17", "security.authority", "security"),
    ("I18", "coverage.source", "environment"),
    ("I19", "outcome.acceptance", "coordination"),
    ("I20", "outcome.resource_efficiency", "coordination"),
    ("I21", "comparison.descriptive", "model"),
    ("I22", "harness.trajectory_vortex", "harness"),
    ("I23", "harness.schema_drift", "harness"),
    ("I24", "security.authority_blast_radius", "security"),
    ("I25", "model.counterfactual_routing", "model"),
    // @vnext-insight-catalog-end
];

/// Insight ids in [`VNEXT_REFERENCE`], in catalog order.
pub fn all_insight_ids() -> Vec<&'static str> {
    VNEXT_REFERENCE.iter().map(|row| row.0).collect()
}

/// Canonical I22 for one dirty subject. The SQL trigger does not run this.
pub fn evaluate_i22_scoped(
    conn: &duckdb::Connection,
    subject_key: &str,
) -> super::contract::InsightResult {
    i22_trajectory_vortex::evaluate_scoped(conn, subject_key)
}

pub fn check_measure_admission(_measure: &Identifier) -> Result<(), InsightRefusal> {
    Err(InsightRefusal::RegistryNotAdmitted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain_v1::Identifier;
    use crate::insight_v1::check_question_admission;
    use std::collections::BTreeSet;

    #[test]
    fn reference_ids_and_names_are_unique() {
        let mut seen = BTreeSet::new();
        for insight in REFERENCE {
            assert!(seen.insert(insight.id()), "duplicate id {}", insight.id());
            assert!(
                seen.insert(insight.name()),
                "duplicate name {}",
                insight.name()
            );
            assert_eq!(insight.version().split('.').count(), 3);
        }
        assert_eq!(REFERENCE.len(), 6);
        assert!(seen.contains("I02"));
        assert!(seen.contains("I04"));
        assert!(seen.contains("I18"));
        assert!(seen.contains("model.tokens.direct"));
        assert!(seen.contains("model.cost"));
        assert!(seen.contains("coverage.source"));
        let mut catalog = BTreeSet::new();
        for insight in CATALOG {
            assert!(
                catalog.insert(insight.id()),
                "duplicate catalog id {}",
                insight.id()
            );
            assert!(
                catalog.insert(insight.name()),
                "duplicate catalog name {}",
                insight.name()
            );
        }
        assert_eq!(CATALOG.len(), 25);
        assert_eq!(
            CATALOG
                .iter()
                .map(|insight| insight.id())
                .collect::<Vec<_>>(),
            VNEXT_REFERENCE.iter().map(|row| row.0).collect::<Vec<_>>()
        );
        assert!(catalog.contains("I01"));
        assert!(catalog.contains("I21"));
        assert!(catalog.contains("I25"));
        assert!(catalog.contains("model.invocations"));
        assert!(catalog.contains("model.counterfactual_routing"));
    }

    #[test]
    fn registered_questions_are_admitted_and_others_stay_refused() {
        for admitted in [
            "I02",
            "I04",
            "I18",
            "model.tokens.direct",
            "model.cost",
            "coverage.source",
        ] {
            let id = Identifier::new(admitted).unwrap();
            assert_eq!(check_question_admission(&id), Ok(()));
        }
        let refused = Identifier::new("I99").unwrap();
        assert_eq!(
            check_question_admission(&refused),
            Err(InsightRefusal::RegistryNotAdmitted)
        );
        let measure = Identifier::new("I02").unwrap();
        assert_eq!(
            check_measure_admission(&measure),
            Err(InsightRefusal::RegistryNotAdmitted)
        );
    }
}
