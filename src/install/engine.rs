use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::effective::{EffectiveHarness, EffectiveManifest};
use crate::install::api::{
    BindingPayload, BindingRequest, InstallResult, InstalledCli, LaunchSpec, ManagedBinding,
    Outcome, Paths, PreparedBinding, ResourceKind, RuntimeEnv, WriteChoice,
};
use crate::install::state::{ArtifactRecord, OperationPhase, OperationRecord, SourceRecord, State};
use crate::install::{adapters, executor, paths, runtime, sources, state, tools, util};
use crate::schema::{HarnessId, Mcp, ResourceFormat, Source};

#[derive(Clone, Debug, Default)]
pub struct RunReport {
    pub operations: Vec<ReportOperation>,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct ReportOperation {
    pub id: String,
    pub outcome: Outcome,
    pub message: Option<String>,
}

impl RunReport {
    pub fn has_failures(&self) -> bool {
        self.operations.iter().any(|operation| {
            matches!(
                operation.outcome,
                Outcome::Failed | Outcome::Blocked | Outcome::Interrupted
            )
        })
    }

    fn push(&mut self, id: impl Into<String>, outcome: Outcome, message: Option<String>) {
        self.operations.push(ReportOperation {
            id: id.into(),
            outcome,
            message,
        });
    }
}

#[derive(Clone)]
struct ResourceInput {
    harness: HarnessId,
    kind: ResourceKind,
    name: String,
    id: String,
    source: Source,
    local_source_path: Option<PathBuf>,
    from: Option<String>,
    format: Option<ResourceFormat>,
    description: Option<String>,
    mcp: Option<Mcp>,
}

pub fn init(effective: &EffectiveManifest, install_paths: &Paths) -> InstallResult<RunReport> {
    let _lock = paths::lock_init(install_paths)?;
    let mut state = state::load(install_paths)?;
    let mut report = RunReport::default();
    let resources = gather_resources(effective);
    let desired_bindings = desired_binding_ids(effective, &resources);
    let desired_clis = effective
        .harnesses
        .keys()
        .map(|harness| harness.as_str().to_owned())
        .collect::<BTreeSet<_>>();

    let missing_host_tools =
        bootstrap_host_tools(install_paths, effective, &mut state, &mut report)?;
    tools::reconcile(
        install_paths,
        effective.herdr.as_ref(),
        &mut state,
        &mut report,
    )?;
    let mut runtime_cache = BTreeMap::<String, Result<RuntimeEnv, String>>::new();
    let mut cli_by_harness = BTreeMap::<HarnessId, InstalledCli>::new();

    for harness in effective.harnesses.values() {
        if let Some(cli) = ensure_cli(
            install_paths,
            harness,
            &mut state,
            &mut report,
            &mut runtime_cache,
        )? {
            cli_by_harness.insert(harness.id, cli);
        }
    }

    let mut source_cache = BTreeMap::new();
    let mut artifact_cache = BTreeMap::new();
    let mut launcher_ready: Option<Result<bool, String>> = None;

    for harness in effective.harnesses.values() {
        for input in resources
            .iter()
            .filter(|input| input.harness == harness.id && input.kind != ResourceKind::Rule)
        {
            let Some(cli) = cli_by_harness.get(&harness.id) else {
                complete_without_mutation(
                    install_paths,
                    &mut state,
                    &mut report,
                    &input.id,
                    Some(input.id.clone()),
                    Outcome::Blocked,
                    Some("harness CLI is unavailable".into()),
                )?;
                continue;
            };
            process_resource(
                install_paths,
                input,
                cli,
                &missing_host_tools,
                &mut state,
                &mut report,
                &mut source_cache,
                &mut artifact_cache,
                &mut runtime_cache,
                &mut launcher_ready,
            )?;
        }

        let rule_inputs = resources
            .iter()
            .filter(|input| input.harness == harness.id && input.kind == ResourceKind::Rule)
            .cloned()
            .collect::<Vec<_>>();
        if !rule_inputs.is_empty() {
            let Some(cli) = cli_by_harness.get(&harness.id) else {
                for input in &rule_inputs {
                    complete_without_mutation(
                        install_paths,
                        &mut state,
                        &mut report,
                        &input.id,
                        Some(input.id.clone()),
                        Outcome::Blocked,
                        Some("harness CLI is unavailable".into()),
                    )?;
                }
                continue;
            };
            process_rules(
                install_paths,
                harness.id,
                &rule_inputs,
                cli,
                &missing_host_tools,
                &mut state,
                &mut report,
                &mut source_cache,
            )?;
        }
    }

    reconcile_removed_bindings(install_paths, &desired_bindings, &mut state, &mut report)?;
    let artifact_candidates = state.artifacts.keys().cloned().collect::<Vec<_>>();
    collect_unreferenced_artifacts(install_paths, &mut state, &mut report, &artifact_candidates)?;
    report_removed_clis(install_paths, &desired_clis, &mut state, &mut report)?;
    report
        .notes
        .push("Authentication and remote model access were not checked.".into());
    Ok(report)
}

fn gather_resources(effective: &EffectiveManifest) -> Vec<ResourceInput> {
    let mut resources = Vec::new();
    for harness in effective.harnesses.values() {
        for (name, resource) in &harness.mcp {
            resources.push(ResourceInput {
                harness: harness.id,
                kind: ResourceKind::Mcp,
                name: name.clone(),
                id: resource_id(harness.id, ResourceKind::Mcp, name),
                source: resource.spec.source.clone(),
                local_source_path: resource.local_source_path.clone(),
                from: resource.spec.from.clone(),
                format: None,
                description: None,
                mcp: Some(resource.spec.clone()),
            });
        }
        for (name, resource) in &harness.skills {
            resources.push(ResourceInput {
                harness: harness.id,
                kind: ResourceKind::Skill,
                name: name.clone(),
                id: resource_id(harness.id, ResourceKind::Skill, name),
                source: resource.spec.source.clone(),
                local_source_path: resource.local_source_path.clone(),
                from: resource.spec.from.clone(),
                format: None,
                description: None,
                mcp: None,
            });
        }
        for (name, resource) in &harness.agents {
            resources.push(ResourceInput {
                harness: harness.id,
                kind: ResourceKind::Agent,
                name: name.clone(),
                id: resource_id(harness.id, ResourceKind::Agent, name),
                source: resource.spec.source.clone(),
                local_source_path: resource.local_source_path.clone(),
                from: resource.spec.from.clone(),
                format: Some(resource.spec.format.clone()),
                description: resource.spec.description.clone(),
                mcp: None,
            });
        }
        for (name, resource) in &harness.rules {
            resources.push(ResourceInput {
                harness: harness.id,
                kind: ResourceKind::Rule,
                name: name.clone(),
                id: resource_id(harness.id, ResourceKind::Rule, name),
                source: resource.spec.source.clone(),
                local_source_path: resource.local_source_path.clone(),
                from: resource.spec.from.clone(),
                format: Some(resource.spec.format.clone()),
                description: None,
                mcp: None,
            });
        }
    }
    resources.sort_by(|left, right| left.id.cmp(&right.id));
    resources
}

fn desired_binding_ids(
    effective: &EffectiveManifest,
    resources: &[ResourceInput],
) -> BTreeSet<String> {
    resources
        .iter()
        .map(|resource| {
            if resource.kind == ResourceKind::Rule && resource.harness != HarnessId::Claude {
                format!("{}/rules-block", resource.harness.as_str())
            } else {
                resource.id.clone()
            }
        })
        .chain(
            effective
                .harnesses
                .values()
                .filter(|harness| !harness.rules.is_empty() && harness.id != HarnessId::Claude)
                .map(|harness| format!("{}/rules-block", harness.id.as_str())),
        )
        .collect()
}

// This operation shares memoized source, artifact, runtime and launcher caches across a run.
#[allow(clippy::too_many_arguments)]
fn process_resource(
    install_paths: &Paths,
    input: &ResourceInput,
    cli: &InstalledCli,
    missing_host_tools: &BTreeSet<String>,
    state: &mut State,
    report: &mut RunReport,
    source_cache: &mut BTreeMap<String, Result<crate::install::api::ResolvedSource, String>>,
    artifact_cache: &mut BTreeMap<String, Result<(PathBuf, Vec<PathBuf>), String>>,
    runtime_cache: &mut BTreeMap<String, Result<RuntimeEnv, String>>,
    launcher_ready: &mut Option<Result<bool, String>>,
) -> InstallResult<()> {
    if let Err(reason) =
        adapters::capability_for_cli(cli, input.kind, input.format.clone(), &input.name)
    {
        let message = if confirm_unsupported(&input.id, &reason) {
            "unsupported resource explicitly skipped".to_owned()
        } else {
            "no skip/cancel choice was provided; existing installation retained".to_owned()
        };
        complete_without_mutation(
            install_paths,
            state,
            report,
            &input.id,
            Some(input.id.clone()),
            Outcome::Skipped,
            Some(format!("{reason}; {message}")),
        )?;
        return Ok(());
    }

    let resolved = match resolve_cached(
        install_paths,
        &input.source,
        input.local_source_path.as_deref(),
        missing_host_tools,
        source_cache,
    ) {
        Ok(resolved) => resolved,
        Err(error) => {
            complete_without_mutation(
                install_paths,
                state,
                report,
                &input.id,
                Some(input.id.clone()),
                Outcome::Failed,
                Some(error),
            )?;
            return Ok(());
        }
    };
    remember_source(install_paths, state, &input.source, &resolved)?;
    let selected = match sources::select(&resolved, input.from.as_deref()) {
        Ok(selected) => selected,
        Err(error) => {
            complete_without_mutation(
                install_paths,
                state,
                report,
                &input.id,
                Some(input.id.clone()),
                Outcome::Failed,
                Some(error),
            )?;
            return Ok(());
        }
    };

    if input.mcp.is_some() && launcher_ready.is_none() {
        *launcher_ready = Some(ensure_self_launcher(install_paths));
    }
    if input.mcp.is_some() {
        match launcher_ready.as_ref() {
            Some(Ok(true)) => {}
            Some(Ok(false)) => {
                complete_without_mutation(
                    install_paths,
                    state,
                    report,
                    &input.id,
                    Some(input.id.clone()),
                    Outcome::Blocked,
                    Some("persistent MCP launcher was not approved for replacement".into()),
                )?;
                return Ok(());
            }
            Some(Err(error)) => {
                complete_without_mutation(
                    install_paths,
                    state,
                    report,
                    &input.id,
                    Some(input.id.clone()),
                    Outcome::Failed,
                    Some(error.clone()),
                )?;
                return Ok(());
            }
            None => unreachable!(),
        }
    }

    let (source_path, artifact_key, launch_spec, launch_id, launch_unavailable) =
        if let Some(mcp) = &input.mcp {
            {
                let artifact_key = mcp_artifact_key(mcp, &resolved)?;
                let requirements = mcp
                    .install
                    .as_ref()
                    .map(|install| install.requires.as_slice())
                    .unwrap_or(&[]);
                let required_bins = match ensure_requirements(
                    install_paths,
                    requirements,
                    state,
                    report,
                    runtime_cache,
                ) {
                    Ok(bins) => bins,
                    Err(error) => {
                        complete_without_mutation(
                            install_paths,
                            state,
                            report,
                            &input.id,
                            Some(input.id.clone()),
                            Outcome::Blocked,
                            Some(error),
                        )?;
                        return Ok(());
                    }
                };
                let artifact = match artifact_cache.get(&artifact_key).cloned() {
                    Some(result) => result,
                    None => {
                        let result = build_artifact(
                            install_paths,
                            mcp,
                            &resolved,
                            &artifact_key,
                            &required_bins,
                            state,
                            report,
                        )?;
                        artifact_cache.insert(artifact_key.clone(), result.clone());
                        result
                    }
                };
                match artifact {
                    Ok((artifact_root, artifact_bins)) => {
                        let mut bins = cli.runtime_bins.clone();
                        bins.extend(artifact_bins);
                        dedup_paths(&mut bins);
                        let launch_id = launch_id(input, &artifact_key, mcp)?;
                        (
                            None,
                            Some(artifact_key),
                            Some(LaunchSpec {
                                artifact_root,
                                run: mcp.run.clone(),
                                runtime_bins: bins,
                            }),
                            Some(launch_id),
                            false,
                        )
                    }
                    Err(error) => {
                        complete_without_mutation(
                            install_paths,
                            state,
                            report,
                            &input.id,
                            Some(input.id.clone()),
                            Outcome::Failed,
                            Some(error),
                        )?;
                        return Ok(());
                    }
                }
            }
        } else {
            (Some(selected), None, None, None, false)
        };

    debug_assert!(!launch_unavailable);

    let request = BindingRequest {
        harness: input.harness,
        kind: input.kind,
        name: input.name.clone(),
        format: input.format.clone(),
        description: input.description.clone(),
        source_path,
        artifact_key: artifact_key.clone(),
        launch_id: launch_id.clone(),
    };
    let launch_spec = launch_id.zip(launch_spec);
    let prepared = match adapters::prepare(install_paths, cli, &[request]) {
        Ok(prepared) => prepared,
        Err(error) => {
            complete_without_mutation(
                install_paths,
                state,
                report,
                &input.id,
                Some(input.id.clone()),
                Outcome::Failed,
                Some(error),
            )?;
            return Ok(());
        }
    };
    for binding in prepared {
        let spec = if binding.kind == ResourceKind::Mcp {
            launch_spec.clone()
        } else {
            None
        };
        reconcile_binding(install_paths, state, report, binding, spec)?;
    }
    Ok(())
}

// Portable rules are prepared as one batch so a single source failure preserves the full block.
#[allow(clippy::too_many_arguments)]
fn process_rules(
    install_paths: &Paths,
    harness: HarnessId,
    inputs: &[ResourceInput],
    cli: &InstalledCli,
    missing_host_tools: &BTreeSet<String>,
    state: &mut State,
    report: &mut RunReport,
    source_cache: &mut BTreeMap<String, Result<crate::install::api::ResolvedSource, String>>,
) -> InstallResult<()> {
    let mut portable_requests = Vec::new();
    let mut portable_ids = Vec::new();
    let mut portable_failed = false;

    for input in inputs {
        if let Err(reason) =
            adapters::capability_for_cli(cli, ResourceKind::Rule, input.format.clone(), &input.name)
        {
            if harness != HarnessId::Claude {
                portable_failed = true;
            }
            let message = if confirm_unsupported(&input.id, &reason) {
                "unsupported resource explicitly skipped".to_owned()
            } else {
                "no skip/cancel choice was provided; existing installation retained".to_owned()
            };
            complete_without_mutation(
                install_paths,
                state,
                report,
                &input.id,
                Some(input.id.clone()),
                Outcome::Skipped,
                Some(format!("{reason}; {message}")),
            )?;
            continue;
        }

        if harness == HarnessId::Claude {
            process_claude_rule(
                install_paths,
                input,
                cli,
                missing_host_tools,
                state,
                report,
                source_cache,
            )?;
            continue;
        }

        let resolved = match resolve_cached(
            install_paths,
            &input.source,
            input.local_source_path.as_deref(),
            missing_host_tools,
            source_cache,
        ) {
            Ok(resolved) => resolved,
            Err(error) => {
                portable_failed = true;
                complete_without_mutation(
                    install_paths,
                    state,
                    report,
                    &input.id,
                    Some(format!("{}/rules-block", harness.as_str())),
                    Outcome::Failed,
                    Some(error),
                )?;
                continue;
            }
        };
        remember_source(install_paths, state, &input.source, &resolved)?;
        let selected = match sources::select(&resolved, input.from.as_deref()) {
            Ok(selected) => selected,
            Err(error) => {
                portable_failed = true;
                complete_without_mutation(
                    install_paths,
                    state,
                    report,
                    &input.id,
                    Some(format!("{}/rules-block", harness.as_str())),
                    Outcome::Failed,
                    Some(error),
                )?;
                continue;
            }
        };
        if input.format != Some(ResourceFormat::Portable) {
            portable_failed = true;
            complete_without_mutation(
                install_paths,
                state,
                report,
                &input.id,
                Some(format!("{}/rules-block", harness.as_str())),
                Outcome::Failed,
                Some("adapter unexpectedly accepted a non-portable rule".into()),
            )?;
            continue;
        }
        portable_ids.push(input.id.clone());
        portable_requests.push(BindingRequest {
            harness,
            kind: ResourceKind::Rule,
            name: input.name.clone(),
            format: input.format.clone(),
            description: None,
            source_path: Some(selected),
            artifact_key: None,
            launch_id: None,
        });
    }

    if portable_requests.is_empty() {
        return Ok(());
    }
    let aggregate_id = format!("{}/rules-block", harness.as_str());
    if portable_failed {
        for resource_id in portable_ids {
            complete_without_mutation(
                install_paths,
                state,
                report,
                &resource_id,
                Some(aggregate_id.clone()),
                Outcome::Blocked,
                Some(
                    "portable rules block retained because another desired rule could not be read"
                        .into(),
                ),
            )?;
        }
        return Ok(());
    }

    let prepared = match adapters::prepare(install_paths, cli, &portable_requests) {
        Ok(prepared) => prepared,
        Err(error) => {
            for resource_id in portable_ids {
                complete_without_mutation(
                    install_paths,
                    state,
                    report,
                    &resource_id,
                    Some(aggregate_id.clone()),
                    Outcome::Failed,
                    Some(error.clone()),
                )?;
            }
            return Ok(());
        }
    };
    for binding in prepared {
        reconcile_binding(install_paths, state, report, binding, None)?;
    }
    Ok(())
}

fn process_claude_rule(
    install_paths: &Paths,
    input: &ResourceInput,
    cli: &InstalledCli,
    missing_host_tools: &BTreeSet<String>,
    state: &mut State,
    report: &mut RunReport,
    source_cache: &mut BTreeMap<String, Result<crate::install::api::ResolvedSource, String>>,
) -> InstallResult<()> {
    let resolved = match resolve_cached(
        install_paths,
        &input.source,
        input.local_source_path.as_deref(),
        missing_host_tools,
        source_cache,
    ) {
        Ok(resolved) => resolved,
        Err(error) => {
            complete_without_mutation(
                install_paths,
                state,
                report,
                &input.id,
                Some(input.id.clone()),
                Outcome::Failed,
                Some(error),
            )?;
            return Ok(());
        }
    };
    remember_source(install_paths, state, &input.source, &resolved)?;
    let selected = match sources::select(&resolved, input.from.as_deref()) {
        Ok(selected) => selected,
        Err(error) => {
            complete_without_mutation(
                install_paths,
                state,
                report,
                &input.id,
                Some(input.id.clone()),
                Outcome::Failed,
                Some(error),
            )?;
            return Ok(());
        }
    };
    let request = BindingRequest {
        harness: input.harness,
        kind: ResourceKind::Rule,
        name: input.name.clone(),
        format: input.format.clone(),
        description: None,
        source_path: Some(selected),
        artifact_key: None,
        launch_id: None,
    };
    let prepared = match adapters::prepare(install_paths, cli, &[request]) {
        Ok(prepared) => prepared,
        Err(error) => {
            complete_without_mutation(
                install_paths,
                state,
                report,
                &input.id,
                Some(input.id.clone()),
                Outcome::Failed,
                Some(error),
            )?;
            return Ok(());
        }
    };
    for binding in prepared {
        reconcile_binding(install_paths, state, report, binding, None)?;
    }
    Ok(())
}

fn reconcile_binding(
    install_paths: &Paths,
    state: &mut State,
    report: &mut RunReport,
    prepared: PreparedBinding,
    launch_spec: Option<(String, LaunchSpec)>,
) -> InstallResult<()> {
    let map_key = prepared.id.clone();
    let previous = state.bindings.get(&map_key).cloned();
    let same_target = previous.as_ref().is_some_and(|managed| {
        managed.target == prepared.target && managed.selector == prepared.selector
    });
    let observation = if same_target {
        adapters::inspect_managed(previous.as_ref().expect("same target has a record"))
    } else {
        adapters::inspect(&prepared)
    };
    let observation = match observation {
        Ok(observation) => observation,
        Err(error) => {
            complete_without_mutation(
                install_paths,
                state,
                report,
                &prepared.id,
                Some(prepared.id.clone()),
                Outcome::Failed,
                Some(error),
            )?;
            return Ok(());
        }
    };

    let is_clean_owned = same_target
        && matches!(&observation, crate::install::api::Observation::Present { fingerprint } if previous.as_ref().is_some_and(|record| record.fingerprint == *fingerprint));
    if let crate::install::api::Observation::Present { fingerprint } = &observation {
        if is_clean_owned
            && previous
                .as_ref()
                .is_some_and(|record| record.desired_fingerprint == prepared.desired_fingerprint)
        {
            complete_without_mutation(
                install_paths,
                state,
                report,
                &prepared.id,
                Some(prepared.id.clone()),
                Outcome::Unchanged,
                Some("desired binding is already present".into()),
            )?;
            return Ok(());
        }
        let choice = if is_clean_owned {
            default_write_choice(&prepared)
        } else {
            match prompt_conflict(&prepared, same_target, fingerprint) {
                Some(choice) => choice,
                None => WriteChoice::Skip,
            }
        };
        if choice == WriteChoice::Skip {
            complete_without_mutation(
                install_paths,
                state,
                report,
                &prepared.id,
                Some(prepared.id.clone()),
                Outcome::Skipped,
                Some("conflicting or drifted target was retained".into()),
            )?;
            return Ok(());
        }
        mutate_binding(
            install_paths,
            state,
            report,
            prepared,
            launch_spec,
            choice,
            previous,
        )
    } else {
        let choice = default_write_choice(&prepared);
        mutate_binding(
            install_paths,
            state,
            report,
            prepared,
            launch_spec,
            choice,
            previous,
        )
    }
}

fn mutate_binding(
    install_paths: &Paths,
    state: &mut State,
    report: &mut RunReport,
    prepared: PreparedBinding,
    launch_spec: Option<(String, LaunchSpec)>,
    choice: WriteChoice,
    previous: Option<ManagedBinding>,
) -> InstallResult<()> {
    let id = prepared.id.clone();
    begin_operation(install_paths, state, &id, Some(id.clone()))?;
    if let Some((launch_id, spec)) = launch_spec {
        if let Err(error) = write_launch_spec(install_paths, &launch_id, &spec) {
            complete_without_mutation(
                install_paths,
                state,
                report,
                &id,
                Some(id.clone()),
                Outcome::Failed,
                Some(error),
            )?;
            return Ok(());
        }
    }
    let managed = match adapters::apply(install_paths, &prepared, choice) {
        Ok(managed) => managed,
        Err(error) => {
            complete_without_mutation(
                install_paths,
                state,
                report,
                &id,
                Some(id.clone()),
                Outcome::Failed,
                Some(error),
            )?;
            return Ok(());
        }
    };
    let obsolete_artifact_keys = previous
        .as_ref()
        .map(|binding| binding.artifact_keys.clone())
        .unwrap_or_default();
    finish_operation(
        install_paths,
        state,
        report,
        &id,
        Some(id.clone()),
        Outcome::Succeeded,
        Some("binding installed".into()),
        |next| {
            if let Some(previous) = previous {
                if previous.target != managed.target || previous.selector != managed.selector {
                    archive_old_binding(next, &previous);
                }
            }
            next.bindings.insert(id.clone(), managed);
        },
    )?;
    collect_unreferenced_artifacts(install_paths, state, report, &obsolete_artifact_keys)
}

fn archive_old_binding(state: &mut State, previous: &ManagedBinding) {
    let hash = util::sha256_bytes(previous.target.to_string_lossy().as_bytes());
    let archive_key = format!("{}@old-{}", previous.id, &hash[..12]);
    let mut archived = previous.clone();
    archived.id = archive_key.clone();
    state.bindings.insert(archive_key, archived);
}

fn default_write_choice(prepared: &PreparedBinding) -> WriteChoice {
    match prepared.payload {
        BindingPayload::File(_) => WriteChoice::Replace,
        BindingPayload::Directory(_) | BindingPayload::Mcp(_) | BindingPayload::RulesBlock(_) => {
            WriteChoice::Merge
        }
    }
}

fn prompt_conflict(
    prepared: &PreparedBinding,
    owned_drift: bool,
    fingerprint: &str,
) -> Option<WriteChoice> {
    let reason = if owned_drift {
        "the Yashik-owned entry has changed"
    } else {
        "the target already contains an unowned entry"
    };
    let supports_merge = !matches!(prepared.payload, BindingPayload::File(_));
    let choices = if supports_merge {
        format!(
            "Conflict for {} at {} ({reason}, fingerprint {}). [m]erge/[r]eplace/[s]kip: ",
            prepared.id,
            prepared.target.display(),
            &fingerprint[..fingerprint.len().min(12)]
        )
    } else {
        format!(
            "Conflict for {} at {} ({reason}; native file). [r]eplace/[s]kip: ",
            prepared.id,
            prepared.target.display()
        )
    };
    match prompt(&choices)
        .as_deref()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("r") | Some("replace") => Some(WriteChoice::Replace),
        Some("m") | Some("merge") if supports_merge => Some(WriteChoice::Merge),
        _ => Some(WriteChoice::Skip),
    }
}

