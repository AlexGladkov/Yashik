use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use fs2::FileExt;

use super::api::{InstallResult, Paths, ResolvedSource};
use super::state::ArtifactRecord;
use super::util::{
    atomic_write, copy_file_checked, copy_tree_checked, hash_tree, private_dir,
    read_file_checked_bounded, remove_file_checked, remove_tree_checked, safe_relative_path,
    sha256_bytes, sha256_file,
};
use crate::schema::Source;

static STAGE_COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn resolve(
    paths: &Paths,
    source: &Source,
    local_source_path: Option<&Path>,
) -> InstallResult<ResolvedSource> {
    match source {
        Source::Local { path } => resolve_local(paths, path, local_source_path),
        Source::Git { url, git_ref } => resolve_git(paths, url, git_ref.as_deref()),
    }
}

/// Validate a remote URL without creating caches or starting Git.
pub fn validate_source_url(input: &str) -> InstallResult<()> {
    validate_git_url(input).map(|_| ())
}

/// Validate a branch, tag, or full commit without creating caches or starting Git.
pub fn validate_source_ref(input: &str) -> InstallResult<()> {
    validate_git_ref(input)
}

pub fn select(source: &ResolvedSource, from: Option<&str>) -> InstallResult<PathBuf> {
    let root = fs::canonicalize(&source.root)
        .map_err(|_| "resolved source snapshot is missing".to_owned())?;
    if root != source.root {
        return Err("resolved source root changed after canonicalization".to_owned());
    }
    let Some(from) = from else {
        return Ok(root);
    };
    if !fs::metadata(&root)
        .map_err(|_| "cannot inspect resolved source root".to_owned())?
        .is_dir()
    {
        return Err("from can only select a path inside a source directory".to_owned());
    }
    let relative = safe_relative_path(from)?;
    let selected = root.join(relative);
    let canonical = fs::canonicalize(&selected)
        .map_err(|_| "selected source path does not exist".to_owned())?;
    if !canonical.starts_with(&root) {
        return Err("selected source path resolves outside its root".to_owned());
    }
    Ok(canonical)
}

pub fn materialize_artifact(
    paths: &Paths,
    source: &ResolvedSource,
    artifact_key: &str,
) -> InstallResult<PathBuf> {
    if artifact_key.is_empty() {
        return Err("artifact key is empty".to_owned());
    }
    private_dir(&paths.data)?;
    let digest = sha256_bytes(artifact_key.as_bytes());
    let artifacts = paths.data.join("artifacts");
    let indexes = paths.data.join("artifact-index");
    private_dir(&artifacts)?;
    private_dir(&indexes)?;
    let target = artifacts.join(&digest);
    let index = indexes.join(format!("{digest}.source"));
    let expected = format!("{}\n{}\n", source.key, source.revision);

    let _lock = key_lock(&paths.cache.join("locks"), &format!("artifact-{digest}"))?;
    if target.is_dir() {
        let stored = fs::read_to_string(&index).ok();
        if stored.as_deref() == Some(expected.as_str()) {
            return Ok(target);
        }
        if stored.is_none() {
            // An unindexed final directory is left by an interrupted materialization.
            fs::remove_dir_all(&target)
                .map_err(|_| "cannot clean incomplete artifact".to_owned())?;
        } else {
            return Err("artifact key is already associated with another source".to_owned());
        }
    }

    let stage = unique_stage(&artifacts, &digest)?;
    let copied = if source.root.is_file() {
        private_dir(&stage)?;
        let file_name = source
            .root
            .file_name()
            .ok_or_else(|| "local source file has no file name".to_owned())?;
        copy_file_checked(&source.root, &source.root, &stage.join(file_name)).map(|_| ())
    } else {
        copy_tree_checked(&source.root, &stage).map(|_| ())
    };
    if let Err(error) = copied {
        let _ = fs::remove_dir_all(&stage);
        return Err(error);
    }
    fs::rename(&stage, &target).map_err(|_| {
        let _ = fs::remove_dir_all(&stage);
        "cannot activate source artifact".to_owned()
    })?;
    atomic_write(&index, expected.as_bytes(), 0o600)?;
    Ok(target)
}

