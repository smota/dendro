#![deny(unsafe_code)]

//! `A03` — Single-Owner Daemon (`dendro serve`) & Typed Capability Transport.
//!
//! Only `dendro serve` holds the raw DuckDB / `AnalyticsStore` / `quack` SQL session.
//! External clients (CLI, TUI, HTTP, Nao, Hook) connect over a private Unix domain
//! socket (mode 0700) or private Windows Named Pipe. Unauthenticated loopback TCP is strictly forbidden.
//! Same-user isolation is enforced via typed RPC capability checks: any request containing
//! raw SQL or attempting to borrow a SQL session is deterministically rejected with
//! `CapabilityRefused("raw_sql_forbidden")`. Hook credentials (`ClientRole::Hook`) are
//! restricted strictly to `AppendFact` and `Advise`.
//!
//! Cryptographic tokens are derived per role via HMAC-SHA256 from the root quack secret,
//! ensuring that a client claiming a role must present the exact derived token for that role.

use anyhow::{bail, Context, Result};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;
use tracing::info;

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use tokio::net::{UnixListener, UnixStream};

#[cfg(windows)]
use tokio::net::windows::named_pipe::{ClientOptions, ServerOptions};

type HmacSha256 = Hmac<Sha256>;

/// Transport validation errors.
#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("unauthenticated loopback TCP is forbidden; quack transport must be a private Unix domain socket or Windows named pipe")]
    LoopbackTcpForbidden,
    #[error("allow_other_hostname is forbidden on quack transport")]
    AllowOtherHostnameForbidden,
    #[error(
        "socket directory mode {0:#o} is not private (must be 0700 with no group or other bits)"
    )]
    InsecureDirectoryPermissions(u32),
    #[error("capability refused: {0}")]
    CapabilityRefused(&'static str),
}

/// Client role determining permitted typed capabilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientRole {
    Cli,
    Tui,
    Http,
    Nao,
    Hook,
}

impl ClientRole {
    pub fn wire_name(&self) -> &'static str {
        match self {
            Self::Cli => "cli",
            Self::Tui => "tui",
            Self::Http => "http",
            Self::Nao => "nao",
            Self::Hook => "hook",
        }
    }
}

