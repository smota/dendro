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
const MAX_OBSERVED_COMMITS_PER_SOURCE: usize = 4096;
/// Upper bound on parents per commit (`fact-export-v1` schema `maxItems = 16`).
const MAX_PARENTS_PER_COMMIT: usize = 16;
/// Maximum shell-wrapper unwrapping depth (`bash -lc "..."`, `sh -c "..."`, etc.).
const MAX_SHELL_WRAPPER_DEPTH: usize = 4;

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

    /// Construct a `FactExportKey` from raw test/vector key bytes (allows arbitrary non-empty key
    /// lengths for contract vector verification while zeroizing on drop).
    #[must_use]
    pub fn from_raw_key_bytes(bytes: &[u8]) -> Self {
        Self(bytes.to_vec())
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

    /// Construct a `FactExportConfig` from raw key bytes and `pseudonym_key_id` (used in tests
    /// and contract vector verification).
    #[must_use]
    pub fn from_raw_key(raw_key: &[u8], pseudonym_key_id: impl Into<String>) -> Self {
        if raw_key.is_empty() {
            return Self {
                key: None,
                pseudonym_key_id: None,
                state: FactExportKeyState::InvalidKeyMaterial,
            };
        }
        Self::from_key(FactExportKey::from_raw_key_bytes(raw_key), pseudonym_key_id)
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
    }
}

