use std::fs;
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use super::api::{
    InstallResult, InstalledOrcaOwnership, InstalledOrcaVersion, InstalledTool, Outcome, Paths,
};
use super::engine::{self, ReportOperation, RunReport};
use super::{state, util};
use crate::effective::EffectiveOrca;
use serde::Deserialize;

#[cfg(all(test, target_os = "linux"))]
#[path = "orca/fixtures.rs"]
mod reconciliation_fixtures;

const TOOL_ID: &str = "orca";
const OPERATION_ID: &str = "tool/orca";
const LATEST_METADATA_URL: &str = "https://api.github.com/repos/stablyai/orca/releases/latest";
const RELEASE_METADATA_PREFIX: &str = "https://api.github.com/repos/stablyai/orca/releases/tags/v";
const RELEASE_ASSET_PREFIX: &str = "https://github.com/stablyai/orca/releases/download/v";
const MAX_METADATA_BYTES: u64 = 2 * 1024 * 1024;
const MAX_APPIMAGE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_EXTRACTED_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_TREE_ENTRIES: usize = 50_000;
const MAX_TREE_DEPTH: usize = 64;
const METADATA_TIMEOUT: Duration = Duration::from_secs(20);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(600);
const EXTRACT_TIMEOUT: Duration = Duration::from_secs(180);
const PROBE_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_EXTRACT_OUTPUT: usize = 64 * 1024;
const MAX_PROBE_OUTPUT: usize = 4 * 1024;
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, PartialEq, Eq)]
struct ResolvedRelease {
    version: String,
    metadata_url: String,
    asset_url: String,
    asset_size: u64,
    sha256: String,
    asset_name: String,
}

#[derive(Deserialize)]
struct ReleaseApi {
    draft: Option<bool>,
    prerelease: Option<bool>,
    tag_name: Option<String>,
    assets: Option<Vec<ReleaseAsset>>,
}

#[derive(Deserialize)]
struct ReleaseAsset {
    name: Option<String>,
    browser_download_url: Option<String>,
    size: Option<u64>,
    digest: Option<String>,
}

enum InstallAttempt {
    Unchanged,
    Activated {
        record: Box<InstalledTool>,
        rollback: ActivationRollback,
    },
}

struct ActivationRollback {
    launcher: PathBuf,
    prior_launcher: Option<Vec<u8>>,
    installed_launcher_sha256: String,
    created_bundle: Option<(PathBuf, String)>,
}

struct StageDirectory(PathBuf);

impl Drop for StageDirectory {
    fn drop(&mut self) {
        let _ = util::remove_tree_checked(&self.0);
    }
}

pub fn reconcile(
    paths: &Paths,
    desired: Option<&EffectiveOrca>,
    installer_state: &mut state::State,
    report: &mut RunReport,
) -> InstallResult<()> {
    let mut metadata_fetcher = fetch_metadata;
    let mut archive_downloader = download_archive;
    reconcile_with_confirmation_using(
        paths,
        desired,
        installer_state,
        report,
        &mut confirm_removal,
        &mut metadata_fetcher,
        &mut archive_downloader,
    )
}

fn reconcile_with_confirmation_using(
    paths: &Paths,
    desired: Option<&EffectiveOrca>,
    installer_state: &mut state::State,
    report: &mut RunReport,
    confirm: &mut dyn FnMut(&str) -> bool,
    metadata_fetcher: &mut impl FnMut(&str, u64) -> InstallResult<Vec<u8>>,
    archive_downloader: &mut impl FnMut(&str, u64, &Path) -> InstallResult<()>,
) -> InstallResult<()> {
    match desired {
        Some(desired) => install_desired_using(
            paths,
            desired,
            installer_state,
            report,
            metadata_fetcher,
            archive_downloader,
        ),
        None => remove_if_obsolete(paths, installer_state, report, confirm),
    }
}

fn install_desired_using(
    paths: &Paths,
    desired: &EffectiveOrca,
    installer_state: &mut state::State,
    report: &mut RunReport,
    metadata_fetcher: &mut impl FnMut(&str, u64) -> InstallResult<Vec<u8>>,
    archive_downloader: &mut impl FnMut(&str, u64, &Path) -> InstallResult<()>,
) -> InstallResult<()> {
    engine::begin_operation(paths, installer_state, OPERATION_ID, None)?;
    let previous = installer_state.tools.get(TOOL_ID).cloned();
    let attempt = match install_requested(
        paths,
        desired,
        previous.as_ref(),
        metadata_fetcher,
        archive_downloader,
    ) {
        Ok(attempt) => attempt,
        Err(error) => {
            return engine::complete_without_mutation(
                paths,
                installer_state,
                report,
                OPERATION_ID,
                None,
                Outcome::Failed,
                Some(error),
            );
        }
    };

    match attempt {
        InstallAttempt::Unchanged => engine::finish_operation(
            paths,
            installer_state,
            report,
            OPERATION_ID,
            None,
            Outcome::Unchanged,
            Some(format!("Orca {} is present and unchanged", desired.version)),
            |_| {},
        ),
        InstallAttempt::Activated { record, rollback } => {
            let message = format!("Orca {} CLI is installed as orca-ide", record.version);
            let operation = state::OperationRecord::new(
                OPERATION_ID,
                None,
                state::OperationPhase::Completed,
                Outcome::Succeeded,
                Some(message.clone()),
            );
            let mut next = installer_state.clone();
            next.tools.insert(TOOL_ID.to_owned(), *record);
            next.operations
                .insert(OPERATION_ID.to_owned(), operation.clone());
            if state::journal(paths, &operation).is_err() || state::save(paths, &next).is_err() {
                rollback_activation(rollback);
                report.operations.push(ReportOperation {
                    id: OPERATION_ID.into(),
                    outcome: Outcome::Failed,
                    message: Some(
                        "Orca activation was rolled back because ownership state could not be committed".into(),
                    ),
                });
                return Ok(());
            }
            *installer_state = next;
            report.operations.push(ReportOperation {
                id: OPERATION_ID.into(),
                outcome: Outcome::Succeeded,
                message: Some(message),
            });
            Ok(())
        }
    }
}

