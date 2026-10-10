//! Binary-level coverage for the #165 identity ratchet: the unit tests in
//! `sentrux_gate.rs` pin `run_gate` directly; these prove the same contract
//! holds through the shipped CLI surface (`sentrux --operation save_baseline`
//! / `--operation check`), which is what the authoritative self-scan and CI
//! actually invoke.
mod common;

use std::fs;
use std::path::PathBuf;
use std::process::Command;

const ROOT_CAUSES: [&str; 5] = [
    "modularity",
    "acyclicity",
    "depth",
    "equality",
    "redundancy",
];

fn fixture_root(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "sentrux-gate-cli-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ));
    fs::create_dir_all(root.join("src")).expect("create fixture root");
    root
}

fn code_intel(args: &[&str]) -> std::process::Output {
    common::cli().args(args).output().expect("run code-intel")
}

fn write_rules(root: &PathBuf) {
    fs::create_dir_all(root.join(".sentrux")).expect("create .sentrux");
    fs::write(
        root.join(".sentrux/rules.toml"),
        "[constraints]\nmax_cycles = 0\nno_god_files = false\n",
    )
    .expect("write rules.toml");
}

fn god_file_body(lines: usize) -> String {
    let mut body = String::from("pub fn entry() {}\n");
    for line in 0..lines {
        body.push_str(&format!("// padding {line}\n"));
    }
    body
}

fn assert_health_contract(health: &serde_json::Value) {
    let root_causes = health["root_causes"]
        .as_object()
        .expect("health root_causes object");
    assert_eq!(root_causes.len(), ROOT_CAUSES.len());
    for name in ROOT_CAUSES {
        let root_cause = root_causes
            .get(name)
            .unwrap_or_else(|| panic!("health root_causes is missing {name}"));
        assert!(root_cause["score"].is_number(), "{name}.score is numeric");
        assert!(!root_cause["raw"].is_null(), "{name}.raw is present");
    }
    let bottleneck = health["bottleneck"]
        .as_str()
        .expect("health bottleneck string");
    assert!(
        root_causes.contains_key(bottleneck),
        "health bottleneck must name one of the five root causes: {bottleneck}"
    );
}

fn registered_implementation(capability: &str) -> serde_json::Value {
    let registry: serde_json::Value = serde_json::from_slice(
        &fs::read(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join("orchestration/integrations.json"),
        )
        .expect("read integrations registry"),
    )
    .expect("integrations registry is JSON");
    registry["integrations"]
        .as_array()
        .expect("integrations array")
        .iter()
        .find(|entry| entry["capabilityDeclaration"]["id"] == capability)
        .unwrap_or_else(|| panic!("{capability} is not registered"))["capabilityDeclaration"]
        ["implementation"]
        .clone()
}

fn run_capability(
    request: &serde_json::Value,
    request_path: &std::path::Path,
    out: &std::path::Path,
    artifact_root: &std::path::Path,
    capability: &str,
) -> std::process::Output {
    fs::write(
        request_path,
        serde_json::to_vec(request).expect("serialize capability request"),
    )
    .expect("write capability request");
    common::cli()
        .args(["capability", "exec", capability, "--request"])
        .arg(request_path)
        .arg("--out")
        .arg(out)
        .arg("--artifact-root")
        .arg(artifact_root)
        .output()
        .expect("run capability executor")
}

#[test]
fn cli_health_exposes_bottleneck_and_all_five_root_causes() {
    let root = fixture_root("health-contract");
    fs::write(root.join("src/lib.rs"), "pub fn fixture() {}\n").expect("write fixture");

    let root_arg = root.to_string_lossy().to_string();
    let output = code_intel(&["sentrux", "--operation", "health", "--repo", &root_arg]);
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let health: serde_json::Value = serde_json::from_slice(&output.stdout).expect("health JSON");
    assert_health_contract(&health);

    fs::remove_dir_all(&root).expect("remove fixture");
}

