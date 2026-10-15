//! Synthetic in-house JSONL example: no automatic HOME discovery.
#![deny(unsafe_code)]
use anyhow::{ensure, Result};
use dendro_connectors::sdk::ExportRoot;
use dendro_core::{CanonicalEvent, CanonicalSession, HarnessConnector, Watermark};
use std::path::{Path, PathBuf};

struct CustomConnector {
    root: ExportRoot,
    privacy_key: [u8; 32],
}
#[async_trait::async_trait]
impl HarnessConnector for CustomConnector {
    fn name(&self) -> &'static str {
        "custom"
    }
    fn discover_files(&self) -> Result<Vec<PathBuf>> {
        self.root.discover()
    }
    fn extract_incremental(
        &self,
        path: &Path,
        watermark: Option<&Watermark>,
    ) -> Result<(Vec<CanonicalEvent>, Watermark)> {
        let batch = self.root.read_batch(path, watermark)?;
        ensure!(
            batch.warnings.is_empty(),
            "malformed custom export; watermark not committed"
        );
        let mut events = Vec::new();
        for (_, value) in batch.records {
            ensure!(
                value["schema"] == "dendro-custom-v1",
                "unsupported custom export schema"
            );
            // An explicit projection prevents prompt, command, and arbitrary metadata persistence.
            let ts = value["timestamp"]
                .as_str()
                .and_then(dendro_core::parse_utc_time)
                .ok_or_else(|| anyhow::anyhow!("custom record needs UTC timestamp"))?;
            let native = value["session_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing session id"))?;
            let session_id = dendro_core::opaque_id(&self.privacy_key, "custom-session", native);
            let input = value["input_tokens"]
                .as_u64()
                .ok_or_else(|| anyhow::anyhow!("missing input tokens"))?;
            let output = value["output_tokens"]
                .as_u64()
                .ok_or_else(|| anyhow::anyhow!("missing output tokens"))?;
            // This demonstration schema contains per-record deltas and explicitly observed cost.
            let cost = value["cost_usd"]
                .as_f64()
                .filter(|v| v.is_finite() && *v >= 0.0)
                .ok_or_else(|| anyhow::anyhow!("missing observed cost"))?;
            events.push(CanonicalEvent::SessionUpdated(CanonicalSession {
                session_id,
                harness_name: self.name().into(),
                project_path: String::new(),
                project_name: String::new(),
                started_at: ts,
                ended_at: None,
                duration_seconds: None,
                user_turns_count: 0,
                assistant_turns_count: 0,
                tool_calls_count: 0,
                input_tokens: input,
                output_tokens: output,
                estimated_cost_usd: cost,
                status: "usage_delta".into(),
                orchestration: None,
            }));
        }
        Ok((events, batch.watermark))
    }
}
fn main() -> Result<()> {
    let mut arguments = std::env::args_os().skip(1);
    let root = arguments.next().ok_or_else(|| {
        anyhow::anyhow!(
            "usage: custom_connector <explicit-export-directory> <explicit-analytics-key-file>"
        )
    })?;
    let key_path = arguments.next().ok_or_else(|| {
        anyhow::anyhow!(
            "provide an explicit analytics key file; this example never reads harness credentials"
        )
    })?;
    let privacy_key = dendro_core::load_or_create_key(Path::new(&key_path), true)?;
    let connector = CustomConnector {
        root: ExportRoot::new(root),
        privacy_key: privacy_key.bytes,
    };
    for file in connector.discover_files()? {
        let mut watermark = None;
        loop {
            let old_offset = watermark.as_ref().map_or(0, |w: &Watermark| w.byte_offset);
            let (events, next) = connector.extract_incremental(&file, watermark.as_ref())?;
            println!(
                "events={} accepted_offset={}",
                events.len(),
                next.byte_offset
            );
            if next.byte_offset == old_offset {
                break;
            }
            // A real consumer commits events and watermark atomically before this assignment.
            watermark = Some(next);
        }
    }
    Ok(())
}
