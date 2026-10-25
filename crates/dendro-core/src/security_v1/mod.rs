//! No grant, tool success or lineage implies effective authority.
//! `SecurityFact` rows are observations of source fields and are not evidence here.
use crate::domain_v1::{PolicyAssessment, SecurityEvidence, SecurityLayer};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityRefusal {
    ComparatorNotAdmitted,
}
pub fn assess_authority(
    evidence: &[SecurityEvidence],
) -> Result<PolicyAssessment, SecurityRefusal> {
    if !evidence.iter().any(|e| e.layer == SecurityLayer::Policy) {
        return Ok(PolicyAssessment::NotAssessed);
    }
    Err(SecurityRefusal::ComparatorNotAdmitted)
}
