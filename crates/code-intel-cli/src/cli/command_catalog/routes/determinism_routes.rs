//! `determinism check` (R1.1's three-run gate over `diagnosis.hospital`'s
//! evidence chain).
//!
//! Split out of `routes/mod.rs` for the same reason `edit_routes`,
//! `run_routes`, and `serve_routes` were: the route table is the one file
//! every new command touches, and adding this route pushed it past the
//! repository's own god-file threshold (loc > 800) again.

pub(super) const CHECK: super::RawRoute = super::RawRoute {
    command: "determinism",
    subcommand: Some("check"),
    argument_offset: 1,
    id: super::CompatibilityRoute::DeterminismCheck,
    contract: super::CommandContract {
        stability: super::CommandStability::Public,
        controller: super::ControllerOwnership::Governance,
        authority: super::CommandAuthority::Advisory,
        effects: &[
            super::CommandEffect::RepoRead,
            super::CommandEffect::LocalWrite,
            super::CommandEffect::ProcessSpawn,
        ],
        output_contract: super::OutputContract::Stdout {
            identities: &["code-intel-determinism-report.v1"],
        },
        exit_contract: super::ExitContract::Exact(&[0, 20, 64, 65, 70, 74]),
        retirement_condition: "retire only once every diagnosis.hospital producer carries its own \
             per-provider replay-stability test and this cross-run sweep is redundant",
    },
};
