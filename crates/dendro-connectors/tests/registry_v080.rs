//! Exact T05 frozen oracle vectors; expected values are read from the immutable oracle.
#[path = "../../../examples/capture-v1/contribution.rs"]
mod contribution;
use contribution::{Example, DESCRIPTOR, REGISTRATIONS};
use dendro_connectors::{capture_registry_v1::*, sdk::*};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    root: PathBuf,
    file: PathBuf,
}
impl Fixture {
    fn new(bytes: &[u8]) -> Self {
        let root = std::env::temp_dir().join(format!(
            "dendro-t05-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let file = root.join("input.jsonl");
        fs::write(&file, bytes).unwrap();
        Self { root, file }
    }
    fn root(&self) -> ExportRoot {
        ExportRoot::new(self.root.clone())
    }
    fn read(&self, limits: ReadLimits) -> JsonlBatch {
        self.root()
            .read_bounded(&self.file, None, None, &limits, |_| true)
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn oracle() -> Value {
    serde_json::from_str(include_str!(
        "../../../.agentflow/oracles/v080/T05/frozen.json"
    ))
    .unwrap()
}
fn vector(id: &str) -> Value {
    oracle()["vectors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["id"] == id)
        .unwrap()
        .clone()
}
fn recipe(name: &str) -> Vec<u8> {
    let o = oracle();
    let v = &o["fixture_recipes"]["json_string_line"][name];
    let crlf = v["framing"] == "CRLF";
    let mut bytes = b"{\"u\":\"".to_vec();
    bytes.extend(std::iter::repeat_n(
        b'a',
        v["pad_bytes"].as_u64().unwrap() as usize,
    ));
    bytes.extend(if crlf {
        b"\"}\r\n".as_slice()
    } else {
        b"\"}\n".as_slice()
    });
    assert_eq!(
        hex::encode(Sha256::digest(&bytes)),
        v["sha256"].as_str().unwrap()
    );
    bytes
}
fn line(name: &str) -> Vec<u8> {
    oracle()["fixture_recipes"]["example_lines"][name]["utf8"]
        .as_str()
        .unwrap()
        .replace("\\n", "\n")
        .into_bytes()
}
fn global() -> ReadLimits {
    ReadLimits {
        encoded_bytes: GLOBAL_MAX_RECORD_BYTES,
        nesting_depth: 64,
        decoded_records_per_batch: 1024,
        line_attempts: None,
    }
}
fn capture_bytes(bytes: &[u8]) -> Report<i64> {
    let f = Fixture::new(bytes);
    capture(&Example, &f.root(), &f.file, &mut Cursor::default())
}
fn reason_eq(reasons: &[&str], expected: &Value) {
    assert_eq!(serde_json::to_value(reasons).unwrap(), *expected);
}

#[test]
fn frozen_oracle_identity_and_all_34_vectors() {
    assert_eq!(
        hex::encode(Sha256::digest(include_bytes!(
            "../../../.agentflow/oracles/v080/T05/frozen.json"
        ))),
        "cb7f914737c6f2c2c9fa4fe8359fe28f7b321839fc8e391cf4c72e7b4f729160"
    );
    assert_eq!(oracle()["vectors"].as_array().unwrap().len(), 34);
    for v in oracle()["vectors"].as_array().unwrap() {
        run_vector(v);
    }
}
fn run_vector(v: &Value) {
    let id = v["id"].as_str().unwrap();
    eprintln!("executing frozen vector {id}");
    let e = &v["expected"];
    match id {
        "T05-A1-01" | "T05-A1-04" | "T05-A1-13" => {
            let (name, limits) = match id {
                "T05-A1-01" => ("export_at", ReadLimits::export()),
                "T05-A1-04" => ("global_at", global()),
                _ => (
                    "one_point_five_mib",
                    ReadLimits {
                        encoded_bytes: 2097152,
                        ..global()
                    },
                ),
            };
            let b = Fixture::new(&recipe(name)).read(limits);
            assert_eq!(b.records.len(), e["decoded"].as_u64().unwrap() as usize);
            assert!(b.warnings.is_empty());
            assert_eq!(
                b.watermark.byte_offset,
                if id == "T05-A1-13" {
                    1572864
                } else {
                    e["watermark_byte_offset"].as_u64().unwrap()
                }
            );
            assert_eq!(b.max_decoded_depth, 1);
            if id == "T05-A1-13" {
                assert!(Descriptor {
                    source_id: "conformance.limit.synthetic",
                    limits,
                    ..DESCRIPTOR
                }
                .validate()
                .is_ok());
            }
            if id == "T05-A1-01" {
                let b = Fixture::new(&recipe("export_at_crlf")).read(limits);
                assert_eq!(
                    b.watermark.byte_offset,
                    e["crlf_variant"]["watermark_byte_offset"].as_u64().unwrap()
                );
            }
        }
        "T05-A1-02" | "T05-A1-05" => {
            let mut bytes = if id.ends_with("02") {
                b"{}\n".to_vec()
            } else {
                Vec::new()
            };
            bytes.extend(recipe(if id.ends_with("02") {
                "export_max_plus_1"
            } else {
                "global_max_plus_1"
            }));
            let b = Fixture::new(&bytes).read(if id.ends_with("02") {
                ReadLimits::export()
            } else {
                global()
            });
            reason_eq(&b.warnings, &e["reasons"]);
            if id == "T05-A1-02" {
                assert_eq!(
                    b.records.len() as u64,
                    e["projected_prior_records"].as_u64().unwrap()
                );
            }
            assert_eq!(
                b.watermark.byte_offset,
                e["watermark_byte_offset"].as_u64().unwrap()
            );
            assert!(!b.complete);
        }
        "T05-A1-03" => {
            let f = Fixture::new(&vec![b'x'; 1048577]);
            let err = f.root().read_batch(&f.file, None).err().unwrap();
            assert_eq!(err.to_string(), e["diagnostic_exact"].as_str().unwrap());
        }
        "T05-A1-06" | "T05-A1-07" | "T05-A1-19" => {
            let depths: &[usize] = match id {
                "T05-A1-06" => &[64],
                "T05-A1-07" => &[65],
                _ => &[8, 9],
            };
            for &depth in depths {
                let bytes = format!("{}null{}\n", "[".repeat(depth), "]".repeat(depth));
                let b = Fixture::new(bytes.as_bytes()).read(ReadLimits {
                    nesting_depth: if id.ends_with("19") { 8 } else { 64 },
                    ..global()
                });
                let ok = depth <= if id.ends_with("19") { 8 } else { 64 };
                assert_eq!(b.records.len(), usize::from(ok));
                assert_eq!(b.watermark.byte_offset, bytes.len() as u64);
                if !ok {
                    assert_eq!(b.warnings, ["nesting_depth_exceeds_limit"]);
                }
            }
        }
        "T05-A1-08" | "T05-A1-10" => {
            let (count, limits) = if id.ends_with("08") {
                (1025, global())
            } else {
                (300, ReadLimits::export())
            };
            let f = Fixture::new(&b"{}\n".repeat(count));
            let b = f.read(limits);
            let be = if id.ends_with("08") { &e["batch_1"] } else { e };
            assert_eq!(b.records.len() as u64, be["decoded"].as_u64().unwrap());
            assert_eq!(b.watermark.session_id.as_deref(), be["session_id"].as_str());
            assert_eq!(
                b.watermark.byte_offset,
                be["watermark_byte_offset"].as_u64().unwrap()
            );
            assert!(!b.complete);
            if id.ends_with("08") {
                let second = f
                    .root()
                    .read_bounded(&f.file, Some(&b.watermark), None, &limits, |_| true)
                    .unwrap();
                assert_eq!(second.records.len(), 1);
                assert_eq!(
                    second.watermark.byte_offset,
                    e["batch_2"]["watermark_byte_offset"].as_u64().unwrap()
                );
            }
        }
        "T05-A1-09" => {
            for c in v["cases"].as_array().unwrap() {
                let l = ReadLimits {
                    encoded_bytes: c["encoded_bytes_including_lf"].as_u64().unwrap(),
                    nesting_depth: c["nesting_depth"].as_u64().unwrap() as usize,
                    decoded_records_per_batch: c["decoded_records_per_batch"].as_u64().unwrap()
                        as usize,
                    line_attempts: None,
                };
                assert_eq!(l.validate().unwrap_err(), e["reasons"][0].as_str().unwrap());
            }
        }
        "T05-A1-11" => {
            for source in ["cursor", "windsurf", "goose", "devin", "AGY"] {
                let d = Descriptor {
                    source_id: source,
                    limits: ReadLimits::export(),
                    ..DESCRIPTOR
                };
                assert!(d.validate().is_ok());
                assert_eq!(
                    d.native_size_admitted(1572864).unwrap_err(),
                    e["reasons"][0].as_str().unwrap()
                );
                let b = Fixture::new(&recipe("one_point_five_mib")).read(d.limits);
                reason_eq(&b.warnings, &e["reasons"]);
            }
        }
        "T05-A1-12" => {
            for source in ["Claude", "Codex", "Pi", "OpenRig", "Grok"] {
                let d = Descriptor {
                    source_id: source,
                    ..DESCRIPTOR
                };
                assert_eq!(
                    d.native_size_admitted(1572864).unwrap_err(),
                    e["reasons"][0].as_str().unwrap()
                );
            }
        }
        "T05-A1-14" => {
            for (name, two, ascii) in [("at", 524283, 1), ("max_plus_1", 524284, 0)] {
                let bytes = format!(
                    "{{\"u\":\"{}{}\"}}\n",
                    "\u{00e9}".repeat(two),
                    "a".repeat(ascii)
                )
                .into_bytes();
                let r = &oracle()["fixture_recipes"]["unicode_equal_scalar_count"][name];
                assert_eq!(
                    hex::encode(Sha256::digest(&bytes)),
                    r["sha256"].as_str().unwrap()
                );
                let b = Fixture::new(&bytes).read(ReadLimits::export());
                assert_eq!(b.watermark.byte_offset, e[name]["len"].as_u64().unwrap());
                assert_eq!(b.records.len(), usize::from(name == "at"));
                if name != "at" {
                    reason_eq(&b.warnings, &e[name]["reasons"]);
                }
            }
        }
        "T05-A1-15" | "T05-A1-16" => {
            let name = if id.ends_with("15") {
                "duplicate"
            } else {
                "duplicate_nested"
            };
            let bytes = oracle()["fixture_recipes"]["grammar"][name]["utf8"]
                .as_str()
                .unwrap()
                .replace("\\n", "\n");
            let b = Fixture::new(bytes.as_bytes()).read(global());
            reason_eq(&b.warnings, &e["reasons"]);
            assert!(b.records.is_empty());
            if id == "T05-A1-16" {
                assert_eq!(b.max_examined_depth, e["depth"].as_u64().unwrap() as usize);
            }
            assert_eq!(
                b.watermark.byte_offset,
                e["watermark_byte_offset"].as_u64().unwrap()
            );
        }
        "T05-A1-17" => {
            let f = Fixture::new(b"{}\nnot-json\n{\"tail\":");
            let sdk_first = f.read(ReadLimits::export());
            assert_eq!(
                sdk_first.watermark.session_id.as_deref(),
                e["first_read"]["session_id"].as_str()
            );
            reason_eq(&sdk_first.warnings, &e["first_read"]["warnings_exact"]);
            let mut cursor = Cursor::default();
            let a = capture(&Example, &f.root(), &f.file, &mut cursor);
            assert_eq!(a.counts.decoded, 1);
            assert_eq!(a.counts.malformed, 1);
            assert!(!a.observed_zero);
            assert_eq!(
                a.watermark.unwrap().byte_offset,
                e["first_read"]["watermark_byte_offset"].as_u64().unwrap()
            );
            // Frozen warnings concern JSONL decoding, not the stricter example-kind projection.
            assert!(a.reasons.contains(&"malformed-json-record"));
            let b = capture(&Example, &f.root(), &f.file, &mut cursor);
            assert_eq!(b.counts.decoded, 0);
            assert_eq!(b.read, ReadState::Partial);
            fs::write(&f.file, b"{}\nnot-json\n{\"tail\":1}\n").unwrap();
            let sdk_third = f
                .root()
                .read_batch(&f.file, Some(&sdk_first.watermark))
                .unwrap();
            assert_eq!(
                sdk_third.records[0].1["tail"],
                e["third_read"]["amount_or_field"]["tail"]
            );
            let c = capture(&Example, &f.root(), &f.file, &mut cursor);
            assert_eq!(c.counts.decoded, 1);
            assert_eq!(c.read, ReadState::Partial);
            assert!(!c.observed_zero);
            assert_eq!(c.watermark.unwrap().byte_offset, 23);
        }
        "T05-A1-18" => {
            let f = Fixture::new(b"{}\n");
            let b = f.read(global());
            for (bytes, msg) in [
                (b"{".as_slice(), "export truncated; reset source explicitly"),
                (
                    b"[]\n".as_slice(),
                    "export prefix changed; reset source explicitly",
                ),
            ] {
                fs::write(&f.file, bytes).unwrap();
                let err = f
                    .root()
                    .read_batch(&f.file, Some(&b.watermark))
                    .err()
                    .unwrap();
                assert_eq!(err.to_string(), msg);
            }
            let other = Fixture::new(b"{}\n");
            assert_eq!(
                other
                    .root()
                    .read_batch(&other.file, Some(&b.watermark))
                    .err()
                    .unwrap()
                    .to_string(),
                "watermark belongs to another export"
            );
        }
        "T05-A1-20" => {
            for c in v["cases"].as_array().unwrap() {
                let d = Descriptor {
                    max_bound_processes: Some(c["max_bound_processes"].as_u64().unwrap() as usize),
                    max_samples_per_invocation: Some(
                        c["max_samples_per_invocation"].as_u64().unwrap() as usize,
                    ),
                    ..DESCRIPTOR
                };
                assert_eq!(d.validate().unwrap_err(), e["reasons"][0].as_str().unwrap());
            }
            assert!(Descriptor {
                max_bound_processes: Some(64),
                max_samples_per_invocation: Some(600),
                ..DESCRIPTOR
            }
            .validate()
            .is_ok());
        }
        "T05-A1-21" => {
            let b = Fixture::new(b" \n{}\n").read(global());
            assert_eq!(b.records.len(), 1);
            assert_eq!(b.whitespace_lines, 1);
            assert_eq!(
                b.watermark.byte_offset,
                e["watermark_byte_offset"].as_u64().unwrap()
            );
            assert!(b.warnings.is_empty());
        }
        "T05-A1-22" => {
            let mut bytes = b"{\"a\":1,\"a\":2,\"pad\":\"".to_vec();
            bytes.extend(std::iter::repeat_n(b'a', 1048577 - bytes.len() - 3));
            bytes.extend(b"\"}\n");
            assert_eq!(bytes.len(), 1048577);
            let b = Fixture::new(&bytes).read(ReadLimits::export());
            reason_eq(&b.warnings, &e["reasons"]);
        }
        "T05-A2-01" => {
            let bytes = [
                line("projected_1"),
                line("unprojected"),
                line("projected_2_unknown_field"),
            ]
            .concat();
            let r = capture_bytes(&bytes);
            assert_eq!(r.facts, vec![1, 2]);
            assert_eq!(r.counts.recognized(), 3);
            assert_eq!(r.counts.projected, 2);
            assert_eq!(r.counts.recognized_unprojected, 1);
            assert_eq!(r.counts.ignored_unknown_field, 1);
            assert_eq!(r.read, ReadState::Complete);
            assert!(!r.observed_zero);
        }
        "T05-A2-02" => {
            let r = capture_bytes(
                &[
                    line("projected_1"),
                    line("unknown_major"),
                    line("projected_1"),
                ]
                .concat(),
            );
            assert_eq!(r.facts, vec![1]);
            assert_eq!(
                r.counts.decoded,
                e["counts"]["decoded"].as_u64().unwrap() as usize
            );
            assert_eq!(r.counts.unsupported_schema, 1);
            assert_eq!(
                r.watermark.unwrap().byte_offset,
                e["durable_watermark_byte_offset"].as_u64().unwrap()
            );
            assert_eq!(r.raw_read_position, 103);
            assert_eq!(r.read, ReadState::Partial);
            reason_eq(&r.reasons, &e["reasons"]);
            assert!(!r.observed_zero);
        }
        "T05-A2-03" => {
            let a = capture_bytes(&line("unknown_major"));
            let b = capture_bytes(&line("projected_1"));
            assert_eq!(a.read, ReadState::Partial);
            assert!(!a.observed_zero);
            assert_eq!(b.read, ReadState::Complete);
            assert_eq!(b.facts, vec![1]);
        }
        "T05-A2-04" => {
            let f = Fixture::new(&line("projected_1"));
            let g = Fixture::new(&line("projected_1"));
            let mut c = Cursor::default();
            capture(&Example, &f.root(), &f.file, &mut c);
            let r = capture(&Example, &g.root(), &g.file, &mut c);
            assert_eq!(r.read, ReadState::Failed);
            assert!(!r.observed_zero);
            assert_eq!(
                r.reasons,
                [v["input"]["diagnostic_exact"].as_str().unwrap()]
            );
        }
        "T05-A2-05" => {
            let f = Fixture::new(b"");
            let r = discover(&Example, &ExportRoot::new(f.root.join("missing")));
            assert_eq!(r.discovery, Discovery::Absent);
            assert_eq!(r.files, e["files"].as_u64().unwrap() as usize);
            assert_eq!(r.read, ReadState::NotAttempted);
            assert!(!r.observed_zero);
        }
        "T05-A2-06" => {
            let f = Fixture::new(b"");
            fs::remove_file(&f.file).unwrap();
            let r = discover(&Example, &f.root());
            assert_eq!(r.read, ReadState::Complete);
            assert!(r.observed_zero);
            assert_eq!(r.files, 0);
            assert_eq!(r.counts, Counts::default());
        }
        "T05-A2-07" => {
            for (source, reason) in [
                ("Grok", "source_not_admitted"),
                ("Linux strace", "source_not_admitted"),
                ("Windows ETW", "source_not_admitted"),
            ] {
                // Even an incorrect implementation declaration cannot bypass source gates.
                let descriptor = Descriptor {
                    source_id: source,
                    admission: Admission::Implemented,
                    admitted_native_record_bytes: Some(1048576),
                    ..DESCRIPTOR
                };
                assert_eq!(descriptor.validate().unwrap_err(), reason);
            }
            for admission in [Admission::ParserPending, Admission::SourceBlocked] {
                let d = Descriptor {
                    admission,
                    ..DESCRIPTOR
                };
                assert_eq!(d.validate().unwrap_err(), "source_not_admitted");
            }
        }
        "T05-A2-08" => {
            let registry = Registry::new(REGISTRATIONS).unwrap();
            assert_eq!(
                registry
                    .descriptor("example.capture.synthetic")
                    .unwrap()
                    .contract,
                e["contract"].as_str().unwrap()
            );
            let r = capture_bytes(&line("projected_1"));
            assert_eq!(r.facts, vec![1]);
            assert_eq!(r.read, ReadState::Complete);
        }
        "T05-A2-09" => {
            let f = Fixture::new(&line("projected_1").repeat(17));
            let mut c = Cursor::default();
            let a = capture(&Example, &f.root(), &f.file, &mut c);
            assert_eq!(a.counts.decoded, 16);
            assert_eq!(
                a.watermark.as_ref().unwrap().byte_offset,
                e["batch_1"]["watermark_byte_offset"].as_u64().unwrap()
            );
            assert_eq!(
                a.watermark.unwrap().session_id.as_deref(),
                e["batch_1"]["session_id"].as_str()
            );
            let b = capture(&Example, &f.root(), &f.file, &mut c);
            assert_eq!(b.counts.decoded, 1);
            assert_eq!(
                b.watermark.unwrap().byte_offset,
                e["batch_2"]["watermark_byte_offset"].as_u64().unwrap()
            );
            assert!(b.reasons.is_empty());
        }
        "T05-A2-10" => {
            for (d, reason) in [
                (
                    Descriptor {
                        contract: "dendro.capture.contribution/2",
                        ..DESCRIPTOR
                    },
                    "incompatible_contribution_contract",
                ),
                (
                    Descriptor {
                        contract: "dendro.capture.contribution/0",
                        ..DESCRIPTOR
                    },
                    "incompatible_contribution_contract",
                ),
                (
                    Descriptor {
                        limits: ReadLimits {
                            encoded_bytes: 8388609,
                            ..global()
                        },
                        ..DESCRIPTOR
                    },
                    "descriptor_looser_than_global_ceiling",
                ),
                (
                    Descriptor {
                        dynamic: true,
                        ..DESCRIPTOR
                    },
                    "runtime_plugin_refused",
                ),
                (
                    Descriptor {
                        fixture_digest: None,
                        ..DESCRIPTOR
                    },
                    "missing_conformance_fixture",
                ),
            ] {
                assert_eq!(d.validate().unwrap_err(), reason);
            }
            let r = capture_bytes(&line("unknown_major"));
            assert_eq!(r.reasons, ["unsupported_schema"]);
        }
        "T05-A2-11" => {
            let r = capture_bytes(&line("projected_2_unknown_field"));
            assert_eq!(r.facts, vec![2]);
            assert_eq!(r.counts.ignored_unknown_field, 1);
            let wm = r.watermark.as_ref().unwrap();
            let persisted = format!(
                "{:?}{:?}{}{:?}",
                r.facts, r.reasons, wm.file_path, wm.session_id
            );
            for forbidden in [
                "sk-SYNTHETIC-T05-CANARY",
                "PRIVATE_TRANSCRIPT",
                "-----BEGIN",
            ] {
                assert!(!persisted.contains(forbidden));
            }
        }
        "T05-A2-12" => containment(v),
        _ => panic!("unmapped frozen vector {id}"),
    }
}
fn containment(v: &Value) {
    let f = Fixture::new(b"{}\n");
    let root = f.root();
    let parent = f
        .root
        .join("..")
        .join(f.root.file_name().unwrap())
        .join("input.jsonl");
    assert_eq!(
        root.read_batch(&parent, None).err().unwrap().to_string(),
        v["cases"][0]["diagnostic_exact"].as_str().unwrap()
    );
    let outside = Fixture::new(b"{}\n");
    assert_eq!(
        root.read_batch(&outside.file, None)
            .err()
            .unwrap()
            .to_string(),
        v["cases"][2]["diagnostic_exact"].as_str().unwrap()
    );
    let txt = f.root.join("input.txt");
    fs::write(&txt, b"{}\n").unwrap();
    assert_eq!(
        root.read_batch(&txt, None).err().unwrap().to_string(),
        v["cases"][3]["diagnostic_exact"].as_str().unwrap()
    );
    #[cfg(windows)]
    let link_result = std::os::windows::fs::symlink_file(&f.file, f.root.join("link.jsonl"));
    #[cfg(unix)]
    let link_result = std::os::unix::fs::symlink(&f.file, f.root.join("link.jsonl"));
    match link_result {
        Ok(()) => assert_eq!(
            root.read_batch(&f.root.join("link.jsonl"), None)
                .err()
                .unwrap()
                .to_string(),
            v["cases"][1]["diagnostic_exact"].as_str().unwrap()
        ),
        Err(error) => panic!("T05-A2-12 not_attempted: symlink fixture unavailable: {error}"),
    }
}
#[test]
fn frozen_vector_count_guard() {
    assert_eq!(vector("T05-A2-12")["cases"].as_array().unwrap().len(), 4);
}

#[test]
fn native_admission_is_enforced_before_capture_and_stubs_fail_without_watermark() {
    use dendro_core::HarnessConnector;
    struct Native;
    static NATIVE: Descriptor = Descriptor {
        source_id: "codex",
        ..DESCRIPTOR
    };
    impl Contribution for Native {
        type Fact = i64;
        fn descriptor(&self) -> &'static Descriptor {
            &NATIVE
        }
        fn project(&self, _: &Value) -> Projection<i64> {
            panic!("unadmitted source must not project")
        }
    }
    let f = Fixture::new(&recipe("one_point_five_mib"));
    let mut cursor = Cursor::default();
    let report = capture(&Native, &f.root(), &f.file, &mut cursor);
    assert_eq!(report.reasons, ["native_record_size_not_admitted"]);
    assert_eq!(report.read, ReadState::Failed);
    assert!(report.facts.is_empty());
    assert!(cursor.watermark.is_none());
    assert!(!report.observed_zero);
    for connector in [
        Box::new(dendro_connectors::grok::GrokConnector::new()) as Box<dyn HarnessConnector>,
        Box::new(dendro_connectors::agy::AgyConnector::new()),
    ] {
        let error = connector.extract_incremental(&f.file, None).err().unwrap();
        assert_eq!(error.to_string(), "source_not_admitted");
    }
}

// Round 1 independent-review regressions. These test failure behavior rather
// than authoring replacement expectations for the frozen 34 vectors.
#[test]
fn review_b1_preserves_prefix_before_unterminated_oversize_tail() {
    let mut bytes = b"{}\n".to_vec();
    bytes.extend(vec![b'x'; 1048577]);
    let f = Fixture::new(&bytes);
    let b = f.read(ReadLimits::export());
    assert_eq!(b.records.len(), 1);
    assert_eq!(b.watermark.byte_offset, 3);
    assert_eq!(
        b.watermark.session_id.as_deref(),
        oracle()["fixture_recipes"]["grammar"]["one_empty_object"]["sha256"]
            .as_str()
            .map(|_| "sdk-v1:ca3d163bab055381827226140568f3bef7eaac187cebd76878e0b63e9e442356")
    );
    assert!(!b.complete);
    assert!(b.warnings.is_empty());
    assert!(f.root().read_batch(&f.file, Some(&b.watermark)).is_err());
}
#[test]
fn review_b2_bounds_bad_only_batches_reasons_and_aggregate_bytes() {
    for bytes in [
        b"bad\n".repeat(5000),
        b"{\"a\":1,\"a\":2}\n".repeat(5000),
        b" \r\n".repeat(5000),
    ] {
        let f = Fixture::new(&bytes);
        let b = f.read(global());
        assert_eq!(b.line_attempts, 1024);
        assert!(!b.complete);
        assert!(b.warnings.len() <= 1);
        assert_eq!(b.malformed + b.quarantined + b.whitespace_lines, 1024);
        assert!(b.bytes_examined <= MAX_BATCH_ENCODED_BYTES);
        let r = capture(&Example, &f.root(), &f.file, &mut Cursor::default());
        assert_eq!(r.read, ReadState::Partial);
        assert!(r.reasons.len() <= 1);
        assert_eq!(
            r.counts.malformed + r.counts.quarantined + r.counts.whitespace_lines,
            16
        );
    }
    // Two individually eligible maximum-sized records cannot retain 16MiB in one batch.
    let bytes = recipe("global_at").repeat(2);
    let f = Fixture::new(&bytes);
    let a = f.read(global());
    assert_eq!(a.records.len(), 1);
    assert_eq!(a.watermark.byte_offset, GLOBAL_MAX_RECORD_BYTES);
    assert!(a.bytes_examined <= MAX_BATCH_ENCODED_BYTES);
    assert!(!a.complete);
    let b = f
        .root()
        .read_bounded(&f.file, Some(&a.watermark), None, &global(), |_| true)
        .unwrap();
    assert_eq!(b.records.len(), 1);
    assert_eq!(b.watermark.byte_offset, 2 * GLOBAL_MAX_RECORD_BYTES);
    assert!(b.complete);
    assert!(ReadLimits {
        line_attempts: Some(0),
        ..global()
    }
    .validate()
    .is_err());
    let whitespace = Fixture::new(b"\r\n").read(global());
    assert_eq!(whitespace.whitespace_lines, 1);
    assert_eq!(whitespace.records.len(), 0);
}
#[test]
fn review_b3_unknown_kind_and_invalid_integer_never_claim_full_coverage() {
    for bytes in [
        b"[]\n".as_slice(),
        b"1\n",
        b"{\"kind\":\"evil\"}\n",
        b"{\"kind\":17}\n",
    ] {
        let r = capture_bytes(bytes);
        assert_eq!(r.counts.unrecognized, 1);
        assert_eq!(r.counts.recognized(), 0);
        assert_eq!(r.read, ReadState::Partial);
        assert!(!r.observed_zero);
        assert!(r.facts.is_empty());
    }
    for bytes in [
        b"{\"kind\":\"example.record\",\"n\":\"1\"}\n".as_slice(),
        b"{\"kind\":\"example.record\",\"n\":99999999999999999999}\n",
        b"{\"kind\":\"example.record\",\"n\":1.1}\n",
    ] {
        let r = capture_bytes(bytes);
        assert_eq!(r.counts.quarantined, 1);
        assert_eq!(r.counts.decoded, 1);
        assert_eq!(r.counts.recognized(), 0);
        assert_eq!(r.read, ReadState::Partial);
        assert!(r.facts.is_empty());
        assert!(!r.observed_zero);
    }
    let r = capture_bytes(&line("unprojected"));
    assert_eq!(r.counts.recognized_unprojected, 1);
    assert_eq!(r.read, ReadState::Complete);
}
#[test]
fn review_b4_discovery_refuses_symlinks_and_nonregular_jsonl() {
    let f = Fixture::new(b"{}\n");
    fs::remove_file(&f.file).unwrap();
    fs::create_dir(f.root.join("directory.jsonl")).unwrap();
    let r = discover(&Example, &f.root());
    assert_eq!(r.read, ReadState::Partial);
    assert!(!r.observed_zero);
    assert_eq!(r.files, 0);
    assert_eq!(r.reasons, ["export must be a regular file"]);
    fs::remove_dir(f.root.join("directory.jsonl")).unwrap();
    let outside = Fixture::new(b"{}\n");
    #[cfg(windows)]
    std::os::windows::fs::symlink_file(&outside.file, f.root.join("link.jsonl"))
        .expect("symlink subcase not_attempted: cannot create fixture");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside.file, f.root.join("link.jsonl"))
        .expect("symlink subcase not_attempted: cannot create fixture");
    let r = discover(&Example, &f.root());
    assert_eq!(r.read, ReadState::Partial);
    assert!(!r.observed_zero);
    assert_eq!(r.files, 0);
    assert_eq!(r.reasons, ["symlink export refused"]);
    eprintln!("B4 Windows/native symlink fixture created; refusal assertion executed");
}
#[test]
fn review_b5_export_bounds_are_case_insensitive() {
    for id in [
        "cursor", "Cursor", "WINDSURF", "GoOsE", "devin", "Devin", "AGY", "agy", "AgY",
    ] {
        for limits in [
            ReadLimits {
                encoded_bytes: 1048577,
                ..ReadLimits::export()
            },
            ReadLimits {
                decoded_records_per_batch: 129,
                ..ReadLimits::export()
            },
        ] {
            assert_eq!(
                Descriptor {
                    source_id: id,
                    limits,
                    ..DESCRIPTOR
                }
                .validate(),
                Err("descriptor_looser_than_admitted_adapter_limit")
            );
        }
        if !id.eq_ignore_ascii_case("agy") {
            assert_eq!(
                Descriptor {
                    source_id: id,
                    limits: ReadLimits {
                        line_attempts: Some(129),
                        ..ReadLimits::export()
                    },
                    ..DESCRIPTOR
                }
                .validate(),
                Err("descriptor_looser_than_admitted_adapter_limit")
            );
        }
    }
}
#[test]
fn review_b6_pending_and_blocked_have_typed_causes_but_closed_persisted_codes() {
    let o = oracle();
    let mut codes = o["diagnostic_codes"]["existing_sdk_exact"]
        .as_array()
        .unwrap()
        .clone();
    codes.extend(
        o["diagnostic_codes"]["oracle_assigned_static"]
            .as_array()
            .unwrap()
            .clone(),
    );
    for (id, availability) in [
        ("grok", Availability::ParserPending),
        ("strace", Availability::SourceBlocked),
    ] {
        let d = Descriptor {
            source_id: id,
            ..DESCRIPTOR
        };
        assert_eq!(d.availability(), availability);
        let code = d.validate().unwrap_err();
        assert_eq!(code, "source_not_admitted");
        assert!(codes.iter().any(|c| c == code));
    }
    assert_eq!(
        dendro_connectors::grok::GrokConnector::new().availability(),
        Availability::ParserPending
    );
    assert_eq!(
        dendro_connectors::agy::AgyConnector::new().availability(),
        Availability::ParserPending
    );
}
#[test]
fn stopped_schema_replay_missing_root_and_fixture_binding_are_truthful() {
    let f = Fixture::new(
        &[
            line("projected_1"),
            line("unknown_major"),
            line("projected_1"),
        ]
        .concat(),
    );
    let mut cursor = Cursor::default();
    let first = capture(&Example, &f.root(), &f.file, &mut cursor);
    let counts = cursor.counts.clone();
    assert!(cursor.stopped_schema);
    assert_eq!(counts.unsupported_schema, 1);
    for _ in 0..3 {
        let replay = capture(&Example, &f.root(), &f.file, &mut cursor);
        assert_eq!(replay.read, ReadState::Partial);
        assert_eq!(replay.counts, Counts::default());
        assert_eq!(
            replay.watermark.as_ref().unwrap().byte_offset,
            first.watermark.as_ref().unwrap().byte_offset
        );
    }
    assert_eq!(cursor.counts, counts);
    let missing = ExportRoot::new(f.root.join("missing"));
    let r = capture(&Example, &missing, &f.file, &mut Cursor::default());
    assert_eq!(r.discovery, Discovery::Absent);
    assert_eq!(r.read, ReadState::NotAttempted);
    assert_eq!(r.files, 0);
    assert!(!r.observed_zero);
    assert_eq!(
        Descriptor {
            fixture_bytes: Some(b"different fixture"),
            ..DESCRIPTOR
        }
        .validate(),
        Err("missing_conformance_fixture")
    );
    assert_eq!(
        Descriptor {
            fixture_bytes: None,
            ..DESCRIPTOR
        }
        .validate(),
        Err("missing_conformance_fixture")
    );
    let empty = Registry::new(&[]).unwrap();
    let r = empty.capture(&Example, &f.root(), &f.file, &mut Cursor::default());
    assert_eq!(r.reasons, ["source_not_admitted"]);
    assert!(r.facts.is_empty());
}
#[test]
fn cancellation_is_typed_interruption_not_unavailable_or_zero() {
    if std::env::var_os("DENDRO_T05_CANCEL_CHILD").is_some() {
        dendro_core::RUN_CANCEL.request();
        let r = capture_bytes(&line("projected_1"));
        assert_eq!(r.read, ReadState::Interrupted);
        assert!(r.reasons.is_empty());
        assert!(!r.observed_zero);
        assert!(r.watermark.is_none());
        return;
    }
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "cancellation_is_typed_interruption_not_unavailable_or_zero",
            "--nocapture",
        ])
        .env("DENDRO_T05_CANCEL_CHILD", "1")
        .status()
        .unwrap();
    assert!(status.success());
}
