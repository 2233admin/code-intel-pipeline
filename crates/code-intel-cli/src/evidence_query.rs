use std::collections::BTreeSet;
use std::path::PathBuf;

use serde_json::{json, Value};

use crate::committed_evidence::{self, EvidenceError};

const DEFAULT_LIMIT: usize = 20;
const MAX_LIMIT: usize = 100;
const PREVIEW_CHARS: usize = 400;

pub(crate) fn run_raw(raw: &[String]) -> i32 {
    match Cli::parse(raw).and_then(execute) {
        Ok(result) => {
            println!("{}", serde_json::to_string(&result).unwrap());
            0
        }
        Err(QueryError::Contract(message)) => {
            eprintln!("{message}");
            65
        }
        Err(QueryError::HostIo(message)) => {
            eprintln!("{message}");
            74
        }
    }
}

struct Cli {
    artifact_root: PathBuf,
    repo: String,
    repo_path: Option<PathBuf>,
    artifact_schema: Option<String>,
    artifact_type: Option<String>,
    contains: Option<String>,
    limit: usize,
}

impl Cli {
    fn parse(raw: &[String]) -> Result<Self, QueryError> {
        if raw.first().map(String::as_str) != Some("query") {
            return Err(QueryError::Contract("usage: artifact query --artifact-root <root> --repo <name> [--repo-path <path>] [--artifact-schema <schema>] [--type <artifact-type>] [--contains <text>] [--limit <1..100>]".into()));
        }
        let mut artifact_root = None;
        let mut repo = None;
        let mut repo_path = None;
        let mut artifact_schema = None;
        let mut artifact_type = None;
        let mut contains = None;
        let mut limit = DEFAULT_LIMIT;
        let mut limit_seen = false;
        let mut index = 1;
        while index < raw.len() {
            let flag = raw[index].as_str();
            if !matches!(
                flag,
                "--artifact-root"
                    | "--repo"
                    | "--repo-path"
                    | "--artifact-schema"
                    | "--type"
                    | "--contains"
                    | "--limit"
            ) {
                return Err(QueryError::Contract(format!(
                    "unknown artifact query argument: {flag}"
                )));
            }
            let value = raw
                .get(index + 1)
                .filter(|value| !value.is_empty() && !value.starts_with("--"))
                .ok_or_else(|| QueryError::Contract(format!("{flag} requires one value")))?;
            match flag {
                "--artifact-root" => {
                    set_once(&mut artifact_root, PathBuf::from(value), "--artifact-root")?
                }
                "--repo" => set_once(&mut repo, value.clone(), "--repo")?,
                "--repo-path" => set_once(&mut repo_path, PathBuf::from(value), "--repo-path")?,
                "--artifact-schema" => {
                    set_once(&mut artifact_schema, value.clone(), "--artifact-schema")?
                }
                "--type" => set_once(&mut artifact_type, value.clone(), "--type")?,
                "--contains" => set_once(&mut contains, value.clone(), "--contains")?,
                "--limit" => {
                    if limit_seen {
                        return Err(QueryError::Contract("duplicate --limit".into()));
                    }
                    limit_seen = true;
                    limit = value.parse::<usize>().map_err(|_| {
                        QueryError::Contract("--limit must be an integer in 1..=100".into())
                    })?;
                    if !(1..=MAX_LIMIT).contains(&limit) {
                        return Err(QueryError::Contract(
                            "--limit must be an integer in 1..=100".into(),
                        ));
                    }
                }
                _ => unreachable!(),
            }
            index += 2;
        }
        let artifact_root = artifact_root
            .ok_or_else(|| QueryError::Contract("--artifact-root is required".into()))?;
        if !artifact_root.is_dir() {
            return Err(QueryError::Contract(
                "artifact root must be an existing directory".into(),
            ));
        }
        let repo = repo.ok_or_else(|| QueryError::Contract("--repo is required".into()))?;
        if repo_path.as_ref().is_some_and(|path| !path.is_dir()) {
            return Err(QueryError::Contract(
                "--repo-path must be an existing directory".into(),
            ));
        }
        Ok(Self {
            artifact_root,
            repo,
            repo_path,
            artifact_schema,
            artifact_type,
            contains,
            limit,
        })
    }
}

fn set_once<T>(slot: &mut Option<T>, value: T, flag: &str) -> Result<(), QueryError> {
    if slot.replace(value).is_some() {
        Err(QueryError::Contract(format!("duplicate {flag}")))
    } else {
        Ok(())
    }
}

