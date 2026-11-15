//! Historical git bridge from session records only (`MTN-21 / D4c`), plus
//! `fact-export-v1` HMAC-SHA256 pseudonym derivations and key loading.
//!
//! ## Architectural Invariants
//!
//! 1. **Session records only**: Reads no git repository, spawns no process, and
//!    performs no network lookup. All git evidence is derived conservatively from
//!    content already present in ingested session records (structured harness git
//!    metadata and captured inputs/outputs of git commands the agent ran).
//! 2. **Canonical full SHA only**: Commit identities are accepted only as lowercase
//!    full 40-hex SHA-1 or 64-hex SHA-256 digests (`is_canonical_commit_sha`).
//!    Abbreviated SHAs (e.g., `[main abc1234] msg`) are never resolved or guessed;
//!    they increment `unresolved_abbreviated` in coverage counters and produce no
//!    evidence row.
//! 3. **Parent & default-branch conservatism**:
//!    - Parents are recorded only when stated in full canonical form in local
//!      recorded order (e.g., `git log --format='%H %P'` or `git cat-file -p <sha>`).
//!      Otherwise `merge_status = "not_observed"` with a `merge_missing_reason`,
//!      and `is_merge = None`. When observed, `is_merge = Some(parents.len() > 1)`
//!      (`Some(false)` for root commits with 0 parents and single-parent commits).
//!    - `default_branch_status` is `"observed"` only when a record explicitly
//!      establishes the default branch (e.g., `symbolic-ref refs/remotes/origin/HEAD`
//!      or explicit default-branch metadata compared against the recorded branch).
//!      It is never inferred from branch names such as `"main"` or `"master"`.
//! 4. **Historical `observed_at`**: Uses the record's own timestamp when the commit
//!    was observed; never wall-clock `"now"` or current `HEAD`.
//! 5. **Session matching (`session_match_status`)**:
//!    - `"matched"` (with sealed `native_session` and `session_key`) when observed
//!      in records of exactly one harness-minted session.
//!    - `"ambiguous"` (`native_session = None`, `session_key = None`) when observed
//!      in more than one session (never choosing one).
//!    - `"unmatched"` / `"unknown"` (`native_session = None`, `session_key = None`)
//!      when records carry no harness-minted session.
//! 6. **Privacy & keys**: Never persists a raw commit SHA, branch name, or commit
//!    message. Commit identities are always sealed locally via `seal_commit`
//!    (`HMAC-SHA256` under the store's `tool_key`). Contract `commit_hmac`,
//!    `parent_hmacs`, `session_key`, and `git_evidence_key` are computed only when
//!    `FactExportConfig` key material is configured under D3's exact precedence and
//!    refusal rules (`process env > private data root > XDG consumer.env`, never CWD).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::Sha256;

use crate::link_id::{lp_encode_str, CanonicalHarness};
use crate::privacy::hmac_domain;

type HmacSha256 = Hmac<Sha256>;

// ---------------------------------------------------------------------------
// Hostile-input bounds
// ---------------------------------------------------------------------------

/// Maximum command string bytes scanned per tool call.
pub const MAX_GIT_COMMAND_BYTES: usize = 16 * 1024;

/// Maximum command output bytes scanned per tool result.
pub const MAX_GIT_OUTPUT_BYTES: usize = 64 * 1024;

/// Maximum lines scanned in a single command or output payload.
pub const MAX_GIT_LINES_PER_RECORD: usize = 512;

/// Maximum parent commits retained per commit observation.
pub const MAX_PARENTS_PER_COMMIT: usize = 32;

/// Maximum pending tool calls tracked per source pass.
pub const MAX_PENDING_CALLS_PER_SOURCE: usize = 512;

/// Maximum distinct commits tracked per source pass.
pub const MAX_OBSERVED_COMMITS_PER_SOURCE: usize = 4096;

/// Maximum bytes accepted for an XDG `consumer.env` file (`16 KiB`, matching D3).
pub const MAX_CONSUMER_ENV_BYTES: u64 = 16 * 1024;

// ---------------------------------------------------------------------------
// Status & reason vocabularies
// ---------------------------------------------------------------------------

pub const MERGE_STATUS_OBSERVED: &str = "observed";
pub const MERGE_STATUS_NOT_OBSERVED: &str = "not_observed";
pub const MERGE_STATUS_EXPORT_UNKNOWN: &str = "unknown";

pub const MERGE_REASON_PARENTS_NOT_RECORDED: &str = "parents_not_recorded";
pub const MERGE_REASON_ABBREVIATED_PARENTS: &str = "abbreviated_parents";

pub const DEFAULT_BRANCH_STATUS_OBSERVED: &str = "observed";
pub const DEFAULT_BRANCH_STATUS_NOT_OBSERVED: &str = "not_observed";
pub const DEFAULT_BRANCH_STATUS_EXPORT_UNKNOWN: &str = "unknown";

pub const OBSERVED_AT_STATUS_OBSERVED: &str = "observed";
pub const OBSERVED_AT_STATUS_NOT_OBSERVABLE: &str = "not_observable";

pub const SESSION_MATCH_MATCHED: &str = "matched";
pub const SESSION_MATCH_AMBIGUOUS: &str = "ambiguous";
pub const SESSION_MATCH_UNMATCHED: &str = "unmatched";
pub const SESSION_MATCH_UNKNOWN: &str = "unknown";

pub const EVIDENCE_SOURCE_LOCAL_GIT_HISTORY: &str = "local_git_history";

pub const EXPORT_STATUS_EXPORTABLE: &str = "exportable";
pub const EXPORT_STATUS_NON_EXPORTABLE: &str = "non_exportable";

// ---------------------------------------------------------------------------
// Environment variable names (Fact-Export Key Configuration, D3 rules)
// ---------------------------------------------------------------------------

pub const ENV_FACT_EXPORT_KEY_HEX: &str = "DENDRO_FACT_EXPORT_KEY_HEX";
pub const ENV_ALT_FACT_EXPORT_KEY_HEX: &str = "AGENT_OTEL_FACT_EXPORT_KEY_HEX";
pub const ENV_FACT_EXPORT_KEY_FILE: &str = "DENDRO_FACT_EXPORT_KEY_FILE";
pub const ENV_ALT_FACT_EXPORT_KEY_FILE: &str = "AGENT_OTEL_FACT_EXPORT_KEY_FILE";
pub const ENV_FACT_EXPORT_KEY_ID: &str = "DENDRO_FACT_EXPORT_KEY_ID";
pub const ENV_ALT_FACT_EXPORT_KEY_ID: &str = "AGENT_OTEL_FACT_EXPORT_KEY_ID";

/// Default file name under `<private_data_root>` for the fact-export key.
pub const DEFAULT_FACT_EXPORT_KEY_FILENAME: &str = "fact_export.key";
/// Optional companion file name under `<private_data_root>` for `pseudonym_key_id`.
pub const DEFAULT_FACT_EXPORT_KEY_ID_FILENAME: &str = "fact_export.key_id";

const FORBIDDEN_ORG_KEY_VARS: &[&str] = &[
    "DENDRO_ORG_KEY",
    "DENDRO_ORG_KEY_HEX",
    "DENDRO_ORG_KEY_FILE",
    "DENDRO_K_ORG",
    "DENDRO_K_ORG_HEX",
    "DENDRO_K_ORG_FILE",
    "DENDRO_LINK_K_ORG",
    "DENDRO_LINK_K_ORG_HEX",
    "DENDRO_LINK_K_ORG_FILE",
    "DENDRO_FACT_EXPORT_K_ORG",
    "DENDRO_FACT_EXPORT_K_ORG_HEX",
    "DENDRO_FACT_EXPORT_K_ORG_FILE",
    "AGENT_OTEL_ORG_KEY",
    "AGENT_OTEL_ORG_KEY_HEX",
    "AGENT_OTEL_ORG_KEY_FILE",
    "AGENT_OTEL_K_ORG",
    "AGENT_OTEL_K_ORG_HEX",
    "AGENT_OTEL_K_ORG_FILE",
    "AGENT_OTEL_LINK_K_ORG",
    "AGENT_OTEL_LINK_K_ORG_HEX",
    "AGENT_OTEL_LINK_K_ORG_FILE",
    "AGENT_OTEL_FACT_EXPORT_K_ORG",
    "AGENT_OTEL_FACT_EXPORT_K_ORG_HEX",
    "AGENT_OTEL_FACT_EXPORT_K_ORG_FILE",
    "K_ORG",
    "K_ORG_HEX",
    "K_ORG_FILE",
];

// ---------------------------------------------------------------------------
// Zeroize-on-drop FactExportKey & FactExportConfig
// ---------------------------------------------------------------------------

/// Customer-held key material for `fact-export-v1` HMAC-SHA256 pseudonyms.
///
/// Zeroized on drop via `write_volatile` + compiler fence, never formatted in
/// `Debug`, `Display`, or error messages, and never persisted to DuckDB.
#[derive(Clone, PartialEq, Eq)]
pub struct FactExportKey(Vec<u8>);

impl FactExportKey {
    /// Construct from raw key bytes. Rejects empty or all-zero slices.
    pub fn from_raw_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.is_empty() || bytes.iter().all(|&b| b == 0) {
            return None;
        }
        Some(Self(bytes.to_vec()))
    }

    /// Construct from a 64-character hex string (32 bytes). Rejects non-hex,
    /// wrong length, or all-zero key material.
    pub fn from_hex(hex_str: &str) -> Option<Self> {
        let trimmed = hex_str.trim();
        if trimmed.len() != 64 {
            return None;
        }
        let mut bytes = [0u8; 32];
        hex::decode_to_slice(trimmed, &mut bytes).ok()?;
        let out = Self::from_raw_bytes(&bytes);
        bytes.fill(0);
        std::hint::black_box(&mut bytes);
        out
    }

    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl Drop for FactExportKey {
    fn drop(&mut self) {
        self.0.fill(0);
        std::hint::black_box(&mut self.0);
    }
}

impl fmt::Debug for FactExportKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("FactExportKey([REDACTED])")
    }
}

fn zeroize_string(s: String) {
    let mut bytes = s.into_bytes();
    bytes.fill(0);
    std::hint::black_box(&mut bytes);
}

/// Validates `pseudonym_key_id` against `fact-export-v1` pattern `^[a-z0-9][a-z0-9._-]{1,63}$`.
pub fn is_valid_pseudonym_key_id(s: &str) -> bool {
    let bytes = s.as_bytes();
    if !(2..=64).contains(&bytes.len()) {
        return false;
    }
    if !matches!(bytes[0], b'a'..=b'z' | b'0'..=b'9') {
        return false;
    }
    bytes[1..]
        .iter()
        .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'-'))
}

/// Deterministic fallback `pseudonym_key_id` (`kh_<16-hex>`) when key material is
/// loaded from a single-file key without an explicit key-id override.
fn default_pseudonym_key_id_for_key(key: &FactExportKey) -> String {
    let mut mac = HmacSha256::new_from_slice(key.as_bytes()).expect("HMAC accepts any key size");
    mac.update(&lp_encode_str("fact-export.v1/key_id"));
    let digest = hex::encode(mac.finalize().into_bytes());
    format!("kh_{}", &digest[..16])
}

