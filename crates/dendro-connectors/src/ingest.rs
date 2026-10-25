use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::{DateTime, Utc};
use dendro_core::{ActivityFact, AnalyticsStore, IdentityFact, QueueFact};
use rusqlite::OpenFlags;
use serde_json::Value;

use crate::capture_registry_v1::{ReaderRoute, Registry};
use crate::stream;

#[derive(Debug, Clone)]
pub struct SourceRoot {
    pub name: String,
    pub path: PathBuf,
}

/// `_frozen` is kept for compatibility with `dendro run --frozen`: every run
/// now reads OpenRig through a coherent private copy, frozen or not (AC12).
pub fn ingest_staged(
    staged_db: &Path,
    roots: &[SourceRoot],
    frozen: bool,
    key: &[u8],
) -> Result<Vec<String>> {
    ingest_staged_with_manifest(staged_db, roots, frozen, key, None).map(|(notes, _)| notes)
}

/// A sealed run rebuilds its stage using only named prefixes. Existing output
/// history may contain facts beyond the requested cutoff and cannot be reused.
pub fn ingest_staged_with_manifest(
    staged_db: &Path,
    roots: &[SourceRoot],
    _frozen: bool,
    key: &[u8],
    sealed: Option<&dendro_core::CutoffManifest>,
) -> Result<(Vec<String>, dendro_core::CutoffManifest)> {
    if sealed.is_some() {
        std::fs::remove_file(staged_db)?;
        let fresh = AnalyticsStore::open_path(staged_db, &dendro_core::key_id(key))?;
        fresh.checkpoint()?;
    }
    let store = AnalyticsStore::open_staged(staged_db)?;
    let stage_dir = staged_db.parent().unwrap_or_else(|| Path::new("."));
    let mut notes = Vec::new();
    let mut lives = Vec::new();
    let mut lifecycle_complete = true;
    let mut manifest = dendro_core::CutoffManifest::default();
    let mut seen_files = std::collections::BTreeSet::new();
    let mut seen_openrig = std::collections::BTreeSet::new();
    for root in roots {
        let shown = dendro_core::redact(&root.name);
        if !root.path.exists() {
            if root.name == "openrig" {
                lifecycle_complete = false;
            }
            notes.push(format!(
                "{shown} missing coverage=refused reason=source_missing"
            ));
            store.record_warning(&dendro_core::WarningNote {
                path: root.path.display().to_string(),
                line: None,
                reason: format!("source {} missing", root.name),
            })?;
            continue;
        }
        match root.name.as_str() {
            // OpenRig is a lifecycle copy, not a capture-registry reader.
            "openrig" => {
                let id = manifest_source_id(root, &root.path);
                let entry = sealed.and_then(|manifest| {
                    manifest.openrig.iter().find(|entry| entry.source_id == id)
                });
                if sealed.is_some() && entry.is_none() {
                    continue;
                }
                if !seen_openrig.insert(id.clone()) {
                    continue;
                }
                let (count, snapshot, cutoff, changed) =
                    ingest_openrig(&store, root, key, &mut lives, stage_dir, &id, entry)?;
                manifest.openrig.push(cutoff);
                let changed_note = if changed {
                    " openrig source changed"
                } else {
                    ""
                };
                notes.push(format!(
                    "openrig events={count} revision={} attempts={} main_bytes={} wal_bytes={} copy_ms={} check_ms={}{changed_note}",
                    &snapshot.revision[..16], snapshot.attempts, snapshot.main_bytes,
                    snapshot.wal_bytes.unwrap_or(0), snapshot.copy_ms, snapshot.check_ms
                ));
            }
            name => {
                match Registry::builtin().route(name) {
                    ReaderRoute::NativeJsonl => {
                        let mut files = match stream::walk_jsonl_checked(&root.path) {
                            Ok(files) => files,
                            Err(_) if sealed.is_none() => {
                                let reason = "coverage=refused reason=native_discovery_failed";
                                store.record_warning(&dendro_core::WarningNote {
                                    path: manifest_source_id(root, &root.path),
                                    line: None,
                                    reason: reason.into(),
                                })?;
                                notes.push(format!("{shown} {reason}"));
                                continue;
                            }
                            Err(error) => return Err(error),
                        };
                        if root.name == "grok" {
                            // Session stream only. Sibling jsonl files hold prompts and
                            // file text; `summary.json` is read beside `updates.jsonl`.
                            files.retain(|file| {
                                file.file_name().and_then(|file_name| file_name.to_str())
                                    == Some("updates.jsonl")
                            });
                        }
                        files.sort();
                        let mut read = stream::FileStats::default();
                        let mut file_count = 0;
                        let mut refused = 0;
                        for file in &files {
                            let id = manifest_source_id(root, file);
                            let entry = sealed.and_then(|manifest| {
                                manifest.files.iter().find(|entry| {
                                    entry.source_id == id && entry.harness == root.name
                                })
                            });
                            if sealed.is_some() && entry.is_none() {
                                continue;
                            }
                            if !seen_files.insert(id.clone()) {
                                continue;
                            }
                            let source_id = stable_source_id(&root.name, &root.path, file, key);
                            let stats = match stream::ingest_jsonl_with_cutoff(
                                &store,
                                &source_id,
                                &root.name,
                                file,
                                Some(key),
                                entry,
                            ) {
                                Ok(stats) => stats,
                                Err(error) if sealed.is_none() && error.is::<std::io::Error>() => {
                                    let reason = "coverage=refused reason=native_file_unreadable";
                                    store.record_warning(&dendro_core::WarningNote {
                                        path: id.clone(),
                                        line: None,
                                        reason: reason.into(),
                                    })?;
                                    notes.push(format!("{shown} file={id} {reason}"));
                                    refused += 1;
                                    continue;
                                }
                                Err(error) => return Err(error),
                            };
                            let (_, offset, digest) =
                                store.source_state(&source_id)?.ok_or_else(|| {
                                    anyhow::anyhow!("missing file cutoff after ingest")
                                })?;
                            manifest.files.push(dendro_core::FileCutoff {
                                source_id: dendro_core::redact(&id),
                                harness: dendro_core::redact(&root.name),
                                byte_length: u64::try_from(offset)?,
                                prefix_sha256: digest,
                            });
                            file_count += 1;
                            read.records_read += stats.records_read;
                            read.usage_records += stats.usage_records;
                            read.coverage.merge(stats.coverage);
                            notes.push(format!("{shown} file={id} {}", stats.coverage.summary()));
                        }
                        read.coverage.partial |= refused != 0;
                        let coverage = if root.name == "codex" {
                            " tool_calls=not read"
                        } else {
                            ""
                        };
                        notes.push(format!(
                        "{shown} files={file_count} refused={refused} records_read={} usage_records={}{coverage} {}",
                        read.records_read, read.usage_records, read.coverage.summary()
                    ));
                    }
                    ReaderRoute::VerifiedExport => {
                        let Some((harness, format)) =
                            crate::capture_registry_v1::verified_export(name)
                        else {
                            notes.push(format!("{shown} unsupported"));
                            continue;
                        };
                        let connector = crate::exports::ExportConnector::new(
                            root.path.clone(),
                            key.to_vec(),
                            harness,
                            format,
                        );
                        let mut file_count = 0;
                        let listing = match connector.inspect_discovery() {
                            Ok(listing) => listing,
                            Err(error)
                                if sealed.is_none() && !error.is::<dendro_core::Interrupted>() =>
                            {
                                let reason = "coverage=refused reason=export_discovery_failed";
                                store.record_warning(&dendro_core::WarningNote {
                                    path: manifest_source_id(root, &root.path),
                                    line: None,
                                    reason: reason.into(),
                                })?;
                                notes.push(format!("{shown} {reason}"));
                                continue;
                            }
                            Err(error) => return Err(error),
                        };
                        let mut refused = listing.refused.len();
                        for (path, reason) in listing.refused {
                            let id = manifest_source_id(root, &path);
                            let reason = format!("coverage=refused reason={reason}");
                            store.record_warning(&dendro_core::WarningNote {
                                path: id.clone(),
                                line: None,
                                reason: reason.clone(),
                            })?;
                            notes.push(format!("{shown} file={id} {reason}"));
                        }
                        let mut coverage = crate::sdk::Coverage::default();
                        for file in listing.paths {
                            let id = manifest_source_id(root, &file);
                            let entry = sealed.and_then(|manifest| {
                                manifest.files.iter().find(|entry| {
                                    entry.source_id == id && entry.harness == root.name
                                })
                            });
                            if sealed.is_some() && entry.is_none() {
                                continue;
                            }
                            if !seen_files.insert(id.clone()) {
                                continue;
                            }
                            let (warnings, watermark, file_coverage) =
                                match connector.ingest_file_with_cutoff(&store, &file, entry) {
                                    Ok(batch) => batch,
                                    Err(error)
                                        if sealed.is_none()
                                            && error.is::<crate::exports::ExportRefused>() =>
                                    {
                                        let failure = error
                                            .downcast_ref::<crate::exports::ExportRefused>()
                                            .expect("typed refusal");
                                        coverage.merge(failure.1);
                                        let reason = format!(
                                            "coverage=refused reason={} {}",
                                            failure.0,
                                            failure.1.summary()
                                        );
                                        store.record_warning(&dendro_core::WarningNote {
                                            path: id.clone(),
                                            line: None,
                                            reason: reason.clone(),
                                        })?;
                                        notes.push(format!("{shown} file={id} {reason}"));
                                        refused += 1;
                                        continue;
                                    }
                                    Err(error) => return Err(error),
                                };
                            coverage.merge(file_coverage);
                            notes.push(format!("{shown} file={id} {}", file_coverage.summary()));
                            for reason in warnings {
                                store.record_warning(&dendro_core::WarningNote {
                                    path: id.clone(),
                                    line: None,
                                    reason: reason.into(),
                                })?;
                                notes.push(format!("{shown} {reason}"));
                            }
                            manifest.files.push(dendro_core::FileCutoff {
                                source_id: id,
                                harness: harness.into(),
                                byte_length: watermark.byte_offset,
                                prefix_sha256: watermark
                                    .session_id
                                    .as_deref()
                                    .and_then(|value| value.strip_prefix("sdk-v1:"))
                                    .ok_or_else(|| anyhow::anyhow!("missing export prefix digest"))?
                                    .into(),
                            });
                            file_count += 1;
                        }
                        coverage.partial |= refused != 0;
                        notes.push(format!("{shown} files={file_count} refused={refused} format=verified-export-only native-storage=unsupported {}", coverage.summary()));
                    }
                    ReaderRoute::PresentNotParsed => {
                        notes.push(format!("{shown} present-not-parsed"));
                    }
                    ReaderRoute::Unsupported => notes.push(format!("{shown} unsupported")),
                }
            }
        }
    }
    if let Some(sealed) = sealed {
        if let Some(id) = sealed
            .files
            .iter()
            .map(|entry| &entry.source_id)
            .filter(|id| !seen_files.contains(*id))
            .chain(
                sealed
                    .openrig
                    .iter()
                    .map(|entry| &entry.source_id)
                    .filter(|id| !seen_openrig.contains(*id)),
            )
            .next()
        {
            return Err(anyhow::anyhow!(
                "mismatch: source {} missing or outside the selected roots",
                dendro_core::redact(id)
            ));
        }
    }
    if roots.iter().any(|root| root.name == "openrig") {
        if lifecycle_complete {
            // A reset's accepted lifetimes remain history on every later ingest.
            // A surviving lifetime's current stop edge replaces its earlier open
            // observation, rather than accumulating duplicate intervals.
            if store
                .load_openrig_row_records()?
                .iter()
                .any(|row| row.table_name == "events" && row.row_state == "superseded")
            {
                for old in store.load_seat_bounds()? {
                    if !lives.iter().any(|current| {
                        current.seat == old.seat && current.created_at == old.created_at
                    }) {
                        lives.push(old);
                    }
                }
            }
            lives.sort_by(|a, b| {
                (&a.seat, a.created_at, a.stopped_at).cmp(&(&b.seat, b.created_at, b.stopped_at))
            });
            lives.dedup_by(|a, b| {
                a.seat == b.seat && a.created_at == b.created_at && a.stopped_at == b.stopped_at
            });
            store.replace_seat_bounds(&lives)?;
        } else {
            notes.push("openrig lifecycle unchanged: a root is missing".to_string());
        }
    }
    manifest.redact_identifiers();
    Ok((notes, manifest))
}

