#[cfg(not(unix))]
use std::fs::OpenOptions;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sha2::{Digest, Sha256};

use super::api::InstallResult;

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn sha256_bytes(bytes: &[u8]) -> String {
    hex_digest(Sha256::digest(bytes).as_slice())
}

pub fn sha256_file(path: &Path) -> InstallResult<String> {
    #[cfg(unix)]
    let mut file = open_regular_absolute(path)?;
    #[cfg(not(unix))]
    let mut file = File::open(path).map_err(|_| "cannot read a file for hashing".to_owned())?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|_| "cannot read a file for hashing".to_owned())?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(hex_digest(&digest.finalize()))
}

pub fn hash_tree(root: &Path) -> InstallResult<String> {
    let root = fs::canonicalize(root).map_err(|_| "source root does not exist".to_owned())?;
    if !fs::metadata(&root)
        .map_err(|_| "cannot inspect source root".to_owned())?
        .is_dir()
    {
        return Err("source root is not a directory".to_owned());
    }
    let mut entries = Vec::new();
    collect_tree(&root, &root, &mut entries)?;
    entries.sort_by_key(|entry| path_bytes(entry.0.as_path()));

    let mut digest = Sha256::new();
    digest.update(b"yashik-tree-v1\0");
    for (relative, kind, mode, content) in entries {
        hash_field(&mut digest, &path_bytes(&relative));
        digest.update([kind]);
        digest.update(mode.to_be_bytes());
        hash_field(&mut digest, &content);
    }
    Ok(hex_digest(&digest.finalize()))
}

pub fn read_file_checked(root: &Path, path: &Path) -> InstallResult<Vec<u8>> {
    read_file_checked_bounded(root, path, u64::MAX)
}

pub fn read_file_checked_bounded(
    root: &Path,
    path: &Path,
    max_bytes: u64,
) -> InstallResult<Vec<u8>> {
    let canonical_root =
        fs::canonicalize(root).map_err(|_| "source root does not exist".to_owned())?;
    let metadata =
        fs::symlink_metadata(path).map_err(|_| "source file does not exist".to_owned())?;
    if !metadata.is_file() {
        return Err("selected source is not a regular file".to_owned());
    }
    let mut file = open_regular_under(&canonical_root, path)?;
    let opened = file
        .metadata()
        .map_err(|_| "cannot inspect source file".to_owned())?;
    if !same_file(&metadata, &opened) {
        return Err("source file changed during inspection".to_owned());
    }
    let bytes = read_bounded_file(&mut file, max_bytes)?;
    let after = file
        .metadata()
        .map_err(|_| "cannot inspect source file".to_owned())?;
    if !same_content_generation(&metadata, &after) {
        return Err("source file changed during inspection".to_owned());
    }
    Ok(bytes)
}

pub fn copy_file_checked(root: &Path, source: &Path, destination: &Path) -> InstallResult<String> {
    let canonical_root =
        fs::canonicalize(root).map_err(|_| "source root does not exist".to_owned())?;
    let before =
        fs::symlink_metadata(source).map_err(|_| "source file does not exist".to_owned())?;
    if !before.is_file() {
        return Err("selected source is not a regular file".to_owned());
    }
    let mut input = open_regular_under(&canonical_root, source)?;
    let opened = input
        .metadata()
        .map_err(|_| "cannot inspect source file".to_owned())?;
    if !opened.is_file() || !same_file(&before, &opened) {
        return Err("source file changed during copy".to_owned());
    }
    let mut output = create_regular_new_nofollow(destination, 0o600)?;
    std::io::copy(&mut input, &mut output).map_err(|_| "cannot copy source file".to_owned())?;
    output
        .sync_all()
        .map_err(|_| "cannot sync copied file".to_owned())?;
    let after = input
        .metadata()
        .map_err(|_| "cannot inspect source file".to_owned())?;
    if !after.is_file() || !same_content_generation(&before, &after) {
        return Err("source file changed during copy".to_owned());
    }
    if output
        .metadata()
        .map_err(|_| "cannot inspect copied file".to_owned())?
        .len()
        != before.len()
    {
        return Err("source file changed during copy".to_owned());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        set_file_mode(&output, before.permissions().mode() & 0o7777)?;
    }
    output
        .sync_all()
        .map_err(|_| "cannot sync copied file".to_owned())?;
    hash_open_file(&mut output)
}

type TreeEntry = (PathBuf, u8, u32, Vec<u8>);

