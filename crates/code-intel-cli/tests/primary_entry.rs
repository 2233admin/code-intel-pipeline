use std::path::PathBuf;
use std::process::Command;

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_code-intel"))
}

#[test]
fn root_help_leads_with_the_compiled_primary_entry() {
    let output = Command::new(binary())
        .arg("--help")
        .output()
        .expect("run code-intel --help");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("help is UTF-8");
    assert!(stdout.contains("code-intel ."));
    assert!(stdout.contains("code-intel <path> --mode lite|normal|full"));
    assert!(!stdout.contains("archive/invoke-code-intel.ps1"));
}

#[test]
fn root_entry_rejects_a_missing_repository_with_usage_exit_code() {
    let missing =
        std::env::temp_dir().join(format!("code-intel-missing-repo-{}", std::process::id()));
    let output = Command::new(binary())
        .arg(&missing)
        .arg("--mode")
        .arg("lite")
        .output()
        .expect("run code-intel with a missing repository");

    assert_eq!(output.status.code(), Some(64));
    let stderr = String::from_utf8(output.stderr).expect("error is UTF-8");
    assert!(stderr.contains("repository path is not a directory:"));
    assert!(!stderr.contains("unknown command"));
}

#[test]
fn root_entry_keeps_json_machine_readable_on_usage_errors() {
    let missing = std::env::temp_dir().join(format!(
        "code-intel-json-missing-repo-{}",
        std::process::id()
    ));
    let output = Command::new(binary())
        .arg(&missing)
        .args(["--mode", "lite", "--json"])
        .output()
        .expect("run code-intel JSON error path");

    assert_eq!(output.status.code(), Some(64));
    assert!(output.stderr.is_empty());
    let result: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("error output is JSON");
    assert_eq!(result["schema"], "code-intel-primary-result.v1");
    assert_eq!(result["outcome"], "error");
    assert_eq!(result["exitCode"], 64);
    assert!(result["diagnostic"]
        .as_str()
        .is_some_and(|message| message.contains("repository path is not a directory:")));
}

#[test]
fn named_commands_are_not_misclassified_as_repository_paths() {
    let output = Command::new(binary())
        .args(["orchestrate", "--action", "List", "--json"])
        .output()
        .expect("run an existing named command");

    assert!(output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("error is UTF-8");
    assert!(!stderr.contains("unknown primary entry argument"));
}
