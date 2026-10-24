//! Bounded, read-only JSONL primitives for explicitly selected export directories.
use anyhow::{bail, ensure, Result};
use dendro_core::Watermark;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    path::{Component, Path, PathBuf},
};

pub const MAX_LINE_BYTES: u64 = 1024 * 1024;
pub const MAX_BATCH_LINES: usize = 128;

/// A batch never includes a trailing unterminated record. Diagnostic codes contain no source text.
pub struct JsonlBatch {
    pub records: Vec<(u64, Value)>,
    pub warnings: Vec<&'static str>,
    pub watermark: Watermark,
    pub metadata: Option<dendro_core::file_metadata::FileMetadata>,
    pub raw_read_position: u64,
    pub complete: bool,
    pub whitespace_lines: usize,
    pub malformed: usize,
    pub quarantined: usize,
    pub line_attempts: usize,
    /// Input-body bytes examined; prefix verification is separate.
    pub bytes_examined: u64,
    pub max_decoded_depth: usize,
    pub max_examined_depth: usize,
}

/// Opt-in export root, with no implicit access to HOME or harness configuration.
pub struct ExportRoot {
    root: PathBuf,
}
impl ExportRoot {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
    pub fn exists(&self) -> bool {
        fs::symlink_metadata(&self.root).is_ok()
    }
    pub fn discover(&self) -> Result<Vec<PathBuf>> {
        if !fs::symlink_metadata(&self.root).is_ok() {
            return Ok(Vec::new());
        }
        ensure!(
            !fs::symlink_metadata(&self.root)?.file_type().is_symlink(),
            "export root must not be a symlink"
        );
        Ok(self.inspect_discovery()?.paths)
    }
    /// Refused JSONL candidates are explicit; they cannot imply an empty admitted root.
    pub fn inspect_discovery(&self) -> Result<DiscoveryListing> {
        let mut listing = DiscoveryListing::default();
        if !self.exists() {
            return Ok(listing);
        }
        ensure!(
            !fs::symlink_metadata(&self.root)?.file_type().is_symlink(),
            "export root must not be a symlink"
        );
        for entry in
            fs::read_dir(&self.root).map_err(|_| anyhow::anyhow!("cannot read export directory"))?
        {
            let entry = entry.map_err(|_| anyhow::anyhow!("cannot read export entry"))?;
            if !entry.path().extension().is_some_and(|ext| ext == "jsonl") {
                continue;
            }
            let metadata = fs::symlink_metadata(entry.path())
                .map_err(|_| anyhow::anyhow!("cannot read export entry"))?;
            if metadata.file_type().is_symlink() {
                push_reason(&mut listing.reasons, "symlink export refused");
            } else if !metadata.is_file() {
                push_reason(&mut listing.reasons, "export must be a regular file");
            } else {
                listing.paths.push(entry.path());
            }
        }
        listing.paths.sort();
        Ok(listing)
    }
    pub fn read_batch(&self, path: &Path, prior: Option<&Watermark>) -> Result<JsonlBatch> {
        self.read_batch_with_cutoff(path, prior, None)
    }
    pub fn read_batch_with_cutoff(
        &self,
        path: &Path,
        prior: Option<&Watermark>,
        cutoff: Option<&dendro_core::FileCutoff>,
    ) -> Result<JsonlBatch> {
        self.read_bounded(path, prior, cutoff, &ReadLimits::export(), |_| true)
    }
    /// Trusted compile-time projections may stop before committing an unsupported record.
    pub fn read_bounded(
        &self,
        path: &Path,
        prior: Option<&Watermark>,
        cutoff: Option<&dendro_core::FileCutoff>,
        limits: &ReadLimits,
        mut commit_record: impl FnMut(&Value) -> bool,
    ) -> Result<JsonlBatch> {
        limits.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            !path.components().any(|c| matches!(c, Component::ParentDir)),
            "parent traversal is not allowed"
        );
        let root = self
            .root
            .canonicalize()
            .map_err(|_| anyhow::anyhow!("export root unavailable"))?;
        ensure!(
            !fs::symlink_metadata(&self.root)?.file_type().is_symlink(),
            "export root must not be a symlink"
        );
        ensure!(
            !fs::symlink_metadata(path)?.file_type().is_symlink(),
            "symlink export refused"
        );
        let resolved = path
            .canonicalize()
            .map_err(|_| anyhow::anyhow!("export unavailable"))?;
        // Only direct children are supported: no recursive configuration/auth scans or symlink parents.
        ensure!(
            resolved.parent() == Some(root.as_path()),
            "export outside selected root"
        );
        ensure!(
            path.extension().is_some_and(|ext| ext == "jsonl"),
            "unsupported export extension; expected .jsonl"
        );
        ensure!(
            fs::symlink_metadata(path)?.is_file(),
            "export must be a regular file"
        );
        let mut file = File::open(path).map_err(|_| anyhow::anyhow!("export unavailable"))?;
        let before = dendro_core::file_metadata::FileMetadata::from_file(&file);
        ensure!(before.is_some(), "export metadata verification unsupported");
        ensure!(
            dendro_core::file_metadata::observe_path(&resolved).metadata == before,
            "export replaced before read"
        );
        ensure!(file.metadata()?.is_file(), "export must be a regular file");
        let actual_length = file.metadata()?.len();
        let length = cutoff.map_or(actual_length, |cutoff| cutoff.byte_length);
        ensure!(
            length <= actual_length,
            "mismatch: export shorter than cutoff"
        );
        if let Some(cutoff) = cutoff {
            let mut digest = Sha256::new();
            let mut bounded = (&mut file).take(length);
            let mut scratch = [0u8; 65536];
            loop {
                dendro_core::RUN_CANCEL.check()?;
                let n = bounded.read(&mut scratch)?;
                if n == 0 {
                    break;
                }
                digest.update(&scratch[..n]);
            }
            ensure!(
                hex::encode(digest.finalize()) == cutoff.prefix_sha256,
                "mismatch: export cutoff digest differs"
            );
            file.seek(SeekFrom::Start(0))?;
        }
        let identity = hex::encode(Sha256::digest(resolved.as_os_str().as_encoded_bytes()));
        let offset = prior.map_or(0, |w| w.byte_offset);
        ensure!(
            offset <= length,
            "export truncated; reset source explicitly"
        );
        if let Some(w) = prior {
            ensure!(
                w.file_path == identity,
                "watermark belongs to another export"
            );
        }
        let mut hasher = Sha256::new();
        let mut prefix = (&mut file).take(offset);
        let mut scratch = [0u8; 65536];
        loop {
            dendro_core::RUN_CANCEL.check()?;
            let n = prefix.read(&mut scratch)?;
            if n == 0 {
                break;
            }
            hasher.update(&scratch[..n]);
        }
        if let Some(w) = prior {
            ensure!(
                w.session_id.as_deref()
                    == Some(&format!(
                        "sdk-v1:{}",
                        hex::encode(hasher.clone().finalize())
                    )),
                "export prefix changed; reset source explicitly"
            );
        }
        file.seek(SeekFrom::Start(offset))?;
        let mut reader = BufReader::new(file.take(length - offset));
        let mut records = Vec::new();
        let mut warnings = Vec::new();
        let mut consumed = offset;
        let mut line = Vec::new();
        let mut attempts = 0;
        let mut whitespace_lines = 0;
        let mut malformed = 0;
        let mut quarantined = 0;
        let mut max_decoded_depth = 0;
        let mut max_examined_depth = 0;
        let mut bytes_examined = 0;
        let mut raw_read_position = offset;
        let attempt_limit = limits.effective_line_attempts();
        'records: while records.len() < limits.decoded_records_per_batch
            && attempts < attempt_limit
            && bytes_examined < MAX_BATCH_ENCODED_BYTES
        {
            dendro_core::RUN_CANCEL.check()?;
            line.clear();
            let before_record_hash = hasher.clone();
            let allowance =
                (limits.encoded_bytes + 1).min(MAX_BATCH_ENCODED_BYTES - bytes_examined);
            let n = (&mut reader).take(allowance).read_until(b'\n', &mut line)?;
            if n == 0 {
                break;
            }
            attempts += 1;
            bytes_examined += n as u64;
            raw_read_position = consumed + n as u64;
            let oversized = n as u64 > limits.encoded_bytes;
            let mut encoded_length = n as u64;
            if oversized {
                hasher.update(&line);
                let mut delimited = line.ends_with(b"\n");
                let mut scratch = Vec::with_capacity(65536);
                while !delimited {
                    dendro_core::RUN_CANCEL.check()?;
                    if bytes_examined == MAX_BATCH_ENCODED_BYTES {
                        hasher = before_record_hash;
                        if consumed == offset {
                            bail!("export line exceeds 1 MiB; watermark unchanged");
                        }
                        break 'records;
                    }
                    scratch.clear();
                    let n = (&mut reader)
                        .take(65536.min(MAX_BATCH_ENCODED_BYTES - bytes_examined))
                        .read_until(b'\n', &mut scratch)?;
                    if n == 0 {
                        hasher = before_record_hash;
                        if consumed == offset {
                            bail!("export line exceeds 1 MiB; watermark unchanged");
                        }
                        break 'records;
                    }
                    encoded_length += n as u64;
                    bytes_examined += n as u64;
                    raw_read_position = consumed + encoded_length;
                    hasher.update(&scratch);
                    delimited = scratch.ends_with(b"\n");
                }
            } else if !line.ends_with(b"\n") {
                break;
            }
            if !oversized {
                max_examined_depth = max_examined_depth.max(container_depth(&line));
            }
            if !oversized && line.iter().all(u8::is_ascii_whitespace) {
                whitespace_lines += 1;
            } else if oversized {
                quarantined += 1;
                push_reason(&mut warnings, oversize_reason(limits));
            } else if container_depth(&line) > limits.nesting_depth {
                quarantined += 1;
                push_reason(&mut warnings, "nesting_depth_exceeds_limit");
            } else {
                match decode_unique(&line) {
                    Ok(value) => {
                        max_decoded_depth = max_decoded_depth.max(container_depth(&line));
                        let commit = commit_record(&value);
                        records.push((consumed, value));
                        if !commit {
                            break;
                        }
                    }
                    Err(code) => {
                        if code == "malformed-json-record" {
                            malformed += 1;
                        } else {
                            quarantined += 1;
                        }
                        push_reason(&mut warnings, code);
                    }
                }
            }
            if !oversized {
                hasher.update(&line);
            }
            consumed += encoded_length;
        }
        let complete = consumed == length && warnings.is_empty();
        let held = reader.into_inner().into_inner();
        ensure!(
            before == dendro_core::file_metadata::FileMetadata::from_file(&held),
            "export changed during read"
        );
        ensure!(
            dendro_core::file_metadata::observe_path(&resolved).metadata == before,
            "export replaced during read"
        );
        let last_modified = held
            .metadata()?
            .modified()?
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs();
        Ok(JsonlBatch {
            metadata: before,
            raw_read_position,
            complete,
            whitespace_lines,
            malformed,
            quarantined,
            line_attempts: attempts,
            bytes_examined,
            max_decoded_depth,
            max_examined_depth,
            records,
            warnings,
            watermark: Watermark {
                file_path: identity,
                byte_offset: consumed,
                last_modified,
                session_id: Some(format!("sdk-v1:{}", hex::encode(hasher.finalize()))),
            },
        })
    }
}

