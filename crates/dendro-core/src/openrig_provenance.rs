use std::collections::{BTreeMap, HashMap};

use anyhow::{anyhow, Result};
use duckdb::params;
use serde_json::{json, Value};

use super::AnalyticsStore;
use crate::privacy::redact;

pub(super) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS openrig_row_records (
 source_id VARCHAR NOT NULL, generation BIGINT NOT NULL, producing_revision VARCHAR NOT NULL,
 table_name VARCHAR NOT NULL, row_id VARCHAR NOT NULL, row_digest VARCHAR NOT NULL,
 row_state VARCHAR NOT NULL, source_key VARCHAR NOT NULL,
 PRIMARY KEY (source_id, generation, table_name, row_id, source_key)
);
CREATE TABLE IF NOT EXISTS mutable_source_disclosures (
 qitem_id VARCHAR NOT NULL, seat VARCHAR NOT NULL, reason VARCHAR NOT NULL,
 PRIMARY KEY (qitem_id, seat, reason)
)";

/// A digest of a complete SQLite row in the accepted private source snapshot.
#[derive(Clone, Debug)]
pub struct OpenRigRow {
    pub table_name: String,
    pub row_id: String,
    pub row_digest: String,
}

#[derive(Clone, Debug)]
pub struct OpenRigIngest {
    pub generation: i64,
    pub changed: bool,
    pub reset: bool,
}

#[derive(Clone, Debug)]
pub struct OpenRigRecord {
    pub source_id: String,
    pub generation: i64,
    pub producing_revision: String,
    pub table_name: String,
    pub row_id: String,
    pub row_digest: String,
    pub row_state: String,
    pub source_key: String,
}

#[derive(Clone)]
pub(super) struct RawProvenance {
    generation: i64,
    revision: String,
    digest: String,
}
pub(super) type OpenRigRowCache = HashMap<(String, String, String), RawProvenance>;

