use serde::Serialize;
use std::fs;
use std::io::Read;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::api::{InstallResult, InstalledCli, LaunchSpec, Outcome, Paths, ResourceKind};
use super::{adapters, state, tools};
use crate::install::engine::{ReportOperation, RunReport};
use crate::install::util;

pub fn run(paths: &Paths) -> InstallResult<RunReport> {
    let state = state::load(paths)?;
    let mut report = RunReport::default();
    report.notes.push(
        "Doctor is read-only: no sources, packages, MCP servers, or config files were changed."
            .into(),
    );
    report
        .notes
        .push("Authentication and remote model access were not checked.".into());

    if state.clis.is_empty()
        && state.bindings.is_empty()
        && state.artifacts.is_empty()
        && state.tools.is_empty()
    {
        report
            .notes
            .push("No recorded Yashik installation was found.".into());
    }

    for (harness, cli) in &state.clis {
        if cli.harness.as_str() != harness || verify_cli_launcher(paths, cli).is_err() {
            report.operations.push(ReportOperation {
                id: format!("cli/{harness}"),
                outcome: Outcome::Failed,
                message: Some("recorded CLI launcher is not a verified Yashik launcher".into()),
            });
            continue;
        }
        match fs::symlink_metadata(&cli.executable) {
            Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
                #[cfg(unix)]
                let executable = {
                    use std::os::unix::fs::PermissionsExt;
                    metadata.permissions().mode() & 0o111 != 0
                };
                #[cfg(not(unix))]
                let executable = true;
                if executable {
                    let (outcome, message) = match probe_cli_version(paths, cli) {
                        Ok(actual) if actual == cli.version => (
                            Outcome::Unchanged,
                            format!(
                                "{} {} is present and passed --version",
                                cli.harness.as_str(),
                                cli.version
                            ),
                        ),
                        Ok(_) => (
                            Outcome::Failed,
                            "installed CLI --version does not match the recorded package version"
                                .into(),
                        ),
                        Err(error) => (Outcome::Failed, error),
                    };
                    report.operations.push(ReportOperation {
                        id: format!("cli/{harness}"),
                        outcome,
                        message: Some(message),
                    });
                } else {
                    report.operations.push(ReportOperation {
                        id: format!("cli/{harness}"),
                        outcome: Outcome::Failed,
                        message: Some("recorded CLI binary is not executable".into()),
                    });
                }
            }
            Ok(_) => report.operations.push(ReportOperation {
                id: format!("cli/{harness}"),
                outcome: Outcome::Failed,
                message: Some("recorded CLI path is not a regular file".into()),
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                report.operations.push(ReportOperation {
                    id: format!("cli/{harness}"),
                    outcome: Outcome::Failed,
                    message: Some("recorded CLI binary is missing".into()),
                });
            }
            Err(_) => report.operations.push(ReportOperation {
                id: format!("cli/{harness}"),
                outcome: Outcome::Failed,
                message: Some("could not inspect recorded CLI binary".into()),
            }),
        }
    }

    for (id, tool) in &state.tools {
        let (outcome, message) = if id != &tool.id {
            (
                Outcome::Failed,
                "recorded tool key does not match its ID".to_owned(),
            )
        } else {
            match tools::doctor(paths, tool) {
                Ok(()) => (
                    Outcome::Unchanged,
                    format!("Herdr {} is present and passed --version", tool.version),
                ),
                Err(error) => (Outcome::Failed, error),
            }
        };
        report.operations.push(ReportOperation {
            id: format!("tool/{id}"),
            outcome,
            message: Some(message),
        });
    }

    for (key, artifact) in &state.artifacts {
        let outcome = if artifact.root.is_dir() {
            Outcome::Unchanged
        } else {
            Outcome::Failed
        };
        report.operations.push(ReportOperation {
            id: format!("artifact/{key}"),
            outcome,
            message: Some(if outcome == Outcome::Unchanged {
                "built artifact is present".into()
            } else {
                "recorded artifact is missing".into()
            }),
        });
    }

    for binding in state.bindings.values() {
        match adapters::inspect_managed(binding) {
            Ok(crate::install::api::Observation::Missing) => {
                report.operations.push(ReportOperation {
                    id: binding.id.clone(),
                    outcome: Outcome::Failed,
                    message: Some("recorded binding is missing".into()),
                });
            }
            Ok(crate::install::api::Observation::Present { fingerprint })
                if fingerprint == binding.fingerprint =>
            {
                report.operations.push(ReportOperation {
                    id: binding.id.clone(),
                    outcome: Outcome::Unchanged,
                    message: Some("recorded binding is present and unchanged".into()),
                });
            }
            Ok(crate::install::api::Observation::Present { .. }) => {
                report.operations.push(ReportOperation {
                    id: binding.id.clone(),
                    outcome: Outcome::Failed,
                    message: Some("recorded binding has drifted".into()),
                });
            }
            Err(_) => report.operations.push(ReportOperation {
                id: binding.id.clone(),
                outcome: Outcome::Failed,
                message: Some("could not inspect recorded binding".into()),
            }),
        }
    }

    report_missing_launch_environment(paths, &state, &mut report);

    for (id, operation) in &state.operations {
        match operation.phase {
            state::OperationPhase::Intent => report.operations.push(ReportOperation {
                id: format!("interrupted/{id}"),
                outcome: Outcome::Interrupted,
                message: Some(
                    "the last run stopped after recording intent; ownership was not inferred"
                        .into(),
                ),
            }),
            state::OperationPhase::Completed
                if matches!(
                    operation.outcome,
                    Outcome::Failed | Outcome::Blocked | Outcome::PendingRemoval | Outcome::Skipped
                ) =>
            {
                report.operations.push(ReportOperation {
                    id: format!("previous/{id}"),
                    outcome: operation.outcome,
                    message: operation
                        .message
                        .clone()
                        .or_else(|| Some("previous run recorded a non-success outcome".into())),
                });
            }
            _ => {}
        }
    }
    report
        .operations
        .sort_by(|left, right| left.id.cmp(&right.id));
    Ok(report)
}

