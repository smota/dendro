use super::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityDefinition {
    pub id: CapabilityId,
    pub kind: CapabilityKind,
    pub authority: Identifier,
    pub native_key: NativeKey,
    pub semantic_version: Option<SemanticVersion>,
    pub definition_digest: Option<[u8; 32]>,
    pub safe_identifier: Identifier,
    pub contexts: Vec<BindingId>,
    pub profiles: Vec<CapabilityId>,
    pub evidence: Vec<EvidenceId>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityState {
    pub id: CapabilityStateId,
    pub definition: CapabilityId,
    pub harness_binding: Option<BindingId>,
    pub context_binding: Option<BindingId>,
    pub stage: CapabilityStage,
    pub scope: ProductionScope,
    pub validity: Option<Interval>,
    pub structural_evidence: Vec<EvidenceId>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionObservation {
    pub id: ActionId,
    pub actor: ParticipantId,
    pub attempt: Option<AttemptId>,
    pub capability: Option<CapabilityId>,
    pub applicable_state: Option<CapabilityStateId>,
    pub category: ActionCategory,
    pub status: ActionStatus,
    pub operation: Operation,
    pub fingerprint: Option<FingerprintId>,
    pub tool_server: Option<PlacementId>,
    pub effect: Option<EnvironmentObservationId>,
}
impl Validate for CapabilityDefinition {
    fn validate(&self) -> Result<(), Refusal> {
        require_evidence(&self.evidence)
    }
}
impl Validate for CapabilityState {
    fn validate(&self) -> Result<(), Refusal> {
        require_evidence(&self.structural_evidence)
    }
}
impl Validate for ActionObservation {
    fn validate(&self) -> Result<(), Refusal> {
        if self.applicable_state.is_some() && self.capability.is_none() {
            Err(Refusal::InvalidReference)
        } else {
            Ok(())
        }
    }
}
