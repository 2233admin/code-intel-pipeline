#[path = "../src/authority.rs"]
mod authority;
#[path = "../src/internalization_record.rs"]
mod internalization_record;

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn recompute_sha(relative: &str) -> String {
    let path = root().join(relative);
    let output = Command::new("pwsh")
        .args([
            "-NoProfile",
            "-CommandWithArgs",
            "(Get-FileHash -LiteralPath $args[0] -Algorithm SHA256).Hash.ToLowerInvariant()",
            path.to_str().unwrap(),
        ])
        .output()
        .expect("run Get-FileHash");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let digest = String::from_utf8(output.stdout).unwrap().trim().to_string();
    assert_eq!(digest.len(), 64);
    digest
}

#[test]
fn ast_grep_record_traces_the_optional_preview_capability_and_stays_research_only() {
    let record: Value = serde_json::from_slice(
        &fs::read(root().join("orchestration/internalization/ast-grep.json")).unwrap(),
    )
    .unwrap();

    let evidence = internalization_record::record_evidence_ids(&record).unwrap();
    let admitted = evidence
        .into_iter()
        .filter(|id| !id.starts_with("gap:"))
        .collect::<Vec<_>>();
    let evaluated_at = record["provenance"]["recordedAt"].as_u64().unwrap();
    let evaluation =
        internalization_record::evaluate_record(&record, evaluated_at, &admitted, &[]).unwrap();
    assert_eq!(evaluation["researchAllowed"], true);
    assert_eq!(evaluation["productionEnabled"], false);
    assert_eq!(evaluation["consumedAuthorityEventId"], Value::Null);
    assert_eq!(record["lifecycle"]["authorityEvent"], Value::Null);
    assert_eq!(record["adoption"]["rung"], "invoke");

    let source_digest = recompute_sha("crates/code-intel-cli/src/structured_edit.rs");
    let conformance_digest = recompute_sha("crates/code-intel-cli/tests/capability_exec.rs");
    let revision = record["subject"]["source"]["revision"].as_str().unwrap();
    assert!(revision.contains(&format!("local-native-source-sha256:{source_digest}")));
    assert!(revision.contains(&format!("local-conformance-sha256:{conformance_digest}")));

    let registry: Value =
        serde_json::from_slice(&fs::read(root().join("orchestration/integrations.json")).unwrap())
            .unwrap();
    let integration = registry["integrations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == "edit.ast-grep-plan")
        .unwrap();
    assert_eq!(integration["required"], false);

    let traces = record["operationTrace"].as_array().unwrap();
    assert_eq!(traces.len(), 1);
    let trace = &traces[0];
    assert_eq!(trace["integrationId"], "edit.ast-grep-plan");
    assert_eq!(trace["operation"], "capabilityExec");
    assert_eq!(trace["command"], integration["commands"]["capabilityExec"]);
    assert_eq!(trace["source"]["sha256"], Value::String(source_digest));
    assert_eq!(
        trace["conformance"]["sha256"],
        Value::String(conformance_digest)
    );
    let test_name = trace["conformance"]["testName"].as_str().unwrap();
    assert_eq!(
        test_name,
        "structured_edit_plan_is_scope_bound_and_preview_only"
    );
    assert!(
        fs::read_to_string(root().join(trace["conformance"]["path"].as_str().unwrap()))
            .unwrap()
            .contains(test_name)
    );

    let record_text = serde_json::to_string(&record).unwrap();
    for gap in [
        "gap:ast-grep:package-provenance",
        "gap:ast-grep:local-license-copy",
        "gap:ast-grep:replacement-command-drill",
        "gap:ast-grep:latency-p50-p95-measurement",
    ] {
        assert!(record_text.contains(gap), "{gap} must stay declared");
    }
}
