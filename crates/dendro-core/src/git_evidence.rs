//! Local git merge & branch evidence extraction and `fact-export-v1` commit pseudonymization (`D4c`).
//!
//! This module implements the local evidence extractor and storage model for `git_evidence` records:
//! - Extracts commit observations, parent commit lists, and default-branch state strictly from
//!   an explicit allow-list of recognized git command/output pairs in session transcripts
//!   (no repository access, no `git` subprocess spawning, no network calls).
//! - Matches tool outputs to tool calls strictly by explicit call ID using a bounded LRU map;
//!   unmatched tool outputs and non-shell tools (`Agent`, `MCP`, `Read`, etc.) are ignored.
//! - Enforces hash algorithm verification per `(session, cwd)`: 40-hex SHA-1 is accepted unless
//!   `git rev-parse --show-object-format` recorded `sha256` in that session and working directory;
//!   64-hex SHA-256 is accepted only when `sha256` was recorded in that session and working
//!   directory (otherwise counted in `unverified_object_format` and ignored).
//! - Enforces parent length consistency within each commit observation: if any parent's hex length
//!   differs from the commit's hex length, parents are dropped with reason
//!   `mixed_parent_object_format`.
//! - Applies default-branch observations (`git symbolic-ref refs/remotes/origin/HEAD` or
//!   `git remote show origin`) only to commits observed in the same session and the same known
//!   working directory at or after the default-branch observation time (never retroactively, and
//!   `not_observed` when `cwd` is unknown).
//! - Seals local commit SHAs (`hmac_domain(tool_key, "git_commit", sha)`) and session identities
//!   before persistence so raw SHAs, branch names, commit messages, author identities, file paths,
//!   and remote URLs are never written to DuckDB.
//! - Reuses `D3`'s secret-key loader in `crate::link_id` (`platform_v1` private data root
//!   `fact_export.key`, lenient `consumer.env`, case-insensitive `K_org` refusal, no `USERPROFILE`
//!   fallback, constant-time key equality, no `Clone`).
//! - Persists per-source local facts and `commit_hmac`/`parent_hmacs` only; provides
//!   [`resolve_git_evidence_across_sources`] to deterministically derive cross-source
//!   `session_match_status` at read time.

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

/// Canonical `default_branch_status` when default-branch membership is directly observed.
pub const DEFAULT_BRANCH_STATUS_OBSERVED: &str = "observed";
/// Canonical `default_branch_status` when default-branch membership is not observed.
pub const DEFAULT_BRANCH_STATUS_NOT_OBSERVED: &str = "not_observed";
/// Canonical `default_branch_status` when default-branch membership is unknown.
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
/// Reason recorded when multiple sources report different `parent_sealed` lists for the same commit.
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
/// Maximum shell-wrapper unwrapping depth (`bash -lc "..."`, `sh -c "..."`, etc.).
pub const MAX_SHELL_WRAPPER_DEPTH: usize = 4;
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
/// - `unresolved_abbreviated` and `unverified_object_format` are counted per rejected occurrence.
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
    }
}

fn configured_export_key_id_from_env() -> Option<String> {
    let cfg = FactExportConfig::from_env();
    if let Some(kid) = cfg.pseudonym_key_id() {
        return Some(kid.to_string());
    }
    let process_env = collect_process_env_vars();
    let consumer_vars = load_consumer_env_vars(consumer_env_path().as_deref());
    let raw_id = pick_var_with_fallback(
        &process_env,
        &consumer_vars,
        &[ENV_FACT_EXPORT_KEY_ID, ENV_ALT_FACT_EXPORT_KEY_ID],
    );
    zeroize_env_map(process_env);
    zeroize_env_map(consumer_vars);
    raw_id.and_then(|s| {
        let trimmed = s.trim();
        is_valid_key_id(trimmed).then(|| trimmed.to_string())
    })
}

/// Deterministically derives cross-source `GitEvidenceFact` records across all per-source rows
/// stored in `git_evidence_facts` (for D4b and read queries).
///
/// Cross-source session matching rules (P3) and read-function coherence rules (R5):
/// - Exactly one distinct non-empty session across all sources (and no source marked `ambiguous`)
///   -> `session_match_status = "matched"`, `native_session = Some(session)`.
/// - More than one distinct session across sources (or any source marked `ambiguous`)
///   -> `session_match_status = "ambiguous"`, `native_session = None`.
/// - Zero sessions across sources
///   -> `session_match_status = "unmatched"` (or `"unknown"` if all observations are `"unknown"`),
///   `native_session = None`.
/// - If sources disagree on parents (different `parent_sealed` lists), sets
///   `merge_status = "not_observed"` with reason `conflicting_parents`.
/// - Derives exported fields (`export_status`, `pseudonym_key_id`, `commit_hmac`, `parent_hmacs`)
///   from ONE coherent observation row (never combining parents from one row with HMACs from
///   another). If HMAC rows carry different key IDs, only rows matching the current configured
///   key ID are exportable.
#[must_use]
pub fn resolve_git_evidence_across_sources(rows: &[GitEvidenceFact]) -> Vec<GitEvidenceFact> {
    let env_key_id = configured_export_key_id_from_env();
    resolve_git_evidence_across_sources_with_key_id(rows, env_key_id.as_deref())
}

/// Deterministically derives cross-source `GitEvidenceFact` records across all per-source rows,
/// using `current_key_id` when rows carry different `pseudonym_key_id` values.
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

    let distinct_key_ids: BTreeSet<&str> = sorted
        .iter()
        .filter(|r| r.export_status == EXPORT_STATUS_EXPORTABLE)
        .filter_map(|r| r.pseudonym_key_id.as_deref())
        .filter(|kid| is_valid_key_id(kid))
        .collect();

    let allowed_key_id: Option<&str> = if let Some(cfg_kid) = current_key_id {
        let trimmed = cfg_kid.trim();
        is_valid_key_id(trimmed).then_some(trimmed)
    } else if distinct_key_ids.len() == 1 {
        distinct_key_ids.into_iter().next()
    } else {
        None
    };

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

        // R5: Merge status & parent disagreement check across sources.
        let observed_parent_rows: Vec<&GitEvidenceFact> = group
            .iter()
            .copied()
            .filter(|r| r.merge_status == MERGE_STATUS_OBSERVED)
            .collect();
        let parents_conflict = observed_parent_rows.len() > 1
            && !observed_parent_rows
                .windows(2)
                .all(|w| w[0].parent_sealed == w[1].parent_sealed);

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

        // Default branch status: observed if observing sources agree on `on_default_branch`.
        let mut observed_def_vals: BTreeSet<bool> = BTreeSet::new();
        for row in &group {
            if row.default_branch_status == DEFAULT_BRANCH_STATUS_OBSERVED {
                if let Some(val) = row.on_default_branch {
                    observed_def_vals.insert(val);
                }
            }
        }
        let (default_branch_status, on_default_branch) = if observed_def_vals.len() == 1 {
            let val = observed_def_vals.into_iter().next().unwrap();
            (DEFAULT_BRANCH_STATUS_OBSERVED.to_string(), Some(val))
        } else {
            (DEFAULT_BRANCH_STATUS_NOT_OBSERVED.to_string(), None)
        };

        // R5: Derive exported fields from ONE coherent row under the eligible `allowed_key_id`.
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
// Strict Allow-Listed Single Git Command Classifier & Output Parser (R1–R4)
// ---------------------------------------------------------------------------

/// Strictly allow-listed git command shapes recognized by D4c.
/// Any command outside this allow-list is classified as [`GitCommandKind::NonGit`] and
/// contributes nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitCommandKind {
    /// Not an allow-listed git command shape. Contributes nothing.
    NonGit,
    /// `git rev-parse HEAD` (or `git rev-parse --short HEAD`).
    RevParseCommit { short: bool },
    /// `git rev-parse --show-object-format`.
    ShowObjectFormat,
    /// `git log` or `git show -s`/`--no-patch` with a format that is EXACTLY `%H`.
    /// `is_head` is true only for `git log -1`/`-n 1` with no revision or `HEAD`.
    LogCommitOnly { is_head: bool },
    /// `git log` or `git show -s`/`--no-patch` with a format that is EXACTLY `%H %P`,
    /// or `git rev-list --parents ...`.
    /// `is_head` is true only for `git log -1`/`-n 1` with no revision or `HEAD`.
    LogCommitAndParents { is_head: bool },
    /// `git cat-file -p <FULL-SHA>` or `git cat-file commit <FULL-SHA>` where `<FULL-SHA>` is
    /// a canonical 40-hex or 64-hex SHA (`cat-file -p HEAD` or non-full-SHA is `NonGit`).
    CatFileCommit { target_sha: String },
    /// `git commit ...` (abbreviated SHA in `[branch <short-sha>]` -> counted unresolved only).
    Commit,
    /// `git symbolic-ref refs/remotes/origin/HEAD`.
    SymbolicDefaultBranch,
    /// `git remote show origin`.
    RemoteShowOrigin,
    /// Branch inspection or switching command (`git rev-parse --abbrev-ref HEAD`,
    /// `git branch --show-current`, `git symbolic-ref --short HEAD`, `git status`,
    /// `git checkout`, `git switch`) used only to track the current branch / detached HEAD
    /// in `(session, cwd)`.
    CurrentBranch {
        checkout_branch: Option<String>,
        detaches_head: bool,
    },
}

/// One classified git invocation along with an optional leading `cd <dir> &&` override.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassifiedGitSubcommand {
    pub kind: GitCommandKind,
    pub cwd_override: Option<String>,
}

/// Result of classifying a shell tool call under R1's single-clean-call gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClassifiedCommandCall {
    /// Not a git invocation or a single non-allow-listed git subcommand.
    NonGit,
    /// A compound, chained, piped, redirected, subshell, env-prefixed, or global-option-bearing
    /// git call that was rejected by R1 and counted in `unparsed_compound_git_calls`.
    SkippedCompoundGit,
    /// A single clean allow-listed git command, optionally preceded by one `cd <path> &&`.
    Allowed(ClassifiedGitSubcommand),
}

/// Classify a shell command into its [`GitCommandKind`] under R1–R3.
#[must_use]
pub fn classify_shell_command(command: &str) -> GitCommandKind {
    match classify_single_git_command(command) {
        ClassifiedCommandCall::Allowed(sub) => sub.kind,
        ClassifiedCommandCall::NonGit | ClassifiedCommandCall::SkippedCompoundGit => {
            GitCommandKind::NonGit
        }
    }
}

