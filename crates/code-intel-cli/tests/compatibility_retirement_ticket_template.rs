use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{json, Value};

fn artifact_ref(schema: &str, kind: &str) -> Value {
    json!({
        "schema":"code-intel-artifact-ref.v1",
        "artifactSchema":schema,
        "type":kind,
        "path":"evidence.json",
        "sha256":"a".repeat(64),
        "consumedSnapshotIdentity":"snapshot-1"
    })
}

fn ticket() -> Value {
    let evidence = artifact_ref(
        "code-intel-compatibility-retirement-evidence.v1",
        "compatibility.retirement-evidence",
    );
    json!({
        "schema":"code-intel-compatibility-retirement-ticket-template.v1",
        "snapshotIdentity":"snapshot-1",
        "ticketId":"ticket-ret-1",
        "retirementId":"ret-1",
        "legacyBranch":{"capabilityId":"legacy.capability","branchId":"legacy.branch","callPath":"run-code-intel.ps1::legacy.branch"},
        "replacement":{"capabilityId":"replacement.atom","dependencies":["D02"]},
        "affectedFiles":["run-code-intel.ps1"],
        "evidence":{"golden":evidence,"contract":evidence,"effects":evidence,"usage":evidence,"rollbackRehearsal":evidence,"deletionDiff":artifact_ref("code-intel-compatibility-retirement-deletion-diff.v1","compatibility.retirement-deletion-diff")},
        "source":{"retirementDecision":artifact_ref("code-intel-compatibility-retirement-decision.v1","compatibility.retirement-decision"),"retirementManifest":artifact_ref("code-intel-compatibility-retirement-manifest.v1","compatibility.retirement-manifest")},
        "owner":"executor-a","verifier":"verifier-b","observationExpiry":4000000,
        "status":"draft",
        "authorityBoundary":"template_only_no_approval_or_deletion_authority"
    })
}

fn write(root: &Path, name: &str, value: &Value) -> PathBuf {
    let path = root.join(name);
    fs::write(&path, serde_json::to_vec(value).unwrap()).unwrap();
    path
}

#[test]
fn lint_accepts_one_branch_and_rejects_multi_branch_ambiguity_and_expiry() {
    let root = std::env::temp_dir().join(format!("code-intel-e01-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let valid = write(&root, "valid.json", &ticket());
    let ok = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args(["compatibility", "retirement-ticket", "lint", "--ticket"])
        .arg(&valid)
        .args(["--evaluated-at", "3000000"])
        .output()
        .unwrap();
    assert_eq!(
        ok.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&ok.stderr)
    );

    let mut multi = ticket();
    multi["legacyBranches"] = json!([multi["legacyBranch"].clone()]);
    let multi = write(&root, "multi.json", &multi);
    let rejected = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args(["compatibility", "retirement-ticket", "lint", "--ticket"])
        .arg(&multi)
        .args(["--evaluated-at", "3000000"])
        .output()
        .unwrap();
    assert_eq!(rejected.status.code(), Some(65));

    let mut expired = ticket();
    expired["observationExpiry"] = json!(2_999_999);
    let expired = write(&root, "expired.json", &expired);
    let rejected = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args(["compatibility", "retirement-ticket", "lint", "--ticket"])
        .arg(&expired)
        .args(["--evaluated-at", "3000000"])
        .output()
        .unwrap();
    assert_eq!(rejected.status.code(), Some(65));

    for field in ["owner", "verifier"] {
        let mut missing = ticket();
        missing.as_object_mut().unwrap().remove(field);
        let path = write(&root, &format!("missing-{field}.json"), &missing);
        let rejected = Command::new(env!("CARGO_BIN_EXE_code-intel"))
            .args(["compatibility", "retirement-ticket", "lint", "--ticket"])
            .arg(path)
            .args(["--evaluated-at", "3000000"])
            .output()
            .unwrap();
        assert_eq!(rejected.status.code(), Some(65));
    }
    let _ = fs::remove_dir_all(root);
}
