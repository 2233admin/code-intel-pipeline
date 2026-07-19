use crate::Result;
use serde_json::{json, Value};
use std::fs;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

#[path = "compete.rs"]
mod compete;
#[path = "react_doctor.rs"]
mod react_doctor;
#[path = "sha256.rs"]
mod sha256;

pub(crate) const ARTIFACT_REF_SCHEMA: &str = "code-intel-artifact-ref.v1";
pub(crate) const ROUTE_SCHEMA: &str = "code-intel-evidence-route-result.v1";

pub(crate) struct Context {
    pub(crate) artifact_root: PathBuf,
    pub(crate) evaluated_at: i64,
    pub(crate) max_age_seconds: i64,
}

pub(crate) struct EvidenceError {
    pub(crate) category: &'static str,
    pub(crate) reason: String,
}

impl EvidenceError {
    pub(crate) fn local(reason: impl Into<String>) -> Self {
        Self {
            category: "local_tool_error",
            reason: reason.into(),
        }
    }

    pub(crate) fn mismatch(reason: impl Into<String>) -> Self {
        Self {
            category: "snapshot_mismatch",
            reason: reason.into(),
        }
    }
}

pub(crate) fn adapt(
    provider: &str,
    request_path: &Path,
    artifact_root: &Path,
    evaluated_at: i64,
    max_age_seconds: i64,
) -> Result<Value> {
    let context = match build_context(artifact_root, evaluated_at, max_age_seconds) {
        Ok(value) => value,
        Err(error) => {
            return Ok(rejected(
                provider,
                None,
                error.category,
                &error.reason,
                evaluated_at,
            ));
        }
    };
    let request = match read_request(request_path) {
        Ok(value) => value,
        Err(error) => {
            return Ok(rejected(
                provider,
                None,
                "local_tool_error",
                &format!("invalid native result JSON: {error}"),
                evaluated_at,
            ));
        }
    };

    Ok(match provider {
        "compete" => compete::adapt(&request, &context),
        "react-doctor" => react_doctor::adapt(&request, &context),
        _ => rejected(
            provider,
            snapshot_identity(&request),
            "local_tool_error",
            "unknown evidence provider",
            evaluated_at,
        ),
    })
}

fn build_context(
    artifact_root: &Path,
    evaluated_at: i64,
    max_age_seconds: i64,
) -> std::result::Result<Context, EvidenceError> {
    if evaluated_at < 0 {
        return Err(EvidenceError::local(
            "--evaluated-at must be a non-negative Unix timestamp",
        ));
    }
    if max_age_seconds < 0 {
        return Err(EvidenceError::local(
            "--max-age-seconds must be non-negative",
        ));
    }
    let artifact_root = artifact_root
        .canonicalize()
        .map_err(|error| EvidenceError::local(format!("invalid artifact root: {error}")))?;
    if !artifact_root.is_dir() {
        return Err(EvidenceError::local("artifact root is not a directory"));
    }
    Ok(Context {
        artifact_root,
        evaluated_at,
        max_age_seconds,
    })
}

fn read_request(path: &Path) -> Result<Value> {
    let mut text = String::new();
    if path == Path::new("-") {
        io::stdin().read_to_string(&mut text)?;
    } else {
        text = fs::read_to_string(path)?;
    }
    Ok(serde_json::from_str(text.trim_start_matches('\u{feff}'))?)
}

pub(crate) fn validate_native<'a>(
    request: &'a Value,
    expected_schema: &str,
    context: &Context,
) -> std::result::Result<(&'a str, &'a str), EvidenceError> {
    if request.get("schema").and_then(Value::as_str) != Some(expected_schema) {
        return Err(EvidenceError::local(format!(
            "expected native schema {expected_schema}"
        )));
    }
    let snapshot = snapshot_identity(request)
        .ok_or_else(|| EvidenceError::local("missing or invalid snapshotIdentity"))?;
    let observed_at = request
        .get("observedAt")
        .and_then(Value::as_i64)
        .ok_or_else(|| EvidenceError::local("missing observedAt Unix timestamp"))?;
    if observed_at > context.evaluated_at {
        return Err(EvidenceError::local("observedAt is later than evaluatedAt"));
    }
    let status = request
        .get("status")
        .and_then(Value::as_str)
        .ok_or_else(|| EvidenceError::local("missing native status"))?;
    if status == "completed" && context.evaluated_at - observed_at > context.max_age_seconds {
        return Err(EvidenceError {
            category: "stale_evidence",
            reason: "native result exceeded max age".to_string(),
        });
    }
    Ok((snapshot, status))
}

