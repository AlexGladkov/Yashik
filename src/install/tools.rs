use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, TryRecvError};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Deserialize;

use super::api::{InstallResult, InstalledTool, InstalledToolVersion, Outcome, Paths};
use super::engine::{self, RunReport};
use super::{state, util};
use crate::effective::EffectiveHerdr;

const METADATA_URL: &str = "https://herdr.dev/latest.json";
const METADATA_FALLBACK_URL: &str =
    "https://raw.githubusercontent.com/herdrdev/herdr/master/distribution/latest.json";
const MAX_METADATA_BYTES: u64 = 2 * 1024 * 1024;
const MAX_BINARY_BYTES: u64 = 256 * 1024 * 1024;
const METADATA_TIMEOUT: Duration = Duration::from_secs(20);
const BINARY_TIMEOUT: Duration = Duration::from_secs(120);
const TOOL_ID: &str = "herdr";
const OPERATION_ID: &str = "tool/herdr";
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, PartialEq, Eq)]
enum DownloadError {
    Transport(String),
    Local(String),
}

type DownloadResult<T> = Result<T, DownloadError>;

impl DownloadError {
    fn message(self) -> String {
        match self {
            Self::Transport(message) | Self::Local(message) => message,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ResolvedRelease {
    version: String,
    asset_url: String,
    sha256: String,
}

#[derive(Deserialize)]
struct Registry {
    version: String,
    #[serde(default)]
    assets: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    sha256: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    releases: std::collections::BTreeMap<String, RegistryRelease>,
}

#[derive(Deserialize)]
struct RegistryRelease {
    #[serde(default)]
    assets: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    sha256: std::collections::BTreeMap<String, String>,
}

pub fn reconcile(
    paths: &Paths,
    desired: Option<&EffectiveHerdr>,
    installer_state: &mut state::State,
    report: &mut RunReport,
) -> InstallResult<()> {
    reconcile_with_confirmation(
        paths,
        desired,
        installer_state,
        report,
        &mut confirm_removal,
    )
}

fn reconcile_with_confirmation(
    paths: &Paths,
    desired: Option<&EffectiveHerdr>,
    installer_state: &mut state::State,
    report: &mut RunReport,
    confirm: &mut dyn FnMut(&str) -> bool,
) -> InstallResult<()> {
    let mut fetcher = fetch;
    reconcile_with_confirmation_using(
        paths,
        desired,
        installer_state,
        report,
        confirm,
        &mut fetcher,
    )
}

fn reconcile_with_confirmation_using(
    paths: &Paths,
    desired: Option<&EffectiveHerdr>,
    installer_state: &mut state::State,
    report: &mut RunReport,
    confirm: &mut dyn FnMut(&str) -> bool,
    fetcher: &mut impl FnMut(&str, u64) -> DownloadResult<Vec<u8>>,
) -> InstallResult<()> {
    match desired {
        Some(desired) => install_desired_using(paths, desired, installer_state, report, fetcher),
        None => remove_if_obsolete(paths, installer_state, report, confirm),
    }
}

fn install_desired_using(
    paths: &Paths,
    desired: &EffectiveHerdr,
    installer_state: &mut state::State,
    report: &mut RunReport,
    fetcher: &mut impl FnMut(&str, u64) -> DownloadResult<Vec<u8>>,
) -> InstallResult<()> {
    let previous = installer_state.tools.get(TOOL_ID).cloned();
    let preflight = match previous.as_ref() {
        Some(record) => verify_record(paths, record),
        None => check_unmanaged_active_path(paths),
    };
    if let Err(error) = preflight {
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

    engine::begin_operation(paths, installer_state, OPERATION_ID, None)?;
    match resolve_and_install_using(paths, &desired.version, previous.as_ref(), fetcher) {
        Ok(InstallAttempt::Unchanged) => engine::finish_operation(
            paths,
            installer_state,
            report,
            OPERATION_ID,
            None,
            Outcome::Unchanged,
            Some("Herdr is already installed and verified".into()),
            |_| {},
        ),
        Ok(InstallAttempt::Installed { record, rollback }) => {
            let result = engine::finish_operation(
                paths,
                installer_state,
                report,
                OPERATION_ID,
                None,
                Outcome::Succeeded,
                Some(format!("Herdr {} installed and verified", record.version)),
                |next| {
                    next.tools.insert(TOOL_ID.to_owned(), *record);
                },
            );
            if result.is_err() {
                rollback_install(rollback);
            }
            result
        }
        Err(error) => engine::complete_without_mutation(
            paths,
            installer_state,
            report,
            OPERATION_ID,
            None,
            Outcome::Failed,
            Some(error),
        ),
    }
}

#[derive(Debug)]
enum InstallAttempt {
    Unchanged,
    Installed {
        record: Box<InstalledTool>,
        rollback: Rollback,
    },
}

#[derive(Debug)]
struct Rollback {
    active: PathBuf,
    previous_active: Option<Vec<u8>>,
    new_sha256: String,
    new_versioned: Option<PathBuf>,
}

fn resolve_and_install_using(
    paths: &Paths,
    requested: &str,
    previous: Option<&InstalledTool>,
    fetcher: &mut impl FnMut(&str, u64) -> DownloadResult<Vec<u8>>,
) -> InstallResult<InstallAttempt> {
    let metadata = fetch_metadata_using(fetcher)?;
    let release = resolve_metadata(
        &metadata,
        requested,
        std::env::consts::OS,
        std::env::consts::ARCH,
    )?;

    if let Some(previous) = previous {
        if previous.version == release.version && previous.sha256 == release.sha256 {
            return Ok(InstallAttempt::Unchanged);
        }
    }

    let target = versioned_binary(paths, &release.version);
    let historical = previous.and_then(|record| {
        record.previous_versions.iter().find(|version| {
            version.version == release.version
                && version.sha256 == release.sha256
                && version.executable == target
        })
    });
    let (bytes, reuse_versioned) = if let Some(historical) = historical {
        verify_version_file(paths, historical)?;
        let bytes = util::read_file_checked_bounded(
            &version_root(paths, &release.version),
            &target,
            MAX_BINARY_BYTES,
        )?;
        (bytes, true)
    } else {
        (
            fetcher(&release.asset_url, MAX_BINARY_BYTES).map_err(DownloadError::message)?,
            false,
        )
    };

    install_verified_bytes(paths, &release, bytes, previous, reuse_versioned)
}

fn fetch_metadata_using(
    fetcher: &mut impl FnMut(&str, u64) -> DownloadResult<Vec<u8>>,
) -> InstallResult<Vec<u8>> {
    match fetcher(METADATA_URL, MAX_METADATA_BYTES) {
        Ok(metadata) => Ok(metadata),
        Err(DownloadError::Local(error)) => Err(error),
        Err(DownloadError::Transport(primary_error)) => {
            match fetcher(METADATA_FALLBACK_URL, MAX_METADATA_BYTES) {
                Ok(metadata) => Ok(metadata),
                Err(DownloadError::Transport(fallback_error)) => Err(format!(
                    "Herdr metadata download failed from both official sources (primary: {primary_error}; fallback: {fallback_error})"
                )),
                Err(DownloadError::Local(error)) => Err(format!(
                    "Herdr metadata fallback failed locally after the primary transport error: {error}"
                )),
            }
        }
    }
}

fn install_verified_bytes(
    paths: &Paths,
    release: &ResolvedRelease,
    executable_bytes: Vec<u8>,
    previous: Option<&InstalledTool>,
    reuse_versioned: bool,
) -> InstallResult<InstallAttempt> {
    if util::sha256_bytes(&executable_bytes) != release.sha256 {
        return Err("Herdr asset checksum does not match official metadata".into());
    }
    let target = versioned_binary(paths, &release.version);
    let mut versioned_created = false;
    if reuse_versioned {
        let prior = previous
            .and_then(|record| {
                record.previous_versions.iter().find(|version| {
                    version.version == release.version
                        && version.sha256 == release.sha256
                        && version.executable == target
                })
            })
            .ok_or_else(|| "recorded Herdr release history is missing".to_owned())?;
        verify_version_file(paths, prior)?;
        probe_version(&target, &release.version)?;
    } else {
        if path_exists(&target)? {
            return Err("Herdr version directory contains an unowned binary; remove it manually before retrying".into());
        }
        let version_dir = version_root(paths, &release.version);
        util::private_dir(&version_dir)?;
        util::atomic_write_if_unchanged(&target, &executable_bytes, 0o755, None)?;
        versioned_created = true;
        if let Err(error) = probe_version(&target, &release.version) {
            let _ = util::remove_file_checked(&target, Some(&release.sha256));
            let _ = fs::remove_dir(&version_dir);
            return Err(error);
        }
    }

    let expected_active = previous.map(|record| record.fingerprint.as_str());
    let active = paths.bin.join("herdr");
    let old_active = match previous {
        Some(record) => Some(util::read_file_checked_bounded(
            &paths.bin,
            &record.executable,
            MAX_BINARY_BYTES,
        )?),
        None => None,
    };
    if let Err(error) =
        util::atomic_write_if_unchanged(&active, &executable_bytes, 0o755, expected_active)
    {
        if versioned_created {
            let _ = util::remove_file_checked(&target, Some(&release.sha256));
            let _ = fs::remove_dir(version_root(paths, &release.version));
        }
        return Err(format!("could not safely activate Herdr: {error}"));
    }

    let mut previous_versions = previous
        .map(|record| record.previous_versions.clone())
        .unwrap_or_default();
    if let Some(old) = previous {
        let old_version = InstalledToolVersion {
            version: old.version.clone(),
            asset_url: old.asset_url.clone(),
            sha256: old.sha256.clone(),
            executable: old.versioned_executable.clone(),
        };
        if !previous_versions
            .iter()
            .any(|version| version.version == old_version.version)
        {
            previous_versions.push(old_version);
        }
    }
    previous_versions.retain(|version| version.version != release.version);

    let record = InstalledTool {
        id: TOOL_ID.to_owned(),
        version: release.version.clone(),
        metadata_url: METADATA_URL.to_owned(),
        asset_url: release.asset_url.clone(),
        sha256: release.sha256.clone(),
        versioned_executable: target.clone(),
        executable: active.clone(),
        fingerprint: release.sha256.clone(),
        previous_versions,
    };
    Ok(InstallAttempt::Installed {
        record: Box::new(record),
        rollback: Rollback {
            active,
            previous_active: old_active,
            new_sha256: release.sha256.clone(),
            new_versioned: versioned_created.then_some(target),
        },
    })
}

fn rollback_install(rollback: Rollback) {
    match rollback.previous_active {
        Some(previous) => {
            let _ = util::atomic_write_if_unchanged(
                &rollback.active,
                &previous,
                0o755,
                Some(&rollback.new_sha256),
            );
        }
        None => {
            let _ = util::remove_file_checked(&rollback.active, Some(&rollback.new_sha256));
        }
    }
    if let Some(versioned) = rollback.new_versioned {
        let _ = util::remove_file_checked(&versioned, Some(&rollback.new_sha256));
        if let Some(parent) = versioned.parent() {
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
                "Herdr removal is pending because an installed file drifted: {error}"
            )),
        );
    }
    if !confirm(&format!(
        "Remove managed Herdr {} and its recorded versioned binaries? [y/N]: ",
        record.version
    )) {
        return engine::complete_without_mutation(
            paths,
            installer_state,
            report,
            OPERATION_ID,
            None,
            Outcome::PendingRemoval,
            Some("managed Herdr remains installed; removal was not confirmed".into()),
        );
    }

    engine::begin_operation(paths, installer_state, OPERATION_ID, None)?;
    let removal = remove_recorded_files(&record);
    match removal {
        Ok(()) => engine::finish_operation(
            paths,
            installer_state,
            report,
            OPERATION_ID,
            None,
            Outcome::Succeeded,
            Some(
                "recorded Herdr binaries removed; user configuration and sessions were retained"
                    .into(),
            ),
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
            Some(error),
        ),
    }
}

fn remove_recorded_files(record: &InstalledTool) -> InstallResult<()> {
    for version in &record.previous_versions {
        util::remove_file_checked(&version.executable, Some(&version.sha256))?;
        if let Some(parent) = version.executable.parent() {
            let _ = fs::remove_dir(parent);
        }
    }
    util::remove_file_checked(&record.versioned_executable, Some(&record.sha256))?;
    util::remove_file_checked(&record.executable, Some(&record.fingerprint))?;
    if let Some(parent) = record.versioned_executable.parent() {
        let _ = fs::remove_dir(parent);
    }
    Ok(())
}

fn confirm_removal(question: &str) -> bool {
    eprint!("{question}");
    let _ = std::io::stderr().flush();
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer).is_ok()
        && matches!(answer.trim(), "y" | "Y" | "yes" | "Yes")
}

fn check_unmanaged_active_path(paths: &Paths) -> InstallResult<()> {
    match fs::symlink_metadata(paths.bin.join("herdr")) {
        Ok(_) => Err(format!(
            "{} already exists and is not recorded as Yashik-managed; preserve it and move it manually before retrying",
            paths.bin.join("herdr").display()
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("could not inspect the Herdr activation path".into()),
    }
}

pub fn verify_record(paths: &Paths, record: &InstalledTool) -> InstallResult<()> {
    if record.id != TOOL_ID
        || record.metadata_url != METADATA_URL
        || !crate::validation::is_valid_exact_harness_version(&record.version)
        || record.fingerprint != record.sha256
        || !valid_sha256(&record.sha256)
        || record.executable != paths.bin.join("herdr")
        || record.versioned_executable != versioned_binary(paths, &record.version)
        || record.asset_url
            != expected_asset_url(
                &record.version,
                platform_key(std::env::consts::OS, std::env::consts::ARCH)?,
            )
    {
        return Err("recorded Herdr ownership paths or metadata are invalid".into());
    }
    verify_file(&record.versioned_executable, &record.sha256)?;
    verify_file(&record.executable, &record.fingerprint)?;
    let mut seen = std::collections::BTreeSet::new();
    for version in &record.previous_versions {
        let key = platform_key(std::env::consts::OS, std::env::consts::ARCH)?;
        if !seen.insert(&version.version)
            || version.version == record.version
            || !crate::validation::is_valid_exact_harness_version(&version.version)
            || !valid_sha256(&version.sha256)
            || version.executable != versioned_binary(paths, &version.version)
            || version.asset_url != expected_asset_url(&version.version, key)
        {
            return Err("recorded Herdr release history is invalid".into());
        }
        verify_file(&version.executable, &version.sha256)?;
    }
    Ok(())
}

pub fn doctor(paths: &Paths, record: &InstalledTool) -> InstallResult<()> {
    verify_record(paths, record)?;
    probe_version(&record.versioned_executable, &record.version)
}

fn verify_version_file(paths: &Paths, record: &InstalledToolVersion) -> InstallResult<()> {
    if !crate::validation::is_valid_exact_harness_version(&record.version)
        || !valid_sha256(&record.sha256)
        || record.executable != versioned_binary(paths, &record.version)
        || record.asset_url
            != expected_asset_url(
                &record.version,
                platform_key(std::env::consts::OS, std::env::consts::ARCH)?,
            )
    {
        return Err("recorded Herdr release history is invalid".into());
    }
    verify_file(&record.executable, &record.sha256)
}

fn verify_file(path: &Path, expected_sha256: &str) -> InstallResult<()> {
    if !path.is_absolute() {
        return Err("recorded Herdr path is not absolute".into());
    }
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| "recorded Herdr binary is missing or unsafe".to_owned())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("recorded Herdr path is not a regular file".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err("recorded Herdr binary is not executable".into());
        }
    }
    if util::sha256_file(path)? != expected_sha256 {
        return Err("recorded Herdr binary has drifted".into());
    }
    Ok(())
}

fn resolve_metadata(
    bytes: &[u8],
    requested: &str,
    os: &str,
    arch: &str,
) -> InstallResult<ResolvedRelease> {
    if requested != "latest" && !crate::validation::is_valid_exact_harness_version(requested) {
        return Err("Herdr version must be `latest` or an exact semantic version".into());
    }
    let registry: Registry = serde_json::from_slice(bytes).map_err(|_| {
        "Herdr release metadata is invalid JSON or has an unsupported shape".to_owned()
    })?;
    let key = platform_key(os, arch)?;
    let (version, assets, checksums): (&str, _, _) = if requested == "latest" {
        (&registry.version, &registry.assets, &registry.sha256)
    } else {
        let release = registry
            .releases
            .get(requested)
            .ok_or_else(|| "requested Herdr release is absent from official metadata".to_owned())?;
        (requested, &release.assets, &release.sha256)
    };
    if !crate::validation::is_valid_exact_harness_version(version)
        || requested != "latest" && requested != version
    {
        return Err("Herdr metadata returned an unexpected version".into());
    }
    let expected_url = expected_asset_url(version, key);
    let asset_url = assets
        .get(key)
        .ok_or_else(|| "official Herdr metadata has no asset for this platform".to_owned())?;
    if asset_url != &expected_url {
        return Err("official Herdr metadata contains an unexpected asset URL".into());
    }
    let checksum = checksums
        .get(key)
        .ok_or_else(|| "official Herdr metadata has no SHA-256 for this asset".to_owned())?;
    if !valid_sha256(checksum) {
        return Err("official Herdr metadata contains an invalid SHA-256".into());
    }
    Ok(ResolvedRelease {
        version: version.to_owned(),
        asset_url: asset_url.clone(),
        sha256: checksum.to_ascii_lowercase(),
    })
}

fn platform_key(os: &str, arch: &str) -> InstallResult<&'static str> {
    let os = match os {
        "linux" => "linux",
        "macos" => "macos",
        _ => return Err("Herdr managed installation supports Linux and macOS only".into()),
    };
    let arch = match arch {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        _ => return Err("Herdr managed installation supports x86_64 and aarch64 only".into()),
    };
    Ok(match (os, arch) {
        ("linux", "x86_64") => "linux-x86_64",
        ("linux", "aarch64") => "linux-aarch64",
        ("macos", "x86_64") => "macos-x86_64",
        ("macos", "aarch64") => "macos-aarch64",
        _ => unreachable!(),
    })
}

