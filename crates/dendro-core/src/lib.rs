// The report payload is one large json! literal.
#![recursion_limit = "256"]
#![deny(unsafe_code)]

pub mod accounting_v1;
pub mod advisory;
pub mod compaction;
pub mod concurrency;
pub mod domain_v1;
pub mod engine_v200;
pub mod explanation_v1;
pub mod facts;
pub mod file_metadata;
mod file_state;
pub mod insight_v1;
pub mod lifecycle;
pub mod lineage_graph_v1;
pub mod lineage_v1;
pub mod link_id;
mod loop_trace;
pub mod manifest;
pub mod metrics;
pub mod migration_v4;
pub mod migration_v5;
pub mod models;
pub mod outcome_v1;
pub mod output;
pub mod overview;
pub mod pipeline;
pub mod platform_v1;
pub mod presentation;
pub mod privacy;
pub mod retrieval_v1;
pub mod routing;
pub mod sealed_run;
pub mod security_v1;
#[cfg(any(windows, target_os = "linux"))]
pub mod selection_binding;
pub mod session_lineage;
pub mod spotlight;
pub mod storage;
pub mod tool_category;
pub mod traits;
pub mod tripwire_v1;
pub mod turnaround;
pub mod vortex;
pub use vortex::{parse_friction_duration, FrictionThresholds};

pub use concurrency::*;
pub use facts::*;
pub use lifecycle::{Cancel, Interrupted, RUN_CANCEL};
pub use link_id::{
    consumer_env_path, default_private_data_root, derive_session_link_with_key,
    is_valid_persisted_link_id, read_consumer_env, read_consumer_env_from, CanonicalHarness,
    LinkIdInput, LinkIdOutcome, LinkKey, SessionLinkConfig, UnlinkedReason, AGENT_SESSION_LINK_ID,
    AGENT_SESSION_LINK_UNLINKED, AGENT_SESSION_UNLINKED_REASON, K_ORG_REFUSAL_WARNING,
    LINK_ID_CONTRACT, LINK_ID_CONTRACT_DIGEST, MAX_CONSUMER_ENV_BYTES,
};
pub use manifest::*;
pub use metrics::*;
pub use models::*;
pub use output::*;
pub use overview::*;
pub use pipeline::*;
pub use privacy::*;
pub use sealed_run::*;
pub use storage::*;
pub use tool_category::*;
pub use traits::*;
pub mod portfolio;
pub mod semantic;

pub mod simulation;
