use std::env;
use std::path::PathBuf;

use crate::artifacts::{self, ARTIFACT_ROOT_ENV};
use crate::dag_run::{self, DagExecutionRequest};
use crate::execution_kernel;
use crate::execution_policy::{ExecutionPolicy, RunMode, RunProfile, SkipFlags, WorkingTreePolicy};
use crate::run_error::RunError;

pub(crate) fn run_raw(raw: &[String]) -> i32 {
    let cli = match Cli::parse(raw) {
        Ok(cli) => cli,
        Err(error) => {
            eprintln!("{error}");
            return 64;
        }
    };
    match execute_cli(cli) {
        Ok(result) => {
            println!(
                "{}",
                serde_json::to_string(&result.output).expect("run result serializes")
            );
            result.exit_code
        }
        Err(error) => {
            eprintln!("{}", error.message);
            error.exit_code
        }
    }
}

struct CliResult {
    output: serde_json::Value,
    exit_code: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RunCommand {
    DagCoordinate,
    Execute,
}

struct Cli {
    command: RunCommand,
    repo: PathBuf,
    out: Option<PathBuf>,
    artifact_root: Option<PathBuf>,
    authority_root: Option<PathBuf>,
    final_name: Option<String>,
    manifest: Option<PathBuf>,
    max_concurrency: usize,
    policy: ExecutionPolicy,
    diagnosis_inputs: Option<PathBuf>,
    seed_artifact_root: Option<PathBuf>,
    session_evidence: Option<PathBuf>,
}

impl Cli {
    fn parse(raw: &[String]) -> Result<Self, String> {
        let command = match raw.first().map(String::as_str) {
            Some("dag-coordinate") => RunCommand::DagCoordinate,
            Some("execute") => RunCommand::Execute,
            _ => return Err(usage()),
        };
        let mut repo = None;
        let mut out = None;
        let mut artifact_root = None;
        let mut authority_root = None;
        let mut final_name = None;
        let mut manifest = None;
        let mut max_concurrency = 2usize;
        let mut working_tree_policy = "explicit_overlay".to_string();
        let mut scopes = Vec::new();
        let mut diagnosis_inputs = None;
        let mut seed_artifact_root = None;
        let mut doctor_tool_path_prefix = None;
        let mut doctor_require_repowise = None;
        let mut doctor_require_understand = None;
        let mut profile = match command {
            RunCommand::DagCoordinate => RunProfile::Compatibility,
            RunCommand::Execute => RunProfile::Default,
        };
        let mut mode = RunMode::default();
        let mut skips = SkipFlags::default();
        let mut session_evidence = None;
        let mut index = 1;
        while index < raw.len() {
            let flag = raw[index].as_str();
            if !matches!(
                flag,
                "--repo"
                    | "--out"
                    | "--artifact-root"
                    | "--authority-root"
                    | "--final-name"
                    | "--manifest"
                    | "--profile"
                    | "--mode"
                    | "--skip-repowise"
                    | "--skip-sentrux"
                    | "--require-understand-graph"
                    | "--max-concurrency"
                    | "--working-tree-policy"
                    | "--scope"
                    | "--diagnosis-inputs"
                    | "--seed-artifact-root"
                    | "--doctor-tool-path-prefix"
                    | "--doctor-require-repowise"
                    | "--doctor-require-understand"
                    | "--session-evidence"
            ) {
                return Err(format!("unknown DAG run argument: {flag}"));
            }
            let value = raw
                .get(index + 1)
                .filter(|value| !value.is_empty() && !value.starts_with("--"))
                .ok_or_else(|| format!("{flag} requires one value"))?;
            match flag {
                "--repo" if repo.replace(PathBuf::from(value)).is_some() => {
                    return Err("duplicate --repo".into())
                }
                "--out" if out.replace(PathBuf::from(value)).is_some() => {
                    return Err("duplicate --out".into())
                }
                "--artifact-root" if artifact_root.replace(PathBuf::from(value)).is_some() => {
                    return Err("duplicate --artifact-root".into())
                }
                "--authority-root" if authority_root.replace(PathBuf::from(value)).is_some() => {
                    return Err("duplicate --authority-root".into())
                }
                "--final-name" if final_name.replace(value.clone()).is_some() => {
                    return Err("duplicate --final-name".into())
                }
                "--manifest" if manifest.replace(PathBuf::from(value)).is_some() => {
                    return Err("duplicate --manifest".into())
                }
                "--profile" => {
                    if command != RunCommand::Execute {
                        return Err("--profile is available only for run execute".into());
                    }
                    profile = RunProfile::parse(value)?;
                }
                "--mode" => {
                    if command != RunCommand::Execute {
                        return Err("--mode is available only for run execute".into());
                    }
                    mode = RunMode::parse(value)?;
                }
                "--skip-repowise" => skips.repowise = parse_bool_flag(flag, value)?,
                "--skip-sentrux" => skips.sentrux = parse_bool_flag(flag, value)?,
                "--require-understand-graph" => {
                    skips.require_understand_graph = parse_bool_flag(flag, value)?
                }
                "--max-concurrency" => {
                    max_concurrency = value
                        .parse::<usize>()
                        .map_err(|_| "--max-concurrency must be an integer".to_string())?;
                }
                "--working-tree-policy" => working_tree_policy = value.clone(),
                "--scope" => scopes.push(value.clone()),
                "--diagnosis-inputs"
                    if diagnosis_inputs.replace(PathBuf::from(value)).is_some() =>
                {
                    return Err("duplicate --diagnosis-inputs".into())
                }
                "--seed-artifact-root"
                    if seed_artifact_root.replace(PathBuf::from(value)).is_some() =>
                {
                    return Err("duplicate --seed-artifact-root".into())
                }
                "--doctor-tool-path-prefix"
                    if doctor_tool_path_prefix
                        .replace(PathBuf::from(value))
                        .is_some() =>
                {
                    return Err("duplicate --doctor-tool-path-prefix".into())
                }
                "--doctor-require-repowise" => {
                    doctor_require_repowise = Some(parse_bool_flag(flag, value)?);
                }
                "--doctor-require-understand" => {
                    doctor_require_understand = Some(parse_bool_flag(flag, value)?);
                }
                "--session-evidence"
                    if session_evidence.replace(PathBuf::from(value)).is_some() =>
                {
                    return Err("duplicate --session-evidence".into())
                }
                _ => {}
            }
            index += 2;
        }
        let repo = repo.ok_or("--repo is required")?;
        if !repo.is_dir() {
            return Err(format!(
                "repository path is not a directory: {}",
                repo.display()
            ));
        }
        if diagnosis_inputs.is_some() != seed_artifact_root.is_some() {
            return Err(
                "--diagnosis-inputs and --seed-artifact-root must be provided together".into(),
            );
        }
        if diagnosis_inputs.is_some() && doctor_tool_path_prefix.is_some() {
            return Err(
                "--doctor-tool-path-prefix is valid only for the default DAG containing doctor"
                    .into(),
            );
        }
        if diagnosis_inputs.is_some() && session_evidence.is_some() {
            return Err("--session-evidence is valid only for the default analysis DAG".into());
        }
        if let Some(path) = &session_evidence {
            if !path.is_file() {
                return Err(format!(
                    "session evidence path is not a file: {}",
                    path.display()
                ));
            }
        }
        if let Some(prefix) = &mut doctor_tool_path_prefix {
            if !prefix.is_dir() {
                return Err(format!(
                    "--doctor-tool-path-prefix is not a directory: {}",
                    prefix.display()
                ));
            }
            if prefix.is_relative() {
                *prefix = std::env::current_dir()
                    .map_err(|error| format!("resolve current directory: {error}"))?
                    .join(&*prefix);
            }
        }
        match command {
            RunCommand::DagCoordinate => {
                if authority_root.is_some() || final_name.is_some() {
                    return Err(
                        "--authority-root and --final-name are available only for run execute"
                            .into(),
                    );
                }
                // The two routes disagree about who owns the path, so accepting
                // both would silently honour one and drop the other.
                if out.is_some() && artifact_root.is_some() {
                    return Err(
                        "--out and --artifact-root are mutually exclusive; --out names the staging directory, --artifact-root composes one"
                            .into(),
                    );
                }
                if out.is_none()
                    && artifact_root.is_none()
                    && env::var_os(ARTIFACT_ROOT_ENV).is_none()
                {
                    return Err(format!(
                        "run dag-coordinate requires --out, --artifact-root, or {ARTIFACT_ROOT_ENV}"
                    ));
                }
            }
            RunCommand::Execute => {
                if artifact_root.is_some() {
                    return Err(
                        "--artifact-root is available only for run dag-coordinate; run execute publishes through --authority-root"
                            .into(),
                    );
                }
                if diagnosis_inputs.is_some() || seed_artifact_root.is_some() {
                    return Err(
                        "run execute does not accept diagnosis-only inputs; use run dag-coordinate for the non-authoritative compatibility primitive"
                            .into(),
                    );
                }
                let authority = authority_root
                    .as_ref()
                    .ok_or("run execute requires --authority-root")?;
                if !authority.is_dir() {
                    return Err(format!(
                        "authority root is not a directory: {}",
                        authority.display()
                    ));
                }
                if final_name.is_none() {
                    return Err("run execute requires --final-name".into());
                }
            }
        }
        // Mode composes after the profile so it can only narrow what the
        // profile permits, and before the doctor overrides so those keep the
        // last word on provider requirements.
        let policy = ExecutionPolicy::for_profile(profile)
            .with_mode(mode)
            .with_skips(skips)
            .with_working_tree(WorkingTreePolicy::parse(&working_tree_policy)?, scopes)
            .with_doctor_overrides(
                doctor_require_repowise,
                doctor_require_understand,
                doctor_tool_path_prefix,
            );
        if command == RunCommand::Execute && out.is_none() {
            return Err("--out is required".into());
        }
        Ok(Self {
            command,
            repo,
            out,
            artifact_root,
            authority_root,
            final_name,
            manifest,
            max_concurrency,
            policy,
            diagnosis_inputs,
            seed_artifact_root,
            session_evidence,
        })
    }
}

fn usage() -> String {
    "usage: run <dag-coordinate|execute> --repo <repo-root> <--out <run-staging-directory> | --artifact-root <root> (dag-coordinate only; also read from CODE_INTEL_ARTIFACT_ROOT)> [--authority-root <publication-root> --final-name <name>] [--profile <default|strict|offline>] [--mode <lite|normal|full>] [--skip-repowise <true|false>] [--skip-sentrux <true|false>] [--require-understand-graph <true|false>] [--manifest <integrations.json>] [--max-concurrency <n>] [--working-tree-policy <head_only|explicit_overlay>] [--scope <relative-path>]... [--session-evidence <session-evidence.json>] [--diagnosis-inputs <artifact-refs.json> --seed-artifact-root <root>] [--doctor-tool-path-prefix <directory>] [--doctor-require-repowise <true|false>] [--doctor-require-understand <true|false>]".into()
}

fn parse_bool_flag(flag: &str, value: &str) -> Result<bool, String> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(format!("{flag} must be true or false")),
    }
}

