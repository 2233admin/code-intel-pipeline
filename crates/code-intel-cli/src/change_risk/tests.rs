use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use super::git::tip_token;
use super::signals::{is_source_file, is_test_file, looks_like_fix_subject};
use super::*;

fn init_repo(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repo = std::env::temp_dir().join(format!(
        "code-intel-change-risk-{name}-{nonce}-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&repo).unwrap();
    for args in [
        vec!["init", "--quiet"],
        vec!["config", "user.name", "Change Risk Test"],
        vec!["config", "user.email", "change-risk@example.invalid"],
        vec!["config", "commit.gpgsign", "false"],
    ] {
        assert!(Command::new("git")
            .args(args)
            .current_dir(&repo)
            .status()
            .unwrap()
            .success());
    }
    repo
}

fn write_file(repo: &Path, relative: &str, contents: &str) {
    let path = repo.join(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, contents).unwrap();
}

fn commit(repo: &Path, message: &str) {
    assert!(Command::new("git")
        .args(["add", "."])
        .current_dir(repo)
        .status()
        .unwrap()
        .success());
    assert!(Command::new("git")
        .args(["commit", "--quiet", "-m", message])
        .current_dir(repo)
        .status()
        .unwrap()
        .success());
}

#[test]
fn weights_sum_to_one_hundred() {
    assert_eq!(
        WEIGHT_DIFF_SHAPE + WEIGHT_TEST_ASYMMETRY + WEIGHT_BUG_MAGNET + WEIGHT_CHURN,
        100.0
    );
}

#[test]
fn is_source_file_matches_the_crates_src_glob() {
    assert!(is_source_file("crates/code-intel-cli/src/main.rs"));
    assert!(is_source_file("crates/code-intel-cli/src/sub/mod.rs"));
    assert!(!is_source_file("crates/code-intel-cli/Cargo.toml"));
    assert!(!is_source_file("crates/code-intel-cli/tests/it.rs"));
    assert!(!is_source_file("README.md"));
}

#[test]
fn is_test_file_matches_the_documented_heuristics() {
    assert!(is_test_file("crates/code-intel-cli/tests/it.rs"));
    assert!(is_test_file("crates/code-intel-cli/src/foo_test.rs"));
    assert!(is_test_file("crates/code-intel-cli/src/test_foo.rs"));
    assert!(!is_test_file("crates/code-intel-cli/src/main.rs"));
}

#[test]
fn looks_like_fix_subject_matches_english_and_chinese_markers() {
    assert!(looks_like_fix_subject("fix(gate): correct off-by-one"));
    assert!(looks_like_fix_subject("Fix regression in parser"));
    assert!(looks_like_fix_subject("修复解析器越界问题"));
    assert!(looks_like_fix_subject("修正边界条件"));
    assert!(!looks_like_fix_subject("feat: add change risk subcommand"));
}

#[test]
fn tip_token_extracts_the_range_head() {
    assert_eq!(tip_token("origin/main..HEAD"), "HEAD");
    assert_eq!(tip_token("abc123...def456"), "def456");
    assert_eq!(tip_token("origin/main.."), "HEAD");
    assert_eq!(tip_token("abc123"), "abc123");
}

#[test]
fn empty_diff_reports_a_warning_without_erroring() {
    let repo = init_repo("empty-diff");
    write_file(&repo, "README.md", "hello\n");
    commit(&repo, "chore: seed repository");

    let identical = execute(&repo, "HEAD..HEAD", 5).expect("identical range never errors");
    assert_eq!(identical["warning"], "empty_diff");
    assert_eq!(identical["score"], 0);
    assert_eq!(identical["risk_percentile"], 0);
    assert_eq!(identical["files"].as_array().unwrap().len(), 0);

    let bogus = execute(&repo, "definitely-not-a-real-branch..HEAD", 5)
        .expect("an unresolvable revspec degrades to a warning, not an error");
    assert_eq!(bogus["warning"], "empty_diff");

    std::fs::remove_dir_all(&repo).ok();
}

#[test]
fn scoring_is_deterministic_for_the_same_revspec() {
    let repo = init_repo("determinism");
    write_file(&repo, "README.md", "hello\n");
    write_file(&repo, "crates/demo/src/lib.rs", "pub fn demo() {}\n");
    commit(&repo, "feat: seed demo crate");

    write_file(
        &repo,
        "crates/demo/src/lib.rs",
        "pub fn demo() { println!(\"hi\"); }\n",
    );
    commit(&repo, "fix: correct demo output");

    let first = execute(&repo, "HEAD~1..HEAD", 10).expect("scoring succeeds");
    let second = execute(&repo, "HEAD~1..HEAD", 10).expect("scoring succeeds");
    assert_eq!(
        first, second,
        "identical input must produce an identical report"
    );
    assert!(first.get("warning").is_none());
    assert_eq!(first["signals"]["testAsymmetry"]["asymmetric"], true);

    std::fs::remove_dir_all(&repo).ok();
}

#[test]
fn root_commit_diffs_against_the_empty_tree() {
    let repo = init_repo("root-commit");
    write_file(&repo, "crates/demo/src/lib.rs", "pub fn demo() {}\n");
    commit(&repo, "feat: initial commit");

    let result = execute(&repo, "HEAD", 5).expect("root commit scoring never errors");
    assert!(
        result.get("warning").is_none(),
        "root commit has a real diff: {result}"
    );
    assert_eq!(result["files"].as_array().unwrap().len(), 1);

    std::fs::remove_dir_all(&repo).ok();
}

#[test]
fn a_commit_inside_the_scored_range_does_not_count_toward_its_own_files_history() {
    let repo = init_repo("self-reference");
    write_file(&repo, "crates/demo/src/lib.rs", "fn a() {}\n");
    commit(&repo, "feat: seed demo crate");

    // An internal commit inside the range about to be scored: a real fix
    // touching the very file the range's own diff will report as
    // changed. Without the exclusion in `file_commit_history`, this
    // would count toward that file's own bug-magnet tally.
    write_file(
        &repo,
        "crates/demo/src/lib.rs",
        "fn a() { /* patched */ }\n",
    );
    commit(&repo, "fix: patch demo crash");

    write_file(
        &repo,
        "crates/demo/src/lib.rs",
        "fn a() { /* patched */ }\nfn b() {}\n",
    );
    commit(&repo, "feat: extend demo crate");

    // Score the whole three-commit range in one shot, so both the fix
    // commit and the tip commit are "inside" the diff being scored.
    let result = execute(&repo, "HEAD~2..HEAD", 0).expect("scoring succeeds");
    assert!(result.get("warning").is_none());
    let files = result["files"].as_array().expect("files array");
    let lib_file = files
        .iter()
        .find(|file| file["path"] == "crates/demo/src/lib.rs")
        .expect("lib.rs is in the diff");
    assert_eq!(
        lib_file["bugFixCommits180d"], 0,
        "a fix commit inside the scored range must not count toward its own file's bug-magnet tally: {result}"
    );
    assert_eq!(result["signals"]["bugMagnet"]["totalFixCommits"], 0);

    std::fs::remove_dir_all(&repo).ok();
}