fn collect_tree(root: &Path, directory: &Path, entries: &mut Vec<TreeEntry>) -> InstallResult<()> {
    let current = fs::canonicalize(directory)
        .map_err(|_| "source tree changed during inspection".to_owned())?;
    if !current.starts_with(root) {
        return Err("source tree contains a path outside its root".to_owned());
    }
    let mut children = fs::read_dir(directory)
        .map_err(|_| "cannot read source directory".to_owned())?
        .map(|entry| {
            entry
                .map(|entry| entry.path())
                .map_err(|_| "cannot read source directory".to_owned())
        })
        .collect::<InstallResult<Vec<_>>>()?;
    children.sort();
    for child in children {
        let metadata = fs::symlink_metadata(&child)
            .map_err(|_| "source tree changed during inspection".to_owned())?;
        let relative = child
            .strip_prefix(root)
            .map_err(|_| "source tree contains a path outside its root".to_owned())?
            .to_path_buf();
        #[cfg(unix)]
        let mode = {
            use std::os::unix::fs::PermissionsExt;
            metadata.permissions().mode() & 0o7777
        };
        #[cfg(not(unix))]
        let mode = 0;

        if metadata.file_type().is_symlink() {
            let target =
                fs::read_link(&child).map_err(|_| "cannot read source symbolic link".to_owned())?;
            if target.is_absolute() {
                return Err("source tree contains an absolute symbolic link".to_owned());
            }
            let resolved = fs::canonicalize(&child).map_err(|_| {
                "source tree contains a dangling or cyclic symbolic link".to_owned()
            })?;
            if !resolved.starts_with(root) {
                return Err("source tree contains a symbolic link outside its root".to_owned());
            }
            let after = fs::read_link(&child)
                .map_err(|_| "source tree changed during inspection".to_owned())?;
            if target != after {
                return Err("source tree changed during inspection".to_owned());
            }
            entries.push((relative, b'l', mode, path_bytes(&target)));
        } else if metadata.is_dir() {
            entries.push((relative, b'd', mode, Vec::new()));
            collect_tree(root, &child, entries)?;
        } else if metadata.is_file() {
            let content = read_checked_file(root, &child, &metadata)?;
            entries.push((relative, b'f', mode, content));
        } else {
            return Err("source tree contains a special file".to_owned());
        }
    }
    Ok(())
}

fn read_checked_file(root: &Path, path: &Path, before: &fs::Metadata) -> InstallResult<Vec<u8>> {
    let mut file = open_regular_under(root, path)?;
    let opened = file
        .metadata()
        .map_err(|_| "cannot inspect source file".to_owned())?;
    if !opened.is_file() || !same_file(before, &opened) {
        return Err("source file changed during inspection".to_owned());
    }
    let content = read_bounded_file(&mut file, u64::MAX)?;
    let after = file
        .metadata()
        .map_err(|_| "cannot inspect source file".to_owned())?;
    if !after.is_file() || !same_content_generation(before, &after) {
        return Err("source file changed during inspection".to_owned());
    }
    Ok(content)
}

fn read_bounded_file(file: &mut File, max_bytes: u64) -> InstallResult<Vec<u8>> {
    let mut bytes = Vec::new();
    file.take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|_| "cannot read source file".to_owned())?;
    if bytes.len() as u64 > max_bytes {
        return Err("source file exceeds the configured read limit".to_owned());
    }
    Ok(bytes)
}

pub fn copy_tree_checked(source_root: &Path, destination: &Path) -> InstallResult<String> {
    if fs::symlink_metadata(source_root).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err("source root became a symbolic link".to_owned());
    }
    let source_root =
        fs::canonicalize(source_root).map_err(|_| "source root does not exist".to_owned())?;
    if !fs::metadata(&source_root)
        .map_err(|_| "cannot inspect source root".to_owned())?
        .is_dir()
    {
        return Err("source root is not a directory".to_owned());
    }
    let parent = destination
        .parent()
        .ok_or_else(|| "copy destination has no parent".to_owned())?;
    ensure_dir(parent, 0o700)?;
    if fs::symlink_metadata(destination).is_ok() {
        return Err("copy destination already exists".to_owned());
    }
    copy_directory_checked(&source_root, &source_root, destination)?;
    hash_tree(destination)
}

fn copy_directory_checked(root: &Path, source: &Path, destination: &Path) -> InstallResult<()> {
    let resolved =
        fs::canonicalize(source).map_err(|_| "source tree changed during copy".to_owned())?;
    if !resolved.starts_with(root) {
        return Err("source tree contains a path outside its root".to_owned());
    }
    create_private_dir_new(destination)?;
    let mut children = fs::read_dir(source)
        .map_err(|_| "cannot read source directory".to_owned())?
        .map(|entry| {
            entry
                .map(|entry| entry.path())
                .map_err(|_| "cannot read source directory".to_owned())
        })
        .collect::<InstallResult<Vec<_>>>()?;
    children.sort();
    for child in children {
        let target = destination.join(
            child
                .file_name()
                .ok_or_else(|| "invalid source path".to_owned())?,
        );
        let metadata = fs::symlink_metadata(&child)
            .map_err(|_| "source tree changed during copy".to_owned())?;
        if metadata.file_type().is_symlink() {
            let link =
                fs::read_link(&child).map_err(|_| "cannot read source symbolic link".to_owned())?;
            if link.is_absolute() {
                return Err("source tree contains an absolute symbolic link".to_owned());
            }
            let resolved = fs::canonicalize(&child).map_err(|_| {
                "source tree contains a dangling or cyclic symbolic link".to_owned()
            })?;
            if !resolved.starts_with(root) {
                return Err("source tree contains a symbolic link outside its root".to_owned());
            }
            if fs::read_link(&child).map_err(|_| "source tree changed during copy".to_owned())?
                != link
            {
                return Err("source tree changed during copy".to_owned());
            }
            create_symlink(&link, &target)?;
        } else if metadata.is_dir() {
            copy_directory_checked(root, &child, &target)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                set_directory_mode_nofollow(&target, metadata.permissions().mode() & 0o7777)?;
            }
        } else if metadata.is_file() {
            copy_regular_checked(root, &child, &target, &metadata)?;
        } else {
            return Err("source tree contains a special file".to_owned());
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let metadata = fs::symlink_metadata(source)
            .map_err(|_| "source tree changed during copy".to_owned())?;
        set_directory_mode_nofollow(destination, metadata.permissions().mode() & 0o7777)?;
    }
    Ok(())
}

