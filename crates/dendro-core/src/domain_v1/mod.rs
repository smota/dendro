//! Platform-neutral, metadata-only domain contract 1.0.0.
//!
//! This additive API neither changes schema 3 nor asserts persistence, capture,
//! accounting or policy-evaluation support. Unknown wire enum values are errors;
//! importers must count/quarantine them rather than map them to success.
//!
//! Admission layers:
//! * `Record::new` and typed deserialization check structural invariants.
//!   `Record::validate_source` additionally checks the keyed source namespace;
//!   a producer-supplied trust label never verifies a signature by itself.
//! * `RevisionLedger` is an in-memory append-only revision selector, not storage.
//!   Select by knowledge cutoff before applying any event-window predicate.
//!   A snapshot separates valid chains from explicit conflict contents; receipt
//!   timestamps cannot redate an already admitted identical revision.
//! * `WorkGraph` consumes already selected revisions. Its read-only issue report
//!   preserves asserted edges. Inclusive traversal follows assignment-bound
//!   spawn/delegation only, and refuses unresolved/conflicting/cyclic folds.
//!   Independent direct observations remain queryable on such a refusal.
//!   Observation selections keep valid IDs alongside ambiguous IDs and reasons.
//! * Semantic digests canonicalize declared unordered collections only. Ordered
//!   histories remain ordered. Source monetary scale is separately retained and
//!   excluded from canonical equality/digests, as are observation receipts.
//! * Source readers own rejection counters, sequence-prefix attestations and
//!   trust verification. These types do not establish native capture admission.
#![deny(unsafe_code)]

mod canonical;
mod identity;
mod quantity;
mod records;
mod revision;
mod work;

pub use canonical::*;
pub use identity::*;
pub use quantity::*;
pub use records::*;
pub use revision::*;
pub use work::*;

pub const CONTRACT_VERSION: &str = "1.0.0";
pub const RELATIONSHIP_CONTRACT: &str = "relationship-contract-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Refusal {
    InvalidIdentifier,
    InvalidQuantity,
    Overflow,
    InvalidInterval,
    MissingEvidence,
    InvalidReference,
    Conflict,
    Cycle,
    AmbiguousMembership,
    UnknownClock,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "domain_v1::{self:?}")
    }
}
impl std::error::Error for Refusal {}

pub trait Validate {
    fn validate(&self) -> Result<(), Refusal>;
}
