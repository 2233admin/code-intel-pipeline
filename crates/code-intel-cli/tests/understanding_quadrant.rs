use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

const SNAPSHOT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
static TEMP_NONCE: AtomicU64 = AtomicU64::new(0);

struct Temp(PathBuf);

impl Temp {
    fn new() -> Self {
        let clock = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let sequence = TEMP_NONCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "code-intel-d03-{}-{clock}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn sha256(path: &Path) -> String {
    let output = Command::new("certutil")
        .arg("-hashfile")
        .arg(path)
        .arg("SHA256")
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .find(|line| line.len() == 64 && line.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .unwrap()
        .to_ascii_lowercase()
}

fn provenance(pointer: &str) -> Value {
    json!([{
        "artifactType":"repository.survival-scan",
        "artifactSha256":"b".repeat(64),
        "jsonPointer":pointer
    }])
}

fn orientation() -> Value {
    json!({
        "schema":"code-intel-project-orientation.v1",
        "snapshotIdentity":SNAPSHOT,
        "identity":{
            "status":"known",
            "repositoryIdentity":format!("content-v1:{}", "c".repeat(64)),
            "repositoryKind":"unversioned",
            "revision":"unversioned",
            "provenance":provenance("/snapshot")
        },
        "purpose":{
            "status":"known",
            "evidence":["README.md"],
            "reason":"README declares the fixture purpose.",
            "provenance":provenance("/purpose")
        },
        "languages":[{"name":"Rust","fileCount":3,"provenance":provenance("/languages/0")}],
        "boundaries":[{"path":"src","kind":"top_level_directory","provenance":provenance("/boundaries/0")}],
        "entryPoints":[{"path":"src/main.rs","classification":"heuristic","provenance":provenance("/entryPoints/0")}],
        "commands":[{"path":"build.ps1","kind":"script_path","provenance":provenance("/commands/0")}],
        "activeChange":{"status":"clean","paths":[],"provenance":provenance("/activeChange")},
        "evidenceAvailability":[
            {"evidence":"survival_scan","status":"available","provenance":provenance("/evidenceAvailability/0")},
            {"evidence":"native_files","status":"available","provenance":provenance("/evidenceAvailability/1")},
            {"evidence":"native_structure","status":"heuristic","provenance":provenance("/evidenceAvailability/2")}
        ],
        "risks":[{"code":"dirty_tree","statement":"No dirty tree is present in the fixture.","provenance":provenance("/risks/0")}],
        "unknowns":[
            {"field":"dependencies.runtime","reason":"Runtime dependency authority is absent.","provenance":provenance("/unknowns/0")},
            {"field":"documentation.examples","reason":"Examples were not inspected.","provenance":provenance("/unknowns/1")}
        ],
        "confidence":{"level":"high","basis":["all fixture evidence is present"],"provenance":provenance("/confidence")}
    })
}

fn declaration() -> Value {
    let registry: Value =
        serde_json::from_slice(&fs::read(root().join("orchestration/integrations.json")).unwrap())
            .unwrap();
    registry["integrations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|integration| integration["id"] == "understanding.quadrant")
        .unwrap()["capabilityDeclaration"]
        .clone()
}

fn input(temp: &Path) -> Value {
    input_value(temp, &orientation(), "project-orientation.json")
}

fn input_value(temp: &Path, value: &Value, name: &str) -> Value {
    let path = temp.join(name);
    fs::write(&path, serde_json::to_vec(value).unwrap()).unwrap();
    json!({
        "schema":"code-intel-artifact-ref.v1",
        "artifactSchema":"code-intel-project-orientation.v1",
        "type":"project.orientation",
        "path":name,
        "sha256":sha256(&path),
        "consumedSnapshotIdentity":SNAPSHOT
    })
}

fn request(input: Value, options: Value) -> Value {
    let declaration = declaration();
    json!({
        "schema":"code-intel-capability-request.v1",
        "capability":"understanding.quadrant",
        "contractVersion":1,
        "implementation":declaration["implementation"],
        "snapshot":{
            "identity":SNAPSHOT,
            "repoIdentity":format!("content-v1:{}", "c".repeat(64)),
            "head":"unversioned",
            "workingTreePolicy":"explicit_overlay",
            "scope":["."],
            "inputDigest":"d".repeat(64)
        },
        "options":options,
        "inputs":[input],
        "effectPolicy":{"allowedEffects":declaration["allowedEffects"]}
    })
}

fn run(temp: &Path, request: &Value, out_name: &str) -> std::process::Output {
    let request_path = temp.join(format!("{out_name}-request.json"));
    fs::write(&request_path, serde_json::to_vec(request).unwrap()).unwrap();
    Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args(["capability", "exec", "understanding.quadrant", "--request"])
        .arg(&request_path)
        .arg("--out")
        .arg(temp.join(out_name))
        .arg("--artifact-root")
        .arg(temp)
        .output()
        .unwrap()
}

fn item<'a>(document: &'a Value, id: &str) -> &'a Value {
    document["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == id)
        .unwrap()
}

#[test]
fn critical_and_supporting_unknowns_remain_visible_with_stable_provenance() {
    let temp = Temp::new();
    let request = request(input(&temp.0), json!({}));
    let output = run(&temp.0, &request, "first");
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
    assert_eq!(envelope["artifacts"].as_array().unwrap().len(), 1);
    assert_eq!(
        envelope["artifacts"][0]["consumedSnapshotIdentity"],
        SNAPSHOT
    );
    let path = temp.0.join("first/understanding-quadrant.json");
    let bytes = fs::read(&path).unwrap();
    let document: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(item(&document, "identity")["quadrant"], "Known Core");
    assert_eq!(
        item(&document, "language:Rust")["quadrant"],
        "Supporting Context"
    );
    assert_eq!(
        item(&document, "unknown:dependencies.runtime")["quadrant"],
        "Critical Unknown"
    );
    assert_eq!(
        item(&document, "unknown:documentation.examples")["quadrant"],
        "Deferred Unknown"
    );
    assert_eq!(
        document["visibleUnknowns"],
        json!([
            "unknown:dependencies.runtime",
            "unknown:documentation.examples"
        ])
    );
    assert_eq!(
        item(&document, "unknown:dependencies.runtime")["provenance"],
        orientation()["unknowns"][0]["provenance"]
    );
    assert_eq!(
        document["classificationPolicy"]["methodConsumerPolicy"],
        "C01_cards_and_C02_selection_may_consume_but_cannot_rewrite"
    );

    let replay = run(&temp.0, &request, "replay");
    assert_eq!(replay.status.code(), Some(0));
    assert_eq!(
        bytes,
        fs::read(temp.0.join("replay/understanding-quadrant.json")).unwrap()
    );

    let schema = Command::new("pwsh")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-Command",
            "param($Document,$Schema); if (-not (Get-Content -Raw -LiteralPath $Document | Test-Json -SchemaFile $Schema -ErrorAction Stop)) { exit 1 }",
        ])
        .arg(&path)
        .arg(root().join("orchestration/schemas/code-intel-understanding-quadrant.v1.schema.json"))
        .output()
        .unwrap();
    assert!(
        schema.status.success(),
        "{}",
        String::from_utf8_lossy(&schema.stderr)
    );
}