fn install_requested(
    paths: &Paths,
    desired: &EffectiveOrca,
    previous: Option<&InstalledTool>,
    metadata_fetcher: &mut impl FnMut(&str, u64) -> InstallResult<Vec<u8>>,
    archive_downloader: &mut impl FnMut(&str, u64, &Path) -> InstallResult<()>,
) -> InstallResult<InstallAttempt> {
    ensure_supported_platform()?;
    if desired.version != "latest"
        && !crate::validation::is_valid_exact_harness_version(&desired.version)
    {
        return Err("Orca version must be `latest` or an exact semantic version".into());
    }
    match previous {
        Some(record) => verify_record(paths, record)?,
        None => check_unmanaged_launcher(paths)?,
    }

    if let Some(record) = previous {
        if desired.version != "latest" && desired.version == record.version {
            return Ok(InstallAttempt::Unchanged);
        }
        if desired.version != "latest" {
            if let Some(old) = record.orca.as_ref().and_then(|ownership| {
                ownership
                    .previous_bundles
                    .iter()
                    .find(|old| old.version == desired.version)
            }) {
                verify_historical_bundle(paths, old)?;
                let release = ResolvedRelease {
                    version: old.version.clone(),
                    metadata_url: old.metadata_url.clone(),
                    asset_url: old.asset_url.clone(),
                    asset_size: 0,
                    sha256: old.sha256.clone(),
                    asset_name: expected_asset_name(std::env::consts::ARCH)?.to_owned(),
                };
                return activate_existing_bundle(paths, record, old, release);
            }
        }
    }

    let metadata_url = metadata_url(&desired.version)?;
    let metadata = metadata_fetcher(&metadata_url, MAX_METADATA_BYTES)?;
    if metadata.len() as u64 > MAX_METADATA_BYTES {
        return Err("Orca release metadata exceeds the supported size limit".into());
    }
    let release = resolve_metadata(
        &metadata,
        &desired.version,
        &metadata_url,
        std::env::consts::OS,
        std::env::consts::ARCH,
    )?;

    if let Some(record) = previous {
        if record.version == release.version && record.sha256 == release.sha256 {
            return Ok(InstallAttempt::Unchanged);
        }
        if let Some(old) = record.orca.as_ref().and_then(|ownership| {
            ownership
                .previous_bundles
                .iter()
                .find(|old| old.version == release.version && old.sha256 == release.sha256)
        }) {
            verify_historical_bundle(paths, old)?;
            return activate_existing_bundle(paths, record, old, release);
        }
    }

    ensure_available_space(paths, release.asset_size)?;
    let expected_bundle = versioned_bundle(paths, &release.version);
    if path_exists(&expected_bundle)? {
        return Err("refusing to replace an unowned Orca bundle directory".into());
    }
    let expected_cli = expected_bundle.join("resources/bin/orca-ide");
    let staged = stage_bundle(paths, &release, archive_downloader)?;
    let version_dir = expected_bundle
        .parent()
        .ok_or_else(|| "Orca bundle path has no parent".to_owned())?;
    let versions_dir = version_dir
        .parent()
        .ok_or_else(|| "Orca version directory has no parent".to_owned())?;
    util::private_dir(versions_dir)?;
    util::create_private_dir_new(version_dir)?;
    if path_exists(&expected_bundle)? {
        return Err("refusing to replace an unowned Orca bundle directory".into());
    }
    if let Err(error) = rename_bundle_new(&staged.bundle_root, &expected_bundle) {
        let _ = fs::remove_dir(version_dir);
        return Err(format!(
            "could not safely place the verified Orca bundle: {error}"
        ));
    }
    let moved_hash = match util::hash_tree_streaming_bounded(
        &expected_bundle,
        MAX_TREE_ENTRIES,
        MAX_EXTRACTED_BYTES,
        MAX_TREE_DEPTH,
    ) {
        Ok((fingerprint, _)) => fingerprint,
        Err(error) => {
            cleanup_created_bundle(&expected_bundle, &staged.bundle_fingerprint);
            return Err(error);
        }
    };
    if moved_hash != staged.bundle_fingerprint {
        cleanup_created_bundle(&expected_bundle, &staged.bundle_fingerprint);
        return Err("Orca bundle changed while it was being activated".into());
    }
    let versioned = InstalledOrcaVersion {
        version: release.version.clone(),
        metadata_url: release.metadata_url.clone(),
        asset_url: release.asset_url.clone(),
        sha256: release.sha256.clone(),
        bundle_root: expected_bundle.clone(),
        cli_executable: expected_cli.clone(),
        bundle_fingerprint: staged.bundle_fingerprint.clone(),
    };
    match activate_new_bundle(paths, previous, &release, versioned, true) {
        Ok(attempt) => Ok(attempt),
        Err(error) => {
            cleanup_created_bundle(&expected_bundle, &staged.bundle_fingerprint);
            Err(error)
        }
    }
}

fn activate_existing_bundle(
    paths: &Paths,
    previous: &InstalledTool,
    old: &InstalledOrcaVersion,
    release: ResolvedRelease,
) -> InstallResult<InstallAttempt> {
    let existing = old.clone();
    activate_new_bundle(paths, Some(previous), &release, existing, false)
}

struct StagedBundle {
    bundle_root: PathBuf,
    bundle_fingerprint: String,
    _stage: StageDirectory,
}

fn stage_bundle(
    paths: &Paths,
    release: &ResolvedRelease,
    archive_downloader: &mut impl FnMut(&str, u64, &Path) -> InstallResult<()>,
) -> InstallResult<StagedBundle> {
    let staging_root = orca_root(paths).join("staging");
    util::private_dir(&staging_root)?;
    let nonce = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let stage_path = staging_root.join(format!("run-{}-{nonce}", std::process::id()));
    util::create_private_dir_new(&stage_path)?;
    let stage = StageDirectory(stage_path.clone());
    let archive = stage.0.join("orca.AppImage");
    archive_downloader(&release.asset_url, release.asset_size, &archive)?;
    verify_download(&archive, release)?;
    set_mode(&archive, 0o500)?;

    let extract_root = stage.0.join("extract");
    util::create_private_dir_new(&extract_root)?;
    extract_appimage(&archive, &extract_root)?;
    let bundle_root = extract_root.join("squashfs-root");
    ensure_real_directory(&bundle_root, "extracted Orca bundle is missing or unsafe")?;
    let cli = bundle_root.join("resources/bin/orca-ide");
    ensure_directory_path(
        cli.parent()
            .ok_or_else(|| "extracted Orca CLI path has no parent".to_owned())?,
    )?;
    verify_executable_file(&cli, "extracted Orca CLI is missing or unsafe")?;
    probe_version(&cli, &stage.0, &release.version)?;
    let (bundle_fingerprint, _) = util::hash_tree_streaming_bounded(
        &bundle_root,
        MAX_TREE_ENTRIES,
        MAX_EXTRACTED_BYTES,
        MAX_TREE_DEPTH,
    )?;
    Ok(StagedBundle {
        bundle_root,
        bundle_fingerprint,
        _stage: stage,
    })
}

fn verify_download(archive: &Path, release: &ResolvedRelease) -> InstallResult<()> {
    let metadata = fs::symlink_metadata(archive)
        .map_err(|_| "downloaded Orca AppImage is missing or unsafe".to_owned())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("downloaded Orca AppImage is not a regular file".into());
    }
    if metadata.len() > MAX_APPIMAGE_BYTES {
        return Err("downloaded Orca AppImage exceeds the supported size limit".into());
    }
    if metadata.len() != release.asset_size {
        return Err(
            "downloaded Orca AppImage size does not match official release metadata".into(),
        );
    }
    if util::sha256_file(archive)? != release.sha256 {
        return Err("downloaded Orca AppImage failed its official SHA-256 check".into());
    }
    Ok(())
}

