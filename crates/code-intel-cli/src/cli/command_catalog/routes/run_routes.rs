pub(super) const EXECUTE: super::RawRoute = super::RawRoute {
    command: "run",
    subcommand: Some("execute"),
    argument_offset: 1,
    id: super::CompatibilityRoute::RunExecute,
    contract: super::CommandContract {
        stability: super::CommandStability::Public,
        controller: super::ControllerOwnership::AuthoritativeRun,
        authority: super::CommandAuthority::Committed,
        effects: &[
            super::CommandEffect::RepoRead,
            super::CommandEffect::LocalWrite,
            super::CommandEffect::ProcessSpawn,
            super::CommandEffect::Network,
        ],
        output_contract: super::OutputContract::ArtifactFilesAndStdout {
            artifact_identities: &[
                "code-intel-run-manifest.v1",
                "code-intel-artifact-index.v1",
            ],
            stdout_identities: &[
                "code-intel-execution-result.v1",
                "code-intel-execution-failure.v1",
            ],
        },
        // 73 is reserved for publication-name collisions; budget truncation
        // uses 76 so callers can distinguish the two terminal conditions.
        exit_contract: super::ExitContract::Exact(&[0, 10, 20, 64, 65, 70, 73, 74, 76]),
        retirement_condition:
            "retain as the stable explicit production spelling; retire only by versioned replacement",
    },
};

pub(super) const DAG_COORDINATE: super::RawRoute = super::RawRoute {
    command: "run",
    subcommand: Some("dag-coordinate"),
    argument_offset: 1,
    id: super::CompatibilityRoute::RunDagCoordinate,
    contract: super::CommandContract {
        stability: super::CommandStability::Compatibility,
        controller: super::ControllerOwnership::Internal,
        authority: super::CommandAuthority::Internal,
        effects: &[
            super::CommandEffect::RepoRead,
            super::CommandEffect::LocalWrite,
            super::CommandEffect::ProcessSpawn,
            super::CommandEffect::Network,
        ],
        output_contract: super::OutputContract::ArtifactFilesAndStdout {
            artifact_identities: &["code-intel-run-manifest.v1"],
            stdout_identities: &["code-intel-run-manifest.v1"],
        },
        exit_contract: super::ExitContract::Exact(&[0, 10, 20, 64, 65, 70, 74, 76]),
        retirement_condition:
            "retire after every DAG integration invokes the typed kernel without this staging primitive",
    },
};
