use anyhow::Result;
use async_trait::async_trait;
use dendro_core::{CanonicalEvent, HarnessConnector, Watermark};
use std::path::{Path, PathBuf};

pub struct GrokConnector;

impl GrokConnector {
    pub const fn availability(&self) -> crate::capture_registry_v1::Availability {
        crate::capture_registry_v1::Availability::ParserPending
    }
    pub fn new() -> Self {
        Self
    }
}

impl Default for GrokConnector {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl HarnessConnector for GrokConnector {
    fn name(&self) -> &'static str {
        "grok"
    }

    fn discover_files(&self) -> Result<Vec<PathBuf>> {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        let sessions_dir = PathBuf::from(home).join(".grok").join("sessions");
        if !sessions_dir.exists() {
            return Ok(vec![]);
        }
        let mut results = vec![];
        if let Ok(entries) = std::fs::read_dir(sessions_dir) {
            for entry in entries.flatten() {
                results.push(entry.path());
            }
        }
        Ok(results)
    }

    fn extract_incremental(
        &self,
        path: &Path,
        watermark: Option<&Watermark>,
    ) -> Result<(Vec<CanonicalEvent>, Watermark)> {
        let _ = (path, watermark);
        anyhow::bail!("source_not_admitted")
    }
}