fn confirm_unsupported(id: &str, reason: &str) -> bool {
    eprintln!("Unsupported capability for {id}: {reason}");
    matches!(
        prompt("Skip this unsupported resource? [s]kip/[c]ancel: ").as_deref(),
        Some("s" | "S" | "skip" | "Skip")
    )
}

fn reconcile_removed_bindings(
    install_paths: &Paths,
    desired: &BTreeSet<String>,
    state: &mut State,
    report: &mut RunReport,
) -> InstallResult<()> {
    let obsolete = state
        .bindings
        .iter()
        .filter(|(key, _)| !desired.contains(*key))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect::<Vec<_>>();
    for (key, binding) in obsolete {
        let observation = match adapters::inspect_managed(&binding) {
            Ok(observation) => observation,
            Err(error) => {
                complete_without_mutation(
                    install_paths,
                    state,
                    report,
                    &key,
                    Some(binding.id.clone()),
                    Outcome::Failed,
                    Some(error),
                )?;
                continue;
            }
        };
        match observation {
            crate::install::api::Observation::Missing => {
                finish_operation(
                    install_paths,
                    state,
                    report,
                    &key,
                    Some(binding.id.clone()),
                    Outcome::Unchanged,
                    Some("removed target was already missing; ownership cleared".into()),
                    |next| {
                        next.bindings.remove(&key);
                    },
                )?;
                collect_unreferenced_artifacts(
                    install_paths,
                    state,
                    report,
                    &binding.artifact_keys,
                )?;
            }
            crate::install::api::Observation::Present { fingerprint } => {
                let drifted = fingerprint != binding.fingerprint;
                let question = if drifted {
                    format!("Owned resource {} was edited and is no longer desired. Remove the edited entry with a protected backup? [y/N]: ", binding.id)
                } else {
                    format!(
                        "Remove owned resource {} from {}? [y/N]: ",
                        binding.id,
                        binding.target.display()
                    )
                };
                if !prompt_yes(&question) {
                    complete_without_mutation(
                        install_paths,
                        state,
                        report,
                        &key,
                        Some(binding.id.clone()),
                        Outcome::PendingRemoval,
                        Some("resource remains installed; removal was not confirmed".into()),
                    )?;
                    continue;
                }
                begin_operation(install_paths, state, &key, Some(binding.id.clone()))?;
                match adapters::remove(install_paths, &binding) {
                    Ok(()) => {
                        finish_operation(
                            install_paths,
                            state,
                            report,
                            &key,
                            Some(binding.id.clone()),
                            Outcome::Succeeded,
                            Some("owned binding removed".into()),
                            |next| {
                                next.bindings.remove(&key);
                            },
                        )?;
                        collect_unreferenced_artifacts(
                            install_paths,
                            state,
                            report,
                            &binding.artifact_keys,
                        )?;
                    }
                    Err(error) => {
                        complete_without_mutation(
                            install_paths,
                            state,
                            report,
                            &key,
                            Some(binding.id.clone()),
                            Outcome::Failed,
                            Some(error),
                        )?;
                    }
                }
            }
        }
    }
    Ok(())
}