fn execute(cli: Cli) -> Result<Value, QueryError> {
    let evidence =
        committed_evidence::load(&cli.artifact_root, &cli.repo).map_err(map_evidence_error)?;
    let entry = &evidence.entry;
    let run = entry["run"].as_str().expect("A08 entry run");
    let snapshot_identity = evidence.snapshot_identity();
    let freshness = evidence
        .freshness(cli.repo_path.as_deref())
        .map_err(map_evidence_error)?;
    let run_outcome = entry["outcome"].as_str().expect("A08 entry outcome");
    let needle = cli.contains.as_ref().map(|value| value.to_lowercase());
    let available_artifact_types = evidence
        .refs
        .iter()
        .filter_map(|artifact| artifact["type"].as_str())
        .map(str::to_string)
        .collect::<BTreeSet<_>>();
    let mut contract_matches = 0usize;
    let mut matches = Vec::new();
    for (artifact, verified) in evidence.refs.iter().zip(evidence.verified.iter()) {
        if cli
            .artifact_schema
            .as_ref()
            .is_some_and(|schema| artifact["artifactSchema"] != *schema)
            || cli
                .artifact_type
                .as_ref()
                .is_some_and(|kind| artifact["type"] != *kind)
        {
            continue;
        }
        contract_matches += 1;
        let text = String::from_utf8_lossy(verified.bytes());
        if needle
            .as_ref()
            .is_some_and(|needle| !text.to_lowercase().contains(needle))
        {
            continue;
        }
        let matched_by = matched_by(&cli);
        matches.push(json!({
            "artifactRef":artifact,
            "matchedBy":matched_by,
            "preview":preview(&text),
            "previewTruncated":text.chars().count() > PREVIEW_CHARS,
            "explanation":format!(
                "A07-committed bytes passed registered schema, digest, and snapshot verification; matched {}.",
                matched_by.join(", ")
            ),
        }));
        if matches.len() == cli.limit {
            break;
        }
    }
    let contract_requested = cli.artifact_schema.is_some() || cli.artifact_type.is_some();
    let requested_evidence_status = if !contract_requested {
        "not_requested"
    } else if contract_matches == 0 {
        "unavailable"
    } else {
        "available"
    };
    let mut unknowns = Vec::new();
    if run_outcome != "completed" {
        unknowns.push(format!(
            "authoritative run outcome is {run_outcome}; published artifacts may be partial"
        ));
    }
    if requested_evidence_status == "unavailable" {
        unknowns.push("the requested artifact contract is absent from this run".to_string());
    }
    if freshness["status"] == "stale" {
        unknowns.push("the committed snapshot is stale relative to the requested checkout".into());
    } else if freshness["status"] == "unknown" {
        unknowns.push("freshness is unknown because no checkout was supplied".into());
    }
    let coverage_status =
        if run_outcome == "completed" && requested_evidence_status != "unavailable" {
            "complete"
        } else {
            "partial"
        };
    let confidence = if coverage_status == "complete" && freshness["status"] == "current" {
        "high"
    } else {
        "limited"
    };
    Ok(json!({
        "schema":"code-intel-evidence-query.v1",
        "repo":cli.repo,
        "run":run,
        "runIdentity":entry["runIdentity"],
        "runOutcome":run_outcome,
        "authority":{"status":"committed","indexSchema":"code-intel-artifact-index.v1"},
        "snapshotIdentity":snapshot_identity,
        "freshness":freshness,
        "coverage":{
            "status":coverage_status,
            "availableArtifactTypes":available_artifact_types,
            "requestedEvidenceStatus":requested_evidence_status,
            "unknowns":unknowns,
        },
        "confidence":confidence,
        "query":{
            "artifactSchema":cli.artifact_schema,
            "type":cli.artifact_type,
            "contains":cli.contains,
            "limit":cli.limit,
        },
        "matches":matches,
    }))
}

fn matched_by(cli: &Cli) -> Vec<&'static str> {
    let mut values = Vec::new();
    if cli.artifact_schema.is_some() {
        values.push("artifact_schema");
    }
    if cli.artifact_type.is_some() {
        values.push("artifact_type");
    }
    if cli.contains.is_some() {
        values.push("content");
    }
    if values.is_empty() {
        values.push("all");
    }
    values
}

fn preview(text: &str) -> String {
    text.chars()
        .take(PREVIEW_CHARS)
        .map(|character| {
            if character.is_control() && !matches!(character, '\n' | '\r' | '\t') {
                '\u{fffd}'
            } else {
                character
            }
        })
        .collect()
}

fn map_evidence_error(error: EvidenceError) -> QueryError {
    match error {
        EvidenceError::Contract(message) => QueryError::Contract(message),
        EvidenceError::HostIo(message) => QueryError::HostIo(message),
    }
}

enum QueryError {
    Contract(String),
    HostIo(String),
}
