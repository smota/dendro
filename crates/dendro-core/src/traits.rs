use std::path::{Path, PathBuf};
use anyhow::Result;
use async_trait::async_trait;
use crate::models::{CanonicalEvent, CanonicalSession, OrchestrationContext, Watermark};

#[async_trait]
pub trait HarnessConnector: Send + Sync {
    fn name(&self) -> &'static str;
    fn discover_files(&self) -> Result<Vec<PathBuf>>;
    fn extract_incremental(
        &self,
        path: &Path,
        watermark: Option<&Watermark>,
    ) -> Result<(Vec<CanonicalEvent>, Watermark)>;
}

#[derive(Debug, Clone)]
pub struct RuntimeSnapshot {
    pub runtime_name: String,
    pub active_rigs: Vec<String>,
}

#[async_trait]
pub trait RuntimeConnector: Send + Sync {
    fn name(&self) -> &'static str;
    fn extract_topology(&self) -> Result<RuntimeSnapshot>;
}

pub trait CorrelationResolver: Send + Sync {
    fn name(&self) -> &'static str;
    fn resolve(
        &self,
        session: &CanonicalSession,
        runtime: &RuntimeSnapshot,
    ) -> Option<OrchestrationContext>;
}
