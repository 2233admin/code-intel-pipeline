use crate::Result;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

const CAPABILITY_MATRIX_RELATIVE_PATH: &str = "orchestration/sentrux-capability-matrix.v1.json";
const CAPABILITY_MATRIX_SCHEMA: &str = "code-intel-sentrux-capability-matrix.v1";

pub(crate) fn run_capabilities(repo: &Path, json: bool) -> Result<()> {
    let matrix = load_capability_matrix(repo)?;
    let audit = capability_audit(&matrix)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&audit)?);
    } else {
        let coverage = audit
            .get("coverage")
            .ok_or("sentrux capabilities audit omitted coverage")?;
        println!(
            "capability coverage: {} ({}/{} required capabilities covered)",
            coverage["status"].as_str().unwrap_or("unknown"),
            coverage["requiredCovered"].as_u64().unwrap_or(0),
            coverage["required"].as_u64().unwrap_or(0),
        );
        println!("complete: {}", audit["complete"].as_bool().unwrap_or(false));
        for capability in audit["capabilities"]
            .as_array()
            .ok_or("sentrux capabilities audit omitted capabilities")?
        {
            let consumers = capability["decisionConsumers"]
                .as_array()
                .map(|values| {
                    values
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(",")
                })
                .unwrap_or_default();
            println!(
                "- {} operation={} currentState={} route={} decisionConsumers=[{}]",
                capability["id"].as_str().unwrap_or("<invalid>"),
                capability["operation"].as_str().unwrap_or("<invalid>"),
                capability["currentState"].as_str().unwrap_or("<invalid>"),
                capability["route"].as_str().unwrap_or("<invalid>"),
                consumers,
            );
        }
    }
    Ok(())
}

fn load_capability_matrix(repo: &Path) -> Result<Value> {
    let path = repo.join(CAPABILITY_MATRIX_RELATIVE_PATH);
    let text = fs::read_to_string(&path).map_err(|error| {
        format!(
            "sentrux capabilities matrix is missing or unreadable at '{}': {error}",
            path.display()
        )
    })?;
    let matrix = serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|error| {
        format!(
            "sentrux capabilities matrix JSON error at '{}': {error}",
            path.display()
        )
    })?;
    validate_capability_matrix(&matrix, &path)?;
    Ok(matrix)
}

