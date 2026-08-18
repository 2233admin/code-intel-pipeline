use std::env;
use std::path::PathBuf;

use serde_json::Value;

use crate::project_context::{EvidenceQuery, ProjectContext, ProjectError, ProjectSelector, Query};

const USAGE: &str = "usage: query [<repo>] --kind evidence [--artifact-schema <schema>] [--type <artifact-type>] [--contains <text>] [--limit <1..100>] --json";
const STATUS_USAGE: &str = "usage: status [<repo>] [--json]";

#[derive(Debug)]
pub(super) struct ProjectQueryArgs {
    repo: PathBuf,
    query: Query,
}

#[derive(Debug)]
pub(super) struct ProjectStatusArgs {
    repo: PathBuf,
    pub(super) json: bool,
}

pub(super) fn parse_project_query_args(raw: &[String]) -> Result<ProjectQueryArgs, String> {
    let mut repo = None;
    let mut kind = None;
    let mut artifact_schema = None;
    let mut artifact_type = None;
    let mut contains = None;
    let mut limit = None;
    let mut json = false;
    let mut index = 0;
    while index < raw.len() {
        match raw[index].as_str() {
            "--kind" | "--artifact-schema" | "--type" | "--contains" | "--limit" => {
                let flag = raw[index].as_str();
                let value = raw
                    .get(index + 1)
                    .filter(|value| !value.is_empty() && !value.starts_with("--"))
                    .ok_or_else(|| format!("{flag} requires one value\n{USAGE}"))?;
                match flag {
                    "--kind" => set_once(&mut kind, value.clone(), flag)?,
                    "--artifact-schema" => set_once(&mut artifact_schema, value.clone(), flag)?,
                    "--type" => set_once(&mut artifact_type, value.clone(), flag)?,
                    "--contains" => set_once(&mut contains, value.clone(), flag)?,
                    "--limit" => {
                        let parsed = value
                            .parse::<usize>()
                            .map_err(|_| "--limit must be an integer in 1..=100".to_string())?;
                        set_once(&mut limit, parsed, flag)?;
                    }
                    _ => unreachable!("project query flags are matched above"),
                }
                index += 2;
            }
            "--json" => {
                if json {
                    return Err("duplicate --json".into());
                }
                json = true;
                index += 1;
            }
            token if token.starts_with('-') => {
                return Err(format!("unknown query argument: {token}\n{USAGE}"));
            }
            token => {
                set_once(&mut repo, PathBuf::from(token), "repository path")?;
                index += 1;
            }
        }
    }
    if kind.as_deref() != Some("evidence") {
        return Err(format!("--kind evidence is required\n{USAGE}"));
    }
    if !json {
        return Err(format!("--json is required\n{USAGE}"));
    }
    let repo = match repo {
        Some(repo) => repo,
        None => {
            env::current_dir().map_err(|error| format!("resolve working directory: {error}"))?
        }
    };
    let query = EvidenceQuery::new(artifact_schema, artifact_type, contains, limit)
        .map_err(|error| error.message().to_string())?;
    Ok(ProjectQueryArgs {
        repo,
        query: Query::Evidence(query),
    })
}

pub(super) fn execute_project_query(arguments: ProjectQueryArgs) -> Result<Value, ProjectError> {
    let context = ProjectContext::resolve(ProjectSelector::new(arguments.repo))?;
    context
        .query(arguments.query)
        .map(|answer| answer.value().clone())
}

pub(super) fn parse_project_status_args(raw: &[String]) -> Result<ProjectStatusArgs, String> {
    let mut repo = None;
    let mut json = false;
    for token in raw {
        match token.as_str() {
            "--json" if !json => json = true,
            "--json" => return Err("duplicate --json".into()),
            token if token.starts_with('-') => {
                return Err(format!("unknown status argument: {token}\n{STATUS_USAGE}"));
            }
            token => {
                if repo.replace(PathBuf::from(token)).is_some() {
                    return Err(format!(
                        "only one repository path may be supplied\n{STATUS_USAGE}"
                    ));
                }
            }
        }
    }
    let repo = repo.unwrap_or(env::current_dir().map_err(|error| error.to_string())?);
    Ok(ProjectStatusArgs { repo, json })
}

