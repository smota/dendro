//! Third-party contribution: only an allowlisted integer reaches the output.
use dendro_connectors::{capture_registry_v1::*, sdk::ReadLimits};
use serde_json::Value;
pub static DESCRIPTOR: Descriptor = Descriptor {
    contract: CONTRACT,
    contribution_version: "1.0.0",
    source_id: "example.capture.synthetic",
    schema: "example.capture.synthetic/1",
    capture_mode: "verified-export-only",
    record_kinds: &["example.record", "example.unsupported"],
    fact_grains: &["usage"],
    privacy_projection: "allowlisted kind and integer n only",
    trust: "unverified",
    admitted_native_record_bytes: None,
    fixture_bytes: Some(b"{\"kind\":\"example.record\",\"n\":1}\n"),
    fixture_digest: Some("22ff04976f4184047dfbdfd7688c6fcb83eac44a2917565c9e5bb43d6f257012"),
    limits: ReadLimits {
        encoded_bytes: 1048576,
        nesting_depth: 8,
        decoded_records_per_batch: 16,
        line_attempts: None,
    },
    admission: Admission::Implemented,
    dynamic: false,
    max_bound_processes: None,
    max_samples_per_invocation: None,
};
pub static REGISTRATIONS: &[Descriptor] = &[DESCRIPTOR];
pub struct Example;
impl Contribution for Example {
    type Fact = i64;
    fn descriptor(&self) -> &'static Descriptor {
        &DESCRIPTOR
    }
    fn project(&self, value: &Value) -> Projection<i64> {
        match (
            value.get("kind").and_then(Value::as_str),
            value.get("n").and_then(Value::as_i64),
        ) {
            (Some("example.record"), Some(n)) => Projection::Projected(n),
            (Some("example.unsupported"), _) => Projection::RecognizedUnprojected,
            (Some("example.record"), None) => Projection::Quarantined,
            _ => Projection::Unrecognized,
        }
    }
    fn ignored_unknown_fields(&self, value: &Value) -> usize {
        value.as_object().map_or(0, |o| {
            o.keys()
                .filter(|k| !matches!(k.as_str(), "kind" | "n" | "schema"))
                .count()
        })
    }
}