/// Remove a previously recorded artifact only from its deterministic managed location.
pub fn remove_artifact(paths: &Paths, record: &ArtifactRecord) -> InstallResult<()> {
    if record.key.is_empty() || record.source_key.is_empty() {
        return Err("artifact record is missing its ownership key".to_owned());
    }
    let digest = sha256_bytes(record.key.as_bytes());
    let artifacts = paths.data.join("artifacts");
    let expected_root = artifacts.join(&digest);
    if record.root != expected_root {
        return Err("artifact record points outside its deterministic managed path".to_owned());
    }
    let index = paths
        .data
        .join("artifact-index")
        .join(format!("{digest}.source"));
    if let Ok(metadata) = fs::symlink_metadata(&index) {
        if !metadata.is_file() {
            return Err("artifact ownership record is not a regular file".to_owned());
        }
        let bytes = read_file_checked_bounded(&paths.data.join("artifact-index"), &index, 4096)?;
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| "artifact ownership record is invalid".to_owned())?;
        let mut fields = text.lines();
        if fields.next() != Some(record.source_key.as_str())
            || fields.next().is_none_or(str::is_empty)
            || fields.next().is_some()
        {
            return Err("artifact ownership record does not match state".to_owned());
        }
        remove_tree_checked(&expected_root)?;
        remove_file_checked(&index, Some(&sha256_bytes(&bytes)))?;
        return Ok(());
    }
    remove_tree_checked(&expected_root)
}

fn resolve_local(
    paths: &Paths,
    input: &str,
    local_source_path: Option<&Path>,
) -> InstallResult<ResolvedSource> {
    let source_path = PathBuf::from(input);
    let requested = match local_source_path {
        Some(resolved) => resolved.to_path_buf(),
        None if source_path.is_absolute() => source_path,
        None => {
            return Err("relative local source path was not resolved from the manifest".to_owned())
        }
    };
    if !requested.is_absolute() {
        return Err("effective local source path must be absolute".to_owned());
    }
    let root =
        fs::canonicalize(requested).map_err(|_| "local source path does not exist".to_owned())?;
    let root_metadata =
        fs::metadata(&root).map_err(|_| "cannot inspect local source".to_owned())?;
    let content_digest = if root_metadata.is_dir() {
        format!("directory:{}", hash_tree(&root)?)
    } else if root_metadata.is_file() {
        #[cfg(unix)]
        let mode = {
            use std::os::unix::fs::PermissionsExt;
            root_metadata.permissions().mode() & 0o7777
        };
        #[cfg(not(unix))]
        let mode = 0u32;
        format!("file:{mode:o}:{}", sha256_file(&root)?)
    } else {
        return Err("local source must be a regular file or directory".to_owned());
    };
    let mut identity = b"local\0".to_vec();
    identity.extend_from_slice(path_identity(&root).as_bytes());
    identity.push(0);
    identity.extend_from_slice(content_digest.as_bytes());
    let key = format!("local-{}", sha256_bytes(&identity));
    let revision = content_digest.clone();
    private_dir(&paths.cache.join("sources"))?;
    private_dir(&paths.cache.join("locks"))?;
    let _lock = key_lock(&paths.cache.join("locks"), &key)?;
    let source_dir = paths.cache.join("sources").join(&key);
    private_dir(&source_dir)?;
    let root_snapshot = if root_metadata.is_file() {
        source_dir.join(
            root.file_name()
                .ok_or_else(|| "local source file has no file name".to_owned())?,
        )
    } else {
        source_dir.join("root")
    };
    let marker = source_dir.join("metadata");
    let tree_marker = source_dir.join("tree-sha256");
    if let Ok(cached_metadata) = fs::symlink_metadata(&root_snapshot) {
        if cached_metadata.file_type().is_symlink() {
            return Err("cached local source snapshot is a symbolic link".to_owned());
        }
        if fs::canonicalize(&root_snapshot).ok().as_deref() != Some(root_snapshot.as_path()) {
            return Err("cached local source snapshot changed after canonicalization".to_owned());
        }
        let stored_revision = fs::read_to_string(&marker).ok();
        let stored_digest = fs::read_to_string(&tree_marker).ok();
        let snapshot_digest = if root_metadata.is_dir() {
            hash_tree(&root_snapshot)
                .ok()
                .map(|hash| format!("directory:{hash}"))
        } else {
            sha256_file(&root_snapshot).ok().and_then(|hash| {
                let metadata = fs::metadata(&root_snapshot).ok()?;
                #[cfg(unix)]
                let mode = {
                    use std::os::unix::fs::PermissionsExt;
                    metadata.permissions().mode() & 0o7777
                };
                #[cfg(not(unix))]
                let mode = 0u32;
                Some(format!("file:{mode:o}:{hash}"))
            })
        };
        if stored_revision.as_deref() == Some(revision.as_str())
            && stored_digest.as_deref() == Some(content_digest.as_str())
            && snapshot_digest.as_deref() == Some(content_digest.as_str())
        {
            return Ok(ResolvedSource {
                key,
                revision,
                root: root_snapshot,
            });
        }
        return Err("cached local source snapshot failed its integrity check".to_owned());
    }
    let stage = unique_stage(&source_dir, &key)?;
    let copied_digest = if root_metadata.is_dir() {
        copy_tree_checked(&root, &stage).map(|hash| format!("directory:{hash}"))
    } else {
        copy_file_checked(&root, &root, &stage).map(|hash| {
            let mode = {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    fs::metadata(&stage)
                        .map(|metadata| metadata.permissions().mode() & 0o7777)
                        .unwrap_or_default()
                }
                #[cfg(not(unix))]
                {
                    0u32
                }
            };
            format!("file:{mode:o}:{hash}")
        })
    };
    match copied_digest {
        Ok(copied_digest) if copied_digest == content_digest => {
            atomic_write(&marker, revision.as_bytes(), 0o600)?;
            atomic_write(&tree_marker, content_digest.as_bytes(), 0o600)?;
            activate_snapshot(&stage, &root_snapshot)?;
        }
        Ok(_) => {
            let _ = fs::remove_dir_all(&stage);
            return Err("local source changed while it was being snapshotted".to_owned());
        }
        Err(error) => {
            let _ = fs::remove_dir_all(&stage);
            return Err(error);
        }
    }
    Ok(ResolvedSource {
        key,
        revision,
        root: root_snapshot,
    })
}

