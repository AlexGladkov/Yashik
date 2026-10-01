use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use jsonc_parser::cst::{CstInputValue, CstNode, CstRootNode};
use jsonc_parser::ParseOptions;
use serde_json::{json, Value as JsonValue};
use toml_edit::{
    Array as TomlArray, DocumentMut, Item as TomlItem, Table as TomlTable, Value as TomlValue,
};

use crate::install::api::{
    BindingPayload, BindingRequest, InstallResult, InstalledCli, ManagedBinding, Observation,
    Paths, PreparedBinding, ResourceKind, WriteChoice,
};
use crate::install::util::{
    atomic_write, copy_tree_checked, create_private_dir_new, hash_tree, private_dir, sha256_bytes,
};
use crate::schema::{HarnessId, ResourceFormat};

const RULES_START: &str = "<!-- yashik:rules:start -->";
const RULES_END: &str = "<!-- yashik:rules:end -->";
const AUTH_FIELDS: &[&str] = &[
    "env",
    "environment",
    "headers",
    "oauth",
    "auth",
    "authentication",
    "authorization",
    "token",
];
const MAX_SOURCE_FILE: u64 = 16 * 1024 * 1024;

pub fn capability(
    harness: HarnessId,
    kind: ResourceKind,
    format: Option<ResourceFormat>,
    name: &str,
) -> InstallResult<()> {
    validate_name(name)?;
    let format = format.unwrap_or(ResourceFormat::Portable);
    let supported = matches!(
        (harness, kind, format.clone()),
        (HarnessId::Codex, ResourceKind::Mcp | ResourceKind::Skill, _)
            | (HarnessId::Codex, ResourceKind::Agent, _)
            | (
                HarnessId::Codex,
                ResourceKind::Rule,
                ResourceFormat::Portable
            )
            | (
                HarnessId::Claude,
                ResourceKind::Mcp | ResourceKind::Skill | ResourceKind::Agent | ResourceKind::Rule,
                _
            )
            | (
                HarnessId::Opencode,
                ResourceKind::Mcp | ResourceKind::Skill,
                _
            )
            | (HarnessId::Opencode, ResourceKind::Agent, _)
            | (
                HarnessId::Opencode,
                ResourceKind::Rule,
                ResourceFormat::Portable
            )
            | (HarnessId::Pi, ResourceKind::Mcp | ResourceKind::Skill, _)
            | (HarnessId::Pi, ResourceKind::Rule, ResourceFormat::Portable)
            | (
                HarnessId::Omp,
                ResourceKind::Mcp | ResourceKind::Skill | ResourceKind::Agent,
                _
            )
            | (HarnessId::Omp, ResourceKind::Rule, ResourceFormat::Portable)
    );
    if supported {
        Ok(())
    } else {
        Err(format!(
            "{} does not support {:?} resources in {:?} format",
            harness.as_str(),
            kind,
            format
        ))
    }
}

/// Check both the resource shape and the exact CLI contract exercised by the
/// adapter tests. Unknown versions remain unsupported until their paths and
/// configuration behavior have been verified.
pub fn capability_for_cli(
    cli: &InstalledCli,
    kind: ResourceKind,
    format: Option<ResourceFormat>,
    name: &str,
) -> InstallResult<()> {
    capability(cli.harness, kind, format, name)?;
    let verified_version = match cli.harness {
        HarnessId::Codex => "0.159.3",
        HarnessId::Claude => "2.1.286",
        HarnessId::Opencode => "1.18.34",
        HarnessId::Pi => "0.99.2",
        HarnessId::Omp => "18.4.8",
    };
    if cli.version != verified_version {
        return Err(format!(
            "{} adapter contract is verified only for version {} (found {})",
            cli.harness.as_str(),
            verified_version,
            cli.version
        ));
    }
    Ok(())
}

pub fn prepare(
    paths: &Paths,
    cli: &InstalledCli,
    requests: &[BindingRequest],
) -> InstallResult<Vec<PreparedBinding>> {
    if cli.harness
        != requests
            .first()
            .map(|request| request.harness)
            .unwrap_or(cli.harness)
    {
        return Err("adapter request does not match installed CLI".into());
    }
    let mut prepared = Vec::with_capacity(requests.len());
    let mut portable_rules = Vec::new();
    for request in requests {
        capability_for_cli(cli, request.kind, request.format.clone(), &request.name)?;
        if request.harness != cli.harness {
            return Err("adapter request does not match installed CLI".into());
        }
        if request.harness == HarnessId::Omp {
            reject_unsupported_omp_profile(&paths.home)?;
        }
        if request.kind == ResourceKind::Rule
            && request.format.as_ref().unwrap_or(&ResourceFormat::Portable)
                == &ResourceFormat::Portable
            && matches!(
                request.harness,
                HarnessId::Codex | HarnessId::Opencode | HarnessId::Pi | HarnessId::Omp
            )
        {
            portable_rules.push(request.clone());
        } else {
            prepared.push(prepare_one(paths, cli, request)?);
        }
    }
    if !portable_rules.is_empty() {
        prepared.push(prepare_rules_block(paths, cli, &portable_rules)?);
    }
    prepared.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(prepared)
}

fn prepare_one(
    paths: &Paths,
    _cli: &InstalledCli,
    request: &BindingRequest,
) -> InstallResult<PreparedBinding> {
    let harness = request.harness;
    let name = &request.name;
    let format = request.format.clone().unwrap_or(ResourceFormat::Portable);
    let (target, selector, payload) = match request.kind {
        ResourceKind::Mcp => {
            let launch_id = request
                .launch_id
                .as_deref()
                .ok_or_else(|| "MCP binding is missing its launch id".to_string())?;
            let payload = mcp_projection(paths, harness, launch_id)?;
            let target = mcp_config_path(paths, harness)?;
            (target, Some(name.clone()), BindingPayload::Mcp(payload))
        }
        ResourceKind::Skill => {
            let source = request
                .source_path
                .as_deref()
                .ok_or_else(|| "skill binding is missing its source directory".to_string())?;
            validate_skill(source, name)?;
            let target = match harness {
                HarnessId::Codex => paths.home.join(".agents/skills").join(name),
                HarnessId::Claude => paths.home.join(".claude/skills").join(name),
                HarnessId::Opencode => opencode_root(paths)?.join("skill").join(name),
                HarnessId::Pi => agent_dir(paths, harness)?.join("skills").join(name),
                HarnessId::Omp => agent_dir(paths, harness)?.join("skills").join(name),
            };
            if harness == HarnessId::Opencode {
                reject_opencode_plural_collision(
                    &target,
                    &opencode_root(paths)?.join("skills").join(name),
                )?;
            }
            (
                target,
                None,
                BindingPayload::Directory(source.to_path_buf()),
            )
        }
        ResourceKind::Agent => {
            let source = request
                .source_path
                .as_deref()
                .ok_or_else(|| "agent binding is missing its source file".to_string())?;
            let source_bytes = read_source_file(source)?;
            let bytes = match (harness, format) {
                (HarnessId::Codex, ResourceFormat::Native) => {
                    validate_codex_native(&source_bytes, name)?;
                    source_bytes
                }
                (HarnessId::Codex, ResourceFormat::Portable) => {
                    render_codex_agent(name, request.description.as_deref(), &source_bytes)?
                }
                (HarnessId::Claude, ResourceFormat::Native)
                | (HarnessId::Opencode, ResourceFormat::Native)
                | (HarnessId::Omp, ResourceFormat::Native) => {
                    validate_markdown_agent(&source_bytes, name)?;
                    source_bytes
                }
                (HarnessId::Claude, ResourceFormat::Portable)
                | (HarnessId::Opencode, ResourceFormat::Portable)
                | (HarnessId::Omp, ResourceFormat::Portable) => render_markdown_agent(
                    harness,
                    name,
                    request.description.as_deref(),
                    &source_bytes,
                )?,
                (HarnessId::Pi, _) => return Err("Pi custom agents are unsupported".into()),
            };
            let target = match harness {
                HarnessId::Codex => paths
                    .home
                    .join(".codex/agents")
                    .join(format!("{name}.toml")),
                HarnessId::Claude => paths.home.join(".claude/agents").join(format!("{name}.md")),
                HarnessId::Opencode => opencode_root(paths)?
                    .join("agent")
                    .join(format!("{name}.md")),
                HarnessId::Omp => agent_dir(paths, harness)?
                    .join("agents")
                    .join(format!("{name}.md")),
                HarnessId::Pi => unreachable!(),
            };
            if harness == HarnessId::Opencode {
                reject_opencode_plural_collision(
                    &target,
                    &opencode_root(paths)?
                        .join("agents")
                        .join(format!("{name}.md")),
                )?;
            }
            (target, None, BindingPayload::File(bytes))
        }
        ResourceKind::Rule => {
            if harness == HarnessId::Claude {
                let source = request
                    .source_path
                    .as_deref()
                    .ok_or_else(|| "rule binding is missing its source file".to_string())?;
                let bytes = read_source_file(source)?;
                (
                    paths.home.join(".claude/rules").join(format!("{name}.md")),
                    None,
                    BindingPayload::File(bytes),
                )
            } else {
                return Err("portable rules must be prepared as one harness block".into());
            }
        }
    };
    let desired_fingerprint = fingerprint_payload(&payload)?;
    Ok(PreparedBinding {
        id: binding_id(harness, request.kind, name),
        harness,
        kind: request.kind,
        names: vec![name.clone()],
        target,
        selector,
        desired_fingerprint,
        payload,
        artifact_keys: request.artifact_key.iter().cloned().collect(),
    })
}