fn collect_unreferenced_artifacts(
    install_paths: &Paths,
    state: &mut State,
    report: &mut RunReport,
    candidates: &[String],
) -> InstallResult<()> {
    for key in candidates {
        if state
            .bindings
            .values()
            .any(|binding| binding.artifact_keys.iter().any(|used| used == key))
        {
            continue;
        }
        let Some(record) = state.artifacts.get(key).cloned() else {
            continue;
        };
        if !is_sha256_name(key) {
            report.notes.push(
                "Artifact cleanup skipped because its recorded key is not a SHA-256 name.".into(),
            );
            continue;
        }
        let expected_root = install_paths
            .data
            .join("artifacts")
            .join(util::sha256_bytes(key.as_bytes()));
        if record.root != expected_root {
            complete_without_mutation(
                install_paths,
                state,
                report,
                &format!("cleanup/artifact/{key}"),
                None,
                Outcome::Failed,
                Some("recorded artifact root is outside its managed path".into()),
            )?;
            continue;
        }

        let operation_id = format!("cleanup/artifact/{key}");
        begin_operation(install_paths, state, &operation_id, None)?;
        let cleanup = remove_launch_specs_for_artifact(install_paths, &record.root)
            .and_then(|()| sources::remove_artifact(install_paths, &record));
        match cleanup {
            Ok(()) => finish_operation(
                install_paths,
                state,
                report,
                &operation_id,
                None,
                Outcome::Succeeded,
                Some("unreferenced MCP artifact and obsolete launch specifications removed".into()),
                |next| {
                    next.artifacts.remove(key);
                },
            )?,
            Err(error) => complete_without_mutation(
                install_paths,
                state,
                report,
                &operation_id,
                None,
                Outcome::Failed,
                Some(format!("unreferenced artifact cleanup failed: {error}")),
            )?,
        }
    }
    Ok(())
}

