#![deny(unsafe_code)]

pub const DEFAULT_DB_HELP: &str = "Analytics database. When omitted, Dendro uses dendro.duckdb in the private per-user default folder (Windows: %LOCALAPPDATA%\\Dendro, Linux: $XDG_DATA_HOME/dendro or ~/.local/share/dendro). A database outside a private folder is refused; Dendro prints a copy command and does not copy the file.";

pub mod advise_v1;
pub mod connected_web;
pub mod serve_v1;