fn expected_asset_url(version: &str, key: &str) -> String {
    format!("https://github.com/herdrdev/herdr/releases/download/v{version}/herdr-{key}")
}

fn version_root(paths: &Paths, version: &str) -> PathBuf {
    paths.data.join("tools/herdr/versions").join(version)
}

fn versioned_binary(paths: &Paths, version: &str) -> PathBuf {
    version_root(paths, version).join("herdr")
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn path_exists(path: &Path) -> InstallResult<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err("could not inspect a Herdr managed path".into()),
    }
}

fn fetch(url: &str, max_bytes: u64) -> DownloadResult<Vec<u8>> {
    let (timeout, hard_timeout) = if url == METADATA_URL || url == METADATA_FALLBACK_URL {
        (METADATA_TIMEOUT, METADATA_TIMEOUT + Duration::from_secs(5))
    } else if url.starts_with("https://github.com/herdrdev/herdr/releases/download/v") {
        (BINARY_TIMEOUT, BINARY_TIMEOUT + Duration::from_secs(5))
    } else {
        return Err(DownloadError::Local(
            "refusing a download outside official Herdr HTTPS endpoints".into(),
        ));
    };
    let curl = find_curl()
        .ok_or_else(|| DownloadError::Local("curl is required to install managed Herdr".into()))?;
    let limit = max_bytes.to_string();
    let timeout_seconds = timeout.as_secs().to_string();
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
            &timeout_seconds,
            "--max-filesize",
            &limit,
            "--silent",
            "--show-error",
            url,
        ])
        .stderr(Stdio::null());
    let mut child = command
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|_| DownloadError::Local("could not start curl for the Herdr download".into()))?;
    let (status, output) = bounded_child_output(
        &mut child,
        max_bytes,
        hard_timeout,
        "official Herdr download",
    )?;
    if !status.success() {
        return Err(curl_failure(status));
    }
    Ok(output)
}

