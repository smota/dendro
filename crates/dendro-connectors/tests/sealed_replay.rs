//! The file side of F13: replay the original bytes into a fresh analytics DB.
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use dendro_connectors::{ingest_staged_with_manifest, SourceRoot};
use dendro_core::{report_json, run_locked_with_manifest, CutoffManifest};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const FROM: &str = "2026-10-09T00:00:00Z";
const TO: &str = "2026-10-10T00:00:00Z";
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/issue-13-tests")
            .join(format!(
                "files-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn line(harness: &str, id: &str, large: bool) -> String {
    let [input, output, read, write] = if large { [30, 7, 9, 11] } else { [10, 2, 3, 4] };
    let value = match harness {
        "claude" => {
            json!({"type":"assistant", "sessionId":"session-c", "timestamp":"2026-10-09T01:00:00Z",
            "message":{"id":id,"usage":{"input_tokens":input, "output_tokens":output,
                "cache_read_input_tokens":read,"cache_creation_input_tokens":write},"content":[]}})
        }
        "codex" => json!({"type":"event_msg", "timestamp":"2026-10-09T01:00:00Z",
            "payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":input,
                "output_tokens":output,"cached_input_tokens":read,"cache_write_input_tokens":write,
                "reasoning_output_tokens":0}}}}),
        "pi" => json!({"type":"message","id":id,"timestamp":"2026-10-09T01:00:00Z",
            "message":{"role":"assistant","usage":{"input":input,"output":output,"cacheRead":read,"cacheWrite":write}}}),
        _ => unreachable!(),
    };
    format!("{value}\n")
}

fn header(harness: &str) -> String {
    match harness {
        "codex" => format!(
            "{}\n",
            json!({"type":"session_meta","payload":{"id":"session-x"},"timestamp":FROM})
        ),
        "pi" => format!(
            "{}\n",
            json!({"type":"session","version":3,"id":"session-p","timestamp":FROM})
        ),
        _ => String::new(),
    }
}

fn roots(dir: &Path) -> Vec<SourceRoot> {
    ["claude", "codex", "pi"]
        .into_iter()
        .map(|name| {
            let path = dir.join(name);
            fs::create_dir_all(&path).unwrap();
            fs::write(
                path.join("first.jsonl"),
                format!("{}{}", header(name), line(name, "first", false)),
            )
            .unwrap();
            SourceRoot {
                name: name.into(),
                path,
            }
        })
        .collect()
}

fn run(
    db: &Path,
    roots: &[SourceRoot],
    write: Option<&Path>,
    sealed: Option<&Path>,
) -> anyhow::Result<Value> {
    let paths: Vec<_> = roots.iter().map(|root| root.path.clone()).collect();
    run_locked_with_manifest(db, &paths, &[], write, sealed, |stage, key, manifest| {
        ingest_staged_with_manifest(stage, roots, false, key, manifest)
    })
}

fn payload(db: &Path) -> Value {
    report_json(db, "tokens", None, FROM, TO).unwrap()["payload"].clone()
}

fn key_copy(from: &Path, to: &Path) {
    fs::copy(
        format!("{}.key", from.display()),
        format!("{}.key", to.display()),
    )
    .unwrap();
}