fn prepare_rules_block(
    paths: &Paths,
    cli: &InstalledCli,
    requests: &[BindingRequest],
) -> InstallResult<PreparedBinding> {
    let harness = requests
        .first()
        .map(|request| request.harness)
        .ok_or_else(|| "rules block is empty".to_string())?;
    if requests
        .iter()
        .any(|request| request.harness != harness || request.kind != ResourceKind::Rule)
    {
        return Err("portable rules block contains mixed harness resources".into());
    }
    if harness != cli.harness {
        return Err("adapter request does not match installed CLI".into());
    }
    let mut names = requests
        .iter()
        .map(|request| request.name.clone())
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    if names.len() != requests.len() {
        return Err("portable rules block contains duplicate names".into());
    }
    let mut block = String::from(RULES_START);
    block.push('\n');
    for request in requests {
        let source = request
            .source_path
            .as_deref()
            .ok_or_else(|| "rule binding is missing its source file".to_string())?;
        let bytes = read_source_file(source)?;
        let body = std::str::from_utf8(&bytes)
            .map_err(|_| "portable rule source is not UTF-8".to_string())?;
        block.push_str("## ");
        block.push_str(&request.name);
        block.push('\n');
        block.push_str(body.trim_end_matches('\n'));
        block.push_str("\n\n");
    }
    block.push_str(RULES_END);
    let target = match harness {
        HarnessId::Codex => paths.home.join(".codex/AGENTS.md"),
        HarnessId::Opencode => opencode_root(paths)?.join("AGENTS.md"),
        HarnessId::Pi | HarnessId::Omp => agent_dir(paths, harness)?.join("AGENTS.md"),
        HarnessId::Claude => return Err("Claude rules use individual Markdown files".into()),
    };
    let payload = BindingPayload::RulesBlock(block);
    Ok(PreparedBinding {
        id: format!("{}/rules-block", harness.as_str()),
        harness,
        kind: ResourceKind::Rule,
        names,
        target,
        selector: Some("rules-block".into()),
        desired_fingerprint: fingerprint_payload(&payload)?,
        payload,
        artifact_keys: requests
            .iter()
            .filter_map(|request| request.artifact_key.clone())
            .collect(),
    })
}

fn mcp_projection(paths: &Paths, harness: HarnessId, launch_id: &str) -> InstallResult<JsonValue> {
    if launch_id.is_empty() || launch_id.contains('/') || launch_id.contains('\\') {
        return Err("MCP launch id is invalid".into());
    }
    let launcher = paths.bin.join("yashik");
    let mut args = vec!["mcp-launch".to_owned(), launch_id.to_owned()];
    let command = launcher.to_string_lossy().into_owned();
    Ok(match harness {
        HarnessId::Opencode => json!({
            "type": "local",
            "command": [command, args.remove(0), args.remove(0)],
            "enabled": true
        }),
        HarnessId::Omp => json!({
            "type": "stdio",
            "command": command,
            "args": args
        }),
        _ => json!({ "command": command, "args": args }),
    })
}

fn mcp_config_path(paths: &Paths, harness: HarnessId) -> InstallResult<PathBuf> {
    Ok(match harness {
        HarnessId::Codex => paths.home.join(".codex/config.toml"),
        HarnessId::Claude => paths.home.join(".claude.json"),
        HarnessId::Opencode => opencode_config_file(paths)?,
        HarnessId::Pi | HarnessId::Omp => agent_dir(paths, harness)?.join("mcp.json"),
    })
}

fn opencode_root(paths: &Paths) -> InstallResult<PathBuf> {
    let root = if let Some(config_dir) = std::env::var_os("OPENCODE_CONFIG_DIR") {
        let path = PathBuf::from(config_dir);
        if !path.is_absolute() {
            return Err("OPENCODE_CONFIG_DIR must be an absolute path".into());
        }
        path
    } else if let Some(config_home) = std::env::var_os("XDG_CONFIG_HOME") {
        let path = PathBuf::from(config_home);
        if !path.is_absolute() {
            return Err("XDG_CONFIG_HOME must be an absolute path".into());
        }
        path.join("opencode")
    } else {
        paths.home.join(".config/opencode")
    };
    Ok(root)
}

fn opencode_config_file(paths: &Paths) -> InstallResult<PathBuf> {
    let root = opencode_root(paths)?;
    for name in ["opencode.jsonc", "opencode.json", "config.json"] {
        let candidate = root.join(name);
        match fs::symlink_metadata(&candidate) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err("OpenCode config path is a symbolic link".into());
            }
            Ok(metadata) if metadata.is_file() => return Ok(candidate),
            Ok(_) => return Err("OpenCode config path is not a regular file".into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("could not inspect OpenCode config".into()),
        }
    }
    Ok(root.join("opencode.jsonc"))
}

fn reject_opencode_plural_collision(singular: &Path, plural: &Path) -> InstallResult<()> {
    check_no_symlink_components(singular)?;
    check_no_symlink_components(plural)?;
    match fs::symlink_metadata(plural) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err("OpenCode plural resource path is a symbolic link".into())
        }
        Ok(_) => Err("OpenCode has an unmanaged resource at its plural discovery path".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("could not inspect OpenCode plural resource path".into()),
    }
}

fn agent_dir(paths: &Paths, harness: HarnessId) -> InstallResult<PathBuf> {
    if let Some(directory) = std::env::var_os("PI_CODING_AGENT_DIR") {
        let path = PathBuf::from(directory);
        if !path.is_absolute() {
            return Err("PI_CODING_AGENT_DIR must be an absolute path".into());
        }
        return Ok(path);
    }
    match harness {
        HarnessId::Pi => Ok(paths.home.join(".pi/agent")),
        HarnessId::Omp => {
            reject_unsupported_omp_profile(&paths.home)?;
            Ok(paths.home.join(".omp/agent"))
        }
        _ => Err("agent directory is only defined for Pi and OMP".into()),
    }
}

fn reject_unsupported_omp_profile(home: &Path) -> InstallResult<()> {
    if std::env::var_os("PI_CODING_AGENT_DIR").is_some() {
        return Ok(());
    }
    if std::env::var_os("OMP_PROFILE").is_some() || std::env::var_os("OH_MY_PI_PROFILE").is_some() {
        return Err("OMP profile selection is unsupported; set PI_CODING_AGENT_DIR to the active profile agent directory".into());
    }
    let profiles = home.join(".omp/profiles");
    if let Ok(entries) = fs::read_dir(&profiles) {
        for entry in entries {
            let entry = entry.map_err(|_| "could not inspect OMP profiles".to_string())?;
            if !entry
                .file_type()
                .map_err(|_| "could not inspect OMP profiles".to_string())?
                .is_dir()
            {
                continue;
            }
            if fs::symlink_metadata(entry.path().join("agent")).is_ok() {
                return Err("OMP has named profiles and the active profile is unknown; set PI_CODING_AGENT_DIR to its agent directory".into());
            }
        }
    }
    Ok(())
}

fn binding_id(harness: HarnessId, kind: ResourceKind, name: &str) -> String {
    format!("{}/{}/{}", harness.as_str(), kind_name(kind), name)
}

fn kind_name(kind: ResourceKind) -> &'static str {
    match kind {
        ResourceKind::Mcp => "mcp",
        ResourceKind::Skill => "skill",
        ResourceKind::Agent => "agent",
        ResourceKind::Rule => "rule",
    }
}

fn validate_name(name: &str) -> InstallResult<()> {
    let bytes = name.as_bytes();
    if bytes.is_empty()
        || bytes.len() > 63
        || !bytes[0].is_ascii_lowercase()
        || !bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
    {
        return Err("resource name must match [a-z][a-z0-9-]{0,62}".into());
    }
    Ok(())
}

fn read_source_file(path: &Path) -> InstallResult<Vec<u8>> {
    check_no_symlink_components(path)?;
    let canonical_before =
        fs::canonicalize(path).map_err(|_| "resource source file is unavailable".to_string())?;
    let before = fs::symlink_metadata(path)
        .map_err(|_| "resource source file is unavailable".to_string())?;
    if before.file_type().is_symlink() || !before.is_file() {
        return Err("resource source must be a regular file".into());
    }
    if before.len() > MAX_SOURCE_FILE {
        return Err("resource source file exceeds the supported size".into());
    }
    let file = File::open(path).map_err(|_| "resource source file is unavailable".to_string())?;
    let opened = file
        .metadata()
        .map_err(|_| "could not inspect resource source".to_string())?;
    if !opened.is_file() || !same_file(&before, &opened) {
        return Err("resource source changed while being read".into());
    }
    let mut bytes = Vec::with_capacity(opened.len() as usize);
    file.take(MAX_SOURCE_FILE + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "could not read resource source".to_string())?;
    if bytes.len() as u64 > MAX_SOURCE_FILE {
        return Err("resource source file exceeds the supported size".into());
    }
    let after = fs::symlink_metadata(path)
        .map_err(|_| "resource source changed while being read".to_string())?;
    let canonical_after = fs::canonicalize(path)
        .map_err(|_| "resource source changed while being read".to_string())?;
    if after.file_type().is_symlink()
        || !same_file(&before, &after)
        || canonical_before != canonical_after
    {
        return Err("resource source changed while being read".into());
    }
    Ok(bytes)
}

