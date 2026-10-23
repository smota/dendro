use super::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentInstance {
    pub id: EnvironmentId,
    pub kind: EnvironmentKind,
    pub incarnation: Option<IncarnationId>,
    pub incarnation_evidence: Vec<EvidenceId>,
    pub parent: Option<EnvironmentId>,
    pub start: Option<Boundary>,
    pub end: Option<Boundary>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Placement {
    pub id: PlacementId,
    pub subject: Subject,
    pub environment: EnvironmentId,
    pub locus: Locus,
    pub validity: Option<Interval>,
    pub evidence: Vec<EvidenceId>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "platform", content = "marker", rename_all = "snake_case")]
pub enum CreationMarker {
    LinuxStartTicks(u64),
    WindowsCreationTime(u64),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessIdentity {
    pub environment: EnvironmentId,
    pub pid: u32,
    pub creation_marker: CreationMarker,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentObservation {
    pub id: EnvironmentObservationId,
    pub placement: Option<PlacementId>,
    pub process: Option<ProcessIdentity>,
    pub kind: EnvironmentObservationKind,
    pub quantity: Option<Quantity>,
    pub interval: Option<Interval>,
    pub operation: Option<Operation>,
    pub resource: Option<ResourceClass>,
    pub result: ActionStatus,
    pub capture_method: CaptureMethod,
    pub visibility_limits: BTreeSet<VisibilityLimit>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IsolationEvidence {
    pub id: IsolationId,
    pub environment: EnvironmentId,
    pub placement: Option<PlacementId>,
    pub dimension: IsolationDimension,
    pub stage: ClaimStage,
    pub method: RuleRef,
    pub boundary: Boundary,
}
impl Validate for EnvironmentInstance {
    fn validate(&self) -> Result<(), Refusal> {
        if self.incarnation.is_some() {
            require_evidence(&self.incarnation_evidence)?;
        }
        if self.parent.as_ref() == Some(&self.id) {
            return Err(Refusal::InvalidReference);
        }
        validate_bounds(&self.start, &self.end)
    }
}
impl Validate for Placement {
    fn validate(&self) -> Result<(), Refusal> {
        require_evidence(&self.evidence)
    }
}
impl Validate for EnvironmentObservation {
    fn validate(&self) -> Result<(), Refusal> {
        if let Some(q) = &self.quantity {
            q.validate()?;
        }
        if self.kind == EnvironmentObservationKind::Resource && self.quantity.is_none() {
            return Err(Refusal::MissingEvidence);
        }
        if let Some(q) = &self.quantity {
            let valid = match self.kind {
                EnvironmentObservationKind::Resource => matches!(
                    q.unit,
                    Unit::Bytes
                        | Unit::Microseconds
                        | Unit::CpuUserMicroseconds
                        | Unit::CpuSystemMicroseconds
                        | Unit::LinuxRssBytes
                        | Unit::WindowsWorkingSetBytes
                ),
                EnvironmentObservationKind::Syscall => {
                    matches!(q.unit, Unit::Events | Unit::Count | Unit::Microseconds)
                }
                EnvironmentObservationKind::Process | EnvironmentObservationKind::Metadata => {
                    matches!(q.unit, Unit::Count | Unit::Bytes)
                }
            };
            if !valid {
                return Err(Refusal::InvalidQuantity);
            }
            if let Some(p) = &self.process {
                if matches!(
                    (&p.creation_marker, q.unit),
                    (
                        CreationMarker::LinuxStartTicks(_),
                        Unit::WindowsWorkingSetBytes
                    ) | (CreationMarker::WindowsCreationTime(_), Unit::LinuxRssBytes)
                ) {
                    return Err(Refusal::InvalidQuantity);
                }
            }
        }
        if self.kind == EnvironmentObservationKind::Syscall
            && (self.operation.is_none() || self.resource.is_none())
        {
            return Err(Refusal::MissingEvidence);
        }
        if self.kind == EnvironmentObservationKind::Syscall
            && !matches!(
                self.capture_method,
                CaptureMethod::Strace | CaptureMethod::Etw
            )
        {
            return Err(Refusal::MissingEvidence);
        }
        Ok(())
    }
}
impl Validate for IsolationEvidence {
    fn validate(&self) -> Result<(), Refusal> {
        self.boundary.validate()
    }
}
