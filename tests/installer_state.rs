use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use yashik::install::api::{Outcome, Paths};
use yashik::install::state::{self, OperationPhase, OperationRecord, State, STATE_VERSION};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct TempTree(PathBuf);

impl TempTree {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "yashik-installer-state-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(fs::canonicalize(path).unwrap())
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

impl Drop for TempTree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn missing_state_load_is_empty_and_read_only() {
    let temp = TempTree::new();
    let paths = temp.paths();

    let state = state::load(&paths).unwrap();

    assert_eq!(state.version, STATE_VERSION);
    assert!(state.bindings.is_empty());
    assert!(!paths.state.exists());
}

#[test]
fn state_save_load_and_journal_keep_private_versioned_records() {
    let temp = TempTree::new();
    let paths = temp.paths();
    let mut record = OperationRecord::new(
        "codex/agent/backend-developer",
        Some("codex/agent/backend-developer".to_owned()),
        OperationPhase::Completed,
        Outcome::Succeeded,
        None,
    );
    record.updated_at_unix_ms = 1234;
    let mut saved = State::empty();
    saved.operations.insert(record.id.clone(), record.clone());

    state::save(&paths, &saved).unwrap();
    state::journal(&paths, &record).unwrap();
    let mut intent = record.clone();
    intent.phase = OperationPhase::Intent;
    state::journal(&paths, &intent).unwrap();
    let loaded = state::load(&paths).unwrap();

    assert_eq!(loaded.version, STATE_VERSION);
    assert_eq!(loaded.operations[&record.id].updated_at_unix_ms, 1234);
    let journal = fs::read_to_string(paths.state.join("journal.jsonl")).unwrap();
    assert_eq!(journal.lines().count(), 2);
    assert!(journal.contains("\"phase\":\"completed\""));
    assert!(journal.contains("\"phase\":\"intent\""));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&paths.state).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(paths.state.join("state.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(paths.state.join("journal.jsonl"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}

#[cfg(unix)]
#[test]
fn state_writes_reject_symlinked_parent_paths() {
    use std::os::unix::fs::symlink;

    let temp = TempTree::new();
    let paths = temp.paths();
    let outside = temp.path().join("outside-state");
    fs::create_dir_all(&outside).unwrap();
    symlink(&outside, &paths.state).unwrap();
    let record = OperationRecord::new(
        "codex/agent/reviewer",
        Some("codex/agent/reviewer".to_owned()),
        OperationPhase::Intent,
        Outcome::PendingRemoval,
        None,
    );

    assert!(state::save(&paths, &State::empty()).is_err());
    assert!(state::journal(&paths, &record).is_err());
    assert_eq!(fs::read_dir(outside).unwrap().count(), 0);
}

#[test]
fn corrupt_or_linked_state_is_rejected_without_repair() {
    let temp = TempTree::new();
    let paths = temp.paths();
    fs::create_dir_all(&paths.state).unwrap();
    fs::write(paths.state.join("state.json"), b"not json").unwrap();
    assert!(state::load(&paths).is_err());

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        fs::remove_file(paths.state.join("state.json")).unwrap();
        symlink(
            temp.path().join("outside.json"),
            paths.state.join("state.json"),
        )
        .unwrap();
        fs::write(temp.path().join("outside.json"), b"{}").unwrap();
        assert!(state::load(&paths).is_err());
        assert!(state::save(&paths, &State::empty()).is_err());
        assert_eq!(fs::read(temp.path().join("outside.json")).unwrap(), b"{}");
    }
}

#[cfg(unix)]
#[test]
fn journal_rejects_a_linked_target_without_changing_its_referent() {
    use std::os::unix::fs::symlink;

    let temp = TempTree::new();
    let paths = temp.paths();
    fs::create_dir_all(&paths.state).unwrap();
    let outside = temp.path().join("outside-journal.jsonl");
    fs::write(&outside, b"untouched\n").unwrap();
    symlink(&outside, paths.state.join("journal.jsonl")).unwrap();
    let record = OperationRecord::new(
        "codex/agent/reviewer",
        Some("codex/agent/reviewer".to_owned()),
        OperationPhase::Intent,
        Outcome::Succeeded,
        None,
    );

    assert!(state::journal(&paths, &record).is_err());
    assert_eq!(fs::read(&outside).unwrap(), b"untouched\n");
}