/// Classify a shell command string into at most one allow-listed [`ClassifiedGitSubcommand`].
#[must_use]
pub fn classify_shell_subcommands(command: &str) -> Vec<ClassifiedGitSubcommand> {
    match classify_single_git_command(command) {
        ClassifiedCommandCall::Allowed(sub) => vec![sub],
        ClassifiedCommandCall::NonGit | ClassifiedCommandCall::SkippedCompoundGit => Vec::new(),
    }
}

/// Classify a shell command under R1: the entire command must be a single allow-listed git
/// invocation, optionally preceded by a single `cd <path> &&` (and optionally inside a clean
/// shell wrapper containing nothing else).
#[must_use]
pub fn classify_single_git_command(command: &str) -> ClassifiedCommandCall {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return ClassifiedCommandCall::NonGit;
    }
    if trimmed.len() > MAX_GIT_COMMAND_BYTES {
        return if command_mentions_git(trimmed) {
            ClassifiedCommandCall::SkippedCompoundGit
        } else {
            ClassifiedCommandCall::NonGit
        };
    }

    let mut cur = trimmed.to_string();
    for depth in 0..=MAX_SHELL_WRAPPER_DEPTH {
        let Some(and_segments) = split_clean_and_segments(&cur) else {
            return if command_mentions_git(&cur) || command_mentions_git(trimmed) {
                ClassifiedCommandCall::SkippedCompoundGit
            } else {
                ClassifiedCommandCall::NonGit
            };
        };

        if and_segments.is_empty() {
            return ClassifiedCommandCall::NonGit;
        }

        if and_segments.len() == 1 {
            let Some(tokens) = tokenize_shell_words_strict(&and_segments[0]) else {
                return if command_mentions_git(&cur) {
                    ClassifiedCommandCall::SkippedCompoundGit
                } else {
                    ClassifiedCommandCall::NonGit
                };
            };
            if tokens.is_empty() {
                return ClassifiedCommandCall::NonGit;
            }
            if is_env_assignment(&tokens[0]) {
                return if tokens.iter().any(|t| {
                    matches!(executable_basename(t), "git" | "git.exe") || t.starts_with("GIT_")
                }) {
                    ClassifiedCommandCall::SkippedCompoundGit
                } else {
                    ClassifiedCommandCall::NonGit
                };
            }
            let exe = executable_basename(&tokens[0]);
            if let Some(inner) = extract_shell_wrapper_script_strict(exe, &tokens[1..]) {
                if depth == MAX_SHELL_WRAPPER_DEPTH {
                    return if command_mentions_git(&inner) {
                        ClassifiedCommandCall::SkippedCompoundGit
                    } else {
                        ClassifiedCommandCall::NonGit
                    };
                }
                cur = inner.trim().to_string();
                continue;
            }
            return classify_single_git_tokens(&tokens, None);
        }

        if and_segments.len() == 2 {
            let Some(cd_tokens) = tokenize_shell_words_strict(&and_segments[0]) else {
                return if command_mentions_git(&cur) {
                    ClassifiedCommandCall::SkippedCompoundGit
                } else {
                    ClassifiedCommandCall::NonGit
                };
            };
            let Some(cd_dir) = parse_strict_cd_tokens(&cd_tokens) else {
                return if command_mentions_git(&cur) {
                    ClassifiedCommandCall::SkippedCompoundGit
                } else {
                    ClassifiedCommandCall::NonGit
                };
            };
            let Some(git_tokens) = tokenize_shell_words_strict(&and_segments[1]) else {
                return if command_mentions_git(&cur) {
                    ClassifiedCommandCall::SkippedCompoundGit
                } else {
                    ClassifiedCommandCall::NonGit
                };
            };
            return classify_single_git_tokens(&git_tokens, Some(cd_dir));
        }

        return if command_mentions_git(&cur) {
            ClassifiedCommandCall::SkippedCompoundGit
        } else {
            ClassifiedCommandCall::NonGit
        };
    }

    if command_mentions_git(&cur) {
        ClassifiedCommandCall::SkippedCompoundGit
    } else {
        ClassifiedCommandCall::NonGit
    }
}

fn split_clean_and_segments(cmd: &str) -> Option<Vec<String>> {
    let mut segments = Vec::new();
    let mut cur = String::new();
    let mut chars = cmd.chars().peekable();
    let mut in_single = false;
    let mut in_double = false;

    while let Some(c) = chars.next() {
        if c == '\n' || c == '\r' {
            return None;
        }
        if c == '\\' && !in_single {
            let next_c = chars.next()?;
            if next_c == '\n' || next_c == '\r' {
                return None;
            }
            cur.push(c);
            cur.push(next_c);
            continue;
        }
        if c == '\'' && !in_double {
            in_single = !in_single;
            cur.push(c);
            continue;
        }
        if c == '"' && !in_single {
            in_double = !in_double;
            cur.push(c);
            continue;
        }
        if !in_single {
            if c == '`' {
                return None;
            }
            if c == '$' && matches!(chars.peek(), Some('(' | '{')) {
                return None;
            }
        }
        if !in_single && !in_double {
            if matches!(c, ';' | '|' | '<' | '>' | '(' | ')') {
                return None;
            }
            if c == '&' {
                if chars.peek() == Some(&'&') {
                    let _ = chars.next();
                    if chars.peek() == Some(&'&') {
                        return None;
                    }
                    let trimmed = cur.trim();
                    if trimmed.is_empty() {
                        return None;
                    }
                    segments.push(trimmed.to_string());
                    cur.clear();
                    continue;
                }
                return None;
            }
        }
        cur.push(c);
    }

    if in_single || in_double {
        return None;
    }
    let trimmed = cur.trim();
    if trimmed.is_empty() {
        if !segments.is_empty() {
            return None;
        }
    } else {
        segments.push(trimmed.to_string());
    }
    Some(segments)
}

fn tokenize_shell_words_strict(segment: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut chars = segment.chars().peekable();
    let mut in_single = false;
    let mut in_double = false;
    let mut saw_quote = false;

    while let Some(c) = chars.next() {
        if c == '\\' && !in_single {
            let next_c = chars.next()?;
            cur.push(next_c);
            continue;
        }
        if c == '\'' && !in_double {
            in_single = !in_single;
            saw_quote = true;
            continue;
        }
        if c == '"' && !in_single {
            in_double = !in_double;
            saw_quote = true;
            continue;
        }
        if c.is_ascii_whitespace() && !in_single && !in_double {
            if !cur.is_empty() || saw_quote {
                words.push(std::mem::take(&mut cur));
                saw_quote = false;
            }
            continue;
        }
        cur.push(c);
    }
    if in_single || in_double {
        return None;
    }
    if !cur.is_empty() || saw_quote {
        words.push(cur);
    }
    Some(words)
}

fn parse_strict_cd_tokens(tokens: &[String]) -> Option<String> {
    if tokens.is_empty() || executable_basename(&tokens[0]) != "cd" {
        return None;
    }
    let dir = match &tokens[1..] {
        [single] if !single.starts_with('-') => single.trim(),
        [dash_dash, single] if dash_dash == "--" => single.trim(),
        _ => return None,
    };
    if dir.is_empty() {
        return None;
    }
    Some(dir.to_string())
}

fn extract_shell_wrapper_script_strict(exe: &str, args: &[String]) -> Option<String> {
    if matches!(
        exe,
        "sh" | "bash" | "zsh" | "dash" | "ksh" | "fish" | "sh.exe" | "bash.exe"
    ) {
        match args {
            [flag, script]
                if flag == "-c"
                    || flag == "-lc"
                    || flag == "-cl"
                    || (flag.starts_with('-')
                        && !flag.starts_with("--")
                        && flag[1..].bytes().all(|b| b.is_ascii_alphabetic())
                        && flag.contains('c')) =>
            {
                return Some(script.clone());
            }
            _ => return None,
        }
    }

    if matches!(exe, "pwsh" | "powershell" | "pwsh.exe" | "powershell.exe") {
        let mut i = 0;
        while i < args.len() {
            let lower = args[i].to_ascii_lowercase();
            if matches!(lower.as_str(), "-c" | "-command" | "/c") {
                if i + 2 == args.len() {
                    return Some(args[i + 1].clone());
                }
                return None;
            }
            if matches!(
                lower.as_str(),
                "-noprofile" | "-nop" | "-noni" | "-noninteractive"
            ) {
                i += 1;
                continue;
            }
            return None;
        }
        return None;
    }

    if matches!(exe, "cmd" | "cmd.exe") {
        if args.len() == 2 && args[0].eq_ignore_ascii_case("/c") {
            return Some(args[1].clone());
        }
        return None;
    }

    None
}

