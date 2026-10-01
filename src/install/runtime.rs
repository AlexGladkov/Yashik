use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use super::api::{BootstrapRecipe, InstallResult, InstalledCli, Paths, RuntimeEnv};
use super::util::{
    atomic_write, atomic_write_if_unchanged, hash_tree, private_dir, read_file_checked,
    read_file_checked_bounded, remove_file_checked, remove_tree_checked, sha256_bytes, sha256_file,
};
use crate::schema::HarnessId;

const NODE_VERSION: &str = "22.22.0";
const NODE_MINIMUM: (u64, u64, u64) = (22, 19, 0);
const BUN_VERSION: &str = "1.4.2";
const BUN_MINIMUM: (u64, u64, u64) = (1, 3, 14);
const CAPTURE_LIMIT: usize = 64 * 1024;

#[derive(Clone, Copy)]
enum RuntimeKind {
    Node,
    Bun,
}

#[derive(Clone, Copy)]
struct CliRecipe {
    package: &'static str,
    executable: &'static str,
    runtime: RuntimeKind,
    ignore_scripts: bool,
}

pub fn ensure(paths: &Paths, requirement: &str) -> InstallResult<RuntimeEnv> {
    match requirement {
        "node" => ensure_node(paths),
        "bun" => ensure_bun(paths),
        _ => Err("runtime requirement is not in the closed registry".to_owned()),
    }
}

pub fn install_cli(
    paths: &Paths,
    harness: HarnessId,
    requested_version: Option<&str>,
) -> InstallResult<InstalledCli> {
    install_cli_checked(paths, harness, requested_version, None)
}

