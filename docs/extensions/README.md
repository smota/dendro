# Dendro — Connector Extension Guide

Dendro is architected to be modular, decoupled, and extensible. If your engineering organization introduces a new agent development harness (such as Cursor, Windsurf, Devin, Goose, Aider, or proprietary internal frameworks), this guide explains how to implement a connector and register it within the analytical engine.

---

## 1. The Canonical Contract: `HarnessConnector`

Every new connector implements the `HarnessConnector` trait defined in `crates/dendro-core/src/traits.rs`:

```rust
use async_trait::async_trait;
use crate::models::{CanonicalSession, Watermark};
use crate::error::DendroResult;
use std::path::Path;

#[async_trait]
pub trait HarnessConnector: Send + Sync {
    /// Canonical identifier for the connector (e.g., "cursor", "windsurf", "devin")
    fn name(&self) -> &'static str;

    /// Checks if the local host environment has data for this connector
    async fn is_available(&self) -> bool;

    /// Discovers local session file paths for ingestion
    async fn discover_sessions(&self) -> DendroResult<Vec<std::path::PathBuf>>;

    /// Extracts and normalizes a session beginning from the provided watermark
    async fn extract_session(
        &self,
        session_path: &Path,
        watermark: Option<&Watermark>,
    ) -> DendroResult<CanonicalSession>;
}
```

---

## 2. Canonical Data Model: `CanonicalSession`

The connector normalizes heterogeneous harness logs into the universal canonical schema:

| Field | Type | Description |
| :--- | :--- | :--- |
| `id` | `String` | Stable unique session identifier (UUID or path hash) |
| `harness` | `String` | Harness name (e.g., `"cursor"`) |
| `agent_name` | `Option<String>` | Declared agent seat or identifier |
| `rig_id` | `Option<String>` | Rig / squad identifier (if resolved) |
| `cwd` | `Option<String>` | Working directory of the session |
| `started_at` | `DateTime<Utc>` | Session start timestamp |
| `ended_at` | `Option<DateTime<Utc>>`| Session termination timestamp |
| `input_tokens` | `u64` | Total prompt tokens consumed |
| `output_tokens`| `u64` | Total generated tokens |
| `cache_read_tokens` | `u64` | Cached tokens read (when supported by LLM) |
| `tool_calls` | `Vec<CanonicalToolCall>`| Tool calls executed during the session |
| `skills_invoked`| `Vec<String>` | Specialized skills or slash commands invoked |

---

## 3. Step-by-Step Implementation Guide

### Step 1: Create the Connector Module in `crates/dendro-connectors/src/`
For example, create `crates/dendro-connectors/src/cursor.rs`:

```rust
pub struct CursorConnector {
    base_dir: std::path::PathBuf,
}

impl CursorConnector {
    pub fn new() -> Self {
        let home = std::env::var("HOME").unwrap_or_default();
        Self {
            base_dir: std::path::PathBuf::from(home).join(".cursor"),
        }
    }
}

#[async_trait::async_trait]
impl HarnessConnector for CursorConnector {
    fn name(&self) -> &'static str { "cursor" }
    // Implement is_available, discover_sessions, and extract_session...
}
```

### Step 2: Export in `crates/dendro-connectors/src/lib.rs`
```rust
pub mod cursor;
pub use cursor::CursorConnector;
```

### Step 3: Register in CLI Pipeline (`crates/dendro-cli/src/main.rs`)
Instantiate the connector in the default ingestion registry executed by `dendro run` and `dendro watch`.

---

## 4. Connector Conformance Testing
Write unit tests against mocked fixture files:

```rust
#[tokio::test]
async fn test_cursor_extraction() {
    let fixture_path = Path::new("tests/fixtures/cursor_session.json");
    let connector = CursorConnector::new();
    let session = connector.extract_session(fixture_path, None).await.unwrap();
    assert_eq!(session.harness, "cursor");
    assert!(session.input_tokens > 0);
}
```