fn validate_skill(directory: &Path, expected_name: &str) -> InstallResult<()> {
    check_no_symlink_components(directory)?;
    let metadata = fs::symlink_metadata(directory)
        .map_err(|_| "skill directory is unavailable".to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("skill source must be a real directory".into());
    }
    reject_tree_symlinks(directory)?;
    let after_walk = fs::symlink_metadata(directory)
        .map_err(|_| "skill directory changed during validation".to_string())?;
    if after_walk.file_type().is_symlink() || !same_file(&metadata, &after_walk) {
        return Err("skill directory changed during validation".into());
    }
    let skill_file = directory.join("SKILL.md");
    let bytes = read_source_file(&skill_file)?;
    let text = std::str::from_utf8(&bytes).map_err(|_| "SKILL.md must be UTF-8".to_string())?;
    let frontmatter = parse_frontmatter(text)?;
    let name = frontmatter
        .get("name")
        .and_then(serde_yaml::Value::as_str)
        .ok_or_else(|| "SKILL.md frontmatter requires a string name".to_string())?;
    let description = frontmatter
        .get("description")
        .and_then(serde_yaml::Value::as_str)
        .ok_or_else(|| "SKILL.md frontmatter requires a string description".to_string())?;
    if name != expected_name || description.trim().is_empty() {
        return Err(
            "SKILL.md name must match the resource key and description must be non-empty".into(),
        );
    }
    Ok(())
}

fn validate_codex_native(bytes: &[u8], expected_name: &str) -> InstallResult<()> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| "native Codex agent TOML must be UTF-8".to_string())?;
    let document = text
        .parse::<DocumentMut>()
        .map_err(|_| "native Codex agent TOML is invalid".to_string())?;
    let name = document.get("name").and_then(TomlItem::as_str);
    let description = document.get("description").and_then(TomlItem::as_str);
    let instructions = document
        .get("developer_instructions")
        .and_then(TomlItem::as_str);
    if name != Some(expected_name) {
        return Err("native Codex agent name must match the resource key".into());
    }
    if description.is_none_or(str::is_empty) || instructions.is_none_or(str::is_empty) {
        return Err(
            "native Codex agent requires name, description, and developer_instructions".into(),
        );
    }
    Ok(())
}

fn render_codex_agent(
    name: &str,
    description: Option<&str>,
    bytes: &[u8],
) -> InstallResult<Vec<u8>> {
    let description = description
        .filter(|description| !description.trim().is_empty())
        .ok_or_else(|| "portable Codex agent requires a description".to_string())?;
    let instructions = std::str::from_utf8(bytes)
        .map_err(|_| "portable agent source must be UTF-8".to_string())?;
    let mut document = DocumentMut::new();
    document["name"] = toml_edit::value(name);
    document["description"] = toml_edit::value(description);
    document["developer_instructions"] = toml_edit::value(instructions);
    Ok(document.to_string().into_bytes())
}

fn validate_markdown_agent(bytes: &[u8], expected_name: &str) -> InstallResult<()> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| "native agent Markdown must be UTF-8".to_string())?;
    let frontmatter = parse_frontmatter(text)?;
    let name = frontmatter
        .get("name")
        .and_then(serde_yaml::Value::as_str)
        .ok_or_else(|| "native agent frontmatter requires a string name".to_string())?;
    let description = frontmatter
        .get("description")
        .and_then(serde_yaml::Value::as_str)
        .ok_or_else(|| "native agent frontmatter requires a string description".to_string())?;
    if name != expected_name || description.trim().is_empty() {
        return Err(
            "native agent name must match the resource key and description must be non-empty"
                .into(),
        );
    }
    Ok(())
}

fn render_markdown_agent(
    harness: HarnessId,
    name: &str,
    description: Option<&str>,
    body: &[u8],
) -> InstallResult<Vec<u8>> {
    let description = description
        .filter(|description| !description.trim().is_empty())
        .ok_or_else(|| "portable agent requires a description".to_string())?;
    let body =
        std::str::from_utf8(body).map_err(|_| "portable agent source must be UTF-8".to_string())?;
    let encoded_name = serde_json::to_string(name)
        .map_err(|_| "could not render agent frontmatter".to_string())?;
    let encoded_description = serde_json::to_string(description)
        .map_err(|_| "could not render agent frontmatter".to_string())?;
    let mut rendered = format!("---\nname: {encoded_name}\ndescription: {encoded_description}\n");
    if harness == HarnessId::Opencode {
        rendered.push_str("mode: subagent\n");
    }
    rendered.push_str("---\n\n");
    rendered.push_str(body);
    if !body.ends_with('\n') {
        rendered.push('\n');
    }
    Ok(rendered.into_bytes())
}

fn parse_frontmatter(text: &str) -> InstallResult<BTreeMap<String, serde_yaml::Value>> {
    let normalized = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = normalized.split_inclusive('\n');
    let Some(start) = lines.next() else {
        return Err("Markdown resource requires YAML frontmatter".into());
    };
    if start.trim_end_matches(['\r', '\n']) != "---" {
        return Err("Markdown resource requires YAML frontmatter".into());
    }
    let mut yaml = String::new();
    let mut closed = false;
    for line in lines {
        if line.trim_end_matches(['\r', '\n']) == "---" {
            closed = true;
            break;
        }
        yaml.push_str(line);
    }
    if !closed {
        return Err("Markdown resource has unterminated YAML frontmatter".into());
    }
    let value: serde_yaml::Value = serde_yaml::from_str(&yaml)
        .map_err(|_| "Markdown resource YAML frontmatter is invalid".to_string())?;
    Ok(value
        .as_mapping()
        .ok_or_else(|| "Markdown resource frontmatter must be a YAML mapping".to_string())?
        .iter()
        .filter_map(|(key, value)| Some((key.as_str()?.to_owned(), value.clone())))
        .collect::<BTreeMap<_, _>>())
}

fn reject_tree_symlinks(root: &Path) -> InstallResult<()> {
    let root_metadata = fs::symlink_metadata(root)
        .map_err(|_| "could not inspect resource directory".to_string())?;
    if root_metadata.file_type().is_symlink()
        || !root_metadata.is_dir()
        || has_special_mode_bits(&root_metadata)
    {
        return Err("resource directory is not a safe regular directory".into());
    }
    fn visit(path: &Path) -> InstallResult<()> {
        let mut entries = fs::read_dir(path)
            .map_err(|_| "could not inspect resource directory".to_string())?
            .map(|entry| {
                entry
                    .map(|entry| entry.path())
                    .map_err(|_| "could not inspect resource directory".to_string())
            })
            .collect::<InstallResult<Vec<_>>>()?;
        entries.sort();
        for entry in entries {
            let metadata = fs::symlink_metadata(&entry)
                .map_err(|_| "resource directory changed".to_string())?;
            if metadata.file_type().is_symlink() {
                return Err("resource directories may not contain symbolic links".into());
            }
            if has_special_mode_bits(&metadata) {
                return Err("resource directories may not contain special permission bits".into());
            }
            if metadata.is_dir() {
                visit(&entry)?;
            } else if !metadata.is_file() {
                return Err("resource directories may not contain special files".into());
            }
        }
        Ok(())
    }
    visit(root)
}

fn fingerprint_payload(payload: &BindingPayload) -> InstallResult<String> {
    match payload {
        BindingPayload::File(bytes) => Ok(sha256_bytes(bytes)),
        BindingPayload::Directory(path) => hash_directory_checked(path),
        BindingPayload::Mcp(value) => serde_json::to_vec(value)
            .map(|bytes| sha256_bytes(&bytes))
            .map_err(|_| "could not fingerprint MCP configuration".to_string()),
        BindingPayload::RulesBlock(text) => Ok(sha256_bytes(text.as_bytes())),
    }
}

fn hash_directory_checked(path: &Path) -> InstallResult<String> {
    check_no_symlink_components(path)?;
    let before =
        fs::symlink_metadata(path).map_err(|_| "resource directory is unavailable".to_string())?;
    if before.file_type().is_symlink() || !before.is_dir() || has_special_mode_bits(&before) {
        return Err("resource directory is not a safe regular directory".into());
    }
    reject_tree_symlinks(path)?;
    let digest = hash_tree(path)?;
    let after = fs::symlink_metadata(path)
        .map_err(|_| "resource directory changed during inspection".to_string())?;
    if after.file_type().is_symlink() || !same_file(&before, &after) {
        return Err("resource directory changed during inspection".into());
    }
    check_no_symlink_components(path)?;
    Ok(digest)
}

pub fn inspect(binding: &PreparedBinding) -> InstallResult<Observation> {
    inspect_parts(
        binding.harness,
        binding.kind,
        &binding.target,
        binding.selector.as_deref(),
    )
}

pub fn inspect_managed(binding: &ManagedBinding) -> InstallResult<Observation> {
    inspect_parts(
        binding.harness,
        binding.kind,
        &binding.target,
        binding.selector.as_deref(),
    )
}

fn inspect_parts(
    harness: HarnessId,
    kind: ResourceKind,
    target: &Path,
    selector: Option<&str>,
) -> InstallResult<Observation> {
    check_no_symlink_components(target)?;
    let metadata = match fs::symlink_metadata(target) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Observation::Missing)
        }
        Err(_) => return Err("could not inspect adapter destination".into()),
    };
    if metadata.file_type().is_symlink() {
        return Err("adapter destination is a symbolic link".into());
    }
    if kind == ResourceKind::Mcp {
        return inspect_mcp(
            harness,
            target,
            selector.ok_or_else(|| "MCP selector is missing".to_string())?,
        );
    }
    if kind == ResourceKind::Rule && selector == Some("rules-block") {
        if !metadata.is_file() {
            return Err("managed rules destination is not a regular file".into());
        }
        let text = read_utf8_file(target, &metadata)?;
        return match extract_rules_block(&text)? {
            Some(block) => Ok(Observation::Present {
                fingerprint: sha256_bytes(block.as_bytes()),
            }),
            None => Ok(Observation::Missing),
        };
    }
    if metadata.is_file() {
        return Ok(Observation::Present {
            fingerprint: hash_regular_file(target, &metadata)?,
        });
    }
    if metadata.is_dir() {
        return Ok(Observation::Present {
            fingerprint: hash_directory_checked(target)?,
        });
    }
    Err("adapter destination is not a regular file or directory".into())
}