impl AnalyticsStore {
    fn refresh_openrig_row_cache(&self) -> Result<()> {
        let mut stmt = self.conn.prepare("SELECT source_id, table_name, row_id, generation, producing_revision, row_digest FROM openrig_row_records WHERE source_key = '' AND row_state = 'present' ORDER BY generation")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                (r.get(0)?, r.get(1)?, r.get(2)?),
                RawProvenance {
                    generation: r.get(3)?,
                    revision: r.get(4)?,
                    digest: r.get(5)?,
                },
            ))
        })?;
        let cache = rows.collect::<duckdb::Result<OpenRigRowCache>>()?;
        *self.openrig_rows.borrow_mut() = Some(cache);
        Ok(())
    }

    /// Batch all facts and their provenance mappings from one accepted source
    /// snapshot into one transaction. A failed reader rolls the batch back.
    pub fn with_openrig_fact_batch<T>(&self, ingest: impl FnOnce() -> Result<T>) -> Result<T> {
        self.canonical_write(|| {
            if self.openrig_pending.borrow().is_some() {
                return Err(anyhow!("OpenRig fact batch is already active"));
            }
            *self.openrig_pending.borrow_mut() = Some(Vec::new());
            let result = ingest();
            // Clear the buffer before propagating reader/flush errors so rollback
            // cannot leave mappings available to the next accepted batch.
            let records = self.openrig_pending.borrow_mut().take().unwrap_or_default();
            let result = result?;
            if !records.is_empty() {
                self.conn.execute_batch("CREATE TEMP TABLE openrig_provenance_stage AS SELECT * FROM openrig_row_records WHERE FALSE")?;
                {
                    let mut appender = self.conn.appender("openrig_provenance_stage")?;
                    for row in &records {
                        crate::RUN_CANCEL.check()?;
                        appender.append_row(params![
                            row.source_id,
                            row.generation,
                            row.producing_revision,
                            row.table_name,
                            row.row_id,
                            row.row_digest,
                            row.row_state,
                            row.source_key
                        ])?;
                    }
                    appender.flush()?;
                }
                self.conn.execute_batch("INSERT INTO openrig_row_records SELECT DISTINCT * FROM openrig_provenance_stage ON CONFLICT DO NOTHING; DROP TABLE openrig_provenance_stage")?;
            }
            crate::RUN_CANCEL.check()?;
            Ok(result)
        })
    }

    /// Include the source inode identity, so a replacement with reused row
    /// keys is distinguishable from an update within the same database.
    pub fn begin_openrig_ingest_with_identity(
        &self,
        source_id: &str,
        revision: &str,
        rows: &[OpenRigRow],
        source_path: &std::path::Path,
    ) -> Result<OpenRigIngest> {
        use sha2::{Digest, Sha256};
        let mut rows = rows.to_vec();
        let file = std::fs::File::open(source_path)?;
        let metadata = crate::file_metadata::held_source_metadata(&file)
            .map_err(|reason| anyhow!("source metadata unavailable: {reason}"))?;
        let identity = crate::file_metadata::reported_identity(&metadata)
            .map_err(|reason| anyhow!("source identity unavailable: {reason}"))?;
        let mut digest = Sha256::new();
        match identity {
            crate::platform_v1::FileIdentity::Unix { device, inode } => {
                digest.update(device.to_be_bytes());
                digest.update(inode.to_be_bytes());
            }
            crate::platform_v1::FileIdentity::WindowsFileId128 {
                volume_serial_64,
                file_id,
            } => {
                digest.update(b"windows-file-id-128\0");
                digest.update(volume_serial_64.to_be_bytes());
                digest.update(file_id);
            }
            crate::platform_v1::FileIdentity::WindowsByHandle64 { .. } => {
                return Err(anyhow!("source identity is too narrow"));
            }
        }
        rows.push(OpenRigRow {
            table_name: "source_identity".into(),
            row_id: "file".into(),
            row_digest: hex::encode(digest.finalize()),
        });
        self.begin_openrig_ingest(source_id, revision, &rows)
    }

    /// Associate new rows with this ingest, retaining the first producing
    /// generation for unchanged rows. Retention never removes accepted facts.
    pub fn begin_openrig_ingest(
        &self,
        source_id: &str,
        revision: &str,
        rows: &[OpenRigRow],
    ) -> Result<OpenRigIngest> {
        *self.openrig_rows.borrow_mut() = None;
        self.conn.execute_batch(SCHEMA)?;
        let source_id = redact(source_id);
        let revision = redact(revision);
        let previous = self.load_openrig_row_records()?;
        let mut latest = BTreeMap::<(String, String), &OpenRigRecord>::new();
        let mut max_generation = 0;
        for record in previous.iter().filter(|r| r.source_id == source_id) {
            max_generation = max_generation.max(record.generation);
            if record.source_key.is_empty() {
                let key = (record.table_name.clone(), record.row_id.clone());
                if latest
                    .get(&key)
                    .is_none_or(|old| old.generation < record.generation)
                {
                    latest.insert(key, record);
                }
            }
        }
        let incoming: BTreeMap<_, _> = rows
            .iter()
            .map(|r| {
                (
                    (redact(&r.table_name), redact(&r.row_id)),
                    redact(&r.row_digest),
                )
            })
            .collect();
        // events is durable append-only evidence; only transition/snapshot
        // disappearance may be ordinary retention.
        let reset = latest.iter().any(|(key, old)| {
            key.0 == "events" && old.row_state != "superseded" && !incoming.contains_key(key)
        }) || latest.iter().any(|(key, old)| {
            key.0 == "source_identity"
                && incoming
                    .get(key)
                    .is_some_and(|digest| digest != &old.row_digest)
        });
        let changed = latest.iter().any(|(key, old)| {
            matches!(key.0.as_str(), "events" | "queue_transitions" | "snapshots")
                && incoming
                    .get(key)
                    .is_some_and(|digest| digest != &old.row_digest)
        });
        let added = incoming
            .iter()
            .any(|(key, digest)| latest.get(key).is_none_or(|old| &old.row_digest != digest));
        let generation = if max_generation == 0 {
            1
        } else if reset || added {
            max_generation + 1
        } else {
            max_generation
        };
        let tx = self.conn.unchecked_transaction()?;
        self.invalidate_semantic()?;
        tx.execute("DELETE FROM mutable_source_disclosures", [])?;
        if reset {
            tx.execute("UPDATE openrig_row_records SET row_state = 'superseded' WHERE source_id = ? AND row_state IN ('present', 'gone')", params![source_id])?;
        }
        for (key, old) in &latest {
            crate::RUN_CANCEL.check()?;
            if reset {
                continue;
            }
            if incoming
                .get(key)
                .is_some_and(|digest| digest != &old.row_digest)
            {
                let immutable =
                    matches!(key.0.as_str(), "events" | "queue_transitions" | "snapshots");
                tx.execute("UPDATE openrig_row_records SET row_state = ? WHERE source_id = ? AND table_name = ? AND row_id = ? AND generation = ?",
                    params![if immutable { "changed" } else { "superseded" }, source_id, key.0, key.1, old.generation])?;
                if immutable {
                    tx.execute(
                        "INSERT INTO warnings (path, line_no, reason) VALUES (?, NULL, ?)",
                        params![
                            source_id,
                            redact(&format!(
                                "openrig source changed: {}:{} changed in place",
                                key.0, key.1
                            ))
                        ],
                    )?;
                }
            } else if !incoming.contains_key(key) && old.row_state == "present" {
                tx.execute("UPDATE openrig_row_records SET row_state = 'gone' WHERE source_id = ? AND table_name = ? AND row_id = ? AND generation = ?",
                    params![source_id, key.0, key.1, old.generation])?;
            }
        }
        {
            let mut insert = tx.appender("openrig_row_records")?;
            let mut restore = tx.prepare("UPDATE openrig_row_records SET row_state = 'present' WHERE source_id = ? AND table_name = ? AND row_id = ? AND generation = ?")?;
            for ((table, row_id), digest) in &incoming {
                crate::RUN_CANCEL.check()?;
                let old = latest.get(&(table.clone(), row_id.clone()));
                if !reset && old.is_some_and(|r| &r.row_digest == digest) {
                    let old = old.expect("checked");
                    if old.row_state != "present" {
                        restore.execute(params![source_id, table, row_id, old.generation])?;
                    }
                    continue;
                }
                insert.append_row(params![
                    source_id, generation, revision, table, row_id, digest, "present", ""
                ])?;
            }
            insert.flush()?;
        }
        tx.execute(
            "UPDATE source_files SET generation = ? WHERE source_id = ?",
            params![generation, source_id],
        )?;
        tx.commit()?;
        self.refresh_openrig_row_cache()?;
        Ok(OpenRigIngest {
            generation,
            changed,
            reset,
        })
    }

    /// Qualify a fact's identity by its producing generation and retain the
    /// association to the precise row that produced it.
    pub fn qualify_openrig_fact(
        &self,
        source_id: &str,
        table: &str,
        row_id: &str,
        source_key: &str,
    ) -> Result<String> {
        let source_id = redact(source_id);
        let table = redact(table);
        let row_id = redact(row_id);
        if self.openrig_rows.borrow().is_none() {
            self.refresh_openrig_row_cache()?;
        }
        let row = self
            .openrig_rows
            .borrow()
            .as_ref()
            .and_then(|rows| rows.get(&(source_id.clone(), table.clone(), row_id.clone())))
            .cloned()
            .ok_or_else(|| anyhow!("OpenRig row provenance is missing"))?;
        let RawProvenance {
            generation,
            revision,
            digest,
        } = row;
        let key = if generation == 1 {
            redact(source_key)
        } else {
            redact(&format!("{source_key}@g{generation}"))
        };
        self.record_openrig_mapping(OpenRigRecord {
            source_id,
            generation,
            producing_revision: revision,
            table_name: table,
            row_id,
            row_digest: digest,
            row_state: "present".into(),
            source_key: key.clone(),
        })?;
        Ok(key)
    }

    fn record_openrig_mapping(&self, row: OpenRigRecord) -> Result<()> {
        self.canonical_write(|| {
            if let Some(pending) = self.openrig_pending.borrow_mut().as_mut() {
                pending.push(row);
                return Ok(());
            }
            self.conn.prepare_cached("INSERT INTO openrig_row_records VALUES (?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT DO NOTHING")?.execute(
                params![row.source_id,row.generation,row.producing_revision,row.table_name,row.row_id,row.row_digest,row.row_state,row.source_key])?;
            Ok(())
        })
    }

    /// Attach stored mutable destination observations to their source row.
    pub fn record_openrig_destination(&self, source_id: &str, qitem_id: &str) -> Result<()> {
        let mut stmt = self
            .conn
            .prepare("SELECT source_key FROM destination_snapshots d WHERE qitem_id = ? AND NOT EXISTS (SELECT 1 FROM openrig_row_records r WHERE r.source_key = d.source_key)")?;
        let rows = stmt.query_map(params![redact(qitem_id)], |r| r.get::<_, String>(0))?;
        let keys = rows.collect::<Result<Vec<_>, _>>()?;
        for key in keys {
            let qualified = self.qualify_openrig_fact(source_id, "queue_items", qitem_id, &key)?;
            // Destination observations already have a content-qualified key;
            // associate that key as well for the backwards-compatible loader.
            if qualified != key {
                let source_id = redact(source_id);
                let row_id = redact(qitem_id);
                let row = self
                    .openrig_rows
                    .borrow()
                    .as_ref()
                    .and_then(|rows| {
                        rows.get(&(source_id.clone(), "queue_items".into(), row_id.clone()))
                    })
                    .cloned()
                    .ok_or_else(|| anyhow!("OpenRig row provenance is missing"))?;
                self.record_openrig_mapping(OpenRigRecord {
                    source_id,
                    generation: row.generation,
                    producing_revision: row.revision,
                    table_name: "queue_items".into(),
                    row_id,
                    row_digest: row.digest,
                    row_state: "present".into(),
                    source_key: key,
                })?;
            }
        }
        Ok(())
    }

    pub fn load_openrig_row_records(&self) -> Result<Vec<OpenRigRecord>> {
        let exists: i64 = self.conn.query_row("SELECT count(*) FROM information_schema.tables WHERE table_name = 'openrig_row_records'", [], |r| r.get(0))?;
        if exists == 0 {
            return Ok(Vec::new());
        }
        let mut stmt = self.conn.prepare("SELECT source_id, generation, producing_revision, table_name, row_id, row_digest, row_state, source_key FROM openrig_row_records ORDER BY source_id, generation, table_name, row_id, source_key")?;
        let rows = stmt.query_map([], |r| {
            Ok(OpenRigRecord {
                source_id: r.get(0)?,
                generation: r.get(1)?,
                producing_revision: r.get(2)?,
                table_name: r.get(3)?,
                row_id: r.get(4)?,
                row_digest: r.get(5)?,
                row_state: r.get(6)?,
                source_key: r.get(7)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn record_mutable_source_disclosure(
        &self,
        qitem_id: &str,
        seat: &str,
        reason: &str,
    ) -> Result<()> {
        self.canonical_write(|| {
            self.conn.execute_batch(SCHEMA)?;
            self.conn.execute(
                "INSERT INTO mutable_source_disclosures VALUES (?, ?, ?) ON CONFLICT DO NOTHING",
                params![redact(qitem_id), redact(seat), redact(reason)],
            )?;
            Ok(())
        })
    }

    pub fn load_mutable_source_disclosures(&self) -> Result<Vec<Value>> {
        let exists: i64 = self.conn.query_row("SELECT count(*) FROM information_schema.tables WHERE table_name = 'mutable_source_disclosures'", [], |r| r.get(0))?;
        if exists == 0 {
            return Ok(Vec::new());
        }
        let mut stmt = self.conn.prepare("SELECT qitem_id, seat, reason FROM mutable_source_disclosures ORDER BY qitem_id, seat, reason")?;
        let rows = stmt.query_map([], |r| Ok(json!({"qitem_id":r.get::<_,String>(0)?, "seat":r.get::<_,String>(1)?, "reason":r.get::<_,String>(2)?, "status":"not reproducible: mutable source"})))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ActivityFact, QueueFact};
    use chrono::{TimeZone, Utc};

    fn row(table: &str, id: &str, digest: &str) -> OpenRigRow {
        OpenRigRow {
            table_name: table.into(),
            row_id: id.into(),
            row_digest: digest.into(),
        }
    }
    fn queue(key: String, seq: i64) -> QueueFact {
        QueueFact {
            source_key: key,
            transition_id: seq,
            qitem_id: format!("q{seq}"),
            ts: Utc.timestamp_opt(100 + seq, 0).unwrap(),
            state: "done".into(),
            source_seat: "a".into(),
            dest_seat: "b".into(),
            closure_reason: Some("completed".into()),
            closure_target: None,
            chain_id: None,
            missions: vec![],
        }
    }

    #[test]
    fn append_retains_producing_generation_and_revision() {
        let store = AnalyticsStore::open_in_memory("key").unwrap();
        let original = row("events", "1", "digest1");
        assert_eq!(
            store
                .begin_openrig_ingest("source", "rev1", std::slice::from_ref(&original))
                .unwrap()
                .generation,
            1
        );
        let first = store
            .qualify_openrig_fact("source", "events", "1", "events:queue.created:1")
            .unwrap();
        store.insert_queue(&queue(first.clone(), 1)).unwrap();
        assert_eq!(
            store
                .begin_openrig_ingest("source", "rev2", &[original, row("events", "2", "digest2")])
                .unwrap()
                .generation,
            2
        );
        assert_eq!(
            store
                .qualify_openrig_fact("source", "events", "1", "events:queue.created:1")
                .unwrap(),
            first
        );
        let second = store
            .qualify_openrig_fact("source", "events", "2", "events:queue.created:2")
            .unwrap();
        assert_eq!(second, "events:queue.created:2@g2");
        store.insert_queue(&queue(second, 2)).unwrap();
        let records = store.load_openrig_row_records().unwrap();
        let old = records.iter().find(|r| r.source_key == first).unwrap();
        assert_eq!(old.generation, 1);
        assert_eq!(old.producing_revision, "rev1");
        assert_eq!(old.row_state, "present");
        assert_eq!(store.load_queue().unwrap().len(), 2);
    }

    #[test]
    fn retention_preserves_facts_and_marks_rows_gone() {
        let store = AnalyticsStore::open_in_memory("key").unwrap();
        let event = row("events", "1", "digest1");
        store
            .begin_openrig_ingest(
                "source",
                "rev1",
                &[
                    event.clone(),
                    row("queue_transitions", "1", "digest2"),
                    row("snapshots", "1", "digest3"),
                ],
            )
            .unwrap();
        let key = store
            .qualify_openrig_fact("source", "queue_transitions", "1", "queue_transitions:1")
            .unwrap();
        store.insert_queue(&queue(key.clone(), 1)).unwrap();
        let next = store
            .begin_openrig_ingest("source", "rev2", &[event])
            .unwrap();
        assert!(!next.reset);
        assert_eq!(store.load_queue().unwrap().len(), 1);
        let records = store.load_openrig_row_records().unwrap();
        assert_eq!(
            records
                .iter()
                .find(|r| r.source_key == key)
                .unwrap()
                .row_state,
            "gone"
        );
        assert_eq!(
            records
                .iter()
                .find(|r| r.table_name == "snapshots")
                .unwrap()
                .row_state,
            "gone"
        );
    }

    #[test]
    fn reset_retains_history_and_accepts_reused_keys() {
        let store = AnalyticsStore::open_in_memory("key").unwrap();
        store
            .begin_openrig_ingest(
                "source",
                "rev1",
                &[row("events", "1", "d1"), row("events", "2", "d2")],
            )
            .unwrap();
        let old = store
            .qualify_openrig_fact("source", "events", "1", "events:queue.created:1")
            .unwrap();
        store.insert_queue(&queue(old.clone(), 1)).unwrap();
        let next = store
            .begin_openrig_ingest("source", "rev2", &[row("events", "1", "new")])
            .unwrap();
        assert!(next.reset);
        assert_eq!(next.generation, 2);
        let new = store
            .qualify_openrig_fact("source", "events", "1", "events:queue.created:1")
            .unwrap();
        store.insert_queue(&queue(new, 1)).unwrap();
        assert_eq!(store.load_queue().unwrap().len(), 2);
        assert_eq!(
            store
                .load_openrig_row_records()
                .unwrap()
                .iter()
                .find(|r| r.source_key == old)
                .unwrap()
                .row_state,
            "superseded"
        );
    }

    #[test]
    fn in_place_change_replaces_current_fact_and_discloses_warning() {
        let store = AnalyticsStore::open_in_memory("key").unwrap();
        store
            .begin_openrig_ingest("source", "rev1", &[row("events", "1", "old")])
            .unwrap();
        let key = store
            .qualify_openrig_fact("source", "events", "1", "seat.activity_changed:1")
            .unwrap();
        let mut activity = ActivityFact {
            source_key: key,
            seat: "seat".into(),
            ts: Utc.timestamp_opt(100, 0).unwrap(),
            state: "running".into(),
            seq: 1,
        };
        store.insert_activity(&activity).unwrap();
        let next = store
            .begin_openrig_ingest("source", "rev2", &[row("events", "1", "new")])
            .unwrap();
        assert!(next.changed);
        assert!(!next.reset);
        activity.source_key = store
            .qualify_openrig_fact("source", "events", "1", "seat.activity_changed:1")
            .unwrap();
        activity.state = "idle".into();
        store.insert_activity(&activity).unwrap();
        assert_eq!(store.load_activity().unwrap().len(), 1);
        assert_eq!(
            store.load_activity().unwrap()[0].source_key,
            "seat.activity_changed:1@g2"
        );
        assert_eq!(
            store
                .load_openrig_row_records()
                .unwrap()
                .iter()
                .filter(|r| r.row_state == "changed")
                .count(),
            2
        );
    }
    #[cfg(any(unix, windows))]
    #[test]
    fn replacing_inode_with_reused_identical_keys_starts_a_new_generation() {
        let store = AnalyticsStore::open_in_memory("key").unwrap();
        #[cfg(windows)]
        let root = crate::platform_v1::create_user_private_root_with_choice(
            "dendro-openrig-identity-",
            &crate::platform_v1::StorageRootChoice::UserProfile,
        )
        .expect("explicit UserProfile identity fixture");
        #[cfg(windows)]
        let dir = root.path().to_path_buf();
        #[cfg(unix)]
        let dir =
            std::env::temp_dir().join(format!("dendro-openrig-identity-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("openrig.sqlite");
        assert!(store
            .begin_openrig_ingest_with_identity("source", "missing", &[], &path)
            .is_err());
        assert!(store.load_openrig_row_records().unwrap().is_empty());
        std::fs::write(&path, b"old").unwrap();
        let rows = [row("events", "1", "same")];
        store
            .begin_openrig_ingest_with_identity("source", "rev1", &rows, &path)
            .unwrap();
        #[cfg(windows)]
        {
            use sha2::{Digest, Sha256};
            let metadata =
                crate::file_metadata::held_source_metadata(&std::fs::File::open(&path).unwrap())
                    .unwrap();
            let crate::platform_v1::FileIdentity::WindowsFileId128 {
                volume_serial_64,
                file_id,
            } = crate::file_metadata::reported_identity(&metadata).unwrap()
            else {
                panic!("full Windows identity required")
            };
            let mut expected = Sha256::new();
            expected.update(b"windows-file-id-128\0");
            expected.update(volume_serial_64.to_be_bytes());
            expected.update(file_id);
            let records = store.load_openrig_row_records().unwrap();
            let identity = records
                .iter()
                .find(|row| row.table_name == "source_identity")
                .unwrap();
            assert_eq!(identity.row_digest, hex::encode(expected.finalize()));
            assert_eq!(identity.row_state, "present");
        }
        let key = store
            .qualify_openrig_fact("source", "events", "1", "events:1")
            .unwrap();
        let replacement = dir.join("replacement.sqlite");
        std::fs::write(&replacement, b"new").unwrap();
        std::fs::rename(replacement, &path).unwrap();
        let result = store
            .begin_openrig_ingest_with_identity("source", "rev2", &rows, &path)
            .unwrap();
        assert!(result.reset);
        assert_eq!(result.generation, 2);
        assert_eq!(
            store
                .qualify_openrig_fact("source", "events", "1", "events:1")
                .unwrap(),
            "events:1@g2"
        );
        assert_eq!(
            store
                .load_openrig_row_records()
                .unwrap()
                .iter()
                .find(|r| r.source_key == key)
                .unwrap()
                .row_state,
            "superseded"
        );
        #[cfg(windows)]
        drop(root);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn failed_fact_batch_rolls_back_facts_and_row_associations() {
        let store = AnalyticsStore::open_in_memory("key").unwrap();
        store
            .begin_openrig_ingest("source", "rev1", &[row("events", "1", "d1")])
            .unwrap();
        let result: Result<()> = store.with_openrig_fact_batch(|| {
            let key =
                store.qualify_openrig_fact("source", "events", "1", "events:queue.created:1")?;
            store.insert_queue(&queue(key, 1))?;
            Err(anyhow!("reader failed"))
        });
        assert!(result.is_err());
        assert!(store.load_queue().unwrap().is_empty());
        assert!(store
            .load_openrig_row_records()
            .unwrap()
            .iter()
            .all(|r| r.source_key.is_empty()));
        store
            .with_openrig_fact_batch(|| {
                let key = store.qualify_openrig_fact(
                    "source",
                    "events",
                    "1",
                    "events:queue.created:1",
                )?;
                store.insert_queue(&queue(key, 1))
            })
            .unwrap();
        assert_eq!(store.load_queue().unwrap().len(), 1);
    }
    #[test]
    fn reopened_store_seeds_provenance_cache_and_later_ingest_invalidates_it() {
        let dir = std::env::temp_dir().join(format!("dendro-openrig-cache-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("analytics.duckdb");
        let store = AnalyticsStore::open_path(&path, "key").unwrap();
        store
            .begin_openrig_ingest("source", "rev1", &[row("events", "1", "old")])
            .unwrap();
        store.checkpoint().unwrap();
        drop(store);
        let store = AnalyticsStore::open_existing(&path, "key").unwrap();
        assert_eq!(
            store
                .qualify_openrig_fact("source", "events", "1", "events:1")
                .unwrap(),
            "events:1"
        );
        store
            .begin_openrig_ingest("source", "rev2", &[row("events", "1", "new")])
            .unwrap();
        assert_eq!(
            store
                .qualify_openrig_fact("source", "events", "1", "events:1")
                .unwrap(),
            "events:1@g2"
        );
        let record = store
            .load_openrig_row_records()
            .unwrap()
            .into_iter()
            .find(|r| r.source_key == "events:1@g2")
            .unwrap();
        assert_eq!(record.producing_revision, "rev2");
        drop(store);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn buffered_fact_mappings_deduplicate_at_commit_and_discard_failed_attempts() {
        let store = AnalyticsStore::open_in_memory("key").unwrap();
        store
            .begin_openrig_ingest("source", "rev1", &[row("events", "1", "digest")])
            .unwrap();
        let failed: Result<()> = store.with_openrig_fact_batch(|| {
            store.qualify_openrig_fact("source", "events", "1", "events:discarded:1")?;
            Err(anyhow!("stop before publication"))
        });
        assert!(failed.is_err());
        store
            .with_openrig_fact_batch(|| {
                store.qualify_openrig_fact("source", "events", "1", "events:1")?;
                store.qualify_openrig_fact("source", "events", "1", "events:1")?;
                Ok(())
            })
            .unwrap();
        let records = store.load_openrig_row_records().unwrap();
        assert_eq!(
            records
                .iter()
                .filter(|r| r.source_key == "events:1")
                .count(),
            1
        );
        assert!(!records.iter().any(|r| r.source_key == "events:discarded:1"));
    }
}
