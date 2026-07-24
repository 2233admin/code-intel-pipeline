use std::path::{Path, PathBuf};

use serde_json::{json, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RunProfile {
    Default,
    Strict,
    Offline,
    Compatibility,
}

impl RunProfile {
    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        match value {
            "default" => Ok(Self::Default),
            "strict" => Ok(Self::Strict),
            "offline" => Ok(Self::Offline),
            _ => Err("--profile must be default, strict, or offline".into()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WorkingTreePolicy {
    HeadOnly,
    ExplicitOverlay,
}

impl WorkingTreePolicy {
    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        match value {
            "head_only" => Ok(Self::HeadOnly),
            "explicit_overlay" => Ok(Self::ExplicitOverlay),
            _ => Err("--working-tree-policy must be head_only or explicit_overlay".into()),
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::HeadOnly => "head_only",
            Self::ExplicitOverlay => "explicit_overlay",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProviderRequirement {
    Required,
    Optional,
    Disabled,
}

impl ProviderRequirement {
    pub(crate) fn is_required(self) -> bool {
        matches!(self, Self::Required)
    }

    pub(crate) fn is_enabled(self) -> bool {
        !matches!(self, Self::Disabled)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProviderPolicy {
    pub(crate) repowise: ProviderRequirement,
    pub(crate) understand: ProviderRequirement,
    graph: ProviderRequirement,
    sentrux: ProviderRequirement,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EffectPolicy {
    RegistryDeclared,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ExecutionPolicy {
    profile: RunProfile,
    working_tree: WorkingTreePolicy,
    scopes: Vec<String>,
    providers: ProviderPolicy,
    effects: EffectPolicy,
    tool_path_prefix: Option<PathBuf>,
}

impl ExecutionPolicy {
    pub(crate) fn for_profile(profile: RunProfile) -> Self {
        let providers = match profile {
            RunProfile::Default => ProviderPolicy {
                repowise: ProviderRequirement::Optional,
                understand: ProviderRequirement::Optional,
                graph: ProviderRequirement::Required,
                sentrux: ProviderRequirement::Optional,
            },
            RunProfile::Strict => ProviderPolicy {
                repowise: ProviderRequirement::Required,
                understand: ProviderRequirement::Required,
                graph: ProviderRequirement::Required,
                sentrux: ProviderRequirement::Required,
            },
            RunProfile::Compatibility => ProviderPolicy {
                repowise: ProviderRequirement::Required,
                understand: ProviderRequirement::Optional,
                graph: ProviderRequirement::Required,
                sentrux: ProviderRequirement::Required,
            },
            RunProfile::Offline => ProviderPolicy {
                repowise: ProviderRequirement::Disabled,
                understand: ProviderRequirement::Disabled,
                graph: ProviderRequirement::Disabled,
                sentrux: ProviderRequirement::Disabled,
            },
        };
        Self {
            profile,
            working_tree: WorkingTreePolicy::ExplicitOverlay,
            scopes: vec![".".into()],
            providers,
            effects: EffectPolicy::RegistryDeclared,
            tool_path_prefix: None,
        }
    }

    pub(crate) fn with_working_tree(
        mut self,
        working_tree: WorkingTreePolicy,
        scopes: Vec<String>,
    ) -> Self {
        self.working_tree = working_tree;
        self.scopes = if scopes.is_empty() {
            vec![".".into()]
        } else {
            scopes
        };
        self
    }

    pub(crate) fn with_doctor_overrides(
        mut self,
        require_repowise: Option<bool>,
        require_understand: Option<bool>,
        tool_path_prefix: Option<PathBuf>,
    ) -> Self {
        if matches!(
            self.profile,
            RunProfile::Default | RunProfile::Compatibility
        ) {
            if let Some(required) = require_repowise {
                self.providers.repowise = requirement_override(required);
            }
            if let Some(required) = require_understand {
                self.providers.understand = requirement_override(required);
            }
        }
        self.tool_path_prefix = tool_path_prefix;
        self
    }

    pub(crate) fn working_tree(&self) -> &'static str {
        self.working_tree.as_str()
    }

    pub(crate) fn scopes(&self) -> &[String] {
        &self.scopes
    }

    pub(crate) fn capability_requirement(&self, capability: &str) -> Option<ProviderRequirement> {
        match capability {
            "provider.graph-adapt" => Some(self.providers.graph),
            "provider.sentrux-adapt" => Some(self.providers.sentrux),
            _ => None,
        }
    }

    pub(crate) fn capability_enabled(&self, capability: &str) -> bool {
        self.capability_requirement(capability)
            .is_none_or(ProviderRequirement::is_enabled)
    }

    pub(crate) fn provider_diagnosis_enabled(&self) -> bool {
        self.providers.graph.is_enabled() || self.providers.sentrux.is_enabled()
    }

    pub(crate) fn capability_options(
        &self,
        capability: &str,
        repo: &Path,
        manifest: &Path,
    ) -> Value {
        let mut options = match capability {
            "diagnosis.hospital" => json!({}),
            "doctor" => json!({
                "repoPath":repo,
                "manifestPath":manifest,
                "requireRepowise":self.providers.repowise.is_required(),
                "requireUnderstand":self.providers.understand.is_required(),
            }),
            _ => json!({"repoPath":repo}),
        };
        if matches!(capability, "doctor" | "provider.sentrux-adapt") {
            if let Some(prefix) = &self.tool_path_prefix {
                options["toolPathPrefix"] = json!(prefix);
            }
        }
        options
    }

    pub(crate) fn allowed_effects(&self, declaration: &Value) -> Value {
        match self.effects {
            EffectPolicy::RegistryDeclared => declaration["allowedEffects"].clone(),
        }
    }
}

fn requirement_override(required: bool) -> ProviderRequirement {
    if required {
        ProviderRequirement::Required
    } else {
        ProviderRequirement::Optional
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_compile_to_one_immutable_provider_policy() {
        let default = ExecutionPolicy::for_profile(RunProfile::Default);
        let strict = ExecutionPolicy::for_profile(RunProfile::Strict);
        let offline = ExecutionPolicy::for_profile(RunProfile::Offline);

        assert!(!default.providers.repowise.is_required());
        assert!(default.providers.graph.is_required());
        assert!(!default.providers.sentrux.is_required());
        assert!(strict.providers.repowise.is_required());
        assert!(strict.providers.understand.is_required());
        assert!(strict.providers.graph.is_required());
        assert!(strict.providers.sentrux.is_required());
        assert_eq!(offline.providers.repowise, ProviderRequirement::Disabled);
        assert_eq!(offline.providers.understand, ProviderRequirement::Disabled);
        assert!(!offline.capability_enabled("provider.graph-adapt"));
        assert!(!offline.capability_enabled("provider.sentrux-adapt"));
        assert!(!offline.provider_diagnosis_enabled());
    }

    #[test]
    fn strict_and_offline_profiles_cannot_be_weakened_or_reenabled_by_compatibility_flags() {
        let strict = ExecutionPolicy::for_profile(RunProfile::Strict).with_doctor_overrides(
            Some(false),
            Some(false),
            None,
        );
        assert!(strict.providers.repowise.is_required());
        assert!(strict.providers.understand.is_required());

        let offline = ExecutionPolicy::for_profile(RunProfile::Offline).with_doctor_overrides(
            Some(true),
            Some(true),
            None,
        );
        assert_eq!(offline.providers.repowise, ProviderRequirement::Disabled);
        assert_eq!(offline.providers.understand, ProviderRequirement::Disabled);
    }
}