fn remove_launch_specs_for_artifact(
    install_paths: &Paths,
    artifact_root: &Path,
) -> InstallResult<()> {
    let directory = install_paths.state.join("launch-specs");
    match fs::symlink_metadata(&directory) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        _ => return Err("MCP launch specification directory is not safe to clean".into()),
    }
    let entries = fs::read_dir(&directory)
        .map_err(|_| "could not inspect MCP launch specifications for cleanup".to_owned())?;
    for entry in entries {
        let entry =
            entry.map_err(|_| "could not inspect an MCP launch specification".to_owned())?;
        let path = entry.path();
        let Some(id) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        if path.extension().and_then(|extension| extension.to_str()) != Some("json")
            || !is_sha256_name(id)
        {
            continue;
        }
        let metadata = fs::symlink_metadata(&path)
            .map_err(|_| "could not inspect an MCP launch specification".to_owned())?;
        if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > 1024 * 1024
        {
            continue;
        }
        let bytes = util::read_file_checked_bounded(&directory, &path, 1024 * 1024)?;
        let Ok(spec) = serde_json::from_slice::<LaunchSpec>(&bytes) else {
            continue;
        };
        if spec.artifact_root == artifact_root {
            fs::remove_file(path)
                .map_err(|_| "could not remove an obsolete MCP launch specification".to_owned())?;
        }
    }
    Ok(())
}