pub fn install_cli_checked(
    paths: &Paths,
    harness: HarnessId,
    requested_version: Option<&str>,
    approved_existing_hash: Option<&str>,
) -> InstallResult<InstalledCli> {
    let recipe = cli_recipe(harness);
    let is_latest = requested_version.is_none() || requested_version == Some("latest");
    if !is_latest && !requested_version.is_some_and(valid_exact_version) {
        return Err("harness version must be latest or an exact package version".to_owned());
    }
    let runtime = ensure(
        paths,
        match recipe.runtime {
            RuntimeKind::Node => "node",
            RuntimeKind::Bun => "bun",
        },
    )?;
    let harness_root = paths.data.join("harnesses").join(harness.as_str());
    private_dir(&harness_root)?;
    let mut resolved = if is_latest {
        Some(resolve_package_version(recipe.package, "latest")?)
    } else {
        let version = requested_version.expect("exact version was supplied");
        if !valid_exact_version(version) {
            return Err("harness version must be latest or an exact package version".to_owned());
        }
        None
    };
    let version = resolved
        .as_ref()
        .map(|package| package.version.clone())
        .or_else(|| requested_version.map(str::to_owned))
        .expect("latest or exact version is available");
    let prefix = harness_root.join(&version);
    let package_json = package_json_path(&prefix, recipe.package, recipe.runtime);

    if package_version(&package_json).as_deref() == Some(version.as_str()) {
        let internal_executable = prefix.join("bin").join(recipe.executable);
        if probe_cli_version(&internal_executable, &runtime, &version).is_ok() {
            if resolved.is_none() {
                resolved =
                    cached_integrity(&harness_root, &version).map(|integrity| ResolvedPackage {
                        version: version.clone(),
                        integrity,
                    });
            }
            if resolved.is_none() {
                resolved = Some(resolve_package_version(recipe.package, &version)?);
            }
            let package = resolved.expect("package metadata was resolved");
            write_integrity_record(&harness_root, &version, &package.integrity)?;
            let (launcher, launcher_fingerprint) = install_launcher(
                paths,
                recipe.executable,
                &internal_executable,
                &runtime.bin_dirs,
                approved_existing_hash,
            )?;
            return Ok(InstalledCli {
                harness,
                version,
                executable: launcher,
                integrity: Some(package.integrity),
                runtime_bins: runtime.bin_dirs,
                launcher_fingerprint: Some(launcher_fingerprint),
            });
        }
    }

    if resolved.is_none() {
        resolved = Some(resolve_package_version(recipe.package, &version)?);
    }
    let package = resolved.expect("package metadata was resolved");

    let staging = harness_root.join(format!(".stage-{}-{}", std::process::id(), current_nonce()));
    if staging.exists() {
        fs::remove_dir_all(&staging)
            .map_err(|_| "cannot clear stale CLI staging directory".to_owned())?;
    }
    private_dir(&staging)?;
    let package_spec = format!("{}@{}", recipe.package, version);
    let install = match recipe.runtime {
        RuntimeKind::Node => {
            install_npm_package(&runtime, &staging, &package_spec, recipe.ignore_scripts)
        }
        RuntimeKind::Bun => install_bun_package(&runtime, &staging, &package_spec),
    };
    if let Err(error) = install {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    let staged_package_json = package_json_path(&staging, recipe.package, recipe.runtime);
    if package_version(&staged_package_json).as_deref() != Some(version.as_str()) {
        let _ = fs::remove_dir_all(&staging);
        return Err("package manager installed an unexpected harness version".to_owned());
    }
    let staged_executable = staging.join("bin").join(recipe.executable);
    if let Err(error) = probe_cli_version(&staged_executable, &runtime, &version) {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    let _ = hash_tree(&staging)?;

    let backup = harness_root.join(format!(".previous-{}-{}", version, current_nonce()));
    if prefix.exists() {
        fs::rename(&prefix, &backup)
            .map_err(|_| "cannot preserve the previous CLI installation".to_owned())?;
    }
    if fs::rename(&staging, &prefix).is_err() {
        if backup.exists() {
            let _ = fs::rename(&backup, &prefix);
        }
        let _ = fs::remove_dir_all(&staging);
        return Err("cannot activate the harness CLI installation".to_owned());
    }
    if backup.exists() {
        let _ = fs::remove_dir_all(backup);
    }

    write_integrity_record(&harness_root, &version, &package.integrity)?;

    let internal_executable = prefix.join("bin").join(recipe.executable);
    let (launcher, launcher_fingerprint) = install_launcher(
        paths,
        recipe.executable,
        &internal_executable,
        &runtime.bin_dirs,
        approved_existing_hash,
    )?;
    Ok(InstalledCli {
        harness,
        version,
        executable: launcher,
        integrity: Some(package.integrity),
        runtime_bins: runtime.bin_dirs,
        launcher_fingerprint: Some(launcher_fingerprint),
    })
}

pub fn bootstrap_recipe(required_tools: &[String]) -> InstallResult<Option<BootstrapRecipe>> {
    let mut missing = required_tools
        .iter()
        .filter(|tool| !tool_available(tool))
        .cloned()
        .collect::<Vec<_>>();
    missing.sort();
    missing.dedup();
    if missing.is_empty() {
        return Ok(None);
    }
    for tool in &missing {
        if !matches!(tool.as_str(), "git" | "curl" | "tar" | "xz" | "unzip") {
            return Err(
                "requested bootstrap dependency is not in the fixed tool registry".to_owned(),
            );
        }
    }

    #[cfg(target_os = "linux")]
    {
        if !is_debian_family() || !tool_available("apt-get") {
            return Err(
                "no fixed bootstrap recipe is available for this Linux distribution".to_owned(),
            );
        }
        let mut packages = missing
            .iter()
            .map(|tool| match tool.as_str() {
                "git" => "git",
                "curl" => "curl",
                "tar" => "tar",
                "xz" => "xz-utils",
                "unzip" => "unzip",
                _ => unreachable!(),
            })
            .collect::<Vec<_>>();
        if missing.iter().any(|tool| tool == "curl") {
            packages.push("ca-certificates");
        }
        packages.sort();
        packages.dedup();
        return Ok(Some(BootstrapRecipe {
            commands: vec![
                vec!["apt-get".into(), "update".into()],
                std::iter::once("apt-get".to_owned())
                    .chain(["install".to_owned(), "-y".to_owned()])
                    .chain(packages.into_iter().map(str::to_owned))
                    .collect(),
            ],
            needs_sudo: true,
            missing_tools: missing,
        }));
    }
    #[cfg(target_os = "macos")]
    {
        if !tool_available("brew") {
            return Err(
                "install Homebrew and the required archive tools before continuing".to_owned(),
            );
        }
        let packages = missing
            .iter()
            .map(|tool| match tool.as_str() {
                "git" => "git",
                "curl" => "curl",
                "tar" => "gnu-tar",
                "xz" => "xz",
                "unzip" => "unzip",
                _ => unreachable!(),
            })
            .collect::<Vec<_>>();
        let mut command = vec!["brew".to_owned(), "install".to_owned()];
        command.extend(packages.into_iter().map(str::to_owned));
        return Ok(Some(BootstrapRecipe {
            commands: vec![command],
            needs_sudo: false,
            missing_tools: missing,
        }));
    }
    #[allow(unreachable_code)]
    Err("no fixed bootstrap recipe is available on this platform".to_owned())
}

fn ensure_node(paths: &Paths) -> InstallResult<RuntimeEnv> {
    let managed_root = paths.data.join("runtimes/node").join(NODE_VERSION);
    let managed_bin = managed_root.join("bin");
    if managed_runtime_meets(
        paths,
        "node",
        NODE_VERSION,
        &managed_root,
        RuntimeKind::Node,
    )? {
        return Ok(RuntimeEnv {
            bin_dirs: vec![managed_bin],
        });
    }
    if let Some(system_node) = find_program("node", &[]) {
        if probe_version(&system_node, &[])
            .is_ok_and(|version| version_meets(&version, NODE_MINIMUM, Some(22)))
        {
            let bin = system_node
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_default();
            if find_program("npm", std::slice::from_ref(&bin)).is_some() {
                return Ok(RuntimeEnv {
                    bin_dirs: vec![bin],
                });
            }
        }
    }
    install_node(paths)?;
    Ok(RuntimeEnv {
        bin_dirs: vec![managed_bin],
    })
}

fn ensure_bun(paths: &Paths) -> InstallResult<RuntimeEnv> {
    let managed_root = paths.data.join("runtimes/bun").join(BUN_VERSION);
    let managed_bin = managed_root.join("bin");
    if managed_runtime_meets(paths, "bun", BUN_VERSION, &managed_root, RuntimeKind::Bun)? {
        return Ok(RuntimeEnv {
            bin_dirs: vec![managed_bin],
        });
    }
    if let Some(system_bun) = find_program("bun", &[]) {
        if probe_version(&system_bun, &[])
            .is_ok_and(|version| version_meets(&version, BUN_MINIMUM, None))
        {
            let bin = system_bun
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_default();
            return Ok(RuntimeEnv {
                bin_dirs: vec![bin],
            });
        }
    }
    install_bun(paths)?;
    Ok(RuntimeEnv {
        bin_dirs: vec![managed_bin],
    })
}

fn install_node(paths: &Paths) -> InstallResult<()> {
    let (triplet, expected) = node_target()?;
    let archive_name = format!("node-v{NODE_VERSION}-{triplet}.tar.xz");
    let url = format!("https://nodejs.org/dist/v{NODE_VERSION}/{archive_name}");
    install_archive_runtime(
        paths,
        "node",
        NODE_VERSION,
        &url,
        expected,
        &archive_name,
        ArchiveKind::TarXz,
    )
}

fn install_bun(paths: &Paths) -> InstallResult<()> {
    let (asset, expected) = bun_target()?;
    let url =
        format!("https://github.com/oven-sh/bun/releases/download/bun-v{BUN_VERSION}/{asset}");
    install_archive_runtime(
        paths,
        "bun",
        BUN_VERSION,
        &url,
        expected,
        asset,
        ArchiveKind::Zip,
    )
}

#[derive(Clone, Copy)]
enum ArchiveKind {
    TarXz,
    Zip,
}

fn install_archive_runtime(
    paths: &Paths,
    name: &str,
    version: &str,
    url: &str,
    expected_sha256: &str,
    archive_name: &str,
    kind: ArchiveKind,
) -> InstallResult<()> {
    private_dir(&paths.data.join("runtimes"))?;
    private_dir(&paths.cache.join("downloads"))?;
    let runtime_parent = paths.data.join("runtimes").join(name);
    private_dir(&runtime_parent)?;
    let target = runtime_parent.join(version);
    let nonce = current_nonce();
    let archive = paths
        .cache
        .join("downloads")
        .join(format!("{name}-{version}-{nonce}.{}", extension(kind)));
    let stage = runtime_parent.join(format!(".stage-{}-{nonce}", std::process::id()));
    private_dir(&stage)?;
    let curl = find_program("curl", &[])
        .ok_or_else(|| "curl is required to download the managed runtime".to_owned())?;
    let (status, _) = run_limited(
        &curl,
        &[
            "--fail",
            "--location",
            "--proto",
            "=https",
            "--tlsv1.2",
            "--silent",
            "--show-error",
            "--output",
            archive.to_string_lossy().as_ref(),
            url,
        ],
        &[],
        &[],
        Some(&paths.home),
    )?;
    if !status.success() {
        let _ = fs::remove_dir_all(&stage);
        let _ = fs::remove_file(&archive);
        return Err("runtime archive download failed".to_owned());
    }
    let actual = match sha256_file(&archive) {
        Ok(value) => value,
        Err(error) => {
            let _ = fs::remove_dir_all(&stage);
            let _ = fs::remove_file(&archive);
            return Err(error);
        }
    };
    if actual != expected_sha256 {
        let _ = fs::remove_dir_all(&stage);
        let _ = fs::remove_file(&archive);
        return Err("runtime archive checksum did not match the shipped registry".to_owned());
    }
    let extract_result = match kind {
        ArchiveKind::TarXz => extract_tar(&archive, &stage),
        ArchiveKind::Zip => extract_bun_zip(&archive, &stage),
    };
    if let Err(error) = extract_result {
        let _ = fs::remove_dir_all(&stage);
        let _ = fs::remove_file(&archive);
        return Err(error);
    }
    let binary = match name {
        "node" => stage.join("bin/node"),
        "bun" => match normalize_bun_layout(&stage) {
            Ok(path) => path,
            Err(error) => {
                let _ = fs::remove_dir_all(&stage);
                let _ = fs::remove_file(&archive);
                return Err(error);
            }
        },
        _ => unreachable!(),
    };
    let bin_dir = stage.join("bin");
    if name == "bun" {
        private_dir(&bin_dir)?;
        let target_binary = bin_dir.join("bun");
        if binary != target_binary {
            fs::rename(&binary, &target_binary)
                .map_err(|_| "cannot stage Bun executable".to_owned())?;
        }
        set_executable(&target_binary)?;
    }
    let runtime_env = RuntimeEnv {
        bin_dirs: vec![bin_dir],
    };
    let actual_version = probe_version(&binary_path(name, &stage), &runtime_env.bin_dirs)?;
    let expected_parts = parse_version(version)
        .ok_or_else(|| "invalid managed runtime registry version".to_owned())?;
    if actual_version != expected_parts {
        let _ = fs::remove_dir_all(&stage);
        let _ = fs::remove_file(&archive);
        return Err("runtime archive contains an unexpected version".to_owned());
    }
    let tree_digest = match hash_tree(&stage) {
        Ok(digest) => digest,
        Err(error) => {
            let _ = fs::remove_dir_all(&stage);
            let _ = fs::remove_file(&archive);
            return Err(error);
        }
    };
    let previous = runtime_parent.join(format!(".previous-{version}-{nonce}"));
    if target.exists() {
        fs::rename(&target, &previous)
            .map_err(|_| "cannot preserve the previous managed runtime".to_owned())?;
    }
    if fs::rename(&stage, &target).is_err() {
        if previous.exists() {
            let _ = fs::rename(&previous, &target);
        }
        let _ = fs::remove_dir_all(&stage);
        let _ = fs::remove_file(&archive);
        return Err("cannot activate the managed runtime".to_owned());
    }
    let marker = runtime_parent.join(format!("{version}.integrity"));
    if let Err(error) = atomic_write(
        &marker,
        format!("{expected_sha256}\n{tree_digest}\n{archive_name}\n").as_bytes(),
        0o600,
    ) {
        let _ = fs::remove_dir_all(&target);
        if previous.exists() {
            let _ = fs::rename(&previous, &target);
        }
        let _ = fs::remove_file(&archive);
        return Err(error);
    }
    if previous.exists() {
        let _ = fs::remove_dir_all(previous);
    }
    let _ = fs::remove_file(archive);
    Ok(())
}

fn extract_tar(archive: &Path, stage: &Path) -> InstallResult<()> {
    let tar = find_program("tar", &[])
        .ok_or_else(|| "tar is required to extract the managed runtime".to_owned())?;
    let (status, _) = run_limited(
        &tar,
        &[
            "--extract",
            "--file",
            archive.to_string_lossy().as_ref(),
            "--directory",
            stage.to_string_lossy().as_ref(),
            "--strip-components=1",
            "--no-same-owner",
        ],
        &[],
        &[],
        None,
    )?;
    if !status.success() {
        return Err("runtime tar archive extraction failed".to_owned());
    }
    let _ = hash_tree(stage)?;
    Ok(())
}

fn extract_bun_zip(archive: &Path, stage: &Path) -> InstallResult<()> {
    let unzip =
        find_program("unzip", &[]).ok_or_else(|| "unzip is required to extract Bun".to_owned())?;
    let (status, _) = run_limited(
        &unzip,
        &[
            "-q",
            "-o",
            archive.to_string_lossy().as_ref(),
            "-d",
            stage.to_string_lossy().as_ref(),
        ],
        &[],
        &[],
        None,
    )?;
    if !status.success() {
        return Err("Bun archive extraction failed".to_owned());
    }
    let _ = hash_tree(stage)?;
    Ok(())
}

fn normalize_bun_layout(stage: &Path) -> InstallResult<PathBuf> {
    let mut found = Vec::new();
    find_named_regular(stage, "bun", &mut found)?;
    if found.len() != 1 {
        return Err("Bun archive did not contain one executable".to_owned());
    }
    Ok(found.remove(0))
}

fn find_named_regular(root: &Path, name: &str, found: &mut Vec<PathBuf>) -> InstallResult<()> {
    for entry in fs::read_dir(root).map_err(|_| "cannot inspect Bun archive contents".to_owned())? {
        let entry = entry.map_err(|_| "cannot inspect Bun archive contents".to_owned())?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)
            .map_err(|_| "cannot inspect Bun archive contents".to_owned())?;
        if metadata.is_dir() {
            find_named_regular(&path, name, found)?;
        } else if metadata.is_file() && path.file_name().is_some_and(|candidate| candidate == name)
        {
            found.push(path);
        } else if !metadata.is_file() {
            return Err("Bun archive contains an unsupported file type".to_owned());
        }
    }
    Ok(())
}

fn ensure_program_command(name: &str, bin_dirs: &[PathBuf]) -> InstallResult<PathBuf> {
    find_program(name, bin_dirs)
        .ok_or_else(|| format!("required executable `{name}` is not available"))
}

fn install_npm_package(
    runtime: &RuntimeEnv,
    prefix: &Path,
    spec: &str,
    ignore_scripts: bool,
) -> InstallResult<()> {
    let npm = ensure_program_command("npm", &runtime.bin_dirs)?;
    let mut args = vec![
        "install".to_owned(),
        "--global".to_owned(),
        "--prefix".to_owned(),
        prefix.to_string_lossy().into_owned(),
    ];
    if ignore_scripts {
        args.push("--ignore-scripts".to_owned());
    }
    args.push(spec.to_owned());
    let (status, _) = run_limited(
        &npm,
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
        &runtime.bin_dirs,
        &[],
        None,
    )?;
    if !status.success() {
        return Err("npm could not install the requested harness package".to_owned());
    }
    Ok(())
}

fn install_bun_package(runtime: &RuntimeEnv, prefix: &Path, spec: &str) -> InstallResult<()> {
    let bun = ensure_program_command("bun", &runtime.bin_dirs)?;
    let global_dir = prefix.join("global");
    let bin_dir = prefix.join("bin");
    private_dir(&global_dir)?;
    private_dir(&bin_dir)?;
    let (status, _) = run_limited(
        &bun,
        &["install", "--global", spec],
        &runtime.bin_dirs,
        &[
            (
                "BUN_INSTALL_GLOBAL_DIR",
                global_dir.to_string_lossy().as_ref(),
            ),
            ("BUN_INSTALL_BIN", bin_dir.to_string_lossy().as_ref()),
        ],
        None,
    )?;
    if !status.success() {
        return Err("Bun could not install the requested harness package".to_owned());
    }
    Ok(())
}

fn package_json_path(prefix: &Path, package: &str, runtime: RuntimeKind) -> PathBuf {
    let root = match runtime {
        RuntimeKind::Node => prefix.join("lib/node_modules"),
        RuntimeKind::Bun => prefix.join("global/node_modules"),
    };
    root.join(package).join("package.json")
}

fn package_version(package_json: &Path) -> Option<String> {
    let value: serde_json::Value = serde_json::from_slice(&fs::read(package_json).ok()?).ok()?;
    value.get("version")?.as_str().map(str::to_owned)
}

fn resolve_package_version(package: &str, requested: &str) -> InstallResult<ResolvedPackage> {
    if requested != "latest" && !valid_exact_version(requested) {
        return Err("harness version must be latest or an exact package version".to_owned());
    }
    let curl = find_program("curl", &[])
        .ok_or_else(|| "curl is required to resolve harness package metadata".to_owned())?;
    let encoded_package = package.replace('@', "%40").replace('/', "%2f");
    let uri = if requested == "latest" {
        format!(
            "https://registry.npmjs.org/{encoded_package}/latest?fresh={}",
            current_nonce()
        )
    } else {
        format!("https://registry.npmjs.org/{encoded_package}/{requested}")
    };
    let (status, output) = run_limited(
        &curl,
        &[
            "--fail",
            "--location",
            "--proto",
            "=https",
            "--tlsv1.2",
            "--silent",
            "--show-error",
            "--header",
            "Cache-Control: no-cache",
            "--header",
            "Pragma: no-cache",
            &uri,
        ],
        &[],
        &[],
        None,
    )?;
    if !status.success() {
        return Err("cannot resolve the harness package version from the registry".to_owned());
    }
    let metadata: serde_json::Value = serde_json::from_slice(&output)
        .map_err(|_| "harness registry returned invalid metadata".to_owned())?;
    let version = metadata
        .get("version")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "harness registry metadata omitted its version".to_owned())?;
    let integrity = metadata
        .pointer("/dist/integrity")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "harness registry metadata omitted package integrity".to_owned())?;
    if !valid_exact_version(version) || requested != "latest" && requested != version {
        return Err("harness registry returned an unexpected package version".to_owned());
    }
    if !valid_integrity(integrity) {
        return Err("harness registry returned an invalid integrity value".to_owned());
    }
    Ok(ResolvedPackage {
        version: version.to_owned(),
        integrity: integrity.to_owned(),
    })
}