/// Reason a `FactExportConfig` load was refused or left unconfigured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FactExportKeyState {
    /// Key material and `pseudonym_key_id` are valid and ready to derive HMACs.
    Ready,
    /// No fact-export key material was configured.
    Unconfigured,
    /// Key material was malformed, conflicted, or failed private-file checks.
    InvalidKeyMaterial,
    /// `pseudonym_key_id` was malformed or conflicted across env variables.
    InvalidKeyId,
    /// Forbidden `K_org` environment variable was present on the workstation.
    ForbiddenOrgKeyPresent,
}

/// Workstation configuration for `fact-export-v1` pseudonym derivations (`D4c` / `D4b`).
///
/// Loaded under D3's exact precedence and security rules:
/// 1. Process environment (`DENDRO_FACT_EXPORT_KEY_HEX` / `AGENT_OTEL_FACT_EXPORT_KEY_HEX`,
///    or process-env-only `DENDRO_FACT_EXPORT_KEY_FILE` / `AGENT_OTEL_FACT_EXPORT_KEY_FILE`).
/// 2. Verified private data root (`<private_data_root>/fact_export.key`).
/// 3. Verified non-symlink regular file `$XDG_CONFIG_HOME/observability/consumer.env`
///    (or `$HOME/.config/observability/consumer.env`). Never reads `./.env` from CWD.
#[derive(Debug, Clone, PartialEq, Eq)]
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

impl FactExportConfig {
    /// Unconfigured state: git evidence rows are stored with sealed local identity
    /// and `export_status = "non_exportable"`.
    pub fn unconfigured() -> Self {
        Self {
            key: None,
            pseudonym_key_id: None,
            state: FactExportKeyState::Unconfigured,
        }
    }

    /// Construct from a 64-hex key string and an optional `pseudonym_key_id`.
    /// If `pseudonym_key_id` is `Some`, it must match `^[a-z0-9][a-z0-9._-]{1,63}$`.
    pub fn from_key_hex(key_hex: &str, pseudonym_key_id: Option<&str>) -> Self {
        let Some(key) = FactExportKey::from_hex(key_hex) else {
            return Self {
                key: None,
                pseudonym_key_id: None,
                state: FactExportKeyState::InvalidKeyMaterial,
            };
        };
        let resolved_id = match pseudonym_key_id {
            Some(id) => {
                let trimmed = id.trim();
                if !is_valid_pseudonym_key_id(trimmed) {
                    return Self {
                        key: None,
                        pseudonym_key_id: None,
                        state: FactExportKeyState::InvalidKeyId,
                    };
                }
                trimmed.to_string()
            }
            None => default_pseudonym_key_id_for_key(&key),
        };
        Self {
            key: Some(key),
            pseudonym_key_id: Some(resolved_id),
            state: FactExportKeyState::Ready,
        }
    }

    /// Construct from raw key bytes and an explicit `pseudonym_key_id` (used by
    /// `fact-export-v1` contract test vectors such as `kh_customer_v1`).
    pub fn from_raw_key(key_bytes: &[u8], pseudonym_key_id: &str) -> Self {
        let Some(key) = FactExportKey::from_raw_bytes(key_bytes) else {
            return Self {
                key: None,
                pseudonym_key_id: None,
                state: FactExportKeyState::InvalidKeyMaterial,
            };
        };
        let trimmed = pseudonym_key_id.trim();
        if !is_valid_pseudonym_key_id(trimmed) {
            return Self {
                key: None,
                pseudonym_key_id: None,
                state: FactExportKeyState::InvalidKeyId,
            };
        }
        Self {
            key: Some(key),
            pseudonym_key_id: Some(trimmed.to_string()),
            state: FactExportKeyState::Ready,
        }
    }

    /// Load configuration from process environment, optional `<private_data_root>`,
    /// and XDG `consumer.env` (never CWD).
    pub fn load(private_data_root: Option<&Path>) -> Self {
        let xdg_env = resolve_xdg_consumer_env_path(|k| std::env::var(k).ok());
        Self::load_with(
            |k| std::env::var(k).ok(),
            private_data_root,
            xdg_env.as_deref(),
        )
    }

    /// Testable loader accepting an environment lookup closure, optional
    /// `private_data_root`, and optional XDG `consumer.env` path.
    pub fn load_with<F>(
        env_lookup: F,
        private_data_root: Option<&Path>,
        xdg_consumer_env: Option<&Path>,
    ) -> Self
    where
        F: Fn(&str) -> Option<String>,
    {
        for &forbidden in FORBIDDEN_ORG_KEY_VARS {
            if let Some(val) = env_lookup(forbidden) {
                let non_empty = !val.trim().is_empty();
                zeroize_string(val);
                if non_empty {
                    return Self {
                        key: None,
                        pseudonym_key_id: None,
                        state: FactExportKeyState::ForbiddenOrgKeyPresent,
                    };
                }
            }
        }

        let consumer_outcome = xdg_consumer_env.map(parse_consumer_env_file);
        if let Some(ConsumerEnvOutcome::Invalid) = consumer_outcome {
            return Self {
                key: None,
                pseudonym_key_id: None,
                state: FactExportKeyState::InvalidKeyMaterial,
            };
        }
        let consumer_map = match consumer_outcome {
            Some(ConsumerEnvOutcome::Loaded(map)) => map,
            _ => BTreeMap::new(),
        };

        for &forbidden in FORBIDDEN_ORG_KEY_VARS {
            if consumer_map
                .get(forbidden)
                .is_some_and(|v| !v.trim().is_empty())
            {
                zeroize_consumer_map(consumer_map);
                return Self {
                    key: None,
                    pseudonym_key_id: None,
                    state: FactExportKeyState::ForbiddenOrgKeyPresent,
                };
            }
        }

        // `*_KEY_FILE` is process-env ONLY; reject if present in `consumer.env`.
        for &file_var in &[ENV_FACT_EXPORT_KEY_FILE, ENV_ALT_FACT_EXPORT_KEY_FILE] {
            if consumer_map.contains_key(file_var) {
                zeroize_consumer_map(consumer_map);
                return Self {
                    key: None,
                    pseudonym_key_id: None,
                    state: FactExportKeyState::InvalidKeyMaterial,
                };
            }
        }

        // Resolve key material: process env > private_data_root/fact_export.key > consumer.env.
        let key_resolution = resolve_fact_export_key(&env_lookup, private_data_root, &consumer_map);
        let key_id_resolution =
            resolve_fact_export_key_id(&env_lookup, private_data_root, &consumer_map);
        zeroize_consumer_map(consumer_map);

        let key = match key_resolution {
            KeyResolution::Valid(k) => k,
            KeyResolution::Invalid => {
                return Self {
                    key: None,
                    pseudonym_key_id: None,
                    state: FactExportKeyState::InvalidKeyMaterial,
                };
            }
            KeyResolution::Missing => {
                return match key_id_resolution {
                    KeyIdResolution::Invalid => Self {
                        key: None,
                        pseudonym_key_id: None,
                        state: FactExportKeyState::InvalidKeyId,
                    },
                    _ => Self::unconfigured(),
                };
            }
        };

        let pseudonym_key_id = match key_id_resolution {
            KeyIdResolution::Valid(id) => id,
            KeyIdResolution::Missing => default_pseudonym_key_id_for_key(&key),
            KeyIdResolution::Invalid => {
                return Self {
                    key: None,
                    pseudonym_key_id: None,
                    state: FactExportKeyState::InvalidKeyId,
                };
            }
        };

        Self {
            key: Some(key),
            pseudonym_key_id: Some(pseudonym_key_id),
            state: FactExportKeyState::Ready,
        }
    }

    pub fn is_ready(&self) -> bool {
        self.state == FactExportKeyState::Ready
            && self.key.is_some()
            && self.pseudonym_key_id.is_some()
    }

    pub fn state(&self) -> FactExportKeyState {
        self.state
    }

    pub fn pseudonym_key_id(&self) -> Option<&str> {
        self.pseudonym_key_id.as_deref()
    }

    pub fn key(&self) -> Option<&FactExportKey> {
        self.key.as_ref()
    }

    /// Derive a domain-separated `fact-export-v1` pseudonym if configured.
    pub fn derive_pseudonym(&self, domain: &str, parts: &[&str]) -> Option<String> {
        let key = self.key.as_ref()?;
        Some(derive_fact_export_pseudonym(key, domain, parts))
    }

    /// Derive `source_id` (`fact-export.v1/source`), preserving an already-derived
    /// 64-character lowercase hex `source_id`.
    pub fn derive_source_id(&self, raw_source_id: &str) -> Option<String> {
        let key = self.key.as_ref()?;
        Some(derive_export_source_id(key, raw_source_id))
    }

    /// Derive `session_key` (`fact-export.v1/session`).
    pub fn derive_session_key(
        &self,
        raw_source_id: &str,
        harness: &str,
        raw_session_id: &str,
    ) -> Option<String> {
        let key = self.key.as_ref()?;
        let canonical = CanonicalHarness::from_alias(harness)?;
        let trimmed_session = raw_session_id.trim();
        if trimmed_session.is_empty() {
            return None;
        }
        let export_source = derive_export_source_id(key, raw_source_id);
        Some(derive_export_session_key(
            key,
            &export_source,
            canonical.as_str(),
            trimmed_session,
        ))
    }

    /// Derive `commit_hmac` (`fact-export.v1/commit`) for a canonical full SHA.
    pub fn derive_commit_hmac(&self, canonical_commit_sha: &str) -> Option<String> {
        let key = self.key.as_ref()?;
        derive_export_commit_hmac(key, canonical_commit_sha)
    }

    /// Derive `git_evidence_key` (`fact-export.v1/git_evidence`).
    pub fn derive_git_evidence_key(
        &self,
        raw_source_id: &str,
        commit_hmac: &str,
        observed_at_us: Option<i64>,
        session_key: Option<&str>,
        session_match_status: &str,
    ) -> Option<String> {
        let key = self.key.as_ref()?;
        let export_source = derive_export_source_id(key, raw_source_id);
        Some(derive_export_git_evidence_key(
            key,
            &export_source,
            commit_hmac,
            observed_at_us,
            session_key,
            session_match_status,
        ))
    }
}

enum KeyResolution {
    Valid(FactExportKey),
    Missing,
    Invalid,
}

enum KeyIdResolution {
    Valid(String),
    Missing,
    Invalid,
}

enum ConsumerEnvOutcome {
    Loaded(BTreeMap<String, String>),
    Missing,
    Invalid,
}

fn zeroize_consumer_map(map: BTreeMap<String, String>) {
    for (k, v) in map {
        zeroize_string(k);
        zeroize_string(v);
    }
}

fn resolve_xdg_consumer_env_path<F>(env_lookup: F) -> Option<PathBuf>
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(xdg) = env_lookup("XDG_CONFIG_HOME")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        let p = PathBuf::from(xdg);
        if p.is_absolute() {
            return Some(p.join("observability").join("consumer.env"));
        }
        return None;
    }
    if let Some(home) = env_lookup("HOME")
        .or_else(|| env_lookup("USERPROFILE"))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        let p = PathBuf::from(home);
        if p.is_absolute() {
            return Some(p.join(".config").join("observability").join("consumer.env"));
        }
    }
    None
}