fn curl_failure(status: ExitStatus) -> DownloadError {
    match status.code() {
        Some(5 | 6 | 7 | 8 | 16 | 18 | 22 | 28 | 35 | 47 | 52 | 55 | 56 | 61 | 62 | 92 | 95) => {
            DownloadError::Transport("official Herdr download failed".into())
        }
        // A known content length above the cap and unrecognized curl failures
        // are treated as local/protective failures, so they cannot trigger a
        // second request.
        Some(63) => {
            DownloadError::Local("official Herdr download exceeds the supported size limit".into())
        }
        Some(_) => DownloadError::Local("official Herdr download failed locally".into()),
        None => DownloadError::Local("official Herdr download process was terminated".into()),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BoundedReadError {
    LimitExceeded,
    ReadFailed,
    AllocationFailed,
}

fn bounded_child_output(
    child: &mut Child,
    max_bytes: u64,
    timeout: Duration,
    operation: &str,
) -> DownloadResult<(ExitStatus, Vec<u8>)> {
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| DownloadError::Local(format!("{operation} has no captured stdout")))?;
    let max_bytes = usize::try_from(max_bytes).map_err(|_| {
        DownloadError::Local(format!(
            "{operation} size limit is unsupported on this platform"
        ))
    })?;
    let (sender, receiver) = mpsc::channel();
    let output_reader = thread::Builder::new()
        .spawn(move || {
            let mut stdout = stdout;
            let result = read_bounded_stream(&mut stdout, max_bytes);
            let _ = sender.send(result);
        })
        .map_err(|_| DownloadError::Local(format!("could not capture {operation} response")))?;

    let deadline = Instant::now() + timeout;
    let mut output = None;
    let mut status = None;
    loop {
        if output.is_none() {
            match receiver.try_recv() {
                Ok(Ok(bytes)) => output = Some(bytes),
                Ok(Err(reason)) => {
                    stop_child(child);
                    let _ = output_reader.join();
                    return Err(DownloadError::Local(match reason {
                        BoundedReadError::LimitExceeded => {
                            format!("{operation} exceeds the supported size limit")
                        }
                        BoundedReadError::ReadFailed => {
                            format!("could not read {operation} response")
                        }
                        BoundedReadError::AllocationFailed => {
                            format!("could not allocate memory for {operation} response")
                        }
                    }));
                }
                Err(TryRecvError::Disconnected) => {
                    stop_child(child);
                    let _ = output_reader.join();
                    return Err(DownloadError::Local(format!(
                        "could not capture {operation} response"
                    )));
                }
                Err(TryRecvError::Empty) => {}
            }
        }

        if status.is_none() {
            match child.try_wait() {
                Ok(Some(exit_status)) => status = Some(exit_status),
                Ok(None) => {}
                Err(_) => {
                    stop_child(child);
                    let _ = output_reader.join();
                    return Err(DownloadError::Local(format!(
                        "could not wait for {operation} process"
                    )));
                }
            }
        }

        if output.is_some() && status.is_some() {
            break;
        }
        if Instant::now() >= deadline {
            stop_child(child);
            let _ = output_reader.join();
            return Err(DownloadError::Transport(format!("{operation} timed out")));
        }
        thread::sleep(Duration::from_millis(20));
    }

    output_reader
        .join()
        .map_err(|_| DownloadError::Local(format!("{operation} output reader failed")))?;
    Ok((
        status.expect("checked above"),
        output.expect("checked above"),
    ))
}

fn stop_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn read_bounded_stream<R: Read>(
    reader: &mut R,
    max_bytes: usize,
) -> Result<Vec<u8>, BoundedReadError> {
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|_| BoundedReadError::ReadFailed)?;
        if count == 0 {
            return Ok(bytes);
        }
        let remaining = max_bytes.saturating_sub(bytes.len());
        if count > remaining {
            return Err(BoundedReadError::LimitExceeded);
        }
        bytes
            .try_reserve_exact(count)
            .map_err(|_| BoundedReadError::AllocationFailed)?;
        bytes.extend_from_slice(&buffer[..count]);
    }
}