fn command_mentions_git(cmd: &str) -> bool {
    let mut word = String::new();
    for ch in cmd.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/' | '\\' | '=') {
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

fn token_is_git_reference(token: &str) -> bool {
    let base = executable_basename(token);
    matches!(base, "git" | "git.exe")
        || token.starts_with("GIT_DIR=")
        || token.starts_with("GIT_WORK_TREE=")
        || token.starts_with("GIT_INDEX_FILE=")
        || token.starts_with("GIT_OBJECT_DIRECTORY=")
}

fn classify_single_git_tokens(
    tokens: &[String],
    cd_override: Option<String>,
) -> ClassifiedCommandCall {
    if tokens.is_empty() {
        return ClassifiedCommandCall::NonGit;
    }
    let first = &tokens[0];
    if is_env_assignment(first)
        || matches!(
            executable_basename(first),
            "env" | "command" | "sudo" | "nice" | "nohup" | "time" | "stdbuf"
        )
    {
        return if tokens.iter().any(|t| token_is_git_reference(t)) {
            ClassifiedCommandCall::SkippedCompoundGit
        } else {
            ClassifiedCommandCall::NonGit
        };
    }

    let exe = executable_basename(first);
    if exe != "git" && exe != "git.exe" {
        return ClassifiedCommandCall::NonGit;
    }

    let args = &tokens[1..];
    if args.is_empty() {
        return ClassifiedCommandCall::NonGit;
    }

    // R1: Any global git options before the subcommand (`-C`, `--git-dir`, `--work-tree`, `-c`, etc.)
    // reject the call and count it in `unparsed_compound_git_calls`.
    let subcmd = args[0].as_str();
    if subcmd.starts_with('-') {
        return ClassifiedCommandCall::SkippedCompoundGit;
    }

    let rest = &args[1..];
    for arg in rest {
        let a = arg.as_str();
        if a == "--git-dir"
            || a.starts_with("--git-dir=")
            || a == "--work-tree"
            || a.starts_with("--work-tree=")
        {
            return ClassifiedCommandCall::SkippedCompoundGit;
        }
    }

    let kind = match subcmd {
        "rev-parse" => classify_rev_parse(rest),
        "log" => classify_log(rest),
        "show" => classify_show(rest),
        "rev-list" => classify_rev_list(rest),
        "cat-file" => classify_cat_file(rest),
        "commit" => Some(GitCommandKind::Commit),
        "symbolic-ref" => classify_symbolic_ref(rest),
        "remote" => classify_remote(rest),
        "branch" => {
            if rest.len() == 1 && rest[0] == "--show-current" {
                Some(GitCommandKind::CurrentBranch {
                    checkout_branch: None,
                    detaches_head: false,
                })
            } else {
                None
            }
        }
        "status" => Some(GitCommandKind::CurrentBranch {
            checkout_branch: None,
            detaches_head: false,
        }),
        "checkout" | "switch" => classify_checkout_or_switch(rest),
        _ => None,
    };

    match kind {
        Some(k) => ClassifiedCommandCall::Allowed(ClassifiedGitSubcommand {
            kind: k,
            cwd_override: cd_override.and_then(|d| normalize_cwd(&d)),
        }),
        None => ClassifiedCommandCall::NonGit,
    }
}

fn classify_rev_parse(rest: &[String]) -> Option<GitCommandKind> {
    if rest.is_empty() {
        return None;
    }
    let non_quiet: Vec<&str> = rest
        .iter()
        .map(String::as_str)
        .filter(|&a| a != "-q" && a != "--quiet")
        .collect();
    // 1. P2: `git rev-parse --show-object-format`
    if non_quiet == ["--show-object-format"] {
        return Some(GitCommandKind::ShowObjectFormat);
    }
    // 2. Branch observation: `git rev-parse --abbrev-ref HEAD` (strictly HEAD)
    if non_quiet == ["--abbrev-ref", "HEAD"] {
        return Some(GitCommandKind::CurrentBranch {
            checkout_branch: None,
            detaches_head: false,
        });
    }
    // 3. R3: Commit identity is ONLY the exact argument `HEAD` (with optional --short).
    // Raw hex, tags, branches, and other refs contribute nothing.
    let mut short = false;
    let mut positional: Vec<&str> = Vec::new();
    for arg in rest {
        let s = arg.as_str();
        if matches!(s, "-q" | "--quiet" | "--verify" | "--end-of-options") {
            continue;
        }
        if s == "--short" {
            short = true;
            continue;
        }
        if let Some(n) = s.strip_prefix("--short=") {
            if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            short = true;
            continue;
        }
        if s.starts_with('-') {
            return None;
        }
        positional.push(s);
    }
    if positional.as_slice() == ["HEAD"] {
        Some(GitCommandKind::RevParseCommit { short })
    } else {
        None
    }
}

fn parse_positive_count(s: &str) -> Option<usize> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n = s.parse::<usize>().ok()?;
    (n > 0).then_some(n)
}

fn is_valid_log_revision(rev: &str) -> bool {
    if rev == "HEAD" {
        return true;
    }
    if rev.contains(':')
        || rev.contains("..")
        || rev.contains("^{")
        || rev.contains("@{")
        || rev.contains('~')
        || rev.contains('^')
        || rev.contains('\\')
    {
        return false;
    }
    is_safe_branch_name(rev)
}

fn classify_log(rest: &[String]) -> Option<GitCommandKind> {
    let mut format_spec: Option<String> = None;
    let mut max_count: Option<usize> = None;
    let mut revisions: Vec<&str> = Vec::new();
    let mut i = 0;

    while i < rest.len() {
        let arg = rest[i].as_str();
        if let Some(val) = arg
            .strip_prefix("--format=")
            .or_else(|| arg.strip_prefix("--pretty="))
        {
            format_spec = Some(val.to_string());
            i += 1;
            continue;
        }
        if arg == "--format" || arg == "--pretty" {
            let val = rest.get(i + 1)?;
            format_spec = Some(val.clone());
            i += 2;
            continue;
        }
        if arg == "-n" || arg == "--max-count" {
            let n_str = rest.get(i + 1)?;
            max_count = Some(parse_positive_count(n_str)?);
            i += 2;
            continue;
        }
        if let Some(n_str) = arg
            .strip_prefix("--max-count=")
            .or_else(|| arg.strip_prefix("-n"))
        {
            max_count = Some(parse_positive_count(n_str)?);
            i += 1;
            continue;
        }
        if let Some(n_str) = arg.strip_prefix('-') {
            if let Some(n) = parse_positive_count(n_str) {
                max_count = Some(n);
                i += 1;
                continue;
            }
            return None;
        }
        if !is_valid_log_revision(arg) {
            return None;
        }
        revisions.push(arg);
        i += 1;
    }

    if revisions.len() > 1 {
        return None;
    }
    let is_head =
        max_count == Some(1) && (revisions.is_empty() || revisions.as_slice() == ["HEAD"]);
    let raw_fmt = format_spec?;
    match normalize_git_format_spec(&raw_fmt) {
        "%H" => Some(GitCommandKind::LogCommitOnly { is_head }),
        "%H %P" => Some(GitCommandKind::LogCommitAndParents { is_head }),
        _ => None,
    }
}

fn classify_show(rest: &[String]) -> Option<GitCommandKind> {
    let mut format_spec: Option<String> = None;
    let mut saw_no_patch = false;
    let mut revisions: Vec<&str> = Vec::new();
    let mut i = 0;

    while i < rest.len() {
        let arg = rest[i].as_str();
        if matches!(arg, "-s" | "--no-patch") {
            saw_no_patch = true;
            i += 1;
            continue;
        }
        if let Some(val) = arg
            .strip_prefix("--format=")
            .or_else(|| arg.strip_prefix("--pretty="))
        {
            format_spec = Some(val.to_string());
            i += 1;
            continue;
        }
        if arg == "--format" || arg == "--pretty" {
            let val = rest.get(i + 1)?;
            format_spec = Some(val.clone());
            i += 2;
            continue;
        }
        if arg.starts_with('-') {
            return None;
        }
        if !is_valid_log_revision(arg) {
            return None;
        }
        revisions.push(arg);
        i += 1;
    }

    if !saw_no_patch || revisions.len() > 1 {
        return None;
    }
    let raw_fmt = format_spec?;
    match normalize_git_format_spec(&raw_fmt) {
        "%H" => Some(GitCommandKind::LogCommitOnly { is_head: false }),
        "%H %P" => Some(GitCommandKind::LogCommitAndParents { is_head: false }),
        _ => None,
    }
}

fn normalize_git_format_spec(raw: &str) -> &str {
    let mut s = raw.trim();
    s = strip_matching_quotes(s);
    if let Some(after) = s.strip_prefix("tformat:") {
        s = strip_matching_quotes(after);
    } else if let Some(after) = s.strip_prefix("format:") {
        s = strip_matching_quotes(after);
    }
    s
}

