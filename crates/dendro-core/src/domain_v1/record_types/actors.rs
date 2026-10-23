use super::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Participant {
    pub id: ParticipantId,
    pub native_actor: NativeKey,
    pub incarnation: IncarnationId,
    pub safe_label: Option<Identifier>,
    pub kind: ParticipantKind,
    pub durable_identity_binding: Option<BindingId>,
    pub lifetime: Option<Interval>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Conversation {
    pub id: ConversationId,
    pub native_session: NativeKey,
    pub namespace: Identifier,
    pub participants: BTreeSet<ParticipantId>,
    pub start: Option<Boundary>,
    pub end: Option<Boundary>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivitySegment {
    pub id: SegmentId,
    pub interval: Interval,
    pub evidence: Vec<EvidenceId>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionAttempt {
    pub id: AttemptId,
    pub participant: ParticipantId,
    pub participant_incarnation: IncarnationId,
    pub conversation: Option<ConversationId>,
    pub work: Option<WorkId>,
    pub start: Option<Boundary>,
    pub end: Option<Boundary>,
    pub result: AttemptResult,
    pub termination_evidence: Vec<EvidenceId>,
    pub activity: Vec<ActivitySegment>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum BindingValue {
    Model {
        provider: Identifier,
        model: Identifier,
    },
    Effort(Identifier),
    Harness {
        product: Identifier,
        version: Option<Identifier>,
    },
    Context {
        used: Option<Quantity>,
        limit: Option<Quantity>,
    },
    DesiredConfig {
        config: Identifier,
        digest: [u8; 32],
    },
    ObservedConfig {
        config: Identifier,
        digest: [u8; 32],
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub id: BindingId,
    pub subject: Subject,
    pub kind: BindingKind,
    pub value: BindingValue,
    pub scope: ProductionScope,
    pub validity: Option<Interval>,
    pub evidence: Vec<EvidenceId>,
}
impl Validate for Binding {
    fn validate(&self) -> Result<(), Refusal> {
        let kind = match &self.value {
            BindingValue::Model { .. } => BindingKind::Model,
            BindingValue::Effort(_) => BindingKind::Effort,
            BindingValue::Harness { .. } => BindingKind::Harness,
            BindingValue::Context { used, limit } => {
                if let Some(v) = used {
                    v.validate()?;
                }
                if let Some(v) = limit {
                    v.validate()?;
                }
                if let (Some(a), Some(b)) = (used, limit) {
                    if a.unit != b.unit {
                        return Err(Refusal::InvalidQuantity);
                    }
                }
                BindingKind::Context
            }
            BindingValue::DesiredConfig { .. } => BindingKind::DesiredConfig,
            BindingValue::ObservedConfig { .. } => BindingKind::ObservedConfig,
        };
        if kind != self.kind {
            return Err(Refusal::InvalidReference);
        }
        require_evidence(&self.evidence)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageObservation {
    pub id: MessageId,
    pub conversation: ConversationId,
    pub invocation: Option<InvocationId>,
    pub attempt: Option<AttemptId>,
    pub direction: Direction,
    pub role: MessageRole,
    pub modalities: BTreeSet<Modality>,
    pub source_order: Option<u64>,
    pub boundary: Boundary,
    pub counts: Vec<Quantity>,
    pub grain: Grain,
}
/// A complete source-local prefix is evidence, not inferred from the earliest captured row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SequenceOrigin {
    pub conversation: ConversationId,
    pub source: SourceInstanceId,
    pub generation: u64,
    pub first_sequence: u64,
    pub evidence: Vec<EvidenceId>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SequencePrefix {
    pub conversation: ConversationId,
    pub source: SourceInstanceId,
    pub generation: u64,
    pub first_sequence: u64,
    pub through_sequence: u64,
    pub complete: bool,
    /// Producer-backed attestation of the conversation's actual sequence origin.
    /// Complete capture of a mid-conversation window is insufficient.
    pub origin: Option<SequenceOrigin>,
    pub evidence: Vec<EvidenceId>,
}
pub fn first_user_message<'a>(
    messages: &'a [Record<MessageObservation>],
    prefix: &SequencePrefix,
) -> Result<Option<&'a Record<MessageObservation>>, Refusal> {
    require_evidence(&prefix.evidence)?;
    let origin = prefix.origin.as_ref().ok_or(Refusal::MissingEvidence)?;
    require_evidence(&origin.evidence)?;
    if origin.conversation != prefix.conversation
        || origin.source != prefix.source
        || origin.generation != prefix.generation
        || origin.first_sequence != prefix.first_sequence
    {
        return Err(Refusal::InvalidReference);
    }
    if !prefix.complete || prefix.through_sequence < prefix.first_sequence {
        return Err(Refusal::MissingEvidence);
    }
    let mut ordered = std::collections::BTreeMap::new();
    for m in messages {
        m.validate()?;
        if m.value.conversation != prefix.conversation
            || m.provenance.source_instance_id != prefix.source
            || m.provenance.source_generation.number != prefix.generation
        {
            continue;
        }
        if let Some(n) = m.value.source_order {
            if n >= prefix.first_sequence
                && n <= prefix.through_sequence
                && ordered.insert(n, m).is_some()
            {
                return Err(Refusal::Conflict);
            }
        }
    }
    let expected = prefix
        .through_sequence
        .checked_sub(prefix.first_sequence)
        .and_then(|n| n.checked_add(1))
        .ok_or(Refusal::Overflow)?;
    if ordered.len() as u64 != expected {
        return Err(Refusal::MissingEvidence);
    }
    for m in ordered.values() {
        if m.value.grain != Grain::Message {
            return Err(Refusal::MissingEvidence);
        }
        if m.value.role == MessageRole::Unknown {
            return Err(Refusal::MissingEvidence);
        }
        if m.value.role == MessageRole::User {
            return Ok(Some(*m));
        }
    }
    Ok(None)
}
impl Validate for MessageObservation {
    fn validate(&self) -> Result<(), Refusal> {
        self.boundary.validate()?;
        if !matches!(
            self.grain,
            Grain::Message | Grain::Fragment | Grain::Aggregate
        ) || self.modalities.is_empty()
        {
            return Err(Refusal::InvalidQuantity);
        }
        for q in &self.counts {
            q.validate()?;
            if !matches!(
                q.unit,
                Unit::Tokens | Unit::Bytes | Unit::Characters | Unit::Messages | Unit::Fragments
            ) {
                return Err(Refusal::InvalidQuantity);
            }
        }
        Ok(())
    }
}
impl Validate for Participant {
    fn validate(&self) -> Result<(), Refusal> {
        Ok(())
    }
}
impl Validate for Conversation {
    fn validate(&self) -> Result<(), Refusal> {
        validate_bounds(&self.start, &self.end)
    }
}
impl Validate for ActivitySegment {
    fn validate(&self) -> Result<(), Refusal> {
        require_evidence(&self.evidence)
    }
}
impl Validate for ExecutionAttempt {
    fn validate(&self) -> Result<(), Refusal> {
        validate_bounds(&self.start, &self.end)?;
        if self.result != AttemptResult::Unknown {
            require_evidence(&self.termination_evidence)?;
            if self.end.is_none() {
                return Err(Refusal::MissingEvidence);
            }
        }
        let mut ids = BTreeSet::new();
        for segment in &self.activity {
            segment.validate()?;
            if !ids.insert(&segment.id) {
                return Err(Refusal::Conflict);
            }
        }
        Ok(())
    }
}
