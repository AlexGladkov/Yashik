use crate::effective::build_effective;
use crate::schema::{HarnessId, Manifest};
use crate::validation::validate_manifest;
use std::path::Path;

fn parse_json(input: &str) -> Result<Manifest, serde_json::Error> {
    serde_json::from_str(input)
}

const EMPTY: &str = r#"{"version":1,"harnesses":{}}"#;

#[test]
fn accepts_empty_manifest_and_rejects_unknown_schema_fields() {
    let manifest = parse_json(EMPTY).expect("empty manifest should be valid");
    assert!(validate_manifest(&manifest).is_ok());

    let error = parse_json(r#"{"version":1,"harnesses":{},"unknown":true}"#)
        .expect_err("unknown root fields must fail");
    assert!(error.to_string().contains("unknown field"));
}

#[test]
fn rejects_unknown_harnesses_required_fields_and_wrong_types() {
    for input in [
        r#"{"version":1,"harnesses":{"not-a-harness":{}}}"#,
        r#"{"version":1,"harnesses":{"codex":{"surprise":true}}}"#,
        r#"{"harnesses":{}}"#,
        r#"{"version":"1","harnesses":{}}"#,
        r#"{"version":1,"harnesses":[],"mcp":null}"#,
    ] {
        assert!(
            parse_json(input).is_err(),
            "input unexpectedly accepted: {input}"
        );
    }
}

#[test]
fn rejects_duplicate_keys_at_every_map_boundary() {
    let duplicate_harness = r#"{"version":1,"harnesses":{"codex":{},"codex":{}}}"#;
    let duplicate_resource = r#"{"version":1,"harnesses":{},"skills":{"same":{"source":{"type":"git","url":"https://example.invalid/a"}},"same":{"source":{"type":"git","url":"https://example.invalid/b"}}}}"#;
    let duplicate_local_resource = r#"{"version":1,"harnesses":{"codex":{"skills":{"same":{"source":{"type":"git","url":"https://example.invalid/a"}},"same":{"source":{"type":"git","url":"https://example.invalid/b"}}}}}}"#;
    let duplicate_env_in_local_override = r#"{"version":1,"harnesses":{"codex":{"mcp":{"same":{"source":{"type":"git","url":"https://example.invalid/a"},"transport":"stdio","run":{"command":"node","env":{"TOKEN":"first","TOKEN":"second"}}}}}}}"#;
    let duplicate_field = r#"{"version":1,"harnesses":{},"mcp":{"same":{"source":{"type":"git","url":"https://example.invalid/a"},"transport":"stdio","run":{"command":"node"},"from":"one","from":"two"}}}"#;

    for input in [
        duplicate_harness,
        duplicate_resource,
        duplicate_local_resource,
        duplicate_env_in_local_override,
        duplicate_field,
    ] {
        assert!(
            parse_json(input).is_err(),
            "duplicate key unexpectedly accepted: {input}"
        );
    }
}

#[test]
fn accepts_only_exact_local_disable_sentinel() {
    let valid = r#"{"version":1,"harnesses":{"codex":{"skills":{"gone":{"enabled":false}}}}}"#;
    let manifest = parse_json(valid).expect("exact false sentinel should parse");
    assert!(validate_manifest(&manifest).is_ok());

    for invalid in [
        r#"{"version":1,"harnesses":{"codex":{"skills":{"gone":{"enabled":true}}}}}"#,
        r#"{"version":1,"harnesses":{"codex":{"skills":{"gone":{"enabled":false,"from":"extra"}}}}}"#,
        r#"{"version":1,"harnesses":{},"skills":{"gone":{"enabled":false}}}"#,
    ] {
        match parse_json(invalid) {
            Ok(manifest) => assert!(
                validate_manifest(&manifest).is_err(),
                "invalid sentinel passed validation: {invalid}"
            ),
            Err(_) => {}
        }
    }
}

#[test]
fn rejects_invalid_argv_and_native_global_resources() {
    let invalid_argv = r#"{"version":1,"harnesses":{},"mcp":{"empty-step":{"source":{"type":"git","url":"https://example.invalid/a"},"transport":"stdio","run":{"command":"node"},"install":{"steps":[[]]}}}}"#;
    let manifest = parse_json(invalid_argv).unwrap();
    assert!(validate_manifest(&manifest)
        .unwrap_err()
        .iter()
        .any(|issue| issue.path == "mcp.empty-step.install.steps[0]"));

    let native_agent = r#"{"version":1,"harnesses":{},"agents":{"native":{"format":"native","source":{"type":"git","url":"https://example.invalid/a"}}}}"#;
    let manifest = parse_json(native_agent).unwrap();
    assert!(validate_manifest(&manifest)
        .unwrap_err()
        .iter()
        .any(|issue| issue.path == "agents.native.format"));
}

#[test]
fn agent_variants_require_portable_description_and_reject_native_description() {
    let valid_native = r#"{"version":1,"harnesses":{"codex":{"agents":{"native":{"format":"native","source":{"type":"git","url":"repo"}}}}}}"#;
    let manifest = parse_json(valid_native).expect("native agent with only source should parse");
    assert!(validate_manifest(&manifest).is_ok());

    for invalid in [
        r#"{"version":1,"harnesses":{"codex":{"agents":{"native":{"format":"native","description":null,"source":{"type":"git","url":"repo"}}}}}}"#,
        r#"{"version":1,"harnesses":{"codex":{"agents":{"native":{"format":"native","description":"unexpected","source":{"type":"git","url":"repo"}}}}}}"#,
        r#"{"version":1,"harnesses":{"codex":{"agents":{"portable":{"format":"portable","description":null,"source":{"type":"git","url":"repo"}}}}}}"#,
        r#"{"version":1,"harnesses":{"codex":{"agents":{"portable":{"format":"portable","source":{"type":"git","url":"repo"}}}}}}"#,
    ] {
        assert!(
            parse_json(invalid).is_err(),
            "invalid agent variant accepted: {invalid}"
        );
    }
}

#[test]
fn validates_all_specs_even_for_disabled_harnesses() {
    let invalid = r#"{"version":1,"harnesses":{"pi":{"enabled":false,"mcp":{"broken":{"source":{"type":"git","url":" "},"transport":"stdio","run":{"command":"node"}}}}}}"#;
    let manifest = parse_json(invalid).unwrap();
    assert!(validate_manifest(&manifest)
        .unwrap_err()
        .iter()
        .any(|issue| issue.path == "harnesses.pi.mcp.broken.source.url"));
}

#[test]
fn inheritance_replaces_adds_and_disables_with_type_independence() {
    let json = r#"{
      "version": 1,
      "harnesses": {
        "codex": {
          "mcp": {
            "shared": {"source":{"type":"local","path":"override"},"transport":"stdio","run":{"command":"replacement","args":["--new"]}},
            "local-only": {"source":{"type":"git","url":"https://example.invalid/local"},"transport":"stdio","run":{"command":"local"}},
            "disabled": {"enabled":false}
          },
          "skills": {"disabled": {"enabled":false}},
          "agents": {"disabled": {"enabled":false}},
          "rules": {"disabled": {"enabled":false}}
        },
        "pi": {"enabled": false}
      },
      "mcp": {
        "shared": {"source":{"type":"git","url":"https://example.invalid/common"},"transport":"stdio","run":{"command":"common"}},
        "disabled": {"source":{"type":"git","url":"https://example.invalid/disabled"},"transport":"stdio","run":{"command":"unused"}},
        "inherited": {"source":{"type":"git","url":"https://example.invalid/inherited"},"transport":"stdio","run":{"command":"inherited"}}
      },
      "skills": {"disabled": {"source":{"type":"git","url":"https://example.invalid/skill"}}},
      "agents": {"disabled": {"format":"portable","description":"valid","source":{"type":"git","url":"https://example.invalid/agent"}}},
      "rules": {"disabled": {"format":"portable","source":{"type":"git","url":"https://example.invalid/rule"}}}
    }"#;
    let manifest = parse_json(json).expect("schema should parse");
    let effective =
        build_effective(&manifest, Path::new("/tmp/project")).expect("schema should validate");
    assert_eq!(
        effective.harnesses.keys().copied().collect::<Vec<_>>(),
        vec![HarnessId::Codex]
    );

    let codex = &effective.harnesses[&HarnessId::Codex];
    assert_eq!(
        codex.mcp.keys().map(String::as_str).collect::<Vec<_>>(),
        vec!["inherited", "local-only", "shared"]
    );
    assert_eq!(codex.mcp["shared"].spec.run.command, "replacement");
    assert_eq!(codex.mcp["shared"].spec.run.args, vec!["--new"]);
    assert_eq!(
        codex.mcp["shared"].local_source_path.as_deref(),
        Some(Path::new("/tmp/project/override"))
    );
    assert!(!codex.skills.contains_key("disabled"));
    assert!(!codex.agents.contains_key("disabled"));
    assert!(!codex.rules.contains_key("disabled"));
}

