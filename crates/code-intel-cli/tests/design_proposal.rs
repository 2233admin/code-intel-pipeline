mod common;
#[path = "support/sha256.rs"]
mod sha256;

use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);
const CAPABILITY: &str = "advisory.design-proposal.compat";
const REQUEST_SCHEMA: &str = "code-intel-design-proposal-request.v1";
const SNAPSHOT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

struct TempTree(PathBuf);
impl Drop for TempTree {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
}

fn temp_repo(label: &str) -> TempTree {
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).expect("clock").as_nanos();
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!("code-intel-design-proposal-{label}-{}-{nonce}-{sequence}", std::process::id()));
    fs::create_dir_all(root.join("repo")).expect("create temporary repository");
    fs::write(root.join("repo/source.rs"), "fn fixture() {}\n").expect("write repository fixture");
    TempTree(root)
}

fn snapshot_for(repo: &Path) -> Value {
    let output = common::cli().args(["snapshot", "identity", "--repo"]).arg(repo).args(["--working-tree-policy", "explicit_overlay", "--scope", "."]).output().expect("compute snapshot");
    assert!(output.status.success(), "snapshot stderr={}", String::from_utf8_lossy(&output.stderr));
    serde_json::from_slice::<Value>(&output.stdout).expect("snapshot JSON")["snapshot"].clone()
}

fn context_request(repo: &Path, out: &Path) -> Value {
    json!({"schema": REQUEST_SCHEMA, "capability": CAPABILITY, "contractVersion": 1, "snapshot": snapshot_for(repo), "options": {"repoPath": repo, "mode": "context"}, "inputs": [], "effectPolicy": {"allowedEffects": ["repo_read", "local_write"]}})
}

fn artifact_ref(path: &Path, schema: &str, kind: &str, snapshot: &Value) -> Value {
    json!({"schema":"code-intel-artifact-ref.v1","artifactSchema":schema,"type":kind,"path":path.file_name().unwrap().to_string_lossy(),"sha256":sha256::sha256(path),"consumedSnapshotIdentity":snapshot["identity"]})
}

fn validate_request(repo: &Path, context: &Path, candidate: &Path, out: &Path) -> Value {
    let snapshot = snapshot_for(repo);
    json!({"schema": REQUEST_SCHEMA, "capability": CAPABILITY, "contractVersion": 1, "snapshot": snapshot, "options": {"repoPath": repo, "mode": "validate"}, "inputs": [artifact_ref(context, "code-intel-design-context.v1", "design.context", &snapshot), artifact_ref(candidate, "code-intel-design-proposal-candidate.v1", "design.proposal-candidate", &snapshot)], "effectPolicy": {"allowedEffects": ["repo_read", "local_write"]}})
}

fn run_capability(request: &Value, path: &Path, out: &Path) -> Output {
    fs::write(path, serde_json::to_vec_pretty(request).expect("serialize request")).expect("write request");
    common::cli().args(["capability", "exec", CAPABILITY, "--request"]).arg(path).args(["--out"]).arg(out).args(["--artifact-root"]).arg(path.parent().unwrap()).output().expect("run capability")
}

fn fixture(name: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/design-proposal").join(name);
    serde_json::from_slice(&fs::read(path).expect("read proposal fixture")).expect("proposal fixture JSON")
}

fn as_path(value: &Value) -> PathBuf {
    PathBuf::from(value.as_str().expect("path value"))
}

fn setup(name: &str) -> (TempTree, PathBuf, PathBuf, Value) {
    let tree = temp_repo(name);
    let repo = tree.0.join("repo");
    let out = tree.0.join("out");
    let mut candidate = fixture(name);
    if name != "stale-snapshot.json" {
        candidate["snapshot"] = snapshot_for(&repo);
    }
    let candidate_path = tree.0.join("candidate.json");
    fs::write(&candidate_path, serde_json::to_vec_pretty(&candidate).unwrap()).unwrap();
    let context_path = tree.0.join("context.json");
    let context = json!({"schema":"code-intel-design-context.v1","type":"design.context","snapshot":candidate["snapshot"],"evidenceRefs":["artifact://sha256/1111111111111111111111111111111111111111111111111111111111111111","artifact://sha256/2222222222222222222222222222222222222222222222222222222222222222","artifact://sha256/3333333333333333333333333333333333333333333333333333333333333333"],"methods":["contract-testing"]});
    fs::write(&context_path, serde_json::to_vec_pretty(&context).unwrap()).unwrap();
    (tree, repo, out, json!({"candidate":candidate_path,"context":context_path}))
}


fn published_artifact_ref<'a>(result: &'a Value, schema: &str, kind: &str) -> &'a Value {
    result["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|artifact| {
            artifact["schema"] == "code-intel-artifact-ref.v1"
                && artifact["artifactSchema"] == schema
                && artifact["type"] == kind
        })
        .expect("published artifact ref")
}

fn staged_artifact_payload(root: &Path, artifact_ref: &Value, schema: &str, kind: &str) -> Value {
    assert_eq!(artifact_ref["schema"], "code-intel-artifact-ref.v1");
    assert_eq!(artifact_ref["artifactSchema"], schema);
    assert_eq!(artifact_ref["type"], kind);
    let relative_path = artifact_ref["path"].as_str().expect("artifact ref path");
    assert!(
        Path::new(relative_path).is_relative(),
        "artifact ref path must be relative"
    );
    let path = root.join(relative_path);
    let bytes = fs::read(&path).expect("staged artifact bytes");
    assert_eq!(artifact_ref["sha256"], sha256::sha256(&path));
    let payload: Value = serde_json::from_slice(&bytes).expect("staged proposal payload");
    assert_eq!(
        artifact_ref["consumedSnapshotIdentity"],
        payload["snapshot"]["identity"]
    );
    payload
}

