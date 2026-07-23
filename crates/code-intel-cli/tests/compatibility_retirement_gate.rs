use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

const SNAPSHOT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const EVIDENCE_SCHEMA: &str = "code-intel-compatibility-retirement-evidence.v1";
const NOW: u64 = 3_000_000;
static NONCE: AtomicU64 = AtomicU64::new(0);

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let clock = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "code-intel-e00-{}-{clock}-{}",
            std::process::id(),
            NONCE.fetch_add(1, Ordering::Relaxed)
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
        .find(|line| line.len() == 64 && line.bytes().all(|b| b.is_ascii_hexdigit()))
        .unwrap()
        .to_ascii_lowercase()
}
fn declaration() -> Value {
    declaration_for("compatibility.retirement-gate")
}
fn declaration_for(id: &str) -> Value {
    let value: Value =
        serde_json::from_slice(&fs::read(root().join("orchestration/integrations.json")).unwrap())
            .unwrap();
    value["integrations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["id"] == id)
        .unwrap()["capabilityDeclaration"]
        .clone()
}
fn write_artifact(temp: &Path, name: &str, schema: &str, kind: &str, value: &Value) -> Value {
    let path = temp.join(name);
    fs::write(&path, serde_json::to_vec(value).unwrap()).unwrap();
    json!({"schema":"code-intel-artifact-ref.v1","artifactSchema":schema,"type":kind,"path":name,"sha256":sha256(&path),"consumedSnapshotIdentity":SNAPSHOT})
}
fn evidence(temp: &Path, class: &str, details: Value) -> Value {
    let value = json!({"schema":EVIDENCE_SCHEMA,"snapshotIdentity":SNAPSHOT,"id":format!("ev-{class}"),"evidenceClass":class,"retirementId":"ret-1","legacyBranchId":"legacy.branch","replacementCapabilityId":"replacement.atom","details":details});
    write_artifact(
        temp,
        &format!("{class}.json"),
        EVIDENCE_SCHEMA,
        "compatibility.retirement-evidence",
        &value,
    )
}
fn value_sha256(temp: &Path, name: &str, value: &Value) -> String {
    let path = temp.join(name);
    fs::write(&path, serde_json::to_vec(value).unwrap()).unwrap();
    sha256(&path)
}
fn bytes_sha256(temp: &Path, name: &str, bytes: &[u8]) -> String {
    let path = temp.join(name);
    fs::write(&path, bytes).unwrap();
    sha256(&path)
}
fn deletion_diff(temp: &Path, affected_files: &[&str]) -> Value {
    let files = affected_files
        .iter()
        .enumerate()
        .map(|(index, path)| {
            let base = format!("legacy-{index}\nkeep-{index}\n");
            let result = format!("keep-{index}\n");
            json!({
                "path":path,
                "baseBlobSha256":bytes_sha256(temp,&format!("base-{index}.txt"),base.as_bytes()),
                "resultBlobSha256":bytes_sha256(temp,&format!("result-{index}.txt"),result.as_bytes()),
                "baseText":base,
                "resultText":result,
                "hunks":[{"oldStart":1,"oldLines":1,"newStart":1,"newLines":0,"deletedLines":[format!("legacy-{index}")],"addedLines":[]}]
            })
        })
        .collect::<Vec<_>>();
    let files_value = Value::Array(files);
    let patch_sha = value_sha256(temp, "deletion-patch-files.json", &files_value);
    json!({
        "schema":"code-intel-compatibility-retirement-deletion-diff.v1",
        "snapshotIdentity":SNAPSHOT,
        "retirementId":"ret-1",
        "legacyBranchId":"legacy.branch",
        "affectedFiles":affected_files,
        "deletionsOnly":true,
        "summary":"summary is descriptive only; replayable hunks are authoritative",
        "patch":{"algorithm":"replayable-delete-only-v1","sha256":patch_sha,"files":files_value}
    })
}
fn signed_event(temp: &Path, subject_sha: &str) -> Value {
    let mut event = json!({
        "schema":"code-intel-authority-event.v1",
        "id":"authority.retirement.ret-1",
        "decision":"approved",
        "approver":{"id":"code-intel-maintainers","role":"repository_governance"},
        "evidenceIds":[subject_sha],
        "issuedAt":NOW-10,
        "expiresAt":NOW+10
    });
    let payload = json!({
        "schema":event["schema"],"id":event["id"],"decision":event["decision"],
        "approver":event["approver"],"evidenceIds":event["evidenceIds"],
        "issuedAt":event["issuedAt"],"expiresAt":event["expiresAt"]
    });
    let digest = value_sha256(temp, "authority-event-payload.json", &payload);
    event["attestation"] = json!({"scheme":"repository-governed-sha256-v1","digest":digest});
    event
}
fn fixture(temp: &Path) -> (Value, Vec<Value>, String) {
    let atom = evidence(
        temp,
        "replacement_atom",
        json!({"status":"production_ready","outcome":"passed"}),
    );
    let golden = evidence(
        temp,
        "golden_parity",
        json!({"outcome":"passed","assertionCount":4}),
    );
    let contract = evidence(
        temp,
        "contract_parity",
        json!({"outcome":"passed","assertionCount":3}),
    );
    let effects = evidence(
        temp,
        "effect_parity",
        json!({"outcome":"passed","assertionCount":2}),
    );
    let registry = evidence(
        temp,
        "registry_reconciliation",
        json!({"outcome":"passed","registryParticipantId":"legacy.registry","replacementCapabilityId":"replacement.atom","status":"declared"}),
    );
    let window = evidence(
        temp,
        "compatibility_window",
        json!({"outcome":"passed","startedAt":1000,"observedThrough":1000+30*86400,"minimumDays":30,"checkedAt":2_600_000,"expiresAt":NOW+100}),
    );
    let rollback = evidence(
        temp,
        "rollback_execution",
        json!({"outcome":"passed","command":"restore legacy.branch","executedAt":9000,"exitCode":0}),
    );
    let usage = evidence(
        temp,
        "usage_observation",
        json!({"outcome":"passed","startedAt":1000,"endedAt":1000+30*86400,"totalInvocations":20,"legacyInvocations":0,"replacementInvocations":20}),
    );
    let trace = json!({"retirementId":"ret-1","legacyBranchId":"legacy.branch","replacementCapabilityId":"replacement.atom"});
    let trace_sha = value_sha256(temp, "necessity-trace.json", &trace);
    let necessity = evidence(
        temp,
        "c00_necessity",
        json!({"outcome":"passed","decision":"admit","changeId":"ret-1","necessityTraceSha256":trace_sha}),
    );
    let dependency = evidence(
        temp,
        "dependency_approval",
        json!({"outcome":"passed","dependencyId":"D02","status":"approved","reviewer":"d02-reviewer"}),
    );
    let subject = json!({
        "legacyBranch":{"capabilityId":"legacy.capability","branchId":"legacy.branch","callPath":"run-code-intel.ps1::legacy.branch","affectedFiles":["run-code-intel.ps1"],"owner":"owner-team","registryParticipantId":"legacy.registry"},
        "replacement":{"capabilityId":"replacement.atom","implementationId":"replacement.atom.compat","dependencies":["D02"],"atomEvidence":atom},
        "parity":{"golden":golden,"contract":contract,"effects":effects},"registryReconciliation":registry,"compatibilityWindow":window,
        "rollback":{"command":"restore legacy.branch","executionEvidence":rollback},"usageObservation":usage,"necessityEvidence":necessity,"dependencyStates":[dependency],"lineReductionEvidence":false
    });
    let subject_path = temp.join("subject.json");
    fs::write(&subject_path, serde_json::to_vec(&subject).unwrap()).unwrap();
    let subject_sha = sha256(&subject_path);
    let approval = evidence(
        temp,
        "independent_approval",
        json!({"outcome":"passed","approved":true,"authorIndependent":true,"subjectSha256":subject_sha,"reviewer":"code-intel-maintainers","authorityEvent":signed_event(temp, &subject_sha)}),
    );
    let manifest = json!({"schema":"code-intel-compatibility-retirement-manifest.v1","snapshotIdentity":SNAPSHOT,"retirementId":"ret-1","approvalSubject":subject,"independentApproval":approval});
    let manifest_ref = write_artifact(
        temp,
        "manifest.json",
        "code-intel-compatibility-retirement-manifest.v1",
        "compatibility.retirement-manifest",
        &manifest,
    );
    let mut inputs = vec![manifest_ref];
    for name in [
        "replacement_atom",
        "golden_parity",
        "contract_parity",
        "effect_parity",
        "registry_reconciliation",
        "compatibility_window",
        "rollback_execution",
        "usage_observation",
        "c00_necessity",
        "dependency_approval",
        "independent_approval",
    ] {
        let path = temp.join(format!("{name}.json"));
        inputs.push(json!({"schema":"code-intel-artifact-ref.v1","artifactSchema":EVIDENCE_SCHEMA,"type":"compatibility.retirement-evidence","path":format!("{name}.json"),"sha256":sha256(&path),"consumedSnapshotIdentity":SNAPSHOT}));
    }
    (manifest, inputs, "rollback_execution.json".into())
}
fn request(inputs: Vec<Value>) -> Value {
    let d = declaration();
    json!({"schema":"code-intel-capability-request.v1","capability":"compatibility.retirement-gate","contractVersion":1,"implementation":d["implementation"],"snapshot":{"identity":SNAPSHOT,"repoIdentity":format!("content-v1:{}","c".repeat(64)),"head":"unversioned","workingTreePolicy":"explicit_overlay","scope":["."],"inputDigest":"d".repeat(64)},"options":{"evaluatedAt":NOW},"inputs":inputs,"effectPolicy":{"allowedEffects":d["allowedEffects"]}})
}
fn run(temp: &Path, request: &Value, out: &str) -> std::process::Output {
    let path = temp.join(format!("{out}-request.json"));
    fs::write(&path, serde_json::to_vec(request).unwrap()).unwrap();
    Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args([
            "capability",
            "exec",
            "compatibility.retirement-gate",
            "--request",
        ])
        .arg(path)
        .arg("--out")
        .arg(temp.join(out))
        .arg("--artifact-root")
        .arg(temp)
        .output()
        .unwrap()
}

fn run_e01(temp: &Path, request: &Value, request_name: &str, out: &str) -> std::process::Output {
    let path = temp.join(request_name);
    fs::write(&path, serde_json::to_vec(request).unwrap()).unwrap();
    Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args([
            "capability",
            "exec",
            "compatibility.retirement-ticket-template",
            "--request",
        ])
        .arg(path)
        .arg("--out")
        .arg(temp.join(out))
        .arg("--artifact-root")
        .arg(temp)
        .output()
        .unwrap()
}

fn assert_schema_valid(document: &Path, schema: &Path) {
    let output = Command::new("powershell")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-Command",
            "param($Document,$Schema); if (-not (Get-Content -Raw -LiteralPath $Document | Test-Json -SchemaFile $Schema -ErrorAction Stop)) { exit 1 }",
        ])
        .arg(document)
        .arg(schema)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn a01_a03_gate_approves_complete_evidence_and_rejects_missing_rollback() {
    let temp = Temp::new();
    let (manifest, inputs, rollback) = fixture(&temp.0);
    let output = run(&temp.0, &request(inputs.clone()), "approved");
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let decision: Value = serde_json::from_slice(
        &fs::read(
            temp.0
                .join("approved/compatibility-retirement-decision.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(decision["decision"], "approved");
    assert_eq!(
        decision["authorityBoundary"],
        "approval_only_no_deletion_authority"
    );
    assert_schema_valid(
        &temp.0.join("manifest.json"),
        &root().join(
            "orchestration/schemas/code-intel-compatibility-retirement-manifest.v1.schema.json",
        ),
    );
    assert_schema_valid(
        &temp.0.join("rollback_execution.json"),
        &root().join(
            "orchestration/schemas/code-intel-compatibility-retirement-evidence.v1.schema.json",
        ),
    );
    assert_schema_valid(
        &temp
            .0
            .join("approved/compatibility-retirement-decision.json"),
        &root().join(
            "orchestration/schemas/code-intel-compatibility-retirement-decision.v1.schema.json",
        ),
    );
    let replay = run(&temp.0, &request(inputs.clone()), "replay");
    assert_eq!(replay.status.code(), Some(0));
    assert_eq!(
        fs::read(
            temp.0
                .join("approved/compatibility-retirement-decision.json")
        )
        .unwrap(),
        fs::read(temp.0.join("replay/compatibility-retirement-decision.json")).unwrap()
    );

    let missing = inputs
        .clone()
        .into_iter()
        .filter(|v| v["path"] != rollback)
        .collect();
    let failed = run(&temp.0, &request(missing), "missing");
    assert_eq!(failed.status.code(), Some(65));
    assert!(!temp
        .0
        .join("missing/compatibility-retirement-decision.json")
        .exists());

    let mut tampered_manifest = manifest;
    tampered_manifest["unexpected"] = json!(true);
    let manifest_path = temp.0.join("manifest.json");
    fs::write(
        &manifest_path,
        serde_json::to_vec(&tampered_manifest).unwrap(),
    )
    .unwrap();
    let mut tampered_inputs = inputs.clone();
    tampered_inputs[0]["sha256"] = json!(sha256(&manifest_path));
    let closed_manifest_failure = run(
        &temp.0,
        &request(tampered_inputs),
        "closed-manifest-failure",
    );
    assert_eq!(closed_manifest_failure.status.code(), Some(65));
    assert!(!temp
        .0
        .join("closed-manifest-failure/compatibility-retirement-decision.json")
        .exists());
}

#[test]
fn evaluated_time_and_trusted_authority_hash_are_enforced_end_to_end() {
    let temp = Temp::new();
    let (mut manifest, mut inputs, _) = fixture(&temp.0);
    let mut missing_time = request(inputs.clone());
    missing_time["options"] = json!({});
    let invalid_options = run(&temp.0, &missing_time, "missing-evaluated-at");
    assert_eq!(invalid_options.status.code(), Some(64));
    assert!(!temp
        .0
        .join("missing-evaluated-at/compatibility-retirement-decision.json")
        .exists());

    let approval_path = temp.0.join("independent_approval.json");
    let mut approval: Value = serde_json::from_slice(&fs::read(&approval_path).unwrap()).unwrap();
    approval["details"]["authorityEvent"]["attestation"]["digest"] = json!("0".repeat(64));
    fs::write(&approval_path, serde_json::to_vec(&approval).unwrap()).unwrap();
    let approval_sha = sha256(&approval_path);
    manifest["independentApproval"]["sha256"] = json!(approval_sha.clone());
    let manifest_path = temp.0.join("manifest.json");
    fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    inputs[0]["sha256"] = json!(sha256(&manifest_path));
    let approval_ref = inputs
        .iter_mut()
        .find(|value| value["path"] == "independent_approval.json")
        .unwrap();
    approval_ref["sha256"] = json!(approval_sha);
    let output = run(&temp.0, &request(inputs), "forged-authority");
    assert_eq!(output.status.code(), Some(0));
    let decision: Value = serde_json::from_slice(
        &fs::read(
            temp.0
                .join("forged-authority/compatibility-retirement-decision.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(decision["decision"], "blocked");
    assert!(decision["blockers"]
        .as_array()
        .unwrap()
        .contains(&json!("unproven_independent_approval")));
}

#[test]
fn e01_ticket_is_content_bound_to_the_approved_e00_subject() {
    let temp = Temp::new();
    let (manifest, inputs, _) = fixture(&temp.0);
    let gate = run(&temp.0, &request(inputs.clone()), "e01-source");
    assert_eq!(
        gate.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&gate.stderr)
    );
    let decision_path = temp
        .0
        .join("e01-source/compatibility-retirement-decision.json");
    let decision: Value = serde_json::from_slice(&fs::read(&decision_path).unwrap()).unwrap();
    assert_eq!(decision["decision"], "approved");
    fs::copy(&decision_path, temp.0.join("decision.json")).unwrap();
    let deletion = deletion_diff(&temp.0, &["run-code-intel.ps1"]);
    let deletion_ref = write_artifact(
        &temp.0,
        "deletion.json",
        "code-intel-compatibility-retirement-deletion-diff.v1",
        "compatibility.retirement-deletion-diff",
        &deletion,
    );
    assert_schema_valid(
        &temp.0.join("deletion.json"),
        &root().join(
            "orchestration/schemas/code-intel-compatibility-retirement-deletion-diff.v1.schema.json",
        ),
    );
    let source_ref = |schema: &str, kind: &str, path: &str| json!({"schema":"code-intel-artifact-ref.v1","artifactSchema":schema,"type":kind,"path":path,"sha256":sha256(&temp.0.join(path)),"consumedSnapshotIdentity":SNAPSHOT});
    let subject = &manifest["approvalSubject"];
    let ticket = json!({
        "schema":"code-intel-compatibility-retirement-ticket-template.v1","snapshotIdentity":SNAPSHOT,"ticketId":"ticket-ret-1","retirementId":"ret-1",
        "legacyBranch":{"capabilityId":"legacy.capability","branchId":"legacy.branch","callPath":"run-code-intel.ps1::legacy.branch"},
        "replacement":{"capabilityId":"replacement.atom","dependencies":["D02"]},"affectedFiles":["run-code-intel.ps1"],
        "evidence":{"golden":subject["parity"]["golden"],"contract":subject["parity"]["contract"],"effects":subject["parity"]["effects"],"usage":subject["usageObservation"],"rollbackRehearsal":subject["rollback"]["executionEvidence"],"deletionDiff":deletion_ref},
        "source":{"retirementDecision":source_ref("code-intel-compatibility-retirement-decision.v1","compatibility.retirement-decision","decision.json"),"retirementManifest":inputs[0]},
        "owner":"executor-a","verifier":"verifier-b","observationExpiry":NOW+100,"status":"draft","authorityBoundary":"template_only_no_approval_or_deletion_authority"
    });
    let ticket_ref = write_artifact(
        &temp.0,
        "ticket.json",
        "code-intel-compatibility-retirement-ticket-template.v1",
        "compatibility.retirement-ticket-template",
        &ticket,
    );
    assert_schema_valid(
        &temp.0.join("ticket.json"),
        &root().join(
            "orchestration/schemas/code-intel-compatibility-retirement-ticket-template.v1.schema.json",
        ),
    );
    let d = declaration_for("compatibility.retirement-ticket-template");
    let req = json!({"schema":"code-intel-capability-request.v1","capability":"compatibility.retirement-ticket-template","contractVersion":1,"implementation":d["implementation"],"snapshot":{"identity":SNAPSHOT,"repoIdentity":format!("content-v1:{}","c".repeat(64)),"head":"unversioned","workingTreePolicy":"explicit_overlay","scope":["."],"inputDigest":"d".repeat(64)},"options":{"evaluatedAt":NOW},"inputs":[ticket_ref,inputs[0],source_ref("code-intel-compatibility-retirement-decision.v1","compatibility.retirement-decision","decision.json"),source_ref("code-intel-compatibility-retirement-deletion-diff.v1","compatibility.retirement-deletion-diff","deletion.json")],"effectPolicy":{"allowedEffects":d["allowedEffects"]}});
    let output = run_e01(&temp.0, &req, "e01-request.json", "e01-ticket");
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read(
            temp.0
                .join("e01-ticket/compatibility-retirement-ticket.json")
        )
        .unwrap(),
        fs::read(temp.0.join("ticket.json")).unwrap()
    );

    let mut forged_deletion = deletion.clone();
    forged_deletion["patch"]["files"][0]["resultText"] = json!("keep-0\nsmuggled()\n");
    forged_deletion["patch"]["files"][0]["resultBlobSha256"] = json!(bytes_sha256(
        &temp.0,
        "forged-result.txt",
        b"keep-0\nsmuggled()\n",
    ));
    forged_deletion["patch"]["files"][0]["hunks"][0]["newLines"] = json!(1);
    forged_deletion["patch"]["files"][0]["hunks"][0]["addedLines"] = json!(["smuggled()"]);
    forged_deletion["patch"]["sha256"] = json!(value_sha256(
        &temp.0,
        "forged-patch-files.json",
        &forged_deletion["patch"]["files"],
    ));
    let forged_deletion_ref = write_artifact(
        &temp.0,
        "forged-deletion.json",
        "code-intel-compatibility-retirement-deletion-diff.v1",
        "compatibility.retirement-deletion-diff",
        &forged_deletion,
    );
    let mut forged_ticket = ticket.clone();
    forged_ticket["evidence"]["deletionDiff"] = forged_deletion_ref.clone();
    let forged_ticket_ref = write_artifact(
        &temp.0,
        "forged-ticket.json",
        "code-intel-compatibility-retirement-ticket-template.v1",
        "compatibility.retirement-ticket-template",
        &forged_ticket,
    );
    let mut forged_req = req.clone();
    forged_req["inputs"][0] = forged_ticket_ref;
    forged_req["inputs"][3] = forged_deletion_ref;
    let forged = run_e01(
        &temp.0,
        &forged_req,
        "forged-e01-request.json",
        "forged-e01-ticket",
    );
    assert_eq!(
        forged.status.code(),
        Some(65),
        "forged added code was not rejected: {}",
        String::from_utf8_lossy(&forged.stderr)
    );

    let hidden_deletion = deletion_diff(&temp.0, &["run-code-intel.ps1", "second-branch.ps1"]);
    let hidden_deletion_ref = write_artifact(
        &temp.0,
        "hidden-deletion.json",
        "code-intel-compatibility-retirement-deletion-diff.v1",
        "compatibility.retirement-deletion-diff",
        &hidden_deletion,
    );
    let mut hidden_ticket = ticket.clone();
    hidden_ticket["affectedFiles"] = json!(["run-code-intel.ps1", "second-branch.ps1"]);
    hidden_ticket["evidence"]["deletionDiff"] = hidden_deletion_ref.clone();
    let hidden_ticket_ref = write_artifact(
        &temp.0,
        "hidden-ticket.json",
        "code-intel-compatibility-retirement-ticket-template.v1",
        "compatibility.retirement-ticket-template",
        &hidden_ticket,
    );
    let mut hidden_req = req.clone();
    hidden_req["inputs"][0] = hidden_ticket_ref;
    hidden_req["inputs"][3] = hidden_deletion_ref;
    let hidden = run_e01(
        &temp.0,
        &hidden_req,
        "hidden-e01-request.json",
        "hidden-e01-ticket",
    );
    assert_eq!(
        hidden.status.code(),
        Some(65),
        "unapproved second path was not rejected: {}",
        String::from_utf8_lossy(&hidden.stderr)
    );

    let mut wrong_decision = decision.clone();
    wrong_decision["approvalSubjectSha256"] = json!("0".repeat(64));
    let wrong_decision_ref = write_artifact(
        &temp.0,
        "wrong-subject-decision.json",
        "code-intel-compatibility-retirement-decision.v1",
        "compatibility.retirement-decision",
        &wrong_decision,
    );
    let mut wrong_subject_ticket = ticket.clone();
    wrong_subject_ticket["source"]["retirementDecision"] = wrong_decision_ref.clone();
    let wrong_subject_ticket_ref = write_artifact(
        &temp.0,
        "wrong-subject-ticket.json",
        "code-intel-compatibility-retirement-ticket-template.v1",
        "compatibility.retirement-ticket-template",
        &wrong_subject_ticket,
    );
    let mut wrong_subject_req = req.clone();
    wrong_subject_req["inputs"][0] = wrong_subject_ticket_ref;
    wrong_subject_req["inputs"][2] = wrong_decision_ref;
    let wrong_subject = run_e01(
        &temp.0,
        &wrong_subject_req,
        "wrong-subject-e01-request.json",
        "wrong-subject-e01-ticket",
    );
    assert_eq!(
        wrong_subject.status.code(),
        Some(65),
        "wrong E00 approval subject was not rejected: {}",
        String::from_utf8_lossy(&wrong_subject.stderr)
    );

    let mut missing_input_req = req.clone();
    missing_input_req["inputs"].as_array_mut().unwrap().pop();
    let missing_input = run_e01(
        &temp.0,
        &missing_input_req,
        "missing-input-e01-request.json",
        "missing-input-e01-ticket",
    );
    assert_eq!(missing_input.status.code(), Some(65));
    assert!(!temp
        .0
        .join("missing-input-e01-ticket/compatibility-retirement-ticket.json")
        .exists());

    let mut extra_input_req = req.clone();
    let fifth = extra_input_req["inputs"][3].clone();
    extra_input_req["inputs"]
        .as_array_mut()
        .unwrap()
        .push(fifth);
    let extra_input = run_e01(
        &temp.0,
        &extra_input_req,
        "extra-input-e01-request.json",
        "extra-input-e01-ticket",
    );
    assert_eq!(extra_input.status.code(), Some(65));
    assert!(!temp
        .0
        .join("extra-input-e01-ticket/compatibility-retirement-ticket.json")
        .exists());

    let mut wrong_call_path_ticket = ticket.clone();
    wrong_call_path_ticket["legacyBranch"]["callPath"] = json!("other-entry.ps1::legacy.branch");
    let wrong_call_path_ticket_ref = write_artifact(
        &temp.0,
        "wrong-call-path-ticket.json",
        "code-intel-compatibility-retirement-ticket-template.v1",
        "compatibility.retirement-ticket-template",
        &wrong_call_path_ticket,
    );
    let mut wrong_call_path_req = req.clone();
    wrong_call_path_req["inputs"][0] = wrong_call_path_ticket_ref;
    let wrong_call_path = run_e01(
        &temp.0,
        &wrong_call_path_req,
        "wrong-call-path-e01-request.json",
        "wrong-call-path-e01-ticket",
    );
    assert_eq!(wrong_call_path.status.code(), Some(65));
    assert!(!temp
        .0
        .join("wrong-call-path-e01-ticket/compatibility-retirement-ticket.json")
        .exists());
}