fn resolve_git(
    paths: &Paths,
    input_url: &str,
    requested_ref: Option<&str>,
) -> InstallResult<ResolvedSource> {
    let url = validate_git_url(input_url)?;
    if let Some(git_ref) = requested_ref {
        validate_git_ref(git_ref)?;
    }
    private_dir(&paths.cache.join("git"))?;
    private_dir(&paths.cache.join("sources"))?;
    private_dir(&paths.cache.join("locks"))?;
    let remote_id = sha256_bytes(url.as_bytes());
    let bare = paths.cache.join("git").join(format!("{remote_id}.git"));
    let _lock = key_lock(&paths.cache.join("locks"), &format!("git-{remote_id}"))?;

    if !bare.exists() {
        let output = git_command(
            paths,
            &[
                "init".to_owned(),
                "--bare".to_owned(),
                bare.to_string_lossy().into_owned(),
            ],
            None,
        )?;
        if !output.status.success() {
            return Err("cannot initialize private Git cache".to_owned());
        }
    }

    let resolved_ref = if let Some(git_ref) = requested_ref {
        resolve_requested_ref(paths, &bare, &url, git_ref)?
    } else {
        resolve_default_head(paths, &url)?
    };
    let full_commit = fetch_commit(paths, &bare, &url, &resolved_ref, requested_ref)?;

    let mut identity = b"git\0".to_vec();
    identity.extend_from_slice(url.as_bytes());
    identity.push(0);
    identity.extend_from_slice(full_commit.as_bytes());
    let key = format!("git-{}", sha256_bytes(&identity));
    let revision = full_commit.clone();
    let final_root = paths.cache.join("sources").join(&key).join("root");
    let marker = paths.cache.join("sources").join(&key).join("metadata");
    if final_root.is_dir() && fs::read_to_string(&marker).ok().as_deref() == Some(revision.as_str())
    {
        let tree_hash = hash_tree(&final_root)?;
        let digest_marker = paths.cache.join("sources").join(&key).join("tree-sha256");
        if fs::read_to_string(&digest_marker).ok().as_deref() == Some(tree_hash.as_str()) {
            return Ok(ResolvedSource {
                key,
                revision,
                root: final_root,
            });
        }
        return Err("cached Git source snapshot failed its integrity check".to_owned());
    }

    private_dir(
        final_root
            .parent()
            .ok_or_else(|| "invalid Git cache path".to_owned())?,
    )?;
    let stage = unique_stage(final_root.parent().unwrap(), &key)?;
    fs::create_dir(&stage)
        .map_err(|_| "cannot create Git snapshot staging directory".to_owned())?;
    match export_commit(paths, &bare, &full_commit, &stage) {
        Ok(()) => {}
        Err(error) => {
            let _ = fs::remove_dir_all(&stage);
            return Err(error);
        }
    }
    let tree_hash = match hash_tree(&stage) {
        Ok(hash) => hash,
        Err(error) => {
            let _ = fs::remove_dir_all(&stage);
            return Err(error);
        }
    };
    atomic_write(&marker, revision.as_bytes(), 0o600)?;
    atomic_write(
        &paths.cache.join("sources").join(&key).join("tree-sha256"),
        tree_hash.as_bytes(),
        0o600,
    )?;
    fs::rename(&stage, &final_root).map_err(|_| {
        let _ = fs::remove_dir_all(&stage);
        "cannot activate Git source snapshot".to_owned()
    })?;
    Ok(ResolvedSource {
        key,
        revision,
        root: final_root,
    })
}

