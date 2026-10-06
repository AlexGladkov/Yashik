use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        let nonce = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("yashik-cli-{}-{nonce}", std::process::id()));
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

fn run(root: &Path, args: &[&str]) -> Output {
    let home = root.join("home");
    fs::create_dir_all(&home).unwrap();
    let root = fs::canonicalize(root).unwrap();
    let home = fs::canonicalize(home).unwrap();
    Command::new(env!("CARGO_BIN_EXE_yashik"))
        .args(args)
        .current_dir(&root)
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("XDG_DATA_HOME", root.join("xdg-data"))
        .env("XDG_CACHE_HOME", root.join("xdg-cache"))
        .env("XDG_STATE_HOME", root.join("xdg-state"))
        .output()
        .expect("run yashik")
}

#[test]
fn init_uses_default_manifest_from_current_directory_without_network_work() {
    let temp = TestDir::new();
    let manifest = temp.path().join("yashik-compose.yaml");
    fs::write(&manifest, "version: 1\nharnesses: {}\n").unwrap();

    let output = run(temp.path(), &["init"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout)
        .contains(&format!("Installing from {}", manifest.display())));
}

#[test]
fn init_missing_default_manifest_names_file_and_does_not_search_parent_or_write_state() {
    let temp = TestDir::new();
    fs::write(
        temp.path().join("yashik-compose.yaml"),
        "version: 1\nharnesses: {}\n",
    )
    .unwrap();
    let child = temp.path().join("child");
    fs::create_dir(&child).unwrap();

    let output = run(&child, &["init"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("yashik-compose.yaml"));
    assert!(stderr.contains("yashik init <manifest>"));
    assert_no_install_state(&child);
}

#[test]
fn init_rejects_invalid_default_manifest_before_writing_state() {
    let temp = TestDir::new();
    fs::write(
        temp.path().join("yashik-compose.yaml"),
        "version: [this is not valid\n",
    )
    .unwrap();

    let output = run(temp.path(), &["init"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("invalid YAML or schema"));
    assert_no_install_state(temp.path());
}

#[test]
fn init_explicit_path_overrides_default_manifest() {
    let temp = TestDir::new();
    fs::write(
        temp.path().join("yashik-compose.yaml"),
        "this default file is intentionally invalid\n",
    )
    .unwrap();
    let explicit = temp.path().join("custom.yaml");
    fs::write(&explicit, "version: 1\nharnesses: {}\n").unwrap();

    let output = run(temp.path(), &["init", explicit.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout)
        .contains(&format!("Installing from {}", explicit.display())));
}

#[test]
fn init_explicit_relative_path_in_subdirectory_overrides_default_manifest() {
    let temp = TestDir::new();
    fs::write(
        temp.path().join("yashik-compose.yaml"),
        "this default file is intentionally invalid\n",
    )
    .unwrap();
    let config_dir = temp.path().join("configs");
    fs::create_dir(&config_dir).unwrap();
    let explicit = config_dir.join("custom.yaml");
    fs::write(&explicit, "version: 1\nharnesses: {}\n").unwrap();

    let output = run(temp.path(), &["init", "configs/custom.yaml"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout)
        .contains(&format!("Installing from {}", explicit.display())));
}

#[test]
fn check_accepts_relative_manifest_with_local_resource_in_subdirectory() {
    let temp = TestDir::new();
    let config_dir = temp.path().join("configs");
    let local_source = config_dir.join("skills/sample");
    fs::create_dir_all(&local_source).unwrap();
    let manifest = config_dir.join("custom.yaml");
    fs::write(
        &manifest,
        "version: 1\nharnesses:\n  codex: {}\nskills:\n  sample:\n    source:\n      type: local\n      path: ./skills/sample\n",
    )
    .unwrap();

    let output = run(temp.path(), &["check", "configs/custom.yaml"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(&format!("Manifest is valid: {}", manifest.display())));
    assert!(stdout.contains("Skills: sample"));
}

#[test]
fn check_accepts_herdr_without_harnesses_and_does_not_create_installer_state() {
    let temp = TestDir::new();
    let manifest = temp.path().join("herdr.yaml");
    fs::write(
        &manifest,
        "version: 1\nharnesses: {}\ntools:\n  herdr: {}\n",
    )
    .unwrap();

    let output = run(temp.path(), &["check", "herdr.yaml"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Harnesses: (none enabled)"));
    assert!(stdout.contains("Herdr (version: latest)"));
    assert_no_install_state(temp.path());
}

#[test]
fn init_and_check_reject_invalid_operand_counts_before_writing_state() {
    let temp = TestDir::new();

    for args in [
        vec!["init", "one.yaml", "two.yaml"],
        vec!["check"],
        vec!["check", "one.yaml", "two.yaml"],
    ] {
        let output = run(temp.path(), &args);
        assert!(!output.status.success(), "unexpected success for {args:?}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("invalid arguments"));
        assert_no_install_state(temp.path());
    }
}

fn assert_no_install_state(root: &Path) {
    let root = fs::canonicalize(root).unwrap();
    #[cfg(target_os = "macos")]
    let home = fs::canonicalize(root.join("home")).unwrap();

    #[cfg(target_os = "macos")]
    let (data, cache, state) = (
        home.join("Library/Application Support/yashik"),
        home.join("Library/Caches/yashik"),
        home.join("Library/Application Support/yashik/state"),
    );

    #[cfg(not(target_os = "macos"))]
    let (data, cache, state) = (
        root.join("xdg-data/yashik"),
        root.join("xdg-cache/yashik"),
        root.join("xdg-state/yashik"),
    );

    assert!(
        !data.exists(),
        "unexpected installer data at {}",
        data.display()
    );
    assert!(
        !cache.exists(),
        "unexpected installer cache at {}",
        cache.display()
    );
    assert!(
        !state.exists(),
        "unexpected installer state at {}",
        state.display()
    );
}

#[test]
fn check_validates_effective_resources_without_creating_install_state() {
    let temp = TestDir::new();
    let home = temp.path().join("home");
    fs::create_dir_all(&home).unwrap();
    fs::write(home.join("existing.txt"), "keep").unwrap();
    let manifest = temp.path().join("manifest.yaml");
    fs::write(
        &manifest,
        "version: 1\nharnesses:\n  codex: {}\nskills:\n  sample:\n    source:\n      type: local\n      path: ./skills/sample\n",
    )
    .unwrap();

    let output = run(temp.path(), &["check", manifest.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Manifest is valid"));
    assert!(stdout.contains("codex (CLI version: latest)"));
    assert!(stdout.contains("Skills: sample"));
    assert_eq!(fs::read(home.join("existing.txt")).unwrap(), b"keep");
    assert_no_install_state(temp.path());
}

#[test]
fn doctor_on_empty_state_is_read_only() {
    let temp = TestDir::new();
    let home = temp.path().join("home");
    fs::create_dir_all(&home).unwrap();
    fs::write(home.join("existing.txt"), "keep").unwrap();

    let output = run(temp.path(), &["doctor"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("read-only"));
    assert!(stdout.contains("No recorded Yashik installation"));
    assert_no_install_state(temp.path());
    assert_eq!(fs::read(home.join("existing.txt")).unwrap(), b"keep");
}

#[test]
fn init_rejects_literal_env_values_without_leaking_them_or_writing_state() {
    let temp = TestDir::new();
    let secret = "YASHIK_CLI_TEST_LITERAL_SECRET_93A7";
    let manifest = temp.path().join("secret.yaml");
    fs::write(
        &manifest,
        format!(
            "version: 1\nharnesses:\n  codex: {{}}\nmcp:\n  private-name:\n    source:\n      type: local\n      path: ./source\n    transport: stdio\n    run:\n      command: node\n      env:\n        API_TOKEN: {secret}\n"
        ),
    )
    .unwrap();

    let output = run(temp.path(), &["init", manifest.to_str().unwrap()]);
    assert!(!output.status.success());
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!combined.contains(secret));
    assert!(combined.contains("run.env values must be a single"));
    assert_no_install_state(temp.path());
}

#[test]
fn init_rejects_malformed_cli_version_before_creating_private_paths() {
    let temp = TestDir::new();
    let manifest = temp.path().join("unsafe-version.yaml");
    fs::write(
        &manifest,
        "version: 1\nharnesses:\n  codex:\n    version: 'latest; echo unsafe'\n",
    )
    .unwrap();

    let output = run(temp.path(), &["init", manifest.to_str().unwrap()]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("must be `latest` or an exact semantic version"));
    assert_no_install_state(temp.path());
}

#[test]
fn init_rejects_unsafe_runtime_and_git_requirements_before_private_writes() {
    let cases = [
        "version: 1\nharnesses:\n  codex: {}\nmcp:\n  echo:\n    source:\n      type: local\n      path: ./source\n    transport: stdio\n    run:\n      command: node\n    install:\n      requires: ['node --version']\n",
        "version: 1\nharnesses: {}\nskills:\n  remote:\n    source:\n      type: git\n      url: https://example.invalid/../repo\n",
        "version: 1\nharnesses: {}\nskills:\n  remote:\n    source:\n      type: git\n      url: https://example.invalid/repo\n      ref: bad/foo..bar\n",
    ];
    for (index, contents) in cases.iter().enumerate() {
        let temp = TestDir::new();
        let manifest = temp.path().join(format!("invalid-{index}.yaml"));
        fs::write(&manifest, contents).unwrap();
        let output = run(temp.path(), &["init", manifest.to_str().unwrap()]);
        assert!(!output.status.success());
        assert_no_install_state(temp.path());
    }
}

#[test]
fn malformed_manifest_errors_do_not_echo_yaml_values() {
    let temp = TestDir::new();
    let secret = "YASHIK_PARSE_TEST_SECRET_73D9";
    let manifest = temp.path().join("invalid.yaml");
    fs::write(&manifest, format!("version: 1\nharnesses: [{secret}]\n")).unwrap();

    let output = run(temp.path(), &["check", manifest.to_str().unwrap()]);
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(!error.contains(secret));
    assert!(error.contains("invalid YAML or schema"));
}

#[test]
fn internal_launcher_rejects_path_like_ids_before_reading_state() {
    let temp = TestDir::new();
    let output = run(temp.path(), &["mcp-launch", "../../etc/passwd"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("invalid launch identifier"));
    assert_no_install_state(temp.path());
}

#[test]
fn help_and_version_are_available() {
    let temp = TestDir::new();
    let help = run(temp.path(), &["--help"]);
    assert!(help.status.success());
    let help = String::from_utf8_lossy(&help.stdout);
    assert!(help.contains("init [<manifest>]"));
    assert!(help.contains("yashik-compose.yaml"));
    assert!(help.contains("check <manifest>"));
    let version = run(temp.path(), &["--version"]);
    assert!(version.status.success());
    assert!(String::from_utf8_lossy(&version.stdout)
        .contains(concat!("yashik ", env!("CARGO_PKG_VERSION"))));
}
