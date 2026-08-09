//! Provider-internal routes that produce local artifact files.
//!
//! Keeping them outside the central command table preserves its governed size
//! boundary while making the provider input/output contracts explicit.

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

pub(super) const DIAPHORA_INSPECT: super::RawRoute = super::RawRoute {
    command: "provider",
    subcommand: Some("diaphora-inspect"),
    argument_offset: 2,
    id: super::CompatibilityRoute::ProviderDiaphoraInspect,
    contract: super::CommandContract {
        stability: super::CommandStability::Internal,
        controller: super::ControllerOwnership::ProviderAdmin,
        authority: super::CommandAuthority::Administrative,
        effects: &[
            super::CommandEffect::RepoRead,
            super::CommandEffect::LocalWrite,
        ],
        output_contract: super::OutputContract::ArtifactFilesAndStdout {
            artifact_identities: &["code-intel-diaphora-observation.v1"],
            stdout_identities: &["code-intel-diaphora-observation.v1"],
        },
        exit_contract: super::ExitContract::Exact(&[0, 64, 65, 69]),
        retirement_condition:
            "retain while external Diaphora evidence needs a local, read-only import boundary",
    },
};
