//! Local git merge evidence extraction and `fact-export-v1` commit pseudonymization (`D4c`).
//!
//! This module implements the local evidence extractor and storage model for `git_evidence` records:
//! - Extracts commit observations and parent commit lists strictly from an exact-match allow-list
//!   of recognized git command templates and output shapes in session transcripts
//!   (no repository access, no `git` subprocess spawning, no network calls, no shell parser).
//! - Rejects any command containing a character outside `[A-Za-z0-9 =%_.:/'"-]` before template
//!   matching; any rejected git-looking command increments `unparsed_compound_git_calls`.
//! - Matches tool outputs to tool calls strictly by explicit call ID using a bounded LRU map;
//!   unmatched tool outputs and non-shell tools (`Agent`, `MCP`, `Read`, etc.) are ignored.
//! - Uses only the record's own `cwd` field (no `cd` handling, no session `cwd` fallback).
//!   Evidence that requires a `cwd` (`git rev-parse --show-object-format` binding) requires the
//!   record `cwd` to be present.
//! - Enforces hash algorithm verification per `(session, record_cwd)`: 40-hex SHA-1 is accepted
//!   unless `git rev-parse --show-object-format` recorded `sha256` for that same session and
//!   record `cwd`; 64-hex SHA-256 is accepted only when `sha256` was recorded for that same
//!   session and record `cwd` (otherwise counted in `unverified_object_format` and ignored).
//! - Enforces parent length consistency (`mixed_parent_object_format`) and intra-source /
//!   cross-source parent agreement (`conflicting_parents`).
//! - Accepts `git cat-file -p <FULL-SHA>` parents ONLY when `<FULL-SHA>` has already been observed
//!   as a commit in the same source (via the `git rev-parse HEAD`, `git log`, or `git show`
//!   templates) at an earlier or the same record position; otherwise the `cat-file` output
//!   contributes nothing and increments `cat_file_unconfirmed` (preventing a crafted blob from
//!   forging a commit or merge).
//! - Shallow-clone behavior: at a shallow boundary (`.git/shallow` / `--depth`), `git cat-file -p`
//!   reads the raw commit object and prints real `parent` lines while `git log`/`git show` `%P`
//!   prints none; running both in one source yields `conflicting_parents` (safe), while `git log`
//!   alone may record `is_merge = false` at a shallow boundary.
//! - In the MVP, `default_branch_status` is always `not_observed` (`on_default_branch = None`),
//!   using a status-only representation consistent with the `fact-export-v1` contract's fields
//!   (which carry no default-branch reason column; default-branch membership is not derivable from
//!   session records). Downstream `EN6b` (Enterprise merged-PR proxy) must treat the default
//!   branch as unknown.
//! - Seals local commit SHAs (`hmac_domain(tool_key, "git_commit", sha)`) and session identities
//!   before persistence so raw SHAs, branch names, commit messages, author identities, file paths,
//!   and remote URLs are never written to DuckDB.
//! - Reuses `D3`'s secret-key loader in `crate::link_id` (`platform_v1` private data root
//!   `fact_export.key`, lenient `consumer.env`, case-insensitive `K_org` refusal, no `USERPROFILE`
//!   fallback, constant-time key equality, no `Clone`).

use crate::link_id::{
    collect_process_env_vars, consumer_env_path, default_private_data_root, hmac_sha256,
    load_consumer_env_vars, load_secret_key_from_sources, lp_encode_str, pick_var_with_fallback,
    zeroize_env_map, CanonicalHarness, SecretKeyLoadOutcome, SecretKeyLoaderSpec, LINK_KEY_BYTES,
};
use chrono::{DateTime, Utc};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use std::path::Path;

/// Normative `fact-export-v1` HMAC domain for source pseudonyms (used by D4b at export time).
pub const FACT_EXPORT_SOURCE_DOMAIN: &str = "fact-export.v1/source";
/// Normative `fact-export-v1` HMAC domain for session pseudonyms (used by D4b at export time).
pub const FACT_EXPORT_SESSION_DOMAIN: &str = "fact-export.v1/session";
/// Normative `fact-export-v1` HMAC domain for commit and parent SHA pseudonyms.
pub const FACT_EXPORT_COMMIT_DOMAIN: &str = "fact-export.v1/commit";
/// Normative `fact-export-v1` HMAC domain for `git_evidence_key` (used by D4b at export time).
pub const FACT_EXPORT_GIT_EVIDENCE_DOMAIN: &str = "fact-export.v1/git_evidence";

/// Canonical `merge_status` when parent SHAs are directly observed in the session transcript.
pub const MERGE_STATUS_OBSERVED: &str = "observed";
/// Canonical `merge_status` when parent SHAs are not recorded in the session transcript.
pub const MERGE_STATUS_NOT_OBSERVED: &str = "not_observed";
/// Canonical `merge_status` when merge state cannot be determined.
pub const MERGE_STATUS_UNKNOWN: &str = "unknown";

/// Canonical `default_branch_status` when default-branch membership is not observed.
///
/// In the MVP, default-branch membership cannot be derived from session records alone, so
/// `default_branch_status` is always `not_observed` (`on_default_branch = None`), using a
/// status-only representation consistent with the `fact-export-v1` contract (which has no
/// default-branch reason field).
pub const DEFAULT_BRANCH_STATUS_NOT_OBSERVED: &str = "not_observed";
/// Canonical `default_branch_status` when default-branch membership is unknown (wire export).
pub const DEFAULT_BRANCH_STATUS_UNKNOWN: &str = "unknown";

/// Canonical `observed_at_status` when the observation timestamp comes from the session record.
pub const OBSERVED_AT_STATUS_OBSERVED: &str = "observed";
/// Canonical `observed_at_status` when the session record has no timestamp.
pub const OBSERVED_AT_STATUS_NOT_OBSERVABLE: &str = "not_observable";

/// Canonical `session_match_status` when a commit maps to exactly one session.
pub const SESSION_MATCH_MATCHED: &str = "matched";
/// Canonical `session_match_status` when a commit maps to multiple sessions in the store.
pub const SESSION_MATCH_AMBIGUOUS: &str = "ambiguous";
/// Canonical `session_match_status` when a commit has no attributable session.
pub const SESSION_MATCH_UNMATCHED: &str = "unmatched";
/// Canonical `session_match_status` when session attribution is unknown.
pub const SESSION_MATCH_UNKNOWN: &str = "unknown";

/// Canonical `evidence_source` for session-transcript local git observations.
pub const EVIDENCE_SOURCE_LOCAL_GIT_HISTORY: &str = "local_git_history";

/// Canonical `export_status` when `fact-export-v1` key material is configured and pseudonyms are populated.
pub const EXPORT_STATUS_EXPORTABLE: &str = "exportable";
/// Canonical `export_status` when `fact-export-v1` key material is absent or invalid.
pub const EXPORT_STATUS_NON_EXPORTABLE: &str = "non_exportable";

/// Reason recorded when a commit SHA was observed without parent hashes (e.g. `git rev-parse HEAD`).
pub const MERGE_REASON_PARENTS_NOT_RECORDED: &str = "parents_not_recorded";
/// Reason recorded when parent hashes were abbreviated rather than full 40/64-hex SHAs.
pub const MERGE_REASON_ABBREVIATED_PARENTS: &str = "abbreviated_parents";
/// Reason recorded when parent SHA hex lengths do not match the commit SHA hex length.
pub const MERGE_REASON_MIXED_PARENT_OBJECT_FORMAT: &str = "mixed_parent_object_format";
/// Reason recorded when observations within a source or across sources report different parent lists for the same commit.
pub const MERGE_REASON_CONFLICTING_PARENTS: &str = "conflicting_parents";

/// Minimum byte length of a `fact-export-v1` secret key loaded from hex or key file (32 bytes / 256 bits).
pub const FACT_EXPORT_KEY_MIN_BYTES: usize = LINK_KEY_BYTES;

/// Primary environment variable holding the hex-encoded 32-byte `fact-export-v1` secret key.
pub const ENV_FACT_EXPORT_KEY_HEX: &str = "DENDRO_FACT_EXPORT_KEY_HEX";
/// Fallback environment variable holding the hex-encoded 32-byte `fact-export-v1` secret key.
pub const ENV_ALT_FACT_EXPORT_KEY_HEX: &str = "AGENT_OTEL_FACT_EXPORT_KEY_HEX";
/// Primary environment variable pointing to an absolute path containing the hex-encoded `fact-export-v1` key.
pub const ENV_FACT_EXPORT_KEY_FILE: &str = "DENDRO_FACT_EXPORT_KEY_FILE";
/// Fallback environment variable pointing to an absolute path containing the hex-encoded `fact-export-v1` key.
pub const ENV_ALT_FACT_EXPORT_KEY_FILE: &str = "AGENT_OTEL_FACT_EXPORT_KEY_FILE";
/// Primary environment variable holding the `pseudonym_key_id` identifier.
pub const ENV_FACT_EXPORT_KEY_ID: &str = "DENDRO_FACT_EXPORT_KEY_ID";
/// Fallback environment variable holding the `pseudonym_key_id` identifier.
pub const ENV_ALT_FACT_EXPORT_KEY_ID: &str = "AGENT_OTEL_FACT_EXPORT_KEY_ID";

/// Default key filename inside Dendro's verified private data root (`platform_v1`).
pub const DEFAULT_FACT_EXPORT_KEY_FILENAME: &str = "fact_export.key";

/// Fixed, key-free warning message emitted at most once when a raw `K_org` variable is detected.
pub const FACT_EXPORT_K_ORG_REFUSAL_WARNING: &str = "dendro: fact-export configuration refused: raw K_org variable is forbidden on endpoints (reason=invalid_key_material)";

static FACT_EXPORT_K_ORG_WARNING_ONCE: std::sync::Once = std::sync::Once::new();
static FACT_EXPORT_K_ORG_WARNING_EMITTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

fn emit_fact_export_k_org_warning_once() {
    FACT_EXPORT_K_ORG_WARNING_ONCE.call_once(|| {
        FACT_EXPORT_K_ORG_WARNING_EMITTED.store(true, std::sync::atomic::Ordering::SeqCst);
        eprintln!("{FACT_EXPORT_K_ORG_REFUSAL_WARNING}");
    });
}

const FACT_EXPORT_KEY_LOADER_SPEC: SecretKeyLoaderSpec<'static> = SecretKeyLoaderSpec {
    domain: FACT_EXPORT_COMMIT_DOMAIN,
    hex_env_vars: &[ENV_FACT_EXPORT_KEY_HEX, ENV_ALT_FACT_EXPORT_KEY_HEX],
    file_env_vars: &[ENV_FACT_EXPORT_KEY_FILE, ENV_ALT_FACT_EXPORT_KEY_FILE],
    key_file_name: DEFAULT_FACT_EXPORT_KEY_FILENAME,
};

/// Upper bound on tracked tool-call IDs per source file (bounded LRU).
pub const MAX_PENDING_CALLS_PER_SOURCE: usize = 512;
/// Upper bound on unique commits tracked per source file to bound memory.
pub const MAX_OBSERVED_COMMITS_PER_SOURCE: usize = 4096;
/// Upper bound on parents per commit (`fact-export-v1` schema `maxItems = 16`).
pub const MAX_PARENTS_PER_COMMIT: usize = 16;
/// Upper bound on raw git command string length inspected per tool call (16 KiB).
pub const MAX_GIT_COMMAND_BYTES: usize = 16 * 1024;
/// Upper bound on raw git tool output length inspected per tool result (64 KiB).
pub const MAX_GIT_OUTPUT_BYTES: usize = 64 * 1024;
/// Upper bound on output lines inspected per git tool result.
pub const MAX_GIT_LINES_PER_RECORD: usize = 512;

/// Zeroize-on-drop wrapper around a `fact-export-v1` HMAC key.
///
/// Intentionally does not implement `Clone`, `Display`, or `ToString`, and redacts `Debug`.
pub struct FactExportKey(Vec<u8>);

impl FactExportKey {
    /// Construct a `FactExportKey` from a 32-byte array.
    #[must_use]
    pub fn from_bytes(bytes: [u8; FACT_EXPORT_KEY_MIN_BYTES]) -> Self {
        Self(bytes.to_vec())
    }

    /// Construct a `FactExportKey` from a byte slice requiring at least 32 bytes (`FACT_EXPORT_KEY_MIN_BYTES`).
    pub fn from_slice(bytes: &[u8]) -> Result<Self, FactExportKeyError> {
        if bytes.len() < FACT_EXPORT_KEY_MIN_BYTES {
            return Err(FactExportKeyError::InvalidKeyMaterial);
        }
        Ok(Self(bytes.to_vec()))
    }

    /// Construct a `FactExportKey` from raw key bytes requiring at least 32 bytes (`FACT_EXPORT_KEY_MIN_BYTES`).
    pub fn from_raw_key_bytes(bytes: &[u8]) -> Result<Self, FactExportKeyError> {
        Self::from_slice(bytes)
    }

    /// Decode a hex-encoded key string requiring at least 32 decoded bytes (64 hex chars).
    pub fn from_hex(hex_str: &str) -> Result<Self, FactExportKeyError> {
        let trimmed = hex_str.trim();
        if trimmed.is_empty() {
            return Err(FactExportKeyError::InvalidKeyMaterial);
        }
        let mut decoded =
            hex::decode(trimmed).map_err(|_| FactExportKeyError::InvalidKeyMaterial)?;
        if decoded.len() < FACT_EXPORT_KEY_MIN_BYTES {
            decoded.fill(0);
            std::hint::black_box(&mut decoded);
            return Err(FactExportKeyError::InvalidKeyMaterial);
        }
        Ok(Self(decoded))
    }

    #[must_use]
    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl PartialEq for FactExportKey {
    fn eq(&self, other: &Self) -> bool {
        if self.0.len() != other.0.len() {
            return false;
        }
        let mut acc = 0u8;
        for (&a, &b) in self.0.iter().zip(other.0.iter()) {
            acc = std::hint::black_box(acc | (a ^ b));
        }
        std::hint::black_box(acc) == 0
    }
}

impl Eq for FactExportKey {}

impl fmt::Debug for FactExportKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("FactExportKey([REDACTED])")
    }
}

impl Drop for FactExportKey {
    fn drop(&mut self) {
        self.0.fill(0);
        std::hint::black_box(&mut self.0);
    }
}

/// Errors encountered when loading or validating `fact-export-v1` key material.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FactExportKeyError {
    /// No key material was configured in any source.
    Unconfigured,
    /// Key material was malformed, shorter than 32 bytes, or failed `platform_v1` file checks.
    InvalidKeyMaterial,
    /// `pseudonym_key_id` was missing or violated the ASCII identifier rules.
    InvalidKeyId,
    /// A forbidden `K_org` environment variable was present; key loading was refused.
    ForbiddenOrgKeyPresent,
}

/// Configuration state for `fact-export-v1` pseudonym derivation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FactExportKeyState {
    /// Key and `pseudonym_key_id` are valid and ready to derive pseudonyms.
    Ready,
    /// Neither key nor key_id was configured.
    Unconfigured,
    /// Key material was invalid or unreadable.
    InvalidKeyMaterial,
    /// Key ID was missing or invalid while a key was provided.
    InvalidKeyId,
    /// A forbidden `K_org` variable was detected in the environment or config files.
    ForbiddenOrgKeyPresent,
}

/// Runtime configuration for `fact-export-v1` commit pseudonymization.
///
/// Intentionally does not implement `Clone` so secret key material is never duplicated implicitly.
pub struct FactExportConfig {
    key: Option<FactExportKey>,
    pseudonym_key_id: Option<String>,
    state: FactExportKeyState,
}

impl Default for FactExportConfig {
    fn default() -> Self {
        Self::unconfigured()
    }
}

impl fmt::Debug for FactExportConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FactExportConfig")
            .field("state", &self.state)
            .field("pseudonym_key_id", &self.pseudonym_key_id)
            .field("has_key", &self.key.is_some())
            .finish()
    }
}

/// Validates a `pseudonym_key_id` token (`1..=64` ASCII chars in `[A-Za-z0-9._:-]`).
#[must_use]
pub fn is_valid_key_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b':'))
}

impl FactExportConfig {
    /// Returns an unconfigured `FactExportConfig` (rows are persisted as `non_exportable` with
    /// local sealed commit/parent identifiers only).
    #[must_use]
    pub const fn unconfigured() -> Self {
        Self {
            key: None,
            pseudonym_key_id: None,
            state: FactExportKeyState::Unconfigured,
        }
    }

    /// Construct a `FactExportConfig` from a validated [`FactExportKey`] and `pseudonym_key_id`.
    #[must_use]
    pub fn from_key(key: FactExportKey, pseudonym_key_id: impl Into<String>) -> Self {
        let kid = pseudonym_key_id.into();
        if !is_valid_key_id(&kid) {
            return Self {
                key: None,
                pseudonym_key_id: None,
                state: FactExportKeyState::InvalidKeyId,
            };
        }
        Self {
            key: Some(key),
            pseudonym_key_id: Some(kid),
            state: FactExportKeyState::Ready,
        }
    }

