use super::contract::{
    AuthorityCondition, CommandAuthority, CommandContract, CommandEffect, CommandStability,
    ControllerOwnership, ExitContract, OutputContract,
};
use super::{matches_primary_pattern, CompatibilityRoute, LegacyRouteId};
use crate::cli::help_contract::{HELP_ALIASES, HELP_COMMAND};

mod edit_routes;
mod repowise_routes;
mod run_routes;
mod serve_routes;
mod types;

pub(super) use types::{CommandRoute, LegacyRoute, RawRoute, VersionRoute};

macro_rules! command_contract {
    ($stability:ident, $controller:ident, Conditional($condition:ident), $effects:expr, $output:expr, $exit:expr, $retirement:literal) => {
        CommandContract {
            stability: CommandStability::$stability,
            controller: ControllerOwnership::$controller,
            authority: CommandAuthority::Conditional(AuthorityCondition::$condition),
            effects: $effects,
            output_contract: $output,
            exit_contract: $exit,
            retirement_condition: $retirement,
        }
    };
    ($stability:ident, $controller:ident, $authority:ident, $effects:expr, $output:expr, $exit:expr, $retirement:literal) => {
        CommandContract {
            stability: CommandStability::$stability,
            controller: ControllerOwnership::$controller,
            authority: CommandAuthority::$authority,
            effects: $effects,
            output_contract: $output,
            exit_contract: $exit,
            retirement_condition: $retirement,
        }
    };
}

macro_rules! stdout {
    ($($identity:literal),+ $(,)?) => {
        OutputContract::Stdout { identities: &[$($identity),+] }
    };
}

macro_rules! artifacts {
    ($($identity:literal),+ $(,)?) => {
        OutputContract::ArtifactFiles { identities: &[$($identity),+] }
    };
}

macro_rules! artifacts_and_stdout {
    ([$($artifact:literal),+ $(,)?], [$($stdout:literal),+ $(,)?]) => {
        OutputContract::ArtifactFilesAndStdout {
            artifact_identities: &[$($artifact),+],
            stdout_identities: &[$($stdout),+],
        }
    };
}

macro_rules! stdout_with_optional_artifacts {
    ([$($artifact:literal),+ $(,)?], [$($stdout:literal),+ $(,)?]) => {
        OutputContract::StdoutWithOptionalArtifactFiles {
            artifact_identities: &[$($artifact),+],
            stdout_identities: &[$($stdout),+],
        }
    };
}

macro_rules! exits {
    ($($code:literal),+ $(,)?) => {
        ExitContract::Exact(&[$($code),+])
    };
}

macro_rules! raw_route {
    ($($field:tt)*) => {
        CommandRoute::Raw(RawRoute { $($field)* })
    };
}

macro_rules! legacy_route {
    ($($field:tt)*) => {
        CommandRoute::Legacy(LegacyRoute { $($field)* })
    };
}