fn activate_new_bundle(
    paths: &Paths,
    previous: Option<&InstalledTool>,
    release: &ResolvedRelease,
    installed: InstalledOrcaVersion,
    bundle_created: bool,
) -> InstallResult<InstallAttempt> {
    let launcher = paths.bin.join("orca-ide");
    let launcher_bytes = launcher_script(&installed.cli_executable)?;
    let launcher_sha256 = util::sha256_bytes(&launcher_bytes);
    let prior_launcher = previous
        .map(|record| util::read_file_checked_bounded(&paths.bin, &record.executable, 64 * 1024))
        .transpose()?;
    let expected_old = previous
        .and_then(|record| record.orca.as_ref())
        .map(|ownership| ownership.launcher_fingerprint.as_str());

    util::atomic_write_if_unchanged(&launcher, &launcher_bytes, 0o755, expected_old)
        .map_err(|error| format!("could not safely activate Orca's orca-ide launcher: {error}"))?;
    if let Err(error) = verify_executable_file(&launcher, "Orca launcher is missing or unsafe")
        .and_then(|_| {
            if util::sha256_file(&launcher)? != launcher_sha256 {
                return Err("Orca launcher changed during activation".into());
            }
            probe_version_temporary(&launcher, &release.version)
        })
    {
        rollback_activation(ActivationRollback {
            launcher: launcher.clone(),
            prior_launcher,
            installed_launcher_sha256: launcher_sha256.clone(),
            created_bundle: bundle_created.then(|| {
                (
                    installed.bundle_root.clone(),
                    installed.bundle_fingerprint.clone(),
                )
            }),
        });
        return Err(format!(
            "Orca CLI activation failed and was rolled back: {error}"
        ));
    }

    let mut history = previous
        .and_then(|record| record.orca.as_ref())
        .map(|ownership| ownership.previous_bundles.clone())
        .unwrap_or_default();
    if let Some(previous) = previous {
        let ownership = previous
            .orca
            .as_ref()
            .ok_or_else(|| "recorded Orca ownership is incomplete".to_owned())?;
        let old = InstalledOrcaVersion {
            version: previous.version.clone(),
            metadata_url: previous.metadata_url.clone(),
            asset_url: previous.asset_url.clone(),
            sha256: previous.sha256.clone(),
            bundle_root: ownership.bundle_root.clone(),
            cli_executable: ownership.cli_executable.clone(),
            bundle_fingerprint: ownership.bundle_fingerprint.clone(),
        };
        if old.version != installed.version
            && !history.iter().any(|version| version.version == old.version)
        {
            history.push(old);
        }
    }
    history.retain(|version| version.version != installed.version);
    let record = InstalledTool {
        id: TOOL_ID.to_owned(),
        version: installed.version.clone(),
        metadata_url: release.metadata_url.clone(),
        asset_url: release.asset_url.clone(),
        sha256: release.sha256.clone(),
        versioned_executable: installed.cli_executable.clone(),
        executable: launcher.clone(),
        fingerprint: launcher_sha256.clone(),
        previous_versions: Vec::new(),
        orca: Some(InstalledOrcaOwnership {
            bundle_root: installed.bundle_root.clone(),
            cli_executable: installed.cli_executable.clone(),
            bundle_fingerprint: installed.bundle_fingerprint.clone(),
            launcher_fingerprint: launcher_sha256.clone(),
            previous_bundles: history,
        }),
    };
    Ok(InstallAttempt::Activated {
        record: Box::new(record),
        rollback: ActivationRollback {
            launcher,
            prior_launcher,
            installed_launcher_sha256: launcher_sha256,
            created_bundle: bundle_created
                .then_some((installed.bundle_root, installed.bundle_fingerprint)),
        },
    })
}

fn rollback_activation(rollback: ActivationRollback) {
    if let Some(previous) = rollback.prior_launcher {
        let _ = util::atomic_write_if_unchanged(
            &rollback.launcher,
            &previous,
            0o755,
            Some(&rollback.installed_launcher_sha256),
        );
    } else {
        let _ = util::remove_file_checked(
            &rollback.launcher,
            Some(&rollback.installed_launcher_sha256),
        );
    }
    if let Some((bundle_root, expected_fingerprint)) = rollback.created_bundle {
        cleanup_created_bundle(&bundle_root, &expected_fingerprint);
    }
}

fn cleanup_created_bundle(bundle_root: &Path, expected_fingerprint: &str) {
    if util::hash_tree_streaming_bounded(
        bundle_root,
        MAX_TREE_ENTRIES,
        MAX_EXTRACTED_BYTES,
        MAX_TREE_DEPTH,
    )
    .map(|result| result.0 == expected_fingerprint)
    .unwrap_or(false)
    {
        let _ = util::remove_tree_checked(bundle_root);
        if let Some(parent) = bundle_root.parent() {
            let _ = fs::remove_dir(parent);
        }
    }
}

fn remove_if_obsolete(
    paths: &Paths,
    installer_state: &mut state::State,
    report: &mut RunReport,
    confirm: &mut dyn FnMut(&str) -> bool,
) -> InstallResult<()> {
    let Some(record) = installer_state.tools.get(TOOL_ID).cloned() else {
        return Ok(());
    };
    if let Err(error) = verify_record(paths, &record) {
        return engine::complete_without_mutation(
            paths,
            installer_state,
            report,
            OPERATION_ID,
            None,
            Outcome::PendingRemoval,
            Some(format!(
                "Orca removal is pending because managed files drifted: {error}"
            )),
        );
    }
    if !confirm(&format!(
        "Remove managed Orca {} bundle and launcher? [y/N]: ",
        record.version
    )) {
        return engine::complete_without_mutation(
            paths,
            installer_state,
            report,
            OPERATION_ID,
            None,
            Outcome::PendingRemoval,
            Some("managed Orca remains installed; removal was not confirmed".into()),
        );
    }

    if let Err(error) = verify_record(paths, &record) {
        return engine::complete_without_mutation(
            paths,
            installer_state,
            report,
            OPERATION_ID,
            None,
            Outcome::PendingRemoval,
            Some(format!(
                "Orca removal is pending because managed files changed after confirmation: {error}"
            )),
        );
    }

    engine::begin_operation(paths, installer_state, OPERATION_ID, None)?;
    match remove_recorded_bundle(&record) {
        Ok(()) => engine::finish_operation(
            paths,
            installer_state,
            report,
            OPERATION_ID,
            None,
            Outcome::Succeeded,
            Some("managed Orca CLI and bundles were removed; user profiles and sessions were retained".into()),
            |next| {
                next.tools.remove(TOOL_ID);
            },
        ),
        Err(error) => engine::complete_without_mutation(
            paths,
            installer_state,
            report,
            OPERATION_ID,
            None,
            Outcome::Failed,
            Some(format!("Orca removal could not be completed safely: {error}")),
        ),
    }
}

fn remove_recorded_bundle(record: &InstalledTool) -> InstallResult<()> {
    let ownership = record
        .orca
        .as_ref()
        .ok_or_else(|| "recorded Orca ownership is incomplete".to_owned())?;
    for old in &ownership.previous_bundles {
        remove_bundle_root(&old.bundle_root)?;
    }
    remove_bundle_root(&ownership.bundle_root)?;
    util::remove_file_checked(&record.executable, Some(&ownership.launcher_fingerprint))?;
    Ok(())
}

fn remove_bundle_root(root: &Path) -> InstallResult<()> {
    util::remove_tree_checked(root)?;
    if let Some(parent) = root.parent() {
        let _ = fs::remove_dir(parent);
    }
    Ok(())
}

pub fn doctor(paths: &Paths, record: &InstalledTool) -> InstallResult<()> {
    verify_record(paths, record)
}

fn verify_record(paths: &Paths, record: &InstalledTool) -> InstallResult<()> {
    ensure_supported_platform()?;
    let ownership = record
        .orca
        .as_ref()
        .ok_or_else(|| "recorded Orca bundle ownership is missing".to_owned())?;
    let expected_bundle = versioned_bundle(paths, &record.version);
    let expected_cli = expected_bundle.join("resources/bin/orca-ide");
    let expected_launcher = paths.bin.join("orca-ide");
    if record.id != TOOL_ID
        || !crate::validation::is_valid_exact_harness_version(&record.version)
        || !valid_metadata_url(&record.metadata_url, &record.version)
        || record.asset_url != expected_asset_url(&record.version, std::env::consts::ARCH)?
        || !valid_sha256(&record.sha256)
        || record.versioned_executable != expected_cli
        || record.executable != expected_launcher
        || record.fingerprint != ownership.launcher_fingerprint
        || ownership.bundle_root != expected_bundle
        || ownership.cli_executable != expected_cli
        || !valid_sha256(&ownership.bundle_fingerprint)
        || !valid_sha256(&ownership.launcher_fingerprint)
        || !record.previous_versions.is_empty()
    {
        return Err("recorded Orca ownership paths or release metadata are invalid".into());
    }
    let mut history_versions = std::collections::BTreeSet::new();
    for version in &ownership.previous_bundles {
        if version.version == record.version || !history_versions.insert(version.version.as_str()) {
            return Err("recorded Orca release history contains duplicate versions".into());
        }
    }
    verify_bundle(
        &ownership.bundle_root,
        &ownership.cli_executable,
        &record.version,
        &ownership.bundle_fingerprint,
    )?;
    verify_launcher(&record.executable, ownership, &record.version)?;
    for version in &ownership.previous_bundles {
        verify_historical_bundle(paths, version)?;
    }
    Ok(())
}

