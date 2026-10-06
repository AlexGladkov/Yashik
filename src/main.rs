use std::path::{Path, PathBuf};
use std::process::ExitCode;

use yashik::effective::{build_effective, EffectiveManifest};
use yashik::install::{doctor, engine, launcher, paths};
use yashik::schema::Manifest;
use yashik::validation::ValidationIssue;

const DEFAULT_MANIFEST: &str = "yashik-compose.yaml";

fn main() -> ExitCode {
    ExitCode::from(run(std::env::args().skip(1).collect()))
}

fn run(args: Vec<String>) -> u8 {
    match args.as_slice() {
        [command] if command == "--help" || command == "-h" || command == "help" => {
            print_help();
            0
        }
        [command] if command == "--version" || command == "-V" || command == "version" => {
            println!("yashik {}", env!("CARGO_PKG_VERSION"));
            0
        }
        [command] if command == "init" => match load_effective(Path::new(DEFAULT_MANIFEST), true) {
            Ok((manifest_path, effective)) => run_init(&manifest_path, &effective),
            Err(code) => code,
        },
        [command, manifest] if command == "check" => {
            match load_effective(Path::new(manifest), false) {
                Ok((manifest_path, effective)) => {
                    println!("Manifest is valid: {}", manifest_path.display());
                    report_effective(&effective);
                    0
                }
                Err(code) => code,
            }
        }
        [command, manifest] if command == "init" => {
            match load_effective(Path::new(manifest), false) {
                Ok((manifest_path, effective)) => run_init(&manifest_path, &effective),
                Err(code) => code,
            }
        }
        [command] if command == "doctor" => run_doctor(),
        [command, launch_id] if command == "mcp-launch" => run_launcher(launch_id),
        _ => {
            eprintln!("error: invalid arguments\n\n{}", usage());
            2
        }
    }
}

fn load_effective(
    manifest_path: &Path,
    is_default_manifest: bool,
) -> Result<(PathBuf, EffectiveManifest), u8> {
    let bytes = match std::fs::read(manifest_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            if is_default_manifest {
                eprintln!(
                    "error: could not read default manifest `{}`: {error}\nhint: pass an explicit path with `yashik init <manifest>`",
                    manifest_path.display()
                );
            } else {
                eprintln!("error: could not read manifest: {error}");
            }
            return Err(1);
        }
    };
    let manifest: Manifest = match serde_yaml::from_slice(&bytes) {
        Ok(manifest) => manifest,
        Err(error) => {
            let location = error
                .location()
                .map(|location| {
                    format!(" at line {}, column {}", location.line(), location.column())
                })
                .unwrap_or_default();
            eprintln!("error: invalid YAML or schema in manifest{location}");
            return Err(1);
        }
    };
    let absolute = match absolute_path(manifest_path) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("error: could not resolve manifest directory: {error}");
            return Err(1);
        }
    };
    let manifest_dir = absolute.parent().unwrap_or_else(|| Path::new("."));
    let effective = match build_effective(&manifest, manifest_dir) {
        Ok(effective) => effective,
        Err(issues) => {
            report_validation_errors(&issues);
            return Err(1);
        }
    };
    Ok((absolute, effective))
}

fn run_init(manifest_path: &Path, effective: &EffectiveManifest) -> u8 {
    let paths = match paths::discover() {
        Ok(paths) => paths,
        Err(error) => {
            eprintln!("error: could not discover user install paths: {error}");
            return 1;
        }
    };
    if let Err(error) = paths::create_private(&paths) {
        eprintln!("error: could not prepare private installer directories: {error}");
        return 1;
    }
    println!("Installing from {}", manifest_path.display());
    match engine::init(effective, &paths) {
        Ok(report) => {
            print_report(&report);
            print_path_hint(&paths.bin);
            if report.has_failures() {
                1
            } else {
                0
            }
        }
        Err(error) => {
            eprintln!("error: installation could not start: {error}");
            1
        }
    }
}

fn print_path_hint(bin: &Path) {
    let Some(current_path) = std::env::var_os("PATH") else {
        return;
    };
    if std::env::split_paths(&current_path).any(|entry| entry == bin) {
        return;
    }
    let path = bin.to_string_lossy();
    if path.contains(':') || path.contains('\n') || path.contains('\r') {
        return;
    }
    let quoted = format!("'{}'", path.replace('\'', "'\\''"));
    eprintln!(
        "Add Yashik's CLI directory to PATH in new shells with: export PATH={quoted}:\"$PATH\""
    );
}

fn run_doctor() -> u8 {
    let paths = match paths::discover() {
        Ok(paths) => paths,
        Err(error) => {
            eprintln!("error: could not discover user install paths: {error}");
            return 1;
        }
    };
    match doctor::run(&paths) {
        Ok(report) => {
            print_report(&report);
            if report.has_failures() {
                1
            } else {
                0
            }
        }
        Err(error) => {
            eprintln!("error: doctor could not read installer state: {error}");
            1
        }
    }
}

