//! Routes for `code-intel friction <log|list|publish|sync>`: this crate's
//! port of wevm/frog's friction-logging concept. Split into its own file for
//! the same god-file reason `edit_routes`/`repowise_routes` were.

pub(super) const LOG: super::RawRoute = super::RawRoute {
    command: "friction",
    subcommand: Some("log"),
    argument_offset: 2,
    id: super::CompatibilityRoute::FrictionLog,
    contract: super::CommandContract {
        stability: super::CommandStability::Public,
        controller: super::ControllerOwnership::WorkspaceAdvisory,
        authority: super::CommandAuthority::Advisory,
        effects: &[super::CommandEffect::LocalWrite],
        output_contract: super::OutputContract::Stdout {
            identities: &["text-format:friction-log-human.v1"],
        },
        exit_contract: super::ExitContract::Exact(&[0, 64, 74]),
        retirement_condition: "retire if friction-log workflow is dropped from this pipeline",
    },
};

pub(super) const LIST: super::RawRoute = super::RawRoute {
    command: "friction",
    subcommand: Some("list"),
    argument_offset: 2,
    id: super::CompatibilityRoute::FrictionList,
    contract: super::CommandContract {
        stability: super::CommandStability::Public,
        controller: super::ControllerOwnership::WorkspaceAdvisory,
        authority: super::CommandAuthority::Advisory,
        effects: &[super::CommandEffect::RepoRead],
        output_contract: super::OutputContract::Stdout {
            identities: &[
                "text-format:friction-list-human.v1",
                "code-friction-list.v1",
            ],
        },
        exit_contract: super::ExitContract::Exact(&[0, 64, 65, 74]),
        retirement_condition: "retire if friction-log workflow is dropped from this pipeline",
    },
};

pub(super) const PUBLISH: super::RawRoute = super::RawRoute {
    command: "friction",
    subcommand: Some("publish"),
    argument_offset: 2,
    id: super::CompatibilityRoute::FrictionPublish,
    contract: super::CommandContract {
        stability: super::CommandStability::Public,
        controller: super::ControllerOwnership::WorkspaceAdvisory,
        authority: super::CommandAuthority::Administrative,
        effects: &[
            super::CommandEffect::RepoRead,
            super::CommandEffect::LocalWrite,
            super::CommandEffect::ProcessSpawn,
            super::CommandEffect::Network,
        ],
        output_contract: super::OutputContract::Stdout {
            identities: &["text-format:friction-publish-human.v1"],
        },
        exit_contract: super::ExitContract::Exact(&[0, 64, 65, 74]),
        retirement_condition: "retire if friction-log workflow is dropped from this pipeline",
    },
};

pub(super) const SYNC: super::RawRoute = super::RawRoute {
    command: "friction",
    subcommand: Some("sync"),
    argument_offset: 2,
    id: super::CompatibilityRoute::FrictionSync,
    contract: super::CommandContract {
        stability: super::CommandStability::Public,
        controller: super::ControllerOwnership::WorkspaceAdvisory,
        authority: super::CommandAuthority::Administrative,
        effects: &[
            super::CommandEffect::RepoRead,
            super::CommandEffect::LocalWrite,
            super::CommandEffect::ProcessSpawn,
            super::CommandEffect::Network,
        ],
        output_contract: super::OutputContract::Stdout {
            identities: &["text-format:friction-sync-human.v1"],
        },
        exit_contract: super::ExitContract::Exact(&[0, 64, 65, 74]),
        retirement_condition: "retire if friction-log workflow is dropped from this pipeline",
    },
};
