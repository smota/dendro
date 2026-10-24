//! Explicit schema migration and rollback through the same native pointer CAS.
//! All preimages and loss evidence travel inside the selected immutable bundle.
use super::*;
use crate::migration_v4::{self as m, MigrationPublicationFile, MigrationPublicationReady};

const CUSTODY: [(&str, &str); 5] = [
    ("migration", "migration.json"),
    ("legacy_database", "schema3-preimage.duckdb"),
    ("legacy_key", "schema3-preimage.key"),
    ("migration_identity", "migration.identity"),
    ("migration_seal", "migration.content.sha256"),
];
const LOSS: [(&str, &str); 2] = [
    ("failed_database", "failed-schema4.duckdb"),
    ("rollback_loss", "rollback-loss.json"),
];

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MigrationRecord {
    schema: String,
    version: u32,
    key_id: String,
    schema3_sha256: String,
    schema3_key_sha256: String,
    legacy_projection_sha256: String,
    initial_schema4_sha256: String,
    identity_record_sha256: String,
    content_seal_sha256: String,
    legacy_anchor: Option<LegacyAnchor>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyAnchor {
    identity: Identity,
    size: u64,
    modified_unix_ns: i128,
    sha256: String,
    scope: String,
}

fn legacy_anchor(binding: &LogicalDbBinding) -> Result<Option<LegacyAnchor>> {
    if binding.record.anchor_kind != "legacy" {
        return Ok(None);
    }
    let metadata = p::reopen_metadata(&binding.logical)?;
    let (hash, size) = hash_file(&binding.logical)?;
    let after = p::reopen_metadata(&binding.logical)?;
    if metadata != after {
        return Err(anyhow!(
            "legacy anchor changed during migration baseline capture"
        ));
    }
    Ok(Some(LegacyAnchor {
        identity: metadata.identity.try_into()?,
        size,
        modified_unix_ns: metadata.modified_unix_ns,
        sha256: hex::encode(hash),
        scope: "captured_at_migration; prior anchor divergence is not inferred".into(),
    }))
}

pub(super) fn validate_legacy_anchor(
    binding: &LogicalDbBinding,
    resolved: &p::ResolvedSelection,
) -> Result<()> {
    if binding.record.anchor_kind != "legacy" || resolved.manifest.payload.files.len() == 4 {
        return Ok(());
    }
    let record: MigrationRecord = read_pinned_record(
        resolved
            .file("migration.json")
            .ok_or_else(|| anyhow!("migration lineage missing"))?,
    )?;
    let expected = record
        .legacy_anchor
        .ok_or_else(|| anyhow!("legacy anchor baseline unavailable"))?;
    let current = p::reopen_metadata(&binding.logical)?;
    if (current.identity != expected.identity.into()
        || current.size != expected.size
        || current.modified_unix_ns != expected.modified_unix_ns)
        && hash_file(&binding.logical)?.0 != digest(&expected.sha256)?
    {
        return Err(anyhow!("legacy_anchor_diverged_after_selection: selected data retained; reconcile the old-binary anchor write explicitly"));
    }
    Ok(())
}

fn digest(value: &str) -> Result<[u8; 32]> {
    hex::decode(value)?
        .try_into()
        .map_err(|_| anyhow!("invalid migration digest"))
}

fn add_file(
    bundle: &PrivateRoot,
    files: &mut Vec<p::ClosedBundleFile>,
    role: &str,
    leaf: &str,
) -> Result<()> {
    let verified = p::open_existing_private_file(bundle, &name(leaf)?)?;
    let (expected_sha256, expected_size) = hash_file(verified.path())?;
    files.push(p::ClosedBundleFile {
        role: role.into(),
        name: name(leaf)?,
        expected_identity: verified.metadata().identity,
        expected_size,
        expected_sha256,
    });
    Ok(())
}

fn copy_held(
    bundle: &PrivateRoot,
    leaf: &str,
    source: &File,
    expected_hash: [u8; 32],
    expected_size: u64,
) -> Result<()> {
    RUN_CANCEL.check()?;
    let mut source = source.try_clone()?;
    source.seek(SeekFrom::Start(0))?;
    let mut destination = bundle.create_file(&name(leaf)?)?;
    let count = std::io::copy(&mut source, destination.file_mut())?;
    destination.file_mut().sync_all()?;
    drop(destination);
    if count != expected_size
        || hash_file(&bundle.path().join(leaf))? != (expected_hash, expected_size)
    {
        return Err(anyhow!("migration private copy differs from held source"));
    }
    RUN_CANCEL.check()?;
    Ok(())
}

fn copy_ready(
    bundle: &PrivateRoot,
    leaf: &str,
    source: &File,
    evidence: &MigrationPublicationFile,
) -> Result<()> {
    copy_held(
        bundle,
        leaf,
        source,
        digest(evidence.sha256())?,
        evidence.size(),
    )
}

fn standard_records(bundle: &PrivateRoot, key_id: &str) -> Result<Vec<p::ClosedBundleFile>> {
    let (database_sha256, _) = hash_file(&bundle.path().join("analytics.duckdb"))?;
    let (key_sha256, _) = hash_file(&bundle.path().join("analytics.key"))?;
    write_record(
        bundle,
        "provenance.json",
        &BundleProvenance {
            schema: "dendro.bundle-provenance".into(),
            version: 1,
            database_sha256,
            key_id: key_id.into(),
        },
    )?;
    let (provenance_sha256, _) = hash_file(&bundle.path().join("provenance.json"))?;
    write_record(
        bundle,
        "version.json",
        &BundleVersion {
            schema: "dendro.bundle-version".into(),
            version: 1,
            database_sha256,
            key_sha256,
            provenance_sha256,
        },
    )?;
    let mut files = Vec::new();
    for (role, leaf) in [
        ("database", "analytics.duckdb"),
        ("key", "analytics.key"),
        ("provenance", "provenance.json"),
        ("version", "version.json"),
    ] {
        add_file(bundle, &mut files, role, leaf)?;
    }
    Ok(files)
}

fn bundle_from_ready(
    binding: &LogicalDbBinding,
    ready: &MigrationPublicationReady,
) -> Result<(PrivateRoot, Vec<p::ClosedBundleFile>)> {
    ready.verify_unchanged()?;
    let bundle = p::create_unique_private_directory(&binding.root, "bundle-")?;
    copy_ready(
        &bundle,
        "analytics.duckdb",
        ready.database_handle(),
        ready.database(),
    )?;
    copy_ready(&bundle, "analytics.key", ready.key_handle(), ready.key())?;
    copy_ready(
        &bundle,
        "schema3-preimage.duckdb",
        ready.preimage_database_handle(),
        ready.preimage_database(),
    )?;
    copy_ready(
        &bundle,
        "schema3-preimage.key",
        ready.key_handle(),
        ready.preimage_key(),
    )?;
    copy_ready(
        &bundle,
        "migration.identity",
        ready.identity_record_handle(),
        ready.identity_record(),
    )?;
    copy_ready(
        &bundle,
        "migration.content.sha256",
        ready.content_seal_handle(),
        ready.content_seal(),
    )?;
    let record = MigrationRecord {
        schema: "dendro.selected-migration/v1".into(),
        version: 1,
        key_id: ready.key_id().into(),
        schema3_sha256: ready.preimage_database().sha256().into(),
        schema3_key_sha256: ready.preimage_key().sha256().into(),
        legacy_projection_sha256: ready.projection_sha256().into(),
        initial_schema4_sha256: ready.database().sha256().into(),
        identity_record_sha256: ready.identity_record().sha256().into(),
        content_seal_sha256: ready.content_seal().sha256().into(),
        legacy_anchor: legacy_anchor(binding)?,
    };
    write_record(&bundle, "migration.json", &record)?;
    let mut files = standard_records(&bundle, ready.key_id())?;
    for (role, leaf) in CUSTODY {
        add_file(&bundle, &mut files, role, leaf)?;
    }
    m::validate_selected_schema4(
        &bundle.path().join("analytics.duckdb"),
        &bundle.path().join("schema3-preimage.duckdb"),
        &bundle.path().join("schema3-preimage.key"),
    )?;
    ready.verify_unchanged()?;
    bundle.sync_dir()?;
    Ok((bundle, files))
}

fn schema(database: &Path) -> Result<i32> {
    let conn = duckdb::Connection::open_with_flags(
        database,
        duckdb::Config::default().access_mode(duckdb::AccessMode::ReadOnly)?,
    )?;
    Ok(
        conn.query_row("SELECT schema_version FROM dendro_meta", [], |row| {
            row.get(0)
        })?,
    )
}

/// The caller explicitly requests migration. The original schema3 anchor and
/// key are never rewritten; an independently closed schema4 bundle is selected.
pub fn migrate_logical_db(db: &Path, choice: &p::StorageRootChoice) -> Result<SelectedRead> {
    crate::pipeline::refuse_secret_path(db)?;
    match resolve_logical_db(db, choice)? {
        SelectedDb::Uninitialized => {
            return Err(anyhow!("migration requires an existing schema3 database"))
        }
        SelectedDb::Selected(selected) if schema(&selected.database)? == 4 => {
            validate_migration_custody(&selected.custody)?;
            return Ok(*selected);
        }
        SelectedDb::Selected(selected) if selected.custody.manifest.payload.files.len() == 11 => return Err(anyhow!("remigration_refused_retained_rollback_lineage: prior failed database and loss evidence remain selected; use a new logical database for another migration epoch")),
        _ => {}
    }
    let binding = admit_for_write(db, choice)?;
    let build = || -> Result<_> {
        p::with_exclusive_lock(
            &binding.root,
            &name(&binding.record.writer)?,
            &p::canonical_identity_from_persisted_record(binding.record.writer_identity.into()),
            |_| {
                Ok((|| -> Result<_> {
                    let current = reopen_binding(&binding)?;
                    let selected = if current.expected.is_some() {
                        Some(selected_from_binding(reopen_binding(&current)?)?)
                    } else {
                        None
                    };
                    let source = selected
                        .as_ref()
                        .map(|read| read.database.as_path())
                        .unwrap_or(current.logical.as_path());
                    let key = selected
                        .as_ref()
                        .map(|read| read.key.clone())
                        .unwrap_or_else(|| key_path(source));
                    if schema(source)? != 3 {
                        return Err(anyhow!("migration requires schema3"));
                    }
                    output::refuse_existing_output_wal(source)?;
                    let request = m::MigrateRequest {
                        source_database: source,
                        key_sidecar: Some(&key),
                        previous_private_root: None,
                        stop_at: None,
                        expected_source_sha256_hex: None,
                        damage_staged_usage_row: false,
                    };
                    let candidate = m::migrate_closed_snapshot_with_root_choice(request, choice)?;
                    let ready = m::validate_publication_candidate(source, &key, &candidate)?;
                    let (bundle, files) = bundle_from_ready(&current, &ready)?;
                    Ok((current, bundle, files))
                })())
            },
        )?
    };
    let (binding, bundle, files) =
        if binding.expected.is_none() && binding.record.anchor_kind == "legacy" {
            output::with_output_lock(&binding.root.path().join(".dendro-output.lock"), build)?
        } else {
            build()?
        };
    publish_candidate(&binding, &bundle, &files)
}

fn has_role(resolved: &p::ResolvedSelection, role: &str, leaf: &str) -> bool {
    resolved
        .manifest
        .payload
        .files
        .iter()
        .filter(|file| file.role == role && file.name == leaf)
        .count()
        == 1
        && resolved.file(leaf).is_some()
}

/// Called by the normal bundle resolver, including after a process restart.
pub(super) fn validate_migration_custody(resolved: &p::ResolvedSelection) -> Result<()> {
    let files = &resolved.manifest.payload.files;
    if files.len() == 4 {
        if schema(&resolved.bundle().path().join("analytics.duckdb"))? != 3 {
            return Err(anyhow!("schema4 requires durable migration custody"));
        }
        return Ok(());
    }
    if !matches!(files.len(), 9 | 11)
        || CUSTODY
            .iter()
            .any(|(role, leaf)| !has_role(resolved, role, leaf))
        || (files.len() == 11
            && LOSS
                .iter()
                .any(|(role, leaf)| !has_role(resolved, role, leaf)))
    {
        return Err(anyhow!("selected migration custody roles invalid"));
    }
    let record: MigrationRecord = read_pinned_record(resolved.file("migration.json").unwrap())?;
    if record.schema != "dendro.selected-migration/v1" || record.version != 1 {
        return Err(anyhow!("selected migration custody schema invalid"));
    }
    for (leaf, expected) in [
        ("schema3-preimage.duckdb", &record.schema3_sha256),
        ("schema3-preimage.key", &record.schema3_key_sha256),
        ("migration.identity", &record.identity_record_sha256),
        ("migration.content.sha256", &record.content_seal_sha256),
    ] {
        if files.iter().find(|file| file.name == leaf).unwrap().sha256 != digest(expected)? {
            return Err(anyhow!(
                "selected migration preimage/provenance digest differs"
            ));
        }
    }
    let bundle = resolved.bundle().path();
    let mut key_file = resolved.file("schema3-preimage.key").unwrap().try_clone()?;
    key_file.seek(SeekFrom::Start(0))?;
    let key = output::decode_key(&mut key_file)?;
    if key.id != record.key_id {
        return Err(anyhow!("selected migration preimage key differs"));
    }
    match schema(&bundle.join("analytics.duckdb"))? {
        4 if files.len() == 9 => {
            m::validate_selected_schema4_identity(
                &bundle.join("analytics.duckdb"),
                &record.key_id,
                &record.schema3_sha256,
                &record.schema3_key_sha256,
                &record.legacy_projection_sha256,
            )?;
            digest(&record.initial_schema4_sha256)?;
        }
        3 if files.len() == 11 => {
            let loss: serde_json::Value =
                read_pinned_record(resolved.file("rollback-loss.json").unwrap())?;
            if loss["selected_bundle"] != "preserved_schema3"
                || loss["destructive_downmigration"] != false
                || loss["failed_schema4_retained_private"] != true
            {
                return Err(anyhow!("selected rollback loss receipt invalid"));
            }
            if schema(&bundle.join("failed-schema4.duckdb"))? != 4 {
                return Err(anyhow!("rollback failed bundle is not schema4"));
            }
            if loss["failed_schema4_path"]
                != serde_json::to_value(bundle.join("failed-schema4.duckdb"))?
                || loss["preserved_schema3_path"]
                    != serde_json::to_value(bundle.join("schema3-preimage.duckdb"))?
            {
                return Err(anyhow!(
                    "selected rollback paths differ from retained custody"
                ));
            }
        }
        _ => {
            return Err(anyhow!(
                "selected migration schema/custody combination invalid"
            ))
        }
    }
    Ok(())
}

/// Preserve the migration lineage across ordinary later selected writes.
pub(super) fn carry_migration_custody(
    selected: &SelectedRead,
    bundle: &PrivateRoot,
    files: &mut Vec<p::ClosedBundleFile>,
) -> Result<()> {
    validate_migration_custody(&selected.custody)?;
    if selected.custody.manifest.payload.files.len() == 4 {
        return Ok(());
    }
    let extras = CUSTODY.into_iter().chain(
        (selected.custody.manifest.payload.files.len() == 11)
            .then_some(LOSS)
            .into_iter()
            .flatten(),
    );
    for (role, leaf) in extras {
        let record = selected
            .custody
            .manifest
            .payload
            .files
            .iter()
            .find(|file| file.role == role && file.name == leaf)
            .ok_or_else(|| anyhow!("missing migration custody file"))?;
        if leaf == "rollback-loss.json" {
            let mut loss: serde_json::Value =
                read_pinned_record(selected.custody.file(leaf).unwrap())?;
            loss["failed_schema4_path"] =
                serde_json::to_value(bundle.path().join("failed-schema4.duckdb"))?;
            loss["preserved_schema3_path"] =
                serde_json::to_value(bundle.path().join("schema3-preimage.duckdb"))?;
            write_record(bundle, leaf, &loss)?;
        } else {
            copy_held(
                bundle,
                leaf,
                selected.custody.file(leaf).unwrap(),
                record.sha256,
                record.size,
            )?;
        }
        add_file(bundle, files, role, leaf)?;
    }
    bundle.sync_dir()?;
    Ok(())
}

/// Validate the prospective complete bundle before native publication. This is
/// deliberately deeper than resolving an already hash-verified immutable bundle.
pub(super) fn validate_migration_candidate(
    bundle: &PrivateRoot,
    files: &[p::ClosedBundleFile],
) -> Result<()> {
    let version = schema(&bundle.path().join("analytics.duckdb"))?;
    if files.len() == 4 {
        if version != 3 {
            return Err(anyhow!(
                "schema4 candidate requires retained migration custody"
            ));
        }
        return Ok(());
    }
    if !matches!(files.len(), 9 | 11) {
        return Err(anyhow!("migration candidate custody file count invalid"));
    }
    for (role, leaf) in CUSTODY
        .into_iter()
        .chain((files.len() == 11).then_some(LOSS).into_iter().flatten())
    {
        if files
            .iter()
            .filter(|file| file.role == role && file.name.as_str() == leaf)
            .count()
            != 1
        {
            return Err(anyhow!("migration candidate custody roles invalid"));
        }
    }
    let record: MigrationRecord = read_record(bundle, "migration.json")?;
    if record.schema != "dendro.selected-migration/v1" || record.version != 1 {
        return Err(anyhow!("migration candidate custody schema invalid"));
    }
    for (leaf, hash) in [
        ("schema3-preimage.duckdb", &record.schema3_sha256),
        ("schema3-preimage.key", &record.schema3_key_sha256),
        ("migration.identity", &record.identity_record_sha256),
        ("migration.content.sha256", &record.content_seal_sha256),
    ] {
        if hash_file(&bundle.path().join(leaf))?.0 != digest(hash)? {
            return Err(anyhow!("migration candidate retained evidence differs"));
        }
    }
    if version == 4 && files.len() == 9 {
        let key = m::validate_selected_schema4(
            &bundle.path().join("analytics.duckdb"),
            &bundle.path().join("schema3-preimage.duckdb"),
            &bundle.path().join("schema3-preimage.key"),
        )?;
        if key != record.key_id {
            return Err(anyhow!("migration candidate key differs"));
        }
        m::validate_selected_schema4_identity(
            &bundle.path().join("analytics.duckdb"),
            &record.key_id,
            &record.schema3_sha256,
            &record.schema3_key_sha256,
            &record.legacy_projection_sha256,
        )?;
        digest(&record.initial_schema4_sha256)?;
    } else if version == 3 && files.len() == 11 {
        let loss: serde_json::Value = read_record(bundle, "rollback-loss.json")?;
        let logs = loss["replay_available"]
            .as_bool()
            .ok_or_else(|| anyhow!("rollback replay evidence missing"))?;
        let mut expected = m::rollback_to_preserved_schema3(
            &bundle.path().join("schema3-preimage.duckdb"),
            &bundle.path().join("failed-schema4.duckdb"),
            logs,
        )?;
        expected.failed_schema4_path = bundle.path().join("failed-schema4.duckdb");
        expected.preserved_schema3_path = bundle.path().join("schema3-preimage.duckdb");
        expected.publication_code = "native_selected_bundle_publication";
        expected.pointer_changed = true;
        if serde_json::to_value(expected)? != loss {
            return Err(anyhow!(
                "rollback candidate loss receipt differs from retained evidence"
            ));
        }
    } else {
        return Err(anyhow!(
            "migration candidate schema/custody combination invalid"
        ));
    }
    Ok(())
}

/// Roll back by selecting a copy of the retained schema3 pair. The failed
/// schema4 image and exact loss receipt remain in the newly selected bundle.
pub fn rollback_logical_db(
    db: &Path,
    choice: &p::StorageRootChoice,
    logs_present: bool,
) -> Result<(SelectedRead, m::LossReceipt)> {
    let initial = match resolve_logical_db(db, choice)? {
        SelectedDb::Selected(read) => *read,
        _ => {
            return Err(anyhow!(
                "rollback requires a selected migrated schema4 database"
            ))
        }
    };
    validate_migration_custody(&initial.custody)?;
    if schema(&initial.database)? != 4 {
        return Err(anyhow!("rollback requires schema4"));
    }
    let binding = reopen_binding(&initial.binding)?;
    let (current, bundle, files, mut loss) = p::with_exclusive_lock(
        &binding.root,
        &name(&binding.record.writer)?,
        &p::canonical_identity_from_persisted_record(binding.record.writer_identity.into()),
        |_| {
            Ok((|| -> Result<_> {
                let current = reopen_binding(&binding)?;
                if current.expected.map(|s| (s.identity, s.sha256))
                    != Some((initial.selection().identity, initial.selection().sha256))
                {
                    return Err(anyhow!("rollback source selection changed"));
                }
                let source = &initial.custody;
                let source_root = source.bundle().path();
                let mut loss = m::rollback_to_preserved_schema3(
                    &source_root.join("schema3-preimage.duckdb"),
                    &initial.database,
                    logs_present,
                )?;
                let bundle = p::create_unique_private_directory(&current.root, "bundle-")?;
                for (target, leaf) in [
                    ("analytics.duckdb", "schema3-preimage.duckdb"),
                    ("analytics.key", "schema3-preimage.key"),
                    ("failed-schema4.duckdb", "analytics.duckdb"),
                ] {
                    let record = source
                        .manifest
                        .payload
                        .files
                        .iter()
                        .find(|file| file.name == leaf)
                        .unwrap();
                    copy_held(
                        &bundle,
                        target,
                        source.file(leaf).unwrap(),
                        record.sha256,
                        record.size,
                    )?;
                }
                let key = selected_key(&initial)?;
                let mut files = standard_records(&bundle, &key.id)?;
                carry_migration_custody(&initial, &bundle, &mut files)?;
                loss.failed_schema4_path = bundle.path().join("failed-schema4.duckdb");
                loss.preserved_schema3_path = bundle.path().join("schema3-preimage.duckdb");
                // This record becomes authoritative only as a selected bundle file.
                loss.publication_code = "native_selected_bundle_publication";
                loss.pointer_changed = true;
                write_record(&bundle, "rollback-loss.json", &loss)?;
                for (role, leaf) in LOSS {
                    add_file(&bundle, &mut files, role, leaf)?;
                }
                bundle.sync_dir()?;
                Ok((current, bundle, files, loss))
            })())
        },
    )??;
    let selected = publish_candidate(&current, &bundle, &files)?;
    loss.pointer_changed = true;
    Ok((selected, loss))
}
