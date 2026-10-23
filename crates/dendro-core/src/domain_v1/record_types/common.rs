use super::*;
use serde::{Deserialize, Serialize};

macro_rules! enums { ($($name:ident { $($variant:ident),+ $(,)? })+) => { $(
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum $name { $($variant),+ }
)+ }; }
enums! {
    Trust { VerifiedSignature, AuthenticatedTransport, Unverified }
    CaptureMode { Native, Export, Snapshot, Sensor, Unknown }
    ParticipantKind { Agent, Human, Service, Unknown }
    AttemptResult { Succeeded, Failed, Cancelled, Unknown }
    BindingKind { Model, Effort, Harness, Context, DesiredConfig, ObservedConfig }
    Direction { Inbound, Outbound, Internal, Unknown }
    MessageRole { User, Assistant, System, Developer, Tool, Unknown }
    Modality { Text, Image, Audio, Video, Structured, Unknown }
    Grain { Message, Fragment, Aggregate, Invocation, Attempt, Assignment, Event, Sample, Legacy }
    CapabilityKind { Primitive, Command, Skill, Tool, Hook, Profile }
    CapabilityStage { Defined, Available, Loaded }
    ProductionScope { Production, Synthetic, Unknown }
    ActionStatus { Attempted, Succeeded, Failed, Unknown }
    Operation { Read, Write, Execute, Connect, Accept, Create, Delete, Rename, Inspect, Coordinate, Unknown }
    ActionCategory { Tool, Command, Capability, Coordination, Unknown }
    ResourceClass { File, Network, Process, Memory, Identity, Queue, Unknown }
    EnvironmentKind { Host, Vm, Container, ProcessScope, Remote, Unknown }
    Locus { HarnessHost, ToolServer, EffectLocus }
    EnvironmentObservationKind { Metadata, Resource, Process, Syscall }
    CaptureMethod { Export, Procfs, Cgroup, WindowsProcessApi, Strace, Etw, ProducerAssertion, Unknown }
    VisibilityLimit { Unauthorized, PermissionDenied, ProcessExited, LostEvents, Filtered, CounterReset, Truncated, UnknownClock, Unknown }
    IsolationDimension { Namespace, Cgroup, Filesystem, Network, Identity }
    ClaimStage { Declared, ObservedConfiguration, VerifiedEffective }
    SecurityLayer { Policy, Grant, Approval, Attempt, EffectiveAuthority, Effect }
    OutcomeTransition { Completed, Reviewed, Accepted, Rejected, Merged, Revoked, Superseded }
    DiscoveryStatus { Present, Absent, Unauthorized, Unknown }
    ReadStatus { Complete, Partial, Failed, NotAttempted }
    ProjectionStatus { Supported, UnsupportedSchema, RecognizedUnprojected }
    GapReason { MissingSource, UnsupportedSchema, UnknownKind, InvalidTimestamp, InvalidQuantity, InvalidReference, PermissionDenied, Truncated, MissingClock, MissingDenominator, StraddleUnsplit, Conflict, Unknown }
    DenominatorBasis { EncounteredRecords, ExpectedPopulation, Unknown }
    TokenComponent { InputUncached, CacheWrite, CacheRead, Output, Reasoning, SourceReportedTotal }
    CounterSemantics { Delta, Cumulative, Gauge, Unknown }
    AttributionScope { Invocation, Attempt, Assignment, Shared, Unattributed, Legacy }
    PolicyAssessment { NotAssessed, InsufficientEvidence, Consistent, Mismatch }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SchemaIdentity {
    pub id: Identifier,
    pub version: Option<Identifier>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleRef {
    pub id: Identifier,
    pub version: Identifier,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceInstance {
    pub id: SourceInstanceId,
    pub authority: Identifier,
    pub product: Identifier,
    pub producer_version: Option<Identifier>,
    pub producer_incarnation: IncarnationId,
    pub capture_mode: CaptureMode,
    pub schema: SchemaIdentity,
    pub configured_root: Option<LocatorToken>,
    pub trust: Trust,
}
impl SourceInstance {
    /// Verify the namespace/incarnation tuple with the admitted private key.
    /// Structural `Validate` alone cannot authenticate externally supplied IDs.
    pub fn validate_identity(&self, factory: &IdentityFactory) -> Result<(), Refusal> {
        if self.id
            != SourceInstanceId::scoped(
                factory,
                &self.authority,
                &self.product,
                &self.producer_incarnation,
            )
        {
            return Err(Refusal::InvalidIdentifier);
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceGeneration {
    pub number: u64,
    pub predecessor: Option<u64>,
    pub predecessor_evidence: Vec<EvidenceId>,
}
impl Validate for SourceGeneration {
    fn validate(&self) -> Result<(), Refusal> {
        if self.predecessor.is_some_and(|p| p >= self.number)
            || (self.predecessor.is_some() && self.predecessor_evidence.is_empty())
        {
            return Err(Refusal::InvalidReference);
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub id: EvidenceId,
    pub locator: LocatorToken,
    pub producer: Identifier,
    pub producer_version: Option<Identifier>,
    pub artifact_sha256: [u8; 32],
    pub projection_recipe: RuleRef,
    pub trust: Trust,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    pub fact_id: FactId,
    pub source_instance_id: SourceInstanceId,
    pub source_generation: SourceGeneration,
    pub source_key: SourceKey,
    pub source_schema: SchemaIdentity,
    pub parser: SchemaIdentity,
    pub event_time: Option<UtcMicros>,
    pub event_sequence: Option<u64>,
    pub clock_id: Option<ClockId>,
    pub clock_error_us: Option<u64>,
    pub observed_at: UtcMicros,
    pub ingested_at: UtcMicros,
    pub granularity: Grain,
    pub evidence_ids: Vec<EvidenceId>,
    pub revision: u64,
    pub supersedes: Option<FactId>,
    pub retracts: Option<FactId>,
}
impl Validate for Provenance {
    fn validate(&self) -> Result<(), Refusal> {
        self.source_generation.validate()?;
        if self.ingested_at < self.observed_at {
            return Err(Refusal::InvalidInterval);
        }
        if self.evidence_ids.is_empty() {
            return Err(Refusal::MissingEvidence);
        }
        if self.clock_id.is_none() && self.clock_error_us.is_some() {
            return Err(Refusal::UnknownClock);
        }
        if self.supersedes.is_some() && self.retracts.is_some()
            || self.supersedes.as_ref() == Some(&self.fact_id)
            || self.retracts.as_ref() == Some(&self.fact_id)
        {
            return Err(Refusal::InvalidReference);
        }
        Ok(())
    }
}
/// Every emitted fact uses this envelope. Payloads contain no transcript fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Record<T> {
    pub provenance: Provenance,
    pub value: T,
}
impl<T: Validate> Validate for Record<T> {
    fn validate(&self) -> Result<(), Refusal> {
        self.provenance.validate()?;
        self.value.validate()
    }
}
impl<T: Validate> Record<T> {
    pub fn validate_source(
        &self,
        source: &SourceInstance,
        factory: &IdentityFactory,
    ) -> Result<(), Refusal> {
        self.validate()?;
        source.validate_identity(factory)?;
        if self.provenance.source_instance_id != source.id
            || self.provenance.source_schema != source.schema
        {
            return Err(Refusal::InvalidReference);
        }
        Ok(())
    }
    pub fn new(provenance: Provenance, value: T) -> Result<Self, Refusal> {
        let record = Self { provenance, value };
        record.validate()?;
        Ok(record)
    }
}
impl<'de, T: Deserialize<'de> + Validate> Deserialize<'de> for Record<T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire<T> {
            provenance: Provenance,
            value: T,
        }
        let wire = Wire::<T>::deserialize(deserializer)?;
        Self::new(wire.provenance, wire.value).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceOrder {
    pub source_instance: SourceInstanceId,
    pub generation: u64,
    pub sequence: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Boundary {
    pub time: Option<UtcMicros>,
    pub clock: Option<ClockId>,
    pub error_us: Option<u64>,
    pub order: Option<SourceOrder>,
    pub evidence: Vec<EvidenceId>,
}
impl Validate for Boundary {
    fn validate(&self) -> Result<(), Refusal> {
        if self.evidence.is_empty() {
            return Err(Refusal::MissingEvidence);
        }
        if self.clock.is_none() && self.error_us.is_some() {
            return Err(Refusal::UnknownClock);
        }
        Ok(())
    }
}
/// Source-local sequence or a shared clock with disjoint uncertainty bounds can
/// establish order. Conflicting projections remain a conflict.
pub fn boundary_no_later(a: &Boundary, b: &Boundary) -> Result<Option<bool>, Refusal> {
    a.validate()?;
    b.validate()?;
    let local_order = match (&a.order, &b.order) {
        (Some(a), Some(b))
            if a.source_instance == b.source_instance && a.generation == b.generation =>
        {
            Some(a.sequence <= b.sequence)
        }
        _ => None,
    };
    let clock_order = match (a.time, b.time, &a.clock, &b.clock, a.error_us, b.error_us) {
        (Some(a_time), Some(b_time), Some(ac), Some(bc), Some(ae), Some(be)) if ac == bc => {
            let latest_a = i128::from(a_time.0) + i128::from(ae);
            let earliest_b = i128::from(b_time.0) - i128::from(be);
            let earliest_a = i128::from(a_time.0) - i128::from(ae);
            let latest_b = i128::from(b_time.0) + i128::from(be);
            if latest_a <= earliest_b {
                Ok(Some(true))
            } else if earliest_a > latest_b {
                Ok(Some(false))
            } else {
                Ok(None)
            }
        }
        _ => Ok(None),
    }?;
    match (local_order, clock_order) {
        (Some(a), Some(b)) if a != b => Err(Refusal::Conflict),
        (Some(order), _) | (_, Some(order)) => Ok(Some(order)),
        _ => Ok(None),
    }
}

pub(crate) fn require_evidence(ids: &[EvidenceId]) -> Result<(), Refusal> {
    if ids.is_empty() {
        Err(Refusal::MissingEvidence)
    } else {
        Ok(())
    }
}
pub(crate) fn validate_bounds(
    start: &Option<Boundary>,
    end: &Option<Boundary>,
) -> Result<(), Refusal> {
    if let Some(a) = start {
        a.validate()?;
    }
    if let Some(b) = end {
        b.validate()?;
    }
    if let (Some(a), Some(b)) = (start, end) {
        if a.clock.is_some() && a.clock == b.clock && a.time.zip(b.time).is_some_and(|(a, b)| a > b)
        {
            return Err(Refusal::InvalidInterval);
        }
        if boundary_no_later(a, b)? == Some(false) {
            return Err(Refusal::InvalidInterval);
        }
    }
    Ok(())
}
impl Validate for SourceInstance {
    fn validate(&self) -> Result<(), Refusal> {
        Ok(())
    }
}
impl Validate for Evidence {
    fn validate(&self) -> Result<(), Refusal> {
        Ok(())
    }
}