fn is_sha256_name(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn report_removed_clis(
    install_paths: &Paths,
    desired: &BTreeSet<String>,
    state: &mut State,
    report: &mut RunReport,
) -> InstallResult<()> {
    let obsolete = state
        .clis
        .iter()
        .filter(|(harness, _)| !desired.contains(*harness))
        .map(|(harness, cli)| (harness.clone(), cli.clone()))
        .collect::<Vec<_>>();
    for (harness, cli) in obsolete {
        let id = format!("cli/{harness}");
        if state
            .bindings
            .values()
            .any(|binding| binding.harness == cli.harness)
        {
            complete_without_mutation(
                install_paths,
                state,
                report,
                &id,
                None,
                Outcome::PendingRemoval,
                Some("harness CLI is retained because managed resources remain installed or pending removal".into()),
            )?;
            continue;
        }
        if !prompt_yes(&format!(
            "{} {} is no longer desired. Remove its managed CLI package and launcher from the Yashik user prefix? [y/N]: ",
            cli.harness.as_str(),
            cli.version
        )) {
            complete_without_mutation(
                install_paths,
                state,
                report,
                &id,
                None,
                Outcome::PendingRemoval,
                Some("owned harness CLI and launcher remain installed; removal was not confirmed".into()),
            )?;
            continue;
        }
        begin_operation(install_paths, state, &id, None)?;
        match runtime::remove_cli(install_paths, &cli) {
            Ok(()) => finish_operation(
                install_paths,
                state,
                report,
                &id,
                None,
                Outcome::Succeeded,
                Some("managed CLI package and launcher removed; shared runtimes retained".into()),
                |next| {
                    next.clis.remove(&harness);
                },
            )?,
            Err(error) => complete_without_mutation(
                install_paths,
                state,
                report,
                &id,
                None,
                Outcome::Failed,
                Some(error),
            )?,
        }
    }
    Ok(())
}

enum CliLauncherApproval {
    NoExisting,
    Approved(String),
    Skip,
}

fn approve_cli_launcher_replacement(
    install_paths: &Paths,
    harness: HarnessId,
    state: &State,
) -> InstallResult<CliLauncherApproval> {
    let target = install_paths.bin.join(harness_cli_binary(harness));
    let metadata = match fs::symlink_metadata(&target) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(CliLauncherApproval::NoExisting)
        }
        Err(_) => return Err("cannot inspect the harness launcher target".into()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("harness launcher target is not a regular file".into());
    }
    if metadata.len() > 1024 * 1024 {
        return Err("existing harness launcher exceeds the supported size".into());
    }
    let bytes = util::read_file_checked_bounded(&install_paths.bin, &target, 1024 * 1024)?;
    let current_hash = util::sha256_bytes(&bytes);
    if state
        .clis
        .get(harness.as_str())
        .and_then(|cli| cli.launcher_fingerprint.as_deref())
        == Some(current_hash.as_str())
    {
        return Ok(CliLauncherApproval::Approved(current_hash));
    }

    let question = format!(
        "A launcher already exists at {} and is not verified as Yashik-owned. Replace it after the CLI package is ready? [r]eplace/[s]kip: ",
        target.display()
    );
    if matches!(
        prompt(&question)
            .as_deref()
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("r" | "replace")
    ) {
        Ok(CliLauncherApproval::Approved(current_hash))
    } else {
        Ok(CliLauncherApproval::Skip)
    }
}

fn harness_cli_binary(harness: HarnessId) -> &'static str {
    match harness {
        HarnessId::Codex => "codex",
        HarnessId::Claude => "claude",
        HarnessId::Opencode => "opencode",
        HarnessId::Pi => "pi",
        HarnessId::Omp => "omp",
    }
}

