use std::fs::File;
use std::path::{Path, PathBuf};

use fs2::FileExt;

use super::api::{InstallResult, Paths};
use super::util::{ensure_dir, open_or_create_regular, private_dir};

pub fn discover() -> InstallResult<Paths> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| "cannot determine an absolute home directory".to_owned())?;
    let home = std::fs::canonicalize(&home)
        .map_err(|_| "cannot determine a canonical home directory".to_owned())?;

    #[cfg(target_os = "macos")]
    let (data, cache, state, bin) = (
        home.join("Library/Application Support/yashik"),
        home.join("Library/Caches/yashik"),
        home.join("Library/Application Support/yashik/state"),
        home.join("Library/Application Support/yashik/bin"),
    );

    #[cfg(not(target_os = "macos"))]
    let (data, cache, state, bin) = (
        xdg_dir("XDG_DATA_HOME", &home, ".local/share").join("yashik"),
        xdg_dir("XDG_CACHE_HOME", &home, ".cache").join("yashik"),
        xdg_dir("XDG_STATE_HOME", &home, ".local/state").join("yashik"),
        home.join(".local/bin"),
    );

    Ok(Paths {
        home,
        data,
        cache,
        state,
        bin,
    })
}

fn xdg_dir(variable: &str, home: &Path, fallback: &str) -> PathBuf {
    std::env::var_os(variable)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| home.join(fallback))
}

pub fn create_private(paths: &Paths) -> InstallResult<()> {
    for directory in [&paths.data, &paths.cache, &paths.state] {
        private_dir(directory)?;
    }
    ensure_dir(&paths.bin, 0o755)
}

/// Hold this file for the duration of an `init` run to prevent concurrent writes.
pub fn lock_init(paths: &Paths) -> InstallResult<File> {
    create_private(paths)?;
    let lock_path = paths.state.join("init.lock");
    let file = open_or_create_regular(&lock_path, 0o600)?;
    file.try_lock_exclusive()
        .map_err(|_| "another Yashik init is already running".to_owned())?;
    Ok(file)
}