#[test]
fn method_rewrite_options_and_snapshot_mismatch_fail_closed_without_artifacts() {
    let temp = Temp::new();
    let reference = input(&temp.0);
    let rewrite = run(
        &temp.0,
        &request(reference.clone(), json!({"methodId":"fmea"})),
        "rewrite",
    );
    assert_eq!(rewrite.status.code(), Some(64));
    assert!(!temp.0.join("rewrite/understanding-quadrant.json").exists());

    let mut mismatched = reference;
    mismatched["consumedSnapshotIdentity"] = json!("f".repeat(64));
    let mismatch = run(&temp.0, &request(mismatched, json!({})), "mismatch");
    assert_eq!(mismatch.status.code(), Some(65));
    assert!(!temp.0.join("mismatch/understanding-quadrant.json").exists());

    let mut invalid_provenance = orientation();
    invalid_provenance["identity"]["provenance"] = json!([null]);
    let invalid_ref = input_value(
        &temp.0,
        &invalid_provenance,
        "invalid-provenance-orientation.json",
    );
    let invalid = run(
        &temp.0,
        &request(invalid_ref, json!({})),
        "invalid-provenance",
    );
    assert_eq!(invalid.status.code(), Some(65));
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("provenance"));
    assert!(!temp
        .0
        .join("invalid-provenance/understanding-quadrant.json")
        .exists());
}

#[test]
fn a03_rejects_tampered_policy_constants_before_a01_dispatch() {
    let temp = Temp::new();
    let output = run(&temp.0, &request(input(&temp.0), json!({})), "source");
    assert_eq!(output.status.code(), Some(0));
    let mut document: Value = serde_json::from_slice(
        &fs::read(temp.0.join("source/understanding-quadrant.json")).unwrap(),
    )
    .unwrap();
    document["classificationPolicy"]["scoreRange"]["maximum"] = json!(999);
    let path = temp.0.join("tampered-quadrant.json");
    fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
    let tampered_ref = json!({
        "schema":"code-intel-artifact-ref.v1",
        "artifactSchema":"code-intel-understanding-quadrant.v1",
        "type":"understanding.quadrant",
        "path":"tampered-quadrant.json",
        "sha256":sha256(&path),
        "consumedSnapshotIdentity":SNAPSHOT
    });
    let rejected = run(
        &temp.0,
        &request(tampered_ref, json!({})),
        "tampered-policy",
    );
    assert_eq!(rejected.status.code(), Some(65));
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("policy"));
    assert!(!temp
        .0
        .join("tampered-policy/understanding-quadrant.json")
        .exists());
}