#[test]
fn sealed_files_ignore_appends_partial_lines_and_new_files_but_unsealed_grows_exactly() {
    let scratch = Scratch::new();
    let roots = roots(&scratch.0);
    let original = scratch.0.join("original.duckdb");
    let manifest_path = scratch.0.join("cutoff.json");
    run(&original, &roots, Some(&manifest_path), None).unwrap();
    let baseline = payload(&original);
    assert_eq!(baseline["tokens"], 50);
    assert_eq!(baseline["usage_rows"], 3);
    let manifest: CutoffManifest =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    assert_eq!(manifest.files.len(), 3);
    for root in &roots {
        let file = root.path.join("first.jsonl");
        let bytes = fs::read(&file).unwrap();
        let cutoff = manifest
            .files
            .iter()
            .find(|entry| entry.harness == root.name)
            .unwrap();
        assert_eq!(cutoff.byte_length, bytes.len() as u64);
        assert_eq!(cutoff.prefix_sha256, hex::encode(Sha256::digest(&bytes)));
        let mut append = OpenOptions::new().append(true).open(&file).unwrap();
        append
            .write_all(line(&root.name, "second", true).as_bytes())
            .unwrap();
        append.write_all(b"{\"type\":\"unfinished").unwrap();
        fs::write(
            root.path.join("new.jsonl"),
            format!("{}{}", header(&root.name), line(&root.name, "new", false)),
        )
        .unwrap();
    }
    let unchanged_sources: Vec<_> = roots
        .iter()
        .flat_map(|root| {
            [root.path.join("first.jsonl"), root.path.join("new.jsonl")]
                .into_iter()
                .map(|path| {
                    let bytes = fs::read(&path).unwrap();
                    let mtime = fs::metadata(&path).unwrap().modified().unwrap();
                    (path, bytes, mtime)
                })
        })
        .collect();
    let replay = scratch.0.join("replay.duckdb");
    key_copy(&original, &replay);
    run(&replay, &roots, None, Some(&manifest_path)).unwrap();
    let replayed = payload(&replay);
    for field in [
        "tokens",
        "usage_rows",
        "token_categories",
        "unlinked_tokens",
        "unlinked_token_categories",
        "m1_yield",
        "m2_return_rate",
        "m3_chains",
        "m3_rework_chains",
        "mission_tokens",
    ] {
        assert_eq!(replayed[field], baseline[field], "sealed metric {field}");
    }
    run(&original, &roots, None, None).unwrap();
    let grown = payload(&original);
    // Claude and Pi each add 57 tokens; Codex adds a cumulative delta of 25.
    // Codex input includes both cache categories, so its original total is 12.
    // Each new file adds the same 19/12/19 categories as its original file.
    assert_eq!(grown["tokens"], 239);
    assert_eq!(grown["usage_rows"], 9);
    for (field, value) in [
        ("input_uncached", 113),
        ("output", 31),
        ("cache_read", 42),
        ("cache_write", 53),
    ] {
        assert_eq!(
            grown["token_categories"][field], value,
            "unsealed category {field}"
        );
    }
    for (path, bytes, mtime) in unchanged_sources {
        assert_eq!(fs::read(&path).unwrap(), bytes, "ingest wrote to source");
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), mtime);
    }
}

#[test]
fn sealed_file_mismatches_leave_published_database_bytes_untouched() {
    for mutation in ["truncated", "rewritten", "missing"] {
        let scratch = Scratch::new();
        let roots = roots(&scratch.0);
        let db = scratch.0.join("analytics.duckdb");
        let manifest_path = scratch.0.join("cutoff.json");
        run(&db, &roots, Some(&manifest_path), None).unwrap();
        let baseline = fs::read(&db).unwrap();
        let manifest: CutoffManifest =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        let id = &manifest
            .files
            .iter()
            .find(|entry| entry.harness == "claude")
            .unwrap()
            .source_id;
        let source = roots[0].path.join("first.jsonl");
        match mutation {
            "truncated" => {
                OpenOptions::new()
                    .write(true)
                    .open(&source)
                    .unwrap()
                    .set_len(1)
                    .unwrap();
            }
            "rewritten" => {
                let mut bytes = fs::read(&source).unwrap();
                let offset = bytes.windows(4).position(|part| part == b"10,\"").unwrap();
                bytes[offset] = b'2';
                fs::write(&source, bytes).unwrap();
            }
            "missing" => fs::remove_file(&source).unwrap(),
            _ => unreachable!(),
        }
        let error = run(&db, &roots, None, Some(&manifest_path))
            .unwrap_err()
            .to_string();
        assert!(error.contains("mismatch"), "{mutation}: {error}");
        assert!(error.contains(id), "{mutation}: {error}");
        assert!(
            error.len() > "mismatch".len() + id.len(),
            "reason missing: {error}"
        );
        assert_eq!(
            fs::read(&db).unwrap(),
            baseline,
            "published DB changed after {mutation}"
        );
    }
}

