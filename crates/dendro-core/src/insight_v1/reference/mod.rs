//! Reference algorithms are not registered by the structural core builder.
use super::InsightRefusal;
use crate::domain_v1::Identifier;
pub fn check_measure_admission(_measure: &Identifier) -> Result<(), InsightRefusal> {
    Err(InsightRefusal::RegistryNotAdmitted)
}