fn verify_cli_launcher(paths: &Paths, cli: &InstalledCli) -> InstallResult<()> {
    let binary = match cli.harness {
        crate::schema::HarnessId::Codex => "codex",
        crate::schema::HarnessId::Claude => "claude",
        crate::schema::HarnessId::Opencode => "opencode",
        crate::schema::HarnessId::Pi => "pi",
        crate::schema::HarnessId::Omp => "omp",
    };
    if cli.executable != paths.bin.join(binary) {
        return Err("recorded launcher path is not managed".into());
    }
    let expected = cli
        .launcher_fingerprint
        .as_deref()
        .filter(|fingerprint| {
            fingerprint.len() == 64 && fingerprint.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
        .ok_or_else(|| "recorded launcher fingerprint is invalid".to_owned())?;
    let bytes = util::read_file_checked_bounded(&paths.bin, &cli.executable, 1024 * 1024)?;
    if util::sha256_bytes(&bytes) != expected {
        return Err("recorded launcher fingerprint has drifted".into());
    }
    Ok(())
}

fn probe_cli_version(paths: &Paths, cli: &InstalledCli) -> InstallResult<String> {
    if !crate::validation::is_valid_exact_harness_version(&cli.version) {
        return Err("recorded CLI version is not an exact semantic version".into());
    }
    let executable_name = match cli.harness {
        crate::schema::HarnessId::Codex => "codex",
        crate::schema::HarnessId::Claude => "claude",
        crate::schema::HarnessId::Opencode => "opencode",
        crate::schema::HarnessId::Pi => "pi",
        crate::schema::HarnessId::Omp => "omp",
    };
    let harness_root = paths.data.join("harnesses").join(cli.harness.as_str());
    let version_root = harness_root.join(&cli.version);
    let bin_root = version_root.join("bin");
    for directory in [
        paths.data.join("harnesses"),
        harness_root,
        version_root.clone(),
        bin_root.clone(),
    ] {
        match fs::symlink_metadata(directory) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            _ => return Err("curated harness installation directory is missing or unsafe".into()),
        }
    }
    let executable = bin_root.join(executable_name);
    // npm and Bun expose package executables through symlinks inside this prefix.
    let executable = fs::canonicalize(&executable)
        .map_err(|_| "curated harness executable is missing or unsafe".to_owned())?;
    if !executable.starts_with(&version_root)
        || !fs::metadata(&executable)
            .map(|m| m.is_file())
            .unwrap_or(false)
    {
        return Err("curated harness executable is missing or unsafe".into());
    }

    let mut search_path = cli.runtime_bins.clone();
    #[cfg(unix)]
    search_path.extend(
        ["/usr/bin", "/bin", "/usr/local/bin", "/opt/homebrew/bin"]
            .into_iter()
            .map(std::path::PathBuf::from),
    );
    #[cfg(not(unix))]
    if let Some(path) = std::env::var_os("PATH") {
        search_path.extend(std::env::split_paths(&path));
    }
    let search_path = std::env::join_paths(search_path)
        .map_err(|_| "cannot prepare the curated CLI version probe".to_owned())?;
    let temporary_home = TemporaryProbeHome::create()?;
    let output = run_version_probe(&executable, temporary_home.path(), &search_path)?;
    first_version(&output)
        .ok_or_else(|| "curated harness --version returned no semantic version".into())
}

struct TemporaryProbeHome(std::path::PathBuf);

impl TemporaryProbeHome {
    fn create() -> InstallResult<Self> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        let path = std::env::temp_dir().join(format!(
            "yashik-doctor-probe-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&path)
            .map_err(|_| "could not prepare an isolated CLI version probe".to_owned())?;
        let private_directories = [
            "tmp",
            ".config",
            ".cache",
            ".local/share",
            ".local/state",
            ".codex",
            ".agent",
        ];
        for relative in private_directories {
            if fs::create_dir_all(path.join(relative)).is_err() {
                let _ = fs::remove_dir_all(&path);
                return Err("could not prepare an isolated CLI version probe".into());
            }
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let setup = (|| {
                fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
                for relative in private_directories {
                    let directory = path.join(relative);
                    fs::create_dir_all(&directory)?;
                    fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
                }
                Ok::<(), std::io::Error>(())
            })();
            if setup.is_err() {
                let _ = fs::remove_dir_all(&path);
                return Err("could not secure the isolated CLI version probe".into());
            }
        }
        Ok(Self(path))
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TemporaryProbeHome {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run_version_probe(
    executable: &std::path::Path,
    home: &std::path::Path,
    search_path: &std::ffi::OsStr,
) -> InstallResult<Vec<u8>> {
    let mut child = Command::new(executable)
        .current_dir(home)
        .arg("--version")
        .env_clear()
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("CODEX_HOME", home.join(".codex"))
        .env("PI_CODING_AGENT_DIR", home.join(".agent"))
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("XDG_CACHE_HOME", home.join(".cache"))
        .env("XDG_DATA_HOME", home.join(".local/share"))
        .env("XDG_STATE_HOME", home.join(".local/state"))
        .env("TMPDIR", home.join("tmp"))
        .env("TEMP", home.join("tmp"))
        .env("TMP", home.join("tmp"))
        .env("PATH", search_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| "could not start the curated harness --version probe".to_owned())?;
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
                return Err("curated harness --version probe timed out".into());
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err("curated harness --version probe could not be completed".into());
            }
        }
    };
    let output = stdout_reader
        .join()
        .map_err(|_| "curated harness --version output capture failed".to_owned())?;
    let _ = stderr_reader.join();
    if !status.success() {
        return Err("curated harness --version probe failed".into());
    }
    Ok(output)
}

fn read_limited<R: Read>(reader: Option<R>, limit: usize) -> Vec<u8> {
    let Some(mut reader) = reader else {
        return Vec::new();
    };
    let mut captured = Vec::new();
    let mut buffer = [0; 1024];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(count) => {
                let remaining = limit.saturating_sub(captured.len());
                captured.extend_from_slice(&buffer[..count.min(remaining)]);
            }
        }
    }
    captured
}

