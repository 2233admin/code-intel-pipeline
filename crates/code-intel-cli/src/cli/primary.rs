use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::{artifacts, authoritative_run, execution_policy, run_error};

#[derive(Debug, PartialEq, Eq)]
pub(super) struct PrimaryArgs {
    pub(super) repo: PathBuf,
    pub(super) mode: String,
    pub(super) artifact_root: Option<PathBuf>,
    pub(super) json: bool,
}

pub(super) fn matches_primary_pattern(raw: &[String]) -> bool {
    raw.is_empty()
        || raw.first().is_some_and(|first| {
            !first.starts_with('-')
                && (Path::new(first).is_dir()
                    || Path::new(first).is_absolute()
                    || first.contains('/')
                    || first.contains('\\')
                    || first.starts_with("__"))
        })
        || matches!(
            raw.first().map(String::as_str),
            Some("--mode" | "--artifact-root" | "--json")
        )
}

pub(super) fn parse_primary_args(raw: &[String]) -> Result<PrimaryArgs, String> {
    let mut repo = None;
    let mut mode = "normal".to_string();
    let mut artifact_root = None;
    let mut json = false;
    let mut index = 0;
    while index < raw.len() {
        match raw[index].as_str() {
            "--mode" | "--artifact-root" => {
                let flag = raw[index].as_str();
                let value = raw
                    .get(index + 1)
                    .filter(|value| !value.is_empty() && !value.starts_with("--"))
                    .ok_or_else(|| format!("{flag} requires one value"))?;
                if flag == "--mode" {
                    if !matches!(value.as_str(), "lite" | "normal" | "full") {
                        return Err("--mode must be lite, normal, or full".into());
                    }
                    mode = value.clone();
                } else if artifact_root.replace(PathBuf::from(value)).is_some() {
                    return Err("duplicate --artifact-root".into());
                }
                index += 2;
            }
            "--json" => {
                json = true;
                index += 1;
            }
            token if token.starts_with('-') => {
                return Err(format!("unknown primary entry argument: {token}"));
            }
            token => {
                if repo.replace(PathBuf::from(token)).is_some() {
                    return Err("only one repository path may be supplied".into());
                }
                index += 1;
            }
        }
    }
    let repo = repo.unwrap_or(env::current_dir().map_err(|error| error.to_string())?);
    if !repo.is_dir() {
        return Err(format!(
            "repository path is not a directory: {}",
            repo.display()
        ));
    }
    Ok(PrimaryArgs {
        repo: fs::canonicalize(repo).map_err(|error| error.to_string())?,
        mode,
        artifact_root,
        json,
    })
}

pub(super) fn execute_primary(args: &PrimaryArgs) -> Result<(i32, Value), run_error::RunError> {
    let artifact_root = artifacts::resolve_artifact_root(args.artifact_root.as_deref())
        .map_err(|error| run_error::RunError::io(error.to_string()))?;
    fs::create_dir_all(&artifact_root)
        .map_err(|error| run_error::RunError::io(format!("create artifact root: {error}")))?;
    let repo_name = args
        .repo
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| run_error::RunError::contract("repository has no usable name"))?;
    let authority_root = artifact_root.join(repo_name);
    fs::create_dir_all(&authority_root).map_err(|error| {
        run_error::RunError::io(format!("create repository authority root: {error}"))
    })?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| run_error::RunError::io(error.to_string()))?
        .as_millis();
    let final_name = format!("{nonce}-{}-core", process::id());
    let staging_root = env::temp_dir().join(format!("code-intel-a09-{final_name}"));
    let profile = match args.mode.as_str() {
        "lite" => execution_policy::RunProfile::Offline,
        "normal" => execution_policy::RunProfile::Default,
        "full" => execution_policy::RunProfile::Strict,
        _ => unreachable!("primary mode is validated"),
    };
    let request = authoritative_run::RunRequest::new(
        args.repo.clone(),
        staging_root,
        authority_root,
        final_name,
        execution_policy::ExecutionPolicy::for_profile(profile),
    )
    .with_staging_cleanup();
    let result = authoritative_run::execute(request)?;
    let output = primary_result(args, &result);
    Ok((result.exit_code(), output))
}

fn primary_result(args: &PrimaryArgs, result: &authoritative_run::ProductionRunResult) -> Value {
    let (failure_node, diagnostic) = first_failure(result.manifest())
        .map(|(node, diagnostic)| (Value::String(node), Value::String(diagnostic)))
        .unwrap_or((Value::Null, Value::Null));
    json!({
        "schema": "code-intel-primary-result.v1",
        "repo": args.repo,
        "mode": args.mode,
        "outcome": result.outcome(),
        "exitCode": result.exit_code(),
        "publication": {
            "path": result.publication_path(),
            "marker": result.publication_path().join("run-complete.json"),
        },
        "failureNode": failure_node,
        "diagnostic": diagnostic,
        "failures": result.failures(),
        "anchors": result.anchors(),
    })
}

fn first_failure(manifest: &Value) -> Option<(String, String)> {
    manifest["nodes"]
        .as_object()?
        .iter()
        .find_map(|(node, value)| {
            matches!(
                value["status"].as_str(),
                Some("process_failed" | "domain_failed" | "domain_unknown")
            )
            .then(|| {
                (
                    node.clone(),
                    value["diagnostic"]
                        .as_str()
                        .or_else(|| value["failure"].as_str())
                        .unwrap_or("")
                        .to_string(),
                )
            })
        })
}
