//! Probe compiled against the immutable historical core, never the candidate core.
#![deny(unsafe_code)]
use dendro_core::AnalyticsStore;
use duckdb::{AccessMode, Config, Connection};
use serde_json::json;
use std::path::Path;

fn snapshot(path: &Path) -> anyhow::Result<serde_json::Value> {
    let conn =
        Connection::open_with_flags(path, Config::default().access_mode(AccessMode::ReadOnly)?)?;
    let mut tables = conn.prepare("SELECT table_name FROM information_schema.tables WHERE table_schema = 'main' AND table_type = 'BASE TABLE' ORDER BY table_name")?;
    let names = tables
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    let mut content = serde_json::Map::new();
    for name in names {
        let quoted = name.replace('"', "\"\"");
        let mut rows = conn.prepare(&format!(
            "SELECT to_json(t)::VARCHAR FROM \"{quoted}\" t ORDER BY to_json(t)::VARCHAR"
        ))?;
        let values = rows
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        let mut columns = conn.prepare("SELECT column_name, data_type, is_nullable FROM information_schema.columns WHERE table_schema = 'main' AND table_name = ? ORDER BY ordinal_position")?;
        let schema = columns
            .query_map([&name], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        content.insert(name, json!({"columns": schema, "rows": values}));
    }
    Ok(json!(content))
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mode = args
        .get(1)
        .ok_or_else(|| anyhow::anyhow!("mode required"))?;
    let path = Path::new(
        args.get(2)
            .ok_or_else(|| anyhow::anyhow!("database required"))?,
    );
    let before = snapshot(path)?;
    if mode == "snapshot" {
        println!("{before}");
        return Ok(());
    }
    let key_id = {
        let conn = Connection::open_with_flags(
            path,
            Config::default().access_mode(AccessMode::ReadOnly)?,
        )?;
        conn.query_row("SELECT key_id FROM dendro_meta", [], |row| {
            row.get::<_, String>(0)
        })?
    };
    let result = match mode.as_str() {
        "open_existing" => AnalyticsStore::open_existing(path, &key_id),
        "open_staged" => AnalyticsStore::open_staged(path),
        "open_read_only" => AnalyticsStore::open_read_only(path),
        _ => anyhow::bail!("unknown mode"),
    };
    let refused = result.is_err();
    let error = result.err().map(|error| error.to_string());
    let after = snapshot(path)?;
    let unchanged = before == after;
    println!(
        "{}",
        json!({"mode": mode, "historical_schema": dendro_core::SCHEMA_VERSION, "refused": refused, "error": error, "canonical_tables_columns_rows_unchanged": unchanged, "canonical_before": before, "canonical_after": after})
    );
    anyhow::ensure!(
        refused && unchanged,
        "historical refusal/no-canonical-write requirement failed"
    );
    Ok(())
}