/// A deterministic, content-free identity. Unlike the persisted display id,
/// the manifest never contains source paths, including ordinary non-secret paths.
fn manifest_source_id(root: &SourceRoot, file: &Path) -> String {
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    let canonical = dendro_core::canonical_root(&root.path);
    for value in [
        root.name.as_bytes(),
        canonical.as_os_str().as_encoded_bytes(),
        file.strip_prefix(&root.path)
            .unwrap_or(file)
            .as_os_str()
            .as_encoded_bytes(),
    ] {
        digest.update((value.len() as u64).to_le_bytes());
        digest.update(value);
    }
    hex::encode(digest.finalize())
}

#[allow(clippy::too_many_arguments)]
fn ingest_openrig(
    store: &AnalyticsStore,
    root: &SourceRoot,
    key: &[u8],
    lives: &mut Vec<dendro_core::SeatBound>,
    stage_dir: &Path,
    manifest_id: &str,
    sealed: Option<&dendro_core::OpenRigCutoff>,
) -> Result<(
    i64,
    crate::snapshot::Snapshot,
    dendro_core::OpenRigCutoff,
    bool,
)> {
    let snap_dir = stage_dir.join(dendro_core::lifecycle::SNAPSHOT_DIR);
    let snapshot =
        crate::snapshot::snapshot_sqlite(&root.path, &snap_dir, &dendro_core::RUN_CANCEL).map_err(
            |error| {
                if sealed.is_some() && error.downcast_ref::<dendro_core::Interrupted>().is_none() {
                    anyhow::anyhow!(
                        "mismatch: source {} snapshot unavailable",
                        dendro_core::redact(manifest_id)
                    )
                } else {
                    error
                }
            },
        )?;
    let mut prepared =
        crate::openrig::prepare_openrig_snapshot(&snapshot.path, manifest_id, key, sealed)?;
    let source_id = stable_source_id(&root.name, &root.path, &root.path, key);
    prepared.rows.push(dendro_core::OpenRigRow {
        table_name: "source_identity".into(),
        row_id: "file".into(),
        row_digest: snapshot.source_identity_sha256.clone(),
    });
    let provenance = store.begin_openrig_ingest(&source_id, &snapshot.revision, &prepared.rows)?;
    store.commit_file_batch(
        &source_id,
        "openrig",
        &root.path.display().to_string(),
        provenance.generation,
        snapshot.main_bytes as i64 + snapshot.wal_bytes.unwrap_or(0) as i64,
        &snapshot.revision,
        None,
        &[],
        &[],
    )?;
    for disclosure in &prepared.disclosures {
        store.record_mutable_source_disclosure(
            &disclosure.qitem_id,
            &disclosure.seat,
            &disclosure.reason,
        )?;
    }
    let result = store.with_openrig_fact_batch(|| {
        read_openrig_with_source(store, &snapshot.path, key, lives, Some(&source_id))
    });
    let removed = remove_snapshot_copy(stage_dir, &snapshot.path);
    let count = result?;
    removed.map_err(|_| anyhow::anyhow!("snapshot cleanup failed"))?;
    Ok((count, snapshot, prepared.cutoff, provenance.changed))
}

/// Cleanup accepts only a snapshot-role direct child of the current stage.
/// The snapshot allocator may return a unique leaf, so the requested name is
/// never used to guess which directory contains the accepted copy.
fn remove_snapshot_copy(stage_dir: &Path, copy_path: &Path) -> Result<()> {
    let stage_dir = std::path::absolute(stage_dir)?;
    let copy_path = std::path::absolute(copy_path)?;
    let snapshot_dir = copy_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("snapshot directory unavailable"))?;
    let name = snapshot_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    let snapshot_role = name == dendro_core::lifecycle::SNAPSHOT_DIR
        || name.strip_prefix("snapshot-").is_some_and(|suffix| {
            suffix.len() == 16
                && suffix
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        });
    if snapshot_dir.parent() != Some(stage_dir.as_path())
        || copy_path.file_name() != Some(std::ffi::OsStr::new(crate::snapshot::SNAPSHOT_MAIN))
        || !snapshot_role
    {
        return Err(anyhow::anyhow!(
            "snapshot cleanup path is outside the allocated stage role"
        ));
    }
    std::fs::remove_dir_all(snapshot_dir)?;
    Ok(())
}