#[test]
fn builtin_provider_health_preserves_bottleneck_and_all_five_root_causes() {
    let root = fixture_root("provider-health-contract");
    let repo = root.join("repo");
    fs::create_dir_all(repo.join("src")).expect("create fixture repo");
    fs::write(repo.join("src/lib.rs"), "pub fn fixture() {}\n").expect("write fixture");

    let snapshot_output = common::cli()
        .args(["snapshot", "identity", "--repo"])
        .arg(&repo)
        .args(["--working-tree-policy", "explicit_overlay", "--scope", "."])
        .output()
        .expect("compute request snapshot");
    assert!(
        snapshot_output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&snapshot_output.stdout),
        String::from_utf8_lossy(&snapshot_output.stderr)
    );
    let snapshot_document: serde_json::Value =
        serde_json::from_slice(&snapshot_output.stdout).expect("snapshot JSON");
    let snapshot = snapshot_document["snapshot"].clone();
    let identity = snapshot["identity"]
        .as_str()
        .expect("snapshot identity")
        .to_string();

    let snapshot_request = serde_json::json!({
        "schema": "code-intel-capability-request.v1",
        "capability": "repo.snapshot",
        "contractVersion": 1,
        "implementation": registered_implementation("repo.snapshot"),
        "snapshot": snapshot,
        "options": {"repoPath": repo},
        "inputs": [],
        "effectPolicy": {"allowedEffects": ["repo_read", "local_write"]}
    });
    let snapshot_out = root.join("repo.snapshot");
    let snapshot_run = run_capability(
        &snapshot_request,
        &root.join("repo.snapshot.request.json"),
        &snapshot_out,
        &root,
        "repo.snapshot",
    );
    assert!(
        snapshot_run.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&snapshot_run.stdout),
        String::from_utf8_lossy(&snapshot_run.stderr)
    );
    let snapshot_result: serde_json::Value =
        serde_json::from_slice(&snapshot_run.stdout).expect("snapshot result JSON");
    let snapshot_artifact = &snapshot_result["artifacts"][0];

    let provider_request = serde_json::json!({
        "schema": "code-intel-capability-request.v1",
        "capability": "provider.sentrux-adapt",
        "contractVersion": 1,
        "implementation": registered_implementation("provider.sentrux-adapt"),
        "snapshot": snapshot,
        "options": {"repoPath": repo},
        "inputs": [{
            "schema": "code-intel-artifact-ref.v1",
            "artifactSchema": snapshot_artifact["artifactSchema"],
            "type": snapshot_artifact["type"],
            "path": "repo.snapshot/snapshot.json",
            "sha256": snapshot_artifact["sha256"],
            "consumedSnapshotIdentity": identity
        }],
        "effectPolicy": {
            "allowedEffects": ["repo_read", "local_write", "process_spawn"]
        }
    });
    let provider_out = root.join("evidence.sentrux");
    let provider_run = run_capability(
        &provider_request,
        &root.join("provider.sentrux-adapt.request.json"),
        &provider_out,
        &root,
        "provider.sentrux-adapt",
    );
    assert!(
        provider_run.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&provider_run.stdout),
        String::from_utf8_lossy(&provider_run.stderr)
    );

    let health_artifact: serde_json::Value = serde_json::from_slice(
        &fs::read(provider_out.join("sentrux-capability-sentrux-health.json"))
            .expect("read provider health artifact"),
    )
    .expect("provider health artifact JSON");
    assert_eq!(health_artifact["capabilityId"], "sentrux.health");
    assert_eq!(health_artifact["status"], "succeeded");
    assert_health_contract(&health_artifact["outputs"]["structuredData"]);

    fs::remove_dir_all(&root).expect("remove fixture");
}

#[test]
fn save_baseline_records_the_v6_god_file_identity_list() {
    let root = fixture_root("save-v6");
    fs::write(root.join("src/big.rs"), god_file_body(850)).expect("write god file");
    fs::write(root.join("src/small.rs"), "pub fn small() {}\n").expect("write small file");
    write_rules(&root);

    let root_arg = root.to_string_lossy().to_string();
    let saved = code_intel(&[
        "sentrux",
        "--operation",
        "save_baseline",
        "--repo",
        &root_arg,
    ]);
    assert!(
        saved.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&saved.stdout),
        String::from_utf8_lossy(&saved.stderr)
    );

    let baseline: serde_json::Value = serde_json::from_slice(
        &fs::read(root.join(".sentrux/baseline.json")).expect("read baseline"),
    )
    .expect("parse baseline");
    // v6 (#385): `quality_signal` became the upstream-compatible Quality
    // Signal, which is why the schema itself bumped (DR-0011) -- the
    // `godFiles` identity-ratchet contract this test exists to pin is
    // otherwise unchanged.
    assert_eq!(baseline["schema"], "code-intel-sentrux-baseline.v6");
    let gods = baseline["godFiles"].as_array().expect("godFiles list");
    assert_eq!(gods.len(), 1);
    assert_eq!(gods[0]["path"], "src/big.rs");
    assert_eq!(gods[0]["loc"], 851);
    assert_eq!(gods[0]["rule"], "loc>800");

    fs::remove_dir_all(&root).expect("remove fixture");
}

