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
            validate_nonblank(
                &mut issues,
                format!("{base}.version"),
                version,
                "must not be blank",
            );
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

fn validate_map<T>(
    section: &str,
    values: &std::collections::BTreeMap<String, T>,
    issues: &mut Vec<ValidationIssue>,
    validate: impl Fn(&str, &T, &mut Vec<ValidationIssue>),
) {
    for (name, value) in values {
        let path = format!("{section}.{name}");
        validate_nonblank(
            issues,
            path.clone(),
            name,
            "resource name must not be blank",
        );
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
        validate_nonblank(
            issues,
            path.clone(),
            name,
            "resource name must not be blank",
        );
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
    for (key, _) in &mcp.run.env {
        validate_nonblank(
            issues,
            format!("{path}.run.env.{key}"),
            key,
            "environment variable name must not be blank",
        );
    }
    if let Some(install) = &mcp.install {
        validate_install(&format!("{path}.install"), install, issues);
    }
}

fn validate_install(path: &str, install: &Install, issues: &mut Vec<ValidationIssue>) {
    for (index, requirement) in install.requires.iter().enumerate() {
        validate_nonblank(
            issues,
            format!("{path}.requires[{index}]"),
            requirement,
            "runtime ID must not be blank",
        );
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

fn issue(issues: &mut Vec<ValidationIssue>, path: impl Into<String>, message: impl Into<String>) {
    issues.push(ValidationIssue {
        path: path.into(),
        message: message.into(),
    });
}
