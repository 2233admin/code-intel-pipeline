mod common;

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
    json!({"schema": REQUEST_SCHEMA, "capability": CAPABILITY, "contractVersion": 1, "mode": "context", "snapshot": snapshot_for(repo), "options": {"repoPath": repo}, "out": out, "inputs": [], "effectPolicy": {"allowedEffects": ["repo_read", "local_write"]}})
}

fn artifact_ref(path: &Path, schema: &str, kind: &str, snapshot: &Value) -> Value {
    json!({"path": path, "artifactSchema": schema, "type": kind, "sha256": "1111111111111111111111111111111111111111111111111111111111111111", "consumedSnapshotIdentity": snapshot["identity"], "verification": "verified"})
}

fn validate_request(repo: &Path, context: &Path, candidate: &Path, out: &Path) -> Value {
    let snapshot = snapshot_for(repo);
    json!({"schema": REQUEST_SCHEMA, "capability": CAPABILITY, "contractVersion": 1, "mode": "validate", "snapshot": snapshot, "options": {"repoPath": repo}, "inputs": [artifact_ref(context, "code-intel-design-context.v1", "design.context", &snapshot), artifact_ref(candidate, "code-intel-design-proposal-candidate.v1", "design.proposal-candidate", &snapshot)], "effectPolicy": {"allowedEffects": ["repo_read", "local_write"]}, "out": out})
}

fn run_capability(request: &Value, path: &Path, out: &Path) -> Output {
    fs::write(path, serde_json::to_vec_pretty(request).expect("serialize request")).expect("write request");
    common::cli().args(["capability", "exec", CAPABILITY, "--request"]).arg(path).args(["--out"]).arg(out).output().expect("run capability")
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
    fs::create_dir_all(&out).expect("create output");
    let mut candidate = fixture(name);
    candidate["snapshot"] = snapshot_for(&repo);
    let candidate_path = tree.0.join("candidate.json");
    fs::write(&candidate_path, serde_json::to_vec_pretty(&candidate).unwrap()).unwrap();
    let context_path = tree.0.join("context.json");
    let context = json!({"schema":"code-intel-design-context.v1","type":"design.context","snapshot":candidate["snapshot"],"evidenceRefs":["artifact://sha256/1111111111111111111111111111111111111111111111111111111111111111"],"methods":["method-contract-testing"]});
    fs::write(&context_path, serde_json::to_vec_pretty(&context).unwrap()).unwrap();
    (tree, repo, out, json!({"candidate":candidate_path,"context":context_path}))
}

#[test]
fn valid_two_option_candidate_stages_advisory_result() {
    let (tree, repo, out, paths) = setup("valid-two-option.json");
    let request_path = tree.0.join("request.json");
    let output = run_capability(&validate_request(&repo, &as_path(&paths["context"]), &as_path(&paths["candidate"]), &out), &request_path, &out);
    assert!(output.status.success(), "stderr={}", String::from_utf8_lossy(&output.stderr));
    let result: Value = serde_json::from_slice(&output.stdout).expect("result JSON");
    assert_eq!(result["authority"], "advisory_only");
    assert_eq!(result["schema"], "code-intel-design-proposal.v1");
    assert_eq!(result["snapshot"]["identity"], snapshot_for(&repo)["identity"]);
}

#[test]
fn valid_three_option_candidate_preserves_all_ids() {
    let (tree, repo, out, paths) = setup("valid-three-option.json");
    let output = run_capability(&validate_request(&repo, &as_path(&paths["context"]), &as_path(&paths["candidate"]), &out), &tree.0.join("request.json"), &out);
    assert!(output.status.success(), "stderr={}", String::from_utf8_lossy(&output.stderr));
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["options"].as_array().unwrap().iter().map(|o| o["id"].as_str().unwrap()).collect::<Vec<_>>(), vec!["option-a", "option-b", "option-c"]);
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
        candidate["snapshot"] = snapshot_for(&repo);
        candidate["options"] = if count == 1 { json!(options) } else { json!([options[0].clone(), options[0].clone(), options[0].clone(), options[0].clone()]) };
        fs::write(&candidate_path, serde_json::to_vec(&candidate).unwrap()).unwrap();
        let output = run_capability(&validate_request(&repo, &as_path(&paths["context"]), &candidate_path, &out), &tree.0.join(format!("request-{count}.json")), &out);
        assert!(!output.status.success());
        assert!(format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr)).contains("proposal_option_count"));
    }
}
