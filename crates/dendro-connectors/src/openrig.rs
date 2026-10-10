use anyhow::Result;
use async_trait::async_trait;
use dendro_core::{RuntimeConnector, RuntimeSnapshot};
use std::path::PathBuf;

pub struct OpenRigConnector;

impl OpenRigConnector {
    pub fn new() -> Self {
        Self
    }

    pub fn db_path() -> PathBuf {
        let home = std::env::var("OPENRIG_HOME")
            .or_else(|_| std::env::var("HOME"))
            .unwrap_or_else(|_| ".".into());
        PathBuf::from(home).join(".openrig").join("openrig.sqlite")
    }
}

impl Default for OpenRigConnector {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl RuntimeConnector for OpenRigConnector {
    fn name(&self) -> &'static str {
        "openrig"
    }

    fn extract_topology(&self) -> Result<RuntimeSnapshot> {
        let db_path = Self::db_path();
        let mut rigs = vec![];
        if db_path.exists() {
            if let Ok(conn) = rusqlite::Connection::open(&db_path) {
                if let Ok(mut stmt) = conn.prepare("SELECT name FROM rigs") {
                    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
                    for row in rows.flatten() {
                        rigs.push(row);
                    }
                }
            }
        }
        Ok(RuntimeSnapshot {
            runtime_name: "openrig".to_string(),
            active_rigs: rigs,
        })
    }
}