pub(crate) fn validate_artifact_ref(
    artifact: &Value,
    snapshot: &str,
    context: &Context,
) -> std::result::Result<PathBuf, EvidenceError> {
    if artifact.get("schema").and_then(Value::as_str) != Some(ARTIFACT_REF_SCHEMA) {
        return Err(EvidenceError::local("invalid Artifact Ref schema"));
    }
    for key in ["artifactSchema", "type", "path", "sha256"] {
        if artifact
            .get(key)
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        {
            return Err(EvidenceError::local(format!("Artifact Ref missing {key}")));
        }
    }
    if artifact
        .get("consumedSnapshotIdentity")
        .and_then(Value::as_str)
        != Some(snapshot)
    {
        return Err(EvidenceError::mismatch(
            "Artifact Ref consumed a different snapshot",
        ));
    }
    let relative = Path::new(artifact["path"].as_str().unwrap_or_default());
    if relative.is_absolute()
        || relative
            .components()
            .any(|part| matches!(part, Component::ParentDir | Component::Prefix(_)))
    {
        return Err(EvidenceError::local(
            "Artifact Ref path escapes artifact root",
        ));
    }
    let resolved = context
        .artifact_root
        .join(relative)
        .canonicalize()
        .map_err(|error| EvidenceError::local(format!("artifact is unavailable: {error}")))?;
    if !resolved.starts_with(&context.artifact_root) || !resolved.is_file() {
        return Err(EvidenceError::local(
            "Artifact Ref path escapes artifact root",
        ));
    }
    let expected = artifact["sha256"].as_str().unwrap_or_default();
    if !is_sha256(expected) {
        return Err(EvidenceError::local("Artifact Ref has invalid SHA-256"));
    }
    let actual = sha256::file_hex(&resolved)
        .map_err(|error| EvidenceError::local(format!("cannot hash artifact: {error}")))?;
    if actual != expected {
        return Err(EvidenceError::local("Artifact Ref SHA-256 mismatch"));
    }
    Ok(resolved)
}

pub(crate) fn load_json(path: &Path) -> std::result::Result<Value, EvidenceError> {
    let text = fs::read_to_string(path)
        .map_err(|error| EvidenceError::local(format!("cannot read JSON artifact: {error}")))?;
    serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .map_err(|error| EvidenceError::local(format!("corrupt JSON artifact: {error}")))
}

pub(crate) fn snapshot_identity(request: &Value) -> Option<&str> {
    request
        .get("snapshotIdentity")
        .and_then(Value::as_str)
        .filter(|value| is_sha256(value))
}

pub(crate) fn status_route(
    provider: &str,
    snapshot: Option<&str>,
    state: &str,
    verdict: &str,
    failure_category: Option<&str>,
    reason: &str,
    context: &Context,
    artifacts: Vec<Value>,
    evidence: Value,
) -> Value {
    json!({
        "schema": ROUTE_SCHEMA,
        "provider": provider,
        "operation": "adapt",
        "snapshotIdentity": snapshot,
        "admissibility": {
            "admitted": state != "rejected",
            "reason": reason
        },
        "status": state,
        "verdict": verdict,
        "advisoryOnly": true,
        "failureCategory": failure_category,
        "evaluatedAt": context.evaluated_at,
        "maxAgeSeconds": context.max_age_seconds,
        "artifacts": artifacts,
        "evidence": evidence
    })
}

pub(crate) fn rejected(
    provider: &str,
    snapshot: Option<&str>,
    category: &str,
    reason: &str,
    evaluated_at: i64,
) -> Value {
    json!({
        "schema": ROUTE_SCHEMA,
        "provider": provider,
        "operation": "adapt",
        "snapshotIdentity": snapshot,
        "admissibility": {"admitted": false, "reason": reason},
        "status": "rejected",
        "verdict": "unknown",
        "advisoryOnly": true,
        "failureCategory": category,
        "evaluatedAt": evaluated_at.max(0),
        "maxAgeSeconds": Value::Null,
        "artifacts": [],
        "evidence": Value::Null
    })
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
#[path = "evidence_tests.rs"]
mod tests;
