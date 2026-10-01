use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use jsonc_parser::cst::CstRootNode;
use jsonc_parser::ParseOptions;
use yashik::install::adapters;
use yashik::install::api::{
    BindingPayload, BindingRequest, InstalledCli, Observation, Paths, ResourceKind, WriteChoice,
};
use yashik::schema::{HarnessId, ResourceFormat};

static NEXT: AtomicU64 = AtomicU64::new(0);
static ENV_LOCK: Mutex<()> = Mutex::new(());

struct TempTree(PathBuf);

impl TempTree {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "yashik-installer-adapters-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        let canonical = fs::canonicalize(path).unwrap();
        Self(canonical)
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

fn cli(harness: HarnessId, version: &str) -> InstalledCli {
    InstalledCli {
        harness,
        version: version.to_owned(),
        executable: PathBuf::from("/usr/bin/client"),
        integrity: None,
        runtime_bins: vec![],
        launcher_fingerprint: None,
    }
}

fn request(
    harness: HarnessId,
    kind: ResourceKind,
    name: &str,
    format: Option<ResourceFormat>,
    source_path: Option<PathBuf>,
) -> BindingRequest {
    BindingRequest {
        harness,
        kind,
        name: name.to_owned(),
        format,
        description: None,
        source_path,
        artifact_key: Some("source-artifact".into()),
        launch_id: None,
    }
}

struct EnvGuard {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl EnvGuard {
    fn set(key: &'static str, value: &Path) -> Self {
        let previous = std::env::var_os(key);
        std::env::set_var(key, value);
        Self { key, previous }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        if let Some(value) = self.previous.take() {
            std::env::set_var(self.key, value);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

#[test]
fn codex_native_agent_is_validated_and_copied_byte_for_byte() {
    let temp = TempTree::new();
    let paths = temp.paths();
    fs::create_dir_all(&paths.home).unwrap();
    let source = temp.path().join("artifact/backend.toml");
    fs::create_dir_all(source.parent().unwrap()).unwrap();
    let native = b"name = \"backend-developer\"\ndescription = \"Backend design and implementation\"\nmodel = \"gpt-5\"\nsandbox_mode = \"workspace-write\"\ndeveloper_instructions = '''\nUse the repository conventions.\nCheck relevant tests.\n'''\n";
    fs::write(&source, native).unwrap();
    let target = paths.home.join(".codex/agents/backend-developer.toml");
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(&target, b"old user-selected content\n").unwrap();

    let request = request(
        HarnessId::Codex,
        ResourceKind::Agent,
        "backend-developer",
        Some(ResourceFormat::Native),
        Some(source),
    );
    let prepared =
        adapters::prepare(&paths, &cli(HarnessId::Codex, "0.159.3"), &[request]).unwrap();
    assert_eq!(prepared.len(), 1);
    assert_eq!(
        adapters::inspect(&prepared[0]).unwrap(),
        Observation::Present {
            fingerprint: yashik::install::util::sha256_bytes(b"old user-selected content\n"),
        }
    );
    assert!(
        matches!(&prepared[0].payload, BindingPayload::File(bytes) if bytes.as_slice() == native)
    );

    let managed = adapters::apply(&paths, &prepared[0], WriteChoice::Replace).unwrap();
    assert_eq!(fs::read(&target).unwrap(), native);
    assert_eq!(
        adapters::inspect_managed(&managed).unwrap(),
        Observation::Present {
            fingerprint: managed.fingerprint.clone()
        }
    );
    let backups = paths.state.join("backups");
    let backup_dirs = fs::read_dir(&backups).unwrap().count();
    assert_eq!(backup_dirs, 1);
    let backup = fs::read_dir(backups)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path()
        .join("content");
    assert_eq!(fs::read(backup).unwrap(), b"old user-selected content\n");

    adapters::apply(&paths, &prepared[0], WriteChoice::Replace).unwrap();
    assert_eq!(
        fs::read_dir(paths.state.join("backups")).unwrap().count(),
        1
    );
}

#[test]
fn codex_native_agent_rejects_a_different_embedded_name() {
    let temp = TempTree::new();
    let paths = temp.paths();
    fs::create_dir_all(&paths.home).unwrap();
    let source = temp.path().join("artifact/reviewer.toml");
    fs::create_dir_all(source.parent().unwrap()).unwrap();
    fs::write(
        &source,
        "name = \"backend-developer\"\ndescription = \"Review code\"\ndeveloper_instructions = \"Find defects\"\n",
    )
    .unwrap();
    let request = request(
        HarnessId::Codex,
        ResourceKind::Agent,
        "reviewer",
        Some(ResourceFormat::Native),
        Some(source),
    );

    assert!(adapters::prepare(&paths, &cli(HarnessId::Codex, "0.159.3"), &[request]).is_err());
    assert!(!paths.home.join(".codex/agents/reviewer.toml").exists());
}

#[cfg(unix)]
#[test]
fn backup_refuses_a_symlinked_backup_root_without_writing_outside_state() {
    use std::os::unix::fs::symlink;

    let temp = TempTree::new();
    let paths = temp.paths();
    let outside = temp.path().join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::create_dir_all(&paths.state).unwrap();
    symlink(&outside, paths.state.join("backups")).unwrap();

    let source = temp.path().join("artifact/reviewer.toml");
    fs::create_dir_all(source.parent().unwrap()).unwrap();
    fs::write(
        &source,
        "name = \"reviewer\"\ndescription = \"Review code\"\ndeveloper_instructions = \"Find defects\"\n",
    )
    .unwrap();
    let target = paths.home.join(".codex/agents/reviewer.toml");
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(&target, b"keep me\n").unwrap();
    let request = request(
        HarnessId::Codex,
        ResourceKind::Agent,
        "reviewer",
        Some(ResourceFormat::Native),
        Some(source),
    );
    let prepared =
        adapters::prepare(&paths, &cli(HarnessId::Codex, "0.159.3"), &[request]).unwrap();

    assert!(adapters::apply(&paths, &prepared[0], WriteChoice::Replace).is_err());
    assert_eq!(fs::read(&target).unwrap(), b"keep me\n");
    assert_eq!(fs::read_dir(outside).unwrap().count(), 0);
}

#[cfg(unix)]
#[test]
fn backup_refuses_a_symlinked_state_parent_without_writing_to_its_target() {
    use std::os::unix::fs::symlink;

    let temp = TempTree::new();
    let paths = temp.paths();
    let outside = temp.path().join("outside-state");
    fs::create_dir_all(outside.join("backups")).unwrap();
    symlink(&outside, &paths.state).unwrap();

    let source = temp.path().join("artifact/reviewer.toml");
    fs::create_dir_all(source.parent().unwrap()).unwrap();
    fs::write(
        &source,
        "name = \"reviewer\"\ndescription = \"Review code\"\ndeveloper_instructions = \"Find defects\"\n",
    )
    .unwrap();
    let target = paths.home.join(".codex/agents/reviewer.toml");
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(&target, b"keep me\n").unwrap();
    let request = request(
        HarnessId::Codex,
        ResourceKind::Agent,
        "reviewer",
        Some(ResourceFormat::Native),
        Some(source),
    );
    let prepared =
        adapters::prepare(&paths, &cli(HarnessId::Codex, "0.159.3"), &[request]).unwrap();

    assert!(adapters::apply(&paths, &prepared[0], WriteChoice::Replace).is_err());
    assert_eq!(fs::read(&target).unwrap(), b"keep me\n");
    assert_eq!(fs::read_dir(outside.join("backups")).unwrap().count(), 0);
}

#[test]
fn opencode_v1_edits_only_its_jsonc_mcp_entry_and_detects_selected_entry_drift() {
    let _env_lock = ENV_LOCK.lock().unwrap();
    let temp = TempTree::new();
    let paths = temp.paths();
    fs::create_dir_all(&paths.home).unwrap();
    fs::create_dir_all(&paths.bin).unwrap();
    let config_dir = temp.path().join("opencode-config");
    fs::create_dir_all(&config_dir).unwrap();
    let _config_env = EnvGuard::set("OPENCODE_CONFIG_DIR", &config_dir);
    let config = config_dir.join("opencode.jsonc");
    let initial = r#"{
  // Keep this user comment.
  "$schema": "https://opencode.ai/config.json",
  "custom": true,
  "mcp": {
    // Keep existing server details too.
    "demo": { "type": "local", "command": ["old"], "environment": { "TOKEN": "existing-auth" }, "extra": true },
    "sibling": { "type": "local", "command": ["sibling"] }
  }
}
"#;
    fs::write(&config, initial).unwrap();
    let mut request = request(HarnessId::Opencode, ResourceKind::Mcp, "demo", None, None);
    request.launch_id = Some("launch-demo".into());
    let prepared =
        adapters::prepare(&paths, &cli(HarnessId::Opencode, "1.18.34"), &[request]).unwrap();
    let BindingPayload::Mcp(desired) = &prepared[0].payload else {
        panic!("expected MCP payload")
    };
    assert_eq!(desired["type"], "local");
    assert_eq!(desired["command"][1], "mcp-launch");

    let managed = adapters::apply(&paths, &prepared[0], WriteChoice::Replace).unwrap();
    let written = fs::read_to_string(&config).unwrap();
    assert!(written.contains("// Keep this user comment."));
    assert!(written.contains("// Keep existing server details too."));
    let root = CstRootNode::parse(&written, &ParseOptions::default()).unwrap();
    let value = root.value().unwrap().to_serde_value().unwrap();
    assert_eq!(value["custom"], true);
    assert_eq!(value["mcp"]["demo"]["type"], "local");
    assert_eq!(
        value["mcp"]["demo"]["environment"]["TOKEN"],
        "existing-auth"
    );
    assert!(value["mcp"]["demo"]["extra"].is_null());
    assert_eq!(value["mcp"]["sibling"]["command"][0], "sibling");
    assert!(
        value["mcp"]["servers"].is_null(),
        "OpenCode 1.x uses direct mcp entries"
    );

    let unrelated_edit = written.replace("\"custom\": true", "\"custom\": false");
    fs::write(&config, unrelated_edit).unwrap();
    assert_eq!(
        adapters::inspect_managed(&managed).unwrap(),
        Observation::Present {
            fingerprint: managed.fingerprint.clone()
        }
    );

    let selected_edit = fs::read_to_string(&config)
        .unwrap()
        .replace("\"enabled\": true", "\"enabled\": false");
    fs::write(&config, selected_edit).unwrap();
    let drifted = adapters::inspect_managed(&managed).unwrap();
    assert!(
        matches!(drifted, Observation::Present { fingerprint } if fingerprint != managed.fingerprint)
    );
}

#[test]
fn all_five_harnesses_write_their_supported_mcp_shape() {
    let _env_lock = ENV_LOCK.lock().unwrap();
    let temp = TempTree::new();
    let paths = temp.paths();
    fs::create_dir_all(&paths.home).unwrap();
    fs::create_dir_all(&paths.bin).unwrap();

    for (harness, version) in [
        (HarnessId::Codex, "0.159.3"),
        (HarnessId::Claude, "2.1.286"),
        (HarnessId::Opencode, "1.18.34"),
        (HarnessId::Pi, "0.99.2"),
        (HarnessId::Omp, "18.4.8"),
    ] {
        let target_override = temp.path().join(format!("agent-dir-{}", harness.as_str()));
        let _agent_dir = if matches!(harness, HarnessId::Pi | HarnessId::Omp) {
            Some(EnvGuard::set("PI_CODING_AGENT_DIR", &target_override))
        } else {
            None
        };
        let opencode_override = temp.path().join("opencode-config-all");
        let _opencode_dir = if harness == HarnessId::Opencode {
            Some(EnvGuard::set("OPENCODE_CONFIG_DIR", &opencode_override))
        } else {
            None
        };
        let mut input = request(harness, ResourceKind::Mcp, "example", None, None);
        input.launch_id = Some(format!("launch-{}", harness.as_str()));
        let prepared = adapters::prepare(&paths, &cli(harness, version), &[input]).unwrap();
        let binding = &prepared[0];
        let expected_target = match harness {
            HarnessId::Codex => paths.home.join(".codex/config.toml"),
            HarnessId::Claude => paths.home.join(".claude.json"),
            HarnessId::Opencode => opencode_override.join("opencode.jsonc"),
            HarnessId::Pi | HarnessId::Omp => target_override.join("mcp.json"),
        };
        assert_eq!(binding.target, expected_target);
        adapters::apply(&paths, binding, WriteChoice::Merge).unwrap();

        if harness == HarnessId::Codex {
            let text = fs::read_to_string(&binding.target).unwrap();
            let document = text.parse::<toml_edit::DocumentMut>().unwrap();
            assert_eq!(
                document["mcp_servers"]["example"]["command"].as_str(),
                Some(paths.bin.join("yashik").to_string_lossy().as_ref())
            );
            assert_eq!(
                document["mcp_servers"]["example"]["args"][0].as_str(),
                Some("mcp-launch")
            );
        } else {
            let text = fs::read_to_string(&binding.target).unwrap();
            let root = CstRootNode::parse(&text, &ParseOptions::default()).unwrap();
            let value = root.value().unwrap().to_serde_value().unwrap();
            if harness == HarnessId::Opencode {
                assert_eq!(value["mcp"]["example"]["type"], "local");
                assert_eq!(value["mcp"]["example"]["command"][1], "mcp-launch");
                assert_eq!(value["mcp"]["example"]["enabled"], true);
                assert!(value["mcp"]["servers"].is_null());
            } else {
                assert!(value["mcpServers"]["example"]["command"].is_string());
                assert_eq!(value["mcpServers"]["example"]["args"][0], "mcp-launch");
                if harness == HarnessId::Omp {
                    assert_eq!(value["mcpServers"]["example"]["type"], "stdio");
                }
            }
        }
        assert!(matches!(
            adapters::inspect(binding).unwrap(),
            Observation::Present { .. }
        ));
        drop(_agent_dir);
        drop(_opencode_dir);
    }
}

#[test]
fn portable_codex_rules_preserve_external_markdown_and_remove_only_the_owned_block() {
    let temp = TempTree::new();
    let paths = temp.paths();
    fs::create_dir_all(paths.home.join(".codex")).unwrap();
    let target = paths.home.join(".codex/AGENTS.md");
    fs::write(&target, "# Personal instructions\n\nKeep this paragraph.\n").unwrap();
    let source_a = temp.path().join("rules/a.md");
    let source_b = temp.path().join("rules/b.md");
    fs::create_dir_all(source_a.parent().unwrap()).unwrap();
    fs::write(&source_a, "Use the existing formatter.\n").unwrap();
    fs::write(&source_b, "Run a focused test before changing behavior.\n").unwrap();
    let requests = [
        request(
            HarnessId::Codex,
            ResourceKind::Rule,
            "formatting",
            Some(ResourceFormat::Portable),
            Some(source_a),
        ),
        request(
            HarnessId::Codex,
            ResourceKind::Rule,
            "testing",
            Some(ResourceFormat::Portable),
            Some(source_b),
        ),
    ];
    let prepared = adapters::prepare(&paths, &cli(HarnessId::Codex, "0.159.3"), &requests).unwrap();
    assert_eq!(prepared.len(), 1);
    assert_eq!(prepared[0].names, vec!["formatting", "testing"]);

    let managed = adapters::apply(&paths, &prepared[0], WriteChoice::Merge).unwrap();
    let written = fs::read_to_string(&target).unwrap();
    assert!(written.starts_with("# Personal instructions\n\nKeep this paragraph."));
    assert!(written.contains("## formatting"));
    assert!(written.contains("## testing"));
    assert_eq!(
        adapters::inspect_managed(&managed).unwrap(),
        Observation::Present {
            fingerprint: managed.fingerprint.clone()
        }
    );

    fs::write(
        &target,
        format!("{written}\n# Added outside the managed block\n"),
    )
    .unwrap();
    assert_eq!(
        adapters::inspect_managed(&managed).unwrap(),
        Observation::Present {
            fingerprint: managed.fingerprint.clone()
        }
    );
    adapters::remove(&paths, &managed).unwrap();
    let after_removal = fs::read_to_string(&target).unwrap();
    assert!(after_removal.contains("# Personal instructions"));
    assert!(after_removal.contains("# Added outside the managed block"));
    assert!(!after_removal.contains("yashik:rules:start"));
}

#[test]
fn skills_require_discovery_frontmatter_and_copy_without_destination_links() {
    let temp = TempTree::new();
    let paths = temp.paths();
    fs::create_dir_all(&paths.home).unwrap();
    let source = temp.path().join("artifact/skill");
    fs::create_dir_all(&source).unwrap();
    fs::write(
        source.join("SKILL.md"),
        "---\nname: sample-skill\ndescription: A test skill\n---\n\nInstructions.\n",
    )
    .unwrap();
    fs::write(source.join("helper.txt"), "helper\n").unwrap();
    let skill_request = request(
        HarnessId::Codex,
        ResourceKind::Skill,
        "sample-skill",
        None,
        Some(source.clone()),
    );
    let prepared =
        adapters::prepare(&paths, &cli(HarnessId::Codex, "0.159.3"), &[skill_request]).unwrap();
    let managed = adapters::apply(&paths, &prepared[0], WriteChoice::Replace).unwrap();
    let target = paths.home.join(".agents/skills/sample-skill");
    assert_eq!(
        fs::read_to_string(target.join("helper.txt")).unwrap(),
        "helper\n"
    );
    assert_eq!(
        adapters::inspect_managed(&managed).unwrap(),
        Observation::Present {
            fingerprint: managed.fingerprint.clone()
        }
    );

    fs::write(
        source.join("SKILL.md"),
        "---\nname: another-name\ndescription: A test skill\n---\n\nInstructions.\n",
    )
    .unwrap();
    let invalid = request(
        HarnessId::Codex,
        ResourceKind::Skill,
        "sample-skill",
        None,
        Some(source),
    );
    assert!(adapters::prepare(&paths, &cli(HarnessId::Codex, "0.159.3"), &[invalid]).is_err());
}

#[test]
fn explicit_replace_swaps_a_foreign_file_for_a_skill_directory() {
    let temp = TempTree::new();
    let paths = temp.paths();
    fs::create_dir_all(&paths.home).unwrap();
    let source = temp.path().join("artifact/skill");
    fs::create_dir_all(&source).unwrap();
    fs::write(
        source.join("SKILL.md"),
        "---\nname: sample-skill\ndescription: A test skill\n---\n\nInstructions.\n",
    )
    .unwrap();
    let request = request(
        HarnessId::Codex,
        ResourceKind::Skill,
        "sample-skill",
        None,
        Some(source),
    );
    let prepared =
        adapters::prepare(&paths, &cli(HarnessId::Codex, "0.159.3"), &[request]).unwrap();
    let target = paths.home.join(".agents/skills/sample-skill");
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(&target, b"existing user file\n").unwrap();

    assert!(adapters::apply(&paths, &prepared[0], WriteChoice::Merge).is_err());
    let managed = adapters::apply(&paths, &prepared[0], WriteChoice::Replace).unwrap();
    assert!(target.is_dir());
    assert_eq!(
        fs::read_to_string(target.join("SKILL.md")).unwrap(),
        "---\nname: sample-skill\ndescription: A test skill\n---\n\nInstructions.\n"
    );
    assert_eq!(
        adapters::inspect_managed(&managed).unwrap(),
        Observation::Present {
            fingerprint: managed.fingerprint.clone()
        }
    );
    let backups = fs::read_dir(paths.state.join("backups"))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(backups.len(), 1);
    assert_eq!(
        fs::read(backups[0].path().join("content")).unwrap(),
        b"existing user file\n"
    );

    adapters::remove(&paths, &managed).unwrap();
    assert!(!target.exists());
    let backups = fs::read_dir(paths.state.join("backups"))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(backups.len(), 2);
    assert!(backups.iter().any(|entry| {
        fs::read_to_string(entry.path().join("tree/SKILL.md")).is_ok_and(|content| {
            content.contains("name: sample-skill") && content.contains("Instructions.")
        })
    }));
    assert_eq!(
        fs::read_dir(target.parent().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry
                .file_name()
                .to_string_lossy()
                .contains("yashik-remove"))
            .count(),
        0
    );
}

#[test]
fn unsupported_capabilities_are_explicit() {
    assert!(adapters::capability(
        HarnessId::Pi,
        ResourceKind::Agent,
        Some(ResourceFormat::Portable),
        "reviewer"
    )
    .is_err());
    assert!(adapters::capability(
        HarnessId::Codex,
        ResourceKind::Rule,
        Some(ResourceFormat::Native),
        "rules"
    )
    .is_err());
    assert!(adapters::capability(
        HarnessId::Omp,
        ResourceKind::Rule,
        Some(ResourceFormat::Native),
        "rules"
    )
    .is_err());
    assert!(adapters::capability(
        HarnessId::Opencode,
        ResourceKind::Rule,
        Some(ResourceFormat::Native),
        "rules"
    )
    .is_err());
}

#[test]
fn resource_contracts_reject_cli_versions_that_have_not_been_verified() {
    let name = "reviewer";
    let native_agent = Some(ResourceFormat::Native);
    let old_codex = cli(HarnessId::Codex, "0.50.0");
    assert!(adapters::capability_for_cli(
        &old_codex,
        ResourceKind::Agent,
        native_agent.clone(),
        name
    )
    .is_err());
    assert!(
        adapters::capability_for_cli(&old_codex, ResourceKind::Skill, None, "sample-skill")
            .is_err()
    );
    assert!(adapters::capability_for_cli(
        &cli(HarnessId::Codex, "0.159.3"),
        ResourceKind::Agent,
        native_agent,
        name
    )
    .is_ok());

    for (harness, version) in [
        (HarnessId::Claude, "2.1.285"),
        (HarnessId::Opencode, "1.19.0"),
        (HarnessId::Opencode, "2.0.0"),
        (HarnessId::Pi, "0.99.1"),
        (HarnessId::Omp, "18.4.7"),
    ] {
        assert!(adapters::capability_for_cli(
            &cli(harness, version),
            ResourceKind::Mcp,
            None,
            name
        )
        .is_err());
    }
}

#[test]
fn json_mcp_replace_preserves_existing_auth_environment_for_all_clients() {
    let _env_lock = ENV_LOCK.lock().unwrap();
    for (harness, version) in [
        (HarnessId::Claude, "2.1.286"),
        (HarnessId::Pi, "0.99.2"),
        (HarnessId::Omp, "18.4.8"),
    ] {
        let temp = TempTree::new();
        let paths = temp.paths();
        fs::create_dir_all(&paths.home).unwrap();
        fs::create_dir_all(&paths.bin).unwrap();
        let mut input = request(harness, ResourceKind::Mcp, "demo", None, None);
        input.launch_id = Some("launch-demo".into());
        let prepared = adapters::prepare(&paths, &cli(harness, version), &[input]).unwrap();
        let target = &prepared[0].target;
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(target, r#"{"mcpServers":{"demo":{"command":"old","environment":{"TOKEN":"preserve-me"},"auth":{"kind":"existing"}}}}"#).unwrap();
        adapters::apply(&paths, &prepared[0], WriteChoice::Replace).unwrap();
        let written: serde_json::Value =
            serde_json::from_slice(&fs::read(target).unwrap()).unwrap();
        assert_eq!(
            written["mcpServers"]["demo"]["environment"]["TOKEN"],
            "preserve-me"
        );
        assert_eq!(written["mcpServers"]["demo"]["auth"]["kind"], "existing");
        assert_ne!(written["mcpServers"]["demo"]["command"], "old");
    }
}
