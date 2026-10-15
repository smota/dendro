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
}

/// Opt-in export root, with no implicit access to HOME or harness configuration.
pub struct ExportRoot {
    root: PathBuf,
}
impl ExportRoot {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
    pub fn discover(&self) -> Result<Vec<PathBuf>> {
        if !self.root.exists() {
            return Ok(Vec::new());
        }
        ensure!(
            !fs::symlink_metadata(&self.root)?.file_type().is_symlink(),
            "export root must not be a symlink"
        );
        let mut paths = Vec::new();
        for entry in
            fs::read_dir(&self.root).map_err(|_| anyhow::anyhow!("cannot read export directory"))?
        {
            let entry = entry.map_err(|_| anyhow::anyhow!("cannot read export entry"))?;
            if entry.file_type()?.is_file()
                && entry.path().extension().is_some_and(|ext| ext == "jsonl")
            {
                paths.push(entry.path());
            }
        }
        paths.sort();
        Ok(paths)
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
        let resolved = path
            .canonicalize()
            .map_err(|_| anyhow::anyhow!("export unavailable"))?;
        // Only direct children are supported: no recursive configuration/auth scans or symlink parents.
        ensure!(
            resolved.parent() == Some(root.as_path()),
            "export outside selected root"
        );
        ensure!(
            !fs::symlink_metadata(path)?.file_type().is_symlink(),
            "symlink export refused"
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
        for _ in 0..MAX_BATCH_LINES {
            dendro_core::RUN_CANCEL.check()?;
            line.clear();
            let n = (&mut reader)
                .take(MAX_LINE_BYTES + 1)
                .read_until(b'\n', &mut line)?;
            if n == 0 {
                break;
            }
            if n as u64 > MAX_LINE_BYTES {
                bail!("export line exceeds 1 MiB; watermark unchanged");
            }
            if !line.ends_with(b"\n") {
                break;
            }
            hasher.update(&line);
            let start = consumed;
            consumed += n as u64;
            if line.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            match serde_json::from_slice(&line) {
                Ok(value) => records.push((start, value)),
                Err(_) => warnings.push("malformed-json-record"),
            }
        }
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