    /// Construct a `FactExportConfig` from raw key bytes (requiring at least 32 bytes) and
    /// `pseudonym_key_id`.
    #[must_use]
    pub fn from_raw_key(raw_key: &[u8], pseudonym_key_id: impl Into<String>) -> Self {
        let Ok(key) = FactExportKey::from_raw_key_bytes(raw_key) else {
            return Self {
                key: None,
                pseudonym_key_id: None,
                state: FactExportKeyState::InvalidKeyMaterial,
            };
        };
        Self::from_key(key, pseudonym_key_id)
    }

    #[cfg(test)]
    pub(crate) fn from_test_vector_key_unchecked(
        raw_key: &[u8],
        pseudonym_key_id: impl Into<String>,
    ) -> Self {
        Self::from_key(FactExportKey(raw_key.to_vec()), pseudonym_key_id)
    }

    /// Construct a `FactExportConfig` from a hex-encoded 32-byte key and `pseudonym_key_id`.
    #[must_use]
    pub fn from_key_hex(key_hex: &str, pseudonym_key_id: Option<&str>) -> Self {
        let key = match FactExportKey::from_hex(key_hex) {
            Ok(k) => k,
            Err(_) => {
                return Self {
                    key: None,
                    pseudonym_key_id: None,
                    state: FactExportKeyState::InvalidKeyMaterial,
                }
            }
        };
        let Some(kid) = pseudonym_key_id.map(str::trim).filter(|s| !s.is_empty()) else {
            return Self {
                key: None,
                pseudonym_key_id: None,
                state: FactExportKeyState::InvalidKeyId,
            };
        };
        Self::from_key(key, kid.to_string())
    }

    /// Load `FactExportConfig` from process environment variables,
    /// `<private_data_root>/fact_export.key` inside Dendro's verified private data root
    /// (`default_private_data_root()`), and `$XDG_CONFIG_HOME/observability/consumer.env`.
    /// Never reads configuration or key files from the database parent or current working directory.
    #[must_use]
    pub fn from_env() -> Self {
        let root = default_private_data_root();
        let consumer = consumer_env_path();
        Self::from_env_and_paths(root.as_deref(), consumer.as_deref())
    }

    /// Load `FactExportConfig` from process environment variables, optional
    /// `<private_data_root>/fact_export.key`, and `$XDG_CONFIG_HOME/observability/consumer.env`.
    #[must_use]
    pub fn from_env_with_root(private_data_root: Option<&Path>) -> Self {
        let consumer = consumer_env_path();
        Self::from_env_and_paths(private_data_root, consumer.as_deref())
    }

    /// Alias for [`Self::from_env_with_root`].
    #[must_use]
    pub fn load(private_data_root: Option<&Path>) -> Self {
        Self::from_env_with_root(private_data_root)
    }

    /// Load `FactExportConfig` from the process environment, an explicit private data root, and
    /// an explicit `consumer.env` path using `D3`'s shared loader.
    #[must_use]
    pub fn from_env_and_paths(
        private_data_root: Option<&Path>,
        consumer_env_file: Option<&Path>,
    ) -> Self {
        let process_env = collect_process_env_vars();
        let result = Self::from_sources(&process_env, private_data_root, consumer_env_file);
        zeroize_env_map(process_env);
        result
    }

    /// Load `FactExportConfig` from a map of process environment variables and an optional private data root.
    #[must_use]
    pub fn from_vars(vars: &BTreeMap<String, String>, private_data_root: Option<&Path>) -> Self {
        Self::from_sources(vars, private_data_root, None)
    }

    /// Load `FactExportConfig` from explicit process environment variables, an optional private
    /// data root (`<private_data_root>/fact_export.key`), and an optional `consumer.env` file path.
    ///
    /// Shares the exact loader implementation in `crate::link_id` (`load_secret_key_from_sources`,
    /// `load_consumer_env_vars`, `is_k_org_var_name`, and `platform_v1` verification).
    #[must_use]
    pub fn from_sources(
        process_env: &BTreeMap<String, String>,
        private_data_root: Option<&Path>,
        consumer_env_file: Option<&Path>,
    ) -> Self {
        let consumer_vars = load_consumer_env_vars(consumer_env_file);

        let key_id = pick_var_with_fallback(
            process_env,
            &consumer_vars,
            &[ENV_FACT_EXPORT_KEY_ID, ENV_ALT_FACT_EXPORT_KEY_ID],
        );

        let outcome = load_secret_key_from_sources(
            &FACT_EXPORT_KEY_LOADER_SPEC,
            process_env,
            &consumer_vars,
            private_data_root,
        );
        zeroize_env_map(consumer_vars);

        match outcome {
            SecretKeyLoadOutcome::KOrgRejected => {
                emit_fact_export_k_org_warning_once();
                Self {
                    key: None,
                    pseudonym_key_id: None,
                    state: FactExportKeyState::ForbiddenOrgKeyPresent,
                }
            }
            SecretKeyLoadOutcome::InvalidKeyMaterial => Self {
                key: None,
                pseudonym_key_id: None,
                state: FactExportKeyState::InvalidKeyMaterial,
            },
            SecretKeyLoadOutcome::Unconfigured => {
                if key_id.is_some() {
                    Self {
                        key: None,
                        pseudonym_key_id: None,
                        state: FactExportKeyState::InvalidKeyMaterial,
                    }
                } else {
                    Self::unconfigured()
                }
            }
            SecretKeyLoadOutcome::Loaded(mut bytes) => {
                let k = FactExportKey::from_bytes(bytes);
                bytes.fill(0);
                std::hint::black_box(&mut bytes);
                let Some(kid) = key_id.as_deref().map(str::trim).filter(|s| !s.is_empty()) else {
                    return Self {
                        key: None,
                        pseudonym_key_id: None,
                        state: FactExportKeyState::InvalidKeyId,
                    };
                };
                Self::from_key(k, kid.to_string())
            }
        }
    }

    #[must_use]
    pub const fn is_ready(&self) -> bool {
        matches!(self.state, FactExportKeyState::Ready) && self.key.is_some()
    }

    #[must_use]
    pub const fn state(&self) -> FactExportKeyState {
        self.state
    }

    #[must_use]
    pub const fn k_org_rejected(&self) -> bool {
        matches!(self.state, FactExportKeyState::ForbiddenOrgKeyPresent)
    }

    #[must_use]
    pub const fn k_org_warning(&self) -> Option<&'static str> {
        if self.k_org_rejected() {
            Some(FACT_EXPORT_K_ORG_REFUSAL_WARNING)
        } else {
            None
        }
    }

    #[must_use]
    pub fn pseudonym_key_id(&self) -> Option<&str> {
        self.pseudonym_key_id.as_deref()
    }

    #[must_use]
    pub fn key(&self) -> Option<&FactExportKey> {
        self.key.as_ref()
    }

    /// Derive the `fact-export-v1` `source_id` pseudonym from a provisioned logical source ID
    /// (for D4b export and contract vector verification).
    #[must_use]
    pub fn derive_source_id(&self, logical_source_id: &str) -> Option<String> {
        let key = self.key.as_ref()?;
        Some(derive_export_source_id(key, logical_source_id))
    }

    /// Derive the `fact-export-v1` `session_key` pseudonym from a provisioned logical source ID
    /// (for D4b export and contract vector verification).
    #[must_use]
    pub fn derive_session_key(
        &self,
        logical_source_id: &str,
        raw_harness: &str,
        raw_native_session: &str,
    ) -> Option<String> {
        let key = self.key.as_ref()?;
        let source_hmac = derive_export_source_id(key, logical_source_id);
        let canonical = CanonicalHarness::from_alias(raw_harness)?;
        let trimmed_session = raw_native_session.trim();
        if trimmed_session.is_empty() {
            return None;
        }
        Some(derive_export_session_key(
            key,
            &source_hmac,
            canonical.as_str(),
            trimmed_session,
        ))
    }

    /// Derive the `fact-export-v1` `commit_hmac` (or an element of `parent_hmacs`).
    #[must_use]
    pub fn derive_commit_hmac(&self, canonical_commit_sha: &str) -> Option<String> {
        let key = self.key.as_ref()?;
        if !is_canonical_commit_sha(canonical_commit_sha) {
            return None;
        }
        Some(derive_export_commit_hmac(key, canonical_commit_sha))
    }

    /// Derive the `fact-export-v1` `git_evidence_key` from a provisioned logical source ID
    /// (for D4b export and contract vector verification).
    #[must_use]
    pub fn derive_git_evidence_key(
        &self,
        logical_source_id: &str,
        commit_hmac: &str,
        observed_at_us: Option<i64>,
        session_key: Option<&str>,
        session_match_status: &str,
    ) -> Option<String> {
        let key = self.key.as_ref()?;
        let source_hmac = derive_export_source_id(key, logical_source_id);
        Some(derive_export_git_evidence_key(
            key,
            &source_hmac,
            commit_hmac,
            observed_at_us,
            session_key,
            session_match_status,
        ))
    }
}

/// Compute `HMAC-SHA256(key, LP("fact-export.v1/" || kind, *fields))` as 64-char lowercase hex.
#[must_use]
pub fn derive_fact_export_pseudonym(key: &FactExportKey, kind: &str, fields: &[&str]) -> String {
    let domain = format!("fact-export.v1/{kind}");
    let total_len: usize =
        4 + domain.len() + fields.iter().map(|field| 4 + field.len()).sum::<usize>();
    let mut payload = Vec::with_capacity(total_len);
    payload.extend_from_slice(&lp_encode_str(&domain));
    for &field in fields {
        payload.extend_from_slice(&lp_encode_str(field));
    }
    hex::encode(hmac_sha256(key.as_bytes(), &payload))
}

/// Derive `source_id = lp_hmac(key, "source", logical_source_id)` per `fact-export-v1`
/// (always applies HMAC; never passes through 64-hex input).
#[must_use]
pub fn derive_export_source_id(key: &FactExportKey, logical_source_id: &str) -> String {
    derive_fact_export_pseudonym(key, "source", &[logical_source_id])
}

/// Derive `session_key = lp_hmac(key, "session", source_hmac, canonical_harness, raw_native_session)`.
#[must_use]
pub fn derive_export_session_key(
    key: &FactExportKey,
    source_hmac: &str,
    canonical_harness: &str,
    raw_native_session: &str,
) -> String {
    derive_fact_export_pseudonym(
        key,
        "session",
        &[source_hmac, canonical_harness, raw_native_session],
    )
}

/// Derive `commit_hmac = lp_hmac(key, "commit", canonical_commit_sha)`.
#[must_use]
pub fn derive_export_commit_hmac(key: &FactExportKey, canonical_commit_sha: &str) -> String {
    derive_fact_export_pseudonym(key, "commit", &[canonical_commit_sha])
}

/// Derive `git_evidence_key = lp_hmac(key, "git_evidence", source_hmac, commit_hmac, observed_at_us_or_empty, session_key_or_empty, session_match_status)`.
#[must_use]
pub fn derive_export_git_evidence_key(
    key: &FactExportKey,
    source_hmac: &str,
    commit_hmac: &str,
    observed_at_us: Option<i64>,
    session_key: Option<&str>,
    session_match_status: &str,
) -> String {
    let obs_str = observed_at_us.map(|us| us.to_string()).unwrap_or_default();
    let sess_str = session_key.unwrap_or("");
    derive_fact_export_pseudonym(
        key,
        "git_evidence",
        &[
            source_hmac,
            commit_hmac,
            &obs_str,
            sess_str,
            session_match_status,
        ],
    )
}

/// Seal a canonical commit SHA with Dendro's local `tool_key` (`commit:<hmac_sha256>`).
#[must_use]
pub fn seal_commit(tool_key: Option<&[u8]>, canonical_sha: &str) -> String {
    let key = tool_key.unwrap_or(b"");
    format!(
        "commit:{}",
        crate::privacy::hmac_domain(key, "git_commit", canonical_sha)
    )
}

/// Returns `true` if `s` is a canonical full lowercase hex git commit SHA:
/// either 40 lowercase hex chars (SHA-1) or 64 lowercase hex chars (SHA-256).
#[must_use]
pub fn is_canonical_commit_sha(s: &str) -> bool {
    (s.len() == 40 || s.len() == 64)
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Returns `true` if `s` is an abbreviated lowercase hex git commit SHA (`7..=39` hex chars).
#[must_use]
pub fn is_abbreviated_commit_sha(s: &str) -> bool {
    (7..40).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// One persisted per-source local git evidence row in `git_evidence_facts`.
///
/// D4c persists only local facts (`commit_sealed`, `parent_sealed`, observed flags/statuses and
/// reasons, `observed_at`, local `native_session`) plus `commit_hmac`/`parent_hmacs` when
/// `FactExportConfig` is configured. Contract `source_id`, `session_key`, and `git_evidence_key`
/// are NOT persisted here; D4b computes them at export time from a provisioned logical source ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitEvidenceFact {
    pub source_key: String,
    pub source_id: String,
    pub generation: i64,
    pub harness: String,
    pub canonical_harness: String,
    pub native_session: Option<String>,
    pub commit_sealed: String,
    pub parent_sealed: Vec<String>,
    pub is_merge: Option<bool>,
    pub merge_status: String,
    pub merge_missing_reason: Option<String>,
    pub on_default_branch: Option<bool>,
    pub default_branch_status: String,
    pub observed_at: Option<DateTime<Utc>>,
    pub observed_at_status: String,
    pub session_match_status: String,
    pub evidence_source: String,
    pub export_status: String,
    pub pseudonym_key_id: Option<String>,
    pub commit_hmac: Option<String>,
    pub parent_hmacs: Vec<String>,
}

/// Per-source git evidence coverage counters persisted in `git_evidence_coverage`.
///
/// - `observed`, `missing_parents`, `ambiguous`, `unmatched`, and `default_branch_not_observed`
///   are counted per unique accepted canonical commit SHA in the source generation.
/// - `unresolved_abbreviated`, `unverified_object_format`, `unparsed_compound_git_calls`, and
///   `cat_file_unconfirmed` are counted per rejected occurrence.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GitEvidenceCoverage {
    pub source_id: String,
    pub generation: i64,
    pub harness: String,
    pub observed: i64,
    pub unresolved_abbreviated: i64,
    pub unverified_object_format: i64,
    pub missing_parents: i64,
    pub ambiguous: i64,
    pub unmatched: i64,
    pub default_branch_not_observed: i64,
    pub unparsed_compound_git_calls: i64,
    pub cat_file_unconfirmed: i64,
}

impl GitEvidenceCoverage {
    /// Returns `true` if any git evidence or diagnostic counter is non-zero.
    #[must_use]
    pub const fn has_activity(&self) -> bool {
        self.observed > 0
            || self.unresolved_abbreviated > 0
            || self.unverified_object_format > 0
            || self.missing_parents > 0
            || self.ambiguous > 0
            || self.unmatched > 0
            || self.default_branch_not_observed > 0
            || self.unparsed_compound_git_calls > 0
            || self.cat_file_unconfirmed > 0
    }
}

/// Deterministically derives cross-source `GitEvidenceFact` records across all per-source rows
/// stored in `git_evidence_facts` using the current process/root/consumer [`FactExportConfig`].
///
/// Per S5, a row is exportable ONLY when the current config is [`FactExportKeyState::Ready`] AND
/// its `pseudonym_key_id` equals the configured `pseudonym_key_id`. No fallback to a bare key-ID
/// variable when the config is not `Ready`; with no valid key configured, nothing is exportable.
#[must_use]
pub fn resolve_git_evidence_across_sources(rows: &[GitEvidenceFact]) -> Vec<GitEvidenceFact> {
    let cfg = FactExportConfig::from_env();
    resolve_git_evidence_across_sources_with_config(rows, &cfg)
}

/// Deterministically derives cross-source `GitEvidenceFact` records across all per-source rows
/// using an explicit [`FactExportConfig`].
///
/// A row is exportable ONLY when `config.is_ready()` is `true` and the row's `pseudonym_key_id`
/// equals `config.pseudonym_key_id()`.
#[must_use]
pub fn resolve_git_evidence_across_sources_with_config(
    rows: &[GitEvidenceFact],
    config: &FactExportConfig,
) -> Vec<GitEvidenceFact> {
    let key_id = if config.is_ready() {
        config.pseudonym_key_id()
    } else {
        None
    };
    resolve_git_evidence_across_sources_with_key_id(rows, key_id)
}