fn copy_regular_checked(
    root: &Path,
    source: &Path,
    destination: &Path,
    before: &fs::Metadata,
) -> InstallResult<()> {
    let mut input = open_regular_under(root, source)?;
    let opened = input
        .metadata()
        .map_err(|_| "cannot inspect source file".to_owned())?;
    if !opened.is_file() || !same_file(before, &opened) {
        return Err("source file changed during copy".to_owned());
    }
    let mut output = create_regular_new_nofollow(destination, 0o600)?;
    std::io::copy(&mut input, &mut output).map_err(|_| "cannot copy source file".to_owned())?;
    output
        .sync_all()
        .map_err(|_| "cannot sync copied file".to_owned())?;
    let after = input
        .metadata()
        .map_err(|_| "cannot inspect source file".to_owned())?;
    if !after.is_file() || !same_content_generation(before, &after) {
        return Err("source file changed during copy".to_owned());
    }
    if output
        .metadata()
        .map_err(|_| "cannot inspect copied file".to_owned())?
        .len()
        != before.len()
    {
        return Err("source file changed during copy".to_owned());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        set_file_mode(&output, before.permissions().mode() & 0o7777)?;
    }
    output
        .sync_all()
        .map_err(|_| "cannot sync copied file".to_owned())?;
    Ok(())
}

#[cfg(unix)]
fn create_symlink(target: &Path, link: &Path) -> InstallResult<()> {
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;

    let (parent, name) = secure_parent_and_name(link, true)?;
    let target = std::ffi::CString::new(target.as_os_str().as_bytes())
        .map_err(|_| "source symbolic link contains a null byte".to_owned())?;
    let result = unsafe { libc::symlinkat(target.as_ptr(), parent.as_raw_fd(), name.as_ptr()) };
    if result == 0 {
        Ok(())
    } else {
        Err("cannot copy source symbolic link".to_owned())
    }
}

#[cfg(not(unix))]
fn create_symlink(_target: &Path, _link: &Path) -> InstallResult<()> {
    Err("symbolic links are unsupported on this platform".to_owned())
}

#[cfg(unix)]
fn same_file(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    left.dev() == right.dev() && left.ino() == right.ino()
}

fn same_content_generation(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        same_file(left, right)
            && left.len() == right.len()
            && left.mtime() == right.mtime()
            && left.mtime_nsec() == right.mtime_nsec()
            && left.ctime() == right.ctime()
            && left.ctime_nsec() == right.ctime_nsec()
    }
    #[cfg(not(unix))]
    {
        same_file(left, right) && left.len() == right.len()
    }
}

#[cfg(not(unix))]
fn same_file(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    left.len() == right.len() && left.modified().ok() == right.modified().ok()
}

pub fn atomic_write(path: &Path, bytes: &[u8], mode: u32) -> InstallResult<()> {
    atomic_write_inner(path, bytes, mode, None)
}

pub fn private_dir(path: &Path) -> InstallResult<()> {
    ensure_dir_mode(path, 0o700, true)
}

/// Create a directory that must not already exist, rejecting linked parents and leaves.
pub fn create_private_dir_new(path: &Path) -> InstallResult<()> {
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        let (parent_path, name) = parent_and_name(path)?;
        let parent = secure_open_directory(&parent_path, true, 0o700)?;
        let result = unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700) };
        if result != 0 {
            return Err("cannot create new private directory".to_owned());
        }
        let child = open_child_directory(&parent, &name)?;
        set_directory_fd_mode(&child, 0o700)
    }
    #[cfg(not(unix))]
    {
        check_no_symlink_components(path, true)?;
        fs::create_dir(path).map_err(|_| "cannot create new private directory".to_owned())?;
        Ok(())
    }
}

/// Ensure a directory exists without following symlinks; existing permissions are preserved.
pub fn ensure_dir(path: &Path, create_mode: u32) -> InstallResult<()> {
    ensure_dir_mode(path, create_mode, false)
}