fn verify_historical_bundle(paths: &Paths, record: &InstalledOrcaVersion) -> InstallResult<()> {
    let expected_bundle = versioned_bundle(paths, &record.version);
    let expected_cli = expected_bundle.join("resources/bin/orca-ide");
    if !crate::validation::is_valid_exact_harness_version(&record.version)
        || !valid_metadata_url(&record.metadata_url, &record.version)
        || record.asset_url != expected_asset_url(&record.version, std::env::consts::ARCH)?
        || !valid_sha256(&record.sha256)
        || record.bundle_root != expected_bundle
        || record.cli_executable != expected_cli
        || !valid_sha256(&record.bundle_fingerprint)
    {
        return Err("recorded Orca release history is invalid".into());
    }
    verify_bundle(
        &record.bundle_root,
        &record.cli_executable,
        &record.version,
        &record.bundle_fingerprint,
    )
}

fn verify_bundle(
    bundle_root: &Path,
    cli: &Path,
    version: &str,
    expected_fingerprint: &str,
) -> InstallResult<()> {
    ensure_directory_path(bundle_root)?;
    ensure_directory_path(
        cli.parent()
            .ok_or_else(|| "recorded Orca CLI path has no parent".to_owned())?,
    )?;
    ensure_real_directory(bundle_root, "recorded Orca bundle is missing or unsafe")?;
    verify_executable_file(cli, "recorded Orca CLI is missing or unsafe")?;
    let (actual_fingerprint, _) = util::hash_tree_streaming_bounded(
        bundle_root,
        MAX_TREE_ENTRIES,
        MAX_EXTRACTED_BYTES,
        MAX_TREE_DEPTH,
    )?;
    if actual_fingerprint != expected_fingerprint {
        return Err("recorded Orca bundle has drifted".into());
    }
    probe_version_temporary(cli, version)
}

fn verify_launcher(
    path: &Path,
    ownership: &InstalledOrcaOwnership,
    version: &str,
) -> InstallResult<()> {
    ensure_directory_path(
        path.parent()
            .ok_or_else(|| "recorded Orca launcher path has no parent".to_owned())?,
    )?;
    verify_executable_file(path, "recorded Orca launcher is missing or unsafe")?;
    if util::sha256_file(path)? != ownership.launcher_fingerprint {
        return Err("recorded Orca launcher has drifted".into());
    }
    let expected = launcher_script(&ownership.cli_executable)?;
    if util::read_file_checked_bounded(path.parent().unwrap_or(Path::new("/")), path, 64 * 1024)?
        != expected
    {
        return Err("recorded Orca launcher does not point to its owned CLI".into());
    }
    probe_version_temporary(path, version)
}

fn check_unmanaged_launcher(paths: &Paths) -> InstallResult<()> {
    let launcher = paths.bin.join("orca-ide");
    if path_exists(&launcher)? {
        return Err("refusing to replace an unowned orca-ide launcher".into());
    }
    Ok(())
}

fn resolve_metadata(
    bytes: &[u8],
    requested: &str,
    metadata_url_value: &str,
    os: &str,
    arch: &str,
) -> InstallResult<ResolvedRelease> {
    if requested != "latest" && !crate::validation::is_valid_exact_harness_version(requested) {
        return Err("Orca version must be `latest` or an exact semantic version".into());
    }
    let metadata = metadata_url(requested)?;
    if metadata != metadata_url_value || bytes.len() as u64 > MAX_METADATA_BYTES {
        return Err("Orca release metadata request or size is invalid".into());
    }
    let release: ReleaseApi = serde_json::from_slice(bytes).map_err(|_| {
        "Orca release metadata is invalid JSON or has an unsupported shape".to_owned()
    })?;
    if release.draft != Some(false) || release.prerelease != Some(false) {
        return Err("Orca release is not a published stable release".into());
    }
    let tag = release
        .tag_name
        .ok_or_else(|| "Orca release tag is missing".to_owned())?;
    let version = tag
        .strip_prefix('v')
        .ok_or_else(|| "Orca release tag has an invalid version".to_owned())?;
    if !crate::validation::is_valid_exact_harness_version(version)
        || tag != format!("v{version}")
        || (requested != "latest" && requested != version)
    {
        return Err("Orca release tag does not match the requested version".into());
    }
    if !matches!(os, "linux") {
        return Err("Orca CLI installation is supported only on Linux".into());
    }
    let asset_name = expected_asset_name(arch)?.to_owned();
    let assets = release
        .assets
        .ok_or_else(|| "Orca release assets are missing".to_owned())?;
    let matching = assets
        .iter()
        .filter(|asset| asset.name.as_deref() == Some(asset_name.as_str()))
        .collect::<Vec<_>>();
    if matching.len() != 1 {
        return Err("Orca release does not contain exactly one expected Linux AppImage".into());
    }
    let asset = matching[0];
    let expected_url = expected_asset_url(version, arch)?;
    if asset.browser_download_url.as_deref() != Some(expected_url.as_str()) {
        return Err("Orca release AppImage URL is not the canonical official URL".into());
    }
    let size = asset
        .size
        .ok_or_else(|| "Orca release AppImage size is missing".to_owned())?;
    if size == 0 || size > MAX_APPIMAGE_BYTES {
        return Err("Orca release AppImage size is outside the supported limit".into());
    }
    let digest = asset
        .digest
        .as_deref()
        .and_then(|value| value.strip_prefix("sha256:"))
        .ok_or_else(|| "Orca release AppImage has no official SHA-256 digest".to_owned())?;
    if !valid_sha256(digest) {
        return Err("Orca release AppImage SHA-256 digest is invalid".into());
    }
    Ok(ResolvedRelease {
        version: version.to_owned(),
        metadata_url: metadata.to_owned(),
        asset_url: expected_url,
        asset_size: size,
        sha256: digest.to_ascii_lowercase(),
        asset_name,
    })
}

fn metadata_url(requested: &str) -> InstallResult<String> {
    if requested == "latest" {
        Ok(LATEST_METADATA_URL.to_owned())
    } else if crate::validation::is_valid_exact_harness_version(requested) {
        Ok(format!("{RELEASE_METADATA_PREFIX}{requested}"))
    } else {
        Err("Orca version must be `latest` or an exact semantic version".into())
    }
}

fn valid_metadata_url(value: &str, version: &str) -> bool {
    value == LATEST_METADATA_URL || value == format!("{RELEASE_METADATA_PREFIX}{version}")
}

fn expected_asset_name(arch: &str) -> InstallResult<&'static str> {
    match arch {
        "x86_64" => Ok("orca-linux.AppImage"),
        "aarch64" => Ok("orca-linux-arm64.AppImage"),
        _ => Err("Orca CLI installation supports Linux x86_64 and aarch64 only".into()),
    }
}