pub fn apply(
    paths: &Paths,
    binding: &PreparedBinding,
    choice: WriteChoice,
) -> InstallResult<ManagedBinding> {
    if choice == WriteChoice::Skip {
        return Err("skipped adapter binding must not be applied".into());
    }
    check_no_symlink_components(&binding.target)?;
    if binding.kind == ResourceKind::Mcp {
        apply_mcp(paths, binding, choice)?;
    } else if binding.kind == ResourceKind::Rule
        && binding.selector.as_deref() == Some("rules-block")
    {
        apply_rules_block(paths, binding, choice)?;
    } else {
        apply_file_or_directory(paths, binding, choice)?;
    }
    let observation = inspect(binding)?;
    let fingerprint = match observation {
        Observation::Present { fingerprint } => fingerprint,
        Observation::Missing => {
            return Err("adapter write completed without an observable binding".into())
        }
    };
    Ok(ManagedBinding {
        id: binding.id.clone(),
        harness: binding.harness,
        kind: binding.kind,
        names: binding.names.clone(),
        target: binding.target.clone(),
        selector: binding.selector.clone(),
        fingerprint,
        desired_fingerprint: binding.desired_fingerprint.clone(),
        artifact_keys: binding.artifact_keys.clone(),
    })
}

pub fn remove(paths: &Paths, binding: &ManagedBinding) -> InstallResult<()> {
    let observed = inspect_managed(binding)?;
    let observed_fingerprint = match observed {
        Observation::Missing => return Ok(()),
        Observation::Present { fingerprint } => fingerprint,
    };
    check_no_symlink_components(&binding.target)?;
    if binding.kind == ResourceKind::Mcp {
        remove_mcp(paths, binding)?;
    } else if binding.kind == ResourceKind::Rule
        && binding.selector.as_deref() == Some("rules-block")
    {
        remove_rules_block(paths, binding)?;
    } else {
        let metadata = fs::symlink_metadata(&binding.target)
            .map_err(|_| "owned adapter resource disappeared".to_string())?;
        if metadata.is_dir() {
            reject_tree_symlinks(&binding.target)?;
            let backup = backup_target_snapshot(paths, &binding.target)?
                .ok_or_else(|| "owned adapter directory disappeared before backup".to_string())?;
            remove_directory_quarantined(
                &binding.target,
                &observed_fingerprint,
                &backup.join("tree"),
            )?;
        } else if metadata.is_file() {
            backup_target(paths, &binding.target)?;
            fs::remove_file(&binding.target)
                .map_err(|_| "could not remove owned adapter file".to_string())?;
        } else {
            return Err("owned adapter resource is not a regular file or directory".into());
        }
    }
    Ok(())
}

fn apply_file_or_directory(
    paths: &Paths,
    binding: &PreparedBinding,
    choice: WriteChoice,
) -> InstallResult<()> {
    match &binding.payload {
        BindingPayload::File(bytes) => {
            let current = inspect_parts(
                binding.harness,
                binding.kind,
                &binding.target,
                binding.selector.as_deref(),
            )?;
            if let (Observation::Present { fingerprint }, Ok(desired)) =
                (current, fingerprint_payload(&binding.payload))
            {
                if fingerprint == desired {
                    return Ok(());
                }
            }
            if choice == WriteChoice::Merge {
                return Err(
                    "file resources cannot be merged safely; choose replace or skip".into(),
                );
            }
            let existing_mode = existing_mode(&binding.target).unwrap_or(0o600);
            ensure_parent_directory(&binding.target)?;
            match fs::symlink_metadata(&binding.target) {
                Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
                    replace_directory_with_file(paths, &binding.target, bytes, existing_mode)
                }
                Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
                    backup_target(paths, &binding.target)?;
                    atomic_write(&binding.target, bytes, existing_mode)
                }
                Ok(_) => Err("adapter file target is not a regular file or directory".into()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    atomic_write(&binding.target, bytes, existing_mode)
                }
                Err(_) => Err("could not inspect adapter file target".into()),
            }
        }
        BindingPayload::Directory(source) => {
            if choice != WriteChoice::Merge && choice != WriteChoice::Replace {
                return Err("directory resource choice is invalid".into());
            }
            reject_tree_symlinks(source)?;
            let current = inspect_parts(
                binding.harness,
                binding.kind,
                &binding.target,
                binding.selector.as_deref(),
            )?;
            let desired = hash_directory_checked(source)?;
            if let Observation::Present { fingerprint } = current {
                if fingerprint == desired {
                    return Ok(());
                }
            }
            let target_exists = match fs::symlink_metadata(&binding.target) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err("adapter directory target is a symbolic link".into())
                }
                Ok(metadata) if metadata.is_dir() => {
                    reject_tree_symlinks(&binding.target)?;
                    true
                }
                Ok(metadata) if metadata.is_file() => {
                    if choice == WriteChoice::Merge {
                        return Err(
                            "file resources cannot be merged into a skill directory; choose replace or skip"
                                .into(),
                        );
                    }
                    true
                }
                Ok(_) => {
                    return Err(
                        "adapter directory target is not a regular file or directory".into(),
                    )
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
                Err(_) => return Err("could not inspect adapter directory target".into()),
            };
            ensure_parent_directory(&binding.target)?;
            let stage = unique_sibling(&binding.target, "stage")?;
            let staging = if choice == WriteChoice::Merge && target_exists {
                copy_directory_no_symlinks(&binding.target, &stage)
                    .and_then(|_| overlay_directory_no_symlinks(source, &stage))
            } else {
                copy_directory_no_symlinks(source, &stage)
            };
            if let Err(error) = staging {
                let _ = remove_path_tree(&stage);
                return Err(error);
            }
            if target_exists {
                if let Err(error) = backup_target(paths, &binding.target) {
                    let _ = remove_path_tree(&stage);
                    return Err(error);
                }
            }
            replace_directory_staged(&binding.target, &stage)
        }
        _ => Err("adapter payload does not match file or directory resource".into()),
    }
}

fn apply_rules_block(
    paths: &Paths,
    binding: &PreparedBinding,
    choice: WriteChoice,
) -> InstallResult<()> {
    let BindingPayload::RulesBlock(desired) = &binding.payload else {
        return Err("rules block has an invalid adapter payload".into());
    };
    let existing = read_optional_utf8(&binding.target)?;
    if let Some(text) = existing.as_deref() {
        if let Some(current) = extract_rules_block(text)? {
            if current.as_str() == desired {
                return Ok(());
            }
        }
    }
    if choice == WriteChoice::Skip {
        return Err("skipped adapter binding must not be applied".into());
    }
    let next = match existing {
        None => format!("{desired}\n"),
        Some(text) => upsert_rules_block(&text, desired)?,
    };
    backup_target(paths, &binding.target)?;
    ensure_parent_directory(&binding.target)?;
    let mode = existing_mode(&binding.target).unwrap_or(0o600);
    atomic_write(&binding.target, next.as_bytes(), mode)
}

fn remove_rules_block(paths: &Paths, binding: &ManagedBinding) -> InstallResult<()> {
    let text = read_optional_utf8(&binding.target)?
        .ok_or_else(|| "owned rules block disappeared".to_string())?;
    let next = remove_rules_block_text(&text)?;
    if next == text {
        return Ok(());
    }
    backup_target(paths, &binding.target)?;
    let mode = existing_mode(&binding.target).unwrap_or(0o600);
    atomic_write(&binding.target, next.as_bytes(), mode)
}

fn extract_rules_block(text: &str) -> InstallResult<Option<String>> {
    let starts = text
        .match_indices(RULES_START)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let ends = text
        .match_indices(RULES_END)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if starts.is_empty() && ends.is_empty() {
        return Ok(None);
    }
    if starts.len() != 1 || ends.len() != 1 || starts[0] >= ends[0] {
        return Err("managed rules markers are missing, duplicated, or out of order".into());
    }
    let end = ends[0] + RULES_END.len();
    Ok(Some(text[starts[0]..end].to_owned()))
}

fn upsert_rules_block(text: &str, desired: &str) -> InstallResult<String> {
    match extract_rules_block(text)? {
        Some(current) => {
            let start = text
                .find(RULES_START)
                .ok_or_else(|| "managed rules markers are ambiguous".to_string())?;
            let end = text
                .find(RULES_END)
                .ok_or_else(|| "managed rules markers are ambiguous".to_string())?
                + RULES_END.len();
            let mut next = String::with_capacity(text.len() + desired.len());
            next.push_str(&text[..start]);
            next.push_str(desired);
            next.push_str(&text[end..]);
            if current == desired {
                Ok(text.to_owned())
            } else {
                Ok(next)
            }
        }
        None => {
            let mut next = text.to_owned();
            if !next.is_empty() && !next.ends_with('\n') {
                next.push('\n');
            }
            if !next.is_empty() {
                next.push('\n');
            }
            next.push_str(desired);
            next.push('\n');
            Ok(next)
        }
    }
}

