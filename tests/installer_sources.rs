use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use yashik::install::api::Paths;
use yashik::install::state::ArtifactRecord;
use yashik::install::{sources, util};
use yashik::schema::Source;

static NEXT: AtomicU64 = AtomicU64::new(0);

struct TempTree(PathBuf);

impl TempTree {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "ys-src-{}-{}",
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
fn local_directory_is_snapshotted_and_content_changes_get_a_new_key() {
    let temp = TempTree::new();
    let paths = temp.paths();
    let source_root = temp.path().join("repository");
    fs::create_dir_all(source_root.join("rules")).unwrap();
    fs::write(source_root.join("rules/first.md"), "first version").unwrap();
    let source = Source::Local {
        path: "./repository".to_owned(),
    };

    let first = sources::resolve(&paths, &source, Some(&source_root)).unwrap();
    let selected = sources::select(&first, Some("rules/first.md")).unwrap();
    assert_eq!(fs::read(&selected).unwrap(), b"first version");
    let first_key = first.key.clone();
    let first_snapshot = first.root.clone();

    fs::write(source_root.join("rules/first.md"), "second version").unwrap();
    let second = sources::resolve(&paths, &source, Some(&source_root)).unwrap();
    assert_ne!(second.key, first_key);
    assert_eq!(
        fs::read(first_snapshot.join("rules/first.md")).unwrap(),
        b"first version"
    );
    assert_eq!(
        fs::read(second.root.join("rules/first.md")).unwrap(),
        b"second version"
    );

    let artifact = sources::materialize_artifact(&paths, &second, "native-rule").unwrap();
    assert_eq!(
        fs::read(artifact.join("rules/first.md")).unwrap(),
        b"second version"
    );
}

#[test]
fn local_file_remains_selectable_and_artifact_keeps_its_filename() {
    let temp = TempTree::new();
    let paths = temp.paths();
    let local_file = temp.path().join("instructions/backend.toml");
    fs::create_dir_all(local_file.parent().unwrap()).unwrap();
    fs::write(&local_file, b"name = \"backend\"\n").unwrap();
    let source = Source::Local {
        path: "instructions/backend.toml".to_owned(),
    };

    let resolved = sources::resolve(&paths, &source, Some(&local_file)).unwrap();
    assert!(resolved.root.is_file());
    assert_eq!(resolved.root.file_name().unwrap(), "backend.toml");
    assert_eq!(sources::select(&resolved, None).unwrap(), resolved.root);
    let artifact = sources::materialize_artifact(&paths, &resolved, "backend-agent").unwrap();
    assert_eq!(
        fs::read(artifact.join("backend.toml")).unwrap(),
        b"name = \"backend\"\n"
    );
}

#[test]
fn source_selection_rejects_traversal_and_symlink_escape() {
    let temp = TempTree::new();
    let paths = temp.paths();
    let source_root = temp.path().join("repository");
    fs::create_dir_all(&source_root).unwrap();
    fs::write(source_root.join("inside.txt"), "inside").unwrap();
    let outside = temp.path().join("outside.txt");
    fs::write(&outside, "sentinel").unwrap();

    let resolved = sources::resolve(
        &paths,
        &Source::Local {
            path: "repository".to_owned(),
        },
        Some(&source_root),
    )
    .unwrap();
    assert!(sources::select(&resolved, Some("../outside.txt")).is_err());
    assert_eq!(fs::read(outside).unwrap(), b"sentinel");
}

#[cfg(unix)]
#[test]
fn local_snapshot_rejects_dangling_and_external_symlinks_and_special_files() {
    use std::os::unix::net::UnixListener;

    let temp = TempTree::new();
    let paths = temp.paths();
    let outside = temp.path().join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("sentinel"), "untouched").unwrap();

    let external_tree = temp.path().join("external-link-tree");
    fs::create_dir_all(&external_tree).unwrap();
    std::os::unix::fs::symlink(&outside, external_tree.join("escape")).unwrap();
    assert!(sources::resolve(
        &paths,
        &Source::Local {
            path: "external-link-tree".to_owned()
        },
        Some(&external_tree),
    )
    .is_err());
    assert_eq!(fs::read(outside.join("sentinel")).unwrap(), b"untouched");

    let dangling_tree = temp.path().join("dangling-link-tree");
    fs::create_dir_all(&dangling_tree).unwrap();
    std::os::unix::fs::symlink("missing", dangling_tree.join("dangling")).unwrap();
    assert!(sources::resolve(
        &paths,
        &Source::Local {
            path: "dangling-link-tree".to_owned()
        },
        Some(&dangling_tree),
    )
    .is_err());

