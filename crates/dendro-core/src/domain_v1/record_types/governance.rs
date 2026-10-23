use super::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecurityEvidence {
    pub id: SecurityId,
    pub layer: SecurityLayer,
    pub subject: Subject,
    pub scope: ProductionScope,
    pub operation: Operation,
    pub resource: ResourceClass,
    pub validity: Option<Interval>,
    pub rules: Vec<RuleRef>,
    pub authority_issuer: Option<ParticipantId>,
    pub effect_locus: Option<PlacementId>,
    pub evidence: Vec<EvidenceId>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum OutcomeTarget {
    Work(WorkId),
    Attempt(AttemptId),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutcomeEvidence {
    pub id: OutcomeId,
    pub target: OutcomeTarget,
    pub transition: OutcomeTransition,
    pub issuer: ParticipantId,
    pub boundary: Boundary,
    pub rule: Option<RuleRef>,
    pub predecessor: Option<OutcomeId>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Beneficiary {
    pub assignment: AssignmentId,
    pub weight: Ratio,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Allocation {
    pub id: AllocationId,
    pub observations: BTreeSet<UsageId>,
    pub beneficiaries: Vec<Beneficiary>,
    pub method: RuleRef,
    pub authority: Identifier,
    pub validity: Option<Interval>,
    pub evidence: Vec<EvidenceId>,
}
impl Allocation {
    pub fn residual(&self) -> Result<Ratio, Refusal> {
        let mut seen = BTreeSet::new();
        let mut total = Ratio::ZERO;
        for b in &self.beneficiaries {
            if !seen.insert(&b.assignment) || !b.weight.is_weight() {
                return Err(Refusal::InvalidQuantity);
            }
            total = total.checked_add(b.weight)?;
        }
        total.remainder()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Coverage {
    pub id: CoverageId,
    pub source: SourceInstanceId,
    pub schema: SchemaIdentity,
    pub record_kind: Identifier,
    pub window: Option<Interval>,
    pub discovery: DiscoveryStatus,
    pub read: ReadStatus,
    pub projection: ProjectionStatus,
    pub accepted: Count,
    pub rejected: Count,
    pub recognized_unprojected: Count,
    pub expected_population: Option<Count>,
    pub denominator_basis: DenominatorBasis,
    pub gaps: BTreeSet<GapReason>,
}
impl Coverage {
    pub fn encountered(&self) -> Result<Count, Refusal> {
        self.accepted
            .checked_add(self.rejected)?
            .checked_add(self.recognized_unprojected)
    }
}

impl Validate for SecurityEvidence {
    fn validate(&self) -> Result<(), Refusal> {
        require_evidence(&self.evidence)
    }
}
impl Validate for OutcomeEvidence {
    fn validate(&self) -> Result<(), Refusal> {
        self.boundary.validate()?;
        if self.predecessor.as_ref() == Some(&self.id) {
            return Err(Refusal::InvalidReference);
        }
        Ok(())
    }
}
impl Validate for Allocation {
    fn validate(&self) -> Result<(), Refusal> {
        require_evidence(&self.evidence)?;
        if self.observations.is_empty() {
            return Err(Refusal::MissingEvidence);
        }
        self.residual().map(|_| ())
    }
}
impl Validate for Coverage {
    fn validate(&self) -> Result<(), Refusal> {
        self.encountered()?;
        if self.denominator_basis == DenominatorBasis::ExpectedPopulation
            && self.expected_population.is_none()
        {
            return Err(Refusal::MissingEvidence);
        }
        Ok(())
    }
}
