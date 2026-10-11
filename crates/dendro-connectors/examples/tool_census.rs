//! Dev-only proof tool, not part of the dendro CLI: opens an analytics
//! database read-only and prints, as JSON, per harness the number of stored
//! tool calls, their outcomes (failed, succeeded, unknown) and how many carry
//! an input fingerprint. Counts only; no stored value is printed.
//!
//! Usage: cargo run -q -p dendro-connectors --example tool_census -- <db>

use duckdb::{AccessMode, Config, Connection};
use serde_json::json;

fn main() {
    let Some(db) = std::env::args().nth(1) else {
        eprintln!("usage: tool_census <db>");
        std::process::exit(2);
    };
    let config = Config::default()
        .access_mode(AccessMode::ReadOnly)
        .expect("read-only config");
    let conn = Connection::open_with_flags(db, config).expect("open read-only");
    let mut stmt = conn
        .prepare(
            "SELECT harness, COUNT(*), \
                    COUNT(*) FILTER (WHERE failed = TRUE), \
                    COUNT(*) FILTER (WHERE failed = FALSE), \
                    COUNT(*) FILTER (WHERE failed IS NULL), \
                    COUNT(fingerprint), \
                    COUNT(*) FILTER (WHERE structured_write) \
             FROM tool_facts GROUP BY harness ORDER BY harness",
        )
        .expect("census query");
    let rows: Vec<serde_json::Value> = stmt
        .query_map([], |row| {
            Ok(json!({
                "harness": row.get::<_, String>(0)?,
                "calls": row.get::<_, i64>(1)?,
                "failed": row.get::<_, i64>(2)?,
                "succeeded": row.get::<_, i64>(3)?,
                "unknown": row.get::<_, i64>(4)?,
                "fingerprinted": row.get::<_, i64>(5)?,
                "structured_writes": row.get::<_, i64>(6)?,
            }))
        })
        .expect("census rows")
        .map(|row| row.expect("census row"))
        .collect();
    println!("{}", json!({ "tool_facts": rows }));
}