fn strip_matching_quotes(s: &str) -> &str {
    if s.len() >= 2
        && ((s.starts_with('\'') && s.ends_with('\'')) || (s.starts_with('"') && s.ends_with('"')))
    {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

fn is_valid_rev_list_revision(rev: &str) -> bool {
    if let Some((left, right)) = rev.split_once("...") {
        let l_ok = left.is_empty() || is_valid_log_revision(left);
        let r_ok = right.is_empty() || is_valid_log_revision(right);
        return l_ok && r_ok && (!left.is_empty() || !right.is_empty());
    }
    if let Some((left, right)) = rev.split_once("..") {
        let l_ok = left.is_empty() || is_valid_log_revision(left);
        let r_ok = right.is_empty() || is_valid_log_revision(right);
        return l_ok && r_ok && (!left.is_empty() || !right.is_empty());
    }
    is_valid_log_revision(rev)
}

fn classify_rev_list(rest: &[String]) -> Option<GitCommandKind> {
    let mut has_parents = false;
    let mut revisions: Vec<&str> = Vec::new();
    let mut i = 0;

    while i < rest.len() {
        let arg = rest[i].as_str();
        if arg == "--parents" {
            has_parents = true;
            i += 1;
            continue;
        }
        if arg == "-n" || arg == "--max-count" {
            let n_str = rest.get(i + 1)?;
            let _ = parse_positive_count(n_str)?;
            i += 2;
            continue;
        }
        if let Some(n_str) = arg
            .strip_prefix("--max-count=")
            .or_else(|| arg.strip_prefix("-n"))
        {
            let _ = parse_positive_count(n_str)?;
            i += 1;
            continue;
        }
        if let Some(n_str) = arg.strip_prefix('-') {
            if parse_positive_count(n_str).is_some() {
                i += 1;
                continue;
            }
            return None;
        }
        if !is_valid_rev_list_revision(arg) {
            return None;
        }
        revisions.push(arg);
        i += 1;
    }

    (has_parents && !revisions.is_empty())
        .then_some(GitCommandKind::LogCommitAndParents { is_head: false })
}

fn classify_cat_file(rest: &[String]) -> Option<GitCommandKind> {
    let mut saw_p_or_commit = false;
    let mut positional: Vec<&str> = Vec::new();
    for arg in rest {
        let s = arg.as_str();
        if s == "-p" {
            saw_p_or_commit = true;
            continue;
        }
        if s.starts_with('-') {
            return None;
        }
        positional.push(s);
    }
    let target = match positional.as_slice() {
        [single] if saw_p_or_commit => *single,
        ["commit", sha] if !saw_p_or_commit => *sha,
        _ => return None,
    };
    if !is_canonical_commit_sha(target) {
        return None;
    }
    Some(GitCommandKind::CatFileCommit {
        target_sha: target.to_string(),
    })
}

fn classify_symbolic_ref(rest: &[String]) -> Option<GitCommandKind> {
    let mut short = false;
    let mut positional: Vec<&str> = Vec::new();
    for arg in rest {
        let s = arg.as_str();
        if matches!(s, "-q" | "--quiet") {
            continue;
        }
        if s == "--short" {
            short = true;
            continue;
        }
        if s.starts_with('-') {
            return None;
        }
        positional.push(s);
    }
    if positional.len() != 1 {
        return None;
    }
    if !short && positional[0] == "refs/remotes/origin/HEAD" {
        Some(GitCommandKind::SymbolicDefaultBranch)
    } else if short && positional[0] == "HEAD" {
        Some(GitCommandKind::CurrentBranch {
            checkout_branch: None,
            detaches_head: false,
        })
    } else {
        None
    }
}

fn classify_remote(rest: &[String]) -> Option<GitCommandKind> {
    let positional: Vec<&str> = rest
        .iter()
        .map(String::as_str)
        .filter(|&a| a != "-n")
        .collect();
    if positional == ["show", "origin"] {
        Some(GitCommandKind::RemoteShowOrigin)
    } else {
        None
    }
}

fn looks_like_detached_checkout_target(target: &str) -> bool {
    if target == "HEAD"
        || target.contains('~')
        || target.contains('^')
        || target.contains("@{")
        || target.starts_with("refs/tags/")
        || target.starts_with("tags/")
        || target.starts_with("remotes/")
        || target.starts_with("origin/")
    {
        return true;
    }
    if (4..=64).contains(&target.len()) && target.bytes().all(|b| b.is_ascii_hexdigit()) {
        return true;
    }
    if target.starts_with('v')
        && target.len() > 1
        && target.as_bytes()[1].is_ascii_digit()
        && target.contains('.')
    {
        return true;
    }
    false
}

fn classify_checkout_or_switch(rest: &[String]) -> Option<GitCommandKind> {
    if rest.is_empty() || rest[0] == "--" {
        return None;
    }
    let mut created_branch: Option<String> = None;
    let mut positional: Vec<&str> = Vec::new();
    let mut i = 0;

    while i < rest.len() {
        let a = rest[i].as_str();
        if a == "--" {
            break;
        }
        if matches!(a, "-d" | "--detach") {
            return Some(GitCommandKind::CurrentBranch {
                checkout_branch: None,
                detaches_head: true,
            });
        }
        if matches!(a, "-b" | "-B" | "-c" | "-C" | "--orphan") {
            let b = rest.get(i + 1)?;
            if is_safe_branch_name(b) && b != "HEAD" {
                created_branch = Some(b.clone());
            } else {
                return Some(GitCommandKind::CurrentBranch {
                    checkout_branch: None,
                    detaches_head: true,
                });
            }
            i += 2;
            continue;
        }
        if matches!(
            a,
            "-f" | "--force"
                | "-q"
                | "--quiet"
                | "-m"
                | "--merge"
                | "--guess"
                | "--no-guess"
                | "--track"
                | "-t"
                | "--no-track"
        ) {
            i += 1;
            continue;
        }
        if a.starts_with('-') {
            return Some(GitCommandKind::CurrentBranch {
                checkout_branch: None,
                detaches_head: true,
            });
        }
        positional.push(a);
        i += 1;
    }

    if let Some(b) = created_branch {
        return Some(GitCommandKind::CurrentBranch {
            checkout_branch: Some(b),
            detaches_head: false,
        });
    }

    if let [target] = positional.as_slice() {
        if looks_like_detached_checkout_target(target) || !is_safe_branch_name(target) {
            return Some(GitCommandKind::CurrentBranch {
                checkout_branch: None,
                detaches_head: true,
            });
        }
        return Some(GitCommandKind::CurrentBranch {
            checkout_branch: Some((*target).to_string()),
            detaches_head: false,
        });
    }

    None
}

fn is_env_assignment(token: &str) -> bool {
    let Some((lhs, _)) = token.split_once('=') else {
        return false;
    };
    !lhs.is_empty()
        && !lhs.starts_with('-')
        && lhs.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

fn executable_basename(token: &str) -> &str {
    token.rsplit(['/', '\\']).next().unwrap_or(token).trim()
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

fn resolve_cwd_path(base: Option<&str>, dir_arg: &str) -> String {
    let clean_arg = dir_arg.trim().replace('\\', "/");
    let is_abs =
        clean_arg.starts_with('/') || (clean_arg.len() >= 2 && clean_arg.as_bytes()[1] == b':');
    if is_abs {
        normalize_cwd(&clean_arg).unwrap_or(clean_arg)
    } else if let Some(b) = base.and_then(normalize_cwd) {
        let joined = format!("{b}/{clean_arg}");
        normalize_cwd(&joined).unwrap_or(joined)
    } else {
        normalize_cwd(&clean_arg).unwrap_or(clean_arg)
    }
}

/// Returns `true` if `tool_name` is in the strict allow-list of known shell execution tool
/// Returns `true` if `tool_name` is in the strict allow-list of known shell execution tool
/// names for `harness`. Unknown or absent harness or tool names always return `false` (R7).
#[must_use]
pub fn is_shell_execution_tool_for_harness(harness: Option<&str>, tool_name: Option<&str>) -> bool {
    let Some(raw_tool) = tool_name.map(str::trim).filter(|s| !s.is_empty()) else {
        return false;
    };
    let Some(raw_harness) = harness.map(str::trim).filter(|s| !s.is_empty()) else {
        return false;
    };
    match CanonicalHarness::from_alias(raw_harness) {
        Some(CanonicalHarness::Claude) => matches!(raw_tool, "Bash" | "bash"),
        Some(CanonicalHarness::Codex) => matches!(
            raw_tool,
            "CommandExecution"
                | "shell"
                | "local_shell"
                | "local_shell_call"
                | "exec_command"
                | "shell_command"
                | "container.exec"
                | "bash"
                | "Bash"
        ),
        Some(CanonicalHarness::Antigravity) => {
            matches!(raw_tool, "run_command" | "bash" | "Bash" | "shell" | "sh")
        }
        Some(CanonicalHarness::Pi) => matches!(raw_tool, "bash" | "Bash" | "sh"),
        Some(CanonicalHarness::Grok) => matches!(
            raw_tool,
            "bash" | "Bash" | "shell" | "sh" | "run_command" | "execute_command"
        ),
        None => false,
    }
}

/// Returns `true` if `tool_name` is in the strict allow-list of known shell execution tool names
/// across supported harnesses (`Claude`, `Codex`, `Pi`, `Grok`).
/// Unknown or empty tool names always return `false` (R7).
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
    /// `true` only when established as `HEAD` by `git rev-parse HEAD` or `git log -1`/`-n 1`
    /// with no revision argument (or revision `HEAD`).
    pub is_head_commit: bool,
}

/// Parsed evidence from a single allow-listed git command's output.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ParsedGitOutput {
    pub commits: Vec<ParsedCommitObservation>,
    pub unresolved_abbreviated: u64,
    pub observed_branch: Option<String>,
    pub detached_head: bool,
    pub observed_default_branch: Option<String>,
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
/// Under R2, output lines must match the expected shape EXACTLY with NO leading whitespace
/// (trailing CR/LF only). Any non-conforming line makes the WHOLE output contribute nothing.
#[must_use]
pub fn parse_git_output(kind: &GitCommandKind, raw_output: &str) -> ParsedGitOutput {
    let mut out = ParsedGitOutput::default();
    if matches!(kind, GitCommandKind::NonGit) || raw_output.len() > MAX_GIT_OUTPUT_BYTES {
        return out;
    }

    let cleaned = strip_output_envelope(raw_output);
    let body = cleaned.trim_end_matches(['\r', '\n']);
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
        GitCommandKind::SymbolicDefaultBranch => {
            if raw_lines.len() == 1 {
                if let Some(def) = extract_default_branch_from_symbolic_ref(raw_lines[0]) {
                    out.observed_default_branch = Some(def);
                }
            }
        }
        GitCommandKind::RemoteShowOrigin => {
            for &line in &raw_lines {
                let trimmed = line.trim();
                if let Some(rest) = trimmed.strip_prefix("HEAD branch:") {
                    let branch = rest.trim();
                    if !branch.is_empty()
                        && branch != "(unknown)"
                        && branch != "unknown"
                        && is_safe_branch_name(branch)
                    {
                        out.observed_default_branch = Some(branch.to_string());
                    }
                }
            }
        }
        GitCommandKind::CurrentBranch {
            checkout_branch,
            detaches_head,
        } => {
            if *detaches_head {
                out.detached_head = true;
                return out;
            }
            if let Some(b) = checkout_branch {
                out.observed_branch = Some(b.clone());
            }
            for &line in &raw_lines {
                if line == "HEAD"
                    || line.starts_with("HEAD detached ")
                    || line.starts_with("HEAD is now at ")
                    || line.starts_with("Note: switching to ")
                {
                    out.detached_head = true;
                    out.observed_branch = None;
                    return out;
                }
                if let Some(rest) = line.strip_prefix("On branch ") {
                    if !rest.contains(char::is_whitespace) && is_safe_branch_name(rest) {
                        out.observed_branch = Some(rest.to_string());
                    }
                } else if let Some(rest) = line
                    .strip_prefix("Switched to a new branch '")
                    .or_else(|| line.strip_prefix("Switched to branch '"))
                {
                    if let Some(branch) = rest.strip_suffix('\'') {
                        if is_safe_branch_name(branch) {
                            out.observed_branch = Some(branch.to_string());
                        }
                    }
                } else if raw_lines.len() == 1
                    && !line.is_empty()
                    && !line.contains(char::is_whitespace)
                    && !is_canonical_commit_sha(line)
                    && !is_abbreviated_commit_sha(line)
                    && is_safe_branch_name(line)
                {
                    out.observed_branch = Some(line.to_string());
                }
            }
        }
        GitCommandKind::Commit => {
            for &line in &raw_lines {
                if let Some((branch_opt, is_detached, is_abbrev)) =
                    parse_git_commit_bracket_line(line)
                {
                    if is_detached {
                        out.detached_head = true;
                    } else if let Some(b) = branch_opt {
                        out.observed_branch = Some(b);
                    }
                    if is_abbrev {
                        out.unresolved_abbreviated += 1;
                    }
                }
            }
        }
        GitCommandKind::RevParseCommit { short } => {
            if raw_lines.len() != 1 {
                return ParsedGitOutput::default();
            }
            let token = raw_lines[0];
            if token.is_empty() || token.contains(char::is_whitespace) {
                return ParsedGitOutput::default();
            }
            if *short {
                if is_abbreviated_commit_sha(token) || is_canonical_commit_sha(token) {
                    out.unresolved_abbreviated += 1;
                } else {
                    return ParsedGitOutput::default();
                }
            } else if is_canonical_commit_sha(token) {
                out.commits.push(ParsedCommitObservation {
                    commit_sha: token.to_string(),
                    parents: None,
                    merge_missing_reason: Some(MERGE_REASON_PARENTS_NOT_RECORDED),
                    is_head_commit: true,
                });
            } else if is_abbreviated_commit_sha(token) {
                out.unresolved_abbreviated += 1;
            } else {
                return ParsedGitOutput::default();
            }
        }
        GitCommandKind::LogCommitOnly { is_head } => {
            if raw_lines.is_empty() {
                return ParsedGitOutput::default();
            }
            let single_head = *is_head && raw_lines.len() == 1;
            for &line in &raw_lines {
                if line.is_empty() || line.contains(char::is_whitespace) {
                    return ParsedGitOutput::default();
                }
                if is_canonical_commit_sha(line) {
                    out.commits.push(ParsedCommitObservation {
                        commit_sha: line.to_string(),
                        parents: None,
                        merge_missing_reason: Some(MERGE_REASON_PARENTS_NOT_RECORDED),
                        is_head_commit: single_head,
                    });
                } else if is_abbreviated_commit_sha(line) {
                    out.unresolved_abbreviated += 1;
                } else {
                    return ParsedGitOutput::default();
                }
            }
        }
        GitCommandKind::LogCommitAndParents { is_head } => {
            if raw_lines.is_empty() {
                return ParsedGitOutput::default();
            }
            let single_head = *is_head && raw_lines.len() == 1;
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
                // R2: Every token on every line must be a valid canonical or abbreviated SHA;
                // any non-conforming line makes the WHOLE output contribute nothing.
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
                        is_head_commit: single_head,
                    });
                } else {
                    let commit_len = first.len();
                    if parent_tokens.iter().all(|t| t.len() == commit_len) {
                        out.commits.push(ParsedCommitObservation {
                            commit_sha: first.to_string(),
                            parents: Some(parent_tokens.iter().map(|s| (*s).to_string()).collect()),
                            merge_missing_reason: None,
                            is_head_commit: single_head,
                        });
                    } else {
                        out.commits.push(ParsedCommitObservation {
                            commit_sha: first.to_string(),
                            parents: None,
                            merge_missing_reason: Some(MERGE_REASON_MIXED_PARENT_OBJECT_FORMAT),
                            is_head_commit: single_head,
                        });
                    }
                }
            }
        }
        GitCommandKind::CatFileCommit { target_sha } => {
            if let Some(parsed_cat) = parse_cat_file_commit(&raw_lines, target_sha) {
                out.unresolved_abbreviated += parsed_cat.unresolved_abbreviated;
                out.commits.extend(parsed_cat.commits);
            }
        }
    }

    out
}