/// Open or create a regular file without following its final component or parent links.
pub fn open_or_create_regular(path: &Path, mode: u32) -> InstallResult<File> {
    #[cfg(unix)]
    {
        use std::os::fd::{AsRawFd, FromRawFd};
        let (parent_path, name) = parent_and_name(path)?;
        let parent = secure_open_directory(&parent_path, true, 0o700)?;
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDWR | libc::O_CREAT | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                mode as libc::mode_t,
            )
        };
        if fd < 0 {
            return Err("cannot open private regular file".to_owned());
        }
        let file = unsafe { File::from_raw_fd(fd) };
        if !file
            .metadata()
            .map_err(|_| "cannot inspect private regular file".to_owned())?
            .is_file()
        {
            return Err("private path is not a regular file".to_owned());
        }
        set_file_mode(&file, mode)?;
        Ok(file)
    }
    #[cfg(not(unix))]
    {
        check_no_symlink_components(path, true)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true);
        let file = options
            .open(path)
            .map_err(|_| "cannot open private regular file".to_owned())?;
        if !file
            .metadata()
            .map_err(|_| "cannot inspect private regular file".to_owned())?
            .is_file()
        {
            return Err("private path is not a regular file".to_owned());
        }
        Ok(file)
    }
}

/// Atomically write only if the current target state matches the expected hash or absence.
pub fn atomic_write_if_unchanged(
    path: &Path,
    bytes: &[u8],
    mode: u32,
    expected_sha256: Option<&str>,
) -> InstallResult<()> {
    atomic_write_inner(path, bytes, mode, Some(expected_sha256))
}

/// Remove a regular file only when its current SHA-256 matches the recorded fingerprint.
pub fn remove_file_checked(path: &Path, expected_sha256: Option<&str>) -> InstallResult<()> {
    #[cfg(unix)]
    {
        use std::os::fd::{AsRawFd, FromRawFd};
        let (parent_path, name) = parent_and_name(path)?;
        let parent = secure_open_directory(&parent_path, false, 0o700)?;
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if fd < 0 {
            if std::io::Error::last_os_error().raw_os_error() == Some(libc::ENOENT) {
                return Ok(());
            }
            return Err("cannot safely open managed file".to_owned());
        }
        let mut file = unsafe { File::from_raw_fd(fd) };
        let before = file
            .metadata()
            .map_err(|_| "cannot inspect managed file".to_owned())?;
        if !before.is_file() {
            return Err("managed path is not a regular file".to_owned());
        }
        let actual = hash_open_file(&mut file)?;
        if expected_sha256.is_some_and(|expected| expected != actual) {
            return Err("managed file no longer matches its recorded fingerprint".to_owned());
        }
        let mut current = std::mem::MaybeUninit::<libc::stat>::uninit();
        let result = unsafe {
            libc::fstatat(
                parent.as_raw_fd(),
                name.as_ptr(),
                current.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        if result != 0 {
            return Err("managed file changed during removal".to_owned());
        }
        let current = unsafe { current.assume_init() };
        if current.st_dev != metadata_dev(&before) || current.st_ino != metadata_ino(&before) {
            return Err("managed file changed during removal".to_owned());
        }
        if unsafe { libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), 0) } != 0 {
            return Err("cannot remove managed file".to_owned());
        }
        parent
            .sync_all()
            .map_err(|_| "cannot sync managed file removal".to_owned())?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let metadata =
            fs::symlink_metadata(path).map_err(|_| "managed file does not exist".to_owned())?;
        if !metadata.is_file() {
            return Err("managed path is not a regular file".to_owned());
        }
        let actual = sha256_file(path)?;
        if expected_sha256.is_some_and(|expected| expected != actual) {
            return Err("managed file no longer matches its recorded fingerprint".to_owned());
        }
        fs::remove_file(path).map_err(|_| "cannot remove managed file".to_owned())
    }
}

/// Remove one tree without following symlinks in the leaf or any descendants.
pub fn remove_tree_checked(path: &Path) -> InstallResult<()> {
    #[cfg(unix)]
    {
        use std::os::fd::{AsRawFd, FromRawFd};
        let (parent_path, name) = parent_and_name(path)?;
        let parent = secure_open_directory(&parent_path, false, 0o700)?;
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if fd < 0 {
            if std::io::Error::last_os_error().raw_os_error() == Some(libc::ENOENT) {
                return Ok(());
            }
            return Err("cannot safely open managed directory for removal".to_owned());
        }
        let directory = unsafe { File::from_raw_fd(fd) };
        let identity = directory
            .metadata()
            .map_err(|_| "cannot inspect managed directory".to_owned())?;
        remove_directory_contents(&directory)?;
        let mut current = std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe {
            libc::fstatat(
                parent.as_raw_fd(),
                name.as_ptr(),
                current.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } != 0
        {
            return Err("managed directory changed during removal".to_owned());
        }
        let current = unsafe { current.assume_init() };
        if current.st_dev != metadata_dev(&identity) || current.st_ino != metadata_ino(&identity) {
            return Err("managed directory changed during removal".to_owned());
        }
        if unsafe { libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), libc::AT_REMOVEDIR) } != 0 {
            return Err("cannot remove managed directory".to_owned());
        }
        parent
            .sync_all()
            .map_err(|_| "cannot sync managed directory removal".to_owned())?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let parent = path
            .parent()
            .ok_or_else(|| "managed directory has no parent".to_owned())?;
        check_no_symlink_components(parent, false)?;
        match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
                fs::remove_dir_all(path).map_err(|_| "cannot remove managed directory".to_owned())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            _ => Err("managed path is not a real directory".to_owned()),
        }
    }
}

