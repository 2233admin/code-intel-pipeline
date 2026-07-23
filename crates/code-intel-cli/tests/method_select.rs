#[path = "../src/admissibility.rs"]
mod admissibility;
#[path = "../src/artifact_ref.rs"]
mod artifact_ref;
#[path = "../src/capability.rs"]
mod capability;
#[path = "../src/capability_inventory.rs"]
mod capability_inventory;
#[path = "../src/method_catalog.rs"]
mod method_catalog;
#[path = "../src/method_select.rs"]
mod method_select;
#[path = "../src/snapshot.rs"]
mod snapshot;
#[path = "../src/stable_artifact.rs"]
mod stable_artifact;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

const SNAPSHOT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "code-intel-c02-{}-{nonce}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
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

struct Case {
    root: Temp,
    request: Value,
}

fn repo_root() -> PathBuf {
    option_env!("CODE_INTEL_REPO_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."))
}

fn fixture(name: &str) -> Case {
    let descriptor: Value = serde_json::from_slice(
        &fs::read(
            repo_root()
                .join("crates/code-intel-cli/tests/fixtures/method-select")
                .join(name),
        )
        .unwrap(),
    )
    .unwrap();
    build_case(&descriptor["facts"], &descriptor["evidenceGaps"])
}

fn base(signals: &[&str], evidence_kinds: &[&str]) -> Case {
    build_case(
        &json!([{
            "id":"fact-1",
            "signalIds":signals,
            "evidenceKinds":evidence_kinds
        }]),
        &json!([]),
    )
}

fn build_case(facts: &Value, gaps: &Value) -> Case {
    let root = Temp::new();
    let (envelope, admission_id) = make_envelope(&root.0, facts, "primary");
    let bound_facts = facts
        .as_array()
        .unwrap()
        .iter()
        .map(|fact| {
            json!({
                "id":fact["id"],
                "signalIds":fact["signalIds"],
                "evidenceKinds":fact["evidenceKinds"],
                "admissionIds":[admission_id]
            })
        })
        .collect::<Vec<_>>();
    Case {
        root,
        request: json!({
            "schema":"code-intel-method-selection-request.v1",
            "snapshotIdentity":SNAPSHOT,
            "evaluatedAt":2_000u64,
            "maxEvidenceAgeSeconds":100u64,
            "admissions":[envelope],
            "facts":bound_facts,
            "evidenceGaps":gaps
        }),
    }
}

fn make_envelope(root: &Path, facts: &Value, tag: &str) -> (Value, String) {
    let payload = json!({
        "schema":"code-intel-evidence-payload.v1",
        "data":{"methodSelectionFacts":facts}
    });
    let bytes = serde_json::to_vec(&payload).unwrap();
    let sha = capability::sha256_hex(&bytes);
    let path = format!("payload-{tag}.json");
    fs::write(root.join(&path), bytes).unwrap();
    let request = json!({
        "schema":"code-intel-evidence-admissibility-request.v1",
        "expectedSnapshotIdentity":SNAPSHOT,
        "policy":{"evaluatedAt":2_000u64,"maxAgeSeconds":100u64},
        "observation":{
            "schema":"code-intel-observed-evidence.v1",
            "provider":{"id":"method-selection-fixture","implementation":{"id":"fixture.adapter","version":"1.0.0","digest":"b".repeat(64)}},
            "source":{"revision":format!("fixture-{tag}")},
            "consumedSnapshotIdentity":SNAPSHOT,
            "observedAt":1_950u64,
            "completeness":"complete",
            "claimedComplete":true,
            "payload":{"schema":"code-intel-artifact-ref.v1","artifactSchema":"code-intel-evidence-payload.v1","type":"observed.evidence.payload","path":path,"sha256":sha,"consumedSnapshotIdentity":SNAPSHOT},
            "provenance":{"collectionId":format!("fixture-{tag}"),"command":"fixture --emit","startedAt":1_949u64,"completedAt":1_950u64},
            "failure":{"kind":"none"}
        }
    });
    let admitted = admissibility::validate_for_consumer(&request, root).unwrap();
    let result = admitted.result().clone();
    let identity = result["admissionIdentity"].as_str().unwrap().to_string();
    (json!({"request":request,"result":result}), identity)
}