pub(super) const COMMAND_ROUTES: &[CommandRoute] = &[
    CommandRoute::Version(VersionRoute {
        command: "--version",
        aliases: &["-V"],
        contract: command_contract!(
            Public,
            Internal,
            Internal,
            &[],
            stdout!("code-intel-version.v1", "text-format:version-text.v1"),
            exits!(0),
            "retain while this major CLI contract is supported"
        ),
    }),
    raw_route! {
        command: "doctor",
        subcommand: Some("bootstrap"),
        argument_offset: 2,
        id: CompatibilityRoute::DoctorBootstrap,
        contract: command_contract!(
            Public,
            ProviderAdmin,
            Administrative,
            &[CommandEffect::RepoRead, CommandEffect::ProcessSpawn],
            stdout!("code-intel-doctor-bootstrap-observation.v1", "text-format:doctor-bootstrap-human.v1"),
            exits!(0, 1, 65),
            "retire only after typed administration parity and release-package verification"
        ),
    },
    raw_route! {
        command: "compatibility",
        subcommand: Some("retirement-ticket"),
        argument_offset: 2,
        id: CompatibilityRoute::CompatibilityRetirementTicket,
        contract: command_contract!(
            Internal,
            ProviderAdmin,
            Administrative,
            &[CommandEffect::RepoRead],
            stdout!("artifact-format:compatibility-retirement-ticket-lint-json.v1"),
            exits!(0, 65),
            "retire with the last compatibility surface after its recorded parity evidence"
        ),
    },
    raw_route! {
        command: "provider",
        subcommand: Some("repowise-adapt"),
        argument_offset: 2,
        id: CompatibilityRoute::ProviderRepowiseAdapt,
        contract: command_contract!(
            Internal,
            Internal,
            Internal,
            &[CommandEffect::RepoRead],
            stdout!("code-intel-repowise-route-result.v1", "code-intel-repowise-adapter-result.v1"),
            exits!(0, 64, 65),
            "retire when the DAG exclusively calls the typed repowise adapter seam"
        ),
    },
    raw_route! {
        command: "provider",
        subcommand: Some("graph-adapt"),
        argument_offset: 2,
        id: CompatibilityRoute::ProviderGraphAdapt,
        contract: command_contract!(
            Internal,
            Internal,
            Internal,
            &[CommandEffect::RepoRead],
            stdout!("code-intel-graph-route-result.v1", "code-intel-graph-adapter-result.v1"),
            exits!(0, 64, 65),
            "retire when the DAG exclusively calls the typed graph adapter seam"
        ),
    },
    raw_route! {
        command: "provider",
        subcommand: Some("sentrux-adapt"),
        argument_offset: 2,
        id: CompatibilityRoute::ProviderSentruxAdapt,
        contract: command_contract!(
            Internal,
            Internal,
            Internal,
            &[CommandEffect::RepoRead],
            stdout!("code-intel-sentrux-route-result.v1", "code-intel-sentrux-adapter-result.v1"),
            exits!(0, 64, 65),
            "retire when the DAG exclusively calls the typed sentrux adapter seam"
        ),
    },
    raw_route! {
        command: "provider",
        subcommand: Some("session-adapt"),
        argument_offset: 2,
        id: CompatibilityRoute::ProviderSessionAdapt,
        contract: command_contract!(
            Internal,
            AgentSession,
            Internal,
            &[CommandEffect::RepoRead, CommandEffect::LocalWrite],
            stdout_with_optional_artifacts!(["code-intel-session-evidence.v1"], ["code-intel-session-evidence.v1", "code-intel-session-adapter-result.v1"]),
            exits!(0, 64, 65, 74),
            "retire when session evidence admission has no argv caller"
        ),
    },
    raw_route! {
        command: "provider",
        subcommand: Some("codenexus-adapt"),
        argument_offset: 2,
        id: CompatibilityRoute::ProviderCodenexusAdapt,
        contract: command_contract!(
            Internal,
            Internal,
            Internal,
            &[CommandEffect::RepoRead],
            stdout!("code-intel-codenexus-route-result.v1", "code-intel-codenexus-adapter-result.v1"),
            exits!(0, 64, 65),
            "retire when the DAG exclusively calls the typed codenexus adapter seam"
        ),
    },
    raw_route! {
        command: "provider",
        subcommand: Some("file-boundary"),
        argument_offset: 2,
        id: CompatibilityRoute::ProviderFileBoundary,
        contract: command_contract!(
            Internal,
            Internal,
            Internal,
            &[CommandEffect::RepoRead, CommandEffect::LocalWrite],
            artifacts!("code-intel-file-boundary-result.v1"),
            exits!(0, 65),
            "retire when all file-boundary consumers use the typed capability seam"
        ),
    },
    raw_route! {
        command: "provider",
        subcommand: Some("runtime-ci-evidence"),
        argument_offset: 2,
        id: CompatibilityRoute::ProviderRuntimeCiEvidence,
        contract: command_contract!(
            Internal,
            Internal,
            Internal,
            &[CommandEffect::RepoRead, CommandEffect::LocalWrite],
            artifacts!("code-intel-runtime-ci-summary.v1"),
            exits!(0, 65),
            "retire when all runtime evidence consumers use the typed capability seam"
        ),
    },
    raw_route! {
        command: "repository",
        subcommand: Some("survival-scan"),
        argument_offset: 2,
        id: CompatibilityRoute::RepositorySurvivalScan,
        contract: command_contract!(
            Public,
            ProviderAdmin,
            Administrative,
            &[
                CommandEffect::RepoRead,
                CommandEffect::LocalWrite,
                CommandEffect::ProcessSpawn
            ],
            stdout!("code-intel-repository-survival-scan-result.v1", "code-intel-repository-survival-scan-rejection.v1"),
            exits!(0, 64, 65),
            "retire only after a typed administration replacement preserves the scan contract"
        ),
    },
    raw_route! {
        command: "audit",
        subcommand: None,
        argument_offset: 1,
        id: CompatibilityRoute::Audit,
        contract: command_contract!(
            Public,
            Governance,
            Advisory,
            &[CommandEffect::RepoRead, CommandEffect::ProcessSpawn],
            stdout!("artifact-format:audit-validate-json.v1", "text-format:audit-render-markdown.v1", "text-format:audit-render-html.v1", "artifact-format:audit-scope-json.v1"),
            exits!(0, 65),
            "retire only through a versioned audit contract replacement"
        ),
    },
    raw_route! {
        command: "artifact",
        subcommand: Some("index"),
        argument_offset: 1,
        id: CompatibilityRoute::ArtifactIndex,
        contract: command_contract!(
            Public,
            ProviderAdmin,
            Administrative,
            &[CommandEffect::RepoRead, CommandEffect::LocalWrite],
            artifacts_and_stdout!(["code-intel-artifact-index.v1"], ["code-intel-artifact-index.v1"]),
            exits!(0, 65, 74),
            "retire only after typed administration parity for rebuild and incremental modes"
        ),
    },
    raw_route! {
        command: "artifact",
        subcommand: Some("query"),
        argument_offset: 1,
        id: CompatibilityRoute::ArtifactQuery,
        contract: command_contract!(
            Public,
            CommittedEvidence,
            Committed,
            &[CommandEffect::RepoRead, CommandEffect::ProcessSpawn],
            stdout!("code-intel-evidence-query.v1"),
            exits!(0, 65, 74),
            "retire only through a versioned committed-evidence query replacement"
        ),
    },
    raw_route! {
        command: "change",
        subcommand: Some("impact"),
        argument_offset: 1,
        id: CompatibilityRoute::ChangeImpact,
        contract: command_contract!(
            Public,
            CommittedEvidence,
            Conditional(CommittedOrStaleAdvisory),
            &[CommandEffect::RepoRead, CommandEffect::ProcessSpawn],
            stdout!("code-intel-change-impact.v1"),
            exits!(0, 65, 74),
            "retire only through a versioned committed impact replacement"
        ),
    },
    raw_route! {
        // Git-only, index-free defect-risk score for the PR gate (issue
        // #95). Same "change" namespace and offset convention as `change
        // impact` above, but deliberately independent of it: `change impact`
        // requires a committed authoritative run, and this must work on
        // every PR before any such run exists.
        command: "change",
        subcommand: Some("risk"),
        argument_offset: 1,
        id: CompatibilityRoute::ChangeRisk,
        contract: command_contract!(
            Public,
            WorkspaceAdvisory,
            Advisory,
            &[CommandEffect::RepoRead, CommandEffect::ProcessSpawn],
            stdout!("code-intel-change-risk.v1", "text-format:change-risk-text.v1"),
            exits!(0, 65, 74),
            "retire only through a versioned workspace risk replacement"
        ),
    },
    raw_route! {
        // Git-only review agenda for the PR gate (issue #150): the changed
        // files partitioned into co-change units, each scored by the same
        // scorer `change risk` scores the whole change with, ranked worst
        // first. Same namespace, offset, and index-free contract as `change
        // risk`; test selection and structural-rule hits stay in the
        // committed-evidence commands rather than being approximated here.
        command: "change",
        subcommand: Some("agenda"),
        argument_offset: 1,
        id: CompatibilityRoute::ChangeAgenda,
        contract: command_contract!(
            Public,
            WorkspaceAdvisory,
            Advisory,
            &[CommandEffect::RepoRead, CommandEffect::ProcessSpawn],
            stdout!("code-intel-change-agenda.v1", "text-format:change-agenda-text.v1"),
            exits!(0, 65, 74),
            "retire only through a versioned workspace agenda replacement"
        ),
    },
    CommandRoute::Raw(edit_routes::APPLY),
    raw_route! {
        // Working-tree sibling of `change impact`. Separate command because
        // it answers without authority; folding it into `change impact` as a
        // flag would have made one command sometimes admissible and sometimes
        // not, which is the property callers cannot afford to guess at.
        command: "edit",
        subcommand: Some("impact"),
        argument_offset: 2,
        id: CompatibilityRoute::EditImpact,
        contract: command_contract!(
            Public,
            WorkspaceAdvisory,
            Advisory,
            &[CommandEffect::RepoRead, CommandEffect::ProcessSpawn],
            stdout!("code-intel-edit-impact.v1"),
            exits!(0, 65, 74),
            "retire only through a versioned workspace edit-impact replacement"
        ),
    },
    raw_route! {
        command: "decision",
        subcommand: Some("record"),
        argument_offset: 1,
        id: CompatibilityRoute::DecisionRecord,
        contract: command_contract!(
            Public,
            Governance,
            Committed,
            &[CommandEffect::RepoRead, CommandEffect::LocalWrite],
            stdout!("code-intel-decision-record-operation-result.v1"),
            exits!(0, 65),
            "retire only through a versioned governance record replacement"
        ),
    },
    raw_route! {
        command: "decision",
        subcommand: Some("replay"),
        argument_offset: 1,
        id: CompatibilityRoute::DecisionReplay,
        contract: command_contract!(
            Public,
            Governance,
            Committed,
            &[CommandEffect::RepoRead],
            stdout!("code-intel-decision-record-operation-result.v1"),
            exits!(0, 65),
            "retire only through a versioned deterministic replay replacement"
        ),
    },
    raw_route! {
        command: "run",
        subcommand: Some("commit"),
        argument_offset: 1,
        id: CompatibilityRoute::RunCommit,
        contract: command_contract!(
            Internal,
            AuthoritativeRun,
            Internal,
            &[CommandEffect::RepoRead, CommandEffect::LocalWrite],
            artifacts_and_stdout!(["code-intel-run-commit.v1", "code-intel-run-manifest.v1"], ["code-intel-run-commit.v1"]),
            exits!(0, 65, 74, 75),
            "retire when publication is reachable only through the iteration kernel"
        ),
    },
    raw_route! {
        command: "capability",
        subcommand: None,
        argument_offset: 1,
        id: CompatibilityRoute::Capability,
        contract: command_contract!(
            Internal,
            Internal,
            Internal,
            &[
                CommandEffect::RepoRead,
                CommandEffect::LocalWrite,
                CommandEffect::ProcessSpawn,
                CommandEffect::Network,
                CommandEffect::RepoMutation
            ],
            artifacts_and_stdout!(["artifact-format:capability-declared-artifacts.v1"], ["code-intel-capability-result.v1"]),
            exits!(0, 64, 65, 69, 70, 74),
            "retire when every capability caller uses typed requests rather than argv"
        ),
    },
    raw_route! {
        command: "model",
        subcommand: None,
        argument_offset: 1,
        id: CompatibilityRoute::Model,
        contract: command_contract!(
            Internal,
            WorkspaceAdvisory,
            Advisory,
            &[CommandEffect::LocalWrite],
            artifacts_and_stdout!(["code-intel-model-channel-inventory-result.v1", "code-intel-model-routing-result.v1"], ["code-intel-model-channel-inventory-result.v1", "code-intel-model-routing-result.v1"]),
            exits!(0, 2, 64, 65, 74),
            "retire when model evaluation is owned by a typed advisory controller"
        ),
    },
    raw_route! {
        command: "benchmark",
        subcommand: None,
        argument_offset: 1,
        id: CompatibilityRoute::Benchmark,
        contract: command_contract!(
            Internal,
            Internal,
            Advisory,
            &[
                CommandEffect::RepoRead,
                CommandEffect::LocalWrite,
                CommandEffect::ProcessSpawn
            ],
            artifacts_and_stdout!(["code-intel-project-orientation-benchmark-observations.v1", "code-intel-project-orientation-benchmark.v1", "code-intel-tool-effectiveness-publication.v1"], ["code-intel-project-orientation-benchmark.v1", "code-intel-tool-effectiveness-benchmark.v1"]),
            exits!(0, 64, 65, 69, 70, 74),
            "retire when benchmark entrypoints move to the evaluation harness"
        ),
    },
    raw_route! {
        command: "snapshot",
        subcommand: None,
        argument_offset: 1,
        id: CompatibilityRoute::Snapshot,
        contract: command_contract!(
            Internal,
            Internal,
            Internal,
            &[CommandEffect::RepoRead, CommandEffect::ProcessSpawn],
            stdout!("code-intel-repository-snapshot.v1"),
            exits!(0, 64, 65, 69, 74),
            "retire when snapshot creation is reachable only through typed kernel requests"
        ),
    },
    raw_route! {
        command: "repin",
        subcommand: None,
        argument_offset: 1,
        id: CompatibilityRoute::Repin,
        contract: command_contract!(
            Public,
            ProviderAdmin,
            Administrative,
            &[
                CommandEffect::RepoRead,
                CommandEffect::LocalWrite,
                CommandEffect::ProcessSpawn,
                CommandEffect::RepoMutation
            ],
            stdout!("code-intel-repin-report.v1", "text-format:repin-human.v1"),
            exits!(0, 1, 64, 65, 74),
            "retire only after a typed administration replacement preserves literal repinning"
        ),
    },
    CommandRoute::Raw(repowise_routes::HOOKS),
    raw_route! {
        command: "evidence",
        subcommand: None,
        argument_offset: 1,
        id: CompatibilityRoute::Evidence,
        contract: command_contract!(
            Internal,
            Internal,
            Internal,
            &[CommandEffect::RepoRead],
            stdout!("code-intel-evidence-admissibility-result.v1"),
            exits!(0, 64, 65, 74),
            "retire when evidence validation is reachable only through the typed capability seam"
        ),
    },
    raw_route! {
        command: "decision",
        subcommand: None,
        argument_offset: 1,
        id: CompatibilityRoute::Decision,
        contract: command_contract!(
            Public,
            Governance,
            Advisory,
            &[CommandEffect::RepoRead],
            stdout!("code-intel-decision-exchange-result.v1"),
            exits!(0, 10, 11, 12, 65, 70),
            "retire only through a versioned governance request-response replacement"
        ),
    },
    CommandRoute::Raw(run_routes::EXECUTE),
    CommandRoute::Raw(run_routes::DAG_COORDINATE),
    CommandRoute::Raw(serve_routes::MCP),
    raw_route! {
        command: "governance",
        subcommand: None,
        argument_offset: 1,
        id: CompatibilityRoute::Governance,
        contract: command_contract!(
            Public,
            Governance,
            Committed,
            &[CommandEffect::RepoRead],
            stdout!("code-intel-ponytail-gate-result.v1"),
            exits!(0, 2, 64, 65, 74),
            "retire only through a versioned governance admission replacement"
        ),
    },
    legacy_route! {
        command: "report",
        aliases: &[],
        id: LegacyRouteId::Report,
        contract: command_contract!(
            Public,
            Internal,
            Internal,
            &[CommandEffect::RepoRead],
            stdout!("code-intel-report.v1", "text-format:report-text.v1"),
            exits!(0, 1),
            "retain while committed-run report reading is the supported human-facing surface"
        ),
    },
    legacy_route! {
        command: "resume",
        aliases: &[],
        id: LegacyRouteId::Resume,
        contract: command_contract!(
            Compatibility,
            Internal,
            Internal,
            &[CommandEffect::RepoRead],
            stdout!("artifact-format:resume-json.v1", "text-format:resume-text.v1"),
            exits!(0, 1),
            "retire after Phase 4 replaces directory-name lookup with verified committed evidence"
        ),
    },
    legacy_route! {
        command: "classify",
        aliases: &[],
        id: LegacyRouteId::Classify,
        contract: command_contract!(
            Compatibility,
            Internal,
            Internal,
            &[CommandEffect::RepoRead],
            stdout!("artifact-format:classify-json.v1", "text-format:classify-text.v1"),
            exits!(0, 1),
            "retire after all diagnostic callers consume typed failure classification"
        ),
    },
    legacy_route! {
        command: "doctor",
        aliases: &[],
        id: LegacyRouteId::Doctor,
        contract: command_contract!(
            Compatibility,
            ProviderAdmin,
            Administrative,
            &[CommandEffect::RepoRead, CommandEffect::ProcessSpawn],
            stdout!("code-intel-doctor-observation.v1", "text-format:doctor-human.v1"),
            exits!(0, 1),
            "retire after typed administration parity and release compatibility coverage"
        ),
    },
    legacy_route! {
        command: "sentrux-normalize",
        aliases: &[],
        id: LegacyRouteId::SentruxNormalize,
        contract: command_contract!(
            Compatibility,
            AgentSession,
            Advisory,
            &[CommandEffect::RepoRead, CommandEffect::LocalWrite],
            artifacts_and_stdout!(["code-intel-sentrux-failures.v1"], ["code-intel-sentrux-failures.v1"]),
            exits!(0, 1),
            "retire after typed session diagnostics preserve normalized output bytes"
        ),
    },
    legacy_route! {
        command: "sentrux-debt-register",
        aliases: &[],
        id: LegacyRouteId::SentruxDebtRegister,
        contract: command_contract!(
            Compatibility,
            AgentSession,
            Advisory,
            &[CommandEffect::RepoRead, CommandEffect::LocalWrite],
            artifacts_and_stdout!(["code-intel-sentrux-debt-register.v1"], ["code-intel-sentrux-debt-register.v1"]),
            exits!(0, 1),
            "retire after typed session diagnostics preserve debt classification"
        ),
    },
    legacy_route! {
        command: "graph",
        aliases: &["understand"],
        id: LegacyRouteId::Graph,
        contract: command_contract!(
            Compatibility,
            WorkspaceAdvisory,
            Advisory,
            &[
                CommandEffect::RepoRead,
                CommandEffect::LocalWrite
            ],
            artifacts_and_stdout!(["code-intel-understand-graph.v1"], ["code-intel-understand-graph.v1", "text-format:graph-human.v1"]),
            exits!(0, 1),
            "retire after graph and understand spellings have typed advisory parity"
        ),
    },
    legacy_route! {
        command: "orchestrate",
        aliases: &["orchestration"],
        id: LegacyRouteId::Orchestrate,
        contract: command_contract!(
            Compatibility,
            ProviderAdmin,
            Administrative,
            &[CommandEffect::RepoRead],
            stdout!("artifact-format:orchestration-list-json.v1", "artifact-format:orchestration-plan-json.v1", "text-format:orchestration-human.v1"),
            exits!(0, 1),
            "retire after registry validation/list/plan parity and release coverage"
        ),
    },
    legacy_route! {
        command: "provider",
        aliases: &["providers"],
        id: LegacyRouteId::Provider,
        contract: command_contract!(
            Compatibility,
            ProviderAdmin,
            Administrative,
            &[
                CommandEffect::RepoRead,
                CommandEffect::LocalWrite,
                CommandEffect::ProcessSpawn
            ],
            artifacts_and_stdout!(["artifact-format:provider-invocation-artifacts.v1"], ["code-intel-provider-api.v1", "text-format:provider-human.v1"]),
            exits!(0, 1),
            "retire after provider administration aliases have typed parity"
        ),
    },
    legacy_route! {
        command: "route",
        aliases: &["routes"],
        id: LegacyRouteId::Route,
        contract: command_contract!(
            Compatibility,
            ProviderAdmin,
            Administrative,
            &[CommandEffect::RepoRead],
            stdout!("code-intel-route-registry.v1", "text-format:route-human.v1"),
            exits!(0, 1),
            "retire after route administration aliases have typed parity"
        ),
    },
    legacy_route! {
        command: "sentrux",
        aliases: &[],
        id: LegacyRouteId::Sentrux,
        contract: command_contract!(
            Compatibility,
            AgentSession,
            Administrative,
            &[
                CommandEffect::RepoRead,
                CommandEffect::LocalWrite,
                CommandEffect::ProcessSpawn
            ],
            stdout!("artifact-format:sentrux-dsm.v1", "artifact-format:sentrux-scan.v1", "text-format:sentrux-health.v1", "text-format:sentrux-gate.v1"),
            exits!(0, 1),
            "retire after the Rust session controller and compatibility shim pass parity"
        ),
    },
    legacy_route! {
        command: HELP_COMMAND,
        aliases: HELP_ALIASES,
        id: LegacyRouteId::Help,
        contract: command_contract!(
            Public,
            Internal,
            Internal,
            &[],
            stdout!("text-format:help-quick.v1", "text-format:help-full.v2"),
            exits!(0, 1),
            "retain while this major CLI contract is supported"
        ),
    },
    legacy_route! {
        command: "language",
        aliases: &[],
        id: LegacyRouteId::Language,
        contract: command_contract!(
            Public,
            ProviderAdmin,
            Administrative,
            &[CommandEffect::RepoRead, CommandEffect::LocalWrite],
            stdout!("code-intel-language-preference.v1", "text-format:language-human.v1"),
            exits!(0, 1),
            "retire only through a versioned language administration replacement"
        ),
    },
    CommandRoute::Primary(command_contract!(
        Public,
        AuthoritativeRun,
        Committed,
        &[
            CommandEffect::RepoRead,
            CommandEffect::LocalWrite,
            CommandEffect::ProcessSpawn,
            CommandEffect::Network
        ],
        artifacts_and_stdout!(
            ["code-intel-run-manifest.v1", "code-intel-artifact-index.v1"],
            [
                "code-intel-primary-result.v1",
                "text-format:primary-summary.v1"
            ]
        ),
        exits!(0, 10, 20, 64, 65, 70, 74),
        "retain as the default production entry; retire only by versioned replacement"
    )),
];