fn remove_rules_block_text(text: &str) -> InstallResult<String> {
    let Some(_block) = extract_rules_block(text)? else {
        return Ok(text.to_owned());
    };
    let start = text
        .find(RULES_START)
        .ok_or_else(|| "managed rules markers are ambiguous".to_string())?;
    let end = text
        .find(RULES_END)
        .ok_or_else(|| "managed rules markers are ambiguous".to_string())?
        + RULES_END.len();
    let mut next = String::with_capacity(text.len());
    next.push_str(&text[..start]);
    next.push_str(&text[end..]);
    Ok(next)
}

fn apply_mcp(paths: &Paths, binding: &PreparedBinding, choice: WriteChoice) -> InstallResult<()> {
    let BindingPayload::Mcp(desired) = &binding.payload else {
        return Err("MCP binding has an invalid adapter payload".into());
    };
    let name = binding
        .selector
        .as_deref()
        .ok_or_else(|| "MCP selector is missing".to_string())?;
    if binding.harness == HarnessId::Codex {
        return apply_codex_mcp(paths, &binding.target, name, desired, choice);
    }
    let existing_bytes = read_optional_bytes(&binding.target)?;
    let current = existing_bytes
        .as_deref()
        .map(read_jsonc_value)
        .transpose()?;
    let current_entry = current
        .as_ref()
        .and_then(|root| mcp_entry(root, binding.harness, name).ok().flatten());
    if current_entry
        .is_some_and(|entry| mcp_entry_satisfied(entry, desired, binding.harness, choice))
    {
        return Ok(());
    }
    if choice == WriteChoice::Skip {
        return Err("skipped adapter binding must not be applied".into());
    }
    let text = existing_bytes
        .as_deref()
        .map(|bytes| {
            std::str::from_utf8(bytes)
                .map(str::to_owned)
                .map_err(|_| "adapter JSON config must be UTF-8".to_string())
        })
        .transpose()?
        .unwrap_or_default();
    let next = edit_jsonc_mcp(&text, binding.harness, name, desired, choice)?;
    let mode = existing_mode(&binding.target).unwrap_or(0o600);
    backup_target(paths, &binding.target)?;
    ensure_parent_directory(&binding.target)?;
    atomic_write(&binding.target, next.as_bytes(), mode)
}

fn remove_mcp(paths: &Paths, binding: &ManagedBinding) -> InstallResult<()> {
    let name = binding
        .selector
        .as_deref()
        .ok_or_else(|| "MCP selector is missing".to_string())?;
    if binding.harness == HarnessId::Codex {
        let text = read_optional_utf8(&binding.target)?;
        let Some(text) = text else {
            return Ok(());
        };
        let mut document = parse_toml(&text)?;
        let mut removed = false;
        if let Some(servers) = document
            .get_mut("mcp_servers")
            .and_then(TomlItem::as_table_mut)
        {
            removed = servers.remove(name).is_some();
        }
        if !removed {
            return Ok(());
        }
        backup_target(paths, &binding.target)?;
        let mode = existing_mode(&binding.target).unwrap_or(0o600);
        return atomic_write(&binding.target, document.to_string().as_bytes(), mode);
    }
    let bytes = read_optional_bytes(&binding.target)?;
    let Some(bytes) = bytes else {
        return Ok(());
    };
    let text =
        std::str::from_utf8(&bytes).map_err(|_| "adapter JSON config must be UTF-8".to_string())?;
    let root = parse_jsonc(text)?;
    let Some(root_object) = root.object_value() else {
        return Err("adapter JSON config must be an object".into());
    };
    let map_name = if binding.harness == HarnessId::Opencode {
        "mcp"
    } else {
        "mcpServers"
    };
    let Some(map) = root_object.object_value(map_name) else {
        return Ok(());
    };
    let Some(property) = map.get(name) else {
        return Ok(());
    };
    backup_target(paths, &binding.target)?;
    property.remove();
    let mode = existing_mode(&binding.target).unwrap_or(0o600);
    atomic_write(&binding.target, root.to_string().as_bytes(), mode)
}

fn inspect_mcp(harness: HarnessId, target: &Path, name: &str) -> InstallResult<Observation> {
    if harness == HarnessId::Codex {
        let Some(text) = read_optional_utf8(target)? else {
            return Ok(Observation::Missing);
        };
        let document = parse_toml(&text)?;
        let Some(entry) = document
            .get("mcp_servers")
            .and_then(TomlItem::as_table)
            .and_then(|table| table.get(name))
        else {
            return Ok(Observation::Missing);
        };
        let value = toml_item_to_json(entry)?;
        return Ok(Observation::Present {
            fingerprint: fingerprint_json(&value)?,
        });
    }
    let Some(bytes) = read_optional_bytes(target)? else {
        return Ok(Observation::Missing);
    };
    let root = read_jsonc_value(&bytes)?;
    match mcp_entry(&root, harness, name)? {
        Some(entry) => Ok(Observation::Present {
            fingerprint: fingerprint_json(entry)?,
        }),
        None => Ok(Observation::Missing),
    }
}

fn apply_codex_mcp(
    paths: &Paths,
    target: &Path,
    name: &str,
    desired: &JsonValue,
    choice: WriteChoice,
) -> InstallResult<()> {
    let text = read_optional_utf8(target)?.unwrap_or_default();
    let mut document = parse_toml(&text)?;
    let table = document.as_table_mut();
    if table
        .get("mcp_servers")
        .is_some_and(|item| !item.is_table())
    {
        return Err("Codex mcp_servers config must be a TOML table".into());
    }
    if !table.contains_key("mcp_servers") {
        table.insert("mcp_servers", TomlItem::Table(TomlTable::new()));
    }
    let servers = document
        .get_mut("mcp_servers")
        .and_then(TomlItem::as_table_mut)
        .ok_or_else(|| "Codex mcp_servers config must be a TOML table".to_string())?;
    let current = servers.get(name).cloned();
    let current_json = current.as_ref().map(toml_item_to_json).transpose()?;
    if current_json
        .as_ref()
        .is_some_and(|entry| mcp_entry_satisfied(entry, desired, HarnessId::Codex, choice))
    {
        return Ok(());
    }
    if choice == WriteChoice::Skip {
        return Err("skipped adapter binding must not be applied".into());
    }
    let desired_item = toml_item_from_json(desired)?;
    let merged = match (current, choice) {
        (Some(TomlItem::Table(mut existing)), WriteChoice::Merge) => {
            let TomlItem::Table(desired_table) = desired_item else {
                unreachable!()
            };
            merge_toml_table(&mut existing, desired_table);
            TomlItem::Table(existing)
        }
        (Some(TomlItem::Table(existing)), WriteChoice::Replace) => {
            let TomlItem::Table(mut desired_table) = desired_item else {
                unreachable!()
            };
            for key in AUTH_FIELDS {
                if let Some(auth) = existing.get(key) {
                    desired_table.insert(key, auth.clone());
                }
            }
            TomlItem::Table(desired_table)
        }
        (Some(_), WriteChoice::Merge) => {
            return Err("Codex MCP entry must be a table to merge".into())
        }
        (_, _) => desired_item,
    };
    servers.insert(name, merged);
    let mode = existing_mode(target).unwrap_or(0o600);
    backup_target(paths, target)?;
    ensure_parent_directory(target)?;
    atomic_write(target, document.to_string().as_bytes(), mode)
}

fn parse_toml(text: &str) -> InstallResult<DocumentMut> {
    if text.trim().is_empty() {
        return Ok(DocumentMut::new());
    }
    text.parse::<DocumentMut>()
        .map_err(|_| "adapter TOML config is invalid".to_string())
}

fn toml_item_from_json(value: &JsonValue) -> InstallResult<TomlItem> {
    match value {
        JsonValue::Object(object) => {
            let mut table = TomlTable::new();
            for (key, value) in object {
                table.insert(key, toml_item_from_json(value)?);
            }
            Ok(TomlItem::Table(table))
        }
        JsonValue::Array(values) => {
            let mut array = TomlArray::new();
            for value in values {
                let item = toml_item_from_json(value)?;
                let TomlItem::Value(value) = item else {
                    return Err("Codex MCP arrays must contain scalar values".into());
                };
                array.push(value);
            }
            Ok(TomlItem::Value(TomlValue::Array(array)))
        }
        JsonValue::String(value) => Ok(TomlItem::Value(TomlValue::from(value.as_str()))),
        JsonValue::Bool(value) => Ok(TomlItem::Value(TomlValue::from(*value))),
        JsonValue::Number(value) => {
            let integer = value
                .as_i64()
                .ok_or_else(|| "Codex MCP numeric option is unsupported".to_string())?;
            Ok(TomlItem::Value(TomlValue::from(integer)))
        }
        JsonValue::Null => Err("Codex MCP configuration cannot contain null".into()),
    }
}

fn merge_toml_table(existing: &mut TomlTable, desired: TomlTable) {
    for (key, value) in desired {
        if let (Some(TomlItem::Table(existing_child)), TomlItem::Table(desired_child)) =
            (existing.get_mut(&key), &value)
        {
            merge_toml_table(existing_child, desired_child.clone());
        } else {
            existing.insert(&key, value);
        }
    }
}

fn read_jsonc_value(bytes: &[u8]) -> InstallResult<JsonValue> {
    let text =
        std::str::from_utf8(bytes).map_err(|_| "adapter JSON config must be UTF-8".to_string())?;
    let root = parse_jsonc(text)?;
    let node = root
        .value()
        .ok_or_else(|| "adapter JSON config is empty".to_string())?;
    node.to_serde_value()
        .ok_or_else(|| "adapter JSON config has no JSON value".into())
}

