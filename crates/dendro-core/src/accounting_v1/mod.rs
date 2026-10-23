//! Exact quantity primitives are available. T14 owns population accounting.
use crate::domain_v1::UsageObservation;
pub use crate::domain_v1::{Decimal, Money, Ratio};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountingRefusal {
    NoObservations,
    AccountingPolicyNotAdmitted,
}
pub fn check_total_admission(observations: &[UsageObservation]) -> Result<(), AccountingRefusal> {
    if observations.is_empty() {
        return Err(AccountingRefusal::NoObservations);
    }
    Err(AccountingRefusal::AccountingPolicyNotAdmitted)
}
