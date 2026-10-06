use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use yashik::install::util::{hash_tree, hash_tree_streaming_bounded};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct TestTree(PathBuf);

impl TestTree {
    fn new() -> Self {
        // Keep fixtures beside the test executable. Some temporary filesystems
        // reject writes while another process has an executable open (ETXTBSY).
        let executable = std::env::current_exe().expect("resolve current test executable");
        let parent = executable.parent().expect("test executable has a parent");
        let path = parent.join(format!(
            "yashik-orca-tree-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("create tree fixture");
        Self(fs::canonicalize(path).expect("canonicalize tree fixture"))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestTree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn bounded_hash(
    root: &Path,
    entries: usize,
    bytes: u64,
    depth: usize,
) -> Result<(String, u64), String> {
    hash_tree_streaming_bounded(root, entries, bytes, depth)
}

#[cfg(unix)]
#[test]
fn bounded_streaming_hash_matches_legacy_tree_hash_for_files_modes_and_relative_symlinks() {
    use std::os::unix::fs::{symlink, PermissionsExt};

    let tree = TestTree::new();
    let root = tree.path().join("root");
    fs::create_dir_all(root.join("nested")).unwrap();
    fs::write(root.join("readme.txt"), b"hello tree\n").unwrap();
    fs::write(root.join("nested/data.bin"), [0, 1, 2, 3, 254, 255]).unwrap();
    fs::set_permissions(root.join("readme.txt"), fs::Permissions::from_mode(0o640)).unwrap();
    fs::set_permissions(
        root.join("nested/data.bin"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    symlink("nested/data.bin", root.join("data-link")).unwrap();

    let expected = hash_tree(&root).unwrap();
    let (actual, total_bytes) = bounded_hash(&root, 8, 1024, 4).unwrap();

    assert_eq!(actual, expected);
    assert_eq!(total_bytes, b"hello tree\n".len() as u64 + 6);
}

#[test]
fn bounded_streaming_hash_rejects_flat_entry_limit() {
    let tree = TestTree::new();
    let root = tree.path().join("root");
    fs::create_dir(&root).unwrap();
    for name in ["one", "two", "three"] {
        fs::write(root.join(name), name).unwrap();
    }

    let error = bounded_hash(&root, 2, 1024, 4).unwrap_err();
    assert!(error.contains("entry limit"), "unexpected error: {error}");
}

#[test]
fn bounded_streaming_hash_rejects_total_file_bytes_limit() {
    let tree = TestTree::new();
    let root = tree.path().join("root");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("payload"), b"four").unwrap();

    let error = bounded_hash(&root, 8, 3, 4).unwrap_err();
    assert!(
        error.contains("expanded size limit"),
        "unexpected error: {error}"
    );
}

#[test]
fn bounded_streaming_hash_rejects_depth_limit() {
    let tree = TestTree::new();
    let root = tree.path().join("root");
    fs::create_dir_all(root.join("one/two")).unwrap();
    fs::write(root.join("one/two/payload"), b"x").unwrap();

    let error = bounded_hash(&root, 8, 1024, 2).unwrap_err();
    assert!(error.contains("depth limit"), "unexpected error: {error}");
}

#[cfg(unix)]
#[test]
fn bounded_streaming_hash_rejects_absolute_dangling_and_outside_symlinks() {
    use std::os::unix::fs::symlink;

    let tree = TestTree::new();
    let outside = tree.path().join("outside.txt");
    fs::write(&outside, b"outside").unwrap();

    let absolute_root = tree.path().join("absolute-root");
    fs::create_dir(&absolute_root).unwrap();
    symlink("/etc/passwd", absolute_root.join("link")).unwrap();
    let error = bounded_hash(&absolute_root, 8, 1024, 4).unwrap_err();
    assert!(
        error.contains("absolute symbolic link"),
        "unexpected error: {error}"
    );

    let dangling_root = tree.path().join("dangling-root");
    fs::create_dir(&dangling_root).unwrap();
    symlink("missing-target", dangling_root.join("link")).unwrap();
    let error = bounded_hash(&dangling_root, 8, 1024, 4).unwrap_err();
    assert!(
        error.contains("dangling or cyclic"),
        "unexpected error: {error}"
    );

    let outside_root = tree.path().join("outside-root");
    fs::create_dir(&outside_root).unwrap();
    symlink("../outside.txt", outside_root.join("link")).unwrap();
    let error = bounded_hash(&outside_root, 8, 1024, 4).unwrap_err();
    assert!(
        error.contains("outside its root"),
        "unexpected error: {error}"
    );
}

#[test]
fn bounded_streaming_hash_changes_when_file_contents_drift() {
    let tree = TestTree::new();
    let root = tree.path().join("root");
    fs::create_dir(&root).unwrap();
    let file = root.join("payload");
    fs::write(&file, b"first").unwrap();
    let first = bounded_hash(&root, 8, 1024, 4).unwrap().0;

    fs::write(&file, b"other").unwrap();
    let second = bounded_hash(&root, 8, 1024, 4).unwrap().0;

    assert_ne!(first, second);
}