struct ResolvedPackage {
    version: String,
    integrity: String,
}

fn probe_cli_version(executable: &Path, runtime: &RuntimeEnv, expected: &str) -> InstallResult<()> {
    if !executable.exists() {
        return Err("installed harness executable is missing".to_owned());
    }
    let (status, output) = run_limited(executable, &["--version"], &runtime.bin_dirs, &[], None)?;
    if !status.success() {
        return Err("installed harness failed its --version probe".to_owned());
    }
    let actual = String::from_utf8_lossy(&output);
    if first_version(&actual).as_deref() != Some(expected) {
        return Err("installed harness --version did not match its package version".to_owned());
    }
    Ok(())
}

fn install_launcher(
    paths: &Paths,
    name: &str,
    executable: &Path,
    runtime_bins: &[PathBuf],
    approved_existing_hash: Option<&str>,
) -> InstallResult<(PathBuf, String)> {
    let launcher = paths.bin.join(name);
    let mut lines = vec!["#!/bin/sh".to_owned()];
    let mut path_entries = Vec::new();
    for entry in runtime_bins {
        let text = entry.to_string_lossy();
        if text.contains(':') || text.contains('\n') {
            return Err("runtime path cannot be represented in a launcher".to_owned());
        }
        path_entries.push(shell_quote(&text));
    }
    if !path_entries.is_empty() {
        lines.push(format!("export PATH={}:\"$PATH\"", path_entries.join(":")));
    }
    let executable = executable.to_string_lossy();
    if executable.contains('\n') {
        return Err("harness executable path cannot be represented in a launcher".to_owned());
    }
    lines.push(format!("exec {} \"$@\"", shell_quote(&executable)));
    lines.push(String::new());
    let contents = lines.join("\n");
    let existing = match fs::symlink_metadata(&launcher) {
        Ok(metadata) if metadata.is_file() => Some(read_file_checked(&paths.bin, &launcher)?),
        Ok(_) => return Err("existing harness launcher is not a regular file".to_owned()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => return Err("cannot inspect existing harness launcher".to_owned()),
    };
    let expected_hash = match existing.as_deref() {
        Some(existing) if existing == contents.as_bytes() => None,
        Some(existing) => {
            let actual_hash = sha256_bytes(existing);
            let Some(approved) = approved_existing_hash else {
                return Err("harness launcher exists and is not managed by this install".to_owned());
            };
            if actual_hash != approved {
                return Err(
                    "harness launcher no longer matches its recorded fingerprint".to_owned(),
                );
            }
            backup_cli_launcher(paths, name, existing)?;
            Some(approved)
        }
        None => None,
    };
    if existing.as_deref() != Some(contents.as_bytes()) {
        atomic_write_if_unchanged(&launcher, contents.as_bytes(), 0o755, expected_hash)?;
    }
    Ok((launcher, sha256_bytes(contents.as_bytes())))
}

pub(crate) fn backup_cli_launcher(
    paths: &Paths,
    name: &str,
    contents: &[u8],
) -> InstallResult<PathBuf> {
    let backup_root = paths.state.join("backups/cli-launchers");
    private_dir(&backup_root)?;
    for sequence in 0..32 {
        let directory = backup_root.join(format!("{}-{}-{sequence}", name, current_nonce()));
        if super::util::create_private_dir_new(&directory).is_err() {
            continue;
        }
        let content = directory.join("content");
        if let Err(error) = atomic_write(&content, contents, 0o600) {
            let _ = remove_tree_checked(&directory);
            return Err(error);
        }
        return Ok(directory);
    }
    Err("cannot reserve a private CLI launcher backup".to_owned())
}

pub fn remove_cli(paths: &Paths, cli: &InstalledCli) -> InstallResult<()> {
    let recipe = cli_recipe(cli.harness);
    let expected_launcher = paths.bin.join(recipe.executable);
    if cli.executable != expected_launcher {
        return Err("recorded CLI launcher path is not managed by this install".to_owned());
    }
    let expected_hash = cli
        .launcher_fingerprint
        .as_deref()
        .ok_or_else(|| "recorded CLI has no launcher ownership fingerprint".to_owned())?;
    if !valid_sha256(expected_hash) || !valid_exact_version(&cli.version) {
        return Err("recorded CLI ownership data is invalid".to_owned());
    }

    let harness_root = paths.data.join("harnesses").join(cli.harness.as_str());
    let prefix = harness_root.join(&cli.version);
    if let Ok(metadata) = fs::symlink_metadata(&prefix) {
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err("recorded CLI prefix is not a real directory".to_owned());
        }
        let package_json = package_json_path(&prefix, recipe.package, recipe.runtime);
        let bytes = read_file_checked_bounded(&prefix, &package_json, 1024 * 1024)?;
        let metadata: serde_json::Value = serde_json::from_slice(&bytes)
            .map_err(|_| "recorded CLI package metadata is invalid".to_owned())?;
        if metadata.get("version").and_then(serde_json::Value::as_str) != Some(&cli.version) {
            return Err("recorded CLI prefix does not match its package version".to_owned());
        }
    }

    let existing_launcher = match fs::symlink_metadata(&expected_launcher) {
        Ok(metadata) if metadata.is_file() => {
            let bytes = read_file_checked(&paths.bin, &expected_launcher)?;
            if sha256_bytes(&bytes) != expected_hash {
                return Err(
                    "harness launcher no longer matches its recorded fingerprint".to_owned(),
                );
            }
            Some(bytes)
        }
        Ok(_) => return Err("recorded harness launcher is not a regular file".to_owned()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => return Err("cannot inspect recorded harness launcher".to_owned()),
    };
    if let Some(bytes) = &existing_launcher {
        backup_cli_launcher(paths, recipe.executable, bytes)?;
        remove_file_checked(&expected_launcher, Some(expected_hash))?;
    }
    if prefix.exists() {
        if let Err(error) = remove_tree_checked(&prefix) {
            if let Some(bytes) = &existing_launcher {
                let _ = atomic_write_if_unchanged(&expected_launcher, bytes, 0o755, None);
            }
            return Err(error);
        }
    }
    Ok(())
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn cached_integrity(harness_root: &Path, version: &str) -> Option<String> {
    let path = harness_root.join(format!("{version}.integrity"));
    let value = fs::read_to_string(path).ok()?;
    let value = value.trim();
    valid_integrity(value).then(|| value.to_owned())
}

fn write_integrity_record(
    harness_root: &Path,
    version: &str,
    integrity: &str,
) -> InstallResult<()> {
    let path = harness_root.join(format!("{version}.integrity"));
    let expected = format!("{integrity}\n");
    if fs::read(&path).ok().as_deref() != Some(expected.as_bytes()) {
        atomic_write(&path, expected.as_bytes(), 0o600)?;
    }
    Ok(())
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn cli_recipe(harness: HarnessId) -> CliRecipe {
    match harness {
        HarnessId::Codex => CliRecipe {
            package: "@openai/codex",
            executable: "codex",
            runtime: RuntimeKind::Node,
            ignore_scripts: false,
        },
        HarnessId::Claude => CliRecipe {
            package: "@anthropic-ai/claude-code",
            executable: "claude",
            runtime: RuntimeKind::Node,
            ignore_scripts: false,
        },
        HarnessId::Opencode => CliRecipe {
            package: "opencode-ai",
            executable: "opencode",
            runtime: RuntimeKind::Node,
            ignore_scripts: false,
        },
        HarnessId::Pi => CliRecipe {
            package: "@earendil-works/pi-coding-agent",
            executable: "pi",
            runtime: RuntimeKind::Node,
            ignore_scripts: true,
        },
        HarnessId::Omp => CliRecipe {
            package: "@oh-my-pi/pi-coding-agent",
            executable: "omp",
            runtime: RuntimeKind::Bun,
            ignore_scripts: false,
        },
    }
}

fn managed_runtime_meets(
    paths: &Paths,
    name: &str,
    version: &str,
    root: &Path,
    runtime: RuntimeKind,
) -> InstallResult<bool> {
    if !root.is_dir() {
        return Ok(false);
    }
    let bin = root.join("bin");
    let executable = binary_path(name, root);
    if !executable.is_file() {
        return Ok(false);
    }
    let expected_archive = match name {
        "node" => node_target()?.1,
        "bun" => bun_target()?.1,
        _ => return Ok(false),
    };
    let marker = paths
        .data
        .join("runtimes")
        .join(name)
        .join(format!("{version}.integrity"));
    let record = match fs::read_to_string(marker) {
        Ok(record) => record,
        Err(_) => return Ok(false),
    };
    let mut fields = record.lines();
    if fields.next() != Some(expected_archive) {
        return Ok(false);
    }
    let expected_tree = match fields.next() {
        Some(value) => value,
        None => return Ok(false),
    };
    let actual_tree = match hash_tree(root) {
        Ok(value) => value,
        Err(_) => return Ok(false),
    };
    if actual_tree != expected_tree {
        return Ok(false);
    }
    let found = match probe_version(&executable, &[bin]) {
        Ok(value) => value,
        Err(_) => return Ok(false),
    };
    let expected =
        parse_version(version).ok_or_else(|| "invalid runtime registry version".to_owned())?;
    Ok(found == expected
        && match runtime {
            RuntimeKind::Node => version_meets(&found, NODE_MINIMUM, Some(22)),
            RuntimeKind::Bun => version_meets(&found, BUN_MINIMUM, None),
        })
}

fn probe_version(executable: &Path, path_dirs: &[PathBuf]) -> InstallResult<(u64, u64, u64)> {
    let (status, output) = run_limited(executable, &["--version"], path_dirs, &[], None)?;
    if !status.success() {
        return Err("runtime version probe failed".to_owned());
    }
    let text = String::from_utf8_lossy(&output);
    first_version(&text)
        .filter(|version| !version.contains('-'))
        .and_then(|version| parse_version(&version))
        .ok_or_else(|| "runtime version probe returned an invalid version".to_owned())
}

fn first_version(text: &str) -> Option<String> {
    for token in text.split(|character: char| {
        !(character.is_ascii_alphanumeric()
            || character == '.'
            || character == '-'
            || character == '+')
    }) {
        let token = token.strip_prefix('v').unwrap_or(token);
        if parse_version(token).is_some() {
            return Some(token.to_owned());
        }
    }
    None
}

fn parse_version(value: &str) -> Option<(u64, u64, u64)> {
    let base = value
        .strip_prefix('v')
        .unwrap_or(value)
        .split(['-', '+'])
        .next()?;
    let mut components = base.split('.');
    let major = components.next()?.parse().ok()?;
    let minor = components.next()?.parse().ok()?;
    let patch = components.next()?.parse().ok()?;
    if components.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

fn valid_exact_version(value: &str) -> bool {
    if value.is_empty()
        || value.starts_with('-')
        || value.starts_with('v')
        || value.chars().any(char::is_whitespace)
    {
        return false;
    }
    if parse_version(value).is_none() {
        return false;
    }
    let base = value.split(['-', '+']).next().unwrap_or(value);
    let suffix = value.strip_prefix(base).unwrap_or_default();
    let suffix_is_valid = if suffix.is_empty() {
        true
    } else if let Some(suffix) = suffix.strip_prefix('-') {
        let (pre, build) = suffix.split_once('+').unwrap_or((suffix, ""));
        let build_is_valid = !suffix.contains('+') || valid_identifiers(build, false);
        valid_identifiers(pre, true) && build_is_valid
    } else if let Some(build) = suffix.strip_prefix('+') {
        valid_identifiers(build, false)
    } else {
        false
    };
    suffix_is_valid
        && base.split('.').all(|part| {
            !part.is_empty()
                && (part == "0" || !part.starts_with('0'))
                && part.bytes().all(|byte| byte.is_ascii_digit())
        })
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || ".-+".contains(character))
}

fn valid_identifiers(value: &str, prerelease: bool) -> bool {
    !value.is_empty()
        && value.split('.').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                && (!prerelease
                    || !part.bytes().all(|byte| byte.is_ascii_digit())
                    || part == "0"
                    || !part.starts_with('0'))
        })
}

fn valid_integrity(value: &str) -> bool {
    value
        .strip_prefix("sha512-")
        .or_else(|| value.strip_prefix("sha1-"))
        .is_some_and(|encoded| {
            !encoded.is_empty()
                && encoded
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || "+/=".contains(byte as char))
        })
}

