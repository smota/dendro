//! Outcome records are structural evidence; T13 supplies rule evaluation.
use crate::domain_v1::{OutcomeEvidence, RuleRef};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomeRefusal {
    NoApplicableRule,
    EvaluatorNotAdmitted,
}
pub fn check_evaluation_admission(
    rule: Option<&RuleRef>,
    _evidence: &[OutcomeEvidence],
) -> Result<(), OutcomeRefusal> {
    if rule.is_none() {
        return Err(OutcomeRefusal::NoApplicableRule);
    }
    Err(OutcomeRefusal::EvaluatorNotAdmitted)
}