fn resolve_default_head(paths: &Paths, url: &str) -> InstallResult<String> {
    let output = git_command(
        paths,
        &[
            "ls-remote".into(),
            "--symref".into(),
            url.into(),
            "HEAD".into(),
        ],
        None,
    )?;
    if !output.status.success() {
        return Err("cannot resolve the Git remote default branch".to_owned());
    }
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("ref: ") {
            if let Some((name, head)) = value.split_once('\t') {
                if head == "HEAD"
                    && (name.starts_with("refs/heads/") || name.starts_with("refs/tags/"))
                {
                    return Ok(name.to_owned());
                }
            }
        }
    }
    if text.lines().any(|line| line.ends_with("\tHEAD")) {
        Ok("HEAD".to_owned())
    } else {
        Err("Git remote does not advertise a default branch".to_owned())
    }
}

fn resolve_requested_ref(
    paths: &Paths,
    bare: &Path,
    url: &str,
    requested: &str,
) -> InstallResult<String> {
    if is_full_commit(requested) {
        let exists = git_command(
            paths,
            &[
                format!("--git-dir={}", bare.to_string_lossy()),
                "cat-file".into(),
                "-e".into(),
                format!("{requested}^{{commit}}"),
            ],
            None,
        )?;
        if exists.status.success() {
            return Ok(requested.to_owned());
        }
        return Ok(requested.to_owned());
    }
    let candidate = if requested.starts_with("refs/heads/") || requested.starts_with("refs/tags/") {
        vec![requested.to_owned()]
    } else {
        vec![
            format!("refs/heads/{requested}"),
            format!("refs/tags/{requested}"),
        ]
    };
    let mut matches = Vec::new();
    for candidate in candidate {
        let output = git_command(
            paths,
            &["ls-remote".into(), url.into(), candidate.clone()],
            None,
        )?;
        if !output.status.success() {
            return Err("cannot resolve the requested Git reference".to_owned());
        }
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            if let Some((object, name)) = line.split_once('\t') {
                if name == candidate && is_full_commit(object) {
                    matches.push(candidate.clone());
                    break;
                }
            }
        }
    }
    matches.sort();
    matches.dedup();
    match matches.as_slice() {
        [only] => Ok(only.clone()),
        [] => Err("requested Git reference was not found".to_owned()),
        _ => Err("requested Git reference is ambiguous; use refs/heads or refs/tags".to_owned()),
    }
}