fn parse_jsonc(text: &str) -> InstallResult<CstRootNode> {
    let root = CstRootNode::parse(text, &ParseOptions::default())
        .map_err(|_| "adapter JSON config is invalid".to_string())?;
    let node = root
        .value()
        .ok_or_else(|| "adapter JSON config is empty".to_string())?;
    if node.to_serde_value().is_none() {
        return Err("adapter JSON config has no JSON value".into());
    }
    validate_unique_jsonc_keys(&node)?;
    Ok(root)
}

fn validate_unique_jsonc_keys(node: &CstNode) -> InstallResult<()> {
    if let Some(object) = node.as_object() {
        let mut keys = BTreeSet::new();
        for property in object.properties() {
            let key = property
                .decoded_name()
                .ok_or_else(|| "adapter JSON config contains an invalid object key".to_string())?;
            if !keys.insert(key) {
                return Err("adapter JSON config contains duplicate object keys".into());
            }
            if let Some(value) = property.value() {
                validate_unique_jsonc_keys(&value)?;
            }
        }
    } else if let Some(array) = node.as_array() {
        for value in array.elements() {
            validate_unique_jsonc_keys(&value)?;
        }
    }
    Ok(())
}

fn mcp_entry<'a>(
    root: &'a JsonValue,
    harness: HarnessId,
    name: &str,
) -> InstallResult<Option<&'a JsonValue>> {
    let root = root
        .as_object()
        .ok_or_else(|| "adapter JSON config must be an object".to_string())?;
    let map_name = if harness == HarnessId::Opencode {
        "mcp"
    } else {
        "mcpServers"
    };
    let Some(map) = root.get(map_name) else {
        return Ok(None);
    };
    let map = map
        .as_object()
        .ok_or_else(|| format!("adapter {map_name} config must be an object"))?;
    Ok(map.get(name))
}

fn edit_jsonc_mcp(
    text: &str,
    harness: HarnessId,
    name: &str,
    desired: &JsonValue,
    choice: WriteChoice,
) -> InstallResult<String> {
    let root = if text.trim().is_empty() {
        CstRootNode::parse("{}", &ParseOptions::default())
            .map_err(|_| "could not create JSON config".to_string())?
    } else {
        parse_jsonc(text)?
    };
    let object = root
        .object_value_or_create()
        .ok_or_else(|| "adapter JSON config root must be an object".to_string())?;
    let map_name = if harness == HarnessId::Opencode {
        "mcp"
    } else {
        "mcpServers"
    };
    if object.get(map_name).is_some() && object.object_value(map_name).is_none() {
        return Err(format!("adapter {map_name} config must be an object"));
    }
    let map = object
        .object_value_or_create(map_name)
        .ok_or_else(|| format!("adapter {map_name} config must be an object"))?;
    if let Some(existing) = map.get(name) {
        if choice == WriteChoice::Merge {
            let current = existing
                .to_serde_value()
                .ok_or_else(|| "adapter MCP entry is invalid".to_string())?;
            if !current.is_object() {
                return Err("adapter MCP entry must be an object to merge".into());
            }
            let desired_object = desired
                .as_object()
                .ok_or_else(|| "desired MCP config must be an object".to_string())?;
            let object = existing
                .object_value()
                .ok_or_else(|| "adapter MCP entry must be an object to merge".to_string())?;
            for (key, value) in desired_object {
                if let Some(property) = object.get(key) {
                    merge_cst_property(&property, value)?;
                } else {
                    object.append(key, cst_value_from_json(value)?);
                }
            }
        } else {
            let current = existing
                .to_serde_value()
                .ok_or_else(|| "adapter MCP entry is invalid".to_string())?;
            let replacement = preserve_auth_fields(&current, desired, harness)?;
            existing.set_value(cst_value_from_json(&replacement)?);
        }
    } else {
        map.append(name, cst_value_from_json(desired)?);
    }
    Ok(root.to_string())
}

fn preserve_auth_fields(
    current: &JsonValue,
    desired: &JsonValue,
    _harness: HarnessId,
) -> InstallResult<JsonValue> {
    let mut output = desired
        .as_object()
        .cloned()
        .ok_or_else(|| "desired MCP config must be an object".to_string())?;
    let Some(current) = current.as_object() else {
        return Ok(JsonValue::Object(output));
    };
    for field in AUTH_FIELDS {
        if let Some(value) = current.get(*field) {
            output.insert((*field).to_owned(), value.clone());
        }
    }
    Ok(JsonValue::Object(output))
}

fn json_contains(current: &JsonValue, desired: &JsonValue) -> bool {
    match (current, desired) {
        (JsonValue::Object(current), JsonValue::Object(desired)) => {
            desired.iter().all(|(key, value)| {
                current
                    .get(key)
                    .is_some_and(|current| json_contains(current, value))
            })
        }
        _ => current == desired,
    }
}

fn mcp_entry_satisfied(
    current: &JsonValue,
    desired: &JsonValue,
    harness: HarnessId,
    choice: WriteChoice,
) -> bool {
    match choice {
        WriteChoice::Merge => json_contains(current, desired),
        WriteChoice::Replace => preserve_auth_fields(current, desired, harness)
            .is_ok_and(|expected| current == &expected),
        WriteChoice::Skip => false,
    }
}

fn cst_value_from_json(value: &JsonValue) -> InstallResult<CstInputValue> {
    Ok(match value {
        JsonValue::Null => CstInputValue::Null,
        JsonValue::Bool(value) => CstInputValue::Bool(*value),
        JsonValue::Number(value) => CstInputValue::Number(value.to_string()),
        JsonValue::String(value) => CstInputValue::String(value.clone()),
        JsonValue::Array(values) => CstInputValue::Array(
            values
                .iter()
                .map(cst_value_from_json)
                .collect::<InstallResult<Vec<_>>>()?,
        ),
        JsonValue::Object(values) => CstInputValue::Object(
            values
                .iter()
                .map(|(key, value)| Ok((key.clone(), cst_value_from_json(value)?)))
                .collect::<InstallResult<Vec<_>>>()?,
        ),
    })
}

fn merge_cst_property(
    property: &jsonc_parser::cst::CstObjectProp,
    desired: &JsonValue,
) -> InstallResult<()> {
    if let JsonValue::Object(desired_object) = desired {
        if let Some(object) = property.object_value() {
            for (key, value) in desired_object {
                if let Some(existing) = object.get(key) {
                    merge_cst_property(&existing, value)?;
                } else {
                    object.append(key, cst_value_from_json(value)?);
                }
            }
            return Ok(());
        }
    }
    property.set_value(cst_value_from_json(desired)?);
    Ok(())
}

fn fingerprint_json(value: &JsonValue) -> InstallResult<String> {
    serde_json::to_vec(value)
        .map(|bytes| sha256_bytes(&bytes))
        .map_err(|_| "could not fingerprint adapter configuration".to_string())
}

fn toml_item_to_json(item: &TomlItem) -> InstallResult<JsonValue> {
    match item {
        TomlItem::None => Err("Codex MCP entry has no value".into()),
        TomlItem::Table(table) => {
            let mut object = serde_json::Map::new();
            for (key, value) in table.iter() {
                object.insert(key.to_owned(), toml_item_to_json(value)?);
            }
            Ok(JsonValue::Object(object))
        }
        TomlItem::ArrayOfTables(tables) => Ok(JsonValue::Array(
            tables
                .iter()
                .map(|table| toml_item_to_json(&TomlItem::Table(table.clone())))
                .collect::<InstallResult<Vec<_>>>()?,
        )),
        TomlItem::Value(value) => toml_value_to_json(value),
    }
}

fn toml_value_to_json(value: &TomlValue) -> InstallResult<JsonValue> {
    if let Some(value) = value.as_str() {
        return Ok(JsonValue::String(value.to_owned()));
    }
    if let Some(value) = value.as_bool() {
        return Ok(JsonValue::Bool(value));
    }
    if let Some(value) = value.as_integer() {
        return Ok(JsonValue::Number(value.into()));
    }
    if let Some(value) = value.as_float().and_then(serde_json::Number::from_f64) {
        return Ok(JsonValue::Number(value));
    }
    if let Some(value) = value.as_datetime() {
        return Ok(JsonValue::String(value.to_string()));
    }
    if let Some(array) = value.as_array() {
        return Ok(JsonValue::Array(
            array
                .iter()
                .map(toml_value_to_json)
                .collect::<InstallResult<Vec<_>>>()?,
        ));
    }
    if let Some(table) = value.as_inline_table() {
        let mut object = serde_json::Map::new();
        for (key, value) in table.iter() {
            object.insert(key.to_owned(), toml_value_to_json(value)?);
        }
        return Ok(JsonValue::Object(object));
    }
    Err("Codex MCP config contains an unsupported TOML value".into())
}

fn read_optional_bytes(path: &Path) -> InstallResult<Option<Vec<u8>>> {
    check_no_symlink_components(path)?;
    let before = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("could not inspect adapter config file".into()),
    };
    if before.file_type().is_symlink() || !before.is_file() {
        return Err("adapter config must be a regular file".into());
    }
    if before.len() > 32 * 1024 * 1024 {
        return Err("adapter config exceeds the supported size".into());
    }
    let file = File::open(path).map_err(|_| "could not read adapter config file".to_string())?;
    let opened = file
        .metadata()
        .map_err(|_| "could not inspect adapter config file".to_string())?;
    if !opened.is_file() || !same_file(&before, &opened) {
        return Err("adapter config changed while being read".into());
    }
    let mut bytes = Vec::with_capacity(opened.len() as usize);
    file.take(32 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "could not read adapter config file".to_string())?;
    if bytes.len() > 32 * 1024 * 1024 {
        return Err("adapter config exceeds the supported size".into());
    }
    let after = fs::symlink_metadata(path)
        .map_err(|_| "adapter config changed while being read".to_string())?;
    if after.file_type().is_symlink() || !same_file(&before, &after) {
        return Err("adapter config changed while being read".into());
    }
    Ok(Some(bytes))
}

