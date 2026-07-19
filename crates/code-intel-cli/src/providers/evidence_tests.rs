use super::*;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const SNAPSHOT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const OTHER_SNAPSHOT: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const REACT_INTEGRITY: &str =
    "sha512-G3spmtZJE/gWWPRJ3rpgUWTPRDJpEmdRja7iNZ7RAXlfpEO+NWVzPTca/cPI9hLwPo2Aq5/BZggo5JDBrwGrlA==";

fn temp_dir(name: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("code-intel-evidence-{name}-{stamp}"));
    fs::create_dir_all(&path).expect("fixture directory");
    path
}

fn write_json(root: &Path, name: &str, value: &Value) -> Value {
    let path = root.join(name);
    fs::write(&path, serde_json::to_vec(value).unwrap()).unwrap();
    artifact_ref(root, &path, "fixture.v1", name, SNAPSHOT)
}

fn artifact_ref(
    root: &Path,
    path: &Path,
    schema: &str,
    artifact_type: &str,
    snapshot: &str,
) -> Value {
    json!({
        "schema": ARTIFACT_REF_SCHEMA,
        "artifactSchema": schema,
        "type": artifact_type,
        "path": path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/"),
        "sha256": sha256::file_hex(path).unwrap(),
        "consumedSnapshotIdentity": snapshot
    })
}

fn react_report(complete: bool, skipped: bool, with_diagnostic: bool) -> Value {
    let diagnostics = if with_diagnostic {
        vec![json!({
            "id": "src/App.tsx::2:3::react-doctor/no-test::digest",
            "normalizedFilePath": "src/App.tsx",
            "filePath": "C:/repo/src/App.tsx",
            "plugin": "react-doctor",
            "rule": "no-test",
            "severity": "warning",
            "message": "test",
            "help": "fix",
            "category": "Correctness",
            "tags": ["correctness"],
            "line": 2,
            "column": 3,
            "offset": 10,
            "length": 4,
            "endLine": 2,
            "endColumn": 7
        })]
    } else {
        Vec::new()
    };
    json!({
        "schemaVersion": 3,
        "version": "0.7.8",
        "ok": true,
        "directory": "C:/repo",
        "mode": "full",
        "reactDetected": true,
        "diff": null,
        "projects": [{
            "directory": "C:/repo",
            "packageRoot": ".",
            "framework": "vite",
            "project": {},
            "diagnostics": diagnostics,
            "score": null,
            "skippedChecks": if skipped { vec!["dead-code"] } else { Vec::<&str>::new() },
            "analyzedFiles": ["src/App.tsx"],
            "analyzedFileCount": 1,
            "complete": complete,
            "elapsedMilliseconds": 5
        }],
        "diagnostics": diagnostics,
        "summary": {
            "errorCount": 0,
            "warningCount": if with_diagnostic { 1 } else { 0 },
            "affectedFileCount": if with_diagnostic { 1 } else { 0 },
            "totalDiagnosticCount": if with_diagnostic { 1 } else { 0 },
            "score": null,
            "scoreLabel": null
        },
        "elapsedMilliseconds": 5,
        "error": null
    })
}

fn react_native(report_ref: Value, observed_at: i64) -> Value {
    json!({
        "schema": "code-intel-react-doctor-native-result.v1",
        "snapshotIdentity": SNAPSHOT,
        "status": "completed",
        "observedAt": observed_at,
        "tool": {
            "version": "0.7.8",
            "integrity": REACT_INTEGRITY,
            "command": ["npx", "--yes", "react-doctor@0.7.8", "--json", "--no-telemetry"]
        },
        "report": report_ref,
        "error": null
    })
}

fn run_fixture(provider: &str, root: &Path, request: &Value, evaluated_at: i64) -> Value {
    let request_path = root.join("native.json");
    fs::write(&request_path, serde_json::to_vec(request).unwrap()).unwrap();
    adapt(provider, &request_path, root, evaluated_at, 100).unwrap()
}

#[test]
fn react_doctor_preserves_diagnostics_and_coverage() {
    let root = temp_dir("react-diagnostic");
    let report_ref = write_json(&root, "report.json", &react_report(true, false, true));
    let result = run_fixture("react-doctor", &root, &react_native(report_ref, 1000), 1050);
    assert_eq!(result["status"], "observed");
    assert_eq!(result["verdict"], "fail");
    assert_eq!(
        result["evidence"]["diagnostics"][0]["id"],
        "src/App.tsx::2:3::react-doctor/no-test::digest"
    );
    assert_eq!(result["evidence"]["diagnostics"][0]["endColumn"], 7);
    assert_eq!(
        result["evidence"]["coverage"][0]["analyzedFiles"][0],
        "src/App.tsx"
    );
}

#[test]
fn react_doctor_distinguishes_clean_partial_and_not_applicable() {
    for (name, report, status, verdict) in [
        (
            "clean",
            react_report(true, false, false),
            "observed",
            "pass",
        ),
        (
            "incomplete",
            react_report(false, false, false),
            "unknown",
            "unknown",
        ),
        (
            "skipped",
            react_report(true, true, false),
            "unknown",
            "unknown",
        ),
    ] {
        let root = temp_dir(name);
        let report_ref = write_json(&root, "report.json", &report);
        let result = run_fixture("react-doctor", &root, &react_native(report_ref, 1000), 1050);
        assert_eq!(result["status"], status);
        assert_eq!(result["verdict"], verdict);
    }

    let root = temp_dir("not-react");
    let mut report = react_report(true, false, false);
    report["ok"] = json!(false);
    report["error"] = json!({
        "message": "No React project found",
        "name": "ProjectNotFoundError",
        "chain": ["No React project found"],
        "sentryEventId": null
    });
    let report_ref = write_json(&root, "report.json", &report);
    let result = run_fixture("react-doctor", &root, &react_native(report_ref, 1000), 1050);
    assert_eq!(result["status"], "not_applicable");
}

