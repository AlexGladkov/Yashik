use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("yashik-cli-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&path).expect("create test directory");
        Self(path)
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

fn snapshot(root: &Path) -> Vec<(PathBuf, Option<Vec<u8>>)> {
    fn visit(root: &Path, current: &Path, items: &mut Vec<(PathBuf, Option<Vec<u8>>)>) {
        let mut entries = fs::read_dir(current)
            .expect("read directory")
            .map(Result::unwrap)
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            if path.is_dir() {
                items.push((path.strip_prefix(root).unwrap().to_path_buf(), None));
                visit(root, &path, items);
            } else {
                items.push((
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    Some(fs::read(path).unwrap()),
                ));
            }
        }
    }

    let mut items = Vec::new();
    visit(root, root, &mut items);
    items
}

fn run(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yashik"))
        .args(args)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .output()
        .expect("run yashik")
}

#[test]
fn commands_report_schema_only_behavior_and_never_change_home() {
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
    let invalid = temp.path().join("invalid.yaml");
    fs::write(&invalid, "version: 1\nharnesses: []\n").unwrap();
    let before = snapshot(&home);

    let manifest_arg = manifest.to_str().unwrap();
    let valid = run(&home, &["init", manifest_arg]);
    assert!(
        valid.status.success(),
        "{}",
        String::from_utf8_lossy(&valid.stderr)
    );
    let stdout = String::from_utf8(valid.stdout).unwrap();
    assert!(stdout.contains("schema is valid"));
    assert!(stdout.contains("does not install software"));
    assert!(stdout.contains("codex (CLI version: latest)"));
    assert!(stdout.contains("Skills: sample"));

    let invalid_arg = invalid.to_str().unwrap();
    let invalid = run(&home, &["init", invalid_arg]);
    assert!(!invalid.status.success());
    let wrong_args = run(&home, &["init"]);
    assert!(!wrong_args.status.success());
    let doctor = run(&home, &["doctor"]);
    assert!(!doctor.status.success());
    assert!(String::from_utf8_lossy(&doctor.stderr).contains("unavailable in this stage"));
    assert!(run(&home, &["--help"]).status.success());
    assert!(run(&home, &["--version"]).status.success());

    assert_eq!(snapshot(&home), before, "CLI commands modified HOME");
}

#[test]
fn output_does_not_print_environment_values_or_source_contents() {
    let temp = TestDir::new();
    let home = temp.path().join("home");
    fs::create_dir_all(&home).unwrap();
    let secret = "DO_NOT_PRINT_THIS_TOKEN";
    let manifest = temp.path().join("secret.yaml");
    fs::write(
        &manifest,
        format!(
            "version: 1\nharnesses:\n  codex: {{}}\nmcp:\n  private-name:\n    source:\n      type: local\n      path: ./local-source\n    transport: stdio\n    run:\n      command: node\n      env:\n        API_TOKEN: {secret}\n"
        ),
    )
    .unwrap();

    let output = run(&home, &["init", manifest.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!combined.contains(secret));
    assert!(combined.contains("private-name"));

    let malformed = temp.path().join("bad-secret.yaml");
    fs::write(
        &malformed,
        format!("version: 1\nharnesses:\n  codex:\n    enabled: {secret}\n"),
    )
    .unwrap();
    let invalid = run(&home, &["init", malformed.to_str().unwrap()]);
    assert!(!invalid.status.success());
    let error = String::from_utf8_lossy(&invalid.stderr);
    assert!(
        !error.contains(secret),
        "parser error exposed a manifest value"
    );
    assert!(
        error.contains("line"),
        "parser error omitted available location"
    );
}