fn ensure_cli(
    install_paths: &Paths,
    harness: &EffectiveHarness,
    state: &mut State,
    report: &mut RunReport,
    runtime_cache: &mut BTreeMap<String, Result<RuntimeEnv, String>>,
) -> InstallResult<Option<InstalledCli>> {
    let id = format!("cli/{}", harness.id.as_str());
    begin_operation(install_paths, state, &id, None)?;
    let approved_launcher_hash =
        match approve_cli_launcher_replacement(install_paths, harness.id, state)? {
            CliLauncherApproval::NoExisting => None,
            CliLauncherApproval::Approved(hash) => Some(hash),
            CliLauncherApproval::Skip => {
                complete_without_mutation(
                    install_paths,
                    state,
                    report,
                    &id,
                    None,
                    Outcome::Skipped,
                    Some("existing unowned or edited launcher was retained".into()),
                )?;
                return Ok(None);
            }
        };
    let runtime_name = if harness.id == HarnessId::Omp {
        "bun"
    } else {
        "node"
    };
    if let Err(error) = ensure_runtime(install_paths, runtime_name, state, report, runtime_cache) {
        complete_without_mutation(
            install_paths,
            state,
            report,
            &id,
            None,
            Outcome::Blocked,
            Some(error),
        )?;
        return Ok(None);
    }
    match runtime::install_cli_checked(
        install_paths,
        harness.id,
        harness.version.as_deref(),
        approved_launcher_hash.as_deref(),
    ) {
        Ok(cli) => {
            let cli_copy = cli.clone();
            finish_operation(
                install_paths,
                state,
                report,
                &id,
                None,
                Outcome::Succeeded,
                Some(format!("{} {} is ready", cli.harness.as_str(), cli.version)),
                |next| {
                    next.clis.insert(cli.harness.as_str().to_owned(), cli_copy);
                },
            )?;
            Ok(Some(cli))
        }
        Err(error) => {
            complete_without_mutation(
                install_paths,
                state,
                report,
                &id,
                None,
                Outcome::Failed,
                Some(error),
            )?;
            Ok(None)
        }
    }
}

fn ensure_runtime(
    install_paths: &Paths,
    runtime_name: &str,
    state: &mut State,
    report: &mut RunReport,
    runtime_cache: &mut BTreeMap<String, Result<RuntimeEnv, String>>,
) -> InstallResult<RuntimeEnv> {
    if let Some(result) = runtime_cache.get(runtime_name) {
        return result.clone();
    }
    let id = format!("runtime/{runtime_name}");
    begin_operation(install_paths, state, &id, None)?;
    let result = runtime::ensure(install_paths, runtime_name);
    runtime_cache.insert(runtime_name.to_owned(), result.clone());
    match result {
        Ok(runtime_env) => {
            complete_without_mutation(
                install_paths,
                state,
                report,
                &id,
                None,
                Outcome::Succeeded,
                Some(format!("{runtime_name} runtime is ready")),
            )?;
            Ok(runtime_env)
        }
        Err(error) => {
            complete_without_mutation(
                install_paths,
                state,
                report,
                &id,
                None,
                Outcome::Failed,
                Some(error.clone()),
            )?;
            Err(error)
        }
    }
}

fn ensure_requirements(
    install_paths: &Paths,
    requirements: &[String],
    state: &mut State,
    report: &mut RunReport,
    runtime_cache: &mut BTreeMap<String, Result<RuntimeEnv, String>>,
) -> InstallResult<Vec<PathBuf>> {
    let mut bins = Vec::new();
    for requirement in requirements {
        let env = ensure_runtime(install_paths, requirement, state, report, runtime_cache)?;
        bins.extend(env.bin_dirs);
    }
    dedup_paths(&mut bins);
    Ok(bins)
}

fn bootstrap_host_tools(
    install_paths: &Paths,
    effective: &EffectiveManifest,
    state: &mut State,
    report: &mut RunReport,
) -> InstallResult<BTreeSet<String>> {
    let mut required = BTreeSet::<String>::new();
    if effective.herdr.is_some() {
        required.insert("curl".into());
    }
    if effective
        .harnesses
        .keys()
        .any(|harness| *harness != HarnessId::Omp)
    {
        required.insert("curl".into());
        required.insert("tar".into());
        required.insert("xz".into());
    }
    if effective.harnesses.contains_key(&HarnessId::Omp) {
        required.insert("curl".into());
        required.insert("unzip".into());
    }
    let resources = gather_resources(effective);
    if resources
        .iter()
        .any(|resource| matches!(resource.source, Source::Git { .. }))
    {
        required.insert("git".into());
    }
    if required.is_empty() {
        return Ok(BTreeSet::new());
    }
    let required = required.into_iter().collect::<Vec<_>>();
    let recipe = match runtime::bootstrap_recipe(&required) {
        Ok(Some(recipe)) => recipe,
        Ok(None) => return Ok(BTreeSet::new()),
        Err(error) => {
            report.push("bootstrap", Outcome::Failed, Some(error));
            return Ok(required.into_iter().collect());
        }
    };
    let id = "bootstrap/system-tools".to_owned();
    begin_operation(install_paths, state, &id, None)?;
    if recipe.needs_sudo
        && !prompt_yes(&format!(
            "Install required system tools ({}) using sudo? [y/N]: ",
            recipe.missing_tools.join(", ")
        ))
    {
        let missing = recipe
            .missing_tools
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        complete_without_mutation(
            install_paths,
            state,
            report,
            &id,
            None,
            Outcome::Blocked,
            Some("system dependency installation was not approved".into()),
        )?;
        return Ok(missing);
    }
    for command in &recipe.commands {
        let mut argv = command.clone();
        if recipe.needs_sudo {
            argv.insert(0, "sudo".into());
        }
        if let Err(error) = executor::run(
            &argv,
            &install_paths.home,
            &RuntimeEnv {
                bin_dirs: Vec::new(),
            },
        ) {
            complete_without_mutation(
                install_paths,
                state,
                report,
                &id,
                None,
                Outcome::Failed,
                Some(error),
            )?;
            return Ok(recipe.missing_tools.into_iter().collect());
        }
    }
    complete_without_mutation(
        install_paths,
        state,
        report,
        &id,
        None,
        Outcome::Succeeded,
        Some("required system tools are available".into()),
    )?;
    Ok(BTreeSet::new())
}

fn resolve_cached(
    install_paths: &Paths,
    source: &Source,
    local_source_path: Option<&Path>,
    missing_host_tools: &BTreeSet<String>,
    source_cache: &mut BTreeMap<String, Result<crate::install::api::ResolvedSource, String>>,
) -> InstallResult<crate::install::api::ResolvedSource> {
    if matches!(source, Source::Git { .. }) && missing_host_tools.contains("git") {
        return Err("Git source is blocked because the required git tool is unavailable".into());
    }
    let key = source_request_key(source, local_source_path)?;
    let result = source_cache
        .entry(key)
        .or_insert_with(|| sources::resolve(install_paths, source, local_source_path));
    result.clone()
}

