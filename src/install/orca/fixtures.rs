use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::effective::EffectiveOrca;
use crate::install::api::{InstalledTool, Outcome, Paths};
use crate::install::engine::RunReport;
use crate::install::state::{self, State};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct TempTree(PathBuf);

impl TempTree {
    fn new() -> Self {
        // Keep fixtures beside the executable so they use the target filesystem,
        // which avoids ETXTBSY failures on some temporary filesystems.
        let executable = std::env::current_exe().expect("resolve current test executable");
        let parent = executable.parent().expect("test executable has a parent");
        let path = parent.join(format!(
            "yashik-orca-fixture-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("create Orca fixture root");
        Self(fs::canonicalize(path).expect("canonicalize Orca fixture root"))
    }

    fn paths(&self) -> Paths {
        let home = self.0.join("home");
        fs::create_dir(&home).expect("create fixture home");
        let paths = Paths {
            home,
            data: self.0.join("data"),
            cache: self.0.join("cache"),
            state: self.0.join("state"),
            bin: self.0.join("bin"),
        };
        crate::install::paths::create_private(&paths).expect("create private paths");
        paths
    }
}

fn sorted_directory_children(path: &Path) -> Vec<std::ffi::OsString> {
    let mut children = fs::read_dir(path)
        .expect("read fixture directory")
        .map(|entry| entry.expect("read fixture directory entry").file_name())
        .collect::<Vec<_>>();
    children.sort();
    children
}

impl Drop for TempTree {
    fn drop(&mut self) {
        let _ = crate::install::util::remove_tree_checked(&self.0);
    }
}

struct FakeRelease {
    version: String,
    metadata_url: String,
    asset_url: String,
    metadata: Vec<u8>,
    archive: Vec<u8>,
}

impl FakeRelease {
    fn new(version: &str, reported_cli_version: &str) -> Self {
        Self::with_declared_digest(version, reported_cli_version, None)
    }

    fn with_declared_digest(
        version: &str,
        reported_cli_version: &str,
        declared_digest: Option<&str>,
    ) -> Self {
        let asset_name = super::expected_asset_name(std::env::consts::ARCH).unwrap();
        let metadata_url = super::metadata_url(version).unwrap();
        let asset_url = super::expected_asset_url(version, std::env::consts::ARCH).unwrap();
        let archive = fake_appimage(reported_cli_version);
        let actual_digest = super::util::sha256_bytes(&archive);
        let digest = declared_digest.unwrap_or(&actual_digest);
        let metadata = format!(
            r#"{{"draft":false,"prerelease":false,"tag_name":"v{version}","assets":[{{"name":"{asset_name}","browser_download_url":"{asset_url}","size":{},"digest":"sha256:{digest}"}}]}}"#,
            archive.len()
        )
        .into_bytes();
        Self {
            version: version.to_owned(),
            metadata_url,
            asset_url,
            metadata,
            archive,
        }
    }
}

fn fake_appimage(reported_cli_version: &str) -> Vec<u8> {
    format!(
        "#!/bin/sh\nset -eu\nif [ \"${{1:-}}\" != \"--appimage-extract\" ]; then exit 2; fi\nmkdir -p squashfs-root/resources/bin\ncat > squashfs-root/resources/bin/orca-ide <<'ORCA_CLI'\n#!/bin/sh\nif [ \"${{1:-}}\" = \"--version\" ]; then printf '%s\\n' \"{reported_cli_version}\"; exit 0; fi\nexit 0\nORCA_CLI\nprintf '%s\\n' \"{reported_cli_version}\" > squashfs-root/resources/build-version\nchmod 755 squashfs-root/resources/bin/orca-ide\nprintf 'squashfs-root\\n'\n"
    )
    .into_bytes()
}

struct ReconcileResult {
    report: RunReport,
    metadata_calls: usize,
    download_calls: usize,
    confirmation_calls: usize,
}

fn reconcile_fixture(
    paths: &Paths,
    desired: Option<&EffectiveOrca>,
    installer_state: &mut State,
    release: Option<&FakeRelease>,
    confirmed: bool,
) -> ReconcileResult {
    reconcile_fixture_with_confirmation_hook(
        paths,
        desired,
        installer_state,
        release,
        confirmed,
        || {},
    )
}

fn reconcile_fixture_with_confirmation_hook(
    paths: &Paths,
    desired: Option<&EffectiveOrca>,
    installer_state: &mut State,
    release: Option<&FakeRelease>,
    confirmed: bool,
    mut on_confirmation: impl FnMut(),
) -> ReconcileResult {
    let mut report = RunReport::default();
    let metadata_calls = Cell::new(0usize);
    let download_calls = Cell::new(0usize);
    let confirmation_calls = Cell::new(0usize);
    {
        let mut confirm = |_question: &str| {
            confirmation_calls.set(confirmation_calls.get() + 1);
            on_confirmation();
            confirmed
        };
        let mut fetch = |url: &str, max_bytes: u64| {
            metadata_calls.set(metadata_calls.get() + 1);
            let release =
                release.ok_or_else(|| "unexpected metadata request in fixture".to_owned())?;
            assert_eq!(url, release.metadata_url);
            assert!(release.metadata.len() as u64 <= max_bytes);
            Ok(release.metadata.clone())
        };
        let mut download = |url: &str, max_bytes: u64, destination: &Path| {
            download_calls.set(download_calls.get() + 1);
            let release =
                release.ok_or_else(|| "unexpected archive request in fixture".to_owned())?;
            assert_eq!(url, release.asset_url);
            assert_eq!(max_bytes, release.archive.len() as u64);
            fs::write(destination, &release.archive)
                .map_err(|_| "could not write fixture AppImage".to_owned())
        };

        super::reconcile_with_confirmation_using(
            paths,
            desired,
            installer_state,
            &mut report,
            &mut confirm,
            &mut fetch,
            &mut download,
        )
        .expect("run Orca reconcile fixture");
    }

    ReconcileResult {
        report,
        metadata_calls: metadata_calls.get(),
        download_calls: download_calls.get(),
        confirmation_calls: confirmation_calls.get(),
    }
}

fn installed_record(installer_state: &State) -> &InstalledTool {
    installer_state
        .tools
        .get("orca")
        .expect("Orca ownership is recorded")
}

fn tool_outcome(report: &RunReport) -> Outcome {
    report
        .operations
        .iter()
        .find(|operation| operation.id == "tool/orca")
        .expect("Orca operation is reported")
        .outcome
}

fn exact(version: &str) -> EffectiveOrca {
    EffectiveOrca {
        version: version.to_owned(),
    }
}

#[cfg(unix)]
fn launcher_identity(path: &Path) -> (u64, i64, i64) {
    use std::os::unix::fs::MetadataExt;
    let metadata = fs::metadata(path).expect("read launcher metadata");
    (metadata.ino(), metadata.mtime(), metadata.mtime_nsec())
}

#[cfg(unix)]
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn first_install_and_pinned_repeat_keep_launcher_inode_and_mtime() {
    let fixture = TempTree::new();
    let paths = fixture.paths();
    let release = FakeRelease::new("1.4.221", "1.4.221");
    let desired = exact(&release.version);
    let mut installer_state = State::empty();

    let first = reconcile_fixture(
        &paths,
        Some(&desired),
        &mut installer_state,
        Some(&release),
        false,
    );
    assert_eq!(tool_outcome(&first.report), Outcome::Succeeded);
    assert_eq!((first.metadata_calls, first.download_calls), (1, 1));
    assert_eq!(installed_record(&installer_state).version, "1.4.221");
    let launcher = paths.bin.join("orca-ide");
    let original_identity = launcher_identity(&launcher);

    let repeat = reconcile_fixture(&paths, Some(&desired), &mut installer_state, None, false);
    assert_eq!(tool_outcome(&repeat.report), Outcome::Unchanged);
    assert_eq!((repeat.metadata_calls, repeat.download_calls), (0, 0));
    assert_eq!(launcher_identity(&launcher), original_identity);

    let loaded = state::load(&paths).unwrap();
    assert_eq!(installed_record(&loaded).version, "1.4.221");
}

#[test]
fn bad_appimage_digest_fails_before_ownership_or_launcher_activation() {
    let fixture = TempTree::new();
    let paths = fixture.paths();
    let release = FakeRelease::with_declared_digest("1.4.221", "1.4.221", Some(&"0".repeat(64)));
    let desired = exact(&release.version);
    let mut installer_state = State::empty();

    let result = reconcile_fixture(
        &paths,
        Some(&desired),
        &mut installer_state,
        Some(&release),
        false,
    );

    assert_eq!(tool_outcome(&result.report), Outcome::Failed);
    assert_eq!((result.metadata_calls, result.download_calls), (1, 1));
    assert!(!installer_state.tools.contains_key("orca"));
    assert!(!paths.bin.join("orca-ide").exists());
    assert!(!paths.data.join("tools/orca/versions/1.4.221").exists());
}

#[test]
fn wrong_cli_probe_fails_without_ownership_or_launcher_activation() {
    let fixture = TempTree::new();
    let paths = fixture.paths();
    let release = FakeRelease::new("1.4.221", "1.4.220");
    let desired = exact(&release.version);
    let mut installer_state = State::empty();

    let result = reconcile_fixture(
        &paths,
        Some(&desired),
        &mut installer_state,
        Some(&release),
        false,
    );

    assert_eq!(tool_outcome(&result.report), Outcome::Failed);
    assert_eq!((result.metadata_calls, result.download_calls), (1, 1));
    assert!(!installer_state.tools.contains_key("orca"));
    assert!(!paths.bin.join("orca-ide").exists());
    assert!(!paths.data.join("tools/orca/versions/1.4.221").exists());
}

#[test]
fn unmanaged_launcher_collision_preserves_existing_bytes_without_network() {
    let fixture = TempTree::new();
    let paths = fixture.paths();
    let launcher = paths.bin.join("orca-ide");
    let original = b"user-owned launcher\n";
    fs::write(&launcher, original).unwrap();
    let release = FakeRelease::new("1.4.221", "1.4.221");
    let desired = exact(&release.version);
    let mut installer_state = State::empty();

    let result = reconcile_fixture(
        &paths,
        Some(&desired),
        &mut installer_state,
        Some(&release),
        false,
    );

    assert_eq!(tool_outcome(&result.report), Outcome::Failed);
    assert_eq!((result.metadata_calls, result.download_calls), (0, 0));
    assert_eq!(fs::read(&launcher).unwrap(), original);
    assert!(!installer_state.tools.contains_key("orca"));
}

#[test]
fn bundle_drift_blocks_doctor_upgrade_and_removal() {
    let fixture = TempTree::new();
    let paths = fixture.paths();
    let first_release = FakeRelease::new("1.4.221", "1.4.221");
    let first_desired = exact(&first_release.version);
    let mut installer_state = State::empty();
    let first = reconcile_fixture(
        &paths,
        Some(&first_desired),
        &mut installer_state,
        Some(&first_release),
        false,
    );
    assert_eq!(tool_outcome(&first.report), Outcome::Succeeded);

    let record = installed_record(&installer_state).clone();
    let ownership = record.orca.as_ref().unwrap();
    let version_file = ownership.bundle_root.join("resources/build-version");
    fs::write(&version_file, b"modified after install\n").unwrap();
    let launcher = paths.bin.join("orca-ide");
    let launcher_bytes = fs::read(&launcher).unwrap();

    assert!(super::doctor(&paths, &record).is_err());

    let upgrade = exact("1.4.222");
    let blocked_upgrade =
        reconcile_fixture(&paths, Some(&upgrade), &mut installer_state, None, false);
    assert_eq!(tool_outcome(&blocked_upgrade.report), Outcome::Failed);
    assert_eq!(
        (
            blocked_upgrade.metadata_calls,
            blocked_upgrade.download_calls
        ),
        (0, 0)
    );

    let pending = reconcile_fixture(&paths, None, &mut installer_state, None, true);
    assert_eq!(tool_outcome(&pending.report), Outcome::PendingRemoval);
    assert_eq!(pending.confirmation_calls, 0);
    assert_eq!(fs::read(&launcher).unwrap(), launcher_bytes);
    assert!(version_file.exists());
    assert_eq!(installed_record(&installer_state).version, "1.4.221");
}

#[test]
fn launcher_drift_blocks_doctor_upgrade_and_removal() {
    let fixture = TempTree::new();
    let paths = fixture.paths();
    let first_release = FakeRelease::new("1.4.221", "1.4.221");
    let first_desired = exact(&first_release.version);
    let mut installer_state = State::empty();
    let first = reconcile_fixture(
        &paths,
        Some(&first_desired),
        &mut installer_state,
        Some(&first_release),
        false,
    );
    assert_eq!(tool_outcome(&first.report), Outcome::Succeeded);

    let launcher = paths.bin.join("orca-ide");
    let drifted_launcher = b"#!/bin/sh\nprintf 'user replacement\\n'\n";
    fs::write(&launcher, drifted_launcher).unwrap();
    make_executable(&launcher);
    let record = installed_record(&installer_state).clone();

    assert!(super::doctor(&paths, &record).is_err());

    let upgrade = exact("1.4.222");
    let blocked_upgrade =
        reconcile_fixture(&paths, Some(&upgrade), &mut installer_state, None, false);
    assert_eq!(tool_outcome(&blocked_upgrade.report), Outcome::Failed);
    assert_eq!(
        (
            blocked_upgrade.metadata_calls,
            blocked_upgrade.download_calls
        ),
        (0, 0)
    );

    let pending = reconcile_fixture(&paths, None, &mut installer_state, None, true);
    assert_eq!(tool_outcome(&pending.report), Outcome::PendingRemoval);
    assert_eq!(pending.confirmation_calls, 0);
    assert_eq!(fs::read(&launcher).unwrap(), drifted_launcher);
    assert_eq!(installed_record(&installer_state).version, "1.4.221");
}

#[test]
fn unconfirmed_removal_stays_pending_and_confirmed_removal_preserves_user_data_and_neighbors() {
    let fixture = TempTree::new();
    let paths = fixture.paths();
    let release = FakeRelease::new("1.4.221", "1.4.221");
    let desired = exact(&release.version);
    let mut installer_state = State::empty();
    let installed = reconcile_fixture(
        &paths,
        Some(&desired),
        &mut installer_state,
        Some(&release),
        false,
    );
    assert_eq!(tool_outcome(&installed.report), Outcome::Succeeded);

    let record = installed_record(&installer_state).clone();
    let bundle = record.orca.as_ref().unwrap().bundle_root.clone();
    let profile = paths.home.join(".config/orca/profiles/default.json");
    fs::create_dir_all(profile.parent().unwrap()).unwrap();
    fs::write(&profile, b"user session profile\n").unwrap();
    let launcher_neighbor = paths.bin.join("orca-ide-neighbor");
    fs::write(&launcher_neighbor, b"neighbor\n").unwrap();
    let bundle_neighbor = bundle.parent().unwrap().join("user-file.txt");
    fs::write(&bundle_neighbor, b"not owned by bundle root\n").unwrap();

    let pending = reconcile_fixture(&paths, None, &mut installer_state, None, false);
    assert_eq!(tool_outcome(&pending.report), Outcome::PendingRemoval);
    assert_eq!(pending.confirmation_calls, 1);
    assert_eq!(installed_record(&installer_state).version, "1.4.221");
    assert!(paths.bin.join("orca-ide").exists());
    assert!(bundle.exists());

    let removed = reconcile_fixture(&paths, None, &mut installer_state, None, true);
    assert_eq!(tool_outcome(&removed.report), Outcome::Succeeded);
    assert_eq!(removed.confirmation_calls, 1);
    assert!(!installer_state.tools.contains_key("orca"));
    assert!(!paths.bin.join("orca-ide").exists());
    assert!(!bundle.exists());
    assert_eq!(fs::read(&profile).unwrap(), b"user session profile\n");
    assert_eq!(fs::read(&launcher_neighbor).unwrap(), b"neighbor\n");
    assert_eq!(
        fs::read(&bundle_neighbor).unwrap(),
        b"not owned by bundle root\n"
    );
}

#[test]
fn confirmation_time_bundle_drift_is_rechecked_and_kept_pending() {
    let fixture = TempTree::new();
    let paths = fixture.paths();
    let release = FakeRelease::new("1.4.221", "1.4.221");
    let desired = exact(&release.version);
    let mut installer_state = State::empty();
    let installed = reconcile_fixture(
        &paths,
        Some(&desired),
        &mut installer_state,
        Some(&release),
        false,
    );
    assert_eq!(tool_outcome(&installed.report), Outcome::Succeeded);

    let record = installed_record(&installer_state).clone();
    let ownership = record.orca.as_ref().unwrap();
    let bundle_file = ownership.bundle_root.join("resources/build-version");
    let original_launcher = fs::read(&record.executable).unwrap();

    let pending = reconcile_fixture_with_confirmation_hook(
        &paths,
        None,
        &mut installer_state,
        None,
        true,
        || fs::write(&bundle_file, b"changed between confirmation and delete\n").unwrap(),
    );

    assert_eq!(tool_outcome(&pending.report), Outcome::PendingRemoval);
    assert_eq!(pending.confirmation_calls, 1);
    assert_eq!(installed_record(&installer_state).version, "1.4.221");
    assert_eq!(
        fs::read(&bundle_file).unwrap(),
        b"changed between confirmation and delete\n"
    );
    assert_eq!(fs::read(&record.executable).unwrap(), original_launcher);
}

#[test]
fn integrated_doctor_is_offline_and_read_only_for_a_valid_orca_installation() {
    let fixture = TempTree::new();
    let paths = fixture.paths();
    let release = FakeRelease::new("1.4.221", "1.4.221");
    let desired = exact(&release.version);
    let mut installer_state = State::empty();
    let installed = reconcile_fixture(
        &paths,
        Some(&desired),
        &mut installer_state,
        Some(&release),
        false,
    );
    assert_eq!(tool_outcome(&installed.report), Outcome::Succeeded);

    let state_file = paths.state.join("state.json");
    let journal_file = paths.state.join("journal.jsonl");
    let original_state = fs::read(&state_file).unwrap();
    let original_journal = fs::read(&journal_file).unwrap();
    let original_state_mtime = fs::metadata(&state_file).unwrap().modified().unwrap();
    let original_journal_mtime = fs::metadata(&journal_file).unwrap().modified().unwrap();
    let original_state_dir_mtime = fs::metadata(&paths.state).unwrap().modified().unwrap();
    let launcher = paths.bin.join("orca-ide");
    let original_launcher_identity = launcher_identity(&launcher);
    let bundle_version_file = installed_record(&installer_state)
        .orca
        .as_ref()
        .unwrap()
        .bundle_root
        .join("resources/build-version");
    let original_bundle_file_mtime = fs::metadata(&bundle_version_file)
        .unwrap()
        .modified()
        .unwrap();
    let original_tree_hash = super::util::hash_tree(
        &installed_record(&installer_state)
            .orca
            .as_ref()
            .unwrap()
            .bundle_root,
    )
    .unwrap();
    let persistent_orca_root = paths.data.join("tools/orca");
    let original_persistent_orca_children = sorted_directory_children(&persistent_orca_root);

    let doctor_report = crate::install::doctor::run(&paths).unwrap();
    let doctor_orca = doctor_report
        .operations
        .iter()
        .find(|operation| operation.id == "tool/orca")
        .expect("doctor reports Orca");
    assert_eq!(doctor_orca.outcome, Outcome::Unchanged);
    assert_eq!(fs::read(&state_file).unwrap(), original_state);
    assert_eq!(fs::read(&journal_file).unwrap(), original_journal);
    assert_eq!(
        fs::metadata(&state_file).unwrap().modified().unwrap(),
        original_state_mtime
    );
    assert_eq!(
        fs::metadata(&journal_file).unwrap().modified().unwrap(),
        original_journal_mtime
    );
    assert_eq!(
        fs::metadata(&paths.state).unwrap().modified().unwrap(),
        original_state_dir_mtime
    );
    assert_eq!(launcher_identity(&launcher), original_launcher_identity);
    assert_eq!(
        fs::metadata(&bundle_version_file)
            .unwrap()
            .modified()
            .unwrap(),
        original_bundle_file_mtime
    );
    assert_eq!(
        super::util::hash_tree(
            &installed_record(&installer_state)
                .orca
                .as_ref()
                .unwrap()
                .bundle_root,
        )
        .unwrap(),
        original_tree_hash
    );
    assert_eq!(
        sorted_directory_children(&persistent_orca_root),
        original_persistent_orca_children,
        "doctor must not leave a persistent probe HOME under the Orca data root"
    );
}

#[test]
fn exact_upgrade_records_previous_bundle_and_returning_to_pin_reactivates_it_offline() {
    let fixture = TempTree::new();
    let paths = fixture.paths();
    let first_release = FakeRelease::new("1.4.221", "1.4.221");
    let first_desired = exact(&first_release.version);
    let mut installer_state = State::empty();
    let first = reconcile_fixture(
        &paths,
        Some(&first_desired),
        &mut installer_state,
        Some(&first_release),
        false,
    );
    assert_eq!(tool_outcome(&first.report), Outcome::Succeeded);
    let first_bundle = installed_record(&installer_state)
        .orca
        .as_ref()
        .unwrap()
        .bundle_root
        .clone();

    let second_release = FakeRelease::new("1.4.222", "1.4.222");
    let second_desired = exact(&second_release.version);
    let upgrade = reconcile_fixture(
        &paths,
        Some(&second_desired),
        &mut installer_state,
        Some(&second_release),
        false,
    );
    assert_eq!(tool_outcome(&upgrade.report), Outcome::Succeeded);
    assert_eq!(installed_record(&installer_state).version, "1.4.222");
    assert!(first_bundle.exists());
    assert!(installed_record(&installer_state)
        .orca
        .as_ref()
        .unwrap()
        .previous_bundles
        .iter()
        .any(|bundle| bundle.version == "1.4.221"));

    let return_to_pin = reconcile_fixture(
        &paths,
        Some(&first_desired),
        &mut installer_state,
        None,
        false,
    );
    assert_eq!(tool_outcome(&return_to_pin.report), Outcome::Succeeded);
    assert_eq!(
        (return_to_pin.metadata_calls, return_to_pin.download_calls),
        (0, 0)
    );
    assert_eq!(installed_record(&installer_state).version, "1.4.221");
    assert_eq!(
        installed_record(&installer_state).versioned_executable,
        first_bundle.join("resources/bin/orca-ide")
    );
    assert!(installed_record(&installer_state)
        .orca
        .as_ref()
        .unwrap()
        .previous_bundles
        .iter()
        .any(|bundle| bundle.version == "1.4.222"));
}

#[test]
fn failed_upgrade_probe_restores_old_launcher_and_keeps_old_state_owner() {
    let fixture = TempTree::new();
    let paths = fixture.paths();
    let first_release = FakeRelease::new("1.4.221", "1.4.221");
    let first_desired = exact(&first_release.version);
    let mut installer_state = State::empty();
    let first = reconcile_fixture(
        &paths,
        Some(&first_desired),
        &mut installer_state,
        Some(&first_release),
        false,
    );
    assert_eq!(tool_outcome(&first.report), Outcome::Succeeded);
    let old_launcher = fs::read(paths.bin.join("orca-ide")).unwrap();

    let bad_upgrade = FakeRelease::new("1.4.222", "1.4.220");
    let desired = exact(&bad_upgrade.version);
    let result = reconcile_fixture(
        &paths,
        Some(&desired),
        &mut installer_state,
        Some(&bad_upgrade),
        false,
    );

    assert_eq!(tool_outcome(&result.report), Outcome::Failed);
    assert_eq!((result.metadata_calls, result.download_calls), (1, 1));
    assert_eq!(fs::read(paths.bin.join("orca-ide")).unwrap(), old_launcher);
    assert_eq!(installed_record(&installer_state).version, "1.4.221");
    assert!(!paths.data.join("tools/orca/versions/1.4.222").exists());
}
