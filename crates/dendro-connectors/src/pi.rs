use anyhow::Result;
use async_trait::async_trait;
use dendro_core::{CanonicalEvent, HarnessConnector, Watermark};
use std::path::{Path, PathBuf};

pub struct PiConnector;

impl PiConnector {
    pub fn new() -> Self {
        Self
    }
}

impl Default for PiConnector {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl HarnessConnector for PiConnector {
    fn name(&self) -> &'static str {
        "pi"
    }

    fn discover_files(&self) -> Result<Vec<PathBuf>> {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        let sessions_dir = PathBuf::from(home)
            .join(".pi")
            .join("agent")
            .join("sessions");
        if !sessions_dir.exists() {
            return Ok(vec![]);
        }
        Ok(crate::stream::walk_jsonl(&sessions_dir))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pi_connector_discover_files() {
        let connector = PiConnector::new();
        assert_eq!(connector.name(), "pi");
        let files = connector.discover_files().unwrap();
        for f in files {
            assert_eq!(f.extension().and_then(|ext| ext.to_str()), Some("jsonl"));
        }
    }
}
