//! Finishing a Skill install (#401).
//!
//! Split out of `routes/mod.rs` like `serve_routes`: the route table already
//! sits at this repository's god-file threshold. `setup` is the only route
//! whose purpose is to change the user's environment (PATH, CODE_INTEL_HOME,
//! shell profiles, agent skill directories) rather than to analyse a
//! repository, so it keeps its own seam.

/// `LocalWrite` covers the bin copy, the env file, shell profiles and skill
/// directories; `ProcessSpawn` is the in-process doctor probe's tool lookups.
/// Exit 1 only when an install step failed: missing external tools are
/// reported with install commands and do not fail setup.
pub(super) const SETUP: super::RawRoute = super::RawRoute {
    command: "setup",
    subcommand: None,
    argument_offset: 1,
    id: super::CompatibilityRoute::Setup,
    contract: super::CommandContract {
        stability: super::CommandStability::Public,
        controller: super::ControllerOwnership::ProviderAdmin,
        authority: super::CommandAuthority::Administrative,
        effects: &[
            super::CommandEffect::RepoRead,
            super::CommandEffect::LocalWrite,
            super::CommandEffect::ProcessSpawn,
        ],
        output_contract: super::OutputContract::Stdout {
            identities: &["code-intel-setup.v1", "text-format:setup-human.v1"],
        },
        exit_contract: super::ExitContract::Exact(&[0, 1, 65]),
        retirement_condition: "retain while the Skill bootstrap installs releases (#401)",
    },
};
