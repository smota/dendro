//! Compiled reference insights. Add one module and one `REFERENCE` line.
mod i02_tokens;
mod i04_cost;
mod i08_action_failure;
mod i09_lineage_shape;
mod i14_resources;
mod i18_coverage;

use super::contract::Insight;
use super::InsightRefusal;
use crate::domain_v1::Identifier;

pub static REFERENCE: &[&dyn Insight] = &[
    &i02_tokens::INSIGHT as &dyn Insight,
    &i04_cost::INSIGHT as &dyn Insight,
    &i08_action_failure::INSIGHT as &dyn Insight,
    &i09_lineage_shape::INSIGHT as &dyn Insight,
    &i14_resources::INSIGHT as &dyn Insight,
    &i18_coverage::INSIGHT as &dyn Insight,
];

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
        assert!(seen.contains("I02"));
        assert!(seen.contains("I04"));
        assert!(seen.contains("I18"));
        assert!(seen.contains("model.tokens.direct"));
        assert!(seen.contains("model.cost"));
        assert!(seen.contains("coverage.source"));
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