#[test]
fn cli_check_blocks_a_new_god_file_by_identity() {
    let root = fixture_root("check-new-god");
    fs::write(root.join("src/small.rs"), "pub fn small() {}\n").expect("write small file");
    write_rules(&root);

    let root_arg = root.to_string_lossy().to_string();
    let saved = code_intel(&[
        "sentrux",
        "--operation",
        "save_baseline",
        "--repo",
        &root_arg,
    ]);
    assert!(saved.status.success());

    fs::write(root.join("src/new_god.rs"), god_file_body(900)).expect("write new god file");

    let check = code_intel(&[
        "sentrux",
        "--operation",
        "check",
        "--repo",
        &root_arg,
        "--json",
    ]);
    assert!(
        !check.status.success(),
        "a new god file must fail the CLI check: stdout={}",
        String::from_utf8_lossy(&check.stdout)
    );
    let result: serde_json::Value = serde_json::from_slice(&check.stdout).expect("check JSON");
    assert_eq!(result["verdict"], "fail");
    assert_eq!(result["ruleScope"], "static_and_ratchet");
    let violation = result["blockingViolations"]
        .as_array()
        .expect("violations")
        .iter()
        .find(|violation| violation["rule"] == "god_files_increased")
        .expect("god violation");
    assert_eq!(violation["targets"], serde_json::json!(["src/new_god.rs"]));

    fs::remove_dir_all(&root).expect("remove fixture");
}

fn cli_result(root: &PathBuf, operation: &str, extra: &[&str]) -> (bool, serde_json::Value) {
    let output = common::cli()
        .args(["sentrux", "--operation", operation, "--repo"])
        .arg(root)
        .arg("--json")
        .args(extra)
        .output()
        .expect("run JSON CLI");
    let value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "CLI JSON: {error}; stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    (output.status.success(), value)
}

fn save_fixture(root: &PathBuf) {
    assert!(cli_result(root, "save_baseline", &[]).0);
}

#[test]
fn quality_only_decline_admits_with_measured_advisory_and_immutable_policy() {
    let root = fixture_root("quality-advisory");
    fs::write(root.join("src/a.rs"), "pub fn alpha() {}\n").expect("first source");
    fs::write(root.join("src/b.rs"), "pub fn beta() {}\n").expect("second source");
    write_rules(&root);
    save_fixture(&root);
    let baseline_bytes = fs::read(root.join(".sentrux/baseline.json")).expect("baseline bytes");
    // Real size inequality lowers aggregate Quality, with no new god path,
    // coupling edge or cycle. This is the protective loss approved in A.
    fs::write(root.join("src/a.rs"), god_file_body(600)).expect("grow one file under threshold");
    // A target policy file is not an authority and cannot choose the policy.
    fs::create_dir_all(root.join("orchestration")).expect("target policy directory");
    fs::write(
        root.join("orchestration/sentrux-gate-policy.v1.json"),
        r#"{"policyId":"forged","policyVersion":99,"blockingRules":[]}"#,
    )
    .expect("forged target policy");
    for operation in ["gate", "check"] {
        let (success, result) = cli_result(&root, operation, &[]);
        assert!(success, "{result}");
        assert_eq!(result["schema"], "code-intel-sentrux-gate-result.v1");
        assert_eq!(
            result["policy"],
            serde_json::json!({
                "policyId": "evidence-quality-admission", "policyVersion": 1,
                "sha256": "f97cfed77d80be09acda1331bf449bb21e09ee58d91e19255313bbc5815d11f0"
            })
        );
        assert_eq!(result["measurement"]["scope"], ".");
        assert_eq!(result["measurement"]["engineVersion"], "3.0.0");
        assert_eq!(result["verdict"], "pass");
        assert_eq!(result["blockingViolations"], serde_json::json!([]));
        assert_eq!(result["advisories"][0]["rule"], "quality_degraded");
        let comparisons = result["comparisons"].as_array().expect("comparisons");
        let quality = comparisons
            .iter()
            .find(|comparison| comparison["rule"] == "quality_degraded")
            .expect("Quality comparison");
        assert!(quality["after"].as_f64().unwrap() < quality["before"].as_f64().unwrap());
        assert_eq!(quality["disposition"], "advisory");
        assert_eq!(quality["verdict"], "fail");
        assert!(comparisons
            .iter()
            .filter(|comparison| comparison["rule"] != "quality_degraded")
            .all(|comparison| comparison["verdict"] == "pass"));
    }
    assert_eq!(
        fs::read(root.join(".sentrux/baseline.json")).unwrap(),
        baseline_bytes
    );
    fs::remove_dir_all(&root).expect("remove fixture");
}

