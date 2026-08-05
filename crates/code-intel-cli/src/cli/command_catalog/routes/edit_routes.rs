//! The write half of the edit surface (#96, charter gate G4 in #139).
//!
//! Split out of `routes.rs` for the same reason `run_routes` was: the route
//! table is the one file every new command touches, and it had grown past the
//! repository's own god-file threshold (loc > 800). The seam is the authority
//! boundary — these are the only agent-facing routes that declare
//! `repo_mutation` — not an arbitrary line-count cut.

/// `edit apply` is a raw route rather than a controller because it owns no
/// authority of its own: the write goes through the `edit.span-apply`
/// capability envelope, which is what declares `repo_mutation` and what
/// refuses on a digest mismatch. Exit 10 is a *refusal*, not a malfunction —
/// the envelope's domain-fail code, reached when the bytes at the addressed
/// span are not the bytes the caller hashed. The exits reached before the
/// envelope exists (64/65/69/74) answer with `code-intel-edit-failure.v1`, so
/// no declared exit is silent.
pub(super) const APPLY: super::RawRoute = super::RawRoute {
    command: "edit",
    subcommand: Some("apply"),
    argument_offset: 2,
    id: super::CompatibilityRoute::EditApply,
    contract: super::CommandContract {
        stability: super::CommandStability::Public,
        controller: super::ControllerOwnership::Internal,
        authority: super::CommandAuthority::Internal,
        effects: &[
            super::CommandEffect::RepoRead,
            super::CommandEffect::LocalWrite,
            super::CommandEffect::RepoMutation,
        ],
        output_contract: super::OutputContract::Stdout {
            identities: &[
                "code-intel-edit-apply.v1",
                "code-intel-edit-failure.v1",
                "code-intel-capability-result.v1",
                "code-intel-span-edit-result.v1",
            ],
        },
        exit_contract: super::ExitContract::Exact(&[0, 10, 64, 65, 69, 70, 74]),
        retirement_condition: "retire only with the span-addressed edit capability it fronts",
    },
};
