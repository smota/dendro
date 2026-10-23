//! Compile-time capture contributions. Descriptors do not sandbox trusted Rust code.
use crate::sdk::{push_reason, ExportRoot, ReadLimits};
use dendro_core::Watermark;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::Path;

pub const CONTRACT: &str = "dendro.capture.contribution/v1";
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Admission {
    Implemented,
    ParserPending,
    SourceBlocked,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Availability {
    Implemented,
    ParserPending,
    SourceBlocked,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Descriptor {
    pub contract: &'static str,
    pub contribution_version: &'static str,
    pub source_id: &'static str,
    pub schema: &'static str,
    pub capture_mode: &'static str,
    pub record_kinds: &'static [&'static str],
    pub fact_grains: &'static [&'static str],
    pub privacy_projection: &'static str,
    pub trust: &'static str,
    /// Numeric producer bound, only when explicitly admitted by the pinned source profile.
    /// The global hostile cap cannot fill an absent native admission.
    pub admitted_native_record_bytes: Option<u64>,
    pub fixture_bytes: Option<&'static [u8]>,
    pub fixture_digest: Option<&'static str>,
    pub limits: ReadLimits,
    pub admission: Admission,
    pub dynamic: bool,
    pub max_bound_processes: Option<usize>,
    pub max_samples_per_invocation: Option<usize>,
}
impl Descriptor {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.contract != CONTRACT {
            return Err("incompatible_contribution_contract");
        }
        if self.dynamic {
            return Err("runtime_plugin_refused");
        }
        self.limits.validate()?;
        if self.max_bound_processes.is_some_and(|v| v > 64)
            || self.max_samples_per_invocation.is_some_and(|v| v > 600)
        {
            return Err("descriptor_looser_than_admitted_adapter_limit");
        }
        if self.availability() != Availability::Implemented {
            return Err("source_not_admitted");
        }
        if !self
            .fixture_digest
            .is_some_and(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return Err("missing_conformance_fixture");
        }
        if !self.fixture_bytes.is_some_and(|bytes| {
            Some(hex::encode(Sha256::digest(bytes)).as_str()) == self.fixture_digest
        }) {
            return Err("missing_conformance_fixture");
        }
        // Export contracts keep their admitted tighter bounds. No vendor grammar is inferred.
        if ["cursor", "windsurf", "goose", "devin", "agy"]
            .iter()
            .any(|source| self.source_id.eq_ignore_ascii_case(source))
            && (self.limits.encoded_bytes > 1048576
                || self.limits.decoded_records_per_batch > 128
                || (!self.source_id.eq_ignore_ascii_case("agy")
                    && self.limits.effective_line_attempts() > 128))
        {
            return Err("descriptor_looser_than_admitted_adapter_limit");
        }
        if self.is_native() && self.admitted_native_record_bytes.is_none() {
            return Err("native_record_size_not_admitted");
        }
        if self
            .admitted_native_record_bytes
            .is_some_and(|max| self.limits.encoded_bytes > max)
        {
            return Err("descriptor_looser_than_admitted_adapter_limit");
        }
        Ok(())
    }
    /// Internal cause is distinct from the frozen persisted refusal code.
    pub fn availability(&self) -> Availability {
        if self.source_id.eq_ignore_ascii_case("grok") || self.admission == Admission::ParserPending
        {
            Availability::ParserPending
        } else if [
            "linux strace",
            "linux_strace",
            "strace",
            "windows etw",
            "windows_etw",
            "etw",
        ]
        .iter()
        .any(|source| self.source_id.eq_ignore_ascii_case(source))
            || self.admission == Admission::SourceBlocked
        {
            Availability::SourceBlocked
        } else {
            Availability::Implemented
        }
    }
    fn is_native(&self) -> bool {
        ["claude", "codex", "pi", "openrig", "grok"]
            .iter()
            .any(|source| self.source_id.eq_ignore_ascii_case(source))
    }
    pub fn native_size_admitted(&self, bytes: u64) -> Result<(), &'static str> {
        if self.is_native()
            && !self
                .admitted_native_record_bytes
                .is_some_and(|max| bytes <= max)
        {
            Err("native_record_size_not_admitted")
        } else if bytes > self.limits.encoded_bytes {
            Err("encoded_record_exceeds_adapter_limit")
        } else {
            Ok(())
        }
    }
}

