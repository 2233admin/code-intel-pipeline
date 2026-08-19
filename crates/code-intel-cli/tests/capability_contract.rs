//! Port of `legacy/scripts/tests/test-atomic-capability-contract.ps1`.
//!
//! The PowerShell host stays on disk so deleting it cannot trip the coupling
//! ratchet. These tests are the authoritative lock for the contract document,
//! registry digest pairing, envelope fixtures, and cross-envelope coherence.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

#[path = "support/sha256.rs"]
mod sha256_support;

const EXPECTED_VOCABULARY: [&str; 7] = [
    "Capability Atom",
    "Snapshot Identity",
    "Artifact Ref",
    "Effect Boundary",
    "Domain Verdict",
    "Run Commit",
    "Materialized View",
];
const EXPECTED_VERDICTS: [&str; 4] = ["pass", "fail", "unknown", "not_applicable"];
const EXPECTED_EFFECTS: [&str; 4] = ["repo_read", "local_write", "network", "repo_mutation"];
const EXPECTED_EXIT_CODES: [u64; 8] = [0, 10, 20, 64, 65, 69, 70, 74];
const DOCUMENTATION: [&str; 4] = [
    "CONTEXT.md",
    "docs/atomic-development-model.md",
    "docs/adr/0009-atomic-capability-execution-model.md",
    "docs/code-intel-architecture.md",
];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crate lives under <repo>/crates")
        .to_path_buf()
}

fn read_json(relative: &str) -> Value {
    let path = repo_root().join(relative);
    serde_json::from_slice(&fs::read(&path).unwrap_or_else(|error| {
        panic!("read {}: {error}", path.display());
    }))
    .unwrap_or_else(|error| panic!("parse {relative}: {error}"))
}

fn string_list<'a>(value: &'a Value, label: &str) -> Vec<&'a str> {
    value
        .as_array()
        .unwrap_or_else(|| panic!("{label} must be an array"))
        .iter()
        .map(|item| {
            item.as_str()
                .unwrap_or_else(|| panic!("{label} entries must be strings"))
        })
        .collect()
}