fn capability_audit(matrix: &Value) -> Result<Value> {
    let object = matrix
        .as_object()
        .ok_or("sentrux capabilities matrix schema/header error: root must be an object")?;
    let capabilities = object["capabilities"]
        .as_array()
        .ok_or("sentrux capabilities matrix schema/header error: capabilities must be an array")?;
    let policy = object["completionPolicy"].as_object().ok_or(
        "sentrux capabilities matrix schema/header error: completionPolicy must be an object",
    )?;
    let required_states = policy["requiredStatesForComplete"]
        .as_array()
        .ok_or("sentrux capabilities matrix schema/header error: required completion states must be an array")?
        .iter()
        .filter_map(Value::as_str)
        .collect::<BTreeSet<_>>();
    let automatic_modes = policy["automaticExecutionModes"]
        .as_array()
        .ok_or("sentrux capabilities matrix schema/header error: automatic execution modes must be an array")?
        .iter()
        .filter_map(Value::as_str)
        .collect::<BTreeSet<_>>();
    let explicit_modes = policy["explicitExecutionModes"]
        .as_array()
        .ok_or("sentrux capabilities matrix schema/header error: explicit execution modes must be an array")?
        .iter()
        .filter_map(Value::as_str)
        .collect::<BTreeSet<_>>();
    let forbidden_states = policy["forbiddenSilentStates"]
        .as_array()
        .ok_or("sentrux capabilities matrix schema/header error: forbidden silent states must be an array")?
        .iter()
        .filter_map(Value::as_str)
        .collect::<BTreeSet<_>>();

    let mut state_counts = BTreeMap::<String, u64>::new();
    let mut required = 0u64;
    let mut covered = 0u64;
    let mut required_covered = 0u64;
    let mut output_capabilities = Vec::with_capacity(capabilities.len());

    for capability in capabilities {
        let item = capability.as_object().ok_or(
            "sentrux capabilities matrix schema/header error: capability must be an object",
        )?;
        let current_state = item["currentState"].as_str().ok_or(
            "sentrux capabilities matrix schema/header error: currentState must be a string",
        )?;
        let execution_mode = item["executionMode"].as_str().ok_or(
            "sentrux capabilities matrix schema/header error: executionMode must be a string",
        )?;
        let is_covered = if automatic_modes.contains(execution_mode) {
            required_states.contains(current_state)
        } else if explicit_modes.contains(execution_mode) {
            !forbidden_states.contains(current_state)
        } else {
            false
        };
        *state_counts.entry(current_state.to_string()).or_default() += 1;
        if is_covered {
            covered += 1;
        }
        if item["requiredForRelease"].as_bool().unwrap_or(false) {
            required += 1;
            if is_covered {
                required_covered += 1;
            }
        }
        output_capabilities.push(serde_json::json!({
            "id": item["id"],
            "operation": item["operation"],
            "currentState": item["currentState"],
            "executionMode": item["executionMode"],
            "route": item["route"],
            "decisionConsumers": item["decisionConsumers"],
        }));
    }

    let complete = capabilities
        .iter()
        .filter(|item| item["requiredForRelease"].as_bool().unwrap_or(false))
        .all(|item| {
            let execution_mode = item["executionMode"].as_str().unwrap_or("");
            let state_allowed = item["currentState"]
                .as_str()
                .is_some_and(|state| required_states.contains(state));
            let explicit_state_allowed = item["currentState"]
                .as_str()
                .is_some_and(|state| !forbidden_states.contains(state));
            let has_artifact = item["artifacts"]
                .as_array()
                .is_some_and(|values| !values.is_empty());
            let has_consumer = item["decisionConsumers"]
                .as_array()
                .is_some_and(|values| !values.is_empty());
            ((automatic_modes.contains(execution_mode) && state_allowed)
                || (explicit_modes.contains(execution_mode) && explicit_state_allowed))
                && has_artifact
                && has_consumer
        });

    let coverage = serde_json::json!({
        "status": object["coverageStatus"],
        "total": capabilities.len(),
        "required": required,
        "covered": covered,
        "requiredCovered": required_covered,
        "byState": state_counts,
    });
    Ok(serde_json::json!({
        "schema": "code-intel-sentrux-capability-audit.v1",
        "coverage": coverage,
        "capabilities": output_capabilities,
        "complete": complete,
    }))
}