#[cfg(unix)]
struct DirectoryStream(*mut libc::DIR);

#[cfg(unix)]
impl Drop for DirectoryStream {
    fn drop(&mut self) {
        unsafe {
            libc::closedir(self.0);
        }
    }
}

#[cfg(unix)]
fn remove_directory_contents(directory: &File) -> InstallResult<()> {
    use std::ffi::CStr;
    use std::os::fd::{AsRawFd, FromRawFd};
    let duplicate = unsafe { libc::fcntl(directory.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 0) };
    if duplicate < 0 {
        return Err("cannot enumerate managed directory".to_owned());
    }
    let stream = unsafe { libc::fdopendir(duplicate) };
    if stream.is_null() {
        unsafe {
            libc::close(duplicate);
        }
        return Err("cannot enumerate managed directory".to_owned());
    }
    let stream = DirectoryStream(stream);
    loop {
        let entry = unsafe { libc::readdir(stream.0) };
        if entry.is_null() {
            break;
        }
        let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) };
        if name.to_bytes() == b"." || name.to_bytes() == b".." {
            continue;
        }
        let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
        let result = unsafe {
            libc::fstatat(
                directory.as_raw_fd(),
                name.as_ptr(),
                metadata.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        if result != 0 {
            if std::io::Error::last_os_error().raw_os_error() == Some(libc::ENOENT) {
                continue;
            }
            return Err("cannot inspect managed directory entry".to_owned());
        }
        let metadata = unsafe { metadata.assume_init() };
        if metadata.st_mode & libc::S_IFMT == libc::S_IFDIR {
            let child_fd = unsafe {
                libc::openat(
                    directory.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                )
            };
            if child_fd < 0 {
                return Err("managed directory entry changed during removal".to_owned());
            }
            let child = unsafe { File::from_raw_fd(child_fd) };
            let identity = child
                .metadata()
                .map_err(|_| "cannot inspect managed directory entry".to_owned())?;
            remove_directory_contents(&child)?;
            let mut current = std::mem::MaybeUninit::<libc::stat>::uninit();
            if unsafe {
                libc::fstatat(
                    directory.as_raw_fd(),
                    name.as_ptr(),
                    current.as_mut_ptr(),
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            } != 0
            {
                return Err("managed directory entry changed during removal".to_owned());
            }
            let current = unsafe { current.assume_init() };
            if current.st_dev != metadata_dev(&identity)
                || current.st_ino != metadata_ino(&identity)
            {
                return Err("managed directory entry changed during removal".to_owned());
            }
            if unsafe { libc::unlinkat(directory.as_raw_fd(), name.as_ptr(), libc::AT_REMOVEDIR) }
                != 0
            {
                return Err("cannot remove managed subdirectory".to_owned());
            }
        } else if unsafe { libc::unlinkat(directory.as_raw_fd(), name.as_ptr(), 0) } != 0
            && std::io::Error::last_os_error().raw_os_error() != Some(libc::ENOENT)
        {
            return Err("cannot remove managed file".to_owned());
        }
    }
    Ok(())
}

#[cfg(unix)]
fn atomic_write_inner(
    path: &Path,
    bytes: &[u8],
    mode: u32,
    expectation: Option<Option<&str>>,
) -> InstallResult<()> {
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    let (parent_path, target_name) = parent_and_name(path)?;
    let parent = secure_open_directory(&parent_path, true, 0o700)?;
    verify_atomic_target(&parent, &target_name, expectation)?;

    let base = path
        .file_name()
        .ok_or_else(|| "atomic file has no file name".to_owned())?;
    let mut temporary_name = None;
    let mut temporary_file = None;
    for _ in 0..32 {
        let sequence = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let mut name = base.to_os_string();
        name.push(format!(".{}.{}.tmp", std::process::id(), sequence));
        let name = std::ffi::CString::new(name.as_os_str().as_bytes())
            .map_err(|_| "atomic file name contains a null byte".to_owned())?;
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                mode as libc::mode_t,
            )
        };
        if fd >= 0 {
            temporary_name = Some(name);
            temporary_file = Some(unsafe { File::from_raw_fd(fd) });
            break;
        }
        if std::io::Error::last_os_error().raw_os_error() != Some(libc::EEXIST) {
            return Err("cannot create atomic temporary file".to_owned());
        }
    }
    let temporary_name =
        temporary_name.ok_or_else(|| "cannot reserve atomic temporary file".to_owned())?;
    let mut temporary_file = temporary_file.expect("temporary name has an open file");
    let write_result = temporary_file
        .write_all(bytes)
        .and_then(|_| temporary_file.sync_all());
    if write_result.is_err() || set_file_mode(&temporary_file, mode).is_err() {
        drop(temporary_file);
        unsafe {
            libc::unlinkat(parent.as_raw_fd(), temporary_name.as_ptr(), 0);
        }
        return Err("cannot write atomic temporary file".to_owned());
    }
    temporary_file.sync_all().map_err(|_| {
        unsafe {
            libc::unlinkat(parent.as_raw_fd(), temporary_name.as_ptr(), 0);
        }
        "cannot sync atomic temporary file".to_owned()
    })?;
    drop(temporary_file);

    if let Err(error) = verify_atomic_target(&parent, &target_name, expectation) {
        unsafe {
            libc::unlinkat(parent.as_raw_fd(), temporary_name.as_ptr(), 0);
        }
        return Err(error);
    }
    let renamed = unsafe {
        libc::renameat(
            parent.as_raw_fd(),
            temporary_name.as_ptr(),
            parent.as_raw_fd(),
            target_name.as_ptr(),
        )
    };
    if renamed != 0 {
        unsafe {
            libc::unlinkat(parent.as_raw_fd(), temporary_name.as_ptr(), 0);
        }
        return Err("cannot replace target file atomically".to_owned());
    }
    parent
        .sync_all()
        .map_err(|_| "cannot sync target directory".to_owned())?;
    Ok(())
}