fn expected_asset_url(version: &str, arch: &str) -> InstallResult<String> {
    let asset = expected_asset_name(arch)?;
    Ok(format!("{RELEASE_ASSET_PREFIX}{version}/{asset}"))
}

fn ensure_supported_platform() -> InstallResult<()> {
    if std::env::consts::OS != "linux" {
        return Err("Orca CLI installation is supported only on Linux".into());
    }
    expected_asset_name(std::env::consts::ARCH).map(|_| ())
}

fn orca_root(paths: &Paths) -> PathBuf {
    paths.data.join("tools").join("orca")
}

fn versioned_bundle(paths: &Paths, version: &str) -> PathBuf {
    orca_root(paths)
        .join("versions")
        .join(version)
        .join("bundle")
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn path_exists(path: &Path) -> InstallResult<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err("could not inspect an Orca managed path".into()),
    }
}

fn launcher_script(cli: &Path) -> InstallResult<Vec<u8>> {
    if !cli.is_absolute() {
        return Err("Orca CLI path is not absolute".into());
    }
    let cli = cli
        .to_str()
        .ok_or_else(|| "Orca CLI path is not valid UTF-8".to_owned())?;
    let quoted = shell_quote(cli);
    Ok(format!("#!/bin/sh\nexec {quoted} \"$@\"\n").into_bytes())
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn set_mode(path: &Path, mode: u32) -> InstallResult<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))
            .map_err(|_| "could not set private Orca file permissions".into())
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
        Err("Orca AppImage installation requires Unix permissions".into())
    }
}

#[cfg(target_os = "linux")]
fn rename_bundle_new(source: &Path, destination: &Path) -> InstallResult<()> {
    use std::os::unix::ffi::OsStrExt;

    let source = std::ffi::CString::new(source.as_os_str().as_bytes())
        .map_err(|_| "Orca staging path contains a null byte".to_owned())?;
    let destination = std::ffi::CString::new(destination.as_os_str().as_bytes())
        .map_err(|_| "Orca bundle path contains a null byte".to_owned())?;
    // Atomic NOREPLACE prevents a racing unowned bundle from being overwritten.
    // `libc::renameat2` is missing from some libcs, including musl. Calling
    // the Linux syscall directly keeps the atomic no-replace guarantee across
    // glibc and musl without falling back to an overwrite-capable rename.
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2 as libc::c_long,
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            destination.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err("destination exists or bundle rename failed".into())
    }
}

#[cfg(not(target_os = "linux"))]
fn rename_bundle_new(_source: &Path, _destination: &Path) -> InstallResult<()> {
    Err("Orca AppImage installation requires Linux no-replace directory rename".into())
}

fn ensure_real_directory(path: &Path, message: &str) -> InstallResult<()> {
    let metadata = fs::symlink_metadata(path).map_err(|_| message.to_owned())?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(message.to_owned());
    }
    Ok(())
}

fn ensure_directory_path(path: &Path) -> InstallResult<()> {
    if !path.is_absolute() {
        return Err("recorded Orca path is not absolute".into());
    }
    let components = path.components().collect::<Vec<_>>();
    let mut current = PathBuf::new();
    for (index, component) in components.iter().enumerate() {
        current.push(component.as_os_str());
        let metadata = fs::symlink_metadata(&current)
            .map_err(|_| "recorded Orca directory is missing or unsafe".to_owned())?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err("recorded Orca path contains a non-directory or symlink".into());
        }
        if index + 1 == components.len() && !metadata.is_dir() {
            return Err("recorded Orca directory is unsafe".into());
        }
    }
    Ok(())
}

fn verify_executable_file(path: &Path, message: &str) -> InstallResult<()> {
    let metadata = fs::symlink_metadata(path).map_err(|_| message.to_owned())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(message.to_owned());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(message.to_owned());
        }
    }
    Ok(())
}

fn create_probe_home(path: &Path) -> InstallResult<()> {
    util::create_private_dir_new(path)?;
    for relative in ["tmp", ".config", ".cache", ".local/share", ".local/state"] {
        util::ensure_dir(&path.join(relative), 0o700)?;
    }
    Ok(())
}

struct ProbeHome(PathBuf);

impl ProbeHome {
    fn create(parent: &Path) -> InstallResult<Self> {
        let nonce = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!("probe-{}-{nonce}", std::process::id()));
        create_probe_home(&path)?;
        Ok(Self(path))
    }

    fn create_temporary() -> InstallResult<Self> {
        let nonce = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("yashik-orca-probe-{}-{nonce}", std::process::id()));
        create_probe_home(&path)?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for ProbeHome {
    fn drop(&mut self) {
        let _ = util::remove_tree_checked(&self.0);
    }
}

fn probe_version_temporary(executable: &Path, expected: &str) -> InstallResult<()> {
    let home = ProbeHome::create_temporary()?;
    probe_version_in_home(executable, &home, expected)
}

fn probe_version(executable: &Path, home_parent: &Path, expected: &str) -> InstallResult<()> {
    let home = ProbeHome::create(home_parent)?;
    probe_version_in_home(executable, &home, expected)
}

fn probe_version_in_home(executable: &Path, home: &ProbeHome, expected: &str) -> InstallResult<()> {
    let mut command = Command::new(executable);
    command
        .current_dir(home.path())
        .arg("--version")
        .env_clear()
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("XDG_CONFIG_HOME", home.path().join(".config"))
        .env("XDG_CACHE_HOME", home.path().join(".cache"))
        .env("XDG_DATA_HOME", home.path().join(".local/share"))
        .env("XDG_STATE_HOME", home.path().join(".local/state"))
        .env("TMPDIR", home.path().join("tmp"))
        .env("TEMP", home.path().join("tmp"))
        .env("TMP", home.path().join("tmp"))
        .env("PATH", "/usr/bin:/bin:/usr/local/bin")
        .env("LC_ALL", "C")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    isolate_process_group(&mut command);
    let mut child = command
        .spawn()
        .map_err(|_| "could not start the Orca CLI --version probe".to_owned())?;
    let (stdout, stderr) = capture_child_output(&mut child, PROBE_TIMEOUT, MAX_PROBE_OUTPUT)?;
    if !child_status_success(&mut child)? {
        return Err("Orca CLI --version probe failed".into());
    }
    if stdout.truncated || stderr.truncated {
        return Err("Orca CLI --version output exceeded the supported limit".into());
    }
    if stdout.bytes != format!("{expected}\n").as_bytes() {
        return Err("Orca CLI --version did not return the expected exact version".into());
    }
    Ok(())
}

struct CapturedOutput {
    bytes: Vec<u8>,
    truncated: bool,
}

fn capture_child_output(
    child: &mut Child,
    timeout: Duration,
    per_stream_limit: usize,
) -> InstallResult<(CapturedOutput, CapturedOutput)> {
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdout_reader = thread::spawn(move || capture_limited(stdout, per_stream_limit));
    let stderr_reader = thread::spawn(move || capture_limited(stderr, per_stream_limit));
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                // A wrapper may have left helper processes with inherited pipes.
                terminate_child_group(child);
                break;
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(25)),
            Ok(None) => {
                terminate_child_group(child);
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err("Orca CLI --version probe timed out".into());
            }
            Err(_) => {
                terminate_child_group(child);
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err("Orca CLI --version probe could not be completed".into());
            }
        }
    }
    let stdout = stdout_reader
        .join()
        .map_err(|_| "could not capture Orca CLI version output".to_owned())?;
    let stderr = stderr_reader
        .join()
        .map_err(|_| "could not capture Orca CLI diagnostic output".to_owned())?;
    Ok((stdout, stderr))
}

