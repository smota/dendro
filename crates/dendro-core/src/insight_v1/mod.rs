//! T15 owns the compiled measure/analyzer registry and answer contract.
pub mod reference;
use crate::domain_v1::Identifier;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsightRefusal {
    RegistryNotAdmitted,
}
pub fn check_question_admission(_question: &Identifier) -> Result<(), InsightRefusal> {
    Err(InsightRefusal::RegistryNotAdmitted)
}