fn source_request_key(source: &Source, local_source_path: Option<&Path>) -> InstallResult<String> {
    #[derive(Serialize)]
    struct Request<'a> {
        source: &'a Source,
        local_source_path: Option<&'a Path>,
    }
    let bytes = serde_json::to_vec(&Request {
        source,
        local_source_path,
    })
    .map_err(|_| "cannot fingerprint source request".to_owned())?;
    Ok(util::sha256_bytes(&bytes))
}

fn remember_source(
    install_paths: &Paths,
    state: &mut State,
    source: &Source,
    resolved: &crate::install::api::ResolvedSource,
) -> InstallResult<()> {
    if state
        .sources
        .get(&resolved.key)
        .is_some_and(|record| record.revision == resolved.revision && record.root == resolved.root)
    {
        return Ok(());
    }
    let mut next = state.clone();
    let url = match source {
        Source::Git { url, .. } => Some(url.clone()),
        Source::Local { .. } => None,
    };
    next.sources.insert(
        resolved.key.clone(),
        SourceRecord {
            key: resolved.key.clone(),
            revision: resolved.revision.clone(),
            root: resolved.root.clone(),
            url,
        },
    );
    state::save(install_paths, &next)?;
    *state = next;
    Ok(())
}

fn mcp_artifact_key(
    mcp: &Mcp,
    resolved: &crate::install::api::ResolvedSource,
) -> InstallResult<String> {
    let spec = serde_json::to_vec(mcp)
        .map_err(|_| "cannot fingerprint MCP build specification".to_owned())?;
    let mut bytes = b"yashik-artifact-v1\0".to_vec();
    bytes.extend_from_slice(resolved.key.as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(resolved.revision.as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(&spec);
    Ok(util::sha256_bytes(&bytes))
}

fn build_artifact(
    install_paths: &Paths,
    mcp: &Mcp,
    resolved: &crate::install::api::ResolvedSource,
    artifact_key: &str,
    runtime_bins: &[PathBuf],
    state: &mut State,
    report: &mut RunReport,
) -> InstallResult<Result<(PathBuf, Vec<PathBuf>), String>> {
    if let Some(record) = state.artifacts.get(artifact_key) {
        if record.root.is_dir() {
            return Ok(Ok((record.root.clone(), runtime_bins.to_vec())));
        }
    }
    let id = format!("build/{artifact_key}");
    begin_operation(install_paths, state, &id, None)?;
    let artifact_root = match sources::materialize_artifact(install_paths, resolved, artifact_key) {
        Ok(path) => path,
        Err(error) => {
            complete_without_mutation(
                install_paths,
                state,
                report,
                &id,
                None,
                Outcome::Failed,
                Some(error.clone()),
            )?;
            return Ok(Err(error));
        }
    };
    if let Some(install) = &mcp.install {
        for step in &install.steps {
            if let Err(error) = executor::run(
                step,
                &artifact_root,
                &RuntimeEnv {
                    bin_dirs: runtime_bins.to_vec(),
                },
            ) {
                complete_without_mutation(
                    install_paths,
                    state,
                    report,
                    &id,
                    None,
                    Outcome::Failed,
                    Some(error.clone()),
                )?;
                return Ok(Err(error));
            }
        }
    }
    let record = ArtifactRecord {
        key: artifact_key.to_owned(),
        source_key: resolved.key.clone(),
        root: artifact_root.clone(),
    };
    finish_operation(
        install_paths,
        state,
        report,
        &id,
        None,
        Outcome::Succeeded,
        Some("MCP artifact materialized and built".into()),
        |next| {
            next.artifacts.insert(artifact_key.to_owned(), record);
        },
    )?;
    Ok(Ok((artifact_root, runtime_bins.to_vec())))
}

fn launch_id(input: &ResourceInput, artifact_key: &str, mcp: &Mcp) -> InstallResult<String> {
    #[derive(Serialize)]
    struct Identity<'a> {
        binding: &'a str,
        artifact: &'a str,
        run: &'a crate::schema::Run,
    }
    let bytes = serde_json::to_vec(&Identity {
        binding: &input.id,
        artifact: artifact_key,
        run: &mcp.run,
    })
    .map_err(|_| "cannot fingerprint MCP launch specification".to_owned())?;
    Ok(util::sha256_bytes(&bytes))
}

fn write_launch_spec(install_paths: &Paths, id: &str, spec: &LaunchSpec) -> InstallResult<()> {
    let directory = install_paths.state.join("launch-specs");
    util::private_dir(&directory)?;
    let path = directory.join(format!("{id}.json"));
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err("MCP launch specification path is not a regular file".into());
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(_) => return Err("cannot inspect MCP launch specification".into()),
    }
    let bytes = serde_json::to_vec(spec)
        .map_err(|_| "cannot serialize MCP launch specification".to_owned())?;
    if util::read_file_checked(&directory, &path).ok().as_deref() == Some(bytes.as_slice()) {
        return Ok(());
    }
    util::atomic_write(&path, &bytes, 0o600)
}

fn ensure_self_launcher(install_paths: &Paths) -> InstallResult<bool> {
    let current = std::env::current_exe()
        .map_err(|_| "cannot locate the running Yashik executable".to_owned())?;
    let source_bytes =
        fs::read(&current).map_err(|_| "cannot read the running Yashik executable".to_owned())?;
    let target = install_paths.bin.join("yashik");
    let expected = match fs::symlink_metadata(&target) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err("Yashik launcher target is not a regular file".into());
        }
        Ok(_) => {
            let existing = util::read_file_checked(&install_paths.bin, &target)?;
            if existing == source_bytes {
                return Ok(true);
            }
            if !matches!(prompt("Replace the existing ~/.local/bin/yashik launcher with this Yashik version? [y/N]: ").as_deref(), Some("y" | "Y" | "yes" | "Yes")) {
                return Ok(false);
            }
            super::runtime::backup_cli_launcher(install_paths, "yashik", &existing)?;
            Some(util::sha256_bytes(&existing))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(_) => return Err("cannot inspect Yashik launcher target".into()),
    };
    util::atomic_write_if_unchanged(&target, &source_bytes, 0o755, expected.as_deref())?;
    Ok(true)
}

// Centralizing durable outcome commits keeps journal/state/report ordering consistent.
#[allow(clippy::too_many_arguments)]
pub(crate) fn finish_operation(
    install_paths: &Paths,
    state: &mut State,
    report: &mut RunReport,
    id: &str,
    binding_id: Option<String>,
    outcome: Outcome,
    message: Option<String>,
    change_state: impl FnOnce(&mut State),
) -> InstallResult<()> {
    let record = OperationRecord::new(
        id,
        binding_id,
        OperationPhase::Completed,
        outcome,
        message.clone(),
    );
    let mut next = state.clone();
    change_state(&mut next);
    next.operations.insert(id.to_owned(), record.clone());
    state::journal(install_paths, &record)?;
    state::save(install_paths, &next)?;
    *state = next;
    report.push(id, outcome, message);
    Ok(())
}