fn parse_consumer_env_file(path: &Path) -> ConsumerEnvOutcome {
    if !path.is_absolute() {
        return ConsumerEnvOutcome::Invalid;
    }
    let sym_meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return ConsumerEnvOutcome::Missing,
        Err(_) => return ConsumerEnvOutcome::Invalid,
    };
    if sym_meta.file_type().is_symlink() || !sym_meta.file_type().is_file() {
        return ConsumerEnvOutcome::Invalid;
    }
    if sym_meta.len() > MAX_CONSUMER_ENV_BYTES {
        return ConsumerEnvOutcome::Invalid;
    }
    let Ok(content) = std::fs::read_to_string(path) else {
        return ConsumerEnvOutcome::Invalid;
    };
    let mut map = BTreeMap::new();
    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").map(str::trim).unwrap_or(line);
        if let Some((k, v)) = line.split_once('=') {
            let key = k.trim();
            let mut val = v.trim();
            if (val.starts_with('"') && val.ends_with('"') && val.len() >= 2)
                || (val.starts_with('\'') && val.ends_with('\'') && val.len() >= 2)
            {
                val = &val[1..val.len() - 1];
            }
            if !key.is_empty() {
                map.insert(key.to_string(), val.to_string());
            }
        }
    }
    zeroize_string(content);
    ConsumerEnvOutcome::Loaded(map)
}

fn open_verified_key_file_handle(path: &Path) -> Option<std::fs::File> {
    if !path.is_absolute() {
        return None;
    }
    #[cfg(windows)]
    {
        let parent = path.parent().filter(|p| !p.as_os_str().is_empty())?;
        let file_name = path.file_name()?;
        let child = crate::platform_v1::parse_child_os(
            file_name,
            crate::platform_v1::NativePlatform::Windows,
        )
        .ok()?;
        if let Ok(root) = crate::platform_v1::open_existing_private_root(parent) {
            let verified = crate::platform_v1::open_existing_private_file(&root, &child).ok()?;
            if verified.metadata().size > MAX_CONSUMER_ENV_BYTES {
                return None;
            }
            let file = verified.into_file();
            drop(root);
            return Some(file);
        }
        let no_follow = crate::platform_v1::reopen_metadata(path).ok()?;
        if no_follow.reparse
            || no_follow.kind != crate::platform_v1::ObjectKind::RegularFile
            || no_follow.size > MAX_CONSUMER_ENV_BYTES
            || !crate::platform_v1::private_regular_links(no_follow.kind, no_follow.links)
                .is_allow()
        {
            return None;
        }
        let file = std::fs::File::open(path).ok()?;
        let held = crate::platform_v1::metadata_from_handle(&file).ok()?;
        if held.reparse
            || held.kind != crate::platform_v1::ObjectKind::RegularFile
            || held.size > MAX_CONSUMER_ENV_BYTES
            || !crate::platform_v1::private_regular_links(held.kind, held.links).is_allow()
            || held.identity != no_follow.identity
        {
            return None;
        }
        Some(file)
    }
    #[cfg(not(windows))]
    {
        let sym = std::fs::symlink_metadata(path).ok()?;
        if !sym.file_type().is_file() || sym.len() > MAX_CONSUMER_ENV_BYTES {
            return None;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if sym.permissions().mode() & 0o077 != 0 {
                return None;
            }
        }
        let file = std::fs::File::open(path).ok()?;
        let held = file.metadata().ok()?;
        if !held.is_file() || held.len() > MAX_CONSUMER_ENV_BYTES {
            return None;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            if held.permissions().mode() & 0o077 != 0
                || held.nlink() != 1
                || sym.dev() != held.dev()
                || sym.ino() != held.ino()
            {
                return None;
            }
        }
        Some(file)
    }
}

fn open_private_root_child_file(root_path: &Path, child_name: &str) -> Option<std::fs::File> {
    #[cfg(any(windows, target_os = "linux"))]
    {
        let root = crate::platform_v1::open_existing_private_root(root_path).ok()?;
        let name = crate::platform_v1::parse_child_name(
            child_name,
            crate::platform_v1::current_native_platform(),
        )
        .ok()?;
        let verified = crate::platform_v1::open_existing_private_file(&root, &name).ok()?;
        if verified.metadata().size > MAX_CONSUMER_ENV_BYTES {
            return None;
        }
        let file = verified.into_file();
        drop(root);
        Some(file)
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        open_verified_key_file_handle(&root_path.join(child_name))
    }
}

fn read_and_parse_fact_export_key_handle(file: std::fs::File) -> KeyResolution {
    use std::io::Read;
    let mut raw = String::new();
    if file
        .take(MAX_CONSUMER_ENV_BYTES + 1)
        .read_to_string(&mut raw)
        .is_err()
        || raw.len() as u64 > MAX_CONSUMER_ENV_BYTES
    {
        zeroize_string(raw);
        return KeyResolution::Invalid;
    }
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let trimmed = trimmed
            .strip_prefix("export ")
            .map(str::trim)
            .unwrap_or(trimmed);
        if let Some((k, _)) = trimmed.split_once('=') {
            if FORBIDDEN_ORG_KEY_VARS.contains(&k.trim()) {
                zeroize_string(raw);
                return KeyResolution::Invalid;
            }
        }
    }
    let res = match FactExportKey::from_hex(&raw) {
        Some(k) => KeyResolution::Valid(k),
        None => KeyResolution::Invalid,
    };
    zeroize_string(raw);
    res
}

fn read_key_from_private_file(path: &Path) -> KeyResolution {
    let Some(file) = open_verified_key_file_handle(path) else {
        return KeyResolution::Invalid;
    };
    read_and_parse_fact_export_key_handle(file)
}

fn resolve_fact_export_key<F>(
    env_lookup: &F,
    private_data_root: Option<&Path>,
    consumer_env: &BTreeMap<String, String>,
) -> KeyResolution
where
    F: Fn(&str) -> Option<String>,
{
    let primary_hex = env_lookup(ENV_FACT_EXPORT_KEY_HEX);
    let alt_hex = env_lookup(ENV_ALT_FACT_EXPORT_KEY_HEX);
    let primary_file = env_lookup(ENV_FACT_EXPORT_KEY_FILE);
    let alt_file = env_lookup(ENV_ALT_FACT_EXPORT_KEY_FILE);

    let has_env_hex = primary_hex.is_some() || alt_hex.is_some();
    let has_env_file = primary_file.is_some() || alt_file.is_some();

    if has_env_hex && has_env_file {
        if let Some(s) = primary_hex {
            zeroize_string(s);
        }
        if let Some(s) = alt_hex {
            zeroize_string(s);
        }
        if let Some(s) = primary_file {
            zeroize_string(s);
        }
        if let Some(s) = alt_file {
            zeroize_string(s);
        }
        return KeyResolution::Invalid;
    }

    if let (Some(p), Some(a)) = (&primary_hex, &alt_hex) {
        if !p.trim().eq_ignore_ascii_case(a.trim()) {
            if let Some(s) = primary_hex {
                zeroize_string(s);
            }
            if let Some(s) = alt_hex {
                zeroize_string(s);
            }
            return KeyResolution::Invalid;
        }
    }

    if let Some(hex_val) = primary_hex.or(alt_hex) {
        let res = match FactExportKey::from_hex(&hex_val) {
            Some(k) => KeyResolution::Valid(k),
            None => KeyResolution::Invalid,
        };
        zeroize_string(hex_val);
        return res;
    }

    if let (Some(p), Some(a)) = (&primary_file, &alt_file) {
        if p.trim() != a.trim() {
            if let Some(s) = primary_file {
                zeroize_string(s);
            }
            if let Some(s) = alt_file {
                zeroize_string(s);
            }
            return KeyResolution::Invalid;
        }
    }

    if let Some(file_path_str) = primary_file.or(alt_file) {
        let trimmed = file_path_str.trim();
        if trimmed.is_empty() {
            zeroize_string(file_path_str);
            return KeyResolution::Invalid;
        }
        let res = read_key_from_private_file(Path::new(trimmed));
        zeroize_string(file_path_str);
        return res;
    }

    if let Some(root) = private_data_root {
        let candidate = root.join(DEFAULT_FACT_EXPORT_KEY_FILENAME);
        if std::fs::symlink_metadata(&candidate).is_ok() {
            let Some(file) = open_private_root_child_file(root, DEFAULT_FACT_EXPORT_KEY_FILENAME)
            else {
                return KeyResolution::Invalid;
            };
            return read_and_parse_fact_export_key_handle(file);
        }
    }

    let c_primary_hex = consumer_env.get(ENV_FACT_EXPORT_KEY_HEX);
    let c_alt_hex = consumer_env.get(ENV_ALT_FACT_EXPORT_KEY_HEX);
    if let (Some(p), Some(a)) = (c_primary_hex, c_alt_hex) {
        if !p.trim().eq_ignore_ascii_case(a.trim()) {
            return KeyResolution::Invalid;
        }
    }
    if let Some(hex_val) = c_primary_hex.or(c_alt_hex) {
        return match FactExportKey::from_hex(hex_val) {
            Some(k) => KeyResolution::Valid(k),
            None => KeyResolution::Invalid,
        };
    }

    KeyResolution::Missing
}

fn resolve_fact_export_key_id<F>(
    env_lookup: &F,
    private_data_root: Option<&Path>,
    consumer_env: &BTreeMap<String, String>,
) -> KeyIdResolution
where
    F: Fn(&str) -> Option<String>,
{
    let primary = env_lookup(ENV_FACT_EXPORT_KEY_ID);
    let alt = env_lookup(ENV_ALT_FACT_EXPORT_KEY_ID);
    if let (Some(p), Some(a)) = (&primary, &alt) {
        if p.trim() != a.trim() {
            return KeyIdResolution::Invalid;
        }
    }
    if let Some(val) = primary.or(alt) {
        let trimmed = val.trim();
        if !is_valid_pseudonym_key_id(trimmed) {
            return KeyIdResolution::Invalid;
        }
        return KeyIdResolution::Valid(trimmed.to_string());
    }

    if let Some(root) = private_data_root {
        let candidate = root.join(DEFAULT_FACT_EXPORT_KEY_ID_FILENAME);
        if std::fs::symlink_metadata(&candidate).is_ok() {
            let Some(file) =
                open_private_root_child_file(root, DEFAULT_FACT_EXPORT_KEY_ID_FILENAME)
            else {
                return KeyIdResolution::Invalid;
            };
            use std::io::Read;
            let mut raw = String::new();
            if file.take(1025).read_to_string(&mut raw).is_err() || raw.len() > 1024 {
                return KeyIdResolution::Invalid;
            }
            let trimmed = raw.trim();
            if !is_valid_pseudonym_key_id(trimmed) {
                return KeyIdResolution::Invalid;
            }
            return KeyIdResolution::Valid(trimmed.to_string());
        }
    }

    let c_primary = consumer_env.get(ENV_FACT_EXPORT_KEY_ID);
    let c_alt = consumer_env.get(ENV_ALT_FACT_EXPORT_KEY_ID);
    if let (Some(p), Some(a)) = (c_primary, c_alt) {
        if p.trim() != a.trim() {
            return KeyIdResolution::Invalid;
        }
    }
    if let Some(val) = c_primary.or(c_alt) {
        let trimmed = val.trim();
        if !is_valid_pseudonym_key_id(trimmed) {
            return KeyIdResolution::Invalid;
        }
        return KeyIdResolution::Valid(trimmed.to_string());
    }

    KeyIdResolution::Missing
}

// ---------------------------------------------------------------------------
// Normative `fact-export-v1` HMAC-SHA256 pseudonym derivations
// ---------------------------------------------------------------------------