fn fetch_commit(
    paths: &Paths,
    bare: &Path,
    url: &str,
    reference: &str,
    requested_ref: Option<&str>,
) -> InstallResult<String> {
    if let Some(commit) = requested_ref.filter(|value| is_full_commit(value)) {
        let exists = git_command(
            paths,
            &[
                format!("--git-dir={}", bare.to_string_lossy()),
                "cat-file".into(),
                "-e".into(),
                format!("{commit}^{{commit}}"),
            ],
            None,
        )?;
        if exists.status.success() {
            let resolved = git_command(
                paths,
                &[
                    format!("--git-dir={}", bare.to_string_lossy()),
                    "rev-parse".into(),
                    "--verify".into(),
                    format!("{commit}^{{commit}}"),
                ],
                None,
            )?;
            if !resolved.status.success() {
                return Err("requested Git reference does not identify a commit".to_owned());
            }
            let value = String::from_utf8_lossy(&resolved.stdout)
                .trim()
                .to_ascii_lowercase();
            if value != commit.to_ascii_lowercase() {
                return Err("requested Git commit is not a commit object".to_owned());
            }
            return Ok(value);
        }
    }
    let fetch = git_command(
        paths,
        &[
            format!("--git-dir={}", bare.to_string_lossy()),
            "fetch".into(),
            "--no-tags".into(),
            "--force".into(),
            url.into(),
            reference.into(),
        ],
        None,
    )?;
    if !fetch.status.success() {
        return Err("cannot fetch the requested Git source".to_owned());
    }
    let output = git_command(
        paths,
        &[
            format!("--git-dir={}", bare.to_string_lossy()),
            "rev-parse".into(),
            "--verify".into(),
            "FETCH_HEAD^{commit}".into(),
        ],
        None,
    )?;
    if !output.status.success() {
        return Err("requested Git reference does not identify a commit".to_owned());
    }
    let commit = String::from_utf8_lossy(&output.stdout)
        .trim()
        .to_ascii_lowercase();
    if !is_full_commit(&commit)
        || requested_ref.is_some_and(is_full_commit)
            && requested_ref.map(str::to_ascii_lowercase).as_deref() != Some(commit.as_str())
    {
        return Err("requested Git commit does not match the resolved commit".to_owned());
    }
    Ok(commit)
}

fn export_commit(paths: &Paths, bare: &Path, commit: &str, stage: &Path) -> InstallResult<()> {
    let output = git_command(
        paths,
        &[
            format!("--git-dir={}", bare.to_string_lossy()),
            "ls-tree".into(),
            "-r".into(),
            "-z".into(),
            "--full-tree".into(),
            commit.into(),
        ],
        None,
    )?;
    if !output.status.success() {
        return Err("cannot inspect Git source tree".to_owned());
    }
    let mut files = Vec::new();
    let mut links = Vec::new();
    for record in output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
    {
        let separator = record
            .iter()
            .position(|byte| *byte == b'\t')
            .ok_or_else(|| "Git source tree has an invalid entry".to_owned())?;
        let (header, tail) = record.split_at(separator);
        let raw_path = &tail[1..];
        let header = std::str::from_utf8(header)
            .map_err(|_| "Git source tree has an invalid entry".to_owned())?;
        let fields = header.split_ascii_whitespace().collect::<Vec<_>>();
        if fields.len() != 3 {
            return Err("Git source tree has an invalid entry".to_owned());
        }
        let mode = fields[0];
        let kind = fields[1];
        let object = fields[2];
        let path = os_path_from_bytes(raw_path)?;
        validate_git_path(&path)?;
        if kind == "commit" || mode == "160000" {
            return Err("Git sources with submodules are unsupported".to_owned());
        }
        if kind != "blob" || !is_full_commit(object) {
            return Err("Git source contains an unsupported entry".to_owned());
        }
        if mode == "100644" || mode == "100755" {
            files.push((path, object.to_owned(), mode == "100755"));
        } else if mode == "120000" {
            links.push((path, object.to_owned()));
        } else {
            return Err("Git source contains a special file".to_owned());
        }
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));
    links.sort_by(|left, right| left.0.cmp(&right.0));
    for (path, object, executable) in files {
        let content = git_blob(paths, bare, &object)?;
        let destination = stage.join(path);
        create_parent_directories(stage, &destination)?;
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(if executable { 0o755 } else { 0o644 });
        }
        let mut file = options
            .open(&destination)
            .map_err(|_| "cannot write Git source file".to_owned())?;
        file.write_all(&content)
            .map_err(|_| "cannot write Git source file".to_owned())?;
        file.sync_all()
            .map_err(|_| "cannot sync Git source file".to_owned())?;
    }
    for (path, object) in links {
        let target = git_blob(paths, bare, &object)?;
        if target.contains(&0) {
            return Err("Git source contains an invalid symbolic link".to_owned());
        }
        let target = os_path_from_bytes(&target)?;
        if target.is_absolute() {
            return Err("Git source contains an absolute symbolic link".to_owned());
        }
        let destination = stage.join(path);
        create_parent_directories(stage, &destination)?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &destination)
            .map_err(|_| "cannot create Git source symbolic link".to_owned())?;
        #[cfg(not(unix))]
        return Err("Git source symbolic links are unsupported on this platform".to_owned());
    }
    Ok(())
}