/// Derives a cryptographic role capability token using HMAC-SHA256 over the role name.
pub fn derive_role_token(root_secret: &[u8], role: ClientRole) -> String {
    let mut mac = HmacSha256::new_from_slice(root_secret).expect("HMAC supports any key length");
    mac.update(format!("quack_role:{}", role.wire_name()).as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

/// Client credential passed on typed RPC requests.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClientCredential {
    pub client_id: String,
    pub role: ClientRole,
    #[serde(default)]
    pub token: String,
}

/// Sidecar token manager.
/// Root token is stored ONLY in a 0600 sidecar file inside the private directory.
/// Hook token is stored in `quack.hook.token` with mode 0600.
pub struct QuackToken;

impl QuackToken {
    pub fn generate_and_save(private_dir: &Path) -> Result<String> {
        let token_path = private_dir.join("quack.token");
        let token_hex = if token_path.exists() {
            let existing = fs::read_to_string(&token_path)?.trim().to_string();
            if existing.len() == 64 {
                existing
            } else {
                let token_bytes: [u8; 32] = rand::random();
                let hex = hex::encode(token_bytes);
                Self::write_private_file(&token_path, &hex)?;
                hex
            }
        } else {
            let token_bytes: [u8; 32] = rand::random();
            let hex = hex::encode(token_bytes);
            Self::write_private_file(&token_path, &hex)?;
            hex
        };

        let root_bytes = hex::decode(&token_hex).unwrap_or_default();
        let hook_token = derive_role_token(&root_bytes, ClientRole::Hook);
        let hook_path = private_dir.join("quack.hook.token");
        Self::write_private_file(&hook_path, &hook_token)?;

        Ok(token_hex)
    }

    fn write_private_file(path: &Path, content: &str) -> Result<()> {
        #[cfg(unix)]
        {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(path)?;
            std::io::Write::write_all(&mut file, content.as_bytes())?;
        }
        #[cfg(not(unix))]
        {
            fs::write(path, content)?;
        }
        Ok(())
    }

    pub fn read_token(private_dir: &Path) -> Result<String> {
        let token_path = private_dir.join("quack.token");
        if !token_path.exists() {
            bail!("quack token sidecar not found");
        }
        #[cfg(unix)]
        {
            let meta = fs::metadata(&token_path)?;
            let mode = meta.permissions().mode();
            if mode & 0o077 != 0 {
                bail!(
                    "quack token file permissions must be 0600, found {:#o}",
                    mode
                );
            }
        }
        let token = fs::read_to_string(&token_path)?.trim().to_string();
        Ok(token)
    }

    pub fn read_hook_token(private_dir: &Path) -> Result<String> {
        let hook_path = private_dir.join("quack.hook.token");
        if !hook_path.exists() {
            bail!("quack hook token sidecar not found");
        }
        let token = fs::read_to_string(&hook_path)?.trim().to_string();
        Ok(token)
    }
}

/// Transport bind address validator.
pub fn validate_bind_address(addr: &str) -> Result<(), TransportError> {
    let lower = addr.to_ascii_lowercase();
    if lower.contains("127.0.0.1")
        || lower.contains("0.0.0.0")
        || lower.contains("localhost")
        || lower.contains("::1")
        || lower.starts_with("tcp://")
        || lower.contains(":80")
        || lower.contains(":443")
        || lower.contains(":9000")
        || lower.contains(":5432")
    {
        return Err(TransportError::LoopbackTcpForbidden);
    }
    if lower.contains("allow_other_hostname") {
        return Err(TransportError::AllowOtherHostnameForbidden);
    }
    Ok(())
}

/// Validates private directory permissions for Unix domain socket or Windows directory.
pub fn validate_socket_dir(dir: &Path) -> Result<(), TransportError> {
    #[cfg(unix)]
    {
        let meta = fs::metadata(dir).map_err(|_| {
            TransportError::CapabilityRefused("socket directory could not be inspected")
        })?;
        let mode = meta.permissions().mode();
        if mode & 0o077 != 0 {
            return Err(TransportError::InsecureDirectoryPermissions(mode));
        }
    }
    let _ = dir;
    Ok(())
}

/// Deterministically scans an incoming JSON payload or string for raw SQL keywords
/// or attempts to borrow a raw SQL session.
pub fn scan_for_raw_sql(val: &serde_json::Value) -> Result<(), TransportError> {
    match val {
        serde_json::Value::String(s) => {
            let lower = s.to_ascii_lowercase();
            let upper = s.trim().to_ascii_uppercase();

            let sql_keywords = [
                "select ", "select\n", "select\t", "insert ", "insert\n", "insert\t", "attach ",
                "attach\n", "attach\t", "copy ", "copy\n", "copy\t", "pragma ", "pragma\n",
                "pragma\t", "update ", "delete ", "drop ", "alter ", "create ", "exec ",
                "execute ",
            ];

            for kw in &sql_keywords {
                if lower.contains(kw) {
                    return Err(TransportError::CapabilityRefused("raw_sql_forbidden"));
                }
            }

            if upper.starts_with("SELECT")
                || upper.starts_with("INSERT")
                || upper.starts_with("ATTACH")
                || upper.starts_with("COPY")
                || upper.starts_with("PRAGMA")
                || upper.starts_with("DROP")
                || upper.starts_with("DELETE")
                || upper.starts_with("ALTER")
                || upper.starts_with("CREATE")
                || upper.starts_with("SHOW")
                || upper.starts_with("DESCRIBE")
            {
                return Err(TransportError::CapabilityRefused("raw_sql_forbidden"));
            }

            if lower.contains("borrow_sql")
                || lower.contains("raw_sql")
                || lower.contains("get_session")
                || lower.contains("sql_session")
                || lower.contains("quack_session")
                || lower.contains("quack_serve")
            {
                return Err(TransportError::CapabilityRefused("raw_sql_forbidden"));
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                scan_for_raw_sql(item)?;
            }
        }
        serde_json::Value::Object(map) => {
            for (key, v) in map {
                let lower_key = key.to_ascii_lowercase();
                if lower_key == "sql"
                    || lower_key == "raw_sql"
                    || lower_key == "query"
                    || lower_key == "statement"
                    || lower_key == "sql_session"
                    || lower_key == "borrow_session"
                {
                    return Err(TransportError::CapabilityRefused("raw_sql_forbidden"));
                }
                scan_for_raw_sql(v)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Typed RPC request envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcEnvelope {
    pub client_credential: ClientCredential,
    pub method: String,
    pub params: serde_json::Value,
}

/// Typed RPC response envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcResponse {
    pub status: String,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
}

static FACT_COUNTER: AtomicU64 = AtomicU64::new(1);

fn next_source_key(prefix: &str) -> String {
    let count = FACT_COUNTER.fetch_add(1, Ordering::Relaxed);
    let rand_val: u128 = rand::random();
    let ts = chrono::Utc::now().timestamp_micros();
    format!("{prefix}:{ts}:{count}:{rand_val:032x}")
}

pub(crate) fn value_ref_to_json(val: duckdb::types::ValueRef) -> serde_json::Value {
    use duckdb::types::ValueRef;
    match val {
        ValueRef::Null => serde_json::Value::Null,
        ValueRef::Boolean(b) => serde_json::Value::Bool(b),
        ValueRef::TinyInt(n) => serde_json::json!(n),
        ValueRef::SmallInt(n) => serde_json::json!(n),
        ValueRef::Int(n) => serde_json::json!(n),
        ValueRef::BigInt(n) => serde_json::json!(n),
        ValueRef::HugeInt(n) => serde_json::json!(n.to_string()),
        ValueRef::UHugeInt(n) => serde_json::json!(n.to_string()),
        ValueRef::UTinyInt(n) => serde_json::json!(n),
        ValueRef::USmallInt(n) => serde_json::json!(n),
        ValueRef::UInt(n) => serde_json::json!(n),
        ValueRef::UBigInt(n) => serde_json::json!(n),
        ValueRef::Float(f) => serde_json::json!(f),
        ValueRef::Double(d) => serde_json::json!(d),
        ValueRef::Decimal(d) => {
            let s = d.to_string();
            if let Ok(n) = s.parse::<f64>() {
                if n.is_finite() {
                    serde_json::json!(n)
                } else {
                    serde_json::json!(s)
                }
            } else {
                serde_json::json!(s)
            }
        }
        ValueRef::Text(bytes) => {
            serde_json::Value::String(String::from_utf8_lossy(bytes).into_owned())
        }
        ValueRef::Blob(bytes) => serde_json::Value::String(hex::encode(bytes)),
        ValueRef::Timestamp(unit, val) => {
            let micros = unit.to_micros(val);
            if let Some(dt) = chrono::DateTime::from_timestamp_micros(micros) {
                serde_json::json!(dt.to_rfc3339())
            } else {
                serde_json::json!(format!("{val}:{unit:?}"))
            }
        }
        ValueRef::Date32(d) => serde_json::json!(d),
        ValueRef::Time64(unit, t) => {
            let (secs, nsecs) = match unit {
                duckdb::types::TimeUnit::Second => (t, 0),
                duckdb::types::TimeUnit::Millisecond => (t / 1_000, (t % 1_000) * 1_000_000),
                duckdb::types::TimeUnit::Microsecond => (t / 1_000_000, (t % 1_000_000) * 1_000),
                duckdb::types::TimeUnit::Nanosecond => (t / 1_000_000_000, t % 1_000_000_000),
            };
            if let Some(time) = chrono::NaiveTime::from_num_seconds_from_midnight_opt(
                secs.max(0) as u32,
                nsecs.max(0) as u32,
            ) {
                serde_json::json!(time.format("%H:%M:%S%.3f").to_string())
            } else {
                serde_json::json!(format!("{t}:{unit:?}"))
            }
        }
        ValueRef::Interval {
            months,
            days,
            nanos,
        } => serde_json::json!({
            "months": months,
            "days": days,
            "nanos": nanos,
        }),
        ValueRef::Enum(_e, _idx) => serde_json::json!({ "unsupported_type": "enum" }),
        ValueRef::List(_list, _idx) => serde_json::json!({ "unsupported_type": "list" }),
        ValueRef::Array(_arr, _idx) => serde_json::json!({ "unsupported_type": "array" }),
        ValueRef::Struct(_s, _idx) => serde_json::json!({ "unsupported_type": "struct" }),
        ValueRef::Map(_m, _idx) => serde_json::json!({ "unsupported_type": "map" }),
        ValueRef::Union(_u, _idx) => serde_json::json!({ "unsupported_type": "union" }),
        ValueRef::Geometry(_bytes) => serde_json::json!({ "unsupported_type": "geometry" }),
        _ => serde_json::Value::Null,
    }
}

/// Single-owner DuckDB store holding the exclusive session and WAL.
pub struct QuackStore {
    conn: duckdb::Connection,
    db_path: PathBuf,
}

impl QuackStore {
    pub fn open(db_path: &Path) -> Result<Self> {
        let conn = duckdb::Connection::open(db_path)
            .context("failed to acquire single-owner raw DuckDB session")?;

        // Initialize metadata and standard fact schemas if not present
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS dendro_meta (
                schema_version INTEGER NOT NULL,
                key_id VARCHAR NOT NULL
            );
            INSERT INTO dendro_meta (schema_version, key_id)
            SELECT 3, 'quack_owner_daemon'
            WHERE NOT EXISTS (SELECT 1 FROM dendro_meta);
            CREATE TABLE IF NOT EXISTS skill_facts (
                source_key VARCHAR PRIMARY KEY,
                seat VARCHAR NOT NULL,
                skill VARCHAR NOT NULL,
                invoked BOOLEAN NOT NULL
            );
            CREATE TABLE IF NOT EXISTS identity_facts (
                source_key VARCHAR PRIMARY KEY,
                harness VARCHAR NOT NULL,
                native_session VARCHAR NOT NULL,
                rig VARCHAR NOT NULL,
                seat VARCHAR NOT NULL,
                ts TIMESTAMP NOT NULL
            );
            CREATE TABLE IF NOT EXISTS activity_facts (
                source_key VARCHAR PRIMARY KEY,
                seat VARCHAR NOT NULL,
                ts TIMESTAMP NOT NULL,
                state VARCHAR NOT NULL,
                seq BIGINT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS tool_facts (
                source_key VARCHAR PRIMARY KEY,
                harness VARCHAR NOT NULL,
                native_session VARCHAR NOT NULL,
                ts TIMESTAMP NOT NULL,
                seq BIGINT NOT NULL,
                tool_name VARCHAR NOT NULL,
                failed BOOLEAN,
                structured_write BOOLEAN NOT NULL,
                generation BIGINT NOT NULL,
                source_id VARCHAR NOT NULL,
                category VARCHAR NOT NULL
            );
            CREATE TABLE IF NOT EXISTS usage_facts (
                source_key VARCHAR PRIMARY KEY,
                harness VARCHAR NOT NULL,
                native_session VARCHAR NOT NULL,
                ts TIMESTAMP NOT NULL,
                seq BIGINT NOT NULL,
                input_uncached BIGINT NOT NULL,
                output_tokens BIGINT NOT NULL,
                cache_read BIGINT NOT NULL,
                cache_write BIGINT NOT NULL,
                reasoning BIGINT NOT NULL,
                currency_total DOUBLE,
                generation BIGINT NOT NULL,
                source_id VARCHAR NOT NULL,
                model VARCHAR
            );
            CREATE TABLE IF NOT EXISTS session_edges (
                source_key VARCHAR PRIMARY KEY,
                source_id VARCHAR NOT NULL,
                harness VARCHAR NOT NULL,
                parent_session VARCHAR NOT NULL,
                child_session VARCHAR NOT NULL,
                relation VARCHAR NOT NULL,
                status VARCHAR NOT NULL,
                observed_at TIMESTAMP,
                evidence_kind VARCHAR NOT NULL,
                generation BIGINT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS security_facts (
                source_key VARCHAR PRIMARY KEY,
                source_id VARCHAR NOT NULL,
                harness VARCHAR NOT NULL,
                native_session VARCHAR,
                ts TIMESTAMP,
                kind VARCHAR NOT NULL,
                value VARCHAR NOT NULL,
                source_field VARCHAR NOT NULL,
                generation BIGINT NOT NULL
            );
            CREATE VIEW IF NOT EXISTS v_tokens AS
            SELECT harness, native_session, SUM(input_uncached)::BIGINT AS input_uncached,
                   SUM(output_tokens)::BIGINT AS output_tokens,
                   SUM(cache_read)::BIGINT AS cache_read,
                   SUM(cache_write)::BIGINT AS cache_write
            FROM usage_facts GROUP BY ALL;
            CREATE VIEW IF NOT EXISTS v_model_usage AS
            SELECT harness, model, COUNT(*)::BIGINT AS call_count,
                   SUM(input_uncached)::BIGINT AS input_uncached,
                   SUM(output_tokens)::BIGINT AS output_tokens
            FROM usage_facts GROUP BY ALL;
            CREATE VIEW IF NOT EXISTS v_skill_inventory AS
            SELECT seat, skill, COUNT(*)::BIGINT AS uses,
                   SUM(CASE WHEN invoked THEN 1 ELSE 0 END)::BIGINT AS invoked_count
            FROM skill_facts GROUP BY ALL;
            CREATE VIEW IF NOT EXISTS v_tool_reliability AS
            SELECT harness, tool_name, COUNT(*)::BIGINT AS call_count,
                   SUM(CASE WHEN failed = true THEN 1 ELSE 0 END)::BIGINT AS failures
            FROM tool_facts GROUP BY ALL;",
        )?;

        Ok(Self {
            conn,
            db_path: db_path.to_path_buf(),
        })
    }

    pub fn with_connection<F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&duckdb::Connection) -> Result<T>,
    {
        f(&self.conn)
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }
}

/// The single-owner daemon holding the exclusive raw DuckDB session.
pub struct QuackOwnerDaemon {
    db_path: PathBuf,
    socket_path: PathBuf,
    root_secret: [u8; 32],
    store: Arc<Mutex<QuackStore>>,
}

impl QuackOwnerDaemon {
    pub fn open(db_path: PathBuf, socket_path: Option<PathBuf>) -> Result<Self> {
        let parent = db_path.parent().unwrap_or_else(|| Path::new("."));
        let socket_path = socket_path.unwrap_or_else(|| {
            #[cfg(windows)]
            {
                PathBuf::from(r"\\.\pipe\LOCAL\dendro-quack")
            }
            #[cfg(not(windows))]
            {
                parent.join("dendro.sock")
            }
        });

        #[cfg(windows)]
        let socket_path = {
            let s = socket_path.to_string_lossy();
            if !s.starts_with(r"\\.\pipe\") {
                PathBuf::from(r"\\.\pipe\LOCAL\dendro-quack")
            } else {
                socket_path
            }
        };

        // Validate socket parent directory on Unix
        #[cfg(not(windows))]
        {
            let socket_parent = socket_path.parent().unwrap_or_else(|| Path::new("."));
            validate_socket_dir(socket_parent)
                .map_err(|e| anyhow::anyhow!("socket directory validation failed: {e}"))?;
        }

        // Generate private root and role tokens in atomic 0600 mode
        let token_hex = QuackToken::generate_and_save(parent)
            .context("failed to create quack token sidecar")?;
        let root_secret: [u8; 32] = hex::decode(token_hex)?
            .try_into()
            .map_err(|_| anyhow::anyhow!("invalid root token length"))?;

        // Open raw DuckDB connection - holds single-owner exclusive lock (no dual open)
        let store = QuackStore::open(&db_path)?;

        Ok(Self {
            db_path,
            socket_path,
            root_secret,
            store: Arc::new(Mutex::new(store)),
        })
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    pub fn token_for_role(&self, role: ClientRole) -> String {
        derive_role_token(&self.root_secret, role)
    }

    /// Dispatches a raw request string, verifying same-user containment and capability checks.
    pub async fn dispatch_request(&self, line: &str) -> RpcResponse {
        let val: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => {
                return RpcResponse {
                    status: "error".to_string(),
                    result: None,
                    error: Some(format!("invalid JSON: {e}")),
                }
            }
        };

        // Deterministic raw SQL rejection
        if let Err(e) = scan_for_raw_sql(&val) {
            return RpcResponse {
                status: "refused".to_string(),
                result: None,
                error: Some(e.to_string()),
            };
        }

        let envelope: RpcEnvelope = match serde_json::from_value(val) {
            Ok(env) => env,
            Err(e) => {
                return RpcResponse {
                    status: "error".to_string(),
                    result: None,
                    error: Some(format!("invalid RPC envelope: {e}")),
                }
            }
        };

        // Cryptographic Role Token Verification (Critical #2)
        let expected_token = self.token_for_role(envelope.client_credential.role);
        if envelope.client_credential.token.is_empty()
            || envelope.client_credential.token != expected_token
        {
            return RpcResponse {
                status: "refused".to_string(),
                result: None,
                error: Some("capability refused: invalid_or_forged_credential".to_string()),
            };
        }

        // Capability Scope Checks: Hook is restricted strictly to AppendFact and Advise
        if envelope.client_credential.role == ClientRole::Hook
            && envelope.method != "AppendFact"
            && envelope.method != "Advise"
        {
            return RpcResponse {
                status: "refused".to_string(),
                result: None,
                error: Some("capability refused: hook_restricted_to_append_and_advise".to_string()),
            };
        }

        match envelope.method.as_str() {
            "AppendFact" => {
                let table = envelope
                    .params
                    .get("table")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let record = envelope.params.get("record").cloned().unwrap_or_default();

                let allowed_tables = [
                    "usage_facts",
                    "tool_facts",
                    "identity_facts",
                    "activity_facts",
                    "skill_facts",
                    "session_edges",
                    "security_facts",
                ];
                if !allowed_tables.contains(&table) {
                    return RpcResponse {
                        status: "refused".to_string(),
                        result: None,
                        error: Some("capability refused: invalid_fact_table".to_string()),
                    };
                }

                // Execute real parameterized persistence for all 7 allowed tables via QuackStore with_connection
                let client_id = &envelope.client_credential.client_id;
                let store = self.store.lock().await;
                let outcome: Result<(String, usize)> = store.with_connection(|conn| {
                    let (source_key, affected) = match table {
                        "skill_facts" => {
                            let seat = record.get("seat").and_then(|v| v.as_str()).ok_or_else(|| {
                                anyhow::anyhow!("invalid_fact_payload: missing required field seat")
                            })?;
                            let skill = record.get("skill").and_then(|v| v.as_str()).ok_or_else(|| {
                                anyhow::anyhow!("invalid_fact_payload: missing required field skill")
                            })?;
                            let invoked = record
                                .get("invoked")
                                .and_then(|v| v.as_bool())
                                .ok_or_else(|| {
                                    anyhow::anyhow!("invalid_fact_payload: missing required field invoked")
                                })?;
                            let source_key = next_source_key("hook:skill");
                            let n = conn.execute(
                                "INSERT INTO skill_facts (source_key, seat, skill, invoked) VALUES (?, ?, ?, ?) ON CONFLICT (source_key) DO NOTHING",
                                duckdb::params![source_key, seat, skill, invoked],
                            )?;
                            (source_key, n)
                        }
                        "identity_facts" => {
                            let harness = record.get("harness").and_then(|v| v.as_str()).ok_or_else(|| {
                                anyhow::anyhow!("invalid_fact_payload: missing required field harness")
                            })?;
                            let session = record
                                .get("native_session")
                                .or_else(|| record.get("session"))
                                .or_else(|| record.get("session_id"))
                                .and_then(|v| v.as_str())
                                .ok_or_else(|| {
                                    anyhow::anyhow!("invalid_fact_payload: missing required field native_session")
                                })?;
                            let rig = record.get("rig").and_then(|v| v.as_str()).ok_or_else(|| {
                                anyhow::anyhow!("invalid_fact_payload: missing required field rig")
                            })?;
                            let seat = record.get("seat").and_then(|v| v.as_str()).ok_or_else(|| {
                                anyhow::anyhow!("invalid_fact_payload: missing required field seat")
                            })?;
                            let source_key = next_source_key("hook:id");
                            let n = conn.execute(
                                "INSERT INTO identity_facts (source_key, harness, native_session, rig, seat, ts) VALUES (?, ?, ?, ?, ?, CURRENT_TIMESTAMP) ON CONFLICT (source_key) DO NOTHING",
                                duckdb::params![source_key, harness, session, rig, seat],
                            )?;
                            (source_key, n)
                        }
                        "activity_facts" => {
                            let seat = record.get("seat").and_then(|v| v.as_str()).ok_or_else(|| {
                                anyhow::anyhow!("invalid_fact_payload: missing required field seat")
                            })?;
                            let state = record.get("state").and_then(|v| v.as_str()).ok_or_else(|| {
                                anyhow::anyhow!("invalid_fact_payload: missing required field state")
                            })?;
                            let seq = record
                                .get("seq")
                                .and_then(|v| v.as_i64())
                                .ok_or_else(|| {
                                    anyhow::anyhow!("invalid_fact_payload: missing required field seq")
                                })?;
                            let source_key = next_source_key("hook:act");
                            let n = conn.execute(
                                "INSERT INTO activity_facts (source_key, seat, ts, state, seq) VALUES (?, ?, CURRENT_TIMESTAMP, ?, ?) ON CONFLICT (source_key) DO NOTHING",
                                duckdb::params![source_key, seat, state, seq],
                            )?;
                            (source_key, n)
                        }
                        "tool_facts" => {
                            let harness = record.get("harness").and_then(|v| v.as_str()).ok_or_else(|| {
                                anyhow::anyhow!("invalid_fact_payload: missing required field harness")
                            })?;
                            let session = record
                                .get("native_session")
                                .or_else(|| record.get("session"))
                                .or_else(|| record.get("session_id"))
                                .and_then(|v| v.as_str())
                                .ok_or_else(|| {
                                    anyhow::anyhow!("invalid_fact_payload: missing required field native_session")
                                })?;
                            let tool_name = record
                                .get("tool_name")
                                .or_else(|| record.get("name"))
                                .and_then(|v| v.as_str())
                                .ok_or_else(|| {
                                    anyhow::anyhow!("invalid_fact_payload: missing required field tool_name")
                                })?;
                            let failed = record.get("failed").and_then(|v| v.as_bool());
                            let structured = record
                                .get("structured_write")
                                .and_then(|v| v.as_bool())
                                .ok_or_else(|| {
                                    anyhow::anyhow!("invalid_fact_payload: missing required field structured_write")
                                })?;
                            let category = record
                                .get("category")
                                .and_then(|v| v.as_str())
                                .ok_or_else(|| {
                                    anyhow::anyhow!("invalid_fact_payload: missing required field category")
                                })?;
                            let seq = record
                                .get("seq")
                                .and_then(|v| v.as_i64())
                                .ok_or_else(|| {
                                    anyhow::anyhow!("invalid_fact_payload: missing required field seq")
                                })?;
                            let generation = record
                                .get("generation")
                                .and_then(|v| v.as_i64())
                                .unwrap_or(1);
                            let source_id = record
                                .get("source_id")
                                .and_then(|v| v.as_str())
                                .unwrap_or(client_id);
                            let source_key = next_source_key("hook:tool");
                            let n = conn.execute(
                                "INSERT INTO tool_facts (source_key, harness, native_session, ts, seq, tool_name, failed, structured_write, generation, source_id, category) VALUES (?, ?, ?, CURRENT_TIMESTAMP, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT (source_key) DO NOTHING",
                                duckdb::params![source_key, harness, session, seq, tool_name, failed, structured, generation, source_id, category],
                            )?;
                            (source_key, n)
                        }
                        "usage_facts" => {
                            let harness = record.get("harness").and_then(|v| v.as_str()).ok_or_else(|| {
                                anyhow::anyhow!("invalid_fact_payload: missing required field harness")
                            })?;
                            let session = record
                                .get("native_session")
                                .or_else(|| record.get("session"))
                                .or_else(|| record.get("session_id"))
                                .and_then(|v| v.as_str())
                                .ok_or_else(|| {
                                    anyhow::anyhow!("invalid_fact_payload: missing required field native_session")
                                })?;
                            let input = record
                                .get("input_uncached")
                                .or_else(|| record.get("input"))
                                .and_then(|v| v.as_i64())
                                .ok_or_else(|| {
                                    anyhow::anyhow!("invalid_fact_payload: missing required field input_uncached")
                                })?;
                            let output = record
                                .get("output_tokens")
                                .or_else(|| record.get("output"))
                                .and_then(|v| v.as_i64())
                                .ok_or_else(|| {
                                    anyhow::anyhow!("invalid_fact_payload: missing required field output_tokens")
                                })?;
                            let cache_read = record
                                .get("cache_read")
                                .and_then(|v| v.as_i64())
                                .unwrap_or(0);
                            let cache_write = record
                                .get("cache_write")
                                .and_then(|v| v.as_i64())
                                .unwrap_or(0);
                            let reasoning = record
                                .get("reasoning")
                                .and_then(|v| v.as_i64())
                                .unwrap_or(0);
                            let currency_total: Option<f64> = record
                                .get("currency_total")
                                .and_then(|v| v.as_f64());
                            let model: Option<&str> = record
                                .get("model")
                                .and_then(|v| v.as_str());
                            let seq = record
                                .get("seq")
                                .and_then(|v| v.as_i64())
                                .ok_or_else(|| {
                                    anyhow::anyhow!("invalid_fact_payload: missing required field seq")
                                })?;
                            let generation = record
                                .get("generation")
                                .and_then(|v| v.as_i64())
                                .unwrap_or(1);
                            let source_id = record
                                .get("source_id")
                                .and_then(|v| v.as_str())
                                .unwrap_or(client_id);
                            let source_key = next_source_key("hook:usage");
                            let n = conn.execute(
                                "INSERT INTO usage_facts (source_key, harness, native_session, ts, seq, input_uncached, output_tokens, cache_read, cache_write, reasoning, currency_total, generation, source_id, model) VALUES (?, ?, ?, CURRENT_TIMESTAMP, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT (source_key) DO NOTHING",
                                duckdb::params![source_key, harness, session, seq, input, output, cache_read, cache_write, reasoning, currency_total, generation, source_id, model],
                            )?;
                            (source_key, n)
                        }
                        "session_edges" => {
                            let parent = record
                                .get("parent_session")
                                .or_else(|| record.get("parent_session_id"))
                                .and_then(|v| v.as_str())
                                .ok_or_else(|| {
                                    anyhow::anyhow!("invalid_fact_payload: missing required field parent_session_id")
                                })?;
                            let child = record
                                .get("child_session")
                                .or_else(|| record.get("child_session_id"))
                                .and_then(|v| v.as_str())
                                .ok_or_else(|| {
                                    anyhow::anyhow!("invalid_fact_payload: missing required field child_session_id")
                                })?;
                            let relation = record
                                .get("relation")
                                .or_else(|| record.get("edge_kind"))
                                .and_then(|v| v.as_str())
                                .ok_or_else(|| {
                                    anyhow::anyhow!("invalid_fact_payload: missing required field edge_kind")
                                })?;
                            let status = record.get("status").and_then(|v| v.as_str()).ok_or_else(|| {
                                anyhow::anyhow!("invalid_fact_payload: missing required field status")
                            })?;
                            let harness = record
                                .get("harness")
                                .and_then(|v| v.as_str())
                                .ok_or_else(|| {
                                    anyhow::anyhow!("invalid_fact_payload: missing required field harness")
                                })?;
                            let evidence_kind = record
                                .get("evidence_kind")
                                .and_then(|v| v.as_str())
                                .ok_or_else(|| {
                                    anyhow::anyhow!("invalid_fact_payload: missing required field evidence_kind")
                                })?;
                            let generation = record
                                .get("generation")
                                .and_then(|v| v.as_i64())
                                .unwrap_or(1);
                            let source_id = record
                                .get("source_id")
                                .and_then(|v| v.as_str())
                                .unwrap_or(client_id);
                            let source_key = next_source_key("hook:edge");
                            let n = conn.execute(
                                "INSERT INTO session_edges (source_key, source_id, harness, parent_session, child_session, relation, status, evidence_kind, generation) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT (source_key) DO NOTHING",
                                duckdb::params![source_key, source_id, harness, parent, child, relation, status, evidence_kind, generation],
                            )?;
                            (source_key, n)
                        }
                        "security_facts" => {
                            let harness = record.get("harness").and_then(|v| v.as_str()).ok_or_else(|| {
                                anyhow::anyhow!("invalid_fact_payload: missing required field harness")
                            })?;
                            let session = record
                                .get("native_session")
                                .or_else(|| record.get("session"))
                                .or_else(|| record.get("session_id"))
                                .and_then(|v| v.as_str())
                                .ok_or_else(|| {
                                    anyhow::anyhow!("invalid_fact_payload: missing required field native_session")
                                })?;
                            let sandbox_mode = record.get("sandbox_mode").and_then(|v| v.as_str());
                            let kind = record
                                .get("kind")
                                .and_then(|v| v.as_str())
                                .or_else(|| if sandbox_mode.is_some() { Some("sandbox_policy") } else { None })
                                .ok_or_else(|| {
                                    anyhow::anyhow!("invalid_fact_payload: missing required field kind/sandbox_mode")
                                })?;
                            let value = record
                                .get("value")
                                .and_then(|v| v.as_str())
                                .or(sandbox_mode)
                                .ok_or_else(|| {
                                    anyhow::anyhow!("invalid_fact_payload: missing required field value/sandbox_mode")
                                })?;
                            let field = record
                                .get("source_field")
                                .and_then(|v| v.as_str())
                                .ok_or_else(|| {
                                    anyhow::anyhow!("invalid_fact_payload: missing required field source_field")
                                })?;
                            let generation = record
                                .get("generation")
                                .and_then(|v| v.as_i64())
                                .unwrap_or(1);
                            let source_id = record
                                .get("source_id")
                                .and_then(|v| v.as_str())
                                .unwrap_or(client_id);
                            let source_key = next_source_key("hook:sec");
                            let n = conn.execute(
                                "INSERT INTO security_facts (source_key, source_id, harness, native_session, ts, kind, value, source_field, generation) VALUES (?, ?, ?, ?, CURRENT_TIMESTAMP, ?, ?, ?, ?) ON CONFLICT (source_key) DO NOTHING",
                                duckdb::params![source_key, source_id, harness, session, kind, value, field, generation],
                            )?;
                            (source_key, n)
                        }
                        _ => (String::new(), 0),
                    };
                    if affected == 0 {
                        anyhow::bail!("insert affected 0 rows (conflict or refused)");
                    }
                    Ok((source_key, affected))
                });

                match outcome {
                    Ok((key, n)) => {
                        crate::advise_v1::clear_advise_cache();
                        RpcResponse {
                            status: "ok".to_string(),
                            result: Some(serde_json::json!({
                                "appended": true,
                                "table": table,
                                "source_key": key,
                                "rows_affected": n,
                            })),
                            error: None,
                        }
                    }
                    Err(e) => RpcResponse {
                        status: "error".to_string(),
                        result: None,
                        error: Some(e.to_string()),
                    },
                }
            }
            "Advise" => {
                let question = envelope
                    .params
                    .get("question")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let harness = envelope
                    .params
                    .get("harness")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string);
                let skill = envelope
                    .params
                    .get("skill")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string);
                let session = envelope
                    .params
                    .get("session")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string);
                let budget_ms = envelope.params.get("budget_ms").and_then(|v| v.as_u64());

                let req = crate::advise_v1::AdviseRequest {
                    question: question.to_string(),
                    harness,
                    skill,
                    session,
                    budget_ms,
                };

                let store = self.store.lock().await;
                let resp = store
                    .with_connection(|conn| Ok(crate::advise_v1::evaluate_advise_conn(conn, &req)));

                match resp {
                    Ok(r) => RpcResponse {
                        status: "ok".to_string(),
                        result: Some(serde_json::to_value(r).unwrap_or_default()),
                        error: None,
                    },
                    Err(e) => RpcResponse {
                        status: "error".to_string(),
                        result: None,
                        error: Some(e.to_string()),
                    },
                }
            }
            "QueryView" => {
                let view_name = envelope
                    .params
                    .get("view_name")
                    .or_else(|| envelope.params.get("view"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let allowed_views = [
                    "v_model_usage",
                    "v_tool_reliability",
                    "v_skill_inventory",
                    "v_session_topology",
                    "v_environment_posture",
                    "v_trajectory_vortex",
                    "v_schema_drift",
                    "v_authority_blast_radius",
                    "v_counterfactual_routing",
                    "v_tokens",
                    "usage_facts",
                    "tool_facts",
                    "skill_facts",
                    "identity_facts",
                    "activity_facts",
                    "session_edges",
                    "security_facts",
                ];
                if !allowed_views.contains(&view_name) {
                    return RpcResponse {
                        status: "refused".to_string(),
                        result: None,
                        error: Some("capability refused: view_name_not_allowed".to_string()),
                    };
                }

                let store = self.store.lock().await;
                let rows_res = store.with_connection(|conn| {
                    let exists: i64 = conn
                        .query_row(
                            "SELECT COUNT(*) FROM information_schema.tables WHERE lower(table_name) = lower(?)",
                            duckdb::params![view_name],
                            |r| r.get(0),
                        )
                        .unwrap_or(0);

                    if exists == 0 {
                        return Ok(vec![]);
                    }

                    let limit_clause = envelope
                        .params
                        .get("limit")
                        .and_then(|v| v.as_i64())
                        .map(|l| format!(" LIMIT {l}"))
                        .unwrap_or_default();
                    let mut stmt =
                        conn.prepare(&format!("SELECT * FROM \"{}\"{limit_clause}", view_name))?;
                    let mapped = stmt.query_map([], |row| {
                        let mut map = serde_json::Map::new();
                        let col_names = row.as_ref().column_names();
                        for (i, name) in col_names.into_iter().enumerate() {
                            let v_ref = row.get_ref(i).unwrap_or(duckdb::types::ValueRef::Null);
                            map.insert(name, value_ref_to_json(v_ref));
                        }
                        Ok(serde_json::Value::Object(map))
                    })?;
                    Ok(mapped.filter_map(Result::ok).collect::<Vec<_>>())
                });

                match rows_res {
                    Ok(rows) => RpcResponse {
                        status: "ok".to_string(),
                        result: Some(serde_json::json!({"view": view_name, "rows": rows})),
                        error: None,
                    },
                    Err(e) => RpcResponse {
                        status: "error".to_string(),
                        result: None,
                        error: Some(e.to_string()),
                    },
                }
            }
            "Status" => RpcResponse {
                status: "ok".to_string(),
                result: Some(serde_json::json!({"daemon": "quack_owner", "status": "healthy"})),
                error: None,
            },
            _ => RpcResponse {
                status: "refused".to_string(),
                result: None,
                error: Some(format!("unknown or forbidden method: {}", envelope.method)),
            },
        }
    }

    /// Serves on Unix domain socket or Windows named pipe.
    pub async fn serve(&self, once: bool) -> Result<()> {
        #[cfg(unix)]
        {
            if self.socket_path.exists() {
                let _ = fs::remove_file(&self.socket_path);
            }

            let listener = UnixListener::bind(&self.socket_path)?;
            let perms = fs::Permissions::from_mode(0o700);
            fs::set_permissions(&self.socket_path, perms)?;

            info!(
                "quack_owner daemon listening on {}",
                self.socket_path.display()
            );

            loop {
                let (stream, _) = listener.accept().await?;
                let (reader, mut writer) = stream.into_split();
                let mut buf_reader = BufReader::new(reader);
                let mut line = String::new();

                while buf_reader.read_line(&mut line).await? > 0 {
                    let resp = self.dispatch_request(line.trim_end()).await;
                    let serialized = serde_json::to_string(&resp)? + "\n";
                    writer.write_all(serialized.as_bytes()).await?;
                    line.clear();
                }

                if once {
                    break;
                }
            }

            if self.socket_path.exists() {
                let _ = fs::remove_file(&self.socket_path);
            }
        }

        #[cfg(windows)]
        {
            let pipe_name = self
                .socket_path
                .to_str()
                .unwrap_or(r"\\.\pipe\LOCAL\dendro-quack");
            let mut server = ServerOptions::new()
                .first_pipe_instance(true)
                .create(pipe_name)?;
            loop {
                server.connect().await?;
                let (reader, mut writer) = tokio::io::split(server);
                let mut buf_reader = BufReader::new(reader);
                let mut line = String::new();

                while buf_reader.read_line(&mut line).await? > 0 {
                    let resp = self.dispatch_request(line.trim_end()).await;
                    let serialized = serde_json::to_string(&resp)? + "\n";
                    writer.write_all(serialized.as_bytes()).await?;
                    line.clear();
                }

                if once {
                    break;
                }
                server = ServerOptions::new().create(pipe_name)?;
            }
        }

        Ok(())
    }
}

/// Helper for client processes to send typed RPC requests over Unix domain socket or Windows named pipe.
pub async fn send_rpc(
    socket_path: &Path,
    cred: &ClientCredential,
    method: &str,
    params: serde_json::Value,
) -> Result<RpcResponse> {
    let envelope = RpcEnvelope {
        client_credential: cred.clone(),
        method: method.to_string(),
        params,
    };
    let serialized = serde_json::to_string(&envelope)? + "\n";
    let resp_str = send_raw_ipc(socket_path, &serialized).await?;
    let resp: RpcResponse = serde_json::from_str(&resp_str)?;
    Ok(resp)
}

/// Helper for client processes to send raw string payload over local transport.
pub async fn send_raw_rpc(socket_path: &Path, raw_payload: &str) -> Result<RpcResponse> {
    let mut payload = raw_payload.to_string();
    if !payload.ends_with('\n') {
        payload.push('\n');
    }
    let resp_str = send_raw_ipc(socket_path, &payload).await?;
    let resp: RpcResponse = serde_json::from_str(&resp_str)?;
    Ok(resp)
}

async fn send_raw_ipc(socket_path: &Path, payload: &str) -> Result<String> {
    #[cfg(unix)]
    {
        let stream = UnixStream::connect(socket_path).await?;
        let (reader, mut writer) = stream.into_split();
        let mut buf_reader = BufReader::new(reader);

        writer.write_all(payload.as_bytes()).await?;

        let mut response_line = String::new();
        buf_reader.read_line(&mut response_line).await?;
        Ok(response_line)
    }

    #[cfg(windows)]
    {
        let pipe_name = socket_path
            .to_str()
            .unwrap_or(r"\\.\pipe\LOCAL\dendro-quack");
        let client = ClientOptions::new().open(pipe_name)?;
        let (reader, mut writer) = tokio::io::split(client);
        let mut buf_reader = BufReader::new(reader);

        writer.write_all(payload.as_bytes()).await?;

        let mut response_line = String::new();
        buf_reader.read_line(&mut response_line).await?;
        Ok(response_line)
    }
}