#[cfg(not(unix))]
fn atomic_write_inner(
    path: &Path,
    bytes: &[u8],
    mode: u32,
    expectation: Option<Option<&str>>,
) -> InstallResult<()> {
    let parent = path
        .parent()
        .ok_or_else(|| "atomic file has no parent directory".to_owned())?;
    ensure_dir(parent, 0o700)?;
    check_no_symlink_components(parent, false)?;
    let current = fs::symlink_metadata(path).ok();
    match (expectation, current.as_ref()) {
        (Some(None), Some(_)) => return Err("atomic target appeared after preflight".to_owned()),
        (Some(Some(expected)), Some(metadata))
            if metadata.is_file() && sha256_file(path)? == expected => {}
        (Some(Some(_)), _) => {
            return Err("atomic target no longer matches its fingerprint".to_owned())
        }
        (_, Some(metadata)) if !metadata.is_file() => {
            return Err("atomic target is not a regular file".to_owned())
        }
        _ => {}
    }
    let sequence = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temporary = parent.join(format!(".atomic-{}-{sequence}", std::process::id()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    let mut file = options
        .open(&temporary)
        .map_err(|_| "cannot create atomic temporary file".to_owned())?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| "cannot write atomic temporary file".to_owned())?;
    if fs::rename(&temporary, path).is_err() {
        let _ = fs::remove_file(temporary);
        return Err("cannot replace target file atomically".to_owned());
    }
    let _ = mode;
    Ok(())
}

#[cfg(unix)]
fn verify_atomic_target(
    parent: &File,
    name: &std::ffi::CString,
    expectation: Option<Option<&str>>,
) -> InstallResult<()> {
    use std::os::fd::{AsRawFd, FromRawFd};
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    let current = if fd >= 0 {
        Some(unsafe { File::from_raw_fd(fd) })
    } else if std::io::Error::last_os_error().raw_os_error() == Some(libc::ENOENT) {
        None
    } else {
        return Err("atomic target is linked, unreadable or unsafe".to_owned());
    };
    if current
        .as_ref()
        .is_some_and(|file| !file.metadata().is_ok_and(|metadata| metadata.is_file()))
    {
        return Err("atomic target is not a regular file".to_owned());
    }
    match expectation {
        Some(None) if current.is_some() => Err("atomic target appeared after preflight".to_owned()),
        Some(Some(expected)) => {
            let Some(mut current) = current else {
                return Err("atomic target disappeared after preflight".to_owned());
            };
            if hash_open_file(&mut current)? == expected {
                Ok(())
            } else {
                Err("atomic target no longer matches its fingerprint".to_owned())
            }
        }
        Some(None) | None => Ok(()),
    }
}

#[cfg(unix)]
fn ensure_dir_mode(path: &Path, mode: u32, set_leaf_mode: bool) -> InstallResult<()> {
    let directory = secure_open_directory(path, true, mode)?;
    if set_leaf_mode {
        set_directory_fd_mode(&directory, mode)?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn ensure_dir_mode(path: &Path, _mode: u32, _set_leaf_mode: bool) -> InstallResult<()> {
    check_no_symlink_components(path, true)
}

#[cfg(unix)]
fn parent_and_name(path: &Path) -> InstallResult<(PathBuf, std::ffi::CString)> {
    use std::os::unix::ffi::OsStrExt;
    if !path.is_absolute() {
        return Err("managed path must be absolute".to_owned());
    }
    let parent = path
        .parent()
        .ok_or_else(|| "managed path has no parent".to_owned())?;
    let name = path
        .file_name()
        .ok_or_else(|| "managed path has no file name".to_owned())?;
    let name = std::ffi::CString::new(name.as_bytes())
        .map_err(|_| "managed path contains a null byte".to_owned())?;
    Ok((parent.to_path_buf(), name))
}

#[cfg(unix)]
fn secure_open_directory(
    path: &Path,
    create_missing: bool,
    create_mode: u32,
) -> InstallResult<File> {
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    if !path.is_absolute() {
        return Err("managed directory path must be absolute".to_owned());
    }
    let mut current = File::open("/").map_err(|_| "cannot open filesystem root".to_owned())?;
    for component in path.components() {
        let Component::Normal(part) = component else {
            match component {
                Component::RootDir | Component::CurDir => continue,
                _ => return Err("managed directory path contains traversal".to_owned()),
            }
        };
        let name = std::ffi::CString::new(part.as_bytes())
            .map_err(|_| "managed directory name contains a null byte".to_owned())?;
        let mut fd = open_child_directory_raw(&current, &name);
        let mut created = false;
        if fd < 0
            && create_missing
            && std::io::Error::last_os_error().raw_os_error() == Some(libc::ENOENT)
        {
            let result = unsafe {
                libc::mkdirat(
                    current.as_raw_fd(),
                    name.as_ptr(),
                    create_mode as libc::mode_t,
                )
            };
            if result == 0 {
                created = true;
            } else if std::io::Error::last_os_error().raw_os_error() != Some(libc::EEXIST) {
                return Err("cannot create managed directory".to_owned());
            }
            fd = open_child_directory_raw(&current, &name);
        }
        if fd < 0 {
            return Err("managed directory has a symlink or unsafe component".to_owned());
        }
        let child = unsafe { File::from_raw_fd(fd) };
        if created {
            set_directory_fd_mode(&child, create_mode)?;
        }
        current = child;
    }
    Ok(current)
}

#[cfg(unix)]
fn open_child_directory(parent: &File, name: &std::ffi::CString) -> InstallResult<File> {
    use std::os::fd::FromRawFd;
    let fd = open_child_directory_raw(parent, name);
    if fd < 0 {
        return Err("cannot open managed directory without following links".to_owned());
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}

#[cfg(unix)]
fn open_child_directory_raw(parent: &File, name: &std::ffi::CString) -> libc::c_int {
    use std::os::fd::AsRawFd;
    unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    }
}

#[cfg(unix)]
fn secure_parent_and_name(
    path: &Path,
    create_parent: bool,
) -> InstallResult<(File, std::ffi::CString)> {
    let (parent_path, name) = parent_and_name(path)?;
    let parent = secure_open_directory(&parent_path, create_parent, 0o700)?;
    Ok((parent, name))
}

#[cfg(unix)]
fn open_regular_under(root: &Path, path: &Path) -> InstallResult<File> {
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    let canonical_root =
        fs::canonicalize(root).map_err(|_| "source root does not exist".to_owned())?;
    let root_metadata =
        fs::metadata(&canonical_root).map_err(|_| "cannot inspect source root".to_owned())?;
    if root_metadata.is_file() {
        if fs::canonicalize(path).ok().as_deref() != Some(canonical_root.as_path()) {
            return Err("source file resolves outside its root".to_owned());
        }
        return open_regular_absolute(path);
    }
    if !root_metadata.is_dir() {
        return Err("source root is not a regular file or directory".to_owned());
    }
    let relative = path
        .strip_prefix(&canonical_root)
        .map_err(|_| "source file resolves outside its root".to_owned())?;
    let mut components = relative.components().peekable();
    let Some(Component::Normal(first)) = components.next() else {
        return Err("source file path is not below its root".to_owned());
    };
    let mut parent = secure_open_directory(&canonical_root, false, 0o700)?;
    let mut name = std::ffi::CString::new(first.as_bytes())
        .map_err(|_| "source file path contains a null byte".to_owned())?;
    while components.peek().is_some() {
        let fd = open_child_directory_raw(&parent, &name);
        if fd < 0 {
            return Err("source path contains a symlink or non-directory component".to_owned());
        }
        parent = unsafe { File::from_raw_fd(fd) };
        let Some(Component::Normal(part)) = components.next() else {
            return Err("source file path contains traversal".to_owned());
        };
        name = std::ffi::CString::new(part.as_bytes())
            .map_err(|_| "source file path contains a null byte".to_owned())?;
    }
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        return Err("cannot open source file without following links".to_owned());
    }
    let file = unsafe { File::from_raw_fd(fd) };
    if !file
        .metadata()
        .map_err(|_| "cannot inspect source file".to_owned())?
        .is_file()
    {
        return Err("selected source is not a regular file".to_owned());
    }
    Ok(file)
}

#[cfg(not(unix))]
fn open_regular_under(root: &Path, path: &Path) -> InstallResult<File> {
    let canonical_root =
        fs::canonicalize(root).map_err(|_| "source root does not exist".to_owned())?;
    let canonical_path =
        fs::canonicalize(path).map_err(|_| "source file does not exist".to_owned())?;
    if !canonical_path.starts_with(&canonical_root) {
        return Err("source file resolves outside its root".to_owned());
    }
    File::open(canonical_path).map_err(|_| "cannot read source file".to_owned())
}

#[cfg(unix)]
fn open_regular_absolute(path: &Path) -> InstallResult<File> {
    use std::os::fd::{AsRawFd, FromRawFd};
    let (parent, name) = secure_parent_and_name(path, false)?;
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        return Err("cannot open source file without following links".to_owned());
    }
    let file = unsafe { File::from_raw_fd(fd) };
    if !file
        .metadata()
        .map_err(|_| "cannot inspect source file".to_owned())?
        .is_file()
    {
        return Err("selected source is not a regular file".to_owned());
    }
    Ok(file)
}

#[cfg(unix)]
fn create_regular_new_nofollow(path: &Path, mode: u32) -> InstallResult<File> {
    use std::os::fd::{AsRawFd, FromRawFd};
    let (parent_path, name) = parent_and_name(path)?;
    let parent = secure_open_directory(&parent_path, false, 0o700)?;
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            mode as libc::mode_t,
        )
    };
    if fd < 0 {
        return Err("cannot create copied file".to_owned());
    }
    let file = unsafe { File::from_raw_fd(fd) };
    set_file_mode(&file, mode)?;
    Ok(file)
}