pub(crate) fn complete_without_mutation(
    install_paths: &Paths,
    state: &mut State,
    report: &mut RunReport,
    id: &str,
    binding_id: Option<String>,
    outcome: Outcome,
    message: Option<String>,
) -> InstallResult<()> {
    finish_operation(
        install_paths,
        state,
        report,
        id,
        binding_id,
        outcome,
        message,
        |_| {},
    )
}

pub(crate) fn begin_operation(
    install_paths: &Paths,
    state: &mut State,
    id: &str,
    binding_id: Option<String>,
) -> InstallResult<()> {
    let record = OperationRecord::new(
        id,
        binding_id,
        OperationPhase::Intent,
        Outcome::Interrupted,
        Some("operation intent recorded before mutation".into()),
    );
    let mut next = state.clone();
    next.operations.insert(id.to_owned(), record.clone());
    state::journal(install_paths, &record)?;
    state::save(install_paths, &next)?;
    *state = next;
    Ok(())
}

fn resource_id(harness: HarnessId, kind: ResourceKind, name: &str) -> String {
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

fn dedup_paths(paths: &mut Vec<PathBuf>) {
    let mut seen = BTreeSet::new();
    paths.retain(|path| seen.insert(path.clone()));
}

fn prompt(question: &str) -> Option<String> {
    eprint!("{question}");
    let _ = io::stderr().flush();
    let mut answer = String::new();
    match io::stdin().read_line(&mut answer) {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(answer.trim().to_owned()),
    }
}

fn prompt_yes(question: &str) -> bool {
    matches!(prompt(question).as_deref(), Some("y" | "Y" | "yes" | "Yes"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::install::api::ResourceKind;
    use crate::schema::Source;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn skipped_native_rule_preserves_the_entire_portable_rules_block() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before Unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("yashik-rules-engine-{nonce}"));
        fs::create_dir_all(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let source_root = root.join("rules-source");
        fs::create_dir_all(&source_root).unwrap();
        fs::write(source_root.join("portable.md"), "new rule text\n").unwrap();

        let paths = Paths {
            home: root.join("home"),
            data: root.join("data/yashik"),
            cache: root.join("cache/yashik"),
            state: root.join("state/yashik"),
            bin: root.join("home/.local/bin"),
        };
        paths::create_private(&paths).unwrap();
        let old_target = paths.home.join(".codex/AGENTS.md");
        fs::create_dir_all(old_target.parent().unwrap()).unwrap();
        let old_bytes = b"managed old A+B rules block\n";
        fs::write(&old_target, old_bytes).unwrap();

        let mut state = State::empty();
        state.bindings.insert(
            "codex/rules-block".into(),
            ManagedBinding {
                id: "codex/rules-block".into(),
                harness: HarnessId::Codex,
                kind: ResourceKind::Rule,
                names: vec!["rule-a".into(), "rule-b".into()],
                target: old_target.clone(),
                selector: Some("rules-block".into()),
                fingerprint: "old-fingerprint".into(),
                desired_fingerprint: "old-desired-fingerprint".into(),
                artifact_keys: vec![],
            },
        );
        let cli = InstalledCli {
            harness: HarnessId::Codex,
            version: "0.159.3".into(),
            executable: root.join("codex"),
            integrity: None,
            runtime_bins: Vec::new(),
            launcher_fingerprint: None,
        };
        let source = Source::Local {
            path: source_root.to_string_lossy().into_owned(),
        };
        let input = |name: &str, format: ResourceFormat| ResourceInput {
            harness: HarnessId::Codex,
            kind: ResourceKind::Rule,
            name: name.into(),
            id: format!("codex/rule/{name}"),
            source: source.clone(),
            local_source_path: Some(source_root.clone()),
            from: Some(if name == "rule-a" {
                "portable.md".into()
            } else {
                "native.toml".into()
            }),
            format: Some(format),
            description: None,
            mcp: None,
        };
        let inputs = vec![
            input("rule-a", ResourceFormat::Portable),
            input("rule-b", ResourceFormat::Native),
        ];
        let mut report = RunReport::default();
        let mut source_cache = BTreeMap::new();
        process_rules(
            &paths,
            HarnessId::Codex,
            &inputs,
            &cli,
            &BTreeSet::new(),
            &mut state,
            &mut report,
            &mut source_cache,
        )
        .unwrap();

        assert_eq!(fs::read(&old_target).unwrap(), old_bytes);
        assert!(state.bindings.contains_key("codex/rules-block"));
        assert!(report
            .operations
            .iter()
            .any(|operation| operation.id == "codex/rule/rule-a"
                && operation.outcome == Outcome::Blocked));
        drop(state);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn artifact_cleanup_waits_for_every_binding_and_removes_its_launch_specs() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before Unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("yashik-artifact-gc-{nonce}"));
        fs::create_dir_all(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let paths = Paths {
            home: root.join("home"),
            data: root.join("data/yashik"),
            cache: root.join("cache/yashik"),
            state: root.join("state/yashik"),
            bin: root.join("home/.local/bin"),
        };
        paths::create_private(&paths).unwrap();
        let key = "a".repeat(64);
        let artifact_root = paths
            .data
            .join("artifacts")
            .join(util::sha256_bytes(key.as_bytes()));
        fs::create_dir_all(&artifact_root).unwrap();
        fs::write(artifact_root.join("server.js"), "console.log('ready')").unwrap();
        let launch_id = "b".repeat(64);
        write_launch_spec(
            &paths,
            &launch_id,
            &LaunchSpec {
                artifact_root: artifact_root.clone(),
                run: crate::schema::Run {
                    command: "node".into(),
                    args: vec!["server.js".into()],
                    env: BTreeMap::new(),
                },
                runtime_bins: Vec::new(),
            },
        )
        .unwrap();

        let mut state = State::empty();
        state.artifacts.insert(
            key.clone(),
            ArtifactRecord {
                key: key.clone(),
                source_key: "source-key".into(),
                root: artifact_root.clone(),
            },
        );
        state.bindings.insert(
            "codex/mcp/echo".into(),
            ManagedBinding {
                id: "codex/mcp/echo".into(),
                harness: HarnessId::Codex,
                kind: ResourceKind::Mcp,
                names: vec!["echo".into()],
                target: root.join("codex.toml"),
                selector: Some("echo".into()),
                fingerprint: "fingerprint".into(),
                desired_fingerprint: "desired".into(),
                artifact_keys: vec![key.clone()],
            },
        );
        let mut report = RunReport::default();
        collect_unreferenced_artifacts(&paths, &mut state, &mut report, std::slice::from_ref(&key))
            .unwrap();
        assert!(artifact_root.exists());
        assert!(state.artifacts.contains_key(&key));
        assert!(paths
            .state
            .join("launch-specs")
            .join(format!("{launch_id}.json"))
            .exists());

        state.bindings.clear();
        collect_unreferenced_artifacts(&paths, &mut state, &mut report, std::slice::from_ref(&key))
            .unwrap();
        assert!(!artifact_root.exists());
        assert!(!state.artifacts.contains_key(&key));
        assert!(!paths
            .state
            .join("launch-specs")
            .join(format!("{launch_id}.json"))
            .exists());
        let _ = fs::remove_dir_all(root);
    }
}
