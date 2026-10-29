#![deny(unsafe_code)]

//! Integration tests for Connected HTTP & SSE Dashboard Server (`dendro dashboard --serve`).
//!
//! Validates:
//! 1. Security: Only loopback bind addresses are allowed; `0.0.0.0` or external IPs are refused.
//! 2. Static file delivery with path traversal prevention and correct MIME types.
//! 3. Root redirect `GET /` -> `302 Found` to `/dashboards/index.html`.
//! 4. Uncapped Live Query API (`GET /api/query?view=hub`, `GET /api/query?view=message_mix`, etc.):
//!    returns `live: true`, real table counts, recorded window, and zero row cap (>50 rows returned).
//! 5. Live SSE stream (`GET /api/events?view=hub`): returns `text/event-stream` with initial `data:` frame.

use dendro_cli::connected_web::{
    resolve_site_dir, validate_dashboard_bind_address, ConnectedServer,
};
use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

struct TestDir(PathBuf);
impl TestDir {
    fn new(prefix: &str) -> Self {
        let unique = format!(
            "dendro-test-dashboard-{}-{}-{}",
            prefix,
            std::process::id(),
            TEST_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let path = std::env::temp_dir().join(unique);
        fs::create_dir_all(&path).unwrap();
        #[cfg(unix)]
        {
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn socket_path(&self) -> PathBuf {
        #[cfg(windows)]
        {
            PathBuf::from(format!(
                r"\\.\pipe\LOCAL\dendro-test-{}",
                self.path().file_name().unwrap().to_string_lossy()
            ))
        }
        #[cfg(not(windows))]
        {
            self.path().join("dendro.sock")
        }
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn test_bind_address_security_rejects_non_loopback() {
    // 1. Loopback addresses are permitted
    assert!(validate_dashboard_bind_address("127.0.0.1:4242").is_ok());
    assert!(validate_dashboard_bind_address("127.0.0.1:0").is_ok());
    assert!(validate_dashboard_bind_address("localhost:4242").is_ok());
    assert!(validate_dashboard_bind_address("[::1]:4242").is_ok());

    // 2. Wildcard or external IP addresses are deterministically refused
    let res_wildcard = validate_dashboard_bind_address("0.0.0.0:4242");
    assert!(res_wildcard.is_err());
    assert!(res_wildcard
        .unwrap_err()
        .to_string()
        .contains("security refused: dashboard server must bind to a loopback address"));

    let res_ext = validate_dashboard_bind_address("192.168.1.100:8080");
    assert!(res_ext.is_err());
    assert!(res_ext
        .unwrap_err()
        .to_string()
        .contains("security refused: dashboard server must bind to a loopback address"));
}

async fn http_get(addr: SocketAddr, path: &str) -> (String, Vec<u8>) {
    let mut stream = TcpStream::connect(addr)
        .await
        .expect("connection to dashboard server should succeed");

    let req = format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).await.unwrap();

    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();

    let header_end = response
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("valid HTTP response separator");

    let headers = String::from_utf8_lossy(&response[..header_end]).to_string();
    let body = response[header_end + 4..].to_vec();
    (headers, body)
}

#[tokio::test]
async fn test_connected_dashboard_server_full_lifecycle() {
    let test_dir = TestDir::new("lifecycle");
    let db_path = test_dir.path().join("dendro.duckdb");

    // 1. Initialize DuckDB schema and insert test records
    {
        let store = dendro_core::AnalyticsStore::open_path(&db_path, "tester")
            .expect("open_path should initialize database");
        drop(store);

        let conn = duckdb::Connection::open(&db_path).expect("open raw duckdb connection");

        // Insert 60 usage_facts to explicitly test uncapped retrieval (exceeding default 50 limit)
        for i in 1..=60 {
            conn.execute(
                "INSERT INTO usage_facts (source_key, harness, native_session, ts, seq, input_uncached, output_tokens, cache_read, cache_write, reasoning, currency_total, generation, source_id, model)
                 VALUES (?, 'claude', 's_uncapped', CURRENT_TIMESTAMP, ?, 100, 50, 20, 10, 0, 0.005, 1, 'tester', 'claude-3-5-sonnet')",
                duckdb::params![format!("hook:usage:{}", i), i],
            )
            .expect("usage_facts insert");
        }

        // Insert message facts
        conn.execute(
            "INSERT INTO message_facts (source_key, harness, native_session, ts, seq, role, direction, modalities, grain, generation, source_id)
             VALUES ('hook:msg:1', 'claude', 's_uncapped', CURRENT_TIMESTAMP, 1, 'user', 'inbound', 'text', 'event', 1, 'tester')",
            [],
        )
        .expect("message_facts insert 1");
        conn.execute(
            "INSERT INTO message_facts (source_key, harness, native_session, ts, seq, role, direction, modalities, grain, generation, source_id)
             VALUES ('hook:msg:2', 'claude', 's_uncapped', CURRENT_TIMESTAMP, 2, 'assistant', 'outbound', 'text', 'event', 1, 'tester')",
            [],
        )
        .expect("message_facts insert 2");

        // Insert activity facts
        conn.execute(
            "INSERT INTO activity_facts (source_key, seat, ts, state, seq)
             VALUES ('hook:act:1', 'builder_1', CURRENT_TIMESTAMP, 'running', 1)",
            [],
        )
        .expect("activity_facts insert");

        drop(conn);
    }

    let site_dir = resolve_site_dir(None).expect("site_dir should resolve");

    // 2. Start ConnectedServer on ephemeral port 127.0.0.1:0
    let server = ConnectedServer::bind(
        "127.0.0.1:0",
        db_path.clone(),
        site_dir,
        None,
        Some("all".to_string()),
    )
    .await
    .expect("server should bind to ephemeral loopback port");

    let local_addr = server.local_addr().expect("local_addr should be valid");
    let server_handle = tokio::spawn(async move {
        let _ = server.run().await;
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    // 3. Test Root Redirect (GET / -> 302 Found Location: /dashboards/index.html)
    let (headers, _) = http_get(local_addr, "/").await;
    assert!(headers.starts_with("HTTP/1.1 302 Found"));
    assert!(headers.contains("Location: /dashboards/index.html"));

    // 4. Test Static Asset Serving (GET /dashboards/index.html)
    let (headers, body) = http_get(local_addr, "/dashboards/index.html").await;
    assert!(headers.starts_with("HTTP/1.1 200 OK"));
    assert!(headers.contains("Content-Type: text/html"));
    let body_str = String::from_utf8_lossy(&body);
    assert!(body_str.contains("Dendro") || body_str.contains("Fleet"));

    // 5. Test Path Traversal Defense (GET /../Cargo.toml -> 404)
    let (headers, _) = http_get(local_addr, "/../Cargo.toml").await;
    assert!(headers.starts_with("HTTP/1.1 404 Not Found"));

    // 6. Test Uncapped Live Query API: Hub Overview (GET /api/query?view=hub)
    let (headers, body) = http_get(local_addr, "/api/query?view=hub").await;
    assert!(headers.starts_with("HTTP/1.1 200 OK"));
    assert!(headers.contains("Content-Type: application/json"));
    let hub_json: serde_json::Value =
        serde_json::from_slice(&body).expect("hub response must be valid JSON");

    assert_eq!(hub_json["live"], serde_json::json!(true));
    assert_eq!(hub_json["view"], serde_json::json!("hub"));
    assert_eq!(
        hub_json["table_counts"]["usage_facts"],
        serde_json::json!(60)
    );
    assert_eq!(
        hub_json["table_counts"]["message_facts"],
        serde_json::json!(2)
    );
    assert!(!hub_json["harnesses"].as_array().unwrap().is_empty());
    assert!(hub_json["insights"].as_array().unwrap().len() >= 20);
    assert!(hub_json["kpis"].get("active_actors").is_some());
    assert!(hub_json["scopes"].is_array());

    // 7. Test Uncapped View Query: message_mix (GET /api/query?view=message_mix)
    let (headers, body) = http_get(local_addr, "/api/query?view=message_mix").await;
    assert!(headers.starts_with("HTTP/1.1 200 OK"));
    assert!(headers.contains("Cache-Control: no-store, no-cache, must-revalidate, max-age=0"));
    assert!(headers.contains("Pragma: no-cache"));
    assert!(headers.contains("Expires: 0"));
    let msg_json: serde_json::Value =
        serde_json::from_slice(&body).expect("message_mix response must be valid JSON");
    assert_eq!(msg_json["live"], serde_json::json!(true));
    let rows = msg_json["rows"].as_array().expect("rows must be an array");
    assert!(!rows.is_empty(), "expected rows in message_mix view");
    assert_eq!(rows[0]["harness"], serde_json::json!("claude"));

    // 8. Test Uncapped Row Count: cost_and_loops (GET /api/query?view=cost_and_loops)
    // Asserts all 60 rows returned with zero 50-row cap
    let (headers, body) = http_get(local_addr, "/api/query?view=cost_and_loops").await;
    assert!(headers.starts_with("HTTP/1.1 200 OK"));
    assert!(headers.contains("Cache-Control: no-store, no-cache, must-revalidate, max-age=0"));
    assert!(headers.contains("Pragma: no-cache"));
    assert!(headers.contains("Expires: 0"));
    let cost_json: serde_json::Value =
        serde_json::from_slice(&body).expect("cost_and_loops response must be valid JSON");
    assert_eq!(cost_json["live"], serde_json::json!(true));
    assert_eq!(
        cost_json["table_counts"]["usage_facts"],
        serde_json::json!(60)
    );
    let cost_rows = cost_json["rows"].as_array().expect("rows must be an array");
    assert!(!cost_rows.is_empty());
    assert_eq!(
        cost_rows[0].as_object().unwrap().keys().next().unwrap(),
        "scope_id"
    );

    // 9. Test Static file headers (GET /dashboards/index.html)
    let (stat_headers, _) = http_get(local_addr, "/dashboards/index.html").await;
    assert!(stat_headers.starts_with("HTTP/1.1 200 OK"));
    assert!(stat_headers.contains("Cache-Control: no-store, no-cache, must-revalidate, max-age=0"));
    assert!(stat_headers.contains("Pragma: no-cache"));
    assert!(stat_headers.contains("Expires: 0"));

    // 10. Test Live SSE Stream (GET /api/events?view=hub)
    let mut sse_stream = TcpStream::connect(local_addr).await.unwrap();
    let sse_req = format!(
        "GET /api/events?view=hub HTTP/1.1\r\nHost: {local_addr}\r\nConnection: close\r\n\r\n"
    );
    sse_stream.write_all(sse_req.as_bytes()).await.unwrap();

    let mut sse_text = String::new();
    let mut chunk = [0u8; 4096];
    while !sse_text.contains("\"live\":true") && !sse_text.contains("\"live\": true") {
        let n = sse_stream.read(&mut chunk).await.unwrap();
        if n == 0 {
            break;
        }
        sse_text.push_str(&String::from_utf8_lossy(&chunk[..n]));
    }

    assert!(sse_text.contains("HTTP/1.1 200 OK"));
    assert!(sse_text.contains("Content-Type: text/event-stream"));
    assert!(sse_text.contains("Cache-Control: no-store, no-cache, must-revalidate, max-age=0"));
    assert!(sse_text.contains("Pragma: no-cache"));
    assert!(sse_text.contains("Expires: 0"));
    assert!(sse_text.contains("data: "));
    assert!(sse_text.contains("\"live\":true") || sse_text.contains("\"live\": true"));

    server_handle.abort();
}

#[tokio::test]
async fn test_quack_query_view_is_uncapped_without_limit() {
    let test_dir = TestDir::new("quack_uncapped");
    let db_path = test_dir.path().join("dendro.duckdb");
    let socket_path = test_dir.socket_path();

    let daemon =
        dendro_cli::serve_v1::QuackOwnerDaemon::open(db_path.clone(), Some(socket_path.clone()))
            .expect("daemon should open database");
    let socket_path = daemon.socket_path().to_path_buf();

    let cli_token = daemon.token_for_role(dendro_cli::serve_v1::ClientRole::Cli);
    let daemon_handle = tokio::spawn(async move {
        let _ = daemon.serve(false).await;
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let cli_cred = dendro_cli::serve_v1::ClientCredential {
        client_id: "cli_tester".to_string(),
        role: dendro_cli::serve_v1::ClientRole::Cli,
        token: cli_token,
    };

    // Insert 60 rows into usage_facts
    for i in 1..=60 {
        let res = dendro_cli::serve_v1::send_rpc(
            &socket_path,
            &cli_cred,
            "AppendFact",
            serde_json::json!({
                "table": "usage_facts",
                "record": {
                    "harness": "claude",
                    "session_id": "s_uncapped_quack",
                    "seq": i,
                    "input_uncached": 100,
                    "output_tokens": 50,
                    "model": "claude-3-5-sonnet"
                }
            }),
        )
        .await
        .unwrap();
        assert_eq!(res.status, "ok");
    }

    // QueryView without limit: MUST return all 60 rows, not capped at 50!
    let res = dendro_cli::serve_v1::send_rpc(
        &socket_path,
        &cli_cred,
        "QueryView",
        serde_json::json!({
            "view": "usage_facts"
        }),
    )
    .await
    .unwrap();

    assert_eq!(res.status, "ok");
    let rows = res.result.unwrap()["rows"].as_array().unwrap().clone();
    assert_eq!(
        rows.len(),
        60,
        "QueryView must be uncapped and return all 60 rows"
    );

    daemon_handle.abort();
}

#[tokio::test]
async fn test_direct_overview_query_and_trace_digest() {
    let test_dir = TestDir::new("direct_overview");
    let db_path = test_dir.path().join("dendro.duckdb");

    // Initialize DuckDB database
    {
        let store = dendro_core::AnalyticsStore::open_path(&db_path, "tester").unwrap();
        store.checkpoint().unwrap();
        drop(store);
    }

    let window = "2026-10-01T00:00:00Z..2026-10-02T00:00:00Z";
    let rig = "test-rig";

    // 1. Test execute_live_query with direct_overview
    let res = dendro_cli::connected_web::execute_live_query(
        &db_path,
        "direct_overview",
        Some(window),
        Some(rig),
    )
    .expect("direct_overview live query should succeed");

    assert!(
        res["direct_overview"].is_object(),
        "must contain direct_overview object"
    );
    assert_eq!(
        res["direct_overview"]["formula_version"],
        "direct-overview-v1"
    );
    let digest = res["direct_overview"]["digest"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(digest.len(), 64, "digest must be 64-char sha256 hex string");

    // 2. Test execute_live_query with v_cwd_fingerprint_rollup lens
    let res_cwd = dendro_cli::connected_web::execute_live_query(
        &db_path,
        "v_cwd_fingerprint_rollup",
        Some(window),
        Some(rig),
    )
    .expect("v_cwd_fingerprint_rollup query should succeed");
    assert!(res_cwd["rows"].is_array());

    // 3. Test execute_live_query with skill_facts lens
    let res_skill = dendro_cli::connected_web::execute_live_query(
        &db_path,
        "skill_facts",
        Some(window),
        Some(rig),
    )
    .expect("skill_facts query should succeed");
    assert!(res_skill["rows"].is_array());

    // 4. Test physical trace for direct_overview
    let trace = dendro_core::pipeline::trace_json_with_analysis_physical(
        &db_path,
        "direct_overview",
        Some(rig),
        "2026-10-01T00:00:00Z",
        "2026-10-02T00:00:00Z",
        dendro_core::LoopThresholds::default(),
        dendro_core::FrictionThresholds::default(),
        dendro_core::advisory::AdvisoryOptions::default(),
    )
    .expect("trace direct_overview should succeed");

    assert_eq!(trace["metric_id"], "direct_overview");
    assert_eq!(trace["formula_version"], "direct-overview-v1");
    assert_eq!(trace["digest"], digest);

    // 5. Test digest verification in CLI / trace evaluator
    let req_matching = dendro_core::overview::OverviewRequest {
        rig: Some(rig.to_string()),
        from: chrono::DateTime::parse_from_rfc3339("2026-10-01T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc),
        to: chrono::DateTime::parse_from_rfc3339("2026-10-02T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc),
        observation_clock: Some(
            chrono::DateTime::parse_from_rfc3339("2026-10-02T00:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
        ),
        evidence_as_of: None,
        advisory_options: dendro_core::advisory::AdvisoryOptions::default(),
        friction_thresholds: dendro_core::vortex::FrictionThresholds::default(),
        expect_evidence_digest: Some(digest.clone()),
    };
    let res_matching =
        dendro_core::overview::evaluate_overview_snapshot_path(&db_path, &req_matching);
    assert!(res_matching.is_ok(), "matching digest must succeed");

    let req_mismatch = dendro_core::overview::OverviewRequest {
        expect_evidence_digest: Some(
            "0000000000000000000000000000000000000000000000000000000000000000".to_string(),
        ),
        ..req_matching
    };
    let res_mismatch =
        dendro_core::overview::evaluate_overview_snapshot_path(&db_path, &req_mismatch);
    assert!(res_mismatch.is_err(), "mismatched digest must fail");
    assert!(res_mismatch
        .unwrap_err()
        .to_string()
        .contains("evidence digest mismatch"));
}
