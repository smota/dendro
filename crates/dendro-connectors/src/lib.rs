#![deny(unsafe_code)]

pub mod agy;
pub mod claude;
pub mod codex;
pub mod grok;
pub mod ingest;
pub mod openrig;
pub mod pi;
pub mod snapshot;
pub mod stream;

pub use ingest::{ingest_staged, ingest_staged_with_manifest, SourceRoot};

pub use claude::ClaudeConnector;
pub use codex::CodexConnector;
pub use openrig::OpenRigConnector;