fn find_curl() -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|directory| directory.join("curl"))
        .find(|path| path.is_file())
        .and_then(|path| fs::canonicalize(&path).ok().or(Some(path)))
}

fn probe_version(executable: &Path, expected: &str) -> InstallResult<()> {
    let home = TemporaryProbeHome::create()?;
    let mut child = Command::new(executable)
        .current_dir(home.path())
        .arg("--version")
        .env_clear()
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("XDG_CONFIG_HOME", home.path().join(".config"))
        .env("XDG_CACHE_HOME", home.path().join(".cache"))
        .env("XDG_DATA_HOME", home.path().join(".local/share"))
        .env("XDG_STATE_HOME", home.path().join(".local/state"))
        .env("PATH", "/usr/bin:/bin:/usr/local/bin:/opt/homebrew/bin")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| "could not start the staged Herdr --version probe".to_owned())?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdout_reader = thread::spawn(move || read_limited(stdout, 4096));
    let stderr_reader = thread::spawn(move || discard_output(stderr));
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err("Herdr --version probe timed out".into());
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err("Herdr --version probe could not be completed".into());
            }
        }
    };
    let stdout = stdout_reader
        .join()
        .map_err(|_| "Herdr version output capture failed".to_owned())?;
    let _ = stderr_reader.join();
    if !status.success() {
        return Err("Herdr --version probe failed".into());
    }
    let actual = std::str::from_utf8(&stdout)
        .map_err(|_| "Herdr --version returned invalid text".to_owned())?
        .trim();
    if actual != format!("herdr {expected}") {
        return Err("Herdr --version did not match official metadata".into());
    }
    Ok(())
}

