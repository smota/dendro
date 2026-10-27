//! Shared fixed-query helpers for reference insights.
use super::super::contract::{Measure, MeasureValue};
use duckdb::{params, Connection};

pub(super) fn table_exists(conn: &Connection, name: &str) -> bool {
    conn.query_row(
        "SELECT COUNT(*) FROM information_schema.tables WHERE lower(table_name) = lower(?)",
        params![name],
        |row| row.get::<_, i64>(0),
    )
    .unwrap_or(0)
        > 0
}

pub(super) fn column_exists(conn: &Connection, table: &str, column: &str) -> bool {
    conn.query_row(
        "SELECT COUNT(*) FROM information_schema.columns
         WHERE lower(table_name) = lower(?) AND lower(column_name) = lower(?)",
        params![table, column],
        |row| row.get::<_, i64>(0),
    )
    .unwrap_or(0)
        > 0
}

pub(super) fn count_measure(id: &str, name: &str, value: i64, unit: &str, basis: &str) -> Measure {
    Measure {
        id: id.to_string(),
        name: name.to_string(),
        value: Some(MeasureValue::Int(value)),
        unit: unit.to_string(),
        basis: basis.to_string(),
    }
}

pub(super) fn unknown_measure(id: &str, name: &str, unit: &str, basis: &str) -> Measure {
    Measure {
        id: id.to_string(),
        name: name.to_string(),
        value: None,
        unit: unit.to_string(),
        basis: basis.to_string(),
    }
}

pub(super) fn float_measure(
    id: &str,
    name: &str,
    value: Option<f64>,
    unit: &str,
    basis: &str,
) -> Measure {
    Measure {
        id: id.to_string(),
        name: name.to_string(),
        value: value.map(MeasureValue::Float),
        unit: unit.to_string(),
        basis: basis.to_string(),
    }
}

pub(super) fn disjoint_tokens(
    input_uncached: i64,
    cache_write: i64,
    cache_read: i64,
    output: i64,
) -> Option<i128> {
    let parts = [input_uncached, cache_write, cache_read, output];
    if parts.iter().any(|value| *value < 0) {
        return None;
    }
    Some(parts.iter().map(|value| i128::from(*value)).sum())
}

pub(super) fn fit_i64(sum: i128) -> Option<i64> {
    i64::try_from(sum).ok()
}
