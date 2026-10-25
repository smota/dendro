#![deny(unsafe_code)]

pub mod agy;
pub mod claude;
pub mod codex;
pub mod grok;
pub mod ingest;
mod lineage;
mod messages;
pub mod openrig;
pub mod pi;
pub mod snapshot;
pub mod stream;

pub use ingest::{ingest_staged, ingest_staged_with_manifest, SourceRoot};

pub use claude::ClaudeConnector;
pub use codex::CodexConnector;
pub use openrig::OpenRigConnector;
pub mod sdk;

pub mod cursor;
pub mod devin;
pub mod exports;
pub mod goose;
pub mod windsurf;
pub use cursor::CursorConnector;
pub use devin::DevinConnector;
pub use goose::GooseConnector;
pub use windsurf::WindsurfConnector;

pub mod capture_registry_v1;
pub mod environment_v1;
pub mod syscall_v1;
pub mod windows_events_v1;
