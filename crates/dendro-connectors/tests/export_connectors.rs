//! Synthetic fixtures derived from linked primary schemas, never owner transcripts.
use dendro_connectors::{
    sdk::ExportRoot, CursorConnector, DevinConnector, GooseConnector, WindsurfConnector,
};
use dendro_core::{AnalyticsStore, HarnessConnector};
use std::{
    fs,
    io::Write,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    dir: PathBuf,
    file: PathBuf,
}
impl Fixture {
    fn new(data: &[u8]) -> Self {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/connector-fixtures")
            .join(format!(
                "{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        fs::create_dir_all(&dir).unwrap();
        let dir = dir.canonicalize().unwrap();
        let file = dir.join("session.jsonl");
        fs::write(&file, data).unwrap();
        Self { dir, file }
    }
    fn append(&self, data: &[u8]) {
        fs::OpenOptions::new()
            .append(true)
            .open(&self.file)
            .unwrap()
            .write_all(data)
            .unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}
const KEY: &[u8] = b"synthetic-test-key";

fn read_export(
    name: &str,
    fixture: &Fixture,
    prior: Option<&dendro_core::Watermark>,
) -> anyhow::Result<dendro_connectors::exports::ExportBatch> {
    match name {
        "cursor" => CursorConnector::new(&fixture.dir, KEY).read_batch(&fixture.file, prior),
        "windsurf" => WindsurfConnector::new(&fixture.dir, KEY).read_batch(&fixture.file, prior),
        "goose" => GooseConnector::new(&fixture.dir, KEY).read_batch(&fixture.file, prior),
        "devin" => DevinConnector::new(&fixture.dir, KEY).read_batch(&fixture.file, prior),
        _ => unreachable!(),
    }
}

#[test]
fn every_export_reader_reports_versions_units_aliases_malformed_and_truncation() {
    for (name, bytes) in [
        (
            "cursor",
            include_bytes!("fixtures/exports/cursor.jsonl").as_slice(),
        ),
        (
            "windsurf",
            include_bytes!("fixtures/exports/windsurf.jsonl").as_slice(),
        ),
        (
            "goose",
            include_bytes!("fixtures/exports/goose.jsonl").as_slice(),
        ),
        (
            "devin",
            include_bytes!("fixtures/exports/devin.jsonl").as_slice(),
        ),
    ] {
        let fixture = Fixture::new(bytes);
        let valid = read_export(name, &fixture, None).unwrap();
        assert!(valid.complete, "{name} fixture must be complete");
        for (field, value) in [
            ("version", "99"),
            ("units", "bytes"),
            ("sessionId", "alias"),
        ] {
            let first = bytes.split(|byte| *byte == b'\n').next().unwrap();
            let mut record: serde_json::Value = serde_json::from_slice(first).unwrap();
            record[field] = value.into();
            let mut body = serde_json::to_vec(&record).unwrap();
            body.push(b'\n');
            fs::write(&fixture.file, body).unwrap();
            assert!(read_export(name, &fixture, None).is_err(), "{name} {field}");
        }
        for (container, field) in [
            ("usage", "units"),
            ("metrics", "currency"),
            ("token_usage", "inputTokens"),
        ] {
            let first = bytes.split(|byte| *byte == b'\n').next().unwrap();
            let mut record: serde_json::Value = serde_json::from_slice(first).unwrap();
            record[container] = serde_json::json!({field: "unknown"});
            let mut body = serde_json::to_vec(&record).unwrap();
            body.push(b'\n');
            fs::write(&fixture.file, body).unwrap();
            assert!(
                read_export(name, &fixture, None).is_err(),
                "{name} nested {field}"
            );
        }
        if name == "devin" {
            let mut record: serde_json::Value =
                serde_json::from_slice(bytes.split(|byte| *byte == b'\n').next().unwrap()).unwrap();
            record["steps"][0]["source"] = "agent".into();
            record["steps"][0]["metrics"] =
                serde_json::json!({"prompt_tokens": 1, "completion_tokens": 2, "unit": "seconds"});
            let mut body = serde_json::to_vec(&record).unwrap();
            body.push(b'\n');
            fs::write(&fixture.file, body).unwrap();
            assert!(read_export(name, &fixture, None).is_err());
        }
        fs::write(&fixture.file, b"not-json\n").unwrap();
        match read_export(name, &fixture, None) {
            Ok(batch) => {
                assert!(!batch.complete, "{name} malformed");
                assert_eq!(batch.coverage.malformed, 1, "{name}");
            }
            Err(_) => assert_eq!(name, "goose", "only Goose requires an offset-zero header"),
        }
        fs::write(&fixture.file, bytes).unwrap();
        fixture.append(b"{\"held\":");
        let held = read_export(name, &fixture, None).unwrap();
        assert!(!held.complete, "{name} held tail");
        assert_eq!(held.coverage.held_tail, 1);
        assert!(held.coverage.summary().contains("held_tail=1"));
        fs::write(&fixture.file, b"{").unwrap();
        assert!(
            read_export(name, &fixture, Some(&valid.watermark)).is_err(),
            "{name} truncation"
        );
    }
}

#[test]
fn native_unknown_kinds_count_each_record_and_persist_partial_coverage() {
    let fixture = Fixture::new(b"{\"type\":\"future_kind\"}\n{\"type\":\"future_kind\"}\n");
    let store = AnalyticsStore::open_in_memory("synthetic").unwrap();
    let stats = dendro_connectors::stream::ingest_jsonl_with_key(
        &store,
        "native-unknown",
        "claude",
        &fixture.file,
        Some(KEY),
    )
    .unwrap();
    assert_eq!(stats.coverage.unknown_kind, 2);
    assert_eq!(stats.coverage.projected, 0);
    assert!(stats.coverage.partial);
    let resumed = dendro_connectors::stream::ingest_jsonl_with_key(
        &store,
        "native-unknown",
        "claude",
        &fixture.file,
        Some(KEY),
    )
    .unwrap();
    assert_eq!(resumed.records_read, 0);
    assert_eq!(resumed.coverage.unknown_kind, 0);
    assert!(resumed.coverage.partial);
    assert!(!store.load_source_coverage().unwrap()[0].complete);
    assert!(store
        .has_warning(
            &fixture.file.display().to_string(),
            &stats.coverage.summary()
        )
        .unwrap());
}

#[test]
fn staged_ingest_continues_after_refused_export_and_failed_source() {
    use dendro_connectors::{ingest_staged_with_manifest, SourceRoot};
    let fixture = Fixture::new(include_bytes!("fixtures/exports/devin.jsonl"));
    fs::write(
        fixture.dir.join("bad.jsonl"),
        b"{\"schema_version\":\"ATIF-v99\"}\n",
    )
    .unwrap();
    let db_root = dendro_core::platform_v1::create_user_private_root_with_choice(
        "dendro-coverage-fixture-",
        &dendro_core::platform_v1::StorageRootChoice::UserProfile,
    )
    .unwrap();
    let db = db_root.path().join("analytics.duckdb");
    drop(AnalyticsStore::open_path(&db, &dendro_core::key_id(KEY)).unwrap());
    let roots = [
        SourceRoot {
            name: "cursor".into(),
            path: fixture.file.clone(),
        },
        SourceRoot {
            name: "devin".into(),
            path: fixture.dir.clone(),
        },
    ];
    let (notes, manifest) = ingest_staged_with_manifest(&db, &roots, false, KEY, None).unwrap();
    assert!(notes
        .iter()
        .any(|note| note.contains("cursor coverage=refused reason=export_discovery_failed")));
    assert!(!notes.iter().any(|note| note.contains("cursor files=0")));
    assert!(notes
        .iter()
        .any(|note| note.contains("coverage=refused reason=unsupported_version")));
    assert!(notes
        .iter()
        .any(|note| note.contains("devin files=1 refused=1") && note.contains("coverage=partial")));
    assert_eq!(manifest.files.len(), 1);
    let store = AnalyticsStore::open_existing(&db, &dendro_core::key_id(KEY)).unwrap();
    assert_eq!(store.load_usage().unwrap().len(), 1);
    assert_eq!(store.load_tools().unwrap().len(), 1);
}
#[test]
fn native_tool_only_projection_and_tail_completion_are_truthful() {
    let fixture = Fixture::new(b"{\"type\":\"assistant\",\"sessionId\":\"s\",\"message\":{\"content\":[{\"type\":\"tool_use\",\"id\":\"c\",\"name\":\"Read\",\"input\":{}}]}}\n");
    let store = AnalyticsStore::open_in_memory("synthetic").unwrap();
    let read = || {
        dendro_connectors::stream::ingest_jsonl_with_key(
            &store,
            "tool-only",
            "claude",
            &fixture.file,
            Some(KEY),
        )
        .unwrap()
    };
    let first = read();
    assert_eq!(first.coverage.projected, 1);
    assert_eq!(first.coverage.not_projected, 0);
    assert_eq!(store.load_tools().unwrap().len(), 1);
    assert!(first.coverage.partial);
    assert!(store
        .has_warning(&fixture.file.display().to_string(), "missing-usage")
        .unwrap());

    let tail_fixture = Fixture::new(b"{\"type\":\"system\",\"sessionId\":\"s\"}\n");
    let tail_store = AnalyticsStore::open_in_memory("synthetic").unwrap();
    let read_tail = || {
        dendro_connectors::stream::ingest_jsonl_with_key(
            &tail_store,
            "tail-only",
            "claude",
            &tail_fixture.file,
            Some(KEY),
        )
        .unwrap()
    };
    read_tail();
    tail_fixture.append(b"{\"type\":\"system\",\"sessionId\":\"s\"}");
    let held = read_tail();
    assert!(held.coverage.partial);
    assert_eq!(held.coverage.held_tail, 1);
    assert!(!tail_store.load_source_coverage().unwrap()[0].complete);
    tail_fixture.append(b"\n");
    let completed = read_tail();
    assert!(!completed.coverage.partial);
    assert_eq!(completed.coverage.held_tail, 0);
    assert!(tail_store.load_source_coverage().unwrap()[0].complete);
}

const CURSOR_TOOL: &str = "{\"session_id\":\"s\",\"type\":\"tool_call\",\"subtype\":\"completed\",\"call_id\":\"c\",\"tool_call\":{\"readToolCall\":{}}}\n";

#[test]
fn export_defects_persist_on_resume_and_tail_only_partial_resolves() {
    let fixture = Fixture::new(
        format!("{CURSOR_TOOL}not-json\n{{\"session_id\":\"s\",\"type\":\"future\"}}\n").as_bytes(),
    );
    let store = AnalyticsStore::open_in_memory("synthetic").unwrap();
    let connector = CursorConnector::new(&fixture.dir, KEY);
    connector.ingest_file(&store, &fixture.file).unwrap();
    connector.ingest_file(&store, &fixture.file).unwrap();
    let source = &store.load_sources().unwrap()[0].0;
    assert!(store.has_warning(source, "coverage-partial").unwrap());
    let expected = dendro_connectors::sdk::Coverage {
        partial: true,
        ..Default::default()
    };
    assert!(store.has_warning(source, &expected.summary()).unwrap());
    assert_eq!(store.load_tools().unwrap().len(), 1);

    let tail =
        Fixture::new(format!("{CURSOR_TOOL}{{\"session_id\":\"s\",\"type\":\"user\"}}").as_bytes());
    let store = AnalyticsStore::open_in_memory("synthetic").unwrap();
    let connector = CursorConnector::new(&tail.dir, KEY);
    let first = connector.read_batch(&tail.file, None).unwrap();
    connector.ingest_file(&store, &tail.file).unwrap();
    let source = &store.load_sources().unwrap()[0].0;
    assert!(store
        .has_warning(source, &first.coverage.summary())
        .unwrap());
    assert!(!store.has_warning(source, "coverage-partial").unwrap());
    tail.append(b"\n");
    let completed = connector
        .read_batch(&tail.file, Some(&first.watermark))
        .unwrap();
    assert!(completed.complete);
    connector.ingest_file(&store, &tail.file).unwrap();
    assert!(store
        .has_warning(source, &completed.coverage.summary())
        .unwrap());
}

#[test]
fn export_same_batch_errors_keep_good_records_and_later_refusal_keeps_counts() {
    let mixed = Fixture::new(format!("{CURSOR_TOOL}{{\"version\":99}}\n{CURSOR_TOOL}").as_bytes());
    let batch = CursorConnector::new(&mixed.dir, KEY)
        .read_batch(&mixed.file, None)
        .unwrap();
    assert!(!batch.complete);
    assert_eq!(batch.tools.len(), 2);
    assert_eq!(batch.coverage.projected, 2);
    assert_eq!(batch.coverage.unsupported_version, 1);

    let body = format!(
        "{}not-json\n{{\"session_id\":\"s\",\"type\":\"future\"}}\n{{}}\n",
        CURSOR_TOOL.repeat(128)
    );
    let fixture = Fixture::new(body.as_bytes());
    let store = AnalyticsStore::open_in_memory("synthetic").unwrap();
    let error = CursorConnector::new(&fixture.dir, KEY)
        .ingest_file(&store, &fixture.file)
        .unwrap_err()
        .to_string();
    for expected in [
        "projected=128",
        "unknown_kind=1",
        "malformed=1",
        "unsupported_schema=1",
        "coverage=partial",
    ] {
        assert!(error.contains(expected), "{error}");
    }
    assert_eq!(store.load_tools().unwrap().len(), 128);
    let source = &store.load_sources().unwrap()[0].0;
    assert!(store.has_warning(source, "malformed-json-record").unwrap());
    assert!(store
        .has_warning(source, "unsupported-cursor-record")
        .unwrap());
}

#[test]
fn mixed_goose_record_counts_its_projection_and_preserves_partial_on_resume() {
    let fixture = Fixture::new(b"{\"working_dir\":\"/synthetic\",\"message_count\":1,\"input_tokens\":0,\"output_tokens\":0}\n{\"role\":\"assistant\",\"content\":[{\"type\":\"toolRequest\",\"id\":\"c\",\"toolCall\":{\"status\":\"success\",\"value\":{\"name\":\"developer__shell\"}}},{\"type\":\"future\"}]}\n");
    let connector = GooseConnector::new(&fixture.dir, KEY);
    let batch = connector.read_batch(&fixture.file, None).unwrap();
    assert!(!batch.complete);
    assert_eq!(batch.coverage.projected, 2);
    assert_eq!(batch.coverage.unknown_kind, 0);
    assert_eq!(batch.tools.len(), 1);
    let store = AnalyticsStore::open_in_memory("synthetic").unwrap();
    connector.ingest_file(&store, &fixture.file).unwrap();
    connector.ingest_file(&store, &fixture.file).unwrap();
    let source = &store.load_sources().unwrap()[0].0;
    assert!(store.has_warning(source, "coverage-partial").unwrap());
}

#[test]
fn discovery_retains_each_refused_export_candidate() {
    let fixture = Fixture::new(b"{}\n");
    let store = AnalyticsStore::open_in_memory("synthetic").unwrap();
    let error = CursorConnector::new(&fixture.dir, KEY)
        .ingest_file(&store, &fixture.dir.join("missing.jsonl"))
        .unwrap_err()
        .to_string();
    assert!(error.contains("coverage=partial"));
    assert!(!error.contains("coverage=complete"));
    fs::create_dir(fixture.dir.join("a.jsonl")).unwrap();
    fs::create_dir(fixture.dir.join("b.jsonl")).unwrap();
    let listing = ExportRoot::new(&fixture.dir).inspect_discovery().unwrap();
    assert_eq!(
        listing.paths.as_slice(),
        std::slice::from_ref(&fixture.file)
    );
    assert_eq!(listing.refused.len(), 2);
    assert!(listing
        .refused
        .iter()
        .all(|(_, reason)| *reason == "export must be a regular file"));
}

#[test]
fn partial_malformed_and_appended_lines_preserve_exact_offsets() {
    let fixture = Fixture::new(b"{}\nnot-json\n{\"tail\":");
    let reader = ExportRoot::new(&fixture.dir);
    let first = reader.read_batch(&fixture.file, None).unwrap();
    assert_eq!(first.records.len(), 1);
    assert_eq!(first.watermark.byte_offset, 12);
    assert_eq!(first.warnings, ["malformed-json-record"]);
    let second = reader
        .read_batch(&fixture.file, Some(&first.watermark))
        .unwrap();
    assert!(second.records.is_empty());
    assert_eq!(second.watermark.byte_offset, 12);
    fixture.append(b"1}\n");
    let third = reader
        .read_batch(&fixture.file, Some(&second.watermark))
        .unwrap();
    assert_eq!(third.records.len(), 1);
    assert_eq!(third.records[0].1["tail"], 1);
    assert_eq!(
        third.watermark.byte_offset,
        fs::metadata(&fixture.file).unwrap().len()
    );
}
#[test]
fn batch_and_line_memory_are_bounded() {
    let fixture = Fixture::new("{}\n".repeat(300).as_bytes());
    let root = ExportRoot::new(&fixture.dir);
    let batch = root.read_batch(&fixture.file, None).unwrap();
    assert_eq!(batch.records.len(), 128);
    assert_eq!(batch.watermark.byte_offset, 384);
    fs::write(&fixture.file, vec![b'x'; 1024 * 1024 + 1]).unwrap();
    assert!(root
        .read_batch(&fixture.file, None)
        .unwrap_err_string()
        .contains("1 MiB"));
}
// Keep errors opaque while avoiding Debug on a batch containing raw source values.
trait ErrorString {
    fn unwrap_err_string(self) -> String;
}
impl<T> ErrorString for anyhow::Result<T> {
    fn unwrap_err_string(self) -> String {
        match self {
            Ok(_) => panic!("expected refusal"),
            Err(e) => e.to_string(),
        }
    }
}
#[test]
fn truncation_prefix_mutation_and_wrong_source_refuse_resume() {
    let a = Fixture::new(b"{}\n");
    let b = Fixture::new(b"{}\n");
    let root = ExportRoot::new(&a.dir);
    let batch = root.read_batch(&a.file, None).unwrap();
    assert!(ExportRoot::new(&b.dir)
        .read_batch(&b.file, Some(&batch.watermark))
        .is_err());
    fs::write(&a.file, b"[]\n").unwrap();
    assert!(root.read_batch(&a.file, Some(&batch.watermark)).is_err());
    fs::write(&a.file, b"{").unwrap();
    assert!(root.read_batch(&a.file, Some(&batch.watermark)).is_err());
}
#[cfg(unix)]
#[test]
fn symlink_and_escape_are_refused_without_following() {
    use std::os::unix::fs::symlink;
    let a = Fixture::new(b"{}\n");
    let b = Fixture::new(b"{}\n");
    let root = ExportRoot::new(&a.dir);
    let linked = a.dir.join("linked.jsonl");
    symlink(&b.file, &linked).unwrap();
    symlink(&a.dir, a.dir.join("cycle")).unwrap();
    assert_eq!(
        root.discover().unwrap().as_slice(),
        std::slice::from_ref(&a.file)
    );
    assert!(root.read_batch(&linked, None).is_err());
    assert!(root.read_batch(&b.file, None).is_err());
    assert!(root
        .read_batch(&a.dir.join("../session.jsonl"), None)
        .is_err());
}
#[test]
fn cursor_completed_tools_have_unknown_tokens_and_time_and_no_secrets() {
    let fixture = Fixture::new(include_bytes!("fixtures/exports/cursor.jsonl"));
    let connector = CursorConnector::new(&fixture.dir, KEY);
    assert_eq!(connector.name(), "cursor");
    assert_eq!(connector.discover_files().unwrap().len(), 1);
    let batch = connector.read_batch(&fixture.file, None).unwrap();
    assert!(batch.usage.is_empty());
    assert_eq!(batch.tools.len(), 1);
    assert_eq!(batch.tools[0].name, "read_file");
    assert_eq!(batch.tools[0].failed, Some(false));
    assert!(batch.tools[0].ts.is_none());
    assert!(connector.extract_incremental(&fixture.file, None).is_err());
    let store = AnalyticsStore::open_in_memory("synthetic").unwrap();
    let warnings = connector.ingest_file(&store, &fixture.file).unwrap();
    assert!(warnings.contains(&"token-usage-no-data"));
    connector.ingest_file(&store, &fixture.file).unwrap();
    assert_eq!(store.load_tools().unwrap().len(), 1);
    assert!(store.load_usage().unwrap().is_empty());
    let facts = serde_json::to_string(&store.load_tools().unwrap()).unwrap();
    for secret in [
        "sk-SYNTHETIC",
        "PRIVATE_TRANSCRIPT",
        "private.txt",
        "synthetic-cursor",
    ] {
        assert!(!facts.contains(secret));
    }
}
#[test]
fn cascade_import_does_not_follow_transcript_path_or_invent_tokens() {
    let fixture = Fixture::new(include_bytes!("fixtures/exports/windsurf.jsonl"));
    let connector = WindsurfConnector::new(&fixture.dir, KEY);
    let batch = connector.read_batch(&fixture.file, None).unwrap();
    assert_eq!(batch.tools.len(), 2);
    assert!(batch.usage.is_empty());
    assert_eq!(batch.tools[0].name, "write_file");
    assert_eq!(batch.tools[1].name, "shell");
    assert_eq!(
        batch.tools[0].failed,
        Some(false),
        "transcript status done is explicit"
    );
    assert_eq!(batch.tools[1].failed, None, "post hook carries no outcome");
    let serialized = serde_json::to_string(&batch.tools).unwrap();
    assert!(!serialized.contains("PRIVATE_TRANSCRIPT"));
    let unknown = Fixture::new(b"{\"type\":\"new_step\",\"status\":\"done\"}\n");
    let unknown_batch = WindsurfConnector::new(&unknown.dir, KEY)
        .read_batch(&unknown.file, None)
        .unwrap();
    assert!(unknown_batch.warnings.contains(&"unsupported-cascade-step"));
}
#[test]
fn goose_legacy_totals_count_once_and_appended_tools_are_incremental() {
    let fixture = Fixture::new(include_bytes!("fixtures/exports/goose.jsonl"));
    let connector = GooseConnector::new(&fixture.dir, KEY);
    let store = AnalyticsStore::open_in_memory("synthetic").unwrap();
    connector.ingest_file(&store, &fixture.file).unwrap();
    connector.ingest_file(&store, &fixture.file).unwrap();
    let usage = store.load_usage().unwrap();
    assert_eq!(usage.len(), 1);
    assert_eq!(usage[0].input_uncached, 30);
    assert_eq!(usage[0].output, 6);
    assert_eq!(usage[0].currency_total, None);
    let tools = store.load_tools().unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "shell");
    assert_eq!(tools[0].failed, None);
    fixture.append(b"{\"id\":\"next\",\"role\":\"assistant\",\"created\":1767225603,\"content\":[{\"type\":\"toolRequest\",\"id\":\"c2\",\"toolCall\":{\"status\":\"success\",\"value\":{\"name\":\"developer__shell\"}}}]}\n");
    connector.ingest_file(&store, &fixture.file).unwrap();
    assert_eq!(store.load_tools().unwrap().len(), 2);
    assert_eq!(store.load_usage().unwrap().len(), 1);
}
#[test]
fn devin_atif_version_boundary_and_observed_metrics_are_explicit() {
    let fixture = Fixture::new(include_bytes!("fixtures/exports/devin.jsonl"));
    let connector = DevinConnector::new(&fixture.dir, KEY);
    let batch = connector.read_batch(&fixture.file, None).unwrap();
    assert_eq!(batch.usage.len(), 1);
    assert_eq!(batch.usage[0].input_uncached, 25);
    assert_eq!(batch.usage[0].output, 5);
    assert_eq!(batch.usage[0].currency_total, Some(0.002));
    assert_eq!(batch.tools.len(), 1);
    assert_eq!(batch.tools[0].name, "shell");
    assert_eq!(
        connector
            .extract_incremental(&fixture.file, None)
            .unwrap()
            .0
            .len(),
        2
    );
    let store = AnalyticsStore::open_in_memory("synthetic").unwrap();
    connector.ingest_file(&store, &fixture.file).unwrap();
    connector.ingest_file(&store, &fixture.file).unwrap();
    assert_eq!(store.load_usage().unwrap().len(), 1);
    let unknown = Fixture::new(b"{\"schema_version\":\"ATIF-v99\"}\n");
    let error = DevinConnector::new(&unknown.dir, KEY)
        .read_batch(&unknown.file, None)
        .unwrap_err_string();
    assert!(error.contains("unsupported Devin export"));
}
#[test]
fn unknown_formats_negative_counters_and_missing_optional_roots_are_safe() {
    let fixture = Fixture::new(b"{\"working_dir\":\"/demo\",\"message_count\":0,\"input_tokens\":-1,\"output_tokens\":1}\n");
    assert!(GooseConnector::new(&fixture.dir, KEY)
        .read_batch(&fixture.file, None)
        .is_err());
    let missing = fixture.dir.join("missing");
    assert!(CursorConnector::new(&missing, KEY)
        .discover_files()
        .unwrap()
        .is_empty());
    assert!(WindsurfConnector::new(&missing, KEY)
        .discover_files()
        .unwrap()
        .is_empty());
    assert!(GooseConnector::new(&missing, KEY)
        .discover_files()
        .unwrap()
        .is_empty());
    assert!(DevinConnector::new(&missing, KEY)
        .discover_files()
        .unwrap()
        .is_empty());
    assert!(CursorConnector::new(&fixture.dir, KEY)
        .read_batch(&fixture.file, None)
        .is_err());
    assert!(CursorConnector::new(&fixture.dir, Vec::new())
        .read_batch(&fixture.file, None)
        .is_err());
}

#[test]
fn staged_ingest_routes_all_export_roots_and_sealed_replay_excludes_append() {
    use dendro_connectors::{ingest_staged_with_manifest, SourceRoot};
    let fixture = Fixture::new(include_bytes!("fixtures/exports/devin.jsonl"));
    // The staged database must live under a private root; the export tree may not.
    let db_root = dendro_core::platform_v1::create_user_private_root_with_choice(
        "dendro-export-fixture-",
        &dendro_core::platform_v1::StorageRootChoice::UserProfile,
    )
    .expect("private staged ingest root");
    let db = db_root.path().join("analytics.duckdb");
    drop(AnalyticsStore::open_path(&db, &dendro_core::key_id(KEY)).unwrap());
    let roots = [SourceRoot {
        name: "devin".into(),
        path: fixture.dir.clone(),
    }];
    let (notes, cutoff) = ingest_staged_with_manifest(&db, &roots, false, KEY, None).unwrap();
    assert!(notes
        .iter()
        .any(|n| n.contains("format=verified-export-only")));
    assert_eq!(cutoff.files.len(), 1);
    assert_eq!(
        cutoff.files[0].byte_length,
        fs::metadata(&fixture.file).unwrap().len()
    );
    fixture.append(b"not-json\n");
    ingest_staged_with_manifest(&db, &roots, false, KEY, Some(&cutoff)).unwrap();
    let store = AnalyticsStore::open_existing(&db, &dendro_core::key_id(KEY)).unwrap();
    assert_eq!(store.load_usage().unwrap().len(), 1);
    assert_eq!(store.load_tools().unwrap().len(), 1);
    drop(store);
    // A same-length modification of the accepted prefix must fail sealed replay.
    let mut changed = fs::read(&fixture.file).unwrap();
    changed[10] = b'x';
    fs::write(&fixture.file, changed).unwrap();
    assert!(ingest_staged_with_manifest(&db, &roots, false, KEY, Some(&cutoff)).is_err());
    for (name, bytes) in [
        (
            "cursor",
            include_bytes!("fixtures/exports/cursor.jsonl").as_slice(),
        ),
        (
            "windsurf",
            include_bytes!("fixtures/exports/windsurf.jsonl").as_slice(),
        ),
        (
            "goose",
            include_bytes!("fixtures/exports/goose.jsonl").as_slice(),
        ),
    ] {
        let f = Fixture::new(bytes);
        let db = db_root.path().join(format!("{name}.duckdb"));
        drop(AnalyticsStore::open_path(&db, &dendro_core::key_id(KEY)).unwrap());
        let roots = [SourceRoot {
            name: name.into(),
            path: f.dir.clone(),
        }];
        let (_, manifest) = ingest_staged_with_manifest(&db, &roots, false, KEY, None).unwrap();
        assert_eq!(manifest.files.len(), 1);
        let store = AnalyticsStore::open_existing(&db, &dendro_core::key_id(KEY)).unwrap();
        assert!(!store.load_tools().unwrap().is_empty());
    }
}

#[test]
fn malformed_first_goose_header_does_not_accept_a_headerless_stream() {
    let f =
        Fixture::new(b"bad-json\n{\"role\":\"assistant\",\"created\":1767225601,\"content\":[]}\n");
    assert!(GooseConnector::new(&f.dir, KEY)
        .read_batch(&f.file, None)
        .is_err());
}

#[test]
fn atif_missing_usage_is_unknown_and_copied_context_is_refused() {
    let f = Fixture::new(b"{\"schema_version\":\"ATIF-v1.7\",\"session_id\":\"demo\",\"steps\":[{\"step_id\":1,\"source\":\"agent\"}]}\n");
    let c = DevinConnector::new(&f.dir, KEY);
    let batch = c.read_batch(&f.file, None).unwrap();
    assert!(batch.usage.is_empty());
    assert!(batch.warnings.contains(&"token-usage-no-data"));
    fs::write(&f.file, b"{\"schema_version\":\"ATIF-v1.7\",\"session_id\":\"demo\",\"steps\":[{\"step_id\":1,\"source\":\"agent\",\"is_copied_context\":true}]}\n").unwrap();
    assert!(c.read_batch(&f.file, None).is_err());
}

#[test]
fn atif_nonzero_cached_prompt_semantics_are_refused() {
    let f = Fixture::new(b"{\"schema_version\":\"ATIF-v1.7\",\"session_id\":\"demo\",\"steps\":[{\"step_id\":1,\"source\":\"agent\",\"metrics\":{\"prompt_tokens\":30,\"completion_tokens\":5,\"cached_tokens\":10}}]}\n");
    let error = DevinConnector::new(&f.dir, KEY)
        .read_batch(&f.file, None)
        .unwrap_err_string();
    assert!(error.contains("nonzero cached_tokens unsupported"));
}

#[cfg(unix)]
#[test]
fn direct_socket_export_is_rejected_before_opening_a_special_file() {
    use std::os::unix::net::UnixListener;
    let f = Fixture::new(b"{}\n");
    let socket = f.dir.join("special.jsonl");
    let _listener = UnixListener::bind(&socket).unwrap();
    let error = ExportRoot::new(&f.dir)
        .read_batch(&socket, None)
        .unwrap_err_string();
    assert!(error.contains("regular file"));
}
