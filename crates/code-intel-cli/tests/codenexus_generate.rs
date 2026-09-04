mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{json, Value};

static TEST_NONCE: AtomicU64 = AtomicU64::new(0);

struct TempRoot(PathBuf);

impl TempRoot {
    fn new() -> Self {
        let nonce = TEST_NONCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "code-intel-codenexus-generate-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create CodeNexus test root");
        Self(path)
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn normalize_context(mut document: Value, repo: &Path) -> Value {
    document["repo"] = json!("@repo@");
    document["target"] = json!("@repo@");
    document["output"] = json!("@output@");
    let canonical_repo = fs::canonicalize(repo).expect("canonicalize test repo");
    let mut repo_prefix = format!("{}/", canonical_repo.to_string_lossy().replace('\\', "/"));
    if let Some(stripped) = repo_prefix.strip_prefix("//?/") {
        repo_prefix = stripped.to_string();
    }
    for file in document["files"].as_array_mut().expect("files array") {
        let references = file["references"].as_array_mut().expect("references array");
        for reference in references.iter_mut() {
            let mut normalized = reference
                .as_str()
                .expect("reference text")
                .replace('\\', "/");
            if let Some(relative) = normalized.strip_prefix(&repo_prefix) {
                normalized = relative.to_string();
            }
            while normalized.starts_with("./") {
                normalized = normalized[2..].to_string();
            }
            *reference = json!(normalized);
        }
        references.sort_by(|left, right| left.as_str().cmp(&right.as_str()));
    }
    document
}

#[test]
fn compiled_route_matches_the_active_powershell_contract() {
    let root = TempRoot::new();
    let repo = root.0.join("repo");
    fs::create_dir_all(repo.join("src")).unwrap();
    fs::create_dir_all(repo.join("docs")).unwrap();
    fs::create_dir_all(repo.join("target")).unwrap();
    fs::write(
        repo.join("src/largest.rs"),
        "pub fn largest() {}\n// largest marker\n",
    )
    .unwrap();
    fs::write(repo.join("src/other.rs"), "fn other() {}\n").unwrap();
    fs::write(
        repo.join("docs/largest-notes.txt"),
        "largest external reference\n",
    )
    .unwrap();
    fs::write(repo.join("target/ignored.rs"), "largest\n".repeat(1_000)).unwrap();
    let output_path = root.0.join("out/codenexus-context.json");

    let output = common::cli()
        .args(["codenexus", "generate", "--repo"])
        .arg(&repo)
        .args(["--target"])
        .arg(repo.join("src/.."))
        .args(["--out"])
        .arg(&output_path)
        .args([
            "--observed-at",
            "1950",
            "--max-files",
            "1",
            "--max-references-per-file",
            "3",
        ])
        .output()
        .expect("run compiled CodeNexus generator");
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout: Value = serde_json::from_slice(&output.stdout).expect("stdout context JSON");
    let written: Value =
        serde_json::from_slice(&fs::read(&output_path).unwrap()).expect("written context JSON");
    assert_eq!(
        stdout, written,
        "stdout and artifact must describe one context"
    );

    let actual = normalize_context(written, &repo);
    assert_eq!(
        actual,
        json!({
            "tool": "codenexus-lite",
            "generatedAt": "1970-01-01T00:32:30.000Z",
            "repo": "@repo@",
            "target": "@repo@",
            "output": "@output@",
            "sources": {"dsm": "", "hotspots": ""},
            "summary": {"files": 1, "references": 3, "recentCommits": 0},
            "files": [{
                "path": "src/largest.rs",
                "reason": "largest_code_file",
                "maxComplexity": null,
                "functionCount": null,
                "riskScore": null,
                "digest": {
                    "exists": true,
                    "loc": 2,
                    "firstLines": ["pub fn largest() {}", "// largest marker"]
                },
                "recentCommits": [],
                "references": [
                    "docs/largest-notes.txt:1:largest external reference",
                    "src/largest.rs:1:pub fn largest() {}",
                    "src/largest.rs:2:// largest marker"
                ]
            }],
            "nextQueries": [
                "Inspect top files by reason=sentrux_hotspot before editing.",
                "Use references to estimate blast radius before changing public functions.",
                "Use recentCommits to identify ownership or churn before accepting a baseline."
            ],
            "limitations": [
                "This is deterministic CodeNexus-lite context, not a semantic embedding graph.",
                "It is designed to be portable on a fresh machine and can be replaced by a full CodeNexus backend later."
            ]
        })
    );
}

#[test]
fn compiled_route_preserves_outside_target_relative_paths() {
    let root = TempRoot::new();
    let repo = root.0.join("repo");
    let outside = root.0.join("external");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(&outside).unwrap();
    fs::write(repo.join("anchor.rs"), "fn anchor() {}\n").unwrap();
    fs::write(
        outside.join("largest.rs"),
        "pub fn largest() {}\n".repeat(20),
    )
    .unwrap();
    let output_path = root.0.join("out/codenexus-context.json");

    let output = common::cli()
        .args(["codenexus", "generate", "--repo"])
        .arg(&repo)
        .args(["--target"])
        .arg(&outside)
        .args(["--out"])
        .arg(&output_path)
        .args(["--observed-at", "0", "--max-files", "1"])
        .output()
        .expect("run outside-target CodeNexus generator");
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let document: Value =
        serde_json::from_slice(&fs::read(&output_path).unwrap()).expect("written context JSON");
    assert_eq!(document["files"][0]["path"], "../external/largest.rs");
    assert_eq!(document["files"][0]["reason"], "largest_code_file");
}
#[test]
fn compiled_route_rejects_unreachable_dsm_and_history_options() {
    let root = TempRoot::new();
    let repo = root.0.join("repo");
    fs::create_dir_all(&repo).unwrap();
    let output_path = root.0.join("codenexus-context.json");

    for unsupported in ["--dsm", "--hotspots", "--max-commits-per-file"] {
        let output = common::cli()
            .args(["codenexus", "generate", "--repo"])
            .arg(&repo)
            .args(["--out"])
            .arg(&output_path)
            .arg(unsupported)
            .arg("unused")
            .output()
            .expect("run rejected CodeNexus option");
        assert_eq!(output.status.code(), Some(64), "unsupported={unsupported}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("unknown codenexus generate argument"),
            "unsupported={unsupported} stderr={}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!output_path.exists());
    }
}
