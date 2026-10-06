use std::path::{Component, Path};

use crate::schema::{
    Agent, Install, LocalEntry, Manifest, Mcp, ResourceFormat, Rule, Skill, Source,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidationIssue {
    pub path: String,
    pub message: String,
}

impl std::fmt::Display for ValidationIssue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.path, self.message)
    }
}

pub fn validate_manifest(manifest: &Manifest) -> Result<(), Vec<ValidationIssue>> {
    let mut issues = Vec::new();
    if manifest.version != 1 {
        issue(&mut issues, "version", "must be exactly 1");
    }

    if let Some(herdr) = &manifest.tools.herdr {
        if !is_valid_harness_version(&herdr.version) {
            issue(
                &mut issues,
                "tools.herdr.version",
                "must be `latest` or an exact semantic version",
            );
        }
    }

    if let Some(orca) = &manifest.tools.orca {
        if !is_valid_harness_version(&orca.version) {
            issue(
                &mut issues,
                "tools.orca.version",
                "must be `latest` or an exact semantic version",
            );
        }
    }

    validate_map("mcp", &manifest.mcp, &mut issues, validate_mcp);
    validate_map("skills", &manifest.skills, &mut issues, validate_skill);
    validate_map(
        "agents",
        &manifest.agents,
        &mut issues,
        |path, agent, issues| validate_agent(path, agent, false, issues),
    );
    validate_map(
        "rules",
        &manifest.rules,
        &mut issues,
        |path, rule, issues| validate_rule(path, rule, false, issues),
    );

    for (harness_id, harness) in &manifest.harnesses {
        let base = format!("harnesses.{}", harness_id.as_str());
        if let Some(version) = &harness.version {
            if !is_valid_harness_version(version) {
                issue(
                    &mut issues,
                    format!("{base}.version"),
                    "must be `latest` or an exact semantic version",
                );
            }
        }
        validate_local_map(&base, "mcp", &harness.mcp, &mut issues, validate_mcp);
        validate_local_map(
            &base,
            "skills",
            &harness.skills,
            &mut issues,
            validate_skill,
        );
        validate_local_map(
            &base,
            "agents",
            &harness.agents,
            &mut issues,
            |path, agent, issues| validate_agent(path, agent, true, issues),
        );
        validate_local_map(
            &base,
            "rules",
            &harness.rules,
            &mut issues,
            |path, rule, issues| validate_rule(path, rule, true, issues),
        );
    }

    if issues.is_empty() {
        Ok(())
    } else {
        Err(issues)
    }
}

/// Accept only the resolver keyword or strict exact semantic-version syntax.
pub fn is_valid_harness_version(value: &str) -> bool {
    value == "latest" || is_valid_exact_harness_version(value)
}

/// Validate SemVer 2.0 syntax for a pinned package version.
pub fn is_valid_exact_harness_version(value: &str) -> bool {
    if value.is_empty()
        || value.starts_with('-')
        || value.starts_with('v')
        || value.chars().any(char::is_whitespace)
    {
        return false;
    }
    let base = value.split(['-', '+']).next().unwrap_or(value);
    let components = base.split('.').collect::<Vec<_>>();
    if components.len() != 3
        || components.iter().any(|part| {
            part.is_empty()
                || (part.len() > 1 && part.starts_with('0'))
                || !part.bytes().all(|byte| byte.is_ascii_digit())
                || part.parse::<u64>().is_err()
        })
    {
        return false;
    }

    let suffix = value.strip_prefix(base).unwrap_or_default();
    let suffix_is_valid = if suffix.is_empty() {
        true
    } else if let Some(suffix) = suffix.strip_prefix('-') {
        let (pre, build) = suffix.split_once('+').unwrap_or((suffix, ""));
        let build_is_valid = !suffix.contains('+') || valid_semver_identifiers(build, false);
        valid_semver_identifiers(pre, true) && build_is_valid
    } else if let Some(build) = suffix.strip_prefix('+') {
        valid_semver_identifiers(build, false)
    } else {
        false
    };
    suffix_is_valid
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || ".-+".contains(character))
}

fn valid_semver_identifiers(value: &str, prerelease: bool) -> bool {
    !value.is_empty()
        && value.split('.').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                && (!prerelease
                    || !part.bytes().all(|byte| byte.is_ascii_digit())
                    || part == "0"
                    || !part.starts_with('0'))
        })
}

fn validate_map<T>(
    section: &str,
    values: &std::collections::BTreeMap<String, T>,
    issues: &mut Vec<ValidationIssue>,
    validate: impl Fn(&str, &T, &mut Vec<ValidationIssue>),
) {
    for (name, value) in values {
        let path = format!("{section}.{name}");
        validate_resource_name(issues, path.clone(), name);
        validate(&path, value, issues);
    }
}

