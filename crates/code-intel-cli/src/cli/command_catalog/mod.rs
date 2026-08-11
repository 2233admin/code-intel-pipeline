use std::error::Error;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::{
    admissibility, artifact_index, audit_report, change_agenda, change_impact, change_risk,
    compatibility_retirement_ticket, decision_port, decision_record, doctor_bootstrap, edit_apply,
    edit_impact, evidence_query, invocation_identity, mcp_serve, model_channels, ponytail_gate,
    providers, repin, repowise_hooks, run_cli, run_commit, session_evidence, snapshot,
    survival_scan,
};

use super::legacy::{
    cmd_classify, cmd_doctor, cmd_graph, cmd_help, cmd_language, cmd_orchestrate, cmd_provider,
    cmd_report, cmd_resume, cmd_route, cmd_sentrux, cmd_sentrux_debt_register,
    cmd_sentrux_normalize, parse_args, run_benchmark, run_capability, run_file_boundary_raw,
    run_runtime_ci_raw, Args,
};
use super::primary::{
    execute_primary, matches_primary_pattern, parse_primary_args, parse_run_alias_args, PrimaryArgs,
};
use super::project_query::{execute_project_query, parse_project_query_args, ProjectQueryArgs};

mod contract;
mod routes;

use routes::{resolve_command_route, resolve_legacy_route, CommandRoute};