struct TemporaryProbeHome(PathBuf);

impl TemporaryProbeHome {
    fn create() -> InstallResult<Self> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let sequence = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "yashik-herdr-probe-{}-{nonce}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path)
            .map_err(|_| "could not prepare an isolated Herdr version probe".to_owned())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).is_err() {
                let _ = fs::remove_dir_all(&path);
                return Err("could not secure an isolated Herdr version probe".into());
            }
        }
        for directory in [".config", ".cache", ".local/share", ".local/state"] {
            if fs::create_dir_all(path.join(directory)).is_err() {
                let _ = fs::remove_dir_all(&path);
                return Err("could not prepare an isolated Herdr version probe".into());
            }
        }
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TemporaryProbeHome {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn read_limited<R: Read>(reader: Option<R>, limit: usize) -> Vec<u8> {
    let Some(mut reader) = reader else {
        return Vec::new();
    };
    let mut result = Vec::new();
    let mut buffer = [0u8; 4096];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(count) if result.len() < limit => {
                let retained = count.min(limit - result.len());
                result.extend_from_slice(&buffer[..retained]);
            }
            Ok(_) => {}
        }
    }
    result
}

fn discard_output<R: Read>(reader: Option<R>) {
    let Some(mut reader) = reader else {
        return;
    };
    let mut buffer = [0u8; 4096];
    while reader.read(&mut buffer).is_ok_and(|count| count > 0) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    struct TempTree(PathBuf);

    impl TempTree {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "yashik-herdr-tools-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(path.join("home")).unwrap();
            let path = fs::canonicalize(path).unwrap();
            Self(path)
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

    fn metadata(version: &str, platform: &str, asset: &str, checksum: &str) -> Vec<u8> {
        serde_json::json!({
            "version": version,
            "assets": { platform: asset },
            "sha256": { platform: checksum },
            "releases": {
                "1.2.3": {
                    "assets": { platform: "https://github.com/herdrdev/herdr/releases/download/v1.2.3/herdr-linux-x86_64" },
                    "sha256": { platform: checksum }
                }
            }
        })
        .to_string()
        .into_bytes()
    }

    fn fixture_release(version: &str) -> (Vec<u8>, Vec<u8>, ResolvedRelease) {
        let key = platform_key(std::env::consts::OS, std::env::consts::ARCH).unwrap();
        let bytes = format!("#!/bin/sh\nprintf 'herdr {version}\\n'\n").into_bytes();
        let checksum = util::sha256_bytes(&bytes);
        let asset_url = expected_asset_url(version, key);
        let metadata = serde_json::json!({
            "version": version,
            "assets": { key: asset_url },
            "sha256": { key: checksum },
            "releases": {
                version: {
                    "assets": { key: expected_asset_url(version, key) },
                    "sha256": { key: checksum }
                }
            }
        })
        .to_string()
        .into_bytes();
        let release = ResolvedRelease {
            version: version.to_owned(),
            asset_url: expected_asset_url(version, key),
            sha256: checksum,
        };
        (metadata, bytes, release)
    }

    #[test]
    fn official_metadata_resolves_latest_and_exact_pin_and_rejects_untrusted_values() {
        let asset = "https://github.com/herdrdev/herdr/releases/download/v2.0.0/herdr-linux-x86_64";
        let hash = "ab".repeat(32);
        let valid = metadata("2.0.0", "linux-x86_64", asset, &hash);
        let latest = resolve_metadata(&valid, "latest", "linux", "x86_64").unwrap();
        assert_eq!(latest.version, "2.0.0");
        assert_eq!(latest.sha256, hash);

        let pin = resolve_metadata(&valid, "1.2.3", "linux", "x86_64").unwrap();
        assert_eq!(pin.version, "1.2.3");
        assert!(resolve_metadata(&valid, "3.0.0", "linux", "x86_64").is_err());
        assert!(resolve_metadata(&valid, "latest", "freebsd", "x86_64").is_err());
        assert!(resolve_metadata(&valid, "latest", "linux", "x86").is_err());

        let missing_checksum = metadata("2.0.0", "linux-x86_64", asset, "");
        assert!(resolve_metadata(&missing_checksum, "latest", "linux", "x86_64").is_err());
        let unexpected_asset =
            metadata("2.0.0", "linux-x86_64", "https://example.com/herdr", &hash);
        assert!(resolve_metadata(&unexpected_asset, "latest", "linux", "x86_64").is_err());
        assert!(resolve_metadata(b"{", "latest", "linux", "x86_64").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn primary_metadata_transport_failure_uses_fixed_mirror_and_keeps_canonical_identity() {
        let tree = TempTree::new();
        let paths = tree.paths();
        util::private_dir(&paths.bin).unwrap();
        let (metadata, binary, release) = fixture_release("1.2.3");
        let (attempt, requested) = {
            let mut requested = Vec::new();
            let mut fetcher = |url: &str, max_bytes| {
                requested.push((url.to_owned(), max_bytes));
                match url {
                    METADATA_URL => Err(DownloadError::Transport("request timed out".into())),
                    METADATA_FALLBACK_URL => Ok(metadata.clone()),
                    asset if asset == release.asset_url => Ok(binary.clone()),
                    _ => panic!("unexpected Herdr download URL: {url}"),
                }
            };
            let attempt = resolve_and_install_using(&paths, "1.2.3", None, &mut fetcher).unwrap();
            (attempt, requested)
        };
        let InstallAttempt::Installed { record, .. } = attempt else {
            panic!("mirror metadata should permit installation");
        };

        assert_eq!(
            requested,
            [
                (METADATA_URL.to_owned(), MAX_METADATA_BYTES),
                (METADATA_FALLBACK_URL.to_owned(), MAX_METADATA_BYTES),
                (release.asset_url, MAX_BINARY_BYTES),
            ]
        );
        assert_eq!(record.metadata_url, METADATA_URL);
        verify_record(&paths, &record).unwrap();
    }

    #[test]
    fn both_metadata_transport_failures_are_reported_without_asset_download() {
        let mut requested = Vec::new();
        let mut fetcher = |url: &str, max_bytes| {
            requested.push((url.to_owned(), max_bytes));
            Err(DownloadError::Transport("connection failed".into()))
        };

        let error = fetch_metadata_using(&mut fetcher).unwrap_err();

        assert!(error.contains("both official sources"));
        assert_eq!(
            requested,
            [
                (METADATA_URL.to_owned(), MAX_METADATA_BYTES),
                (METADATA_FALLBACK_URL.to_owned(), MAX_METADATA_BYTES),
            ]
        );
    }

    #[test]
    fn complete_but_invalid_primary_metadata_does_not_use_mirror() {
        let mut requested = Vec::new();
        let mut fetcher = |url: &str, max_bytes| {
            requested.push((url.to_owned(), max_bytes));
            match url {
                METADATA_URL => Ok(b"{\"version\":\"invalid\"}".to_vec()),
                METADATA_FALLBACK_URL => panic!("semantic errors must not try the mirror"),
                _ => panic!("unexpected Herdr download URL: {url}"),
            }
        };

        let error =
            resolve_and_install_using(&TempTree::new().paths(), "latest", None, &mut fetcher)
                .unwrap_err();

        assert!(error.contains("metadata returned an unexpected version"));
        assert_eq!(requested, [(METADATA_URL.to_owned(), MAX_METADATA_BYTES)]);
    }

    #[test]
    fn local_size_or_process_failures_do_not_use_metadata_mirror() {
        for message in [
            "response exceeds size cap",
            "curl could not start",
            "out of memory",
        ] {
            let mut requested = Vec::new();
            let mut fetcher = |url: &str, max_bytes| {
                requested.push((url.to_owned(), max_bytes));
                Err(DownloadError::Local(message.into()))
            };

            let error = fetch_metadata_using(&mut fetcher).unwrap_err();

            assert_eq!(error, message);
            assert_eq!(requested, [(METADATA_URL.to_owned(), MAX_METADATA_BYTES)]);
        }
    }

    #[cfg(unix)]
    #[test]
    fn failed_install_claims_no_ownership_and_retry_can_install_from_mirror() {
        let tree = TempTree::new();
        let paths = tree.paths();
        util::private_dir(&paths.bin).unwrap();
        let desired = EffectiveHerdr {
            version: "latest".into(),
        };
        let mut installer_state = state::State::empty();
        let mut report = RunReport::default();
        let mut confirm = |_: &str| false;
        let mut failed_fetcher = |url: &str, _| match url {
            METADATA_URL | METADATA_FALLBACK_URL => {
                Err(DownloadError::Transport("connection failed".into()))
            }
            _ => panic!("metadata failures must stop before the asset download"),
        };

        reconcile_with_confirmation_using(
            &paths,
            Some(&desired),
            &mut installer_state,
            &mut report,
            &mut confirm,
            &mut failed_fetcher,
        )
        .unwrap();

        assert!(installer_state.tools.is_empty());
        assert_eq!(report.operations[0].outcome, Outcome::Failed);
        assert!(!paths.bin.join("herdr").exists());

        let (metadata, binary, release) = fixture_release("1.2.3");
        let mut retry_fetcher = |url: &str, _| match url {
            METADATA_URL => Err(DownloadError::Transport("request timed out".into())),
            METADATA_FALLBACK_URL => Ok(metadata.clone()),
            asset if asset == release.asset_url => Ok(binary.clone()),
            _ => panic!("unexpected Herdr download URL: {url}"),
        };
        reconcile_with_confirmation_using(
            &paths,
            Some(&desired),
            &mut installer_state,
            &mut report,
            &mut confirm,
            &mut retry_fetcher,
        )
        .unwrap();

        assert_eq!(report.operations[1].outcome, Outcome::Succeeded);
        let record = installer_state.tools.get(TOOL_ID).unwrap();
        assert_eq!(record.metadata_url, METADATA_URL);
        verify_record(&paths, record).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn curl_classifies_http_transfer_errors_separately_from_local_failures() {
        for (code, expected_transport) in [
            (22, true),
            (18, true),
            (23, false),
            (27, false),
            (63, false),
            (126, false),
        ] {
            let status = Command::new("sh")
                .args(["-c", &format!("exit {code}")])
                .status()
                .unwrap();
            let failure = curl_failure(status);

            assert_eq!(
                matches!(&failure, DownloadError::Transport(_)),
                expected_transport,
                "curl exit code {code} was classified incorrectly"
            );
            if code == 63 {
                assert!(
                    matches!(&failure, DownloadError::Local(message) if message.contains("size limit"))
                );
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn unknown_length_stream_over_limit_is_killed_and_reaped() {
        let mut child = Command::new("/bin/sh")
            .args(["-c", "while :; do printf '0123456789abcdef'; done"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let started = Instant::now();

        let error =
            bounded_child_output(&mut child, 4096, Duration::from_secs(5), "fixture download")
                .unwrap_err();

        assert!(
            matches!(error, DownloadError::Local(message) if message.contains("exceeds the supported size limit"))
        );
        assert!(
            child.try_wait().unwrap().is_some(),
            "overflow child was not reaped"
        );
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[cfg(unix)]
    #[test]
    fn installation_fixture_checks_checksum_before_activation_and_probes_exact_version() {
        let tree = TempTree::new();
        let paths = tree.paths();
        util::private_dir(&paths.bin).unwrap();
        let version = "1.2.3";
        let key = platform_key(std::env::consts::OS, std::env::consts::ARCH).unwrap();
        let asset_url = expected_asset_url(version, key);
        let bytes = b"#!/bin/sh\nprintf 'herdr 1.2.3\\n'\n";
        let hash = util::sha256_bytes(bytes);
        let release = ResolvedRelease {
            version: version.into(),
            asset_url,
            sha256: hash.clone(),
        };
        let bad_checksum = ResolvedRelease {
            sha256: "00".repeat(32),
            ..release.clone()
        };
        assert!(
            install_verified_bytes(&paths, &bad_checksum, bytes.to_vec(), None, false).is_err()
        );
        assert!(!paths.bin.join("herdr").exists());
        assert!(!versioned_binary(&paths, version).exists());

        let wrong_version = ResolvedRelease {
            version: "1.2.4".into(),
            asset_url: expected_asset_url("1.2.4", key),
            sha256: hash.clone(),
        };
        assert!(
            install_verified_bytes(&paths, &wrong_version, bytes.to_vec(), None, false).is_err()
        );
        assert!(!paths.bin.join("herdr").exists());
        assert!(!versioned_binary(&paths, "1.2.4").exists());

        let InstallAttempt::Installed { record, .. } =
            install_verified_bytes(&paths, &release, bytes.to_vec(), None, false).unwrap()
        else {
            panic!("first install must produce a managed Herdr record");
        };
        verify_record(&paths, &record).unwrap();
        let active = paths.bin.join("herdr");
        assert_eq!(fs::read(&active).unwrap(), bytes);
        assert_eq!(hash, util::sha256_file(&active).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn pinned_repeat_resolves_metadata_but_preserves_active_inode_and_mtime() {
        use std::os::unix::fs::MetadataExt;

        let tree = TempTree::new();
        let paths = tree.paths();
        util::private_dir(&paths.bin).unwrap();
        let (metadata, binary, _) = fixture_release("1.2.3");
        let mut first_fetcher = |url: &str, _| {
            if url == METADATA_URL {
                Ok(metadata.clone())
            } else {
                Ok(binary.clone())
            }
        };
        let first = resolve_and_install_using(&paths, "1.2.3", None, &mut first_fetcher).unwrap();
        let InstallAttempt::Installed { record, .. } = first else {
            panic!("first install should create binaries");
        };
        let active = fs::metadata(&record.executable).unwrap();

        let mut repeat_fetcher = |url: &str, _| {
            assert_eq!(
                url, METADATA_URL,
                "a pinned repeat must not fetch the asset"
            );
            Ok(metadata.clone())
        };
        let repeat =
            resolve_and_install_using(&paths, "1.2.3", Some(&record), &mut repeat_fetcher).unwrap();
        assert!(matches!(repeat, InstallAttempt::Unchanged));

        let after = fs::metadata(&record.executable).unwrap();
        assert_eq!(active.ino(), after.ino());
        assert_eq!(active.modified().unwrap(), after.modified().unwrap());
    }

    #[test]
    fn unmanaged_activation_collision_is_preserved_and_not_claimed() {
        let tree = TempTree::new();
        let paths = tree.paths();
        util::private_dir(&paths.bin).unwrap();
        let active = paths.bin.join("herdr");
        fs::write(&active, b"user-owned binary").unwrap();
        let mut installer_state = state::State::empty();
        let mut report = RunReport::default();

        reconcile_with_confirmation(
            &paths,
            Some(&EffectiveHerdr {
                version: "latest".into(),
            }),
            &mut installer_state,
            &mut report,
            &mut |_| false,
        )
        .unwrap();

        assert_eq!(fs::read(active).unwrap(), b"user-owned binary");
        assert!(installer_state.tools.is_empty());
        assert_eq!(report.operations[0].outcome, Outcome::Failed);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_activation_collision_is_preserved_and_not_claimed() {
        use std::os::unix::fs::symlink;

        let tree = TempTree::new();
        let paths = tree.paths();
        util::private_dir(&paths.bin).unwrap();
        let outside = tree.0.join("outside-binary");
        fs::write(&outside, b"user-owned binary").unwrap();
        symlink(&outside, paths.bin.join("herdr")).unwrap();
        let mut installer_state = state::State::empty();
        let mut report = RunReport::default();

        reconcile_with_confirmation(
            &paths,
            Some(&EffectiveHerdr {
                version: "latest".into(),
            }),
            &mut installer_state,
            &mut report,
            &mut |_| false,
        )
        .unwrap();

        assert_eq!(fs::read(&outside).unwrap(), b"user-owned binary");
        assert!(installer_state.tools.is_empty());
        assert_eq!(report.operations[0].outcome, Outcome::Failed);
    }

    #[cfg(unix)]
    #[test]
    fn removal_without_confirmation_retains_verified_binaries_and_state() {
        let tree = TempTree::new();
        let paths = tree.paths();
        util::private_dir(&paths.bin).unwrap();
        let (metadata, binary, _) = fixture_release("1.2.3");
        let mut fetcher = |url: &str, _| {
            if url == METADATA_URL {
                Ok(metadata.clone())
            } else {
                Ok(binary.clone())
            }
        };
        let attempt = resolve_and_install_using(&paths, "1.2.3", None, &mut fetcher).unwrap();
        let InstallAttempt::Installed { record, .. } = attempt else {
            panic!("fixture installation must produce a managed record");
        };
        let active_bytes = fs::read(&record.executable).unwrap();
        let mut installer_state = state::State::empty();
        installer_state
            .tools
            .insert(TOOL_ID.into(), (*record).clone());
        let mut report = RunReport::default();

        reconcile_with_confirmation(&paths, None, &mut installer_state, &mut report, &mut |_| {
            false
        })
        .unwrap();

        assert_eq!(report.operations[0].outcome, Outcome::PendingRemoval);
        assert_eq!(fs::read(&record.executable).unwrap(), active_bytes);
        assert!(installer_state.tools.contains_key(TOOL_ID));
    }

    #[cfg(unix)]
    #[test]
    fn doctor_detects_active_binary_tampering_without_network_access() {
        let tree = TempTree::new();
        let paths = tree.paths();
        util::private_dir(&paths.data).unwrap();
        util::private_dir(&paths.bin).unwrap();
        let version = "1.2.3";
        let asset_url = expected_asset_url(version, "linux-x86_64");
        let bytes = b"#!/bin/sh\nprintf 'herdr 1.2.3\\n'\n";
        let hash = util::sha256_bytes(bytes);
        let target = versioned_binary(&paths, version);
        util::private_dir(&version_root(&paths, version)).unwrap();
        util::atomic_write_if_unchanged(&target, bytes, 0o755, None).unwrap();
        let active = paths.bin.join("herdr");
        util::atomic_write_if_unchanged(&active, bytes, 0o755, None).unwrap();
        let record = InstalledTool {
            id: TOOL_ID.into(),
            version: version.into(),
            metadata_url: METADATA_URL.into(),
            asset_url,
            sha256: hash.clone(),
            versioned_executable: target,
            executable: active.clone(),
            fingerprint: hash,
            previous_versions: Vec::new(),
        };
        doctor(&paths, &record).unwrap();
        fs::write(&active, b"tampered").unwrap();
        assert!(doctor(&paths, &record).is_err());
    }
}