fn read_openrig_with_source(
    store: &AnalyticsStore,
    open_path: &Path,
    key: &[u8],
    lives: &mut Vec<dendro_core::SeatBound>,
    source_id: Option<&str>,
) -> Result<i64> {
    let fact_key = |table: &str, row: &str, key: String| -> Result<String> {
        match source_id {
            Some(source) => store.qualify_openrig_fact(source, table, row, &key),
            None => Ok(key),
        }
    };
    // `open_path` is always the private copy, so SQLite may keep its own
    // index next to it inside the private stage.
    let conn = rusqlite::Connection::open_with_flags(open_path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    let mut rig_names = std::collections::HashMap::<String, String>::new();
    if let Ok(mut rigs) = conn.prepare("SELECT id, name FROM rigs") {
        let rows = rigs.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows.flatten() {
            dendro_core::RUN_CANCEL.check()?;
            rig_names.insert(row.0, row.1);
        }
    }
    let created_items = crate::openrig::created_items(&conn)?;
    let mut item_meta = queue_item_metadata(&conn, &created_items)?;
    let mut event_nudges = std::collections::BTreeMap::new();
    let mut count = 0i64;
    let mut lifecycle = Vec::new();
    // K5 reroute marks, by raw qitem id.
    let mut rerouted = std::collections::BTreeSet::<String>::new();
    // Chain of record: a handoff closes the item and opens a new one in the same
    // chain, so returns are found across the items of one chain.
    let mut chains = Vec::new();
    if let Ok(mut stmt) = conn.prepare("SELECT qitem_id, chain_of_record FROM queue_items") {
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })?;
        chains.extend(rows.flatten().map(|(qitem, chain)| {
            let chain = created_items
                .get(&qitem)
                .and_then(|v| v.get("chainOfRecord"))
                .map(|v| {
                    if let Some(text) = v.as_str() {
                        text.to_string()
                    } else {
                        v.to_string()
                    }
                })
                .or(chain);
            (qitem, chain)
        }));
    }
    // None when the source has no handed_off_from column: no walk to cross-check.
    let parents = match conn.prepare("SELECT qitem_id, handed_off_from FROM queue_items") {
        Ok(mut stmt) => {
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
            })?;
            Some(
                rows.flatten()
                    .map(|(qitem, parent)| {
                        let parent = created_items
                            .get(&qitem)
                            .and_then(|v| v.get("handedOffFrom"))
                            .map(|v| v.as_str().map(str::to_owned))
                            .unwrap_or(parent);
                        (qitem, parent)
                    })
                    .collect::<Vec<_>>(),
            )
        }
        Err(_) => None,
    };
    let (chain_root, marked) = chain_roots(&chains, parents.as_deref());
    rerouted.extend(marked);
    // None: the chain is unavailable, and returns are found within the item only.
    let chain_of = |qitem: &str| -> Option<String> {
        chain_root
            .get(qitem)
            .map(|root| dendro_core::seal_identifier(key, "qitem", root))
    };
    let mut stmt =
        conn.prepare("SELECT seq, type, payload, created_at FROM events ORDER BY seq")?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
        ))
    })?;
    let mut stale_activity_seqs = Vec::<i64>::new();
    for row in rows {
        dendro_core::RUN_CANCEL.check()?;
        let (seq, kind, payload, created_at) = row?;
        count += 1;
        let Ok(parsed) = serde_json::from_str::<Value>(&payload) else {
            continue;
        };
        if let Some(qitem) = parsed
            .get("qitemId")
            .or_else(|| parsed.get("qitem_id"))
            .and_then(Value::as_str)
        {
            let meta = item_meta.entry(qitem.to_owned()).or_default();
            meta.tags.extend(queue_tags(parsed.get("tags")));
            if let Some(value) = parsed
                .get("lastNudgeResult")
                .or_else(|| parsed.get("last_nudge_result"))
                .and_then(Value::as_str)
            {
                event_nudges.insert(qitem.to_owned(), value.to_owned());
            }
        }
        let ts = dendro_core::parse_utc_time(&created_at);
        if let Some(event) = lifecycle_event(&kind, &parsed) {
            if let Some(at) = ts {
                lifecycle.push((at, seq, event));
            }
            continue;
        }
        if kind == "agent.session_identity" {
            let Some(session_id) = parsed.get("sessionId").and_then(|v| v.as_str()) else {
                continue;
            };
            let Some(runtime) = parsed.get("runtime").and_then(|v| v.as_str()) else {
                continue;
            };
            let Some(seat) = parsed.get("sessionName").and_then(|v| v.as_str()) else {
                continue;
            };
            let rig_id = parsed.get("rigId").and_then(|v| v.as_str()).unwrap_or("");
            let rig = rig_names.get(rig_id).map(String::as_str).unwrap_or(rig_id);
            store.insert_identity(&IdentityFact {
                source_key: fact_key("events", &seq.to_string(), format!("events:{seq}"))?,
                harness: normalize_harness(key, runtime),
                native_session: dendro_core::seal_identifier(
                    key,
                    &format!("session:{}", normalize_harness(key, runtime)),
                    session_id,
                ),
                rig: dendro_core::seal_identifier(key, "rig", rig),
                seat: dendro_core::seal_identifier(key, "seat", seat),
                ts,
            })?;
        } else if kind == "seat.activity_changed" || kind == "agent.activity" {
            let raw_state = parsed.get("state").and_then(|v| v.as_str()).or_else(|| {
                parsed
                    .get("activity")
                    .and_then(|a| a.get("state"))
                    .and_then(|v| v.as_str())
            });
            let Some(raw_state) = raw_state else {
                if kind == "seat.activity_changed" {
                    stale_activity_seqs.push(seq);
                }
                continue;
            };
            let Some(seat) = parsed
                .get("sessionName")
                .and_then(|v| v.as_str())
                .or_else(|| parsed.get("seat").and_then(|v| v.as_str()))
            else {
                continue;
            };
            let when = parsed
                .get("at")
                .and_then(|v| v.as_str())
                .or_else(|| {
                    parsed.get("activity").and_then(|a| {
                        a.get("eventAt")
                            .or_else(|| a.get("sampledAt"))
                            .and_then(|v| v.as_str())
                    })
                })
                .and_then(dendro_core::parse_utc_time)
                .or(ts);
            let Some(when) = when else { continue };
            store.insert_activity(&ActivityFact {
                source_key: fact_key("events", &seq.to_string(), format!("{kind}:{seq}"))?,
                seat: dendro_core::seal_identifier(key, "seat", seat),
                ts: when,
                state: raw_state.to_string(),
                seq,
            })?;
        } else if kind == "qitem.fallback_routed" {
            let text = |name: &str| parsed.get(name).and_then(|v| v.as_str());
            let Some(qitem) = text("qitemId") else {
                continue;
            };
            rerouted.insert(qitem.to_string());
            let (Some(original), Some(reroute), Some(when)) =
                (text("originalDestination"), text("rerouteDestination"), ts)
            else {
                continue;
            };
            store.insert_queue(&QueueFact {
                source_key: fact_key("events", &seq.to_string(), format!("events:{kind}:{seq}"))?,
                transition_id: seq,
                qitem_id: dendro_core::seal_identifier(key, "qitem", qitem),
                ts: when,
                state: "pending".to_string(),
                source_seat: dendro_core::seal_identifier(key, "seat", original),
                dest_seat: dendro_core::seal_identifier(key, "seat", reroute),
                closure_reason: None,
                closure_target: None,
                chain_id: chain_of(qitem),
                missions: Vec::new(),
            })?;
        } else if kind == "queue.unclaimed" {
            // Not a closure and no destination change: it ends an in-progress interval (K5).
            let text = |name: &str| parsed.get(name).and_then(|v| v.as_str());
            let (Some(qitem), Some(destination), Some(when)) =
                (text("qitemId"), text("destinationSession"), ts)
            else {
                continue;
            };
            store.insert_queue(&QueueFact {
                source_key: fact_key("events", &seq.to_string(), format!("events:{kind}:{seq}"))?,
                transition_id: seq,
                qitem_id: dendro_core::seal_identifier(key, "qitem", qitem),
                ts: when,
                state: "pending".to_string(),
                source_seat: dendro_core::seal_identifier(key, "seat", destination),
                dest_seat: dendro_core::seal_identifier(key, "seat", destination),
                closure_reason: None,
                closure_target: None,
                chain_id: chain_of(qitem),
                missions: Vec::new(),
            })?;
        } else if kind == "queue.created" || kind == "queue.claimed" {
            let Some(qitem) = parsed.get("qitemId").and_then(|v| v.as_str()) else {
                continue;
            };
            let Some(destination) = parsed.get("destinationSession").and_then(|v| v.as_str())
            else {
                continue;
            };
            let when = parsed
                .get("claimedAt")
                .and_then(|v| v.as_str())
                .and_then(|text| DateTime::parse_from_rfc3339(text).ok())
                .map(|value| value.with_timezone(&Utc))
                .or(ts);
            let Some(when) = when else { continue };
            let source = parsed
                .get("sourceSession")
                .and_then(|v| v.as_str())
                .unwrap_or(destination);
            store.insert_queue(&QueueFact {
                source_key: fact_key("events", &seq.to_string(), format!("events:{kind}:{seq}"))?,
                transition_id: seq,
                qitem_id: dendro_core::seal_identifier(key, "qitem", qitem),
                ts: when,
                state: if kind == "queue.claimed" {
                    "in-progress".to_string()
                } else {
                    "pending".to_string()
                },
                source_seat: dendro_core::seal_identifier(key, "seat", source),
                dest_seat: dendro_core::seal_identifier(key, "seat", destination),
                closure_reason: None,
                closure_target: None,
                chain_id: chain_of(qitem),
                missions: if kind == "queue.created" {
                    queue_tags(parsed.get("tags"))
                        .iter()
                        .filter_map(|tag| tag.strip_prefix("mission:"))
                        .map(|mission| dendro_core::seal_identifier(key, "mission", mission))
                        .collect()
                } else {
                    Vec::new()
                },
            })?;
        }
    }
    store.purge_stateless_activity(&stale_activity_seqs)?;
    lives.extend(
        seat_lives(lifecycle)
            .into_iter()
            .map(|life| dendro_core::SeatBound {
                seat: dendro_core::seal_identifier(key, "seat", &life.seat),
                ..life
            }),
    );
    let mut transitions = conn.prepare(
        "SELECT transition_id, qitem_id, ts, state, actor_session, closure_reason, closure_target, transition_note FROM queue_transitions",
    );
    if let Ok(ref mut stmt) = transitions {
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<String>>(7)?,
            ))
        })?;
        for row in rows.flatten() {
            dendro_core::RUN_CANCEL.check()?;
            if row
                .7
                .as_deref()
                .is_some_and(|note| note.starts_with("fallback-routed:"))
            {
                rerouted.insert(row.1.clone());
            }
            let ts = DateTime::parse_from_rfc3339(&row.2)
                .ok()
                .map(|value| value.with_timezone(&Utc));
            let Some(ts) = ts else { continue };
            let actor_raw = row.4.clone().unwrap_or_default();
            let actor = dendro_core::seal_identifier(key, "seat", &actor_raw);
            let target = match row.6.as_deref() {
                Some(value) => dendro_core::seal_identifier(key, "seat", value),
                None => "unknown".to_string(),
            };
            store.insert_queue(&QueueFact {
                source_key: fact_key(
                    "queue_transitions",
                    &row.0.to_string(),
                    format!("queue_transitions:{}", row.0),
                )?,
                transition_id: row.0,
                qitem_id: dendro_core::seal_identifier(key, "qitem", &row.1),
                ts,
                state: dendro_core::queue_state(&row.3),
                source_seat: actor,
                dest_seat: target,
                closure_reason: dendro_core::closure_reason(row.5.as_deref()),
                closure_target: row
                    .6
                    .as_deref()
                    .map(|target| dendro_core::seal_identifier(key, "seat", target)),
                chain_id: chain_of(&row.1),
                missions: Vec::new(),
            })?;
        }
    }
    // K3: mission tags as set at creation. OpenRig 0.6.6 only appends
    // prompt-episode and recovery tags after creation, never mission tags.
    let mut item_missions = std::collections::BTreeMap::<String, Vec<String>>::new();
    if let Ok(mut stmt) = conn.prepare("SELECT qitem_id, tags FROM queue_items") {
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })?;
        for (qitem, tags) in rows.flatten() {
            dendro_core::RUN_CANCEL.check()?;
            let event_tags = created_items
                .get(&qitem)
                .and_then(|v| v.get("tags"))
                .map(|value| {
                    value
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| value.to_string())
                });
            let tags: Vec<String> = event_tags
                .as_deref()
                .or(tags.as_deref())
                .and_then(|text| serde_json::from_str(text).ok())
                .unwrap_or_default();
            let missions = tags
                .iter()
                .filter_map(|tag| tag.strip_prefix("mission:"))
                .map(|mission| dendro_core::seal_identifier(key, "mission", mission))
                .collect();
            item_missions.insert(qitem, missions);
        }
    }
    if let Ok(mut stmt) = conn.prepare("SELECT rowid, data FROM snapshots") {
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?;
        for (snapshot_rowid, data) in rows.flatten() {
            dendro_core::RUN_CANCEL.check()?;
            for (seat, role, evidence) in snapshot_roles(&data) {
                let seat = dendro_core::seal_identifier(key, "seat", &seat);
                let evidence = if source_id.is_some() {
                    fact_key(
                        "snapshots",
                        &snapshot_rowid.to_string(),
                        format!("snapshots:role:{snapshot_rowid}:{seat}:{role}:{evidence}"),
                    )?
                } else {
                    evidence
                };
                store.insert_seat_role(&seat, role, &evidence)?;
            }
            for (seat, skill_name) in snapshot_skills(&data) {
                let seat_sealed = dendro_core::seal_identifier(key, "seat", &seat);
                let skill_sealed = dendro_core::seal_identifier(key, "skill", &skill_name);
                store.insert_skill(&dendro_core::SkillFact {
                    source_key: fact_key(
                        "snapshots",
                        &snapshot_rowid.to_string(),
                        format!("snapshots:skill:{snapshot_rowid}:{seat_sealed}:{skill_sealed}"),
                    )?,
                    seat: seat_sealed,
                    skill: skill_sealed,
                    invoked: false,
                })?;
            }
        }
    }
    let mut items =
        conn.prepare("SELECT qitem_id, destination_session, ts_updated FROM queue_items");
    if let Ok(ref mut stmt) = items {
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?;
        for row in rows.flatten() {
            dendro_core::RUN_CANCEL.check()?;
            let Some(destination) = row.1 else { continue };
            let cutoff = row.2.unwrap_or_else(|| "unknown".to_string());
            store.insert_destination_snapshot(
                &dendro_core::seal_identifier(key, "qitem", &row.0),
                &dendro_core::seal_identifier(key, "seat", &destination),
                &cutoff,
                rerouted.contains(&row.0),
                item_missions.get(&row.0).map(Vec::as_slice).unwrap_or(&[]),
            )?;
            if let Some(source_id) = source_id {
                store.record_openrig_destination(
                    source_id,
                    &dendro_core::seal_identifier(key, "qitem", &row.0),
                )?;
            }
        }
    }
    // Write enrichment after all queue facts exist. Mission tags above retain
    // their K3 creation-time meaning; parent links expose additive lineage.
    for (qitem, mut raw) in item_meta {
        dendro_core::RUN_CANCEL.check()?;
        if raw.last_nudge_result.is_none() {
            raw.last_nudge_result = event_nudges.remove(&qitem);
        }
        raw.tags.sort();
        raw.tags.dedup();
        let recovery_prefix = qitem.starts_with("qitem-recovery-");
        let recovery_reason = raw
            .tags
            .iter()
            .filter_map(|tag| tag.strip_prefix("stuck-sweep:"))
            .find_map(safe_queue_token)
            .or_else(|| recovery_prefix.then(|| "qitem-recovery".to_owned()));
        store.record_queue_item_meta(&dendro_core::QueueItemMeta {
            qitem_id: dendro_core::seal_identifier(key, "qitem", &qitem),
            is_recovery: recovery_prefix
                || raw.tags.iter().any(|tag| tag.starts_with("stuck-sweep:")),
            recovery_reason,
            last_nudge_result: raw.last_nudge_result.as_deref().and_then(safe_queue_token),
            issue_tags: raw
                .tags
                .iter()
                .filter_map(|tag| tag.strip_prefix("issue:"))
                .map(|value| dendro_core::seal_identifier(key, "issue", value))
                .collect(),
            phase_tags: raw
                .tags
                .iter()
                .filter_map(|tag| tag.strip_prefix("phase:"))
                .map(|value| dendro_core::seal_identifier(key, "phase", value))
                .collect(),
            parent_qitem_id: raw
                .parent
                .as_deref()
                .filter(|value| !value.is_empty())
                .map(|value| dendro_core::seal_identifier(key, "qitem", value)),
        })?;
    }
    Ok(count)
}

