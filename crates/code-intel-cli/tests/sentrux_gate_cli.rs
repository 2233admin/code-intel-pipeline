//! Binary-level coverage for the #165 identity ratchet: the unit tests in
//! `sentrux_gate.rs` pin `run_gate` directly; these prove the same contract
//! holds through the shipped CLI surface (`sentrux --operation save_baseline`
//! / `--operation check`), which is what the authoritative self-scan and CI
//! actually invoke.
mod common;

use std::fs;
use std::path::PathBuf;
use std::process::Command;

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