/// Standing registration takes statically compiled adapters; there is no dynamic loader.
pub struct Registry {
    descriptors: &'static [Descriptor],
}
impl Registry {
    pub fn new(descriptors: &'static [Descriptor]) -> Result<Self, &'static str> {
        for descriptor in descriptors {
            descriptor.validate()?;
        }
        for (index, descriptor) in descriptors.iter().enumerate() {
            if descriptors[..index]
                .iter()
                .any(|d| d.source_id == descriptor.source_id)
            {
                return Err("incompatible_contribution_contract");
            }
        }
        Ok(Self { descriptors })
    }
    /// Enforces standing registration for callers that do not use the trusted low-level API.
    pub fn capture<A: Contribution>(
        &self,
        adapter: &A,
        root: &ExportRoot,
        path: &Path,
        cursor: &mut Cursor,
    ) -> Report<A::Fact> {
        if self.descriptor(adapter.descriptor().source_id) != Some(adapter.descriptor()) {
            let mut report = Report::empty(Discovery::Present, ReadState::Failed);
            report.reasons.push("source_not_admitted");
            return report;
        }
        capture(adapter, root, path, cursor)
    }
    pub fn descriptor(&self, source_id: &str) -> Option<&Descriptor> {
        self.descriptors.iter().find(|d| d.source_id == source_id)
    }
}

pub enum Projection<F> {
    Projected(F),
    RecognizedUnprojected,
    /// Undeclared kind/shape, outside recognized counts.
    Unrecognized,
    /// Declared kind whose required fields fail the contribution schema.
    Quarantined,
}
pub trait Contribution {
    type Fact;
    fn descriptor(&self) -> &'static Descriptor;
    /// Projection must retain only allowlisted structured fields, never raw records.
    fn project(&self, value: &Value) -> Projection<Self::Fact>;
    fn ignored_unknown_fields(&self, _value: &Value) -> usize {
        0
    }
}
#[derive(Default, Clone, Debug, PartialEq, Eq)]
pub struct Counts {
    pub decoded: usize,
    pub projected: usize,
    pub recognized_unprojected: usize,
    pub unsupported_schema: usize,
    pub quarantined: usize,
    pub unrecognized: usize,
    pub malformed: usize,
    pub whitespace_lines: usize,
    pub ignored_unknown_field: usize,
}
impl Counts {
    pub fn recognized(&self) -> usize {
        self.projected + self.recognized_unprojected
    }
    fn problematic(&self) -> bool {
        self.unsupported_schema + self.quarantined + self.malformed + self.unrecognized != 0
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Discovery {
    Absent,
    Present,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadState {
    NotAttempted,
    Complete,
    Partial,
    Failed,
    Interrupted,
}
#[derive(Default)]
pub struct Cursor {
    pub watermark: Option<Watermark>,
    pub counts: Counts,
    pub stopped_schema: bool,
}
pub struct Report<F> {
    pub facts: Vec<F>,
    pub counts: Counts,
    pub discovery: Discovery,
    pub files: usize,
    pub read: ReadState,
    pub reasons: Vec<&'static str>,
    pub raw_read_position: u64,
    pub watermark: Option<Watermark>,
    pub observed_zero: bool,
}
impl<F> Report<F> {
    fn empty(discovery: Discovery, read: ReadState) -> Self {
        Self {
            facts: Vec::new(),
            counts: Counts::default(),
            discovery,
            files: 0,
            read,
            reasons: Vec::new(),
            raw_read_position: 0,
            watermark: None,
            observed_zero: false,
        }
    }
}
/// Reads one bounded batch from one source, stopping before an unsupported major.
/// Cursors retain prior coverage defects across resumed batches.
pub fn capture<A: Contribution>(
    adapter: &A,
    root: &ExportRoot,
    path: &Path,
    cursor: &mut Cursor,
) -> Report<A::Fact> {
    let mut report = Report::empty(Discovery::Present, ReadState::Failed);
    let descriptor = adapter.descriptor();
    if !root.exists() {
        report.discovery = Discovery::Absent;
        report.read = ReadState::NotAttempted;
        return report;
    }
    if cursor.stopped_schema {
        report.read = ReadState::Partial;
        report.reasons.push("unsupported_schema");
        report.watermark = cursor.watermark.clone();
        report.raw_read_position = cursor.watermark.as_ref().map_or(0, |w| w.byte_offset);
        return report;
    }
    if let Err(reason) = descriptor.validate() {
        report.reasons.push(reason);
        return report;
    }
    let batch = root.read_bounded(
        path,
        cursor.watermark.as_ref(),
        None,
        &descriptor.limits,
        |value| schema_supported(descriptor, value),
    );
    let batch = match batch {
        Ok(batch) => batch,
        Err(error) => {
            if error.is::<dendro_core::Interrupted>() {
                report.read = ReadState::Interrupted;
            } else {
                report.reasons.push(safe_error(&error.to_string()));
            }
            return report;
        }
    };
    report.files = 1;
    report.raw_read_position = batch.raw_read_position;
    report.counts.decoded = batch.records.len();
    report.counts.whitespace_lines = batch.whitespace_lines;
    report.counts.malformed = batch.malformed;
    report.counts.quarantined = batch.quarantined;
    for reason in batch.warnings {
        push_reason(&mut report.reasons, reason);
    }
    for (_, value) in batch.records {
        if !schema_supported(descriptor, &value) {
            report.counts.unsupported_schema += 1;
            cursor.stopped_schema = true;
            push_reason(&mut report.reasons, "unsupported_schema");
            break;
        }
        report.counts.ignored_unknown_field += adapter.ignored_unknown_fields(&value);
        match adapter.project(&value) {
            Projection::Projected(fact) => {
                report.facts.push(fact);
                report.counts.projected += 1;
            }
            Projection::RecognizedUnprojected => report.counts.recognized_unprojected += 1,
            Projection::Unrecognized => {
                report.counts.unrecognized += 1;
                push_reason(&mut report.reasons, "unsupported_schema");
            }
            Projection::Quarantined => {
                report.counts.quarantined += 1;
                push_reason(&mut report.reasons, "malformed-json-record");
            }
        }
    }
    cursor.counts.decoded += report.counts.decoded;
    cursor.counts.projected += report.counts.projected;
    cursor.counts.recognized_unprojected += report.counts.recognized_unprojected;
    cursor.counts.unsupported_schema += report.counts.unsupported_schema;
    cursor.counts.quarantined += report.counts.quarantined;
    cursor.counts.unrecognized += report.counts.unrecognized;
    cursor.counts.malformed += report.counts.malformed;
    cursor.watermark = Some(batch.watermark.clone());
    report.watermark = Some(batch.watermark);
    report.read = if batch.complete && !cursor.counts.problematic() {
        ReadState::Complete
    } else {
        ReadState::Partial
    };
    report.observed_zero = report.read == ReadState::Complete
        && cursor.counts.decoded == 0
        && !cursor.counts.problematic();
    report
}
fn schema_supported(descriptor: &Descriptor, value: &Value) -> bool {
    value
        .get("schema")
        .is_none_or(|schema| schema.as_str() == Some(descriptor.schema))
}
/// Scoped directory discovery, with no implicit roots or grammar assumptions.
pub fn discover<A: Contribution>(adapter: &A, root: &ExportRoot) -> Report<A::Fact> {
    let mut report = Report::empty(Discovery::Present, ReadState::Failed);
    if let Err(reason) = adapter.descriptor().validate() {
        report.reasons.push(reason);
        return report;
    }
    if !root.exists() {
        report.discovery = Discovery::Absent;
        report.read = ReadState::NotAttempted;
        return report;
    }
    match root.inspect_discovery() {
        Ok(listing) => {
            report.files = listing.paths.len();
            report.reasons = listing.reasons;
            if !report.reasons.is_empty() {
                report.read = ReadState::Partial;
            } else if listing.paths.is_empty() {
                report.read = ReadState::Complete;
                report.observed_zero = true;
            } else {
                report.read = ReadState::NotAttempted;
            }
        }
        Err(error) => report.reasons.push(safe_error(&error.to_string())),
    }
    report
}
fn safe_error(message: &str) -> &'static str {
    const CODES: &[&str] = &[
        "export line exceeds 1 MiB; watermark unchanged",
        "export truncated; reset source explicitly",
        "export prefix changed; reset source explicitly",
        "watermark belongs to another export",
        "parent traversal is not allowed",
        "export root must not be a symlink",
        "export outside selected root",
        "symlink export refused",
        "unsupported export extension; expected .jsonl",
        "export must be a regular file",
        "mismatch: export shorter than cutoff",
        "mismatch: export cutoff digest differs",
        "export changed during read",
        "export replaced during read",
        "export replaced before read",
        "export metadata verification unsupported",
        "export root unavailable",
        "export unavailable",
        "cannot read export directory",
        "cannot read export entry",
    ];
    CODES
        .iter()
        .copied()
        .find(|code| *code == message)
        .unwrap_or("export unavailable")
}
