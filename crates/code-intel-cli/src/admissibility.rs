use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::artifact_ref::{self, ArtifactContract, ArtifactError};
use crate::capability::{reject_duplicate_json_keys, sha256_hex};

const MAX_REQUEST_BYTES: u64 = 8 * 1024 * 1024;
const MAX_PAYLOAD_BYTES: u64 = 64 * 1024 * 1024;

pub(crate) fn run_raw(raw: &[String]) -> i32 {
    let cli = match parse_cli(raw) {
        Ok(cli) => cli,
        Err(message) => {
            eprintln!("{message}");
            return 64;
        }
    };
    let request = match read_request(&cli.request) {
        Ok(value) => value,
        Err((code, message)) => {
            println!("{}", serde_json::to_string(&rejected(&message)).unwrap());
            eprintln!("{message}");
            return code;
        }
    };
    match validate(&request, &cli.artifact_root) {
        Ok(result) => {
            println!("{}", serde_json::to_string(&result).unwrap());
            0
        }
        Err(error) => {
            let (code, message) = match error {
                ValidationError::Contract(message) => (65, message),
                ValidationError::Io(message) => (74, message),
            };
            println!("{}", serde_json::to_string(&rejected(&message)).unwrap());
            eprintln!("{message}");
            code
        }
    }
}

struct Cli {
    request: PathBuf,
    artifact_root: PathBuf,
}

fn parse_cli(raw: &[String]) -> Result<Cli, String> {
    if raw.first().map(String::as_str) != Some("validate") {
        return Err(
            "usage: evidence validate --request <request.json> --artifact-root <directory>"
                .to_string(),
        );
    }
    let mut request = None;
    let mut artifact_root = None;
    let mut index = 1;
    while index < raw.len() {
        let flag = raw[index].as_str();
        if !matches!(flag, "--request" | "--artifact-root") {
            return Err(format!("unknown evidence argument: {flag}"));
        }
        let value = raw
            .get(index + 1)
            .filter(|v| !v.starts_with("--"))
            .ok_or_else(|| format!("{flag} requires exactly one value"))?;
        let slot = if flag == "--request" {
            &mut request
        } else {
            &mut artifact_root
        };
        if slot.replace(PathBuf::from(value)).is_some() {
            return Err(format!("duplicate evidence argument: {flag}"));
        }
        index += 2;
    }
    Ok(Cli {
        request: request.ok_or("evidence validate requires --request")?,
        artifact_root: artifact_root.ok_or("evidence validate requires --artifact-root")?,
    })
}

fn read_request(path: &Path) -> Result<Value, (i32, String)> {
    let metadata =
        fs::metadata(path).map_err(|e| (74, format!("read evidence request metadata: {e}")))?;
    if !metadata.is_file() {
        return Err((65, "evidence request must be a regular file".to_string()));
    }
    if metadata.len() > MAX_REQUEST_BYTES {
        return Err((65, "evidence request exceeds size limit".to_string()));
    }
    let bytes = fs::read(path).map_err(|e| (74, format!("read evidence request: {e}")))?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|e| (65, format!("evidence request is not UTF-8: {e}")))?;
    reject_duplicate_json_keys(text).map_err(|e| (65, e))?;
    serde_json::from_str(text).map_err(|e| (65, format!("invalid evidence request JSON: {e}")))
}

enum ValidationError {
    Contract(String),
    Io(String),
}

pub(crate) struct ValidatedAdmission {
    result: Value,
    payload: Value,
}

impl ValidatedAdmission {
    pub(crate) fn result(&self) -> &Value {
        &self.result
    }

    pub(crate) fn payload(&self) -> &Value {
        &self.payload
    }
}

pub(crate) fn validate_for_consumer(
    request: &Value,
    root: &Path,
) -> Result<ValidatedAdmission, String> {
    validate_sealed(request, root).map_err(|error| match error {
        ValidationError::Contract(message) | ValidationError::Io(message) => message,
    })
}
impl From<ArtifactError> for ValidationError {
    fn from(value: ArtifactError) -> Self {
        match value {
            ArtifactError::Contract(m) => Self::Contract(m),
            ArtifactError::Io(m) => Self::Io(m),
        }
    }
}

fn validate(request: &Value, root: &Path) -> Result<Value, ValidationError> {
    validate_sealed(request, root).map(|validated| validated.result)
}

