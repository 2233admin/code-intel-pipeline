use std::path::Path;
use std::process::Command;

#[test]
fn missing_explicit_native_binary_does_not_fall_back_to_another_engine() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let missing = std::env::temp_dir().join(format!(
        "code-intel-missing-explicit-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let output = Command::new("pwsh")
        .args(["-NoLogo", "-NoProfile", "-File"])
        .arg(root.join("../../legacy/Invoke-SentruxAgentTool.ps1"))
        .arg("scan")
        .arg(root)
        .env("CODE_INTEL_RUST_CLI", &missing)
        .output()
        .expect("run legacy entry with missing explicit binary");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Explicit CODE_INTEL_RUST_CLI is missing"),
        "{stderr}"
    );
}
