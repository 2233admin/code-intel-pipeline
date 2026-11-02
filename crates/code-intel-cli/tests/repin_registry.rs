//! `repin` audits the toolchain digest pins of `orchestration/integrations.json`
//! by their declared paths, at any generation (issue #129).

mod common;
#[path = "../src/content_contract/sha256.rs"]
mod content_sha256;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

const REGISTRY: &str = "orchestration/integrations.json";

struct TempTree(PathBuf);

impl TempTree {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("code-intel-registry-{label}-{nonce}"));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.0.join(relative)
    }

    fn read(&self, relative: &str) -> String {
        fs::read_to_string(self.path(relative)).unwrap()
    }

    fn write(&self, relative: &str, text: &str) {
        let path = self.path(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn digest(&self, relative: &str) -> String {
        content_sha256::sha256_hex(&fs::read(self.path(relative)).unwrap())
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

fn commit_all(repo: &Path) {
    git(repo, &["add", "."]);
    git(repo, &["commit", "--quiet", "-m", "fixture"]);
}

fn registry_text(tool: &str, other: &str) -> String {
    format!(
        r#"{{
  "schemaVersion": 1,
  "integrations": [
    {{
      "id": "demo.paired",
      "toolchainDigestEvidence": {{ "algorithm": "sha256", "inputs": ["src/tool.rs", "src/other.rs"] }},
      "capabilityDeclaration": {{ "implementation": {{ "toolchainDigests": ["{tool}", "{other}"] }} }}
    }}
  ]
}}
"#
    )
}

fn fixture(label: &str) -> TempTree {
    let tree = TempTree::new(label);
    git(&tree.0, &["init", "--quiet"]);
    git(&tree.0, &["config", "user.name", "Repin Test"]);
    git(&tree.0, &["config", "user.email", "repin@example.invalid"]);
    git(&tree.0, &["config", "core.autocrlf", "false"]);
    git(&tree.0, &["config", "commit.gpgsign", "false"]);
    git(&tree.0, &["config", "core.hooksPath", ""]);
    tree.write("src/tool.rs", "fn tool() { 0 }\n");
    tree.write("src/other.rs", "fn other() { 0 }\n");
    let text = registry_text(&tree.digest("src/tool.rs"), &tree.digest("src/other.rs"));
    tree.write(REGISTRY, &text);
    commit_all(&tree.0);
    tree
}

fn repin(tree: &TempTree, write: bool) -> Output {
    let mut command = common::cli();
    command.args(["repin", "--repo"]).arg(&tree.0);
    if write {
        command.arg("--write");
    }
    command.output().unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn assert_code(output: &Output, code: i32) {
    assert_eq!(
        output.status.code(),
        Some(code),
        "stdout: {}\nstderr: {}",
        stdout(output),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn second_uncommitted_edit_is_reported_and_resynced_every_round() {
    let tree = fixture("rounds");
    let other = tree.digest("src/other.rs");
    for round in 1..=3 {
        let old = tree.digest("src/tool.rs");
        tree.write("src/tool.rs", &format!("fn tool() {{ {round}{round} }}\n"));
        let first_edit = round == 1;
        assert_code(&repin(&tree, true), 0);
        let after_first = tree.digest("src/tool.rs");
        assert_ne!(old, after_first);
        assert_code(&repin(&tree, false), 0);

        // Edit again without committing: the pin now matches no HEAD digest.
        tree.write(
            "src/tool.rs",
            &format!("fn tool() {{ {round}{round}{round} }}\n"),
        );
        let before = tree.read(REGISTRY);
        let report = repin(&tree, false);
        assert_code(&report, 1);
        let text = stdout(&report);
        assert!(
            text.contains("STALE") && text.contains("src/tool.rs"),
            "{text}"
        );
        assert_eq!(tree.read(REGISTRY), before, "report mode must not write");

        assert_code(&repin(&tree, true), 0);
        let current = tree.digest("src/tool.rs");
        assert_eq!(
            tree.read(REGISTRY),
            before.replace(&after_first, &current),
            "only the tool digest changes (round {round})"
        );
        assert!(tree.read(REGISTRY).contains(&other));
        assert_code(&repin(&tree, false), 0);
        if first_edit {
            commit_all(&tree.0);
        }
    }
}

#[test]
fn verify_lists_the_stale_registry_pin_and_not_the_fresh_one() {
    let tree = fixture("verify");
    let round_one = tree.digest("src/tool.rs");
    tree.write("src/tool.rs", "fn tool() { 1 }\n");
    assert_code(&repin(&tree, true), 0);
    let committed_digest = tree.digest("src/tool.rs");
    tree.write("src/tool.rs", "fn tool() { 2 }\n");
    let before = tree.read(REGISTRY);

    let output = common::cli()
        .arg("verify")
        .arg(&tree.0)
        .arg("--json")
        .output()
        .unwrap();
    assert_code(&output, 1);
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["ok"], false);
    let check = report["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|check| check["name"] == "repin (check-only)")
        .unwrap();
    assert_eq!(check["status"], "fail");
    let pins = check["detail"]["declaredPins"].as_array().unwrap();
    assert_eq!(pins.len(), 1, "{pins:?}");
    assert_eq!(pins[0]["record"], REGISTRY);
    assert_eq!(pins[0]["path"], "src/tool.rs");
    assert_eq!(pins[0]["state"], "stale");
    assert_eq!(pins[0]["declared"], committed_digest);
    assert_eq!(pins[0]["actual"], tree.digest("src/tool.rs"));
    assert_ne!(round_one, committed_digest);
    assert_eq!(tree.read(REGISTRY), before);
}

#[test]
fn a_record_pinning_the_registry_converges_in_one_write() {
    let tree = fixture("chain");
    tree.write(
        "orchestration/internalization/chain.json",
        &format!(
            "{{\n  \"path\": \"{REGISTRY}\",\n  \"sha256\": \"{}\"\n}}\n",
            tree.digest(REGISTRY)
        ),
    );
    commit_all(&tree.0);
    for round in 1..=3 {
        tree.write("src/tool.rs", &format!("fn tool() {{ {round} }}\n"));
        assert_code(&repin(&tree, true), 0);
        assert_code(&repin(&tree, false), 0);
        assert!(
            tree.read("orchestration/internalization/chain.json")
                .contains(&tree.digest(REGISTRY)),
            "chain.json follows the registry (round {round})"
        );
    }
}

#[test]
fn unpaired_integration_is_unresolved_and_never_rewritten() {
    let tree = fixture("unpaired");
    let tool = tree.digest("src/tool.rs");
    let text = format!(
        r#"{{
  "integrations": [
    {{
      "id": "demo.unpaired",
      "toolchainDigestEvidence": {{ "algorithm": "sha256", "inputs": ["src/tool.rs", "src/other.rs"] }},
      "capabilityDeclaration": {{ "implementation": {{ "toolchainDigests": ["{tool}"] }} }}
    }}
  ]
}}
"#
    );
    tree.write(REGISTRY, &text);
    commit_all(&tree.0);

    let report = repin(&tree, false);
    assert_code(&report, 1);
    let out = stdout(&report);
    assert!(
        out.lines().any(|line| line.contains("UNRESOLVED")
            && line.contains(REGISTRY)
            && line.contains("demo.unpaired")),
        "{out}"
    );

    tree.write("src/tool.rs", "fn tool() { 7 }\n");
    let written = repin(&tree, true);
    assert_ne!(written.status.code(), Some(0));
    assert_eq!(tree.read(REGISTRY), text);
}
