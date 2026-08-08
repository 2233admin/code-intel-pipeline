//! Provider-internal routes that produce local artifact files.
//!
//! Kept beside the provider implementations rather than in the central route
//! table: these commands share the same internal authority and typed-capability
//! retirement boundary, and the table is already a governed god-file boundary.

pub(super) const RUNTIME_CI_EVIDENCE: super::RawRoute = super::RawRoute {
    command: "provider",
    subcommand: Some("runtime-ci-evidence"),
    argument_offset: 2,
    id: super::CompatibilityRoute::ProviderRuntimeCiEvidence,
    contract: super::CommandContract {
        stability: super::CommandStability::Internal,
        controller: super::ControllerOwnership::Internal,
        authority: super::CommandAuthority::Internal,
        effects: &[
            super::CommandEffect::RepoRead,
            super::CommandEffect::LocalWrite,
        ],
        output_contract: super::OutputContract::ArtifactFiles {
            identities: &["code-intel-runtime-ci-summary.v1"],
        },
        exit_contract: super::ExitContract::Exact(&[0, 65]),
        retirement_condition:
            "retire when all runtime evidence consumers use the typed capability seam",
    },
};

pub(super) const QUALITY_OBSERVATION: super::RawRoute = super::RawRoute {
    command: "provider",
    subcommand: Some("quality-observation"),
    argument_offset: 2,
    id: super::CompatibilityRoute::ProviderQualityObservation,
    contract: super::CommandContract {
        stability: super::CommandStability::Internal,
        controller: super::ControllerOwnership::Internal,
        authority: super::CommandAuthority::Internal,
        effects: &[
            super::CommandEffect::RepoRead,
            super::CommandEffect::LocalWrite,
        ],
        output_contract: super::OutputContract::ArtifactFiles {
            identities: &["code-intel-runtime-ci-observation.v1"],
        },
        exit_contract: super::ExitContract::Exact(&[0, 65]),
        retirement_condition:
            "retire when quality observation producers use the typed capability seam",
    },
};