#[test]
fn inherited_local_source_paths_are_resolved_from_manifest_directory() {
    let input = r#"{"version":1,"harnesses":{"codex":{}},"skills":{"relative":{"source":{"type":"local","path":"./kit"}},"absolute":{"source":{"type":"local","path":"/opt/kit"}}}}"#;
    let manifest = parse_json(input).unwrap();
    let effective = build_effective(&manifest, Path::new("/home/user/config")).unwrap();
    let skills = &effective.harnesses[&HarnessId::Codex].skills;
    assert_eq!(
        skills["relative"].local_source_path.as_deref(),
        Some(Path::new("/home/user/config/./kit"))
    );
    assert_eq!(
        skills["absolute"].local_source_path.as_deref(),
        Some(Path::new("/opt/kit"))
    );
}

#[test]
fn from_paths_are_checked_lexically_and_local_source_paths_are_not_canonicalized() {
    let valid_local_path = r#"{"version":1,"harnesses":{"codex":{}},"skills":{"external":{"source":{"type":"local","path":"../outside"},"from":"skills/tool"}}}"#;
    let manifest = parse_json(valid_local_path).unwrap();
    let effective = build_effective(&manifest, Path::new("/workspace/config")).unwrap();
    assert_eq!(
        effective.harnesses[&HarnessId::Codex].skills["external"]
            .local_source_path
            .as_deref(),
        Some(Path::new("/workspace/config/../outside"))
    );

    for from in [
        "",
        "  ",
        "../escape",
        "a/../../escape",
        "/absolute",
        "C:/absolute",
    ] {
        let json = format!(
            r#"{{"version":1,"harnesses":{{}},"skills":{{"bad":{{"source":{{"type":"git","url":"repo"}},"from":{}}}}}}}"#,
            serde_json::to_string(from).unwrap()
        );
        let manifest = parse_json(&json).unwrap();
        assert!(
            validate_manifest(&manifest).is_err(),
            "invalid from accepted: {from:?}"
        );
    }
}
