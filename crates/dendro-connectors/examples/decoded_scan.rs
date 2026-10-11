//! Dev-only proof tool, not part of the dendro CLI: opens an analytics
//! database read-only, lists every VARCHAR column from information_schema and
//! prints, as JSON, how many decoded values contain each search string.
//!
//! Usage: cargo run -q -p dendro-connectors --example decoded_scan -- <db> <needle>...

use duckdb::{AccessMode, Config, Connection};
use serde_json::json;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((db, needles)) = args.split_first() else {
        eprintln!("usage: decoded_scan <db> <needle>...");
        std::process::exit(2);
    };
    let config = Config::default()
        .access_mode(AccessMode::ReadOnly)
        .expect("read-only config");
    let conn = Connection::open_with_flags(db, config).expect("open read-only");
    let mut stmt = conn
        .prepare(
            "SELECT table_name, column_name FROM information_schema.columns \
             WHERE data_type = 'VARCHAR' ORDER BY table_name, column_name",
        )
        .expect("schema query");
    let columns: Vec<(String, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("schema rows")
        .map(|row| row.expect("schema row"))
        .collect();
    let mut rows = Vec::new();
    for (table, column) in &columns {
        let values: i64 = conn
            .query_row(&format!("SELECT COUNT({column}) FROM {table}"), [], |row| {
                row.get(0)
            })
            .expect("count");
        let mut hits = serde_json::Map::new();
        for needle in needles {
            let found: i64 = conn
                .query_row(
                    &format!("SELECT COUNT(*) FROM {table} WHERE {column} LIKE '%' || ? || '%'"),
                    [needle],
                    |row| row.get(0),
                )
                .expect("scan");
            hits.insert(needle.clone(), json!(found));
        }
        rows.push(json!({"column": format!("{table}.{column}"), "values": values, "hits": hits}));
    }
    println!("{}", json!({"columns": rows}));
}
