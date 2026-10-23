//! T19 owns evidence-linked explanations of admitted immutable answers.
use crate::domain_v1::EvidenceId;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExplanationRefusal {
    MissingEvidence,
    AnswerContractNotAdmitted,
}
pub fn check_explanation_admission(evidence: &[EvidenceId]) -> Result<(), ExplanationRefusal> {
    if evidence.is_empty() {
        return Err(ExplanationRefusal::MissingEvidence);
    }
    Err(ExplanationRefusal::AnswerContractNotAdmitted)
}
