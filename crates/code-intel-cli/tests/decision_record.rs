use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

#[path = "../src/artifact_ref.rs"]
mod artifact_ref;
#[path = "../src/authority.rs"]
mod authority;
#[path = "../src/capability.rs"]
mod capability;
#[path = "../src/capability_inventory.rs"]
mod capability_inventory;
#[path = "../src/decision_port.rs"]
mod decision_port;
#[path = "../src/decision_record.rs"]
mod decision_record;
#[path = "../src/run_commit.rs"]
mod run_commit;
#[path = "../src/snapshot.rs"]
mod snapshot;
#[path = "../src/stable_artifact.rs"]
mod stable_artifact;
#[path = "../src/staged_artifact.rs"]
mod staged_artifact;

const SNAPSHOT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const EVIDENCE: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
static NONCE: AtomicU64 = AtomicU64::new(0);

struct Temp(PathBuf);

impl Temp {
    fn new() -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "code-intel-c07-{}-{now}-{}",
            std::process::id(),
            NONCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn resolution() -> Value {
    let options = json!([
        {"id":"safe","label":"Use safe path","consequence":"slower delivery"},
        {"id":"fast","label":"Use fast path","consequence":"accept migration risk"}
    ]);
    let evidence = json!([{
        "refId":"ev-current","sha256":EVIDENCE,"observedAt":1900,"expiresAt":2200
    }]);
    json!({
        "schema":"code-intel-decision-record-request.v1",
        "gap":{
            "schema":"code-intel-decision-gap.v1","id":"gap-risk","kind":"risk_acceptance",
            "blockedDecision":"choose migration path","discoverableFactsChecked":[{"factId":"fact-cost","status":"resolved"}],
            "options":options,"recommendedAnswer":{"kind":"proposal","optionId":"safe","rationale":"preserve rollback"},
            "affectedBranches":["deploy"],"authorityRequired":true,"authorityState":"unresolved","effects":[]
        },
        "request":{
            "schema":"code-intel-decision-request.v1","correlationId":"corr-risk-1","gapId":"gap-risk",
            "question":"Which migration path is authorized?","recommendation":{"optionId":"safe","rationale":"preserve rollback"},
            "evidenceRefs":evidence,"options":options,"authorityNeeded":{"kind":"release_owner","actorIds":["alice"]},
            "issuedAt":1950,"expiresAt":2150,"affectedBranches":["deploy"]
        },
        "response":{
            "schema":"code-intel-decision-response.v1","correlationId":"corr-risk-1","gapId":"gap-risk",
            "answer":{"kind":"choice","optionId":"safe"},
            "actorProvenance":{"actorId":"alice","authorityKind":"release_owner","source":"native-ui"},"timestamp":2000
        },
        "authorityEvent":{
            "schema":"code-intel-authority-event.v1","id":"authority-risk-1","decision":"approved",
            "approver":{"id":"alice","role":"release_owner"},"evidenceIds":["ev-current"],"issuedAt":1990,"expiresAt":2100
        },
        "snapshotIdentity":SNAPSHOT,
        "recordedAt":2000
    })
}

fn replay_query() -> Value {
    json!({
        "schema":"code-intel-decision-replay-query.v1","gapId":"gap-risk",
        "snapshotIdentity":SNAPSHOT,
        "evidenceRefs":[{"refId":"ev-current","sha256":EVIDENCE,"observedAt":1900,"expiresAt":2200}],
        "affectedBranches":["deploy"],"now":2050
    })
}

fn committed_runs(root: &Path) -> Vec<PathBuf> {
    let mut runs = fs::read_dir(root)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry.file_type().unwrap().is_dir()
                && entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.starts_with("decision-"))
        })
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    runs.sort();
    runs
}