#[cfg(not(unix))]
fn create_regular_new_nofollow(path: &Path, _mode: u32) -> InstallResult<File> {
    let parent = path.parent().unwrap_or(Path::new("."));
    check_no_symlink_components(parent, false)?;
    OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| "cannot create copied file".to_owned())
}

#[cfg(unix)]
fn set_directory_mode_nofollow(path: &Path, mode: u32) -> InstallResult<()> {
    let directory = secure_open_directory(path, false, mode)?;
    set_directory_fd_mode(&directory, mode)
}

#[cfg(unix)]
fn set_directory_fd_mode(directory: &File, mode: u32) -> InstallResult<()> {
    use std::os::fd::AsRawFd;
    if unsafe { libc::fchmod(directory.as_raw_fd(), mode as libc::mode_t) } != 0 {
        return Err("cannot set private directory permissions".to_owned());
    }
    Ok(())
}

#[cfg(unix)]
fn set_file_mode(file: &File, mode: u32) -> InstallResult<()> {
    use std::os::fd::AsRawFd;
    if unsafe { libc::fchmod(file.as_raw_fd(), mode as libc::mode_t) } != 0 {
        return Err("cannot set private file permissions".to_owned());
    }
    Ok(())
}

#[cfg(not(unix))]
fn set_file_mode(_file: &File, _mode: u32) -> InstallResult<()> {
    Ok(())
}