/// Global hostile ceilings are defenses, never producer format admission.
pub const GLOBAL_MAX_RECORD_BYTES: u64 = 8 * 1024 * 1024;
pub const GLOBAL_MAX_DEPTH: usize = 64;
pub const GLOBAL_MAX_BATCH_RECORDS: usize = 1024;
/// Allows one max-sized record (and the max+1 quarantine boundary), while
/// bounding all retained input across a batch independently of record count.
pub const MAX_BATCH_ENCODED_BYTES: u64 = GLOBAL_MAX_RECORD_BYTES + 1;
#[derive(Default)]
pub struct DiscoveryListing {
    pub paths: Vec<PathBuf>,
    pub reasons: Vec<&'static str>,
}
pub(crate) fn oversize_reason(limits: &ReadLimits) -> &'static str {
    if limits.encoded_bytes == GLOBAL_MAX_RECORD_BYTES {
        "encoded_record_exceeds_limit"
    } else {
        "encoded_record_exceeds_adapter_limit"
    }
}
pub(crate) fn push_reason(reasons: &mut Vec<&'static str>, reason: &'static str) {
    if !reasons.contains(&reason) {
        reasons.push(reason);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReadLimits {
    pub encoded_bytes: u64,
    pub nesting_depth: usize,
    pub decoded_records_per_batch: usize,
    pub line_attempts: Option<usize>,
}
impl ReadLimits {
    pub const fn export() -> Self {
        Self {
            encoded_bytes: MAX_LINE_BYTES,
            nesting_depth: 64,
            decoded_records_per_batch: MAX_BATCH_LINES,
            line_attempts: Some(MAX_BATCH_LINES),
        }
    }
    pub fn effective_line_attempts(&self) -> usize {
        self.line_attempts
            .unwrap_or(self.decoded_records_per_batch)
            .min(GLOBAL_MAX_BATCH_RECORDS)
    }
    pub fn validate(&self) -> std::result::Result<(), &'static str> {
        if self.line_attempts == Some(0)
            || self.encoded_bytes == 0
            || self.decoded_records_per_batch == 0
            || self.encoded_bytes > GLOBAL_MAX_RECORD_BYTES
            || self.nesting_depth > GLOBAL_MAX_DEPTH
            || self.decoded_records_per_batch > GLOBAL_MAX_BATCH_RECORDS
        {
            return Err("descriptor_looser_than_global_ceiling");
        }
        Ok(())
    }
}

// Strings and escaped quotes do not contribute container depth. This scan precedes
// deserialization, bounding recursive serde visitors independently of serde defaults.
pub(crate) fn container_depth(bytes: &[u8]) -> usize {
    let (mut depth, mut maximum, mut quoted, mut escaped) = (0usize, 0, false, false);
    for &byte in bytes {
        if quoted {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                quoted = false;
            }
        } else {
            match byte {
                b'"' => quoted = true,
                b'[' | b'{' => {
                    depth += 1;
                    maximum = maximum.max(depth);
                }
                b']' | b'}' => depth = depth.saturating_sub(1),
                _ => (),
            }
        }
    }
    maximum
}

