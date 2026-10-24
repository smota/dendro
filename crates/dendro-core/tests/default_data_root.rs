//! Default private data folder. Env overrides stay inside this process and
//! point at a disposable private parent, never the real profile's Dendro folder.
#![cfg(any(windows, target_os = "linux"))]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use dendro_core::platform_v1::{
    create_user_private_root_with_choice, open_existing_private_root,
    open_or_create_default_data_root, StorageRootChoice,
};

static ENV_LOCK: Mutex<()> = Mutex::new(());

struct EnvGuard {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
    dir: PathBuf,
}

impl EnvGuard {
    fn set(dir: &Path) -> Self {
        let key = if cfg!(windows) {
            "LOCALAPPDATA"
        } else {
            "XDG_DATA_HOME"
        };
        let previous = std::env::var_os(key);
        std::env::set_var(key, dir);
        Self {
            key,
            previous,
            dir: dir.to_path_buf(),
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn private_parent() -> PathBuf {
    let root = create_user_private_root_with_choice(
        "dendro-default-root-",
        &StorageRootChoice::UserProfile,
    )
    .expect("disposable private parent");
    root.path().to_path_buf()
}

fn folder_name() -> &'static str {
    if cfg!(windows) {
        "Dendro"
    } else {
        "dendro"
    }
}

fn real_default_folder() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("LOCALAPPDATA").map(|path| PathBuf::from(path).join("Dendro"))
    }
    #[cfg(target_os = "linux")]
    {
        if let Some(xdg) = std::env::var_os("XDG_DATA_HOME").filter(|path| {
            let text = path.to_string_lossy();
            text.starts_with('/')
        }) {
            return Some(PathBuf::from(xdg).join("dendro"));
        }
        std::env::var_os("HOME").map(|path| PathBuf::from(path).join(".local/share/dendro"))
    }
}

#[test]
fn default_data_root_creates_reopens_and_refuses_unprotected() {
    let _lock = ENV_LOCK.lock().expect("env lock");
    let real = real_default_folder();
    let real_existed = real.as_ref().is_some_and(|path| path.exists());

    {
        let parent = private_parent();
        let _env = EnvGuard::set(&parent);
        let first = open_or_create_default_data_root().expect("create default folder");
        let path = first.path().to_path_buf();
        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some(folder_name())
        );
        assert!(path.starts_with(&parent), "{}", path.display());
        first.check_anchor().expect("created root stays pinned");
        drop(first);
        let second = open_or_create_default_data_root().expect("reopen default folder");
        assert_eq!(second.path(), path);
        second.check_anchor().expect("reopened root stays pinned");
        drop(second);
    }

    {
        let parent = private_parent();
        let folder = parent.join(folder_name());
        fs::create_dir(&folder).expect("unprotected folder");
        fs::write(folder.join("marker.txt"), b"keep").unwrap();
        #[cfg(target_os = "linux")]
        let mode = {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = fs::metadata(&folder).unwrap().permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&folder, permissions).unwrap();
            fs::metadata(&folder).unwrap().permissions().mode() & 0o777
        };
        let _env = EnvGuard::set(&parent);
        let error = open_or_create_default_data_root().expect_err("unprotected folder must fail");
        let text = error.to_string();
        assert!(text.contains("not private"), "{text}");
        assert!(
            text.contains("refusing to change its permissions"),
            "{text}"
        );
        assert_eq!(fs::read(folder.join("marker.txt")).unwrap(), b"keep");
        assert!(open_existing_private_root(&folder).is_err());
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::PermissionsExt;
            let after = fs::metadata(&folder).unwrap().permissions().mode() & 0o777;
            assert_eq!(after, mode, "existing directory mode must stay unchanged");
        }
    }

    if !real_existed {
        if let Some(path) = real {
            assert!(
                !path.exists(),
                "test touched the real default folder {}",
                path.display()
            );
        }
    }
}