/// Normative `fact-export-v1` pseudonym derivation:
/// `hex(HMAC_SHA256(key, LP("fact-export.v1/" + domain, *parts)))`.
pub fn derive_fact_export_pseudonym(key: &FactExportKey, domain: &str, parts: &[&str]) -> String {
    let mut mac = HmacSha256::new_from_slice(key.as_bytes()).expect("HMAC accepts any key size");
    let full_domain = format!("fact-export.v1/{domain}");
    mac.update(&lp_encode_str(&full_domain));
    for part in parts {
        mac.update(&lp_encode_str(part));
    }
    hex::encode(mac.finalize().into_bytes())
}

fn is_64_lowercase_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Derive `source_id` under `fact-export.v1/source` (or preserve an existing 64-hex digest).
pub fn derive_export_source_id(key: &FactExportKey, raw_source_id: &str) -> String {
    if is_64_lowercase_hex(raw_source_id) {
        raw_source_id.to_string()
    } else {
        derive_fact_export_pseudonym(key, "source", &[raw_source_id])
    }
}

/// Derive `session_key` under `fact-export.v1/session`.
pub fn derive_export_session_key(
    key: &FactExportKey,
    export_source_id: &str,
    canonical_harness: &str,
    raw_session_id: &str,
) -> String {
    derive_fact_export_pseudonym(
        key,
        "session",
        &[export_source_id, canonical_harness, raw_session_id],
    )
}

/// Derive `commit_hmac` (or an element of `parent_hmacs`) under `fact-export.v1/commit`.
/// Returns `None` if `canonical_commit_sha` is not a lowercase 40-hex or 64-hex SHA.
pub fn derive_export_commit_hmac(
    key: &FactExportKey,
    canonical_commit_sha: &str,
) -> Option<String> {
    if !is_canonical_commit_sha(canonical_commit_sha) {
        return None;
    }
    Some(derive_fact_export_pseudonym(
        key,
        "commit",
        &[canonical_commit_sha],
    ))
}

/// Derive `git_evidence_key` under `fact-export.v1/git_evidence`.
pub fn derive_export_git_evidence_key(
    key: &FactExportKey,
    export_source_id: &str,
    commit_hmac: &str,
    observed_at_us: Option<i64>,
    session_key: Option<&str>,
    session_match_status: &str,
) -> String {
    let obs_str = observed_at_us.map(|v| v.to_string()).unwrap_or_default();
    let sess_str = session_key.unwrap_or("");
    derive_fact_export_pseudonym(
        key,
        "git_evidence",
        &[
            export_source_id,
            commit_hmac,
            &obs_str,
            sess_str,
            session_match_status,
        ],
    )
}

/// Derive `branch_name_hmac` under `fact-export.v1/branch`.
pub fn derive_export_branch_hmac(key: &FactExportKey, branch_name: &str) -> String {
    derive_fact_export_pseudonym(key, "branch", &[branch_name])
}

/// Derive `repo_remote_hmac` under `fact-export.v1/repo_remote`.
pub fn derive_export_repo_remote_hmac(key: &FactExportKey, sanitized_url: &str) -> String {
    derive_fact_export_pseudonym(key, "repo_remote", &[sanitized_url])
}

/// Seal a canonical commit SHA locally using the store's `tool_key`.
///
/// Unlike `seal_identifier` (which only hashes secret-marker strings), `seal_commit`
/// always computes an HMAC-SHA256 digest so raw commit SHAs never touch DuckDB.
pub fn seal_commit(tool_key: Option<&[u8]>, canonical_sha: &str) -> String {
    let key = tool_key.unwrap_or(b"dendro-unkeyed-git-commit");
    format!("commit:{}", hmac_domain(key, "git_commit", canonical_sha))
}

// ---------------------------------------------------------------------------
// Persisted fact and coverage models
// ---------------------------------------------------------------------------

/// Persisted row in `git_evidence_facts` (`MTN-21 / D4c`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GitEvidenceFact {
    pub source_key: String,
    pub source_id: String,
    pub generation: i64,
    pub harness: String,
    pub canonical_harness: String,
    /// Sealed local session identifier when `session_match_status == "matched"`; `None` otherwise.
    pub native_session: Option<String>,
    /// Sealed commit identity (`commit:<64-hex>` under `tool_key`); never the raw SHA.
    pub commit_sealed: String,
    /// Ordered sealed parent commit identities (`[]` when root commit or when `merge_status != "observed"`).
    pub parent_sealed: Vec<String>,
    /// `Some(parent_count > 1)` when `merge_status == "observed"`; `None` when `"not_observed"`.
    pub is_merge: Option<bool>,
    /// `"observed"` or `"not_observed"`.
    pub merge_status: String,
    /// Reason when `merge_status == "not_observed"` (e.g. `"parents_not_recorded"`, `"abbreviated_parents"`).
    pub merge_missing_reason: Option<String>,
    /// `Some(bool)` when `default_branch_status == "observed"`; `None` when `"not_observed"`.
    pub on_default_branch: Option<bool>,
    /// `"observed"` or `"not_observed"`.
    pub default_branch_status: String,
    /// Record timestamp when the commit was first observed.
    pub observed_at: Option<DateTime<Utc>>,
    /// `"observed"` when `observed_at.is_some()`, else `"not_observable"`.
    pub observed_at_status: String,
    /// `"matched"`, `"ambiguous"`, `"unmatched"`, or `"unknown"`.
    pub session_match_status: String,
    /// Always `"local_git_history"`.
    pub evidence_source: String,
    /// `"exportable"` when `FactExportConfig` key material is configured; `"non_exportable"` otherwise.
    pub export_status: String,
    /// Customer key identifier (`^[a-z0-9][a-z0-9._-]{1,63}$`) when exportable; `None` otherwise.
    pub pseudonym_key_id: Option<String>,
    /// `fact-export.v1/git_evidence` HMAC when exportable; `None` otherwise.
    pub git_evidence_key: Option<String>,
    /// `fact-export.v1/session` HMAC when exportable and `session_match_status == "matched"`; `None` otherwise.
    pub session_key: Option<String>,
    /// `fact-export.v1/commit` HMAC when exportable; `None` otherwise.
    pub commit_hmac: Option<String>,
    /// Ordered `fact-export.v1/commit` parent HMACs when exportable (`[]` when unconfigured, root, or not observed).
    pub parent_hmacs: Vec<String>,
}

impl GitEvidenceFact {
    /// Returns `true` when `export_status == "exportable"` and all required `fact-export-v1`
    /// HMAC columns (`pseudonym_key_id`, `git_evidence_key`, `commit_hmac`) are populated.
    pub fn is_exportable(&self) -> bool {
        self.export_status == EXPORT_STATUS_EXPORTABLE
            && self.pseudonym_key_id.is_some()
            && self.git_evidence_key.is_some()
            && self.commit_hmac.is_some()
    }

    /// Maps local `merge_status` (`"observed" | "not_observed"`) to the `fact-export-v1`
    /// wire enum (`"observed" | "unknown"`).
    pub fn export_merge_status(&self) -> &'static str {
        if self.merge_status == MERGE_STATUS_OBSERVED {
            MERGE_STATUS_OBSERVED
        } else {
            MERGE_STATUS_EXPORT_UNKNOWN
        }
    }

    /// Maps local `default_branch_status` (`"observed" | "not_observed"`) to the
    /// `fact-export-v1` wire enum (`"observed" | "unknown"`).
    pub fn export_default_branch_status(&self) -> &'static str {
        if self.default_branch_status == DEFAULT_BRANCH_STATUS_OBSERVED {
            DEFAULT_BRANCH_STATUS_OBSERVED
        } else {
            DEFAULT_BRANCH_STATUS_EXPORT_UNKNOWN
        }
    }

    /// Microseconds since Unix epoch for `fact-export-v1` `observed_at_us`.
    pub fn observed_at_us(&self) -> Option<i64> {
        self.observed_at.map(|ts| ts.timestamp_micros())
    }
}

/// Per-source git evidence coverage and missingness counters (`git_evidence_coverage`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct GitEvidenceCoverage {
    pub source_id: String,
    pub generation: i64,
    pub harness: String,
    /// Number of unique canonical full-SHA commits observed in this source generation.
    pub observed: i64,
    /// Number of abbreviated commit/parent SHAs refused (not resolved without repo access).
    pub unresolved_abbreviated: i64,
    /// Number of observed commits with `merge_status == "not_observed"`.
    pub missing_parents: i64,
    /// Number of observed commits with `session_match_status == "ambiguous"`.
    pub ambiguous: i64,
    /// Number of observed commits with `session_match_status` in `("unmatched", "unknown")`.
    pub unmatched: i64,
    /// Number of observed commits with `default_branch_status == "not_observed"`.
    pub default_branch_not_observed: i64,
}

impl GitEvidenceCoverage {
    pub fn has_activity(&self) -> bool {
        self.observed > 0
            || self.unresolved_abbreviated > 0
            || self.missing_parents > 0
            || self.ambiguous > 0
            || self.unmatched > 0
            || self.default_branch_not_observed > 0
    }
}

// ---------------------------------------------------------------------------
// Pure Git Evidence Parser (bounded, zero-regex)
// ---------------------------------------------------------------------------

