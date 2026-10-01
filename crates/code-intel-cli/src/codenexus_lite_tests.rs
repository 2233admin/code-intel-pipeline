use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

static TEST_NONCE: AtomicU64 = AtomicU64::new(0);

/// Unique temp dir per test, mirroring the crate's existing pattern
/// (hardened_git.rs, sentrux_gate.rs). No external tempfile dep.
struct TempRepo(PathBuf);

impl TempRepo {
    fn new() -> Self {
        let nonce = TEST_NONCE.fetch_add(1, Ordering::Relaxed);
        let base = std::env::temp_dir().join(format!(
            "codenexus-lite-test-{}-{}-{nonce}",
            module_path!().replace("::", "-"),
            std::process::id()
        ));
        let repo = base.join("repo");
        fs::create_dir_all(&repo).unwrap();
        TempRepo(base)
    }

    fn repo(&self) -> PathBuf {
        self.0.join("repo")
    }
}

impl Drop for TempRepo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn write(path: &Path, content: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

#[test]
fn generated_path_exclusions() {
    assert!(is_generated_path("target/debug/foo.rs"));
    assert!(is_generated_path("crates/.code-intel/session.json"));
    assert!(is_generated_path("node_modules/x/index.js"));
    assert!(is_generated_path("./staging/out.json"));
    assert!(is_generated_path("work/tmp.rs"));
    assert!(!is_generated_path("crates/tdx-types/src/lib.rs"));
    assert!(!is_generated_path("src/main.rs"));
}

#[test]
fn generated_path_exclusions_are_case_insensitive() {
    assert!(is_generated_path("TARGET/debug/foo.rs"));
    assert!(is_generated_path("Node_Modules/x/index.js"));
    assert!(is_generated_path(".GIT/config"));
}

#[test]
fn hotspots_rank_first_then_dsm_then_largest() {
    let fixture = TempRepo::new();
    let repo = fixture.repo().to_path_buf();
    write(&repo.join("big.rs"), "fn main() {}\n".repeat(100).as_str());
    write(&repo.join("small.rs"), "fn main() {}\n");

    let hotspots = json!({
        "files": [
            { "path": "hot.rs", "maxComplexity": 12, "functionCount": 3 }
        ]
    });
    let dsm = json!({
        "modules": [
            { "metrics": { "risk": 50 }, "files": ["mod_a.rs"] },
            { "metrics": { "risk": 10 }, "files": ["mod_b.rs"] }
        ]
    });
    let target = repo.clone();
    let selected = select_hotspot_files(&repo, &target, Some(&hotspots), Some(&dsm), 8);
    let paths: Vec<&str> = selected
        .iter()
        .map(|v| v["path"].as_str().unwrap())
        .collect();
    // hotspot first, then DSM by risk desc, then largest fallback
    assert_eq!(paths[0], "hot.rs");
    assert_eq!(paths[1], "mod_a.rs");
    assert_eq!(paths[2], "mod_b.rs");
    assert_eq!(paths[3], "big.rs");
    assert_eq!(paths[4], "small.rs");
}

#[test]
fn dsm_risk_ordering_is_descending() {
    let fixture = TempRepo::new();
    let repo = fixture.repo().to_path_buf();
    write(&repo.join("a.rs"), "// a");
    write(&repo.join("b.rs"), "// b");
    let dsm = json!({
        "modules": [
            { "metrics": { "risk": 10 }, "files": ["a.rs"] },
            { "metrics": { "risk": 90 }, "files": ["b.rs"] }
        ]
    });
    let selected = select_hotspot_files(&repo, &repo, None, Some(&dsm), 8);
    let paths: Vec<&str> = selected
        .iter()
        .map(|v| v["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths[0], "b.rs");
    assert_eq!(paths[1], "a.rs");
}

#[test]
fn file_digest_reports_exists_loc_and_first_lines() {
    let fixture = TempRepo::new();
    let repo = fixture.repo().to_path_buf();
    let file = repo.join("src/lib.rs");
    write(&file, "line1\nline2\nline3\n");
    let digest = file_digest(&repo, "src/lib.rs");
    assert_eq!(digest["exists"], json!(true));
    assert_eq!(digest["loc"], 3);
    assert_eq!(digest["firstLines"].as_array().unwrap().len(), 3);
    assert_eq!(digest["firstLines"][0], "line1");

    let missing = file_digest(&repo, "nope.rs");
    assert_eq!(missing["exists"], json!(false));
}

#[test]
fn build_context_shape_matches_contract() {
    let fixture = TempRepo::new();
    let repo = fixture.repo().to_path_buf();
    write(&repo.join("main.rs"), "fn main() {}\n");
    let context = build_context(&repo, &repo, None, None, 8, 12, 0);
    assert_eq!(context["tool"], "codenexus-lite");
    assert_eq!(context["summary"]["files"], 1);
    assert_eq!(context["files"][0]["path"], "main.rs");
    assert_eq!(context["files"][0]["reason"], "largest_code_file");
    assert!(context["nextQueries"].is_array());
    assert!(context["limitations"].is_array());
}

#[test]
fn active_context_is_fixed_time_fallback_only_and_no_history() {
    let fixture = TempRepo::new();
    let repo = fixture.repo().to_path_buf();
    let output = repo.join(".code-intel/codenexus-context.json");
    write(&repo.join("main.rs"), "fn main() {}\n");
    let context = build_active_context(&repo, &repo, &output, iso_from_unix_seconds(1_950), 8, 0);

    assert_eq!(context["generatedAt"], "1970-01-01T00:32:30.000Z");
    assert_eq!(context["output"], output.to_string_lossy().as_ref());
    assert_eq!(context["sources"], json!({"dsm": "", "hotspots": ""}));
    assert_eq!(context["files"][0]["reason"], "largest_code_file");
    assert_eq!(context["files"][0]["recentCommits"], json!([]));
    assert_eq!(context["summary"]["recentCommits"], 0);
}
#[test]
fn relative_path_supports_targets_outside_repo() {
    let fixture = TempRepo::new();
    let repo = fixture.repo();
    let outside = fixture.0.join("external");
    let path = outside.join("main.rs");
    write(&path, "fn main() {}\n");

    assert_eq!(relative_path(&repo, &path), "../external/main.rs");
}
#[test]
fn recent_commits_zero_limit_returns_empty() {
    let fixture = TempRepo::new();
    let commits = recent_commits(&fixture.repo(), "src/lib.rs", 0);
    assert!(commits.is_empty());
}
#[test]
fn walk_code_files_ignores_unreadable_root() {
    let fixture = TempRepo::new();
    let missing = fixture.repo().join("does-not-exist");

    assert!(walk_code_files(&missing).unwrap().is_empty());
}

#[cfg(unix)]
#[test]
fn walk_code_files_keeps_readable_siblings_when_directory_is_unreadable() {
    let fixture = TempRepo::new();
    let repo = fixture.repo();
    write(&repo.join("visible.rs"), "fn visible() {}\n");
    let blocked = repo.join("blocked");
    write(&blocked.join("hidden.rs"), "fn hidden() {}\n");
    fs::set_permissions(
        &blocked,
        <fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o000),
    )
    .unwrap();

    let walked = walk_code_files(&repo).unwrap();
    fs::set_permissions(
        &blocked,
        <fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o755),
    )
    .unwrap();

    assert!(walked.iter().any(|(path, _)| path.ends_with("visible.rs")));
    assert!(!walked.iter().any(|(path, _)| path.ends_with("hidden.rs")));
}
