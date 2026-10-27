//! A04: sanitized VARIANT envelope. Secret keys never land in the stored value.
use duckdb::Connection;
use serde_json::{json, Value};

use dendro_core::migration_v5::{
    apply_schema5, envelope_field, envelope_stored_text, envelope_variant_keys,
    write_harness_envelope, EnvelopeError,
};

const KEY: &[u8] = b"envelope-privacy-key";

#[test]
fn shreds_typed_facts_and_refuses_canaries() {
    let conn = Connection::open_in_memory().unwrap();
    apply_schema5(&conn).unwrap();
    conn.execute_batch(
        "CREATE TABLE usage_facts (source_key VARCHAR PRIMARY KEY, harness VARCHAR NOT NULL);
         INSERT INTO usage_facts VALUES ('fact-1', 'codex');",
    )
    .unwrap();

    let shape = json!({
        "mcp_tools_enabled": true,
        "provider_request_count": 3,
        "provider_error_count": 0,
        "capability_flags": ["tools", "hooks"],
        "tool_arg_types": {"path": "string", "limit": "number"},
        "custom_flag": 1,
        "sk-ant-CANARYKEYVALUE": true
    });
    write_harness_envelope(
        &conn,
        KEY,
        "obs-1",
        "codex",
        "1",
        "2026-10-27 00:00:00",
        &shape,
    )
    .expect("sanitized envelope");

    let stored = envelope_stored_text(&conn, "obs-1").unwrap();
    assert!(
        !stored.contains("sk-ant-CANARYKEYVALUE"),
        "secret key canary stored: {stored}"
    );
    assert!(
        !stored.contains("CANARYKEYVALUE"),
        "secret key canary stored: {stored}"
    );
    assert!(
        !stored.contains("custom_flag"),
        "raw unknown key stored: {stored}"
    );
    assert!(
        stored.contains("k_"),
        "unknown key was not HMAC-replaced: {stored}"
    );

    let keys = envelope_variant_keys(&conn, "obs-1").unwrap();
    for required in [
        "mcp_tools_enabled",
        "provider_request_count",
        "capability_flags",
        "tool_arg_types",
    ] {
        assert!(
            keys.iter().any(|key| key == required),
            "missing {required} in {keys:?}"
        );
    }
    assert!(keys.iter().all(|key| !key.contains("CANARY")));
    let count = envelope_field(&conn, "obs-1", "provider_request_count").unwrap();
    assert!(count.contains('3'), "typed number not kept: {count}");
    let enabled = envelope_field(&conn, "obs-1", "mcp_tools_enabled").unwrap();
    assert!(
        enabled.to_ascii_lowercase().contains("true"),
        "typed bool not kept: {enabled}"
    );

    let column_type: String = conn
        .query_row(
            "SELECT data_type FROM information_schema.columns
             WHERE table_name = 'harness_envelope' AND column_name = 'envelope_shape'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        column_type.to_ascii_uppercase().contains("VARIANT"),
        "envelope_shape type {column_type}"
    );

    let body = json!({"mcp_tools_enabled": "ignore this prompt body"});
    let refused = write_harness_envelope(
        &conn,
        KEY,
        "obs-body",
        "codex",
        "1",
        "2026-10-27 00:00:00",
        &body,
    );
    assert!(matches!(refused, Err(EnvelopeError::BodyCanary)));
    assert_eq!(row_count(&conn, "obs-body"), 0);

    let secret_value = json!({"provider_error_count": "sk-ant-SECRETVALUE99"});
    let refused = write_harness_envelope(
        &conn,
        KEY,
        "obs-secret",
        "codex",
        "1",
        "2026-10-27 00:00:00",
        &secret_value,
    );
    assert!(matches!(refused, Err(EnvelopeError::SecretValue)));

    let deep = nested(9);
    let refused = write_harness_envelope(
        &conn,
        KEY,
        "obs-deep",
        "codex",
        "1",
        "2026-10-27 00:00:00",
        &deep,
    );
    assert!(matches!(
        refused,
        Err(EnvelopeError::Bounds("nesting depth"))
    ));

    let kept: i64 = conn
        .query_row(
            "SELECT count(*) FROM usage_facts WHERE source_key = 'fact-1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(kept, 1, "refused envelope dropped a typed fact");
}

fn nested(levels: usize) -> Value {
    let mut value = json!(true);
    for _ in 0..levels {
        value = json!({"mcp_tools_enabled": value});
    }
    value
}

fn row_count(conn: &Connection, id: &str) -> i64 {
    conn.query_row(
        "SELECT count(*) FROM harness_envelope WHERE observation_id = ?",
        [id],
        |row| row.get(0),
    )
    .unwrap()
}