fn version_meets(version: &(u64, u64, u64), minimum: (u64, u64, u64), major: Option<u64>) -> bool {
    major.is_none_or(|major| version.0 == major) && *version >= minimum
}

fn node_target() -> InstallResult<(&'static str, &'static str)> {
    node_target_for(std::env::consts::OS, std::env::consts::ARCH)
}

fn node_target_for(os: &str, arch: &str) -> InstallResult<(&'static str, &'static str)> {
    match (os, arch) {
        ("linux", "x86_64") => Ok((
            "linux-x64",
            "9aa8e9d2298ab68c600bd6fb86a6c13bce11a4eca1ba9b39d79fa021755d7c37",
        )),
        ("linux", "aarch64") => Ok((
            "linux-arm64",
            "1bf1eb9ee63ffc4e5d324c0b9b62cf4a289f44332dfef9607cea1a0d9596ba6f",
        )),
        ("macos", "x86_64") => Ok((
            "darwin-x64",
            "48bc437e00e0c1483da34c21dca196efcb8d22e5dcb0bc7c65386afb00fabb85",
        )),
        ("macos", "aarch64") => Ok((
            "darwin-arm64",
            "2bd596bbfc4a275ceb8721a5954ee97daea5ebe673e96a185ebd732f6fb023ac",
        )),
        _ => Err("Node runtime bootstrap is unsupported on this OS and architecture".to_owned()),
    }
}