fn child_status_success(child: &mut Child) -> InstallResult<bool> {
    child
        .try_wait()
        .map(|status| status.is_some_and(|status| status.success()))
        .map_err(|_| "could not inspect Orca CLI probe status".into())
}

fn capture_limited<R: Read>(reader: Option<R>, limit: usize) -> CapturedOutput {
    let Some(mut reader) = reader else {
        return CapturedOutput {
            bytes: Vec::new(),
            truncated: false,
        };
    };
    let mut bytes = Vec::new();
    let mut truncated = false;
    let mut buffer = [0u8; 4096];
    loop {
        let count = match reader.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(count) => count,
        };
        let remaining = limit.saturating_sub(bytes.len());
        let keep = count.min(remaining);
        bytes.extend_from_slice(&buffer[..keep]);
        truncated |= keep < count;
    }
    CapturedOutput { bytes, truncated }
}

fn extract_appimage(archive: &Path, extract_root: &Path) -> InstallResult<()> {
    let home = extract_root.join(".extract-home");
    create_probe_home(&home)?;
    let mut command = Command::new(archive);
    command
        .current_dir(extract_root)
        .arg("--appimage-extract")
        .env_clear()
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("XDG_CACHE_HOME", home.join(".cache"))
        .env("XDG_DATA_HOME", home.join(".local/share"))
        .env("XDG_STATE_HOME", home.join(".local/state"))
        .env("TMPDIR", home.join("tmp"))
        .env("PATH", "/usr/bin:/bin")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    isolate_process_group(&mut command);
    let mut child = command
        .spawn()
        .map_err(|_| "could not start the verified Orca AppImage extractor".to_owned())?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdout_reader = thread::spawn(move || capture_limited(stdout, MAX_EXTRACT_OUTPUT / 2));
    let stderr_reader = thread::spawn(move || capture_limited(stderr, MAX_EXTRACT_OUTPUT / 2));
    let deadline = Instant::now() + EXTRACT_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                terminate_child_group(&mut child);
                break status;
            }
            Ok(None) if Instant::now() < deadline => {
                match extraction_usage(&extract_root.join("squashfs-root")) {
                    Ok((entries, bytes))
                        if entries <= MAX_TREE_ENTRIES && bytes <= MAX_EXTRACTED_BYTES =>
                    {
                        thread::sleep(Duration::from_millis(100));
                    }
                    Ok(_) => {
                        terminate_child_group(&mut child);
                        let _ = stdout_reader.join();
                        let _ = stderr_reader.join();
                        return Err("Orca AppImage extraction exceeded its resource limits".into());
                    }
                    Err(error) => {
                        terminate_child_group(&mut child);
                        let _ = stdout_reader.join();
                        let _ = stderr_reader.join();
                        return Err(error);
                    }
                }
            }
            Ok(None) => {
                terminate_child_group(&mut child);
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err("Orca AppImage extraction exceeded its 180 second time limit".into());
            }
            Err(_) => {
                terminate_child_group(&mut child);
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err("could not inspect Orca AppImage extractor status".into());
            }
        }
    };
    let stdout = stdout_reader
        .join()
        .map_err(|_| "could not capture Orca extractor output".to_owned())?;
    let stderr = stderr_reader
        .join()
        .map_err(|_| "could not capture Orca extractor diagnostics".to_owned())?;
    if stdout.truncated || stderr.truncated {
        return Err("Orca AppImage extractor output exceeded the supported limit".into());
    }
    if !status.success() {
        return Err("verified Orca AppImage extraction failed".into());
    }
    Ok(())
}

fn extraction_usage(root: &Path) -> InstallResult<(usize, u64)> {
    extraction_usage_with_limits(root, MAX_TREE_ENTRIES, MAX_EXTRACTED_BYTES, MAX_TREE_DEPTH)
}

fn extraction_usage_with_limits(
    root: &Path,
    max_entries: usize,
    max_total_file_bytes: u64,
    max_depth: usize,
) -> InstallResult<(usize, u64)> {
    match fs::symlink_metadata(root) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok((0, 0)),
        Err(_) => return Err("could not inspect the extracted Orca tree".into()),
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err("extracted Orca bundle root is not a real directory".into())
        }
        Ok(_) => {}
    }
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    let mut entries = 0usize;
    let mut total = 0u64;
    while let Some((directory, depth)) = stack.pop() {
        if depth > max_depth {
            return Err("Orca AppImage extraction exceeded its depth limit".into());
        }
        let children = fs::read_dir(&directory)
            .map_err(|_| "could not inspect the extracted Orca tree".to_owned())?;
        for child in children {
            let child = child
                .map_err(|_| "could not inspect the extracted Orca tree".to_owned())?
                .path();
            entries = entries.saturating_add(1);
            if entries > max_entries {
                return Err("Orca AppImage extraction exceeded its entry limit".into());
            }
            let metadata = fs::symlink_metadata(&child)
                .map_err(|_| "could not inspect the extracted Orca tree".to_owned())?;
            if metadata.file_type().is_symlink() {
                let target = fs::read_link(&child)
                    .map_err(|_| "could not inspect an extracted Orca symlink".to_owned())?;
                let relative_parent = child
                    .parent()
                    .and_then(|parent| parent.strip_prefix(root).ok())
                    .unwrap_or(Path::new(""));
                if !link_stays_inside(relative_parent, &target) {
                    return Err("Orca AppImage extraction contains an escaping symlink".into());
                }
            } else if metadata.is_dir() {
                stack.push((child, depth + 1));
            } else if metadata.is_file() {
                total = total.saturating_add(metadata.len());
                if total > max_total_file_bytes {
                    return Err("Orca AppImage extraction exceeded its expanded size limit".into());
                }
            } else {
                return Err("Orca AppImage extraction contains a special file".into());
            }
        }
    }
    Ok((entries, total))
}

fn link_stays_inside(parent: &Path, target: &Path) -> bool {
    if target.is_absolute() {
        return false;
    }
    let mut depth = parent
        .components()
        .filter(|component| matches!(component, Component::Normal(_)))
        .count();
    for component in target.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(_) => depth = depth.saturating_add(1),
            Component::ParentDir if depth > 0 => depth -= 1,
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return false,
        }
    }
    true
}

fn ensure_available_space(paths: &Paths, archive_size: u64) -> InstallResult<()> {
    let required = archive_size.saturating_add(MAX_EXTRACTED_BYTES);
    let available = available_space(&paths.data)?;
    if available < required {
        return Err(format!(
            "not enough free space for the Orca AppImage and bounded extraction (need at least {required} bytes, have {available})"
        ));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn available_space(path: &Path) -> InstallResult<u64> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|_| "Yashik data path contains a null byte".to_owned())?;
    let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) } != 0 {
        return Err("could not determine free space for the Orca installation".into());
    }
    let stats = unsafe { stats.assume_init() };
    let unit = if stats.f_frsize > 0 {
        stats.f_frsize
    } else {
        stats.f_bsize
    };
    Ok(stats.f_bavail.saturating_mul(unit))
}

#[cfg(not(target_os = "linux"))]
fn available_space(_path: &Path) -> InstallResult<u64> {
    Err("Orca AppImage installation requires Linux filesystem space checks".into())
}

fn fetch_metadata(url: &str, max_bytes: u64) -> InstallResult<Vec<u8>> {
    if !matches!(url, LATEST_METADATA_URL) && !url.starts_with(RELEASE_METADATA_PREFIX) {
        return Err("refusing an Orca metadata request outside the official GitHub API".into());
    }
    if max_bytes > MAX_METADATA_BYTES {
        return Err("Orca metadata request exceeds the supported size limit".into());
    }
    curl_to_bytes(url, max_bytes, METADATA_TIMEOUT, "Orca release metadata")
}

