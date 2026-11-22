//! `link-id-v1` privacy-preserving session link derivation (`MTN-19 / D3`).
//!
//! Normative specification: `docs/contracts/link-id-v1/SPEC.md`
//! Pinned contract digest: `07dff9562958ec286c1b096f0eb15c91deddb284c26d99904afeba2b882d0ab5`
//!
//! Security boundary:
//! - Ingest and endpoint paths accept ONLY derived `K_link` material ([`LinkKey`] / [`SessionLinkConfig`]).
//! - Raw organization master keys (`K_org`) and HKDF-SHA256 derivation live strictly inside the
//!   explicit [`admin`] module for key provisioning and contract vector verification, and are
//!   never accepted by [`SessionLinkConfig`] or any ingest API.

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

type HmacSha256 = Hmac<Sha256>;

/// Contract identifier for `link-id-v1`.
pub const LINK_ID_CONTRACT: &str = "link-id-v1";

/// Pinned `BUNDLE-SHA256` from `docs/contracts/link-id-v1/CONTRACT-DIGEST`.
pub const LINK_ID_CONTRACT_DIGEST: &str =
    "07dff9562958ec286c1b096f0eb15c91deddb284c26d99904afeba2b882d0ab5";

/// Canonical attribute key for the session link identifier.
pub const AGENT_SESSION_LINK_ID: &str = "agent.session.link_id";

/// Canonical attribute key for the unlinked diagnostic reason.
pub const AGENT_SESSION_UNLINKED_REASON: &str = "agent.session.unlinked_reason";

/// Sentinel value emitted when a session cannot or must not be linked.
pub const AGENT_SESSION_LINK_UNLINKED: &str = "unlinked";

/// Domain separator for `link_id` HMAC-SHA256 computation.
pub const LINK_ID_DOMAIN: &str = "agent.session.link/v1";

/// Domain separator for HKDF-SHA256 `K_link` derivation in admin provisioning.
pub const LINK_KEY_PURPOSE: &str = "agent.session.link.key/v1";

/// Fixed ASCII salt for HKDF-Extract in `link-id-v1`.
pub const LINK_KEY_HKDF_SALT: &[u8] = b"agent-otel-bridge:link-id-v1:salt";

/// Minimum required length in bytes for `K_org` (256 bits).
pub const MIN_ORG_MASTER_KEY_BYTES: usize = 32;

/// Exact required length in bytes for `K_link` (256 bits).
pub const LINK_KEY_BYTES: usize = 32;

/// Maximum UTF-8 byte length for any single `link-id-v1` input field.
pub const MAX_LINK_FIELD_BYTES: usize = 1024;

/// Closed vocabulary of canonical harness identifiers from `link-id-v1/vocabulary.json`
/// (aligned with `DATA_DICTIONARY.md` "I01").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CanonicalHarness {
    Antigravity,
    Claude,
    Codex,
    Grok,
    Pi,
}

impl CanonicalHarness {
    /// Returns the canonical `agent.harness` slug used in `link-id-v1` MAC framing.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Antigravity => "antigravity",
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Grok => "grok",
            Self::Pi => "pi",
        }
    }

    /// Returns the numeric IPC wire `client_id` corresponding to this canonical harness.
    #[must_use]
    pub const fn wire_client_id(self) -> u32 {
        match self {
            Self::Antigravity => 1,
            Self::Claude => 2,
            Self::Codex => 3,
            Self::Grok => 4,
            Self::Pi => 5,
        }
    }

    /// Parses a numeric IPC wire `client_id` according to `link-id-v1/vocabulary.json` §5.2.2:
    /// - `0` => `Ok(None)` (unspecified)
    /// - `1..=5` => `Ok(Some(CanonicalHarness))`
    /// - any other value => `Err(UnlinkedReason::UnknownHarness)`
    pub const fn from_wire_client_id(wire_id: u32) -> Result<Option<Self>, UnlinkedReason> {
        match wire_id {
            0 => Ok(None),
            1 => Ok(Some(Self::Antigravity)),
            2 => Ok(Some(Self::Claude)),
            3 => Ok(Some(Self::Codex)),
            4 => Ok(Some(Self::Grok)),
            5 => Ok(Some(Self::Pi)),
            _ => Err(UnlinkedReason::UnknownHarness),
        }
    }

    /// Parses either a canonical `agent.harness` value or a known alias from `vocabulary.json`:
    /// requires ASCII-only input (`0x00..=0x7F`), converts ASCII uppercase `A-Z` to lowercase `a-z`
    /// without trimming whitespace, and looks up in the closed vocabulary.
    #[must_use]
    pub fn from_alias(raw: &str) -> Option<Self> {
        if !raw.is_ascii() {
            return None;
        }
        match raw.to_ascii_lowercase().as_str() {
            "antigravity" | "agy" => Some(Self::Antigravity),
            "claude" | "claude-code" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            "grok" => Some(Self::Grok),
            "pi" => Some(Self::Pi),
            _ => None,
        }
    }
}

/// Resolves and cross-checks `(raw_harness, wire_client_id)` according to `link-id-v1/SPEC.md` §5.2.
pub fn resolve_harness_identity(
    raw_harness: Option<&str>,
    wire_client_id: Option<u32>,
) -> Result<CanonicalHarness, UnlinkedReason> {
    let parsed_wire = match wire_client_id {
        Some(id) => CanonicalHarness::from_wire_client_id(id)?,
        None => None,
    };

    let parsed_harness = match raw_harness {
        Some(h) if !is_unicode_whitespace_only(h) => {
            Some(CanonicalHarness::from_alias(h).ok_or(UnlinkedReason::UnknownHarness)?)
        }
        _ => None,
    };

    match (parsed_harness, parsed_wire) {
        (Some(h), Some(w)) if h != w => Err(UnlinkedReason::ConflictingHarness),
        (Some(h), _) => Ok(h),
        (None, Some(w)) => Ok(w),
        (None, None) => Err(UnlinkedReason::MissingHarness),
    }
}

/// Closed-set diagnostic reason codes when `agent.session.link_id == "unlinked"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnlinkedReason {
    InvalidKeyMaterial,
    NonHarnessOrigin,
    MissingOrg,
    MissingEndpointScope,
    MissingHarness,
    UnknownHarness,
    ConflictingHarness,
    MissingNativeSession,
    InvalidFieldCharacters,
}

impl UnlinkedReason {
    /// Returns the normative snake_case string from `link-id-v1/SPEC.md` §5.1.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidKeyMaterial => "invalid_key_material",
            Self::NonHarnessOrigin => "non_harness_origin",
            Self::MissingOrg => "missing_org",
            Self::MissingEndpointScope => "missing_endpoint_scope",
            Self::MissingHarness => "missing_harness",
            Self::UnknownHarness => "unknown_harness",
            Self::ConflictingHarness => "conflicting_harness",
            Self::MissingNativeSession => "missing_native_session",
            Self::InvalidFieldCharacters => "invalid_field_characters",
        }
    }

    /// Parses a serialized `unlinked_reason` code back into [`UnlinkedReason`].
    #[must_use]
    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "invalid_key_material" => Some(Self::InvalidKeyMaterial),
            "non_harness_origin" => Some(Self::NonHarnessOrigin),
            "missing_org" => Some(Self::MissingOrg),
            "missing_endpoint_scope" => Some(Self::MissingEndpointScope),
            "missing_harness" => Some(Self::MissingHarness),
            "unknown_harness" => Some(Self::UnknownHarness),
            "conflicting_harness" => Some(Self::ConflictingHarness),
            "missing_native_session" => Some(Self::MissingNativeSession),
            "invalid_field_characters" => Some(Self::InvalidFieldCharacters),
            _ => None,
        }
    }
}

impl fmt::Display for UnlinkedReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Checks whether a persisted `link_id` string is either `"unlinked"` or a 64-char lowercase hex digest.
#[must_use]
pub fn is_valid_persisted_link_id(value: &str) -> bool {
    if value == AGENT_SESSION_LINK_UNLINKED {
        return true;
    }
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Checks if a character is Unicode whitespace (`White_Space` / general category `Z` / control whitespace).
#[must_use]
pub fn is_unicode_whitespace(ch: char) -> bool {
    ch.is_whitespace()
}

/// Returns `true` when `value` is empty or consists solely of Unicode whitespace characters.
#[must_use]
pub fn is_unicode_whitespace_only(value: &str) -> bool {
    value.is_empty() || value.chars().all(is_unicode_whitespace)
}

/// Validates an ASCII slug (`org`, `team`, `endpoint_scope`, `epoch`) per `link-id-v1/SPEC.md` §4.3.3:
/// non-empty, `<= 1024` bytes, and every byte in printable non-whitespace ASCII `0x21..=0x7E`.
#[must_use]
pub fn is_valid_ascii_slug(value: &str) -> bool {
    if value.is_empty() || value.len() > MAX_LINK_FIELD_BYTES {
        return false;
    }
    value.bytes().all(|b| (0x21..=0x7E).contains(&b))
}

/// Checks if a character is in Unicode general category `Cc` (C0 or C1 control character).
#[must_use]
fn is_control_cc(ch: char) -> bool {
    let code = ch as u32;
    code <= 0x1F || (0x7F..=0x9F).contains(&code)
}

/// Validates `native_session` per `link-id-v1/SPEC.md` §4.3.4:
/// - Non-empty and `<= 1024` UTF-8 bytes
/// - Not whitespace-only
/// - Does not start or end with any Unicode `White_Space` character
/// - Contains no NUL (`\0`), no ASCII control (`< 0x20` or `0x7F`), and no Unicode `Cc` (`0x80..=0x9F`).
#[must_use]
pub fn is_valid_native_session(value: &str) -> bool {
    if value.is_empty() || value.len() > MAX_LINK_FIELD_BYTES {
        return false;
    }
    if is_unicode_whitespace_only(value) {
        return false;
    }
    if value.chars().next().is_some_and(is_unicode_whitespace)
        || value.chars().next_back().is_some_and(is_unicode_whitespace)
    {
        return false;
    }
    !value.chars().any(is_control_cc)
}

/// Encodes a byte slice with a 4-byte unsigned big-endian length prefix (`LP(x)`).
#[must_use]
pub fn lp_encode(bytes: &[u8]) -> Vec<u8> {
    let len = u32::try_from(bytes.len()).expect("LP field length fits in u32");
    let mut out = Vec::with_capacity(4 + bytes.len());
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(bytes);
    out
}

/// Encodes a UTF-8 string with a 4-byte unsigned big-endian length prefix (`LP(x)`).
#[must_use]
pub fn lp_encode_str(value: &str) -> Vec<u8> {
    lp_encode(value.as_bytes())
}

/// Computes `HMAC-SHA256(key, data)`.
#[must_use]
pub fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC-SHA256 accepts any key length");
    mac.update(data);
    let result = mac.finalize().into_bytes();
    let mut out = [0u8; 32];
    out.copy_from_slice(&result);
    out
}

