//! Retained private parents for DuckDB-created files. No ambient umask changes.
use crate::platform_v1::{self as p, PrivateRoot};
use anyhow::{anyhow, bail, Result};
use duckdb::Connection;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};

pub(crate) struct PrivateLibraryRuntime(Arc<Runtime>);
struct Runtime {
    database: PathBuf,
    database_parent: Option<PrivateRoot>,
    _root: PrivateRoot,
    spill: PrivateRoot,
    extensions: PrivateRoot,
}
static RUNTIMES: OnceLock<Mutex<HashMap<PathBuf, Weak<Runtime>>>> = OnceLock::new();
fn child(value: &std::ffi::OsStr) -> Result<p::ChildName> {
    p::parse_child_name(
        value
            .to_str()
            .ok_or_else(|| anyhow!("non-Unicode library leaf"))?,
        p::current_native_platform(),
    )
    .map_err(p::PlatformError::Rejected)
    .map_err(Into::into)
}
fn appended(path: &Path, suffix: &str) -> PathBuf {
    let mut value = path.as_os_str().to_owned();
    value.push(suffix);
    PathBuf::from(value)
}
fn setting(conn: &Connection, key: &str) -> Result<String> {
    Ok(
        conn.query_row("SELECT current_setting(?)::VARCHAR", [key], |row| {
            row.get(0)
        })?,
    )
}
impl PrivateLibraryRuntime {
    pub(crate) fn prepare(database: &Path) -> Result<Self> {
        Self::prepare_for(database, true)
    }
    pub(crate) fn prepare_read_only(database: &Path) -> Result<Self> {
        Self::prepare_for(database, false)
    }
    fn prepare_for(database: &Path, writable: bool) -> Result<Self> {
        let database = std::path::absolute(database)?;
        let parent = database
            .parent()
            .ok_or_else(|| anyhow!("database parent absent"))?
            .to_owned();
        // Admission uses the lexical ancestry. Canonicalization must not waive
        // the ordinary no-ancestor-link policy for writable library outputs.
        let database_parent = match p::open_existing_private_root(&parent) {
            Ok(root) => Some(root),
            Err(error) if writable => return Err(error.into()),
            Err(_) => None, // immutable source input; no library outputs allowed here
        };
        let database = parent.join(
            database
                .file_name()
                .ok_or_else(|| anyhow!("database leaf absent"))?,
        );
        let registry = RUNTIMES.get_or_init(Default::default);
        let mut registry = registry
            .lock()
            .map_err(|_| anyhow!("library runtime registry poisoned"))?;
        registry.retain(|_, value| value.strong_count() != 0);
        if let Some(runtime) = registry.get(&database).and_then(Weak::upgrade) {
            if writable && runtime.database_parent.is_none() {
                bail!("writable DuckDB requires an admitted private database parent");
            }
            let result = Self(runtime);
            result.verify_files()?;
            return Ok(result);
        }
        // Existing default spill trees are never adopted, traversed or repaired.
        let default_spill = appended(&database, ".tmp");
        if std::fs::symlink_metadata(default_spill).is_ok() {
            bail!("unadmitted default DuckDB spill directory");
        }
        let root = p::create_user_private_root_with_choice(
            "dendro-library-",
            &p::StorageRootChoice::UserProfile,
        )?;
        let spill = p::create_unique_private_directory(&root, "spill-")?;
        let extensions = p::create_unique_private_directory(&root, "extensions-")?;
        let result = Self(Arc::new(Runtime {
            database: database.clone(),
            database_parent,
            _root: root,
            spill,
            extensions,
        }));
        result.verify_files()?;
        registry.insert(database, Arc::downgrade(&result.0));
        Ok(result)
    }
    pub(crate) fn configure(&self, conn: &Connection) -> Result<()> {
        if setting(conn, "lock_configuration")? != "true" {
            if !setting(conn, "access_mode")?.eq_ignore_ascii_case("read_only")
                && self.0.database_parent.is_none()
            {
                bail!("writable DuckDB requires an admitted private database parent");
            }
            // DuckDB SET path syntax has no bind parameters. Quote only trusted,
            // freshly created native paths; never interpolate user SQL.
            let quote = |path: &Path| -> Result<String> {
                Ok(path
                    .to_str()
                    .ok_or_else(|| anyhow!("non-Unicode private runtime path"))?
                    .replace('\'', "''"))
            };
            conn.execute_batch(&format!("SET temp_directory='{}'; SET extension_directory='{}'; SET autoinstall_known_extensions=false; SET autoload_known_extensions=false; SET allowed_configs=['memory_limit','threads','preserve_insertion_order']; SET lock_configuration=true;", quote(self.0.spill.path())?, quote(self.0.extensions.path())?))?;
        }
        self.verify(conn)
    }
    pub(crate) fn verify(&self, conn: &Connection) -> Result<()> {
        for (key, expected) in [
            ("temp_directory", self.0.spill.path()),
            ("extension_directory", self.0.extensions.path()),
        ] {
            if Path::new(&setting(conn, key)?) != expected {
                bail!("DuckDB {key} escaped its retained private parent");
            }
        }
        if setting(conn, "lock_configuration")? != "true"
            || setting(conn, "autoinstall_known_extensions")? != "false"
            || setting(conn, "autoload_known_extensions")? != "false"
        {
            bail!("DuckDB private runtime configuration changed");
        }
        self.verify_files()
    }
    fn verify_files(&self) -> Result<()> {
        self.0._root.check_anchor()?;
        for root in [&self.0.spill, &self.0.extensions] {
            root.check_anchor()?;
            for (index, entry) in std::fs::read_dir(root.path())?.enumerate() {
                if index >= 4096 {
                    bail!("library leaf enumeration limit");
                }
                p::verify_library_leaf(root, &child(&entry?.file_name())?)?;
            }
            root.check_anchor()?;
        }
        let wal = appended(&self.0.database, ".wal");
        if std::fs::symlink_metadata(&wal).is_ok() {
            let root = self
                .0
                .database_parent
                .as_ref()
                .ok_or_else(|| anyhow!("WAL has no admitted private parent"))?;
            p::verify_library_leaf(root, &child(wal.file_name().unwrap())?)?;
        }
        if let Some(root) = &self.0.database_parent {
            root.check_anchor()?;
        }
        Ok(())
    }
}