fn download_archive(url: &str, max_bytes: u64, destination: &Path) -> InstallResult<()> {
    if !url.starts_with(RELEASE_ASSET_PREFIX)
        || max_bytes == 0
        || max_bytes > MAX_APPIMAGE_BYTES
        || path_exists(destination)?
    {
        return Err("refusing an Orca download outside the official release endpoint".into());
    }
    let curl = find_curl().ok_or_else(|| "curl is required to install Orca".to_owned())?;
    let output = util::open_or_create_regular(destination, 0o600)?;
    let limit = max_bytes.to_string();
    let timeout = DOWNLOAD_TIMEOUT.as_secs().to_string();
    let mut command = Command::new(curl);
    command
        .args([
            "--disable",
            "--fail",
            "--location",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--tlsv1.2",
            "--connect-timeout",
            "15",
            "--max-time",
            &timeout,
            "--max-filesize",
            &limit,
            "--silent",
            "--show-error",
        ])
        .arg(url)
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    isolate_process_group(&mut command);
    let mut child = command
        .spawn()
        .map_err(|_| "could not start curl for the Orca AppImage".to_owned())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "could not read the Orca AppImage download stream".to_owned())?;
    let (sender, receiver) = mpsc::sync_channel(1);
    let reader = thread::spawn(move || {
        let result = copy_bounded_stream(stdout, output, max_bytes);
        let _ = sender.send(result);
    });
    let deadline = Instant::now() + DOWNLOAD_TIMEOUT;
    let mut stream_result: Option<Result<(), String>> = None;
    let mut status = None;
    loop {
        if stream_result.is_none() {
            match receiver.try_recv() {
                Ok(Ok(_)) => stream_result = Some(Ok(())),
                Ok(Err(error)) => {
                    terminate_child_group(&mut child);
                    let _ = reader.join();
                    return Err(error);
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    terminate_child_group(&mut child);
                    let _ = reader.join();
                    return Err("Orca AppImage download stream stopped unexpectedly".into());
                }
            }
        }
        if status.is_none() {
            match child.try_wait() {
                Ok(Some(child_status)) => {
                    terminate_child_group(&mut child);
                    status = Some(child_status);
                }
                Ok(None) => {}
                Err(_) => {
                    terminate_child_group(&mut child);
                    let _ = reader.join();
                    return Err("could not inspect Orca download status".into());
                }
            }
        }
        if let Some(child_status) = status {
            if !child_status.success() {
                let _ = reader.join();
                return Err("official Orca AppImage download failed".into());
            }
            if let Some(Ok(())) = stream_result {
                let _ = reader.join();
                let metadata = fs::symlink_metadata(destination)
                    .map_err(|_| "downloaded Orca AppImage is missing or unsafe".to_owned())?;
                if metadata.file_type().is_symlink()
                    || !metadata.is_file()
                    || metadata.len() > max_bytes
                {
                    return Err("downloaded Orca AppImage exceeded its size limit".into());
                }
                return Ok(());
            }
        }
        if Instant::now() >= deadline {
            terminate_child_group(&mut child);
            let _ = reader.join();
            return Err("Orca AppImage download exceeded its 600 second time limit".into());
        }
        thread::sleep(Duration::from_millis(25));
    }
}

fn curl_to_bytes(
    url: &str,
    max_bytes: u64,
    timeout: Duration,
    operation: &str,
) -> InstallResult<Vec<u8>> {
    if max_bytes == 0 {
        return Err(format!("{operation} limit is invalid"));
    }
    let curl = find_curl().ok_or_else(|| "curl is required to install Orca".to_owned())?;
    let limit = max_bytes.to_string();
    let timeout_arg = timeout.as_secs().to_string();
    let mut command = Command::new(curl);
    command
        .args([
            "--disable",
            "--fail",
            "--location",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--tlsv1.2",
            "--connect-timeout",
            "15",
            "--max-time",
            &timeout_arg,
            "--max-filesize",
            &limit,
            "--silent",
            "--show-error",
            url,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    isolate_process_group(&mut command);
    let mut child = command
        .spawn()
        .map_err(|_| format!("could not start curl for {operation}"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| format!("could not read {operation} response"))?;
    let (sender, receiver) = mpsc::sync_channel(1);
    let reader = thread::spawn(move || {
        let result = read_bounded_stream(stdout, max_bytes);
        let _ = sender.send(result);
    });
    let deadline = Instant::now() + timeout;
    let mut stream_result: Option<Result<Vec<u8>, String>> = None;
    let mut status = None;
    loop {
        if stream_result.is_none() {
            match receiver.try_recv() {
                Ok(result) => stream_result = Some(result),
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    terminate_child_group(&mut child);
                    let _ = reader.join();
                    return Err(format!("{operation} response stopped unexpectedly"));
                }
            }
        }
        if matches!(&stream_result, Some(Err(_))) {
            terminate_child_group(&mut child);
            let _ = reader.join();
            return stream_result
                .unwrap()
                .map_err(|error| format!("{operation}: {error}"));
        }
        if status.is_none() {
            match child.try_wait() {
                Ok(Some(child_status)) => {
                    terminate_child_group(&mut child);
                    status = Some(child_status);
                }
                Ok(None) => {}
                Err(_) => {
                    terminate_child_group(&mut child);
                    let _ = reader.join();
                    return Err(format!("could not inspect {operation} request status"));
                }
            }
        }
        if let Some(child_status) = status {
            if !child_status.success() {
                let _ = reader.join();
                return Err(format!("official {operation} download failed"));
            }
            if let Some(Ok(bytes)) = stream_result {
                let _ = reader.join();
                return Ok(bytes);
            }
        }
        if Instant::now() >= deadline {
            terminate_child_group(&mut child);
            let _ = reader.join();
            return Err(format!("{operation} request timed out"));
        }
        thread::sleep(Duration::from_millis(25));
    }
}

fn read_bounded_stream(mut reader: impl Read, limit: u64) -> InstallResult<Vec<u8>> {
    let capacity =
        usize::try_from(limit).map_err(|_| "download size limit is not supported".to_owned())?;
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|_| "could not read bounded download stream".to_owned())?;
        if count == 0 {
            return Ok(bytes);
        }
        if count > capacity.saturating_sub(bytes.len()) {
            return Err("download stream exceeds the supported size limit".into());
        }
        bytes
            .try_reserve(count)
            .map_err(|_| "could not allocate bounded download buffer".to_owned())?;
        bytes.extend_from_slice(&buffer[..count]);
    }
}

fn copy_bounded_stream(
    mut reader: impl Read,
    mut output: fs::File,
    limit: u64,
) -> InstallResult<u64> {
    let mut total = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|_| "could not read bounded Orca AppImage stream".to_owned())?;
        if count == 0 {
            output
                .sync_all()
                .map_err(|_| "could not sync downloaded Orca AppImage".to_owned())?;
            return Ok(total);
        }
        let remaining = limit.saturating_sub(total);
        if count as u64 > remaining {
            let keep = usize::try_from(remaining).unwrap_or(usize::MAX);
            if keep > 0 {
                output
                    .write_all(&buffer[..keep])
                    .map_err(|_| "could not write downloaded Orca AppImage".to_owned())?;
            }
            return Err("Orca AppImage response exceeds its official asset size".into());
        }
        output
            .write_all(&buffer[..count])
            .map_err(|_| "could not write downloaded Orca AppImage".to_owned())?;
        total = total.saturating_add(count as u64);
    }
}