/// Zeroize-on-drop wrapper around the 32-byte endpoint-scoped `K_link` key.
///
/// Never implements `Clone`, never logs or serializes key bytes, compares in constant time,
/// and overwrites memory on drop.
pub struct LinkKey([u8; LINK_KEY_BYTES]);

impl PartialEq for LinkKey {
    fn eq(&self, other: &Self) -> bool {
        let diff = self
            .0
            .iter()
            .zip(other.0.iter())
            .fold(0u8, |acc, (&a, &b)| std::hint::black_box(acc | (a ^ b)));
        diff == 0
    }
}

impl Eq for LinkKey {}

impl LinkKey {
    /// Creates a `LinkKey` from an exact 32-byte array.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; LINK_KEY_BYTES]) -> Self {
        Self(bytes)
    }

    /// Creates a `LinkKey` from a byte slice, rejecting any slice whose length is not 32 bytes.
    pub fn from_slice(slice: &[u8]) -> Result<Self, UnlinkedReason> {
        if slice.len() != LINK_KEY_BYTES {
            return Err(UnlinkedReason::InvalidKeyMaterial);
        }
        let mut arr = [0u8; LINK_KEY_BYTES];
        arr.copy_from_slice(slice);
        Ok(Self(arr))
    }

    /// Parses a 64-character hex-encoded `K_link`, rejecting short, long, or invalid hex.
    pub fn from_hex(hex_str: &str) -> Result<Self, UnlinkedReason> {
        let trimmed = hex_str.trim();
        if trimmed.is_empty() {
            return Err(UnlinkedReason::InvalidKeyMaterial);
        }
        let mut decoded = hex::decode(trimmed).map_err(|_| UnlinkedReason::InvalidKeyMaterial)?;
        let res = Self::from_slice(&decoded);
        decoded.fill(0);
        std::hint::black_box(&mut decoded);
        res
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn to_hex(&self) -> String {
        hex::encode(self.0)
    }
}

impl fmt::Debug for LinkKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("LinkKey([REDACTED])")
    }
}

impl Drop for LinkKey {
    fn drop(&mut self) {
        self.0.fill(0);
        std::hint::black_box(&mut self.0);
    }
}

/// Borrowed input bundle for `link-id-v1` evaluation.
#[derive(Debug, Clone, Default)]
pub struct LinkIdInput<'a> {
    pub org: Option<&'a str>,
    pub team: Option<&'a str>,
    pub endpoint_scope: Option<&'a str>,
    pub epoch: Option<&'a str>,
    pub harness: Option<&'a str>,
    pub wire_client_id: Option<u32>,
    pub native_session: Option<&'a str>,
    /// Session origin (`None` or `Some("harness")` means harness-minted; any other value is unlinked).
    pub origin: Option<&'a str>,
}

/// Output of `link-id-v1` evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkIdOutcome {
    Linked {
        link_id: String,
        canonical_harness: CanonicalHarness,
    },
    Unlinked {
        reason: UnlinkedReason,
    },
}

impl LinkIdOutcome {
    /// Returns the `agent.session.link_id` string (`64` lowercase hex chars or `"unlinked"`).
    #[must_use]
    pub fn link_id(&self) -> &str {
        match self {
            Self::Linked { link_id, .. } => link_id.as_str(),
            Self::Unlinked { .. } => AGENT_SESSION_LINK_UNLINKED,
        }
    }

    /// Returns the `UnlinkedReason` when unlinked, or `None` when linked.
    #[must_use]
    pub const fn unlinked_reason(&self) -> Option<UnlinkedReason> {
        match self {
            Self::Linked { .. } => None,
            Self::Unlinked { reason } => Some(*reason),
        }
    }

    /// Converts into `(link_id, Option<UnlinkedReason>)`.
    #[must_use]
    pub fn into_parts(self) -> (String, Option<UnlinkedReason>) {
        match self {
            Self::Linked { link_id, .. } => (link_id, None),
            Self::Unlinked { reason } => (AGENT_SESSION_LINK_UNLINKED.to_string(), Some(reason)),
        }
    }
}

/// Validates the scope metadata and session fields in `input` in normative precedence order.
fn validate_link_input<'a>(
    input: &'a LinkIdInput<'a>,
) -> Result<(&'a str, &'a str, CanonicalHarness, &'a str), UnlinkedReason> {
    // 1. Key-derivation context slugs (`team`, `epoch`): when supplied, must be valid ASCII slugs.
    if let Some(team) = input.team {
        if !is_valid_ascii_slug(team) {
            return Err(UnlinkedReason::InvalidKeyMaterial);
        }
    }
    if let Some(epoch) = input.epoch {
        if !is_valid_ascii_slug(epoch) {
            return Err(UnlinkedReason::InvalidKeyMaterial);
        }
    }

    // 2. Origin check: absent (`None`) or exact `"harness"` -> harness; any other value -> NonHarnessOrigin.
    if let Some(origin) = input.origin {
        if origin != "harness" {
            return Err(UnlinkedReason::NonHarnessOrigin);
        }
    }

    // 3. org check
    let org = match input.org {
        Some(v) if !is_unicode_whitespace_only(v) => v,
        _ => return Err(UnlinkedReason::MissingOrg),
    };
    if !is_valid_ascii_slug(org) {
        return Err(UnlinkedReason::InvalidFieldCharacters);
    }

    // 4. endpoint_scope check
    let endpoint_scope = match input.endpoint_scope {
        Some(v) if !is_unicode_whitespace_only(v) => v,
        _ => return Err(UnlinkedReason::MissingEndpointScope),
    };
    if !is_valid_ascii_slug(endpoint_scope) {
        return Err(UnlinkedReason::InvalidFieldCharacters);
    }

    // 5. Harness resolution & cross-check
    let canonical_harness = resolve_harness_identity(input.harness, input.wire_client_id)?;

    // 6. native_session check
    let native_session = match input.native_session {
        Some(ns) if !is_unicode_whitespace_only(ns) => ns,
        _ => return Err(UnlinkedReason::MissingNativeSession),
    };
    if !is_valid_native_session(native_session) {
        return Err(UnlinkedReason::InvalidFieldCharacters);
    }

    Ok((org, endpoint_scope, canonical_harness, native_session))
}

/// Computes `agent.session.link_id` from a pre-derived endpoint `K_link` ([`LinkKey`]) and [`LinkIdInput`].
///
/// This is the sole cryptographic function used on the ingest/endpoint path; it accepts only [`LinkKey`],
/// never `K_org`.
#[must_use]
pub fn derive_session_link_with_key(k_link: &LinkKey, input: &LinkIdInput<'_>) -> LinkIdOutcome {
    let (org, endpoint_scope, canonical_harness, native_session) = match validate_link_input(input)
    {
        Ok(valid) => valid,
        Err(reason) => return LinkIdOutcome::Unlinked { reason },
    };

    let mut msg = Vec::with_capacity(
        4 + LINK_ID_DOMAIN.len()
            + 4
            + org.len()
            + 4
            + endpoint_scope.len()
            + 4
            + canonical_harness.as_str().len()
            + 4
            + native_session.len(),
    );
    msg.extend_from_slice(&lp_encode_str(LINK_ID_DOMAIN));
    msg.extend_from_slice(&lp_encode_str(org));
    msg.extend_from_slice(&lp_encode_str(endpoint_scope));
    msg.extend_from_slice(&lp_encode_str(canonical_harness.as_str()));
    msg.extend_from_slice(&lp_encode_str(native_session));

    let digest = hmac_sha256(&k_link.0, &msg);
    LinkIdOutcome::Linked {
        link_id: hex::encode(digest),
        canonical_harness,
    }
}

/// Size cap for `$XDG_CONFIG_HOME/observability/consumer.env` (16 KiB), matching `telemetry.rs`.
pub const MAX_CONSUMER_ENV_BYTES: u64 = 16 * 1024;

/// Fixed, key-free warning message emitted at most once when a raw `K_org` variable is detected.
pub const K_ORG_REFUSAL_WARNING: &str = "dendro: link-id configuration refused: raw K_org variable is forbidden on endpoints (reason=invalid_key_material)";

static K_ORG_WARNING_ONCE: std::sync::Once = std::sync::Once::new();
static K_ORG_WARNING_EMITTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

fn emit_k_org_warning_once() {
    K_ORG_WARNING_ONCE.call_once(|| {
        K_ORG_WARNING_EMITTED.store(true, std::sync::atomic::Ordering::SeqCst);
        eprintln!("{K_ORG_REFUSAL_WARNING}");
    });
}

const FORBIDDEN_K_ORG_VARS: &[&str] = &[
    "DENDRO_LINK_K_ORG_HEX",
    "DENDRO_K_ORG_HEX",
    "DENDRO_LINK_K_ORG_FILE",
    "DENDRO_K_ORG_FILE",
    "DENDRO_LINK_K_ORG",
    "DENDRO_K_ORG",
    "AGENT_OTEL_LINK_K_ORG_HEX",
    "AGENT_OTEL_K_ORG_HEX",
    "AGENT_OTEL_LINK_K_ORG_FILE",
    "AGENT_OTEL_K_ORG_FILE",
    "AGENT_OTEL_LINK_K_ORG",
    "AGENT_OTEL_K_ORG",
    "K_ORG",
    "K_ORG_HEX",
    "K_ORG_FILE",
];

fn strip_export_prefix(name: &str) -> &str {
    let trimmed = name.trim();
    if let Some(rest) = trimmed.strip_prefix("export") {
        if rest.starts_with(char::is_whitespace) {
            return rest.trim();
        }
    }
    trimmed
}

fn is_k_org_var_name(name: &str) -> bool {
    let cleaned = strip_export_prefix(name);
    FORBIDDEN_K_ORG_VARS
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(cleaned))
}

#[must_use]
fn consumer_env_path_from_vars(
    xdg_config_home: Option<&std::ffi::OsStr>,
    home: Option<&std::ffi::OsStr>,
) -> Option<PathBuf> {
    let root = xdg_config_home
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            home.filter(|v| !v.is_empty())
                .map(|h| PathBuf::from(h).join(".config"))
        })?;
    if !root.is_absolute() {
        return None;
    }
    Some(root.join("observability/consumer.env"))
}

