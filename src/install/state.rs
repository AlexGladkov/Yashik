use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::install::api::{
    InstallResult, InstalledCli, InstalledTool, ManagedBinding, Outcome, Paths,
};
use crate::install::util::{atomic_write, private_dir, read_file_checked_bounded};

pub const STATE_VERSION: u32 = 1;
const STATE_FILE: &str = "state.json";
const JOURNAL_FILE: &str = "journal.jsonl";
const MAX_STATE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_JOURNAL_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct State {
    pub version: u32,
    pub sources: BTreeMap<String, SourceRecord>,
    pub artifacts: BTreeMap<String, ArtifactRecord>,
    /// CLI records are keyed by the harness id (`codex`, `claude`, etc.).
    pub clis: BTreeMap<String, InstalledCli>,
    /// Independently managed tools are keyed by their closed manifest ID.
    #[serde(default)]
    pub tools: BTreeMap<String, InstalledTool>,
    pub bindings: BTreeMap<String, ManagedBinding>,
    /// Latest durable operation outcome per stable operation id.
    pub operations: BTreeMap<String, OperationRecord>,
}

impl Default for State {
    fn default() -> Self {
        Self::empty()
    }
}

impl State {
    pub fn empty() -> Self {
        Self {
            version: STATE_VERSION,
            sources: BTreeMap::new(),
            artifacts: BTreeMap::new(),
            clis: BTreeMap::new(),
            tools: BTreeMap::new(),
            bindings: BTreeMap::new(),
            operations: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceRecord {
    pub key: String,
    pub revision: String,
    pub root: PathBuf,
    /// Set only after the source URL has passed credential validation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ArtifactRecord {
    pub key: String,
    pub source_key: String,
    pub root: PathBuf,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OperationPhase {
    Intent,
    Completed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OperationRecord {
    pub id: String,
    pub binding_id: Option<String>,
    pub phase: OperationPhase,
    pub outcome: Outcome,
    pub updated_at_unix_ms: u128,
    pub message: Option<String>,
}

impl OperationRecord {
    pub fn new(
        id: impl Into<String>,
        binding_id: Option<String>,
        phase: OperationPhase,
        outcome: Outcome,
        message: Option<String>,
    ) -> Self {
        Self {
            id: id.into(),
            binding_id,
            phase,
            outcome,
            updated_at_unix_ms: now_unix_ms(),
            message,
        }
    }
}

pub fn load(paths: &Paths) -> InstallResult<State> {
    reject_symlink_components(&paths.state)?;
    let state_path = state_path(paths);
    let metadata = match fs::symlink_metadata(&state_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(State::empty()),
        Err(_) => return Err("could not inspect installer state".into()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("installer state is not a regular file".into());
    }
    if metadata.len() > MAX_STATE_BYTES {
        return Err("installer state exceeds the supported size".into());
    }

    let bytes = read_file_checked_bounded(&paths.state, &state_path, MAX_STATE_BYTES)?;
    let state: State = serde_json::from_slice(&bytes)
        .map_err(|_| "installer state is corrupt or has an unsupported format".to_string())?;
    if state.version != STATE_VERSION {
        return Err("installer state version is unsupported".into());
    }
    Ok(state)
}

pub fn save(paths: &Paths, state: &State) -> InstallResult<()> {
    if state.version != STATE_VERSION {
        return Err("refusing to save an unsupported installer state version".into());
    }
    ensure_state_dir(&paths.state)?;
    let destination = state_path(paths);
    reject_symlink_file(&destination)?;
    let bytes = serde_json::to_vec_pretty(state)
        .map_err(|_| "could not serialize installer state".to_string())?;
    if bytes.len() as u64 > MAX_STATE_BYTES {
        return Err("installer state exceeds the supported size".into());
    }
    atomic_write(&destination, &bytes, 0o600)
}

/// Append an intent or operation result to the durable journal.
///
/// Journal entries do not establish resource ownership. The engine commits a
/// successful `ManagedBinding` separately through `save`.
pub fn journal(paths: &Paths, record: &OperationRecord) -> InstallResult<()> {
    ensure_state_dir(&paths.state)?;
    let path = paths.state.join(JOURNAL_FILE);
    reject_symlink_file(&path)?;
    let mut bytes = match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err("installer operation journal is not a regular file".into());
        }
        Ok(metadata) if metadata.len() > MAX_JOURNAL_BYTES => {
            return Err("installer operation journal exceeds the supported size".into());
        }
        Ok(_) => read_file_checked_bounded(&paths.state, &path, MAX_JOURNAL_BYTES)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(_) => return Err("could not inspect installer operation journal".into()),
    };
    if !bytes.is_empty() && !bytes.ends_with(b"\n") {
        return Err("installer operation journal has an incomplete final record".into());
    }
    let record_bytes = serde_json::to_vec(record)
        .map_err(|_| "could not serialize installer operation record".to_string())?;
    let next_size = bytes
        .len()
        .saturating_add(record_bytes.len())
        .saturating_add(1);
    if next_size as u64 > MAX_JOURNAL_BYTES {
        return Err("installer operation journal exceeds the supported size".into());
    }
    bytes.extend_from_slice(&record_bytes);
    bytes.push(b'\n');
    atomic_write(&path, &bytes, 0o600)
}

fn state_path(paths: &Paths) -> PathBuf {
    paths.state.join(STATE_FILE)
}

fn ensure_state_dir(path: &Path) -> InstallResult<()> {
    reject_symlink_components(path)?;
    private_dir(path)?;
    reject_symlink_components(path)?;
    Ok(())
}

fn reject_symlink_components(path: &Path) -> InstallResult<()> {
    if !path.is_absolute() {
        return Err("installer state directory must be an absolute path".into());
    }
    let components = path.components().collect::<Vec<_>>();
    let mut current = PathBuf::new();
    for (index, component) in components.iter().enumerate() {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err("installer state path contains a symbolic link".into());
            }
            Ok(metadata) if index + 1 < components.len() && !metadata.is_dir() => {
                return Err("installer state parent is not a directory".into());
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(_) => return Err("could not inspect installer state path".into()),
        }
    }
    Ok(())
}

fn reject_symlink_file(path: &Path) -> InstallResult<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err("installer private file is not a regular file".into())
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("could not inspect installer private file".into()),
    }
}

fn now_unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}