fn git_blob(paths: &Paths, bare: &Path, object: &str) -> InstallResult<Vec<u8>> {
    let output = git_command(
        paths,
        &[
            format!("--git-dir={}", bare.to_string_lossy()),
            "cat-file".into(),
            "blob".into(),
            object.into(),
        ],
        None,
    )?;
    if !output.status.success() {
        return Err("cannot read a Git source object".to_owned());
    }
    Ok(output.stdout)
}

fn create_parent_directories(root: &Path, child: &Path) -> InstallResult<()> {
    let parent = child
        .parent()
        .ok_or_else(|| "Git source path has no parent".to_owned())?;
    let relative = parent
        .strip_prefix(root)
        .map_err(|_| "Git source path escapes its root".to_owned())?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        match component {
            Component::Normal(part) => {
                current.push(part);
                if !current.exists() {
                    fs::create_dir(&current)
                        .map_err(|_| "cannot create Git source directory".to_owned())?;
                } else if !fs::symlink_metadata(&current)
                    .map_err(|_| "cannot inspect Git source directory".to_owned())?
                    .is_dir()
                {
                    return Err("Git source path has a non-directory parent".to_owned());
                }
            }
            _ => return Err("Git source path escapes its root".to_owned()),
        }
    }
    Ok(())
}

fn validate_git_path(path: &Path) -> InstallResult<()> {
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err("Git source contains a path outside its root".to_owned());
    }
    Ok(())
}

fn validate_git_ref(reference: &str) -> InstallResult<()> {
    if reference.is_empty()
        || reference.starts_with('-')
        || reference.chars().any(char::is_whitespace)
        || reference
            .chars()
            .any(|character| character.is_control() || "~^:?*[\\".contains(character))
        || reference.contains("..")
        || reference.contains("@{")
        || reference.contains("//")
        || reference.ends_with('/')
        || reference.ends_with('.')
        || reference.ends_with(".lock")
        || reference.starts_with('.')
    {
        return Err("Git ref is not a safe branch, tag or commit".to_owned());
    }
    if reference.starts_with("refs/")
        && !(reference.starts_with("refs/heads/") || reference.starts_with("refs/tags/"))
    {
        return Err("only Git branches, tags and full commits are supported".to_owned());
    }
    Ok(())
}

fn validate_git_url(input: &str) -> InstallResult<String> {
    if input.is_empty()
        || input
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
    {
        return Err("Git URL must be a credential-free HTTPS or SSH remote".to_owned());
    }
    if let Some(rest) = input.strip_prefix("https://") {
        let authority = rest.split('/').next().unwrap_or_default();
        if authority.is_empty()
            || authority.contains('@')
            || authority.contains('%')
            || input.contains('?')
            || input.contains('#')
        {
            return Err("Git URL must not contain credentials, query data or fragments".to_owned());
        }
        if !authority
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || ".-:[]".contains(character))
        {
            return Err("Git URL has an invalid HTTPS host".to_owned());
        }
        if rest
            .split('/')
            .skip(1)
            .any(|part| part == ".." || part == ".")
        {
            return Err("Git URL contains an invalid path".to_owned());
        }
        return Ok(input.to_owned());
    }
    if let Some(rest) = input.strip_prefix("ssh://") {
        let authority = rest.split('/').next().unwrap_or_default();
        let host = if let Some((user, host)) = authority.rsplit_once('@') {
            if user.is_empty()
                || user.contains(':')
                || !user
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
            {
                return Err("SSH Git URL contains unsupported credentials".to_owned());
            }
            host
        } else {
            authority
        };
        if host.is_empty()
            || authority.contains('%')
            || input.contains('?')
            || input.contains('#')
            || !host
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || ".-:[]".contains(character))
        {
            return Err("Git URL must be a credential-free HTTPS or SSH remote".to_owned());
        }
        if rest
            .split('/')
            .skip(1)
            .any(|part| part == ".." || part == ".")
        {
            return Err("Git URL contains an invalid path".to_owned());
        }
        return Ok(input.to_owned());
    }
    // SCP-style SSH remotes are intentionally limited to a simple user/host/path.
    if !input.starts_with('-') && input.contains('@') {
        if let Some((user, host_path)) = input.split_once('@') {
            if !user.is_empty()
                && !user.contains(':')
                && user
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
            {
                if let Some((host, path)) = host_path.split_once(':') {
                    if !host.is_empty()
                        && !path.is_empty()
                        && !path.starts_with('/')
                        && host
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || ".-_".contains(c))
                        && !path.split('/').any(|part| part == ".." || part == ".")
                    {
                        return Ok(input.to_owned());
                    }
                }
            }
        }
    }
    Err("Git URL must be a credential-free HTTPS or SSH remote".to_owned())
}