fn strip_output_envelope(raw: &str) -> &str {
    let mut rest = raw;
    let mut saw_header = false;
    while !rest.is_empty() {
        let (line_raw, after) = match rest.split_once('\n') {
            Some((l, a)) => (l, a),
            None => (rest, ""),
        };
        let trimmed = line_raw.strip_suffix('\r').unwrap_or(line_raw).trim();
        let lower = trimmed.to_ascii_lowercase();
        let is_meta = [
            "chunk id:",
            "wall time:",
            "original token count:",
            "total output lines:",
            "session id:",
            "process running with session id",
            "process exited with exit code",
            "process exited with code",
            "exit code:",
            "exit_code:",
        ]
        .iter()
        .any(|p| lower.starts_with(p));
        if is_meta {
            saw_header = true;
            rest = after;
            continue;
        }
        if trimmed.eq_ignore_ascii_case("output:") {
            return after;
        }
        if saw_header && trimmed.is_empty() {
            rest = after;
            continue;
        }
        break;
    }
    if saw_header {
        rest
    } else {
        raw
    }
}

fn parse_git_commit_bracket_line(line: &str) -> Option<(Option<String>, bool, bool)> {
    let rest = line.strip_prefix('[')?;
    let (bracket_inner, _after) = rest.split_once(']')?;
    let tokens: Vec<&str> = bracket_inner.split_whitespace().collect();
    if tokens.len() < 2 {
        return None;
    }
    let (branch, is_detached, sha_token) = match tokens.as_slice() {
        [branch, "(root-commit)", sha] => (Some(*branch), false, *sha),
        ["detached", "HEAD", sha] => (None, true, *sha),
        [branch, sha] => (Some(*branch), false, *sha),
        _ => return None,
    };
    if let Some(b) = branch {
        if !is_safe_branch_name(b) {
            return None;
        }
    }
    if is_abbreviated_commit_sha(sha_token) {
        Some((branch.map(str::to_string), is_detached, true))
    } else if is_canonical_commit_sha(sha_token) {
        Some((branch.map(str::to_string), is_detached, false))
    } else {
        None
    }
}

fn parse_cat_file_commit(lines: &[&str], target_sha: &str) -> Option<ParsedGitOutput> {
    let first = *lines.first()?;
    let tree_sha = first.strip_prefix("tree ")?;
    if tree_sha.contains(char::is_whitespace)
        || (!is_canonical_commit_sha(tree_sha) && !is_abbreviated_commit_sha(tree_sha))
    {
        return None;
    }
    let mut parents = Vec::new();
    let mut saw_abbrev_parent = false;
    let mut unresolved_abbreviated = 0u64;

    for &line in &lines[1..] {
        if line.is_empty() {
            break;
        }
        if let Some(p_sha) = line.strip_prefix("parent ") {
            if p_sha.contains(char::is_whitespace) {
                return None;
            }
            if is_canonical_commit_sha(p_sha) && parents.len() < MAX_PARENTS_PER_COMMIT {
                parents.push(p_sha.to_string());
            } else if is_abbreviated_commit_sha(p_sha) {
                saw_abbrev_parent = true;
                unresolved_abbreviated += 1;
            } else {
                return None;
            }
        }
    }

    let commit_len = target_sha.len();
    let (obs_parents, reason) = if saw_abbrev_parent {
        (None, Some(MERGE_REASON_ABBREVIATED_PARENTS))
    } else if !parents.iter().all(|p| p.len() == commit_len) {
        (None, Some(MERGE_REASON_MIXED_PARENT_OBJECT_FORMAT))
    } else {
        (Some(parents), None)
    };

    Some(ParsedGitOutput {
        commits: vec![ParsedCommitObservation {
            commit_sha: target_sha.to_string(),
            parents: obs_parents,
            merge_missing_reason: reason,
            is_head_commit: false,
        }],
        unresolved_abbreviated,
        ..Default::default()
    })
}

fn extract_default_branch_from_symbolic_ref(line: &str) -> Option<String> {
    if line.contains(char::is_whitespace) {
        return None;
    }
    let branch = line.strip_prefix("refs/remotes/origin/")?;
    if branch.is_empty() || branch == "HEAD" || !is_safe_branch_name(branch) {
        return None;
    }
    Some(branch.to_string())
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
    merge_missing_reason: Option<&'static str>,
    observed_at: Option<DateTime<Utc>>,
    sessions: BTreeSet<String>,
    saw_unmatched_session: bool,
    saw_conflicted_session: bool,
    on_default_branch_observations: BTreeSet<bool>,
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
    /// Most recent working directory observed per session from harness records.
    session_cwd: BTreeMap<String, String>,
    /// Current branch observed per `(session, cwd)` from a record in that `cwd`
    /// (invalidated when `git checkout` / `git switch` detaches `HEAD`).
    branch_by_session_cwd: BTreeMap<(String, String), String>,
    /// Default branch observed via `git symbolic-ref refs/remotes/origin/HEAD` or
    /// `git remote show origin` per `(session, cwd)`.
    default_branch_by_session_cwd: BTreeMap<(String, String), String>,
    /// Object format (`sha1` vs `sha256`) observed via `git rev-parse --show-object-format`
    /// per `(session, cwd_opt)`.
    object_format_by_session_cwd: BTreeMap<(String, Option<String>), GitObjectFormat>,
}

