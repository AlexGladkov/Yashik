use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use yashik::install::api::Paths;
use yashik::install::{paths, runtime};
use yashik::schema::HarnessId;

static NEXT: AtomicU64 = AtomicU64::new(0);

struct TempTree(PathBuf);

impl TempTree {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "yashik-installer-runtime-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(fs::canonicalize(path).unwrap())
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn paths(&self) -> Paths {
        Paths {
            home: self.0.join("home"),
            data: self.0.join("data"),
            cache: self.0.join("cache"),
            state: self.0.join("state"),
            bin: self.0.join("bin"),
        }
    }
}

impl Drop for TempTree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn runtime_requirements_are_closed_and_unknown_values_have_no_effects() {
    let temp = TempTree::new();
    let paths = temp.paths();
    assert!(runtime::ensure(&paths, "node --version").is_err());
    assert!(!paths.data.exists());
    assert!(!paths.cache.exists());
}

#[test]
fn malformed_exact_cli_version_is_rejected_before_bootstrap_or_network() {
    let temp = TempTree::new();
    let paths = temp.paths();
    let result = runtime::install_cli(&paths, HarnessId::Codex, Some("latest; touch /tmp/pwned"));
    assert!(result.is_err());
    assert!(!paths.data.exists());
    assert!(!paths.cache.exists());
}

#[test]
fn empty_bootstrap_is_a_noop_and_unknown_dependencies_are_rejected() {
    assert!(runtime::bootstrap_recipe(&[]).unwrap().is_none());
    assert!(runtime::bootstrap_recipe(&["not-a-real-package".to_owned()]).is_err());
}

#[test]
fn init_lock_is_exclusive_and_private() {
    let temp = TempTree::new();
    let paths = temp.paths();
    let first = paths::lock_init(&paths).unwrap();
    assert!(paths::lock_init(&paths).is_err());
    drop(first);
    assert!(paths::lock_init(&paths).is_ok());

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let metadata = fs::metadata(paths.state.join("init.lock")).unwrap();
        assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
    }
}

#[test]
fn private_paths_are_created_with_restricted_permissions() {
    let temp = TempTree::new();
    let paths = temp.paths();
    paths::create_private(&paths).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for directory in [&paths.data, &paths.cache, &paths.state] {
            assert_eq!(
                fs::metadata(directory).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
    }
    assert!(!temp.path().join("home").exists());
}

#[cfg(unix)]
#[test]
fn private_paths_and_init_lock_reject_symlink_escapes() {
    use std::os::unix::fs::{symlink, PermissionsExt};

    let temp = TempTree::new();
    let paths = temp.paths();
    let outside = temp.path().join("outside");
    fs::create_dir_all(&outside).unwrap();
    let sentinel = outside.join("sentinel");
    fs::write(&sentinel, "private data").unwrap();
    fs::set_permissions(&sentinel, fs::Permissions::from_mode(0o640)).unwrap();

    symlink(&outside, &paths.data).unwrap();
    assert!(paths::create_private(&paths).is_err());
    assert_eq!(fs::read(&sentinel).unwrap(), b"private data");
    assert_eq!(
        fs::metadata(&sentinel).unwrap().permissions().mode() & 0o777,
        0o640
    );
    fs::remove_file(&paths.data).unwrap();

    paths::create_private(&paths).unwrap();
    symlink(&sentinel, paths.state.join("init.lock")).unwrap();
    assert!(paths::lock_init(&paths).is_err());
    assert_eq!(fs::read(&sentinel).unwrap(), b"private data");
    assert_eq!(
        fs::metadata(&sentinel).unwrap().permissions().mode() & 0o777,
        0o640
    );
}