#[test]
fn cutoff_is_the_last_complete_line_and_manifest_contains_no_content_or_paths() {
    let scratch = Scratch::new();
    let root = scratch.0.join("claude");
    let directory = root.join("ghp_manifestDirectoryCanary1234567890");
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("sk-manifestFileCanary1234567890.jsonl");
    let complete = line("claude", "sk-messageCanary1234567890", false);
    fs::write(
        &source,
        format!("{complete}{{\"private_prompt\":\"raw-content-canary\"}}"),
    )
    .unwrap();
    let roots = vec![SourceRoot {
        name: "claude".into(),
        path: root,
    }];
    let db = scratch.0.join("analytics.duckdb");
    let manifest_path = scratch.0.join("cutoff.json");
    run(&db, &roots, Some(&manifest_path), None).unwrap();
    let encoded = fs::read_to_string(&manifest_path).unwrap();
    let manifest: CutoffManifest = serde_json::from_str(&encoded).unwrap();
    assert_eq!(manifest.files.len(), 1);
    assert_eq!(manifest.files[0].byte_length, complete.len() as u64);
    assert_eq!(
        manifest.files[0].prefix_sha256,
        hex::encode(Sha256::digest(complete.as_bytes()))
    );
    for forbidden in [
        "raw-content-canary",
        "private_prompt",
        "manifestDirectoryCanary",
        "manifestFileCanary",
        "messageCanary",
        "input_tokens",
        "session-c",
    ] {
        assert!(
            !encoded.contains(forbidden),
            "manifest contains {forbidden}"
        );
    }
    assert!(!encoded.contains(&directory.to_string_lossy().to_string()));
    let entry = serde_json::to_value(&manifest.files[0]).unwrap();
    let fields: Vec<_> = entry
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        fields,
        ["byte_length", "harness", "prefix_sha256", "source_id"]
    );
}

#[test]
fn sealed_replay_keeps_history_present_when_manifest_was_written() {
    let scratch = Scratch::new();
    let root = scratch.0.join("claude");
    fs::create_dir_all(&root).unwrap();
    let source = root.join("session.jsonl");
    fs::write(&source, line("claude", "first-generation", false)).unwrap();
    let roots = vec![SourceRoot {
        name: "claude".into(),
        path: root,
    }];
    let db = scratch.0.join("analytics.duckdb");
    let manifest = scratch.0.join("cutoff.json");
    run(&db, &roots, None, None).unwrap();
    // A producer replaces a file; R1 preserves the earlier generation's tokens.
    fs::write(&source, line("claude", "second-generation", true)).unwrap();
    run(&db, &roots, Some(&manifest), None).unwrap();
    let baseline = payload(&db);
    assert_eq!(baseline["tokens"], 76);
    run(&db, &roots, None, Some(&manifest)).unwrap();
    let replayed = payload(&db);
    assert_eq!(replayed["tokens"], baseline["tokens"]);
    assert_eq!(replayed["usage_rows"], baseline["usage_rows"]);
    let manifest_json: Value = serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    let digest = manifest_json["analytics_snapshot_sha256"].as_str().unwrap();
    assert_eq!(digest.len(), 64);
    let companion = manifest
        .parent()
        .unwrap()
        .join(format!(".dendro-sealed-{digest}.duckdb"));
    let historical = fs::read(&companion).unwrap();
    assert_eq!(hex::encode(Sha256::digest(&historical)), digest);
    let published = fs::read(&db).unwrap();
    for missing in [false, true] {
        if missing {
            fs::remove_file(&companion).unwrap();
        } else {
            let mut altered = historical.clone();
            altered.push(1);
            fs::write(&companion, altered).unwrap();
        }
        let error = run(&db, &roots, None, Some(&manifest))
            .unwrap_err()
            .to_string();
        assert!(error.contains("mismatch"), "{error}");
        assert!(error.contains("history"), "{error}");
        assert_eq!(fs::read(&db).unwrap(), published);
        fs::write(&companion, &historical).unwrap();
    }
}