fn validate_local_map<T>(
    base: &str,
    section: &str,
    values: &std::collections::BTreeMap<String, LocalEntry<T>>,
    issues: &mut Vec<ValidationIssue>,
    validate: impl Fn(&str, &T, &mut Vec<ValidationIssue>),
) {
    for (name, value) in values {
        let path = format!("{base}.{section}.{name}");
        validate_resource_name(issues, path.clone(), name);
        match value {
            LocalEntry::Disabled(disabled) if disabled.enabled => {
                issue(
                    issues,
                    path,
                    "the only supported disable entry is exactly {enabled: false}",
                );
            }
            LocalEntry::Disabled(_) => {}
            LocalEntry::Spec(spec) => validate(&path, spec, issues),
        }
    }
}

fn validate_mcp(path: &str, mcp: &Mcp, issues: &mut Vec<ValidationIssue>) {
    validate_source(&format!("{path}.source"), &mcp.source, issues);
    if let Some(from) = &mcp.from {
        validate_from(&format!("{path}.from"), from, issues);
    }
    validate_nonblank(
        issues,
        format!("{path}.run.command"),
        &mcp.run.command,
        "command must not be blank",
    );
    validate_interpolations(
        issues,
        format!("{path}.run.command"),
        &mcp.run.command,
        false,
    );
    for (index, argument) in mcp.run.args.iter().enumerate() {
        validate_interpolations(issues, format!("{path}.run.args[{index}]"), argument, false);
    }
    for (key, value) in &mcp.run.env {
        let env_path = format!("{path}.run.env.{key}");
        if !is_env_name(key) {
            issue(
                issues,
                env_path.clone(),
                "environment variable name must match [A-Za-z_][A-Za-z0-9_]*",
            );
        }
        if !is_env_reference(value) {
            issue(
                issues,
                env_path,
                "run.env values must be a single ${env:NAME} reference; literal values are not stored",
            );
        }
    }
    if let Some(install) = &mcp.install {
        validate_install(&format!("{path}.install"), install, issues);
    }
}

fn validate_install(path: &str, install: &Install, issues: &mut Vec<ValidationIssue>) {
    for (index, requirement) in install.requires.iter().enumerate() {
        if !matches!(requirement.as_str(), "node" | "bun") {
            issue(
                issues,
                format!("{path}.requires[{index}]"),
                "runtime ID must be one of the supported values: node or bun",
            );
        }
    }
    for (index, step) in install.steps.iter().enumerate() {
        let step_path = format!("{path}.steps[{index}]");
        let Some(command) = step.first() else {
            issue(issues, step_path, "argv step must contain a command");
            continue;
        };
        validate_nonblank(
            issues,
            format!("{step_path}[0]"),
            command,
            "command must not be blank",
        );
        for (argument_index, argument) in step.iter().enumerate() {
            if argument.contains("${") {
                issue(
                    issues,
                    format!("{step_path}[{argument_index}]"),
                    "install argv values are literal and must not contain interpolation tokens",
                );
            }
        }
    }
}

fn validate_skill(path: &str, skill: &Skill, issues: &mut Vec<ValidationIssue>) {
    validate_source(&format!("{path}.source"), &skill.source, issues);
    if let Some(from) = &skill.from {
        validate_from(&format!("{path}.from"), from, issues);
    }
}

fn validate_agent(path: &str, agent: &Agent, local: bool, issues: &mut Vec<ValidationIssue>) {
    validate_source(&format!("{path}.source"), &agent.source, issues);
    if let Some(from) = &agent.from {
        validate_from(&format!("{path}.from"), from, issues);
    }
    match agent.format {
        ResourceFormat::Portable => match &agent.description {
            Some(description) => validate_nonblank(
                issues,
                format!("{path}.description"),
                description,
                "portable agent description must not be blank",
            ),
            None => issue(
                issues,
                format!("{path}.description"),
                "required for portable agents",
            ),
        },
        ResourceFormat::Native => {
            if !local {
                issue(
                    issues,
                    format!("{path}.format"),
                    "native agents are allowed only inside a harness",
                );
            }
            if agent.description.is_some() {
                issue(
                    issues,
                    format!("{path}.description"),
                    "native agents do not accept description",
                );
            }
        }
    }
}

fn validate_rule(path: &str, rule: &Rule, local: bool, issues: &mut Vec<ValidationIssue>) {
    validate_source(&format!("{path}.source"), &rule.source, issues);
    if let Some(from) = &rule.from {
        validate_from(&format!("{path}.from"), from, issues);
    }
    if rule.format == ResourceFormat::Native && !local {
        issue(
            issues,
            format!("{path}.format"),
            "native rules are allowed only inside a harness",
        );
    }
}