/// Deterministically derives cross-source `GitEvidenceFact` records across all per-source rows
/// stored in `git_evidence_facts` (for D4b and read queries).
///
/// Cross-source session matching rules (P3):
/// - Exactly one distinct non-empty session across all sources (and no source marked `ambiguous`)
///   -> `session_match_status = "matched"`, `native_session = Some(session)`.
/// - More than one distinct session across sources (or any source marked `ambiguous`)
///   -> `session_match_status = "ambiguous"`, `native_session = None`.
/// - Zero sessions across sources
///   -> `session_match_status = "unmatched"` (or `"unknown"` if all observations are `"unknown"`),
///   `native_session = None`.
/// - `"matched"` requires a non-empty session identity; never picks one when ambiguous.
#[must_use]
pub fn resolve_git_evidence_across_sources(rows: &[GitEvidenceFact]) -> Vec<GitEvidenceFact> {
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

    let mut groups: BTreeMap<String, Vec<&GitEvidenceFact>> = BTreeMap::new();
    for row in sorted {
        groups
            .entry(row.commit_sealed.clone())
            .or_default()
            .push(row);
    }

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

        // Merge status: if any source observed parents for this commit, use those parents.
        let (merge_status, is_merge, merge_missing_reason, parent_sealed, mut parent_hmacs) =
            if let Some(obs_row) = group
                .iter()
                .copied()
                .find(|r| r.merge_status == MERGE_STATUS_OBSERVED)
            {
                (
                    MERGE_STATUS_OBSERVED.to_string(),
                    obs_row.is_merge,
                    None,
                    obs_row.parent_sealed.clone(),
                    obs_row.parent_hmacs.clone(),
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

        // Export status and commit HMACs.
        let (export_status, pseudonym_key_id, commit_hmac) = if let Some(exp_row) = group
            .iter()
            .copied()
            .find(|r| r.export_status == EXPORT_STATUS_EXPORTABLE && r.commit_hmac.is_some())
        {
            if parent_hmacs.is_empty() && !exp_row.parent_hmacs.is_empty() {
                parent_hmacs = exp_row.parent_hmacs.clone();
            }
            (
                EXPORT_STATUS_EXPORTABLE.to_string(),
                exp_row.pseudonym_key_id.clone(),
                exp_row.commit_hmac.clone(),
            )
        } else {
            parent_hmacs.clear();
            (EXPORT_STATUS_NON_EXPORTABLE.to_string(), None, None)
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
// Strict Allow-Listed Git Command Classifier & Output Parser (P1 & P2)
// ---------------------------------------------------------------------------

/// Strictly allow-listed git command shapes recognized by D4c.
/// Any command outside this allow-list is classified as [`GitCommandKind::NonGit`] and
/// contributes nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitCommandKind {
    /// Not an allow-listed git command shape. Contributes nothing.
    NonGit,
    /// `git rev-parse HEAD` or `git rev-parse <ref>` where `<ref>` has no `:`, `^{`, `@{`,
    /// `/`, `\`, or path component.
    RevParseCommit { short: bool },
    /// `git rev-parse --show-object-format`.
    ShowObjectFormat,
    /// `git log` or `git show` with a format that is EXACTLY `%H`.
    LogCommitOnly,
    /// `git log` or `git show` with a format that is EXACTLY `%H %P`, or `git rev-list --parents ...`.
    LogCommitAndParents,
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
    /// `git checkout`, `git switch`) used only to track the current branch in `(session, cwd)`.
    CurrentBranch { checkout_branch: Option<String> },
}

/// One classified git subcommand along with any `cd <dir>` / `git -C <dir>` working directory override.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassifiedGitSubcommand {
    pub kind: GitCommandKind,
    pub cwd_override: Option<String>,
}

/// Classify a shell command into its primary [`GitCommandKind`] (for single-command inspection).
#[must_use]
pub fn classify_shell_command(command: &str) -> GitCommandKind {
    let subcommands = classify_shell_subcommands(command);
    // Prefer commit-bearing or default-branch/object-format subcommands over CurrentBranch/Commit.
    for sub in &subcommands {
        if matches!(
            sub.kind,
            GitCommandKind::LogCommitAndParents
                | GitCommandKind::CatFileCommit { .. }
                | GitCommandKind::LogCommitOnly
                | GitCommandKind::RevParseCommit { .. }
                | GitCommandKind::SymbolicDefaultBranch
                | GitCommandKind::RemoteShowOrigin
                | GitCommandKind::ShowObjectFormat
        ) {
            return sub.kind.clone();
        }
    }
    subcommands
        .into_iter()
        .next()
        .map(|s| s.kind)
        .unwrap_or(GitCommandKind::NonGit)
}

/// Classify a shell command string (unwrapping shell wrappers up to `MAX_SHELL_WRAPPER_DEPTH`)
/// into an ordered list of allow-listed git subcommands.
#[must_use]
pub fn classify_shell_subcommands(command: &str) -> Vec<ClassifiedGitSubcommand> {
    let mut out = Vec::new();
    let mut active_cwd: Option<String> = None;
    collect_shell_subcommands_bounded(command, 0, &mut active_cwd, &mut out);
    out
}

fn collect_shell_subcommands_bounded(
    command: &str,
    depth: usize,
    active_cwd: &mut Option<String>,
    out: &mut Vec<ClassifiedGitSubcommand>,
) {
    if depth > MAX_SHELL_WRAPPER_DEPTH {
        return;
    }
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return;
    }

    for segment in split_shell_pipeline(trimmed) {
        let tokens = tokenize_shell_words(&segment);
        if tokens.is_empty() {
            continue;
        }
        let mut idx = 0;
        while idx < tokens.len() && is_env_assignment(&tokens[idx]) {
            idx += 1;
        }
        while idx < tokens.len()
            && matches!(
                executable_basename(&tokens[idx]),
                "command" | "env" | "sudo" | "nice" | "nohup" | "time" | "stdbuf"
            )
        {
            idx += 1;
            while idx < tokens.len()
                && (tokens[idx].starts_with('-') || is_env_assignment(&tokens[idx]))
            {
                idx += 1;
            }
        }
        if idx >= tokens.len() {
            continue;
        }

        let exe = executable_basename(&tokens[idx]);
        if matches!(exe, "cd" | "pushd") {
            let mut arg_idx = idx + 1;
            while arg_idx < tokens.len() && tokens[arg_idx].starts_with('-') {
                arg_idx += 1;
            }
            if let Some(dir_arg) = tokens
                .get(arg_idx)
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
            {
                *active_cwd = Some(resolve_cwd_path(active_cwd.as_deref(), dir_arg));
            }
            continue;
        }

        if let Some(inner) = extract_shell_wrapper_script(exe, &tokens[idx + 1..]) {
            collect_shell_subcommands_bounded(&inner, depth + 1, active_cwd, out);
            continue;
        }

        if exe == "git" || exe == "git.exe" {
            if let Some(classified) = classify_git_tokens(&tokens[idx + 1..], active_cwd.as_deref())
            {
                out.push(classified);
            }
        }
    }
}

fn extract_shell_wrapper_script(exe: &str, args: &[String]) -> Option<String> {
    if matches!(
        exe,
        "sh" | "bash" | "zsh" | "dash" | "ksh" | "fish" | "sh.exe" | "bash.exe"
    ) {
        let mut i = 0;
        while i < args.len() {
            let a = args[i].as_str();
            if a == "-c"
                || a == "-lc"
                || a == "-cl"
                || (a.starts_with('-')
                    && !a.starts_with("--")
                    && a[1..].bytes().all(|b| b.is_ascii_alphabetic())
                    && a.contains('c'))
            {
                return args.get(i + 1).cloned();
            }
            if a.starts_with('-') {
                i += 1;
                continue;
            }
            break;
        }
        return None;
    }

    if matches!(exe, "pwsh" | "powershell" | "pwsh.exe" | "powershell.exe") {
        let mut i = 0;
        while i < args.len() {
            let lower = args[i].to_ascii_lowercase();
            if matches!(lower.as_str(), "-c" | "-command" | "/c") {
                if i + 1 < args.len() {
                    return Some(args[i + 1..].join(" "));
                }
                return None;
            }
            i += 1;
        }
        return None;
    }

    if matches!(exe, "cmd" | "cmd.exe") {
        for (i, arg) in args.iter().enumerate() {
            if arg.eq_ignore_ascii_case("/c") && i + 1 < args.len() {
                return Some(args[i + 1..].join(" "));
            }
        }
        return None;
    }

    None
}

fn classify_git_tokens(
    args: &[String],
    inherited_cwd: Option<&str>,
) -> Option<ClassifiedGitSubcommand> {
    let mut i = 0;
    let mut cwd_override: Option<String> = inherited_cwd.map(str::to_string);

    while i < args.len() {
        let arg = args[i].as_str();
        if arg == "-C" {
            let dir = args.get(i + 1)?.trim();
            if dir.is_empty() {
                return None;
            }
            cwd_override = Some(resolve_cwd_path(cwd_override.as_deref(), dir));
            i += 2;
            continue;
        }
        if let Some(dir) = arg.strip_prefix("-C") {
            let dir = dir.trim();
            if dir.is_empty() {
                return None;
            }
            cwd_override = Some(resolve_cwd_path(cwd_override.as_deref(), dir));
            i += 1;
            continue;
        }
        if matches!(
            arg,
            "-c" | "--git-dir" | "--work-tree" | "--namespace" | "--super-prefix"
        ) {
            i += 2;
            continue;
        }
        if arg.starts_with("--git-dir=")
            || arg.starts_with("--work-tree=")
            || arg.starts_with("--namespace=")
            || arg.starts_with("--super-prefix=")
            || matches!(
                arg,
                "--no-pager"
                    | "-p"
                    | "--paginate"
                    | "--bare"
                    | "--no-replace-objects"
                    | "--literal-pathspecs"
                    | "--glob-pathspecs"
                    | "--noglob-pathspecs"
                    | "--icase-pathspecs"
                    | "--no-optional-locks"
            )
        {
            i += 1;
            continue;
        }
        if arg.starts_with('-') {
            return None;
        }
        break;
    }

    let subcmd = args.get(i)?.as_str();
    let rest = &args[i + 1..];

    let kind = match subcmd {
        "rev-parse" => classify_rev_parse(rest)?,
        "log" | "show" => classify_log_or_show(rest)?,
        "rev-list" => classify_rev_list(rest)?,
        "cat-file" => classify_cat_file(rest)?,
        "commit" => GitCommandKind::Commit,
        "symbolic-ref" => classify_symbolic_ref(rest)?,
        "remote" => classify_remote(rest)?,
        "branch" => {
            if rest.len() == 1 && rest[0] == "--show-current" {
                GitCommandKind::CurrentBranch {
                    checkout_branch: None,
                }
            } else {
                return None;
            }
        }
        "status" => GitCommandKind::CurrentBranch {
            checkout_branch: None,
        },
        "checkout" | "switch" => GitCommandKind::CurrentBranch {
            checkout_branch: extract_checkout_branch(rest),
        },
        _ => return None,
    };

    Some(ClassifiedGitSubcommand { kind, cwd_override })
}

fn classify_rev_parse(rest: &[String]) -> Option<GitCommandKind> {
    if rest.is_empty() {
        return None;
    }
    // 1. Check `git rev-parse --show-object-format`
    let non_quiet: Vec<&str> = rest
        .iter()
        .map(String::as_str)
        .filter(|&a| a != "-q" && a != "--quiet")
        .collect();
    if non_quiet == ["--show-object-format"] {
        return Some(GitCommandKind::ShowObjectFormat);
    }
    // 2. Check `git rev-parse --abbrev-ref HEAD` (strictly HEAD; never @{u} or other refs)
    if non_quiet == ["--abbrev-ref", "HEAD"] {
        return Some(GitCommandKind::CurrentBranch {
            checkout_branch: None,
        });
    }
    // 3. Commit identity: `git rev-parse HEAD` or `git rev-parse <ref>` where `<ref>` has no
    //    `:`, `^{`, `@{`, or path component.
    let mut short = false;
    let mut positional: Vec<&str> = Vec::new();
    for arg in rest {
        let s = arg.as_str();
        if matches!(s, "-q" | "--quiet" | "--verify" | "--end-of-options") {
            continue;
        }
        if s == "--short" || s.starts_with("--short=") {
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
    let target = positional[0];
    if !is_valid_rev_parse_commit_ref(target) {
        return None;
    }
    Some(GitCommandKind::RevParseCommit { short })
}

fn is_valid_rev_parse_commit_ref(r: &str) -> bool {
    let trimmed = r.trim();
    if trimmed.is_empty() || trimmed.len() > 256 || trimmed.starts_with('-') {
        return false;
    }
    if trimmed.contains(':')
        || trimmed.contains("^{")
        || trimmed.contains("@{")
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.contains("..")
    {
        return false;
    }
    trimmed
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b'~' | b'^'))
}

fn classify_log_or_show(rest: &[String]) -> Option<GitCommandKind> {
    let mut format_spec: Option<String> = None;
    let mut i = 0;
    while i < rest.len() {
        let arg = rest[i].as_str();
        if arg == "--oneline" {
            return None;
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
        i += 1;
    }

    let raw_fmt = format_spec?;
    match normalize_git_format_spec(&raw_fmt) {
        "%H" => Some(GitCommandKind::LogCommitOnly),
        "%H %P" => Some(GitCommandKind::LogCommitAndParents),
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

fn classify_rev_list(rest: &[String]) -> Option<GitCommandKind> {
    let mut has_parents = false;
    let mut i = 0;
    while i < rest.len() {
        let arg = rest[i].as_str();
        if arg == "--parents" {
            has_parents = true;
            i += 1;
            continue;
        }
        if matches!(
            arg,
            "--objects" | "--objects-edge" | "--objects-edge-aggressive" | "--count" | "--header"
        ) {
            return None;
        }
        if let Some(val) = arg
            .strip_prefix("--format=")
            .or_else(|| arg.strip_prefix("--pretty="))
        {
            if normalize_git_format_spec(val) != "%H %P" {
                return None;
            }
            i += 1;
            continue;
        }
        if arg == "--format" || arg == "--pretty" {
            let val = rest.get(i + 1)?;
            if normalize_git_format_spec(val) != "%H %P" {
                return None;
            }
            i += 2;
            continue;
        }
        i += 1;
    }
    has_parents.then_some(GitCommandKind::LogCommitAndParents)
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

fn extract_checkout_branch(rest: &[String]) -> Option<String> {
    let mut i = 0;
    while i < rest.len() {
        let a = rest[i].as_str();
        if matches!(a, "-b" | "-B" | "-c" | "-C") {
            if let Some(b) = rest.get(i + 1) {
                if is_safe_branch_name(b) {
                    return Some(b.clone());
                }
            }
            return None;
        }
        i += 1;
    }
    None
}

fn split_shell_pipeline(cmd: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = cmd.chars().peekable();
    let mut in_single = false;
    let mut in_double = false;

    while let Some(c) = chars.next() {
        if c == '\\' && !in_single {
            cur.push(c);
            if let Some(next_c) = chars.next() {
                cur.push(next_c);
            }
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
        if !in_single && !in_double {
            if c == ';' || c == '\n' {
                if !cur.trim().is_empty() {
                    out.push(std::mem::take(&mut cur));
                } else {
                    cur.clear();
                }
                continue;
            }
            if (c == '&' || c == '|') && chars.peek() == Some(&c) {
                let _ = chars.next();
                if !cur.trim().is_empty() {
                    out.push(std::mem::take(&mut cur));
                } else {
                    cur.clear();
                }
                continue;
            }
            if c == '|' {
                if !cur.trim().is_empty() {
                    out.push(std::mem::take(&mut cur));
                } else {
                    cur.clear();
                }
                continue;
            }
        }
        cur.push(c);
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out
}

fn tokenize_shell_words(segment: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut chars = segment.chars().peekable();
    let mut in_single = false;
    let mut in_double = false;

    while let Some(c) = chars.next() {
        if c == '\\' && !in_single {
            if let Some(next_c) = chars.next() {
                cur.push(next_c);
            }
            continue;
        }
        if c == '\'' && !in_double {
            in_single = !in_single;
            continue;
        }
        if c == '"' && !in_single {
            in_double = !in_double;
            continue;
        }
        if c.is_ascii_whitespace() && !in_single && !in_double {
            if !cur.is_empty() {
                words.push(std::mem::take(&mut cur));
            }
            continue;
        }
        cur.push(c);
    }
    if !cur.is_empty() {
        words.push(cur);
    }
    words
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

/// Returns `true` if `tool_name` can be a shell/command execution tool.
/// Rejects `Agent`, `Task`, `subagent`, `MCP` (`mcp__*`), and file/search/web tools so their
/// outputs are never parsed as git command output even if their input contains a `"command"` field.
#[must_use]
pub fn is_shell_execution_tool_name(tool_name: &str) -> bool {
    let trimmed = tool_name.trim();
    if trimmed.is_empty() {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("mcp__")
        || lower.starts_with("mcp:")
        || lower.starts_with("mcp.")
        || lower.contains("mcp_")
        || matches!(
            lower.as_str(),
            "agent"
                | "task"
                | "subagent"
                | "invoke_subagent"
                | "dispatch_agent"
                | "read"
                | "read_file"
                | "write"
                | "write_file"
                | "edit"
                | "edit_file"
                | "multiedit"
                | "glob"
                | "grep"
                | "ls"
                | "notebookedit"
                | "webfetch"
                | "websearch"
                | "search_web"
                | "todowrite"
                | "todoread"
                | "ask_question"
                | "skill"
                | "use_skill"
        )
    {
        return false;
    }
    true
}

/// One commit observation extracted from an allow-listed git command's output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCommitObservation {
    pub commit_sha: String,
    /// `Some(parents)` when parents were directly observed (`Vec::new()` for a root commit),
    /// or `None` when parents were not recorded or dropped.
    pub parents: Option<Vec<String>>,
    pub merge_missing_reason: Option<&'static str>,
    pub branch: Option<String>,
}

/// Parsed evidence from a single allow-listed git command's output.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ParsedGitOutput {
    pub commits: Vec<ParsedCommitObservation>,
    pub unresolved_abbreviated: u64,
    pub observed_branch: Option<String>,
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
/// Anything outside the strict allow-list (`GitCommandKind::NonGit`) returns an empty [`ParsedGitOutput`].
#[must_use]
pub fn parse_git_output(kind: &GitCommandKind, raw_output: &str) -> ParsedGitOutput {
    let mut out = ParsedGitOutput::default();
    if matches!(kind, GitCommandKind::NonGit) {
        return out;
    }

    let cleaned = strip_output_envelope(raw_output);
    let non_empty_lines: Vec<&str> = cleaned
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();

    match kind {
        GitCommandKind::NonGit => {}
        GitCommandKind::ShowObjectFormat => {
            if non_empty_lines.len() == 1 {
                match non_empty_lines[0] {
                    "sha256" => out.observed_object_format = Some(GitObjectFormat::Sha256),
                    "sha1" => out.observed_object_format = Some(GitObjectFormat::Sha1),
                    _ => {}
                }
            }
        }
        GitCommandKind::SymbolicDefaultBranch => {
            if non_empty_lines.len() == 1 {
                if let Some(def) = extract_default_branch_from_symbolic_ref(non_empty_lines[0]) {
                    out.observed_default_branch = Some(def);
                }
            }
        }
        GitCommandKind::RemoteShowOrigin => {
            for line in &non_empty_lines {
                if let Some(rest) = line.strip_prefix("HEAD branch:") {
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
        GitCommandKind::CurrentBranch { checkout_branch } => {
            if let Some(b) = checkout_branch {
                out.observed_branch = Some(b.clone());
            }
            for line in &non_empty_lines {
                if let Some(rest) = line.strip_prefix("On branch ") {
                    let branch = rest.trim();
                    if is_safe_branch_name(branch) {
                        out.observed_branch = Some(branch.to_string());
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
                } else if non_empty_lines.len() == 1
                    && !line.contains(char::is_whitespace)
                    && *line != "HEAD"
                    && !is_canonical_commit_sha(line)
                    && !is_abbreviated_commit_sha(line)
                    && is_safe_branch_name(line)
                {
                    out.observed_branch = Some((*line).to_string());
                }
            }
        }
        GitCommandKind::Commit => {
            // P1: `git commit` output (an abbreviated SHA -> counted unresolved only).
            for line in &non_empty_lines {
                if let Some((branch_opt, is_abbrev)) = parse_git_commit_bracket_line(line) {
                    if let Some(b) = branch_opt {
                        out.observed_branch = Some(b);
                    }
                    if is_abbrev {
                        out.unresolved_abbreviated += 1;
                    }
                }
            }
        }
        GitCommandKind::RevParseCommit { short } => {
            // P1: `git rev-parse HEAD` or `git rev-parse <ref>` (-> one full SHA line; or
            // abbreviated SHA -> counted unresolved only).
            if non_empty_lines.len() == 1 {
                let token = non_empty_lines[0];
                if !token.contains(char::is_whitespace) {
                    if *short {
                        if is_abbreviated_commit_sha(token) || is_canonical_commit_sha(token) {
                            out.unresolved_abbreviated += 1;
                        }
                    } else if is_canonical_commit_sha(token) {
                        out.commits.push(ParsedCommitObservation {
                            commit_sha: token.to_string(),
                            parents: None,
                            merge_missing_reason: Some(MERGE_REASON_PARENTS_NOT_RECORDED),
                            branch: None,
                        });
                    } else if is_abbreviated_commit_sha(token) {
                        out.unresolved_abbreviated += 1;
                    }
                }
            }
        }
        GitCommandKind::LogCommitOnly => {
            // P1: `git log`/`git show` with format EXACTLY `%H` (-> one SHA per line).
            for line in &non_empty_lines {
                if !line.contains(char::is_whitespace) {
                    if is_canonical_commit_sha(line) {
                        out.commits.push(ParsedCommitObservation {
                            commit_sha: (*line).to_string(),
                            parents: None,
                            merge_missing_reason: Some(MERGE_REASON_PARENTS_NOT_RECORDED),
                            branch: None,
                        });
                    } else if is_abbreviated_commit_sha(line) {
                        out.unresolved_abbreviated += 1;
                    }
                }
            }
        }
        GitCommandKind::LogCommitAndParents => {
            // P1: `git log`/`git show` with format EXACTLY `%H %P`, or `git rev-list --parents`.
            for line in &non_empty_lines {
                let tokens: Vec<&str> = line.split_whitespace().collect();
                if tokens.is_empty() || tokens.len() > 1 + MAX_PARENTS_PER_COMMIT {
                    continue;
                }
                let first = tokens[0];
                if is_abbreviated_commit_sha(first)
                    && tokens[1..]
                        .iter()
                        .all(|t| is_abbreviated_commit_sha(t) || is_canonical_commit_sha(t))
                {
                    out.unresolved_abbreviated += 1;
                    continue;
                }
                if !is_canonical_commit_sha(first) {
                    continue;
                }
                let parent_tokens = &tokens[1..];
                if !parent_tokens
                    .iter()
                    .all(|t| is_canonical_commit_sha(t) || is_abbreviated_commit_sha(t))
                {
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
                        branch: None,
                    });
                } else {
                    // Check P2: parent lengths must equal the commit length; otherwise drop parents.
                    let commit_len = first.len();
                    if parent_tokens.iter().all(|t| t.len() == commit_len) {
                        out.commits.push(ParsedCommitObservation {
                            commit_sha: first.to_string(),
                            parents: Some(parent_tokens.iter().map(|s| (*s).to_string()).collect()),
                            merge_missing_reason: None,
                            branch: None,
                        });
                    } else {
                        out.commits.push(ParsedCommitObservation {
                            commit_sha: first.to_string(),
                            parents: None,
                            merge_missing_reason: Some(MERGE_REASON_MIXED_PARENT_OBJECT_FORMAT),
                            branch: None,
                        });
                    }
                }
            }
        }
        GitCommandKind::CatFileCommit { target_sha } => {
            if let Some(parsed_cat) = parse_cat_file_commit(&non_empty_lines, target_sha) {
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

fn parse_git_commit_bracket_line(line: &str) -> Option<(Option<String>, bool)> {
    let rest = line.strip_prefix('[')?;
    let (bracket_inner, _after) = rest.split_once(']')?;
    let tokens: Vec<&str> = bracket_inner.split_whitespace().collect();
    if tokens.len() < 2 {
        return None;
    }
    let (branch, sha_token) = match tokens.as_slice() {
        [branch, "(root-commit)", sha] => (Some(*branch), *sha),
        ["detached", "HEAD", sha] => (None, *sha),
        [branch, sha] => (Some(*branch), *sha),
        _ => return None,
    };
    if let Some(b) = branch {
        if !is_safe_branch_name(b) {
            return None;
        }
    }
    if is_abbreviated_commit_sha(sha_token) {
        Some((branch.map(str::to_string), true))
    } else if is_canonical_commit_sha(sha_token) {
        Some((branch.map(str::to_string), false))
    } else {
        None
    }
}

fn parse_cat_file_commit(lines: &[&str], target_sha: &str) -> Option<ParsedGitOutput> {
    let first = lines.first()?.trim();
    let tree_sha = first.strip_prefix("tree ")?.trim();
    if !is_canonical_commit_sha(tree_sha) && !is_abbreviated_commit_sha(tree_sha) {
        return None;
    }
    let mut parents = Vec::new();
    let mut saw_abbrev_parent = false;
    let mut unresolved_abbreviated = 0u64;

    for &line in &lines[1..] {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            break;
        }
        if let Some(p_sha) = trimmed.strip_prefix("parent ") {
            let p_tok = p_sha.trim();
            if is_canonical_commit_sha(p_tok) && parents.len() < MAX_PARENTS_PER_COMMIT {
                parents.push(p_tok.to_string());
            } else if is_abbreviated_commit_sha(p_tok) {
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
            branch: None,
        }],
        unresolved_abbreviated,
        ..Default::default()
    })
}

fn extract_default_branch_from_symbolic_ref(line: &str) -> Option<String> {
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
    subcommands: Vec<ClassifiedGitSubcommand>,
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
    /// Most recent working directory observed per session from harness records.
    session_cwd: BTreeMap<String, String>,
    /// Current branch observed per `(session, cwd)`.
    branch_by_session_cwd: BTreeMap<(String, String), String>,
    /// Most recent branch observed on a session record (when record had `gitBranch`).
    branch_by_session: BTreeMap<String, String>,
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
    /// Note: per P1, commit identities and default-branch state are NEVER extracted from
    /// structured metadata fields; only `cwd` and the active branch name (`gitBranch`) are
    /// tracked here so commits observed in `(session, cwd)` can be compared against an
    /// allow-listed default-branch observation in the same `(session, cwd)`.
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
        let norm_cwd = cwd
            .and_then(normalize_cwd)
            .or_else(|| self.session_cwd.get(sess).cloned());
        if let Some(ref c) = cwd.and_then(normalize_cwd) {
            self.session_cwd.insert(sess.to_string(), c.clone());
        }
        if let Some(b) = branch.map(str::trim).filter(|s| is_safe_branch_name(s)) {
            self.branch_by_session
                .insert(sess.to_string(), b.to_string());
            if let Some(c) = norm_cwd {
                self.branch_by_session_cwd
                    .insert((sess.to_string(), c), b.to_string());
            }
        }
    }

    /// Register a tool call by explicit `call_id` in the bounded LRU.
    ///
    /// All tool kinds (`Bash`, `Agent`, `MCP`, `Read`, etc.) are registered for `call_id`
    /// matching, but only shell execution tools have their command classified into allow-listed
    /// git subcommands. When the LRU reaches [`MAX_PENDING_CALLS_PER_SOURCE`], the oldest entry
    /// is evicted.
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
        let is_shell = tool_name.map(is_shell_execution_tool_name).unwrap_or(true);
        let subcommands = if is_shell {
            command.map(classify_shell_subcommands).unwrap_or_default()
        } else {
            Vec::new()
        };
        let pending = PendingToolCall {
            subcommands,
            cwd: cwd.and_then(normalize_cwd),
        };
        self.insert_pending_lru(id.to_string(), pending);
    }

    /// Convenience wrapper for registering a shell tool call without explicit tool name or cwd.
    pub fn observe_tool_call(&mut self, call_id: Option<&str>, command: &str) {
        self.observe_tool_call_with_context(call_id, None, Some(command), None);
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
            let subcommands = classify_shell_subcommands(cmd);
            let call = PendingToolCall {
                subcommands,
                cwd: cwd.and_then(normalize_cwd),
            };
            self.insert_pending_lru(id.to_string(), call.clone());
            call
        } else {
            // Unmatched tool result or evicted from LRU -> ignore!
            return;
        };

        if pending.subcommands.is_empty() {
            return;
        }

        let base_cwd = pending
            .cwd
            .or_else(|| cwd.and_then(normalize_cwd))
            .or_else(|| clean_session.and_then(|s| self.session_cwd.get(s).cloned()));

        for sub in &pending.subcommands {
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

            if let Some(ref b) = parsed.observed_branch {
                if let Some(sess) = clean_session {
                    self.branch_by_session.insert(sess.to_string(), b.clone());
                    if let Some(ref c) = effective_cwd {
                        self.branch_by_session_cwd
                            .insert((sess.to_string(), c.clone()), b.clone());
                    }
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

        // P1: Default branch applies ONLY to commits observed in the SAME session AND the same
        // recorded working directory (when cwd is unknown -> not_observed), using the value known
        // AT the commit's observation time (never applied retroactively).
        let on_default_at_obs = match (clean_session, norm_cwd.as_ref()) {
            (Some(sess), Some(cwd_str)) => {
                let key = (sess.to_string(), cwd_str.clone());
                if let Some(def_branch) = self.default_branch_by_session_cwd.get(&key) {
                    let cur_branch = obs
                        .branch
                        .as_deref()
                        .or_else(|| self.branch_by_session_cwd.get(&key).map(String::as_str))
                        .or_else(|| self.branch_by_session.get(sess).map(String::as_str));
                    cur_branch.map(|b| b == def_branch.as_str())
                } else {
                    None
                }
            }
            _ => None,
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
        let cfg =
            FactExportConfig::from_raw_key(b"fact-export-v1-public-test-key", "kh_customer_v1");
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
}
