//! Rollback selects the preserved schema3 bundle. It does not rewrite schema4.

use std::path::{Path, PathBuf};

use duckdb::params;

use super::{open_ro, sha256_file, MigrationError, Refusal, MIGRATION_TARGET_SCHEMA_VERSION};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Watermark {
    pub source_id: String,
    pub generation: i64,
    pub byte_offset: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct V4OnlyFact {
    pub id: String,
    pub ts: String,
    pub source_id: String,
    pub generation: i64,
    pub byte_range_half_open: [i64; 2],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRange {
    pub source_id: String,
    pub generation: i64,
    pub byte_range_half_open: [i64; 2],
}

/// Missing tables leave provenance unknown; an observed empty table is evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvidenceScope {
    Absent,
    Empty,
    Observed,
}

fn evidence_scope(present: bool, rows: usize) -> EvidenceScope {
    if !present {
        EvidenceScope::Absent
    } else if rows == 0 {
        EvidenceScope::Empty
    } else {
        EvidenceScope::Observed
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LossReceipt {
    pub selected_bundle: &'static str,
    pub destructive_downmigration: bool,
    pub failed_schema4_retained_private: bool,
    pub failed_schema4_path: PathBuf,
    pub preserved_schema3_path: PathBuf,
    /// Singular convenience values exist only when the preserved scope is unique.
    pub last_common_watermark: Option<Watermark>,
    pub last_common_cutoff: Option<String>,
    pub last_common_watermarks: Vec<Watermark>,
    /// Distinct stored destination cutoffs, with no inferred source association.
    pub last_common_cutoffs: Vec<String>,
    pub watermark_scope: EvidenceScope,
    pub cutoff_scope: EvidenceScope,
    pub legacy_usage_scope: EvidenceScope,
    pub v4_only_membership_verified: bool,
    pub excluded_fact_count: usize,
    pub excluded_windows: Vec<(String, String)>,
    pub excluded_source_ranges: Vec<SourceRange>,
    pub replay_available: bool,
    pub full_continuity_claim: bool,
    pub unresolved_loss: bool,
    pub publication_code: &'static str,
    pub pointer_changed: bool,
}

pub fn record_v4_only_fact(schema4_db: &Path, fact: &V4OnlyFact) -> Result<(), MigrationError> {
    if fact.byte_range_half_open[0] > fact.byte_range_half_open[1] {
        return Err(MigrationError::Refused(Refusal {
            code: "inverted_source_range",
            source_mutated: false,
            pointer_changed: false,
            stage_retained: true,
            detail: fact.id.clone(),
        }));
    }
    let conn = super::open_rw(schema4_db)?;
    let version: i32 = conn.query_row("SELECT schema_version FROM dendro_meta", [], |row| {
        row.get(0)
    })?;
    if version != MIGRATION_TARGET_SCHEMA_VERSION {
        return Err(MigrationError::Refused(Refusal {
            code: "v4_fact_requires_schema4",
            source_mutated: false,
            pointer_changed: false,
            stage_retained: true,
            detail: format!("schema {version}"),
        }));
    }
    conn.execute(
        "INSERT INTO dendro_v4_only_facts (fact_id, ts, source_id, generation, range_start, range_end)
         VALUES (?, ?, ?, ?, ?, ?)",
        params![
            fact.id,
            fact.ts,
            fact.source_id,
            fact.generation,
            fact.byte_range_half_open[0],
            fact.byte_range_half_open[1]
        ],
    )?;
    Ok(())
}

/// Evidence-backed rollback. Schema4 is opened read-only. Schema3 is opened read-only.
/// Neither file is rewritten. Consumer publication stays refused.
pub fn rollback_to_preserved_schema3(
    preserved_schema3: &Path,
    failed_schema4: &Path,
    logs_present: bool,
) -> Result<LossReceipt, MigrationError> {
    let schema3_before = sha256_file(preserved_schema3)?;
    let schema4_before = sha256_file(failed_schema4)?;
    let source = open_ro(preserved_schema3)?;
    let version: i32 = source.query_row("SELECT schema_version FROM dendro_meta", [], |row| {
        row.get(0)
    })?;
    if version != 3 {
        return Err(MigrationError::Refused(Refusal {
            code: "rollback_requires_preserved_schema3",
            source_mutated: false,
            pointer_changed: false,
            stage_retained: true,
            detail: format!("schema {version}"),
        }));
    }
    let inspection = super::engine::inspect_read_only(preserved_schema3)?;
    let mut watermarks = Vec::new();
    if inspection
        .tables
        .iter()
        .any(|table| table == "source_files")
    {
        let mut stmt = source.prepare(
            "SELECT source_id, generation, byte_offset FROM source_files ORDER BY source_id, generation",
        )?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            watermarks.push(Watermark {
                source_id: row.get(0)?,
                generation: row.get(1)?,
                byte_offset: row.get(2)?,
            });
        }
    }
    // These are independent destination observations, not a source watermark join.
    let mut cutoffs = Vec::new();
    if inspection
        .tables
        .iter()
        .any(|table| table == "destination_snapshots")
    {
        let mut stmt = source.prepare(
            "SELECT DISTINCT snapshot_cutoff FROM destination_snapshots
             WHERE snapshot_cutoff IS NOT NULL ORDER BY snapshot_cutoff",
        )?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            cutoffs.push(row.get::<_, String>(0)?);
        }
    }
    let watermark = if watermarks.len() == 1 {
        Some(watermarks[0].clone())
    } else {
        None
    };
    let cutoff = if cutoffs.len() == 1 {
        Some(cutoffs[0].clone())
    } else {
        None
    };

    let staged = open_ro(failed_schema4)?;
    let staged_version: i32 =
        staged.query_row("SELECT schema_version FROM dendro_meta", [], |row| {
            row.get(0)
        })?;
    if staged_version != MIGRATION_TARGET_SCHEMA_VERSION {
        return Err(MigrationError::Refused(Refusal {
            code: "rollback_failed_bundle_not_schema4",
            source_mutated: false,
            pointer_changed: false,
            stage_retained: true,
            detail: format!("schema {staged_version}"),
        }));
    }
    let recorded_preimage: String = staged.query_row(
        "SELECT source_db_sha256 FROM dendro_migration_operation WHERE id = 1",
        [],
        |row| row.get(0),
    )?;
    if recorded_preimage != schema3_before {
        return Err(MigrationError::Refused(Refusal {
            code: "rollback_preimage_mismatch",
            source_mutated: false,
            pointer_changed: false,
            stage_retained: true,
            detail: "preserved schema3 is not the failed stage preimage".into(),
        }));
    }
    let mut staged_columns = std::collections::BTreeMap::new();
    for table in &inspection.tables {
        staged_columns.insert(table.clone(), super::engine::list_columns(&staged, table)?);
    }
    let legacy_unchanged = staged_columns == inspection.columns
        && super::engine::legacy_counts_match(&staged, &inspection)?
        && super::engine::staged_projection_hash(&staged, &inspection.tables, &staged_columns)?
            == inspection.projection_sha256;
    // A damaged or additional v4 metadata shape cannot prove no loss. Rollback
    // can still report declared exclusions while continuity stays unresolved.
    let pristine_metadata =
        super::engine::pristine_metadata_matches(&staged, &inspection, &schema3_before)
            .unwrap_or(false);
    let usage_present = inspection.tables.iter().any(|table| table == "usage_facts");
    let usage_rows = inspection.counts.get("usage_facts").copied().unwrap_or(0);
    let legacy_usage_scope = evidence_scope(usage_present, usage_rows as usize);
    let watermark_scope = evidence_scope(
        inspection
            .tables
            .iter()
            .any(|table| table == "source_files"),
        watermarks.len(),
    );
    let cutoff_scope = evidence_scope(
        inspection
            .tables
            .iter()
            .any(|table| table == "destination_snapshots"),
        cutoffs.len(),
    );
    let mut stmt = staged.prepare(
        "SELECT fact_id, ts, source_id, generation, range_start, range_end
         FROM dendro_v4_only_facts ORDER BY fact_id",
    )?;
    let mut rows = stmt.query([])?;
    let mut facts = Vec::new();
    while let Some(row) = rows.next()? {
        let fact = V4OnlyFact {
            id: row.get(0)?,
            ts: row.get(1)?,
            source_id: row.get(2)?,
            generation: row.get(3)?,
            byte_range_half_open: [row.get(4)?, row.get(5)?],
        };
        let in_schema3: i64 = if usage_present {
            source.query_row(
                "SELECT COUNT(*) FROM usage_facts WHERE source_key = ?",
                params![fact.id],
                |r| r.get(0),
            )?
        } else {
            // This is declared exclusion evidence, not verified non-membership.
            0
        };
        if in_schema3 != 0 {
            return Err(MigrationError::Refused(Refusal {
                code: "v4_only_fact_present_in_schema3",
                source_mutated: false,
                pointer_changed: false,
                stage_retained: true,
                detail: fact.id,
            }));
        }
        facts.push(fact);
    }
    drop(rows);
    drop(stmt);
    drop(staged);
    drop(source);

    ensure_file_unchanged(preserved_schema3, &schema3_before)?;
    ensure_file_unchanged(failed_schema4, &schema4_before)?;

    let excluded_windows = if facts.is_empty() {
        Vec::new()
    } else {
        let mut times: Vec<&str> = facts.iter().map(|fact| fact.ts.as_str()).collect();
        times.sort_unstable();
        vec![(times[0].to_string(), times[times.len() - 1].to_string())]
    };
    let excluded_source_ranges = merge_source_ranges(&facts);
    let excluded_fact_count = facts.len();
    let replay_available = logs_present;
    // Continuity requires evidence of zero excluded facts and no missing logs.
    let full_continuity_claim = excluded_fact_count == 0
        && !logs_present
        && legacy_unchanged
        && pristine_metadata
        && watermark_scope != EvidenceScope::Absent
        && cutoff_scope != EvidenceScope::Absent
        && legacy_usage_scope != EvidenceScope::Absent;
    let unresolved_loss = !full_continuity_claim;
    let publication_code = super::refuse_consumer_switch()?;
    Ok(LossReceipt {
        selected_bundle: "preserved_schema3",
        destructive_downmigration: false,
        failed_schema4_retained_private: true,
        failed_schema4_path: failed_schema4.to_path_buf(),
        preserved_schema3_path: preserved_schema3.to_path_buf(),
        last_common_watermark: watermark,
        last_common_cutoff: cutoff,
        last_common_watermarks: watermarks,
        last_common_cutoffs: cutoffs,
        watermark_scope,
        cutoff_scope,
        legacy_usage_scope,
        v4_only_membership_verified: usage_present,
        excluded_fact_count,
        excluded_windows,
        excluded_source_ranges,
        replay_available,
        full_continuity_claim,
        unresolved_loss,
        publication_code,
        pointer_changed: false,
    })
}

fn ensure_file_unchanged(path: &Path, before: &str) -> Result<(), MigrationError> {
    let after = sha256_file(path)?;
    if after != before {
        return Err(MigrationError::SourceMutated {
            before: before.to_string(),
            after,
        });
    }
    Ok(())
}

pub fn merge_half_open_ranges(mut ranges: Vec<[i64; 2]>) -> Vec<[i64; 2]> {
    ranges.sort_by_key(|range| (range[0], range[1]));
    let mut merged: Vec<[i64; 2]> = Vec::new();
    for range in ranges {
        if let Some(last) = merged.last_mut() {
            if range[0] <= last[1] {
                last[1] = last[1].max(range[1]);
                continue;
            }
        }
        merged.push(range);
    }
    merged
}

fn merge_source_ranges(facts: &[V4OnlyFact]) -> Vec<SourceRange> {
    type SourceRangeGroup = ((String, i64), Vec<[i64; 2]>);
    let mut groups: Vec<SourceRangeGroup> = Vec::new();
    for fact in facts {
        let key = (fact.source_id.clone(), fact.generation);
        if let Some((_, ranges)) = groups.iter_mut().find(|(candidate, _)| *candidate == key) {
            ranges.push(fact.byte_range_half_open);
        } else {
            groups.push((key, vec![fact.byte_range_half_open]));
        }
    }
    groups
        .into_iter()
        .flat_map(|((source_id, generation), ranges)| {
            merge_half_open_ranges(ranges)
                .into_iter()
                .map(move |byte_range_half_open| SourceRange {
                    source_id: source_id.clone(),
                    generation,
                    byte_range_half_open,
                })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changed_file_error_reports_that_files_actual_hashes() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "dendro-rollback-hash-{}-{nonce}",
            std::process::id()
        ));
        std::fs::write(&path, b"synthetic old schema4 bytes").unwrap();
        let before_hash = sha256_file(&path).unwrap();
        std::fs::write(&path, b"synthetic changed schema4 bytes").unwrap();
        let after_hash = sha256_file(&path).unwrap();
        match ensure_file_unchanged(&path, &before_hash).unwrap_err() {
            MigrationError::SourceMutated { before, after } => {
                assert_eq!(before, before_hash);
                assert_eq!(after, after_hash);
                assert_ne!(before, after);
            }
            other => panic!("{other}"),
        }
        std::fs::remove_file(path).unwrap();
    }
}