fn hash_open_file(file: &mut File) -> InstallResult<String> {
    use std::io::{Seek, SeekFrom};
    file.seek(SeekFrom::Start(0))
        .map_err(|_| "cannot seek a file for hashing".to_owned())?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|_| "cannot read a file for hashing".to_owned())?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(hex_digest(&digest.finalize()))
}

#[cfg(unix)]
fn metadata_dev(metadata: &fs::Metadata) -> libc::dev_t {
    use std::os::unix::fs::MetadataExt;
    metadata.dev()
}

#[cfg(unix)]
fn metadata_ino(metadata: &fs::Metadata) -> libc::ino_t {
    use std::os::unix::fs::MetadataExt;
    metadata.ino()
}

#[cfg(not(unix))]
fn check_no_symlink_components(path: &Path, create_missing: bool) -> InstallResult<()> {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err("managed path contains a symbolic link".to_owned())
            }
            Ok(metadata) if !metadata.is_dir() => {
                return Err("managed path contains a non-directory component".to_owned())
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && create_missing => {
                fs::create_dir(&current)
                    .map_err(|_| "cannot create managed directory".to_owned())?;
            }
            Err(_) => return Err("cannot inspect managed path".to_owned()),
        }
    }
    Ok(())
}

pub fn safe_relative_path(path: &str) -> InstallResult<PathBuf> {
    let path = Path::new(path);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err("source selection must be a relative path without traversal".to_owned());
    }
    Ok(path.to_path_buf())
}

fn hash_field(digest: &mut Sha256, bytes: &[u8]) {
    digest.update((bytes.len() as u64).to_be_bytes());
    digest.update(bytes);
}

fn hex_digest(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

#[cfg(unix)]
fn path_bytes(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes().to_vec()
}

#[cfg(not(unix))]
fn path_bytes(path: &Path) -> Vec<u8> {
    path.to_string_lossy().as_bytes().to_vec()
}