#[derive(Default)]
struct RawQueueItemMeta {
    tags: Vec<String>,
    parent: Option<String>,
    last_nudge_result: Option<String>,
}

fn queue_tags(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(tags)) => tags
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        Some(Value::String(text)) => serde_json::from_str(text).unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn safe_queue_token(value: &str) -> Option<String> {
    (!value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-._".contains(&byte))
        && !dendro_core::contains_secret(value))
    .then(|| value.to_owned())
}

/// OpenRig versions differ in their optional queue-item columns. Read named
/// columns without making any one enrichment column mandatory.
fn queue_item_metadata(
    conn: &rusqlite::Connection,
    created: &std::collections::BTreeMap<String, Value>,
) -> Result<std::collections::BTreeMap<String, RawQueueItemMeta>> {
    let mut items = std::collections::BTreeMap::new();
    if let Ok(mut stmt) = conn.prepare("SELECT * FROM queue_items") {
        let id = stmt.column_index("qitem_id").ok();
        let tags = stmt.column_index("tags").ok();
        let parent = stmt.column_index("handed_off_from").ok();
        let nudge = stmt.column_index("last_nudge_result").ok();
        if let Some(id) = id {
            let mut rows = stmt.query([])?;
            while let Some(row) = rows.next()? {
                dendro_core::RUN_CANCEL.check()?;
                let text = |column: Option<usize>| -> rusqlite::Result<Option<String>> {
                    column.map(|index| row.get(index)).unwrap_or(Ok(None))
                };
                let tags = text(tags)?;
                items.insert(
                    row.get::<_, String>(id)?,
                    RawQueueItemMeta {
                        tags: tags
                            .as_deref()
                            .and_then(|text| serde_json::from_str(text).ok())
                            .unwrap_or_default(),
                        parent: text(parent)?,
                        last_nudge_result: text(nudge)?,
                    },
                );
            }
        }
    }
    for (qitem, value) in created {
        let meta = items.entry(qitem.clone()).or_default();
        meta.tags.extend(queue_tags(value.get("tags")));
        if let Some(parent) = value
            .get("handedOffFrom")
            .or_else(|| value.get("handed_off_from"))
        {
            meta.parent = parent.as_str().map(str::to_owned);
        }
    }
    Ok(items)
}

/// Each item's chain root. Two source readings must agree: the first item in
/// `chain_of_record` (the item itself when the list is empty) and the end of the
/// `handed_off_from` walk. Without the walk column the explicit chain stands
/// alone. An item gets no root, never a guessed one, when the readings disagree,
/// the walk meets a cycle or a parent with no row, or chain_of_record is
/// malformed and the walk cannot finish. Also returns the items with a
/// `fallback-from:` reroute mark (K5).
fn chain_roots(
    chains: &[(String, Option<String>)],
    parents: Option<&[(String, Option<String>)]>,
) -> (
    std::collections::BTreeMap<String, String>,
    std::collections::BTreeSet<String>,
) {
    use std::collections::{BTreeMap, BTreeSet};
    let parent: Option<BTreeMap<&str, Option<&str>>> = parents.map(|rows| {
        rows.iter()
            .map(|(qitem, from)| (qitem.as_str(), from.as_deref().filter(|f| !f.is_empty())))
            .collect()
    });
    let walk = |start: &str| -> Option<String> {
        let parent = parent.as_ref()?;
        let mut seen = BTreeSet::new();
        let mut at = start;
        loop {
            if !seen.insert(at) {
                return None;
            }
            match parent.get(at)? {
                Some(next) => at = next,
                None => return Some(at.to_string()),
            }
        }
    };
    let mut roots = BTreeMap::new();
    let mut marked = BTreeSet::new();
    for (qitem, chain) in chains {
        let listed: Option<Vec<String>> = match chain.as_deref() {
            None | Some("") => Some(Vec::new()),
            Some(text) => serde_json::from_str(text).ok(),
        };
        if let Some(entries) = &listed {
            if entries
                .iter()
                .any(|entry| entry.starts_with("fallback-from:"))
            {
                marked.insert(qitem.clone());
            }
        }
        let head = listed.map(|entries| {
            entries
                .into_iter()
                .find(|entry| !entry.starts_with("fallback-from:"))
                .unwrap_or_else(|| qitem.clone())
        });
        let root = match (head, walk(qitem), parent.is_some()) {
            (head, _, false) => head,
            (Some(head), Some(walked), true) => (head == walked).then_some(head),
            (Some(_), None, true) => None,
            (None, walked, true) => walked,
        };
        if let Some(root) = root {
            roots.insert(qitem.clone(), root);
        }
    }
    (roots, marked)
}

/// K7 role evidence from one recorded rig snapshot: (session, role, evidence).
/// AgentFlow phase seats (`agentflow-seat` spec) check when they are `dev.qa`
/// or `rev.review`. Elsewhere a seat checks when its recorded spec is a QA or
/// review spec. Every other recorded seat is `other`. Names are never read.
fn snapshot_roles(data: &str) -> Vec<(String, &'static str, String)> {
    let Ok(snapshot) = serde_json::from_str::<Value>(data) else {
        return Vec::new();
    };
    let text =
        |value: &Value, name: &str| value.get(name).and_then(|v| v.as_str()).map(str::to_string);
    let nodes: std::collections::BTreeMap<String, &Value> = snapshot
        .get("nodes")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|node| Some((text(node, "id")?, node)))
        .collect();
    let mut roles = Vec::new();
    for session in snapshot
        .get("sessions")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
    {
        let (Some(name), Some(node)) = (
            text(session, "sessionName"),
            text(session, "nodeId").and_then(|id| nodes.get(&id).copied()),
        ) else {
            continue;
        };
        let spec = text(node, "resolvedSpecName").unwrap_or_default();
        let agent = text(node, "agentRef").unwrap_or_default();
        let logical = text(node, "logicalId").unwrap_or_default();
        let (role, evidence) = if spec == "agentflow-seat" {
            let check = logical == "dev.qa" || logical == "rev.review";
            (check, format!("agentflow-seat:{logical}"))
        } else {
            let last = agent.rsplit('/').next().unwrap_or("");
            let check = ["qa", "review", "reviewer"]
                .iter()
                .any(|kind| spec == *kind || last == *kind);
            (check, format!("spec:{agent}|{spec}"))
        };
        roles.push((name, if role { "check" } else { "other" }, evidence));
    }
    roles
}