/// Deterministically derives cross-source `GitEvidenceFact` records across all per-source rows,
/// using `current_key_id` (from a `Ready` [`FactExportConfig`]) to gate exportability.
///
/// When `current_key_id` is `None`, nothing is exportable (`export_status = "non_exportable"`).
#[must_use]
pub fn resolve_git_evidence_across_sources_with_key_id(
    rows: &[GitEvidenceFact],
    current_key_id: Option<&str>,
) -> Vec<GitEvidenceFact> {
    if rows.is_empty() {
        return Vec::new();
    }
    let mut sorted: Vec<&GitEvidenceFact> = rows.iter().collect();
    sorted.sort_by(|a, b| {
        (&a.commit_sealed, &a.source_id, &a.source_key).cmp(&(
            &b.commit_sealed,
            &b.source_id,
            &b.source_key,
        ))
    });

    let allowed_key_id: Option<&str> = current_key_id
        .map(str::trim)
        .filter(|kid| is_valid_key_id(kid));

    let mut groups: BTreeMap<String, Vec<&GitEvidenceFact>> = BTreeMap::new();
    for row in sorted {
        groups
            .entry(row.commit_sealed.clone())
            .or_default()
            .push(row);
    }

    let is_valid_hmac_hex = |s: &str| s.len() == 64 && is_canonical_commit_sha(s);

    let mut resolved = Vec::with_capacity(groups.len());
    for (_commit_sealed, group) in groups {
        let mut any_ambiguous = false;
        let mut any_unmatched = false;
        let mut distinct_sessions: BTreeSet<(String, String)> = BTreeSet::new();

        for row in &group {
            match row.session_match_status.as_str() {
                SESSION_MATCH_AMBIGUOUS => {
                    any_ambiguous = true;
                }
                SESSION_MATCH_MATCHED => {
                    if let Some(sess) = row
                        .native_session
                        .as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                    {
                        distinct_sessions.insert((row.canonical_harness.clone(), sess.to_string()));
                    } else {
                        any_unmatched = true;
                    }
                }
                SESSION_MATCH_UNMATCHED => {
                    any_unmatched = true;
                }
                _ => {}
            }
        }

        let (session_match_status, native_session) = if any_ambiguous || distinct_sessions.len() > 1
        {
            (SESSION_MATCH_AMBIGUOUS.to_string(), None)
        } else if distinct_sessions.len() == 1 {
            let (_h, s) = distinct_sessions.into_iter().next().unwrap();
            (SESSION_MATCH_MATCHED.to_string(), Some(s))
        } else if any_unmatched {
            (SESSION_MATCH_UNMATCHED.to_string(), None)
        } else {
            (SESSION_MATCH_UNKNOWN.to_string(), None)
        };

        // Pick base row: if matched to a single session, prefer the row that carries that session.
        let base = if let Some(ref matched_sess) = native_session {
            group
                .iter()
                .copied()
                .find(|r| r.native_session.as_deref() == Some(matched_sess.as_str()))
                .unwrap_or(group[0])
        } else {
            group[0]
        };

        // Earliest observed_at across all observations of this commit.
        let observed_at = group.iter().filter_map(|r| r.observed_at).min();
        let observed_at_status = if observed_at.is_some() {
            OBSERVED_AT_STATUS_OBSERVED.to_string()
        } else {
            OBSERVED_AT_STATUS_NOT_OBSERVABLE.to_string()
        };

        // S4 & R5: Merge status & parent disagreement check within or across sources.
        let any_intra_source_conflict = group
            .iter()
            .any(|r| r.merge_missing_reason.as_deref() == Some(MERGE_REASON_CONFLICTING_PARENTS));
        let observed_parent_rows: Vec<&GitEvidenceFact> = group
            .iter()
            .copied()
            .filter(|r| r.merge_status == MERGE_STATUS_OBSERVED)
            .collect();
        let parents_conflict = any_intra_source_conflict
            || (observed_parent_rows.len() > 1
                && !observed_parent_rows
                    .windows(2)
                    .all(|w| w[0].parent_sealed == w[1].parent_sealed));

        let (merge_status, is_merge, merge_missing_reason, parent_sealed) = if parents_conflict {
            (
                MERGE_STATUS_NOT_OBSERVED.to_string(),
                None,
                Some(MERGE_REASON_CONFLICTING_PARENTS.to_string()),
                Vec::new(),
            )
        } else if let Some(obs_row) = observed_parent_rows.first().copied() {
            (
                MERGE_STATUS_OBSERVED.to_string(),
                Some(obs_row.parent_sealed.len() > 1),
                None,
                obs_row.parent_sealed.clone(),
            )
        } else {
            let reason = group
                .iter()
                .find_map(|r| r.merge_missing_reason.clone())
                .or_else(|| Some(MERGE_REASON_PARENTS_NOT_RECORDED.to_string()));
            (
                MERGE_STATUS_NOT_OBSERVED.to_string(),
                None,
                reason,
                Vec::new(),
            )
        };

        // S3: `on_default_branch` is ALWAYS `not_observed` in the MVP (status-only representation,
        // consistent with the `fact-export-v1` contract's fields).
        let default_branch_status = DEFAULT_BRANCH_STATUS_NOT_OBSERVED.to_string();
        let on_default_branch = None;

        // S5 & R5: Derive exported fields from ONE coherent row under the eligible `allowed_key_id`.
        // Never combine parents from one row with HMACs from another.
        let row_has_coherent_hmacs = |r: &GitEvidenceFact| -> bool {
            let Some(kid) = allowed_key_id else {
                return false;
            };
            if r.export_status != EXPORT_STATUS_EXPORTABLE
                || r.pseudonym_key_id.as_deref() != Some(kid)
                || !r.commit_hmac.as_deref().is_some_and(is_valid_hmac_hex)
            {
                return false;
            }
            if r.merge_status == MERGE_STATUS_OBSERVED {
                r.parent_hmacs.len() == r.parent_sealed.len()
                    && r.parent_hmacs.iter().all(|h| is_valid_hmac_hex(h))
            } else {
                r.parent_hmacs.is_empty()
            }
        };

        let (export_status, pseudonym_key_id, commit_hmac, parent_hmacs) = if merge_status
            == MERGE_STATUS_OBSERVED
        {
            if let Some(coherent_row) = group.iter().copied().find(|r| {
                r.merge_status == MERGE_STATUS_OBSERVED
                    && r.parent_sealed == parent_sealed
                    && row_has_coherent_hmacs(r)
            }) {
                (
                    EXPORT_STATUS_EXPORTABLE.to_string(),
                    coherent_row.pseudonym_key_id.clone(),
                    coherent_row.commit_hmac.clone(),
                    coherent_row.parent_hmacs.clone(),
                )
            } else {
                (
                    EXPORT_STATUS_NON_EXPORTABLE.to_string(),
                    None,
                    None,
                    Vec::new(),
                )
            }
        } else if let Some(coherent_row) = group.iter().copied().find(|r| row_has_coherent_hmacs(r))
        {
            (
                EXPORT_STATUS_EXPORTABLE.to_string(),
                coherent_row.pseudonym_key_id.clone(),
                coherent_row.commit_hmac.clone(),
                Vec::new(),
            )
        } else {
            (
                EXPORT_STATUS_NON_EXPORTABLE.to_string(),
                None,
                None,
                Vec::new(),
            )
        };

        resolved.push(GitEvidenceFact {
            source_key: base.source_key.clone(),
            source_id: base.source_id.clone(),
            generation: base.generation,
            harness: base.harness.clone(),
            canonical_harness: base.canonical_harness.clone(),
            native_session,
            commit_sealed: base.commit_sealed.clone(),
            parent_sealed,
            is_merge,
            merge_status,
            merge_missing_reason,
            on_default_branch,
            default_branch_status,
            observed_at,
            observed_at_status,
            session_match_status,
            evidence_source: base.evidence_source.clone(),
            export_status,
            pseudonym_key_id,
            commit_hmac,
            parent_hmacs,
        });
    }

    resolved
}

// ---------------------------------------------------------------------------
// Exact-Match Allow-Listed Git Command Classifier & Output Parser (S1–S3)
// ---------------------------------------------------------------------------

/// Strictly allow-listed git command shapes recognized by D4c (`S1`).
/// Any command outside this exact-match allow-list is classified as [`GitCommandKind::NonGit`]
/// and contributes nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitCommandKind {
    /// Not an allow-listed git command shape. Contributes nothing.
    NonGit,
    /// `git rev-parse HEAD`.
    RevParseCommit,
    /// `git rev-parse --show-object-format`.
    ShowObjectFormat,
    /// `git log` or `git show -s`/`--no-patch` with `--format=%H` (or `--pretty=format:%H`).
    LogCommitOnly,
    /// `git log` or `git show -s`/`--no-patch` with `--format=Q%H %PQ` (or `--pretty=format:Q%H %PQ`).
    LogCommitAndParents,
    /// `git cat-file -p <FULL-SHA>` where `<FULL-SHA>` is a canonical 40-hex or 64-hex SHA.
    CatFileCommit { target_sha: String },
    /// `git commit ...` (first output line matching `[<branch> <abbrev>] ...` -> counted unresolved only).
    Commit,
}

/// Raw command representation extracted from a session tool-call record (`S1`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitCommandInput {
    /// A single command string (e.g. Claude `Bash`, Pi `bash`, Grok `bash`, or Codex string command).
    CommandString(String),
    /// An `argv` array of strings (e.g. Codex `CommandExecution` / `exec_command`).
    Argv(Vec<String>),
}

/// Result of classifying a shell tool call under `S1`'s exact-match allow-list and character gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClassifiedCommandCall {
    /// Not a git invocation.
    NonGit,
    /// A rejected git-looking command (counted in `unparsed_compound_git_calls`).
    SkippedCompoundGit,
    /// An exact-match allow-listed git command.
    Allowed(GitCommandKind),
}

/// Returns `true` if `b` is in the strict character allow-list `[A-Za-z0-9 =%_.:/'"-]` (`S1`).
///
/// Rejects `$`, `\`, backtick, `;`, `|`, `&`, `<`, `>`, `(`, `)`, newline, tab, and all other bytes.
#[must_use]
pub const fn is_allowed_git_command_char(b: u8) -> bool {
    b.is_ascii_alphanumeric()
        || matches!(
            b,
            b' ' | b'=' | b'%' | b'_' | b'.' | b':' | b'/' | b'\'' | b'"' | b'-'
        )
}

/// Classify a command string into its [`GitCommandKind`] under `S1`.
#[must_use]
pub fn classify_shell_command(command: &str) -> GitCommandKind {
    match classify_single_git_command(command) {
        ClassifiedCommandCall::Allowed(kind) => kind,
        ClassifiedCommandCall::NonGit | ClassifiedCommandCall::SkippedCompoundGit => {
            GitCommandKind::NonGit
        }
    }
}

/// Classify a command string under `S1`:
/// 1. Rejects any command exceeding [`MAX_GIT_COMMAND_BYTES`] or containing any byte outside
///    `[A-Za-z0-9 =%_.:/'"-]` (no `$`, `\`, backtick, `;`, `|`, `&`, `<`, `>`, `(`, `)`,
///    newline, or tab).
/// 2. After trimming outer ASCII whitespace, the whole command string must be an exact match
///    for one of the `S1` templates.
/// 3. Any rejected git-looking command returns [`ClassifiedCommandCall::SkippedCompoundGit`]
///    and increments `unparsed_compound_git_calls`.
#[must_use]
pub fn classify_single_git_command(command: &str) -> ClassifiedCommandCall {
    if command.trim_ascii().is_empty() {
        return ClassifiedCommandCall::NonGit;
    }
    if command.len() > MAX_GIT_COMMAND_BYTES || !command.bytes().all(is_allowed_git_command_char) {
        return if command_mentions_git(command) {
            ClassifiedCommandCall::SkippedCompoundGit
        } else {
            ClassifiedCommandCall::NonGit
        };
    }

    let trimmed = command.trim_ascii();
    if let Some(kind) = match_exact_git_string_template(trimmed) {
        return ClassifiedCommandCall::Allowed(kind);
    }

    if command_mentions_git(trimmed) {
        ClassifiedCommandCall::SkippedCompoundGit
    } else {
        ClassifiedCommandCall::NonGit
    }
}

/// Classify an `argv` array (Codex) under `S1`:
/// - A shell wrapper argv `["bash"|"sh"|"zsh", "-c"|"-lc", CMD]` or
///   `["pwsh"|"powershell", "-Command"|"-c", CMD]` is accepted only if `CMD` alone passes
///   [`classify_single_git_command`]. No other wrappers and no `cmd /c`.
/// - Otherwise, accepts ONLY an `argv` whose elements are the exact tokens of an `S1` template,
///   compared element by element after character-safety validation.
#[must_use]
pub fn classify_git_argv<S: AsRef<str>>(argv: &[S]) -> ClassifiedCommandCall {
    if argv.is_empty() {
        return ClassifiedCommandCall::NonGit;
    }
    let total_bytes: usize = argv.iter().map(|s| s.as_ref().len()).sum();
    if total_bytes > MAX_GIT_COMMAND_BYTES {
        return if argv_mentions_git(argv) {
            ClassifiedCommandCall::SkippedCompoundGit
        } else {
            ClassifiedCommandCall::NonGit
        };
    }

    if argv.len() == 3 {
        let a0 = argv[0].as_ref();
        let a1 = argv[1].as_ref();
        let a2 = argv[2].as_ref();
        let is_posix_wrapper = matches!(a0, "bash" | "sh" | "zsh") && matches!(a1, "-c" | "-lc");
        let is_pwsh_wrapper =
            matches!(a0, "pwsh" | "powershell") && matches!(a1, "-Command" | "-c");
        if is_posix_wrapper || is_pwsh_wrapper {
            return classify_single_git_command(a2);
        }
    }

    if argv
        .iter()
        .any(|s| s.as_ref().is_empty() || !s.as_ref().bytes().all(is_allowed_git_command_char))
    {
        return if argv_mentions_git(argv) {
            ClassifiedCommandCall::SkippedCompoundGit
        } else {
            ClassifiedCommandCall::NonGit
        };
    }

    let tokens: Vec<&str> = argv.iter().map(AsRef::as_ref).collect();
    if let Some(kind) = match_exact_git_argv_tokens(&tokens) {
        return ClassifiedCommandCall::Allowed(kind);
    }

    if argv_mentions_git(argv) {
        ClassifiedCommandCall::SkippedCompoundGit
    } else {
        ClassifiedCommandCall::NonGit
    }
}

/// Classify a [`GitCommandInput`] (either a command string or an `argv` array).
#[must_use]
pub fn classify_git_command_input(input: &GitCommandInput) -> ClassifiedCommandCall {
    match input {
        GitCommandInput::CommandString(s) => classify_single_git_command(s),
        GitCommandInput::Argv(argv) => classify_git_argv(argv),
    }
}

fn is_valid_log_count(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('0')
        && s.bytes().all(|b| b.is_ascii_digit())
        && s.parse::<u32>().is_ok_and(|n| (1..=1000).contains(&n))
}

fn match_string_format_spec(fmt: &str) -> Option<GitCommandKind> {
    match fmt {
        "--format=%H" | "--pretty=format:%H" => Some(GitCommandKind::LogCommitOnly),
        "--format='%H %P'"
        | "--format=\"%H %P\""
        | "--pretty=format:'%H %P'"
        | "--pretty=format:\"%H %P\"" => Some(GitCommandKind::LogCommitAndParents),
        _ => None,
    }
}

fn match_argv_format_spec(fmt: &str) -> Option<GitCommandKind> {
    match fmt {
        "--format=%H" | "--pretty=format:%H" => Some(GitCommandKind::LogCommitOnly),
        "--format=%H %P" | "--pretty=format:%H %P" => Some(GitCommandKind::LogCommitAndParents),
        _ => None,
    }
}

fn match_exact_git_string_template(trimmed: &str) -> Option<GitCommandKind> {
    if trimmed == "git rev-parse HEAD" {
        return Some(GitCommandKind::RevParseCommit);
    }
    if trimmed == "git rev-parse --show-object-format" {
        return Some(GitCommandKind::ShowObjectFormat);
    }
    if let Some(rest) = trimmed.strip_prefix("git log ") {
        if let Some(fmt) = rest.strip_prefix("-1 ") {
            return match_string_format_spec(fmt);
        }
        if let Some(after_n) = rest.strip_prefix("-n ") {
            let (n_str, fmt) = after_n.split_once(' ')?;
            if is_valid_log_count(n_str) {
                return match_string_format_spec(fmt);
            }
            return None;
        }
        if let Some(after_mc) = rest.strip_prefix("--max-count=") {
            let (n_str, fmt) = after_mc.split_once(' ')?;
            if is_valid_log_count(n_str) {
                return match_string_format_spec(fmt);
            }
            return None;
        }
        return None;
    }
    if let Some(rest) = trimmed
        .strip_prefix("git show -s ")
        .or_else(|| trimmed.strip_prefix("git show --no-patch "))
    {
        let fmt = rest.strip_suffix(" HEAD")?;
        return match_string_format_spec(fmt);
    }
    if let Some(sha) = trimmed.strip_prefix("git cat-file -p ") {
        if is_canonical_commit_sha(sha) {
            return Some(GitCommandKind::CatFileCommit {
                target_sha: sha.to_string(),
            });
        }
        return None;
    }
    if let Some(rest) = trimmed.strip_prefix("git commit ") {
        if !rest.trim_ascii().is_empty() {
            return Some(GitCommandKind::Commit);
        }
        return None;
    }
    None
}