fn bun_target() -> InstallResult<(&'static str, &'static str)> {
    bun_target_for(std::env::consts::OS, std::env::consts::ARCH)
}

fn bun_target_for(os: &str, arch: &str) -> InstallResult<(&'static str, &'static str)> {
    match (os, arch) {
        ("linux", "x86_64") => Ok((
            "bun-linux-x64-baseline.zip",
            "c678040f14fe0440eb839d37cbd0ce4c051a32da72806ac97de6a6aab6bf728f",
        )),
        ("linux", "aarch64") => Ok((
            "bun-linux-aarch64.zip",
            "54328bbc2d9c8e0c9f892c544d66c57a83b84139e34909e5ee81758f1ac8fda7",
        )),
        ("macos", "x86_64") => Ok((
            "bun-darwin-x64-baseline.zip",
            "bad5bbd6cf14d0980d115f5954c9ff904df619d5e994d2da1ffccd3f316300b0",
        )),
        ("macos", "aarch64") => Ok((
            "bun-darwin-aarch64.zip",
            "90987a3a16d7db556d886ac3d551e7b6d3edf0a1cf43acaed622e8676be1d12f",
        )),
        _ => Err("Bun runtime bootstrap is unsupported on this OS and architecture".to_owned()),
    }
}

fn binary_path(name: &str, stage: &Path) -> PathBuf {
    match name {
        "node" => stage.join("bin/node"),
        "bun" => stage.join("bin/bun"),
        _ => unreachable!(),
    }
}