fn validate_sealed(request: &Value, root: &Path) -> Result<ValidatedAdmission, ValidationError> {
    validate_request_shape(request).map_err(ValidationError::Contract)?;
    let observation = &request["observation"];
    let expected_snapshot = request["expectedSnapshotIdentity"].as_str().unwrap();
    if observation["consumedSnapshotIdentity"] != request["expectedSnapshotIdentity"] {
        return Err(ValidationError::Contract(
            "observed evidence consumed snapshot mismatch".to_string(),
        ));
    }
    let evaluated_at = request["policy"]["evaluatedAt"].as_u64().unwrap();
    let observed_at = observation["observedAt"].as_u64().unwrap();
    let max_age = request["policy"]["maxAgeSeconds"].as_u64().unwrap();
    if observed_at > evaluated_at || evaluated_at - observed_at > max_age {
        return Err(ValidationError::Contract(
            "observed evidence is stale for freshness policy".to_string(),
        ));
    }
    let artifact = artifact_ref::verify_artifact_ref(
        root,
        expected_snapshot,
        ArtifactContract {
            artifact_schema: "code-intel-evidence-payload.v1",
            artifact_type: "observed.evidence.payload",
            max_bytes: MAX_PAYLOAD_BYTES,
            validate_payload,
        },
        &observation["payload"],
    )?;
    let verdict = if observation["completeness"] == "complete" {
        "observed"
    } else {
        "unknown"
    };
    let admission_identity = sha256_hex(
        &serde_json::to_vec(observation).expect("validated observation always serializes"),
    );
    let payload: Value = serde_json::from_slice(artifact.bytes())
        .expect("the A04 payload validator accepted JSON bytes");
    let result = json!({
        "schema":"code-intel-evidence-admissibility-result.v1",
        "status":"admitted",
        "domainVerdict":verdict,
        "admissionIdentity":admission_identity,
        "evidence":observation,
        "verifiedPayload":{"sha256":artifact.sha256(),"artifactSchema":artifact.artifact_schema(),"type":artifact.artifact_type(),"consumedSnapshotIdentity":artifact.consumed_snapshot_identity(),"data":payload["data"]},
        "engineeringFacts":[]
    });
    Ok(ValidatedAdmission { result, payload })
}

fn validate_request_shape(request: &Value) -> Result<(), String> {
    exact_object(
        request,
        &[
            "schema",
            "expectedSnapshotIdentity",
            "policy",
            "observation",
        ],
        "request",
    )?;
    if request["schema"] != "code-intel-evidence-admissibility-request.v1"
        || !digest(&request["expectedSnapshotIdentity"])
    {
        return Err("evidence request schema/snapshot identity is invalid".to_string());
    }
    let policy = &request["policy"];
    exact_object(
        policy,
        &["evaluatedAt", "maxAgeSeconds"],
        "freshness policy",
    )?;
    if policy["evaluatedAt"].as_u64().is_none()
        || !policy["maxAgeSeconds"].as_u64().is_some_and(|n| n > 0)
    {
        return Err("freshness policy is invalid".to_string());
    }
    let o = &request["observation"];
    exact_object(
        o,
        &[
            "schema",
            "provider",
            "source",
            "consumedSnapshotIdentity",
            "observedAt",
            "completeness",
            "claimedComplete",
            "payload",
            "provenance",
            "failure",
        ],
        "observation",
    )?;
    if o["schema"] != "code-intel-observed-evidence.v1"
        || !digest(&o["consumedSnapshotIdentity"])
        || o["observedAt"].as_u64().is_none()
    {
        return Err("observation identity/time is invalid".to_string());
    }
    validate_provider(&o["provider"])?;
    validate_source(&o["source"])?;
    crate::capability::validate_artifact_ref_shape(&o["payload"])?;
    if o["payload"]["artifactSchema"] != "code-intel-evidence-payload.v1"
        || o["payload"]["type"] != "observed.evidence.payload"
    {
        return Err("evidence payload contract is invalid".to_string());
    }
    validate_provenance(&o["provenance"])?;
    if o["observedAt"] != o["provenance"]["completedAt"] {
        return Err("observation time must equal provenance completion time".to_string());
    }
    let completeness = o["completeness"].as_str().unwrap_or("");
    let claimed = o["claimedComplete"]
        .as_bool()
        .ok_or("claimedComplete must be boolean")?;
    if !matches!(completeness, "complete" | "partial") || claimed != (completeness == "complete") {
        return Err("evidence completeness claim is inconsistent".to_string());
    }
    let failure = &o["failure"];
    let fields = failure.as_object().ok_or("failure must be an object")?;
    if !fields
        .keys()
        .all(|k| matches!(k.as_str(), "kind" | "message"))
        || !fields.contains_key("kind")
    {
        return Err("failure fields are invalid".to_string());
    }
    let kind = failure["kind"].as_str().unwrap_or("");
    if !matches!(
        kind,
        "none" | "provider_unavailable" | "domain_unknown" | "process_failure"
    ) {
        return Err("failure kind is invalid".to_string());
    }
    if completeness == "complete" && kind != "none" {
        return Err("complete evidence cannot report a failure".to_string());
    }
    if kind == "process_failure" {
        return Err("process failure output is not admissible evidence".to_string());
    }
    if kind != "none" && !failure["message"].as_str().is_some_and(|s| !s.is_empty()) {
        return Err("non-none failure requires a message".to_string());
    }
    if kind == "none" && fields.len() != 1 {
        return Err("none failure cannot carry a message".to_string());
    }
    Ok(())
}