#[test]
fn react_doctor_rejects_wrong_schema_stale_and_snapshot_mismatch() {
    let root = temp_dir("react-rejections");
    let corrupt_path = root.join("corrupt-report.json");
    fs::write(&corrupt_path, b"{not-json").unwrap();
    let corrupt_ref = artifact_ref(
        &root,
        &corrupt_path,
        "react-doctor-json-report.v3",
        "react-doctor-report",
        SNAPSHOT,
    );
    let result = run_fixture(
        "react-doctor",
        &root,
        &react_native(corrupt_ref, 1000),
        1050,
    );
    assert_eq!(result["failureCategory"], "local_tool_error");

    let mut report = react_report(true, false, false);
    report["schemaVersion"] = json!(2);
    let report_ref = write_json(&root, "report.json", &report);
    let result = run_fixture("react-doctor", &root, &react_native(report_ref, 1000), 1050);
    assert_eq!(result["failureCategory"], "local_tool_error");

    let report_ref = write_json(&root, "report-v3.json", &react_report(true, false, false));
    let result = run_fixture(
        "react-doctor",
        &root,
        &react_native(report_ref.clone(), 1),
        1050,
    );
    assert_eq!(result["failureCategory"], "stale_evidence");

    let mut mismatched = react_native(report_ref, 1000);
    mismatched["report"]["consumedSnapshotIdentity"] = json!(OTHER_SNAPSHOT);
    let result = run_fixture("react-doctor", &root, &mismatched, 1050);
    assert_eq!(result["failureCategory"], "snapshot_mismatch");
}

fn compete_native(root: &Path, complete: bool, observed_at: i64) -> Value {
    let datasets = [
        ("product", "identity"),
        ("competitors", "competitors"),
        ("companies", "companies"),
        ("pricing", "pricing"),
        ("techstack", "techstack"),
        ("social", "presence"),
        ("marketing", "marketing"),
        ("seo", "seo"),
        ("features", "features"),
    ];
    let mut refs = serde_json::Map::new();
    for (name, top_key) in datasets {
        if !complete && name == "features" {
            continue;
        }
        refs.insert(
            name.to_string(),
            write_json(
                root,
                &format!("{name}.json"),
                &json!({"meta": {"dataset": name}, top_key: []}),
            ),
        );
    }
    let report_ref = write_json(
        root,
        "report.json",
        &json!({
            "meta": {"dataset": "report"},
            "executive_summary": {},
            "competitor_analysis": []
        }),
    );
    let html_path = root.join("report.html");
    fs::write(&html_path, "<html><body>report</body></html>").unwrap();
    let html_ref = artifact_ref(
        root,
        &html_path,
        "insightkit-html.v1",
        "report-html",
        SNAPSHOT,
    );
    let provenance = write_json(
        root,
        "findings.json",
        &json!({"sources": ["https://example.test"]}),
    );
    json!({
        "schema": "code-intel-compete-native-result.v1",
        "snapshotIdentity": SNAPSHOT,
        "status": "completed",
        "observedAt": observed_at,
        "tool": {
            "revision": "ec13028fc8da620c73a114ffe403a772b29a78cb",
            "license": "MIT"
        },
        "artifacts": {
            "datasets": refs,
            "reportJson": report_ref,
            "reportHtml": html_ref,
            "provenance": [provenance]
        },
        "error": null
    })
}

#[test]
fn compete_admits_only_complete_output_as_observed() {
    let root = temp_dir("compete-complete");
    let result = run_fixture("compete", &root, &compete_native(&root, true, 1000), 1050);
    assert_eq!(result["status"], "observed");
    assert_eq!(result["verdict"], "unknown");
    assert_eq!(result["advisoryOnly"], true);

    let root = temp_dir("compete-partial");
    let result = run_fixture("compete", &root, &compete_native(&root, false, 1000), 1050);
    assert_eq!(result["status"], "unknown");
    assert_eq!(result["evidence"]["missing"][0], "features.json");

    let mut not_run = compete_native(&root, true, 1000);
    not_run["status"] = json!("not_run");
    not_run["artifacts"] = Value::Null;
    let result = run_fixture("compete", &root, &not_run, 1050);
    assert_eq!(result["status"], "unknown");

    not_run["status"] = json!("provider_unavailable");
    let result = run_fixture("compete", &root, &not_run, 1050);
    assert_eq!(result["status"], "unknown");
    assert_eq!(result["failureCategory"], "provider_unavailable");
}

#[test]
fn compete_rejects_stale_snapshot_and_unsafe_paths() {
    let root = temp_dir("compete-rejections");
    let result = run_fixture("compete", &root, &compete_native(&root, true, 1), 1050);
    assert_eq!(result["failureCategory"], "stale_evidence");

    let mut mismatch = compete_native(&root, true, 1000);
    mismatch["artifacts"]["datasets"]["product"]["consumedSnapshotIdentity"] =
        json!(OTHER_SNAPSHOT);
    let result = run_fixture("compete", &root, &mismatch, 1050);
    assert_eq!(result["failureCategory"], "snapshot_mismatch");

    let mut unsafe_request = compete_native(&root, true, 1000);
    unsafe_request["artifacts"]["datasets"]["product"]["path"] = json!("../product.json");
    let result = run_fixture("compete", &root, &unsafe_request, 1050);
    assert_eq!(result["failureCategory"], "local_tool_error");
}