fn run(case: &Case) -> Value {
    run_request(&case.request, &case.root.0).unwrap()
}

fn run_error(case: &Case) -> String {
    run_request(&case.request, &case.root.0)
        .unwrap_err()
        .to_string()
}

fn run_request(request: &Value, artifact_root: &Path) -> Result<Value, method_select::SelectError> {
    let catalog = method_catalog::load_catalog(&repo_root().join("orchestration/methods")).unwrap();
    let rules = method_select::load_rule_table(
        &repo_root().join("orchestration/method-selection-rules.v1.json"),
        &catalog,
    )
    .unwrap();
    method_select::select(request, artifact_root, &catalog, &rules)
}

#[test]
fn dependency_delay_selects_critical_path_and_value_stream_as_tied_proposals() {
    let result = run(&fixture("dependency-delay.json"));
    assert_eq!(result["outcome"], "proposal");
    assert_eq!(result["tie"], true);
    let methods = result["matches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["methodId"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        methods,
        vec!["critical-path-pert", "value-stream-queue-delay"]
    );
    assert!(result["matches"].as_array().unwrap().iter().all(|item| {
        item["outcome"] == "proposal"
            && item["missingEvidence"].as_array().unwrap().is_empty()
            && item["cost"].is_object()
            && item["confidenceRules"].is_array()
    }));
}

