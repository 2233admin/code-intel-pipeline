use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::project_context::{ProjectContext, ProjectError, ProjectSelector, RunIntent, RunMode};

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

pub(super) fn parse_run_alias_args(raw: &[String]) -> Result<PrimaryArgs, String> {
    if raw.iter().any(|argument| argument == "--artifact-root") {
        return Err(
            "run resolves artifact placement from ProjectContext; --artifact-root is not accepted"
                .into(),
        );
    }
    parse_primary_args(raw)
}

pub(super) fn execute_primary(args: &PrimaryArgs) -> Result<(i32, Value), ProjectError> {
    let context = ProjectContext::resolve(
        ProjectSelector::new(args.repo.clone()).with_artifact_root(args.artifact_root.clone()),
    )?;
    let mode = RunMode::parse(&args.mode)?;
    let answer = context.run(RunIntent::new(mode))?;
    Ok((answer.exit_code(), answer.value().clone()))
}
