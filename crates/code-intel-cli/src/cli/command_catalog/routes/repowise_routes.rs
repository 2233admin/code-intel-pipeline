//! Detection-and-optional-install route for the `repowise` integration
//! hooks (#B in the Repowise utilization thread): split out of `routes.rs`
//! for the same god-file reason `edit_routes`/`run_routes` were.
//!
//! `repowise` is an optional external dependency this route shells out to
//! by absolute path (never located inside the scanned repo); it is never
//! required to build, test, or run this crate.

pub(super) const HOOKS: super::RawRoute = super::RawRoute {
    command: "repowise-hooks",
    subcommand: None,
    argument_offset: 1,
    id: super::CompatibilityRoute::RepowiseHooks,
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
            identities: &["text-format:repowise-hooks-human.v1"],
        },
        exit_contract: super::ExitContract::Exact(&[0, 64, 74]),
        retirement_condition: "retire if repowise integration is dropped from this pipeline",
    },
};
