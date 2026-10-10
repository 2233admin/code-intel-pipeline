mod common;
#[path = "support/sha256.rs"]
mod sha256;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static SEQ: AtomicU64 = AtomicU64::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "code-intel-admission-provider-{}-{nonce}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
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

fn sentrux(repo: &Path, operation: &str) -> Value {
    let output = common::cli()
        .args(["sentrux", operation])
        .arg(repo)
        .arg("--json")
        .output()
        .unwrap();
    serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
        panic!(
            "invalid {operation} JSON: {}",
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

// The admission claims below come from real public gate/check requests. No
// hand-authored pass verdict or copied implementation algorithm is a fixture.
fn case(root: &Path, quality_decline: bool) -> Value {
    let repo = root.join("repo");
    fs::create_dir_all(repo.join("src")).unwrap();
    fs::create_dir_all(repo.join(".sentrux")).unwrap();
    fs::write(repo.join("src/a.rs"), "pub fn alpha() {}\n").unwrap();
    fs::write(repo.join("src/b.rs"), "pub fn beta() {}\n").unwrap();
    fs::write(
        repo.join(".sentrux/rules.toml"),
        "[constraints]\nmax_cycles = 0\nno_god_files = false\n",
    )
    .unwrap();
    let saved = common::cli()
        .args(["sentrux", "--operation", "save_baseline", "--repo"])
        .arg(&repo)
        .output()
        .unwrap();
    assert!(
        saved.status.success(),
        "{}",
        String::from_utf8_lossy(&saved.stderr)
    );
    if quality_decline {
        let mut source = String::from("pub fn entry() {}\n");
        for i in 0..600 {
            source.push_str(&format!("// padding {i}\n"));
        }
        fs::write(repo.join("src/a.rs"), source).unwrap();
    }
    let gate = sentrux(&repo, "gate");
    let check = sentrux(&repo, "check");
    let snapshot = common::cli()
        .args(["snapshot", "identity", "--repo"])
        .arg(&repo)
        .args(["--working-tree-policy", "explicit_overlay"])
        .output()
        .unwrap();
    assert!(snapshot.status.success());
    let snapshot: Value = serde_json::from_slice(&snapshot.stdout).unwrap();
    let identity = &snapshot["snapshot"]["identity"];
    let rules = json!([
        {"kind":"sentrux_check","status":"evaluated","verdict":check["verdict"],"failure":{"kind":"none"}},
        {"kind":"sentrux_gate","status":"evaluated","verdict":gate["verdict"],"failure":{"kind":"none"}}
    ]);
    let mut advisories = gate["advisories"].as_array().unwrap().clone();
    for advisory in check["advisories"].as_array().unwrap() {
        if !advisories.contains(advisory) {
            advisories.push(advisory.clone());
        }
    }
    let gate_results = json!([{"kind":"sentrux_gate","admission":gate}, {"kind":"sentrux_check","admission":check}]);
    let effects = json!(["local_write", "process_spawn", "repo_read"]);
    let payload = json!({"schema":"code-intel-evidence-payload.v1","data":{"structuralEvidence":{
        "schema":"code-intel-structural-evidence-payload.v2", "gatePolicy":gate["policy"],
        "gateResults":gate_results, "advisories":advisories, "snapshotIdentity":identity,
        "provider":{"implementationId":"sentrux.command-adapter","rollbackIdentity":"external gate/check"},
        "provenance":{"sourceRevision":gate["current"]["sourceCommit"]},
        "effects":{"declared":effects,"observed":effects,"match":true},
        "completeness":"complete", "rules":rules
    }}});
    let bytes = serde_json::to_vec(&payload).unwrap();
    fs::write(root.join("payload.json"), &bytes).unwrap();
    json!({
        "schema":"code-intel-sentrux-provider-native.v2", "gatePolicy":gate["policy"],
        "gateResults":gate_results, "status":"complete",
        "implementation":{"id":"sentrux.command-adapter","version":"1.0.0","digest":"b".repeat(64)},
        "rollbackIdentity":"external gate/check", "sourceRevision":gate["current"]["sourceCommit"],
        "expectedSnapshotIdentity":identity, "sourceSnapshotIdentity":identity,
        "collectedAt":1940,"observedAt":1950,"declaredEffects":effects,"observedEffects":effects,
        "authoritativeRules":rules,"nativeFailure":{"kind":"none"},
        "payload":{"schema":"code-intel-artifact-ref.v1","artifactSchema":"code-intel-evidence-payload.v1","type":"observed.evidence.payload","path":"payload.json","sha256":sha256::sha256_hex(&bytes),"consumedSnapshotIdentity":identity}
    })
}

fn route(root: &Path, native: &Value) -> (i32, Value, String) {
    let request = root.join("native.json");
    fs::write(&request, serde_json::to_vec(native).unwrap()).unwrap();
    let output = common::cli()
        .args(["provider", "sentrux-adapt", "--request"])
        .arg(request)
        .arg("--artifact-root")
        .arg(root)
        .args(["--evaluated-at", "2000", "--max-age-seconds", "100"])
        .output()
        .unwrap();
    (
        output.status.code().unwrap(),
        serde_json::from_slice(&output.stdout).unwrap(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn replace_payload(root: &Path, native: &mut Value, change: impl FnOnce(&mut Value)) {
    let mut payload: Value =
        serde_json::from_slice(&fs::read(root.join("payload.json")).unwrap()).unwrap();
    change(&mut payload);
    let bytes = serde_json::to_vec(&payload).unwrap();
    fs::write(root.join("payload.json"), &bytes).unwrap();
    native["payload"]["sha256"] = json!(sha256::sha256_hex(&bytes));
}

#[test]
fn real_quality_only_decline_is_admitted_as_advisory_not_a_hard_violation() {
    let root = Temp::new();
    let native = case(&root.0, true);
    let (code, result, stderr) = route(&root.0, &native);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(result["admission"]["domainVerdict"], "observed");
    assert_eq!(result["adapter"]["port"]["diagnosisEligible"], true);
    let port = &result["adapter"]["port"];
    assert_eq!(port["advisories"].as_array().unwrap().len(), 1);
    assert_eq!(port["advisories"][0]["rule"], "quality_degraded");
    for entry in port["gateResults"].as_array().unwrap() {
        assert_eq!(entry["admission"]["verdict"], "pass");
        assert_eq!(entry["admission"]["blockingViolations"], json!([]));
        let quality = entry["admission"]["comparisons"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["rule"] == "quality_degraded")
            .unwrap();
        assert!(quality["after"].as_f64().unwrap() < quality["before"].as_f64().unwrap());
        assert_eq!(quality["disposition"], "advisory");
        assert_eq!(quality["verdict"], "fail");
    }
    assert_eq!(result["engineeringFacts"], json!([]));
}

#[test]
fn external_exit_zero_rule_labels_cannot_replace_a_typed_policy_handshake() {
    let root = Temp::new();
    let mut native = case(&root.0, false);
    for entry in native["gateResults"].as_array_mut().unwrap() {
        entry["admission"] = Value::Null;
    }
    replace_payload(&root.0, &mut native, |payload| {
        payload["data"]["structuralEvidence"]["gateResults"] = json!([
            {"kind":"sentrux_gate","admission":null},{"kind":"sentrux_check","admission":null}
        ]);
        payload["data"]["structuralEvidence"]["completeness"] = json!("partial");
        for rule in payload["data"]["structuralEvidence"]["rules"]
            .as_array_mut()
            .unwrap()
        {
            rule["status"] = json!("not_evaluated");
            rule["verdict"] = json!("unknown");
            rule["failure"] = json!({"kind":"domain_unknown","message":"same-policy typed gate admission is unavailable"});
        }
    });
    let (code, result, stderr) = route(&root.0, &native);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(result["admission"]["domainVerdict"], "unknown");
    assert_eq!(result["adapter"]["port"]["diagnosisEligible"], false);
    assert_eq!(result["adapter"]["port"]["completeness"], "partial");
}

#[test]
fn missing_evaluation_or_crashed_provider_cannot_promote_a_complete_label() {
    for state in ["missing_gate", "not_evaluated", "crashed"] {
        let root = Temp::new();
        let mut native = case(&root.0, false);
        match state {
            "missing_gate" => native["authoritativeRules"]
                .as_array_mut()
                .unwrap()
                .retain(|rule| rule["kind"] != "sentrux_gate"),
            "not_evaluated" => {
                let rule = &mut native["authoritativeRules"][0];
                rule["status"] = json!("not_evaluated");
                rule["verdict"] = json!("unknown");
                rule["failure"] = json!({"kind":"domain_unknown","message":"provider did not evaluate this command"});
            }
            "crashed" => {
                native["status"] = json!("crashed");
                native["nativeFailure"] =
                    json!({"kind":"provider_unavailable","message":"provider process crashed"});
                native["authoritativeRules"] = json!([]);
                for result in native["gateResults"].as_array_mut().unwrap() {
                    result["admission"] = Value::Null;
                }
            }
            _ => unreachable!(),
        }
        let rules = native["authoritativeRules"].clone();
        let results = native["gateResults"].clone();
        replace_payload(&root.0, &mut native, |payload| {
            let evidence = &mut payload["data"]["structuralEvidence"];
            evidence["completeness"] = json!("partial");
            evidence["rules"] = rules;
            evidence["gateResults"] = results;
        });
        let (code, result, stderr) = route(&root.0, &native);
        assert_eq!(code, 0, "{state}: {stderr}");
        assert_eq!(result["admission"]["domainVerdict"], "unknown", "{state}");
        assert_eq!(
            result["adapter"]["port"]["diagnosisEligible"], false,
            "{state}"
        );
    }
}

#[test]
fn forged_missing_old_or_contradictory_policy_claims_are_rejected_at_public_route() {
    let root = Temp::new();
    let base = case(&root.0, false);
    let mut cases = Vec::new();
    let mut value = base.clone();
    value["observedEffects"] = json!(["repo_read"]);
    cases.push(value);
    let mut value = base.clone();
    value.as_object_mut().unwrap().remove("gatePolicy");
    cases.push(value);
    let mut value = base.clone();
    value["gatePolicy"]["sha256"] = json!("f".repeat(64));
    cases.push(value);
    let mut value = base.clone();
    value["schema"] = json!("code-intel-sentrux-provider-native.v1");
    cases.push(value);
    let mut value = base.clone();
    value["gateResults"][0]["admission"]["schema"] = json!("code-intel-sentrux-gate-result.v0");
    cases.push(value);
    let mut value = base.clone();
    value["gateResults"][0]["admission"]["measurement"]["evidenceProfile"] = json!("four-factor");
    cases.push(value);
    let mut value = base.clone();
    value["gateResults"][0]["admission"]["comparisons"][0]["verdict"] = json!("fail");
    cases.push(value);
    let mut value = base.clone();
    value["gateResults"][0]["admission"]["current"]["scope"] = json!("src");
    cases.push(value);
    let mut value = base.clone();
    value["gateResults"][0]["admission"]["current"]["sourceCommit"] = json!("foreign-revision");
    cases.push(value);
    let mut value = base.clone();
    value["implementation"]["id"] = json!("forged-provider");
    cases.push(value);
    let mut value = base.clone();
    value["authoritativeRules"][0]["verdict"] = json!("fail");
    cases.push(value);
    let mut value = base.clone();
    value["gateResults"][0]["admission"]["baseline"]["schema"] =
        json!("code-intel-sentrux-baseline.v5");
    cases.push(value);
    let mut value = base.clone();
    value["gateResults"][0]["admission"]["comparisons"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|comparison| comparison["rule"] == "cycles_increased")
        .unwrap()["after"] = json!(0.5);
    cases.push(value);
    let mut value = base.clone();
    value["gateResults"][0]["admission"]["comparisons"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|comparison| comparison["rule"] == "god_files_increased")
        .unwrap()["after"] = json!(["../escaped.rs"]);
    cases.push(value);
    let mut value = base.clone();
    value["gateResults"][0]["admission"]["comparisons"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|comparison| comparison["rule"] == "coupling_increased")
        .unwrap()["disposition"] = json!("advisory");
    cases.push(value);
    let mut value = base.clone();
    let quality = value["gateResults"][0]["admission"]["comparisons"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|comparison| comparison["rule"] == "quality_degraded")
        .unwrap();
    // Equal fractional values keep comparison/aggregate verdicts and empty
    // diagnostics coherent; only the integer measurement profile is forged.
    quality["before"] = json!(7777.5);
    quality["after"] = json!(7777.5);
    cases.push(value);
    for native in cases {
        let (code, result, _) = route(&root.0, &native);
        assert_eq!(code, 65, "{result}");
        assert_eq!(result["status"], "rejected");
        assert!(result["admission"].is_null());
    }
}

#[test]
fn stale_snapshot_and_digest_bound_payload_relabel_cannot_be_admitted() {
    let root = Temp::new();
    let base = case(&root.0, false);
    let mut native = base.clone();
    native["expectedSnapshotIdentity"] = json!("f".repeat(64));
    let (code, result, stderr) = route(&root.0, &native);
    assert_eq!(code, 0, "{stderr}");
    assert_ne!(result["admission"]["domainVerdict"], "observed");
    assert_eq!(result["adapter"]["port"]["diagnosisEligible"], false);
    let mut native = base;
    replace_payload(&root.0, &mut native, |payload| {
        payload["data"]["structuralEvidence"]["gatePolicy"]["policyVersion"] = json!(2);
    });
    assert_eq!(route(&root.0, &native).0, 65);
}

#[test]
fn secret_shaped_extra_input_is_rejected_without_echo() {
    let root = Temp::new();
    let mut native = case(&root.0, false);
    native["apiToken"] = json!("SENTINEL_DO_NOT_ECHO");
    let (code, result, stderr) = route(&root.0, &native);
    assert_eq!(code, 65);
    assert!(!format!("{result}{stderr}").contains("SENTINEL_DO_NOT_ECHO"));
}