/// Returns `true` iff `token` is a canonical full commit SHA:
/// lowercase 40-hex SHA-1 or lowercase 64-hex SHA-256.
pub fn is_canonical_commit_sha(token: &str) -> bool {
    (token.len() == 40 || token.len() == 64)
        && token
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Returns `true` iff `token` is an abbreviated lowercase hex SHA (`7..=63` hex chars,
/// excluding `40`, with at least one digit or a typical 7..16 short-SHA length).
pub fn is_abbreviated_commit_sha(token: &str) -> bool {
    let len = token.len();
    if !(7..=63).contains(&len) || len == 40 {
        return false;
    }
    token
        .bytes()
        .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn bounded_str(input: &str, max_bytes: usize) -> &str {
    if input.len() <= max_bytes {
        return input;
    }
    let mut end = max_bytes;
    while end > 0 && !input.is_char_boundary(end) {
        end -= 1;
    }
    &input[..end]
}

/// Classification of a tool command for git output interpretation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitCommandKind {
    /// Not a git command (e.g. `cargo test`, `ls`); output is ignored.
    NonGit,
    /// `git rev-parse` resolving a commit (`HEAD`, `--verify`, `--short`, etc.).
    RevParseCommit { short: bool },
    /// `git log` / `git rev-list` / `git show`.
    Log {
        expects_full_parents: bool,
        expects_short_parents: bool,
        expects_short_commit: bool,
    },
    /// `git cat-file -p <target>` or `git cat-file commit <target>`.
    CatFile {
        target_sha: Option<String>,
        target_abbreviated: bool,
    },
    /// `git commit ...`
    Commit,
    /// `git symbolic-ref refs/remotes/origin/HEAD` or `git rev-parse --symbolic-full-name origin/HEAD`.
    SymbolicDefaultBranch,
    /// `git rev-parse --abbrev-ref HEAD` or `git branch --show-current`.
    CurrentBranch,
    /// `git status`, `git checkout`, `git switch`, `git branch`.
    BranchStatus,
    /// `git remote show origin`.
    RemoteShowOrigin,
    /// Other git command (`git diff`, `git add`, etc.) that may still carry standard git output.
    OtherGit,
    /// Unpaired tool output where no prior tool call was seen.
    Inferred,
}

/// Parse a shell command string into zero or more `GitCommandKind` invocations.
/// Strictly bounded to `MAX_GIT_COMMAND_BYTES` and `MAX_GIT_LINES_PER_RECORD` segments.
pub fn classify_shell_command(command: &str) -> GitCommandKind {
    let bounded = bounded_str(command, MAX_GIT_COMMAND_BYTES);
    let mut found_git = Vec::new();

    for (seg_idx, segment) in split_shell_segments(bounded).into_iter().enumerate() {
        if seg_idx >= MAX_GIT_LINES_PER_RECORD {
            break;
        }
        if let Some(kind) = parse_single_git_segment(segment) {
            found_git.push(kind);
        }
    }

    if found_git.is_empty() {
        return GitCommandKind::NonGit;
    }
    // Prefer the richest git subcommand in a pipeline/chain (e.g. `git commit -m msg && git log -1 --format='%H %P'`).
    fn rank(kind: &GitCommandKind) -> u8 {
        match kind {
            GitCommandKind::Log {
                expects_full_parents: true,
                ..
            } => 9,
            GitCommandKind::CatFile { .. } => 8,
            GitCommandKind::Log { .. } => 7,
            GitCommandKind::RevParseCommit { .. } => 6,
            GitCommandKind::Commit => 5,
            GitCommandKind::SymbolicDefaultBranch | GitCommandKind::RemoteShowOrigin => 4,
            GitCommandKind::CurrentBranch => 3,
            GitCommandKind::BranchStatus => 2,
            GitCommandKind::OtherGit | GitCommandKind::Inferred => 1,
            GitCommandKind::NonGit => 0,
        }
    }
    found_git
        .into_iter()
        .max_by_key(rank)
        .unwrap_or(GitCommandKind::NonGit)
}

fn split_shell_segments(cmd: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let bytes = cmd.as_bytes();
    let mut start = 0usize;
    let mut i = 0usize;
    let mut in_single = false;
    let mut in_double = false;
    while i < bytes.len() {
        match bytes[i] {
            b'\'' if !in_double => in_single = !in_single,
            b'"' if !in_single => in_double = !in_double,
            b'\\' if in_double && i + 1 < bytes.len() => {
                i += 2;
                continue;
            }
            b';' | b'\n' | b'|' if !in_single && !in_double => {
                out.push(&cmd[start..i]);
                if i + 1 < bytes.len() && bytes[i] == b'|' && bytes[i + 1] == b'|' {
                    i += 1;
                }
                start = i + 1;
            }
            b'&' if !in_single && !in_double && i + 1 < bytes.len() && bytes[i + 1] == b'&' => {
                out.push(&cmd[start..i]);
                i += 1;
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
        if out.len() >= MAX_GIT_LINES_PER_RECORD {
            break;
        }
    }
    if start < bytes.len() && out.len() < MAX_GIT_LINES_PER_RECORD {
        out.push(&cmd[start..]);
    }
    out
}

fn tokenize_shell_words(segment: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let bytes = segment.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'\'' if !in_double => {
                in_single = !in_single;
            }
            b'"' if !in_single => {
                in_double = !in_double;
            }
            b'\\' if !in_single && i + 1 < bytes.len() => {
                i += 1;
                current.push(bytes[i] as char);
            }
            c if c.is_ascii_whitespace() && !in_single && !in_double => {
                if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                    if words.len() >= 128 {
                        break;
                    }
                }
            }
            _ => current.push(b as char),
        }
        i += 1;
    }
    if !current.is_empty() && words.len() < 128 {
        words.push(current);
    }
    words
}

fn is_git_executable(word: &str) -> bool {
    let trimmed = word.trim_matches(|c| c == '"' || c == '\'');
    trimmed == "git"
        || trimmed == "git.exe"
        || trimmed.ends_with("/git")
        || trimmed.ends_with("\\git")
        || trimmed.ends_with("\\git.exe")
}

fn parse_single_git_segment(segment: &str) -> Option<GitCommandKind> {
    let words = tokenize_shell_words(segment.trim());
    if words.is_empty() {
        return None;
    }

    // Handle shell wrappers such as `bash -lc "git rev-parse HEAD"` or `sh -c "git ..."`.
    let mut idx = 0usize;
    while idx < words.len() {
        let w = &words[idx];
        if w.contains('=') && !w.starts_with('-') && !is_git_executable(w) {
            idx += 1;
            continue;
        }
        if matches!(
            w.as_str(),
            "bash"
                | "/bin/bash"
                | "sh"
                | "/bin/sh"
                | "zsh"
                | "pwsh"
                | "powershell"
                | "cmd"
                | "cmd.exe"
        ) {
            idx += 1;
            while idx < words.len() && words[idx].starts_with('-') {
                idx += 1;
            }
            if idx < words.len() {
                let inner = words[idx..].join(" ");
                if inner != segment.trim() {
                    let classified = classify_shell_command(&inner);
                    if classified != GitCommandKind::NonGit {
                        return Some(classified);
                    }
                }
            }
            return None;
        }
        break;
    }

    // Find `git` executable token.
    while idx < words.len() && !is_git_executable(&words[idx]) {
        if words[idx].contains('=') && !words[idx].starts_with('-') {
            idx += 1;
        } else {
            return None;
        }
    }
    if idx >= words.len() {
        return None;
    }
    idx += 1; // skip `git`

    // Skip global git flags before the subcommand.
    while idx < words.len() {
        let arg = words[idx].as_str();
        if matches!(
            arg,
            "-C" | "-c" | "--git-dir" | "--work-tree" | "--namespace"
        ) {
            idx += 2;
        } else if arg.starts_with('-') {
            idx += 1;
        } else {
            break;
        }
    }

    let subcmd = words.get(idx)?.as_str();
    let args = &words[idx + 1..];

    match subcmd {
        "rev-parse" => {
            let mut has_short = false;
            let mut is_abbrev_ref = false;
            let mut is_symbolic_full = false;
            let mut is_meta_query = false;
            let mut mentions_origin_head = false;
            let mut mentions_head = false;
            for a in args {
                if a == "--short" || a.starts_with("--short=") {
                    has_short = true;
                } else if a == "--abbrev-ref" || a.starts_with("--abbrev-ref=") {
                    is_abbrev_ref = true;
                } else if a == "--symbolic-full-name" {
                    is_symbolic_full = true;
                } else if matches!(
                    a.as_str(),
                    "--is-inside-work-tree"
                        | "--is-bare-repository"
                        | "--is-inside-git-dir"
                        | "--show-toplevel"
                        | "--git-dir"
                        | "--git-common-dir"
                        | "--show-cdup"
                        | "--show-prefix"
                ) {
                    is_meta_query = true;
                } else if a.contains("origin/HEAD") {
                    mentions_origin_head = true;
                } else if a == "HEAD" {
                    mentions_head = true;
                }
            }
            if is_meta_query {
                Some(GitCommandKind::OtherGit)
            } else if (is_abbrev_ref || is_symbolic_full) && mentions_origin_head {
                Some(GitCommandKind::SymbolicDefaultBranch)
            } else if is_abbrev_ref && mentions_head {
                Some(GitCommandKind::CurrentBranch)
            } else if is_abbrev_ref || is_symbolic_full {
                Some(GitCommandKind::OtherGit)
            } else {
                Some(GitCommandKind::RevParseCommit { short: has_short })
            }
        }
        "log" | "rev-list" | "show" => {
            let mut expects_full_parents = false;
            let mut expects_short_parents = false;
            let mut expects_short_commit = false;
            let mut i = 0usize;
            while i < args.len() {
                let a = args[i].as_str();
                if a == "--parents" {
                    expects_full_parents = true;
                } else if a == "--oneline" || a == "--abbrev-commit" {
                    expects_short_commit = true;
                } else if let Some(fmt) = a
                    .strip_prefix("--format=")
                    .or_else(|| a.strip_prefix("--pretty="))
                {
                    inspect_log_format(
                        fmt,
                        &mut expects_full_parents,
                        &mut expects_short_parents,
                        &mut expects_short_commit,
                    );
                } else if matches!(a, "--format" | "--pretty") && i + 1 < args.len() {
                    inspect_log_format(
                        &args[i + 1],
                        &mut expects_full_parents,
                        &mut expects_short_parents,
                        &mut expects_short_commit,
                    );
                    i += 1;
                }
                i += 1;
            }
            Some(GitCommandKind::Log {
                expects_full_parents,
                expects_short_parents,
                expects_short_commit,
            })
        }
        "cat-file" => {
            let mut target_sha = None;
            let mut target_abbreviated = false;
            for a in args {
                if a.starts_with('-') || a == "commit" || a == "tag" || a == "tree" || a == "blob" {
                    continue;
                }
                let candidate = a.strip_suffix("^{commit}").unwrap_or(a.as_str());
                if is_canonical_commit_sha(candidate) {
                    target_sha = Some(candidate.to_string());
                } else if is_abbreviated_commit_sha(candidate) {
                    target_abbreviated = true;
                }
            }
            Some(GitCommandKind::CatFile {
                target_sha,
                target_abbreviated,
            })
        }
        "commit" => Some(GitCommandKind::Commit),
        "symbolic-ref" => {
            if args.iter().any(|a| a.contains("origin/HEAD")) {
                Some(GitCommandKind::SymbolicDefaultBranch)
            } else if args.iter().any(|a| a == "HEAD") {
                Some(GitCommandKind::CurrentBranch)
            } else {
                Some(GitCommandKind::OtherGit)
            }
        }
        "branch" if args.iter().any(|a| a == "--show-current") => {
            Some(GitCommandKind::CurrentBranch)
        }
        "status" | "checkout" | "switch" | "branch" => Some(GitCommandKind::BranchStatus),
        "remote" if args.iter().any(|a| a == "show") => Some(GitCommandKind::RemoteShowOrigin),
        _ => Some(GitCommandKind::OtherGit),
    }
}

fn inspect_log_format(
    fmt: &str,
    expects_full_parents: &mut bool,
    expects_short_parents: &mut bool,
    expects_short_commit: &mut bool,
) {
    let trimmed = fmt.strip_prefix("format:").unwrap_or(fmt);
    if trimmed == "oneline" {
        return;
    }
    if trimmed.contains("%P") {
        *expects_full_parents = true;
    }
    if trimmed.contains("%p") {
        *expects_short_parents = true;
    }
    if trimmed.contains("%h") && !trimmed.contains("%H") {
        *expects_short_commit = true;
    }
}

/// Strip Codex's execution envelope (`Chunk ID: ...\nOutput:\n...`) or JSON output
/// wrapper (`{"output": "...", "exit_code": 0}`) if present.
pub fn unwrap_command_output_text(raw_output: &str) -> String {
    let bounded = bounded_str(raw_output, MAX_GIT_OUTPUT_BYTES);
    let trimmed = bounded.trim();
    if trimmed.starts_with('{') {
        if let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(trimmed) {
            if let Some(inner) = obj
                .get("output")
                .or_else(|| obj.get("stdout"))
                .and_then(Value::as_str)
            {
                return strip_codex_envelope(bounded_str(inner, MAX_GIT_OUTPUT_BYTES));
            }
        }
    }
    strip_codex_envelope(bounded)
}

fn strip_codex_envelope(text: &str) -> String {
    let mut saw_header = false;
    let mut byte_offset = 0usize;
    for line in text.lines() {
        let line_len_with_nl = text[byte_offset..]
            .find('\n')
            .map(|p| p + 1)
            .unwrap_or(text.len() - byte_offset);
        let trimmed = line.trim();
        let lower = trimmed.to_ascii_lowercase();
        if trimmed.eq_ignore_ascii_case("output:") && saw_header {
            let start = byte_offset + line_len_with_nl;
            return text[start..].to_string();
        }
        let is_meta = [
            "chunk id:",
            "wall time:",
            "process exited with code",
            "process exited with exit code",
            "original token count:",
            "total output lines:",
            "session id:",
        ]
        .iter()
        .any(|prefix| lower.starts_with(prefix));
        if is_meta {
            saw_header = true;
            byte_offset += line_len_with_nl;
        } else {
            break;
        }
    }
    text.to_string()
}

/// A single commit observation extracted from a record before cross-record/session merging.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCommitObservation {
    pub commit_sha: String,
    /// `Some(parents)` when parents were explicitly observed in canonical form (`[]` = root commit);
    /// `None` when parents were not observed.
    pub parents: Option<Vec<String>>,
    pub merge_missing_reason: Option<&'static str>,
    pub branch: Option<String>,
}

