use super::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelInvocation {
    pub id: InvocationId,
    pub provider: Identifier,
    pub native_call: NativeKey,
    pub attempt: Option<AttemptId>,
    pub model_binding: Option<BindingId>,
    pub start: Option<Boundary>,
    pub end: Option<Boundary>,
    pub status: AttemptResult,
    pub call_evidence: Vec<EvidenceId>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "component", rename_all = "snake_case")]
pub enum Inclusion {
    Exclusive,
    IncludedIn(TokenComponent),
    Unknown,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageComponent {
    pub component: TokenComponent,
    pub quantity: Count,
    pub inclusion: Inclusion,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CounterEvidence {
    pub semantics: CounterSemantics,
    pub identity: Option<CounterId>,
    pub initialized_since_zero: Option<EvidenceId>,
    pub reset: Option<EvidenceId>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageObservation {
    pub id: UsageId,
    pub invocation: Option<InvocationId>,
    pub attempt: Option<AttemptId>,
    pub components: Vec<UsageComponent>,
    pub observed_currency: Option<Money>,
    pub attribution: AttributionScope,
    pub assignment: Option<AssignmentId>,
    pub attribution_evidence: Vec<EvidenceId>,
    pub attribution_event: Option<Boundary>,
    pub interval: Option<Interval>,
    pub counter: CounterEvidence,
    pub equivalence_evidence: Vec<EvidenceId>,
}
impl Validate for ModelInvocation {
    fn validate(&self) -> Result<(), Refusal> {
        require_evidence(&self.call_evidence)?;
        validate_bounds(&self.start, &self.end)
    }
}
impl Validate for UsageObservation {
    fn validate(&self) -> Result<(), Refusal> {
        let mut seen = BTreeSet::new();
        for c in &self.components {
            if !seen.insert(c.component) || c.inclusion == Inclusion::IncludedIn(c.component) {
                return Err(Refusal::InvalidQuantity);
            }
        }
        if self.assignment.is_some() {
            require_evidence(&self.attribution_evidence)?;
        }
        if self.assignment.is_some() != (self.attribution == AttributionScope::Assignment) {
            return Err(Refusal::InvalidReference);
        }
        if let Some(money) = &self.observed_currency {
            money.validate()?;
        }
        if self.attribution == AttributionScope::Invocation && self.invocation.is_none()
            || self.attribution == AttributionScope::Attempt && self.attempt.is_none()
        {
            return Err(Refusal::MissingEvidence);
        }
        if let Some(b) = &self.attribution_event {
            b.validate()?;
        }
        if self.counter.semantics == CounterSemantics::Cumulative && self.counter.identity.is_none()
        {
            return Err(Refusal::MissingEvidence);
        }
        Ok(())
    }
}