fn read_optional_utf8(path: &Path) -> InstallResult<Option<String>> {
    read_optional_bytes(path)?
        .map(|bytes| {
            String::from_utf8(bytes).map_err(|_| "adapter config must be UTF-8".to_string())
        })
        .transpose()
}

fn read_utf8_file(path: &Path, metadata: &fs::Metadata) -> InstallResult<String> {
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("adapter destination must be a regular file".into());
    }
    let bytes =
        read_optional_bytes(path)?.ok_or_else(|| "adapter destination disappeared".to_string())?;
    String::from_utf8(bytes).map_err(|_| "adapter destination must be UTF-8".into())
}

fn hash_regular_file(path: &Path, metadata: &fs::Metadata) -> InstallResult<String> {
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("adapter destination must be a regular file".into());
    }
    let bytes =
        read_optional_bytes(path)?.ok_or_else(|| "adapter destination disappeared".to_string())?;
    Ok(sha256_bytes(&bytes))
}

fn check_no_symlink_components(path: &Path) -> InstallResult<()> {
    if !path.is_absolute() {
        return Err("adapter destination must be an absolute path".into());
    }
    let mut current = PathBuf::new();
    let components = path.components().collect::<Vec<_>>();
    for (index, component) in components.iter().enumerate() {
        match component {
            Component::Prefix(_) | Component::RootDir | Component::CurDir => {
                current.push(component.as_os_str())
            }
            Component::ParentDir => {
                return Err("adapter destination contains path traversal".into())
            }
            Component::Normal(part) => current.push(part),
        }
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err("adapter destination contains a symbolic link".into());
            }
            Ok(metadata) if index + 1 < components.len() && !metadata.is_dir() => {
                return Err("adapter destination parent is not a directory".into());
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(_) => return Err("could not inspect adapter destination path".into()),
        }
    }
    Ok(())
}

fn ensure_parent_directory(target: &Path) -> InstallResult<()> {
    let parent = target
        .parent()
        .ok_or_else(|| "adapter destination has no parent directory".to_string())?;
    check_no_symlink_components(parent)?;
    fs::create_dir_all(parent)
        .map_err(|_| "could not create adapter destination directory".to_string())?;
    check_no_symlink_components(parent)
}

fn existing_mode(path: &Path) -> Option<u32> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        Some(metadata.permissions().mode() & 0o777)
    }
    #[cfg(not(unix))]
    {
        Some(0o600)
    }
}

fn backup_target(paths: &Paths, target: &Path) -> InstallResult<()> {
    backup_target_snapshot(paths, target).map(|_| ())
}

fn backup_target_snapshot(paths: &Paths, target: &Path) -> InstallResult<Option<PathBuf>> {
    check_no_symlink_components(target)?;
    let metadata = match fs::symlink_metadata(target) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("could not inspect adapter destination for backup".into()),
    };
    if metadata.file_type().is_symlink() {
        return Err("adapter destination is a symbolic link".into());
    }
    let backup_root = paths.state.join("backups");
    ensure_private_backup_root(&paths.state, &backup_root)?;
    let backup_dir = unique_child_directory(&backup_root, "binding-backup")?;
    ensure_private_backup_root(&paths.state, &backup_root)?;
    if metadata.is_file() {
        let bytes = read_optional_bytes(target)?
            .ok_or_else(|| "adapter destination disappeared before backup".to_string())?;
        atomic_write(&backup_dir.join("content"), &bytes, 0o600)?;
    } else if metadata.is_dir() {
        reject_tree_symlinks(target)?;
        let snapshot = backup_dir.join("tree");
        copy_tree_checked(target, &snapshot)?;
    } else {
        return Err("adapter destination is not a regular file or directory".into());
    }
    Ok(Some(backup_dir))
}

fn ensure_private_backup_root(state_root: &Path, backup_root: &Path) -> InstallResult<()> {
    check_no_symlink_components(state_root)?;
    match fs::symlink_metadata(state_root) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err("installer state path is not a real directory".into());
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err("could not inspect installer state path".into()),
    }
    private_dir(state_root)?;
    check_no_symlink_components(state_root)?;

    check_no_symlink_components(backup_root)?;
    match fs::symlink_metadata(backup_root) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err("installer backup root is not a real directory".into());
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err("could not inspect installer backup root".into()),
    }
    private_dir(backup_root)?;
    check_no_symlink_components(state_root)?;
    check_no_symlink_components(backup_root)?;

    let state_real = fs::canonicalize(state_root)
        .map_err(|_| "could not resolve installer state path".to_string())?;
    let backup_real = fs::canonicalize(backup_root)
        .map_err(|_| "could not resolve installer backup path".to_string())?;
    if !backup_real.starts_with(&state_real) {
        return Err("installer backup path escaped its private state directory".into());
    }
    Ok(())
}

fn remove_directory_quarantined(
    target: &Path,
    expected_fingerprint: &str,
    backup_snapshot: &Path,
) -> InstallResult<()> {
    remove_directory_quarantined_with(
        target,
        expected_fingerprint,
        backup_snapshot,
        remove_path_tree,
    )
}

fn remove_directory_quarantined_with<F>(
    target: &Path,
    expected_fingerprint: &str,
    backup_snapshot: &Path,
    cleanup: F,
) -> InstallResult<()>
where
    F: FnOnce(&Path) -> InstallResult<()>,
{
    let quarantine = unique_sibling(target, "remove")?;
    fs::rename(target, &quarantine).map_err(|_| {
        "could not atomically stage owned adapter directory for removal".to_string()
    })?;
    match cleanup(&quarantine) {
        Ok(()) => Ok(()),
        Err(error) => {
            let unchanged = hash_directory_checked(&quarantine)
                .is_ok_and(|fingerprint| fingerprint == expected_fingerprint);
            if unchanged && fs::rename(&quarantine, target).is_ok() {
                return Err(error);
            }
            if restore_directory_from_snapshot(backup_snapshot, target).is_ok() {
                return Err(format!(
                    "{error}; original adapter directory was restored, partial removal remains at {}",
                    quarantine.display()
                ));
            }
            Err(format!(
                "{error}; adapter directory recovery copy remains at {} and partial removal remains at {}",
                backup_snapshot.display(),
                quarantine.display(),
            ))
        }
    }
}