/// Output of parsing a single tool output or command result.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedGitOutput {
    pub commits: Vec<ParsedCommitObservation>,
    /// Parent list discovered from `git cat-file -p` when target SHA was in the command or prior context.
    pub cat_file_parents: Option<(Option<String>, Vec<String>)>,
    pub unresolved_abbreviated: u64,
    pub observed_branch: Option<String>,
    pub observed_default_branch: Option<String>,
}

/// Parse git command output conservatively under `kind` and hostile-input bounds.
pub fn parse_git_output(kind: &GitCommandKind, raw_output: &str) -> ParsedGitOutput {
    if matches!(kind, GitCommandKind::NonGit) {
        return ParsedGitOutput::default();
    }
    let unwrapped = unwrap_command_output_text(raw_output);
    let bounded = bounded_str(&unwrapped, MAX_GIT_OUTPUT_BYTES);
    let mut out = ParsedGitOutput::default();

    let lines: Vec<&str> = bounded.lines().take(MAX_GIT_LINES_PER_RECORD).collect();
    if lines.is_empty() {
        return out;
    }

    // 1. Check for `git cat-file -p` commit object header:
    //    `tree <sha>` followed by 0+ `parent <sha>` and `author ...`
    if let Some(cat_obs) = try_parse_cat_file_lines(&lines, kind) {
        out.unresolved_abbreviated += cat_obs.unresolved_abbreviated;
        out.commits.extend(cat_obs.commits);
        out.cat_file_parents = cat_obs.cat_file_parents;
        if !out.commits.is_empty()
            || out.cat_file_parents.is_some()
            || out.unresolved_abbreviated > 0
        {
            return out;
        }
    }

    // 2. Dispatch specific branch/default-branch metadata commands.
    match kind {
        GitCommandKind::SymbolicDefaultBranch => {
            for line in &lines {
                if let Some(def) = extract_default_branch_from_ref(line.trim()) {
                    out.observed_default_branch = Some(def);
                    return out;
                }
            }
            return out;
        }
        GitCommandKind::CurrentBranch => {
            if let Some(first) = lines.iter().map(|l| l.trim()).find(|l| !l.is_empty()) {
                if is_safe_branch_name(first) && first != "HEAD" {
                    out.observed_branch = Some(first.to_string());
                }
            }
            return out;
        }
        GitCommandKind::RemoteShowOrigin => {
            for line in &lines {
                let trimmed = line.trim();
                if let Some(rest) = trimmed.strip_prefix("HEAD branch:") {
                    let branch = rest.trim();
                    if is_safe_branch_name(branch) && branch != "(unknown)" {
                        out.observed_default_branch = Some(branch.to_string());
                    }
                }
            }
            return out;
        }
        _ => {}
    }

    // 3. Scan lines for commit outputs (`git commit`, `git log`, `git rev-parse`, etc.).
    let mut i = 0usize;
    while i < lines.len() {
        let trimmed = lines[i].trim();
        if trimmed.is_empty() {
            i += 1;
            continue;
        }

        // 3a. `refs/remotes/origin/<branch>` symbolic-ref line.
        if let Some(def) = extract_default_branch_from_ref(trimmed) {
            out.observed_default_branch = Some(def);
            i += 1;
            continue;
        }

        // 3b. `HEAD branch: <branch>` from `git remote show origin`.
        if let Some(rest) = trimmed.strip_prefix("HEAD branch:") {
            let b = rest.trim();
            if is_safe_branch_name(b) && b != "(unknown)" {
                out.observed_default_branch = Some(b.to_string());
                i += 1;
                continue;
            }
        }

        // 3c. `On branch <branch>` from `git status`.
        if let Some(rest) = trimmed.strip_prefix("On branch ") {
            let b = rest.trim();
            if is_safe_branch_name(b) {
                out.observed_branch = Some(b.to_string());
                i += 1;
                continue;
            }
        }

        // 3d. `git commit` summary line: `[<branch> <sha>] <subject>` or `[<branch> (root-commit) <sha>] <subject>`.
        if let Some(commit_bracket) = try_parse_git_commit_bracket_line(trimmed) {
            if let Some(b) = &commit_bracket.branch {
                out.observed_branch = Some(b.clone());
            }
            if commit_bracket.is_abbreviated {
                out.unresolved_abbreviated += 1;
            } else if let Some(obs) = commit_bracket.observation {
                if out.commits.len() < MAX_OBSERVED_COMMITS_PER_SOURCE {
                    out.commits.push(obs);
                }
            }
            i += 1;
            continue;
        }

        // 3e. Standard `git log` / `git show` header: `commit <sha>` (optionally followed by ` (...)`).
        if let Some(rest) = trimmed.strip_prefix("commit ") {
            let first_tok = rest.split_whitespace().next().unwrap_or("");
            if is_canonical_commit_sha(first_tok) {
                // Inspect following header lines for `parent <sha>` or `Merge: ...`.
                let mut parents = Vec::new();
                let mut saw_parent_header = false;
                let mut saw_abbrev_parent = false;
                let mut j = i + 1;
                while j < lines.len() {
                    let hdr = lines[j].trim();
                    if hdr.is_empty() {
                        break;
                    }
                    if let Some(p_rest) = hdr.strip_prefix("parent ") {
                        saw_parent_header = true;
                        let p_tok = p_rest.trim();
                        if is_canonical_commit_sha(p_tok) && parents.len() < MAX_PARENTS_PER_COMMIT
                        {
                            parents.push(p_tok.to_string());
                        } else if is_abbreviated_commit_sha(p_tok) {
                            saw_abbrev_parent = true;
                            out.unresolved_abbreviated += 1;
                        }
                    } else if let Some(m_rest) = hdr.strip_prefix("Merge:") {
                        for m_tok in m_rest.split_whitespace() {
                            if is_canonical_commit_sha(m_tok)
                                && parents.len() < MAX_PARENTS_PER_COMMIT
                            {
                                saw_parent_header = true;
                                parents.push(m_tok.to_string());
                            } else if is_abbreviated_commit_sha(m_tok) {
                                saw_abbrev_parent = true;
                                out.unresolved_abbreviated += 1;
                            }
                        }
                    } else if hdr.starts_with("commit ") {
                        break;
                    }
                    j += 1;
                }
                let (obs_parents, reason) = if saw_abbrev_parent {
                    (None, Some(MERGE_REASON_ABBREVIATED_PARENTS))
                } else if saw_parent_header {
                    (Some(parents), None)
                } else {
                    (None, Some(MERGE_REASON_PARENTS_NOT_RECORDED))
                };
                if out.commits.len() < MAX_OBSERVED_COMMITS_PER_SOURCE {
                    out.commits.push(ParsedCommitObservation {
                        commit_sha: first_tok.to_string(),
                        parents: obs_parents,
                        merge_missing_reason: reason,
                        branch: out.observed_branch.clone(),
                    });
                }
                i = j;
                continue;
            } else if is_abbreviated_commit_sha(first_tok) {
                out.unresolved_abbreviated += 1;
                i += 1;
                continue;
            }
        }

        // 3f. Space-separated hex tokens line (`git rev-parse HEAD`, `git log --format='%H %P'`, etc.).
        let tokens: Vec<&str> = trimmed
            .split_whitespace()
            .take(MAX_PARENTS_PER_COMMIT + 2)
            .collect();
        if !tokens.is_empty() {
            let first = tokens[0];
            let rest_tokens = &tokens[1..];
            if is_canonical_commit_sha(first) {
                if rest_tokens.is_empty() {
                    // Single canonical SHA on the line:
                    // If the command explicitly requested full parents (`%P` or `--parents`),
                    // an empty parent list on that line means a root commit (0 parents).
                    // Otherwise (e.g. `git rev-parse HEAD` or `git log --format=%H`), parents were not recorded.
                    let (parents, reason) = match kind {
                        GitCommandKind::Log {
                            expects_full_parents: true,
                            expects_short_parents: false,
                            ..
                        } => (Some(Vec::new()), None),
                        _ => (None, Some(MERGE_REASON_PARENTS_NOT_RECORDED)),
                    };
                    if out.commits.len() < MAX_OBSERVED_COMMITS_PER_SOURCE {
                        out.commits.push(ParsedCommitObservation {
                            commit_sha: first.to_string(),
                            parents,
                            merge_missing_reason: reason,
                            branch: out.observed_branch.clone(),
                        });
                    }
                } else if rest_tokens.iter().all(|t| is_canonical_commit_sha(t)) {
                    let parents: Vec<String> = rest_tokens
                        .iter()
                        .take(MAX_PARENTS_PER_COMMIT)
                        .map(|s| (*s).to_string())
                        .collect();
                    if out.commits.len() < MAX_OBSERVED_COMMITS_PER_SOURCE {
                        out.commits.push(ParsedCommitObservation {
                            commit_sha: first.to_string(),
                            parents: Some(parents),
                            merge_missing_reason: None,
                            branch: out.observed_branch.clone(),
                        });
                    }
                } else if rest_tokens
                    .iter()
                    .all(|t| is_abbreviated_commit_sha(t) || is_canonical_commit_sha(t))
                {
                    // Commit is canonical full SHA, but one or more parents are abbreviated (e.g. `%H %p`).
                    let abbrev_count = rest_tokens
                        .iter()
                        .filter(|t| is_abbreviated_commit_sha(t))
                        .count() as u64;
                    out.unresolved_abbreviated += abbrev_count.max(1);
                    if out.commits.len() < MAX_OBSERVED_COMMITS_PER_SOURCE {
                        out.commits.push(ParsedCommitObservation {
                            commit_sha: first.to_string(),
                            parents: None,
                            merge_missing_reason: Some(MERGE_REASON_ABBREVIATED_PARENTS),
                            branch: out.observed_branch.clone(),
                        });
                    }
                }
            } else if is_abbreviated_commit_sha(first) {
                // Abbreviated SHA in rev-parse or git log --oneline / --format='%h ...'
                let accept_abbrev = match kind {
                    GitCommandKind::RevParseCommit { .. }
                    | GitCommandKind::Log { .. }
                    | GitCommandKind::Commit
                    | GitCommandKind::CatFile { .. } => true,
                    GitCommandKind::Inferred | GitCommandKind::OtherGit => {
                        // In unpaired/inferred output, accept a single abbreviated hex token line
                        // (e.g. `abc1234` from `git rev-parse --short HEAD`) or `<short_sha> <msg>`
                        // when the token has at least one ASCII digit (`0..=9`) so plain English
                        // words like `"deadbeef"` or `"feedback"` are not miscounted.
                        first.bytes().any(|b| b.is_ascii_digit())
                            && (rest_tokens.is_empty()
                                || rest_tokens.iter().all(|t| is_abbreviated_commit_sha(t)))
                    }
                    _ => false,
                };
                if accept_abbrev {
                    out.unresolved_abbreviated += 1;
                }
            }
        }

        i += 1;
    }

    out
}

