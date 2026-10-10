use anyhow::Result;
use async_trait::async_trait;
use dendro_core::{CanonicalEvent, HarnessConnector, Watermark};
use std::path::{Path, PathBuf};

pub struct ClaudeConnector;

impl ClaudeConnector {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ClaudeConnector {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl HarnessConnector for ClaudeConnector {
    fn name(&self) -> &'static str {
        "claude-code"
    }

    fn discover_files(&self) -> Result<Vec<PathBuf>> {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        let projects_dir = PathBuf::from(home).join(".claude").join("projects");
        if !projects_dir.exists() {
            return Ok(vec![]);
        }
        let mut results = vec![];
        if let Ok(entries) = std::fs::read_dir(projects_dir) {
            for entry in entries.flatten() {
                let sessions_dir = entry.path().join("sessions");
                if sessions_dir.exists() {
                    if let Ok(sess_entries) = std::fs::read_dir(sessions_dir) {
                        for sess in sess_entries.flatten() {
                            if sess.path().extension().and_then(|s| s.to_str()) == Some("jsonl") {
                                results.push(sess.path());
                            }
                        }
                    }
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
        // Incremental events extraction logic
        let events = Vec::new();
        let _ = watermark;
        Ok((events, new_watermark))
    }
}
