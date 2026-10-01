use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

use super::api::{InstallResult, LaunchSpec, Paths};

pub fn run(paths: &Paths, launch_id: &str) -> InstallResult<()> {
    if !valid_launch_id(launch_id) {
        return Err("invalid launch identifier".to_owned());
    }
    let spec_path = paths
        .state
        .join("launch-specs")
        .join(format!("{launch_id}.json"));
    let contents = std::fs::read(&spec_path)
        .map_err(|_| "launch specification is missing or unreadable".to_owned())?;
    let spec: LaunchSpec = serde_json::from_slice(&contents)
        .map_err(|_| "launch specification is invalid".to_owned())?;
    let command = expand(&spec.run.command, &spec.artifact_root)?;
    if command.is_empty() {
        return Err("expanded MCP command is empty".to_owned());
    }
    let args = spec
        .run
        .args
        .iter()
        .map(|argument| expand(argument, &spec.artifact_root))
        .collect::<InstallResult<Vec<_>>>()?;
    let env = spec
        .run
        .env
        .iter()
        .map(|(key, value)| {
            let name = value
                .strip_prefix("${env:")
                .and_then(|reference| reference.strip_suffix('}'))
                .filter(|name| valid_env_name(name))
                .ok_or_else(|| {
                    "launch specification has invalid environment references".to_owned()
                })?;
            let value = std::env::var_os(name)
                .ok_or_else(|| format!("required environment variable {name} is not set"))?;
            Ok((key.clone(), value))
        })
        .collect::<InstallResult<Vec<_>>>()?;

    let mut process = Command::new(command);
    process
        .args(args)
        .current_dir(&spec.artifact_root)
        .envs(env);
    prepend_path(&mut process, &spec.runtime_bins)?;

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let error = process.exec();
        Err(format!("could not start MCP process: {error}"))
    }
    #[cfg(not(unix))]
    {
        let status = process
            .status()
            .map_err(|_| "could not start MCP process".to_owned())?;
        if status.success() {
            Ok(())
        } else {
            Err(format!(
                "MCP process exited with status {}",
                status.code().unwrap_or(1)
            ))
        }
    }
}

fn expand(value: &str, artifact_root: &Path) -> InstallResult<OsString> {
    let mut result = OsString::new();
    let mut cursor = 0;
    for (start, end, token) in interpolation_tokens(value)? {
        result.push(&value[cursor..start]);
        match token {
            "source" => result.push(artifact_root.as_os_str()),
            _ => {
                let name = token
                    .strip_prefix("env:")
                    .filter(|name| valid_env_name(name))
                    .ok_or_else(|| {
                        "launch value contains an unknown interpolation token".to_owned()
                    })?;
                let env_value = std::env::var_os(name)
                    .ok_or_else(|| format!("required environment variable {name} is not set"))?;
                result.push(env_value);
            }
        }
        cursor = end + 1;
    }
    result.push(&value[cursor..]);
    Ok(result)
}

pub(crate) fn referenced_env_names(spec: &LaunchSpec) -> InstallResult<BTreeSet<String>> {
    let values = std::iter::once(spec.run.command.as_str())
        .chain(spec.run.args.iter().map(String::as_str))
        .collect::<Vec<_>>();
    let mut names = BTreeSet::new();
    for value in values {
        for (_, _, token) in interpolation_tokens(value)? {
            if token == "source" {
                continue;
            }
            if let Some(name) = token
                .strip_prefix("env:")
                .filter(|name| valid_env_name(name))
            {
                names.insert(name.to_owned());
            } else {
                return Err("launch value contains an unknown interpolation token".to_owned());
            }
        }
    }
    for value in spec.run.env.values() {
        let tokens = interpolation_tokens(value)?;
        if tokens.len() != 1
            || tokens[0].0 != 0
            || tokens[0].1 + 1 != value.len()
            || !tokens[0].2.starts_with("env:")
        {
            return Err("launch environment contains an invalid interpolation reference".into());
        }
        let name = tokens[0].2.strip_prefix("env:").expect("prefix checked");
        if !valid_env_name(name) {
            return Err("launch environment contains an invalid interpolation reference".into());
        }
        names.insert(name.to_owned());
    }
    Ok(names)
}

fn interpolation_tokens(value: &str) -> InstallResult<Vec<(usize, usize, &str)>> {
    let mut tokens = Vec::new();
    let mut cursor = 0;
    while let Some(relative_start) = value[cursor..].find("${") {
        let start = cursor + relative_start;
        let token_start = start + 2;
        let Some(relative_end) = value[token_start..].find('}') else {
            return Err("launch value contains an unclosed interpolation token".to_owned());
        };
        let end = token_start + relative_end;
        tokens.push((start, end, &value[token_start..end]));
        cursor = end + 1;
    }
    Ok(tokens)
}

fn prepend_path(command: &mut Command, runtime_bins: &[std::path::PathBuf]) -> InstallResult<()> {
    if runtime_bins.is_empty() {
        return Ok(());
    }
    let mut paths = runtime_bins.to_vec();
    if let Some(existing) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&existing));
    }
    let joined = std::env::join_paths(paths)
        .map_err(|_| "runtime PATH contains an invalid entry".to_owned())?;
    command.env("PATH", joined);
    Ok(())
}

fn valid_launch_id(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn valid_env_name(value: &str) -> bool {
    let mut bytes = value.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}