/// The single typed command seam between process argv and CLI execution.
///
/// Compatibility commands intentionally keep their already-sliced arguments
/// private. Existing command-family parsers still consume those arguments in
/// Phase 2, but controllers never receive process argv and the descriptive
/// catalog no longer contains executable function pointers.
#[derive(Debug)]
pub(super) enum Command {
    Version(VersionCommand),
    Primary(PrimaryArgs),
    ProjectQuery(ProjectQueryArgs),
    Compatibility(CompatibilityCommand),
    Legacy(LegacyCommand),
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct VersionCommand {
    json: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct CompatibilityCommand {
    route: CompatibilityRoute,
    arguments: Vec<String>,
}

#[derive(Debug)]
pub(super) struct LegacyCommand {
    route: LegacyRouteId,
    arguments: Args,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CompatibilityRoute {
    DoctorBootstrap,
    CompatibilityRetirementTicket,
    ProviderRepowiseAdapt,
    ProviderGraphAdapt,
    ProviderSentruxAdapt,
    ProviderSessionAdapt,
    ProviderCodenexusAdapt,
    ProviderFileBoundary,
    ProviderRuntimeCiEvidence,
    RepositorySurvivalScan,
    Audit,
    ArtifactIndex,
    ArtifactQuery,
    ChangeImpact,
    ChangeRisk,
    ChangeAgenda,
    EditApply,
    EditImpact,
    DecisionRecord,
    DecisionReplay,
    RunCommit,
    Capability,
    Model,
    Benchmark,
    Snapshot,
    Repin,
    RepowiseHooks,
    Evidence,
    Decision,
    RunExecute,
    RunDagCoordinate,
    Serve,
    Governance,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LegacyRouteId {
    Report,
    Resume,
    Classify,
    Doctor,
    SentruxNormalize,
    SentruxDebtRegister,
    Graph,
    Orchestrate,
    Provider,
    Route,
    Sentrux,
    Help,
    Language,
}

#[derive(Debug)]
pub(super) struct CommandOutcome {
    exit_code: i32,
    emission: CommandEmission,
}

#[derive(Debug)]
enum CommandEmission {
    Buffered {
        stdout: String,
        stderr: String,
    },
    /// A private Phase-2 adapter invoked a legacy implementation which writes
    /// directly to the process streams. It is explicit so later phases can
    /// remove passthrough one command family at a time without hidden capture.
    CompatibilityAlreadyEmitted,
}

#[derive(Debug)]
pub(super) enum CommandError {
    Primary {
        json: bool,
        repo: Option<PathBuf>,
        mode: Option<String>,
        exit_code: i32,
        message: String,
    },
    Project {
        kind: &'static str,
        exit_code: i32,
        message: String,
    },
    Legacy {
        message: String,
    },
    Usage {
        message: String,
        exit_code: i32,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RenderedOutcome {
    pub(crate) stdout: String,
    pub(crate) stderr: String,
    pub(crate) exit_code: i32,
}

pub(super) fn parse_command(raw: &[String]) -> std::result::Result<Command, CommandError> {
    match resolve_command_route(raw) {
        Some(CommandRoute::Version(_)) => Ok(Command::Version(VersionCommand {
            json: raw.iter().any(|argument| argument == "--json"),
        })),
        Some(CommandRoute::Primary(_)) => {
            parse_primary_args(raw)
                .map(Command::Primary)
                .map_err(|message| CommandError::Primary {
                    json: raw.iter().any(|argument| argument == "--json"),
                    repo: None,
                    mode: None,
                    exit_code: 64,
                    message,
                })
        }
        Some(CommandRoute::RunAlias(_)) => parse_run_alias_args(&raw[1..])
            .map(Command::Primary)
            .map_err(|message| CommandError::Primary {
                json: raw.iter().any(|argument| argument == "--json"),
                repo: None,
                mode: None,
                exit_code: 64,
                message,
            }),
        Some(CommandRoute::ProjectQuery(_)) => parse_project_query_args(&raw[1..])
            .map(Command::ProjectQuery)
            .map_err(|message| {
                if raw.iter().any(|argument| argument == "--json") {
                    CommandError::Project {
                        kind: "usage",
                        exit_code: 64,
                        message,
                    }
                } else {
                    CommandError::Usage {
                        message,
                        exit_code: 64,
                    }
                }
            }),
        Some(CommandRoute::Raw(route)) => Ok(Command::Compatibility(CompatibilityCommand {
            route: route.id,
            arguments: raw[route.argument_offset..].to_vec(),
        })),
        Some(CommandRoute::Legacy(_)) => {
            parse_args(raw.to_vec())
                .map_err(legacy_error)
                .and_then(|arguments| {
                    let route = resolve_legacy_route(arguments.command()).ok_or_else(|| {
                        CommandError::Legacy {
                            message: format!(
                                "parsed legacy command has no registered route: {}",
                                arguments.command()
                            ),
                        }
                    })?;
                    Ok(Command::Legacy(LegacyCommand {
                        route: route.id,
                        arguments,
                    }))
                })
        }
        None => match parse_args(raw.to_vec()) {
            Ok(arguments) => Err(CommandError::Usage {
                message: format!(
                    "unknown command: {}; run `code-intel --help` for available commands",
                    arguments.command()
                ),
                exit_code: 64,
            }),
            Err(error) => Err(legacy_error(error)),
        },
    }
}

pub(super) fn execute_command(
    command: Command,
) -> std::result::Result<CommandOutcome, CommandError> {
    match command {
        Command::Version(command) => {
            Ok(buffered_outcome(0, render_version(&command), String::new()))
        }
        Command::Primary(arguments) => match execute_primary(&arguments) {
            Ok((exit_code, output)) => Ok(buffered_outcome(
                exit_code,
                render_primary_success(&arguments, &output),
                String::new(),
            )),
            Err(error) => Err(CommandError::Primary {
                json: arguments.json,
                repo: Some(arguments.repo),
                mode: Some(arguments.mode),
                exit_code: error.exit_code(),
                message: error.message().to_string(),
            }),
        },
        Command::ProjectQuery(arguments) => match execute_project_query(arguments) {
            Ok(output) => Ok(buffered_outcome(
                0,
                format!(
                    "{}\n",
                    serde_json::to_string(&output).expect("project query result serializes")
                ),
                String::new(),
            )),
            Err(error) => Err(CommandError::Project {
                kind: error.kind(),
                exit_code: error.exit_code(),
                message: error.message().to_string(),
            }),
        },
        Command::Compatibility(command) => Ok(CommandOutcome {
            exit_code: execute_compatibility(command),
            emission: CommandEmission::CompatibilityAlreadyEmitted,
        }),
        Command::Legacy(command) => {
            execute_legacy(&command).map_err(legacy_error)?;
            Ok(CommandOutcome {
                exit_code: 0,
                emission: CommandEmission::CompatibilityAlreadyEmitted,
            })
        }
    }
}

pub(super) fn render_outcome(
    result: std::result::Result<CommandOutcome, CommandError>,
) -> RenderedOutcome {
    match result {
        Ok(CommandOutcome {
            exit_code,
            emission: CommandEmission::Buffered { stdout, stderr },
        }) => RenderedOutcome {
            stdout,
            stderr,
            exit_code,
        },
        Ok(CommandOutcome {
            exit_code,
            emission: CommandEmission::CompatibilityAlreadyEmitted,
        }) => RenderedOutcome {
            stdout: String::new(),
            stderr: String::new(),
            exit_code,
        },
        Err(CommandError::Primary {
            json,
            repo,
            mode,
            exit_code,
            message,
        }) => render_primary_error(json, repo.as_deref(), mode.as_deref(), exit_code, &message),
        Err(CommandError::Legacy { message }) => RenderedOutcome {
            stdout: String::new(),
            stderr: format!("error: {message}\n"),
            exit_code: 1,
        },
        Err(CommandError::Project {
            kind,
            exit_code,
            message,
        }) => RenderedOutcome {
            stdout: format!(
                "{}\n",
                serde_json::to_string(&json!({
                    "schema": "code-intel-project-error.v1",
                    "outcome": "error",
                    "kind": kind,
                    "exitCode": exit_code,
                    "diagnostic": message,
                }))
                .expect("project error serializes")
            ),
            stderr: String::new(),
            exit_code,
        },
        Err(CommandError::Usage { message, exit_code }) => RenderedOutcome {
            stdout: String::new(),
            stderr: format!("{message}\n"),
            exit_code,
        },
    }
}

fn buffered_outcome(exit_code: i32, stdout: String, stderr: String) -> CommandOutcome {
    CommandOutcome {
        exit_code,
        emission: CommandEmission::Buffered { stdout, stderr },
    }
}

fn legacy_error(error: Box<dyn Error>) -> CommandError {
    CommandError::Legacy {
        message: error.to_string(),
    }
}

/// Answers `--version` before any named route.
///
/// This remains a self-declared build identity. Installer-recorded binary
/// provenance is the stronger substitution signal.
fn render_version(command: &VersionCommand) -> String {
    if command.json {
        format!(
            "{}\n",
            json!({
                "schema": "code-intel-version.v1",
                "name": env!("CARGO_PKG_NAME"),
                "version": env!("CARGO_PKG_VERSION"),
            })
        )
    } else {
        format!("{} {}\n", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"))
    }
}

fn render_primary_success(arguments: &PrimaryArgs, output: &Value) -> String {
    if arguments.json {
        format!(
            "{}\n",
            serde_json::to_string(output).expect("primary result serializes")
        )
    } else {
        render_primary_summary(output)
    }
}

fn render_primary_error(
    json_output: bool,
    repo: Option<&Path>,
    mode: Option<&str>,
    exit_code: i32,
    diagnostic: &str,
) -> RenderedOutcome {
    if json_output {
        RenderedOutcome {
            stdout: format!(
                "{}\n",
                serde_json::to_string(&json!({
                    "schema": "code-intel-primary-result.v1",
                    "repo": repo,
                    "mode": mode,
                    "outcome": "error",
                    "exitCode": exit_code,
                    "publication": Value::Null,
                    "failureNode": Value::Null,
                    "diagnostic": diagnostic,
                }))
                .expect("primary error result serializes")
            ),
            stderr: String::new(),
            exit_code,
        }
    } else {
        RenderedOutcome {
            stdout: String::new(),
            stderr: format!("Code Intel error: {diagnostic}\n"),
            exit_code,
        }
    }
}

fn render_primary_summary(output: &Value) -> String {
    let passed = output["exitCode"].as_i64() == Some(0);
    let mut lines = vec![
        format!(
            "[{}] {}",
            if passed { "PASS" } else { "FAIL" },
            output["repo"].as_str().unwrap_or("")
        ),
        format!("  Outcome: {}", output["outcome"].as_str().unwrap_or("")),
        format!(
            "  Run evidence: {}",
            output["publication"]["marker"].as_str().unwrap_or("")
        ),
    ];
    if let Some(artifacts) = output["readableArtifacts"].as_object() {
        for (name, path) in artifacts {
            if let Some(path) = path.as_str() {
                lines.push(format!("  {name}: {path}"));
            }
        }
    }
    if let Some(node) = output["failureNode"].as_str() {
        let diagnostic = output["diagnostic"].as_str().unwrap_or("");
        lines.push(format!(
            "  Cause: {node}{}",
            if diagnostic.is_empty() {
                String::new()
            } else {
                format!(" - {diagnostic}")
            }
        ));
    }
    for entry in output["failures"]["process"]
        .as_array()
        .into_iter()
        .flatten()
    {
        lines.push(format!(
            "  Process failure: {} - {}",
            entry["node"].as_str().unwrap_or(""),
            entry["diagnostic"].as_str().unwrap_or("")
        ));
    }
    for entry in output["failures"]["domain"]
        .as_array()
        .into_iter()
        .flatten()
    {
        lines.push(format!(
            "  Domain failure: {} (verdict: {})",
            entry["node"].as_str().unwrap_or(""),
            entry["verdict"].as_str().unwrap_or("fail")
        ));
    }
    format!("{}\n", lines.join("\n"))
}

/// Private, explicit Phase-2 adapters. No controller can call this with argv;
/// only a typed route ID and its catalog-sliced compatibility arguments arrive.
fn execute_compatibility(command: CompatibilityCommand) -> i32 {
    if let Some(label) = invocation_identity_label(command.route, &command.arguments) {
        invocation_identity::emit(label);
    }
    let raw = &command.arguments;
    match command.route {
        CompatibilityRoute::DoctorBootstrap => doctor_bootstrap::run_raw(raw),
        CompatibilityRoute::CompatibilityRetirementTicket => {
            compatibility_retirement_ticket::run_raw(raw)
        }
        CompatibilityRoute::ProviderRepowiseAdapt => providers::run_repowise_adapt_raw(raw),
        CompatibilityRoute::ProviderGraphAdapt => providers::run_graph_adapt_raw(raw),
        CompatibilityRoute::ProviderSentruxAdapt => providers::run_sentrux_adapt_raw(raw),
        CompatibilityRoute::ProviderSessionAdapt => session_evidence::run_raw(raw),
        CompatibilityRoute::ProviderCodenexusAdapt => providers::run_codenexus_adapt_raw(raw),
        CompatibilityRoute::ProviderFileBoundary => run_file_boundary_raw(raw),
        CompatibilityRoute::ProviderRuntimeCiEvidence => run_runtime_ci_raw(raw),
        CompatibilityRoute::RepositorySurvivalScan => survival_scan::run_raw(raw),
        CompatibilityRoute::Audit => audit_report::run_raw(raw),
        CompatibilityRoute::ArtifactIndex => artifact_index::run_raw(raw),
        CompatibilityRoute::ArtifactQuery => evidence_query::run_raw(raw),
        CompatibilityRoute::ChangeImpact => change_impact::run_raw(raw),
        CompatibilityRoute::ChangeRisk => change_risk::run_raw(raw),
        CompatibilityRoute::ChangeAgenda => change_agenda::run_raw(raw),
        CompatibilityRoute::EditApply => edit_apply::run_raw(raw),
        CompatibilityRoute::EditImpact => edit_impact::run_raw(raw),
        CompatibilityRoute::DecisionRecord | CompatibilityRoute::DecisionReplay => {
            decision_record::run_raw(raw)
        }
        CompatibilityRoute::RunCommit => run_commit::run_raw(raw),
        CompatibilityRoute::Capability => run_capability(raw),
        CompatibilityRoute::Model => model_channels::run_raw(raw),
        CompatibilityRoute::Benchmark => run_benchmark(raw),
        CompatibilityRoute::Snapshot => snapshot::run_raw(raw),
        CompatibilityRoute::Repin => repin::run_raw(raw),
        CompatibilityRoute::RepowiseHooks => repowise_hooks::run_raw(raw),
        CompatibilityRoute::Evidence => admissibility::run_raw(raw),
        CompatibilityRoute::Decision => decision_port::run_raw(raw),
        CompatibilityRoute::RunExecute | CompatibilityRoute::RunDagCoordinate => {
            run_cli::run_raw(raw)
        }
        CompatibilityRoute::Serve => mcp_serve::run_raw(raw),
        CompatibilityRoute::Governance => ponytail_gate::run_raw(raw),
    }
}

/// Verdict-producing routes echo an anti-replay identity line (#197) before
/// their own output. Contract probes are catalog introspection, not verdicts,
/// and the head-parity fixture byte-compares their stderr, so they stay
/// silent. Doctor is excluded: its envelope contract asserts an empty stderr.
fn invocation_identity_label(
    route: CompatibilityRoute,
    arguments: &[String],
) -> Option<&'static str> {
    if arguments
        .iter()
        .any(|argument| argument == "--contract-probe")
    {
        return None;
    }
    match route {
        CompatibilityRoute::RunExecute => Some("run-execute"),
        CompatibilityRoute::RunDagCoordinate => Some("run-dag-coordinate"),
        CompatibilityRoute::Audit => Some("audit"),
        CompatibilityRoute::Benchmark => Some("benchmark"),
        _ => None,
    }
}

fn execute_legacy(command: &LegacyCommand) -> std::result::Result<(), Box<dyn Error>> {
    match command.route {
        LegacyRouteId::Report => cmd_report(&command.arguments),
        LegacyRouteId::Resume => cmd_resume(&command.arguments),
        LegacyRouteId::Classify => cmd_classify(&command.arguments),
        LegacyRouteId::Doctor => cmd_doctor(&command.arguments),
        LegacyRouteId::SentruxNormalize => cmd_sentrux_normalize(&command.arguments),
        LegacyRouteId::SentruxDebtRegister => cmd_sentrux_debt_register(&command.arguments),
        LegacyRouteId::Graph => cmd_graph(&command.arguments),
        LegacyRouteId::Orchestrate => cmd_orchestrate(&command.arguments),
        LegacyRouteId::Provider => cmd_provider(&command.arguments),
        LegacyRouteId::Route => cmd_route(&command.arguments),
        LegacyRouteId::Sentrux => cmd_sentrux(&command.arguments),
        LegacyRouteId::Help => cmd_help(&command.arguments),
        LegacyRouteId::Language => cmd_language(&command.arguments),
    }
}

#[cfg(test)]
mod tests;