fn validate_source(path: &str, source: &Source, issues: &mut Vec<ValidationIssue>) {
    match source {
        Source::Git { url, git_ref } => {
            validate_nonblank(
                issues,
                format!("{path}.url"),
                url,
                "Git URL must not be blank",
            );
            if let Some(git_ref) = git_ref {
                validate_nonblank(
                    issues,
                    format!("{path}.ref"),
                    git_ref,
                    "Git ref must not be blank",
                );
            }
            if crate::install::sources::validate_source_url(url).is_err() {
                issue(
                    issues,
                    format!("{path}.url"),
                    "Git source must use credential-free HTTPS or SSH",
                );
            }
            if let Some(git_ref) = git_ref {
                if crate::install::sources::validate_source_ref(git_ref).is_err() {
                    issue(
                        issues,
                        format!("{path}.ref"),
                        "Git ref is not a safe branch, tag or commit",
                    );
                }
            }
        }
        Source::Local { path: local_path } => {
            validate_nonblank(
                issues,
                format!("{path}.path"),
                local_path,
                "local path must not be blank",
            );
        }
    }
}

fn validate_from(path: &str, value: &str, issues: &mut Vec<ValidationIssue>) {
    validate_nonblank(issues, path.to_owned(), value, "path must not be blank");
    let path_value = Path::new(value);
    let windows_absolute = value.starts_with("\\\\")
        || value.starts_with("//")
        || value
            .as_bytes()
            .get(1)
            .is_some_and(|separator| *separator == b':');
    if path_value.is_absolute() || windows_absolute {
        issue(issues, path.to_owned(), "from must be a relative path");
        return;
    }
    if path_value
        .components()
        .any(|component| component == Component::ParentDir)
    {
        issue(
            issues,
            path.to_owned(),
            "from must not contain a `..` component",
        );
    }
}

fn validate_nonblank(issues: &mut Vec<ValidationIssue>, path: String, value: &str, message: &str) {
    if value.trim().is_empty() {
        issue(issues, path, message);
    }
}

fn validate_resource_name(issues: &mut Vec<ValidationIssue>, path: String, value: &str) {
    let mut bytes = value.bytes();
    let valid_first = bytes.next().is_some_and(|byte| byte.is_ascii_lowercase());
    let valid_rest =
        bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
    if !valid_first || !valid_rest || value.len() > 63 {
        issue(
            issues,
            path,
            "resource name must match ^[a-z][a-z0-9-]{0,62}$",
        );
    }
}

fn is_env_name(value: &str) -> bool {
    let mut bytes = value.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn is_env_reference(value: &str) -> bool {
    let Some(name) = value
        .strip_prefix("${env:")
        .and_then(|value| value.strip_suffix('}'))
    else {
        return false;
    };
    is_env_name(name)
}

fn validate_interpolations(
    issues: &mut Vec<ValidationIssue>,
    path: String,
    value: &str,
    allow_only_env: bool,
) {
    let mut rest = value;
    while let Some(start) = rest.find("${") {
        rest = &rest[start + 2..];
        let Some(end) = rest.find('}') else {
            issue(issues, path, "unclosed interpolation token");
            return;
        };
        let token = &rest[..end];
        if token == "source" && !allow_only_env {
            // `${source}` is resolved to the persistent writable artifact root.
        } else if let Some(name) = token.strip_prefix("env:") {
            if !is_env_name(name) {
                issue(issues, path.clone(), "invalid environment reference token");
            }
        } else {
            issue(issues, path.clone(), "unknown interpolation token");
        }
        rest = &rest[end + 1..];
    }
}

fn issue(issues: &mut Vec<ValidationIssue>, path: impl Into<String>, message: impl Into<String>) {
    issues.push(ValidationIssue {
        path: path.into(),
        message: message.into(),
    });
}

#[cfg(test)]
mod tests {
    use super::{is_valid_exact_harness_version, is_valid_harness_version};

    #[test]
    fn harness_version_accepts_only_latest_or_strict_semver() {
        assert!(is_valid_harness_version("latest"));
        assert!(is_valid_exact_harness_version("0.159.3"));
        assert!(is_valid_exact_harness_version("2.1.286-beta.1+build.42"));
        for invalid in [
            "",
            "latest; echo unsafe",
            "1.2",
            "v1.2.3",
            "01.2.3",
            "1.02.3",
            "1.2.03",
            "1.2.3-01",
            "1.2.3+",
            "1.2.3\nnode",
        ] {
            assert!(!is_valid_harness_version(invalid), "accepted {invalid:?}");
        }
    }
}
