pub(super) const ROUTE: super::CommandRoute = super::CommandRoute::Raw(super::RawRoute {
    // The packet is intentionally advisory: it makes existing claim evidence
    // readable and deterministic, but cannot confer merge authority or replace
    // required human/CI approval.
    command: "pr",
    subcommand: Some("evidence"),
    argument_offset: 1,
    id: super::CompatibilityRoute::PrEvidence,
    contract: super::CommandContract {
        stability: super::CommandStability::Public,
        controller: super::ControllerOwnership::WorkspaceAdvisory,
        authority: super::CommandAuthority::Advisory,
        effects: &[super::CommandEffect::RepoRead, super::CommandEffect::LocalWrite],
        output_contract: super::OutputContract::ArtifactFilesAndStdout {
            artifact_identities: &["code-intel-pr-evidence-packet.v1"],
            stdout_identities: &["code-intel-pr-evidence-packet.v1"],
        },
        exit_contract: super::ExitContract::Exact(&[0, 64, 65]),
        retirement_condition: "retire only after a verified committed-artifact adapter and CI projection preserve the advisory authority boundary",
    },
});