fn spawn_record(resolution: &Path, store: &Path) -> std::process::Child {
    Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args(["decision", "record", "--resolution"])
        .arg(resolution)
        .arg("--store")
        .arg(store)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

fn wait_record_pair(mut children: [std::process::Child; 2]) -> [std::process::Output; 2] {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let exited = children
            .iter_mut()
            .map(|child| child.try_wait().unwrap())
            .collect::<Vec<_>>();
        if exited.iter().all(Option::is_some) {
            return children.map(|child| child.wait_with_output().unwrap());
        }
        if Instant::now() >= deadline {
            for (child, status) in children.iter_mut().zip(exited) {
                if status.is_none() {
                    let _ = child.kill();
                }
            }
            let outputs = children.map(|child| child.wait_with_output().unwrap());
            panic!(
                "decision record child pair timed out; stderr: {:?}",
                outputs.map(|output| String::from_utf8_lossy(&output.stderr).into_owned())
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn resolve_once_then_replay_without_question() {
    let root = Temp::new();
    let store = decision_record::DecisionRecordStore::new(root.0.clone());
    let resolution = resolution();
    let mut port = decision_port::InMemoryDecisionPort::default();
    port.supply_response(resolution["response"].clone())
        .unwrap();
    let exchange = decision_port::DecisionExchange::default()
        .advance(
            &resolution["request"],
            &mut port,
            resolution["recordedAt"].as_u64().unwrap(),
            &["deploy", "unrelated"],
        )
        .unwrap();
    assert_eq!(exchange["status"], "resolved");
    assert_eq!(exchange["branches"][1]["status"], "continues");

    let recorded = store.record(&resolution).unwrap();
    assert_eq!(recorded["status"], "recorded");
    assert_eq!(recorded["record"]["acceptedChoice"]["optionId"], "safe");
    assert_eq!(
        recorded["record"]["consequences"],
        json!(["slower delivery"])
    );

    let replayed = store.replay(&replay_query()).unwrap();
    assert_eq!(replayed["status"], "replay");
    assert_eq!(replayed["questionRequired"], false);
    assert_eq!(
        replayed["record"]["response"]["correlationId"],
        "corr-risk-1"
    );

    let duplicate = store.record(&resolution).unwrap();
    assert_eq!(duplicate["status"], "replay");
    assert_eq!(duplicate["questionRequired"], false);
}

#[test]
fn changed_evidence_or_snapshot_requires_reopen() {
    let root = Temp::new();
    let store = decision_record::DecisionRecordStore::new(root.0.clone());
    store.record(&resolution()).unwrap();

    let mut changed = replay_query();
    changed["evidenceRefs"][0]["sha256"] = json!("c".repeat(64));
    let result = store.replay(&changed).unwrap();
    assert_eq!(result["status"], "reopen");
    assert_eq!(result["reason"], "evidence_changed");
    assert_eq!(result["questionRequired"], true);

    let mut changed_snapshot = replay_query();
    changed_snapshot["snapshotIdentity"] = json!("d".repeat(64));
    assert_eq!(
        store.replay(&changed_snapshot).unwrap()["reason"],
        "snapshot_changed"
    );
}

#[test]
fn stale_evidence_requires_reopen() {
    let root = Temp::new();
    let store = decision_record::DecisionRecordStore::new(root.0.clone());
    store.record(&resolution()).unwrap();
    let mut query = replay_query();
    query["now"] = json!(2201);
    assert_eq!(store.replay(&query).unwrap()["reason"], "evidence_stale");
}

#[test]
fn forged_authority_and_replayed_authority_event_are_rejected() {
    let root = Temp::new();
    let store = decision_record::DecisionRecordStore::new(root.0.clone());
    let mut forged = resolution();
    forged["authorityEvent"]["approver"]["id"] = json!("mallory");
    assert!(store
        .record(&forged)
        .unwrap_err()
        .to_string()
        .contains("approver"));

    store.record(&resolution()).unwrap();
    let mut second = resolution();
    second["gap"]["id"] = json!("gap-second");
    second["request"]["gapId"] = json!("gap-second");
    second["response"]["gapId"] = json!("gap-second");
    second["request"]["correlationId"] = json!("corr-second");
    second["response"]["correlationId"] = json!("corr-second");
    assert!(store
        .record(&second)
        .unwrap_err()
        .to_string()
        .contains("replay"));
}

#[test]
fn wrong_response_correlation_and_gap_are_rejected() {
    let root = Temp::new();
    let store = decision_record::DecisionRecordStore::new(root.0.clone());
    let mut wrong_correlation = resolution();
    wrong_correlation["response"]["correlationId"] = json!("corr-wrong");
    assert!(store
        .record(&wrong_correlation)
        .unwrap_err()
        .to_string()
        .contains("correlation"));

    let mut wrong_gap = resolution();
    wrong_gap["response"]["gapId"] = json!("gap-wrong");
    assert!(store
        .record(&wrong_gap)
        .unwrap_err()
        .to_string()
        .contains("gap"));
}

#[test]
fn branch_scope_mismatch_is_rejected_not_replayed() {
    let root = Temp::new();
    let store = decision_record::DecisionRecordStore::new(root.0.clone());
    store.record(&resolution()).unwrap();
    let mut query = replay_query();
    query["affectedBranches"] = json!(["unrelated"]);
    assert!(store
        .replay(&query)
        .unwrap_err()
        .to_string()
        .contains("branch scope"));
}

#[test]
fn invalid_stored_records_are_ignored_with_diagnosis() {
    let root = Temp::new();
    fs::write(root.0.join("forged.json"), b"{\"schema\":\"forged\"}").unwrap();
    let store = decision_record::DecisionRecordStore::new(root.0.clone());
    let result = store.replay(&replay_query()).unwrap();
    assert_eq!(result["status"], "reopen");
    assert_eq!(result["reason"], "no_record");
    assert_eq!(result["diagnostics"].as_array().unwrap().len(), 1);
}

#[test]
fn committed_store_uses_a06_a07_manifest_marker_and_real_schema_validation() {
    let root = Temp::new();
    let store = decision_record::DecisionRecordStore::new(root.0.join("records"));
    let result = store.record(&resolution()).unwrap();
    let record = &result["record"];
    let bytes = serde_json::to_vec_pretty(record).unwrap();
    decision_record::validate_decision_record_artifact(&bytes).unwrap();

    let runs = committed_runs(&root.0.join("records"));
    assert_eq!(runs.len(), 1);
    assert_eq!(run_commit::classify(&runs[0]), "committed");
    let (marker, manifest) = run_commit::validate_committed_run(&runs[0]).unwrap();
    assert_eq!(marker["schema"], "code-intel-run-commit.v1");
    assert_eq!(manifest["schema"], "code-intel-run-manifest.v1");
    assert!(runs[0].join("run-complete.json").is_file());
    let artifact = &manifest["nodes"]["decision_record"]["artifacts"][0];
    assert_eq!(artifact["artifactSchema"], "code-intel-decision-record.v1");
    assert_eq!(artifact["type"], "decision.record");
    let artifact_path = runs[0].join(artifact["path"].as_str().unwrap());
    let emitted = fs::read(&artifact_path).unwrap();
    decision_record::validate_decision_record_artifact(&emitted).unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&emitted).unwrap(), *record);

    let schema_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("orchestration/schemas");
    let checked_in_schema: Value = serde_json::from_slice(
        &fs::read(schema_dir.join("code-intel-decision-record.v1.schema.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        checked_in_schema["$id"],
        "code-intel-decision-record.v1.schema.json"
    );
    assert_eq!(checked_in_schema["additionalProperties"], false);
    assert_eq!(
        checked_in_schema["properties"]["schema"]["const"],
        "code-intel-decision-record.v1"
    );

    let mut malformed = record.clone();
    malformed["consequences"] = json!([1]);
    assert!(decision_record::validate_decision_record_artifact(
        &serde_json::to_vec(&malformed).unwrap()
    )
    .is_err());

    let reopened = decision_record::DecisionRecordStore::new(root.0.join("records"));
    assert_eq!(
        reopened.replay(&replay_query()).unwrap()["status"],
        "replay"
    );
}

#[test]
fn store_lock_serializes_same_binding_and_authority_reuse_across_processes() {
    let root = Temp::new();
    let same_store = root.0.join("same-store");
    let same_path = root.0.join("same.json");
    fs::write(&same_path, serde_json::to_vec(&resolution()).unwrap()).unwrap();
    let first = spawn_record(&same_path, &same_store);
    let second = spawn_record(&same_path, &same_store);
    let outputs = wait_record_pair([first, second]);
    assert!(outputs.iter().all(|output| output.status.success()));
    let mut statuses = outputs
        .iter()
        .map(|output| {
            serde_json::from_slice::<Value>(&output.stdout).unwrap()["status"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect::<Vec<_>>();
    statuses.sort();
    assert_eq!(statuses, ["recorded", "replay"]);
    assert_eq!(committed_runs(&same_store).len(), 1);

    let authority_store = root.0.join("authority-store");
    let first_path = root.0.join("first.json");
    let second_path = root.0.join("second.json");
    fs::write(&first_path, serde_json::to_vec(&resolution()).unwrap()).unwrap();
    let mut second_resolution = resolution();
    second_resolution["gap"]["id"] = json!("gap-second");
    second_resolution["request"]["gapId"] = json!("gap-second");
    second_resolution["response"]["gapId"] = json!("gap-second");
    second_resolution["request"]["correlationId"] = json!("corr-second");
    second_resolution["response"]["correlationId"] = json!("corr-second");
    fs::write(
        &second_path,
        serde_json::to_vec(&second_resolution).unwrap(),
    )
    .unwrap();
    let first = spawn_record(&first_path, &authority_store);
    let second = spawn_record(&second_path, &authority_store);
    let outputs = wait_record_pair([first, second]);
    assert_eq!(
        outputs
            .iter()
            .filter(|output| output.status.success())
            .count(),
        1
    );
    assert_eq!(committed_runs(&authority_store).len(), 1);
    let rejected = outputs
        .iter()
        .find(|output| !output.status.success())
        .unwrap();
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("authority event replay"));
}

#[test]
fn binding_and_replay_reject_temporal_backdating() {
    let root = Temp::new();
    let store = decision_record::DecisionRecordStore::new(root.0.clone());
    let mut late = resolution();
    late["recordedAt"] = json!(2160);
    late["authorityEvent"]["expiresAt"] = json!(2190);
    assert!(store
        .record(&late)
        .unwrap_err()
        .to_string()
        .contains("outside request"));

    store.record(&resolution()).unwrap();
    let mut before_record = replay_query();
    before_record["now"] = json!(1999);
    assert!(store
        .replay(&before_record)
        .unwrap_err()
        .to_string()
        .contains("precedes"));
    let mut before_observation = replay_query();
    before_observation["evidenceRefs"][0]["observedAt"] = json!(2050);
    before_observation["now"] = json!(2025);
    assert!(store
        .replay(&before_observation)
        .unwrap_err()
        .to_string()
        .contains("observed evidence"));
}

#[test]
fn store_entry_failures_and_symlinks_are_reported_not_silently_filtered() {
    let root = Temp::new();
    fs::write(root.0.join("forged.json"), b"{\"schema\":\"forged\"}").unwrap();
    fs::create_dir(root.0.join("uncommitted-run")).unwrap();
    let mut expected = 2;
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        symlink(root.0.join("forged.json"), root.0.join("record-link")).unwrap();
        expected += 1;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::symlink_file;
        if symlink_file(root.0.join("forged.json"), root.0.join("record-link")).is_ok() {
            expected += 1;
        }
    }
    let store = decision_record::DecisionRecordStore::new(root.0.clone());
    let result = store.replay(&replay_query()).unwrap();
    let diagnostics = result["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), expected);
    let text = serde_json::to_string(diagnostics).unwrap();
    assert!(text.contains("forged.json"));
    assert!(text.contains("uncommitted-run"));
    if expected == 3 {
        assert!(text.contains("symlink"));
    }
}

#[test]
fn production_cli_registry_and_schema_are_wired() {
    let root = Temp::new();
    let resolution_path = root.0.join("resolution.json");
    let query_path = root.0.join("query.json");
    let store_path = root.0.join("records");
    fs::write(&resolution_path, serde_json::to_vec(&resolution()).unwrap()).unwrap();
    fs::write(&query_path, serde_json::to_vec(&replay_query()).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args([
            "decision",
            "record",
            "--resolution",
            resolution_path.to_str().unwrap(),
            "--store",
            store_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["status"],
        "recorded"
    );
    let replay = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args([
            "decision",
            "replay",
            "--query",
            query_path.to_str().unwrap(),
            "--store",
            store_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(
        replay.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&replay.stderr)
    );
    let replay: Value = serde_json::from_slice(&replay.stdout).unwrap();
    assert_eq!(replay["status"], "replay");
    assert_eq!(replay["questionRequired"], false);

    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let manifest: Value =
        serde_json::from_slice(&fs::read(repo.join("orchestration/integrations.json")).unwrap())
            .unwrap();
    let entry = manifest["integrations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == "decision.record")
        .unwrap();
    assert_eq!(entry["required"], true);
    assert!(entry["commands"]["record"]
        .as_str()
        .unwrap()
        .contains("decision record"));
    let schema: Value = serde_json::from_slice(
        &fs::read(repo.join("orchestration/schemas/code-intel-decision-record.v1.schema.json"))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(
        schema["properties"]["reopenRule"]["additionalProperties"],
        false
    );
}
