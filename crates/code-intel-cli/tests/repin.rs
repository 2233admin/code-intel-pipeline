#[path = "../src/content_contract.rs"]
mod content_contract;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

struct TempTree(PathBuf);

impl TempTree {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("code-intel-{label}-{nonce}"));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempTree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn git(repo: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn init_repo(repo: &Path) {
    git(repo, &["init", "--quiet"]);
    git(repo, &["config", "user.name", "Repin Test"]);
    git(repo, &["config", "user.email", "repin@example.invalid"]);
    git(repo, &["config", "core.autocrlf", "false"]);
}

fn commit_all(repo: &Path, message: &str) {
    git(repo, &["add", "."]);
    git(repo, &["commit", "--quiet", "-m", message]);
}

fn sha256_of(path: &Path) -> String {
    content_contract::sha256_hex(&fs::read(path).unwrap())
}

fn repin(repo: &Path, extra_args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_code-intel"));
    command.arg("repin").arg("--repo").arg(repo).arg("--json");
    for arg in extra_args {
        command.arg(arg);
    }
    command.output().unwrap()
}

fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "repin output is not JSON: {error}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

#[test]
fn clean_repo_reports_no_stale_pins() {
    let tree = TempTree::new("repin-clean");
    init_repo(&tree.0);
    fs::write(tree.0.join("source.rs"), "fn a() {}\n").unwrap();
    commit_all(&tree.0, "init");

    let output = repin(&tree.0, &[]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = json(&output);
    assert_eq!(report["clean"], true);
    assert_eq!(report["filesChanged"], 0);
}

#[test]
fn report_mode_detects_a_stale_pin_without_writing() {
    let tree = TempTree::new("repin-detect");
    init_repo(&tree.0);
    fs::write(tree.0.join("source.rs"), "fn a() {}\n").unwrap();
    commit_all(&tree.0, "init source");

    let head_digest = sha256_of(&tree.0.join("source.rs"));
    fs::write(
        tree.0.join("record.json"),
        format!(r#"{{"source":{{"path":"source.rs","sha256":"{head_digest}"}}}}"#),
    )
    .unwrap();
    commit_all(&tree.0, "pin record");

    // Edit the source; record.json's pin is now stale.
    fs::write(tree.0.join("source.rs"), "fn a() { changed(); }\n").unwrap();
    let before = fs::read_to_string(tree.0.join("record.json")).unwrap();

    let output = repin(&tree.0, &[]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = json(&output);
    assert_eq!(report["clean"], false);
    assert_eq!(report["mode"], "report");
    assert_eq!(report["stalePins"][0]["file"], "record.json");
    assert_eq!(report["stalePins"][0]["sites"][0]["old"], head_digest);

    // Report mode must never touch disk.
    assert_eq!(
        fs::read_to_string(tree.0.join("record.json")).unwrap(),
        before
    );
}

#[test]
fn write_mode_fixes_a_stale_pin_and_a_self_referential_chain() {
    let tree = TempTree::new("repin-write");
    init_repo(&tree.0);
    fs::write(tree.0.join("source.rs"), "fn a() {}\n").unwrap();
    fs::write(tree.0.join("measurements.json"), "{\"placeholder\":true}\n").unwrap();
    commit_all(&tree.0, "init source and measurements");

    let source_head = sha256_of(&tree.0.join("source.rs"));
    let measurements_head = sha256_of(&tree.0.join("measurements.json"));
    // record-a pins BOTH source.rs and the shared measurements ledger;
    // record-b only pins the ledger. Editing both source files in one shot
    // means measurements.json's own digest is only known to have changed
    // once the first pass re-hashes it — propagating that to record-b
    // requires a second pass, not a single sweep.
    fs::write(
        tree.0.join("record-a.json"),
        format!(
            r#"{{"source":{{"path":"source.rs","sha256":"{source_head}"}},"measurement":"{measurements_head}"}}"#
        ),
    )
    .unwrap();
    fs::write(
        tree.0.join("record-b.json"),
        format!(r#"{{"measurement":"{measurements_head}"}}"#),
    )
    .unwrap();
    commit_all(&tree.0, "pin records");

    fs::write(tree.0.join("source.rs"), "fn a() { changed(); }\n").unwrap();
    fs::write(
        tree.0.join("measurements.json"),
        "{\"placeholder\":false}\n",
    )
    .unwrap();

    let output = repin(&tree.0, &["--write"]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = json(&output);
    assert_eq!(report["mode"], "write");
    assert!(
        report["passes"].as_u64().unwrap() >= 2,
        "expected a multi-pass fixpoint: {report}"
    );

    let record_a: Value =
        serde_json::from_str(&fs::read_to_string(tree.0.join("record-a.json")).unwrap()).unwrap();
    let record_b: Value =
        serde_json::from_str(&fs::read_to_string(tree.0.join("record-b.json")).unwrap()).unwrap();
    let new_source = sha256_of(&tree.0.join("source.rs"));
    let new_measurements = sha256_of(&tree.0.join("measurements.json"));
    assert_eq!(record_a["source"]["sha256"], new_source);
    assert_eq!(record_a["measurement"], new_measurements);
    assert_eq!(record_b["measurement"], new_measurements);

    // A second run must find nothing left to do.
    let verify = json(&repin(&tree.0, &[]));
    assert_eq!(verify["clean"], true, "not clean after write: {verify}");
}

#[test]
fn retirements_directory_is_never_rewritten() {
    let tree = TempTree::new("repin-frozen");
    init_repo(&tree.0);
    fs::write(tree.0.join("source.rs"), "fn a() {}\n").unwrap();
    commit_all(&tree.0, "init source");

    let source_head = sha256_of(&tree.0.join("source.rs"));
    fs::create_dir_all(tree.0.join("orchestration/retirements/e01")).unwrap();
    fs::write(
        tree.0.join("orchestration/retirements/e01/frozen.json"),
        format!(r#"{{"source":{{"path":"source.rs","sha256":"{source_head}"}}}}"#),
    )
    .unwrap();
    commit_all(&tree.0, "freeze packet");

    fs::write(tree.0.join("source.rs"), "fn a() { changed(); }\n").unwrap();
    let before =
        fs::read_to_string(tree.0.join("orchestration/retirements/e01/frozen.json")).unwrap();

    let output = repin(&tree.0, &["--write"]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let after =
        fs::read_to_string(tree.0.join("orchestration/retirements/e01/frozen.json")).unwrap();
    assert_eq!(
        before, after,
        "frozen retirement packet must never be rewritten"
    );
}

#[test]
fn deleted_source_is_reported_as_an_orphaned_pin() {
    let tree = TempTree::new("repin-orphan");
    init_repo(&tree.0);
    fs::write(tree.0.join("source.rs"), "fn a() {}\n").unwrap();
    commit_all(&tree.0, "init source");
    let source_head = sha256_of(&tree.0.join("source.rs"));
    fs::write(
        tree.0.join("record.json"),
        format!(r#"{{"source":{{"path":"source.rs","sha256":"{source_head}"}}}}"#),
    )
    .unwrap();
    commit_all(&tree.0, "pin record");

    fs::remove_file(tree.0.join("source.rs")).unwrap();

    let output = repin(&tree.0, &[]);
    assert_eq!(output.status.code(), Some(1));
    let report = json(&output);
    assert_eq!(report["orphanedPins"][0]["file"], "record.json");
    assert_eq!(report["orphanedPins"][0]["deletedSourcePath"], "source.rs");
}
