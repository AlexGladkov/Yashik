use std::collections::BTreeMap;

use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};

pub type StrictMap<K, V> = BTreeMap<K, V>;

pub fn deserialize_unique_map<'de, D, K, V>(deserializer: D) -> Result<StrictMap<K, V>, D::Error>
where
    D: Deserializer<'de>,
    K: Deserialize<'de> + Ord,
    V: Deserialize<'de>,
{
    struct UniqueMapVisitor<K, V>(std::marker::PhantomData<(K, V)>);

    impl<'de, K, V> Visitor<'de> for UniqueMapVisitor<K, V>
    where
        K: Deserialize<'de> + Ord,
        V: Deserialize<'de>,
    {
        type Value = StrictMap<K, V>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a mapping with unique keys")
        }

        fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
        where
            A: MapAccess<'de>,
        {
            let mut values = StrictMap::new();
            while let Some(key) = map.next_key::<K>()? {
                if values.contains_key(&key) {
                    return Err(de::Error::custom("duplicate mapping key"));
                }
                let value = map.next_value::<V>()?;
                values.insert(key, value);
            }
            Ok(values)
        }
    }

    deserializer.deserialize_map(UniqueMapVisitor(std::marker::PhantomData))
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub version: u32,
    #[serde(deserialize_with = "deserialize_unique_map")]
    pub harnesses: StrictMap<HarnessId, Harness>,
    #[serde(default, deserialize_with = "deserialize_unique_map")]
    pub mcp: StrictMap<String, Mcp>,
    #[serde(default, deserialize_with = "deserialize_unique_map")]
    pub skills: StrictMap<String, Skill>,
    #[serde(default, deserialize_with = "deserialize_unique_map")]
    pub agents: StrictMap<String, Agent>,
    #[serde(default, deserialize_with = "deserialize_unique_map")]
    pub rules: StrictMap<String, Rule>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum HarnessId {
    Codex,
    Claude,
    Opencode,
    Pi,
    Omp,
}

impl HarnessId {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::Opencode => "opencode",
            Self::Pi => "pi",
            Self::Omp => "omp",
        }
    }
}

impl PartialOrd for HarnessId {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for HarnessId {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.as_str().cmp(other.as_str())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Harness {
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub version: Option<String>,
    #[serde(default, deserialize_with = "deserialize_unique_map")]
    pub mcp: StrictMap<String, LocalEntry<Mcp>>,
    #[serde(default, deserialize_with = "deserialize_unique_map")]
    pub skills: StrictMap<String, LocalEntry<Skill>>,
    #[serde(default, deserialize_with = "deserialize_unique_map")]
    pub agents: StrictMap<String, LocalEntry<Agent>>,
    #[serde(default, deserialize_with = "deserialize_unique_map")]
    pub rules: StrictMap<String, LocalEntry<Rule>>,
}

const fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum LocalEntry<T> {
    Disabled(DisabledEntry),
    Spec(T),
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DisabledEntry {
    pub enabled: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum Source {
    Git {
        url: String,
        #[serde(rename = "ref", default)]
        git_ref: Option<String>,
    },
    Local {
        path: String,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Transport {
    Stdio,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Mcp {
    pub source: Source,
    pub transport: Transport,
    pub run: Run,
    pub from: Option<String>,
    pub install: Option<Install>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Install {
    #[serde(default)]
    pub requires: Vec<String>,
    #[serde(default)]
    pub steps: Vec<Vec<String>>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Run {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default, deserialize_with = "deserialize_unique_map")]
    pub env: StrictMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Skill {
    pub source: Source,
    pub from: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ResourceFormat {
    Portable,
    Native,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Agent {
    pub format: ResourceFormat,
    pub description: Option<String>,
    pub source: Source,
    pub from: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(tag = "format", rename_all = "lowercase", deny_unknown_fields)]
enum AgentWire {
    Portable {
        description: String,
        source: Source,
        from: Option<String>,
    },
    Native {
        source: Source,
        from: Option<String>,
    },
}

impl<'de> Deserialize<'de> for Agent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let (format, description, source, from) = match AgentWire::deserialize(deserializer)? {
            AgentWire::Portable {
                description,
                source,
                from,
            } => (ResourceFormat::Portable, Some(description), source, from),
            AgentWire::Native { source, from } => (ResourceFormat::Native, None, source, from),
        };
        Ok(Self {
            format,
            description,
            source,
            from,
        })
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub format: ResourceFormat,
    pub source: Source,
    pub from: Option<String>,
}