fn find_curl() -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|directory| directory.join("curl"))
        .find(|path| path.is_file())
        .and_then(|path| fs::canonicalize(&path).ok().or(Some(path)))
}

fn isolate_process_group(command: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: setpgid is async-signal-safe and uses only this child process.
        unsafe {
            command.pre_exec(|| {
                if libc::setpgid(0, 0) == 0 {
                    Ok(())
                } else {
                    Err(io::Error::last_os_error())
                }
            });
        }
    }
    #[cfg(not(unix))]
    let _ = command;
}

fn terminate_child_group(child: &mut Child) {
    #[cfg(unix)]
    {
        let process_group = child.id() as libc::pid_t;
        if process_group > 0 {
            // SAFETY: a negative PID addresses only the group created for this child.
            unsafe {
                libc::kill(-process_group, libc::SIGKILL);
            }
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn confirm_removal(question: &str) -> bool {
    eprint!("{question}");
    let _ = io::stderr().flush();
    let mut answer = String::new();
    io::stdin().read_line(&mut answer).is_ok() && matches!(answer.trim(), "y" | "Y" | "yes" | "Yes")
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "linux")]
    use super::probe_version_temporary;
    use super::TEMP_COUNTER;
    use super::{
        copy_bounded_stream, expected_asset_name, extraction_usage_with_limits,
        read_bounded_stream, resolve_metadata, LATEST_METADATA_URL,
    };
    use crate::install::util;
    use std::fs;
    use std::io::Cursor;
    use std::path::PathBuf;
    use std::sync::atomic::Ordering;
    #[cfg(target_os = "linux")]
    use std::time::{Duration, Instant};

    fn fixture_directory(name: &str) -> PathBuf {
        let executable_directory = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let nonce = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path =
            executable_directory.join(format!("yashik-orca-{name}-{}-{nonce}", std::process::id()));
        fs::create_dir(&path).unwrap();
        path
    }

    #[test]
    fn bounded_stream_readers_stop_at_the_limit_without_needing_content_length() {
        let directory = fixture_directory("download-limit");
        let destination = directory.join("download.bin");
        let output = util::open_or_create_regular(&destination, 0o600).unwrap();
        assert!(copy_bounded_stream(Cursor::new(b"12345"), output, 4).is_err());
        assert_eq!(fs::read(&destination).unwrap(), b"1234");
        assert!(read_bounded_stream(Cursor::new(b"12345"), 4).is_err());
        let _ = util::remove_tree_checked(&directory);
    }

    #[test]
    fn extraction_walker_enforces_small_fixture_limits_during_walk() {
        let directory = fixture_directory("extract-limit");
        let root = directory.join("squashfs-root");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("a"), b"a").unwrap();
        fs::write(root.join("b"), b"b").unwrap();
        fs::write(root.join("c"), b"c").unwrap();
        assert!(extraction_usage_with_limits(&root, 2, 100, 10).is_err());
        assert!(extraction_usage_with_limits(&root, 10, 2, 10).is_err());
        let _ = util::remove_tree_checked(&directory);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn bundle_rename_uses_atomic_no_replace_semantics() {
        let directory = fixture_directory("rename-noreplace");
        let source = directory.join("source");
        let destination = directory.join("destination");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&destination).unwrap();
        fs::write(source.join("owned"), b"source").unwrap();
        fs::write(destination.join("foreign"), b"destination").unwrap();

        assert!(super::rename_bundle_new(&source, &destination).is_err());
        assert_eq!(fs::read(source.join("owned")).unwrap(), b"source");
        assert_eq!(
            fs::read(destination.join("foreign")).unwrap(),
            b"destination"
        );

        let movable = directory.join("movable");
        let moved = directory.join("moved");
        fs::create_dir(&movable).unwrap();
        fs::write(movable.join("owned"), b"moved source").unwrap();
        super::rename_bundle_new(&movable, &moved).unwrap();
        assert!(!movable.exists());
        assert_eq!(fs::read(moved.join("owned")).unwrap(), b"moved source");

        let _ = util::remove_tree_checked(&directory);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn version_probe_kills_background_helpers_that_keep_pipes_open() {
        use std::os::unix::fs::PermissionsExt;

        let directory = fixture_directory("probe-group");
        let executable = directory.join("orca-ide");
        fs::write(
            &executable,
            b"#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then (sleep 5) & printf '1.2.3\\n'; exit 0; fi\nexit 3\n",
        )
        .unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
        let started = Instant::now();
        probe_version_temporary(&executable, "1.2.3").unwrap();
        assert!(started.elapsed() < Duration::from_secs(3));
        let _ = util::remove_tree_checked(&directory);
    }

    fn metadata(tag: &str, name: &str, url: &str, size: u64, digest: &str) -> Vec<u8> {
        format!(
            r#"{{"draft":false,"prerelease":false,"tag_name":"{tag}","assets":[{{"name":"{name}","browser_download_url":"{url}","size":{size},"digest":"sha256:{digest}"}}]}}"#
        )
        .into_bytes()
    }

    #[test]
    fn release_resolver_accepts_latest_and_exact_official_assets() {
        let digest = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let latest_url =
            "https://github.com/stablyai/orca/releases/download/v1.4.221/orca-linux.AppImage";
        let latest = metadata("v1.4.221", "orca-linux.AppImage", latest_url, 123, digest);
        let resolved =
            resolve_metadata(&latest, "latest", LATEST_METADATA_URL, "linux", "x86_64").unwrap();
        assert_eq!(resolved.version, "1.4.221");
        assert_eq!(resolved.asset_url, latest_url);
        assert_eq!(resolved.sha256, digest);

        let arm_name = expected_asset_name("aarch64").unwrap();
        let arm_url =
            "https://github.com/stablyai/orca/releases/download/v1.4.221/orca-linux-arm64.AppImage";
        let pinned = metadata("v1.4.221", arm_name, arm_url, 234, digest);
        let resolved = resolve_metadata(
            &pinned,
            "1.4.221",
            "https://api.github.com/repos/stablyai/orca/releases/tags/v1.4.221",
            "linux",
            "aarch64",
        )
        .unwrap();
        assert_eq!(resolved.asset_name, arm_name);
        assert_eq!(resolved.asset_size, 234);
    }

    #[test]
    fn release_resolver_rejects_bad_tag_url_digest_and_platform() {
        let digest = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let url = "https://github.com/stablyai/orca/releases/download/v1.4.221/orca-linux.AppImage";
        let good = metadata("v1.4.221", "orca-linux.AppImage", url, 123, digest);
        assert!(resolve_metadata(
            &good,
            "1.4.220",
            "https://api.github.com/repos/stablyai/orca/releases/tags/v1.4.220",
            "linux",
            "x86_64"
        )
        .is_err());
        let bad_url = metadata(
            "v1.4.221",
            "orca-linux.AppImage",
            "https://attacker.example/orca.AppImage",
            123,
            digest,
        );
        assert!(
            resolve_metadata(&bad_url, "latest", LATEST_METADATA_URL, "linux", "x86_64").is_err()
        );
        let bad_digest = metadata("v1.4.221", "orca-linux.AppImage", url, 123, "missing");
        assert!(resolve_metadata(
            &bad_digest,
            "latest",
            LATEST_METADATA_URL,
            "linux",
            "x86_64"
        )
        .is_err());
        assert!(resolve_metadata(&good, "latest", LATEST_METADATA_URL, "macos", "x86_64").is_err());
        assert!(resolve_metadata(&good, "latest", LATEST_METADATA_URL, "linux", "x86").is_err());
    }
}