    let special_tree = temp.path().join("special");
    fs::create_dir_all(&special_tree).unwrap();
    let _socket = UnixListener::bind(special_tree.join("socket")).unwrap();
    assert!(sources::resolve(
        &paths,
        &Source::Local {
            path: "special".to_owned()
        },
        Some(&special_tree),
    )
    .is_err());
}

#[test]
fn git_credentials_and_unsafe_protocols_are_rejected_before_git_runs() {
    let temp = TempTree::new();
    let paths = temp.paths();
    for url in [
        "https://user:secret@example.invalid/team/repo.git",
        "file:///tmp/repo.git",
        "ext::sh -c echo% hello",
    ] {
        assert!(sources::validate_source_url(url).is_err());
        let error = sources::resolve(
            &paths,
            &Source::Git {
                url: url.to_owned(),
                git_ref: None,
            },
            None,
        )
        .unwrap_err();
        assert!(!error.is_empty());
    }
    assert!(!paths.cache.join("git").exists());
    assert!(sources::validate_source_ref("--upload-pack=cmd").is_err());
    assert!(!paths.cache.exists());
}

#[test]
fn file_hash_and_tree_hash_are_stable_for_copied_payloads() {
    let temp = TempTree::new();
    let original = temp.path().join("original");
    let copy = temp.path().join("copy");
    fs::create_dir_all(&original).unwrap();
    fs::write(original.join("payload"), b"same bytes").unwrap();
    let original_hash = util::hash_tree(&original).unwrap();
    util::copy_tree_checked(&original, &copy).unwrap();
    assert_eq!(util::hash_tree(&copy).unwrap(), original_hash);
}

#[test]
fn artifact_removal_is_limited_to_the_recorded_deterministic_path() {
    let temp = TempTree::new();
    let paths = temp.paths();
    let source_root = temp.path().join("source");
    fs::create_dir_all(&source_root).unwrap();
    fs::write(source_root.join("payload"), "owned artifact").unwrap();
    let source = sources::resolve(
        &paths,
        &Source::Local {
            path: "source".to_owned(),
        },
        Some(&source_root),
    )
    .unwrap();
    let key = "artifact-to-remove";
    let artifact = sources::materialize_artifact(&paths, &source, key).unwrap();
    let record = ArtifactRecord {
        key: key.to_owned(),
        source_key: source.key,
        root: artifact.clone(),
    };
    sources::remove_artifact(&paths, &record).unwrap();
    assert!(!artifact.exists());

    let sentinel = temp.path().join("outside/sentinel");
    fs::create_dir_all(sentinel.parent().unwrap()).unwrap();
    fs::write(&sentinel, "leave me").unwrap();
    let forged = ArtifactRecord {
        key: "another-key".to_owned(),
        source_key: "some-source".to_owned(),
        root: sentinel.parent().unwrap().to_path_buf(),
    };
    assert!(sources::remove_artifact(&paths, &forged).is_err());
    assert_eq!(fs::read(sentinel).unwrap(), b"leave me");
}

#[cfg(unix)]
#[test]
fn private_directory_and_atomic_write_refuse_symlinked_parent_paths() {
    use std::os::unix::fs::PermissionsExt;

    let temp = TempTree::new();
    let outside = temp.path().join("outside");
    fs::create_dir_all(&outside).unwrap();
    let sentinel = outside.join("sentinel");
    fs::write(&sentinel, "do not modify").unwrap();
    fs::set_permissions(&sentinel, fs::Permissions::from_mode(0o640)).unwrap();
    let link = temp.path().join("linked-parent");
    std::os::unix::fs::symlink(&outside, &link).unwrap();

    assert!(util::private_dir(&link.join("created")).is_err());
    assert!(util::create_private_dir_new(&link.join("also-created")).is_err());
    assert!(util::atomic_write(&link.join("sentinel"), b"overwritten", 0o600).is_err());
    assert!(!outside.join("created").exists());
    assert!(!outside.join("also-created").exists());
    assert_eq!(fs::read(&sentinel).unwrap(), b"do not modify");
    assert_eq!(
        fs::metadata(&sentinel).unwrap().permissions().mode() & 0o777,
        0o640
    );
}

#[cfg(unix)]
#[test]
fn checked_tree_removal_unlinks_internal_symlinks_without_following_them() {
    let temp = TempTree::new();
    let root = temp.path().join("owned-tree");
    let outside = temp.path().join("outside");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&outside).unwrap();
    let sentinel = outside.join("sentinel");
    fs::write(&sentinel, "untouched").unwrap();
    std::os::unix::fs::symlink(&outside, root.join("external-link")).unwrap();
    util::remove_tree_checked(&root).unwrap();
    assert!(!root.exists());
    assert_eq!(fs::read(sentinel).unwrap(), b"untouched");
}
