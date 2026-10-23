use super::Refusal;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

/// An allowlisted identifier, never arbitrary prose, command text or a path.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Identifier(String);
impl Identifier {
    pub fn new(value: impl Into<String>) -> Result<Self, Refusal> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 128
            || !value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-:@".contains(&b))
            || ["sk-", "ghp_", "gho_", "xai-"].iter().any(|p| {
                value
                    .match_indices(p)
                    .any(|(at, _)| at == 0 || !value.as_bytes()[at - 1].is_ascii_alphanumeric())
            })
        {
            return Err(Refusal::InvalidIdentifier);
        }
        Ok(Self(value))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl TryFrom<String> for Identifier {
    type Error = Refusal;
    fn try_from(v: String) -> Result<Self, Self::Error> {
        Self::new(v)
    }
}
impl From<Identifier> for String {
    fn from(v: Identifier) -> Self {
        v.0
    }
}

/// Semantic version when supplied by a capability producer. Absence is represented
/// by `Option`, never an inferred production version.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SemanticVersion(String);
impl TryFrom<String> for SemanticVersion {
    type Error = Refusal;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.len() > 128 {
            return Err(Refusal::InvalidIdentifier);
        }
        let (version, build) = value
            .split_once('+')
            .map_or((value.as_str(), None), |(v, b)| (v, Some(b)));
        let (core, pre) = version
            .split_once('-')
            .map_or((version, None), |(v, p)| (v, Some(p)));
        let valid_identifiers = |s: &str, prerelease: bool| {
            s.split('.').all(|p| {
                !p.is_empty()
                    && p.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                    && !(prerelease
                        && p.len() > 1
                        && p.starts_with('0')
                        && p.bytes().all(|b| b.is_ascii_digit()))
            })
        };
        let pieces: Vec<_> = core.split('.').collect();
        if pieces.len() != 3
            || !pieces.iter().all(|p| {
                !p.is_empty()
                    && p.bytes().all(|b| b.is_ascii_digit())
                    && (p.len() == 1 || !p.starts_with('0'))
            })
            || build.is_some_and(|b| !valid_identifiers(b, false))
            || pre.is_some_and(|p| !valid_identifiers(p, true))
        {
            return Err(Refusal::InvalidIdentifier);
        }
        Ok(Self(value))
    }
}
impl From<SemanticVersion> for String {
    fn from(v: SemanticVersion) -> Self {
        v.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentityAlgorithm {
    HmacSha256,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdentityDigest {
    pub algorithm: IdentityAlgorithm,
    pub key_id: Identifier,
    pub digest: [u8; 32],
}

/// Caller owns private key storage. No key bytes or native tuple values are retained.
/// Length prefixes include the contract and domain as well as every tuple field.
pub struct IdentityFactory {
    mac: Hmac<Sha256>,
    key_id: Identifier,
}
impl IdentityFactory {
    pub fn new(key: &[u8], key_id: Identifier) -> Result<Self, Refusal> {
        if key.len() < 32 {
            return Err(Refusal::InvalidIdentifier);
        }
        let mac = Hmac::<Sha256>::new_from_slice(key).map_err(|_| Refusal::InvalidIdentifier)?;
        Ok(Self { mac, key_id })
    }
    fn derive(&self, domain: &str, fields: &[&[u8]]) -> IdentityDigest {
        let mut mac = self.mac.clone();
        for field in [b"dendro.domain/1.0.0".as_slice(), domain.as_bytes()]
            .into_iter()
            .chain(fields.iter().copied())
        {
            mac.update(&(field.len() as u64).to_be_bytes());
            mac.update(field);
        }
        IdentityDigest {
            algorithm: IdentityAlgorithm::HmacSha256,
            key_id: self.key_id.clone(),
            digest: mac.finalize().into_bytes().into(),
        }
    }
}

macro_rules! ids {
    ($($name:ident => $domain:literal),+ $(,)?) => {$ (
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(IdentityDigest);
        impl $name {
            pub fn derive(factory: &IdentityFactory, fields: &[&[u8]]) -> Self { Self(factory.derive($domain, fields)) }
            pub fn identity(&self) -> &IdentityDigest { &self.0 }
        }
    )+};
}
ids! {
    FactId => "fact", SourceInstanceId => "source_instance", SourceKey => "source_key",
    ParticipantId => "participant", ConversationId => "conversation", AttemptId => "attempt",
    BindingId => "binding", MessageId => "message", InvocationId => "invocation",
    UsageId => "usage", WorkId => "work", AssignmentId => "assignment", EdgeId => "edge",
    CapabilityId => "capability", CapabilityStateId => "capability_state", ActionId => "action",
    EnvironmentId => "environment", PlacementId => "placement", EnvironmentObservationId => "environment_observation",
    IsolationId => "isolation", SecurityId => "security", OutcomeId => "outcome",
    AllocationId => "allocation", CoverageId => "coverage", EvidenceId => "evidence",
    ClockId => "clock", NativeKey => "native_key", LocatorToken => "locator",
    IncarnationId => "incarnation", CounterId => "counter", SegmentId => "segment",
    FingerprintId => "fingerprint"
}

impl SourceInstanceId {
    pub fn scoped(
        factory: &IdentityFactory,
        authority: &Identifier,
        product: &Identifier,
        incarnation: &IncarnationId,
    ) -> Self {
        Self::derive(
            factory,
            &[
                authority.as_str().as_bytes(),
                product.as_str().as_bytes(),
                incarnation.identity().key_id.as_str().as_bytes(),
                &incarnation.identity().digest,
            ],
        )
    }
}
impl NativeKey {
    pub fn scoped(
        factory: &IdentityFactory,
        source: &SourceInstanceId,
        generation: u64,
        native: &[u8],
    ) -> Self {
        Self::derive(
            factory,
            &[
                source.identity().key_id.as_str().as_bytes(),
                &source.identity().digest,
                &generation.to_be_bytes(),
                native,
            ],
        )
    }
}
impl SourceKey {
    pub fn scoped(
        factory: &IdentityFactory,
        source: &SourceInstanceId,
        generation: u64,
        native: &[u8],
    ) -> Self {
        Self::derive(
            factory,
            &[
                source.identity().key_id.as_str().as_bytes(),
                &source.identity().digest,
                &generation.to_be_bytes(),
                native,
            ],
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum Subject {
    Participant(ParticipantId),
    Conversation(ConversationId),
    Attempt(AttemptId),
    Invocation(InvocationId),
    Work(WorkId),
    Assignment(AssignmentId),
    Environment(EnvironmentId),
    Action(ActionId),
    Capability(CapabilityId),
}
