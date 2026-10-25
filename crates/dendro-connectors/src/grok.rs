use anyhow::Result;
use async_trait::async_trait;
use dendro_core::{CanonicalEvent, HarnessConnector, Watermark};
use std::path::{Path, PathBuf};

pub struct GrokConnector;

impl GrokConnector {
    pub fn availability(&self) -> crate::capture_registry_v1::Availability {
        crate::capture_registry_v1::Registry::builtin()
            .descriptor("grok")
            .map(crate::capture_registry_v1::Descriptor::availability)
            .expect("grok is a builtin capture reader")
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
        let sessions_dir = grok_home().join(".grok").join("sessions");
        if !sessions_dir.exists() {
            return Ok(vec![]);
        }
        let mut files = crate::stream::walk_jsonl(&sessions_dir);
        files.retain(|file| {
            file.file_name().and_then(|name| name.to_str()) == Some("updates.jsonl")
        });
        Ok(files)
    }

    fn extract_incremental(
        &self,
        path: &Path,
        watermark: Option<&Watermark>,
    ) -> Result<(Vec<CanonicalEvent>, Watermark)> {
        // `dendro run` reads sessions through `stream::ingest_jsonl`. This
        // trait method has no store, so it must not advance a watermark.
        let _ = (path, watermark);
        anyhow::bail!("source_not_admitted")
    }
}

/// Linux `HOME/.grok`, Windows `USERPROFILE\.grok`. Same split as
/// `admitted_user_home`: the profile wins on Windows.
fn grok_home() -> PathBuf {
    #[cfg(windows)]
    if let Some(profile) = env_nonempty("USERPROFILE") {
        return PathBuf::from(profile);
    }
    PathBuf::from(env_nonempty("HOME").unwrap_or_else(|| ".".into()))
}

fn env_nonempty(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|value| !value.is_empty())
}
