//! Public CLI regression: frozen old HTTP requests plus redirect method and hop contracts.
//! The Python driver checks every body, status, application error, and retained header.

mod common;

use common::TempDir;

use std::process::Command;

#[test]
fn public_http_contract_matches_the_pre_ureq3_mother_copy() {
    let scratch = TempDir::new("http-capture");
    // The driver creates its output directory itself and refuses an existing one.
    let output = scratch.path().join("capture");
    let interpreter = if cfg!(windows) { "python" } else { "python3" };
    let mut driver = Command::new(interpreter);
    for name in common::env_contract::PIPELINE_VARS {
        driver.env_remove(name);
    }
    let result = driver
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args([
            "tests/http_capture_replay.py",
            "--cli",
            env!("CARGO_BIN_EXE_code-intel"),
            "--capture",
            "tests/fixtures/http-migration-ureq2/capture.json",
            "--output",
        ])
        .arg(&output)
        .output()
        .expect("run the real-CLI HTTP capture comparison");
    print!("{}", String::from_utf8_lossy(&result.stdout));
    assert!(
        result.status.success(),
        "public HTTP contract mismatch:\n{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}
