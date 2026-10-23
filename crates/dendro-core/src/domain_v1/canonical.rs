//! Explicit semantic digest projection. No field-name heuristics or blanket
//! array sorting: new fact types must declare which collections are sets.
use super::*;
use serde::Serialize;
use sha2::{Digest, Sha256};

pub trait CanonicalFact: Clone + Serialize {
    /// Normalize a clone for semantic comparison/hashing, preserving the source
    /// record and all meaningful sequence order.
    fn canonicalize(&mut self);
}

fn set<T: Ord>(values: &mut Vec<T>) {
    values.sort();
    values.dedup();
}
fn bounds(start: &mut Option<Boundary>, end: &mut Option<Boundary>) {
    if let Some(b) = start {
        b.canonicalize();
    }
    if let Some(b) = end {
        b.canonicalize();
    }
}
fn activity(segments: &mut [ActivitySegment]) {
    // Activity order is retained. Only each segment's evidence is a set.
    for segment in segments {
        segment.canonicalize();
    }
}
pub fn canonical_value_digest<T: CanonicalFact>(value: &T) -> Result<[u8; 32], Refusal> {
    let mut value = value.clone();
    value.canonicalize();
    let bytes = serde_json::to_vec(&value).map_err(|_| Refusal::InvalidQuantity)?;
    Ok(Sha256::digest(bytes).into())
}
macro_rules! unchanged {
    ($($kind:ty),+ $(,)?) => { $(impl CanonicalFact for $kind { fn canonicalize(&mut self) {} })+ };
}
unchanged!(
    SourceInstance,
    Evidence,
    Participant,
    ActionObservation,
    EnvironmentObservation,
    Coverage,
    Decimal,
    Ratio,
    Quantity
);

impl CanonicalFact for SourceGeneration {
    fn canonicalize(&mut self) {
        set(&mut self.predecessor_evidence);
    }
}
impl CanonicalFact for Provenance {
    fn canonicalize(&mut self) {
        self.observed_at = UtcMicros(0);
        self.ingested_at = UtcMicros(0);
        set(&mut self.evidence_ids);
        self.source_generation.canonicalize();
    }
}
impl CanonicalFact for Boundary {
    fn canonicalize(&mut self) {
        set(&mut self.evidence);
    }
}
impl CanonicalFact for Conversation {
    fn canonicalize(&mut self) {
        bounds(&mut self.start, &mut self.end);
    }
}
impl CanonicalFact for ActivitySegment {
    fn canonicalize(&mut self) {
        set(&mut self.evidence);
    }
}
impl CanonicalFact for ExecutionAttempt {
    fn canonicalize(&mut self) {
        bounds(&mut self.start, &mut self.end);
        set(&mut self.termination_evidence);
        activity(&mut self.activity);
    }
}
impl CanonicalFact for Binding {
    fn canonicalize(&mut self) {
        set(&mut self.evidence);
    }
}
impl CanonicalFact for MessageObservation {
    fn canonicalize(&mut self) {
        self.boundary.canonicalize();
        // Named metadata quantities are an unordered bag, not a sequence.
        // Keep multiplicity; duplicate quantities are not erased.
        self.counts.sort_by_key(|q| (q.unit as u8, q.value.0));
    }
}
impl CanonicalFact for SequenceOrigin {
    fn canonicalize(&mut self) {
        set(&mut self.evidence);
    }
}
impl CanonicalFact for SequencePrefix {
    fn canonicalize(&mut self) {
        set(&mut self.evidence);
        if let Some(origin) = &mut self.origin {
            origin.canonicalize();
        }
    }
}
impl CanonicalFact for ModelInvocation {
    fn canonicalize(&mut self) {
        set(&mut self.call_evidence);
        bounds(&mut self.start, &mut self.end);
    }
}
impl CanonicalFact for Money {
    fn canonicalize(&mut self) {
        self.source_scale = None;
    }
}
impl CanonicalFact for UsageObservation {
    fn canonicalize(&mut self) {
        self.components.sort_by_key(|c| c.component);
        set(&mut self.attribution_evidence);
        set(&mut self.equivalence_evidence);
        if let Some(b) = &mut self.attribution_event {
            b.canonicalize();
        }
        if let Some(m) = &mut self.observed_currency {
            m.canonicalize();
        }
    }
}
impl CanonicalFact for WorkItem {
    fn canonicalize(&mut self) {
        set(&mut self.dependencies);
        set(&mut self.outcomes);
    }
}
impl CanonicalFact for WorkAssignment {
    fn canonicalize(&mut self) {
        set(&mut self.evidence);
        activity(&mut self.activity);
    }
}
impl CanonicalFact for WorkEdge {
    fn canonicalize(&mut self) {
        self.observed_at = UtcMicros(0);
        self.boundary.canonicalize();
        set(&mut self.evidence);
        match &mut self.relation {
            WorkRelation::Spawn {
                child_work_link, ..
            } => set(child_work_link),
            WorkRelation::DelegateExisting { work_relation, .. } => set(work_relation),
            WorkRelation::Handoff {
                transfer_evidence, ..
            } => set(transfer_evidence),
            WorkRelation::Return {
                result_boundary, ..
            } => result_boundary.canonicalize(),
            WorkRelation::Join { receipt, .. } => set(receipt),
            WorkRelation::Resume {
                continuation_evidence,
                ..
            } => set(continuation_evidence),
            WorkRelation::Retry { .. } | WorkRelation::Dependency { .. } => {}
        }
    }
}
impl CanonicalFact for CapabilityDefinition {
    fn canonicalize(&mut self) {
        set(&mut self.contexts);
        set(&mut self.profiles);
        set(&mut self.evidence);
    }
}
impl CanonicalFact for CapabilityState {
    fn canonicalize(&mut self) {
        set(&mut self.structural_evidence);
    }
}
impl CanonicalFact for EnvironmentInstance {
    fn canonicalize(&mut self) {
        set(&mut self.incarnation_evidence);
        bounds(&mut self.start, &mut self.end);
    }
}
impl CanonicalFact for Placement {
    fn canonicalize(&mut self) {
        set(&mut self.evidence);
    }
}
impl CanonicalFact for IsolationEvidence {
    fn canonicalize(&mut self) {
        self.boundary.canonicalize();
    }
}
impl CanonicalFact for SecurityEvidence {
    fn canonicalize(&mut self) {
        set(&mut self.evidence);
        // Rule references are a set. Rule evaluation precedence is not encoded
        // by this list (it belongs to the referenced versioned rule contract).
        self.rules
            .sort_by(|a, b| (&a.id, &a.version).cmp(&(&b.id, &b.version)));
        self.rules.dedup();
    }
}
impl CanonicalFact for OutcomeEvidence {
    fn canonicalize(&mut self) {
        self.boundary.canonicalize();
    }
}
impl CanonicalFact for Allocation {
    fn canonicalize(&mut self) {
        set(&mut self.evidence);
        self.beneficiaries
            .sort_by(|a, b| a.assignment.cmp(&b.assignment));
    }
}