fn validate_provider(v: &Value) -> Result<(), String> {
    exact_object(v, &["id", "implementation"], "provider")?;
    nonempty(&v["id"], "provider id")?;
    let implementation = &v["implementation"];
    exact_object(
        implementation,
        &["id", "version", "digest"],
        "provider implementation",
    )?;
    nonempty(&implementation["id"], "implementation id")?;
    nonempty(&implementation["version"], "implementation version")?;
    if !digest(&implementation["digest"]) {
        return Err("implementation digest is invalid".to_string());
    }
    Ok(())
}

fn validate_source(v: &Value) -> Result<(), String> {
    let o = v.as_object().ok_or("source must be an object")?;
    if o.is_empty()
        || !o
            .keys()
            .all(|k| matches!(k.as_str(), "revision" | "endpointIdentity"))
    {
        return Err("source fields are invalid".to_string());
    }
    let identities = ["revision", "endpointIdentity"]
        .iter()
        .filter(|k| v[**k].as_str().is_some_and(|s| !s.is_empty()))
        .count();
    if identities != 1 {
        return Err("source requires exactly one of revision or endpoint identity".to_string());
    }
    if o.values()
        .any(|x| !x.as_str().is_some_and(|s| !s.is_empty()))
    {
        return Err("source identity is invalid".to_string());
    }
    Ok(())
}

fn validate_provenance(v: &Value) -> Result<(), String> {
    exact_object(
        v,
        &["collectionId", "command", "startedAt", "completedAt"],
        "provenance",
    )?;
    nonempty(&v["collectionId"], "collectionId")?;
    nonempty(&v["command"], "command")?;
    let start = v["startedAt"]
        .as_u64()
        .ok_or("provenance startedAt is invalid")?;
    let end = v["completedAt"]
        .as_u64()
        .ok_or("provenance completedAt is invalid")?;
    if end < start {
        return Err("provenance time range is invalid".to_string());
    }
    Ok(())
}

fn validate_payload(bytes: &[u8]) -> Result<(), String> {
    let text =
        std::str::from_utf8(bytes).map_err(|e| format!("evidence payload is not UTF-8: {e}"))?;
    reject_duplicate_json_keys(text)?;
    let value: Value =
        serde_json::from_str(text).map_err(|e| format!("evidence payload is invalid JSON: {e}"))?;
    exact_object(&value, &["schema", "data"], "evidence payload")?;
    if value["schema"] != "code-intel-evidence-payload.v1" || !value["data"].is_object() {
        return Err("evidence payload schema/data is invalid".to_string());
    }
    Ok(())
}

fn exact_object(value: &Value, fields: &[&str], label: &str) -> Result<(), String> {
    let actual = value
        .as_object()
        .ok_or_else(|| format!("{label} must be an object"))?
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let expected = fields.iter().copied().collect::<BTreeSet<_>>();
    if actual != expected {
        return Err(format!("{label} fields are invalid"));
    }
    Ok(())
}
fn nonempty(v: &Value, label: &str) -> Result<(), String> {
    if v.as_str().is_some_and(|s| !s.is_empty()) {
        Ok(())
    } else {
        Err(format!("{label} is invalid"))
    }
}
fn digest(v: &Value) -> bool {
    v.as_str().is_some_and(|s| {
        s.len() == 64
            && s.bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    })
}
fn rejected(message: &str) -> Value {
    json!({"schema":"code-intel-evidence-admissibility-result.v1","status":"rejected","domainVerdict":"unknown","admissionIdentity":null,"evidence":null,"verifiedPayload":null,"engineeringFacts":[],"diagnostics":[message]})
}

#[cfg(test)]
mod tests {
    #[test]
    fn core_source_contains_no_provider_specific_branch_names() {
        let source = include_str!("admissibility.rs").to_ascii_lowercase();
        for forbidden in [
            "repo".to_string() + "wise",
            "sen".to_string() + "trux",
            "code".to_string() + "nexus",
            "gra".to_string() + "ph",
        ] {
            assert!(
                !source.contains(&forbidden),
                "provider-specific name leaked: {forbidden}"
            );
        }
    }
}