fn git_command(
    paths: &Paths,
    arguments: &[String],
    cwd: Option<&Path>,
) -> InstallResult<std::process::Output> {
    let git = find_program(paths, "git")
        .ok_or_else(|| "Git is not installed or could not start".to_owned())?;
    let mut command = Command::new(git);
    command
        .args(arguments)
        .env("GIT_ALLOW_PROTOCOL", "https:ssh")
        .env("GIT_TERMINAL_PROMPT", "0");
    for name in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_COMMON_DIR",
        "GIT_PREFIX",
        "GIT_CONFIG",
        "GIT_CONFIG_COUNT",
        "GIT_CONFIG_PARAMETERS",
    ] {
        command.env_remove(name);
    }
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let mut path = vec![paths.bin.clone()];
    if let Some(path_env) = std::env::var_os("PATH") {
        path.extend(std::env::split_paths(&path_env));
    }
    if let Ok(path_env) = std::env::join_paths(path) {
        command.env("PATH", path_env);
    }
    command.stdout(Stdio::piped()).stderr(Stdio::null());
    command
        .output()
        .map_err(|_| "Git is not installed or could not start".to_owned())
}

fn find_program(paths: &Paths, name: &str) -> Option<PathBuf> {
    let path_dirs = std::env::var_os("PATH")
        .as_ref()
        .map(|path| std::env::split_paths(path).collect::<Vec<_>>())
        .unwrap_or_default();
    std::iter::once(paths.bin.clone())
        .chain(path_dirs)
        .map(|directory| directory.join(name))
        .find(|candidate| candidate.is_file())
}

fn key_lock(directory: &Path, name: &str) -> InstallResult<File> {
    private_dir(directory)?;
    let path = directory.join(format!("{}.lock", sha256_bytes(name.as_bytes())));
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let lock = options
        .open(path)
        .map_err(|_| "cannot open the source cache lock".to_owned())?;
    lock.lock_exclusive()
        .map_err(|_| "cannot lock the source cache".to_owned())?;
    Ok(lock)
}

fn unique_stage(parent: &Path, key: &str) -> InstallResult<PathBuf> {
    for _ in 0..64 {
        let number = STAGE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let candidate = parent.join(format!(".stage-{}-{number}", &key[..key.len().min(16)]));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err("cannot reserve a source staging path".to_owned())
}

fn activate_snapshot(stage: &Path, destination: &Path) -> InstallResult<()> {
    let parent = destination
        .parent()
        .ok_or_else(|| "invalid source cache path".to_owned())?;
    fs::create_dir_all(parent).map_err(|_| "cannot create source cache directory".to_owned())?;
    fs::rename(stage, destination).map_err(|_| "cannot activate source cache snapshot".to_owned())
}

fn is_full_commit(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn path_identity(path: &Path) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        String::from_utf8_lossy(path.as_os_str().as_bytes()).into_owned()
    }
    #[cfg(not(unix))]
    {
        path.to_string_lossy().into_owned()
    }
}

#[cfg(unix)]
fn os_path_from_bytes(bytes: &[u8]) -> InstallResult<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    Ok(PathBuf::from(OsString::from_vec(bytes.to_vec())))
}

#[cfg(not(unix))]
fn os_path_from_bytes(bytes: &[u8]) -> InstallResult<PathBuf> {
    String::from_utf8(bytes.to_vec())
        .map(PathBuf::from)
        .map_err(|_| "Git source contains an unsupported non-UTF-8 path".to_owned())
}