#[test]
fn insufficient_evidence_returns_unknown_with_missing_evidence_explanation() {
    let result = run(&fixture("insufficient-evidence.json"));
    assert_eq!(result["outcome"], "unknown");
    assert_eq!(result["matches"][0]["outcome"], "unknown");
    assert!(!result["matches"][0]["missingEvidence"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(result["matches"][0]["selectionConfidence"], "unknown");
}

#[test]
fn unrelated_fact_is_a_false_positive_guard_and_returns_none() {
    let result = run(&base(
        &["unrelated-code-size-change"],
        &["arbitrary-measurement"],
    ));
    assert_eq!(result["outcome"], "none");
    assert!(result["matches"].as_array().unwrap().is_empty());
}

#[test]
fn contraindication_blocks_proposal_but_preserves_the_explanation() {
    let result = run(&base(
        &["integration-drift", "no-observable-contract"],
        &[
            "consumer-provider-contracts",
            "provider-verification-context",
        ],
    ));
    assert_eq!(result["outcome"], "none");
    assert_eq!(result["matches"][0]["methodId"], "contract-testing");
    assert_eq!(result["matches"][0]["outcome"], "none");
    assert!(!result["matches"][0]["triggeredContraindications"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[test]
fn selection_is_order_independent_and_stably_sorted() {
    let mut case = fixture("dependency-delay.json");
    let extra_facts = json!([{
        "id":"schedule-fact",
        "signalIds":["schedule-slippage"],
        "evidenceKinds":["activity-duration-estimates", "activity-network"]
    }]);
    let (extra, extra_id) = make_envelope(&case.root.0, &extra_facts, "order-extra");
    case.request["admissions"]
        .as_array_mut()
        .unwrap()
        .push(extra);
    case.request["facts"].as_array_mut().unwrap().push(json!({
        "id":"schedule-fact",
        "signalIds":["schedule-slippage"],
        "evidenceKinds":["activity-duration-estimates", "activity-network"],
        "admissionIds":[extra_id]
    }));
    let expected = run(&case);
    case.request["facts"].as_array_mut().unwrap().reverse();
    case.request["admissions"].as_array_mut().unwrap().reverse();
    case.request["evidenceGaps"]
        .as_array_mut()
        .unwrap()
        .reverse();
    assert_eq!(run(&case), expected);
}

#[test]
fn stale_unadmitted_unknown_and_duplicate_admissions_fail_closed() {
    let mut stale = base(&["dependency-congestion"], &["activity-network"]);
    stale.request["admissions"][0]["request"]["observation"]["observedAt"] = json!(1_899u64);
    stale.request["admissions"][0]["request"]["observation"]["provenance"]["startedAt"] =
        json!(1_898u64);
    stale.request["admissions"][0]["request"]["observation"]["provenance"]["completedAt"] =
        json!(1_899u64);
    assert!(run_error(&stale).contains("stale"));

    let mut unadmitted = base(&["dependency-congestion"], &["activity-network"]);
    unadmitted.request["admissions"][0]["result"]["status"] = json!("rejected");
    assert!(run_error(&unadmitted).contains("forged"));

    let mut unknown = base(&["dependency-congestion"], &["activity-network"]);
    unknown.request["facts"][0]["admissionIds"] = json!(["d".repeat(64)]);
    assert!(run_error(&unknown).contains("unknown admission"));

    let mut duplicate = base(&["dependency-congestion"], &["activity-network"]);
    let repeated = duplicate.request["admissions"][0].clone();
    duplicate.request["admissions"]
        .as_array_mut()
        .unwrap()
        .push(repeated);
    assert!(run_error(&duplicate).contains("duplicate admission identity"));
}

#[test]
fn forged_minimal_a04_result_and_caller_relabeling_are_rejected() {
    let mut forged = base(
        &["dependency-congestion"],
        &["activity-network", "activity-duration-estimates"],
    );
    forged.request["admissions"][0]["result"] = json!({
        "schema":"code-intel-evidence-admissibility-result.v1",
        "status":"admitted",
        "domainVerdict":"observed",
        "admissionIdentity":"b".repeat(64),
        "evidence":{"consumedSnapshotIdentity":SNAPSHOT,"observedAt":1_950u64},
        "verifiedPayload":{"consumedSnapshotIdentity":SNAPSHOT},
        "engineeringFacts":[]
    });
    assert!(run_error(&forged).contains("forged"));

    let mut relabeled = base(&["dependency-congestion"], &["activity-network"]);
    relabeled.request["facts"][0]["signalIds"] = json!(["schedule-slippage"]);
    assert!(run_error(&relabeled).contains("differ from A04 admitted payload"));
}

#[test]
fn every_admission_must_be_referenced_by_a_payload_bound_fact() {
    let mut case = fixture("dependency-delay.json");
    let unused_facts = json!([{
        "id":"unused-fact",
        "signalIds":["process-instability"],
        "evidenceKinds":["ordered-process-measurements"]
    }]);
    let (unused, _) = make_envelope(&case.root.0, &unused_facts, "unused");
    case.request["admissions"]
        .as_array_mut()
        .unwrap()
        .push(unused);
    assert!(run_error(&case).contains("unused admission"));
}

#[test]
fn output_is_advisory_only_and_contains_no_execution_fact_or_decision_claim() {
    let result = run(&fixture("dependency-delay.json"));
    let text = serde_json::to_string(&result).unwrap().to_ascii_lowercase();
    assert!(!text.contains("adoption_decision"));
    assert!(!text.contains("committed_engineering_plan"));
    assert!(!text.contains("method_executed"));
    assert!(result["matches"]
        .as_array()
        .unwrap()
        .iter()
        .all(|item| ["proposal", "unknown", "none"].contains(&item["outcome"].as_str().unwrap())));
}

#[test]
fn checked_rules_and_schema_are_closed_and_reference_c01_cards() {
    let rules: Value = serde_json::from_slice(
        &fs::read(repo_root().join("orchestration/method-selection-rules.v1.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(rules["schema"], "code-intel-method-selection-rules.v1");
    let schema: Value = serde_json::from_slice(
        &fs::read(
            repo_root().join("orchestration/schemas/code-intel-method-selection.v1.schema.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(schema["$id"], "code-intel-method-selection.v1");
    for definition in [
        "request",
        "admission",
        "fact",
        "match",
        "result",
        "cost",
        "confidenceRule",
    ] {
        assert_eq!(schema["$defs"][definition]["additionalProperties"], false);
    }
}