impl GitEvidenceScanner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record structured session context (`cwd` and current `gitBranch`) from a harness record.
    ///
    /// Per R4(a), the current branch is recorded ONLY for the explicit `cwd` on that record
    /// (never inherited from another `cwd`, and never stored in a session-wide fallback).
    pub fn observe_record_context(
        &mut self,
        raw_session: Option<&str>,
        header_conflict: bool,
        cwd: Option<&str>,
        branch: Option<&str>,
    ) {
        if header_conflict {
            return;
        }
        let Some(sess) = raw_session.map(str::trim).filter(|s| !s.is_empty()) else {
            return;
        };
        let Some(c) = cwd.and_then(normalize_cwd) else {
            return;
        };
        self.session_cwd.insert(sess.to_string(), c.clone());
        if let Some(raw_b) = branch.map(str::trim).filter(|s| !s.is_empty()) {
            if raw_b == "HEAD" || raw_b.to_ascii_lowercase().contains("detached") {
                self.branch_by_session_cwd.remove(&(sess.to_string(), c));
            } else if is_safe_branch_name(raw_b) {
                self.branch_by_session_cwd
                    .insert((sess.to_string(), c), raw_b.to_string());
            }
        }
    }

    /// Register a tool call by explicit `call_id` in the bounded LRU.
    ///
    /// Under R7, only tool names in the strict shell allow-list (`is_shell_execution_tool_name`)
    /// have their command inspected; unknown or absent tool names are treated as non-shell tools.
    pub fn observe_tool_call_with_context(
        &mut self,
        call_id: Option<&str>,
        tool_name: Option<&str>,
        command: Option<&str>,
        cwd: Option<&str>,
    ) {
        let Some(id) = call_id.map(str::trim).filter(|s| !s.is_empty()) else {
            return;
        };
        let is_shell = tool_name.is_some_and(is_shell_execution_tool_name);
        let norm_call_cwd = cwd.and_then(normalize_cwd);
        let command_call = if is_shell {
            match command {
                Some(cmd) => {
                    let classified = classify_single_git_command(cmd);
                    if matches!(classified, ClassifiedCommandCall::SkippedCompoundGit) {
                        self.unparsed_compound_git_calls += 1;
                    } else if let ClassifiedCommandCall::Allowed(ref sub) = classified {
                        if let GitCommandKind::CurrentBranch {
                            detaches_head: true,
                            ..
                        } = sub.kind
                        {
                            let target_cwd = match (&norm_call_cwd, &sub.cwd_override) {
                                (Some(b), Some(ov)) => Some(resolve_cwd_path(Some(b.as_str()), ov)),
                                (None, Some(ov)) => normalize_cwd(ov),
                                (Some(b), None) => Some(b.clone()),
                                (None, None) => None,
                            };
                            if let Some(ref c) = target_cwd {
                                self.branch_by_session_cwd
                                    .retain(|(_, existing_cwd), _| existing_cwd != c);
                            }
                        }
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

    /// Convenience wrapper for registering a shell (`Bash`) tool call without explicit cwd.
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
    /// Unmatched tool results (`call_id` missing or not in the bounded LRU) are ignored.
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
        if failed == Some(true) {
            return;
        }
        let clean_session = if header_conflict {
            None
        } else {
            raw_session.map(str::trim).filter(|s| !s.is_empty())
        };
        if let (Some(sess), Some(c)) = (clean_session, cwd.and_then(normalize_cwd)) {
            self.session_cwd.insert(sess.to_string(), c);
        }

        let Some(id) = call_id.map(str::trim).filter(|s| !s.is_empty()) else {
            return;
        };

        let pending = if let Some(found) = self.lookup_pending_lru(id) {
            found
        } else if let Some(cmd) = atomic_command {
            // Atomic single-record execution (e.g. Codex `CommandExecution` item_completed)
            // that carries its own explicit `id` and `command` on the same record.
            let command_call = classify_single_git_command(cmd);
            if matches!(command_call, ClassifiedCommandCall::SkippedCompoundGit) {
                self.unparsed_compound_git_calls += 1;
            }
            let call = PendingToolCall {
                command_call,
                cwd: cwd.and_then(normalize_cwd),
            };
            self.insert_pending_lru(id.to_string(), call.clone());
            call
        } else {
            // Unmatched tool result or evicted from LRU -> ignore!
            return;
        };

        let ClassifiedCommandCall::Allowed(sub) = pending.command_call else {
            return;
        };

        let base_cwd = pending
            .cwd
            .or_else(|| cwd.and_then(normalize_cwd))
            .or_else(|| clean_session.and_then(|s| self.session_cwd.get(s).cloned()));

        let effective_cwd = match (&base_cwd, &sub.cwd_override) {
            (Some(b), Some(ov)) => Some(resolve_cwd_path(Some(b.as_str()), ov)),
            (None, Some(ov)) => normalize_cwd(ov),
            (Some(b), None) => Some(b.clone()),
            (None, None) => None,
        };

        let parsed = parse_git_output(&sub.kind, raw_output);
        self.unresolved_abbreviated += parsed.unresolved_abbreviated;

        if let Some(fmt) = parsed.observed_object_format {
            if let Some(sess) = clean_session {
                self.object_format_by_session_cwd
                    .insert((sess.to_string(), effective_cwd.clone()), fmt);
            }
        }

        if parsed.detached_head {
            if let (Some(sess), Some(c)) = (clean_session, effective_cwd.as_ref()) {
                self.branch_by_session_cwd
                    .remove(&(sess.to_string(), c.clone()));
            }
        } else if let Some(ref b) = parsed.observed_branch {
            if let (Some(sess), Some(c)) = (clean_session, effective_cwd.as_ref()) {
                self.branch_by_session_cwd
                    .insert((sess.to_string(), c.clone()), b.clone());
            }
        }

        if let Some(def) = parsed.observed_default_branch {
            if let (Some(sess), Some(c)) = (clean_session, effective_cwd.as_ref()) {
                self.default_branch_by_session_cwd
                    .insert((sess.to_string(), c.clone()), def);
            }
        }

        for obs in parsed.commits {
            self.record_commit_observation(
                raw_session,
                header_conflict,
                ts,
                effective_cwd.as_deref(),
                obs,
            );
        }
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
        obs: ParsedCommitObservation,
    ) {
        if !is_canonical_commit_sha(&obs.commit_sha) {
            return;
        }
        let clean_session = if header_conflict {
            None
        } else {
            raw_session.map(str::trim).filter(|s| !s.is_empty())
        };
        let norm_cwd = effective_cwd.and_then(normalize_cwd);

        // P2: Hash algorithm check per `(session, cwd)`:
        // - 40-hex is accepted as SHA-1 unless the same session and cwd recorded `sha256`.
        // - 64-hex is accepted ONLY when `sha256` was recorded for that session and cwd;
        //   otherwise it is counted as `unverified_object_format` and ignored.
        let recorded_format = clean_session
            .and_then(|sess| {
                self.object_format_by_session_cwd
                    .get(&(sess.to_string(), norm_cwd.clone()))
                    .copied()
            })
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

        // R4: `on_default_branch` is observed ONLY for a commit established as HEAD by
        // `git rev-parse HEAD` or `git log -1`/`-n 1` with no revision (or `HEAD`), AND only when
        // at that time and for the SAME session and SAME cwd both the current branch (not detached)
        // and the default branch are known in that cwd.
        let on_default_at_obs = if obs.is_head_commit {
            match (clean_session, norm_cwd.as_ref()) {
                (Some(sess), Some(cwd_str)) => {
                    let key = (sess.to_string(), cwd_str.clone());
                    match (
                        self.default_branch_by_session_cwd.get(&key),
                        self.branch_by_session_cwd.get(&key),
                    ) {
                        (Some(def_branch), Some(cur_branch)) => {
                            Some(cur_branch.as_str() == def_branch.as_str())
                        }
                        _ => None,
                    }
                }
                _ => None,
            }
        } else {
            None
        };

        if let Some(existing) = self.commits_by_sha.get_mut(&obs.commit_sha) {
            if existing.observed_at.is_none() && ts.is_some() {
                existing.observed_at = ts;
            }
            if existing.parents.is_none() && obs.parents.is_some() {
                existing.parents = obs.parents;
                existing.merge_missing_reason = None;
            } else if existing.parents.is_none()
                && (existing.merge_missing_reason.is_none()
                    || (existing.merge_missing_reason == Some(MERGE_REASON_PARENTS_NOT_RECORDED)
                        && obs.merge_missing_reason.is_some()))
            {
                existing.merge_missing_reason = obs.merge_missing_reason;
            }
            if let Some(on_def) = on_default_at_obs {
                existing.on_default_branch_observations.insert(on_def);
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

        let mut on_default_branch_observations = BTreeSet::new();
        if let Some(on_def) = on_default_at_obs {
            on_default_branch_observations.insert(on_def);
        }

        let sha = obs.commit_sha.clone();
        self.commit_order.push(sha.clone());
        self.commits_by_sha.insert(
            sha,
            AccumulatedCommit {
                commit_sha: obs.commit_sha,
                parents: obs.parents,
                merge_missing_reason: obs.merge_missing_reason,
                observed_at: ts,
                sessions,
                saw_unmatched_session,
                saw_conflicted_session,
                on_default_branch_observations,
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

            // Default branch status: observed ONLY if known at observation time in the same
            // `(session, cwd)` without conflict.
            let (default_branch_status, on_default_branch) =
                if acc.on_default_branch_observations.len() == 1 {
                    let val = *acc.on_default_branch_observations.iter().next().unwrap();
                    (DEFAULT_BRANCH_STATUS_OBSERVED.to_string(), Some(val))
                } else {
                    default_branch_not_observed += 1;
                    (DEFAULT_BRANCH_STATUS_NOT_OBSERVED.to_string(), None)
                };

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
    fn strict_allow_list_rejects_non_allow_listed_git_and_non_git_shapes() {
        let sha1 = "1".repeat(40);
        let sha2 = "2".repeat(40);
        let tree_sha = "3".repeat(40);

        // 1. bare `--format=%P` -> nothing
        let kind_bare_p = classify_shell_command("git log -1 --format=%P");
        assert_eq!(kind_bare_p, GitCommandKind::NonGit);
        let out_bare_p = parse_git_output(&kind_bare_p, &format!("{sha1}\n"));
        assert!(out_bare_p.commits.is_empty());

        // 2. `--format='%H %T'` -> no parents (and rejected by strict format allow-list)
        let kind_ht = classify_shell_command("git log -1 --format='%H %T'");
        assert_eq!(kind_ht, GitCommandKind::NonGit);
        let out_ht = parse_git_output(&kind_ht, &format!("{sha1} {tree_sha}\n"));
        assert!(out_ht.commits.is_empty());

        // 3. any 2-hex line from a non-allow-listed command -> nothing
        for non_allowed in [
            "git status",
            "git diff HEAD~1 HEAD",
            "git fetch origin",
            "echo 1111111111111111111111111111111111111111 2222222222222222222222222222222222222222",
            "python3 script.py",
        ] {
            let kind = classify_shell_command(non_allowed);
            let out = parse_git_output(&kind, &format!("{sha1} {sha2}\n"));
            assert!(
                out.commits.is_empty(),
                "non-allow-listed command `{non_allowed}` must produce no commits"
            );
        }

        // 4. `git rev-parse --abbrev-ref @{u}` printing `origin/feat-x` -> no default branch
        let kind_upstream = classify_shell_command("git rev-parse --abbrev-ref @{u}");
        assert_eq!(kind_upstream, GitCommandKind::NonGit);
        let out_upstream = parse_git_output(&kind_upstream, "origin/feat-x\n");
        assert_eq!(out_upstream.observed_default_branch, None);

        // 5. `git branch -r` -> none
        let kind_branch_r = classify_shell_command("git branch -r");
        assert_eq!(kind_branch_r, GitCommandKind::NonGit);
        let out_branch_r = parse_git_output(
            &kind_branch_r,
            "  origin/HEAD -> origin/main\n  origin/main\n",
        );
        assert_eq!(out_branch_r.observed_default_branch, None);
        assert_eq!(out_branch_r.observed_branch, None);

        // 6. `cat-file -p HEAD` -> nothing; `cat-file -p <full>` -> parents bound to that SHA
        let kind_cat_head = classify_shell_command("git cat-file -p HEAD");
        assert_eq!(kind_cat_head, GitCommandKind::NonGit);
        let cat_body = format!(
            "tree {tree_sha}\nparent {sha2}\nauthor Dev <dev@example.com> 1700000000 +0000\ncommitter Dev <dev@example.com> 1700000000 +0000\n\nmsg\n"
        );
        let out_cat_head = parse_git_output(&kind_cat_head, &cat_body);
        assert!(out_cat_head.commits.is_empty());

        let kind_cat_full = classify_shell_command(&format!("git cat-file -p {sha1}"));
        assert_eq!(
            kind_cat_full,
            GitCommandKind::CatFileCommit {
                target_sha: sha1.clone()
            }
        );
        let out_cat_full = parse_git_output(&kind_cat_full, &cat_body);
        assert_eq!(out_cat_full.commits.len(), 1);
        assert_eq!(out_cat_full.commits[0].commit_sha, sha1);
        assert_eq!(out_cat_full.commits[0].parents, Some(vec![sha2.clone()]));

        // 7. `rev-parse HEAD:file`, `HEAD^{tree}`, `hash-object`, `write-tree` -> nothing
        for non_commit_git in [
            "git rev-parse HEAD:src/lib.rs",
            "git rev-parse HEAD^{tree}",
            "git rev-parse HEAD@{1}",
            "git rev-parse origin/main",
            "git hash-object -w README.md",
            "git write-tree",
        ] {
            let kind = classify_shell_command(non_commit_git);
            assert_eq!(
                kind,
                GitCommandKind::NonGit,
                "`{non_commit_git}` must be classified as NonGit"
            );
            let out = parse_git_output(&kind, &format!("{sha1}\n"));
            assert!(out.commits.is_empty());
        }
    }

    #[test]
    fn scanner_handles_600_tool_calls_openssl_rand_agent_mcp_and_object_format_and_cwd() {
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
        assert_eq!(cov_after_unv.unverified_object_format, 1);

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

        // 6. Default branch in `/repo-other` is NOT applied to commits in `/repo-a`
        scanner.observe_record_context(Some("sess-1"), false, Some("/repo-a"), Some("main"));
        scanner.observe_tool_call_with_context(
            Some("call-sym-other"),
            Some("Bash"),
            Some("git symbolic-ref refs/remotes/origin/HEAD"),
            Some("/repo-other"),
        );
        scanner.observe_tool_output_with_cwd(
            Some("call-sym-other"),
            None,
            Some("sess-1"),
            false,
            None,
            Some(false),
            Some("/repo-other"),
            "refs/remotes/origin/main\n",
        );
        let new_sha_in_repo_a = "e".repeat(40);
        scanner.observe_tool_call_with_context(
            Some("call-rp-a"),
            Some("Bash"),
            Some("git rev-parse HEAD"),
            Some("/repo-a"),
        );
        scanner.observe_tool_output_with_cwd(
            Some("call-rp-a"),
            None,
            Some("sess-1"),
            false,
            None,
            Some(false),
            Some("/repo-a"),
            &format!("{new_sha_in_repo_a}\n"),
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
        assert_eq!(facts.len(), 3);
        assert_eq!(cov.observed, 3);
        assert_eq!(cov.unverified_object_format, 1);

        let sealed_256 = seal_commit(Some(b"tool-key"), &sha256_commit);
        let row_256 = facts
            .iter()
            .find(|r| r.commit_sealed == sealed_256)
            .unwrap();
        assert_eq!(row_256.merge_status, "observed");
        assert_eq!(row_256.is_merge, Some(false));

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

        let sealed_repo_a = seal_commit(Some(b"tool-key"), &new_sha_in_repo_a);
        let row_repo_a = facts
            .iter()
            .find(|r| r.commit_sealed == sealed_repo_a)
            .unwrap();
        assert_eq!(row_repo_a.default_branch_status, "not_observed");
        assert_eq!(row_repo_a.on_default_branch, None);
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
            let priv_path: PathBuf = dir.join("private_data_root");
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
    fn reviewer_r1_to_r7_scenarios_compound_log_show_revparse_default_branch_coherence_and_lows() {
        let sha1 = "a".repeat(40);
        let sha2 = "b".repeat(40);
        let sha3 = "c".repeat(40);
        let empty_tree_hex = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

        // R1: Compound git commands, pipes, subshells, redirections, env assignments, and global
        // options (-C, --git-dir, --work-tree, -c) must contribute NOTHING and increment
        // `unparsed_compound_git_calls`. Only a single clean allow-listed git command (optionally
        // preceded by a single `cd <path> &&`) is parsed.
        let mut r1_scanner = GitEvidenceScanner::new();
        let compound_cmds = [
            "git log --format=%H && git hash-object -w README.md",
            "git rev-list --parents HEAD; git rev-list --objects HEAD",
            "git rev-parse HEAD; git log -1 --format='%H %P'",
            "GIT_DIR=/other/.git git rev-parse HEAD",
            "GIT_WORK_TREE=/other git rev-parse HEAD",
            "git -C /other rev-parse HEAD",
            "git -C/other rev-parse HEAD",
            "git --git-dir=/other/.git rev-parse HEAD",
            "git --work-tree=/other rev-parse HEAD",
            "git -c core.abbrev=40 rev-parse HEAD",
            "git rev-parse HEAD | head -n 1",
            "git rev-parse HEAD > /tmp/out",
            "(git rev-parse HEAD)",
            "bash -c 'git rev-parse HEAD; git log -1 --format=%H'",
            "cd /a && cd /b && git rev-parse HEAD",
            "git rev-parse HEAD || true",
        ];
        for (i, cmd) in compound_cmds.iter().enumerate() {
            assert_eq!(
                classify_shell_command(cmd),
                GitCommandKind::NonGit,
                "R1: `{cmd}` must be rejected as NonGit"
            );
            let cid = format!("r1-{i}");
            r1_scanner.observe_tool_call_with_context(
                Some(&cid),
                Some("Bash"),
                Some(cmd),
                Some("/repo"),
            );
            r1_scanner.observe_tool_output_with_cwd(
                Some(&cid),
                None,
                Some("sess-r1"),
                false,
                None,
                Some(false),
                Some("/repo"),
                &format!("{sha1} {sha2}\n{sha3}\n"),
            );
        }
        let (r1_facts, r1_cov) = r1_scanner.materialize(
            "src-r1",
            1,
            "claude",
            false,
            Some(b"tool-key"),
            &FactExportConfig::unconfigured(),
            |s| format!("sealed:{s}"),
        );
        assert!(
            r1_facts.is_empty(),
            "R1: compound/env/global-flag git calls must contribute nothing"
        );
        assert_eq!(
            r1_cov.unparsed_compound_git_calls,
            compound_cmds.len() as i64,
            "R1: every skipped compound/env/global-flag git call must be counted in unparsed_compound_git_calls"
        );

        // Single leading `cd <path> && <git ...>` IS allowed and sets cwd for that call only:
        r1_scanner.observe_record_context(Some("sess-r1"), false, Some("/repo-cd"), Some("main"));
        r1_scanner.observe_tool_call_with_context(
            Some("r1-cd-sym"),
            Some("Bash"),
            Some("cd /repo-cd && git symbolic-ref refs/remotes/origin/HEAD"),
            Some("/start-dir"),
        );
        r1_scanner.observe_tool_output_with_cwd(
            Some("r1-cd-sym"),
            None,
            Some("sess-r1"),
            false,
            None,
            Some(false),
            Some("/start-dir"),
            "refs/remotes/origin/main\n",
        );
        r1_scanner.observe_tool_call_with_context(
            Some("r1-cd-rp"),
            Some("Bash"),
            Some("cd /repo-cd && git rev-parse HEAD"),
            Some("/start-dir"),
        );
        r1_scanner.observe_tool_output_with_cwd(
            Some("r1-cd-rp"),
            None,
            Some("sess-r1"),
            false,
            None,
            Some(false),
            Some("/start-dir"),
            &format!("{sha1}\n"),
        );
        let (r1_cd_facts, _) = r1_scanner.materialize(
            "src-r1",
            1,
            "claude",
            false,
            Some(b"tool-key"),
            &FactExportConfig::unconfigured(),
            |s| format!("sealed:{s}"),
        );
        assert_eq!(r1_cd_facts.len(), 1);
        assert_eq!(r1_cd_facts[0].on_default_branch, Some(true));

        // R2 & R3: Disallowed log, show, rev-list, and rev-parse shapes
        for disallowed_single in [
            "git show --format=%H HEAD:file",
            "git show -s --format=%H HEAD:file",
            "git show --format='%H %P' c",
            "git show --format=%H HEAD",
            "git log -p --format=%H",
            "git log --patch --format=%H",
            "git log --stat --format=%H",
            "git log --name-only --format=%H",
            "git log --name-status --format=%H",
            "git log --numstat --format=%H",
            "git log --raw --format=%H",
            "git log --graph --format=%H",
            "git log --all --format=%H",
            "git log --format=%H HEAD:src/lib.rs",
            "git log --format=%H -- src/lib.rs",
            "git rev-parse 4b825dc642cb6eb9a060e54bf8d69288fbee4904",
            "git rev-parse v1.0",
            "git rev-parse main",
            "git rev-parse HEAD~1",
        ] {
            assert_eq!(
                classify_shell_command(disallowed_single),
                GitCommandKind::NonGit,
                "R2/R3: `{disallowed_single}` must be rejected as NonGit"
            );
        }
        assert_eq!(
            classify_shell_command(&format!("git rev-parse {empty_tree_hex}")),
            GitCommandKind::NonGit
        );

        // R2: Output lines must match expected shape EXACTLY with NO leading whitespace;
        // any non-conforming line makes the WHOLE output contribute nothing.
        let kind_log_h = classify_shell_command("git log -1 --format=%H");
        let kind_log_hp = classify_shell_command("git log -2 --format='%H %P'");
        assert_ne!(kind_log_h, GitCommandKind::NonGit);
        assert_ne!(kind_log_hp, GitCommandKind::NonGit);
        for bad_output in [
            format!("  {sha1}\n"),
            format!("\t{sha1}\n"),
            format!("{sha1}\ndiff --git a/f b/f\n"),
            format!("{sha1}\n{empty_tree_hex} README.md\n"),
            format!("{sha1}\n\n{sha2}\n"),
        ] {
            assert!(
                parse_git_output(&kind_log_h, &bad_output)
                    .commits
                    .is_empty(),
                "R2: non-conforming output `{bad_output:?}` for %H must contribute nothing"
            );
            assert!(
                parse_git_output(&kind_log_hp, &bad_output)
                    .commits
                    .is_empty(),
                "R2: non-conforming output `{bad_output:?}` for %H %P must contribute nothing"
            );
        }

        // R4: `on_default_branch` ONLY for `git rev-parse HEAD` or `git log -1`/`-n 1` with no rev (or HEAD),
        // AND same session + same cwd, AND current branch known from a record in that cwd (no cross-cwd
        // or branch_by_session fallback), AND not detached.
        let mut r4_scanner = GitEvidenceScanner::new();
        r4_scanner.observe_record_context(Some("sess-r4"), false, Some("/repo-main"), Some("main"));
        r4_scanner.observe_tool_call_with_context(
            Some("sym-main"),
            Some("Bash"),
            Some("git symbolic-ref refs/remotes/origin/HEAD"),
            Some("/repo-main"),
        );
        r4_scanner.observe_tool_output_with_cwd(
            Some("sym-main"),
            None,
            Some("sess-r4"),
            false,
            None,
            Some(false),
            Some("/repo-main"),
            "refs/remotes/origin/main\n",
        );

        // (a) `git rev-list --parents feature-x` on main -> default_branch not_observed
        let sha_revlist = "1".repeat(40);
        r4_scanner.observe_tool_call_with_context(
            Some("rl-feat"),
            Some("Bash"),
            Some("git rev-list --parents feature-x"),
            Some("/repo-main"),
        );
        r4_scanner.observe_tool_output_with_cwd(
            Some("rl-feat"),
            None,
            Some("sess-r4"),
            false,
            None,
            Some(false),
            Some("/repo-main"),
            &format!("{sha_revlist} {sha2}\n"),
        );

        // (b) `git cat-file -p <sha>` on main -> default_branch not_observed
        let sha_cat = "2".repeat(40);
        r4_scanner.observe_tool_call_with_context(
            Some("cat-main"),
            Some("Bash"),
            Some(&format!("git cat-file -p {sha_cat}")),
            Some("/repo-main"),
        );
        r4_scanner.observe_tool_output_with_cwd(
            Some("cat-main"),
            None,
            Some("sess-r4"),
            false,
            None,
            Some(false),
            Some("/repo-main"),
            &format!("tree {empty_tree_hex}\nparent {sha2}\n\nmsg\n"),
        );

        // (c) Cross-cwd branch: `/repo-other` has symbolic-ref `origin/main`, but NO branch record in `/repo-other`
        // (must NOT inherit `/repo-main`'s branch via `branch_by_session`)
        let sha_cross_cwd = "3".repeat(40);
        r4_scanner.observe_tool_call_with_context(
            Some("sym-other"),
            Some("Bash"),
            Some("git symbolic-ref refs/remotes/origin/HEAD"),
            Some("/repo-other"),
        );
        r4_scanner.observe_tool_output_with_cwd(
            Some("sym-other"),
            None,
            Some("sess-r4"),
            false,
            None,
            Some(false),
            Some("/repo-other"),
            "refs/remotes/origin/main\n",
        );
        r4_scanner.observe_tool_call_with_context(
            Some("rp-other"),
            Some("Bash"),
            Some("git rev-parse HEAD"),
            Some("/repo-other"),
        );
        r4_scanner.observe_tool_output_with_cwd(
            Some("rp-other"),
            None,
            Some("sess-r4"),
            false,
            None,
            Some(false),
            Some("/repo-other"),
            &format!("{sha_cross_cwd}\n"),
        );

        // (d) Detached HEAD in `/repo-main`: `git checkout --detach` invalidates current branch until re-observed
        let sha_detached = "4".repeat(40);
        r4_scanner.observe_tool_call_with_context(
            Some("co-detach"),
            Some("Bash"),
            Some("git checkout --detach"),
            Some("/repo-main"),
        );
        r4_scanner.observe_tool_output_with_cwd(
            Some("co-detach"),
            None,
            Some("sess-r4"),
            false,
            None,
            Some(false),
            Some("/repo-main"),
            "HEAD is now at 4444444\n",
        );
        r4_scanner.observe_tool_call_with_context(
            Some("rp-detached"),
            Some("Bash"),
            Some("git rev-parse HEAD"),
            Some("/repo-main"),
        );
        r4_scanner.observe_tool_output_with_cwd(
            Some("rp-detached"),
            None,
            Some("sess-r4"),
            false,
            None,
            Some(false),
            Some("/repo-main"),
            &format!("{sha_detached}\n"),
        );

        let (r4_facts, _) = r4_scanner.materialize(
            "src-r4",
            1,
            "claude",
            false,
            Some(b"tool-key"),
            &FactExportConfig::unconfigured(),
            |s| format!("sealed:{s}"),
        );
        assert_eq!(r4_facts.len(), 4);
        for f in &r4_facts {
            assert_eq!(
                f.default_branch_status, "not_observed",
                "R4: commit {} must have default_branch_status = not_observed",
                f.commit_sealed
            );
            assert_eq!(f.on_default_branch, None);
        }

        // R5: Read-function coherence (`resolve_git_evidence_across_sources_with_key_id`)
        // Case 1: Sources disagree on parents -> merge_status = not_observed, reason = conflicting_parents
        let base_fact = GitEvidenceFact {
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
            pseudonym_key_id: Some("kid_current".to_string()),
            commit_hmac: Some("1".repeat(64)),
            parent_hmacs: vec!["2".repeat(64)],
        };
        let mut conflicting_parent_fact = base_fact.clone();
        conflicting_parent_fact.source_key = "src-2:git:commit:c1".to_string();
        conflicting_parent_fact.source_id = "src-2".to_string();
        conflicting_parent_fact.parent_sealed = vec!["commit:p2".to_string()];
        conflicting_parent_fact.parent_hmacs = vec!["3".repeat(64)];
        let resolved_conflict = resolve_git_evidence_across_sources_with_key_id(
            &[base_fact.clone(), conflicting_parent_fact],
            Some("kid_current"),
        );
        assert_eq!(resolved_conflict.len(), 1);
        assert_eq!(resolved_conflict[0].merge_status, MERGE_STATUS_NOT_OBSERVED);
        assert_eq!(resolved_conflict[0].is_merge, None);
        assert_eq!(
            resolved_conflict[0].merge_missing_reason.as_deref(),
            Some(MERGE_REASON_CONFLICTING_PARENTS)
        );
        assert!(resolved_conflict[0].parent_sealed.is_empty());
        assert!(resolved_conflict[0].parent_hmacs.is_empty());

        // Case 2: Row 1 has parents (non_exportable), Row 2 has commit_hmac without parents (exportable)
        // -> Never combine parents from Row 1 with HMACs from Row 2!
        let mut row_parents_unexp = base_fact.clone();
        row_parents_unexp.export_status = EXPORT_STATUS_NON_EXPORTABLE.to_string();
        row_parents_unexp.pseudonym_key_id = None;
        row_parents_unexp.commit_hmac = None;
        row_parents_unexp.parent_hmacs = Vec::new();

        let mut row_no_parents_exp = base_fact.clone();
        row_no_parents_exp.source_key = "src-2:git:commit:c1".to_string();
        row_no_parents_exp.source_id = "src-2".to_string();
        row_no_parents_exp.merge_status = MERGE_STATUS_NOT_OBSERVED.to_string();
        row_no_parents_exp.is_merge = None;
        row_no_parents_exp.merge_missing_reason =
            Some(MERGE_REASON_PARENTS_NOT_RECORDED.to_string());
        row_no_parents_exp.parent_sealed = Vec::new();
        row_no_parents_exp.parent_hmacs = Vec::new();

        let resolved_split = resolve_git_evidence_across_sources_with_key_id(
            &[row_parents_unexp, row_no_parents_exp],
            Some("kid_current"),
        );
        assert_eq!(resolved_split.len(), 1);
        assert_eq!(resolved_split[0].merge_status, MERGE_STATUS_OBSERVED);
        assert_eq!(
            resolved_split[0].export_status,
            EXPORT_STATUS_NON_EXPORTABLE,
            "R5: must never combine observed parents from a non-exportable row with commit_hmac from a parentless row"
        );
        assert_eq!(resolved_split[0].commit_hmac, None);
        assert!(resolved_split[0].parent_hmacs.is_empty());

        // Case 3: Different key IDs -> use only current configured key id; report others as non-exportable
        let mut row_old_key = base_fact.clone();
        row_old_key.commit_sealed = "commit:c2".to_string();
        row_old_key.source_key = "src-1:git:commit:c2".to_string();
        row_old_key.pseudonym_key_id = Some("kid_old".to_string());
        let resolved_keys = resolve_git_evidence_across_sources_with_key_id(
            &[base_fact.clone(), row_old_key],
            Some("kid_current"),
        );
        assert_eq!(resolved_keys.len(), 2);
        let r_cur = resolved_keys
            .iter()
            .find(|r| r.commit_sealed == "commit:c1")
            .unwrap();
        assert_eq!(r_cur.export_status, EXPORT_STATUS_EXPORTABLE);
        assert_eq!(r_cur.pseudonym_key_id.as_deref(), Some("kid_current"));
        let r_old = resolved_keys
            .iter()
            .find(|r| r.commit_sealed == "commit:c2")
            .unwrap();
        assert_eq!(r_old.export_status, EXPORT_STATUS_NON_EXPORTABLE);
        assert_eq!(r_old.pseudonym_key_id, None);
        assert_eq!(r_old.commit_hmac, None);
        assert!(r_old.parent_hmacs.is_empty());

        // R7: Shell detection is an allow-list (unknown or absent -> not a shell)
        assert!(!is_shell_execution_tool_name("CustomUnknownTool"));
        assert!(!is_shell_execution_tool_name(""));
        let mut r7_scanner = GitEvidenceScanner::new();
        r7_scanner.observe_tool_call_with_context(
            Some("r7-none"),
            None,
            Some("git rev-parse HEAD"),
            Some("/repo"),
        );
        r7_scanner.observe_tool_output_with_cwd(
            Some("r7-none"),
            None,
            Some("sess-r7"),
            false,
            None,
            Some(false),
            Some("/repo"),
            &format!("{sha1}\n"),
        );
        let (r7_facts, _) = r7_scanner.materialize(
            "src-r7",
            1,
            "claude",
            false,
            Some(b"tool-key"),
            &FactExportConfig::unconfigured(),
            |s| format!("sealed:{s}"),
        );
        assert!(
            r7_facts.is_empty(),
            "R7: absent tool_name (None) must not be treated as a shell tool"
        );

        // R7: `from_raw_key` rejects keys shorter than 32 bytes
        let short_cfg = FactExportConfig::from_raw_key(b"too-short-key-under-32-bytes", "kid_1");
        assert!(
            !short_cfg.is_ready(),
            "R7: from_raw_key must reject keys shorter than 32 bytes"
        );
        assert_eq!(short_cfg.state(), FactExportKeyState::InvalidKeyMaterial);
    }
}
