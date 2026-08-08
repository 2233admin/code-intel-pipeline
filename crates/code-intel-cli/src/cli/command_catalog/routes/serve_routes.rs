//! The agent-native query surface (#54, #58 proposal 3).
//!
//! Split out of `routes.rs` for the reason `edit_routes` and `run_routes`
//! were: the route table is the file every new command touches, and it already
//! sits just under this repository's own god-file threshold. The seam here is
//! the transport — this is the only route that speaks a protocol rather than
//! argv-in / stdout-out, and the only one whose process lifetime is a client
//! session rather than a single answer.

/// `serve` takes no subcommand so the transport stays a flag: `--mcp` is the
/// only one today, and a future transport should be `serve --http`, not a
/// second route with a duplicated contract. The parser refuses when no
/// transport is named rather than defaulting to one, because "which protocol
/// is this process speaking on stdio" is not a question to guess at.
///
/// `LocalWrite` and `ProcessSpawn` are declared for the single tool that
/// executes anything — `plan_structural_edit` stages an ast-grep preview into
/// a temporary directory. `RepoMutation` is absent and the handler refuses if
/// the registry ever declares it, so the effect set here is the enforced
/// boundary, not a description of intent.
pub(super) const MCP: super::RawRoute = super::RawRoute {
    command: "serve",
    subcommand: None,
    argument_offset: 1,
    id: super::CompatibilityRoute::Serve,
    contract: super::CommandContract {
        stability: super::CommandStability::Public,
        controller: super::ControllerOwnership::AgentSession,
        authority: super::CommandAuthority::Conditional(
            super::AuthorityCondition::CommittedOrStaleAdvisory,
        ),
        effects: &[
            super::CommandEffect::RepoRead,
            super::CommandEffect::LocalWrite,
            super::CommandEffect::ProcessSpawn,
        ],
        output_contract: super::OutputContract::Stdout {
            identities: &[
                "text-format:mcp-jsonrpc-stream.v1",
                "code-intel-mcp-gate-verdict.v1",
                "code-intel-mcp-evidence-chain.v1",
                "code-intel-mcp-audit-status.v1",
                "code-intel-mcp-structural-edit-plan.v1",
                "code-intel-mcp-tool-error.v1",
                "code-intel-evidence-query.v1",
                "code-intel-change-impact.v1",
            ],
        },
        exit_contract: super::ExitContract::Exact(&[0, 64, 74]),
        retirement_condition:
            "retire only through a versioned agent query-surface replacement; the served payloads \
are projections and may be retired individually with their source contracts",
    },
};
