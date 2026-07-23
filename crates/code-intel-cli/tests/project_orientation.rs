use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

#[path = "../src/artifact_ref.rs"]
mod artifact_ref;
#[path = "../src/capability.rs"]
mod capability;
#[path = "../src/capability_inventory.rs"]
mod capability_inventory;
#[path = "../src/snapshot.rs"]
mod snapshot;
#[path = "../src/stable_artifact.rs"]
mod stable_artifact;

const SNAPSHOT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

struct Temp(PathBuf);

impl Temp {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("code-intel-d01-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn write_ref(root: &Path, path: &str, schema: &str, kind: &str, bytes: &[u8]) -> Value {
    fs::write(root.join(path), bytes).unwrap();
    json!({
        "schema":"code-intel-artifact-ref.v1",
        "artifactSchema":schema,
        "type":kind,
        "path":path,
        "sha256":capability::sha256_hex(bytes),
        "consumedSnapshotIdentity":SNAPSHOT
    })
}

#[test]
fn fixture_composes_first_actionable_view_without_fabricating_purpose() {
    let temp = Temp::new();
    let fixture: Value = serde_json::from_str(include_str!(
        "fixtures/project-orientation/missing-purpose.json"
    ))
    .unwrap();
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let registry: Value = serde_json::from_slice(
        &fs::read(repo_root.join("orchestration/integrations.json")).unwrap(),
    )
    .unwrap();
    let integration = registry["integrations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == "project.orientation")
        .expect("D01 must be registered as a real A01 atom");
    let bytes = |value: &Value| serde_json::to_vec(value).unwrap();
    let snapshot_ref = write_ref(
        &temp.0,
        "snapshot.json",
        "code-intel-repository-snapshot.v1",
        "repository.snapshot",
        &bytes(&fixture["snapshot"]),
    );
    let inventory_ref = write_ref(
        &temp.0,
        "files.txt",
        "code-intel-file-inventory.v1",
        "inventory.files",
        fixture["inventory"].as_str().unwrap().as_bytes(),
    );
    let mut survival = fixture["survival"].clone();
    survival["repository"]["sourceSha256"] = snapshot_ref["sha256"].clone();
    survival["inventory"]["sourceSha256"] = inventory_ref["sha256"].clone();
    let inputs = vec![
        snapshot_ref,
        inventory_ref,
        write_ref(
            &temp.0,
            "survival.json",
            "code-intel-repository-survival-scan-result.v1",
            "repository.survival-scan",
            &bytes(&survival),
        ),
        write_ref(
            &temp.0,
            "native-files.json",
            "code-evidence-files.v1",
            "code_evidence.files",
            &bytes(&fixture["nativeFiles"]),
        ),
        write_ref(
            &temp.0,
            "native-coverage.json",
            "code-evidence-coverage.v1",
            "code_evidence.coverage",
            &bytes(&fixture["nativeCoverage"]),
        ),
        write_ref(
            &temp.0,
            "native-ranking.json",
            "agent-code-slice-ranking.v1",
            "code_evidence.agent_slice",
            &bytes(&fixture["nativeRanking"]),
        ),
    ];
    let request = json!({
        "schema":"code-intel-capability-request.v1",
        "capability":"project.orientation",
        "contractVersion":1,
        "implementation":integration["capabilityDeclaration"]["implementation"],
        "snapshot":fixture["snapshot"]["snapshot"],
        "options":{},
        "inputs":inputs,
        "effectPolicy":{"allowedEffects":["local_write"]}
    });
    let request_path = temp.0.join("request.json");
    fs::write(&request_path, serde_json::to_vec(&request).unwrap()).unwrap();
    let out = temp.0.join("out");
    let output = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args(["capability", "exec", "project.orientation", "--request"])
        .arg(&request_path)
        .arg("--out")
        .arg(&out)
        .arg("--artifact-root")
        .arg(&temp.0)
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let envelope: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(envelope["status"], "completed");
    assert_eq!(envelope["observedEffects"], json!(["local_write"]));
    assert_eq!(envelope["artifacts"].as_array().unwrap().len(), 2);
    let orientation: Value =
        serde_json::from_slice(&fs::read(out.join("project-orientation.json")).unwrap()).unwrap();
    assert_eq!(orientation["schema"], "code-intel-project-orientation.v1");
    assert_eq!(orientation["purpose"]["status"], "unknown");
    assert_eq!(orientation["purpose"]["evidence"], json!([]));
    assert_eq!(orientation["languages"][0]["name"], "rust");
    assert_eq!(orientation["languages"][0]["fileCount"], 3);
    assert_eq!(
        orientation["boundaries"],
        json!([
            {"path":"src","kind":"top_level_directory","provenance":orientation["boundaries"][0]["provenance"]},
            {"path":"tests","kind":"top_level_directory","provenance":orientation["boundaries"][1]["provenance"]}
        ])
    );
    assert_eq!(orientation["entryPoints"][0]["path"], "src/main.rs");
    assert_eq!(orientation["activeChange"]["status"], "dirty");
    assert_eq!(orientation["activeChange"]["paths"], json!(["src/main.rs"]));
    assert!(orientation["risks"]
        .as_array()
        .unwrap()
        .iter()
        .any(|risk| risk["code"] == "structural_evidence_unavailable"));
    assert!(orientation["unknowns"]
        .as_array()
        .unwrap()
        .iter()
        .any(|unknown| unknown["field"] == "purpose"));
    assert_eq!(orientation["confidence"]["level"], "low");
    for field in ["identity", "purpose", "activeChange", "confidence"] {
        assert!(
            !orientation[field]["provenance"]
                .as_array()
                .unwrap()
                .is_empty(),
            "{field} lacks provenance"
        );
    }
    for field in [
        "languages",
        "boundaries",
        "entryPoints",
        "evidenceAvailability",
        "risks",
        "unknowns",
    ] {
        assert!(
            orientation[field]
                .as_array()
                .unwrap()
                .iter()
                .all(|claim| !claim["provenance"].as_array().unwrap().is_empty()),
            "{field} contains a provenance-free claim"
        );
    }
    let summary = fs::read_to_string(out.join("project-orientation.md")).unwrap();
    for section in [
        "Identity",
        "Purpose",
        "Languages",
        "Boundaries",
        "Entry Points",
        "Commands",
        "Active Change",
        "Risks",
        "Unknowns",
        "Confidence",
    ] {
        assert!(
            summary.contains(&format!("## {section}")),
            "summary projection lacks {section}"
        );
    }
    let schema_check = Command::new("pwsh")
        .args(["-NoLogo", "-NoProfile", "-Command", "param($Document,$Schema); if (-not (Get-Content -Raw -LiteralPath $Document | Test-Json -SchemaFile $Schema -ErrorAction Stop)) { exit 1 }"])
        .arg(out.join("project-orientation.json"))
        .arg(repo_root.join("orchestration/schemas/code-intel-project-orientation.v1.schema.json"))
        .output()
        .unwrap();
    assert!(
        schema_check.status.success(),
        "schema stderr={}",
        String::from_utf8_lossy(&schema_check.stderr)
    );

    let mut incoherent_survival = survival;
    incoherent_survival["repository"]["sourceSha256"] = json!("9".repeat(64));
    let incoherent_ref = write_ref(
        &temp.0,
        "incoherent-survival.json",
        "code-intel-repository-survival-scan-result.v1",
        "repository.survival-scan",
        &bytes(&incoherent_survival),
    );
    let mut incoherent_request = request;
    let survival_input = incoherent_request["inputs"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|input| input["type"] == "repository.survival-scan")
        .unwrap();
    *survival_input = incoherent_ref;
    let incoherent_request_path = temp.0.join("incoherent-request.json");
    fs::write(
        &incoherent_request_path,
        serde_json::to_vec(&incoherent_request).unwrap(),
    )
    .unwrap();
    let incoherent_out = temp.0.join("incoherent-out");
    let rejected = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args(["capability", "exec", "project.orientation", "--request"])
        .arg(&incoherent_request_path)
        .arg("--out")
        .arg(&incoherent_out)
        .arg("--artifact-root")
        .arg(&temp.0)
        .output()
        .unwrap();
    assert_eq!(rejected.status.code(), Some(65));
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("incoherent"));
    assert!(!incoherent_out.join("project-orientation.json").exists());
}
