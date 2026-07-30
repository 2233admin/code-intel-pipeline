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

/// How much work a run does, ported from the launcher's `-Mode`.
///
/// Deliberately a separate axis from [`RunProfile`]: the profile says how
/// strictly providers are *required*, the mode says how much is *attempted*.
/// A strict lite run is a meaningful combination, so the two must not be
/// flattened into one enum — see docs/ps1-exit/t2-launcher-classification.md §2.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum RunMode {
    /// Skip the optional structural stage; the fastest useful run.
    Lite,
    #[default]
    Normal,
    /// Normal plus the deeper understand sweep.
    Full,
}

impl RunMode {
    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        match value {
            "lite" => Ok(Self::Lite),
            "normal" => Ok(Self::Normal),
            "full" => Ok(Self::Full),
            _ => Err("--mode must be lite, normal, or full".into()),
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Lite => "lite",
            Self::Normal => "normal",
            Self::Full => "full",
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
    mode: RunMode,
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
            mode: RunMode::Normal,
            working_tree: WorkingTreePolicy::ExplicitOverlay,
            scopes: vec![".".into()],
            providers,
            effects: EffectPolicy::RegistryDeclared,
            tool_path_prefix: None,
        }
    }

    /// Compose the run mode onto the profile's requirement policy.
    ///
    /// Mode may only *narrow within what the profile already permits*. `lite`
    /// drops the structural stage when the profile left it optional, but a
    /// profile that marks it `Required` keeps it: otherwise `--mode lite`
    /// would be a way to silently disarm a strict run, which is the same
    /// hazard `with_doctor_overrides` already refuses. `Offline` stays
    /// disabled regardless — mode never re-enables.
    pub(crate) fn with_mode(mut self, mode: RunMode) -> Self {
        self.mode = mode;
        if mode == RunMode::Lite && self.providers.sentrux == ProviderRequirement::Optional {
            self.providers.sentrux = ProviderRequirement::Disabled;
        }
        self
    }

    pub(crate) fn mode(&self) -> RunMode {
        self.mode
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
            // The hospital must be able to tell "the structural stage was
            // never requested" from "it was requested and produced nothing".
            // Only the policy can say this, and the policy refuses to disable
            // a capability its profile marks Required — so a narrow scope can
            // never be claimed for a run that demanded the gate.
            "diagnosis.hospital" => json!({
                "structuralEvidenceInScope": self.capability_enabled("provider.sentrux-adapt")
            }),
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
    fn mode_is_a_separate_axis_and_leaves_the_profile_intact() {
        // Mode and profile are orthogonal: every combination has to stay
        // expressible, so composing a mode must not change which profile the
        // policy reports or what `normal`/`full` require.
        for mode in [RunMode::Lite, RunMode::Normal, RunMode::Full] {
            let policy = ExecutionPolicy::for_profile(RunProfile::Default).with_mode(mode);
            assert_eq!(policy.profile, RunProfile::Default);
            assert_eq!(policy.mode(), mode);
        }
        let normal = ExecutionPolicy::for_profile(RunProfile::Default).with_mode(RunMode::Normal);
        let full = ExecutionPolicy::for_profile(RunProfile::Default).with_mode(RunMode::Full);
        assert_eq!(normal.providers, full.providers);
        assert!(normal.capability_enabled("provider.sentrux-adapt"));
    }

    #[test]
    fn lite_drops_the_structural_stage_only_where_the_profile_left_it_optional() {
        let lite = ExecutionPolicy::for_profile(RunProfile::Default).with_mode(RunMode::Lite);
        assert!(!lite.capability_enabled("provider.sentrux-adapt"));
        // The graph stage is Required under Default and must survive lite.
        assert!(lite.capability_enabled("provider.graph-adapt"));
        assert!(lite.provider_diagnosis_enabled());
    }

    #[test]
    fn lite_cannot_disarm_a_strict_run_or_reenable_an_offline_one() {
        // The hazard this pins: if `--mode lite` could drop a Required
        // capability, it would be a compatibility flag that silently weakens
        // a strict gate — exactly what with_doctor_overrides already refuses.
        let strict = ExecutionPolicy::for_profile(RunProfile::Strict).with_mode(RunMode::Lite);
        assert!(strict.providers.sentrux.is_required());
        assert!(strict.capability_enabled("provider.sentrux-adapt"));

        let compatibility =
            ExecutionPolicy::for_profile(RunProfile::Compatibility).with_mode(RunMode::Lite);
        assert!(compatibility.providers.sentrux.is_required());

        // ...and mode never turns anything back on.
        for mode in [RunMode::Lite, RunMode::Normal, RunMode::Full] {
            let offline = ExecutionPolicy::for_profile(RunProfile::Offline).with_mode(mode);
            assert_eq!(offline.providers.sentrux, ProviderRequirement::Disabled);
            assert_eq!(offline.providers.graph, ProviderRequirement::Disabled);
            assert!(!offline.provider_diagnosis_enabled());
        }
    }

    #[test]
    fn mode_parsing_accepts_the_launchers_vocabulary_and_rejects_the_rest() {
        assert_eq!(RunMode::parse("lite").unwrap(), RunMode::Lite);
        assert_eq!(RunMode::parse("normal").unwrap(), RunMode::Normal);
        assert_eq!(RunMode::parse("full").unwrap(), RunMode::Full);
        assert_eq!(RunMode::default(), RunMode::Normal);
        assert!(RunMode::parse("deep").is_err());
        assert!(RunMode::parse("").is_err());
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