fn execute_cli(cli: Cli) -> Result<CliResult, RunError> {
    match cli.command {
        RunCommand::DagCoordinate => {
            let out = match cli.out {
                Some(out) => out,
                None => artifacts::compose_dag_staging_dir(cli.artifact_root.as_deref(), &cli.repo)
                    .map_err(|error| {
                        RunError::io(format!("compose DAG staging directory: {error}"))
                    })?,
            };
            let result = dag_run::execute_dag(DagExecutionRequest {
                repo: cli.repo,
                out,
                manifest: cli.manifest,
                max_concurrency: cli.max_concurrency,
                policy: cli.policy,
                diagnosis_inputs: cli.diagnosis_inputs,
                seed_artifact_root: cli.seed_artifact_root,
                session_evidence: cli.session_evidence,
            })?;
            Ok(CliResult {
                output: result.manifest,
                exit_code: result.outcome.exit_code(),
            })
        }
        RunCommand::Execute => {
            let result = execution_kernel::execute(execution_kernel::RunRequest {
                repo: cli.repo,
                staging_root: cli.out.expect("validated execute staging root"),
                authority_root: cli
                    .authority_root
                    .expect("validated execute authority root"),
                final_name: cli.final_name.expect("validated execute final name"),
                manifest: cli.manifest,
                max_concurrency: cli.max_concurrency,
                policy: cli.policy,
                session_evidence: cli.session_evidence,
            })?;
            let exit_code = result.exit_code();
            Ok(CliResult {
                output: result.to_json(),
                exit_code,
            })
        }
    }
}
