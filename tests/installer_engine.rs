#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use yashik::install::api::{InstalledCli, LaunchSpec, ManagedBinding, Paths, ResourceKind};
use yashik::install::state::State;
use yashik::schema::Run;

struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before Unix epoch")
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("yashik-engine-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&path).expect("create test directory");
        Self(fs::canonicalize(path).expect("canonicalize test directory"))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn test_paths(root: &Path) -> Paths {
    let root = fs::canonicalize(root).expect("canonicalize test root");
    let home = root.join("home");
    fs::create_dir_all(&home).unwrap();
    let home = fs::canonicalize(home).unwrap();

    #[cfg(target_os = "macos")]
    let (data, cache, state, bin) = (
        home.join("Library/Application Support/yashik"),
        home.join("Library/Caches/yashik"),
        home.join("Library/Application Support/yashik/state"),
        home.join("Library/Application Support/yashik/bin"),
    );

    #[cfg(not(target_os = "macos"))]
    let (data, cache, state, bin) = (
        root.join("xdg-data/yashik"),
        root.join("xdg-cache/yashik"),
        root.join("xdg-state/yashik"),
        home.join(".local/bin"),
    );

    Paths {
        home,
        data,
        cache,
        state,
        bin,
    }
}

fn configure_child_paths(command: &mut Command, root: &Path) {
    let root = fs::canonicalize(root).expect("canonicalize test root");
    let paths = test_paths(&root);
    command
        .env("HOME", &paths.home)
        .env("USERPROFILE", &paths.home)
        .env("XDG_DATA_HOME", root.join("xdg-data"))
        .env("XDG_CACHE_HOME", root.join("xdg-cache"))
        .env("XDG_STATE_HOME", root.join("xdg-state"));
}

fn write_launch_spec(root: &Path, launch_id: &str, spec: &LaunchSpec) {
    let state = test_paths(root).state;
    let directory = state.join("launch-specs");
    fs::create_dir_all(&directory).unwrap();
    fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
    let path = directory.join(format!("{launch_id}.json"));
    fs::write(&path, serde_json::to_vec(spec).unwrap()).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

fn launch(root: &Path, launch_id: &str, marker: Option<&str>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_yashik"));
    command
        .args(["mcp-launch", launch_id])
        .env("CAPTURE_PATH", root.join("capture.txt"));
    configure_child_paths(&mut command, root);
    match marker {
        Some(marker) => {
            command.env("YASHIK_TEST_MCP_SECRET_68C4", marker);
        }
        None => {
            command.env_remove("YASHIK_TEST_MCP_SECRET_68C4");
        }
    }
    command.output().expect("launch MCP process")
}

fn doctor(root: &Path, missing_env: &str, marker: &str) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_yashik"));
    command
        .arg("doctor")
        .current_dir(root)
        .env("DOCTOR_SECRET_MARKER", marker)
        .env("API_TOKEN", marker)
        .env_remove(missing_env);
    configure_child_paths(&mut command, root);
    command.output().expect("run doctor")
}

#[derive(Serialize)]
struct LaunchIdentity<'a> {
    binding: &'a str,
    artifact: &'a str,
    run: &'a Run,
}

#[test]
fn doctor_lists_missing_mcp_environment_names_without_values() {
    let temp = TestDir::new();
    let root = temp.path();
    let paths = test_paths(root);
    let binding_id = "codex/mcp/private-service";
    let artifact_key = "private-artifact";
    let command_env = "YASHIK_DOCTOR_COMMAND_MISSING_7F21";
    let args_env = "YASHIK_DOCTOR_ARGS_MISSING_7F21";
    let map_env = "YASHIK_DOCTOR_MAP_MISSING_7F21";
    let run = Run {
        command: format!("node ${{env:{command_env}}}"),
        args: vec![format!("${{env:{args_env}}}"), "${source}/server.js".into()],
        env: [("API_TOKEN".to_owned(), format!("${{env:{map_env}}}"))]
            .into_iter()
            .collect(),
    };
    let launch_id = yashik::install::util::sha256_bytes(
        &serde_json::to_vec(&LaunchIdentity {
            binding: binding_id,
            artifact: artifact_key,
            run: &run,
        })
        .unwrap(),
    );
    let spec = LaunchSpec {
        artifact_root: root.join("artifact"),
        run,
        runtime_bins: Vec::new(),
    };
    write_launch_spec(root, &launch_id, &spec);

    let mut state = State::empty();
    state.bindings.insert(
        binding_id.to_owned(),
        ManagedBinding {
            id: binding_id.to_owned(),
            harness: yashik::schema::HarnessId::Codex,
            kind: ResourceKind::Mcp,
            names: vec!["private-service".to_owned()],
            target: root.join("config.toml"),
            selector: Some("private-service".to_owned()),
            fingerprint: "observed".to_owned(),
            desired_fingerprint: "desired".to_owned(),
            artifact_keys: vec![artifact_key.to_owned()],
        },
    );
    yashik::install::state::save(&paths, &state).unwrap();

    let marker = "SECRET_VALUE_MUST_NOT_BE_PRINTED_82AF";
    let output = doctor(root, command_env, marker);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!output.status.success(), "{stdout}");
    assert!(stdout.contains(command_env), "{stdout}");
    assert!(stdout.contains(args_env), "{stdout}");
    assert!(stdout.contains(map_env), "{stdout}");
    assert!(!stdout.contains(marker), "{stdout}");
}

