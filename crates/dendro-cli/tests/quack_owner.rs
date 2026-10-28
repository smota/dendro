#![deny(unsafe_code)]

//! `A03` Proof Suite — Single-Owner Daemon (`dendro serve`) & Typed Capability Transport.
//!
//! Validates:
//! 1. The daemon is the only raw DuckDB or `quack` session; a second direct writer is refused while the owner holds the lock.
//! 2. External clients (CLI, TUI, HTTP, Nao, and same-user Hook subprocesses) receive no raw SQL session.
//! 3. Same-user Hook process is restricted to typed append/advise capabilities; any raw SQL string is rejected.
//! 4. Token is 0600 on disk and absent from logs, responses, and client credentials.
//! 5. Unix socket is 0700 inside a 0700 private directory; binding any loopback TCP listener (127.0.0.1 or 0.0.0.0) is rejected.
//! 6. File mode 0700 alone must not be the assertion that a same-user child is contained.

use dendro_cli::serve_v1::{
    send_raw_rpc, send_rpc, validate_bind_address, validate_socket_dir, ClientCredential,
    ClientRole, QuackOwnerDaemon, QuackToken, TransportError,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

struct TestDir(PathBuf);
impl TestDir {
    fn new(_prefix: &str) -> Self {
        let unique = format!(
            "dq-{}-{}",
            std::process::id(),
            TEST_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        // CI's TMPDIR can already consume most of sockaddr_un.sun_path.
        // Keep synthetic Unix sockets short, private and cleaned by Drop.
        #[cfg(unix)]
        let path = Path::new("/tmp").join(unique);
        #[cfg(not(unix))]
        let path = std::env::temp_dir().join(unique);
        fs::create_dir(&path).unwrap();
        #[cfg(unix)]
        {
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(unix)]
#[test]
fn test_socket_fixture_path_fits_portable_unix_limit() {
    let test_dir = TestDir::new("numeric_serialization");
    let socket = test_dir.path().join("dendro.sock");
    assert!(socket.as_os_str().as_encoded_bytes().len() < 104);
}

#[tokio::test]
async fn test_single_owner_daemon_locks_database() {
    let test_dir = TestDir::new("owner_lock");
    let db_path = test_dir.path().join("dendro.duckdb");
    let socket_path = test_dir.path().join("dendro.sock");

    // 1. Daemon starts and holds the raw exclusive DuckDB session
    let daemon = QuackOwnerDaemon::open(db_path.clone(), Some(socket_path.clone()))
        .expect("daemon should open database");

    // 2. A second direct writer attempting to open the raw DuckDB database must be refused
    let second_writer = dendro_core::AnalyticsStore::open_path(&db_path, "second_writer");
    assert!(
        second_writer.is_err(),
        "second direct writer must be refused while single-owner daemon holds the lock"
    );

    let err_str = second_writer.err().unwrap().to_string();
    assert!(
        err_str.contains("key id does not match")
            || err_str.contains("lock")
            || err_str.contains("Resource temporarily unavailable")
            || err_str.contains("IO Error")
            || err_str.contains("Could not set lock"),
        "expected refusal error, got: {err_str}"
    );

    drop(daemon);
}

#[tokio::test]
async fn test_transport_bind_refuses_loopback_tcp() {
    // Unauthenticated loopback TCP is strictly forbidden
    assert!(matches!(
        validate_bind_address("127.0.0.1:8080"),
        Err(TransportError::LoopbackTcpForbidden)
    ));
    assert!(matches!(
        validate_bind_address("0.0.0.0:8080"),
        Err(TransportError::LoopbackTcpForbidden)
    ));
    assert!(matches!(
        validate_bind_address("localhost:9000"),
        Err(TransportError::LoopbackTcpForbidden)
    ));
    assert!(matches!(
        validate_bind_address("tcp://127.0.0.1:4000"),
        Err(TransportError::LoopbackTcpForbidden)
    ));
    assert!(matches!(
        validate_bind_address("allow_other_hostname=true"),
        Err(TransportError::AllowOtherHostnameForbidden)
    ));
}

#[tokio::test]
async fn test_socket_directory_permissions_and_token_sidecar() {
    let test_dir = TestDir::new("socket_perms");
    let dir = test_dir.path();

    // Verify 0700 socket directory is accepted
    assert!(validate_socket_dir(dir).is_ok());

    #[cfg(unix)]
    {
        // A non-private directory (e.g. 0755 with group/other readable) must be refused
        let bad_dir = dir.join("bad_dir");
        fs::create_dir(&bad_dir).unwrap();
        fs::set_permissions(&bad_dir, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(matches!(
            validate_socket_dir(&bad_dir),
            Err(TransportError::InsecureDirectoryPermissions(_))
        ));
    }

    // Token sidecar verification
    let token = QuackToken::generate_and_save(dir).expect("token generation should succeed");
    assert_eq!(token.len(), 64); // 32 bytes hex encoded

    let token_path = dir.join("quack.token");
    assert!(token_path.exists());

    #[cfg(unix)]
    {
        let mode = fs::metadata(&token_path).unwrap().permissions().mode();
        assert_eq!(
            mode & 0o777,
            0o600,
            "token file must have mode 0600 on disk, found {mode:#o}"
        );
    }

    let read_back = QuackToken::read_token(dir).expect("reading token should succeed");
    assert_eq!(read_back, token);
}

#[tokio::test]
async fn test_same_user_containment_and_raw_sql_rejection() {
    let test_dir = TestDir::new("containment");
    let db_path = test_dir.path().join("dendro.duckdb");
    let socket_path = test_dir.path().join("dendro.sock");

    let daemon = QuackOwnerDaemon::open(db_path.clone(), Some(socket_path.clone()))
        .expect("daemon should start");

    let hook_token = daemon.token_for_role(ClientRole::Hook);
    let cli_token = daemon.token_for_role(ClientRole::Cli);

    // Spawn daemon server in background
    tokio::spawn(async move {
        let _ = daemon.serve(false).await;
    });

    // Give server a moment to bind
    tokio::time::sleep(Duration::from_millis(50)).await;

    // 1. Same-user Hook credential
    let hook_cred = ClientCredential {
        client_id: "agent_hook_worker".to_string(),
        role: ClientRole::Hook,
        token: hook_token.clone(),
    };

    // Assert root token is NOT in client credential
    let root_token = QuackToken::read_token(test_dir.path()).unwrap();
    let cred_json = serde_json::to_string(&hook_cred).unwrap();
    assert!(!cred_json.contains(&root_token));

    // 2. Hook can call AppendFact (permitted capability)
    let append_res = send_rpc(
        &socket_path,
        &hook_cred,
        "AppendFact",
        serde_json::json!({
            "table": "skill_facts",
            "record": {
                "seat": "builder_1",
                "skill": "test-driven-development",
                "invoked": true
            }
        }),
    )
    .await
    .expect("RPC delivery should succeed");

    assert_eq!(append_res.status, "ok");
    assert!(append_res.error.is_none());

    // 3. Hook can call Advise (permitted capability)
    let advise_res = send_rpc(
        &socket_path,
        &hook_cred,
        "Advise",
        serde_json::json!({
            "question": "delegation_loop_risk"
        }),
    )
    .await
    .expect("RPC delivery should succeed");

    assert_eq!(advise_res.status, "ok");
    assert!(advise_res.error.is_none());

    // 4. Hook calling QueryView is REFUSED (Hook restricted strictly to AppendFact and Advise)
    let query_res = send_rpc(
        &socket_path,
        &hook_cred,
        "QueryView",
        serde_json::json!({"view_name": "v_tokens"}),
    )
    .await
    .expect("RPC delivery should succeed");

    assert_eq!(query_res.status, "refused");
    assert!(query_res
        .error
        .as_ref()
        .unwrap()
        .contains("hook_restricted_to_append_and_advise"));

    // 5. Hook calling Status is REFUSED
    let status_res = send_rpc(&socket_path, &hook_cred, "Status", serde_json::json!({}))
        .await
        .expect("RPC delivery should succeed");

    assert_eq!(status_res.status, "refused");
    assert!(status_res
        .error
        .as_ref()
        .unwrap()
        .contains("hook_restricted_to_append_and_advise"));

    // 6. Cryptographic role token forgery rejection
    let forged_cli_cred = ClientCredential {
        client_id: "sneaky_hook".to_string(),
        role: ClientRole::Cli,
        token: hook_token.clone(), // Hook token presented for Cli role
    };
    let forge_res = send_rpc(
        &socket_path,
        &forged_cli_cred,
        "QueryView",
        serde_json::json!({"view_name": "v_tokens"}),
    )
    .await
    .expect("RPC delivery should succeed");
    assert_eq!(forge_res.status, "refused");
    assert!(forge_res
        .error
        .as_ref()
        .unwrap()
        .contains("invalid_or_forged_credential"));

    let invalid_token_cred = ClientCredential {
        client_id: "bad_token_user".to_string(),
        role: ClientRole::Cli,
        token: "deadbeef00000000deadbeef00000000deadbeef00000000deadbeef00000000".to_string(),
    };
    let invalid_res = send_rpc(
        &socket_path,
        &invalid_token_cred,
        "QueryView",
        serde_json::json!({"view_name": "v_tokens"}),
    )
    .await
    .expect("RPC delivery should succeed");
    assert_eq!(invalid_res.status, "refused");
    assert!(invalid_res
        .error
        .as_ref()
        .unwrap()
        .contains("invalid_or_forged_credential"));

    // 7. Any client sending raw SQL strings is deterministically rejected with raw_sql_forbidden!
    let cli_cred = ClientCredential {
        client_id: "cli_user".to_string(),
        role: ClientRole::Cli,
        token: cli_token,
    };

    let forbidden_queries = [
        "SELECT * FROM usage_facts",
        "ATTACH 'quack:test' AS quack_db",
        "PRAGMA table_info('usage_facts')",
        "COPY usage_facts TO '/tmp/leak.csv'",
        "INSERT INTO usage_facts VALUES (1, 2)",
        "DROP TABLE source_files",
    ];

    for raw_sql in forbidden_queries {
        let sql_res = send_rpc(
            &socket_path,
            &cli_cred,
            "AppendFact",
            serde_json::json!({
                "table": "skill_facts",
                "record": {
                    "query": raw_sql
                }
            }),
        )
        .await
        .expect("RPC delivery should succeed");

        assert_eq!(sql_res.status, "refused");
        assert!(
            sql_res
                .error
                .as_ref()
                .unwrap()
                .contains("raw_sql_forbidden"),
            "expected raw_sql_forbidden for {raw_sql}"
        );
    }

    // 8. Test raw JSON payload attempting to invoke raw_sql or borrow a session
    let raw_attack = serde_json::json!({
        "client_credential": {
            "client_id": "attacker",
            "role": "hook",
            "token": hook_token
        },
        "method": "raw_sql",
        "params": {
            "sql": "SELECT 1"
        }
    });

    let attack_res = send_raw_rpc(&socket_path, &raw_attack.to_string())
        .await
        .expect("raw RPC should succeed");
    assert_eq!(attack_res.status, "refused");
    assert!(attack_res
        .error
        .as_ref()
        .unwrap()
        .contains("raw_sql_forbidden"));

    // Verify token is absent from all responses
    let all_responses = [append_res, advise_res, query_res, status_res, attack_res];
    let token = QuackToken::read_token(test_dir.path()).unwrap();
    for r in &all_responses {
        let serialized = serde_json::to_string(r).unwrap();
        assert!(
            !serialized.contains(&token),
            "token must NEVER appear in any response!"
        );
    }
}

#[tokio::test]
async fn test_query_view_numeric_column_serialization() {
    let test_dir = TestDir::new("numeric_serialization");
    let db_path = test_dir.path().join("dendro.duckdb");
    let socket_path = test_dir.path().join("dendro.sock");

    let daemon = QuackOwnerDaemon::open(db_path.clone(), Some(socket_path.clone()))
        .expect("daemon should open database");

    let cli_token = daemon.token_for_role(ClientRole::Cli);
    let daemon_handle = tokio::spawn(async move {
        let _ = daemon.serve(false).await;
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let cli_cred = ClientCredential {
        client_id: "cli_tester".to_string(),
        role: ClientRole::Cli,
        token: cli_token,
    };

    // Append usage_facts with numeric / float / bigint fields
    let append_res = send_rpc(
        &socket_path,
        &cli_cred,
        "AppendFact",
        serde_json::json!({
            "table": "usage_facts",
            "record": {
                "harness": "claude",
                "session_id": "s_numeric_test",
                "seq": 42,
                "input_uncached": 1234,
                "output_tokens": 567,
                "cache_read": 89,
                "cache_write": 12,
                "currency_total": 0.0456,
                "model": "claude-3-5-sonnet"
            }
        }),
    )
    .await
    .expect("AppendFact RPC delivery should succeed");
    assert_eq!(append_res.status, "ok");

    // QueryView on usage_facts
    let query_res = send_rpc(
        &socket_path,
        &cli_cred,
        "QueryView",
        serde_json::json!({
            "view": "usage_facts",
            "limit": 10
        }),
    )
    .await
    .expect("QueryView RPC delivery should succeed");
    assert_eq!(query_res.status, "ok");

    let rows = query_res.result.unwrap()["rows"]
        .as_array()
        .unwrap()
        .clone();
    assert!(
        !rows.is_empty(),
        "expected at least one row in usage_facts query view"
    );

    let row = &rows[0];
    assert_eq!(row["seq"], serde_json::json!(42));
    assert_eq!(row["input_uncached"], serde_json::json!(1234));
    assert_eq!(row["output_tokens"], serde_json::json!(567));
    assert_eq!(row["cache_read"], serde_json::json!(89));
    assert_eq!(row["cache_write"], serde_json::json!(12));
    assert_eq!(row["currency_total"], serde_json::json!(0.0456));
    assert_eq!(row["harness"], serde_json::json!("claude"));
    assert_eq!(row["native_session"], serde_json::json!("s_numeric_test"));

    daemon_handle.abort();
}

#[tokio::test]
async fn test_quack_append_fact_required_fields_validation() {
    let test_dir = TestDir::new("req_fields");
    let db_path = test_dir.path().join("dendro.duckdb");
    let socket_path = test_dir.path().join("dendro.sock");

    let daemon = QuackOwnerDaemon::open(db_path.clone(), Some(socket_path.clone()))
        .expect("daemon should open database");

    let cli_token = daemon.token_for_role(ClientRole::Cli);
    let daemon_handle = tokio::spawn(async move {
        let _ = daemon.serve(false).await;
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let cli_cred = ClientCredential {
        client_id: "cli_tester".to_string(),
        role: ClientRole::Cli,
        token: cli_token,
    };

    // 1. tool_facts missing seq is rejected with error
    let res = send_rpc(
        &socket_path,
        &cli_cred,
        "AppendFact",
        serde_json::json!({
            "table": "tool_facts",
            "record": {
                "harness": "claude",
                "session_id": "s_tool_test",
                "tool_name": "read_file",
                "structured_write": false,
                "category": "fs"
            }
        }),
    )
    .await
    .unwrap();
    assert_eq!(res.status, "error");
    assert!(res.error.unwrap().contains("missing required field seq"));

    // 2. tool_facts with seq succeeds
    let res = send_rpc(
        &socket_path,
        &cli_cred,
        "AppendFact",
        serde_json::json!({
            "table": "tool_facts",
            "record": {
                "harness": "claude",
                "session_id": "s_tool_test",
                "tool_name": "read_file",
                "structured_write": false,
                "category": "fs",
                "seq": 1
            }
        }),
    )
    .await
    .unwrap();
    assert_eq!(res.status, "ok");

    // 3. usage_facts missing seq is rejected with error
    let res = send_rpc(
        &socket_path,
        &cli_cred,
        "AppendFact",
        serde_json::json!({
            "table": "usage_facts",
            "record": {
                "harness": "claude",
                "session_id": "s_usage_test",
                "input_uncached": 100,
                "output_tokens": 50
            }
        }),
    )
    .await
    .unwrap();
    assert_eq!(res.status, "error");
    assert!(res.error.unwrap().contains("missing required field seq"));

    // 4. security_facts missing source_field is rejected with error
    let res = send_rpc(
        &socket_path,
        &cli_cred,
        "AppendFact",
        serde_json::json!({
            "table": "security_facts",
            "record": {
                "harness": "claude",
                "session_id": "s_sec_test",
                "kind": "sandbox_policy",
                "value": "read_only"
            }
        }),
    )
    .await
    .unwrap();
    assert_eq!(res.status, "error");
    assert!(res
        .error
        .unwrap()
        .contains("missing required field source_field"));

    // 5. security_facts with source_field succeeds
    let res = send_rpc(
        &socket_path,
        &cli_cred,
        "AppendFact",
        serde_json::json!({
            "table": "security_facts",
            "record": {
                "harness": "claude",
                "session_id": "s_sec_test",
                "kind": "sandbox_policy",
                "value": "read_only",
                "source_field": "permission_boundary"
            }
        }),
    )
    .await
    .unwrap();
    assert_eq!(res.status, "ok");

    daemon_handle.abort();
}
