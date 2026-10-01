use std::path::Path;
use std::process::{Command, Stdio};

use super::api::{InstallResult, RuntimeEnv};

const MAX_CAPTURE_BYTES: u64 = 32 * 1024;

/// Execute literal argv with an artifact working directory and curated runtime PATH.
/// Child output is drained with a fixed memory cap and never copied into errors.
pub fn run(argv: &[String], cwd: &Path, runtime: &RuntimeEnv) -> InstallResult<()> {
    let (program, arguments) = argv
        .split_first()
        .ok_or_else(|| "install step has no executable".to_owned())?;
    if program.is_empty() {
        return Err("install step has no executable".to_owned());
    }

    let mut command = Command::new(program);
    command
        .args(arguments)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut path_entries = runtime.bin_dirs.clone();
    if let Some(path) = std::env::var_os("PATH") {
        path_entries.extend(std::env::split_paths(&path));
    }
    let path =
        std::env::join_paths(path_entries).map_err(|_| "cannot construct child PATH".to_owned())?;
    command.env("PATH", path);

    let mut child = command
        .spawn()
        .map_err(|_| "could not start an install step executable".to_owned())?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdout_reader = std::thread::spawn(move || drain_bounded(stdout, MAX_CAPTURE_BYTES));
    let stderr_reader = std::thread::spawn(move || drain_bounded(stderr, MAX_CAPTURE_BYTES));
    let status = child
        .wait()
        .map_err(|_| "install step did not finish cleanly".to_owned())?;
    let _ = stdout_reader.join();
    let _ = stderr_reader.join();
    if !status.success() {
        return Err(format!(
            "install step failed{}",
            status
                .code()
                .map(|code| format!(" with exit code {code}"))
                .unwrap_or_default()
        ));
    }
    Ok(())
}

fn drain_bounded<R: std::io::Read>(reader: Option<R>, limit: u64) -> Vec<u8> {
    let Some(mut reader) = reader else {
        return Vec::new();
    };
    let mut retained = Vec::new();
    let mut buffer = [0u8; 8192];
    let mut seen = 0u64;
    loop {
        match reader.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(count) => {
                let remaining = limit.saturating_sub(seen) as usize;
                if remaining > 0 {
                    retained.extend_from_slice(&buffer[..count.min(remaining)]);
                }
                seen = seen.saturating_add(count as u64);
            }
        }
    }
    retained
}
