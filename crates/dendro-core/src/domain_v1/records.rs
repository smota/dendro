//! Typed record families, all wrapped in `Record<T>` at admission.
use super::*;
#[path = "record_types/common.rs"]
mod common;
pub use common::*;
#[path = "record_types/actors.rs"]
mod actors;
pub use actors::*;
#[path = "record_types/usage.rs"]
mod usage;
pub use usage::*;
#[path = "record_types/capabilities.rs"]
mod capabilities;
pub use capabilities::*;
#[path = "record_types/environment.rs"]
mod environment;
pub use environment::*;
#[path = "record_types/governance.rs"]
mod governance;
pub(crate) use common::{require_evidence, validate_bounds};
pub use governance::*;