/// Resolves the canonical `consumer.env` path (`$XDG_CONFIG_HOME/observability/consumer.env`
/// or `$HOME/.config/observability/consumer.env`). Never resolves relative to the current directory.
#[must_use]
pub fn consumer_env_path() -> Option<PathBuf> {
    consumer_env_path_from_vars(
        std::env::var_os("XDG_CONFIG_HOME").as_deref(),
        std::env::var_os("HOME").as_deref(),
    )
}

/// Reads `$XDG_CONFIG_HOME/observability/consumer.env` (or `$HOME/.config/observability/consumer.env`)
/// with the 16 KiB size cap (`MAX_CONSUMER_ENV_BYTES`). Never reads relative to CWD.
#[must_use]
pub fn read_consumer_env() -> Option<String> {
    let path = consumer_env_path()?;
    read_consumer_env_from(&path)
}

/// Reads an explicit `consumer.env` file path with the 16 KiB size cap (`MAX_CONSUMER_ENV_BYTES`),
/// refusing relative paths, non-regular files, and symlinks.
#[must_use]
pub fn read_consumer_env_from(path: &Path) -> Option<String> {
    use std::io::Read;
    if !path.is_absolute() {
        return None;
    }
    let sym = std::fs::symlink_metadata(path).ok()?;
    if !sym.file_type().is_file() || sym.len() > MAX_CONSUMER_ENV_BYTES {
        return None;
    }
    let file = std::fs::File::open(path).ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.len() > MAX_CONSUMER_ENV_BYTES {
        return None;
    }
    let mut input = String::new();
    file.take(MAX_CONSUMER_ENV_BYTES + 1)
        .read_to_string(&mut input)
        .ok()?;
    if input.len() as u64 > MAX_CONSUMER_ENV_BYTES {
        zeroize_string(&mut input);
        return None;
    }
    Some(input)
}

/// Resolves Dendro's verified private data root (`$XDG_DATA_HOME/dendro` or
/// `$HOME/.local/share/dendro` on Linux/Unix, `%LOCALAPPDATA%\Dendro` on Windows).
/// Never resolves relative to the output DB's parent or the current working directory.
#[must_use]
pub fn default_private_data_root() -> Option<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        let base = std::env::var_os("XDG_DATA_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .filter(|v| !v.is_empty())
                    .map(|h| PathBuf::from(h).join(".local/share"))
            })?;
        if !base.is_absolute() {
            return None;
        }
        let candidate = base.join("dendro");
        crate::platform_v1::open_existing_private_root(&candidate)
            .ok()
            .map(|root| root.path().to_path_buf())
    }
    #[cfg(windows)]
    {
        let base = std::env::var_os("LOCALAPPDATA")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)?;
        if !base.is_absolute() {
            return None;
        }
        let candidate = base.join("Dendro");
        crate::platform_v1::open_existing_private_root(&candidate)
            .ok()
            .map(|root| root.path().to_path_buf())
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let base = std::env::var_os("XDG_DATA_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .filter(|v| !v.is_empty())
                    .map(|h| PathBuf::from(h).join(".local/share"))
            })?;
        if !base.is_absolute() {
            return None;
        }
        let candidate = base.join("dendro");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let sym = std::fs::symlink_metadata(&candidate).ok()?;
            if !sym.file_type().is_dir() || (sym.permissions().mode() & 0o077 != 0) {
                return None;
            }
        }
        Some(candidate)
    }
}

/// Endpoint-scoped `link-id-v1` configuration for Dendro session ingestion.
///
/// Holds only the pre-derived `K_link` ([`LinkKey`]) and scope identifiers (`org`, `endpoint_scope`,
/// optional `team` and `epoch`). Without valid link configuration, every session resolves to
/// `("unlinked", Some(UnlinkedReason::InvalidKeyMaterial))` without failing ingestion.
#[derive(Default)]
pub struct SessionLinkConfig {
    org: Option<String>,
    endpoint_scope: Option<String>,
    team: Option<String>,
    epoch: Option<String>,
    key: Option<LinkKey>,
    k_org_rejected: bool,
}

impl fmt::Debug for SessionLinkConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionLinkConfig")
            .field("org", &self.org)
            .field("endpoint_scope", &self.endpoint_scope)
            .field("team", &self.team)
            .field("epoch", &self.epoch)
            .field("has_key", &self.key.is_some())
            .field("k_org_rejected", &self.k_org_rejected)
            .finish()
    }
}

impl SessionLinkConfig {
    /// Returns an unconfigured `SessionLinkConfig`. Every session will resolve to `"unlinked"`
    /// with `UnlinkedReason::InvalidKeyMaterial`.
    #[must_use]
    pub fn unconfigured() -> Self {
        Self::default()
    }

    /// Constructs a `SessionLinkConfig` from a pre-derived [`LinkKey`] and scope strings.
    #[must_use]
    pub fn from_link_key(
        k_link: LinkKey,
        org: impl Into<String>,
        endpoint_scope: impl Into<String>,
        team: Option<String>,
        epoch: Option<String>,
    ) -> Self {
        Self {
            org: Some(org.into()),
            endpoint_scope: Some(endpoint_scope.into()),
            team,
            epoch,
            key: Some(k_link),
            k_org_rejected: false,
        }
    }

    /// Constructs a `SessionLinkConfig` from raw `K_link` bytes and optional scope strings.
    /// If `key_bytes` is not exactly 32 bytes, `key` is set to `None` so all sessions fail closed
    /// to `"unlinked"` (`invalid_key_material`).
    #[must_use]
    pub fn from_key_slice(
        key_bytes: &[u8],
        org: Option<&str>,
        endpoint_scope: Option<&str>,
        team: Option<&str>,
        epoch: Option<&str>,
    ) -> Self {
        Self {
            org: org.map(ToOwned::to_owned),
            endpoint_scope: endpoint_scope.map(ToOwned::to_owned),
            team: team.map(ToOwned::to_owned),
            epoch: epoch.map(ToOwned::to_owned),
            key: LinkKey::from_slice(key_bytes).ok(),
            k_org_rejected: false,
        }
    }

    /// Constructs a `SessionLinkConfig` from hex-encoded `K_link` and optional scope strings.
    /// If `key_hex` does not decode to 32 bytes, `key` is set to `None` so all sessions fail closed
    /// to `"unlinked"` (`invalid_key_material`).
    #[must_use]
    pub fn from_key_hex(
        key_hex: &str,
        org: Option<&str>,
        endpoint_scope: Option<&str>,
        team: Option<&str>,
        epoch: Option<&str>,
    ) -> Self {
        Self {
            org: org.map(ToOwned::to_owned),
            endpoint_scope: endpoint_scope.map(ToOwned::to_owned),
            team: team.map(ToOwned::to_owned),
            epoch: epoch.map(ToOwned::to_owned),
            key: LinkKey::from_hex(key_hex).ok(),
            k_org_rejected: false,
        }
    }

    /// Loads `SessionLinkConfig` from process environment variables, `<private_data_root>/link.key`
    /// inside Dendro's verified private data root, and `$XDG_CONFIG_HOME/observability/consumer.env`.
    /// Never reads configuration or key files relative to the current working directory.
    #[must_use]
    pub fn from_env() -> Self {
        let root = default_private_data_root();
        let consumer = consumer_env_path();
        Self::from_env_and_paths(root.as_deref(), consumer.as_deref())
    }

    /// Loads `SessionLinkConfig` from process environment variables, optional `<private_data_root>/link.key`,
    /// and `$XDG_CONFIG_HOME/observability/consumer.env`.
    #[must_use]
    pub fn from_env_with_root(private_data_root: Option<&Path>) -> Self {
        let consumer = consumer_env_path();
        Self::from_env_and_paths(private_data_root, consumer.as_deref())
    }

    /// Loads `SessionLinkConfig` from the process environment, an explicit private data root, and
    /// an explicit `consumer.env` path.
    #[must_use]
    pub fn from_env_and_paths(
        private_data_root: Option<&Path>,
        consumer_env_file: Option<&Path>,
    ) -> Self {
        let mut process_env = BTreeMap::new();
        for (k, v) in std::env::vars() {
            if k.starts_with("DENDRO_") || k.starts_with("AGENT_OTEL_") || is_k_org_var_name(&k) {
                process_env.insert(k, v);
            }
        }
        let result = Self::from_sources(&process_env, private_data_root, consumer_env_file);
        for (_, mut val) in process_env {
            zeroize_string(&mut val);
        }
        result
    }

    /// Loads `SessionLinkConfig` from a map of process environment variables and an optional private data root.
    #[must_use]
    pub fn from_vars(vars: &BTreeMap<String, String>, private_data_root: Option<&Path>) -> Self {
        Self::from_sources(vars, private_data_root, None)
    }

