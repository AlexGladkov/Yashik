use std::path::Path;

use yashik::effective::build_effective;
use yashik::schema::{HarnessId, Manifest};
use yashik::validation::validate_manifest;

#[test]
fn checked_in_example_parses_and_builds_expected_effective_resources() {
    let manifest: Manifest = serde_yaml::from_str(include_str!("../examples/yashik.yaml")).unwrap();
    let effective = build_effective(&manifest, Path::new("/tmp/yashik-config")).unwrap();
    let codex = &effective.harnesses[&HarnessId::Codex];
    assert_eq!(
        codex.mcp.keys().map(String::as_str).collect::<Vec<_>>(),
        vec!["knowledge-search"]
    );
    assert!(codex.agents.contains_key("portable-reviewer"));
    assert!(effective.harnesses[&HarnessId::Pi].agents.is_empty());
    assert!(
        effective.harnesses[&HarnessId::Claude].agents["portable-reviewer"]
            .spec
            .format
            == yashik::schema::ResourceFormat::Native
    );
    assert_eq!(
        effective.harnesses[&HarnessId::Omp].rules["terminal-notes"]
            .local_source_path
            .as_deref(),
        Some(Path::new("/tmp/yashik-config/./resources"))
    );
    assert_eq!(
        codex.mcp["knowledge-search"].spec.run.env["SEARCH_API_TOKEN"],
        "${env:SEARCH_API_TOKEN}"
    );
}

#[test]
fn yaml_parser_rejects_malformed_unknown_duplicate_and_null_collections() {
    for input in [
        "version: [\nharnesses: {}",
        "version: 1\nharnesses: {}\nextra: true\n",
        "version: 1\nharnesses: {}\nskills: null\n",
        "version: 1\nharnesses: {}\nskills:\n  one:\n    source:\n      type: git\n      url: repo\n  one:\n    source:\n      type: git\n      url: another\n",
        "version: 1\nharnesses: {}\nmcp:\n  server:\n    source:\n      type: git\n      url: repo\n    transport: stdio\n    run:\n      command: node\n      env:\n        TOKEN: first\n        TOKEN: second\n",
        "version: 1\nharnesses:\n  codex:\n    skills:\n      remove:\n        enabled: false\n        from: extra\n",
        "version: 1\nharnesses:\n  codex:\n    agents:\n      native:\n        format: native\n        description: null\n        source:\n          type: git\n          url: repo\n",
        "version: 1\nharnesses:\n  codex:\n    agents:\n      portable:\n        format: portable\n        description: null\n        source:\n          type: git\n          url: repo\n",
        "version: 1\nharnesses:\n  codex:\n    agents:\n      portable:\n        format: portable\n        source:\n          type: git\n          url: repo\n",
    ] {
        assert!(serde_yaml::from_str::<Manifest>(input).is_err(), "YAML unexpectedly parsed: {input}");
    }
}

#[test]
fn yaml_parser_requires_fields_and_semantics_report_paths() {
    for input in [
        "harnesses: {}\n",
        "version: 1\nharnesses: []\n",
        "version: 1\nharnesses:\n  unknown: {}\n",
        "version: 1\nharnesses: {}\nmcp:\n  empty-step:\n    source:\n      type: git\n      url: repo\n    transport: stdio\n    run:\n      command: node\n    install:\n      steps:\n        - []\n",
    ] {
        let manifest = serde_yaml::from_str::<Manifest>(input);
        if let Ok(manifest) = manifest {
            assert!(validate_manifest(&manifest).is_err(), "semantic invalidity passed: {input}");
        }
    }
}

#[test]
fn herdr_is_a_closed_root_tool_with_defaults_and_semver_validation() {
    let manifest: Manifest =
        serde_yaml::from_str("version: 1\nharnesses: {}\ntools:\n  herdr: {}\n").unwrap();
    assert!(validate_manifest(&manifest).is_ok());
    let effective = build_effective(&manifest, Path::new("/tmp/config")).unwrap();
    assert_eq!(effective.herdr.unwrap().version, "latest");

    let pinned: Manifest = serde_yaml::from_str(
        "version: 1\nharnesses: {}\ntools:\n  herdr:\n    enabled: true\n    version: '0.9.3'\n",
    )
    .unwrap();
    assert_eq!(
        build_effective(&pinned, Path::new("/tmp/config"))
            .unwrap()
            .herdr
            .unwrap()
            .version,
        "0.9.3"
    );

    let disabled: Manifest =
        serde_yaml::from_str("version: 1\nharnesses: {}\ntools:\n  herdr:\n    enabled: false\n")
            .unwrap();
    assert!(build_effective(&disabled, Path::new("/tmp/config"))
        .unwrap()
        .herdr
        .is_none());

    let invalid: Manifest =
        serde_yaml::from_str("version: 1\nharnesses: {}\ntools:\n  herdr:\n    version: v0.9.3\n")
            .unwrap();
    let issues = validate_manifest(&invalid).unwrap_err();
    assert!(issues
        .iter()
        .any(|issue| issue.path == "tools.herdr.version"));
}

#[test]
fn herdr_schema_rejects_unknown_fields_duplicates_null_and_missing_harnesses() {
    for input in [
        "version: 1\nharnesses: {}\ntools: null\n",
        "version: 1\nharnesses: {}\ntools:\n  herdr: null\n",
        "version: 1\nharnesses: {}\ntools:\n  extra: {}\n",
        "version: 1\nharnesses: {}\ntools:\n  herdr:\n    enabled: 'true'\n",
        "version: 1\nharnesses: {}\ntools:\n  herdr:\n    version: latest\n    version: 1.2.3\n",
        "version: 1\ntools:\n  herdr: {}\n",
    ] {
        assert!(
            serde_yaml::from_str::<Manifest>(input).is_err(),
            "Herdr schema unexpectedly accepted: {input}"
        );
    }
}
