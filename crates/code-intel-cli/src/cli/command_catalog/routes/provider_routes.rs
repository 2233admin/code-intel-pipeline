//! The internal `provider <x>-adapt` seams (typed adapter routing for
//! repowise, the architecture graph, structural evidence, sessions,
//! codenexus, file-boundary, and runtime CI evidence).
//!
//! Split out of `routes/mod.rs` for the same reason `edit_routes`,
//! `run_routes`, and `serve_routes` were: the route table is the one file
//! every new command touches, and it had grown past the repository's own
//! god-file threshold (loc > 800) again. The seam here is the authority
//! boundary these already shared before extraction — every route in this file
//! is `Internal`/`Internal` (never a caller-facing controller or authority)
//! and exists only so the DAG can route through a typed adapter instead of
//! argv — not an arbitrary line-count cut.

pub(super) const REPOWISE_ADAPT: super::RawRoute = super::RawRoute {
    command: "provider",
    subcommand: Some("repowise-adapt"),
    argument_offset: 2,
    id: super::CompatibilityRoute::ProviderRepowiseAdapt,
    contract: super::CommandContract {
        stability: super::CommandStability::Internal,
        controller: super::ControllerOwnership::Internal,
        authority: super::CommandAuthority::Internal,
        effects: &[super::CommandEffect::RepoRead],
        output_contract: super::OutputContract::Stdout {
            identities: &[
                "code-intel-repowise-route-result.v1",
                "code-intel-repowise-adapter-result.v1",
            ],
        },
        exit_contract: super::ExitContract::Exact(&[0, 64, 65]),
        retirement_condition:
            "retire when the DAG exclusively calls the typed repowise adapter seam",
    },
};

pub(super) const GRAPH_ADAPT: super::RawRoute = super::RawRoute {
    command: "provider",
    subcommand: Some("graph-adapt"),
    argument_offset: 2,
    id: super::CompatibilityRoute::ProviderGraphAdapt,
    contract: super::CommandContract {
        stability: super::CommandStability::Internal,
        controller: super::ControllerOwnership::Internal,
        authority: super::CommandAuthority::Internal,
        effects: &[super::CommandEffect::RepoRead],
        output_contract: super::OutputContract::Stdout {
            identities: &[
                "code-intel-graph-route-result.v1",
                "code-intel-graph-adapter-result.v1",
            ],
        },
        exit_contract: super::ExitContract::Exact(&[0, 64, 65]),
        retirement_condition: "retire when the DAG exclusively calls the typed graph adapter seam",
    },
};

pub(super) const SENTRUX_ADAPT: super::RawRoute = super::RawRoute {
    command: "provider",
    subcommand: Some("sentrux-adapt"),
    argument_offset: 2,
    id: super::CompatibilityRoute::ProviderSentruxAdapt,
    contract: super::CommandContract {
        stability: super::CommandStability::Internal,
        controller: super::ControllerOwnership::Internal,
        authority: super::CommandAuthority::Internal,
        effects: &[super::CommandEffect::RepoRead],
        output_contract: super::OutputContract::Stdout {
            identities: &[
                "code-intel-sentrux-route-result.v1",
                "code-intel-sentrux-adapter-result.v1",
            ],
        },
        exit_contract: super::ExitContract::Exact(&[0, 64, 65]),
        retirement_condition:
            "retire when the DAG exclusively calls the typed sentrux adapter seam",
    },
};

pub(super) const SESSION_ADAPT: super::RawRoute = super::RawRoute {
    command: "provider",
    subcommand: Some("session-adapt"),
    argument_offset: 2,
    id: super::CompatibilityRoute::ProviderSessionAdapt,
    contract: super::CommandContract {
        stability: super::CommandStability::Internal,
        controller: super::ControllerOwnership::AgentSession,
        authority: super::CommandAuthority::Internal,
        effects: &[
            super::CommandEffect::RepoRead,
            super::CommandEffect::LocalWrite,
        ],
        output_contract: super::OutputContract::StdoutWithOptionalArtifactFiles {
            artifact_identities: &["code-intel-session-evidence.v1"],
            stdout_identities: &[
                "code-intel-session-evidence.v1",
                "code-intel-session-adapter-result.v1",
            ],
        },
        exit_contract: super::ExitContract::Exact(&[0, 64, 65, 74]),
        retirement_condition: "retire when session evidence admission has no argv caller",
    },
};

pub(super) const CODENEXUS_ADAPT: super::RawRoute = super::RawRoute {
    command: "provider",
    subcommand: Some("codenexus-adapt"),
    argument_offset: 2,
    id: super::CompatibilityRoute::ProviderCodenexusAdapt,
    contract: super::CommandContract {
        stability: super::CommandStability::Internal,
        controller: super::ControllerOwnership::Internal,
        authority: super::CommandAuthority::Internal,
        effects: &[super::CommandEffect::RepoRead],
        output_contract: super::OutputContract::Stdout {
            identities: &[
                "code-intel-codenexus-route-result.v1",
                "code-intel-codenexus-adapter-result.v1",
            ],
        },
        exit_contract: super::ExitContract::Exact(&[0, 64, 65]),
        retirement_condition:
            "retire when the DAG exclusively calls the typed codenexus adapter seam",
    },
};

pub(super) const FILE_BOUNDARY: super::RawRoute = super::RawRoute {
    command: "provider",
    subcommand: Some("file-boundary"),
    argument_offset: 2,
    id: super::CompatibilityRoute::ProviderFileBoundary,
    contract: super::CommandContract {
        stability: super::CommandStability::Internal,
        controller: super::ControllerOwnership::Internal,
        authority: super::CommandAuthority::Internal,
        effects: &[
            super::CommandEffect::RepoRead,
            super::CommandEffect::LocalWrite,
        ],
        output_contract: super::OutputContract::ArtifactFiles {
            identities: &["code-intel-file-boundary-result.v1"],
        },
        exit_contract: super::ExitContract::Exact(&[0, 65]),
        retirement_condition:
            "retire when all file-boundary consumers use the typed capability seam",
    },
};

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
