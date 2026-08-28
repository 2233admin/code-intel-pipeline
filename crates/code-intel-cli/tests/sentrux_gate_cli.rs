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
fn cli_check_fails_naming_a_new_god_file_and_its_rule_branch() {
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

    let check = code_intel(&["sentrux", "--operation", "check", "--repo", &root_arg]);
    assert!(
        !check.status.success(),
        "a new god file must fail the CLI check: stdout={}",
        String::from_utf8_lossy(&check.stdout)
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
    assert!(
        combined.contains("src/new_god.rs (loc 901, functions 1; rule: loc>800)"),
        "violation must name the file, rule branch, and measured values: {combined}"
    );

    fs::remove_dir_all(&root).expect("remove fixture");
}

#[test]
fn cli_check_stays_green_for_grandfathered_god_files_and_reports_slack() {
    let root = fixture_root("check-grandfather");
    fs::write(root.join("src/big.rs"), god_file_body(850)).expect("write god file");
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

    // Standing debt stays tolerated: same tree, green verdict.
    let unchanged = code_intel(&["sentrux", "--operation", "check", "--repo", &root_arg]);
    assert!(
        unchanged.status.success(),
        "grandfathered god files must not fail: stdout={}",
        String::from_utf8_lossy(&unchanged.stdout)
    );

    // Fixing the god file leaves reclaimable slack, and the green run says so.
    fs::write(root.join("src/big.rs"), "pub fn entry() {}\n").expect("shrink god file");
    let fixed = code_intel(&["sentrux", "--operation", "check", "--repo", &root_arg]);
    assert!(fixed.status.success());
    assert!(
        String::from_utf8_lossy(&fixed.stdout).contains("no longer over threshold"),
        "green run must surface reclaimable slack: {}",
        String::from_utf8_lossy(&fixed.stdout)
    );

    fs::remove_dir_all(&root).expect("remove fixture");
}
