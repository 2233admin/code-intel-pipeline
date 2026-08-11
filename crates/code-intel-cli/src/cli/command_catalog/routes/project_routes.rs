const PRIMARY_EFFECTS: &[super::CommandEffect] = &[
    super::CommandEffect::RepoRead,
    super::CommandEffect::LocalWrite,
    super::CommandEffect::ProcessSpawn,
    super::CommandEffect::Network,
];

const PRIMARY_OUTPUT: super::OutputContract = super::OutputContract::ArtifactFilesAndStdout {
    artifact_identities: &["code-intel-run-manifest.v1", "code-intel-artifact-index.v1"],
    stdout_identities: &[
        "code-intel-primary-result.v1",
        "text-format:primary-summary.v1",
    ],
};

const PRIMARY_EXITS: super::ExitContract = super::ExitContract::Exact(&[0, 10, 20, 64, 65, 70, 74]);

pub(super) const RUN_ALIAS: super::CommandRoute =
    super::CommandRoute::RunAlias(super::CommandContract {
        stability: super::CommandStability::Public,
        controller: super::ControllerOwnership::AuthoritativeRun,
        authority: super::CommandAuthority::Committed,
        effects: PRIMARY_EFFECTS,
        output_contract: PRIMARY_OUTPUT,
        exit_contract: PRIMARY_EXITS,
        retirement_condition: "retain as the named alias of the default production entry",
    });

pub(super) const QUERY: super::CommandRoute =
    super::CommandRoute::ProjectQuery(super::CommandContract {
        stability: super::CommandStability::Public,
        controller: super::ControllerOwnership::CommittedEvidence,
        authority: super::CommandAuthority::Committed,
        effects: &[
            super::CommandEffect::RepoRead,
            super::CommandEffect::ProcessSpawn,
        ],
        output_contract: super::OutputContract::Stdout {
            identities: &[
                "code-intel-evidence-query.v1",
                "code-intel-project-error.v1",
            ],
        },
        exit_contract: super::ExitContract::Exact(&[0, 64, 65, 74]),
        retirement_condition: "retain as the project-context committed-evidence entry",
    });

pub(super) const PRIMARY: super::CommandRoute =
    super::CommandRoute::Primary(super::CommandContract {
        stability: super::CommandStability::Public,
        controller: super::ControllerOwnership::AuthoritativeRun,
        authority: super::CommandAuthority::Committed,
        effects: PRIMARY_EFFECTS,
        output_contract: PRIMARY_OUTPUT,
        exit_contract: PRIMARY_EXITS,
        retirement_condition:
            "retain as the default production entry; retire only by versioned replacement",
    });