#[test]
fn doctor_runs_only_the_curated_version_probe_and_discards_stderr() {
    let temp = TestDir::new();
    let root = temp.path();
    let paths = test_paths(root);
    let home = paths.home.clone();
    let internal = paths.data.join("harnesses/codex/1.2.3/bin/codex");
    fs::create_dir_all(internal.parent().unwrap()).unwrap();
    fs::create_dir_all(&paths.bin).unwrap();
    let launcher = paths.bin.join("codex");
    for executable in [&internal, &launcher] {
        fs::write(
            executable,
            "#!/bin/sh\nprintf 'codex 1.2.3\\n'\nmkdir -p \"$CODEX_HOME\"\ntouch \"$CODEX_HOME/probe-side-effect\"\ntouch probe-cwd-side-effect\nprintf '%s\\n' \"$API_TOKEN\" >&2\n",
        )
        .unwrap();
        fs::set_permissions(executable, fs::Permissions::from_mode(0o755)).unwrap();
    }
    // Match the actual npm prefix: bin/codex is a link to a package entrypoint.
    let package_entry = paths
        .data
        .join("harnesses/codex/1.2.3/lib/node_modules/codex/cli.js");
    fs::create_dir_all(package_entry.parent().unwrap()).unwrap();
    fs::rename(&internal, &package_entry).unwrap();
    std::os::unix::fs::symlink("../lib/node_modules/codex/cli.js", &internal).unwrap();
    let launcher_fingerprint = yashik::install::util::sha256_bytes(&fs::read(&launcher).unwrap());
    let mut state = State::empty();
    state.clis.insert(
        "codex".into(),
        InstalledCli {
            harness: yashik::schema::HarnessId::Codex,
            version: "1.2.3".into(),
            executable: launcher,
            integrity: None,
            runtime_bins: Vec::new(),
            launcher_fingerprint: Some(launcher_fingerprint),
        },
    );
    yashik::install::state::save(&paths, &state).unwrap();

    let marker = "DOCTOR_PROBE_SECRET_MUST_NOT_ESCAPE_189D";
    let matching = doctor(root, "DOCTOR_UNUSED", marker);
    let matching_stdout = String::from_utf8_lossy(&matching.stdout);
    let matching_stderr = String::from_utf8_lossy(&matching.stderr);
    assert!(
        matching.status.success(),
        "{matching_stdout}{matching_stderr}"
    );
    assert!(matching_stdout.contains("passed --version"));
    assert!(!matching_stdout.contains(marker));
    assert!(!matching_stderr.contains(marker));
    assert!(!home.join(".codex/probe-side-effect").exists());
    assert!(!root.join("probe-cwd-side-effect").exists());

    fs::write(
        &internal,
        "#!/bin/sh\nprintf 'codex 9.9.9\\n'\nprintf '%s\\n' \\\"$API_TOKEN\\\" >&2\n",
    )
    .unwrap();
    let mismatched = doctor(root, "DOCTOR_UNUSED", marker);
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&mismatched.stdout),
        String::from_utf8_lossy(&mismatched.stderr)
    );
    assert!(!mismatched.status.success());
    assert!(combined.contains("does not match the recorded package version"));
    assert!(!combined.contains("9.9.9"));
    assert!(!combined.contains(marker));
}

#[test]
fn mcp_launcher_expands_source_and_environment_only_when_started() {
    let temp = TestDir::new();
    let root = temp.path();
    let artifact = root.join("artifact");
    fs::create_dir_all(&artifact).unwrap();
    fs::write(artifact.join("payload.txt"), "ready").unwrap();
    let script = root.join("fake-mcp");
    fs::write(
        &script,
        "#!/bin/sh\nprintf '%s\\n%s\\n%s' \"$1\" \"$2\" \"$MCP_OVERRIDE\" > \"$CAPTURE_PATH\"\n",
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();

    let launch_id = "ab".repeat(32);
    let spec = LaunchSpec {
        artifact_root: artifact.clone(),
        run: Run {
            command: script.to_string_lossy().into_owned(),
            args: vec![
                "${source}/payload.txt".into(),
                "${env:YASHIK_TEST_MCP_SECRET_68C4}".into(),
            ],
            env: [(
                "MCP_OVERRIDE".to_owned(),
                "${env:YASHIK_TEST_MCP_SECRET_68C4}".to_owned(),
            )]
            .into_iter()
            .collect(),
        },
        runtime_bins: Vec::new(),
    };
    write_launch_spec(root, &launch_id, &spec);

    let marker = "MCP_SECRET_VALUE_NOT_FOR_LOGS_41CE";
    let output = launch(root, &launch_id, Some(marker));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains(marker));
    assert!(!String::from_utf8_lossy(&output.stderr).contains(marker));
    let captured = fs::read_to_string(root.join("capture.txt")).unwrap();
    assert_eq!(
        captured,
        format!(
            "{}\n{marker}\n{marker}",
            artifact.join("payload.txt").display()
        )
    );
}

#[test]
fn missing_launch_environment_reports_name_without_value() {
    let temp = TestDir::new();
    let root = temp.path();
    let artifact = root.join("artifact");
    fs::create_dir_all(&artifact).unwrap();
    let launch_id = "cd".repeat(32);
    let spec = LaunchSpec {
        artifact_root: artifact,
        run: Run {
            command: "/bin/true".into(),
            args: vec!["${env:YASHIK_TEST_MCP_SECRET_68C4}".into()],
            env: Default::default(),
        },
        runtime_bins: Vec::new(),
    };
    write_launch_spec(root, &launch_id, &spec);

    let output = launch(root, &launch_id, None);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("YASHIK_TEST_MCP_SECRET_68C4"));
    assert!(!stderr.contains("MCP_SECRET_VALUE_NOT_FOR_LOGS"));
    assert!(!root.join("capture.txt").exists());
}
