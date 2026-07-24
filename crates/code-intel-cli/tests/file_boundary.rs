use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use serde_json::{json, Value};

#[path = "../src/file_boundary.rs"]
mod file_boundary;

const SNAPSHOT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const OTHER: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const SOURCE: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

fn request(target: &str) -> Value {
    json!({
        "schema":"code-intel-file-boundary-request.v1",
        "targetFile":target,
        "expectedSnapshotIdentity":SNAPSHOT,
        "policy":{"evaluatedAt":2_000,"maxAgeSeconds":100},
        "document":{
            "schema":"code-intel-file-boundary-document.v1",
            "snapshotIdentity":SNAPSHOT,
            "observedAt":1_950,
            "source":{
                "kind":"local_boundary_document",
                "path":".aigx/files.aigx",
                "sha256":SOURCE
            },
            "entries":[{
                "path":"src/lib.rs",
                "role":"library boundary",
                "forbid":[{"id":"BOUNDARY-001","summary":"Do not import provider storage"}],
                "gotcha":[{"id":"GOTCHA-001","summary":"Preserve snapshot identity"}],
                "checks":[{"id":"CHECK-001","command":"cargo test -p code-intel --test file_boundary"}]
            }],
            "unsupportedConstructs":[]
        }
    })
}

#[test]
fn exact_file_resolution_is_normalized_snapshot_bound_and_complete() {
    let result = file_boundary::resolve(&request(".\\src\\lib.rs")).unwrap();
    assert_eq!(result["status"], "resolved");
    assert_eq!(result["normalizedTargetFile"], "src/lib.rs");
    assert_eq!(result["freshness"], "current");
    assert_eq!(result["completeness"], "complete");
    assert_eq!(result["boundary"]["path"], "src/lib.rs");
    assert_eq!(result["boundary"]["forbid"][0]["id"], "BOUNDARY-001");
    assert_eq!(result["boundary"]["gotcha"][0]["id"], "GOTCHA-001");
    assert_eq!(result["boundary"]["checks"][0]["id"], "CHECK-001");
    assert_eq!(result["provenance"]["sourcePath"], ".aigx/files.aigx");
    assert_eq!(result["diagnostics"], json!([]));
}

#[test]
fn missing_file_and_unsupported_constructs_are_explicit_unknowns() {
    let mut value = request("src/missing.rs");
    value["document"]["unsupportedConstructs"] = json!(["selector:owner-group"]);
    let result = file_boundary::resolve(&value).unwrap();
    assert_eq!(result["status"], "unknown");
    assert_eq!(result["completeness"], "unknown");
    assert!(result["boundary"].is_null());
    assert_eq!(result["diagnostics"][0]["code"], "unsupported_construct");
    assert_eq!(result["diagnostics"][1]["code"], "no_matching_boundary");

    let mut matched = request("src/lib.rs");
    matched["document"]["unsupportedConstructs"] = json!(["selector:owner-group"]);
    let matched_result = file_boundary::resolve(&matched).unwrap();
    assert_eq!(matched_result["status"], "resolved");
    assert_eq!(matched_result["completeness"], "partial");
}

#[test]
fn snapshot_mismatch_stale_future_and_unsafe_paths_fail_closed() {
    let mut mismatch = request("src/lib.rs");
    mismatch["document"]["snapshotIdentity"] = json!(OTHER);
    assert!(file_boundary::resolve(&mismatch)
        .unwrap_err()
        .contains("snapshot mismatch"));

    let mut stale = request("src/lib.rs");
    stale["document"]["observedAt"] = json!(1_899);
    assert!(file_boundary::resolve(&stale)
        .unwrap_err()
        .contains("stale"));

    let mut future = request("src/lib.rs");
    future["document"]["observedAt"] = json!(2_001);
    assert!(file_boundary::resolve(&future)
        .unwrap_err()
        .contains("future"));

    assert!(file_boundary::resolve(&request("../outside.rs"))
        .unwrap_err()
        .contains("escape"));
    assert!(file_boundary::resolve(&request("C:\\outside.rs"))
        .unwrap_err()
        .contains("repository-relative"));
}

#[test]
fn duplicate_case_folded_paths_wildcards_and_duplicate_ids_fail_closed() {
    let mut duplicate = request("src/lib.rs");
    let mut second = duplicate["document"]["entries"][0].clone();
    second["path"] = json!("SRC/LIB.RS");
    duplicate["document"]["entries"]
        .as_array_mut()
        .unwrap()
        .push(second);
    assert!(file_boundary::resolve(&duplicate)
        .unwrap_err()
        .contains("duplicate or ambiguous"));

    let mut wildcard = request("src/lib.rs");
    wildcard["document"]["entries"][0]["path"] = json!("src/*.rs");
    assert!(file_boundary::resolve(&wildcard)
        .unwrap_err()
        .contains("exact paths"));

    let mut ids = request("src/lib.rs");
    ids["document"]["entries"][0]["checks"][0]["id"] = json!("BOUNDARY-001");
    assert!(file_boundary::resolve(&ids)
        .unwrap_err()
        .contains("rule IDs"));
}

#[test]
fn unknown_fields_are_rejected_without_echoing_their_values() {
    let mut top_level = request("src/lib.rs");
    top_level["apiToken"] = json!("BOUNDARY-SECRET-SENTINEL");
    let top_error = file_boundary::resolve(&top_level).unwrap_err();
    assert!(top_error.contains("fields are invalid"));
    assert!(!top_error.contains("BOUNDARY-SECRET-SENTINEL"));

    let mut entry = request("src/lib.rs");
    entry["document"]["entries"][0]["ownerDatabase"] = json!("C:/private/index.db");
    let entry_error = file_boundary::resolve(&entry).unwrap_err();
    assert!(entry_error.contains("fields are invalid"));
    assert!(!entry_error.contains("private/index.db"));
}

#[test]
fn request_document_and_result_schemas_are_closed_and_docs_keep_aigx_optional() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for name in [
        "code-intel-file-boundary-document.v1.schema.json",
        "code-intel-file-boundary-request.v1.schema.json",
        "code-intel-file-boundary-result.v1.schema.json",
    ] {
        let schema: Value = serde_json::from_slice(
            &fs::read(root.join("orchestration/schemas").join(name)).unwrap(),
        )
        .unwrap();
        assert_eq!(schema["additionalProperties"], false, "{name}");
    }

    let result_schema: Value = serde_json::from_slice(
        &fs::read(
            root.join("orchestration/schemas/code-intel-file-boundary-result.v1.schema.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let required = result_schema["required"]
        .as_array()
        .unwrap()
        .iter()
        .map(Value::as_str)
        .collect::<Option<BTreeSet<_>>>()
        .unwrap();
    assert!(required.contains("provenance"));
    assert!(required.contains("freshness"));
    assert!(required.contains("diagnostics"));

    let docs = fs::read_to_string(root.join("docs/file-boundary-observation.md")).unwrap();
    assert!(docs.contains("does not require AIGX"));
    assert!(docs.contains("exact repository-relative path"));
    assert!(docs.contains("fail closed"));
}