fn is_sha256(value: &Value) -> bool {
    value
        .as_str()
        .is_some_and(|text| text.len() == 64 && text.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn closed_object(value: &Value, required: &[&str]) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    required.iter().all(|key| object.contains_key(*key))
        && object.keys().all(|key| required.contains(&key.as_str()))
}

fn valid_effect_set(value: &Value) -> bool {
    let Some(items) = value.as_array() else {
        return false;
    };
    let mut seen = Vec::new();
    items.iter().all(|item| {
        let Some(effect) = item.as_str() else {
            return false;
        };
        let known = matches!(
            effect,
            "repo_read" | "local_write" | "process_spawn" | "network" | "repo_mutation"
        );
        let unique = !seen.contains(&effect);
        seen.push(effect);
        known && unique
    })
}

fn valid_implementation(value: &Value) -> bool {
    closed_object(value, &["id", "version", "toolchainDigests"])
        && value["id"].as_str().is_some_and(|id| !id.is_empty())
        && value["version"]
            .as_str()
            .is_some_and(|version| !version.is_empty())
        && value["toolchainDigests"]
            .as_array()
            .is_some_and(|digests| digests.iter().all(is_sha256))
}

fn valid_artifact_ref(value: &Value) -> bool {
    closed_object(
        value,
        &[
            "schema",
            "artifactSchema",
            "type",
            "path",
            "sha256",
            "consumedSnapshotIdentity",
        ],
    ) && value["schema"] == "code-intel-artifact-ref.v1"
        && value["artifactSchema"]
            .as_str()
            .is_some_and(|schema| !schema.is_empty())
        && value["type"].as_str().is_some_and(|kind| !kind.is_empty())
        && value["path"].as_str().is_some_and(|path| !path.is_empty())
        && is_sha256(&value["sha256"])
        && (value["consumedSnapshotIdentity"].is_null()
            || is_sha256(&value["consumedSnapshotIdentity"]))
}

fn valid_snapshot(value: &Value) -> bool {
    closed_object(
        value,
        &[
            "identity",
            "repoIdentity",
            "head",
            "workingTreePolicy",
            "scope",
            "inputDigest",
        ],
    ) && is_sha256(&value["identity"])
        && value["repoIdentity"]
            .as_str()
            .is_some_and(|identity| !identity.is_empty())
        && value["head"].as_str().is_some_and(|head| !head.is_empty())
        && matches!(
            value["workingTreePolicy"].as_str(),
            Some("head_only") | Some("explicit_overlay")
        )
        && value["scope"].as_array().is_some_and(|scope| {
            scope
                .iter()
                .all(|item| item.as_str().is_some_and(|path| !path.is_empty()))
        })
        && is_sha256(&value["inputDigest"])
}

fn schema_accepts(value: &Value) -> bool {
    match value["schema"].as_str() {
        Some("code-intel-artifact-ref.v1") => valid_artifact_ref(value),
        Some("code-intel-capability-declaration.v1") => {
            closed_object(
                value,
                &[
                    "schema",
                    "id",
                    "contractVersion",
                    "implementation",
                    "determinism",
                    "allowedEffects",
                    "dependencies",
                ],
            ) && valid_implementation(&value["implementation"])
                && matches!(
                    value["determinism"].as_str(),
                    Some("deterministic") | Some("external_nondeterministic")
                )
                && valid_effect_set(&value["allowedEffects"])
                && value["dependencies"].is_array()
        }
        Some("code-intel-capability-request.v1") => {
            closed_object(
                value,
                &[
                    "schema",
                    "capability",
                    "contractVersion",
                    "implementation",
                    "snapshot",
                    "options",
                    "inputs",
                    "effectPolicy",
                ],
            ) && valid_implementation(&value["implementation"])
                && valid_snapshot(&value["snapshot"])
                && value["options"].is_object()
                && value["inputs"]
                    .as_array()
                    .is_some_and(|inputs| inputs.iter().all(valid_artifact_ref))
                && closed_object(&value["effectPolicy"], &["allowedEffects"])
                && valid_effect_set(&value["effectPolicy"]["allowedEffects"])
        }
        Some("code-intel-capability-result.v1") => {
            closed_object(
                value,
                &[
                    "schema",
                    "capability",
                    "implementation",
                    "snapshotIdentity",
                    "status",
                    "verdict",
                    "domainVerdict",
                    "exitCode",
                    "determinism",
                    "declaredEffects",
                    "observedEffects",
                    "cache",
                    "artifacts",
                    "diagnostics",
                    "provenance",
                ],
            ) && valid_implementation(&value["implementation"])
                && is_sha256(&value["snapshotIdentity"])
                && valid_effect_set(&value["declaredEffects"])
                && valid_effect_set(&value["observedEffects"])
                && value["artifacts"]
                    .as_array()
                    .is_some_and(|artifacts| artifacts.iter().all(valid_artifact_ref))
                && value["diagnostics"].is_array()
                && result_outcome_allowed_by_schema(value)
        }
        _ => false,
    }
}

fn result_outcome_allowed_by_schema(value: &Value) -> bool {
    let status = value["status"].as_str();
    let verdict = value["verdict"].as_str();
    let domain = value["domainVerdict"].as_str();
    let exit = value["exitCode"].as_u64();
    matches!(
        (status, verdict, domain, exit),
        (
            Some("completed"),
            Some("pass") | Some("not_applicable"),
            Some("pass") | Some("unknown") | Some("not_applicable"),
            Some(0)
        ) | (Some("completed"), Some("fail"), Some("fail"), Some(10))
            | (Some("blocked"), Some("unknown"), Some("unknown"), Some(20))
            | (
                Some("failed"),
                Some("unknown"),
                Some("unknown"),
                Some(64 | 65 | 69 | 70 | 74)
            )
    )
}

fn fixture_set() -> (Value, Value, Value, Value) {
    let digest_a = "a".repeat(64);
    let digest_b = "b".repeat(64);
    let implementation = json!({
        "id": "inventory.rg",
        "version": "1.0.0",
        "toolchainDigests": [digest_a]
    });
    let artifact = json!({
        "schema": "code-intel-artifact-ref.v1",
        "artifactSchema": "code-intel-file-inventory.v1",
        "type": "inventory.files",
        "path": "artifacts/inventory.json",
        "sha256": digest_b,
        "consumedSnapshotIdentity": digest_a
    });
    let declaration = json!({
        "schema": "code-intel-capability-declaration.v1",
        "id": "inventory.rg",
        "contractVersion": 1,
        "implementation": implementation,
        "determinism": "deterministic",
        "allowedEffects": ["repo_read", "local_write"],
        "dependencies": []
    });
    let request = json!({
        "schema": "code-intel-capability-request.v1",
        "capability": "inventory.rg",
        "contractVersion": 1,
        "implementation": implementation,
        "snapshot": {
            "identity": digest_a,
            "repoIdentity": "github.com/2233admin/code-intel-pipeline",
            "head": "0123456789abcdef0123456789abcdef01234567",
            "workingTreePolicy": "head_only",
            "scope": ["."],
            "inputDigest": digest_a
        },
        "options": {},
        "inputs": [artifact],
        "effectPolicy": { "allowedEffects": ["repo_read", "local_write"] }
    });
    let result = json!({
        "schema": "code-intel-capability-result.v1",
        "capability": "inventory.rg",
        "implementation": implementation,
        "snapshotIdentity": digest_a,
        "status": "completed",
        "verdict": "pass",
        "domainVerdict": "pass",
        "exitCode": 0,
        "determinism": "deterministic",
        "declaredEffects": ["repo_read", "local_write"],
        "observedEffects": ["repo_read", "local_write"],
        "cache": { "key": digest_b, "hit": false },
        "artifacts": [artifact],
        "diagnostics": [],
        "provenance": {
            "attemptId": "fixture-1",
            "generatedAt": "2026-07-13T00:00:00Z"
        }
    });
    (artifact, declaration, request, result)
}

fn string_set(value: &Value) -> Vec<String> {
    value
        .as_array()
        .unwrap_or(&Vec::new())
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

fn envelopes_coherent(declaration: &Value, request: &Value, result: &Value) -> Result<(), String> {
    if request["capability"] != declaration["id"] {
        return Err("request capability differs from declaration id".into());
    }
    if request["implementation"] != declaration["implementation"] {
        return Err("request implementation differs from declaration".into());
    }
    let declared = string_set(&declaration["allowedEffects"]);
    let requested = string_set(&request["effectPolicy"]["allowedEffects"]);
    if requested.iter().any(|effect| !declared.contains(effect)) {
        return Err("request asks for effects outside declaration".into());
    }
    if result["capability"] != request["capability"] {
        return Err("result capability differs from request".into());
    }
    if result["implementation"] != request["implementation"] {
        return Err("result implementation differs from request".into());
    }
    if result["determinism"] != declaration["determinism"] {
        return Err("result determinism differs from declaration".into());
    }
    if string_set(&result["declaredEffects"]) != requested {
        return Err("result declared effects differ from request".into());
    }
    let observed = string_set(&result["observedEffects"]);
    if observed
        .iter()
        .any(|effect| !string_set(&result["declaredEffects"]).contains(effect))
    {
        return Err("observed undeclared effect".into());
    }
    if result["snapshotIdentity"] != request["snapshot"]["identity"] {
        return Err("result snapshot identity differs from request".into());
    }
    for artifact in result["artifacts"].as_array().unwrap_or(&Vec::new()) {
        if !artifact["consumedSnapshotIdentity"].is_null()
            && artifact["consumedSnapshotIdentity"] != result["snapshotIdentity"]
        {
            return Err("output artifact consumed snapshot differs from result".into());
        }
    }
    Ok(())
}

#[test]
fn capability_contract_vocabulary_and_bindings_are_stable() {
    let contract = read_json("orchestration/capability-contract.v1.json");
    let registry = read_json("orchestration/integrations.json");
    assert_eq!(contract["schema"], "code-intel-capability-contract.v1");
    assert_eq!(contract["contractVersion"], 1);
    assert_eq!(
        contract["envelopeSchema"],
        "orchestration/schemas/code-intel-capability-envelope.v1.schema.json"
    );
    assert_eq!(
        registry["policy"]["capabilityContract"],
        "orchestration/capability-contract.v1.json"
    );
    assert_eq!(
        string_list(&contract["vocabulary"], "vocabulary"),
        EXPECTED_VOCABULARY
    );
    assert_eq!(
        string_list(&contract["result"]["verdicts"], "verdicts"),
        EXPECTED_VERDICTS
    );
    assert_eq!(
        string_list(&contract["effectBoundary"]["effects"], "effects"),
        EXPECTED_EFFECTS
    );
    let codes: Vec<u64> = contract["exitCodes"]
        .as_array()
        .expect("exitCodes")
        .iter()
        .map(|entry| entry["code"].as_u64().expect("exit code"))
        .collect();
    assert_eq!(codes, EXPECTED_EXIT_CODES);
    assert!(
        !string_list(&contract["effectBoundary"]["effects"], "effects").contains(&"pure"),
        "purity must not be encoded as a permission effect"
    );
    let mut unique = codes.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique, codes, "exit codes must be unique");
    assert_eq!(contract["cacheKey"]["algorithm"], "sha256");
    let components = string_list(
        &contract["cacheKey"]["orderedComponents"],
        "cache components",
    );
    assert!(components.contains(&"snapshotIdentity"));
    assert!(components.contains(&"orderedInputArtifactDigests"));
    let forbidden = string_list(&contract["cacheKey"]["forbiddenComponents"], "forbidden");
    assert!(forbidden.contains(&"generatedAt"));
    assert_eq!(
        contract["publication"]["completionMarker"],
        "run-complete.json"
    );
}

#[test]
fn documentation_names_the_canonical_vocabulary() {
    for relative in DOCUMENTATION {
        let text = fs::read_to_string(repo_root().join(relative))
            .unwrap_or_else(|error| panic!("read {relative}: {error}"));
        for term in EXPECTED_VOCABULARY {
            assert!(
                text.contains(term),
                "{relative} is missing canonical atomic vocabulary: {term}"
            );
        }
    }
}

#[test]
fn registry_toolchain_digest_evidence_pairs_and_matches_bytes() {
    let root = repo_root();
    let registry = read_json("orchestration/integrations.json");
    let mut checked = 0usize;
    for integration in registry["integrations"]
        .as_array()
        .expect("integrations array")
    {
        if integration.get("toolchainDigestEvidence").is_none() {
            continue;
        }
        let id = integration["id"].as_str().unwrap_or("<missing-id>");
        assert_eq!(
            integration["toolchainDigestEvidence"]["algorithm"], "sha256",
            "{id} toolchain evidence must use SHA-256"
        );
        let inputs = integration["toolchainDigestEvidence"]["inputs"]
            .as_array()
            .unwrap_or_else(|| panic!("{id} toolchain evidence inputs"));
        let declared = integration["capabilityDeclaration"]["implementation"]["toolchainDigests"]
            .as_array()
            .unwrap_or_else(|| {
                panic!("{id} toolchain evidence requires an implementation declaration")
            });
        assert!(
            !inputs.is_empty(),
            "{id} toolchain evidence must declare at least one input"
        );
        assert_eq!(
            inputs.len(),
            declared.len(),
            "{id} toolchain evidence input/digest counts differ"
        );
        for (index, input) in inputs.iter().enumerate() {
            let relative = input
                .as_str()
                .unwrap_or_else(|| panic!("{id} toolchain evidence contains a non-string input"));
            assert!(
                !relative.is_empty(),
                "{id} toolchain evidence contains an empty input path"
            );
            assert!(
                !Path::new(relative).is_absolute(),
                "{id} toolchain evidence input must be repository-relative: {relative}"
            );
            let source = root.join(relative);
            assert!(
                source.is_file(),
                "{id} toolchain evidence input is missing: {relative}"
            );
            let actual = sha256_support::sha256(&source);
            let expected = declared[index]
                .as_str()
                .unwrap_or_else(|| panic!("{id} toolchain digest is not a string"));
            assert_eq!(
                expected.to_ascii_lowercase(),
                actual,
                "{id} toolchain digest is stale for {relative}"
            );
        }
        checked += 1;
    }
    assert!(
        checked > 0,
        "registry must declare at least one toolchainDigestEvidence block"
    );
}

#[test]
fn envelope_fixtures_and_outcome_matrix_match_the_schema_and_contract() {
    let contract = read_json("orchestration/capability-contract.v1.json");
    let (artifact, declaration, request, result) = fixture_set();
    assert!(schema_accepts(&artifact), "valid artifact ref");
    assert!(schema_accepts(&declaration), "valid capability declaration");
    assert!(schema_accepts(&request), "valid capability request");
    assert!(schema_accepts(&result), "valid completed result");
    envelopes_coherent(&declaration, &request, &result).expect("valid envelope chain");

    let mut domain_fail = result.clone();
    domain_fail["status"] = json!("completed");
    domain_fail["verdict"] = json!("fail");
    domain_fail["domainVerdict"] = json!("fail");
    domain_fail["exitCode"] = json!(10);
    assert!(schema_accepts(&domain_fail), "valid domain-fail result");

    let mut blocked = result.clone();
    blocked["status"] = json!("blocked");
    blocked["verdict"] = json!("unknown");
    blocked["domainVerdict"] = json!("unknown");
    blocked["exitCode"] = json!(20);
    assert!(schema_accepts(&blocked), "valid blocked result");

    let mut cases = 0usize;
    for status in string_list(&contract["result"]["statuses"], "statuses") {
        for verdict in string_list(&contract["result"]["verdicts"], "verdicts") {
            for entry in contract["exitCodes"].as_array().expect("exitCodes") {
                let exit = entry["code"].as_u64().expect("code");
                let mut candidate = result.clone();
                candidate["status"] = json!(status);
                candidate["verdict"] = json!(verdict);
                candidate["domainVerdict"] = json!(verdict);
                candidate["exitCode"] = json!(exit);
                let mapping_status = entry["status"].as_str().expect("mapping status");
                let mapping_verdicts = string_list(&entry["verdicts"], "mapping verdicts");
                let expected = status == mapping_status && mapping_verdicts.contains(&verdict);
                assert_eq!(
                    schema_accepts(&candidate),
                    expected,
                    "outcome matrix drift for status={status} verdict={verdict} exitCode={exit}"
                );
                cases += 1;
            }
        }
    }
    assert_eq!(cases, 3 * 4 * 8);

    let mut invalid_outcome = result.clone();
    invalid_outcome["status"] = json!("failed");
    invalid_outcome["verdict"] = json!("pass");
    invalid_outcome["exitCode"] = json!(70);
    assert!(!schema_accepts(&invalid_outcome), "failed/pass outcome");

    let mut invalid_effect_type = result.clone();
    invalid_effect_type["observedEffects"] = json!("repo_read");
    assert!(!schema_accepts(&invalid_effect_type), "string effect set");

    let mut unknown_field = request.clone();
    unknown_field["surprise"] = json!(true);
    assert!(
        !schema_accepts(&unknown_field),
        "request with unknown field"
    );

    let mut bad_digest = artifact.clone();
    bad_digest["sha256"] = json!("not-a-digest");
    assert!(!schema_accepts(&bad_digest), "artifact with invalid digest");

    let mut missing_payload_schema = artifact.clone();
    missing_payload_schema
        .as_object_mut()
        .unwrap()
        .remove("artifactSchema");
    assert!(
        !schema_accepts(&missing_payload_schema),
        "artifact without payload schema"
    );

    let mut missing_consumed = artifact.clone();
    missing_consumed
        .as_object_mut()
        .unwrap()
        .remove("consumedSnapshotIdentity");
    assert!(
        !schema_accepts(&missing_consumed),
        "artifact without consumed snapshot identity"
    );
}

#[test]
fn envelope_coherence_rejects_the_cross_envelope_mutators() {
    let (mutators, rejected) = {
        let mut rejected = 0usize;
        let mutators: [(&str, fn(&mut Value, &mut Value, &mut Value)); 10] = [
            ("request capability mismatch", |_, request, _| {
                request["capability"] = json!("other.capability");
            }),
            ("request implementation mismatch", |_, request, _| {
                request["implementation"]["version"] = json!("2.0.0");
            }),
            ("request effect outside declaration", |_, request, _| {
                request["effectPolicy"]["allowedEffects"] = json!(["repo_read", "network"]);
            }),
            ("result capability mismatch", |_, _, result| {
                result["capability"] = json!("other.capability");
            }),
            ("result implementation mismatch", |_, _, result| {
                result["implementation"]["version"] = json!("2.0.0");
            }),
            ("result determinism mismatch", |_, _, result| {
                result["determinism"] = json!("external_nondeterministic");
            }),
            ("result declared effect drift", |_, _, result| {
                result["declaredEffects"] = json!(["repo_read"]);
            }),
            ("observed undeclared effect", |_, _, result| {
                result["observedEffects"] = json!(["repo_read", "network"]);
            }),
            ("result snapshot mismatch", |_, _, result| {
                result["snapshotIdentity"] = json!("c".repeat(64));
            }),
            ("output artifact snapshot mismatch", |_, _, result| {
                result["artifacts"][0]["consumedSnapshotIdentity"] = json!("c".repeat(64));
            }),
        ];
        for (name, apply) in mutators {
            let (_, mut declaration, mut request, mut result) = fixture_set();
            apply(&mut declaration, &mut request, &mut result);
            assert!(
                envelopes_coherent(&declaration, &request, &result).is_err(),
                "{name} must be rejected by cross-envelope coherence rules"
            );
            rejected += 1;
        }
        (mutators.len(), rejected)
    };
    assert_eq!(rejected, mutators);
}