fn extension(kind: ArchiveKind) -> &'static str {
    match kind {
        ArchiveKind::TarXz => "tar.xz",
        ArchiveKind::Zip => "zip",
    }
}

fn set_executable(path: &Path) -> InstallResult<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path)
            .map_err(|_| "cannot inspect runtime executable".to_owned())?
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions)
            .map_err(|_| "cannot make runtime executable".to_owned())?;
    }
    Ok(())
}

fn run_limited(
    executable: &Path,
    arguments: &[&str],
    path_dirs: &[PathBuf],
    extra_env: &[(&str, &str)],
    cwd: Option<&Path>,
) -> InstallResult<(ExitStatus, Vec<u8>)> {
    let mut command = Command::new(executable);
    command
        .args(arguments)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let mut path = path_dirs.to_vec();
    let existing = std::env::var_os("PATH");
    if let Some(existing) = existing.as_ref() {
        path.extend(std::env::split_paths(existing));
    }
    if let Ok(path) = std::env::join_paths(path) {
        command.env("PATH", path);
    }
    for (key, value) in extra_env {
        command.env(key, value);
    }
    let mut child = command
        .spawn()
        .map_err(|_| "could not start a required installer process".to_owned())?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let out_thread = thread::spawn(move || read_bounded(stdout, CAPTURE_LIMIT));
    let err_thread = thread::spawn(move || read_bounded(stderr, CAPTURE_LIMIT));
    let status = child
        .wait()
        .map_err(|_| "installer process did not finish".to_owned())?;
    let output = out_thread
        .join()
        .map_err(|_| "installer output capture failed".to_owned())?;
    let _ = err_thread.join();
    Ok((status, output))
}