    /// Loads `SessionLinkConfig` from explicit process environment variables, an optional private
    /// data root (`<private_data_root>/link.key`), and an optional `consumer.env` file path.
    ///
    /// Security and precedence rules:
    /// 1. Any `K_org` variable present anywhere (process env, `consumer.env`, or key file) refuses
    ///    link configuration (`key = None`, `unlinked_reason = invalid_key_material`) and emits a
    ///    one-time warning containing no key material.
    /// 2. Key precedence (highest first):
    ///    - (a) Process env `DENDRO_LINK_KEY_HEX` (`AGENT_OTEL_LINK_KEY_HEX`), or process env
    ///      `DENDRO_LINK_KEY_FILE` (`AGENT_OTEL_LINK_KEY_FILE`) which is accepted ONLY from the
    ///      process environment and MUST be an absolute path,
    ///    - (b) `<private_data_root>/link.key` inside Dendro's private data root,
    ///    - (c) `consumer.env` `DENDRO_LINK_KEY_HEX` (`AGENT_OTEL_LINK_KEY_HEX`). `DENDRO_LINK_KEY_FILE`
    ///      inside `consumer.env` is ignored.
    #[must_use]
    pub fn from_sources(
        process_env: &BTreeMap<String, String>,
        private_data_root: Option<&Path>,
        consumer_env_file: Option<&Path>,
    ) -> Self {
        let consumer_vars = consumer_env_file
            .and_then(read_consumer_env_from)
            .map(|mut content| {
                let parsed = parse_consumer_env_vars(&content);
                zeroize_string(&mut content);
                parsed
            })
            .unwrap_or_default();

        let org = pick_var_with_fallback(
            process_env,
            &consumer_vars,
            &["DENDRO_LINK_ORG", "AGENT_OTEL_LINK_ORG"],
        );
        let endpoint_scope = pick_var_with_fallback(
            process_env,
            &consumer_vars,
            &[
                "DENDRO_LINK_ENDPOINT_SCOPE",
                "AGENT_OTEL_LINK_ENDPOINT_SCOPE",
            ],
        );
        let team = pick_var_with_fallback(
            process_env,
            &consumer_vars,
            &["DENDRO_LINK_TEAM", "AGENT_OTEL_LINK_TEAM"],
        );
        let epoch = pick_var_with_fallback(
            process_env,
            &consumer_vars,
            &["DENDRO_LINK_EPOCH", "AGENT_OTEL_LINK_EPOCH"],
        );

        let mut k_org_rejected = process_env.keys().any(|k| is_k_org_var_name(k))
            || consumer_vars.keys().any(|k| is_k_org_var_name(k));

        let key = if k_org_rejected {
            None
        } else if let Some(mut raw_hex) = pick_var(
            process_env,
            &["DENDRO_LINK_KEY_HEX", "AGENT_OTEL_LINK_KEY_HEX"],
        ) {
            let parsed = LinkKey::from_hex(&raw_hex).ok();
            zeroize_string(&mut raw_hex);
            parsed
        } else if let Some(raw_file) = pick_var_raw(
            process_env,
            &["DENDRO_LINK_KEY_FILE", "AGENT_OTEL_LINK_KEY_FILE"],
        ) {
            let path = Path::new(raw_file.trim());
            if raw_file.trim().is_empty() || !path.is_absolute() {
                None
            } else {
                match load_link_key_file(path) {
                    Ok(k) => Some(k),
                    Err(KeyFileLoadError::KOrgPresent) => {
                        k_org_rejected = true;
                        None
                    }
                    Err(KeyFileLoadError::Invalid) => None,
                }
            }
        } else if let Some(root_path) = private_data_root
            .filter(|root| root.is_absolute())
            .filter(|root| std::fs::symlink_metadata(root.join("link.key")).is_ok())
        {
            match load_link_key_from_private_root(root_path) {
                Ok(k) => Some(k),
                Err(KeyFileLoadError::KOrgPresent) => {
                    k_org_rejected = true;
                    None
                }
                Err(KeyFileLoadError::Invalid) => None,
            }
        } else if let Some(mut raw_hex) = pick_var(
            &consumer_vars,
            &["DENDRO_LINK_KEY_HEX", "AGENT_OTEL_LINK_KEY_HEX"],
        ) {
            let parsed = LinkKey::from_hex(&raw_hex).ok();
            zeroize_string(&mut raw_hex);
            parsed
        } else {
            None
        };

        for (_, mut val) in consumer_vars {
            zeroize_string(&mut val);
        }

        if k_org_rejected {
            emit_k_org_warning_once();
        }

        Self {
            org,
            endpoint_scope,
            team,
            epoch,
            key,
            k_org_rejected,
        }
    }

    /// Returns `true` if a valid 32-byte `K_link` is loaded.
    #[must_use]
    pub const fn has_valid_key(&self) -> bool {
        self.key.is_some()
    }

    /// Returns `true` if link configuration was refused due to a `K_org` variable.
    #[must_use]
    pub const fn k_org_rejected(&self) -> bool {
        self.k_org_rejected
    }

    /// Returns the redacted one-time refusal warning when `k_org_rejected` is true.
    #[must_use]
    pub const fn k_org_warning(&self) -> Option<&'static str> {
        if self.k_org_rejected {
            Some(K_ORG_REFUSAL_WARNING)
        } else {
            None
        }
    }

    /// Evaluates `link-id-v1` for a session before sealing.
    #[must_use]
    pub fn evaluate_session_link(
        &self,
        raw_harness: Option<&str>,
        wire_client_id: Option<u32>,
        raw_native_session: Option<&str>,
        origin: Option<&str>,
    ) -> LinkIdOutcome {
        if self.k_org_rejected {
            return LinkIdOutcome::Unlinked {
                reason: UnlinkedReason::InvalidKeyMaterial,
            };
        }
        let Some(k_link) = self.key.as_ref() else {
            return LinkIdOutcome::Unlinked {
                reason: UnlinkedReason::InvalidKeyMaterial,
            };
        };

        let input = LinkIdInput {
            org: self.org.as_deref(),
            team: self.team.as_deref(),
            endpoint_scope: self.endpoint_scope.as_deref(),
            epoch: self.epoch.as_deref(),
            harness: raw_harness,
            wire_client_id,
            native_session: raw_native_session,
            origin,
        };

        derive_session_link_with_key(k_link, &input)
    }

    /// Convenience helper returning `(link_id, Option<UnlinkedReason>)` for a session before sealing.
    #[must_use]
    pub fn resolve_session_link(
        &self,
        raw_harness: Option<&str>,
        raw_native_session: Option<&str>,
        origin: Option<&str>,
    ) -> (String, Option<UnlinkedReason>) {
        self.evaluate_session_link(raw_harness, None, raw_native_session, origin)
            .into_parts()
    }
}

fn pick_var(vars: &BTreeMap<String, String>, names: &[&str]) -> Option<String> {
    for name in names {
        if let Some(val) = vars.get(*name) {
            if !val.is_empty() {
                return Some(val.clone());
            }
        }
    }
    None
}

fn pick_var_raw(vars: &BTreeMap<String, String>, names: &[&str]) -> Option<String> {
    for name in names {
        if let Some(val) = vars.get(*name) {
            return Some(val.clone());
        }
    }
    None
}

fn pick_var_with_fallback(
    primary: &BTreeMap<String, String>,
    fallback: &BTreeMap<String, String>,
    names: &[&str],
) -> Option<String> {
    pick_var(primary, names).or_else(|| pick_var(fallback, names))
}

enum KeyFileLoadError {
    Invalid,
    KOrgPresent,
}

fn load_link_key_from_private_root(root_path: &Path) -> Result<LinkKey, KeyFileLoadError> {
    #[cfg(any(windows, target_os = "linux"))]
    {
        let root = crate::platform_v1::open_existing_private_root(root_path)
            .map_err(|_| KeyFileLoadError::Invalid)?;
        let name = crate::platform_v1::parse_child_name(
            "link.key",
            crate::platform_v1::current_native_platform(),
        )
        .map_err(|_| KeyFileLoadError::Invalid)?;
        let verified = crate::platform_v1::open_existing_private_file(&root, &name)
            .map_err(|_| KeyFileLoadError::Invalid)?;
        if verified.metadata().size > MAX_CONSUMER_ENV_BYTES {
            return Err(KeyFileLoadError::Invalid);
        }
        let res = read_and_parse_link_key_handle(verified.into_file());
        drop(root);
        res
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        load_link_key_file(&root_path.join("link.key"))
    }
}

fn load_link_key_file(path: &Path) -> Result<LinkKey, KeyFileLoadError> {
    if !path.is_absolute() {
        return Err(KeyFileLoadError::Invalid);
    }
    #[cfg(windows)]
    {
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .ok_or(KeyFileLoadError::Invalid)?;
        let file_name = path.file_name().ok_or(KeyFileLoadError::Invalid)?;
        let child = crate::platform_v1::parse_child_os(
            file_name,
            crate::platform_v1::NativePlatform::Windows,
        )
        .map_err(|_| KeyFileLoadError::Invalid)?;
        if let Ok(root) = crate::platform_v1::open_existing_private_root(parent) {
            let verified = crate::platform_v1::open_existing_private_file(&root, &child)
                .map_err(|_| KeyFileLoadError::Invalid)?;
            if verified.metadata().size > MAX_CONSUMER_ENV_BYTES {
                return Err(KeyFileLoadError::Invalid);
            }
            let res = read_and_parse_link_key_handle(verified.into_file());
            drop(root);
            return res;
        }
        // When an explicit DENDRO_LINK_KEY_FILE resides in a directory whose DACL is not
        // `SE_DACL_PROTECTED` (so `open_existing_private_root` refuses the parent), `platform_v1`
        // still enforces no-follow reparse rejection, regular-file kind, `links == 1` hard-link
        // rejection, size cap, and 128-bit `FILE_ID_INFO` handle identity matching. Full DACL/owner
        // evaluation and ancestor pinning require placing the key file in a `SE_DACL_PROTECTED` root.
        let no_follow =
            crate::platform_v1::reopen_metadata(path).map_err(|_| KeyFileLoadError::Invalid)?;
        if no_follow.reparse
            || no_follow.kind != crate::platform_v1::ObjectKind::RegularFile
            || no_follow.size > MAX_CONSUMER_ENV_BYTES
            || !crate::platform_v1::private_regular_links(no_follow.kind, no_follow.links)
                .is_allow()
        {
            return Err(KeyFileLoadError::Invalid);
        }
        let file = std::fs::File::open(path).map_err(|_| KeyFileLoadError::Invalid)?;
        let held = crate::platform_v1::metadata_from_handle(&file)
            .map_err(|_| KeyFileLoadError::Invalid)?;
        if held.reparse
            || held.kind != crate::platform_v1::ObjectKind::RegularFile
            || held.size > MAX_CONSUMER_ENV_BYTES
            || !crate::platform_v1::private_regular_links(held.kind, held.links).is_allow()
            || held.identity != no_follow.identity
        {
            return Err(KeyFileLoadError::Invalid);
        }
        read_and_parse_link_key_handle(file)
    }
    #[cfg(not(windows))]
    {
        let sym = std::fs::symlink_metadata(path).map_err(|_| KeyFileLoadError::Invalid)?;
        if !sym.file_type().is_file() || sym.len() > MAX_CONSUMER_ENV_BYTES {
            return Err(KeyFileLoadError::Invalid);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if sym.permissions().mode() & 0o077 != 0 {
                return Err(KeyFileLoadError::Invalid);
            }
        }
        let file = std::fs::File::open(path).map_err(|_| KeyFileLoadError::Invalid)?;
        let held = file.metadata().map_err(|_| KeyFileLoadError::Invalid)?;
        if !held.is_file() || held.len() > MAX_CONSUMER_ENV_BYTES {
            return Err(KeyFileLoadError::Invalid);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            if held.permissions().mode() & 0o077 != 0
                || held.nlink() != 1
                || sym.dev() != held.dev()
                || sym.ino() != held.ino()
            {
                return Err(KeyFileLoadError::Invalid);
            }
        }
        read_and_parse_link_key_handle(file)
    }
}

