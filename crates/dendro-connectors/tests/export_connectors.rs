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
    let db = fixture.dir.join("analytics.duckdb");
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
        let db = f.dir.join("analytics.duckdb");
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