fn read_bounded<R: Read>(reader: Option<R>, limit: usize) -> Vec<u8> {
    let Some(mut reader) = reader else {
        return Vec::new();
    };
    let mut captured = Vec::new();
    let mut buffer = [0; 8192];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(count) if captured.len() < limit => {
                let retained = count.min(limit - captured.len());
                captured.extend_from_slice(&buffer[..retained]);
            }
            Ok(_) => {}
        }
    }
    captured
}

fn find_program(name: &str, preferred_dirs: &[PathBuf]) -> Option<PathBuf> {
    let path_env = std::env::var_os("PATH");
    let path_dirs = path_env
        .as_ref()
        .map(|path| std::env::split_paths(path).collect::<Vec<_>>())
        .unwrap_or_default();
    for directory in preferred_dirs.iter().cloned().chain(path_dirs) {
        let candidate = directory.join(name);
        if candidate.is_file() {
            return fs::canonicalize(&candidate).ok().or(Some(candidate));
        }
    }
    None
}

fn tool_available(name: &str) -> bool {
    find_program(name, &[]).is_some()
}

fn is_debian_family() -> bool {
    let contents = fs::read_to_string("/etc/os-release")
        .unwrap_or_default()
        .to_ascii_lowercase();
    contents.lines().any(|line| {
        line.starts_with("id=ubuntu")
            || line.starts_with("id=debian")
            || line.starts_with("id_like=") && (line.contains("debian") || line.contains("ubuntu"))
    })
}