fn read_and_parse_link_key_handle(file: std::fs::File) -> Result<LinkKey, KeyFileLoadError> {
    use std::io::Read;
    let mut raw = String::new();
    file.take(MAX_CONSUMER_ENV_BYTES + 1)
        .read_to_string(&mut raw)
        .map_err(|_| KeyFileLoadError::Invalid)?;
    if raw.len() as u64 > MAX_CONSUMER_ENV_BYTES {
        zeroize_string(&mut raw);
        return Err(KeyFileLoadError::Invalid);
    }
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some((k, _)) = trimmed.split_once('=') {
            if is_k_org_var_name(k) {
                zeroize_string(&mut raw);
                return Err(KeyFileLoadError::KOrgPresent);
            }
        }
    }
    let parsed = LinkKey::from_hex(&raw).map_err(|_| KeyFileLoadError::Invalid);
    zeroize_string(&mut raw);
    parsed
}

fn zeroize_string(value: &mut String) {
    // Overwrite the UTF-8 buffer with ASCII '0' bytes in safe Rust before clearing.
    let len = value.len();
    if len > 0 {
        value.replace_range(.., &"0".repeat(len));
        std::hint::black_box(value.as_str());
        value.clear();
    }
}

fn parse_consumer_env_vars(content: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some((raw_key, value)) = trimmed.split_once('=') {
            let key = strip_export_prefix(raw_key);
            let value = value.trim().trim_matches('"').trim_matches('\'');
            if key == "DENDRO_LINK_KEY_FILE" || key == "AGENT_OTEL_LINK_KEY_FILE" {
                // Key-file paths are accepted ONLY from the process environment, never consumer.env.
                continue;
            }
            if key.starts_with("DENDRO_")
                || key.starts_with("AGENT_OTEL_")
                || is_k_org_var_name(key)
            {
                out.insert(key.to_string(), value.to_string());
            }
        }
    }
    out
}

/// Explicit admin-only provisioning module for deriving endpoint `K_link` keys from `K_org`.
///
/// Never call this module on the ingest or endpoint path.
pub mod admin {
    use super::{
        derive_session_link_with_key, hmac_sha256, is_valid_ascii_slug, lp_encode_str, LinkIdInput,
        LinkIdOutcome, LinkKey, UnlinkedReason, LINK_KEY_BYTES, LINK_KEY_HKDF_SALT,
        LINK_KEY_PURPOSE, MIN_ORG_MASTER_KEY_BYTES,
    };
    use std::fmt;

    /// Zeroize-on-drop wrapper around an organization master key (`K_org`) used strictly in admin
    /// provisioning and contract vector checks.
    pub struct AdminOrgMasterKey(Vec<u8>);

    impl AdminOrgMasterKey {
        /// Wraps raw `K_org` bytes, rejecting keys shorter than 32 bytes (`MIN_ORG_MASTER_KEY_BYTES`).
        pub fn from_slice(bytes: &[u8]) -> Result<Self, UnlinkedReason> {
            if bytes.len() < MIN_ORG_MASTER_KEY_BYTES {
                return Err(UnlinkedReason::InvalidKeyMaterial);
            }
            Ok(Self(bytes.to_vec()))
        }

        /// Parses hex-encoded `K_org` bytes, rejecting invalid hex or keys shorter than 32 bytes.
        pub fn from_hex(hex_str: &str) -> Result<Self, UnlinkedReason> {
            let trimmed = hex_str.trim();
            if trimmed.is_empty() {
                return Err(UnlinkedReason::InvalidKeyMaterial);
            }
            let mut decoded =
                hex::decode(trimmed).map_err(|_| UnlinkedReason::InvalidKeyMaterial)?;
            let res = Self::from_slice(&decoded);
            decoded.fill(0);
            std::hint::black_box(&mut decoded);
            res
        }
    }