fn discard_output<R: Read>(reader: Option<R>) {
    if let Some(mut reader) = reader {
        let _ = std::io::copy(&mut reader, &mut std::io::sink());
    }
}

fn first_version(output: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(output);
    text.split(|character: char| {
        !(character.is_ascii_alphanumeric()
            || character == '.'
            || character == '-'
            || character == '+')
    })
    .find_map(|token| {
        let token = token.strip_prefix('v').unwrap_or(token);
        crate::validation::is_valid_exact_harness_version(token).then(|| token.to_owned())
    })
}

fn report_missing_launch_environment(paths: &Paths, state: &state::State, report: &mut RunReport) {
    let directory = paths.state.join("launch-specs");
    match fs::symlink_metadata(&directory) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
        _ => {
            report.operations.push(ReportOperation {
                id: "launch-specs".into(),
                outcome: Outcome::Failed,
                message: Some("MCP launch specification path is not a real directory".into()),
            });
            return;
        }
    };
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(_) => {
            report.operations.push(ReportOperation {
                id: "launch-specs".into(),
                outcome: Outcome::Failed,
                message: Some("could not inspect saved MCP launch specifications".into()),
            });
            return;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let Some(id) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        if path.extension().and_then(|extension| extension.to_str()) != Some("json")
            || !valid_launch_id(id)
        {
            continue;
        }
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_file() || file_type.is_symlink() {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if metadata.len() > 1024 * 1024 {
            continue;
        }
        let Ok(bytes) = fs::read(&path) else {
            continue;
        };
        let Ok(spec) = serde_json::from_slice::<LaunchSpec>(&bytes) else {
            continue;
        };
        let Some(binding) = state
            .bindings
            .values()
            .find(|binding| binding.kind == ResourceKind::Mcp && launch_id(binding, &spec) == id)
        else {
            // Old specifications left after a confirmed removal are inert.
            continue;
        };
        let referenced = match super::launcher::referenced_env_names(&spec) {
            Ok(referenced) => referenced,
            Err(_) => {
                report.operations.push(ReportOperation {
                    id: format!("launch-env/{}", binding.id),
                    outcome: Outcome::Failed,
                    message: Some(
                        "saved MCP launch specification contains invalid interpolation".into(),
                    ),
                });
                continue;
            }
        };
        let mut missing = referenced
            .into_iter()
            .filter(|name| std::env::var_os(name).is_none())
            .collect::<Vec<_>>();
        missing.sort();
        missing.dedup();
        if !missing.is_empty() {
            report.operations.push(ReportOperation {
                id: format!("launch-env/{}", binding.id),
                outcome: Outcome::Blocked,
                message: Some(format!(
                    "MCP launch needs unset environment variables: {}",
                    missing.join(", ")
                )),
            });
        }
    }
}

#[derive(Serialize)]
struct LaunchIdentity<'a> {
    binding: &'a str,
    artifact: &'a str,
    run: &'a crate::schema::Run,
}

fn launch_id(binding: &super::api::ManagedBinding, spec: &LaunchSpec) -> String {
    let Some(artifact) = binding.artifact_keys.first() else {
        return String::new();
    };
    let Ok(bytes) = serde_json::to_vec(&LaunchIdentity {
        binding: &binding.id,
        artifact,
        run: &spec.run,
    }) else {
        return String::new();
    };
    util::sha256_bytes(&bytes)
}

fn valid_launch_id(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}
