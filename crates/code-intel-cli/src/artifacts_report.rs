use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::run_commit;

use super::{absolute_existing_dir, resolve_artifact_root, Result};

#[derive(Debug)]
struct ReportArtifact {
    schema: String,
    artifact_type: String,
    path: PathBuf,
}

impl ReportArtifact {
    fn to_json(&self) -> Value {
        serde_json::json!({
            "schema": self.schema,
            "type": self.artifact_type,
            "path": self.path,
        })
    }
}

pub(super) fn report(repo: &Path, artifact_root: Option<&Path>, json: bool) -> Result<()> {
    let repo = absolute_existing_dir(repo)?;
    let artifact_root = resolve_artifact_root(artifact_root)?;
    let repo_name = repo
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("repo path has no final directory name")?;
    let repo_artifacts = artifact_root.join(repo_name);
    let (run_root, manifest) = latest_committed_run(&repo_artifacts)?;
    let hospital = required_report_artifact(&run_root, &manifest, "diagnosis.hospital")?;
    let hospital_markdown =
        required_report_artifact(&run_root, &manifest, "diagnosis.hospital-view")?;
    let hospital_text = fs::read_to_string(&hospital_markdown.path)?;
    let agent_code_slice_ranking =
        optional_report_artifact(&run_root, &manifest, "code_evidence.agent_slice")?;

    let out = serde_json::json!({
        "schema": "code-intel-report.v1",
        "repo": repo,
        "run": run_root.file_name().and_then(|name| name.to_str()),
        "hospital": hospital.to_json(),
        "hospitalMarkdown": hospital_markdown.to_json(),
        "agentCodeSliceRanking": agent_code_slice_ranking.as_ref().map(ReportArtifact::to_json),
    });
    if json {
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        println!("Code Intel Report");
        println!("repo: {}", repo.display());
        println!("run: {}", out["run"].as_str().unwrap_or(""));
        println!("hospital: {}", hospital.path.display());
        println!("hospitalMarkdown: {}", hospital_markdown.path.display());
        if let Some(ranking) = &agent_code_slice_ranking {
            println!("agentCodeSliceRanking: {}", ranking.path.display());
        }
        println!();
        println!("--- hospital.md ---");
        print!("{hospital_text}");
        if !hospital_text.ends_with('\n') {
            println!();
        }
    }
    Ok(())
}

fn required_report_artifact(
    run_root: &Path,
    manifest: &Value,
    artifact_type: &str,
) -> Result<ReportArtifact> {
    optional_report_artifact(run_root, manifest, artifact_type)?.ok_or_else(|| {
        format!(
            "committed run {} has no `{artifact_type}` Artifact Ref",
            run_root.display()
        )
        .into()
    })
}

fn optional_report_artifact(
    run_root: &Path,
    manifest: &Value,
    artifact_type: &str,
) -> Result<Option<ReportArtifact>> {
    let reference = manifest["nodes"]
        .as_object()
        .into_iter()
        .flat_map(|nodes| nodes.values())
        .flat_map(|node| node["artifacts"].as_array().into_iter().flatten())
        .find(|reference| reference["type"].as_str() == Some(artifact_type));
    let Some(reference) = reference else {
        return Ok(None);
    };
    let path = reference["path"].as_str().ok_or_else(|| {
        format!(
            "Artifact Ref `{artifact_type}` has no path in {}",
            run_root.display()
        )
    })?;
    let path = run_root.join(path);
    if !path.is_file() {
        return Err(format!(
            "Artifact Ref `{artifact_type}` points to missing file {}",
            path.display()
        )
        .into());
    }
    Ok(Some(ReportArtifact {
        schema: reference["artifactSchema"]
            .as_str()
            .unwrap_or("unknown")
            .to_string(),
        artifact_type: artifact_type.to_string(),
        path,
    }))
}

fn latest_committed_run(repo_artifacts: &Path) -> Result<(PathBuf, Value)> {
    if !repo_artifacts.is_dir() {
        return Err(format!(
            "no artifact directory for repository under {}",
            repo_artifacts.display()
        )
        .into());
    }
    let mut dirs = Vec::new();
    for entry in fs::read_dir(repo_artifacts)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            dirs.push(entry.path());
        }
    }
    dirs.sort();
    let mut rejection = None;
    for path in dirs.iter().rev() {
        match run_commit::validate_committed_run(path) {
            Ok((_marker, manifest)) => return Ok((path.clone(), manifest)),
            Err(error) => rejection = Some(format!("{}: {error}", path.display())),
        }
    }
    Err(format!(
        "no committed A07 run under {}; latest candidate rejected: {}",
        repo_artifacts.display(),
        rejection.unwrap_or_else(|| "no run directories found".to_string())
    )
    .into())
}
