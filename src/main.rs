use std::path::{Path, PathBuf};
use std::process::ExitCode;

use yashik::effective::{build_effective, EffectiveManifest};
use yashik::schema::Manifest;
use yashik::validation::ValidationIssue;

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
        [command, manifest] if command == "init" => run_init(Path::new(manifest)),
        [command] if command == "doctor" => {
            eprintln!(
                "error: doctor is unavailable in this stage; no environment diagnostics were run"
            );
            2
        }
        _ => {
            eprintln!("error: invalid arguments\n\n{}", usage());
            2
        }
    }
}

fn run_init(manifest_path: &Path) -> u8 {
    let bytes = match std::fs::read(manifest_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!(
                "error: could not read manifest {}: {error}",
                manifest_path.display()
            );
            return 1;
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
            eprintln!(
                "error: invalid YAML or schema in manifest {}{location}",
                manifest_path.display()
            );
            return 1;
        }
    };
    let manifest_path = match absolute_path(manifest_path) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("error: could not resolve manifest directory: {error}");
            return 1;
        }
    };
    let manifest_dir = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    let effective = match build_effective(&manifest, manifest_dir) {
        Ok(effective) => effective,
        Err(issues) => {
            report_validation_errors(&manifest_path, &issues);
            return 1;
        }
    };
    report_effective(&manifest_path, &effective);
    0
}

fn absolute_path(path: &Path) -> std::io::Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn report_validation_errors(path: &Path, issues: &[ValidationIssue]) {
    eprintln!("error: manifest validation failed: {}", path.display());
    for issue in issues {
        eprintln!("  {}: {}", issue.path, issue.message);
    }
}

fn report_effective(path: &Path, effective: &EffectiveManifest) {
    println!("Manifest schema is valid: {}", path.display());
    println!("This stage only validates the schema; it does not install software, access sources, or change configuration.");
    if effective.harnesses.is_empty() {
        println!("No enabled harnesses are listed.");
        return;
    }
    for harness in effective.harnesses.values() {
        let version = harness.version.as_deref().unwrap_or("latest");
        println!("\n{} (CLI version: {version})", harness.id.as_str());
        print_resource_names("MCP", harness.mcp.keys().map(String::as_str));
        print_resource_names("Skills", harness.skills.keys().map(String::as_str));
        print_resource_names("Agents", harness.agents.keys().map(String::as_str));
        print_resource_names("Rules", harness.rules.keys().map(String::as_str));
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

fn print_help() {
    println!("{}", usage());
    println!("\n`init` validates a manifest and displays effective resource names. It performs no installation or configuration changes.");
    println!("`doctor` is unavailable in this stage.");
}

fn usage() -> &'static str {
    "Usage:\n  yashik init <manifest>\n  yashik doctor\n  yashik --help\n  yashik --version"
}