struct CommitBracketLine {
    branch: Option<String>,
    is_abbreviated: bool,
    observation: Option<ParsedCommitObservation>,
}

fn try_parse_git_commit_bracket_line(line: &str) -> Option<CommitBracketLine> {
    if !line.starts_with('[') {
        return None;
    }
    let close_idx = line.find("] ")?;
    let inside = &line[1..close_idx];
    let after = line[close_idx + 2..].trim();
    if inside.is_empty() || after.is_empty() {
        return None;
    }
    if inside
        .bytes()
        .any(|b| matches!(b, b'"' | b'\'' | b':' | b',' | b'{' | b'}' | b'[' | b']'))
    {
        return None;
    }
    let parts: Vec<&str> = inside.split_whitespace().collect();
    let (branch, is_root, sha_token) = match parts.as_slice() {
        [branch, "(root-commit)", sha] => (Some(*branch), true, *sha),
        ["detached", "HEAD", sha] => (None, false, *sha),
        [branch, sha] => (Some(*branch), false, *sha),
        _ => return None,
    };
    if let Some(b) = branch {
        if !is_safe_branch_name(b) {
            return None;
        }
    }
    if is_canonical_commit_sha(sha_token) {
        let (parents, reason) = if is_root {
            (Some(Vec::new()), None)
        } else {
            (None, Some(MERGE_REASON_PARENTS_NOT_RECORDED))
        };
        Some(CommitBracketLine {
            branch: branch.map(str::to_string),
            is_abbreviated: false,
            observation: Some(ParsedCommitObservation {
                commit_sha: sha_token.to_string(),
                parents,
                merge_missing_reason: reason,
                branch: branch.map(str::to_string),
            }),
        })
    } else if is_abbreviated_commit_sha(sha_token) {
        Some(CommitBracketLine {
            branch: branch.map(str::to_string),
            is_abbreviated: true,
            observation: None,
        })
    } else {
        None
    }
}

fn try_parse_cat_file_lines(lines: &[&str], kind: &GitCommandKind) -> Option<ParsedGitOutput> {
    let first = lines.first()?.trim();
    let tree_sha = first.strip_prefix("tree ")?.trim();
    if !is_canonical_commit_sha(tree_sha) && !is_abbreviated_commit_sha(tree_sha) {
        return None;
    }
    let mut parents = Vec::new();
    let mut saw_author = false;
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
        } else if trimmed.starts_with("author ") || trimmed.starts_with("committer ") {
            saw_author = true;
        }
    }
    if !saw_author {
        return None;
    }

    let mut out = ParsedGitOutput {
        unresolved_abbreviated,
        ..Default::default()
    };

    if let GitCommandKind::CatFile {
        target_sha,
        target_abbreviated,
    } = kind
    {
        if *target_abbreviated {
            out.unresolved_abbreviated += 1;
            return Some(out);
        }
        if let Some(commit_sha) = target_sha {
            let (obs_parents, reason) = if saw_abbrev_parent {
                (None, Some(MERGE_REASON_ABBREVIATED_PARENTS))
            } else {
                (Some(parents.clone()), None)
            };
            out.commits.push(ParsedCommitObservation {
                commit_sha: commit_sha.clone(),
                parents: obs_parents,
                merge_missing_reason: reason,
                branch: None,
            });
            out.cat_file_parents = Some((Some(commit_sha.clone()), parents));
            return Some(out);
        }
    }

    if !saw_abbrev_parent {
        out.cat_file_parents = Some((None, parents));
    }
    Some(out)
}

