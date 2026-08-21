use std::path::{Path, PathBuf};

use serde_json::{json, Value};

mod inputs;

pub(crate) use inputs::{ProviderRequirement, RunMode, RunProfile, SkipFlags, WorkingTreePolicy};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProviderPolicy {
    pub(crate) repowise: ProviderRequirement,
    pub(crate) understand: ProviderRequirement,
    graph: ProviderRequirement,
    sentrux: ProviderRequirement,
    codenexus: ProviderRequirement,
    open_spec: ProviderRequirement,
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
    open_spec_auto: bool,
}

impl ExecutionPolicy {
    pub(crate) fn for_profile(profile: RunProfile) -> Self {
        let providers = match profile {
            RunProfile::Default => ProviderPolicy {
                repowise: ProviderRequirement::Optional,
                understand: ProviderRequirement::Optional,
                graph: ProviderRequirement::Required,
                sentrux: ProviderRequirement::Optional,
                codenexus: ProviderRequirement::Optional,
                open_spec: ProviderRequirement::Optional,
            },
            RunProfile::Strict => ProviderPolicy {
                repowise: ProviderRequirement::Required,
                understand: ProviderRequirement::Required,
                graph: ProviderRequirement::Required,
                sentrux: ProviderRequirement::Required,
                codenexus: ProviderRequirement::Required,
                open_spec: ProviderRequirement::Required,
            },
            RunProfile::Compatibility => ProviderPolicy {
                repowise: ProviderRequirement::Required,
                understand: ProviderRequirement::Optional,
                graph: ProviderRequirement::Required,
                sentrux: ProviderRequirement::Required,
                codenexus: ProviderRequirement::Required,
                open_spec: ProviderRequirement::Required,
            },
            RunProfile::Offline => ProviderPolicy {
                repowise: ProviderRequirement::Disabled,
                understand: ProviderRequirement::Disabled,
                graph: ProviderRequirement::Disabled,
                sentrux: ProviderRequirement::Disabled,
                codenexus: ProviderRequirement::Disabled,
                open_spec: ProviderRequirement::Disabled,
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
            open_spec_auto: false,
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
        if mode == RunMode::Lite {
            self.providers.sentrux = self.providers.sentrux.narrowed();
        }
        self
    }

    pub(crate) fn mode(&self) -> RunMode {
        self.mode
    }

    /// Compose the launcher's stage switches onto the requirement policy.
    ///
    /// Same rule as [`Self::with_mode`], for the same reason: a skip may only
    /// narrow within what the profile already permits. `Required` survives
    /// every skip, so no compatibility switch can disarm a strict gate, and
    /// `Disabled` is never turned back on, so `Offline` stays offline.
    ///
    /// `require_understand_graph` moves in the other direction — it
    /// *strengthens* — which is always safe, but it still may not resurrect a
    /// capability the profile disabled.
    pub(crate) fn with_skips(mut self, skips: SkipFlags) -> Self {
        if skips.repowise {
            self.providers.repowise = self.providers.repowise.narrowed();
        }
        if skips.sentrux {
            self.providers.sentrux = self.providers.sentrux.narrowed();
        }
        if skips.open_spec {
            self.providers.open_spec = self.providers.open_spec.narrowed();
        }
        if skips.require_understand_graph
            && self.providers.understand == ProviderRequirement::Optional
        {
            self.providers.understand = ProviderRequirement::Required;
        }
        self
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

    /// Compose the launcher's `-AutoOpenSpec` passthrough option.
    ///
    /// Unlike a skip, this never changes whether `advisory.workflow-recommend`
    /// runs — only whether its recommendation is auto-applied. It therefore
    /// composes independently of the requirement-narrowing rules above.
    pub(crate) fn with_open_spec_auto(mut self, auto: bool) -> Self {
        self.open_spec_auto = auto;
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
            "provider.codenexus-adapt" => Some(self.providers.codenexus),
            "advisory.workflow-recommend" => Some(self.providers.open_spec),
            _ => None,
        }
    }

    pub(crate) fn capability_enabled(&self, capability: &str) -> bool {
        self.capability_requirement(capability)
            .is_none_or(ProviderRequirement::is_enabled)
    }

    pub(crate) fn provider_diagnosis_enabled(&self) -> bool {
        self.providers.graph.is_enabled()
            || self.providers.sentrux.is_enabled()
            || self.providers.codenexus.is_enabled()
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
            "advisory.workflow-recommend" => json!({
                "repoPath":repo,
                "auto":self.open_spec_auto,
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
        assert!(!default.providers.open_spec.is_required());
        assert!(strict.providers.repowise.is_required());
        assert!(strict.providers.understand.is_required());
        assert!(strict.providers.graph.is_required());
        assert!(strict.providers.sentrux.is_required());
        assert!(strict.providers.open_spec.is_required());
        assert_eq!(offline.providers.repowise, ProviderRequirement::Disabled);
        assert_eq!(offline.providers.understand, ProviderRequirement::Disabled);
        assert_eq!(offline.providers.open_spec, ProviderRequirement::Disabled);
        assert!(!offline.capability_enabled("provider.graph-adapt"));
        assert!(!offline.capability_enabled("provider.sentrux-adapt"));
        assert!(!offline.capability_enabled("advisory.workflow-recommend"));
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
    fn skips_drop_optional_stages_and_narrow_the_hospital_scope_with_them() {
        let policy = ExecutionPolicy::for_profile(RunProfile::Default).with_skips(SkipFlags {
            repowise: true,
            sentrux: true,
            ..SkipFlags::default()
        });
        assert_eq!(policy.providers.repowise, ProviderRequirement::Disabled);
        assert!(!policy.capability_enabled("provider.sentrux-adapt"));
        // Skipping the structural stage must tell the hospital the stage was
        // excluded, not that its evidence went missing.
        let options = policy.capability_options(
            "diagnosis.hospital",
            Path::new("repo"),
            Path::new("integrations.json"),
        );
        assert_eq!(options["structuralEvidenceInScope"], json!(false));
    }

    #[test]
    fn skips_cannot_disarm_a_strict_run_or_reenable_an_offline_one() {
        let all = SkipFlags {
            repowise: true,
            sentrux: true,
            open_spec: true,
            require_understand_graph: true,
        };
        let strict = ExecutionPolicy::for_profile(RunProfile::Strict).with_skips(all);
        assert!(strict.providers.repowise.is_required());
        assert!(strict.providers.sentrux.is_required());
        assert!(strict.capability_enabled("provider.sentrux-adapt"));
        assert!(strict.capability_enabled("advisory.workflow-recommend"));

        let offline = ExecutionPolicy::for_profile(RunProfile::Offline).with_skips(all);
        assert_eq!(offline.providers.repowise, ProviderRequirement::Disabled);
        assert_eq!(offline.providers.sentrux, ProviderRequirement::Disabled);
        assert_eq!(offline.providers.open_spec, ProviderRequirement::Disabled);
        // require_understand_graph strengthens, but must not resurrect a
        // capability the profile disabled.
        assert_eq!(offline.providers.understand, ProviderRequirement::Disabled);
    }

    #[test]
    fn skip_open_spec_narrows_the_default_profile_and_auto_is_independent() {
        let skipped = ExecutionPolicy::for_profile(RunProfile::Default).with_skips(SkipFlags {
            open_spec: true,
            ..SkipFlags::default()
        });
        assert!(!skipped.capability_enabled("advisory.workflow-recommend"));

        // `-AutoOpenSpec` is a passthrough option, not a requirement toggle:
        // it must reach the capability request even when the stage still runs,
        // and it must not itself enable/disable the stage.
        let auto = ExecutionPolicy::for_profile(RunProfile::Default).with_open_spec_auto(true);
        assert!(auto.capability_enabled("advisory.workflow-recommend"));
        let options = auto.capability_options(
            "advisory.workflow-recommend",
            Path::new("repo"),
            Path::new("integrations.json"),
        );
        assert_eq!(options["repoPath"], json!(Path::new("repo")));
        assert_eq!(options["auto"], json!(true));

        let no_auto = ExecutionPolicy::for_profile(RunProfile::Default);
        let options = no_auto.capability_options(
            "advisory.workflow-recommend",
            Path::new("repo"),
            Path::new("integrations.json"),
        );
        assert_eq!(options["auto"], json!(false));
    }

    #[test]
    fn require_understand_graph_promotes_an_optional_stage_to_required() {
        let policy = ExecutionPolicy::for_profile(RunProfile::Default).with_skips(SkipFlags {
            require_understand_graph: true,
            ..SkipFlags::default()
        });
        assert!(policy.providers.understand.is_required());
        // ...and it reaches the doctor, which is what makes a missing graph
        // fatal rather than advisory.
        let options =
            policy.capability_options("doctor", Path::new("repo"), Path::new("integrations.json"));
        assert_eq!(options["requireUnderstand"], json!(true));
    }

    #[test]
    fn no_skips_is_the_identity_on_the_provider_policy() {
        let plain = ExecutionPolicy::for_profile(RunProfile::Default);
        let skipped =
            ExecutionPolicy::for_profile(RunProfile::Default).with_skips(SkipFlags::default());
        assert_eq!(plain.providers, skipped.providers);
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