    impl fmt::Debug for AdminOrgMasterKey {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("AdminOrgMasterKey([REDACTED])")
        }
    }

    impl Drop for AdminOrgMasterKey {
        fn drop(&mut self) {
            self.0.fill(0);
            std::hint::black_box(&mut self.0);
        }
    }

    /// RFC 5869 `HKDF-Extract(salt, IKM)` with `HMAC-SHA256`.
    #[must_use]
    pub fn hkdf_extract_sha256(salt: &[u8], ikm: &[u8]) -> [u8; 32] {
        hmac_sha256(salt, ikm)
    }

    /// RFC 5869 `HKDF-Expand(PRK, info, L=32)` with `HMAC-SHA256`.
    #[must_use]
    pub fn hkdf_expand_sha256_32(prk: &[u8; 32], info: &[u8]) -> [u8; LINK_KEY_BYTES] {
        let mut t1_input = Vec::with_capacity(info.len() + 1);
        t1_input.extend_from_slice(info);
        t1_input.push(0x01);
        hmac_sha256(prk, &t1_input)
    }

    /// RFC 5869 `HKDF-SHA256(salt, IKM, info, L=32)`.
    #[must_use]
    pub fn hkdf_sha256_32(salt: &[u8], ikm: &[u8], info: &[u8]) -> [u8; LINK_KEY_BYTES] {
        let mut prk = hkdf_extract_sha256(salt, ikm);
        let okm = hkdf_expand_sha256_32(&prk, info);
        prk.fill(0);
        std::hint::black_box(&mut prk);
        okm
    }

    /// Encodes the normative HKDF `info` parameter `LP("agent.session.link.key/v1", team, epoch)`.
    pub fn encode_hkdf_info(
        team: Option<&str>,
        epoch: Option<&str>,
    ) -> Result<Vec<u8>, UnlinkedReason> {
        let team_str = match team {
            Some(v) if is_valid_ascii_slug(v) => v,
            _ => return Err(UnlinkedReason::InvalidKeyMaterial),
        };
        let epoch_str = match epoch {
            Some(v) if is_valid_ascii_slug(v) => v,
            _ => return Err(UnlinkedReason::InvalidKeyMaterial),
        };

        let mut info = Vec::with_capacity(
            4 + LINK_KEY_PURPOSE.len() + 4 + team_str.len() + 4 + epoch_str.len(),
        );
        info.extend_from_slice(&lp_encode_str(LINK_KEY_PURPOSE));
        info.extend_from_slice(&lp_encode_str(team_str));
        info.extend_from_slice(&lp_encode_str(epoch_str));
        Ok(info)
    }

    /// Derives an endpoint-scoped `K_link` ([`LinkKey`]) from `K_org`, `team`, and `epoch` per `SPEC.md` §3.
    pub fn derive_link_key(
        k_org: &AdminOrgMasterKey,
        team: Option<&str>,
        epoch: Option<&str>,
    ) -> Result<LinkKey, UnlinkedReason> {
        let info = encode_hkdf_info(team, epoch)?;
        let mut okm = hkdf_sha256_32(LINK_KEY_HKDF_SALT, &k_org.0, &info);
        let key = LinkKey::from_bytes(okm);
        okm.fill(0);
        std::hint::black_box(&mut okm);
        Ok(key)
    }

    /// Admin/vector helper that derives `K_link` from `K_org` and evaluates `link-id-v1`.
    #[must_use]
    pub fn derive_session_link_id(k_org_bytes: &[u8], input: &LinkIdInput<'_>) -> LinkIdOutcome {
        let k_org = match AdminOrgMasterKey::from_slice(k_org_bytes) {
            Ok(k) => k,
            Err(reason) => return LinkIdOutcome::Unlinked { reason },
        };
        let k_link = match derive_link_key(&k_org, input.team, input.epoch) {
            Ok(k) => k,
            Err(reason) => return LinkIdOutcome::Unlinked { reason },
        };
        derive_session_link_with_key(&k_link, input)
    }

    /// Admin/vector helper that evaluates candidate `link_id`s across an ordered epoch ring.
    pub fn derive_session_link_candidates_across_epochs(
        k_org_bytes: &[u8],
        base_input: &LinkIdInput<'_>,
        epochs: &[&str],
    ) -> Result<Vec<(String, String)>, UnlinkedReason> {
        let k_org = AdminOrgMasterKey::from_slice(k_org_bytes)?;
        let mut out = Vec::with_capacity(epochs.len());
        for &epoch in epochs {
            let mut input = base_input.clone();
            input.epoch = Some(epoch);
            let k_link = derive_link_key(&k_org, input.team, input.epoch)?;
            match derive_session_link_with_key(&k_link, &input) {
                LinkIdOutcome::Linked { link_id, .. } => out.push((epoch.to_string(), link_id)),
                LinkIdOutcome::Unlinked { reason } => return Err(reason),
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::admin::{
        derive_link_key, derive_session_link_candidates_across_epochs, derive_session_link_id,
        encode_hkdf_info, hkdf_extract_sha256, AdminOrgMasterKey,
    };
    use super::*;
    use serde::Deserialize;
    use sha2::Digest;
    use std::collections::{BTreeMap, BTreeSet};

    fn contract_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("workspace root")
            .join("docs")
            .join("contracts")
            .join("link-id-v1")
    }

    fn sha256_bytes_hex(bytes: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        hex::encode(hasher.finalize())
    }

    fn compute_bundle_digest(files: &[(String, Vec<u8>)]) -> (String, String) {
        let mut manifest = String::new();
        for (rel, content) in files {
            assert!(
                !content.contains(&b'\r'),
                "CRLF detected in vendored contract file {rel}"
            );
            let file_digest = sha256_bytes_hex(content);
            manifest.push_str(&format!("{file_digest}  {rel}\n"));
        }
        let bundle_digest = sha256_bytes_hex(manifest.as_bytes());
        (manifest, bundle_digest)
    }

    #[test]
    fn vendored_contract_digest_matches_pinned_bundle_and_detects_drift() {
        let root = contract_root();
        let vector_names = [
            "basic.json",
            "collisions.json",
            "g0a.json",
            "negative.json",
            "rotation.json",
        ];

        // 1. Verify vectors/SHA256SUMS
        let mut expected_sums = String::new();
        for name in vector_names {
            let bytes = std::fs::read(root.join("vectors").join(name)).expect("read vector file");
            assert!(!bytes.contains(&b'\r'), "CRLF in vectors/{name}");
            expected_sums.push_str(&format!("{}  {}\n", sha256_bytes_hex(&bytes), name));
        }
        let actual_sums =
            std::fs::read_to_string(root.join("vectors").join("SHA256SUMS")).expect("SHA256SUMS");
        assert_eq!(actual_sums, expected_sums, "vectors/SHA256SUMS mismatch");

        // 2. Recompute CONTRACT-DIGEST in normative manifest order
        let mut bundle_files = Vec::new();
        for rel in [
            "SPEC.md",
            "vocabulary.json",
            "vectors/SHA256SUMS",
            "vectors/basic.json",
            "vectors/collisions.json",
            "vectors/g0a.json",
            "vectors/negative.json",
            "vectors/rotation.json",
        ] {
            let bytes = std::fs::read(root.join(rel)).expect("read contract file");
            bundle_files.push((rel.to_string(), bytes));
        }

        let (manifest, bundle_digest) = compute_bundle_digest(&bundle_files);
        assert_eq!(bundle_digest, LINK_ID_CONTRACT_DIGEST);

        let digest_file =
            std::fs::read_to_string(root.join("CONTRACT-DIGEST")).expect("read CONTRACT-DIGEST");
        let expected_digest_file = format!(
            "# link-id-v1 Contract Digest Manifest (Apache-2.0)\n# Format: <sha256>  <relative-path>\n{manifest}# BUNDLE-SHA256: {bundle_digest}\n"
        );
        assert_eq!(digest_file, expected_digest_file);

        // Verify vendored LICENSE exists alongside the contract bundle without altering CONTRACT-DIGEST
        // and matches AOB `origin/main:LICENSE` byte-for-byte.
        let license_bytes =
            std::fs::read(root.join("LICENSE")).expect("read vendored contract LICENSE");
        assert!(!license_bytes.contains(&b'\r'), "CRLF in vendored LICENSE");
        assert_eq!(
            sha256_bytes_hex(&license_bytes),
            "b67a6ec8a9785d251a91a571a9aa22ed030a4a7b5ad5ada20c9386a095849e88",
            "vendored LICENSE must match AOB origin/main:LICENSE SHA-256"
        );
        let license_text = std::str::from_utf8(&license_bytes).expect("utf-8 LICENSE");
        assert!(license_text.contains("Apache License"));
        assert!(license_text.contains("Version 2.0, January 2004"));

        // 3. Tamper test: 1-byte mutation in SPEC.md must fail digest check
        let mut tampered = bundle_files.clone();
        tampered[0].1.push(b' ');
        let (_, tampered_digest) = compute_bundle_digest(&tampered);
        assert_ne!(tampered_digest, LINK_ID_CONTRACT_DIGEST);
    }

    #[derive(Debug, Deserialize)]
    struct VectorInputs {
        k_org_hex: Option<String>,
        salt_hex: Option<String>,
        org: Option<String>,
        team: Option<String>,
        endpoint_scope: Option<String>,
        epoch: Option<String>,
        raw_harness: Option<String>,
        wire_client_id: Option<u32>,
        native_session: Option<String>,
        origin: Option<String>,
    }

    impl VectorInputs {
        fn as_link_input(&self) -> LinkIdInput<'_> {
            LinkIdInput {
                org: self.org.as_deref(),
                team: self.team.as_deref(),
                endpoint_scope: self.endpoint_scope.as_deref(),
                epoch: self.epoch.as_deref(),
                harness: self.raw_harness.as_deref(),
                wire_client_id: self.wire_client_id,
                native_session: self.native_session.as_deref(),
                origin: self.origin.as_deref(),
            }
        }
    }

    #[derive(Debug, Deserialize)]
    struct VectorIntermediates {
        hkdf_info_hex: Option<String>,
        prk_hex: Option<String>,
        k_link_hex: Option<String>,
        canonical_harness: Option<String>,
        lp_payload_hex: Option<String>,
    }

    #[derive(Debug, Deserialize)]
    struct VectorExpected {
        link_id: String,
        unlinked_reason: Option<String>,
    }

    #[derive(Debug, Deserialize)]
    struct VectorCase {
        id: String,
        inputs: VectorInputs,
        intermediates: Option<VectorIntermediates>,
        expected: VectorExpected,
    }

    #[derive(Debug, Deserialize)]
    struct OverlapCandidate {
        epoch: String,
        link_id: String,
    }

    #[derive(Debug, Deserialize)]
    struct OverlapWindowExample {
        active_epochs: Vec<String>,
        expected_candidates: Vec<OverlapCandidate>,
    }

    #[derive(Debug, Deserialize)]
    struct SuiteFile {
        vectors: Vec<VectorCase>,
        #[serde(default)]
        assert_distinct_link_id_groups: Vec<Vec<String>>,
        #[serde(default)]
        assert_equal_link_id_groups: Vec<Vec<String>>,
        #[serde(default)]
        assert_distinct_k_link_groups: Vec<Vec<String>>,
        overlap_window_example: Option<OverlapWindowExample>,
    }

    fn verify_case(case: &VectorCase) -> (String, Option<String>) {
        if let Some(salt_hex) = case.inputs.salt_hex.as_deref() {
            assert_eq!(
                hex::decode(salt_hex).unwrap(),
                LINK_KEY_HKDF_SALT,
                "vector {} salt mismatch",
                case.id
            );
        }

        let raw_k_org = case.inputs.k_org_hex.as_deref().unwrap_or("");
        let k_org_bytes = hex::decode(raw_k_org).ok();
        let link_input = case.inputs.as_link_input();

        let outcome = match k_org_bytes.as_deref() {
            Some(bytes) if !raw_k_org.is_empty() => derive_session_link_id(bytes, &link_input),
            _ => LinkIdOutcome::Unlinked {
                reason: UnlinkedReason::InvalidKeyMaterial,
            },
        };

        assert_eq!(
            outcome.link_id(),
            case.expected.link_id,
            "vector {} link_id mismatch",
            case.id
        );
        assert_eq!(
            outcome.unlinked_reason().map(UnlinkedReason::as_str),
            case.expected.unlinked_reason.as_deref(),
            "vector {} unlinked_reason mismatch",
            case.id
        );

        let mut derived_k_link_hex = None;
        if let Some(bytes) = k_org_bytes.as_deref() {
            if let Ok(k_org) = AdminOrgMasterKey::from_slice(bytes) {
                if let Ok(info) = encode_hkdf_info(link_input.team, link_input.epoch) {
                    let prk = hkdf_extract_sha256(LINK_KEY_HKDF_SALT, bytes);
                    let k_link =
                        derive_link_key(&k_org, link_input.team, link_input.epoch).unwrap();
                    if let Some(inter) = &case.intermediates {
                        if let Some(expected_info) = inter.hkdf_info_hex.as_deref() {
                            assert_eq!(
                                hex::encode(&info),
                                expected_info,
                                "vector {} hkdf_info mismatch",
                                case.id
                            );
                        }
                        if let Some(expected_prk) = inter.prk_hex.as_deref() {
                            assert_eq!(
                                hex::encode(prk),
                                expected_prk,
                                "vector {} prk mismatch",
                                case.id
                            );
                        }
                        if let Some(expected_k_link) = inter.k_link_hex.as_deref() {
                            assert_eq!(
                                k_link.to_hex(),
                                expected_k_link,
                                "vector {} k_link mismatch",
                                case.id
                            );
                        }
                    }
                    derived_k_link_hex = Some(k_link.to_hex());
                }
            }
        }

        match &outcome {
            LinkIdOutcome::Linked {
                link_id,
                canonical_harness,
            } => {
                if let Some(inter) = &case.intermediates {
                    if let Some(expected_h) = inter.canonical_harness.as_deref() {
                        assert_eq!(
                            canonical_harness.as_str(),
                            expected_h,
                            "vector {} canonical_harness mismatch",
                            case.id
                        );
                    }
                    if let Some(expected_lp) = inter.lp_payload_hex.as_deref() {
                        let mut lp = Vec::new();
                        lp.extend_from_slice(&lp_encode_str(LINK_ID_DOMAIN));
                        lp.extend_from_slice(&lp_encode_str(link_input.org.unwrap()));
                        lp.extend_from_slice(&lp_encode_str(link_input.endpoint_scope.unwrap()));
                        lp.extend_from_slice(&lp_encode_str(canonical_harness.as_str()));
                        lp.extend_from_slice(&lp_encode_str(link_input.native_session.unwrap()));
                        assert_eq!(
                            hex::encode(lp),
                            expected_lp,
                            "vector {} lp_payload_hex mismatch",
                            case.id
                        );
                    }
                }

                let k_org = AdminOrgMasterKey::from_hex(raw_k_org).expect("valid k_org");
                let k_link = derive_link_key(&k_org, link_input.team, link_input.epoch)
                    .expect("derive k_link");

                // Also verify the endpoint SessionLinkConfig path produces the exact same link_id
                let cfg = SessionLinkConfig::from_link_key(
                    k_link,
                    link_input.org.unwrap_or_default(),
                    link_input.endpoint_scope.unwrap_or_default(),
                    link_input.team.map(ToOwned::to_owned),
                    link_input.epoch.map(ToOwned::to_owned),
                );
                let endpoint_outcome = cfg.evaluate_session_link(
                    link_input.harness,
                    link_input.wire_client_id,
                    link_input.native_session,
                    link_input.origin,
                );
                assert_eq!(
                    endpoint_outcome.link_id(),
                    link_id.as_str(),
                    "vector {} SessionLinkConfig mismatch",
                    case.id
                );
            }
            LinkIdOutcome::Unlinked { reason } => {
                assert_eq!(case.expected.link_id, AGENT_SESSION_LINK_UNLINKED);
                let key_hex = derived_k_link_hex.as_deref().unwrap_or(raw_k_org);
                let cfg = SessionLinkConfig::from_key_hex(
                    key_hex,
                    link_input.org,
                    link_input.endpoint_scope,
                    link_input.team,
                    link_input.epoch,
                );
                let endpoint_outcome = cfg.evaluate_session_link(
                    link_input.harness,
                    link_input.wire_client_id,
                    link_input.native_session,
                    link_input.origin,
                );
                assert_eq!(
                    endpoint_outcome,
                    LinkIdOutcome::Unlinked { reason: *reason },
                    "vector {} SessionLinkConfig unlinked mismatch",
                    case.id
                );
            }
        }

        (outcome.link_id().to_string(), derived_k_link_hex)
    }

    fn verify_suite(suite: &SuiteFile, suite_name: &str) {
        assert!(!suite.vectors.is_empty(), "{suite_name} has no vectors");
        let mut link_by_id = BTreeMap::new();
        let mut k_link_by_id = BTreeMap::new();
        for case in &suite.vectors {
            let (lid, k_link) = verify_case(case);
            link_by_id.insert(case.id.clone(), lid);
            if let Some(kl) = k_link {
                k_link_by_id.insert(case.id.clone(), kl);
            }
        }
        for group in &suite.assert_distinct_link_id_groups {
            let mut seen = BTreeSet::new();
            for vid in group {
                let lid = link_by_id.get(vid).expect("vector id");
                assert!(
                    seen.insert(lid.clone()),
                    "collision in {suite_name} distinct link_id group: {vid}"
                );
            }
        }
        for group in &suite.assert_equal_link_id_groups {
            let first = link_by_id.get(&group[0]).expect("first vector id");
            for vid in &group[1..] {
                let other = link_by_id.get(vid).expect("vector id");
                assert_eq!(
                    first, other,
                    "mismatch in {suite_name} equal link_id group: {} vs {}",
                    group[0], vid
                );
            }
        }
        for group in &suite.assert_distinct_k_link_groups {
            let mut seen = BTreeSet::new();
            for vid in group {
                let kl = k_link_by_id.get(vid).expect("k_link vector id");
                assert!(
                    seen.insert(kl.clone()),
                    "collision in {suite_name} distinct k_link group: {vid}"
                );
            }
        }
    }

    #[test]
    fn all_contract_vectors_pass_byte_exactly() {
        let root = contract_root().join("vectors");

        for name in [
            "basic.json",
            "collisions.json",
            "rotation.json",
            "negative.json",
            "g0a.json",
        ] {
            let suite: SuiteFile =
                serde_json::from_slice(&std::fs::read(root.join(name)).unwrap()).unwrap();
            verify_suite(&suite, name);

            if let Some(overlap) = &suite.overlap_window_example {
                let first_case = &suite.vectors[0];
                let k_org_bytes =
                    hex::decode(first_case.inputs.k_org_hex.as_deref().unwrap()).unwrap();
                let base = first_case.inputs.as_link_input();
                let epoch_refs: Vec<&str> =
                    overlap.active_epochs.iter().map(String::as_str).collect();
                let candidates =
                    derive_session_link_candidates_across_epochs(&k_org_bytes, &base, &epoch_refs)
                        .expect("overlap candidates");
                assert_eq!(candidates.len(), overlap.expected_candidates.len());
                for ((epoch, link_id), expected) in
                    candidates.iter().zip(overlap.expected_candidates.iter())
                {
                    assert_eq!(epoch, &expected.epoch);
                    assert_eq!(link_id, &expected.link_id);
                }
            }
        }
    }

    #[test]
    fn endpoint_config_rejects_k_org_and_short_keys_and_handles_unconfigured() {
        // 1. Unconfigured -> unlinked (invalid_key_material)
        let unconfigured = SessionLinkConfig::unconfigured();
        assert!(!unconfigured.has_valid_key());
        assert_eq!(
            unconfigured.resolve_session_link(Some("claude"), Some("sess-123"), None),
            (
                AGENT_SESSION_LINK_UNLINKED.to_string(),
                Some(UnlinkedReason::InvalidKeyMaterial)
            )
        );

        // 2. Short key (< 32 bytes) -> unlinked (invalid_key_material)
        let short_cfg = SessionLinkConfig::from_key_slice(
            &[0x42; 31],
            Some("acme"),
            Some("macbook-01"),
            None,
            None,
        );
        assert!(!short_cfg.has_valid_key());
        assert_eq!(
            short_cfg.resolve_session_link(Some("claude"), Some("sess-123"), None),
            (
                AGENT_SESSION_LINK_UNLINKED.to_string(),
                Some(UnlinkedReason::InvalidKeyMaterial)
            )
        );

        // 3. Forbidden K_org env var in vars -> refuses key even if K_link is also supplied,
        //    and emits a one-time warning that contains no key material.
        let secret_k_org = "deadbeefcafebabe00112233445566778899aabbccddeeff0011223344556677";
        let secret_k_link = "11".repeat(32);
        let mut vars = BTreeMap::new();
        vars.insert("DENDRO_LINK_ORG".to_string(), "acme".to_string());
        vars.insert(
            "DENDRO_LINK_ENDPOINT_SCOPE".to_string(),
            "macbook-01".to_string(),
        );
        vars.insert("DENDRO_LINK_KEY_HEX".to_string(), secret_k_link.clone());
        vars.insert(
            "DENDRO_LINK_K_ORG_HEX".to_string(),
            secret_k_org.to_string(),
        );
        let rejected_cfg = SessionLinkConfig::from_vars(&vars, None);
        assert!(!rejected_cfg.has_valid_key());
        assert!(rejected_cfg.k_org_rejected());
        assert!(K_ORG_WARNING_EMITTED.load(std::sync::atomic::Ordering::SeqCst));
        let warning = rejected_cfg.k_org_warning().expect("k_org warning");
        assert!(!warning.contains(secret_k_org));
        assert!(!warning.contains(&secret_k_link));
        assert_eq!(
            rejected_cfg.resolve_session_link(Some("claude"), Some("sess-123"), None),
            (
                AGENT_SESSION_LINK_UNLINKED.to_string(),
                Some(UnlinkedReason::InvalidKeyMaterial)
            )
        );

        // 4. Origin "Harness" (capital H) -> unlinked (non_harness_origin)
        let valid_cfg = SessionLinkConfig::from_key_hex(
            &"11".repeat(32),
            Some("acme"),
            Some("macbook-01"),
            None,
            None,
        );
        assert!(valid_cfg.has_valid_key());
        assert_eq!(
            valid_cfg.resolve_session_link(Some("claude"), Some("sess-123"), Some("Harness")),
            (
                AGENT_SESSION_LINK_UNLINKED.to_string(),
                Some(UnlinkedReason::NonHarnessOrigin)
            )
        );

        // 5. Absent origin (None) and exact "harness" produce identical linked digests
        let (lid_none, reason_none) =
            valid_cfg.resolve_session_link(Some("claude"), Some("sess-123"), None);
        let (lid_harness, reason_harness) =
            valid_cfg.resolve_session_link(Some("claude"), Some("sess-123"), Some("harness"));
        assert!(reason_none.is_none());
        assert!(reason_harness.is_none());
        assert_eq!(lid_none, lid_harness);
        assert!(is_valid_persisted_link_id(&lid_none));

        // 6. LinkKey constant-time equality works as expected
        let k1 = LinkKey::from_bytes([0xAA; 32]);
        let k2 = LinkKey::from_bytes([0xAA; 32]);
        let k3 = LinkKey::from_bytes([0xAB; 32]);
        assert_eq!(k1, k2);
        assert_ne!(k1, k3);
    }

    #[cfg(not(any(windows, target_os = "linux")))]
    fn write_private_key_file(path: &Path, content: &str) {
        std::fs::write(path, content).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
    }

    #[test]
    fn config_precedence_and_cwd_isolation_and_key_file_rules() {
        #[cfg(any(windows, target_os = "linux"))]
        let dir = {
            let root = crate::platform_v1::create_user_private_root_with_choice(
                "dendro-link-cfg-",
                &crate::platform_v1::StorageRootChoice::UserProfile,
            )
            .unwrap();
            root.path().to_path_buf()
        };
        #[cfg(not(any(windows, target_os = "linux")))]
        let dir = {
            let d = std::env::temp_dir().join(format!(
                "dendro-link-cfg-test-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&d).unwrap();
            d
        };

        let key_env_hex = "11".repeat(32);
        let key_file_hex = "22".repeat(32);
        let key_root_hex = "33".repeat(32);
        let key_consumer_hex = "44".repeat(32);

        let expected_for = |k_hex: &str| -> String {
            SessionLinkConfig::from_key_hex(k_hex, Some("acme"), Some("ws-01"), None, None)
                .resolve_session_link(Some("claude"), Some("sess-1"), None)
                .0
        };

        // 1. CWD isolation holds by construction (`consumer_env_path`, `default_private_data_root`,
        //    `read_consumer_env_from`, and `load_link_key_file` require `path.is_absolute()` and
        //    never consult `std::env::current_dir()`). Verify that relative XDG_CONFIG_HOME / HOME
        //    paths pointing at a populated `observability/consumer.env` resolve to `None` and are ignored.
        let cwd_obs_dir = dir.join("cwd_sim").join("observability");
        std::fs::create_dir_all(&cwd_obs_dir).unwrap();
        let cwd_consumer = cwd_obs_dir.join("consumer.env");
        std::fs::write(
            &cwd_consumer,
            format!(
                "DENDRO_LINK_ORG=acme\nDENDRO_LINK_ENDPOINT_SCOPE=ws-01\nDENDRO_LINK_KEY_HEX={key_consumer_hex}\n"
            ),
        )
        .unwrap();
        assert!(
            read_consumer_env_from(Path::new("observability/consumer.env")).is_none(),
            "relative consumer.env path must always be refused"
        );
        for (rel_xdg, rel_home) in [
            (Some(std::ffi::OsStr::new("cwd_sim")), None),
            (Some(std::ffi::OsStr::new(".")), None),
            (None, Some(std::ffi::OsStr::new("cwd_sim"))),
            (None, Some(std::ffi::OsStr::new("."))),
        ] {
            let resolved_consumer = consumer_env_path_from_vars(rel_xdg, rel_home);
            assert!(
                resolved_consumer.is_none(),
                "relative XDG_CONFIG_HOME/HOME must resolve to None"
            );
            let rel_cfg = SessionLinkConfig::from_sources(
                &BTreeMap::new(),
                None,
                resolved_consumer.as_deref(),
            );
            assert!(
                !rel_cfg.has_valid_key(),
                "relative XDG_CONFIG_HOME/HOME must never load consumer.env"
            );
        }
        let rel_consumer_cfg = SessionLinkConfig::from_sources(
            &BTreeMap::new(),
            None,
            Some(Path::new("observability/consumer.env")),
        );
        assert!(
            !rel_consumer_cfg.has_valid_key(),
            "relative observability/consumer.env must be ignored"
        );

        // Prepare private data root `<private_data_root>/link.key` and explicit key file via `platform_v1`
        #[cfg(any(windows, target_os = "linux"))]
        let (private_root, explicit_key_path, empty_root) = {
            use std::io::Write;
            let parent_root = crate::platform_v1::open_existing_private_root(&dir).unwrap();
            let priv_cap =
                crate::platform_v1::create_unique_private_directory(&parent_root, "priv-root-")
                    .unwrap();
            let link_child = crate::platform_v1::parse_child_name(
                "link.key",
                crate::platform_v1::current_native_platform(),
            )
            .unwrap();
            let mut vf = priv_cap.create_file(&link_child).unwrap();
            vf.file_mut()
                .write_all(format!("{key_root_hex}\n").as_bytes())
                .unwrap();
            drop(vf);
            let priv_path = priv_cap.path().to_path_buf();
            drop(priv_cap);

            let exp_child = crate::platform_v1::parse_child_name(
                "explicit_link.key",
                crate::platform_v1::current_native_platform(),
            )
            .unwrap();
            let mut vf_exp = parent_root.create_file(&exp_child).unwrap();
            vf_exp
                .file_mut()
                .write_all(format!("{key_file_hex}\n").as_bytes())
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
            let root_key_path = priv_path.join("link.key");
            write_private_key_file(&root_key_path, &format!("{key_root_hex}\n"));
            let exp_path = dir.join("explicit_link.key");
            write_private_key_file(&exp_path, &format!("{key_file_hex}\n"));
            let empty_path = dir.join("empty_private_root");
            std::fs::create_dir_all(&empty_path).unwrap();
            (priv_path, exp_path, empty_path)
        };

        let xdg_consumer = dir.join("xdg").join("observability").join("consumer.env");
        std::fs::create_dir_all(xdg_consumer.parent().unwrap()).unwrap();
        std::fs::write(
            &xdg_consumer,
            format!(
                "DENDRO_LINK_ORG=acme\nDENDRO_LINK_ENDPOINT_SCOPE=ws-01\nDENDRO_LINK_KEY_HEX={key_consumer_hex}\n"
            ),
        )
        .unwrap();

        // 2. Precedence:
        //    (a) process env DENDRO_LINK_KEY_HEX beats private_root/link.key and consumer.env
        let mut env_a = BTreeMap::new();
        env_a.insert("DENDRO_LINK_KEY_HEX".to_string(), key_env_hex.clone());
        let cfg_a =
            SessionLinkConfig::from_sources(&env_a, Some(&private_root), Some(&xdg_consumer));
        assert_eq!(
            cfg_a
                .resolve_session_link(Some("claude"), Some("sess-1"), None)
                .0,
            expected_for(&key_env_hex)
        );

        //    Process env DENDRO_LINK_KEY_HEX also beats process env DENDRO_LINK_KEY_FILE
        let mut env_a_and_file = env_a;
        env_a_and_file.insert(
            "DENDRO_LINK_KEY_FILE".to_string(),
            explicit_key_path.display().to_string(),
        );
        let cfg_a_file = SessionLinkConfig::from_sources(
            &env_a_and_file,
            Some(&private_root),
            Some(&xdg_consumer),
        );
        assert_eq!(
            cfg_a_file
                .resolve_session_link(Some("claude"), Some("sess-1"), None)
                .0,
            expected_for(&key_env_hex)
        );

        //    Process env DENDRO_LINK_KEY_FILE beats private_root/link.key and consumer.env
        let mut env_only_file = BTreeMap::new();
        env_only_file.insert(
            "DENDRO_LINK_KEY_FILE".to_string(),
            explicit_key_path.display().to_string(),
        );
        let cfg_env_file = SessionLinkConfig::from_sources(
            &env_only_file,
            Some(&private_root),
            Some(&xdg_consumer),
        );
        assert_eq!(
            cfg_env_file
                .resolve_session_link(Some("claude"), Some("sess-1"), None)
                .0,
            expected_for(&key_file_hex)
        );

        //    (b) private key file in dendro's private data root beats consumer.env
        let cfg_b = SessionLinkConfig::from_sources(
            &BTreeMap::new(),
            Some(&private_root),
            Some(&xdg_consumer),
        );
        assert_eq!(
            cfg_b
                .resolve_session_link(Some("claude"), Some("sess-1"), None)
                .0,
            expected_for(&key_root_hex)
        );

        //    (c) consumer.env values are used when neither process env nor private_root/link.key is present
        let cfg_c = SessionLinkConfig::from_sources(
            &BTreeMap::new(),
            Some(&empty_root),
            Some(&xdg_consumer),
        );
        assert_eq!(
            cfg_c
                .resolve_session_link(Some("claude"), Some("sess-1"), None)
                .0,
            expected_for(&key_consumer_hex)
        );

        // 3. DENDRO_LINK_KEY_FILE inside consumer.env is ignored
        let consumer_with_key_file = dir.join("consumer_with_key_file.env");
        std::fs::write(
            &consumer_with_key_file,
            format!(
                "DENDRO_LINK_ORG=acme\nDENDRO_LINK_ENDPOINT_SCOPE=ws-01\nDENDRO_LINK_KEY_FILE={}\nAGENT_OTEL_LINK_KEY_FILE={}\n",
                explicit_key_path.display(),
                explicit_key_path.display()
            ),
        )
        .unwrap();
        let cfg_ignored_file = SessionLinkConfig::from_sources(
            &BTreeMap::new(),
            Some(&empty_root),
            Some(&consumer_with_key_file),
        );
        assert!(
            !cfg_ignored_file.has_valid_key(),
            "DENDRO_LINK_KEY_FILE inside consumer.env must be ignored"
        );
        assert_eq!(
            cfg_ignored_file.resolve_session_link(Some("claude"), Some("sess-1"), None),
            (
                AGENT_SESSION_LINK_UNLINKED.to_string(),
                Some(UnlinkedReason::InvalidKeyMaterial)
            )
        );

        // 4. Relative DENDRO_LINK_KEY_FILE path in process env is refused (even if consumer.env has a key)
        let mut env_rel = BTreeMap::new();
        env_rel.insert(
            "DENDRO_LINK_KEY_FILE".to_string(),
            "relative/link.key".to_string(),
        );
        let cfg_rel =
            SessionLinkConfig::from_sources(&env_rel, Some(&private_root), Some(&xdg_consumer));
        assert!(
            !cfg_rel.has_valid_key(),
            "relative DENDRO_LINK_KEY_FILE must be refused without falling back"
        );
        assert_eq!(
            cfg_rel.resolve_session_link(Some("claude"), Some("sess-1"), None),
            (
                AGENT_SESSION_LINK_UNLINKED.to_string(),
                Some(UnlinkedReason::InvalidKeyMaterial)
            )
        );

        // 5. K_org variable inside consumer.env (both `K_ORG_HEX=...` and `export DENDRO_LINK_K_ORG_HEX=...`)
        //    refuses link configuration even when process env has a valid key; unrelated `K_ORGANIC` is ignored.
        for k_org_line in [
            "K_ORG_HEX=deadbeef",
            "export K_ORG_HEX=deadbeef",
            "export DENDRO_LINK_K_ORG_HEX=deadbeef",
        ] {
            let consumer_with_k_org = dir.join("consumer_with_k_org.env");
            std::fs::write(
                &consumer_with_k_org,
                format!("DENDRO_LINK_ORG=acme\nDENDRO_LINK_ENDPOINT_SCOPE=ws-01\n{k_org_line}\n"),
            )
            .unwrap();
            let mut env_valid_key = BTreeMap::new();
            env_valid_key.insert("DENDRO_LINK_KEY_HEX".to_string(), key_env_hex.clone());
            let cfg_k_org_in_consumer = SessionLinkConfig::from_sources(
                &env_valid_key,
                Some(&private_root),
                Some(&consumer_with_k_org),
            );
            assert!(
                !cfg_k_org_in_consumer.has_valid_key(),
                "failed to refuse {k_org_line}"
            );
            assert!(cfg_k_org_in_consumer.k_org_rejected());
            assert_eq!(
                cfg_k_org_in_consumer.resolve_session_link(Some("claude"), Some("sess-1"), None),
                (
                    AGENT_SESSION_LINK_UNLINKED.to_string(),
                    Some(UnlinkedReason::InvalidKeyMaterial)
                )
            );
        }
        assert!(
            !is_k_org_var_name("K_ORGANIC"),
            "narrowed is_k_org_var_name must not match unrelated K_ORGANIC"
        );
        assert!(
            !is_k_org_var_name("MY_K_ORG_LIKE_FLAG"),
            "narrowed is_k_org_var_name must only match documented names"
        );

        // 6. Hard-linked key files (`links > 1`) in `<private_data_root>/link.key` or `DENDRO_LINK_KEY_FILE`
        //    are refused on Windows and Unix via `platform_v1::open_existing_private_file`.
        let hardlink_alias = dir.join("hardlinked_alias.key");
        std::fs::hard_link(&explicit_key_path, &hardlink_alias).unwrap();
        let mut env_hardlinked = BTreeMap::new();
        env_hardlinked.insert(
            "DENDRO_LINK_KEY_FILE".to_string(),
            explicit_key_path.display().to_string(),
        );
        let cfg_hardlinked =
            SessionLinkConfig::from_sources(&env_hardlinked, None, Some(&xdg_consumer));
        assert!(
            !cfg_hardlinked.has_valid_key(),
            "hard-linked DENDRO_LINK_KEY_FILE (links > 1) must be refused"
        );
        std::fs::remove_file(&hardlink_alias).unwrap();

        let root_hardlink_alias = private_root.join("link_alias.key");
        std::fs::hard_link(private_root.join("link.key"), &root_hardlink_alias).unwrap();
        let cfg_root_hardlinked = SessionLinkConfig::from_sources(
            &BTreeMap::new(),
            Some(&private_root),
            Some(&xdg_consumer),
        );
        assert!(
            !cfg_root_hardlinked.has_valid_key(),
            "hard-linked <private_data_root>/link.key (links > 1) must be refused"
        );
        std::fs::remove_file(&root_hardlink_alias).unwrap();

        // 7. On Unix, group- or world-readable key files are refused
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let world_readable = dir.join("world_readable.key");
            std::fs::write(&world_readable, &format!("{key_file_hex}\n")).unwrap();
            std::fs::set_permissions(&world_readable, std::fs::Permissions::from_mode(0o644))
                .unwrap();
            let mut env_bad_perms = BTreeMap::new();
            env_bad_perms.insert(
                "DENDRO_LINK_KEY_FILE".to_string(),
                world_readable.display().to_string(),
            );
            let cfg_bad_perms =
                SessionLinkConfig::from_sources(&env_bad_perms, None, Some(&xdg_consumer));
            assert!(
                !cfg_bad_perms.has_valid_key(),
                "world/group-readable key file must be refused on Unix"
            );
        }

        std::fs::remove_dir_all(dir).unwrap();
    }
}