fn extract_default_branch_from_ref(line: &str) -> Option<String> {
    let branch = line
        .strip_prefix("refs/remotes/origin/")
        .or_else(|| line.strip_prefix("origin/"))?;
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
struct AccumulatedCommit {
    commit_sha: String,
    parents: Option<Vec<String>>,
    merge_missing_reason: Option<&'static str>,
    observed_at: Option<DateTime<Utc>>,
    sessions: BTreeSet<String>,
    saw_unmatched_session: bool,
    saw_conflicted_session: bool,
    branch: Option<String>,
    explicit_on_default_branch: Option<bool>,
}

/// Accumulates git evidence across records of a single source file pass (`0..consumed`).
#[derive(Debug, Default, Clone)]
pub struct GitEvidenceScanner {
    pending_calls_by_id: BTreeMap<String, GitCommandKind>,
    last_pending_call: Option<GitCommandKind>,
    commits_by_sha: BTreeMap<String, AccumulatedCommit>,
    commit_order: Vec<String>,
    unresolved_abbreviated: u64,
    current_branch: Option<String>,
    recorded_default_branch: Option<String>,
    explicit_on_default_branch: Option<bool>,
}

impl GitEvidenceScanner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record structured branch / default-branch / commit metadata from a harness record.
    #[allow(clippy::too_many_arguments)]
    pub fn observe_structured_metadata(
        &mut self,
        raw_session: Option<&str>,
        header_conflict: bool,
        ts: Option<DateTime<Utc>>,
        branch: Option<&str>,
        default_branch: Option<&str>,
        explicit_on_default: Option<bool>,
        commit_candidate: Option<&str>,
    ) {
        if let Some(b) = branch.map(str::trim).filter(|s| is_safe_branch_name(s)) {
            self.current_branch = Some(b.to_string());
        }
        if let Some(d) = default_branch.map(str::trim).and_then(|s| {
            extract_default_branch_from_ref(s)
                .or_else(|| is_safe_branch_name(s).then(|| s.to_string()))
        }) {
            self.recorded_default_branch = Some(d);
        }
        if let Some(on_def) = explicit_on_default {
            self.explicit_on_default_branch = Some(on_def);
        }
        if let Some(cand) = commit_candidate.map(str::trim).filter(|s| !s.is_empty()) {
            if is_canonical_commit_sha(cand) {
                self.record_commit_observation(
                    raw_session,
                    header_conflict,
                    ts,
                    ParsedCommitObservation {
                        commit_sha: cand.to_string(),
                        parents: None,
                        merge_missing_reason: Some(MERGE_REASON_PARENTS_NOT_RECORDED),
                        branch: self.current_branch.clone(),
                    },
                );
            } else if is_abbreviated_commit_sha(cand) {
                self.unresolved_abbreviated += 1;
            }
        }
    }

    /// Record a tool call that may execute a git command.
    pub fn observe_tool_call(&mut self, call_id: Option<&str>, command: &str) {
        let kind = classify_shell_command(command);
        if let Some(id) = call_id.map(str::trim).filter(|s| !s.is_empty()) {
            if self.pending_calls_by_id.len() < MAX_PENDING_CALLS_PER_SOURCE {
                self.pending_calls_by_id
                    .insert(id.to_string(), kind.clone());
            }
        }
        if kind != GitCommandKind::NonGit {
            self.last_pending_call = Some(kind);
        }
    }

    /// Record a tool result / command output and extract any git evidence.
    #[allow(clippy::too_many_arguments)]
    pub fn observe_tool_output(
        &mut self,
        call_id: Option<&str>,
        inline_command: Option<&str>,
        raw_session: Option<&str>,
        header_conflict: bool,
        ts: Option<DateTime<Utc>>,
        failed: Option<bool>,
        raw_output: &str,
    ) {
        if failed == Some(true) {
            return;
        }
        let kind = if let Some(cmd) = inline_command {
            classify_shell_command(cmd)
        } else if let Some(id) = call_id.map(str::trim).filter(|s| !s.is_empty()) {
            self.pending_calls_by_id
                .get(id)
                .cloned()
                .or_else(|| self.last_pending_call.take())
                .unwrap_or(GitCommandKind::Inferred)
        } else {
            self.last_pending_call
                .take()
                .unwrap_or(GitCommandKind::Inferred)
        };

        let parsed = parse_git_output(&kind, raw_output);
        self.unresolved_abbreviated += parsed.unresolved_abbreviated;

        if let Some(b) = parsed.observed_branch {
            self.current_branch = Some(b);
        }
        if let Some(def) = parsed.observed_default_branch {
            self.recorded_default_branch = Some(def);
        }

        for obs in parsed.commits {
            self.record_commit_observation(raw_session, header_conflict, ts, obs);
        }

        // If `git cat-file -p` reported parents without a target SHA on the command,
        // and the session has exactly one observed commit so far, enrich that commit's parents.
        if let Some((target_opt, parents)) = parsed.cat_file_parents {
            let target_sha = target_opt
                .or_else(|| (self.commit_order.len() == 1).then(|| self.commit_order[0].clone()));
            if let Some(sha) = target_sha {
                if let Some(entry) = self.commits_by_sha.get_mut(&sha) {
                    if entry.parents.is_none() {
                        entry.parents = Some(parents);
                        entry.merge_missing_reason = None;
                    }
                }
            }
        }
    }

    fn record_commit_observation(
        &mut self,
        raw_session: Option<&str>,
        header_conflict: bool,
        ts: Option<DateTime<Utc>>,
        obs: ParsedCommitObservation,
    ) {
        if !is_canonical_commit_sha(&obs.commit_sha) {
            return;
        }
        let clean_session = raw_session.map(str::trim).filter(|s| !s.is_empty());
        let branch = obs.branch.or_else(|| self.current_branch.clone());
        let explicit_on_def = self.explicit_on_default_branch;

        if let Some(existing) = self.commits_by_sha.get_mut(&obs.commit_sha) {
            if existing.observed_at.is_none() && ts.is_some() {
                existing.observed_at = ts;
            }
            if existing.parents.is_none() && obs.parents.is_some() {
                existing.parents = obs.parents;
                existing.merge_missing_reason = None;
            } else if existing.parents.is_none() && existing.merge_missing_reason.is_none() {
                existing.merge_missing_reason = obs.merge_missing_reason;
            }
            if existing.branch.is_none() && branch.is_some() {
                existing.branch = branch;
            }
            if existing.explicit_on_default_branch.is_none() && explicit_on_def.is_some() {
                existing.explicit_on_default_branch = explicit_on_def;
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
                merge_missing_reason: obs.merge_missing_reason,
                observed_at: ts,
                sessions,
                saw_unmatched_session,
                saw_conflicted_session,
                branch,
                explicit_on_default_branch: explicit_on_def,
            },
        );
    }

    /// Materialize the `GitEvidenceFact` rows and `GitEvidenceCoverage` counters
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

            // Default branch status: observed ONLY if explicitly established
            let (default_branch_status, on_default_branch) = if let Some(on_def) = acc
                .explicit_on_default_branch
                .or(self.explicit_on_default_branch)
            {
                (DEFAULT_BRANCH_STATUS_OBSERVED.to_string(), Some(on_def))
            } else if let (Some(branch), Some(def_branch)) = (
                acc.branch.as_deref().or(self.current_branch.as_deref()),
                self.recorded_default_branch.as_deref(),
            ) {
                (
                    DEFAULT_BRANCH_STATUS_OBSERVED.to_string(),
                    Some(branch == def_branch),
                )
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

            // Session match status
            let (session_match_status, matched_raw_session, native_session) =
                if header_conflict || acc.saw_conflicted_session || acc.sessions.len() > 1 {
                    ambiguous += 1;
                    (SESSION_MATCH_AMBIGUOUS.to_string(), None, None)
                } else if acc.sessions.len() == 1 {
                    let raw_s = acc.sessions.iter().next().unwrap().as_str();
                    (
                        SESSION_MATCH_MATCHED.to_string(),
                        Some(raw_s),
                        Some(seal_session_fn(raw_s)),
                    )
                } else if acc.saw_unmatched_session {
                    unmatched += 1;
                    (SESSION_MATCH_UNMATCHED.to_string(), None, None)
                } else {
                    unmatched += 1;
                    (SESSION_MATCH_UNKNOWN.to_string(), None, None)
                };

            // Fact-export pseudonym columns (computed only when key material is Ready)
            let (
                export_status,
                pseudonym_key_id,
                commit_hmac,
                parent_hmacs,
                session_key,
                git_evidence_key,
            ) = if export_config.is_ready() {
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
                let s_key = matched_raw_session
                    .and_then(|raw_s| export_config.derive_session_key(source_id, harness, raw_s));
                let obs_us = acc.observed_at.map(|ts| ts.timestamp_micros());
                let g_key = export_config.derive_git_evidence_key(
                    source_id,
                    &c_hmac,
                    obs_us,
                    s_key.as_deref(),
                    &session_match_status,
                );
                (
                    EXPORT_STATUS_EXPORTABLE.to_string(),
                    key_id,
                    Some(c_hmac),
                    p_hmacs,
                    s_key,
                    g_key,
                )
            } else {
                (
                    EXPORT_STATUS_NON_EXPORTABLE.to_string(),
                    None,
                    None,
                    Vec::new(),
                    None,
                    None,
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
                git_evidence_key,
                session_key,
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
        let sess_a = cfg
            .derive_session_key("fixture-source-a", "codex", "codex-a")
            .unwrap();
        assert_eq!(
            sess_a,
            "a611d713964de4e128bb2c164ae859a338454881afd7ce079a2afb34d562169d"
        );

        // Commit & parent HMACs from `generate_fact_vectors.py` / `merge_evidence.json`:
        // - `"a" * 40` (40-hex SHA-1) -> `bc07f227444a11a858de0f4321da9826c54d4c5def78f1110d799ee523e11137`
        // - `"b" * 40` -> `fc234836df3c3fdda2c9fba641de25da4385401bccb4b72c42ab186026fb1138`
        // - `"c" * 40` -> `ea0c2c4cac64a682cd4e0536629f337406009c747f5c5d04d316fbdecb4e097b`
        // - `"d" * 40` -> `64b5f1c4221dd313c441f42af4cf19f248f862f6898e51aaadb9df915d01c47b`
        // - `"d" * 64` (64-hex SHA-256) -> `fc2ca46bc3253793e1bbbf308ba546183ef796657c761e2b127c7f6ce2f8b6f8`
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

        // Ambiguous row (`index = 2`: `commit = "c" * 40`, `observed_at_us = TIME - 2`, `session_key = None`, `session_match_status = "ambiguous"`)
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
        assert!(!is_abbreviated_commit_sha(&"a".repeat(40)));
        assert!(!is_abbreviated_commit_sha("abc"));
    }

    #[test]
    fn parser_handles_rev_parse_log_parents_root_and_abbreviated() {
        let c = "1".repeat(40);
        let p1 = "2".repeat(40);
        let p2 = "3".repeat(40);

        // 1. git rev-parse HEAD -> commit observed, parents not observed
        let kind_rp = classify_shell_command("git rev-parse HEAD");
        let out_rp = parse_git_output(&kind_rp, &format!("{c}\n"));
        assert_eq!(out_rp.commits.len(), 1);
        assert_eq!(out_rp.commits[0].commit_sha, c);
        assert_eq!(out_rp.commits[0].parents, None);
        assert_eq!(
            out_rp.commits[0].merge_missing_reason,
            Some(MERGE_REASON_PARENTS_NOT_RECORDED)
        );

        // 2. git log --format='%H %P' with 2 parents -> merge
        let kind_log = classify_shell_command("git log -1 --format='%H %P'");
        let out_merge = parse_git_output(&kind_log, &format!("{c} {p1} {p2}\n"));
        assert_eq!(out_merge.commits.len(), 1);
        assert_eq!(
            out_merge.commits[0].parents,
            Some(vec![p1.clone(), p2.clone()])
        );

        // 3. git log --format='%H %P' with 1 parent -> non-merge
        let out_single = parse_git_output(&kind_log, &format!("{c} {p1}\n"));
        assert_eq!(out_single.commits.len(), 1);
        assert_eq!(out_single.commits[0].parents, Some(vec![p1]));

        // 4. git log --format='%H %P' with 0 parents -> root commit
        let out_root = parse_git_output(&kind_log, &format!("{c}\n"));
        assert_eq!(out_root.commits.len(), 1);
        assert_eq!(out_root.commits[0].parents, Some(Vec::new()));

        // 5. Abbreviated SHA in git commit output -> unresolved counter incremented, 0 commits
        let kind_commit = classify_shell_command("git commit -m 'msg'");
        let out_abbrev = parse_git_output(
            &kind_commit,
            "[main abc1234] feat: add feature\n 1 file changed\n",
        );
        assert!(out_abbrev.commits.is_empty());
        assert_eq!(out_abbrev.unresolved_abbreviated, 1);
        assert_eq!(out_abbrev.observed_branch.as_deref(), Some("main"));
    }

    #[test]
    fn fact_export_config_precedence_and_refusals_match_d3_rules() {
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

        let xdg_dir = dir.join("xdg_config");
        let obs_dir = xdg_dir.join("observability");
        std::fs::create_dir_all(&obs_dir).unwrap();
        let consumer_env_path = obs_dir.join("consumer.env");

        let key_env = "11".repeat(32);
        let key_root = "22".repeat(32);
        let key_consumer = "33".repeat(32);

        std::fs::write(
            &consumer_env_path,
            format!(
                "DENDRO_FACT_EXPORT_KEY_HEX={key_consumer}\nDENDRO_FACT_EXPORT_KEY_ID=kid_consumer\n"
            ),
        )
        .unwrap();

        // 1. Unconfigured when nothing is provided
        let uncfg = FactExportConfig::load_with(|_| None, None, None);
        assert_eq!(uncfg.state(), FactExportKeyState::Unconfigured);
        assert!(!uncfg.is_ready());

        // 2. XDG consumer.env used when process env and private root are absent
        let from_consumer = FactExportConfig::load_with(|_| None, None, Some(&consumer_env_path));
        assert!(from_consumer.is_ready());
        assert_eq!(from_consumer.pseudonym_key_id(), Some("kid_consumer"));
        assert_eq!(
            from_consumer.derive_commit_hmac(&"a".repeat(40)),
            FactExportConfig::from_key_hex(&key_consumer, Some("kid_consumer"))
                .derive_commit_hmac(&"a".repeat(40))
        );

        // 3. Private data root overrides consumer.env, and process env overrides private data root
        #[cfg(any(windows, target_os = "linux"))]
        let priv_root = {
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
            let kid_child = crate::platform_v1::parse_child_name(
                DEFAULT_FACT_EXPORT_KEY_ID_FILENAME,
                crate::platform_v1::current_native_platform(),
            )
            .unwrap();
            let mut kf = priv_cap.create_file(&key_child).unwrap();
            kf.file_mut()
                .write_all(format!("{key_root}\n").as_bytes())
                .unwrap();
            drop(kf);
            let mut idf = priv_cap.create_file(&kid_child).unwrap();
            idf.file_mut().write_all(b"kid_root\n").unwrap();
            drop(idf);
            let p = priv_cap.path().to_path_buf();
            drop(priv_cap);
            p
        };
        #[cfg(not(any(windows, target_os = "linux")))]
        let priv_root = {
            let p = dir.join("priv_root");
            std::fs::create_dir_all(&p).unwrap();
            let kp = p.join(DEFAULT_FACT_EXPORT_KEY_FILENAME);
            let idp = p.join(DEFAULT_FACT_EXPORT_KEY_ID_FILENAME);
            std::fs::write(&kp, format!("{key_root}\n")).unwrap();
            std::fs::write(&idp, "kid_root\n").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o700)).unwrap();
                std::fs::set_permissions(&kp, std::fs::Permissions::from_mode(0o600)).unwrap();
                std::fs::set_permissions(&idp, std::fs::Permissions::from_mode(0o600)).unwrap();
            }
            p
        };

        let from_root =
            FactExportConfig::load_with(|_| None, Some(&priv_root), Some(&consumer_env_path));
        assert!(from_root.is_ready());
        assert_eq!(from_root.pseudonym_key_id(), Some("kid_root"));
        assert_eq!(
            from_root.derive_commit_hmac(&"a".repeat(40)),
            FactExportConfig::from_key_hex(&key_root, Some("kid_root"))
                .derive_commit_hmac(&"a".repeat(40))
        );

        let from_env = FactExportConfig::load_with(
            |k| match k {
                ENV_FACT_EXPORT_KEY_HEX => Some(key_env.clone()),
                ENV_FACT_EXPORT_KEY_ID => Some("kid_env".to_string()),
                _ => None,
            },
            Some(&priv_root),
            Some(&consumer_env_path),
        );
        assert!(from_env.is_ready());
        assert_eq!(from_env.pseudonym_key_id(), Some("kid_env"));
        assert_eq!(
            from_env.derive_commit_hmac(&"a".repeat(40)),
            FactExportConfig::from_key_hex(&key_env, Some("kid_env"))
                .derive_commit_hmac(&"a".repeat(40))
        );

        // 4. Malformed hex key or invalid key_id or forbidden K_org is refused
        let malformed = FactExportConfig::load_with(
            |k| match k {
                ENV_FACT_EXPORT_KEY_HEX => Some("not-hex".to_string()),
                ENV_FACT_EXPORT_KEY_ID => Some("kid_env".to_string()),
                _ => None,
            },
            None,
            None,
        );
        assert_eq!(malformed.state(), FactExportKeyState::InvalidKeyMaterial);

        let invalid_kid = FactExportConfig::load_with(
            |k| match k {
                ENV_FACT_EXPORT_KEY_HEX => Some(key_env.clone()),
                ENV_FACT_EXPORT_KEY_ID => Some("BAD KEY ID WITH SPACES".to_string()),
                _ => None,
            },
            None,
            None,
        );
        assert_eq!(invalid_kid.state(), FactExportKeyState::InvalidKeyId);

        let k_org_refused = FactExportConfig::load_with(
            |k| match k {
                ENV_FACT_EXPORT_KEY_HEX => Some(key_env.clone()),
                ENV_FACT_EXPORT_KEY_ID => Some("kid_env".to_string()),
                "DENDRO_K_ORG_HEX" => Some("ff".repeat(32)),
                _ => None,
            },
            None,
            None,
        );
        assert_eq!(
            k_org_refused.state(),
            FactExportKeyState::ForbiddenOrgKeyPresent
        );

        // 5. Key-file variable inside consumer.env is refused (must only come from process env)
        let bad_consumer = obs_dir.join("bad_consumer.env");
        std::fs::write(
            &bad_consumer,
            format!(
                "DENDRO_FACT_EXPORT_KEY_FILE={}\nDENDRO_FACT_EXPORT_KEY_ID=kid_c\n",
                priv_root.join(DEFAULT_FACT_EXPORT_KEY_FILENAME).display()
            ),
        )
        .unwrap();
        let file_in_consumer = FactExportConfig::load_with(|_| None, None, Some(&bad_consumer));
        assert_eq!(
            file_in_consumer.state(),
            FactExportKeyState::InvalidKeyMaterial
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