fn match_exact_git_argv_tokens(tokens: &[&str]) -> Option<GitCommandKind> {
    match tokens {
        ["git", "rev-parse", "HEAD"] => Some(GitCommandKind::RevParseCommit),
        ["git", "rev-parse", "--show-object-format"] => Some(GitCommandKind::ShowObjectFormat),
        ["git", "log", "-1", fmt] => match_argv_format_spec(fmt),
        ["git", "log", "-n", n_str, fmt] if is_valid_log_count(n_str) => {
            match_argv_format_spec(fmt)
        }
        ["git", "log", mc, fmt]
            if mc
                .strip_prefix("--max-count=")
                .is_some_and(is_valid_log_count) =>
        {
            match_argv_format_spec(fmt)
        }
        ["git", "show", "-s" | "--no-patch", fmt, "HEAD"] => match_argv_format_spec(fmt),
        ["git", "cat-file", "-p", sha] if is_canonical_commit_sha(sha) => {
            Some(GitCommandKind::CatFileCommit {
                target_sha: (*sha).to_string(),
            })
        }
        ["git", "commit", rest @ ..] if !rest.is_empty() => Some(GitCommandKind::Commit),
        _ => None,
    }
}

fn argv_mentions_git<S: AsRef<str>>(argv: &[S]) -> bool {
    argv.iter().any(|s| command_mentions_git(s.as_ref()))
}

fn command_mentions_git(cmd: &str) -> bool {
    let mut word = String::new();
    for ch in cmd.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '=') {
            word.push(ch);
        } else if !word.is_empty() {
            if token_is_git_reference(&word) {
                return true;
            }
            word.clear();
        }
    }
    if !word.is_empty() && token_is_git_reference(&word) {
        return true;
    }
    false
}

/// Matches `"git"` / `"git.exe"` and `GIT_*=` environment prefixes case-insensitively for
/// COUNTING rejected git-looking commands (`unparsed_compound_git_calls`) ONLY — never for
/// command acceptance (`match_exact_git_string_template` and `match_exact_git_argv_tokens`
/// require exact lowercase `"git"`).
fn token_is_git_reference(token: &str) -> bool {
    token.eq_ignore_ascii_case("git")
        || token.eq_ignore_ascii_case("git.exe")
        || token
            .get(..8)
            .is_some_and(|p| p.eq_ignore_ascii_case("GIT_DIR="))
        || token
            .get(..14)
            .is_some_and(|p| p.eq_ignore_ascii_case("GIT_WORK_TREE="))
        || token
            .get(..15)
            .is_some_and(|p| p.eq_ignore_ascii_case("GIT_INDEX_FILE="))
        || token
            .get(..21)
            .is_some_and(|p| p.eq_ignore_ascii_case("GIT_OBJECT_DIRECTORY="))
}

/// Normalize a working directory string for deterministic `(session, cwd)` matching.
#[must_use]
pub fn normalize_cwd(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let normalized = trimmed.replace('\\', "/");
    let stripped = if normalized.len() > 1 {
        normalized.trim_end_matches('/')
    } else {
        normalized.as_str()
    };
    (!stripped.is_empty()).then(|| stripped.to_string())
}

/// Returns `true` if `tool_name` is in the strict allow-list of known shell execution tool names
/// across supported harnesses (`Claude`, `Codex`, `Pi`, `Grok`).
/// Unknown or empty tool names always return `false`.
#[must_use]
pub fn is_shell_execution_tool_name(tool_name: &str) -> bool {
    let trimmed = tool_name.trim();
    matches!(
        trimmed,
        "Bash"
            | "bash"
            | "sh"
            | "shell"
            | "CommandExecution"
            | "local_shell"
            | "local_shell_call"
            | "exec_command"
            | "shell_command"
            | "container.exec"
            | "run_command"
            | "execute_command"
    )
}

/// One commit observation extracted from an allow-listed git command's output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCommitObservation {
    pub commit_sha: String,
    /// `Some(parents)` when parents were directly observed (`Vec::new()` for a root commit),
    /// or `None` when parents were not recorded or dropped.
    pub parents: Option<Vec<String>>,
    pub merge_missing_reason: Option<&'static str>,
}

/// Parsed evidence from a single allow-listed git command's output.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ParsedGitOutput {
    pub commits: Vec<ParsedCommitObservation>,
    pub unresolved_abbreviated: u64,
    pub observed_object_format: Option<GitObjectFormat>,
}

/// Git repository object format reported by `git rev-parse --show-object-format`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitObjectFormat {
    Sha1,
    Sha256,
}

/// Parse the output of an allow-listed [`GitCommandKind`].
///
/// Output lines must match the expected shape EXACTLY with NO leading whitespace
/// (trailing CR/LF only). Any non-conforming line makes the WHOLE output contribute nothing.
#[must_use]
pub fn parse_git_output(kind: &GitCommandKind, raw_output: &str) -> ParsedGitOutput {
    let mut out = ParsedGitOutput::default();
    if matches!(kind, GitCommandKind::NonGit) || raw_output.len() > MAX_GIT_OUTPUT_BYTES {
        return out;
    }

    let body = raw_output.trim_end_matches(['\r', '\n']);
    let raw_lines: Vec<&str> = if body.is_empty() {
        Vec::new()
    } else {
        body.split('\n')
            .map(|l| l.strip_suffix('\r').unwrap_or(l))
            .collect()
    };

    if raw_lines.len() > MAX_GIT_LINES_PER_RECORD {
        return out;
    }

    match kind {
        GitCommandKind::NonGit => {}
        GitCommandKind::ShowObjectFormat => {
            if raw_lines.len() == 1 {
                match raw_lines[0] {
                    "sha256" => out.observed_object_format = Some(GitObjectFormat::Sha256),
                    "sha1" => out.observed_object_format = Some(GitObjectFormat::Sha1),
                    _ => {}
                }
            }
        }
        GitCommandKind::Commit => {
            if let Some(&first_line) = raw_lines.first() {
                if is_git_commit_abbreviated_first_line(first_line) {
                    out.unresolved_abbreviated = 1;
                }
            }
        }
        GitCommandKind::RevParseCommit => {
            if raw_lines.len() != 1 {
                return ParsedGitOutput::default();
            }
            let token = raw_lines[0];
            if token.is_empty() || token.contains(char::is_whitespace) {
                return ParsedGitOutput::default();
            }
            if is_canonical_commit_sha(token) {
                out.commits.push(ParsedCommitObservation {
                    commit_sha: token.to_string(),
                    parents: None,
                    merge_missing_reason: Some(MERGE_REASON_PARENTS_NOT_RECORDED),
                });
            } else if is_abbreviated_commit_sha(token) {
                out.unresolved_abbreviated += 1;
            } else {
                return ParsedGitOutput::default();
            }
        }
        GitCommandKind::LogCommitOnly => {
            if raw_lines.is_empty() {
                return ParsedGitOutput::default();
            }
            for &line in &raw_lines {
                if line.is_empty() || line.contains(char::is_whitespace) {
                    return ParsedGitOutput::default();
                }
                if is_canonical_commit_sha(line) {
                    out.commits.push(ParsedCommitObservation {
                        commit_sha: line.to_string(),
                        parents: None,
                        merge_missing_reason: Some(MERGE_REASON_PARENTS_NOT_RECORDED),
                    });
                } else if is_abbreviated_commit_sha(line) {
                    out.unresolved_abbreviated += 1;
                } else {
                    return ParsedGitOutput::default();
                }
            }
        }
        GitCommandKind::LogCommitAndParents => {
            if raw_lines.is_empty() {
                return ParsedGitOutput::default();
            }
            for &line in &raw_lines {
                if line.is_empty() || line.starts_with(char::is_whitespace) {
                    return ParsedGitOutput::default();
                }
                // A root commit in `git log --format='%H %P'` has an empty `%P`, which may leave
                // a single trailing space `<sha> `. Reject any other trailing whitespace.
                let line_clean = line.strip_suffix(' ').unwrap_or(line);
                if line_clean.is_empty() || line_clean.ends_with(char::is_whitespace) {
                    return ParsedGitOutput::default();
                }
                let tokens: Vec<&str> = line_clean.split(' ').collect();
                if tokens.is_empty()
                    || tokens.len() > 1 + MAX_PARENTS_PER_COMMIT
                    || tokens
                        .iter()
                        .any(|t| t.is_empty() || t.contains(char::is_whitespace))
                {
                    return ParsedGitOutput::default();
                }
                if !tokens
                    .iter()
                    .all(|t| is_canonical_commit_sha(t) || is_abbreviated_commit_sha(t))
                {
                    return ParsedGitOutput::default();
                }
                let first = tokens[0];
                let parent_tokens = &tokens[1..];
                if is_abbreviated_commit_sha(first) {
                    out.unresolved_abbreviated += 1;
                    continue;
                }
                if parent_tokens.iter().any(|t| is_abbreviated_commit_sha(t)) {
                    let abbrev_count = parent_tokens
                        .iter()
                        .filter(|t| is_abbreviated_commit_sha(t))
                        .count() as u64;
                    out.unresolved_abbreviated += abbrev_count;
                    out.commits.push(ParsedCommitObservation {
                        commit_sha: first.to_string(),
                        parents: None,
                        merge_missing_reason: Some(MERGE_REASON_ABBREVIATED_PARENTS),
                    });
                } else {
                    let commit_len = first.len();
                    if parent_tokens.iter().all(|t| t.len() == commit_len) {
                        out.commits.push(ParsedCommitObservation {
                            commit_sha: first.to_string(),
                            parents: Some(parent_tokens.iter().map(|s| (*s).to_string()).collect()),
                            merge_missing_reason: None,
                        });
                    } else {
                        out.commits.push(ParsedCommitObservation {
                            commit_sha: first.to_string(),
                            parents: None,
                            merge_missing_reason: Some(MERGE_REASON_MIXED_PARENT_OBJECT_FORMAT),
                        });
                    }
                }
            }
        }
        GitCommandKind::CatFileCommit { target_sha } => {
            if let Some(parsed_cat) = parse_cat_file_commit(&raw_lines, target_sha) {
                out.commits.extend(parsed_cat.commits);
            }
        }
    }

    out
}

fn is_git_commit_abbreviated_first_line(line: &str) -> bool {
    let Some(rest) = line.strip_prefix('[') else {
        return false;
    };
    let Some((bracket_inner, after)) = rest.split_once(']') else {
        return false;
    };
    if !after.is_empty() && !after.starts_with(' ') {
        return false;
    }
    let tokens: Vec<&str> = bracket_inner.split_whitespace().collect();
    let (branch_opt, sha_token) = match tokens.as_slice() {
        [branch, "(root-commit)", sha] => (Some(*branch), *sha),
        ["detached", "HEAD", sha] => (None, *sha),
        [branch, sha] => (Some(*branch), *sha),
        _ => return false,
    };
    if let Some(b) = branch_opt {
        if !is_safe_branch_name(b) {
            return false;
        }
    }
    is_abbreviated_commit_sha(sha_token)
}

/// S1: `git cat-file -p <FULL-SHA>` parses candidate parents for `<FULL-SHA>` ONLY if the output's
/// first line is `tree <hex>` of the exact same hex length, followed only by `parent <hex>` lines
/// of the same length and then the `author ` and `committer ` header lines; anything else
/// contributes nothing.
///
/// Because `git cat-file -p <sha>` also prints raw blob contents (so a blob crafted to look like
/// a commit header could otherwise forge a commit or merge), [`GitEvidenceScanner`] accepts
/// `cat-file` parents ONLY when `<FULL-SHA>` has ALREADY been observed as a commit in the same
/// source (via the `git rev-parse HEAD`, `git log`, or `git show` templates) at an earlier or the
/// same record position; otherwise the `cat-file` output contributes nothing and increments
/// `cat_file_unconfirmed`.
fn parse_cat_file_commit(lines: &[&str], target_sha: &str) -> Option<ParsedGitOutput> {
    if !is_canonical_commit_sha(target_sha) {
        return None;
    }
    let commit_len = target_sha.len();
    let first = *lines.first()?;
    let tree_sha = first.strip_prefix("tree ")?;
    if !is_canonical_commit_sha(tree_sha) || tree_sha.len() != commit_len {
        return None;
    }

    let mut parents = Vec::new();
    let mut idx = 1;
    while idx < lines.len() {
        let Some(p_sha) = lines[idx].strip_prefix("parent ") else {
            break;
        };
        if !is_canonical_commit_sha(p_sha)
            || p_sha.len() != commit_len
            || parents.len() >= MAX_PARENTS_PER_COMMIT
        {
            return None;
        }
        parents.push(p_sha.to_string());
        idx += 1;
    }

    let author_line = *lines.get(idx)?;
    let author_rest = author_line.strip_prefix("author ")?;
    if author_rest.trim().is_empty() {
        return None;
    }
    let committer_line = *lines.get(idx + 1)?;
    let committer_rest = committer_line.strip_prefix("committer ")?;
    if committer_rest.trim().is_empty() {
        return None;
    }

    Some(ParsedGitOutput {
        commits: vec![ParsedCommitObservation {
            commit_sha: target_sha.to_string(),
            parents: Some(parents),
            merge_missing_reason: None,
        }],
        ..Default::default()
    })
}

fn is_safe_branch_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 256
        && !s.starts_with('-')
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'_' | b'-' | b'.'))
}

// ---------------------------------------------------------------------------
// Stateful Per-Source Git Evidence Scanner
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct PendingToolCall {
    command_call: ClassifiedCommandCall,
    cwd: Option<String>,
}

#[derive(Debug, Clone)]
struct AccumulatedCommit {
    commit_sha: String,
    parents: Option<Vec<String>>,
    parents_conflict: bool,
    merge_missing_reason: Option<&'static str>,
    observed_at: Option<DateTime<Utc>>,
    sessions: BTreeSet<String>,
    saw_unmatched_session: bool,
    saw_conflicted_session: bool,
}

/// Accumulates git evidence across records of a single source file pass (`0..consumed`).
#[derive(Debug, Default, Clone)]
pub struct GitEvidenceScanner {
    pending_calls_lru: VecDeque<(String, PendingToolCall)>,
    commits_by_sha: BTreeMap<String, AccumulatedCommit>,
    commit_order: Vec<String>,
    unresolved_abbreviated: u64,
    unverified_object_format: u64,
    unparsed_compound_git_calls: u64,
    cat_file_unconfirmed: u64,
    /// Object format (`sha1` vs `sha256`) observed via `git rev-parse --show-object-format`
    /// per `(session, record_cwd)` (`S1`, `S2`). Requires the record `cwd` to be present.
    object_format_by_session_cwd: BTreeMap<(String, String), GitObjectFormat>,
}