fn snapshot_skills(data: &str) -> Vec<(String, String)> {
    let Ok(snapshot) = serde_json::from_str::<Value>(data) else {
        return Vec::new();
    };
    let text =
        |value: &Value, name: &str| value.get(name).and_then(|v| v.as_str()).map(str::to_string);
    let nodes: std::collections::BTreeMap<String, &Value> = snapshot
        .get("nodes")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|node| Some((text(node, "id")?, node)))
        .collect();
    let startup_context = snapshot.get("nodeStartupContext");
    let mut skills = Vec::new();
    for session in snapshot
        .get("sessions")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
    {
        let (Some(seat), Some(node_id)) = (text(session, "sessionName"), text(session, "nodeId"))
        else {
            continue;
        };
        let Some(node) = nodes.get(&node_id) else {
            continue;
        };
        if let Some(refs) = node.get("packageRefs").and_then(|v| v.as_array()) {
            for item in refs {
                if let Some(s) = item.as_str() {
                    let trimmed = s.trim();
                    if !trimmed.is_empty() {
                        skills.push((seat.clone(), trimmed.to_string()));
                    }
                }
            }
        }
        let startup_files = startup_context
            .and_then(|ctx| {
                if let Some(map) = ctx.as_object() {
                    map.get(&node_id)
                } else if let Some(arr) = ctx.as_array() {
                    arr.iter()
                        .find(|item| text(item, "nodeId").as_deref() == Some(&node_id))
                } else {
                    None
                }
            })
            .and_then(|nctx| nctx.get("resolvedStartupFiles"))
            .and_then(|v| v.as_array());

        if let Some(files) = startup_files {
            for file_entry in files {
                let path_str = file_entry
                    .get("path")
                    .and_then(|v| v.as_str())
                    .or_else(|| file_entry.get("absolutePath").and_then(|v| v.as_str()));
                if let Some(path) = path_str {
                    for skill_name in crate::stream::extract_skills_from_text(path) {
                        skills.push((seat.clone(), skill_name));
                    }
                }
            }
        }
    }
    skills
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Lifecycle {
    Launched {
        rig: String,
        node: String,
        seat: String,
    },
    SessionStopped {
        rig: String,
        node: String,
        seat: String,
    },
    NodeRemoved {
        rig: String,
        node: String,
    },
    RigEnded {
        rig: String,
    },
}

/// The K4 lifecycle events. `seat.fresh_launched` is launch evidence only: it
/// never proves the retired generation stopped. `session.detached` is a binding
/// detach, not a stop.
fn lifecycle_event(kind: &str, payload: &Value) -> Option<Lifecycle> {
    let text = |name: &str| {
        payload
            .get(name)
            .and_then(|v| v.as_str())
            .map(str::to_string)
    };
    let rig = text("rigId").unwrap_or_default();
    let node = text("nodeId").unwrap_or_default();
    match kind {
        "node.launched" | "seat.fresh_launched" => Some(Lifecycle::Launched {
            rig,
            node,
            seat: text("sessionName")?,
        }),
        "session.stopped" => Some(Lifecycle::SessionStopped {
            rig,
            node,
            seat: text("sessionName")?,
        }),
        "node.removed" => Some(Lifecycle::NodeRemoved {
            rig,
            node: text("nodeId")?,
        }),
        "rig.stopped" | "rig.deleted" => Some(Lifecycle::RigEnded {
            rig: text("rigId")?,
        }),
        _ => None,
    }
}

/// Builds each seat's recorded lives from lifecycle events, ordered by time then
/// sequence. A relaunch of a running seat continues its life. A stop with no
/// earlier launch keeps an unknown start. A stop is never inferred.
fn seat_lives(mut events: Vec<(DateTime<Utc>, i64, Lifecycle)>) -> Vec<dendro_core::SeatBound> {
    use std::collections::{BTreeMap, BTreeSet};
    events.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    let mut node_seat = BTreeMap::<(String, String), String>::new();
    let mut rig_seats = BTreeMap::<String, BTreeSet<String>>::new();
    let mut open = BTreeMap::<String, DateTime<Utc>>::new();
    let mut seen = BTreeSet::<String>::new();
    let mut lives = Vec::new();
    // A stop closes the open life, or records one with an unknown start for a
    // seat never seen before. A repeated stop changes nothing.
    let stop = |seat: &str,
                at: DateTime<Utc>,
                open: &mut BTreeMap<String, DateTime<Utc>>,
                seen: &mut BTreeSet<String>,
                lives: &mut Vec<_>| {
        if let Some(start) = open.remove(seat) {
            lives.push((seat.to_string(), Some(start), Some(at)));
        } else if seen.insert(seat.to_string()) {
            lives.push((seat.to_string(), None, Some(at)));
        }
    };
    for (at, _, event) in events {
        match event {
            Lifecycle::Launched { rig, node, seat } => {
                node_seat.insert((rig.clone(), node), seat.clone());
                rig_seats.entry(rig).or_default().insert(seat.clone());
                seen.insert(seat.clone());
                open.entry(seat).or_insert(at);
            }
            Lifecycle::SessionStopped { rig, node, seat } => {
                node_seat.insert((rig.clone(), node), seat.clone());
                rig_seats.entry(rig).or_default().insert(seat.clone());
                stop(&seat, at, &mut open, &mut seen, &mut lives);
            }
            Lifecycle::NodeRemoved { rig, node } => {
                if let Some(seat) = node_seat.get(&(rig, node)).cloned() {
                    stop(&seat, at, &mut open, &mut seen, &mut lives);
                }
            }
            Lifecycle::RigEnded { rig } => {
                for seat in rig_seats.get(&rig).cloned().unwrap_or_default() {
                    stop(&seat, at, &mut open, &mut seen, &mut lives);
                }
            }
        }
    }
    lives.extend(
        open.into_iter()
            .map(|(seat, start)| (seat, Some(start), None)),
    );
    lives
        .into_iter()
        .map(|(seat, created_at, stopped_at)| dendro_core::SeatBound {
            seat,
            created_at,
            stopped_at,
        })
        .collect()
}

pub(crate) fn stable_source_id(root_name: &str, root: &Path, file: &Path, key: &[u8]) -> String {
    let relative = file.strip_prefix(root).unwrap_or(file);
    dendro_core::seal_identifier(
        key,
        "source",
        &format!("{root_name}\n{}\n{}", root.display(), relative.display()),
    )
}

fn normalize_harness(key: &[u8], runtime: &str) -> String {
    let name = if runtime == "claude-code" {
        "claude"
    } else {
        runtime
    };
    dendro_core::seal_identifier(key, "harness", name)
}

