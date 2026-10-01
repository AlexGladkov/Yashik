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