fn restore_directory_from_snapshot(snapshot: &Path, target: &Path) -> InstallResult<()> {
    check_no_symlink_components(target)?;
    match fs::symlink_metadata(target) {
        Ok(_) => return Err("adapter destination was recreated during removal".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err("could not inspect adapter destination after removal".into()),
    }
    let parent = target
        .parent()
        .ok_or_else(|| "adapter destination has no parent".to_string())?;
    check_no_symlink_components(parent)?;
    let stage = unique_sibling(target, "restore")?;
    copy_tree_checked(snapshot, &stage)?;
    if fs::rename(&stage, target).is_err() {
        let _ = remove_path_tree(&stage);
        return Err("could not restore adapter directory from its backup".into());
    }
    Ok(())
}

fn unique_child_directory(parent: &Path, label: &str) -> InstallResult<PathBuf> {
    for _ in 0..32 {
        let candidate = parent.join(format!(
            "{label}-{}-{}",
            now_nanos(),
            NEXT_TEMP.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        match create_private_dir_new(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(_) if fs::symlink_metadata(&candidate).is_ok() => continue,
            Err(error) => return Err(error),
        }
    }
    Err("could not reserve private adapter backup".into())
}

static NEXT_TEMP: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn unique_sibling(target: &Path, label: &str) -> InstallResult<PathBuf> {
    let parent = target
        .parent()
        .ok_or_else(|| "adapter destination has no parent".to_string())?;
    let file_name = target
        .file_name()
        .ok_or_else(|| "adapter destination has no name".to_string())?
        .to_string_lossy();
    for _ in 0..32 {
        let candidate = parent.join(format!(
            ".{file_name}.yashik-{label}-{}-{}",
            now_nanos(),
            NEXT_TEMP.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        if fs::symlink_metadata(&candidate).is_err() {
            return Ok(candidate);
        }
    }
    Err("could not reserve adapter staging path".into())
}

fn copy_directory_no_symlinks(source: &Path, destination: &Path) -> InstallResult<()> {
    check_no_symlink_components(source)?;
    let metadata = fs::symlink_metadata(source)
        .map_err(|_| "resource directory is unavailable".to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("resource source must be a real directory".into());
    }
    if has_special_mode_bits(&metadata) {
        return Err("resource directories may not contain special permission bits".into());
    }
    fs::create_dir(destination)
        .map_err(|_| "could not create adapter staging directory".to_string())?;
    set_directory_mode(destination, source_mode(source).unwrap_or(0o700))?;
    let mut entries = fs::read_dir(source)
        .map_err(|_| "could not read resource directory".to_string())?
        .map(|entry| {
            entry
                .map(|entry| entry.path())
                .map_err(|_| "could not read resource directory".to_string())
        })
        .collect::<InstallResult<Vec<_>>>()?;
    entries.sort();
    for entry in entries {
        let source_metadata = fs::symlink_metadata(&entry)
            .map_err(|_| "resource directory changed during copy".to_string())?;
        if has_special_mode_bits(&source_metadata) {
            return Err("resource directories may not contain special permission bits".into());
        }
        let destination_entry = destination.join(
            entry
                .file_name()
                .ok_or_else(|| "resource path is invalid".to_string())?,
        );
        if source_metadata.file_type().is_symlink() {
            return Err("resource directories may not contain symbolic links".into());
        } else if source_metadata.is_dir() {
            copy_directory_no_symlinks(&entry, &destination_entry)?;
            let after = fs::symlink_metadata(&entry)
                .map_err(|_| "resource directory changed during copy".to_string())?;
            if after.file_type().is_symlink() || !same_file(&source_metadata, &after) {
                return Err("resource directory changed during copy".into());
            }
        } else if source_metadata.is_file() {
            let before = source_metadata;
            let mut input =
                File::open(&entry).map_err(|_| "could not read resource file".to_string())?;
            let opened = input
                .metadata()
                .map_err(|_| "could not inspect resource file".to_string())?;
            if !opened.is_file() || !same_file(&before, &opened) {
                return Err("resource file changed during copy".into());
            }
            let mut output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&destination_entry)
                .map_err(|_| "could not create copied resource file".to_string())?;
            std::io::copy(&mut input, &mut output)
                .map_err(|_| "could not copy resource file".to_string())?;
            output
                .sync_all()
                .map_err(|_| "could not sync copied resource file".to_string())?;
            set_file_path_mode(&destination_entry, source_mode(&entry).unwrap_or(0o600))?;
            let after = fs::symlink_metadata(&entry)
                .map_err(|_| "resource file changed during copy".to_string())?;
            if after.file_type().is_symlink() || !same_file(&before, &after) {
                return Err("resource file changed during copy".into());
            }
        } else {
            return Err("resource directories may not contain special files".into());
        }
    }
    let after = fs::symlink_metadata(source)
        .map_err(|_| "resource directory changed during copy".to_string())?;
    if after.file_type().is_symlink() || !same_file(&metadata, &after) {
        return Err("resource directory changed during copy".into());
    }
    Ok(())
}

fn overlay_directory_no_symlinks(source: &Path, destination: &Path) -> InstallResult<()> {
    reject_tree_symlinks(source)?;
    reject_tree_symlinks(destination)?;
    let mut entries = fs::read_dir(source)
        .map_err(|_| "could not read resource directory".to_string())?
        .map(|entry| {
            entry
                .map(|entry| entry.path())
                .map_err(|_| "could not read resource directory".to_string())
        })
        .collect::<InstallResult<Vec<_>>>()?;
    entries.sort();
    for entry in entries {
        let metadata = fs::symlink_metadata(&entry)
            .map_err(|_| "resource directory changed during merge".to_string())?;
        let target = destination.join(
            entry
                .file_name()
                .ok_or_else(|| "resource path is invalid".to_string())?,
        );
        if metadata.is_dir() {
            match fs::symlink_metadata(&target) {
                Ok(existing) if existing.is_dir() && !existing.file_type().is_symlink() => {
                    overlay_directory_no_symlinks(&entry, &target)?;
                }
                Ok(existing) => {
                    if existing.file_type().is_symlink() {
                        return Err("existing skill contains a symbolic link".into());
                    }
                    remove_path_tree(&target)?;
                    copy_directory_no_symlinks(&entry, &target)?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    copy_directory_no_symlinks(&entry, &target)?
                }
                Err(_) => return Err("could not inspect existing skill file".into()),
            }
        } else if metadata.is_file() {
            if let Ok(existing) = fs::symlink_metadata(&target) {
                if existing.file_type().is_symlink() {
                    return Err("existing skill contains a symbolic link".into());
                }
                if existing.is_dir() {
                    remove_path_tree(&target)?;
                }
            }
            let bytes = read_optional_bytes(&entry)?
                .ok_or_else(|| "resource file disappeared during merge".to_string())?;
            atomic_write(&target, &bytes, source_mode(&entry).unwrap_or(0o600))?;
        } else if metadata.file_type().is_symlink() {
            return Err("resource directories may not contain symbolic links".into());
        } else {
            return Err("resource directories may not contain special files".into());
        }
    }
    Ok(())
}

fn replace_directory_staged(target: &Path, stage: &Path) -> InstallResult<()> {
    let old = unique_sibling(target, "old")?;
    let had_target = match fs::symlink_metadata(target) {
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(_) => return Err("could not inspect adapter destination".into()),
    };
    if had_target && fs::rename(target, &old).is_err() {
        let _ = remove_path_tree(stage);
        return Err("could not stage existing adapter directory".into());
    }
    if fs::rename(stage, target).is_err() {
        if had_target {
            let _ = fs::rename(&old, target);
        }
        let _ = remove_path_tree(stage);
        return Err("could not replace adapter directory".into());
    }
    if had_target {
        let _ = remove_path_tree(&old);
    }
    Ok(())
}

fn replace_directory_with_file(
    paths: &Paths,
    target: &Path,
    bytes: &[u8],
    mode: u32,
) -> InstallResult<()> {
    let stage = unique_sibling(target, "file-stage")?;
    atomic_write(&stage, bytes, mode)?;
    if let Err(error) = backup_target(paths, target) {
        let _ = fs::remove_file(&stage);
        return Err(error);
    }
    let old = unique_sibling(target, "old")?;
    if fs::rename(target, &old).is_err() {
        let _ = fs::remove_file(&stage);
        return Err("could not stage existing adapter directory".into());
    }
    if fs::rename(&stage, target).is_err() {
        let _ = fs::rename(&old, target);
        let _ = fs::remove_file(&stage);
        return Err("could not replace adapter directory with file".into());
    }
    let _ = remove_path_tree(&old);
    Ok(())
}

fn remove_path_tree(path: &Path) -> InstallResult<()> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| "could not inspect staged adapter path".to_string())?;
    if metadata.file_type().is_symlink() {
        return Err("staged adapter path is a symbolic link".into());
    }
    if metadata.is_dir() {
        reject_tree_symlinks(path)?;
        fs::remove_dir_all(path)
            .map_err(|_| "could not remove staged adapter directory".to_string())
    } else if metadata.is_file() {
        fs::remove_file(path).map_err(|_| "could not remove staged adapter file".to_string())
    } else {
        Err("staged adapter path is not a regular file or directory".into())
    }
}

fn source_mode(path: &Path) -> Option<u32> {
    let metadata = fs::symlink_metadata(path).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        Some(metadata.permissions().mode() & 0o777)
    }
    #[cfg(not(unix))]
    {
        Some(if metadata.is_dir() { 0o700 } else { 0o600 })
    }
}

fn has_special_mode_bits(metadata: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o7000 != 0
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        false
    }
}

fn set_file_path_mode(path: &Path, mode: u32) -> InstallResult<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode & 0o777))
            .map_err(|_| "could not set adapter file permissions".into())
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
        Ok(())
    }
}

fn set_directory_mode(path: &Path, mode: u32) -> InstallResult<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode & 0o777))
            .map_err(|_| "could not set adapter directory permissions".into())
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
        Ok(())
    }
}

#[cfg(unix)]
fn same_file(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    left.dev() == right.dev() && left.ino() == right.ino()
}

#[cfg(not(unix))]
fn same_file(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    left.len() == right.len() && left.modified().ok() == right.modified().ok()
}

fn now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

#[cfg(test)]
mod removal_tests {
    use super::{hash_directory_checked, remove_directory_quarantined_with};
    use crate::install::util::copy_tree_checked;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    struct TempTree(PathBuf);

    impl TempTree {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "yashik-adapter-remove-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempTree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn add_resource(root: &Path) -> PathBuf {
        let target = root.join("skill");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("SKILL.md"), b"instructions\n").unwrap();
        fs::write(target.join("helper.txt"), b"helper\n").unwrap();
        target
    }

    #[test]
    fn failed_quarantine_cleanup_restores_the_complete_owned_directory() {
        let temp = TempTree::new();
        let target = add_resource(&temp.0);
        let original = hash_directory_checked(&target).unwrap();
        let backup = temp.0.join("backup");
        copy_tree_checked(&target, &backup).unwrap();

        let result = remove_directory_quarantined_with(&target, &original, &backup, |_| {
            Err("injected cleanup failure".into())
        });

        assert!(result.is_err());
        assert!(target.is_dir());
        assert_eq!(hash_directory_checked(&target).unwrap(), original);
        assert_eq!(fs::read(target.join("helper.txt")).unwrap(), b"helper\n");
    }

    #[test]
    fn partial_quarantine_cleanup_never_leaves_a_partly_removed_active_target() {
        let temp = TempTree::new();
        let target = add_resource(&temp.0);
        let original = hash_directory_checked(&target).unwrap();
        let backup = temp.0.join("backup");
        copy_tree_checked(&target, &backup).unwrap();
        let mut quarantine = None;

        let result = remove_directory_quarantined_with(&target, &original, &backup, |path| {
            quarantine = Some(path.to_path_buf());
            fs::remove_file(path.join("helper.txt")).unwrap();
            Err("injected partial cleanup failure".into())
        });

        assert!(result.is_err());
        assert!(target.is_dir());
        assert_eq!(hash_directory_checked(&target).unwrap(), original);
        let quarantine = quarantine.unwrap();
        assert!(quarantine.is_dir());
        assert!(quarantine.join("SKILL.md").is_file());
        assert!(!quarantine.join("helper.txt").exists());
    }
}