#[test]
fn valid_two_option_candidate_stages_advisory_result() {
    let (tree, repo, out, paths) = setup("valid-two-option.json");
    let output = run_capability(&validate_request(&repo, &as_path(&paths["context"]), &as_path(&paths["candidate"]), &out), &tree.0.join("request.json"), &out);
    assert!(output.status.success(), "stderr={}", String::from_utf8_lossy(&output.stderr));
    let result: Value = serde_json::from_slice(&output.stdout).expect("result JSON");
    assert_eq!(result["schema"], "code-intel-capability-result.v1");
    let artifact_ref = published_artifact_ref(&result, "code-intel-design-proposal.v1", "design.proposal");
    let payload = staged_artifact_payload(&out, artifact_ref, "code-intel-design-proposal.v1", "design.proposal");
    assert_eq!(payload["schema"], "code-intel-design-proposal.v1");
    assert_eq!(payload["authority"], "advisory_only");
    assert_eq!(payload["recommendation"]["optionId"], "option-a");
    assert_eq!(payload["snapshot"]["identity"], snapshot_for(&repo)["identity"]);
    let ids: Vec<_> = payload["options"].as_array().unwrap().iter().map(|option| option["id"].as_str().unwrap()).collect();
    assert_eq!(ids, vec!["option-a", "option-b"]);
}
#[test]
fn valid_three_option_candidate_preserves_all_ids() {
    let (tree, repo, out, paths) = setup("valid-three-option.json");
    let output = run_capability(&validate_request(&repo, &as_path(&paths["context"]), &as_path(&paths["candidate"]), &out), &tree.0.join("request.json"), &out);
    assert!(output.status.success(), "stderr={}", String::from_utf8_lossy(&output.stderr));
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["schema"], "code-intel-capability-result.v1");
    let artifact_ref = published_artifact_ref(&result, "code-intel-design-proposal.v1", "design.proposal");
    let payload = staged_artifact_payload(&out, artifact_ref, "code-intel-design-proposal.v1", "design.proposal");
    assert_eq!(payload["schema"], "code-intel-design-proposal.v1");
    assert_eq!(payload["authority"], "advisory_only");
    assert_eq!(payload["recommendation"]["optionId"], "option-b");
    assert_eq!(payload["snapshot"]["identity"], snapshot_for(&repo)["identity"]);
    let ids: Vec<_> = payload["options"].as_array().unwrap().iter().map(|o| o["id"].as_str().unwrap()).collect();
    assert_eq!(ids, vec!["option-a", "option-b", "option-c"]);
}
#[test]
fn context_mode_stages_design_context() {
    let tree = temp_repo("context-mode");
    let repo = tree.0.join("repo");
    let out = tree.0.join("context-out");
    let output = run_capability(&context_request(&repo, &out), &tree.0.join("context-request.json"), &out);
    assert!(output.status.success(), "stderr={}", String::from_utf8_lossy(&output.stderr));
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(result["artifacts"].as_array().unwrap().iter().any(|a| a["artifactSchema"] == "code-intel-design-context.v1" && a["type"] == "design.context"));
}

fn assert_invalid(fixture_name: &str, rule: &str) {
    let (tree, repo, out, paths) = setup(fixture_name);
    let output = run_capability(&validate_request(&repo, &as_path(&paths["context"]), &as_path(&paths["candidate"]), &out), &tree.0.join("request.json"), &out);
    assert!(!output.status.success(), "expected failure");
    let text = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    assert!(text.contains(rule), "missing {rule}: {text}");
    assert!(!text.contains("code-intel-design-proposal.v1"));
    assert!(!out.join("proposal.json").exists());
}

#[test] fn invalid_recommendation_is_rejected() { assert_invalid("invalid-recommendation.json", "proposal_option_reference_invalid"); }
#[test] fn stale_snapshot_is_rejected() { assert_invalid("stale-snapshot.json", "proposal_snapshot_mismatch"); }
#[test] fn drifted_evidence_is_rejected() { assert_invalid("drifted-evidence.json", "proposal_evidence_drifted"); }
#[test] fn missing_method_evidence_is_rejected() { assert_invalid("missing-method-evidence.json", "proposal_method_not_applicable"); }
#[test] fn authority_escalation_is_rejected() { assert_invalid("authority-escalation.json", "proposal_authority_escalation"); }

#[test]
fn option_count_diagnostic_rejects_one_and_four_options() {
    let (tree, repo, out, paths) = setup("invalid-option-count.json");
    for count in [1usize, 4] {
        let candidate_path = tree.0.join(format!("candidate-{count}.json"));
        let mut candidate = fixture("invalid-option-count.json");
        let options = candidate["options"].as_array().unwrap().clone();
        let mut four = options.clone();
        four.push(options[0].clone());
        four.push(options[0].clone());
        four.push(options[0].clone());
        for (option, id) in four.iter_mut().zip(["option-a", "option-b", "option-c", "option-d"]) {
            option["id"] = json!(id);
        }
        candidate["snapshot"] = snapshot_for(&repo);
        candidate["options"] = if count == 1 { json!(options) } else { json!(four) };
        fs::write(&candidate_path, serde_json::to_vec(&candidate).unwrap()).unwrap();
        let output = run_capability(&validate_request(&repo, &as_path(&paths["context"]), &candidate_path, &out), &tree.0.join(format!("request-{count}.json")), &out);
        assert!(!output.status.success());
        let text = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
        assert!(text.contains("proposal_option_count"));
        assert!(!text.contains("code-intel-design-proposal.v1"));
        assert!(!out.join("proposal.json").exists());
    }
}