fn validate_capability_matrix(matrix: &Value, path: &Path) -> Result<()> {
    let object = matrix
        .as_object()
        .ok_or_else(|| matrix_error(path, "root must be an object"))?;
    let schema = object
        .get("schema")
        .and_then(Value::as_str)
        .ok_or_else(|| matrix_error(path, "schema header must be a string"))?;
    if schema != CAPABILITY_MATRIX_SCHEMA {
        return Err(matrix_error(
            path,
            &format!("schema header must be '{CAPABILITY_MATRIX_SCHEMA}', got '{schema}'"),
        ));
    }
    if object.get("contractVersion").and_then(Value::as_u64) != Some(1) {
        return Err(matrix_error(path, "contractVersion header must be 1"));
    }
    if object
        .get("coverageStatus")
        .and_then(Value::as_str)
        .is_none()
    {
        return Err(matrix_error(path, "coverageStatus header must be a string"));
    }

    let policy = object
        .get("completionPolicy")
        .and_then(Value::as_object)
        .ok_or_else(|| matrix_error(path, "completionPolicy header must be an object"))?;
    string_array(
        policy,
        "requiredStatesForComplete",
        path,
        "completionPolicy",
    )?;
    string_array(policy, "automaticExecutionModes", path, "completionPolicy")?;
    string_array(policy, "explicitExecutionModes", path, "completionPolicy")?;
    string_array(policy, "forbiddenSilentStates", path, "completionPolicy")?;
    if policy.get("rule").and_then(Value::as_str).is_none() {
        return Err(matrix_error(path, "completionPolicy.rule must be a string"));
    }

    let capabilities = object
        .get("capabilities")
        .and_then(Value::as_array)
        .ok_or_else(|| matrix_error(path, "capabilities must be an array"))?;
    if capabilities.is_empty() {
        return Err(matrix_error(path, "capabilities must not be empty"));
    }
    let mut ids = BTreeSet::new();
    let mut operations = BTreeSet::new();
    let mut aliases = BTreeSet::new();
    for (index, capability) in capabilities.iter().enumerate() {
        let item = capability.as_object().ok_or_else(|| {
            matrix_error(path, &format!("capabilities[{index}] must be an object"))
        })?;
        for field in ["id", "operation", "executionMode", "currentState", "route"] {
            if item.get(field).and_then(Value::as_str).is_none() {
                return Err(matrix_error(
                    path,
                    &format!("capabilities[{index}].{field} must be a string"),
                ));
            }
        }
        let id = item["id"].as_str().expect("validated capability id");
        if !ids.insert(id) {
            return Err(matrix_error(
                path,
                &format!("duplicate capability id '{id}'"),
            ));
        }
        let operation = item["operation"].as_str().expect("validated operation");
        if !operations.insert(operation) {
            return Err(matrix_error(
                path,
                &format!("duplicate capability operation '{operation}'"),
            ));
        }
        for alias in string_array(item, "aliases", path, &format!("capabilities[{index}]"))? {
            if !aliases.insert(alias.clone()) || ids.contains(alias.as_str()) {
                return Err(matrix_error(
                    path,
                    &format!("duplicate capability alias '{alias}'"),
                ));
            }
        }
        if item
            .get("requiredForRelease")
            .and_then(Value::as_bool)
            .is_none()
        {
            return Err(matrix_error(
                path,
                &format!("capabilities[{index}].requiredForRelease must be a boolean"),
            ));
        }
        for field in ["artifacts", "decisionConsumers"] {
            string_array(item, field, path, &format!("capabilities[{index}]"))?;
        }
    }
    Ok(())
}

fn string_array(
    object: &Map<String, Value>,
    field: &str,
    path: &Path,
    context: &str,
) -> Result<Vec<String>> {
    let values = object.get(field).and_then(Value::as_array).ok_or_else(|| {
        matrix_error(
            path,
            &format!("{context}.{field} must be an array of strings"),
        )
    })?;
    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            value.as_str().map(ToString::to_string).ok_or_else(|| {
                matrix_error(
                    path,
                    &format!("{context}.{field}[{index}] must be a string"),
                )
            })
        })
        .collect()
}

fn matrix_error(path: &Path, message: &str) -> Box<dyn std::error::Error> {
    format!(
        "sentrux capabilities matrix schema/header error at '{}': {message}",
        path.display()
    )
    .into()
}