struct UniqueValue(Value);
impl<'de> serde::Deserialize<'de> for UniqueValue {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = UniqueValue;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("JSON value")
            }
            fn visit_bool<E: serde::de::Error>(
                self,
                v: bool,
            ) -> std::result::Result<UniqueValue, E> {
                Ok(UniqueValue(v.into()))
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> std::result::Result<UniqueValue, E> {
                Ok(UniqueValue(v.into()))
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> std::result::Result<UniqueValue, E> {
                Ok(UniqueValue(v.into()))
            }
            fn visit_f64<E: serde::de::Error>(self, v: f64) -> std::result::Result<UniqueValue, E> {
                serde_json::Number::from_f64(v)
                    .map(|n| UniqueValue(Value::Number(n)))
                    .ok_or_else(|| E::custom("invalid number"))
            }
            fn visit_str<E: serde::de::Error>(
                self,
                v: &str,
            ) -> std::result::Result<UniqueValue, E> {
                Ok(UniqueValue(v.into()))
            }
            fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<UniqueValue, E> {
                Ok(UniqueValue(Value::Null))
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> std::result::Result<UniqueValue, A::Error> {
                let mut values = Vec::new();
                while let Some(UniqueValue(value)) = seq.next_element()? {
                    values.push(value);
                }
                Ok(UniqueValue(Value::Array(values)))
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> std::result::Result<UniqueValue, A::Error> {
                let mut values = serde_json::Map::new();
                while let Some(key) = map.next_key::<String>()? {
                    if values.contains_key(&key) {
                        return Err(serde::de::Error::custom("duplicate_json_key"));
                    }
                    let UniqueValue(value) = map.next_value()?;
                    values.insert(key, value);
                }
                Ok(UniqueValue(Value::Object(values)))
            }
        }
        deserializer.deserialize_any(Visitor)
    }
}
fn decode_unique(bytes: &[u8]) -> std::result::Result<Value, &'static str> {
    serde_json::from_slice::<UniqueValue>(bytes)
        .map(|v| v.0)
        .map_err(|e| {
            if e.to_string().starts_with("duplicate_json_key") {
                "duplicate_json_key"
            } else {
                "malformed-json-record"
            }
        })
}
