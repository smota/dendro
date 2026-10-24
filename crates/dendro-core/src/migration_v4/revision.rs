//! Schema4 revision cutoff queries. Legacy migration does not insert these rows.

use duckdb::{params, Connection};

use super::{open_rw, MigrationError};

#[derive(Debug, Clone, PartialEq)]
pub struct FactRevision {
    pub fact: String,
    pub revision: i64,
    pub ingested: i64,
    pub quantity: Option<f64>,
    pub retracted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevisionState {
    Present,
    Retracted,
}

pub fn revision_count(db: &std::path::Path) -> Result<i64, MigrationError> {
    let conn = super::open_ro(db)?;
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM dendro_fact_revisions", [], |row| {
        row.get(0)
    })?;
    conn.verify_runtime()?;
    Ok(count)
}

pub fn insert_fact_revisions(
    db: &std::path::Path,
    rows: &[FactRevision],
) -> Result<(), MigrationError> {
    let conn = open_rw(db)?;
    require_schema4(&conn)?;
    let tx = conn.unchecked_transaction()?;
    for row in rows {
        let state = if row.retracted {
            "retracted"
        } else {
            "present"
        };
        conn.execute(
            "INSERT INTO dendro_fact_revisions (fact_id, revision, ingested, quantity, retracted, state)
             VALUES (?, ?, ?, ?, ?, ?)",
            params![
                row.fact,
                row.revision,
                row.ingested,
                row.quantity,
                row.retracted,
                state
            ],
        )?;
    }
    tx.commit()?;
    conn.verify_runtime()?;
    Ok(())
}

pub fn query_revision_cutoff(
    db: &std::path::Path,
    fact: &str,
    ingested_cutoff: i64,
) -> Result<(Option<f64>, RevisionState), MigrationError> {
    let conn = super::open_ro(db)?;
    let mut stmt = conn.prepare(
        "SELECT quantity, retracted FROM dendro_fact_revisions
         WHERE fact_id = ? AND ingested <= ?
         ORDER BY revision DESC
         LIMIT 1",
    )?;
    let mut rows = stmt.query(params![fact, ingested_cutoff])?;
    let Some(row) = rows.next()? else {
        return Err(MigrationError::Refused(super::Refusal {
            code: "revision_cutoff_unknown",
            source_mutated: false,
            pointer_changed: false,
            stage_retained: true,
            detail: "no stored revision at or before the cutoff; migration does not synthesize one"
                .into(),
        }));
    };
    let quantity: Option<f64> = row.get(0)?;
    let retracted: bool = row.get(1)?;
    drop(rows);
    drop(stmt);
    conn.verify_runtime()?;
    if retracted {
        Ok((None, RevisionState::Retracted))
    } else {
        Ok((quantity, RevisionState::Present))
    }
}

fn require_schema4(conn: &Connection) -> Result<(), MigrationError> {
    let version: i32 = conn.query_row("SELECT schema_version FROM dendro_meta", [], |row| {
        row.get(0)
    })?;
    if version != super::MIGRATION_TARGET_SCHEMA_VERSION {
        return Err(MigrationError::Refused(super::Refusal {
            code: "revision_write_requires_schema4",
            source_mutated: false,
            pointer_changed: false,
            stage_retained: true,
            detail: format!("schema {version}"),
        }));
    }
    Ok(())
}