#[cfg(test)]
mod tests {
    #[test]
    fn snapshot_cleanup_rejects_foreign_stage_and_non_role_paths() {
        let dir =
            std::env::temp_dir().join(format!("dendro-cleanup-containment-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let stage = dir.join("stage");
        let foreign = dir.join("foreign").join("snapshot-0123456789abcdef");
        let unrelated = stage.join("unrelated");
        std::fs::create_dir_all(&foreign).unwrap();
        std::fs::create_dir_all(&unrelated).unwrap();
        let foreign_copy = foreign.join(crate::snapshot::SNAPSHOT_MAIN);
        let unrelated_copy = unrelated.join(crate::snapshot::SNAPSHOT_MAIN);
        std::fs::write(&foreign_copy, b"foreign evidence").unwrap();
        std::fs::write(&unrelated_copy, b"unrelated evidence").unwrap();
        assert!(super::remove_snapshot_copy(&stage, &foreign_copy).is_err());
        assert!(super::remove_snapshot_copy(&stage, &unrelated_copy).is_err());
        assert_eq!(std::fs::read(&foreign_copy).unwrap(), b"foreign evidence");
        assert_eq!(
            std::fs::read(&unrelated_copy).unwrap(),
            b"unrelated evidence"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    use super::*;
    use chrono::TimeZone;

    fn at(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000 + secs, 0).unwrap()
    }

    fn launched(rig: &str, node: &str, seat: &str) -> Lifecycle {
        Lifecycle::Launched {
            rig: rig.into(),
            node: node.into(),
            seat: seat.into(),
        }
    }

    type SeatLifeSpan = (Option<DateTime<Utc>>, Option<DateTime<Utc>>);

    fn lives_of(events: Vec<(DateTime<Utc>, i64, Lifecycle)>, seat: &str) -> Vec<SeatLifeSpan> {
        seat_lives(events)
            .into_iter()
            .filter(|life| life.seat == seat)
            .map(|life| (life.created_at, life.stopped_at))
            .collect()
    }

    #[test]
    fn relaunch_after_a_stop_starts_a_new_life() {
        let events = vec![
            (at(0), 1, launched("r", "n", "s")),
            (
                at(10),
                2,
                Lifecycle::SessionStopped {
                    rig: "r".into(),
                    node: "n".into(),
                    seat: "s".into(),
                },
            ),
            (at(20), 3, launched("r", "n", "s")),
        ];
        assert_eq!(
            lives_of(events, "s"),
            vec![(Some(at(0)), Some(at(10))), (Some(at(20)), None)]
        );
    }

    #[test]
    fn relaunch_of_a_running_seat_continues_its_life() {
        let events = vec![
            (at(0), 1, launched("r", "n", "s")),
            (at(5), 2, launched("r", "n", "s")),
        ];
        assert_eq!(lives_of(events, "s"), vec![(Some(at(0)), None)]);
    }

    #[test]
    fn a_fresh_launch_opens_a_life_and_never_closes_one() {
        let payload = serde_json::json!({
            "rigId": "r", "nodeId": "n", "sessionName": "s", "sessionId": "new",
            "supersededSessionIds": ["old"], "retiringGeneration": "g1", "newGeneration": "g2"
        });
        let fresh = || lifecycle_event("seat.fresh_launched", &payload).unwrap();
        assert_eq!(fresh(), launched("r", "n", "s"));
        // While running: no stop for the retired generation, the life continues.
        let events = vec![(at(0), 1, launched("r", "n", "s")), (at(5), 2, fresh())];
        assert_eq!(lives_of(events, "s"), vec![(Some(at(0)), None)]);
        // After a stop: it opens a new life.
        let events = vec![
            (at(0), 1, launched("r", "n", "s")),
            (at(5), 2, lifecycle_event("rig.stopped", &payload).unwrap()),
            (at(9), 3, fresh()),
        ];
        assert_eq!(
            lives_of(events, "s"),
            vec![(Some(at(0)), Some(at(5))), (Some(at(9)), None)]
        );
        // As the only event: it opens the first life.
        assert_eq!(
            lives_of(vec![(at(3), 1, fresh())], "s"),
            vec![(Some(at(3)), None)]
        );
    }

    #[test]
    fn rig_stop_and_delete_end_every_seat_of_that_rig_only() {
        for kind in ["rig.stopped", "rig.deleted"] {
            let ended = lifecycle_event(kind, &serde_json::json!({"rigId": "a"})).unwrap();
            let events = vec![
                (at(0), 1, launched("a", "n1", "a1")),
                (at(1), 2, launched("a", "n2", "a2")),
                (at(2), 3, launched("b", "n3", "b1")),
                (at(9), 4, ended),
            ];
            assert_eq!(
                lives_of(events.clone(), "a1"),
                vec![(Some(at(0)), Some(at(9)))]
            );
            assert_eq!(
                lives_of(events.clone(), "a2"),
                vec![(Some(at(1)), Some(at(9)))]
            );
            assert_eq!(lives_of(events, "b1"), vec![(Some(at(2)), None)], "{kind}");
        }
    }

    #[test]
    fn node_removal_resolves_the_seat_through_its_launch() {
        let removed = |node: &str| Lifecycle::NodeRemoved {
            rig: "r".into(),
            node: node.into(),
        };
        let events = vec![
            (at(0), 1, launched("r", "n", "s")),
            (at(4), 2, removed("other")),
            (at(7), 3, removed("n")),
        ];
        let all = seat_lives(events);
        assert_eq!(all.len(), 1, "an unknown node names no seat");
        assert_eq!(all[0].stopped_at, Some(at(7)));
    }

    #[test]
    fn detach_status_and_unknown_events_are_not_stops() {
        let payload = serde_json::json!({"rigId": "r", "nodeId": "n", "sessionName": "s"});
        for kind in [
            "session.detached",
            "session.status_changed",
            "session.cleaned",
        ] {
            assert_eq!(lifecycle_event(kind, &payload), None, "{kind}");
        }
        let events = vec![(at(0), 1, launched("r", "n", "s"))];
        assert_eq!(lives_of(events, "s"), vec![(Some(at(0)), None)]);
    }

    #[test]
    fn a_stop_without_a_launch_keeps_an_unknown_start_and_counts_once() {
        let stopped = || Lifecycle::SessionStopped {
            rig: "r".into(),
            node: "n".into(),
            seat: "s".into(),
        };
        let events = vec![(at(3), 1, stopped()), (at(8), 2, stopped())];
        assert_eq!(lives_of(events, "s"), vec![(None, Some(at(3)))]);
    }

    #[test]
    fn equal_times_order_by_sequence() {
        let stopped = Lifecycle::SessionStopped {
            rig: "r".into(),
            node: "n".into(),
            seat: "s".into(),
        };
        // Listed out of order; seq 1 (launch) runs before seq 2 (stop).
        let events = vec![(at(5), 2, stopped), (at(5), 1, launched("r", "n", "s"))];
        assert_eq!(lives_of(events, "s"), vec![(Some(at(5)), Some(at(5)))]);
    }

    fn openrig_root(dir: &Path, name: &str, seat: &str) -> SourceRoot {
        let path = dir.join(format!("{name}.sqlite"));
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute(
            "CREATE TABLE events (seq INTEGER, type TEXT, payload TEXT, created_at TEXT)",
            [],
        )
        .unwrap();
        let payload =
            format!("{{\"rigId\":\"{name}\",\"nodeId\":\"n\",\"sessionName\":\"{seat}\"}}");
        conn.execute(
            "INSERT INTO events VALUES (1,'node.launched',?,'2026-10-09 01:00:00')",
            [&payload],
        )
        .unwrap();
        SourceRoot {
            name: "openrig".into(),
            path,
        }
    }

    fn stored_seats(db: &Path) -> Vec<String> {
        let store = AnalyticsStore::open_path(db, "key").unwrap();
        let mut seats: Vec<String> = store
            .load_seat_bounds()
            .unwrap()
            .into_iter()
            .map(|life| life.seat)
            .collect();
        seats.sort();
        seats
    }

    /// A note starting with `prefix` that contains every field in `fields`.
    fn note_has(notes: &[String], prefix: &str, fields: &[&str]) -> bool {
        notes
            .iter()
            .any(|n| n.starts_with(prefix) && fields.iter().all(|f| n.contains(f)))
    }

    #[test]
    fn ac1_counts_records_read_per_harness_and_a_missing_root_warns_and_continues() {
        #[cfg(any(windows, target_os = "linux"))]
        let fixture_root = dendro_core::platform_v1::create_user_private_root_with_choice(
            "dendro-ingest-fixture-",
            &dendro_core::platform_v1::StorageRootChoice::UserProfile,
        )
        .expect("explicit UserProfile staged ingest fixture");
        #[cfg(any(windows, target_os = "linux"))]
        let dir = fixture_root.path().to_path_buf();
        #[cfg(not(any(windows, target_os = "linux")))]
        let dir = std::env::temp_dir().join(format!("dendro-ac1-count-{}", std::process::id()));
        #[cfg(not(any(windows, target_os = "linux")))]
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(dir.join("claude")).unwrap();
        let db = dir.join("analytics.duckdb");
        drop(AnalyticsStore::open_path(&db, "key").unwrap());
        let line = |id: &str, input: i64| {
            format!("{{\"type\":\"assistant\",\"sessionId\":\"s\",\"timestamp\":\"2026-10-09T01:00:00Z\",\"message\":{{\"id\":\"{id}\",\"usage\":{{\"input_tokens\":{input},\"output_tokens\":0,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0}}}}}}")
        };
        let file = dir.join("claude").join("s.jsonl");
        // Records: a usage line, a blank line (not a record), a malformed line,
        // a duplicate of the first message, and a trailing partial line.
        std::fs::write(
            &file,
            format!(
                "{}\n\nnot-json\n{}\n{}",
                line("m1", 5),
                line("m1", 5),
                line("m2", 7)
            ),
        )
        .unwrap();
        let roots = [
            SourceRoot {
                name: "claude".into(),
                path: dir.join("claude"),
            },
            SourceRoot {
                name: "codex".into(),
                path: dir.join("absent-codex"),
            },
        ];
        let key = b"ac1-count-key";
        let notes = ingest_staged(&db, &roots, false, key).unwrap();
        assert!(
            note_has(
                &notes,
                "claude files=1",
                &["records_read=3", "usage_records=1", "coverage=partial"]
            ),
            "{notes:?}"
        );
        assert!(
            note_has(
                &notes,
                "codex missing",
                &["coverage=refused", "reason=source_missing"]
            ),
            "{notes:?}"
        );
        let store = AnalyticsStore::open_path(&db, "key").unwrap();
        assert!(store
            .warning_reasons()
            .unwrap()
            .contains(&"source codex missing".to_string()));
        // The other source still ran.
        assert_eq!(store.load_usage().unwrap().len(), 1);
        drop(store);
        // Unchanged file: nothing is read again.
        let notes = ingest_staged(&db, &roots, false, key).unwrap();
        assert!(
            note_has(
                &notes,
                "claude files=1",
                &["records_read=0", "usage_records=0"]
            ),
            "{notes:?}"
        );
        // Completing the partial line makes it exactly one new record.
        std::io::Write::write_all(
            &mut std::fs::OpenOptions::new()
                .append(true)
                .open(&file)
                .unwrap(),
            b"\n",
        )
        .unwrap();
        let notes = ingest_staged(&db, &roots, false, key).unwrap();
        assert!(
            note_has(
                &notes,
                "claude files=1",
                &["records_read=1", "usage_records=1"]
            ),
            "{notes:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn registry_routes_each_source_to_its_reader() {
        #[cfg(any(windows, target_os = "linux"))]
        let fixture_root = dendro_core::platform_v1::create_user_private_root_with_choice(
            "dendro-ingest-dispatch-",
            &dendro_core::platform_v1::StorageRootChoice::UserProfile,
        )
        .expect("explicit UserProfile staged ingest fixture");
        #[cfg(any(windows, target_os = "linux"))]
        let dir = fixture_root.path().to_path_buf();
        #[cfg(not(any(windows, target_os = "linux")))]
        let dir =
            std::env::temp_dir().join(format!("dendro-ingest-dispatch-{}", std::process::id()));
        #[cfg(not(any(windows, target_os = "linux")))]
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let present = [
            "claude", "codex", "pi", "grok", "cursor", "windsurf", "goose", "devin", "agy",
            "custom",
        ];
        for name in present {
            std::fs::create_dir_all(dir.join(name)).unwrap();
        }
        let db = dir.join("analytics.duckdb");
        drop(AnalyticsStore::open_path(&db, "key").unwrap());
        let mut roots: Vec<SourceRoot> = present
            .into_iter()
            .map(|name| SourceRoot {
                name: name.into(),
                path: dir.join(name),
            })
            .collect();
        roots.push(SourceRoot {
            name: "absent".into(),
            path: dir.join("absent"),
        });
        let notes = ingest_staged(&db, &roots, false, b"dispatch-key").unwrap();
        assert_eq!(notes.len(), 11, "{notes:?}");
        for h in ["claude", "pi", "grok"] {
            assert!(
                note_has(
                    &notes,
                    &format!("{h} files=0"),
                    &["records_read=0", "usage_records=0"]
                ),
                "{notes:?}"
            );
        }
        assert!(
            note_has(
                &notes,
                "codex files=0",
                &["records_read=0", "usage_records=0", "tool_calls=not read"]
            ),
            "{notes:?}"
        );
        for h in ["cursor", "windsurf", "goose", "devin"] {
            assert!(
                note_has(
                    &notes,
                    &format!("{h} files=0"),
                    &["format=verified-export-only", "native-storage=unsupported"]
                ),
                "{notes:?}"
            );
        }
        assert!(
            notes.contains(&"agy present-not-parsed".to_string()),
            "{notes:?}"
        );
        assert!(
            notes.contains(&"custom unsupported".to_string()),
            "{notes:?}"
        );
        assert!(
            note_has(
                &notes,
                "absent missing",
                &["coverage=refused", "reason=source_missing"]
            ),
            "{notes:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn openrig_roots_do_not_erase_each_others_lives() {
        #[cfg(any(windows, target_os = "linux"))]
        let fixture_root = dendro_core::platform_v1::create_user_private_root_with_choice(
            "dendro-ingest-fixture-",
            &dendro_core::platform_v1::StorageRootChoice::UserProfile,
        )
        .expect("explicit UserProfile staged ingest fixture");
        #[cfg(any(windows, target_os = "linux"))]
        let dir = fixture_root.path().to_path_buf();
        #[cfg(not(any(windows, target_os = "linux")))]
        let dir = std::env::temp_dir().join(format!("dendro-roots-{}", std::process::id()));
        #[cfg(not(any(windows, target_os = "linux")))]
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("analytics.duckdb");
        drop(AnalyticsStore::open_path(&db, "key").unwrap());
        let roots = [
            openrig_root(&dir, "rig-a", "seat-a"),
            openrig_root(&dir, "rig-b", "seat-b"),
        ];
        let key = b"roots-test-key";
        ingest_staged(&db, &roots, false, key).unwrap();
        assert!(
            std::fs::read_dir(&dir).unwrap().all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("snapshot")),
            "actual allocated snapshots were removed"
        );
        assert_eq!(stored_seats(&db), vec!["seat-a", "seat-b"]);
        // A second run rebuilds the same lives, without duplicates.
        ingest_staged(&db, &roots, false, key).unwrap();
        assert_eq!(stored_seats(&db), vec!["seat-a", "seat-b"]);
        // A run with one root missing keeps every stored life.
        let partial = [
            roots[0].clone(),
            SourceRoot {
                name: "openrig".into(),
                path: dir.join("absent.sqlite"),
            },
        ];
        let notes = ingest_staged(&db, &partial, false, key).unwrap();
        assert!(notes
            .iter()
            .any(|note| note == "openrig lifecycle unchanged: a root is missing"));
        assert_eq!(stored_seats(&db), vec!["seat-a", "seat-b"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn k5_reroute_marks_and_unclaim_reach_the_k2_report() {
        #[cfg(any(windows, target_os = "linux"))]
        let fixture_root = dendro_core::platform_v1::create_user_private_root_with_choice(
            "dendro-ingest-fixture-",
            &dendro_core::platform_v1::StorageRootChoice::UserProfile,
        )
        .expect("explicit UserProfile staged ingest fixture");
        #[cfg(any(windows, target_os = "linux"))]
        let dir = fixture_root.path().to_path_buf();
        #[cfg(not(any(windows, target_os = "linux")))]
        let dir = std::env::temp_dir().join(format!("dendro-k5-{}", std::process::id()));
        #[cfg(not(any(windows, target_os = "linux")))]
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("openrig.sqlite");
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE events (seq INTEGER, type TEXT, payload TEXT, created_at TEXT);
             CREATE TABLE queue_transitions (transition_id INTEGER, qitem_id TEXT, ts TEXT, state TEXT,
               actor_session TEXT, closure_reason TEXT, closure_target TEXT, transition_note TEXT);
             CREATE TABLE queue_items (qitem_id TEXT, destination_session TEXT, ts_updated TEXT,
               chain_of_record TEXT);",
        )
        .unwrap();
        let mut seq = 0;
        let mut event = |kind: &str, payload: serde_json::Value, at: &str| {
            seq += 1;
            conn.execute(
                "INSERT INTO events VALUES (?,?,?,?)",
                rusqlite::params![seq, kind, payload.to_string(), at],
            )
            .unwrap();
        };
        for item in ["q-note", "q-chain", "q-event", "q-clean"] {
            event(
                "queue.created",
                serde_json::json!({"qitemId": item, "sourceSession": "orch", "destinationSession": "seat-a"}),
                "2026-10-09 01:00:00",
            );
        }
        event(
            "qitem.fallback_routed",
            serde_json::json!({"qitemId": "q-event", "originalDestination": "seat-a",
                "rerouteDestination": "seat-b", "reason": "unreachable"}),
            "2026-10-09 01:30:00",
        );
        event(
            "queue.unclaimed",
            serde_json::json!({"qitemId": "q-clean", "destinationSession": "seat-a"}),
            "2026-10-09 01:40:00",
        );
        let mut transition = 0;
        let mut step = |item: &str, state: &str, note: Option<&str>| {
            transition += 1;
            conn.execute(
                "INSERT INTO queue_transitions VALUES (?,?,?,?,?,?,?,?)",
                rusqlite::params![
                    transition,
                    item,
                    format!("2026-10-09T02:00:{transition:02}.000Z"),
                    state,
                    "actor",
                    Option::<String>::None,
                    Option::<String>::None,
                    note
                ],
            )
            .unwrap();
        };
        step(
            "q-note",
            "pending",
            Some("fallback-routed: seat-a → seat-b (gone)"),
        );
        for item in ["q-note", "q-chain", "q-event", "q-clean", "q-nocreate"] {
            step(item, "done", None);
        }
        for (item, chain) in [
            ("q-note", "[]"),
            ("q-chain", "[\"fallback-from:seat-a\"]"),
            ("q-event", "[]"),
            ("q-clean", "[]"),
            ("q-nocreate", "[]"),
        ] {
            let destination = if item == "q-event" {
                "seat-b"
            } else {
                "seat-a"
            };
            conn.execute(
                "INSERT INTO queue_items VALUES (?,?,'2026-10-09T02:00:09.000Z',?)",
                rusqlite::params![item, destination, chain],
            )
            .unwrap();
        }
        drop(conn);
        let db = dir.join("analytics.duckdb");
        drop(AnalyticsStore::open_path(&db, "key").unwrap());
        let roots = [SourceRoot {
            name: "openrig".into(),
            path,
        }];
        ingest_staged(&db, &roots, false, b"k5-test-key").unwrap();
        let store = AnalyticsStore::open_path(&db, "key").unwrap();
        let marked: std::collections::BTreeMap<String, bool> = store
            .load_destination_snapshots()
            .unwrap()
            .into_iter()
            .map(|(item, snapshot)| (item, snapshot.reroute_marked))
            .collect();
        assert!(marked["q-note"], "transition note mark");
        assert!(marked["q-chain"], "chain_of_record mark");
        assert!(marked["q-event"], "reroute event mark");
        assert!(!marked["q-clean"]);
        assert!(!marked["q-nocreate"]);
        drop(store);
        let report = dendro_core::report_json(
            &db,
            "decisions",
            None,
            "2026-10-09T00:00:00Z",
            "2026-10-10T00:00:00Z",
        )
        .unwrap();
        let payload = &report["payload"];
        let mut insufficient: Vec<&str> = payload["k2_insufficient_history_members"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["qitem"].as_str().unwrap())
            .collect();
        insufficient.sort();
        assert_eq!(insufficient, vec!["q-chain", "q-nocreate", "q-note"]);
        assert_eq!(payload["k2_closed_items_with_insufficient_history"], 3);
        let closed: Vec<(String, String)> = metrics_members(payload);
        assert_eq!(
            closed,
            vec![
                ("q-clean".to_string(), "events:queue.created:4".to_string()),
                (
                    "q-event".to_string(),
                    "events:qitem.fallback_routed:5".to_string()
                ),
            ],
            "unclaim neither closes nor moves q-clean; the reroute moves q-event to seat-b"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn metrics_members(payload: &serde_json::Value) -> Vec<(String, String)> {
        let mut members: Vec<(String, String)> = payload["m1_by_seat"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|seat| seat["closed_item_members"].as_array().unwrap().clone())
            .map(|item| {
                (
                    item["qitem"].as_str().unwrap().to_string(),
                    item["destination_evidence"].as_str().unwrap().to_string(),
                )
            })
            .collect();
        members.sort();
        members
    }

    fn row(qitem: &str, value: Option<&str>) -> (String, Option<String>) {
        (qitem.to_string(), value.map(str::to_string))
    }

    #[test]
    fn chain_roots_need_agreeing_source_readings_and_never_guess() {
        let chains = vec![
            row("a", Some("[]")),
            row("b", Some("[\"a\"]")),
            row("c", Some("[\"a\",\"b\",\"fallback-from:seat-x\"]")),
            row("malformed-ok", Some("not json")),
            row("malformed-broken", Some("not json")),
            row("cycle-1", Some("[\"cycle-2\"]")),
            row("cycle-2", Some("[\"cycle-1\"]")),
            row("orphan", Some("[\"gone\"]")),
            row("disagree", Some("[\"a\"]")),
            row("no-chain-column-value", None),
        ];
        let parents = vec![
            row("a", None),
            row("b", Some("a")),
            row("c", Some("b")),
            row("malformed-ok", Some("a")),
            row("malformed-broken", Some("gone")),
            row("cycle-1", Some("cycle-2")),
            row("cycle-2", Some("cycle-1")),
            row("orphan", Some("gone")),
            row("disagree", Some("z")),
            row("z", None),
            row("no-chain-column-value", None),
        ];
        let (roots, marked) = chain_roots(&chains, Some(&parents));
        let root = |q: &str| roots.get(q).map(String::as_str);
        assert_eq!(root("a"), Some("a"), "a root names itself");
        assert_eq!(root("b"), Some("a"));
        assert_eq!(
            root("c"),
            Some("a"),
            "the reroute marker is not a chain item"
        );
        assert_eq!(root("malformed-ok"), Some("a"), "a finished walk stands in");
        assert_eq!(root("malformed-broken"), None);
        assert_eq!(root("cycle-1"), None);
        assert_eq!(root("cycle-2"), None);
        assert_eq!(root("orphan"), None, "a parent with no row ends the walk");
        assert_eq!(
            root("disagree"),
            None,
            "readings that disagree are not guessed"
        );
        assert_eq!(root("no-chain-column-value"), Some("no-chain-column-value"));
        assert!(marked.contains("c") && marked.len() == 1);
        // Without a handed_off_from column the explicit chain stands alone.
        let (alone, _) = chain_roots(&chains, None);
        assert_eq!(alone.get("disagree").map(String::as_str), Some("a"));
        assert_eq!(alone.get("malformed-ok"), None);
    }

    #[test]
    fn snapshot_roles_come_from_recorded_specs_not_names() {
        let snapshot = serde_json::json!({
            "nodes": [
                {"id": "n1", "logicalId": "dev.qa", "agentRef": "local:agents/agentflow", "resolvedSpecName": "agentflow-seat"},
                {"id": "n2", "logicalId": "rev.review", "agentRef": "local:agents/agentflow", "resolvedSpecName": "agentflow-seat"},
                {"id": "n3", "logicalId": "orch.arch", "agentRef": "local:agents/agentflow", "resolvedSpecName": "agentflow-seat"},
                {"id": "n4", "logicalId": "dev.review", "agentRef": "local:../../../agents/development/qa", "resolvedSpecName": "qa"},
                {"id": "n5", "logicalId": "dev.build", "agentRef": "local:../../../agents/development/implementer", "resolvedSpecName": "implementer"},
                {"id": "n6", "logicalId": "rev.lookalike", "agentRef": "local:agents/other", "resolvedSpecName": "writer"}
            ],
            "sessions": [
                {"nodeId": "n1", "sessionName": "dev-qa@agentflow-dev"},
                {"nodeId": "n2", "sessionName": "rev-review@agentflow-dev"},
                {"nodeId": "n3", "sessionName": "orch-arch@agentflow-dev"},
                {"nodeId": "n4", "sessionName": "dev-review@starter"},
                {"nodeId": "n5", "sessionName": "dev-build@starter"},
                {"nodeId": "n6", "sessionName": "rev-lookalike@other"},
                {"nodeId": "missing", "sessionName": "ghost@rig"}
            ]
        });
        let roles = snapshot_roles(&snapshot.to_string());
        let role = |seat: &str| {
            roles
                .iter()
                .find(|row| row.0 == seat)
                .map(|row| (row.1, row.2.as_str()))
        };
        assert_eq!(
            role("dev-qa@agentflow-dev"),
            Some(("check", "agentflow-seat:dev.qa"))
        );
        assert_eq!(
            role("rev-review@agentflow-dev"),
            Some(("check", "agentflow-seat:rev.review"))
        );
        assert_eq!(
            role("orch-arch@agentflow-dev"),
            Some(("other", "agentflow-seat:orch.arch"))
        );
        assert_eq!(
            role("dev-review@starter"),
            Some(("check", "spec:local:../../../agents/development/qa|qa"))
        );
        assert_eq!(role("dev-build@starter").unwrap().0, "other");
        assert_eq!(
            role("rev-lookalike@other").unwrap().0,
            "other",
            "a rev- name is not evidence"
        );
        assert_eq!(
            role("ghost@rig"),
            None,
            "a session without its node has no evidence"
        );
        assert!(snapshot_roles("not json").is_empty());
    }

    #[test]
    fn openrig_created_at_is_utc_seconds_without_a_zone() {
        assert_eq!(
            dendro_core::parse_utc_time("2026-10-10 15:38:41"),
            Some(Utc.with_ymd_and_hms(2026, 10, 10, 15, 38, 41).unwrap())
        );
    }

    #[test]
    fn snapshot_skills_extracts_package_refs_and_startup_files() {
        let snapshot = serde_json::json!({
            "nodes": [
                {
                    "id": "n1",
                    "packageRefs": ["git-workflow", "sk-ant-secret-pkg", ""]
                },
                {
                    "id": "n2",
                    "packageRefs": ["code-review"]
                }
            ],
            "sessions": [
                {"nodeId": "n1", "sessionName": "dev.build-sr"},
                {"nodeId": "n2", "sessionName": "dev.qa"},
                {"nodeId": "n_missing", "sessionName": "orphan"}
            ],
            "nodeStartupContext": {
                "n1": {
                    "resolvedStartupFiles": [
                        {"path": "skills/unit-test/SKILL.md"},
                        {"absolutePath": "/home/user/.openrig/skills/linter/SKILL.md"},
                        {"path": "other/file.txt"}
                    ]
                },
                "n2": {
                    "resolvedStartupFiles": [
                        {"path": "skills/qa-review/SKILL.md"}
                    ]
                }
            }
        });
        let skills = snapshot_skills(&snapshot.to_string());
        assert!(skills.contains(&("dev.build-sr".to_string(), "git-workflow".to_string())));
        assert!(skills.contains(&("dev.build-sr".to_string(), "sk-ant-secret-pkg".to_string())));
        assert!(skills.contains(&("dev.build-sr".to_string(), "unit-test".to_string())));
        assert!(skills.contains(&("dev.build-sr".to_string(), "linter".to_string())));
        assert!(skills.contains(&("dev.qa".to_string(), "code-review".to_string())));
        assert!(skills.contains(&("dev.qa".to_string(), "qa-review".to_string())));
        assert!(!skills.iter().any(|(seat, _)| seat == "orphan"));
        assert!(!skills
            .iter()
            .any(|(_, skill)| skill.is_empty() || skill == "file.txt"));
    }

    #[test]
    fn dual_root_pi_ingests_both_trees_without_collision() {
        #[cfg(any(windows, target_os = "linux"))]
        let fixture_root = dendro_core::platform_v1::create_user_private_root_with_choice(
            "dendro-ingest-fixture-",
            &dendro_core::platform_v1::StorageRootChoice::UserProfile,
        )
        .expect("explicit UserProfile staged ingest fixture");
        #[cfg(any(windows, target_os = "linux"))]
        let dir = fixture_root.path().to_path_buf();
        #[cfg(not(any(windows, target_os = "linux")))]
        let dir = std::env::temp_dir().join(format!(
            "dendro-dual-pi-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        #[cfg(not(any(windows, target_os = "linux")))]
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let openrig_tree = dir.join("openrig").join("state").join("pi");
        let standalone_tree = dir.join("home").join(".pi").join("agent").join("sessions");
        std::fs::create_dir_all(&openrig_tree).unwrap();
        std::fs::create_dir_all(&standalone_tree).unwrap();

        let s1_file = openrig_tree.join("s1.jsonl");
        let s2_file = standalone_tree.join("s2.jsonl");

        let pi_header = |id: &str| {
            format!("{{\"type\":\"session\",\"version\":3,\"id\":\"{id}\",\"timestamp\":\"2026-10-09T00:00:00Z\"}}")
        };
        let pi_usage = |msg: &str, input: i64| {
            format!("{{\"type\":\"message\",\"id\":\"{msg}\",\"timestamp\":\"2026-10-09T00:00:01Z\",\"message\":{{\"role\":\"assistant\",\"usage\":{{\"input\":{input},\"output\":0,\"cacheRead\":0,\"cacheWrite\":0}}}}}}")
        };

        std::fs::write(
            &s1_file,
            format!(
                "{}\n{}\n",
                pi_header("session-openrig-1"),
                pi_usage("m1", 42)
            ),
        )
        .unwrap();

        std::fs::write(
            &s2_file,
            format!(
                "{}\n{}\n",
                pi_header("session-standalone-2"),
                pi_usage("m2", 84)
            ),
        )
        .unwrap();

        let db = dir.join("analytics.duckdb");
        drop(AnalyticsStore::open_path(&db, "key").unwrap());
        let roots = vec![
            SourceRoot {
                name: "pi".into(),
                path: openrig_tree,
            },
            SourceRoot {
                name: "pi".into(),
                path: standalone_tree,
            },
        ];
        let key = b"dual-root-pi-test-key";
        let (notes, manifest) = ingest_staged_with_manifest(&db, &roots, false, key, None).unwrap();

        assert_eq!(manifest.files.len(), 2);
        assert_ne!(manifest.files[0].source_id, manifest.files[1].source_id);

        // Aggregate lines only (per-file lines read "pi file=<hash> ...").
        let pi_notes: Vec<_> = notes
            .iter()
            .filter(|n| n.starts_with("pi files="))
            .collect();
        assert_eq!(pi_notes.len(), 2, "{notes:?}");
        for n in pi_notes {
            assert!(
                n.contains("files=1")
                    && n.contains("records_read=2")
                    && n.contains("usage_records=1")
                    && n.contains("coverage=complete"),
                "{n}"
            );
        }

        let store = AnalyticsStore::open_path(&db, "key").unwrap();
        let sources = store.load_sources().unwrap();
        assert_eq!(sources.len(), 2);
        assert_ne!(sources[0].0, sources[1].0);

        let usage = store.load_usage().unwrap();
        assert_eq!(usage.len(), 2);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