pub(super) fn execute_project_status(arguments: &ProjectStatusArgs) -> Result<Value, ProjectError> {
    let context = ProjectContext::resolve(ProjectSelector::new(arguments.repo.clone()))?;
    context.status()
}

pub(super) fn render_project_status(arguments: &ProjectStatusArgs, status: &Value) -> String {
    if arguments.json {
        return format!(
            "{}\n",
            serde_json::to_string(status).expect("project status serializes")
        );
    }

    let state = status["status"].as_str().unwrap_or("unknown");
    let project = &status["project"];
    let mut lines = vec![
        format!(
            "[{}] {}",
            state.to_uppercase(),
            project["repo"].as_str().unwrap_or("")
        ),
        format!("  Path: {}", project["path"].as_str().unwrap_or("")),
        format!(
            "  Freshness: {}",
            status["freshness"]["status"]
                .as_str()
                .unwrap_or("unavailable")
        ),
    ];
    if let Some(run) = status["committedRun"]["run"].as_str() {
        lines.push(format!("  Committed run: {run}"));
        lines.push(format!(
            "  Recorded snapshot: {}",
            status["freshness"]["recordedIdentity"]
                .as_str()
                .unwrap_or("")
        ));
        lines.push(format!(
            "  Current snapshot: {}",
            status["freshness"]["currentIdentity"]
                .as_str()
                .unwrap_or("")
        ));
    }
    if let Some(reason) = status["reason"].as_str() {
        lines.push(format!("  Reason: {reason}"));
    }
    lines.push("Next actions:".into());
    for action in status["nextActions"].as_array().into_iter().flatten() {
        let id = action["id"].as_str().unwrap_or("next");
        let summary = action["summary"].as_str().unwrap_or("");
        lines.push(format!("  {id}: {summary}"));
        if let Some(argv) = action["argv"].as_array() {
            lines.push(format!("    {}", render_argv(argv)));
        } else if let Some(argv) = action["serverArgv"].as_array() {
            lines.push(format!("    start: {}", render_argv(argv)));
            lines.push(format!(
                "    tool: {}",
                action["tool"].as_str().unwrap_or("")
            ));
        }
    }
    format!("{}\n", lines.join("\n"))
}

fn render_argv(argv: &[Value]) -> String {
    argv.iter()
        .filter_map(Value::as_str)
        .map(|argument| {
            if argument.chars().any(char::is_whitespace) {
                format!("\"{}\"", argument.replace('"', "\\\""))
            } else {
                argument.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn set_once<T>(slot: &mut Option<T>, value: T, name: &str) -> Result<(), String> {
    if slot.replace(value).is_some() {
        Err(format!("duplicate {name}"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn parser_keeps_the_first_public_query_kind_closed() {
        let parsed = parse_project_query_args(&argv(&[
            ".",
            "--kind",
            "evidence",
            "--type",
            "diagnosis.hospital",
            "--json",
        ]));
        assert!(parsed.is_ok());
        assert!(
            parse_project_query_args(&argv(&[".", "--kind", "impact", "--json"]))
                .unwrap_err()
                .contains("--kind evidence is required")
        );
    }

    #[test]
    fn parser_refuses_transport_and_placement_overrides() {
        for flag in ["--artifact-root", "--repo", "--run"] {
            let error = parse_project_query_args(&argv(&[
                ".",
                "--kind",
                "evidence",
                flag,
                "elsewhere",
                "--json",
            ]))
            .unwrap_err();
            assert!(error.contains("unknown query argument"), "{flag}: {error}");
        }
    }

    #[test]
    fn status_parser_accepts_one_repository_and_an_optional_json_flag() {
        let parsed = parse_project_status_args(&argv(&["repository", "--json"]))
            .expect("status arguments parse");
        assert_eq!(parsed.repo, PathBuf::from("repository"));
        assert!(parsed.json);
    }

    #[test]
    fn status_parser_rejects_placement_overrides_and_duplicate_inputs() {
        for arguments in [
            argv(&["--artifact-root", "elsewhere"]),
            argv(&["one", "two"]),
            argv(&["--json", "--json"]),
        ] {
            assert!(
                parse_project_status_args(&arguments).is_err(),
                "{arguments:?}"
            );
        }
    }
}