pub(super) fn resolve_command_route(raw: &[String]) -> Option<&'static CommandRoute> {
    COMMAND_ROUTES.iter().find(|route| match route {
        CommandRoute::Version(route) => raw.first().is_some_and(|command| {
            route.command == command || route.aliases.iter().any(|alias| alias == command)
        }),
        CommandRoute::Primary(_) => matches_primary_pattern(raw),
        CommandRoute::Raw(route) => raw.first().is_some_and(|command| {
            route.command == command
                && route.subcommand.map_or(true, |subcommand| {
                    raw.get(1).map(String::as_str) == Some(subcommand)
                })
        }),
        CommandRoute::Legacy(route) => raw.first().is_some_and(|command| {
            route.command == command || route.aliases.iter().any(|alias| alias == command)
        }),
    })
}

pub(super) fn resolve_legacy_route(command: &str) -> Option<&'static LegacyRoute> {
    COMMAND_ROUTES.iter().find_map(|route| match route {
        CommandRoute::Legacy(route)
            if route.command == command || route.aliases.iter().any(|alias| *alias == command) =>
        {
            Some(route)
        }
        _ => None,
    })
}

pub(super) fn resolve_raw_route(raw: &[String]) -> Option<&'static RawRoute> {
    match resolve_command_route(raw)? {
        CommandRoute::Raw(route) => Some(route),
        _ => None,
    }
}
