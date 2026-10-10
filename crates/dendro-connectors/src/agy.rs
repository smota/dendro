use std::path::{Path, PathBuf};
use anyhow::Result;
use async_trait::async_trait;
use dendro_core::{CanonicalEvent, HarnessConnector, Watermark};

pub struct AgyConnector;

impl AgyConnector {
    pub fn new() -> Self {
        Self
    }
}

impl Default for AgyConnector {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl HarnessConnector for AgyConnector {
    fn name(&self) -> &'static str {
        "agy"
    }

    fn discover_files(&self) -> Result<Vec<PathBuf>> {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        let brain_dir = PathBuf::from(home)
            .join(".gemini")
            .join("antigravity-cli")
            .join("brain");
        if !brain_dir.exists() {
            return Ok(vec![]);
        }
        let mut results = vec![];
        if let Ok(entries) = std::fs::read_dir(brain_dir) {
            for entry in entries.flatten() {
                let log = entry
                    .path()
                    .join(".system_generated")
                    .join("logs")
                    .join("transcript.jsonl");
                if log.exists() {
                    results.push(log);
                }
            }
        }
        Ok(results)
    }

    fn extract_incremental(
        &self,
        path: &Path,
        watermark: Option<&Watermark>,
    ) -> Result<(Vec<CanonicalEvent>, Watermark)> {
        let metadata = std::fs::metadata(path)?;
        let last_modified = metadata
            .modified()?
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs();
        let new_watermark = Watermark {
            file_path: path.to_string_lossy().to_string(),
            byte_offset: metadata.len(),
            last_modified,
            session_id: None,
        };
        let events = Vec::new();
        let _ = watermark;
        Ok((events, new_watermark))
    }
}
