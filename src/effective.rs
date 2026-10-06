use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::schema::{
    Agent, Harness, HarnessId, HerdrTool, LocalEntry, Manifest, Mcp, OrcaTool, Rule, Skill, Source,
    StrictMap,
};
use crate::validation::{validate_manifest, ValidationIssue};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectiveResource<T> {
    pub spec: T,
    pub local_source_path: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectiveHarness {
    pub id: HarnessId,
    pub version: Option<String>,
    pub mcp: BTreeMap<String, EffectiveResource<Mcp>>,
    pub skills: BTreeMap<String, EffectiveResource<Skill>>,
    pub agents: BTreeMap<String, EffectiveResource<Agent>>,
    pub rules: BTreeMap<String, EffectiveResource<Rule>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectiveManifest {
    pub harnesses: BTreeMap<HarnessId, EffectiveHarness>,
    pub herdr: Option<EffectiveHerdr>,
    pub orca: Option<EffectiveOrca>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectiveHerdr {
    pub version: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectiveOrca {
    pub version: String,
}

pub fn build_effective(
    manifest: &Manifest,
    manifest_dir: &Path,
) -> Result<EffectiveManifest, Vec<ValidationIssue>> {
    validate_manifest(manifest)?;
    let mut harnesses = BTreeMap::new();
    for (id, harness) in &manifest.harnesses {
        if !harness.enabled {
            continue;
        }
        harnesses.insert(*id, build_harness(*id, harness, manifest, manifest_dir));
    }
    let herdr = manifest.tools.herdr.as_ref().and_then(effective_herdr);
    let orca = manifest.tools.orca.as_ref().and_then(effective_orca);
    Ok(EffectiveManifest {
        harnesses,
        herdr,
        orca,
    })
}

fn effective_herdr(tool: &HerdrTool) -> Option<EffectiveHerdr> {
    tool.enabled.then(|| EffectiveHerdr {
        version: tool.version.clone(),
    })
}

fn effective_orca(tool: &OrcaTool) -> Option<EffectiveOrca> {
    tool.enabled.then(|| EffectiveOrca {
        version: tool.version.clone(),
    })
}

fn build_harness(
    id: HarnessId,
    harness: &Harness,
    manifest: &Manifest,
    manifest_dir: &Path,
) -> EffectiveHarness {
    EffectiveHarness {
        id,
        version: harness.version.clone(),
        mcp: effective_map(&manifest.mcp, &harness.mcp, manifest_dir),
        skills: effective_map(&manifest.skills, &harness.skills, manifest_dir),
        agents: effective_map(&manifest.agents, &harness.agents, manifest_dir),
        rules: effective_map(&manifest.rules, &harness.rules, manifest_dir),
    }
}

fn effective_map<T: Clone + HasSource>(
    common: &StrictMap<String, T>,
    local: &StrictMap<String, LocalEntry<T>>,
    manifest_dir: &Path,
) -> BTreeMap<String, EffectiveResource<T>> {
    let mut effective = common
        .iter()
        .map(|(name, spec)| (name.clone(), resource(spec.clone(), manifest_dir)))
        .collect::<BTreeMap<_, _>>();
    for (name, entry) in local {
        match entry {
            LocalEntry::Disabled(_) => {
                effective.remove(name);
            }
            LocalEntry::Spec(spec) => {
                effective.insert(name.clone(), resource(spec.clone(), manifest_dir));
            }
        }
    }
    effective
}

fn resource<T: HasSource>(spec: T, manifest_dir: &Path) -> EffectiveResource<T> {
    let local_source_path = match spec.source() {
        Source::Local { path } => {
            let source = Path::new(path);
            Some(if source.is_absolute() {
                source.to_path_buf()
            } else {
                manifest_dir.join(source)
            })
        }
        Source::Git { .. } => None,
    };
    EffectiveResource {
        spec,
        local_source_path,
    }
}

pub trait HasSource {
    fn source(&self) -> &Source;
}

impl HasSource for Mcp {
    fn source(&self) -> &Source {
        &self.source
    }
}

impl HasSource for Skill {
    fn source(&self) -> &Source {
        &self.source
    }
}

impl HasSource for Agent {
    fn source(&self) -> &Source {
        &self.source
    }
}

impl HasSource for Rule {
    fn source(&self) -> &Source {
        &self.source
    }
}
