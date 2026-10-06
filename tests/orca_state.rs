use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use yashik::install::api::{InstalledTool, Outcome, Paths};
use yashik::install::{doctor, state};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct TestRoot(PathBuf);

impl TestRoot {
    fn new() -> Self {
        let executable = std::env::current_exe().expect("resolve current test executable");
        let parent = executable.parent().expect("test executable has a parent");
        let path = parent.join(format!(
            "yashik-orca-state-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("create state fixture");
        Self(fs::canonicalize(path).expect("canonicalize state fixture"))
    }

    fn path(&self) -> &Path {
        &self.0
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

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn old_herdr_tool_state_without_orca_ownership_still_loads() {
    let fixture = TestRoot::new();
    let paths = fixture.paths();
    fs::create_dir_all(&paths.state).unwrap();
    fs::write(
        paths.state.join("state.json"),
        br#"{"version":1,"sources":{},"artifacts":{},"clis":{},"tools":{"herdr":{"id":"herdr","version":"0.9.3","metadata_url":"https://herdr.dev/latest.json","asset_url":"https://herdr.dev/releases/v0.9.3/herdr-linux-x86_64","sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","versioned_executable":"/home/example/.local/share/yashik/tools/herdr/versions/0.9.3/herdr","executable":"/home/example/.local/bin/herdr","fingerprint":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}},"bindings":{},"operations":{}}"#,
    )
    .unwrap();

    let loaded = state::load(&paths).unwrap();

    let herdr = loaded.tools.get("herdr").expect("load prior Herdr record");
    assert_eq!(herdr.version, "0.9.3");
    assert!(herdr.orca.is_none());
    assert!(!fixture.path().join("state/journal.jsonl").exists());
}

#[test]
fn doctor_reports_unknown_tool_ids_as_failed_without_network_or_state_changes() {
    let fixture = TestRoot::new();
    let paths = fixture.paths();
    let unknown = InstalledTool {
        id: "unknown-tool".into(),
        version: "1.2.3".into(),
        metadata_url: "https://invalid.example.test/releases/latest".into(),
        asset_url: "https://invalid.example.test/releases/1.2.3/asset".into(),
        sha256: "a".repeat(64),
        versioned_executable: fixture.path().join("data/unknown-tool/1.2.3/bin/tool"),
        executable: fixture.path().join("bin/tool"),
        fingerprint: "b".repeat(64),
        previous_versions: Vec::new(),
        orca: None,
    };
    let mut initial = state::State::empty();
    initial.tools.insert(unknown.id.clone(), unknown);
    state::save(&paths, &initial).unwrap();

    let state_file = paths.state.join("state.json");
    let original_bytes = fs::read(&state_file).unwrap();
    let original_file_modified = fs::metadata(&state_file).unwrap().modified().unwrap();
    let original_directory_modified = fs::metadata(&paths.state).unwrap().modified().unwrap();

    let report = doctor::run(&paths).unwrap();
    let operation = report
        .operations
        .iter()
        .find(|operation| operation.id == "tool/unknown-tool")
        .expect("report unknown tool");
    assert_eq!(operation.outcome, Outcome::Failed);
    assert!(operation
        .message
        .as_deref()
        .unwrap_or_default()
        .contains("unknown"));
    assert!(report.has_failures());

    assert_eq!(fs::read(&state_file).unwrap(), original_bytes);
    assert_eq!(
        fs::metadata(&state_file).unwrap().modified().unwrap(),
        original_file_modified
    );
    assert_eq!(
        fs::metadata(&paths.state).unwrap().modified().unwrap(),
        original_directory_modified
    );
    assert!(!paths.state.join("journal.jsonl").exists());
}