impl GitEvidenceScanner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Count of `git cat-file -p <sha>` commit-shaped outputs rejected because `<sha>` was not
    /// already observed as a commit in the same source at an earlier or the same record position.
    #[must_use]
    pub const fn cat_file_unconfirmed(&self) -> u64 {
        self.cat_file_unconfirmed
    }

    /// Returns `true` if `call_id` (in the pending LRU) or `atomic_command` classifies as
    /// [`GitCommandKind::CatFileCommit`]. Used by multi-block record parsers so commit-producing
    /// outputs (`rev-parse HEAD`, `log`, `show`) at the same record position are recorded before
    /// `cat-file` parent bindings in that same record.
    #[must_use]
    pub fn is_pending_cat_file(
        &self,
        call_id: Option<&str>,
        atomic_command: Option<&GitCommandInput>,
    ) -> bool {
        let Some(id) = call_id.map(str::trim).filter(|s| !s.is_empty()) else {
            return false;
        };
        if let Some((_, pending)) = self
            .pending_calls_lru
            .iter()
            .find(|(existing_id, _)| existing_id == id)
        {
            return matches!(
                pending.command_call,
                ClassifiedCommandCall::Allowed(GitCommandKind::CatFileCommit { .. })
            );
        }
        if let Some(cmd_inp) = atomic_command {
            return matches!(
                classify_git_command_input(cmd_inp),
                ClassifiedCommandCall::Allowed(GitCommandKind::CatFileCommit { .. })
            );
        }
        false
    }

    /// Register a tool call (with a command string or `argv` array) by explicit `call_id` in the
    /// bounded LRU.
    ///
    /// Only tool names in the strict shell allow-list (`is_shell_execution_tool_name`) have their
    /// command inspected; unknown or absent tool names are treated as non-shell tools.
    pub fn observe_tool_call_input_with_context(
        &mut self,
        call_id: Option<&str>,
        tool_name: Option<&str>,
        command_input: Option<&GitCommandInput>,
        cwd: Option<&str>,
    ) {
        let Some(id) = call_id.map(str::trim).filter(|s| !s.is_empty()) else {
            return;
        };
        let is_shell = tool_name.is_some_and(is_shell_execution_tool_name);
        let norm_call_cwd = cwd.and_then(normalize_cwd);
        let command_call = if is_shell {
            match command_input {
                Some(inp) => {
                    let classified = classify_git_command_input(inp);
                    if matches!(classified, ClassifiedCommandCall::SkippedCompoundGit) {
                        self.unparsed_compound_git_calls += 1;
                    }
                    classified
                }
                None => ClassifiedCommandCall::NonGit,
            }
        } else {
            ClassifiedCommandCall::NonGit
        };

        let pending = PendingToolCall {
            command_call,
            cwd: norm_call_cwd,
        };
        self.insert_pending_lru(id.to_string(), pending);
    }

    /// Register a tool call with a command string by explicit `call_id` in the bounded LRU.
    pub fn observe_tool_call_with_context(
        &mut self,
        call_id: Option<&str>,
        tool_name: Option<&str>,
        command: Option<&str>,
        cwd: Option<&str>,
    ) {
        let input = command.map(|c| GitCommandInput::CommandString(c.to_string()));
        self.observe_tool_call_input_with_context(call_id, tool_name, input.as_ref(), cwd);
    }

    /// Register a tool call with an `argv` array by explicit `call_id` in the bounded LRU.
    pub fn observe_tool_call_argv_with_context<S: AsRef<str>>(
        &mut self,
        call_id: Option<&str>,
        tool_name: Option<&str>,
        argv: &[S],
        cwd: Option<&str>,
    ) {
        let input = GitCommandInput::Argv(argv.iter().map(|s| s.as_ref().to_string()).collect());
        self.observe_tool_call_input_with_context(call_id, tool_name, Some(&input), cwd);
    }

    /// Convenience wrapper for registering a shell (`Bash`) tool call without explicit `cwd`.
    pub fn observe_tool_call(&mut self, call_id: Option<&str>, command: &str) {
        self.observe_tool_call_with_context(call_id, Some("Bash"), Some(command), None);
    }

    fn insert_pending_lru(&mut self, call_id: String, pending: PendingToolCall) {
        if let Some(pos) = self
            .pending_calls_lru
            .iter()
            .position(|(existing_id, _)| existing_id == &call_id)
        {
            self.pending_calls_lru.remove(pos);
        } else if self.pending_calls_lru.len() >= MAX_PENDING_CALLS_PER_SOURCE {
            self.pending_calls_lru.pop_front();
        }
        self.pending_calls_lru.push_back((call_id, pending));
    }

    fn lookup_pending_lru(&mut self, call_id: &str) -> Option<PendingToolCall> {
        let pos = self
            .pending_calls_lru
            .iter()
            .position(|(existing_id, _)| existing_id == call_id)?;
        let (id, call) = self.pending_calls_lru.remove(pos)?;
        let cloned = call.clone();
        self.pending_calls_lru.push_back((id, call));
        Some(cloned)
    }

    /// Record a tool result matched strictly by explicit `call_id` (or atomic `CommandExecution`
    /// record where call and output are in the same event).
    ///
    /// Per `S1`, uses ONLY the record's own `cwd` field (no `cd` handling, no session fallback).
    /// Evidence that needs a `cwd` (the object-format binding) requires the record `cwd` to be present.
    #[allow(clippy::too_many_arguments)]
    pub fn observe_tool_output_input_with_cwd(
        &mut self,
        call_id: Option<&str>,
        atomic_command: Option<&GitCommandInput>,
        raw_session: Option<&str>,
        header_conflict: bool,
        ts: Option<DateTime<Utc>>,
        failed: Option<bool>,
        cwd: Option<&str>,
        raw_output: &str,
    ) {
        if failed == Some(true) {
            return;
        }
        let clean_session = if header_conflict {
            None
        } else {
            raw_session.map(str::trim).filter(|s| !s.is_empty())
        };
        let norm_record_cwd = cwd.and_then(normalize_cwd);

        let Some(id) = call_id.map(str::trim).filter(|s| !s.is_empty()) else {
            return;
        };

        let pending = if let Some(found) = self.lookup_pending_lru(id) {
            found
        } else if let Some(cmd_inp) = atomic_command {
            let command_call = classify_git_command_input(cmd_inp);
            if matches!(command_call, ClassifiedCommandCall::SkippedCompoundGit) {
                self.unparsed_compound_git_calls += 1;
            }
            let call = PendingToolCall {
                command_call,
                cwd: norm_record_cwd.clone(),
            };
            self.insert_pending_lru(id.to_string(), call.clone());
            call
        } else {
            return;
        };

        let ClassifiedCommandCall::Allowed(kind) = pending.command_call else {
            return;
        };

        // S1: ONLY the record's own cwd field (no cd handling, no session fallback).
        // If both the call record and output record carry a cwd, they must agree; if the output
        // record has no cwd, effective_cwd is None.
        let effective_cwd = match (&pending.cwd, norm_record_cwd) {
            (Some(call_cwd), Some(out_cwd)) if *call_cwd == out_cwd => Some(out_cwd),
            (None, Some(out_cwd)) => Some(out_cwd),
            _ => None,
        };

        let is_cat_file = matches!(kind, GitCommandKind::CatFileCommit { .. });
        let parsed = parse_git_output(&kind, raw_output);
        self.unresolved_abbreviated += parsed.unresolved_abbreviated;

        // S1 & S2: Object format binding requires both session and record cwd to be present.
        if let Some(fmt) = parsed.observed_object_format {
            if let (Some(sess), Some(ref c)) = (clean_session, &effective_cwd) {
                self.object_format_by_session_cwd
                    .insert((sess.to_string(), c.clone()), fmt);
            }
        }

        for obs in parsed.commits {
            self.record_commit_observation(
                raw_session,
                header_conflict,
                ts,
                effective_cwd.as_deref(),
                is_cat_file,
                obs,
            );
        }
    }

    /// Record a tool result with an optional atomic command string matched strictly by `call_id`.
    #[allow(clippy::too_many_arguments)]
    pub fn observe_tool_output_with_cwd(
        &mut self,
        call_id: Option<&str>,
        atomic_command: Option<&str>,
        raw_session: Option<&str>,
        header_conflict: bool,
        ts: Option<DateTime<Utc>>,
        failed: Option<bool>,
        cwd: Option<&str>,
        raw_output: &str,
    ) {
        let inp = atomic_command.map(|c| GitCommandInput::CommandString(c.to_string()));
        self.observe_tool_output_input_with_cwd(
            call_id,
            inp.as_ref(),
            raw_session,
            header_conflict,
            ts,
            failed,
            cwd,
            raw_output,
        );
    }

    /// Convenience wrapper for [`Self::observe_tool_output_with_cwd`] without explicit record `cwd`.
    #[allow(clippy::too_many_arguments)]
    pub fn observe_tool_output(
        &mut self,
        call_id: Option<&str>,
        atomic_command: Option<&str>,
        raw_session: Option<&str>,
        header_conflict: bool,
        ts: Option<DateTime<Utc>>,
        failed: Option<bool>,
        raw_output: &str,
    ) {
        self.observe_tool_output_with_cwd(
            call_id,
            atomic_command,
            raw_session,
            header_conflict,
            ts,
            failed,
            None,
            raw_output,
        );
    }

    fn record_commit_observation(
        &mut self,
        raw_session: Option<&str>,
        header_conflict: bool,
        ts: Option<DateTime<Utc>>,
        effective_cwd: Option<&str>,
        is_cat_file: bool,
        obs: ParsedCommitObservation,
    ) {
        if !is_canonical_commit_sha(&obs.commit_sha) {
            return;
        }
        // `git cat-file -p <sha>` prints raw blob contents, so a blob crafted to look like a
        // commit header could forge a merge commit. Accept `cat-file` parents ONLY for a SHA
        // already observed as a COMMIT in the same source (via `rev-parse HEAD`, `log`, or `show`)
        // at an earlier or the same record position; otherwise contribute nothing and count it in
        // `cat_file_unconfirmed`.
        if is_cat_file && !self.commits_by_sha.contains_key(&obs.commit_sha) {
            self.cat_file_unconfirmed += 1;
            return;
        }
        let clean_session = if header_conflict {
            None
        } else {
            raw_session.map(str::trim).filter(|s| !s.is_empty())
        };
        let norm_cwd = effective_cwd.and_then(normalize_cwd);

        // S1 & S2: Object format check per `(session, record_cwd)`:
        // - 40-hex is accepted as SHA-1 unless `sha256` was recorded for the same session and record cwd.
        // - 64-hex is accepted ONLY when `sha256` was recorded for the same session and record cwd;
        //   otherwise it is counted as `unverified_object_format` and ignored.
        let recorded_format = match (clean_session, norm_cwd.as_deref()) {
            (Some(sess), Some(c)) => self
                .object_format_by_session_cwd
                .get(&(sess.to_string(), c.to_string()))
                .copied(),
            _ => None,
        }
        .unwrap_or(GitObjectFormat::Sha1);

        let sha_len = obs.commit_sha.len();
        let format_matches = matches!(
            (sha_len, recorded_format),
            (40, GitObjectFormat::Sha1) | (64, GitObjectFormat::Sha256)
        );
        if !format_matches {
            self.unverified_object_format += 1;
            return;
        }

        if let Some(existing) = self.commits_by_sha.get_mut(&obs.commit_sha) {
            if existing.observed_at.is_none() && ts.is_some() {
                existing.observed_at = ts;
            }
            // S4: Within ONE source, conflicting parent lists for the same commit ->
            // merge_status = not_observed with reason conflicting_parents (not first-wins).
            if !existing.parents_conflict {
                if let Some(new_parents) = obs.parents {
                    if let Some(ref cur_parents) = existing.parents {
                        if *cur_parents != new_parents {
                            existing.parents = None;
                            existing.parents_conflict = true;
                            existing.merge_missing_reason = Some(MERGE_REASON_CONFLICTING_PARENTS);
                        }
                    } else {
                        existing.parents = Some(new_parents);
                        existing.merge_missing_reason = None;
                    }
                } else if existing.parents.is_none()
                    && (existing.merge_missing_reason.is_none()
                        || (existing.merge_missing_reason
                            == Some(MERGE_REASON_PARENTS_NOT_RECORDED)
                            && obs.merge_missing_reason.is_some()))
                {
                    existing.merge_missing_reason = obs.merge_missing_reason;
                }
            }
            if header_conflict {
                existing.saw_conflicted_session = true;
            } else if let Some(s) = clean_session {
                existing.sessions.insert(s.to_string());
            } else {
                existing.saw_unmatched_session = true;
            }
            return;
        }

        if self.commits_by_sha.len() >= MAX_OBSERVED_COMMITS_PER_SOURCE {
            return;
        }

        let mut sessions = BTreeSet::new();
        let mut saw_unmatched_session = false;
        let mut saw_conflicted_session = false;
        if header_conflict {
            saw_conflicted_session = true;
        } else if let Some(s) = clean_session {
            sessions.insert(s.to_string());
        } else {
            saw_unmatched_session = true;
        }

        let sha = obs.commit_sha.clone();
        self.commit_order.push(sha.clone());
        self.commits_by_sha.insert(
            sha,
            AccumulatedCommit {
                commit_sha: obs.commit_sha,
                parents: obs.parents,
                parents_conflict: false,
                merge_missing_reason: obs.merge_missing_reason,
                observed_at: ts,
                sessions,
                saw_unmatched_session,
                saw_conflicted_session,
            },
        );
    }

    /// Materialize the per-source `GitEvidenceFact` rows and `GitEvidenceCoverage` counters
    /// for the prefix observed so far.
    #[allow(clippy::too_many_arguments)]
    pub fn materialize<F>(
        &self,
        source_id: &str,
        generation: i64,
        harness: &str,
        header_conflict: bool,
        tool_key: Option<&[u8]>,
        export_config: &FactExportConfig,
        seal_session_fn: F,
    ) -> (Vec<GitEvidenceFact>, GitEvidenceCoverage)
    where
        F: Fn(&str) -> String,
    {
        let canonical_harness = CanonicalHarness::from_alias(harness)
            .map(|h| h.as_str().to_string())
            .unwrap_or_else(|| "unknown".to_string());

        let mut facts = Vec::with_capacity(self.commit_order.len());
        let mut missing_parents = 0i64;
        let mut ambiguous = 0i64;
        let mut unmatched = 0i64;
        let mut default_branch_not_observed = 0i64;

        for sha in &self.commit_order {
            let Some(acc) = self.commits_by_sha.get(sha) else {
                continue;
            };

            let commit_sealed = seal_commit(tool_key, &acc.commit_sha);
            let source_key = format!("{source_id}:git:{commit_sealed}");

            // Merge status & parents
            let (merge_status, is_merge, merge_missing_reason, parent_sealed) = match &acc.parents {
                Some(parents) => {
                    let sealed_parents: Vec<String> =
                        parents.iter().map(|p| seal_commit(tool_key, p)).collect();
                    (
                        MERGE_STATUS_OBSERVED.to_string(),
                        Some(parents.len() > 1),
                        None,
                        sealed_parents,
                    )
                }
                None => {
                    missing_parents += 1;
                    (
                        MERGE_STATUS_NOT_OBSERVED.to_string(),
                        None,
                        Some(
                            acc.merge_missing_reason
                                .unwrap_or(MERGE_REASON_PARENTS_NOT_RECORDED)
                                .to_string(),
                        ),
                        Vec::new(),
                    )
                }
            };

            // S3: Default branch status is ALWAYS `not_observed` in the MVP (status-only
            // representation, consistent with the `fact-export-v1` contract's fields).
            default_branch_not_observed += 1;
            let default_branch_status = DEFAULT_BRANCH_STATUS_NOT_OBSERVED.to_string();
            let on_default_branch = None;

            // Timestamp status
            let observed_at_status = if acc.observed_at.is_some() {
                OBSERVED_AT_STATUS_OBSERVED.to_string()
            } else {
                OBSERVED_AT_STATUS_NOT_OBSERVABLE.to_string()
            };

            // Session match status ("matched" requires a non-empty session identity; never pick one)
            let (session_match_status, native_session) =
                if header_conflict || acc.saw_conflicted_session || acc.sessions.len() > 1 {
                    ambiguous += 1;
                    (SESSION_MATCH_AMBIGUOUS.to_string(), None)
                } else if acc.sessions.len() == 1 {
                    let raw_s = acc.sessions.iter().next().unwrap().as_str();
                    let sealed_s = seal_session_fn(raw_s);
                    if sealed_s.trim().is_empty() {
                        unmatched += 1;
                        (SESSION_MATCH_UNMATCHED.to_string(), None)
                    } else {
                        (SESSION_MATCH_MATCHED.to_string(), Some(sealed_s))
                    }
                } else if acc.saw_unmatched_session {
                    unmatched += 1;
                    (SESSION_MATCH_UNMATCHED.to_string(), None)
                } else {
                    unmatched += 1;
                    (SESSION_MATCH_UNKNOWN.to_string(), None)
                };

            // Fact-export commit pseudonym columns (computed only when key material is Ready)
            let (export_status, pseudonym_key_id, commit_hmac, parent_hmacs) =
                if export_config.is_ready() {
                    let key_id = export_config.pseudonym_key_id().map(str::to_string);
                    let c_hmac = export_config
                        .derive_commit_hmac(&acc.commit_sha)
                        .expect("canonical commit SHA");
                    let p_hmacs: Vec<String> = acc
                        .parents
                        .as_ref()
                        .map(|ps| {
                            ps.iter()
                                .filter_map(|p| export_config.derive_commit_hmac(p))
                                .collect()
                        })
                        .unwrap_or_default();
                    (
                        EXPORT_STATUS_EXPORTABLE.to_string(),
                        key_id,
                        Some(c_hmac),
                        p_hmacs,
                    )
                } else {
                    (
                        EXPORT_STATUS_NON_EXPORTABLE.to_string(),
                        None,
                        None,
                        Vec::new(),
                    )
                };

            facts.push(GitEvidenceFact {
                source_key,
                source_id: source_id.to_string(),
                generation,
                harness: harness.to_string(),
                canonical_harness: canonical_harness.clone(),
                native_session,
                commit_sealed,
                parent_sealed,
                is_merge,
                merge_status,
                merge_missing_reason,
                on_default_branch,
                default_branch_status,
                observed_at: acc.observed_at,
                observed_at_status,
                session_match_status,
                evidence_source: EVIDENCE_SOURCE_LOCAL_GIT_HISTORY.to_string(),
                export_status,
                pseudonym_key_id,
                commit_hmac,
                parent_hmacs,
            });
        }

        let coverage = GitEvidenceCoverage {
            source_id: source_id.to_string(),
            generation,
            harness: harness.to_string(),
            observed: facts.len() as i64,
            unresolved_abbreviated: self.unresolved_abbreviated as i64,
            unverified_object_format: self.unverified_object_format as i64,
            missing_parents,
            ambiguous,
            unmatched,
            default_branch_not_observed,
            unparsed_compound_git_calls: self.unparsed_compound_git_calls as i64,
            cat_file_unconfirmed: self.cat_file_unconfirmed as i64,
        };

        (facts, coverage)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link_id::{consumer_env_path_from_vars, read_consumer_env_from};

    #[test]
    fn contract_merge_evidence_vectors_match_normative_python_reference() {
        let cfg = FactExportConfig::from_test_vector_key_unchecked(
            b"fact-export-v1-public-test-key",
            "kh_customer_v1",
        );
        assert!(cfg.is_ready());

        // Source & session pseudonyms from `generate_fact_vectors.py` / `merge_evidence.json`
        let source_hmac = cfg.derive_source_id("fixture-source-a").unwrap();
        assert_eq!(
            source_hmac,
            "89c03e5ffaf28d96226951dca8fd2e9132c26041f1985f1507269d701171f192"
        );
        // Verify 64-hex logical_source_id is ALSO HMACed (no 64-hex passthrough!)
        let hex64_logical = "a".repeat(64);
        let hex64_hmac = cfg.derive_source_id(&hex64_logical).unwrap();
        assert_ne!(hex64_hmac, hex64_logical);

        let sess_a = cfg
            .derive_session_key("fixture-source-a", "codex", "codex-a")
            .unwrap();
        assert_eq!(
            sess_a,
            "a611d713964de4e128bb2c164ae859a338454881afd7ce079a2afb34d562169d"
        );

        let commit_a = cfg.derive_commit_hmac(&"a".repeat(40)).unwrap();
        let parent_b = cfg.derive_commit_hmac(&"b".repeat(40)).unwrap();
        let parent_c = cfg.derive_commit_hmac(&"c".repeat(40)).unwrap();
        let commit_d40 = cfg.derive_commit_hmac(&"d".repeat(40)).unwrap();
        let commit_d64 = cfg.derive_commit_hmac(&"d".repeat(64)).unwrap();
        assert_eq!(
            commit_a,
            "bc07f227444a11a858de0f4321da9826c54d4c5def78f1110d799ee523e11137"
        );
        assert_eq!(
            parent_b,
            "fc234836df3c3fdda2c9fba641de25da4385401bccb4b72c42ab186026fb1138"
        );
        assert_eq!(
            parent_c,
            "ea0c2c4cac64a682cd4e0536629f337406009c747f5c5d04d316fbdecb4e097b"
        );
        assert_eq!(
            commit_d40,
            "fc2ca46bc3253793e1bbbf308ba546183ef796657c761e2b127c7f6ce2f8b6f8"
        );
        assert_eq!(
            commit_d64,
            "64b5f1c4221dd313c441f42af4cf19f248f862f6898e51aaadb9df915d01c47b"
        );

        let ev_key = cfg
            .derive_git_evidence_key(
                "fixture-source-a",
                &commit_a,
                Some(1_762_900_000_000_000),
                Some(&sess_a),
                "matched",
            )
            .unwrap();
        assert_eq!(
            ev_key,
            "39d45b850aaf27008aa7b5462dc26bff9a1388e5898cb28b2045b9c2227fe742"
        );

        let ambig_ev_key = cfg
            .derive_git_evidence_key(
                "fixture-source-a",
                &parent_c,
                Some(1_762_899_999_999_998),
                None,
                "ambiguous",
            )
            .unwrap();
        assert_eq!(
            ambig_ev_key,
            "91bb6639e10a93de5707a1000cb29a708bfed6a1995aae408dcd153cd6adab3c"
        );
    }

    #[test]
    fn canonical_and_abbreviated_sha_validation() {
        assert!(is_canonical_commit_sha(&"0123456789abcdef".repeat(3)[..40]));
        assert!(is_canonical_commit_sha(&"a".repeat(40)));
        assert!(is_canonical_commit_sha(&"f".repeat(64)));
        assert!(!is_canonical_commit_sha(&"A".repeat(40)));
        assert!(!is_canonical_commit_sha("abc1234"));
        assert!(!is_canonical_commit_sha(&"g".repeat(40)));
        assert!(is_abbreviated_commit_sha("abc1234"));
        assert!(is_abbreviated_commit_sha(&"a".repeat(12)));
        assert!(is_abbreviated_commit_sha(&"a".repeat(39)));
        assert!(!is_abbreviated_commit_sha(&"a".repeat(40)));
        assert!(!is_abbreviated_commit_sha("abc"));
    }

    #[test]
    fn s1_exact_templates_accepted_with_expected_rows_and_argv_variants() {
        let sha1 = "1".repeat(40);
        let sha2 = "2".repeat(40);
        let sha3 = "3".repeat(40);
        let tree_sha = "4".repeat(40);

        // 1. `git rev-parse HEAD`
        assert_eq!(
            classify_shell_command("  git rev-parse HEAD  "),
            GitCommandKind::RevParseCommit
        );
        let out_rp = parse_git_output(&GitCommandKind::RevParseCommit, &format!("{sha1}\n"));
        assert_eq!(out_rp.commits.len(), 1);
        assert_eq!(out_rp.commits[0].commit_sha, sha1);
        assert_eq!(out_rp.commits[0].parents, None);

        // 2. `git rev-parse --show-object-format`
        assert_eq!(
            classify_shell_command("git rev-parse --show-object-format"),
            GitCommandKind::ShowObjectFormat
        );
        let out_fmt = parse_git_output(&GitCommandKind::ShowObjectFormat, "sha256\n");
        assert_eq!(
            out_fmt.observed_object_format,
            Some(GitObjectFormat::Sha256)
        );

        // 3. `git log` CommitOnly templates (N = 1..=1000)
        for cmd in [
            "git log -1 --format=%H",
            "git log -n 1 --format=%H",
            "git log -n 1000 --format=%H",
            "git log --max-count=1 --format=%H",
            "git log --max-count=1000 --format=%H",
            "git log -1 --pretty=format:%H",
            "git log -n 5 --pretty=format:%H",
            "git log --max-count=5 --pretty=format:%H",
        ] {
            let kind = classify_shell_command(cmd);
            assert_eq!(kind, GitCommandKind::LogCommitOnly, "must accept `{cmd}`");
            let out = parse_git_output(&kind, &format!("{sha1}\n{sha2}\n"));
            assert_eq!(out.commits.len(), 2);
            assert_eq!(out.commits[0].commit_sha, sha1);
            assert_eq!(out.commits[0].parents, None);
            assert_eq!(out.commits[1].commit_sha, sha2);
        }

        // 4. `git log` CommitAndParents templates (Q = ' or ", matching on both sides)
        for cmd in [
            "git log -1 --format='%H %P'",
            "git log -1 --format=\"%H %P\"",
            "git log -n 2 --format='%H %P'",
            "git log -n 1000 --format=\"%H %P\"",
            "git log --max-count=3 --format='%H %P'",
            "git log --max-count=1000 --format=\"%H %P\"",
            "git log -1 --pretty=format:'%H %P'",
            "git log -n 2 --pretty=format:\"%H %P\"",
            "git log --max-count=2 --pretty=format:'%H %P'",
        ] {
            let kind = classify_shell_command(cmd);
            assert_eq!(
                kind,
                GitCommandKind::LogCommitAndParents,
                "must accept `{cmd}`"
            );
            let out = parse_git_output(&kind, &format!("{sha1} {sha2} {sha3}\n{sha2}\n"));
            assert_eq!(out.commits.len(), 2);
            assert_eq!(
                out.commits[0].parents,
                Some(vec![sha2.clone(), sha3.clone()])
            );
            assert_eq!(out.commits[1].parents, Some(Vec::new()));
        }

        // 5. `git show` templates (`-s` and `--no-patch`)
        for (cmd, expected_kind) in [
            (
                "git show -s --format=%H HEAD",
                GitCommandKind::LogCommitOnly,
            ),
            (
                "git show --no-patch --format=%H HEAD",
                GitCommandKind::LogCommitOnly,
            ),
            (
                "git show -s --pretty=format:%H HEAD",
                GitCommandKind::LogCommitOnly,
            ),
            (
                "git show -s --format='%H %P' HEAD",
                GitCommandKind::LogCommitAndParents,
            ),
            (
                "git show -s --format=\"%H %P\" HEAD",
                GitCommandKind::LogCommitAndParents,
            ),
            (
                "git show --no-patch --format='%H %P' HEAD",
                GitCommandKind::LogCommitAndParents,
            ),
            (
                "git show --no-patch --format=\"%H %P\" HEAD",
                GitCommandKind::LogCommitAndParents,
            ),
        ] {
            let kind = classify_shell_command(cmd);
            assert_eq!(kind, expected_kind, "must accept `{cmd}`");
        }

        // 6. `git cat-file -p <FULL-SHA>` with required `tree <hex>`, `parent <hex>`, `author `, `committer `
        let cat_cmd = format!("git cat-file -p {sha1}");
        let cat_kind = classify_shell_command(&cat_cmd);
        assert_eq!(
            cat_kind,
            GitCommandKind::CatFileCommit {
                target_sha: sha1.clone()
            }
        );
        let valid_cat_body = format!(
            "tree {tree_sha}\nparent {sha2}\nauthor Dev <dev@example.com> 1700000000 +0000\ncommitter Dev <dev@example.com> 1700000000 +0000\n\ncommit msg\n"
        );
        let out_cat = parse_git_output(&cat_kind, &valid_cat_body);
        assert_eq!(out_cat.commits.len(), 1);
        assert_eq!(out_cat.commits[0].commit_sha, sha1);
        assert_eq!(out_cat.commits[0].parents, Some(vec![sha2.clone()]));

        // `cat-file -p` missing author/committer or with wrong-length tree SHA contributes nothing
        let cat_no_author = format!("tree {tree_sha}\nparent {sha2}\n\nmsg\n");
        assert!(parse_git_output(&cat_kind, &cat_no_author)
            .commits
            .is_empty());
        let cat_extra_header = format!(
            "tree {tree_sha}\nparent {sha2}\nencoding UTF-8\nauthor Dev <d@e> 1 +0000\ncommitter Dev <d@e> 1 +0000\n"
        );
        assert!(parse_git_output(&cat_kind, &cat_extra_header)
            .commits
            .is_empty());
        let tree_64 = "4".repeat(64);
        let cat_wrong_tree_len = format!(
            "tree {tree_64}\nparent {sha2}\nauthor Dev <d@e> 1 +0000\ncommitter Dev <d@e> 1 +0000\n"
        );
        assert!(parse_git_output(&cat_kind, &cat_wrong_tree_len)
            .commits
            .is_empty());

        // 7. `git commit ...` -> counted as unresolved_abbreviated only (no evidence row)
        let commit_kind = classify_shell_command("git commit -m 'feat: simple'");
        assert_eq!(commit_kind, GitCommandKind::Commit);
        let out_commit = parse_git_output(
            &commit_kind,
            "[main abc1234] feat: simple\n 1 file changed\n",
        );
        assert!(out_commit.commits.is_empty());
        assert_eq!(out_commit.unresolved_abbreviated, 1);

        // 8. Argv arrays (Codex): exact token match + allowed 3-element wrappers
        assert_eq!(
            classify_git_argv(&["git", "rev-parse", "HEAD"]),
            ClassifiedCommandCall::Allowed(GitCommandKind::RevParseCommit)
        );
        assert_eq!(
            classify_git_argv(&["git", "rev-parse", "--show-object-format"]),
            ClassifiedCommandCall::Allowed(GitCommandKind::ShowObjectFormat)
        );
        assert_eq!(
            classify_git_argv(&["git", "log", "-1", "--format=%H %P"]),
            ClassifiedCommandCall::Allowed(GitCommandKind::LogCommitAndParents)
        );
        assert_eq!(
            classify_git_argv(&["git", "log", "-n", "10", "--format=%H"]),
            ClassifiedCommandCall::Allowed(GitCommandKind::LogCommitOnly)
        );
        assert_eq!(
            classify_git_argv(&["git", "show", "-s", "--format=%H %P", "HEAD"]),
            ClassifiedCommandCall::Allowed(GitCommandKind::LogCommitAndParents)
        );
        assert_eq!(
            classify_git_argv(&["bash", "-lc", "git rev-parse HEAD"]),
            ClassifiedCommandCall::Allowed(GitCommandKind::RevParseCommit)
        );
        assert_eq!(
            classify_git_argv(&["sh", "-c", "git log -1 --format='%H %P'"]),
            ClassifiedCommandCall::Allowed(GitCommandKind::LogCommitAndParents)
        );
        assert_eq!(
            classify_git_argv(&["zsh", "-lc", "git show --no-patch --format=%H HEAD"]),
            ClassifiedCommandCall::Allowed(GitCommandKind::LogCommitOnly)
        );
        assert_eq!(
            classify_git_argv(&["pwsh", "-Command", "git rev-parse HEAD"]),
            ClassifiedCommandCall::Allowed(GitCommandKind::RevParseCommit)
        );
        assert_eq!(
            classify_git_argv(&["powershell", "-c", "git log -n 5 --format=\"%H %P\""]),
            ClassifiedCommandCall::Allowed(GitCommandKind::LogCommitAndParents)
        );
    }

    #[test]
    fn s1_to_s5_reviewer_attacks_rejected_and_counted_unparsed_intra_source_conflict_and_exportability(
    ) {
        let sha1 = "a".repeat(40);
        let sha2 = "b".repeat(40);
        let sha3 = "c".repeat(40);

        let mut scanner = GitEvidenceScanner::new();

        // Reviewer's string attacks (all rejected, emit 0 rows, and increment unparsed_compound_git_calls):
        let string_attacks = [
            // `cd $'...'` forgery
            "cd $'/repo' && git rev-parse HEAD",
            "cd $'/repo\\n' && git rev-parse HEAD",
            // PowerShell `\"` forgery
            "powershell -Command \"git rev-parse \\\"HEAD\\\"\"",
            "git log -1 --format=\"\\\"%H %P\\\"\"",
            // `checkout main -- file`
            "git checkout main -- file.txt",
            "git checkout main",
            // `rev-list` with paths (and without paths)
            "git rev-list --parents HEAD -- src/lib.rs",
            "git rev-list --parents HEAD src/lib.rs",
            "git rev-list --parents HEAD",
            // `log ... stash`
            "git log -1 --format=%H stash",
            "git log -1 --format='%H %P' refs/stash",
            "git log -1 --format=%H HEAD",
            // `show -s --format=%H <blob>`
            "git show -s --format=%H deadbeefcafe0123456789abcdef0123456789ab",
            "git show -s --format=%H HEAD:src/lib.rs",
            "git show --format=%H HEAD",
            // `cmd /c` and string shell wrappers
            "cmd /c git rev-parse HEAD",
            "cmd.exe /c git rev-parse HEAD",
            "bash -lc 'git rev-parse HEAD'",
            // Any `$`, `\`, `;`, `&&`, `|`, backtick, `<`, `>`, `(`, `)`, newline, tab
            "git rev-parse $HEAD",
            "git\\ rev-parse HEAD",
            "git rev-parse HEAD; git status",
            "git rev-parse HEAD && true",
            "git rev-parse HEAD | cat",
            "git rev-parse `echo HEAD`",
            "git rev-parse HEAD > /tmp/out",
            "git rev-parse HEAD < /dev/null",
            "(git rev-parse HEAD)",
            "git rev-parse HEAD\ngit status",
            "git\trev-parse HEAD",
            // Out-of-range N or mismatched quotes
            "git log -n 0 --format=%H",
            "git log -n 1001 --format=%H",
            "git log -n 01 --format=%H",
            "git log -1 --format='%H %P\"",
            // Uppercase / mixed-case `GIT ...` commands (counted in unparsed_compound_git_calls, never accepted)
            "GIT rev-parse HEAD",
            "Git log -1 --format=%H",
            "GIT.EXE rev-parse HEAD",
            "git_dir=/other/.git git rev-parse HEAD",
        ];

        for (i, cmd) in string_attacks.iter().enumerate() {
            assert_eq!(
                classify_single_git_command(cmd),
                ClassifiedCommandCall::SkippedCompoundGit,
                "string attack `{cmd}` must be rejected as SkippedCompoundGit"
            );
            let cid = format!("atk-str-{i}");
            scanner.observe_tool_call_with_context(
                Some(&cid),
                Some("Bash"),
                Some(cmd),
                Some("/repo"),
            );
            scanner.observe_tool_output_with_cwd(
                Some(&cid),
                None,
                Some("sess-atk"),
                false,
                None,
                Some(false),
                Some("/repo"),
                &format!("{sha1} {sha2}\n"),
            );
        }

        // Reviewer's argv attacks:
        // - argv `bash -lc` with a joined compound string or extra args
        // - argv with joined tokens `["git", "rev-parse HEAD"]`
        // - argv `cmd /c`
        // - argv PowerShell `\"` forgery
        let argv_attacks: &[&[&str]] = &[
            &["bash", "-lc", "cd /repo && git rev-parse HEAD"],
            &["bash", "-lc", "git rev-parse HEAD; git log -1 --format=%H"],
            &["bash", "-lc", "git", "rev-parse", "HEAD"],
            &["git", "rev-parse HEAD"],
            &["git", "log -1 --format=%H"],
            &["cmd", "/c", "git rev-parse HEAD"],
            &["cmd.exe", "/c", "git rev-parse HEAD"],
            &["pwsh", "-Command", "git rev-parse \"HEAD\""],
            &["powershell", "-NoProfile", "-Command", "git rev-parse HEAD"],
            &["git", "show", "-s", "--format=%H", &sha1],
            &["git", "rev-list", "--parents", "HEAD", "--", "file.rs"],
            &["git", "checkout", "main", "--", "file.rs"],
            &["git", "log", "-1", "--format=%H", "stash"],
            // Uppercase / mixed-case `GIT` in argv (counted in unparsed_compound_git_calls, never accepted)
            &["GIT", "rev-parse", "HEAD"],
            &["Git", "log", "-1", "--format=%H"],
            &["GIT.EXE", "rev-parse", "HEAD"],
            // Dead literal-quote format specs in argv (rejected, never accepted)
            &["git", "log", "-1", "--format='%H %P'"],
            &["git", "log", "-1", "--format=\"%H %P\""],
            &["git", "log", "-1", "--pretty=format:'%H %P'"],
            &["git", "log", "-1", "--pretty=format:\"%H %P\""],
            &["git", "show", "-s", "--format='%H %P'", "HEAD"],
            &["git", "show", "-s", "--format=\"%H %P\"", "HEAD"],
        ];

        for (i, argv) in argv_attacks.iter().enumerate() {
            assert_eq!(
                classify_git_argv(argv),
                ClassifiedCommandCall::SkippedCompoundGit,
                "argv attack `{argv:?}` must be rejected as SkippedCompoundGit"
            );
            let cid = format!("atk-argv-{i}");
            scanner.observe_tool_call_argv_with_context(
                Some(&cid),
                Some("CommandExecution"),
                argv,
                Some("/repo"),
            );
            scanner.observe_tool_output_with_cwd(
                Some(&cid),
                None,
                Some("sess-atk"),
                false,
                None,
                Some(false),
                Some("/repo"),
                &format!("{sha1}\n"),
            );
        }

        let (atk_facts, atk_cov) = scanner.materialize(
            "src-atk",
            1,
            "claude",
            false,
            Some(b"tool-key"),
            &FactExportConfig::unconfigured(),
            |s| format!("sealed:{s}"),
        );
        assert!(atk_facts.is_empty(), "all attacks must produce 0 rows");
        assert_eq!(
            atk_cov.unparsed_compound_git_calls,
            (string_attacks.len() + argv_attacks.len()) as i64,
            "every rejected git attack must increment unparsed_compound_git_calls"
        );

        // S3 & S4: Within ONE source, conflicting parent lists for the same commit ->
        // merge_status = not_observed with reason conflicting_parents (not first-wins),
        // and on_default_branch is ALWAYS not_observed.
        let mut s4_scanner = GitEvidenceScanner::new();
        s4_scanner.observe_tool_call_with_context(
            Some("c1"),
            Some("Bash"),
            Some("git log -1 --format='%H %P'"),
            Some("/repo"),
        );
        s4_scanner.observe_tool_output_with_cwd(
            Some("c1"),
            None,
            Some("sess-s4"),
            false,
            None,
            Some(false),
            Some("/repo"),
            &format!("{sha1} {sha2}\n"),
        );
        s4_scanner.observe_tool_call_with_context(
            Some("c2"),
            Some("Bash"),
            Some("git log -1 --format='%H %P'"),
            Some("/repo"),
        );
        s4_scanner.observe_tool_output_with_cwd(
            Some("c2"),
            None,
            Some("sess-s4"),
            false,
            None,
            Some(false),
            Some("/repo"),
            &format!("{sha1} {sha3}\n"),
        );
        // Even if a 3rd observation repeats sha2, conflict is sticky within the source:
        s4_scanner.observe_tool_call_with_context(
            Some("c3"),
            Some("Bash"),
            Some("git log -1 --format='%H %P'"),
            Some("/repo"),
        );
        s4_scanner.observe_tool_output_with_cwd(
            Some("c3"),
            None,
            Some("sess-s4"),
            false,
            None,
            Some(false),
            Some("/repo"),
            &format!("{sha1} {sha2}\n"),
        );

        let export_ready =
            FactExportConfig::from_raw_key(b"fact-export-v1-test-key-32-bytes", "kid_ready");
        let (s4_facts, s4_cov) = s4_scanner.materialize(
            "src-s4",
            1,
            "claude",
            false,
            Some(b"tool-key"),
            &export_ready,
            |s| format!("sealed:{s}"),
        );
        assert_eq!(s4_facts.len(), 1);
        assert_eq!(s4_facts[0].merge_status, MERGE_STATUS_NOT_OBSERVED);
        assert_eq!(s4_facts[0].is_merge, None);
        assert_eq!(
            s4_facts[0].merge_missing_reason.as_deref(),
            Some(MERGE_REASON_CONFLICTING_PARENTS)
        );
        assert!(s4_facts[0].parent_sealed.is_empty());
        assert!(s4_facts[0].parent_hmacs.is_empty());
        assert_eq!(
            s4_facts[0].default_branch_status,
            DEFAULT_BRANCH_STATUS_NOT_OBSERVED
        );
        assert_eq!(s4_facts[0].on_default_branch, None);
        assert_eq!(s4_cov.missing_parents, 1);
        assert_eq!(s4_cov.default_branch_not_observed, 1);

        // S5: Exportability with config Unconfigured / InvalidKeyMaterial / InvalidKeyId / ForbiddenOrgKeyPresent -> not exportable!
        let valid_row = GitEvidenceFact {
            source_key: "src-1:git:commit:c1".to_string(),
            source_id: "src-1".to_string(),
            generation: 1,
            harness: "claude".to_string(),
            canonical_harness: "claude".to_string(),
            native_session: Some("session:s1".to_string()),
            commit_sealed: "commit:c1".to_string(),
            parent_sealed: vec!["commit:p1".to_string()],
            is_merge: Some(false),
            merge_status: MERGE_STATUS_OBSERVED.to_string(),
            merge_missing_reason: None,
            on_default_branch: None,
            default_branch_status: DEFAULT_BRANCH_STATUS_NOT_OBSERVED.to_string(),
            observed_at: None,
            observed_at_status: OBSERVED_AT_STATUS_NOT_OBSERVABLE.to_string(),
            session_match_status: SESSION_MATCH_MATCHED.to_string(),
            evidence_source: EVIDENCE_SOURCE_LOCAL_GIT_HISTORY.to_string(),
            export_status: EXPORT_STATUS_EXPORTABLE.to_string(),
            pseudonym_key_id: Some("kid_ready".to_string()),
            commit_hmac: Some("1".repeat(64)),
            parent_hmacs: vec!["2".repeat(64)],
        };

        // (a) Unconfigured (even if bare KEY_ID was set in vars -> state is InvalidKeyMaterial)
        let cfg_unconfigured = FactExportConfig::unconfigured();
        let cfg_invalid_key = FactExportConfig::from_key_hex("not-hex", Some("kid_ready"));
        let cfg_invalid_kid = FactExportConfig::from_key_hex(&"11".repeat(32), None);
        let mut k_org_vars = BTreeMap::new();
        k_org_vars.insert(ENV_FACT_EXPORT_KEY_HEX.to_string(), "11".repeat(32));
        k_org_vars.insert(ENV_FACT_EXPORT_KEY_ID.to_string(), "kid_ready".to_string());
        k_org_vars.insert("K_ORG_HEX".to_string(), "22".repeat(32));
        let cfg_refused = FactExportConfig::from_vars(&k_org_vars, None);
        assert_eq!(
            cfg_refused.state(),
            FactExportKeyState::ForbiddenOrgKeyPresent
        );
        let cfg_wrong_kid =
            FactExportConfig::from_raw_key(b"fact-export-v1-test-key-32-bytes", "kid_other");

        for non_matching_cfg in [
            &cfg_unconfigured,
            &cfg_invalid_key,
            &cfg_invalid_kid,
            &cfg_refused,
            &cfg_wrong_kid,
        ] {
            let res = resolve_git_evidence_across_sources_with_config(
                std::slice::from_ref(&valid_row),
                non_matching_cfg,
            );
            assert_eq!(res.len(), 1);
            assert_eq!(
                res[0].export_status,
                EXPORT_STATUS_NON_EXPORTABLE,
                "config in state {:?} must not export rows",
                non_matching_cfg.state()
            );
            assert_eq!(res[0].pseudonym_key_id, None);
            assert_eq!(res[0].commit_hmac, None);
            assert!(res[0].parent_hmacs.is_empty());
        }

        // Also verify `resolve_git_evidence_across_sources_with_key_id(rows, None)` returns non_exportable
        let res_none =
            resolve_git_evidence_across_sources_with_key_id(std::slice::from_ref(&valid_row), None);
        assert_eq!(res_none[0].export_status, EXPORT_STATUS_NON_EXPORTABLE);

        // Only when config is Ready AND key_id equals configured key_id is the row exportable:
        let res_ready = resolve_git_evidence_across_sources_with_config(
            std::slice::from_ref(&valid_row),
            &export_ready,
        );
        assert_eq!(res_ready[0].export_status, EXPORT_STATUS_EXPORTABLE);
        assert_eq!(res_ready[0].pseudonym_key_id.as_deref(), Some("kid_ready"));
    }

    #[test]
    fn scanner_handles_600_tool_calls_openssl_rand_agent_mcp_and_object_format_and_record_cwd() {
        let mut scanner = GitEvidenceScanner::new();
        let sha1_commit = "a".repeat(40);
        let sha256_commit = "c".repeat(64);
        let sha256_parent = "d".repeat(64);

        // 1. 600 tool calls followed by `openssl rand -hex 32` -> nothing
        for idx in 0..600 {
            let cid = format!("call-{idx}");
            scanner.observe_tool_call_with_context(
                Some(&cid),
                Some("Bash"),
                Some("echo hello"),
                Some("/repo-a"),
            );
        }
        scanner.observe_tool_call_with_context(
            Some("call-600"),
            Some("Bash"),
            Some("openssl rand -hex 32"),
            Some("/repo-a"),
        );
        scanner.observe_tool_output_with_cwd(
            Some("call-600"),
            None,
            Some("sess-1"),
            false,
            None,
            Some(false),
            Some("/repo-a"),
            &format!("{sha256_commit}\n"),
        );

        // 2. Agent and MCP tool calls whose results contain SHAs -> nothing
        scanner.observe_tool_call_with_context(
            Some("call-agent"),
            Some("Agent"),
            Some("git rev-parse HEAD"),
            Some("/repo-a"),
        );
        scanner.observe_tool_output_with_cwd(
            Some("call-agent"),
            None,
            Some("sess-1"),
            false,
            None,
            Some(false),
            Some("/repo-a"),
            &format!("{sha1_commit}\n"),
        );
        scanner.observe_tool_call_with_context(
            Some("call-mcp"),
            Some("mcp__git__rev_parse"),
            Some("git rev-parse HEAD"),
            Some("/repo-a"),
        );
        scanner.observe_tool_output_with_cwd(
            Some("call-mcp"),
            None,
            Some("sess-1"),
            false,
            None,
            Some(false),
            Some("/repo-a"),
            &format!("{sha1_commit}\n"),
        );

        let (facts_empty, cov_empty) = scanner.materialize(
            "src-1",
            1,
            "claude",
            false,
            Some(b"tool-key"),
            &FactExportConfig::unconfigured(),
            |s| format!("sealed:{s}"),
        );
        assert!(facts_empty.is_empty());
        assert_eq!(cov_empty.observed, 0);
        assert_eq!(cov_empty.unverified_object_format, 0);

        // 3. 64-hex commit in `git rev-parse HEAD` WITHOUT `show-object-format` -> ignored and counted
        scanner.observe_tool_call_with_context(
            Some("call-64-unverified"),
            Some("Bash"),
            Some("git rev-parse HEAD"),
            Some("/repo-a"),
        );
        scanner.observe_tool_output_with_cwd(
            Some("call-64-unverified"),
            None,
            Some("sess-1"),
            false,
            None,
            Some(false),
            Some("/repo-a"),
            &format!("{sha256_commit}\n"),
        );
        // S1: `show-object-format` WITHOUT record cwd is ignored (requires record cwd to be present!)
        scanner.observe_tool_call_with_context(
            Some("call-fmt-nocwd"),
            Some("Bash"),
            Some("git rev-parse --show-object-format"),
            None,
        );
        scanner.observe_tool_output_with_cwd(
            Some("call-fmt-nocwd"),
            None,
            Some("sess-1"),
            false,
            None,
            Some(false),
            None,
            "sha256\n",
        );
        scanner.observe_tool_call_with_context(
            Some("call-64-nocwd"),
            Some("Bash"),
            Some("git rev-parse HEAD"),
            None,
        );
        scanner.observe_tool_output_with_cwd(
            Some("call-64-nocwd"),
            None,
            Some("sess-1"),
            false,
            None,
            Some(false),
            None,
            &format!("{sha256_commit}\n"),
        );

        let (facts_after_unv, cov_after_unv) = scanner.materialize(
            "src-1",
            1,
            "claude",
            false,
            Some(b"tool-key"),
            &FactExportConfig::unconfigured(),
            |s| format!("sealed:{s}"),
        );
        assert!(facts_after_unv.is_empty());
        assert_eq!(cov_after_unv.unverified_object_format, 2);

        // 4. Record `git rev-parse --show-object-format` = `sha256` in `/repo-256`, then 64-hex is accepted
        scanner.observe_tool_call_with_context(
            Some("call-fmt"),
            Some("Bash"),
            Some("git rev-parse --show-object-format"),
            Some("/repo-256"),
        );
        scanner.observe_tool_output_with_cwd(
            Some("call-fmt"),
            None,
            Some("sess-1"),
            false,
            None,
            Some(false),
            Some("/repo-256"),
            "sha256\n",
        );
        scanner.observe_tool_call_with_context(
            Some("call-64-ok"),
            Some("Bash"),
            Some("git log -1 --format='%H %P'"),
            Some("/repo-256"),
        );
        scanner.observe_tool_output_with_cwd(
            Some("call-64-ok"),
            None,
            Some("sess-1"),
            false,
            None,
            Some(false),
            Some("/repo-256"),
            &format!("{sha256_commit} {sha256_parent}\n"),
        );

        // 5. Mixed parent lengths (40-hex commit with 64-hex parent in `/repo-a`) -> parents dropped
        scanner.observe_tool_call_with_context(
            Some("call-mixed"),
            Some("Bash"),
            Some("git log -1 --format='%H %P'"),
            Some("/repo-a"),
        );
        scanner.observe_tool_output_with_cwd(
            Some("call-mixed"),
            None,
            Some("sess-1"),
            false,
            None,
            Some(false),
            Some("/repo-a"),
            &format!("{sha1_commit} {sha256_parent}\n"),
        );

        let (facts, cov) = scanner.materialize(
            "src-1",
            1,
            "claude",
            false,
            Some(b"tool-key"),
            &FactExportConfig::unconfigured(),
            |s| format!("sealed:{s}"),
        );
        assert_eq!(facts.len(), 2);
        assert_eq!(cov.observed, 2);
        assert_eq!(cov.unverified_object_format, 2);

        let sealed_256 = seal_commit(Some(b"tool-key"), &sha256_commit);
        let row_256 = facts
            .iter()
            .find(|r| r.commit_sealed == sealed_256)
            .unwrap();
        assert_eq!(row_256.merge_status, "observed");
        assert_eq!(row_256.is_merge, Some(false));
        assert_eq!(row_256.default_branch_status, "not_observed");
        assert_eq!(row_256.on_default_branch, None);

        let sealed_mixed = seal_commit(Some(b"tool-key"), &sha1_commit);
        let row_mixed = facts
            .iter()
            .find(|r| r.commit_sealed == sealed_mixed)
            .unwrap();
        assert_eq!(row_mixed.merge_status, "not_observed");
        assert_eq!(row_mixed.is_merge, None);
        assert!(row_mixed.parent_sealed.is_empty());
        assert_eq!(
            row_mixed.merge_missing_reason.as_deref(),
            Some(MERGE_REASON_MIXED_PARENT_OBJECT_FORMAT)
        );
    }

    #[test]
    fn key_loader_parity_with_d3_cwd_ignored_private_root_only_case_insensitive_k_org_lenient_consumer_env(
    ) {
        #[cfg(any(windows, target_os = "linux"))]
        let dir = {
            let root = crate::platform_v1::create_user_private_root_with_choice(
                "dendro-d4c-cfg-",
                &crate::platform_v1::StorageRootChoice::UserProfile,
            )
            .unwrap();
            root.path().to_path_buf()
        };
        #[cfg(not(any(windows, target_os = "linux")))]
        let dir = {
            let d = std::env::temp_dir().join(format!(
                "dendro-d4c-fact-export-cfg-{}",
                rand::random::<u64>()
            ));
            std::fs::create_dir_all(&d).unwrap();
            d
        };

        let key_env = "11".repeat(32);
        let key_file = "22".repeat(32);
        let key_root = "33".repeat(32);
        let key_consumer = "44".repeat(32);

        // 1. Constant-time equality on FactExportKey (and no Clone)
        let k1 = FactExportKey::from_bytes([0xAA; 32]);
        let k2 = FactExportKey::from_bytes([0xAA; 32]);
        let k3 = FactExportKey::from_bytes([0xAB; 32]);
        assert_eq!(k1, k2);
        assert_ne!(k1, k3);

        // 2. CWD isolation & no USERPROFILE fallback: relative XDG_CONFIG_HOME / HOME resolve to None
        assert!(
            read_consumer_env_from(Path::new("observability/consumer.env")).is_none(),
            "relative consumer.env path must always be refused"
        );
        for (rel_xdg, rel_home) in [
            (Some(std::ffi::OsStr::new("cwd_sim")), None),
            (Some(std::ffi::OsStr::new(".")), None),
            (None, Some(std::ffi::OsStr::new("cwd_sim"))),
            (None, Some(std::ffi::OsStr::new("."))),
            (None, None),
        ] {
            assert!(
                consumer_env_path_from_vars(rel_xdg, rel_home).is_none(),
                "relative or absent XDG_CONFIG_HOME/HOME must resolve to None (no USERPROFILE fallback)"
            );
        }

        // 3. Lenient consumer.env: a missing, directory, or oversized (>16 KiB) consumer.env is
        //    ignored (not fatal) and yields Unconfigured when no other source is set, or allows
        //    env/private_root keys to succeed.
        let oversized_consumer = dir.join("oversized_consumer.env");
        std::fs::write(&oversized_consumer, "A".repeat(20 * 1024)).unwrap();
        let uncfg_with_oversized =
            FactExportConfig::from_sources(&BTreeMap::new(), None, Some(&oversized_consumer));
        assert_eq!(
            uncfg_with_oversized.state(),
            FactExportKeyState::Unconfigured
        );

        let mut valid_env = BTreeMap::new();
        valid_env.insert(ENV_FACT_EXPORT_KEY_HEX.to_string(), key_env.clone());
        valid_env.insert(ENV_FACT_EXPORT_KEY_ID.to_string(), "kid_env".to_string());
        let env_with_oversized_consumer =
            FactExportConfig::from_sources(&valid_env, None, Some(&oversized_consumer));
        assert!(
            env_with_oversized_consumer.is_ready(),
            "unusable consumer.env must be leniently ignored, not fatal"
        );

        // 4. Prepare private data root `<private_data_root>/fact_export.key` and explicit key file via `platform_v1`
        #[cfg(any(windows, target_os = "linux"))]
        let (private_root, explicit_key_path, empty_root) = {
            use std::io::Write;
            let parent_root = crate::platform_v1::open_existing_private_root(&dir).unwrap();
            let priv_cap =
                crate::platform_v1::create_unique_private_directory(&parent_root, "priv-root-")
                    .unwrap();
            let key_child = crate::platform_v1::parse_child_name(
                DEFAULT_FACT_EXPORT_KEY_FILENAME,
                crate::platform_v1::current_native_platform(),
            )
            .unwrap();
            let mut vf = priv_cap.create_file(&key_child).unwrap();
            vf.file_mut()
                .write_all(format!("{key_root}\n").as_bytes())
                .unwrap();
            drop(vf);
            let priv_path = priv_cap.path().to_path_buf();
            drop(priv_cap);

            let exp_child = crate::platform_v1::parse_child_name(
                "explicit_fact_export.key",
                crate::platform_v1::current_native_platform(),
            )
            .unwrap();
            let mut vf_exp = parent_root.create_file(&exp_child).unwrap();
            vf_exp
                .file_mut()
                .write_all(format!("{key_file}\n").as_bytes())
                .unwrap();
            let exp_path = vf_exp.path().to_path_buf();
            drop(vf_exp);

            let empty_cap =
                crate::platform_v1::create_unique_private_directory(&parent_root, "empty-root-")
                    .unwrap();
            let empty_path = empty_cap.path().to_path_buf();
            drop(empty_cap);
            drop(parent_root);
            (priv_path, exp_path, empty_path)
        };
        #[cfg(not(any(windows, target_os = "linux")))]
        let (private_root, explicit_key_path, empty_root) = {
            let priv_path = dir.join("private_data_root");
            std::fs::create_dir_all(&priv_path).unwrap();
            let root_key_path = priv_path.join(DEFAULT_FACT_EXPORT_KEY_FILENAME);
            std::fs::write(&root_key_path, format!("{key_root}\n")).unwrap();
            let exp_path = dir.join("explicit_fact_export.key");
            std::fs::write(&exp_path, format!("{key_file}\n")).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&priv_path, std::fs::Permissions::from_mode(0o700))
                    .unwrap();
                std::fs::set_permissions(&root_key_path, std::fs::Permissions::from_mode(0o600))
                    .unwrap();
                std::fs::set_permissions(&exp_path, std::fs::Permissions::from_mode(0o600))
                    .unwrap();
            }
            let empty_path = dir.join("empty_private_root");
            std::fs::create_dir_all(&empty_path).unwrap();
            (priv_path, exp_path, empty_path)
        };

        let xdg_consumer = dir.join("xdg").join("observability").join("consumer.env");
        std::fs::create_dir_all(xdg_consumer.parent().unwrap()).unwrap();
        std::fs::write(
            &xdg_consumer,
            format!(
                "DENDRO_FACT_EXPORT_KEY_ID=kid_consumer\nDENDRO_FACT_EXPORT_KEY_HEX={key_consumer}\n"
            ),
        )
        .unwrap();

        let sha_test = "a".repeat(40);
        let expected_for = |k_hex: &str, kid: &str| -> Option<String> {
            FactExportConfig::from_key_hex(k_hex, Some(kid)).derive_commit_hmac(&sha_test)
        };

        // Precedence:
        // (a) process env HEX beats FILE, private_root/fact_export.key, and consumer.env
        let mut env_hex_and_file = valid_env.clone();
        env_hex_and_file.insert(
            ENV_FACT_EXPORT_KEY_FILE.to_string(),
            explicit_key_path.display().to_string(),
        );
        let cfg_a = FactExportConfig::from_sources(
            &env_hex_and_file,
            Some(&private_root),
            Some(&xdg_consumer),
        );
        assert_eq!(
            cfg_a.derive_commit_hmac(&sha_test),
            expected_for(&key_env, "kid_env")
        );

        // (b) process env FILE beats private_root/fact_export.key and consumer.env
        let mut env_file = BTreeMap::new();
        env_file.insert(
            ENV_FACT_EXPORT_KEY_FILE.to_string(),
            explicit_key_path.display().to_string(),
        );
        let cfg_file =
            FactExportConfig::from_sources(&env_file, Some(&private_root), Some(&xdg_consumer));
        assert_eq!(
            cfg_file.derive_commit_hmac(&sha_test),
            expected_for(&key_file, "kid_consumer")
        );

        // (c) private_root/fact_export.key beats consumer.env
        let cfg_root = FactExportConfig::from_sources(
            &BTreeMap::new(),
            Some(&private_root),
            Some(&xdg_consumer),
        );
        assert_eq!(
            cfg_root.derive_commit_hmac(&sha_test),
            expected_for(&key_root, "kid_consumer")
        );

        // (d) consumer.env used when process env and private_root/fact_export.key are absent
        let cfg_consumer = FactExportConfig::from_sources(
            &BTreeMap::new(),
            Some(&empty_root),
            Some(&xdg_consumer),
        );
        assert_eq!(
            cfg_consumer.derive_commit_hmac(&sha_test),
            expected_for(&key_consumer, "kid_consumer")
        );

        // 5. Case-insensitive K_org refusal with any whitespace after `export`
        for k_org_line in [
            "k_org_hex=deadbeef",
            "export\tK_ORG_HEX=deadbeef",
            "EXPORT   dendro_fact_export_k_org_hex=deadbeef",
            "export \t AGENT_OTEL_K_ORG=deadbeef",
        ] {
            let consumer_k_org = dir.join("consumer_k_org.env");
            std::fs::write(
                &consumer_k_org,
                format!("DENDRO_FACT_EXPORT_KEY_ID=kid_c\n{k_org_line}\n"),
            )
            .unwrap();
            let refused = FactExportConfig::from_sources(
                &valid_env,
                Some(&private_root),
                Some(&consumer_k_org),
            );
            assert!(
                !refused.is_ready(),
                "must refuse case-insensitive K_org line: {k_org_line}"
            );
            assert_eq!(refused.state(), FactExportKeyState::ForbiddenOrgKeyPresent);
            assert!(refused.k_org_rejected());
        }

        // 6. DENDRO_FACT_EXPORT_KEY_FILE inside consumer.env is leniently ignored
        let consumer_with_file = dir.join("consumer_with_file.env");
        std::fs::write(
            &consumer_with_file,
            format!(
                "DENDRO_FACT_EXPORT_KEY_ID=kid_c\nDENDRO_FACT_EXPORT_KEY_FILE={}\n",
                explicit_key_path.display()
            ),
        )
        .unwrap();
        let cfg_ignored_file = FactExportConfig::from_sources(
            &BTreeMap::new(),
            Some(&empty_root),
            Some(&consumer_with_file),
        );
        assert!(!cfg_ignored_file.is_ready());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cat_file_requires_prior_or_same_record_commit_observation_and_shallow_boundary_conflicts() {
        let sha_blob = "1".repeat(40);
        let sha_later = "2".repeat(40);
        let sha_confirmed = "3".repeat(40);
        let sha_shallow = "4".repeat(40);
        let sha_p1 = "5".repeat(40);
        let sha_p2 = "6".repeat(40);
        let tree_sha = "7".repeat(40);

        let crafted_merge_body = format!(
            "tree {tree_sha}\nparent {sha_p1}\nparent {sha_p2}\nauthor Dev <d@e> 1700000000 +0000\ncommitter Dev <d@e> 1700000000 +0000\n\ncrafted blob\n"
        );
        let shallow_cat_body = format!(
            "tree {tree_sha}\nparent {sha_p1}\nauthor Dev <d@e> 1700000000 +0000\ncommitter Dev <d@e> 1700000000 +0000\n\nshallow boundary commit\n"
        );

        let mut scanner = GitEvidenceScanner::new();

        // 1. Crafted-blob `git cat-file -p <sha_blob>` WITHOUT any prior commit observation ->
        //    contributes nothing and increments `cat_file_unconfirmed`.
        scanner.observe_tool_call_with_context(
            Some("c-blob"),
            Some("Bash"),
            Some(&format!("git cat-file -p {sha_blob}")),
            Some("/repo"),
        );
        scanner.observe_tool_output_with_cwd(
            Some("c-blob"),
            None,
            Some("sess-1"),
            false,
            None,
            Some(false),
            Some("/repo"),
            &crafted_merge_body,
        );
        assert_eq!(scanner.cat_file_unconfirmed(), 1);
        let (facts_after_blob, cov_after_blob) = scanner.materialize(
            "src-cat",
            1,
            "claude",
            false,
            Some(b"tool-key"),
            &FactExportConfig::unconfigured(),
            |s| format!("sealed:{s}"),
        );
        assert!(
            facts_after_blob.is_empty(),
            "crafted-blob cat-file without prior commit observation must produce 0 rows"
        );
        assert_eq!(cov_after_blob.observed, 0);
        assert_eq!(cov_after_blob.cat_file_unconfirmed, 1);

        // 2. `git cat-file -p <sha_later>` BEFORE a later `git rev-parse HEAD` observation of `<sha_later>` ->
        //    the earlier `cat-file` is rejected (`cat_file_unconfirmed` = 2) and does NOT bind parents
        //    when `<sha_later>` is subsequently observed by `rev-parse HEAD`.
        scanner.observe_tool_call_with_context(
            Some("c-early-cat"),
            Some("Bash"),
            Some(&format!("git cat-file -p {sha_later}")),
            Some("/repo"),
        );
        scanner.observe_tool_output_with_cwd(
            Some("c-early-cat"),
            None,
            Some("sess-1"),
            false,
            None,
            Some(false),
            Some("/repo"),
            &crafted_merge_body,
        );
        assert_eq!(scanner.cat_file_unconfirmed(), 2);

        scanner.observe_tool_call_with_context(
            Some("c-later-rp"),
            Some("Bash"),
            Some("git rev-parse HEAD"),
            Some("/repo"),
        );
        scanner.observe_tool_output_with_cwd(
            Some("c-later-rp"),
            None,
            Some("sess-1"),
            false,
            None,
            Some(false),
            Some("/repo"),
            &format!("{sha_later}\n"),
        );

        // 3. Prior `git rev-parse HEAD` (or `git log -1 --format=%H`) observation of `<sha_confirmed>`
        //    followed by `git cat-file -p <sha_confirmed>` -> parents ARE bound (`is_merge = Some(true)`).
        scanner.observe_tool_call_with_context(
            Some("c-conf-rp"),
            Some("Bash"),
            Some("git rev-parse HEAD"),
            Some("/repo"),
        );
        scanner.observe_tool_output_with_cwd(
            Some("c-conf-rp"),
            None,
            Some("sess-1"),
            false,
            None,
            Some(false),
            Some("/repo"),
            &format!("{sha_confirmed}\n"),
        );
        scanner.observe_tool_call_with_context(
            Some("c-conf-cat"),
            Some("Bash"),
            Some(&format!("git cat-file -p {sha_confirmed}")),
            Some("/repo"),
        );
        scanner.observe_tool_output_with_cwd(
            Some("c-conf-cat"),
            None,
            Some("sess-1"),
            false,
            None,
            Some(false),
            Some("/repo"),
            &crafted_merge_body,
        );
        assert_eq!(
            scanner.cat_file_unconfirmed(),
            2,
            "confirmed cat-file must not increment cat_file_unconfirmed"
        );

        // 4. Shallow-clone boundary: `git log -1 --format='%H %P'` prints 0 parents (`<sha_shallow>`)
        //    while `git cat-file -p <sha_shallow>` prints real `parent <sha_p1>` ->
        //    running both in one source yields `conflicting_parents` (`merge_status = not_observed`).
        scanner.observe_tool_call_with_context(
            Some("c-shallow-log"),
            Some("Bash"),
            Some("git log -1 --format='%H %P'"),
            Some("/repo"),
        );
        scanner.observe_tool_output_with_cwd(
            Some("c-shallow-log"),
            None,
            Some("sess-1"),
            false,
            None,
            Some(false),
            Some("/repo"),
            &format!("{sha_shallow}\n"),
        );
        scanner.observe_tool_call_with_context(
            Some("c-shallow-cat"),
            Some("Bash"),
            Some(&format!("git cat-file -p {sha_shallow}")),
            Some("/repo"),
        );
        scanner.observe_tool_output_with_cwd(
            Some("c-shallow-cat"),
            None,
            Some("sess-1"),
            false,
            None,
            Some(false),
            Some("/repo"),
            &shallow_cat_body,
        );

        let export_cfg =
            FactExportConfig::from_raw_key(b"fact-export-v1-test-key-32-bytes", "kid_cat");
        let (facts, cov) = scanner.materialize(
            "src-cat",
            1,
            "claude",
            false,
            Some(b"tool-key"),
            &export_cfg,
            |s| format!("sealed:{s}"),
        );
        assert_eq!(facts.len(), 3, "sha_blob must never appear in facts");
        assert_eq!(cov.observed, 3);
        assert_eq!(cov.cat_file_unconfirmed, 2);

        let sealed_later = seal_commit(Some(b"tool-key"), &sha_later);
        let row_later = facts
            .iter()
            .find(|r| r.commit_sealed == sealed_later)
            .unwrap();
        assert_eq!(row_later.merge_status, MERGE_STATUS_NOT_OBSERVED);
        assert_eq!(
            row_later.merge_missing_reason.as_deref(),
            Some(MERGE_REASON_PARENTS_NOT_RECORDED)
        );
        assert!(row_later.parent_sealed.is_empty());

        let sealed_conf = seal_commit(Some(b"tool-key"), &sha_confirmed);
        let row_conf = facts
            .iter()
            .find(|r| r.commit_sealed == sealed_confirmed_helper(&sha_confirmed))
            .unwrap_or_else(|| {
                facts
                    .iter()
                    .find(|r| r.commit_sealed == sealed_conf)
                    .unwrap()
            });
        assert_eq!(row_conf.merge_status, MERGE_STATUS_OBSERVED);
        assert_eq!(row_conf.is_merge, Some(true));
        assert_eq!(
            row_conf.parent_sealed,
            vec![
                seal_commit(Some(b"tool-key"), &sha_p1),
                seal_commit(Some(b"tool-key"), &sha_p2)
            ]
        );

        let sealed_shallow = seal_commit(Some(b"tool-key"), &sha_shallow);
        let row_shallow = facts
            .iter()
            .find(|r| r.commit_sealed == sealed_shallow)
            .unwrap();
        assert_eq!(row_shallow.merge_status, MERGE_STATUS_NOT_OBSERVED);
        assert_eq!(row_shallow.is_merge, None);
        assert_eq!(
            row_shallow.merge_missing_reason.as_deref(),
            Some(MERGE_REASON_CONFLICTING_PARENTS)
        );
        assert!(row_shallow.parent_sealed.is_empty());
    }

    fn sealed_confirmed_helper(sha: &str) -> String {
        seal_commit(Some(b"tool-key"), sha)
    }
}