fn run_launcher(launch_id: &str) -> u8 {
    let paths = match paths::discover() {
        Ok(paths) => paths,
        Err(error) => {
            eprintln!("error: could not discover user install paths: {error}");
            return 1;
        }
    };
    match launcher::run(&paths, launch_id) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("error: MCP launcher: {error}");
            1
        }
    }
}

fn absolute_path(path: &Path) -> std::io::Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn report_validation_errors(issues: &[ValidationIssue]) {
    eprintln!("error: manifest validation failed");
    for issue in issues {
        eprintln!("  {}: {}", sanitize_issue_path(&issue.path), issue.message);
    }
}

fn sanitize_issue_path(path: &str) -> String {
    path.split('.')
        .map(|part| {
            let indexed = part.split_once('[').is_some_and(|(name, index)| {
                matches!(name, "requires" | "steps" | "args")
                    && index
                        .strip_suffix(']')
                        .is_some_and(|i| !i.is_empty() && i.bytes().all(|b| b.is_ascii_digit()))
            });
            let known = matches!(
                part,
                "version"
                    | "harnesses"
                    | "codex"
                    | "claude"
                    | "opencode"
                    | "pi"
                    | "omp"
                    | "tools"
                    | "herdr"
                    | "orca"
                    | "mcp"
                    | "skills"
                    | "agents"
                    | "rules"
                    | "source"
                    | "url"
                    | "ref"
                    | "path"
                    | "from"
                    | "run"
                    | "command"
                    | "env"
                    | "install"
                    | "requires"
                    | "steps"
                    | "format"
                    | "description"
                    | "enabled"
            );
            if known || indexed || valid_safe_identifier(part) {
                part.to_owned()
            } else {
                "<name>".to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(".")
}

fn valid_safe_identifier(value: &str) -> bool {
    let mut bytes = value.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    first.is_ascii_lowercase()
        && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && value.len() <= 63
}

fn report_effective(effective: &EffectiveManifest) {
    if effective.harnesses.is_empty() {
        println!("Harnesses: (none enabled)");
    }
    for harness in effective.harnesses.values() {
        let version = harness.version.as_deref().unwrap_or("latest");
        println!("\n{} (CLI version: {version})", harness.id.as_str());
        print_resource_names("MCP", harness.mcp.keys().map(String::as_str));
        print_resource_names("Skills", harness.skills.keys().map(String::as_str));
        print_resource_names("Agents", harness.agents.keys().map(String::as_str));
        print_resource_names("Rules", harness.rules.keys().map(String::as_str));
    }
    if let Some(herdr) = &effective.herdr {
        println!("\nHerdr (version: {})", herdr.version);
    }
    if let Some(orca) = &effective.orca {
        println!("\nOrca (version: {})", orca.version);
    }
}

fn print_resource_names<'a>(section: &str, names: impl Iterator<Item = &'a str>) {
    let names = names.collect::<Vec<_>>();
    if names.is_empty() {
        println!("  {section}: (none)");
    } else {
        println!("  {section}: {}", names.join(", "));
    }
}

fn print_report(report: &engine::RunReport) {
    for note in &report.notes {
        println!("{note}");
    }
    for operation in &report.operations {
        match &operation.message {
            Some(message) => println!("{:?}: {} — {message}", operation.outcome, operation.id),
            None => println!("{:?}: {}", operation.outcome, operation.id),
        }
    }
}

fn print_help() {
    println!("{}", usage());
    println!("`check` validates a manifest without changing the environment.");
    println!(
        "`init [<manifest>]` installs and reconciles the requested CLIs, tools, and resources."
    );
    println!("Without a path, `init` reads `./{DEFAULT_MANIFEST}` from the current directory.");
    println!(
        "`doctor` inspects recorded installations without changing files or accessing the network."
    );
    println!("`mcp-launch` is the internal stdio launcher used by registered MCP servers.");
}

fn usage() -> &'static str {
    "Usage:\n  yashik init [<manifest>]\n  yashik check <manifest>\n  yashik doctor\n  yashik mcp-launch <launch-id>\n  yashik --help\n  yashik --version"
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn explicit_manifest_resolves_relative_local_resources_from_manifest_directory() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("yashik-main-{}-{nonce}", std::process::id()));
        let manifest_dir = root.join("configs");
        let local_source = manifest_dir.join("skills/sample");
        fs::create_dir_all(&local_source).unwrap();
        let manifest_path = manifest_dir.join("custom.yaml");
        fs::write(
            &manifest_path,
            "version: 1\nharnesses:\n  codex: {}\nskills:\n  sample:\n    source:\n      type: local\n      path: ./skills/sample\n",
        )
        .unwrap();

        let (selected_path, effective) = load_effective(&manifest_path, false).unwrap();
        assert_eq!(selected_path, manifest_path);
        let resource = &effective.harnesses.values().next().unwrap().skills["sample"];
        assert_eq!(
            resource.local_source_path.as_deref(),
            Some(local_source.as_path())
        );

        fs::remove_dir_all(root).unwrap();
    }
}