#[test]
fn diagnostic_checks_cannot_impersonate_authoritative_admission() {
    let root = fixture_root("diagnostic-check");
    fs::write(root.join("src/lib.rs"), "pub fn fixture() {}\n").expect("source");
    write_rules(&root);
    let (gate_success, gate) = cli_result(&root, "gate", &[]);
    assert!(!gate_success);
    assert_eq!(gate["verdict"], "unknown");
    assert_eq!(gate["baseline"], serde_json::Value::Null);
    assert_eq!(gate["blockingViolations"][0]["rule"], "baseline_missing");
    let (check_success, check) = cli_result(&root, "check", &[]);
    assert!(check_success, "static diagnostic affordance is retained");
    assert_eq!(check["verdict"], "unknown");
    assert_eq!(check["blockingViolations"][0]["rule"], "baseline_missing");
    save_fixture(&root);
    let (success, diagnostic) = cli_result(&root, "check", &["--no-ratchet"]);
    assert!(success);
    assert_eq!(diagnostic["admission"], serde_json::Value::Null);
    assert_eq!(diagnostic["verdict"], "unknown");
    assert!(diagnostic.get("schema").is_none());
    fs::remove_dir_all(&root).expect("remove fixture");
}

#[test]
fn invalid_baseline_identities_and_values_never_admit() {
    let root = fixture_root("invalid-baseline");
    fs::write(root.join("src/lib.rs"), "pub fn fixture() {}\n").expect("source");
    write_rules(&root);
    save_fixture(&root);
    let path = root.join(".sentrux/baseline.json");
    let baseline: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let mutations = [
        (
            "/schema",
            serde_json::json!("code-intel-sentrux-baseline.v5"),
        ),
        ("/engine/id", serde_json::Value::Null),
        ("/engine/version", serde_json::json!("2.2.0")),
        ("/scope", serde_json::json!("src")),
        ("/metrics/quality_signal", serde_json::Value::Null),
        ("/metrics/coupling_score", serde_json::json!(-1)),
        ("/metrics/cycle_count", serde_json::json!(0.5)),
        ("/godFiles", serde_json::json!([{"path":"../outside.rs"}])),
    ];
    for (pointer, replacement) in mutations {
        let mut invalid = baseline.clone();
        *invalid.pointer_mut(pointer).expect("baseline field") = replacement;
        fs::write(&path, serde_json::to_vec(&invalid).unwrap()).unwrap();
        for operation in ["gate", "check"] {
            let (success, result) = cli_result(&root, operation, &[]);
            assert!(!success, "{pointer}: {result}");
            assert_eq!(result["verdict"], "fail");
            assert!(result["baseline"].is_null());
            assert_eq!(
                result["blockingViolations"][0]["rule"],
                "baseline_engine_mismatch"
            );
            assert_eq!(result["comparisons"], serde_json::json!([]));
        }
    }
    for entries in [
        serde_json::json!([{"path":"../outside.rs"}]),
        serde_json::json!([{"path":"C:/outside.rs"}]),
        serde_json::json!([{"path":false}]),
        serde_json::json!([{"path":"src/debt.rs"},{"path":"src/debt.rs"}]),
    ] {
        let mut invalid = baseline.clone();
        // Match the count so this tests malformed identities, not count mismatch.
        invalid["metrics"]["god_file_count"] = serde_json::json!(entries.as_array().unwrap().len());
        invalid["godFiles"] = entries;
        fs::write(&path, serde_json::to_vec(&invalid).unwrap()).unwrap();
        let (success, result) = cli_result(&root, "gate", &[]);
        assert!(!success, "{result}");
        assert_eq!(
            result["blockingViolations"][0]["rule"],
            "baseline_engine_mismatch"
        );
    }
    fs::write(&path, "{not JSON").unwrap();
    let (success, result) = cli_result(&root, "gate", &[]);
    assert!(!success);
    assert_eq!(result["verdict"], "fail");
    fs::remove_dir_all(&root).expect("remove fixture");
}