#[cfg(test)]
mod tests {
    use super::{capability_audit, load_capability_matrix, validate_capability_matrix};
    use serde_json::json;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn temp_repo(label: &str) -> PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "code-intel-sentrux-capabilities-{label}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(path.join("orchestration")).expect("create matrix fixture");
        path
    }

    fn write_matrix(repo: &Path, contents: &str) {
        fs::write(
            repo.join("orchestration/sentrux-capability-matrix.v1.json"),
            contents,
        )
        .expect("write matrix fixture");
    }

    #[test]
    fn current_matrix_reports_partial_coverage_without_claiming_execution() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let repo = manifest_dir
            .parent()
            .and_then(Path::parent)
            .expect("repository root");
        let matrix = load_capability_matrix(repo).expect("load repository matrix");
        let audit = capability_audit(&matrix).expect("render audit");
        assert_eq!(audit["coverage"]["status"], "partial");
        assert_eq!(audit["complete"], false);
        assert!(audit["capabilities"][0].get("id").is_some());
        assert!(audit["capabilities"][0].get("operation").is_some());
        assert!(audit["capabilities"][0].get("currentState").is_some());
        assert!(audit["capabilities"][0].get("route").is_some());
        assert!(audit["capabilities"][0].get("decisionConsumers").is_some());
    }

    #[test]
    fn scan_and_rescan_are_declared_authoritative_automatic() {
        // #375/DR-0009: sentrux.scan and sentrux.rescan were promoted from
        // automatic_degraded to authoritative_automatic once
        // sentrux_gate.rs::metrics_json stopped fabricating
        // unresolved_imports. capability_audit only ever trusts this
        // hand-declared currentState field -- it never executes scan/rescan
        // or re-derives the value from their actual output (DR-0009 traces
        // this in full) -- so nothing else in this codebase protects the
        // promotion decision from a silent revert. Pin it here: a future
        // edit that flips currentState back must fail this test, not just
        // change a JSON file quietly.
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let repo = manifest_dir
            .parent()
            .and_then(Path::parent)
            .expect("repository root");
        let matrix = load_capability_matrix(repo).expect("load repository matrix");
        let audit = capability_audit(&matrix).expect("render audit");
        let capabilities = audit["capabilities"]
            .as_array()
            .expect("capabilities array");
        for id in ["sentrux.scan", "sentrux.rescan"] {
            let entry = capabilities
                .iter()
                .find(|item| item["id"] == id)
                .unwrap_or_else(|| panic!("{id} missing from capability audit"));
            assert_eq!(
                entry["executionMode"], "automatic",
                "{id} executionMode must stay automatic for this test's covered-bit reasoning to hold"
            );
            assert_eq!(
                entry["currentState"], "authoritative_automatic",
                "{id} regressed off authoritative_automatic -- if this is an intentional new decision, update DR-0009 rather than just this assertion"
            );
        }
    }

    #[test]
    fn missing_matrix_is_an_explicit_error() {
        let repo = temp_repo("missing");
        let error = load_capability_matrix(&repo).expect_err("missing matrix must fail");
        assert!(error.to_string().contains("missing or unreadable"));
        let _ = fs::remove_dir_all(repo);
    }

    #[test]
    fn malformed_matrix_json_is_an_explicit_error() {
        let repo = temp_repo("json");
        write_matrix(&repo, "{not-json");
        let error = load_capability_matrix(&repo).expect_err("malformed matrix must fail");
        assert!(error.to_string().contains("JSON error"));
        let _ = fs::remove_dir_all(repo);
    }

    #[test]
    fn invalid_matrix_header_is_an_explicit_error() {
        let repo = temp_repo("header");
        write_matrix(
            &repo,
            &serde_json::to_string(&json!({
                "schema": "wrong-schema",
                "contractVersion": 1,
                "coverageStatus": "partial",
                "completionPolicy": {
                    "requiredStatesForComplete": ["authoritative_automatic"],
                    "automaticExecutionModes": ["automatic"],
                    "explicitExecutionModes": ["explicit_authority", "lifecycle_external"],
                    "forbiddenSilentStates": ["declared_only"],
                    "rule": "rule"
                },
                "capabilities": []
            }))
            .expect("serialize matrix fixture"),
        );
        let error = load_capability_matrix(&repo).expect_err("invalid header must fail");
        assert!(error.to_string().contains("schema/header error"));
        assert!(error.to_string().contains("schema header"));
        let _ = fs::remove_dir_all(repo);
    }

    #[test]
    fn minimal_valid_matrix_obeys_completion_policy_fields() {
        let matrix = json!({
            "schema": "code-intel-sentrux-capability-matrix.v1",
            "contractVersion": 1,
            "coverageStatus": "complete",
            "completionPolicy": {
                "requiredStatesForComplete": ["authoritative_automatic"],
                "automaticExecutionModes": ["automatic"],
                "explicitExecutionModes": ["explicit_authority", "lifecycle_external"],
                "forbiddenSilentStates": ["declared_only"],
                "rule": "rule"
            },
            "capabilities": [{
                "id": "sentrux.example",
                "operation": "example",
                "aliases": [],
                "currentState": "authoritative_automatic",
                "executionMode": "automatic",
                "route": "provider.sentrux-adapt",
                "requiredForRelease": true,
                "artifacts": ["example.v1"],
                "decisionConsumers": ["release_gate"]
            }]
        });
        validate_capability_matrix(&matrix, Path::new("fixture.json"))
            .expect("minimal matrix should validate");
        assert_eq!(
            capability_audit(&matrix).expect("render audit")["complete"],
            true
        );
    }
}
