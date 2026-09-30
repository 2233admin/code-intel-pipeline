use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::read_json;

struct SessionRepo(PathBuf);

impl SessionRepo {
    fn new(label: &str) -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "code-intel-session-{label}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(root.join("src")).expect("source directory");
        fs::create_dir_all(root.join(".sentrux/cache")).expect("baseline directory");
        fs::write(root.join("src/main.py"), "def main():\n    return 1\n").expect("source");
        fs::write(
            root.join(".sentrux/baseline.json"),
            "{\"tool\":\"sentrux-lite\"}\n",
        )
        .expect("old canonical baseline");
        fs::write(
            root.join(".sentrux/cache/lite-baseline.json"),
            "{\"quality_signal\":123}\n",
        )
        .expect("old lite baseline");
        Self(root)
    }

    fn invoke(&self, operation: &str, binary: &Path) -> serde_json::Value {
        let mut command = Command::new("pwsh");
        for name in super::common::env_contract::PIPELINE_VARS {
            command.env_remove(name);
        }
        let output = command
            .args(["-NoLogo", "-NoProfile", "-File"])
            .arg(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../legacy/Invoke-SentruxAgentTool.ps1"),
            )
            .arg(operation)
            .arg(&self.0)
            .args(["-SessionId", "contract"])
            .env("CODE_INTEL_RUST_CLI", binary)
            .output()
            .expect("run legacy session entry");
        assert!(
            output.status.success(),
            "status={:?}; stdout={}; stderr={}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).expect("session JSON")
    }

    fn native(&self, operation: &str) -> serde_json::Value {
        let cli = super::common::cli();
        self.invoke(operation, Path::new(cli.get_program()))
    }

    fn old_baselines(&self) -> [Vec<u8>; 2] {
        [
            ".sentrux/baseline.json",
            ".sentrux/cache/lite-baseline.json",
        ]
        .map(|relative| fs::read(self.0.join(relative)).expect("preserved baseline"))
    }
}

impl Drop for SessionRepo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn session_gate_accepts_unchanged_source_and_preserves_old_baselines() {
    let repo = SessionRepo::new("unchanged");
    let before = repo.old_baselines();
    let start = repo.native("session_start");
    assert_eq!(start["gate"]["pass"], true, "{start}");
    let end = repo.native("session_end");
    assert_eq!(end["pass"], true, "{end}");
    assert_eq!(end["delta"], 0);
    assert_eq!(end["metrics_observed_count"], 4);
    assert_eq!(repo.old_baselines(), before);
    assert_eq!(
        read_json(&repo.0.join(".sentrux/agent-sessions/contract.end.json")),
        end
    );
}

#[test]
fn production_import_regression_fails_the_real_native_session_gate() {
    let repo = SessionRepo::new("regression");
    let before = repo.old_baselines();
    assert_eq!(repo.native("session_start")["gate"]["pass"], true);
    fs::write(
        repo.0.join("src/main.py"),
        "import json\ndef main():\n    return json.dumps(1)\n",
    )
    .expect("add production dependency");
    let end = repo.native("session_end");
    assert_eq!(end["pass"], false, "{end}");
    assert_eq!(end["gate"]["pass"], false, "{end}");
    assert_ne!(end["gate"]["exit_code"], 0);
    assert!(
        end["gate"]["metrics"]["coupling"].as_f64().unwrap()
            > end["gate"]["metrics"]["coupling_before"].as_f64().unwrap()
    );
    assert_eq!(repo.old_baselines(), before);
}

#[test]
fn incompatible_session_baseline_is_not_reported_as_quality_regression() {
    let repo = SessionRepo::new("incompatible");
    assert_eq!(repo.native("session_start")["gate"]["pass"], true);
    let path = repo.0.join(".sentrux/cache/native-session-baseline.json");
    let old = b"{\"tool\":\"sentrux-lite\",\"quality_signal\":123}";
    fs::write(&path, old).expect("legacy-shaped session baseline");
    let end = repo.native("session_end");
    assert_eq!(end["pass"], false, "{end}");
    assert_eq!(end["metrics_observed_count"], 0);
    let summary = end["summary"].as_str().expect("diagnostic");
    assert!(summary.contains("baseline engine mismatch"), "{summary}");
    assert!(!summary.contains("Quality degraded"), "{summary}");
    assert_eq!(fs::read(path).unwrap(), old);
}

fn output_fixture(repo: &SessionRepo, output: &str) -> PathBuf {
    #[cfg(windows)]
    let path = repo.0.join("broken-cli.cmd");
    #[cfg(not(windows))]
    let path = repo.0.join("broken-cli");
    #[cfg(windows)]
    let script = format!(
        "@echo off\r\necho {}\r\nexit /b 0\r\n",
        output.replace('>', "^>")
    );
    #[cfg(not(windows))]
    let script = format!("#!/bin/sh\nprintf '%s\\n' '{output}'\n");
    fs::write(&path, script).expect("write faulty process fixture");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    path
}

#[test]
fn successful_process_without_metrics_cannot_pass_the_session_gate() {
    let repo = SessionRepo::new("unparseable");
    assert_eq!(repo.native("session_start")["gate"]["pass"], true);
    let binary = output_fixture(&repo, "invalid protocol output");
    let end = repo.invoke("session_end", &binary);
    assert_eq!(end["pass"], false, "{end}");
    assert_eq!(end["metrics_observed_count"], 0);
    assert!(end["summary"].as_str().unwrap().contains("unparseable"));
}

#[test]
fn partial_process_output_exposes_backfilled_metrics() {
    let repo = SessionRepo::new("partial");
    let start = repo.native("session_start");
    assert_eq!(start["gate"]["pass"], true);
    let quality = start["quality_signal"].as_i64().unwrap();
    let binary = output_fixture(&repo, &format!("Quality: {quality} -> {quality}"));
    let end = repo.invoke("session_end", &binary);
    assert_eq!(end["metrics_observed_count"], 1);
    let gaps = end["backfilled_metrics"].as_array().unwrap();
    for name in ["coupling", "cycles", "god_files"] {
        assert!(gaps.iter().any(|gap| gap == name), "{end}");
        assert!(end["summary"].as_str().unwrap().contains(name), "{end}");
    }
}
