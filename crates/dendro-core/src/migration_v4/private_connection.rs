//! Protected library runtime for every migration inspection, copy and rollback
//! reader. Connection drops before the pinned runtime directories.
use std::ops::Deref;
use std::path::Path;

use super::MigrationError;
use duckdb::Connection;

pub(crate) struct PrivateConnection {
    connection: Connection,
    runtime: crate::storage::PrivateLibraryRuntime,
}

impl Deref for PrivateConnection {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        &self.connection
    }
}

impl PrivateConnection {
    pub(crate) fn verify_runtime(&self) -> Result<(), MigrationError> {
        self.runtime
            .verify(&self.connection)
            .map_err(|error| MigrationError::Platform(error.to_string()))
    }
}

pub(crate) fn open(path: &Path, read_only: bool) -> Result<PrivateConnection, MigrationError> {
    let runtime = if read_only {
        crate::storage::PrivateLibraryRuntime::prepare_read_only(path)
    } else {
        crate::storage::PrivateLibraryRuntime::prepare(path)
    }
    .map_err(|error| MigrationError::Platform(error.to_string()))?;
    let connection = runtime.open(read_only).map_err(|error| {
        error
            .downcast_ref::<duckdb::Error>()
            .map(|db| MigrationError::Duck(db.to_string()))
            .unwrap_or_else(|| MigrationError::Platform(error.to_string()))
    })?;
    Ok(PrivateConnection {
        connection,
        runtime,
    })
}