fn current_nonce() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("{nanos}-{}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    struct TempTree(PathBuf);

    impl TempTree {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "yashik-runtime-unit-{}-{}-{}",
                std::process::id(),
                current_nonce(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(fs::canonicalize(path).unwrap())
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
    fn runtime_checksum_registry_covers_supported_platforms() {
        let node = [
            (
                ("linux", "x86_64"),
                (
                    "linux-x64",
                    "9aa8e9d2298ab68c600bd6fb86a6c13bce11a4eca1ba9b39d79fa021755d7c37",
                ),
            ),
            (
                ("linux", "aarch64"),
                (
                    "linux-arm64",
                    "1bf1eb9ee63ffc4e5d324c0b9b62cf4a289f44332dfef9607cea1a0d9596ba6f",
                ),
            ),
            (
                ("macos", "x86_64"),
                (
                    "darwin-x64",
                    "48bc437e00e0c1483da34c21dca196efcb8d22e5dcb0bc7c65386afb00fabb85",
                ),
            ),
            (
                ("macos", "aarch64"),
                (
                    "darwin-arm64",
                    "2bd596bbfc4a275ceb8721a5954ee97daea5ebe673e96a185ebd732f6fb023ac",
                ),
            ),
        ];
        for ((os, arch), expected) in node {
            assert_eq!(node_target_for(os, arch).unwrap(), expected);
            assert!(
                expected.1.len() == 64 && expected.1.bytes().all(|byte| byte.is_ascii_hexdigit())
            );
        }

        let bun = [
            (
                ("linux", "x86_64"),
                (
                    "bun-linux-x64-baseline.zip",
                    "c678040f14fe0440eb839d37cbd0ce4c051a32da72806ac97de6a6aab6bf728f",
                ),
            ),
            (
                ("linux", "aarch64"),
                (
                    "bun-linux-aarch64.zip",
                    "54328bbc2d9c8e0c9f892c544d66c57a83b84139e34909e5ee81758f1ac8fda7",
                ),
            ),
            (
                ("macos", "x86_64"),
                (
                    "bun-darwin-x64-baseline.zip",
                    "bad5bbd6cf14d0980d115f5954c9ff904df619d5e994d2da1ffccd3f316300b0",
                ),
            ),
            (
                ("macos", "aarch64"),
                (
                    "bun-darwin-aarch64.zip",
                    "90987a3a16d7db556d886ac3d551e7b6d3edf0a1cf43acaed622e8676be1d12f",
                ),
            ),
        ];
        for ((os, arch), expected) in bun {
            assert_eq!(bun_target_for(os, arch).unwrap(), expected);
            assert!(
                expected.1.len() == 64 && expected.1.bytes().all(|byte| byte.is_ascii_hexdigit())
            );
        }
        assert!(node_target_for("freebsd", "x86_64").is_err());
        assert!(bun_target_for("windows", "x86_64").is_err());
    }

    #[test]
    fn runtime_and_package_versions_obey_the_closed_minimums() {
        assert_eq!(parse_version("v22.19.0"), Some((22, 19, 0)));
        assert!(version_meets(&(22, 19, 0), NODE_MINIMUM, Some(22)));
        assert!(!version_meets(&(22, 18, 9), NODE_MINIMUM, Some(22)));
        assert!(!version_meets(&(23, 0, 0), NODE_MINIMUM, Some(22)));
        assert!(version_meets(&(1, 3, 14), BUN_MINIMUM, None));
        assert!(!version_meets(&(1, 3, 13), BUN_MINIMUM, None));
        assert!(valid_exact_version("2.1.286"));
        assert!(valid_exact_version("1.0.0-beta.2+build.7"));
        assert!(valid_exact_version("1.2.3-rc-1+build-5"));
        for rejected in [
            "latest",
            "v1.2.3",
            "1.02.3",
            "1.2",
            "1.2.3-01",
            "1.2.3-alpha+build+other",
            "1.2.3;touch",
        ] {
            assert!(!valid_exact_version(rejected), "accepted {rejected:?}");
        }
    }

    #[test]
    fn cli_recipe_registry_matches_all_supported_harnesses() {
        let cases = [
            (HarnessId::Codex, "@openai/codex", "codex", false, "node"),
            (
                HarnessId::Claude,
                "@anthropic-ai/claude-code",
                "claude",
                false,
                "node",
            ),
            (
                HarnessId::Opencode,
                "opencode-ai",
                "opencode",
                false,
                "node",
            ),
            (
                HarnessId::Pi,
                "@earendil-works/pi-coding-agent",
                "pi",
                true,
                "node",
            ),
            (
                HarnessId::Omp,
                "@oh-my-pi/pi-coding-agent",
                "omp",
                false,
                "bun",
            ),
        ];
        for (harness, package, executable, ignore_scripts, runtime) in cases {
            let recipe = cli_recipe(harness);
            assert_eq!(recipe.package, package);
            assert_eq!(recipe.executable, executable);
            assert_eq!(recipe.ignore_scripts, ignore_scripts);
            assert_eq!(
                match recipe.runtime {
                    RuntimeKind::Node => "node",
                    RuntimeKind::Bun => "bun",
                },
                runtime
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn launcher_collision_is_preserved_and_managed_replacement_is_backed_up_once() {
        use std::os::unix::fs::MetadataExt;

        let temp = TempTree::new();
        let paths = temp.paths();
        fs::create_dir_all(&paths.bin).unwrap();
        let outside = temp.0.join("outside");
        fs::create_dir_all(&outside).unwrap();
        let sentinel = outside.join("sentinel");
        fs::write(&sentinel, "outside content").unwrap();
        let launcher = paths.bin.join("codex");
        fs::write(&launcher, b"user-owned launcher\n").unwrap();
        let before = fs::read(&launcher).unwrap();

        assert!(install_launcher(
            &paths,
            "codex",
            &temp.0.join("new-codex"),
            &[temp.0.join("node/bin")],
            None,
        )
        .is_err());
        assert_eq!(fs::read(&launcher).unwrap(), before);
        assert_eq!(fs::read(&sentinel).unwrap(), b"outside content");
        assert!(!paths.state.join("backups").exists());

        let approved = sha256_bytes(&before);
        let (installed, fingerprint) = install_launcher(
            &paths,
            "codex",
            &temp.0.join("new-codex"),
            &[temp.0.join("node/bin")],
            Some(&approved),
        )
        .unwrap();
        assert_eq!(installed, launcher);
        assert_ne!(fs::read(&launcher).unwrap(), before);
        assert_eq!(fingerprint, sha256_bytes(&fs::read(&launcher).unwrap()));
        let backup_root = paths.state.join("backups/cli-launchers");
        let backup_dirs = fs::read_dir(&backup_root).unwrap().count();
        assert_eq!(backup_dirs, 1);
        let backup_content = fs::read_dir(&backup_root)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path()
            .join("content");
        assert_eq!(fs::read(backup_content).unwrap(), before);

        let inode = fs::metadata(&launcher).unwrap().ino();
        let (_, repeated_fingerprint) = install_launcher(
            &paths,
            "codex",
            &temp.0.join("new-codex"),
            &[temp.0.join("node/bin")],
            None,
        )
        .unwrap();
        assert_eq!(repeated_fingerprint, fingerprint);
        assert_eq!(fs::metadata(&launcher).unwrap().ino(), inode);
        assert_eq!(fs::read_dir(&backup_root).unwrap().count(), 1);

        fs::write(&launcher, b"changed after ownership check\n").unwrap();
        let changed = fs::read(&launcher).unwrap();
        assert!(install_launcher(
            &paths,
            "codex",
            &temp.0.join("next-codex"),
            &[],
            Some(&fingerprint),
        )
        .is_err());
        assert_eq!(fs::read(&launcher).unwrap(), changed);
        assert_eq!(fs::read_dir(&backup_root).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn remove_cli_backs_up_only_the_recorded_launcher_and_version_prefix() {
        let temp = TempTree::new();
        let paths = temp.paths();
        fs::create_dir_all(&paths.bin).unwrap();
        let version = "0.159.3";
        let prefix = paths.data.join("harnesses/codex").join(version);
        let package_json = prefix.join("lib/node_modules/@openai/codex/package.json");
        fs::create_dir_all(package_json.parent().unwrap()).unwrap();
        fs::write(&package_json, format!("{{\"version\":\"{version}\"}}")).unwrap();
        let internal_cli = prefix.join("bin/codex");
        let (launcher, fingerprint) =
            install_launcher(&paths, "codex", &internal_cli, &[], None).unwrap();

        let retained_runtime = paths.data.join("runtimes/node/22.22.0/bin/node");
        fs::create_dir_all(retained_runtime.parent().unwrap()).unwrap();
        fs::write(&retained_runtime, b"runtime stays installed").unwrap();
        let cli = InstalledCli {
            harness: HarnessId::Codex,
            version: version.to_owned(),
            executable: launcher.clone(),
            integrity: Some("sha512-test".to_owned()),
            runtime_bins: Vec::new(),
            launcher_fingerprint: Some(fingerprint),
        };

        remove_cli(&paths, &cli).unwrap();
        assert!(!launcher.exists());
        assert!(!prefix.exists());
        assert_eq!(
            fs::read(retained_runtime).unwrap(),
            b"runtime stays installed"
        );
        let backup_root = paths.state.join("backups/cli-launchers");
        assert_eq!(fs::read_dir(backup_root).unwrap().count(), 1);
    }
}
