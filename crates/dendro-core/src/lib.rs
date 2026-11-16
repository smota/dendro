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
pub mod git_evidence;
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
pub use git_evidence::{
    classify_shell_command, classify_shell_subcommands, classify_single_git_command,
    derive_export_commit_hmac, derive_export_git_evidence_key, derive_export_session_key,
    derive_export_source_id, derive_fact_export_pseudonym, is_abbreviated_commit_sha,
    is_canonical_commit_sha, is_shell_execution_tool_for_harness, is_shell_execution_tool_name,
    is_valid_key_id, normalize_cwd, parse_git_output, resolve_git_evidence_across_sources,
    resolve_git_evidence_across_sources_with_key_id, seal_commit, ClassifiedCommandCall,
    ClassifiedGitSubcommand, FactExportConfig, FactExportKey, FactExportKeyError,
    FactExportKeyState, GitCommandKind, GitEvidenceCoverage, GitEvidenceFact, GitEvidenceScanner,
    GitObjectFormat, ParsedCommitObservation, ParsedGitOutput, DEFAULT_BRANCH_STATUS_NOT_OBSERVED,
    DEFAULT_BRANCH_STATUS_OBSERVED, DEFAULT_BRANCH_STATUS_UNKNOWN,
    DEFAULT_FACT_EXPORT_KEY_FILENAME, ENV_ALT_FACT_EXPORT_KEY_FILE, ENV_ALT_FACT_EXPORT_KEY_HEX,
    ENV_ALT_FACT_EXPORT_KEY_ID, ENV_FACT_EXPORT_KEY_FILE, ENV_FACT_EXPORT_KEY_HEX,
    ENV_FACT_EXPORT_KEY_ID, EVIDENCE_SOURCE_LOCAL_GIT_HISTORY, EXPORT_STATUS_EXPORTABLE,
    EXPORT_STATUS_NON_EXPORTABLE, FACT_EXPORT_COMMIT_DOMAIN, FACT_EXPORT_GIT_EVIDENCE_DOMAIN,
    FACT_EXPORT_KEY_MIN_BYTES, FACT_EXPORT_K_ORG_REFUSAL_WARNING, FACT_EXPORT_SESSION_DOMAIN,
    FACT_EXPORT_SOURCE_DOMAIN, MAX_GIT_COMMAND_BYTES, MAX_GIT_LINES_PER_RECORD,
    MAX_GIT_OUTPUT_BYTES, MAX_OBSERVED_COMMITS_PER_SOURCE, MAX_PARENTS_PER_COMMIT,
    MAX_PENDING_CALLS_PER_SOURCE, MAX_SHELL_WRAPPER_DEPTH, MERGE_REASON_ABBREVIATED_PARENTS,
    MERGE_REASON_CONFLICTING_PARENTS, MERGE_REASON_MIXED_PARENT_OBJECT_FORMAT,
    MERGE_REASON_PARENTS_NOT_RECORDED, MERGE_STATUS_NOT_OBSERVED, MERGE_STATUS_OBSERVED,
    MERGE_STATUS_UNKNOWN, OBSERVED_AT_STATUS_NOT_OBSERVABLE, OBSERVED_AT_STATUS_OBSERVED,
    SESSION_MATCH_AMBIGUOUS, SESSION_MATCH_MATCHED, SESSION_MATCH_UNKNOWN, SESSION_MATCH_UNMATCHED,
};
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
