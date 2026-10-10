mod common;

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

#[test]
fn real_native_projection_preserves_five_factor_units_and_advisory_admission() {
    let root = std::env::temp_dir().join(format!(
        "code-intel-projection-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let repo = root.join("repo");
    let authority = root.join("authority");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(&authority).unwrap();
    fs::write(repo.join("a.py"), "import b\n").unwrap();
    fs::write(repo.join("b.py"), "import c\n").unwrap();
    fs::write(repo.join("c.py"), "value = 1\n").unwrap();
    for args in [
        vec!["init", "-q"],
        vec!["add", "a.py", "b.py", "c.py"],
        vec![
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.com",
            "commit",
            "-q",
            "-m",
            "fixture",
        ],
    ] {
        let output = Command::new("git")
            .current_dir(&repo)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let head = Command::new("git")
        .current_dir(&repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    assert!(head.status.success());
    let head = String::from_utf8(head.stdout).unwrap();
    let saved = common::cli()
        .args(["sentrux", "gate_save"])
        .arg(&repo)
        .output()
        .unwrap();
    assert!(
        saved.status.success(),
        "{}",
        String::from_utf8_lossy(&saved.stderr)
    );
    let baseline = fs::read(repo.join(".sentrux/baseline.json")).unwrap();
    fs::write(repo.join("b.py"), "import a\n").unwrap();

    // Only the child's PATH changes. Do not select an external Sentrux overlay
    // with --doctor-tool-path-prefix: this contract exercises the native engine.
    let binary_dir = PathBuf::from(env!("CARGO_BIN_EXE_code-intel"))
        .parent()
        .unwrap()
        .to_path_buf();
    let mut paths = vec![binary_dir];
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    let run = common::cli()
        .env("PATH", std::env::join_paths(paths).unwrap())
        .args(["run", "execute", "--repo"])
        .arg(&repo)
        .arg("--out")
        .arg(root.join("staging"))
        .arg("--authority-root")
        .arg(&authority)
        .args([
            "--final-name",
            "projection",
            "--doctor-require-repowise",
            "false",
        ])
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    let projection = common::cli()
        .args(["quality-projection", "build", "--artifact-root"])
        .arg(&authority)
        .args(["--repo", "repo", "--repo-path"])
        .arg(&repo)
        .arg("--commit")
        .arg(head.trim())
        .output()
        .unwrap();
    assert!(
        projection.status.success(),
        "{}",
        String::from_utf8_lossy(&projection.stderr)
    );
    let value: Value = serde_json::from_slice(&projection.stdout).unwrap();
    let signal = &value["qualitySignal"];
    assert_eq!(signal["total"]["baseline"], 7677);
    assert_eq!(signal["total"]["current"], 6988);
    assert_eq!(signal["total"]["delta"], -689);
    assert_eq!(signal["bottleneck"], "modularity");
    assert_eq!(
        signal["formulaVersion"],
        "sentrux-upstream@6f8ff3c14b0423e4b58f42d1813d4d5f7fdc1d11+max-floor-0.01"
    );
    let acyclicity = signal["rootCauses"]
        .as_array()
        .unwrap()
        .iter()
        .find(|cause| cause["id"] == "acyclicity")
        .unwrap();
    assert_eq!(acyclicity["raw"]["current"].as_f64(), Some(1.0));
    assert_eq!(acyclicity["score"].as_f64(), Some(5000.0));
    assert_eq!(value["admission"]["verdict"], "pass");
    assert_eq!(
        value["admission"]["advisories"][0]["rule"],
        "quality_degraded"
    );
    assert!(value["admission"]["blockingViolations"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(
        fs::read(repo.join(".sentrux/baseline.json")).unwrap(),
        baseline
    );
    fs::remove_dir_all(root).unwrap();
}
