use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::schema::{HarnessId, ResourceFormat, Run};

pub type InstallResult<T> = Result<T, String>;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Paths {
    pub home: PathBuf,
    pub data: PathBuf,
    pub cache: PathBuf,
    pub state: PathBuf,
    pub bin: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResolvedSource {
    pub key: String,
    pub revision: String,
    pub root: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeEnv {
    pub bin_dirs: Vec<PathBuf>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InstalledCli {
    pub harness: HarnessId,
    pub version: String,
    pub executable: PathBuf,
    pub integrity: Option<String>,
    pub runtime_bins: Vec<PathBuf>,
    #[serde(default)]
    pub launcher_fingerprint: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct InstalledToolVersion {
    pub version: String,
    pub asset_url: String,
    pub sha256: String,
    pub executable: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct InstalledTool {
    pub id: String,
    pub version: String,
    pub metadata_url: String,
    pub asset_url: String,
    pub sha256: String,
    pub versioned_executable: PathBuf,
    pub executable: PathBuf,
    pub fingerprint: String,
    /// Previously resolved releases retained under the Yashik data directory.
    #[serde(default)]
    pub previous_versions: Vec<InstalledToolVersion>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    Mcp,
    Skill,
    Agent,
    Rule,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BindingRequest {
    pub harness: HarnessId,
    pub kind: ResourceKind,
    pub name: String,
    pub format: Option<ResourceFormat>,
    pub description: Option<String>,
    pub source_path: Option<PathBuf>,
    pub artifact_key: Option<String>,
    pub launch_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum BindingPayload {
    File(Vec<u8>),
    Directory(PathBuf),
    Mcp(serde_json::Value),
    RulesBlock(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PreparedBinding {
    pub id: String,
    pub harness: HarnessId,
    pub kind: ResourceKind,
    pub names: Vec<String>,
    pub target: PathBuf,
    pub selector: Option<String>,
    pub desired_fingerprint: String,
    pub payload: BindingPayload,
    pub artifact_keys: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ManagedBinding {
    pub id: String,
    pub harness: HarnessId,
    pub kind: ResourceKind,
    pub names: Vec<String>,
    pub target: PathBuf,
    pub selector: Option<String>,
    pub fingerprint: String,
    pub desired_fingerprint: String,
    pub artifact_keys: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteChoice {
    Merge,
    Replace,
    Skip,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Observation {
    Missing,
    Present { fingerprint: String },
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Succeeded,
    Unchanged,
    Failed,
    Blocked,
    Skipped,
    PendingRemoval,
    Interrupted,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaunchSpec {
    pub artifact_root: PathBuf,
    pub run: Run,
    pub runtime_bins: Vec<PathBuf>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BootstrapRecipe {
    pub commands: Vec<Vec<String>>,
    pub needs_sudo: bool,
    pub missing_tools: Vec<String>,
}
