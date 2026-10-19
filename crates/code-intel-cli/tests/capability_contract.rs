//! Lock for the capability contract document (DR-0018).
//!
//! These tests are the authoritative lock for the contract document,
//! registry digest pairing, envelope fixtures, and cross-envelope coherence.

use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

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

/// Validates against the canonical envelope schema itself, so a drifting
/// schema cannot leave these tests green (#453 review).
fn schema_accepts(value: &Value) -> bool {
    static VALIDATOR: OnceLock<jsonschema::Validator> = OnceLock::new();
    VALIDATOR
        .get_or_init(|| {
            jsonschema::draft7::new(&read_json(
                "orchestration/schemas/code-intel-capability-envelope.v1.schema.json",
            ))
            .expect("capability envelope schema compiles")
        })
        .is_valid(value)
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
            assert!(
                Path::new(relative)
                    .components()
                    .all(|part| matches!(part, Component::Normal(_) | Component::CurDir)),
                "{id} toolchain evidence escapes the repository: {relative}"
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
#[test]
fn design_proposal_declaration_is_registered_and_bounded() {
    let contract = read_json("orchestration/capability-contract.v1.json");
    let registry = read_json("orchestration/integrations.json");
    let declaration = &contract["declaration"]["designProposal"];
    assert_eq!(declaration["capability"], "advisory.design-proposal.compat");
    assert_eq!(declaration["transport"], "code-intel-capability-request.v1");
    assert_eq!(
        declaration["request"],
        "code-intel-design-proposal-request.v1"
    );
    assert_eq!(declaration["authority"], "advisory_only");
    assert_eq!(
        string_list(&declaration["allowedEffects"], "design proposal effects"),
        ["repo_read", "local_write"]
    );
    let identities = declaration["artifactIdentities"]
        .as_array()
        .expect("design proposal artifact identities");
    assert_eq!(identities.len(), 3);
    assert_eq!(
        identities[0],
        json!({"schema":"code-intel-design-context.v1","type":"design.context"})
    );
    assert_eq!(
        identities[1],
        json!({"schema":"code-intel-design-proposal-candidate.v1","type":"design.proposal-candidate"})
    );
    assert_eq!(
        identities[2],
        json!({"schema":"code-intel-design-proposal.v1","type":"design.proposal"})
    );
    assert_eq!(declaration["operations"]["context"]["consumes"], json!([]));
    assert_eq!(
        declaration["operations"]["context"]["emits"],
        json!(["design.context"])
    );
    assert_eq!(
        declaration["operations"]["validate"]["consumes"],
        json!(["design.context", "design.proposal-candidate"])
    );
    assert_eq!(
        declaration["operations"]["validate"]["emits"],
        json!(["design.proposal"])
    );

    let integration = registry["integrations"]
        .as_array()
        .expect("integrations")
        .iter()
        .find(|entry| entry["id"] == "advisory.design-proposal.compat")
        .expect("design proposal integration");
    assert_eq!(
        integration["capabilityDeclaration"],
        json!({
            "schema": "code-intel-capability-declaration.v1",
            "id": "advisory.design-proposal.compat",
            "contractVersion": 1,
            "implementation": {
                "id": "design-proposal.rust.v1",
                "version": "1.0.0",
                "toolchainDigests": ["bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"]
            },
            "determinism": "deterministic",
            "allowedEffects": ["repo_read", "local_write"],
            "dependencies": ["repo.snapshot"]
        })
    );
    assert_eq!(integration["authority"], "advisory_only");
    assert_eq!(integration["commands"]["capabilityExec"], "code-intel capability exec advisory.design-proposal.compat --request <request.json|-> --out <staging-dir> --artifact-root <run-root>");
    assert_eq!(
        string_list(
            &integration["effects"],
            "design proposal integration effects"
        ),
        ["repo_read", "local_write"]
    );
    assert_eq!(
        integration["artifactIdentities"],
        declaration["artifactIdentities"]
    );
    assert_eq!(
        integration["operations"],
        json!({
            "context": {
                "requestSchema": "code-intel-capability-request.v1",
                "operationSchema": "code-intel-design-proposal-request.v1",
                "consumes": [],
                "emits": [{"schema":"code-intel-design-context.v1","type":"design.context"}]
            },
            "validate": {
                "requestSchema": "code-intel-capability-request.v1",
                "operationSchema": "code-intel-design-proposal-request.v1",
                "consumes": [
                    {"schema":"code-intel-design-context.v1","type":"design.context"},
                    {"schema":"code-intel-design-proposal-candidate.v1","type":"design.proposal-candidate"}
                ],
                "emits": [{"schema":"code-intel-design-proposal.v1","type":"design.proposal"}]
            }
        })
    );

    let internalization = read_json("orchestration/internalization/design-proposal.json");
    assert_eq!(internalization["lifecycle"]["status"], "research");
    assert!(internalization["lifecycle"]["authorityEvent"].is_null());
}