#[test]
fn coupling_regression_remains_blocking() {
    let root = fixture_root("blocking-coupling");
    fs::write(root.join("src/lib.rs"), "pub fn fixture() {}\n").expect("source");
    write_rules(&root);
    save_fixture(&root);
    fs::write(
        root.join("src/lib.rs"),
        "use std::fs;\npub fn fixture() {}\n",
    )
    .expect("add import");
    for operation in ["gate", "check"] {
        let (success, result) = cli_result(&root, operation, &[]);
        assert!(!success);
        assert_eq!(result["verdict"], "fail");
        assert!(result["blockingViolations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|violation| violation["rule"] == "coupling_increased"));
    }
    fs::remove_dir_all(&root).expect("remove fixture");
}

#[test]
fn aligned_check_preserves_static_failures_even_when_ratchet_passes() {
    let root = fixture_root("blocking-static");
    fs::write(
        root.join("src/lib.rs"),
        "use std::fs;\npub fn fixture() {}\n",
    )
    .expect("source");
    write_rules(&root);
    save_fixture(&root);
    fs::write(
        root.join(".sentrux/rules.toml"),
        "[constraints]\nmax_coupling = \"0\"\n",
    )
    .expect("strict static limit");
    let (gate_success, gate) = cli_result(&root, "gate", &[]);
    assert!(gate_success);
    assert_eq!(gate["verdict"], "pass");
    let (check_success, check) = cli_result(&root, "check", &[]);
    assert!(!check_success);
    assert_eq!(check["verdict"], "fail");
    assert_eq!(check["ruleScope"], "static_and_ratchet");
    assert_eq!(check["blockingViolations"][0]["rule"], "max_coupling");
    assert!(check["comparisons"]
        .as_array()
        .unwrap()
        .iter()
        .any(|comparison| comparison["rule"] == "max_coupling"
            && comparison["disposition"] == "blocking"
            && comparison["verdict"] == "fail"));
    fs::remove_dir_all(&root).expect("remove fixture");
}

#[test]
fn cycles_remain_blocking_in_gate_and_static_check() {
    let root = fixture_root("blocking-cycles");
    fs::write(root.join("Cargo.toml"), "[package]\n").expect("crate marker");
    fs::write(root.join("src/lib.rs"), "mod a;\nmod b;\n").expect("modules");
    fs::write(root.join("src/a.rs"), "pub fn a() {}\n").expect("first module");
    fs::write(root.join("src/b.rs"), "pub fn b() {}\n").expect("second module");
    write_rules(&root);
    save_fixture(&root);
    fs::write(
        root.join("src/a.rs"),
        "use crate::b;\npub fn a() { b::b(); }\n",
    )
    .expect("first edge");
    fs::write(
        root.join("src/b.rs"),
        "use crate::a;\npub fn b() { a::a(); }\n",
    )
    .expect("cycle edge");
    for operation in ["gate", "check"] {
        let (success, result) = cli_result(&root, operation, &[]);
        assert!(!success, "{result}");
        assert_eq!(result["verdict"], "fail");
        let violations = result["blockingViolations"].as_array().unwrap();
        assert!(violations
            .iter()
            .any(|violation| violation["rule"] == "cycles_increased"));
        if operation == "check" {
            assert!(violations
                .iter()
                .any(|violation| violation["rule"] == "max_cycles"));
        }
    }
    fs::remove_dir_all(&root).expect("remove fixture");
}

#[test]
fn aligned_check_preserves_layer_and_boundary_failures() {
    let root = fixture_root("blocking-layers");
    fs::write(root.join("Cargo.toml"), "[package]\n").expect("crate marker");
    fs::write(root.join("src/lib.rs"), "mod a;\nmod b;\n").expect("modules");
    fs::write(
        root.join("src/a.rs"),
        "use crate::b;\npub fn a() { b::b(); }\n",
    )
    .expect("forbidden dependency");
    fs::write(root.join("src/b.rs"), "pub fn b() {}\n").expect("upper module");
    write_rules(&root);
    save_fixture(&root);
    fs::write(
        root.join(".sentrux/rules.toml"),
        concat!(
            "[[layer]]\nmodules = [\"a\"]\n[[layer]]\nmodules = [\"b\"]\n",
            "[[boundary]]\nfrom = [\"a\"]\nforbid = [\"b\"]\ndescription = \"isolation\"\n"
        ),
    )
    .expect("layer and boundary rules");
    assert!(cli_result(&root, "gate", &[]).0, "unchanged ratchet passes");
    let (success, result) = cli_result(&root, "check", &[]);
    assert!(!success, "{result}");
    assert_eq!(result["verdict"], "fail");
    let violations = result["blockingViolations"].as_array().unwrap();
    for rule in ["layer_order", "boundary_dependency"] {
        assert!(violations.iter().any(|violation| violation["rule"] == rule));
        assert!(result["comparisons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|comparison| comparison["rule"] == rule
                && comparison["disposition"] == "blocking"
                && comparison["verdict"] == "fail"));
    }
    fs::remove_dir_all(&root).expect("remove fixture");
}

#[test]
fn replacing_a_grandfathered_god_path_blocks_even_when_count_is_unchanged() {
    let root = fixture_root("blocking-god-swap");
    fs::write(root.join("src/old.rs"), god_file_body(850)).expect("standing debt");
    write_rules(&root);
    save_fixture(&root);
    fs::write(root.join("src/old.rs"), "pub fn old() {}\n").expect("resolve standing debt");
    fs::write(root.join("src/new.rs"), god_file_body(850)).expect("different god path");
    let (success, result) = cli_result(&root, "gate", &[]);
    assert!(!success, "{result}");
    assert_eq!(result["verdict"], "fail");
    let comparison = result["comparisons"]
        .as_array()
        .unwrap()
        .iter()
        .find(|comparison| comparison["rule"] == "god_files_increased")
        .unwrap();
    assert_eq!(comparison["before"], serde_json::json!(["src/old.rs"]));
    assert_eq!(comparison["after"], serde_json::json!(["src/new.rs"]));
    assert_eq!(comparison["verdict"], "fail");
    let violation = result["blockingViolations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|violation| violation["rule"] == "god_files_increased")
        .unwrap();
    assert_eq!(violation["targets"], serde_json::json!(["src/new.rs"]));
    fs::remove_dir_all(&root).expect("remove fixture");
}

#[test]
fn aggregate_only_python_cycle_is_advisory_without_claiming_equivalent_protection() {
    let root = fixture_root("accepted-python-cycle-loss");
    for (path, body) in [
        ("a.py", "import b\n\ndef one():\n    return 11\n"),
        ("b.py", "import c\n\ndef two():\n    return 22\n"),
        ("c.py", "# leaf c\n\ndef three():\n    return 33\n"),
    ] {
        fs::write(root.join(path), body).expect("write acyclic Python fixture");
    }
    write_rules(&root);
    save_fixture(&root);
    let before = cli_result(&root, "scan", &[]).1;
    let baseline_bytes = fs::read(root.join(".sentrux/baseline.json")).expect("baseline");
    fs::write(root.join("b.py"), "import a\n\ndef two():\n    return 22\n")
        .expect("introduce a real Python import cycle without more imports or LOC");
    let after = cli_result(&root, "scan", &[]).1;
    let factors_before = &before["quality_signal_detail"]["root_causes"];
    let factors_after = &after["quality_signal_detail"]["root_causes"];
    assert_eq!(factors_before["acyclicity"]["raw"], 0);
    assert!(factors_after["acyclicity"]["raw"].as_u64().unwrap() > 0);
    assert!(
        factors_after["acyclicity"]["score"].as_u64().unwrap()
            < factors_before["acyclicity"]["score"].as_u64().unwrap()
    );
    assert_eq!(factors_after["equality"], factors_before["equality"]);
    assert!(after["quality_signal"].as_u64().unwrap() < before["quality_signal"].as_u64().unwrap());
    for operation in ["gate", "check"] {
        let (success, admission) = cli_result(&root, operation, &[]);
        assert!(success, "{admission}");
        assert_eq!(admission["verdict"], "pass");
        assert_eq!(admission["blockingViolations"], serde_json::json!([]));
        assert_eq!(admission["advisories"][0]["rule"], "quality_degraded");
        for comparison in admission["comparisons"].as_array().unwrap() {
            if comparison["rule"] == "quality_degraded" {
                assert_eq!(comparison["disposition"], "advisory");
                assert_eq!(comparison["verdict"], "fail");
                assert_eq!(
                    comparison["before"].as_f64(),
                    before["quality_signal"].as_f64()
                );
                assert_eq!(
                    comparison["after"].as_f64(),
                    after["quality_signal"].as_f64()
                );
            } else {
                assert_eq!(comparison["verdict"], "pass");
                assert_eq!(comparison["before"], comparison["after"]);
            }
        }
    }
    assert_eq!(
        fs::read(root.join(".sentrux/baseline.json")).unwrap(),
        baseline_bytes
    );
    fs::remove_dir_all(root).expect("remove fixture");
}
