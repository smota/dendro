use super::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RevisionPolicy {
    CorrectionsAtCutoffV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnswerSelection {
    pub event_window: Interval,
    pub knowledge_cutoff: UtcMicros,
    pub revision_policy: RevisionPolicy,
}

/// Validated append-only ledger. Callers cannot supply a digest instead of content.
/// Ingestion/observation timestamps are receipt metadata: re-ingesting identical
/// content is idempotent and preserves its first-admitted receipt, even if a
/// later re-ingestion carries an earlier timestamp.
type RevisionKey = (SourceInstanceId, u64, SourceKey, u64);
type RevisionEntry = ([u8; 32], usize);
pub struct RevisionLedger<T> {
    records: Vec<Record<T>>,
    revisions: BTreeMap<RevisionKey, RevisionEntry>,
    ids: BTreeMap<FactId, usize>,
    conflicts: BTreeMap<RevisionKey, Vec<Record<T>>>,
}
impl<T> Default for RevisionLedger<T> {
    fn default() -> Self {
        Self {
            records: Vec::new(),
            revisions: BTreeMap::new(),
            ids: BTreeMap::new(),
            conflicts: BTreeMap::new(),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppendResult {
    Inserted,
    Idempotent,
    Conflict,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct FactChain {
    pub source_instance: SourceInstanceId,
    pub generation: u64,
    pub source_key: SourceKey,
}
impl FactChain {
    fn of(p: &Provenance) -> Self {
        Self {
            source_instance: p.source_instance_id.clone(),
            generation: p.source_generation.number,
            source_key: p.source_key.clone(),
        }
    }
}
/// Valid chains remain usable. Conflict chains retain every cutoff-visible
/// admitted revision and alternative content; no winner is selected.
pub struct RevisionSnapshot<'a, T> {
    pub valid: Vec<&'a Record<T>>,
    pub conflicts: BTreeMap<FactChain, Vec<&'a Record<T>>>,
}

/// Stable for this typed contract: struct field order, ordered sets and canonical
/// numeric constructors are used. No arbitrary JSON maps enter the record model.
pub fn revision_digest<T: CanonicalFact>(record: &Record<T>) -> Result<[u8; 32], Refusal> {
    let mut provenance = record.provenance.clone();
    provenance.canonicalize();
    let mut value = record.value.clone();
    value.canonicalize();
    let bytes = serde_json::to_vec(&(provenance, value)).map_err(|_| Refusal::InvalidQuantity)?;
    Ok(Sha256::digest(bytes).into())
}
impl<T: CanonicalFact + Validate> RevisionLedger<T> {
    pub fn records(&self) -> &[Record<T>] {
        &self.records
    }
    pub fn conflicts(&self) -> impl Iterator<Item = &Record<T>> {
        self.conflicts.values().flatten()
    }
    pub fn append(&mut self, record: Record<T>) -> Result<AppendResult, Refusal> {
        record.validate()?;
        let p = &record.provenance;
        let key = (
            p.source_instance_id.clone(),
            p.source_generation.number,
            p.source_key.clone(),
            p.revision,
        );
        let digest = revision_digest(&record)?;
        if let Some((old_digest, _)) = self.revisions.get(&key) {
            if old_digest == &digest {
                return Ok(AppendResult::Idempotent);
            }
            if let Some(alternatives) = self.conflicts.get(&key) {
                for alternative in alternatives {
                    if revision_digest(alternative)? == digest {
                        return Ok(AppendResult::Idempotent);
                    }
                }
            }
            self.conflicts.entry(key).or_default().push(record);
            return Ok(AppendResult::Conflict);
        }
        if self.ids.contains_key(&p.fact_id) {
            return Err(Refusal::Conflict);
        }
        let target = p.supersedes.as_ref().or(p.retracts.as_ref());
        if let Some(id) = target {
            let old = self
                .ids
                .get(id)
                .and_then(|i| self.records.get(*i))
                .ok_or(Refusal::InvalidReference)?;
            let o = &old.provenance;
            if o.source_instance_id != p.source_instance_id
                || o.source_generation.number != p.source_generation.number
                || o.source_key != p.source_key
                || o.revision >= p.revision
                || o.ingested_at > p.ingested_at
            {
                return Err(Refusal::InvalidReference);
            }
            // A revision must follow the current leaf; no silent branch or resurrection
            // after a tombstone. Different content at the same revision is retained above.
            if self.records.iter().any(|r| {
                r.provenance.supersedes.as_ref() == Some(id)
                    || r.provenance.retracts.as_ref() == Some(id)
            }) || o.retracts.is_some()
            {
                return Err(Refusal::Conflict);
            }
        } else if self
            .revisions
            .keys()
            .any(|k| k.0 == key.0 && k.1 == key.1 && k.2 == key.2)
        {
            return Err(Refusal::InvalidReference);
        }
        let index = self.records.len();
        self.revisions.insert(key, (digest, index));
        self.ids.insert(p.fact_id.clone(), index);
        self.records.push(record);
        Ok(AppendResult::Inserted)
    }
    /// Select revisions by ingestion cutoff before filtering event time. A correction
    /// moving an event outside the window cannot resurrect its earlier version.
    pub fn at_cutoff(
        &self,
        cutoff: UtcMicros,
        _policy: RevisionPolicy,
    ) -> Result<RevisionSnapshot<'_, T>, Refusal> {
        let alternatives: Vec<_> = self
            .conflicts
            .iter()
            // A divergent receipt cannot expose a conflict before the original
            // admitted receipt, even if the alternative carries an earlier time.
            .filter(|(key, _)| {
                self.revisions
                    .get(*key)
                    .and_then(|(_, index)| self.records.get(*index))
                    .is_some_and(|original| original.provenance.ingested_at <= cutoff)
            })
            .flat_map(|(_, alternatives)| alternatives)
            .filter(|r| r.provenance.ingested_at <= cutoff)
            .collect();
        let chains: BTreeSet<_> = alternatives
            .iter()
            .map(|r| FactChain::of(&r.provenance))
            .collect();
        let visible: Vec<_> = self
            .records
            .iter()
            .filter(|r| r.provenance.ingested_at <= cutoff)
            .collect();
        let removed: BTreeSet<_> = visible
            .iter()
            .flat_map(|r| {
                r.provenance
                    .supersedes
                    .iter()
                    .chain(r.provenance.retracts.iter())
            })
            .collect();
        let mut conflicts: BTreeMap<_, Vec<_>> =
            chains.iter().cloned().map(|c| (c, Vec::new())).collect();
        for r in visible.iter().copied().chain(alternatives) {
            if let Some(contents) = conflicts.get_mut(&FactChain::of(&r.provenance)) {
                contents.push(r);
            }
        }
        let valid = visible
            .into_iter()
            .filter(|r| {
                !chains.contains(&FactChain::of(&r.provenance))
                    && r.provenance.retracts.is_none()
                    && !removed.contains(&r.provenance.fact_id)
            })
            .collect();
        Ok(RevisionSnapshot { valid, conflicts })
    }
}
