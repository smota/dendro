// The report payload is one large json! literal.
#![recursion_limit = "256"]
#![deny(unsafe_code)]

pub mod compaction;
pub mod concurrency;
pub mod facts;
pub mod file_metadata;
mod file_state;
pub mod lifecycle;
mod loop_trace;
pub mod manifest;
pub mod metrics;
pub mod models;
pub mod output;
pub mod pipeline;
pub mod privacy;
pub mod sealed_run;
pub mod storage;
pub mod tool_category;
pub mod traits;
pub mod turnaround;
pub mod vortex;
pub use vortex::{parse_friction_duration, FrictionThresholds};

pub use concurrency::*;
pub use facts::*;
pub use lifecycle::{Cancel, Interrupted, RUN_CANCEL};
pub use manifest::*;
pub use metrics::*;
pub use models::*;
pub use output::*;
pub use pipeline::*;
pub use privacy::*;
pub use sealed_run::*;
pub use storage::*;
pub use tool_category::*;
pub use traits::*;
