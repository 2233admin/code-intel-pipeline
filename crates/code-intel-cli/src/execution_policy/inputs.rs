//! The parsed inputs an execution policy is compiled from.
//!
//! These are small value types with one job each: turn a CLI string into a
//! checked value, or name a requirement level. They are separated from
//! `ExecutionPolicy` itself because the policy is about *composition rules*
//! — what may narrow what — while these are about *vocabulary*.

/// How strictly providers are required.
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

    /// Narrow one requirement: optional stages switch off, required ones
    /// survive. The asymmetry is the whole point — see `ExecutionPolicy`'s
    /// composition rules.
    pub(crate) fn narrowed(self) -> Self {
        match self {
            Self::Optional => Self::Disabled,
            other => other,
        }
    }
}

/// The launcher stage switches that map cleanly onto provider requirements.
///
/// Deliberately not every `-Skip*` the PowerShell launcher accepts. The ones
/// left out need a Rust surface that does not exist yet, and shipping a flag
/// that silently does nothing would be worse than not shipping it:
///
/// - `-SkipSentruxCheck` / `-SkipSentruxGate` are sub-stage granularity; the
///   DAG has a single `provider.sentrux-adapt` node, so honouring them means
///   splitting that node or giving it options, not toggling a requirement.
/// - `-SkipOpenSpec` / `-AutoOpenSpec` and `-SkipRepomix` / `-RepomixStyle` /
///   `-RepomixCompress` gate stages with no Rust node at all.
/// - `-WorkspaceAdd` and `-AllowRepowiseShadowMutation` are options of the
///   repowise stage rather than switches over whether it runs.
///
/// See docs/ps1-exit/t2-launcher-classification.md §1.2.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SkipFlags {
    pub(crate) repowise: bool,
    pub(crate) sentrux: bool,
    pub(crate) require_understand_graph: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn profile_parsing_does_not_expose_compatibility_as_a_user_choice() {
        assert_eq!(RunProfile::parse("strict").unwrap(), RunProfile::Strict);
        assert_eq!(RunProfile::parse("offline").unwrap(), RunProfile::Offline);
        // Compatibility is selected by the dag-coordinate entry point, never
        // by a user-supplied --profile value.
        assert!(RunProfile::parse("compatibility").is_err());
    }

    #[test]
    fn narrowing_switches_off_optional_and_leaves_the_others_alone() {
        assert_eq!(
            ProviderRequirement::Optional.narrowed(),
            ProviderRequirement::Disabled
        );
        assert_eq!(
            ProviderRequirement::Required.narrowed(),
            ProviderRequirement::Required
        );
        assert_eq!(
            ProviderRequirement::Disabled.narrowed(),
            ProviderRequirement::Disabled
        );
    }

    #[test]
    fn working_tree_policy_round_trips_its_wire_vocabulary() {
        for value in ["head_only", "explicit_overlay"] {
            assert_eq!(WorkingTreePolicy::parse(value).unwrap().as_str(), value);
        }
        assert!(WorkingTreePolicy::parse("dirty").is_err());
    }
}
